use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct CropBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CropPreset {
    pub name: String,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub locked: bool,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct OutputSize {
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub path: PathBuf,
    #[serde(default)]
    pub rotation: u16,
    #[serde(default)]
    pub crop: Option<CropBox>,
    #[serde(default)]
    pub crop_preset: Option<usize>,
    #[serde(default)]
    pub output: Option<OutputSize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub pages: Vec<Page>,
    #[serde(default)]
    pub crop_presets: Vec<CropPreset>,
    pub export: ExportSettings,
    pub output_dir: PathBuf,
    pub prefix: String,
    #[serde(default)]
    pub brightness: f32,
    #[serde(default)]
    pub contrast: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ExportSettings {
    Png,
    Jpeg { quality: u8 },
    Tiff,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            crop_presets: Vec::new(),
            export: ExportSettings::Png,
            output_dir: PathBuf::new(),
            prefix: "page".into(),
            brightness: 0.0,
            contrast: 0.0,
        }
    }
}
