use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    Pdf { quality: u8 },
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
    let path = pdf_meta_path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(path, json.as_bytes())?;
    Ok(())
}

// --- Project save / load ----------------------------------------------------

pub fn save_project(project: &Project, file_path: &Path) -> Result<()> {
    let base = file_path.parent().unwrap_or(Path::new("."));
    let rel_pages: Vec<Page> = project
        .pages
        .iter()
        .map(|p| {
            let path = p
                .path
                .strip_prefix(base)
                .map(|r| r.to_path_buf())
                .unwrap_or_else(|_| p.path.clone());
            Page { path, ..p.clone() }
        })
        .collect();
    let saved = Project { pages: rel_pages, ..project.clone() };
    let json = serde_json::to_string_pretty(&saved)?;
    std::fs::write(file_path, json.as_bytes())?;
    Ok(())
}

pub fn load_project(file_path: &Path) -> Result<Project> {
    let base = file_path.parent().unwrap_or(Path::new("."));
    let json = std::fs::read_to_string(file_path)?;
    let mut project: Project = serde_json::from_str(&json)?;
    for page in &mut project.pages {
        if page.path.is_relative() {
            page.path = base.join(&page.path);
        }
    }
    Ok(project)
}
