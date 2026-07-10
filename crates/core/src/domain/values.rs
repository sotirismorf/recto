use serde::{Deserialize, Serialize};

/// A valid rotation angle. Only 0, 90, 180, and 270 are representable.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rotation(u16);

impl Rotation {
    pub const ZERO: Self = Self(0);
    pub const DEG90: Self = Self(90);
    pub const DEG180: Self = Self(180);
    pub const DEG270: Self = Self(270);

    /// Normalize any angle to the nearest valid step.
    pub fn new(degrees: u16) -> Self {
        match degrees % 360 {
            0 => Self(0),
            90 => Self(90),
            180 => Self(180),
            _ => Self(270),
        }
    }

    pub fn as_degrees(self) -> u16 {
        self.0
    }
}

impl Default for Rotation {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Brightness in [-1.0, 1.0]. Clamped at construction.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Brightness(f32);

impl Brightness {
    pub const ZERO: Self = Self(0.0);
    pub const MIN: Self = Self(-1.0);
    pub const MAX: Self = Self(1.0);

    pub fn new(val: f32) -> Self {
        Self(val.clamp(-1.0, 1.0))
    }

    pub fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for Brightness {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Contrast in [-1.0, 1.0]. Clamped at construction.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Contrast(f32);

impl Contrast {
    pub const ZERO: Self = Self(0.0);
    pub const MIN: Self = Self(-1.0);
    pub const MAX: Self = Self(1.0);

    pub fn new(val: f32) -> Self {
        Self(val.clamp(-1.0, 1.0))
    }

    pub fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for Contrast {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Saturation in [-1.0, 1.0]. Clamped at construction.
/// -1.0 = fully desaturated (grayscale), 0.0 = unchanged, 1.0 = doubly saturated.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Saturation(f32);

impl Saturation {
    pub const ZERO: Self = Self(0.0);
    pub const MIN: Self = Self(-1.0);
    pub const MAX: Self = Self(1.0);

    pub fn new(val: f32) -> Self {
        Self(val.clamp(-1.0, 1.0))
    }

    pub fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for Saturation {
    fn default() -> Self {
        Self::ZERO
    }
}

/// JPEG/PDF quality in [1, 100].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JpegQuality(u8);

impl JpegQuality {
    pub const DEFAULT: Self = Self(90);

    pub fn new(val: u8) -> Self {
        Self(val.clamp(1, 100))
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }
}

impl Default for JpegQuality {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Export scale factor in (0.0, 1.0]. Clamped at construction.
/// Represents the fraction of pixel dimensions to keep.
/// 1.0 = original size, 0.5 = half width and height.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scale(f64);

impl Scale {
    pub const FULL: Self = Self(1.0);

    pub fn new(val: f64) -> Self {
        Self(if val <= 0.0 { 0.01 } else { val.min(1.0) })
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }

    pub fn as_factor(self) -> f64 {
        self.0
    }
}

impl Default for Scale {
    fn default() -> Self {
        Self::FULL
    }
}

/// Bleed margin in source-image pixels, added around each crop for PDF
/// export only. Hidden by the PDF page crop box. Clamped to [0, 2000].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bleed(u32);

impl Bleed {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(2000);

    pub fn new(val: u32) -> Self {
        Self(val.min(Self::MAX.0))
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl Default for Bleed {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Dots per inch for PDF rendering. Clamped to [1, 2400].
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dpi(f64);

impl Dpi {
    pub const DEFAULT: Self = Self(300.0);

    pub fn new(val: f64) -> Self {
        Self(val.clamp(1.0, 2400.0))
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }
}

impl Default for Dpi {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_normalizes() {
        assert_eq!(Rotation::new(0), Rotation::ZERO);
        assert_eq!(Rotation::new(90), Rotation::DEG90);
        assert_eq!(Rotation::new(180), Rotation::DEG180);
        assert_eq!(Rotation::new(270), Rotation::DEG270);
        assert_eq!(Rotation::new(360), Rotation::ZERO);
        assert_eq!(Rotation::new(45), Rotation::DEG270);
    }

    #[test]
    fn brightness_clamps_to_range() {
        assert_eq!(Brightness::new(2.0), Brightness::MAX);
        assert_eq!(Brightness::new(-5.0), Brightness::MIN);
        assert_eq!(Brightness::new(0.5), Brightness(0.5));
    }

    #[test]
    fn contrast_clamps_to_range() {
        assert_eq!(Contrast::new(5.0), Contrast(1.0));
        assert_eq!(Contrast::new(-3.0), Contrast(-1.0));
    }

    #[test]
    fn saturation_clamps_to_range() {
        assert_eq!(Saturation::new(2.0), Saturation::MAX);
        assert_eq!(Saturation::new(-5.0), Saturation::MIN);
        assert_eq!(Saturation::new(0.25), Saturation(0.25));
    }

    #[test]
    fn jpeg_quality_clamps() {
        assert_eq!(JpegQuality::new(0), JpegQuality(1));
        assert_eq!(JpegQuality::new(200), JpegQuality(100));
        assert_eq!(JpegQuality::new(85), JpegQuality(85));
    }

    #[test]
    fn bleed_clamps() {
        assert_eq!(Bleed::new(0), Bleed::ZERO);
        assert_eq!(Bleed::new(5000), Bleed::MAX);
        assert_eq!(Bleed::new(20), Bleed(20));
        assert!(Bleed::ZERO.is_zero());
        assert!(!Bleed::new(1).is_zero());
    }

    #[test]
    fn dpi_clamps() {
        assert_eq!(Dpi::new(0.0), Dpi(1.0));
        assert_eq!(Dpi::new(5000.0), Dpi(2400.0));
    }

    #[test]
    fn scale_clamps() {
        assert_eq!(Scale::new(0.0), Scale(0.01));
        assert_eq!(Scale::new(-0.5), Scale(0.01));
        assert_eq!(Scale::new(2.0), Scale(1.0));
        assert_eq!(Scale::new(0.5), Scale(0.5));
    }
}
