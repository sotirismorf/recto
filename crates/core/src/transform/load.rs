use image::DynamicImage;
use std::path::Path;

use crate::error::Result;

/// Load an image from disk (handles format auto-detection).
pub fn load_image(path: &Path) -> Result<DynamicImage> {
    crate::io::load(path)
}
