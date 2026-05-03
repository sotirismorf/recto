use image::{imageops::FilterType, DynamicImage};

use crate::domain::export::OutputSize;
use crate::domain::values::Scale;

/// Resize to output dimensions, then apply export scale.
/// None output size uses natural image dimensions.
/// Scale 1.0 is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, output: Option<OutputSize>, scale: Scale) -> DynamicImage {
    let (w, h) = match output {
        Some(o) => (o.w, o.h),
        None => (img.width(), img.height()),
    };
    let factor = scale.as_factor();
    let nw = if factor >= 1.0 {
        w
    } else {
        ((w as f64 * factor).round() as u32).max(1)
    };
    let nh = if factor >= 1.0 {
        h
    } else {
        ((h as f64 * factor).round() as u32).max(1)
    };
    if nw == img.width() && nh == img.height() {
        return img;
    }
    img.resize_exact(nw, nh, FilterType::Lanczos3)
}
