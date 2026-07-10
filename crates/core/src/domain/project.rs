use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::crop::{CropBox, CropPreset};
use crate::domain::export::{ExportSettings, OutputSize};
use crate::domain::values::{Bleed, Brightness, Contrast, Rotation, Saturation, Scale};

pub const CURRENT_SCHEMA: u32 = 1;

fn schema_v1() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub path: PathBuf,
    #[serde(default)]
    pub rotation: Rotation,
    #[serde(default)]
    pub crop: Option<CropBox>,
    #[serde(default)]
    pub crop_preset: Option<usize>,
    #[serde(default)]
    pub output: Option<OutputSize>,
}

impl Page {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            rotation: Rotation::ZERO,
            crop: None,
            crop_preset: None,
            output: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "schema_v1")]
    pub version: u32,
    pub pages: Vec<Page>,
    #[serde(default)]
    pub crop_presets: Vec<CropPreset>,
    pub export: ExportSettings,
    pub output_dir: PathBuf,
    pub prefix: String,
    #[serde(default)]
    pub brightness: Brightness,
    #[serde(default)]
    pub contrast: Contrast,
    #[serde(default)]
    pub saturation: Saturation,
    #[serde(default)]
    pub export_scale: Scale,
    #[serde(default)]
    pub bleed: Bleed,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA,
            pages: Vec::new(),
            crop_presets: Vec::new(),
            export: ExportSettings::default(),
            output_dir: PathBuf::new(),
            prefix: "page".into(),
            brightness: Brightness::ZERO,
            contrast: Contrast::ZERO,
            saturation: Saturation::ZERO,
            export_scale: Scale::FULL,
            bleed: Bleed::ZERO,
        }
    }
}
