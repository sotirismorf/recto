use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropPreset {
    pub name: String,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub locked: bool,
}

/// Actual per-side bleed applied after clamping to image bounds, in the
/// same pixel space as the expanded crop (pre-resize).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BleedInsets {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl BleedInsets {
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }

    /// Per-side insets as fractions (left, top, right, bottom) of the
    /// expanded dimensions. Fractions stay valid after any per-axis resize.
    pub fn fractions(&self, expanded_w: u32, expanded_h: u32) -> (f64, f64, f64, f64) {
        let ew = expanded_w.max(1) as f64;
        let eh = expanded_h.max(1) as f64;
        (
            self.left as f64 / ew,
            self.top as f64 / eh,
            self.right as f64 / ew,
            self.bottom as f64 / eh,
        )
    }
}

/// Expand `crop` outward by `bleed` px on all sides, clamped to an
/// `iw` x `ih` image. The crop is first clamped to the image the same way
/// `transform::crop::apply` does, so both agree on geometry. Returns the
/// expanded box and the actual per-side expansion after clamping.
pub fn expand_crop(crop: CropBox, bleed: u32, iw: u32, ih: u32) -> (CropBox, BleedInsets) {
    let x = crop.x.min(iw.saturating_sub(1));
    let y = crop.y.min(ih.saturating_sub(1));
    let w = crop.w.min(iw - x);
    let h = crop.h.min(ih - y);

    let insets = BleedInsets {
        left: bleed.min(x),
        top: bleed.min(y),
        right: bleed.min(iw - (x + w)),
        bottom: bleed.min(ih - (y + h)),
    };
    let expanded = CropBox {
        x: x - insets.left,
        y: y - insets.top,
        w: w + insets.left + insets.right,
        h: h + insets.top + insets.bottom,
    };
    (expanded, insets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_crop_centered_gets_full_insets() {
        let crop = CropBox {
            x: 50,
            y: 60,
            w: 100,
            h: 80,
        };
        let (e, i) = expand_crop(crop, 20, 300, 300);
        assert_eq!(
            i,
            BleedInsets {
                left: 20,
                top: 20,
                right: 20,
                bottom: 20
            }
        );
        assert_eq!(
            e,
            CropBox {
                x: 30,
                y: 40,
                w: 140,
                h: 120
            }
        );
    }

    #[test]
    fn expand_crop_at_origin_has_zero_left_top() {
        let crop = CropBox {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        let (e, i) = expand_crop(crop, 20, 300, 300);
        assert_eq!(
            i,
            BleedInsets {
                left: 0,
                top: 0,
                right: 20,
                bottom: 20
            }
        );
        assert_eq!(
            e,
            CropBox {
                x: 0,
                y: 0,
                w: 120,
                h: 120
            }
        );
    }

    #[test]
    fn expand_crop_clamps_when_bleed_exceeds_margins() {
        let crop = CropBox {
            x: 5,
            y: 5,
            w: 90,
            h: 90,
        };
        let (e, i) = expand_crop(crop, 50, 100, 100);
        assert_eq!(
            i,
            BleedInsets {
                left: 5,
                top: 5,
                right: 5,
                bottom: 5
            }
        );
        assert_eq!(
            e,
            CropBox {
                x: 0,
                y: 0,
                w: 100,
                h: 100
            }
        );
    }

    #[test]
    fn expand_crop_zero_bleed_is_identity() {
        let crop = CropBox {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
        };
        let (e, i) = expand_crop(crop, 0, 100, 100);
        assert!(i.is_zero());
        assert_eq!(e, crop);
    }

    #[test]
    fn expand_crop_clamps_oversized_input_crop() {
        let crop = CropBox {
            x: 90,
            y: 90,
            w: 50,
            h: 50,
        };
        let (e, i) = expand_crop(crop, 5, 100, 100);
        // Crop clamps to (90, 90, 10, 10) first, then expands inward only.
        assert_eq!(
            i,
            BleedInsets {
                left: 5,
                top: 5,
                right: 0,
                bottom: 0
            }
        );
        assert_eq!(
            e,
            CropBox {
                x: 85,
                y: 85,
                w: 15,
                h: 15
            }
        );
    }

    #[test]
    fn fractions_divide_by_expanded_dims() {
        let i = BleedInsets {
            left: 10,
            top: 20,
            right: 30,
            bottom: 40,
        };
        let (l, t, r, b) = i.fractions(200, 400);
        assert_eq!(l, 0.05);
        assert_eq!(t, 0.05);
        assert_eq!(r, 0.15);
        assert_eq!(b, 0.10);
    }
}
