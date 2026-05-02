use image::DynamicImage;

use crate::domain::values::Rotation;

/// Apply a user-requested rotation. Zero degrees is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, rotation: Rotation) -> DynamicImage {
    match rotation {
        Rotation::ZERO => img,
        Rotation::DEG90 => img.rotate90(),
        Rotation::DEG180 => img.rotate180(),
        Rotation::DEG270 => img.rotate270(),
        _ => img,
    }
}
