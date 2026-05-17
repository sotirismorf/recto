use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk_pixbuf, glib};
use image::DynamicImage;

use crate::latest::RequestDedup;
use recto_core::{Brightness, Contrast, CropBox, Project, Saturation};

pub(crate) fn pixbuf_to_dynamic_image(pb: &gdk_pixbuf::Pixbuf) -> DynamicImage {
    let w = pb.width() as u32;
    let h = pb.height() as u32;
    let stride = pb.rowstride() as usize;
    let src = pb.read_pixel_bytes();
    let channels = if pb.has_alpha() { 4 } else { 3 };
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 3);
    for row in 0..h {
        for col in 0..w {
            let si = row as usize * stride + col as usize * channels;
            pixels.push(src[si]);
            pixels.push(src[si + 1]);
            pixels.push(src[si + 2]);
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

pub(crate) struct ColorReq {
    path: PathBuf,
    rotation: u32,
    crop: Option<CropBox>,
    brightness: Brightness,
    contrast: Contrast,
    saturation: Saturation,
}

pub(crate) struct ColorResult {
    pub(crate) bytes: glib::Bytes,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) rowstride: i32,
    pub(crate) has_alpha: bool,
}

pub(crate) fn send_preview_req(
    project: &Project,
    selection: &gtk::MultiSelection,
    req: &RequestDedup<ColorReq>,
) {
    let bs = selection.selection();
    let first = match gtk::BitsetIter::init_first(&bs) {
        Some((_, idx)) => idx as usize,
        None => {
            req.send_dummy();
            return;
        }
    };
    let Some(page) = project.pages.get(first) else {
        return;
    };
    req.send(ColorReq {
        path: page.path.clone(),
        rotation: page.rotation.as_degrees() as u32,
        crop: page.crop,
        brightness: project.brightness,
        contrast: project.contrast,
        saturation: project.saturation,
    });
}

pub(crate) fn render_preview(req: &ColorReq) -> Option<ColorResult> {
    let pb = gdk_pixbuf::Pixbuf::from_file(&req.path).ok()?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let pb = crate::widgets::page_item::rotate(&pb, req.rotation);
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
    let pb = scale_down(pb, 2048);
    let img = pixbuf_to_dynamic_image(&pb);
    let img = recto_core::transform::color::apply(img, req.brightness, req.contrast, req.saturation);
    let pb = dynamic_image_to_pixbuf(&img);
    Some(ColorResult {
        bytes: pb.read_pixel_bytes(),
        width: pb.width(),
        height: pb.height(),
        rowstride: pb.rowstride(),
        has_alpha: pb.has_alpha(),
    })
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
