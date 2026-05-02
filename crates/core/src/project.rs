use crate::error::{Error, Result};
use crate::io;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CURRENT_SCHEMA: u32 = 1;

fn schema_v1() -> u32 {
    1
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropPreset {
    pub name: String,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub locked: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default = "schema_v1")]
    pub version: u32,
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
    Pdf { quality: u8 },
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA,
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

// --- PDF metadata -----------------------------------------------------------

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

pub fn load_pdf_meta() -> PdfMeta {
    pdf_meta_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_pdf_meta(meta: &PdfMeta) -> Result<()> {
    let path = pdf_meta_path().ok_or(Error::NoConfigDir)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(meta)?;
    io::write_atomic(&path, json.as_bytes())
}

// --- Project save / load ----------------------------------------------------

#[derive(Serialize)]
struct ProjectView<'a> {
    version: u32,
    pages: Vec<PageView<'a>>,
    crop_presets: &'a [CropPreset],
    export: &'a ExportSettings,
    output_dir: &'a Path,
    prefix: &'a str,
    #[serde(default)]
    brightness: f32,
    #[serde(default)]
    contrast: f32,
}

#[derive(Serialize)]
struct PageView<'a> {
    path: &'a Path,
    #[serde(default)]
    rotation: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop: Option<CropBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop_preset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<OutputSize>,
}

pub fn save_project(project: &Project, file_path: &Path) -> Result<()> {
    let base = file_path.parent().unwrap_or(Path::new("."));
    let pages: Vec<PageView> = project
        .pages
        .iter()
        .map(|p| PageView {
            path: p.path.strip_prefix(base).unwrap_or(&p.path),
            rotation: p.rotation,
            crop: p.crop,
            crop_preset: p.crop_preset,
            output: p.output,
        })
        .collect();
    let view = ProjectView {
        version: CURRENT_SCHEMA,
        pages,
        crop_presets: &project.crop_presets,
        export: &project.export,
        output_dir: &project.output_dir,
        prefix: &project.prefix,
        brightness: project.brightness,
        contrast: project.contrast,
    };
    let json = serde_json::to_string_pretty(&view)?;
    io::write_atomic(file_path, json.as_bytes())
}

pub fn load_project(file_path: &Path) -> Result<Project> {
    let base = file_path.parent().unwrap_or(Path::new("."));
    let json = std::fs::read_to_string(file_path)?;
    let mut project: Project = serde_json::from_str(&json)?;
    if project.version > CURRENT_SCHEMA {
        return Err(Error::UnsupportedProjectVersion {
            found: project.version,
            supported: CURRENT_SCHEMA,
        });
    }
    for page in &mut project.pages {
        if page.path.is_relative() {
            page.path = base.join(&page.path);
        }
    }
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_round_trip_save_load() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("img/page.png");
        std::fs::create_dir_all(img_path.parent().unwrap()).unwrap();
        std::fs::write(&img_path, b"fake").unwrap();

        let project = Project {
            version: CURRENT_SCHEMA,
            pages: vec![Page {
                path: img_path.clone(),
                rotation: 90,
                crop: Some(CropBox {
                    x: 10,
                    y: 20,
                    w: 100,
                    h: 200,
                }),
                crop_preset: Some(0),
                output: Some(OutputSize { w: 640, h: 480 }),
            }],
            crop_presets: vec![CropPreset {
                name: "test".into(),
                w: 100,
                h: 200,
                locked: false,
            }],
            export: ExportSettings::Jpeg { quality: 85 },
            output_dir: tmp.path().join("out"),
            prefix: "img".into(),
            brightness: 5.0,
            contrast: 10.0,
        };

        let proj_path = tmp.path().join("test.pcut");
        save_project(&project, &proj_path).unwrap();

        let loaded = load_project(&proj_path).unwrap();
        assert_eq!(loaded.version, CURRENT_SCHEMA);
        assert_eq!(loaded.pages.len(), 1);
        assert_eq!(loaded.pages[0].path, img_path);
        assert_eq!(loaded.pages[0].rotation, 90);
        assert_eq!(loaded.pages[0].crop, Some(CropBox { x: 10, y: 20, w: 100, h: 200 }));
        assert_eq!(loaded.crop_presets[0].name, "test");
        assert!(matches!(loaded.export, ExportSettings::Jpeg { quality: 85 }));
        assert_eq!(loaded.brightness, 5.0);
        assert_eq!(loaded.contrast, 10.0);
    }

    #[test]
    fn load_rejects_future_version() {
        let tmp = tempfile::tempdir().unwrap();
        let json = serde_json::json!({
            "version": 99,
            "pages": [],
            "crop_presets": [],
            "export": {"format": "png"},
            "output_dir": "",
            "prefix": "p"
        });
        let proj_path = tmp.path().join("future.pcut");
        std::fs::write(&proj_path, serde_json::to_string(&json).unwrap()).unwrap();
        let err = load_project(&proj_path).unwrap_err();
        assert!(matches!(err, Error::UnsupportedProjectVersion { found: 99, supported: _ }));
    }

    #[test]
    fn version_defaults_to_1() {
        let tmp = tempfile::tempdir().unwrap();
        let json = r#"{"pages":[],"crop_presets":[],"export":{"format":"png"},"output_dir":"","prefix":"p"}"#;
        let proj_path = tmp.path().join("old.pcut");
        std::fs::write(&proj_path, json).unwrap();
        let project = load_project(&proj_path).unwrap();
        assert_eq!(project.version, 1);
    }
}
