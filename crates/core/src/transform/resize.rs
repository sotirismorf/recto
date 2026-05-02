use image::{imageops::FilterType, DynamicImage};

use crate::domain::export::OutputSize;

/// Resize to output dimensions. None or already-correct dimensions is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, output: Option<OutputSize>) -> DynamicImage {
    let Some(o) = output else { return img };
    if o.w == img.width() && o.h == img.height() {
        return img;
    }
    img.resize_exact(o.w, o.h, FilterType::Lanczos3)
}
