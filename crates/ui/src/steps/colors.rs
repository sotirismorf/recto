use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk_pixbuf, glib};

use crate::app::State;
use recto_core::project::CropBox;

pub(crate) struct ColorReq {
    id: u64,
    path: PathBuf,
    rotation: u32,
    crop: Option<CropBox>,
    brightness: f32,
    contrast: f32,
}

pub(crate) struct ColorResult {
    pub(crate) id: u64,
    pub(crate) bytes: glib::Bytes,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) rowstride: i32,
    pub(crate) has_alpha: bool,
}

pub(crate) fn send_preview_req(
    state: &State,
    selection: &gtk::MultiSelection,
    req_id: &Rc<Cell<u64>>,
    tx: &async_channel::Sender<ColorReq>,
) {
    let bs = selection.selection();
    let first = match gtk::BitsetIter::init_first(&bs) {
        Some((_, idx)) => idx as usize,
        None => {
            req_id.set(req_id.get().wrapping_add(1));
            return;
        }
    };
    let project = state.borrow();
    let Some(page) = project.pages.get(first) else {
        return;
    };
    let id = req_id.get().wrapping_add(1);
    req_id.set(id);
    let req = ColorReq {
        id,
        path: page.path.clone(),
        rotation: page.rotation as u32,
        crop: page.crop,
        brightness: project.brightness,
        contrast: project.contrast,
    };
    drop(project);
    let _ = tx.send_blocking(req);
}

pub(crate) fn render_preview(req: &ColorReq) -> Option<ColorResult> {
    let pb = gdk_pixbuf::Pixbuf::from_file(&req.path).ok()?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let pb = super::page_item::rotate(&pb, req.rotation);
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
    let pb = adjust_colors(pb, req.brightness, req.contrast);
    Some(ColorResult {
        id: req.id,
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

pub(crate) fn adjust_colors(pb: gdk_pixbuf::Pixbuf, brightness: f32, contrast: f32) -> gdk_pixbuf::Pixbuf {
    if brightness == 0.0 && contrast == 0.0 {
        return pb;
    }
    let c = (contrast + 1.0_f32).max(0.0);
    let b = (brightness * 128.0_f32).round() as i32;
    let channels = if pb.has_alpha() { 4 } else { 3 } as usize;
    let w = pb.width() as usize;
    let h = pb.height() as usize;
    let src_stride = pb.rowstride() as usize;
    let src_bytes = pb.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let dst_stride = w * channels;
    let mut dst = vec![0u8; h * dst_stride];
    for row in 0..h {
        for col in 0..w {
            let si = row * src_stride + col * channels;
            let di = row * dst_stride + col * channels;
            for ch in 0..3_usize {
                let v = ((src[si + ch] as f32 - 128.0) * c + 128.0).round() as i32 + b;
                dst[di + ch] = v.clamp(0, 255) as u8;
            }
            if channels == 4 {
                dst[di + 3] = src[si + 3];
            }
        }
    }
    let bytes = glib::Bytes::from(dst.as_slice());
    gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        pb.colorspace(),
        pb.has_alpha(),
        pb.bits_per_sample(),
        pb.width(),
        pb.height(),
        dst_stride as i32,
    )
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
    fn adjust_colors_identity() {
        let pb = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 10, 10).unwrap();
        let result = adjust_colors(pb, 0.0, 0.0);
        assert_eq!(result.width(), 10);
    }
}
