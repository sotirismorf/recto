use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::crop::{CropBox, CropPreset};
use crate::domain::values::{BlackLevel, Bleed, Exposure, Rotation, Saturation, Temperature, Tint};

pub const CURRENT_SCHEMA: u32 = 1;

fn schema_v1() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub path: PathBuf,
    #[serde(default)]
    pub rotation: Rotation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<CropBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_preset: Option<usize>,
}

impl Page {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            rotation: Rotation::ZERO,
            crop: None,
            crop_preset: None,
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
    pub output_dir: PathBuf,
    pub prefix: String,
    #[serde(default)]
    pub exposure: Exposure,
    #[serde(default)]
    pub black_level: BlackLevel,
    #[serde(default)]
    pub temperature: Temperature,
    #[serde(default)]
    pub tint: Tint,
    #[serde(default)]
    pub saturation: Saturation,
    #[serde(default)]
    pub bleed: Bleed,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA,
            pages: Vec::new(),
            crop_presets: Vec::new(),
            output_dir: PathBuf::new(),
            prefix: "page".into(),
            exposure: Exposure::ZERO,
            black_level: BlackLevel::ZERO,
            temperature: Temperature::ZERO,
            tint: Tint::ZERO,
            saturation: Saturation::ZERO,
            bleed: Bleed::ZERO,
        }
    }
}
