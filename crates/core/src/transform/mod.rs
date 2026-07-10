pub mod color;
pub mod context;
pub mod crop;
pub mod exif;
pub mod load;
pub mod resize;
pub mod rotate;

use image::DynamicImage;

use crate::domain::crop::{expand_crop, BleedInsets};
use crate::domain::export::OutputSize;
use crate::error::Result;

pub use context::PipelineContext;

/// Full export pipeline: Load → EXIF → Rotate → Crop → Color → Resize.
pub fn transform_page(ctx: &PipelineContext) -> Result<DynamicImage> {
    transform_page_bleed(ctx, 0).map(|(img, _, _)| img)
}

/// PDF export pipeline: like [`transform_page`] but expands the crop by
/// `bleed` px on all sides, clamped to image bounds. Returns the image, the
/// actual per-side expansion, and the post-crop pre-resize dimensions
/// (needed to place the PDF crop box after any resize).
pub fn transform_page_bleed(
    ctx: &PipelineContext,
    bleed: u32,
) -> Result<(DynamicImage, BleedInsets, (u32, u32))> {
    let img = load::load_image(ctx.source_path)?;
    let img = exif::correct_orientation(img, ctx.source_path);
    let img = rotate::apply(img, ctx.rotation);

    let (crop, insets) = match ctx.crop {
        Some(c) if bleed > 0 => {
            let (expanded, insets) = expand_crop(c, bleed, img.width(), img.height());
            (Some(expanded), insets)
        }
        c => (c, BleedInsets::default()),
    };
    let img = crop::apply(img, crop);
    let expanded_dims = (img.width(), img.height());

    let output = ctx
        .output
        .map(|o| grow_output_for_bleed(o, insets, expanded_dims));

    let img = color::apply(img, ctx.brightness, ctx.contrast, ctx.saturation);
    let img = resize::apply(img, output, ctx.scale);
    Ok((img, insets, expanded_dims))
}

/// Scale a per-page [`OutputSize`] by the bleed expansion ratio so the inner
/// crop still resizes to the requested size instead of being distorted.
fn grow_output_for_bleed(o: OutputSize, insets: BleedInsets, expanded: (u32, u32)) -> OutputSize {
    if insets.is_zero() {
        return o;
    }
    let (ew, eh) = expanded;
    let inner_w = (ew - insets.left - insets.right).max(1);
    let inner_h = (eh - insets.top - insets.bottom).max(1);
    OutputSize {
        w: ((o.w as f64 * ew as f64 / inner_w as f64).round() as u32).max(1),
        h: ((o.h as f64 * eh as f64 / inner_h as f64).round() as u32).max(1),
    }
}

/// Lightweight preview pipeline: Load → EXIF → Rotate → Crop → Color.
/// Skips resize (preview canvas handles scaling) and limits dimensions.
pub fn transform_page_preview(ctx: &PipelineContext) -> Result<DynamicImage> {
    let img = load::load_image(ctx.source_path)?;
    let img = exif::correct_orientation(img, ctx.source_path);
    let img = rotate::apply(img, ctx.rotation);
    let img = crop::apply(img, ctx.crop);
    let img = color::apply(img, ctx.brightness, ctx.contrast, ctx.saturation);
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::crop::CropBox;
    use crate::domain::export::OutputSize;
    use crate::domain::project::{Page, Project};
    use crate::domain::values::{Brightness, Contrast, Saturation};

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
            output: None,
        };
        let ctx = PipelineContext::from_page(&project, &page);

        let result = transform_page(&ctx).unwrap();
        assert_eq!(result.width(), 64);
        assert_eq!(result.height(), 48);
    }

    #[test]
    fn transform_page_with_crop_and_resize() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test.png");
        let test_img =
            image::ImageBuffer::from_fn(128, 128, |x, y| image::Rgb([x as u8, y as u8, 255u8]));
        test_img.save(&img_path).unwrap();

        let project = Project {
            brightness: Brightness::MAX,
            contrast: Contrast::MAX,
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
            output: Some(OutputSize { w: 32, h: 32 }),
        };
        let ctx = PipelineContext::from_page(&project, &page);

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
            output: None,
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
        let ctx = PipelineContext::from_page(&project, &page);

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
        let ctx = PipelineContext::from_page(&project, &page);

        let (img, insets, _) = transform_page_bleed(&ctx, 0).unwrap();
        let plain = transform_page(&ctx).unwrap();
        assert!(insets.is_zero());
        assert_eq!((img.width(), img.height()), (plain.width(), plain.height()));
    }

    #[test]
    fn bleed_with_scale_keeps_fraction_geometry() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = write_test_image(tmp.path(), 200, 200);

        let project = Project {
            export_scale: crate::domain::values::Scale::new(0.5),
            ..Project::default()
        };
        let page = page_with_crop(
            img_path,
            CropBox {
                x: 50,
                y: 50,
                w: 100,
                h: 100,
            },
        );
        let ctx = PipelineContext::from_page(&project, &page);

        let (img, insets, expanded) = transform_page_bleed(&ctx, 10).unwrap();
        assert_eq!(expanded, (120, 120));
        assert_eq!((img.width(), img.height()), (60, 60));
        let (l, t, r, b) = insets.fractions(expanded.0, expanded.1);
        // 10/120 on every side, regardless of the resize.
        for f in [l, t, r, b] {
            assert!((f - 10.0 / 120.0).abs() < 1e-9);
        }
    }

    #[test]
    fn bleed_compensates_per_page_output_size() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = write_test_image(tmp.path(), 200, 200);

        let project = Project::default();
        let page = Page {
            output: Some(OutputSize { w: 50, h: 50 }),
            ..page_with_crop(
                img_path,
                CropBox {
                    x: 50,
                    y: 50,
                    w: 100,
                    h: 100,
                },
            )
        };
        let ctx = PipelineContext::from_page(&project, &page);

        let (img, insets, expanded) = transform_page_bleed(&ctx, 10).unwrap();
        assert_eq!(expanded, (120, 120));
        // Inner crop (100 px) must land on 50 px, so the full image is 60 px.
        assert_eq!((img.width(), img.height()), (60, 60));
        let (l, _, r, _) = insets.fractions(expanded.0, expanded.1);
        let inner_w = img.width() as f64 * (1.0 - l - r);
        assert!((inner_w - 50.0).abs() < 1.0);
    }
}
