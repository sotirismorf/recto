//! Physical page-size estimation from camera EXIF metadata.
//!
//! With a copy stand the scale of a capture is fixed by three numbers: the
//! lens focal length, the sensor pixel pitch and the distance from the
//! sensor to the page. The first two are standard EXIF tags; the distance is
//! only recorded in maker notes (Canon) or, rarely, in `SubjectDistance`.
//! Canon stores the distance as a coarse range, so estimates carry roughly
//! ±10 % error.

use std::path::Path;

use exif::{In, Tag, Value};

use crate::domain::project::Page;

/// Camera geometry for one capture, all lengths in millimetres.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CameraGeometry {
    pub focal_mm: f64,
    /// Size of one image pixel on the sensor.
    pub pixel_pitch_mm: f64,
    /// Distance from the sensor (focal plane) to the subject.
    pub focus_distance_mm: f64,
}

impl CameraGeometry {
    /// Physical size of an object spanning `px` image pixels, using the
    /// thin-lens equation with object + image distance = focus distance.
    pub fn pixels_to_mm(&self, px: f64) -> Option<f64> {
        let f = self.focal_mm;
        let d = self.focus_distance_mm;
        let disc = 1.0 - 4.0 * f / d;
        if !(f > 0.0 && d > 0.0 && disc > 0.0 && self.pixel_pitch_mm > 0.0) {
            return None;
        }
        // Object distance u (the larger root of u + v = d, 1/u + 1/v = 1/f).
        let u = d / 2.0 * (1.0 + disc.sqrt());
        let magnification = f / (u - f);
        Some(px * self.pixel_pitch_mm / magnification)
    }
}

/// Estimate the visible height of `page` in millimetres from its crop and
/// the EXIF data of its source image. `None` when the page is uncropped or
/// the metadata is insufficient.
pub fn estimate_page_height_mm(page: &Page) -> Option<f64> {
    let crop = page.crop?;
    let geom = read_camera_geometry(&page.path)?;
    geom.pixels_to_mm(crop.h as f64)
}

/// Read focal length, pixel pitch and focus distance from `path`.
pub fn read_camera_geometry(path: &Path) -> Option<CameraGeometry> {
    let file = std::fs::File::open(path).ok()?;
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::BufReader::new(file))
        .ok()?;

    let rational = |tag: Tag| -> Option<f64> {
        match &exif.get_field(tag, In::PRIMARY)?.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            _ => None,
        }
    };

    let focal_mm = rational(Tag::FocalLength)?;

    let unit_mm = match exif
        .get_field(Tag::FocalPlaneResolutionUnit, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(2)
    {
        3 => 10.0,
        4 => 1.0,
        _ => 25.4,
    };
    let xres = rational(Tag::FocalPlaneXResolution);
    let yres = rational(Tag::FocalPlaneYResolution);
    let res = match (xres, yres) {
        (Some(x), Some(y)) => (x + y) / 2.0,
        (Some(r), None) | (None, Some(r)) => r,
        _ => return None,
    };
    if res <= 0.0 {
        return None;
    }
    let mut pixel_pitch_mm = unit_mm / res;

    // FocalPlane*Resolution describes the original capture; correct the
    // pitch if the file was downscaled afterwards.
    let exif_w = exif
        .get_field(Tag::PixelXDimension, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0));
    let exif_h = exif
        .get_field(Tag::PixelYDimension, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0));
    if let (Some(ew), Some(eh), Ok((w, h))) = (exif_w, exif_h, image::image_dimensions(path)) {
        let (ew, eh) = (ew.max(eh) as f64, ew.min(eh) as f64);
        let (w, h) = (w.max(h) as f64, w.min(h) as f64);
        if w > 0.0 && h > 0.0 && (ew / w - eh / h).abs() < 0.01 {
            pixel_pitch_mm *= ew / w;
        }
    }

    let focus_distance_mm = canon_focus_distance_mm(&exif).or_else(|| {
        rational(Tag::SubjectDistance)
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(|m| m * 1000.0)
    })?;

    Some(CameraGeometry {
        focal_mm,
        pixel_pitch_mm,
        focus_distance_mm,
    })
}

/// Midpoint of Canon's FocusDistanceUpper/Lower in centimetres, from the
/// FileInfo (0x0093, indices 20/21) or ShotInfo (0x0004, indices 19/20)
/// maker-note arrays.
fn canon_focus_distance_mm(exif: &exif::Exif) -> Option<f64> {
    let make = exif.get_field(Tag::Make, In::PRIMARY)?;
    if !make.display_value().to_string().contains("Canon") {
        return None;
    }
    let Value::Undefined(_, offset) = &exif.get_field(Tag::MakerNote, In::PRIMARY)?.value else {
        return None;
    };
    let buf = exif.buf();
    let le = exif.little_endian();
    let u16_at = |o: usize| -> Option<u16> {
        let b = buf.get(o..o + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = buf.get(o..o + 4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };

    // Canon maker notes are a plain IFD whose value offsets are relative to
    // the TIFF header, i.e. to the start of `buf`.
    let ifd = *offset as usize;
    let count = u16_at(ifd)? as usize;
    let array_offset = |wanted: u16| -> Option<usize> {
        (0..count).find_map(|i| {
            let e = ifd + 2 + i * 12;
            let (tag, typ, n) = (u16_at(e)?, u16_at(e + 2)?, u32_at(e + 4)?);
            // int16u / int16s arrays, long enough to hold index 21.
            (tag == wanted && (typ == 3 || typ == 8) && n > 21)
                .then(|| u32_at(e + 8).map(|o| o as usize))
                .flatten()
        })
    };

    [(0x0093, 20), (0x0004, 19)]
        .into_iter()
        .find_map(|(tag, idx)| {
            let base = array_offset(tag)?;
            let upper = u16_at(base + idx * 2)?;
            let lower = u16_at(base + (idx + 1) * 2)?;
            let valid = |v: u16| v > 0 && v != 0xffff;
            let cm = match (valid(upper), valid(lower)) {
                (true, true) => (upper as f64 + lower as f64) / 2.0,
                (true, false) => upper as f64,
                (false, true) => lower as f64,
                _ => return None,
            };
            Some(cm * 10.0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_lens_matches_hand_calculation() {
        // Canon 700D at 38 mm, focused at ~0.805 m: 3200 px ≈ 266 mm.
        let g = CameraGeometry {
            focal_mm: 38.0,
            pixel_pitch_mm: 25.4 / 5793.8,
            focus_distance_mm: 805.0,
        };
        let mm = g.pixels_to_mm(3200.0).unwrap();
        assert!((mm - 266.0).abs() < 3.0, "{mm}");
    }

    #[test]
    fn focus_closer_than_4f_is_rejected() {
        let g = CameraGeometry {
            focal_mm: 50.0,
            pixel_pitch_mm: 0.004,
            focus_distance_mm: 150.0,
        };
        assert!(g.pixels_to_mm(1000.0).is_none());
    }
}
