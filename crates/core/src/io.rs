use crate::error::Result;
use image::{DynamicImage, ImageFormat};
use std::io::Write;
use std::path::Path;

/// Load an image from `path` using the `image` crate's auto-detection.
pub fn load(path: &Path) -> Result<DynamicImage> {
    image::open(path).map_err(Into::into)
}

/// Save an image as PNG, writing atomically via a temporary file.
pub fn save_png(img: &DynamicImage, path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    img.save_with_format(tmp.path(), ImageFormat::Png)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Save an image as JPEG with the given quality (0–100), writing atomically.
pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let file = std::fs::File::create(tmp.path())?;
        let mut w = std::io::BufWriter::new(file);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality);
        img.write_with_encoder(encoder)?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Save an image as TIFF, writing atomically via a temporary file.
pub fn save_tiff(img: &DynamicImage, path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    img.save_with_format(tmp.path(), ImageFormat::Tiff)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Write `bytes` to `path` atomically — data is flushed and the temporary
/// file is renamed into place, so a partial write is never visible.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
