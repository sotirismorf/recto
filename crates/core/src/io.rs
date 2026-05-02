use crate::error::Result;
use image::{DynamicImage, ImageFormat};
use std::io::Write;
use std::path::Path;

pub fn load(path: &Path) -> Result<DynamicImage> {
    image::open(path).map_err(Into::into)
}

pub fn save_png(img: &DynamicImage, path: &Path) -> Result<()> {
    img.save_with_format(path, ImageFormat::Png)
        .map_err(Into::into)
}

pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(file);
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality);
    img.write_with_encoder(encoder).map_err(Into::into)
}

pub fn save_tiff(img: &DynamicImage, path: &Path) -> Result<()> {
    img.save_with_format(path, ImageFormat::Tiff)
        .map_err(Into::into)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
