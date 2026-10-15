use std::path::PathBuf;

use gtk::{gdk_pixbuf, glib};
use image::DynamicImage;

use crate::pixbuf::PixelData;
use recto_core::transform::color::ColorAdjust;
use recto_core::{CropBox, Project};

pub(crate) fn pixbuf_to_dynamic_image(pb: &gdk_pixbuf::Pixbuf) -> DynamicImage {
    let w = pb.width() as u32;
    let h = pb.height() as u32;
    let stride = pb.rowstride() as usize;
    let src = pb.read_pixel_bytes();
    let channels = if pb.has_alpha() { 4 } else { 3 };
    let row_len = w as usize * 3;
    let mut pixels = Vec::with_capacity(row_len * h as usize);
    for row in src.chunks(stride).take(h as usize) {
        if channels == 3 {
            // Rows are contiguous RGB apart from stride padding: copy whole rows.
            pixels.extend_from_slice(&row[..row_len]);
        } else {
            for px in row[..w as usize * 4].as_chunks::<4>().0 {
                pixels.extend_from_slice(&px[..3]);
            }
        }
    }
    DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels).expect("valid rgb image"))
}

pub(crate) fn dynamic_image_to_pixbuf(img: &DynamicImage) -> gdk_pixbuf::Pixbuf {
    let rgb = img.as_rgb8().expect("rgb8 image");
    let (w, h) = rgb.dimensions();
    let stride = w as usize * 3;
    let data = rgb.as_raw().clone();
    let bytes = glib::Bytes::from(data.as_slice());
    gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gdk_pixbuf::Colorspace::Rgb,
        false,
        8,
        w as i32,
        h as i32,
        stride as i32,
    )
}

/// Common GL implementations cap textures at 16384px per side; anything
/// larger would fail to upload. Below that, the preview is full resolution.
const MAX_PREVIEW_PX: i32 = 16384;

pub(crate) struct ColorReq {
    path: PathBuf,
    rotation: u32,
    crop: Option<CropBox>,
    color: ColorAdjust,
}

/// Worker-side cache of the decoded, oriented, rotated, and cropped image.
/// Color adjustments are cheap compared to decoding a full scan, so keeping
/// the base pixels lets slider drags skip everything but the color math.
#[derive(Default)]
pub(crate) struct BaseCache {
    entry: Option<(PathBuf, u32, Option<CropBox>, DynamicImage)>,
}

impl BaseCache {
    fn get(&self, req: &ColorReq) -> Option<&DynamicImage> {
        self.entry
            .as_ref()
            .filter(|(path, rotation, crop, _)| {
                *path == req.path && *rotation == req.rotation && *crop == req.crop
            })
            .map(|(_, _, _, img)| img)
    }

    fn put(&mut self, req: &ColorReq, img: DynamicImage) {
        self.entry = Some((req.path.clone(), req.rotation, req.crop, img));
    }
}

impl ColorReq {
    /// Request a preview of page `index` with the project's colour settings.
    pub(crate) fn for_page(project: &Project, index: usize) -> Option<Self> {
        let page = project.pages.get(index)?;
        Some(Self {
            path: page.path.clone(),
            rotation: page.rotation.as_degrees() as u32,
            crop: page.crop,
            color: ColorAdjust::from_project(project),
        })
    }
}

pub(crate) fn render_preview(req: &ColorReq, cache: &mut BaseCache) -> Option<PixelData> {
    let base = match cache.get(req) {
        Some(img) => img.clone(),
        None => {
            let pb = crate::pixbuf::load_page(&req.path, req.rotation, None)?;
            let pb = match req.crop {
                Some(c) => {
                    let x = (c.x as i32).min(pb.width().saturating_sub(1));
                    let y = (c.y as i32).min(pb.height().saturating_sub(1));
                    let w = (c.w as i32).min(pb.width() - x).max(1);
                    let h = (c.h as i32).min(pb.height() - y).max(1);
                    pb.new_subpixbuf(x, y, w, h)
                }
                None => pb,
            };
            let pb = scale_down(pb, MAX_PREVIEW_PX);
            let img = pixbuf_to_dynamic_image(&pb);
            cache.put(req, img.clone());
            img
        }
    };
    let img = recto_core::transform::color::apply(base, &req.color);
    Some(PixelData::from_pixbuf(&dynamic_image_to_pixbuf(&img)))
}

pub(crate) fn scale_down(pb: gdk_pixbuf::Pixbuf, max_px: i32) -> gdk_pixbuf::Pixbuf {
    if pb.width() <= max_px && pb.height() <= max_px {
        return pb;
    }
    let s = (max_px as f64 / pb.width() as f64).min(max_px as f64 / pb.height() as f64);
    let nw = ((pb.width() as f64 * s).round() as i32).max(1);
    let nh = ((pb.height() as f64 * s).round() as i32).max(1);
    pb.scale_simple(nw, nh, gdk_pixbuf::InterpType::Bilinear)
        .unwrap_or(pb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_down_small_image_unchanged() {
        let pb = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 100, 100).unwrap();
        let result = scale_down(pb, 2048);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn pixbuf_to_image_roundtrip() {
        let pb = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 10, 10).unwrap();
        let img = pixbuf_to_dynamic_image(&pb);
        let pb2 = dynamic_image_to_pixbuf(&img);
        assert_eq!(pb2.width(), 10);
        assert_eq!(pb2.height(), 10);
    }
}
