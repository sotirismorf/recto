use image::DynamicImage;
use rayon::prelude::*;

use crate::domain::values::{Brightness, Contrast};

/// Adjust brightness and contrast. Both at zero is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, brightness: Brightness, contrast: Contrast) -> DynamicImage {
    if brightness == Brightness::ZERO && contrast == Contrast::ZERO {
        return img;
    }
    let c = (contrast.as_f32() as f64 + 1.0).max(0.0);
    let b = (brightness.as_f32() as f64 * 128.0).round() as i32;
    let mut rgb = img.into_rgb8();
    rgb.par_iter_mut().for_each(|ch| {
        let v = ((f64::from(*ch) - 128.0) * c + 128.0).round() as i32 + b;
        *ch = v.clamp(0, 255) as u8;
    });
    DynamicImage::ImageRgb8(rgb)
}
