use serde::{Deserialize, Serialize};

use crate::domain::values::JpegQuality;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSize {
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ExportSettings {
    Png,
    Jpeg { quality: JpegQuality },
    Tiff,
    Pdf { quality: JpegQuality },
}
