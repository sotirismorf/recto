use serde::{Deserialize, Serialize};

use crate::domain::values::JpegQuality;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSize {
    pub w: u32,
    pub h: u32,
}

/// How page images are encoded inside an exported PDF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdfCompression {
    /// Lossy JPEG (DCTDecode) at [`PdfOptions::quality`]; small files.
    #[default]
    Jpeg,
    /// Lossless zlib (FlateDecode); exact pixels, larger files. Configs from
    /// before CCITT was removed load as Flate, the other non-JPEG choice.
    #[serde(alias = "ccit")]
    Flate,
}

/// PDF export options.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PdfOptions {
    #[serde(default)]
    pub compression: PdfCompression,
    /// JPEG quality; only used with [`PdfCompression::Jpeg`].
    #[serde(default)]
    pub quality: JpegQuality,
    /// Height of every page in millimetres; 0 keeps the size implied by DPI.
    #[serde(default)]
    pub page_height_mm: u32,
    /// Derive the page height from the first page's camera EXIF data
    /// instead of `page_height_mm` (which remains the fallback).
    #[serde(default)]
    pub estimate_page_height: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ExportSettings {
    Png {
        #[serde(default)]
        compression: u8,
    },
    Jpeg {
        quality: JpegQuality,
    },
    Tiff,
    Pdf(PdfOptions),
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self::Png { compression: 3 }
    }
}

impl ExportSettings {
    /// The per-page image format, or `None` for PDF.
    pub fn image_format(&self) -> Option<ImageFormat> {
        match *self {
            Self::Png { compression } => Some(ImageFormat::Png { compression }),
            Self::Jpeg { quality } => Some(ImageFormat::Jpeg { quality }),
            Self::Tiff => Some(ImageFormat::Tiff),
            Self::Pdf(_) => None,
        }
    }
}

/// Exporting every page as its own image file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageFormat {
    Png { compression: u8 },
    Jpeg { quality: JpegQuality },
    Tiff,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png { .. } => "png",
            Self::Jpeg { .. } => "jpg",
            Self::Tiff => "tiff",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_settings_keep_their_flat_json_shape() {
        let json = r#"{"format":"pdf","compression":"jpeg","quality":80,"page_height_mm":210,"estimate_page_height":true}"#;
        let settings: ExportSettings = serde_json::from_str(json).unwrap();
        assert_eq!(
            settings,
            ExportSettings::Pdf(PdfOptions {
                compression: PdfCompression::Jpeg,
                quality: JpegQuality::new(80),
                page_height_mm: 210,
                estimate_page_height: true,
            })
        );
        let back: ExportSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn removed_ccitt_choice_loads_as_flate() {
        let json = r#"{"format":"pdf","compression":"ccit"}"#;
        let settings: ExportSettings = serde_json::from_str(json).unwrap();
        assert!(matches!(
            settings,
            ExportSettings::Pdf(PdfOptions {
                compression: PdfCompression::Flate,
                ..
            })
        ));
    }
}
