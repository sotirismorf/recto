use std::collections::HashSet;

use crate::domain::crop::CropBox;

/// A rectangle in image-space coordinates.
///
/// Coordinates and dimensions are stored as `f64` to allow fractional positions
/// during interactive operations.  Convert to/from the integer [`CropBox`] only
/// at persistence boundaries.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    #[must_use]
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// Clamp position and size so the rectangle fits inside `0..iw, 0..ih`.
    ///
    /// `min` enforces a minimum width/height (use `1.0` for "any").
    /// Returns a new clamped rectangle — the receiver is unchanged.
    #[must_use]
    pub fn clamp_to(&self, iw: f64, ih: f64, min: f64) -> Self {
        let min = min.max(1.0);
        let w = self.w.min(iw).max(min);
        let h = self.h.min(ih).max(min);
        let x = self.x.clamp(0.0, (iw - w).max(0.0));
        let y = self.y.clamp(0.0, (ih - h).max(0.0));
        Self::new(x, y, w, h)
    }

    /// Whether `p` lies inside the rectangle (inclusive of right and bottom edges).
    #[must_use]
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }

    /// The four corner points in order: top-left, top-right, bottom-left, bottom-right.
    #[must_use]
    pub fn corners(&self) -> [(f64, f64); 4] {
        [
            (self.x, self.y),
            (self.x + self.w, self.y),
            (self.x, self.y + self.h),
            (self.x + self.w, self.y + self.h),
        ]
    }

    /// Midpoints of the four edges in order: top, bottom, left, right.
    #[must_use]
    pub fn edge_midpoints(&self) -> [(f64, f64); 4] {
        [
            (self.x + self.w * 0.5, self.y),
            (self.x + self.w * 0.5, self.y + self.h),
            (self.x, self.y + self.h * 0.5),
            (self.x + self.w, self.y + self.h * 0.5),
        ]
    }

    /// Build a rectangle from two arbitrary corner points, clamped to bounds
    /// `0..iw` and `0..ih`.
    ///
    /// Returns `None` if the resulting rectangle would be smaller than `min`
    /// on either axis.
    #[must_use]
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

impl From<CropBox> for Rect {
    fn from(b: CropBox) -> Self {
        Self::new(b.x as f64, b.y as f64, b.w as f64, b.h as f64)
    }
}

impl From<Rect> for CropBox {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x.round().max(0.0) as u32,
            y: r.y.round().max(0.0) as u32,
            w: r.w.round().clamp(1.0, u32::MAX as f64) as u32,
            h: r.h.round().clamp(1.0, u32::MAX as f64) as u32,
        }
    }
}

/// Calculate output dimensions that fit inside `(natural_w, natural_h)` while
/// matching `target_ar`.
///
/// If either dimension or the target ratio is non-positive, the original
/// dimensions are returned unchanged.
#[must_use]
pub fn output_dimensions(natural_w: f64, natural_h: f64, target_ar: f64) -> (u32, u32) {
    if natural_w <= 0.0 || natural_h <= 0.0 || target_ar <= 0.0 {
        return (
            natural_w.max(1.0).round().clamp(1.0, u32::MAX as f64) as u32,
            natural_h.max(1.0).round().clamp(1.0, u32::MAX as f64) as u32,
        );
    }
    let natural_ar = natural_w / natural_h;
    let (w, h) = if natural_ar >= target_ar {
        (natural_h * target_ar, natural_h)
    } else {
        (natural_w, natural_w / target_ar)
    };
    (
        w.round().clamp(1.0, u32::MAX as f64) as u32,
        h.round().clamp(1.0, u32::MAX as f64) as u32,
    )
}

fn median_sorted(xs: &[f64]) -> f64 {
    debug_assert!(!xs.is_empty());
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

/// Compute the median aspect ratio from an iterator of `(width, height)` pairs.
///
/// Non-finite and non-positive dimensions are filtered out.  Returns `1.0` if
/// no valid dimensions are provided.
#[must_use]
pub fn median_aspect_ratio<I: IntoIterator<Item = (f64, f64)>>(dims: I) -> f64 {
    let mut ratios: Vec<f64> = dims
        .into_iter()
        .filter(|(w, h)| w.is_finite() && *w > 0.0 && h.is_finite() && *h > 0.0)
        .map(|(w, h)| w / h)
        .collect();
    if ratios.is_empty() {
        return 1.0;
    }
    ratios.sort_by(f64::total_cmp);
    median_sorted(&ratios)
}

/// Find indices of outlier entries whose aspect ratio deviates from the median
/// by more than `threshold` (a fraction, e.g. `0.1` = 10%).
///
/// Returns an empty set when there are fewer than three valid entries.
#[must_use]
pub fn outlier_indices(dims: &[(f64, f64)], threshold: f64) -> HashSet<usize> {
    let mut valid: Vec<f64> = dims
        .iter()
        .filter(|(w, h)| w.is_finite() && *w > 0.0 && h.is_finite() && *h > 0.0)
        .map(|(w, h)| w / h)
        .collect();
    if valid.len() < 3 {
        return HashSet::new();
    }
    valid.sort_by(f64::total_cmp);
    let median = median_sorted(&valid);
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
        let b = CropBox {
            x: 12,
            y: 34,
            w: 100,
            h: 80,
        };
        let r: Rect = b.into();
        let cb: CropBox = r.into();
        assert_eq!(cb.x, b.x);
        assert_eq!(cb.w, b.w);
    }

    #[test]
    fn contains_inclusive() {
        let r = Rect::new(10.0, 10.0, 100.0, 100.0);
        assert!(r.contains((10.0, 10.0)));
        assert!(r.contains((110.0, 110.0)));
        assert!(!r.contains((9.9, 50.0)));
    }

    #[test]
    fn median_aspect_ratio_ignores_nan() {
        let dims = vec![(f64::NAN, 100.0), (200.0, 100.0), (300.0, 100.0)];
        let m = median_aspect_ratio(dims);
        assert!((m - 2.5).abs() < 0.01);
    }

    #[test]
    fn median_aspect_ratio_single() {
        let m = median_aspect_ratio(vec![(200.0, 100.0)]);
        assert!((m - 2.0).abs() < 0.01);
    }

    #[test]
    fn median_aspect_ratio_empty() {
        assert!((median_aspect_ratio(vec![]) - 1.0).abs() < 0.01);
    }

    #[test]
    fn from_points_identical_yields_zero_size() {
        assert!(Rect::from_points((10.0, 10.0), (10.0, 10.0), 100.0, 100.0, 1.0).is_none());
    }

    #[test]
    fn from_points_clamps_to_bounds() {
        let r = Rect::from_points((-5.0, -5.0), (150.0, 150.0), 100.0, 100.0, 1.0).unwrap();
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 0.0);
        assert_eq!(r.w, 100.0);
        assert_eq!(r.h, 100.0);
    }

    #[test]
    fn output_dimensions_zero_natural() {
        let (w, h) = output_dimensions(0.0, 0.0, 1.5);
        assert_eq!(w, 1);
        assert_eq!(h, 1);
    }

    #[test]
    fn output_dimensions_negative_target() {
        let (w, h) = output_dimensions(100.0, 50.0, -1.0);
        assert_eq!((w, h), (100, 50));
    }

    #[test]
    fn output_dimensions_wider_than_target() {
        let (w, h) = output_dimensions(200.0, 100.0, 1.0);
        assert_eq!(w, 100);
        assert_eq!(h, 100);
    }

    #[test]
    fn output_dimensions_taller_than_target() {
        let (w, h) = output_dimensions(100.0, 200.0, 1.0);
        assert_eq!(w, 100);
        assert_eq!(h, 100);
    }
}
