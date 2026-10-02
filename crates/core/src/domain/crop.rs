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

/// Move `crop` the least distance needed to lie inside a `w`×`h` image,
/// shrinking it first if it is larger than the image.
fn fit_inside(crop: CropBox, (w, h): (u32, u32)) -> CropBox {
    let cw = crop.w.clamp(1, w.max(1));
    let ch = crop.h.clamp(1, h.max(1));
    CropBox {
        x: crop.x.min(w.saturating_sub(cw)),
        y: crop.y.min(h.saturating_sub(ch)),
        w: cw,
        h: ch,
    }
}

/// A `w`×`h` crop centred on `around` (typically a detected page), scaled
/// down proportionally if larger than the image, then kept inside it.
pub fn centered_crop(w: u32, h: u32, around: CropBox, image: (u32, u32)) -> CropBox {
    let (iw, ih) = image;
    let (mut w, mut h) = (w.max(1), h.max(1));
    if iw > 0 && ih > 0 && (w > iw || h > ih) {
        let s = (iw as f64 / w as f64).min(ih as f64 / h as f64);
        w = ((w as f64 * s) as u32).max(1);
        h = ((h as f64 * s) as u32).max(1);
    }
    let cx = (around.x + around.w / 2) as i64;
    let cy = (around.y + around.h / 2) as i64;
    let x = (cx - (w / 2) as i64).max(0) as u32;
    let y = (cy - (h / 2) as i64).max(0) as u32;
    fit_inside(CropBox { x, y, w, h }, image)
}

/// Give `crop` the size `w`×`h`, keeping its top-left corner where possible
/// and staying inside the image. A page without a crop starts at the origin.
pub fn resize_crop(crop: Option<CropBox>, w: u32, h: u32, image: (u32, u32)) -> CropBox {
    let base = crop.unwrap_or(CropBox { x: 0, y: 0, w, h });
    fit_inside(CropBox { w, h, ..base }, image)
}

/// Give `crop` the size `w`×`h` around the same centre, staying inside the
/// image: growing a preset's margin grows every page's crop evenly.
pub fn resize_crop_centered(crop: CropBox, w: u32, h: u32, image: (u32, u32)) -> CropBox {
    let x = crop.x as i64 - (w as i64 - crop.w as i64) / 2;
    let y = crop.y as i64 - (h as i64 - crop.h as i64) / 2;
    fit_inside(
        CropBox {
            x: x.max(0) as u32,
            y: y.max(0) as u32,
            w,
            h,
        },
        image,
    )
}

/// Group pages by identical image size, largest group first (ties keep page
/// order), so each group can share one crop preset.
pub fn group_by_size(sizes: &[(usize, (u32, u32))]) -> Vec<((u32, u32), Vec<usize>)> {
    let mut groups: Vec<((u32, u32), Vec<usize>)> = Vec::new();
    for &(page, size) in sizes {
        match groups.iter_mut().find(|(s, _)| *s == size) {
            Some((_, pages)) => pages.push(page),
            None => groups.push((size, vec![page])),
        }
    }
    // Stable sort: equal-sized groups stay in order of their first page.
    groups.sort_by_key(|(_, pages)| std::cmp::Reverse(pages.len()));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cb(x: u32, y: u32, w: u32, h: u32) -> CropBox {
        CropBox { x, y, w, h }
    }

    #[test]
    fn centered_crop_centres_on_detection() {
        let c = centered_crop(100, 200, cb(400, 300, 120, 220), (1000, 1000));
        assert_eq!(c, cb(410, 310, 100, 200));
    }

    #[test]
    fn centered_crop_scales_down_and_stays_inside() {
        let c = centered_crop(400, 200, cb(0, 0, 50, 50), (200, 100));
        assert_eq!(c, cb(0, 0, 200, 100));
        let c = centered_crop(100, 100, cb(950, 950, 50, 50), (1000, 1000));
        assert_eq!(c, cb(900, 900, 100, 100));
    }

    #[test]
    fn resize_crop_keeps_corner_and_clamps_per_image() {
        assert_eq!(
            resize_crop(Some(cb(10, 20, 5, 5)), 50, 60, (100, 100)),
            cb(10, 20, 50, 60)
        );
        assert_eq!(
            resize_crop(Some(cb(80, 90, 5, 5)), 50, 60, (100, 100)),
            cb(50, 40, 50, 60)
        );
        assert_eq!(resize_crop(None, 50, 60, (100, 100)), cb(0, 0, 50, 60));
    }

    #[test]
    fn resize_crop_centered_grows_evenly() {
        assert_eq!(
            resize_crop_centered(cb(100, 100, 50, 50), 70, 70, (1000, 1000)),
            cb(90, 90, 70, 70)
        );
        assert_eq!(
            resize_crop_centered(cb(100, 100, 50, 50), 30, 30, (1000, 1000)),
            cb(110, 110, 30, 30)
        );
        assert_eq!(
            resize_crop_centered(cb(0, 0, 50, 50), 70, 70, (60, 1000)),
            cb(0, 0, 60, 70)
        );
    }

    #[test]
    fn group_by_size_is_deterministic() {
        let groups = group_by_size(&[(0, (4, 3)), (1, (3, 4)), (2, (4, 3)), (3, (9, 9))]);
        assert_eq!(
            groups,
            vec![((4, 3), vec![0, 2]), ((3, 4), vec![1]), ((9, 9), vec![3])]
        );
    }

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
