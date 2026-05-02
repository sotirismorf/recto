use crate::error::{Error, Result};
use crate::io;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Metadata embedded into exported PDF files.
///
/// Persisted globally in the user's config directory, not per-project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PdfMeta {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default = "default_creator")]
    pub creator: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub keywords: String,
    #[serde(default = "default_dpi")]
    pub dpi: f64,
}

fn default_creator() -> String {
    "pagecutter".into()
}

fn default_dpi() -> f64 {
    300.0
}

impl Default for PdfMeta {
    fn default() -> Self {
        Self {
            title: String::new(),
            author: String::new(),
            creator: default_creator(),
            subject: String::new(),
            keywords: String::new(),
            dpi: default_dpi(),
        }
    }
}

fn pdf_meta_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("pagecutter").join("pdf_meta.json"))
}

/// Load persisted PDF metadata, falling back to [`PdfMeta::default`] if the file
/// doesn't exist or is malformed.
pub fn load_pdf_meta() -> PdfMeta {
    pdf_meta_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist PDF metadata atomically to the user's config directory.
pub fn save_pdf_meta(meta: &PdfMeta) -> Result<()> {
    let path = pdf_meta_path().ok_or(Error::NoConfigDir)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(meta)?;
    io::write_atomic(&path, json.as_bytes())
}
