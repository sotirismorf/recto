use image::DynamicImage;

use crate::domain::crop::CropBox;

/// Crop to the given box, clamped to image bounds. None is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, crop: Option<CropBox>) -> DynamicImage {
    let Some(c) = crop else { return img };
    let (iw, ih) = (img.width(), img.height());
    let x = c.x.min(iw.saturating_sub(1));
    let y = c.y.min(ih.saturating_sub(1));
    let w = c.w.min(iw - x);
    let h = c.h.min(ih - y);
    img.crop_imm(x, y, w, h)
}
