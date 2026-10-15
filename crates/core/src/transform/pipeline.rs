//! The per-page pipeline: Load → EXIF → Rotate → Crop → Color → Resize.

use std::path::Path;

use image::{imageops::FilterType, DynamicImage};

use super::color::{self, ColorAdjust};
use crate::domain::crop::{expand_crop, BleedInsets, CropBox};
use crate::domain::project::{Page, Project};
use crate::domain::values::{Rotation, Scale};
use crate::error::Result;

/// Immutable snapshot of all parameters for one page's transform.
/// Constructed before any work begins. Once created, project mutations
/// cannot affect an in-flight transform.
#[derive(Clone)]
pub struct PipelineContext<'a> {
    pub source_path: &'a Path,
    pub rotation: Rotation,
    pub crop: Option<CropBox>,
    pub color: ColorAdjust,
    pub scale: Scale,
}

impl<'a> PipelineContext<'a> {
    /// `scale` is the export scale, an export preference rather than part
    /// of the project.
    pub fn from_page(project: &'a Project, page: &'a Page, scale: Scale) -> Self {
        Self {
            source_path: &page.path,
            rotation: page.rotation,
            crop: page.crop,
            color: ColorAdjust::from_project(project),
            scale,
        }
    }
}

/// Full export pipeline: Load → EXIF → Rotate → Crop → Color → Resize.
pub fn transform_page(ctx: &PipelineContext<'_>) -> Result<DynamicImage> {
    transform_page_bleed(ctx, 0).map(|(img, _, _)| img)
}

/// PDF export pipeline: like [`transform_page`] but expands the crop by
/// `bleed` px on all sides, clamped to image bounds. Returns the image, the
/// actual per-side expansion, and the post-crop pre-resize dimensions
/// (needed to place the PDF crop box after any resize).
pub fn transform_page_bleed(
    ctx: &PipelineContext<'_>,
    bleed: u32,
) -> Result<(DynamicImage, BleedInsets, (u32, u32))> {
    let img = load_oriented(ctx.source_path)?;
    let img = rotate(img, ctx.rotation);

    let (crop_box, insets) = match ctx.crop {
        Some(c) if bleed > 0 => {
            let (expanded, insets) = expand_crop(c, bleed, img.width(), img.height());
            (Some(expanded), insets)
        }
        c => (c, BleedInsets::default()),
    };
    let img = crop(img, crop_box);
    let expanded_dims = (img.width(), img.height());

    let img = color::apply(img, &ctx.color);
    let img = resize(img, ctx.scale);
    Ok((img, insets, expanded_dims))
}

/// Load an image from disk and apply its EXIF orientation (a no-op for
/// sources without EXIF).
fn load_oriented(path: &Path) -> Result<DynamicImage> {
    let img = crate::io::load(path)?;
    Ok(crate::io::read_orientation(path).apply(img))
}

/// Apply a user-requested rotation. Zero degrees is a no-op.
#[must_use]
fn rotate(img: DynamicImage, rotation: Rotation) -> DynamicImage {
    match rotation {
        Rotation::ZERO => img,
        Rotation::DEG90 => img.rotate90(),
        Rotation::DEG180 => img.rotate180(),
        Rotation::DEG270 => img.rotate270(),
        _ => img,
    }
}

/// Crop to the given box, clamped to image bounds. None is a no-op.
#[must_use]
fn crop(img: DynamicImage, crop: Option<CropBox>) -> DynamicImage {
    let Some(c) = crop else { return img };
    let (iw, ih) = (img.width(), img.height());
    let x = c.x.min(iw.saturating_sub(1));
    let y = c.y.min(ih.saturating_sub(1));
    let w = c.w.min(iw - x);
    let h = c.h.min(ih - y);
    img.crop_imm(x, y, w, h)
}

/// Apply the export scale. Scale 1.0 is a no-op.
#[must_use]
fn resize(img: DynamicImage, scale: Scale) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::{BlackLevel, Exposure, Saturation, Temperature};

    #[test]
    fn transform_page_smoke() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test.png");
        let test_img =
            image::ImageBuffer::from_fn(64, 48, |x, y| image::Rgb([x as u8, y as u8, 128u8]));
        test_img.save(&img_path).unwrap();

        let project = Project::default();
        let page = Page {
            path: img_path,
            rotation: Default::default(),
            crop: None,
            crop_preset: None,
        };
        let ctx = PipelineContext::from_page(&project, &page, Scale::FULL);

        let result = transform_page(&ctx).unwrap();
        assert_eq!(result.width(), 64);
        assert_eq!(result.height(), 48);
    }

    #[test]
    fn transform_page_with_crop_and_scale() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test.png");
        let test_img =
            image::ImageBuffer::from_fn(128, 128, |x, y| image::Rgb([x as u8, y as u8, 255u8]));
        test_img.save(&img_path).unwrap();

        let project = Project {
            exposure: Exposure::MAX,
            black_level: BlackLevel::MAX,
            temperature: Temperature::MAX,
            saturation: Saturation::MAX,
            ..Project::default()
        };
        let page = Page {
            path: img_path,
            rotation: Default::default(),
            crop: Some(CropBox {
                x: 16,
                y: 16,
                w: 64,
                h: 64,
            }),
            crop_preset: None,
        };
        let ctx = PipelineContext::from_page(&project, &page, Scale::new(0.5));

        let result = transform_page(&ctx).unwrap();
        assert_eq!(result.width(), 32);
        assert_eq!(result.height(), 32);
    }

    fn write_test_image(dir: &std::path::Path, w: u32, h: u32) -> std::path::PathBuf {
        let img_path = dir.join("test.png");
        let test_img =
            image::ImageBuffer::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 128u8]));
        test_img.save(&img_path).unwrap();
        img_path
    }

    fn page_with_crop(path: std::path::PathBuf, crop: CropBox) -> Page {
        Page {
            path,
            rotation: Default::default(),
            crop: Some(crop),
            crop_preset: None,
        }
    }

    #[test]
    fn bleed_grows_output_by_clamped_insets() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = write_test_image(tmp.path(), 128, 128);

        let project = Project::default();
        // Near the left edge: only 4 px available on the left.
        let page = page_with_crop(
            img_path,
            CropBox {
                x: 4,
                y: 30,
                w: 60,
                h: 60,
            },
        );
        let ctx = PipelineContext::from_page(&project, &page, Scale::FULL);

        let (img, insets, expanded) = transform_page_bleed(&ctx, 20).unwrap();
        assert_eq!(
            insets,
            crate::domain::crop::BleedInsets {
                left: 4,
                top: 20,
                right: 20,
                bottom: 20
            }
        );
        assert_eq!(expanded, (84, 100));
        assert_eq!((img.width(), img.height()), expanded);
    }

    #[test]
    fn bleed_zero_matches_transform_page() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = write_test_image(tmp.path(), 64, 64);

        let project = Project::default();
        let page = page_with_crop(
            img_path,
            CropBox {
                x: 8,
                y: 8,
                w: 32,
                h: 32,
            },
        );
        let ctx = PipelineContext::from_page(&project, &page, Scale::FULL);

        let (img, insets, _) = transform_page_bleed(&ctx, 0).unwrap();
        let plain = transform_page(&ctx).unwrap();
        assert!(insets.is_zero());
        assert_eq!((img.width(), img.height()), (plain.width(), plain.height()));
    }

    #[test]
    fn bleed_with_scale_keeps_fraction_geometry() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = write_test_image(tmp.path(), 200, 200);

        let project = Project::default();
        let page = page_with_crop(
            img_path,
            CropBox {
                x: 50,
                y: 50,
                w: 100,
                h: 100,
            },
        );
        let ctx = PipelineContext::from_page(&project, &page, Scale::new(0.5));

        let (img, insets, expanded) = transform_page_bleed(&ctx, 10).unwrap();
        assert_eq!(expanded, (120, 120));
        assert_eq!((img.width(), img.height()), (60, 60));
        let (l, t, r, b) = insets.fractions(expanded.0, expanded.1);
        // 10/120 on every side, regardless of the resize.
        for f in [l, t, r, b] {
            assert!((f - 10.0 / 120.0).abs() < 1e-9);
        }
    }
}
