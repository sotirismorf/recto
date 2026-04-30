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

impl Orientation {
    pub fn from_tag(tag: u32) -> Self {
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
    let Ok(file) = std::fs::File::open(path) else { return Orientation::Normal };
    let mut reader = std::io::BufReader::new(file);
    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(e) => e,
        Err(_) => return Orientation::Normal,
    };
    let field = match exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY) {
        Some(f) => f,
        None => return Orientation::Normal,
    };
    let tag = field.value.get_uint(0).unwrap_or(1);
    Orientation::from_tag(tag)
}
