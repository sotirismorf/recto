use image::DynamicImage;
use std::path::Path;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
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

pub fn read_orientation(path: &Path) -> Orientation {
    let result: Option<Orientation> = std::fs::File::open(path)
        .ok()
        .and_then(|file| {
            let mut reader = std::io::BufReader::new(file);
            exif::Reader::new()
                .read_from_container(&mut reader)
                .ok()
        })
        .and_then(|exif| {
            let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
            field.value.get_uint(0)
        })
        .map(|tag| Orientation::from(tag as u16));

    match result {
        Some(o) => o,
        None => {
            tracing::debug!("no EXIF orientation found for {}", path.display());
            Orientation::Normal
        }
    }
}
