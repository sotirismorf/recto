use serde::{Deserialize, Serialize};

use crate::domain::values::JpegQuality;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSize {
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdfCompression {
    #[default]
    Jpeg,
    Flate,
    Ccit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ExportSettings {
    Png { #[serde(default)] compression: u8 },
    Jpeg { quality: JpegQuality },
    Tiff,
    #[serde(rename = "pdf")]
    Pdf {
        #[serde(default)]
        compression: PdfCompression,
        #[serde(default = "JpegQuality::default")]
        quality: JpegQuality,
    },
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self::Png { compression: 3 }
    }
}
