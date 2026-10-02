use std::sync::OnceLock;

use image::DynamicImage;
use rayon::prelude::*;

use crate::domain::project::Project;
use crate::domain::values::{BlackLevel, Exposure, Saturation, Temperature, Tint};

/// The project's colour adjustments, applied together by [`apply`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ColorAdjust {
    pub exposure: Exposure,
    pub black_level: BlackLevel,
    pub temperature: Temperature,
    pub tint: Tint,
    pub saturation: Saturation,
}

impl ColorAdjust {
    pub fn from_project(project: &Project) -> Self {
        Self {
            exposure: project.exposure,
            black_level: project.black_level,
            temperature: project.temperature,
            tint: project.tint,
            saturation: project.saturation,
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// Per-channel white-balance gain at the ends of the temperature and tint
/// sliders, in stops. Small on purpose: these are corrections, not grading.
const WB_STOPS: f32 = 0.3;

/// Rec. 709 / sRGB luminance weights for linear RGB.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Entries in the linear → sRGB encoding table. Fine enough that adjacent
/// entries never skip an 8-bit output code, even in the shadows.
const ENCODE_LUT_SIZE: usize = 16384;

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn decode_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))
}

fn encode_lut() -> &'static [u8] {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    LUT.get_or_init(|| {
        (0..ENCODE_LUT_SIZE)
            .map(|i| {
                let v = linear_to_srgb(i as f32 / (ENCODE_LUT_SIZE - 1) as f32);
                (v * 255.0).round() as u8
            })
            .collect()
    })
}

fn encode(lut: &[u8], v: f32) -> u8 {
    let i = (v.clamp(0.0, 1.0) * (ENCODE_LUT_SIZE - 1) as f32 + 0.5) as usize;
    lut[i]
}

/// Linear channel gains for temperature (red ↔ blue) and tint (green ↔
/// magenta), normalised so a neutral grey keeps its luminance.
fn white_balance_gains(temperature: Temperature, tint: Tint) -> [f32; 3] {
    let t = temperature.as_f32() * WB_STOPS;
    let m = tint.as_f32() * WB_STOPS;
    let gains = [t.exp2(), (-m).exp2(), (-t).exp2()];
    let y: f32 = gains.iter().zip(LUMA).map(|(g, w)| g * w).sum();
    gains.map(|g| g / y)
}

/// The adjustments as one affine map on linear RGB: `out = m · in + offset`.
///
/// Black level, white balance, exposure and saturation are all linear in
/// linear light, so they compose into a single 3×3 matrix and offset.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinearMap {
    pub m: [[f32; 3]; 3],
    pub offset: [f32; 3],
}

impl ColorAdjust {
    pub fn linear_map(&self) -> LinearMap {
        let black = self.black_level.as_f32();
        let gain = self.exposure.as_f32().exp2() / (1.0 - black);
        let wb = white_balance_gains(self.temperature, self.tint).map(|g| g * gain);
        let sat = self.saturation.as_f32() + 1.0;
        // Saturation around luminance: out_i = sat·c_i + (1 − sat)·Σ_j L_j·c_j,
        // applied after the per-channel gains c_j = wb_j · (in_j − black).
        let m: [[f32; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let diag = if i == j { sat } else { 0.0 };
                (diag + (1.0 - sat) * LUMA[j]) * wb[j]
            })
        });
        let offset = m.map(|row| -black * row.iter().sum::<f32>());
        LinearMap { m, offset }
    }
}

/// Apply the colour adjustments in linear light. Identity is a no-op.
///
/// Pipeline per pixel: decode sRGB → [`ColorAdjust::linear_map`] (black
/// level, white balance, exposure, saturation) → encode sRGB. Working in
/// linear light makes exposure and white balance behave like changes to the
/// light itself, so colour ratios between paper and ink are preserved.
#[must_use]
pub fn apply(img: DynamicImage, adjust: &ColorAdjust) -> DynamicImage {
    if adjust.is_identity() {
        return img;
    }
    let decode = decode_lut();
    let enc = encode_lut();
    let LinearMap { m, offset } = adjust.linear_map();

    let mut rgb = img.into_rgb8();
    // Rows in parallel; per-pixel work stays branch-free and unrolled.
    let row_len = rgb.width() as usize * 3;
    rgb.par_chunks_mut(row_len.max(3)).for_each(|row| {
        for px in row.as_chunks_mut::<3>().0 {
            let r = decode[px[0] as usize];
            let g = decode[px[1] as usize];
            let b = decode[px[2] as usize];
            px[0] = encode(enc, m[0][0] * r + m[0][1] * g + m[0][2] * b + offset[0]);
            px[1] = encode(enc, m[1][0] * r + m[1][1] * g + m[1][2] * b + offset[1]);
            px[2] = encode(enc, m[2][0] * r + m[2][1] * g + m[2][2] * b + offset[2]);
        }
    });
    DynamicImage::ImageRgb8(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rgb: [u8; 3], adjust: ColorAdjust) -> [u8; 3] {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb(rgb)));
        apply(img, &adjust).into_rgb8().get_pixel(0, 0).0
    }

    #[test]
    fn identity_is_noop() {
        assert_eq!(run([12, 128, 250], ColorAdjust::default()), [12, 128, 250]);
    }

    #[test]
    fn encode_decode_round_trips_every_code() {
        let decode = decode_lut();
        for v in 0..=255u8 {
            assert_eq!(encode(encode_lut(), decode[v as usize]), v);
        }
    }

    #[test]
    fn plus_one_stop_doubles_linear_light() {
        // sRGB 118 ≈ 18% grey; doubled to 36% ≈ sRGB 163.
        let adjust = ColorAdjust {
            exposure: Exposure::new(1.0),
            ..Default::default()
        };
        let [r, g, b] = run([118, 118, 118], adjust);
        assert_eq!((r, g), (g, b));
        assert!((162..=164).contains(&r), "{r}");
    }

    #[test]
    fn black_level_darkens_ink_and_keeps_white() {
        let adjust = ColorAdjust {
            black_level: BlackLevel::new(0.05),
            ..Default::default()
        };
        assert_eq!(run([255, 255, 255], adjust), [255, 255, 255]);
        let ink = run([90, 90, 90], adjust)[0];
        let paper = run([225, 225, 225], adjust)[0];
        assert!(90 - ink > 225 - paper, "ink {ink}, paper {paper}");
    }

    #[test]
    fn warmer_temperature_shifts_grey_to_yellow() {
        let adjust = ColorAdjust {
            temperature: Temperature::new(0.5),
            ..Default::default()
        };
        let [r, _, b] = run([150, 150, 150], adjust);
        assert!(r > 150 && b < 150, "r {r}, b {b}");
    }

    #[test]
    fn magenta_tint_lowers_green() {
        let adjust = ColorAdjust {
            tint: Tint::new(0.5),
            ..Default::default()
        };
        let [r, g, b] = run([150, 150, 150], adjust);
        assert!(g < r && g < b, "{r} {g} {b}");
    }

    #[test]
    fn full_desaturation_gives_grey() {
        let adjust = ColorAdjust {
            saturation: Saturation::MIN,
            ..Default::default()
        };
        let [r, g, b] = run([200, 120, 40], adjust);
        assert!(r == g && g == b, "{r} {g} {b}");
    }
}
