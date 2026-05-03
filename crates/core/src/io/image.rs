use crate::domain::values::JpegQuality;
use crate::error::Result;
use image::{DynamicImage, ImageFormat};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use std::path::Path;

/// Load an image from `path` using the `image` crate's auto-detection.
pub fn load(path: &Path) -> Result<DynamicImage> {
    image::open(path).map_err(Into::into)
}

/// Save an image as PNG with the given compression level (0-9), writing atomically.
pub fn save_png(img: &DynamicImage, path: &Path, compression: u8) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let file = std::fs::File::create(tmp.path())?;
        let mut w = std::io::BufWriter::new(file);
        let encoder = PngEncoder::new_with_quality(
            &mut w,
            CompressionType::Level(compression.clamp(0, 9)),
            FilterType::Adaptive,
        );
        img.write_with_encoder(encoder)?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Save an image as JPEG with the given quality (0–100), writing atomically.
pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: JpegQuality) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let file = std::fs::File::create(tmp.path())?;
        let mut w = std::io::BufWriter::new(file);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality.as_u8());
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
