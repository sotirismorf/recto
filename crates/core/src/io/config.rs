use crate::domain::export::ExportSettings;
use crate::domain::values::{Dpi, Scale};
use crate::error::{Error, Result};
use crate::io;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Metadata embedded into exported PDF files.
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
    pub dpi: Dpi,
}

fn default_creator() -> String {
    "Recto".into()
}

fn default_dpi() -> Dpi {
    Dpi::DEFAULT
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

/// Global application configuration persisted in the user's config directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub export: ExportSettings,
    #[serde(default)]
    pub export_scale: Scale,
    #[serde(default)]
    pub pdf_meta: PdfMeta,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            export: ExportSettings::default(),
            export_scale: Scale::default(),
            pdf_meta: PdfMeta::default(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("recto").join("config.json"))
}

/// Load the global application configuration.
pub fn load_config() -> AppConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            // Fallback for transition from old pdf_meta.json
            let meta_path = dirs::config_dir().map(|d| d.join("recto").join("pdf_meta.json"));
            let meta = meta_path
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            AppConfig {
                pdf_meta: meta,
                ..Default::default()
            }
        })
}

/// Save the global application configuration atomically.
pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path().ok_or(Error::NoConfigDir)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    io::write_atomic(&path, json.as_bytes())
}
