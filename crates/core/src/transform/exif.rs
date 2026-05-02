use image::DynamicImage;
use std::path::Path;

/// Read EXIF orientation and apply rotation/flip. Non-EXIF sources are a no-op.
#[must_use]
pub fn correct_orientation(img: DynamicImage, path: &Path) -> DynamicImage {
    let orientation = crate::io::exif::read_orientation(path);
    orientation.apply(img)
}
