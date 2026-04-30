use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};
use std::path::Path;

pub fn load(path: &Path) -> Result<DynamicImage> {
    image::open(path).with_context(|| format!("decode {}", path.display()))
}

pub fn save_png(img: &DynamicImage, path: &Path) -> Result<()> {
    img.save_with_format(path, ImageFormat::Png)
        .with_context(|| format!("write png {}", path.display()))
}

pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("create {}", path.display()))?;
    let mut w = std::io::BufWriter::new(file);
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality);
    img.write_with_encoder(encoder)
        .with_context(|| format!("write jpeg {}", path.display()))
}

pub fn save_tiff(img: &DynamicImage, path: &Path) -> Result<()> {
    img.save_with_format(path, ImageFormat::Tiff)
        .with_context(|| format!("write tiff {}", path.display()))
}
