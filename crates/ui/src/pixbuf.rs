//! Loading page images as pixbufs for display.
//!
//! Every on-screen image must be in the same frame the export pipeline crops
//! in, or crops drawn on screen land elsewhere in the output. So orientation
//! comes from recto-core ([`recto_core::io::read_orientation`]), never from
//! gdk-pixbuf's own EXIF handling, which differs by format.

use std::path::Path;

use gtk::gdk_pixbuf::{Colorspace, Pixbuf, PixbufRotation};
use gtk::{gdk, glib};
use recto_core::io::Orientation;

/// Decode `path` in its EXIF-corrected frame, then apply the page's user
/// `rotation` (degrees). With `max_px`, decode scaled to fit that box, which
/// is much cheaper for thumbnails.
pub fn load_page(path: &Path, rotation: u32, max_px: Option<i32>) -> Option<Pixbuf> {
    let pb = match max_px {
        Some(px) => Pixbuf::from_file_at_scale(path, px, px, true),
        None => Pixbuf::from_file(path),
    }
    .inspect_err(|e| tracing::warn!("cannot load {}: {e}", path.display()))
    .ok()?;
    let pb = orient(&pb, recto_core::io::read_orientation(path));
    Some(rotate(&pb, rotation))
}

/// Apply an EXIF orientation, matching [`Orientation::apply`] in core.
pub fn orient(pb: &Pixbuf, orientation: Orientation) -> Pixbuf {
    let turn = |p: &Pixbuf, r| p.rotate_simple(r).unwrap_or_else(|| p.clone());
    let flip_h = |p: &Pixbuf| p.flip(true).unwrap_or_else(|| p.clone());
    match orientation {
        Orientation::FlipH => flip_h(pb),
        Orientation::Rotate180 => turn(pb, PixbufRotation::Upsidedown),
        Orientation::FlipV => pb.flip(false).unwrap_or_else(|| pb.clone()),
        Orientation::Transpose => flip_h(&turn(pb, PixbufRotation::Clockwise)),
        Orientation::Rotate90 => turn(pb, PixbufRotation::Clockwise),
        Orientation::Transverse => flip_h(&turn(pb, PixbufRotation::Counterclockwise)),
        Orientation::Rotate270 => turn(pb, PixbufRotation::Counterclockwise),
        _ => pb.clone(),
    }
}

/// Rotate clockwise by a multiple of 90 degrees.
pub fn rotate(pb: &Pixbuf, deg: u32) -> Pixbuf {
    let rotation = match deg % 360 {
        90 => PixbufRotation::Clockwise,
        180 => PixbufRotation::Upsidedown,
        270 => PixbufRotation::Counterclockwise,
        _ => return pb.clone(),
    };
    pb.rotate_simple(rotation).unwrap_or_else(|| pb.clone())
}

/// Pixbuf pixels in a form that can cross threads (`Pixbuf` is not `Send`).
pub struct PixelData {
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    rowstride: i32,
    has_alpha: bool,
}

impl PixelData {
    pub fn from_pixbuf(pb: &Pixbuf) -> Self {
        Self {
            bytes: pb.read_pixel_bytes(),
            width: pb.width(),
            height: pb.height(),
            rowstride: pb.rowstride(),
            has_alpha: pb.has_alpha(),
        }
    }

    pub fn to_pixbuf(&self) -> Pixbuf {
        Pixbuf::from_bytes(
            &self.bytes,
            Colorspace::Rgb,
            self.has_alpha,
            8,
            self.width,
            self.height,
            self.rowstride,
        )
    }

    pub fn to_texture(&self) -> gdk::Texture {
        gdk::Texture::for_pixbuf(&self.to_pixbuf())
    }
}
