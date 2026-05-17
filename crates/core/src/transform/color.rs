use image::DynamicImage;
use rayon::prelude::*;

use crate::domain::values::{Brightness, Contrast, Saturation};

/// Adjust brightness, contrast, and saturation in one pass.
/// All three at zero is a no-op.
///
/// Pipeline per pixel: contrast around mid-gray → brightness shift →
/// saturation interpolation between Rec. 601 luminance and the channel value.
#[must_use]
pub fn apply(
    img: DynamicImage,
    brightness: Brightness,
    contrast: Contrast,
    saturation: Saturation,
) -> DynamicImage {
    if brightness == Brightness::ZERO
        && contrast == Contrast::ZERO
        && saturation == Saturation::ZERO
    {
        return img;
    }
    let c = (contrast.as_f32() as f64 + 1.0).max(0.0);
    let b = (brightness.as_f32() as f64 * 128.0).round();
    let s = saturation.as_f32() as f64 + 1.0;
    let mut rgb = img.into_rgb8();
    rgb.par_chunks_mut(3).for_each(|px| {
        let r0 = f64::from(px[0]);
        let g0 = f64::from(px[1]);
        let b0 = f64::from(px[2]);

        let r1 = (r0 - 128.0) * c + 128.0 + b;
        let g1 = (g0 - 128.0) * c + 128.0 + b;
        let b1 = (b0 - 128.0) * c + 128.0 + b;

        let luma = 0.299 * r1 + 0.587 * g1 + 0.114 * b1;
        let r2 = luma + (r1 - luma) * s;
        let g2 = luma + (g1 - luma) * s;
        let b2 = luma + (b1 - luma) * s;

        px[0] = r2.round().clamp(0.0, 255.0) as u8;
        px[1] = g2.round().clamp(0.0, 255.0) as u8;
        px[2] = b2.round().clamp(0.0, 255.0) as u8;
    });
    DynamicImage::ImageRgb8(rgb)
}
