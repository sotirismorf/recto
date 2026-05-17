pub mod color;
pub mod context;
pub mod crop;
pub mod exif;
pub mod load;
pub mod resize;
pub mod rotate;

use image::DynamicImage;

use crate::error::Result;

pub use context::PipelineContext;

/// Full export pipeline: Load → EXIF → Rotate → Crop → Color → Resize.
pub fn transform_page(ctx: &PipelineContext) -> Result<DynamicImage> {
    let img = load::load_image(ctx.source_path)?;
    let img = exif::correct_orientation(img, ctx.source_path);
    let img = rotate::apply(img, ctx.rotation);
    let img = crop::apply(img, ctx.crop);
    let img = color::apply(img, ctx.brightness, ctx.contrast, ctx.saturation);
    let img = resize::apply(img, ctx.output, ctx.scale);
    Ok(img)
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
}
