use image::DynamicImage;
use std::path::Path;

/// EXIF orientation tag values mapped to descriptive variants.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Orientation {
    #[default]
    Normal,
    FlipH,
    Rotate180,
    FlipV,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl From<u16> for Orientation {
    fn from(tag: u16) -> Self {
        match tag {
            2 => Self::FlipH,
            3 => Self::Rotate180,
            4 => Self::FlipV,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => Self::Normal,
        }
    }
}

impl Orientation {
    /// Apply the orientation correction to an image.
    #[must_use]
    pub fn apply(self, img: DynamicImage) -> DynamicImage {
        match self {
            Self::Normal => img,
            Self::FlipH => img.fliph(),
            Self::Rotate180 => img.rotate180(),
            Self::FlipV => img.flipv(),
            Self::Transpose => img.rotate90().fliph(),
            Self::Rotate90 => img.rotate90(),
            Self::Transverse => img.rotate270().fliph(),
            Self::Rotate270 => img.rotate270(),
        }
    }
}

/// Read the EXIF orientation tag from `path`, falling back to [`Orientation::Normal`].
///
/// File-open errors are logged at warn level; a missing EXIF orientation field
/// is logged at debug level (most images don't have one).
pub fn read_orientation(path: &Path) -> Orientation {
    fn try_read(path: &Path) -> Option<Orientation> {
        let file = std::fs::File::open(path)
            .inspect_err(|e| tracing::warn!("cannot open {path}: {e}", path = path.display()))
            .ok()?;
        let mut reader = std::io::BufReader::new(file);
        let exif = exif::Reader::new()
            .read_from_container(&mut reader)
            .inspect_err(|e| {
                tracing::warn!("cannot parse EXIF for {path}: {e}", path = path.display())
            })
            .ok()?;
        let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        let tag = field.value.get_uint(0)?;
        Some(Orientation::from(tag as u16))
    }

    try_read(path).unwrap_or_else(|| {
        tracing::debug!("no EXIF orientation found for {}", path.display());
        Orientation::Normal
    })
}
