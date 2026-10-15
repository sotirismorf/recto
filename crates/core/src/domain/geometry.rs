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
}
