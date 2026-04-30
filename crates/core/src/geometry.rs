use std::collections::HashSet;

use crate::project::CropBox;

/// A rectangle in image-space (pixels, but stored as `f64` so we can express
/// fractional positions during interactive drags). Convert to/from the
/// integer `CropBox` only at persistence boundaries.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_box(b: CropBox) -> Self {
        Self::new(b.x as f64, b.y as f64, b.w as f64, b.h as f64)
    }

    pub fn to_box(self) -> CropBox {
        CropBox {
            x: self.x.round().max(0.0) as u32,
            y: self.y.round().max(0.0) as u32,
            w: self.w.round().max(1.0) as u32,
            h: self.h.round().max(1.0) as u32,
        }
    }

    /// Clamp position and size so the rect fits inside `0..iw, 0..ih`.
    /// `min` enforces a minimum width/height (use 1.0 for "any").
    pub fn clamp_to(self, iw: f64, ih: f64, min: f64) -> Self {
        let min = min.max(1.0);
        let w = self.w.min(iw).max(min);
        let h = self.h.min(ih).max(min);
        let x = self.x.clamp(0.0, (iw - w).max(0.0));
        let y = self.y.clamp(0.0, (ih - h).max(0.0));
        Self::new(x, y, w, h)
    }

    pub fn contains(self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }

    pub fn corners(self) -> [(f64, f64); 4] {
        [
            (self.x, self.y),
            (self.x + self.w, self.y),
            (self.x, self.y + self.h),
            (self.x + self.w, self.y + self.h),
        ]
    }

    pub fn edge_midpoints(self) -> [(f64, f64); 4] {
        [
            (self.x + self.w * 0.5, self.y),
            (self.x + self.w * 0.5, self.y + self.h),
            (self.x, self.y + self.h * 0.5),
            (self.x + self.w, self.y + self.h * 0.5),
        ]
    }

    /// Build a normalised rect from two arbitrary points, clamped to bounds.
    /// Returns `None` if the resulting rect would be smaller than `min` on
    /// either axis.
    pub fn from_points(a: (f64, f64), b: (f64, f64), iw: f64, ih: f64, min: f64) -> Option<Self> {
        let x0 = a.0.min(b.0).clamp(0.0, iw);
        let y0 = a.1.min(b.1).clamp(0.0, ih);
        let x1 = a.0.max(b.0).clamp(0.0, iw);
        let y1 = a.1.max(b.1).clamp(0.0, ih);
        let w = x1 - x0;
        let h = y1 - y0;
        if w < min || h < min {
            None
        } else {
            Some(Self::new(x0, y0, w, h))
        }
    }
}


pub fn output_dimensions(natural_w: f64, natural_h: f64, target_ar: f64) -> (u32, u32) {
    if natural_w <= 0.0 || natural_h <= 0.0 || target_ar <= 0.0 {
        return (natural_w.max(1.0) as u32, natural_h.max(1.0) as u32);
    }
    let natural_ar = natural_w / natural_h;
    let (w, h) = if natural_ar >= target_ar {
        (natural_h * target_ar, natural_h)
    } else {
        (natural_w, natural_w / target_ar)
    };
    (w.round().max(1.0) as u32, h.round().max(1.0) as u32)
}

pub fn median_aspect_ratio<I: IntoIterator<Item = (f64, f64)>>(dims: I) -> f64 {
    let mut ratios: Vec<f64> = dims
        .into_iter()
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .map(|(w, h)| w / h)
        .collect();
    if ratios.is_empty() {
        return 1.0;
    }
    ratios.sort_by(|a, b| {
        a.partial_cmp(b)
            .expect("ratios are finite non-NaN")
    });
    let n = ratios.len();
    if n % 2 == 1 {
        ratios[n / 2]
    } else {
        (ratios[n / 2 - 1] + ratios[n / 2]) / 2.0
    }
}

pub fn outlier_indices(dims: &[(f64, f64)], threshold: f64) -> HashSet<usize> {
    let valid: Vec<f64> = dims
        .iter()
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .map(|(w, h)| w / h)
        .collect();
    if valid.len() < 3 {
        return HashSet::new();
    }
    let mut sorted = valid.clone();
    sorted.sort_by(|a, b| {
        a.partial_cmp(b)
            .expect("ratios are finite non-NaN")
    });
    let n = sorted.len();
    let median = if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 };
    if median == 0.0 {
        return HashSet::new();
    }

    dims.iter()
        .enumerate()
        .filter_map(|(i, (w, h))| {
            if *w <= 0.0 || *h <= 0.0 {
                return None;
            }
            let r = w / h;
            if (r - median).abs() / median > threshold {
                Some(i)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_inside_bounds() {
        let r = Rect::new(-5.0, -5.0, 50.0, 50.0).clamp_to(100.0, 100.0, 1.0);
        assert_eq!(r, Rect::new(0.0, 0.0, 50.0, 50.0));

        let r = Rect::new(80.0, 80.0, 50.0, 50.0).clamp_to(100.0, 100.0, 1.0);
        assert_eq!(r, Rect::new(50.0, 50.0, 50.0, 50.0));
    }

    #[test]
    fn clamp_shrinks_oversize() {
        let r = Rect::new(0.0, 0.0, 200.0, 200.0).clamp_to(100.0, 100.0, 1.0);
        assert_eq!(r, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn clamp_enforces_minimum() {
        let r = Rect::new(50.0, 50.0, 0.5, 0.5).clamp_to(100.0, 100.0, 16.0);
        assert!(r.w >= 16.0 && r.h >= 16.0);
    }

    #[test]
    fn from_points_rejects_too_small() {
        assert!(Rect::from_points((10.0, 10.0), (10.5, 10.5), 100.0, 100.0, 16.0).is_none());
    }

    #[test]
    fn cropbox_round_trip() {
        let b = CropBox { x: 12, y: 34, w: 100, h: 80 };
        assert_eq!(Rect::from_box(b).to_box().x, b.x);
        assert_eq!(Rect::from_box(b).to_box().w, b.w);
    }

    #[test]
    fn contains_inclusive() {
        let r = Rect::new(10.0, 10.0, 100.0, 100.0);
        assert!(r.contains((10.0, 10.0)));
        assert!(r.contains((110.0, 110.0)));
        assert!(!r.contains((9.9, 50.0)));
    }
}
