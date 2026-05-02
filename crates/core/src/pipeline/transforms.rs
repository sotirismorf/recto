use crate::project::types::Page;
use image::{imageops::FilterType, DynamicImage};
use rayon::prelude::*;

/// Apply a user-requested rotation (0, 90, 180, or 270 degrees).
#[must_use]
pub fn apply_rotation(img: DynamicImage, rotation: u16) -> DynamicImage {
    match rotation % 360 {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    }
}

/// Crop the image to the page's [`CropBox`], clamping to image bounds.
#[must_use]
pub fn apply_crop(img: DynamicImage, page: &Page) -> DynamicImage {
    let Some(c) = page.crop else { return img };
    let (iw, ih) = (img.width(), img.height());
    let x = c.x.min(iw.saturating_sub(1));
    let y = c.y.min(ih.saturating_sub(1));
    let w = c.w.min(iw - x);
    let h = c.h.min(ih - y);
    img.crop_imm(x, y, w, h)
}

/// Adjust brightness and contrast in-place on the image's RGB channels.
///
/// Values of `0.0` for both parameters are a no-op.
#[must_use]
pub fn apply_colors(img: DynamicImage, brightness: f32, contrast: f32) -> DynamicImage {
    if brightness == 0.0 && contrast == 0.0 {
        return img;
    }
    let c = (f64::from(contrast) + 1.0).max(0.0);
    let b = (f64::from(brightness) * 128.0).round() as i32;
    let mut rgb = img.into_rgb8();
    rgb.par_iter_mut().for_each(|ch| {
        let v = ((f64::from(*ch) - 128.0) * c + 128.0).round() as i32 + b;
        *ch = v.clamp(0, 255) as u8;
    });
    DynamicImage::ImageRgb8(rgb)
}

/// Resize the image to the page's requested [`OutputSize`] if set.
#[must_use]
pub fn apply_resize(img: DynamicImage, page: &Page) -> DynamicImage {
    let Some(o) = page.output else { return img };
    if o.w == img.width() && o.h == img.height() {
        return img;
    }
    img.resize_exact(o.w, o.h, FilterType::Lanczos3)
}
