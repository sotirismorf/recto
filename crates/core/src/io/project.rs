use crate::domain::crop::CropPreset;
use crate::domain::export::ExportSettings;
use crate::domain::project::{Page, Project, CURRENT_SCHEMA};
use crate::domain::values::{Brightness, Contrast, Rotation, Saturation, Scale};
use crate::error::{Error, Result};
use crate::io;
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct ProjectView<'a> {
    version: u32,
    pages: Vec<PageView<'a>>,
    crop_presets: &'a [CropPreset],
    export: &'a ExportSettings,
    output_dir: &'a Path,
    prefix: &'a str,
    #[serde(default)]
    brightness: Brightness,
    #[serde(default)]
    contrast: Contrast,
    #[serde(default)]
    saturation: Saturation,
    #[serde(default)]
    export_scale: Scale,
}

#[derive(Serialize)]
struct PageView<'a> {
    path: &'a Path,
    #[serde(default)]
    rotation: Rotation,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop: Option<crate::domain::crop::CropBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crop_preset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<crate::domain::export::OutputSize>,
}

impl<'a> From<&'a Page> for PageView<'a> {
    fn from(page: &'a Page) -> Self {
        Self {
            path: &page.path,
            rotation: page.rotation,
            crop: page.crop,
            crop_preset: page.crop_preset,
            output: page.output,
        }
    }
}

pub fn save_project(project: &Project, file_path: &Path) -> Result<()> {
    let base = file_path.parent().unwrap_or(Path::new("."));
    let pages: Vec<PageView> = project
        .pages
        .iter()
        .map(|p| PageView {
            path: p.path.strip_prefix(base).unwrap_or(&p.path),
            ..PageView::from(p)
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
        saturation: project.saturation,
        export_scale: project.export_scale,
    };
    let json = serde_json::to_string_pretty(&view)?;
    io::write_atomic(file_path, json.as_bytes())
}

/// Upgrade a loaded project from an older schema version to the current one.
///
/// Version migrations are applied sequentially.  Right now this is a no-op
/// (current schema == 1), but it provides the hook for future version bumps.
pub fn migrate_project(project: &mut Project) {
    while project.version < CURRENT_SCHEMA {
        match project.version {
            0 => project.version = 1,
            _ => break,
        }
    }
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
    migrate_project(&mut project);
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
    use crate::domain::crop::{CropBox, CropPreset};
    use crate::domain::export::{ExportSettings, OutputSize};
    use crate::domain::project::Page;
    use crate::domain::values::{Brightness, Contrast, JpegQuality, Saturation};

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
                rotation: Rotation::DEG90,
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
            export: ExportSettings::Jpeg {
                quality: JpegQuality::new(85),
            },
            output_dir: tmp.path().join("out"),
            prefix: "img".into(),
            brightness: Brightness::MAX,
            contrast: Contrast::MAX,
            saturation: Saturation::new(-0.5),
            export_scale: Scale::new(0.5),
        };

        let proj_path = tmp.path().join("test.recto");
        save_project(&project, &proj_path).unwrap();

        let loaded = load_project(&proj_path).unwrap();
        assert_eq!(loaded.version, CURRENT_SCHEMA);
        assert_eq!(loaded.pages.len(), 1);
        assert_eq!(loaded.pages[0].path, img_path);
        assert_eq!(loaded.pages[0].rotation, Rotation::DEG90);
        assert_eq!(
            loaded.pages[0].crop,
            Some(CropBox {
                x: 10,
                y: 20,
                w: 100,
                h: 200
            })
        );
        assert_eq!(loaded.crop_presets[0].name, "test");
        assert_eq!(
            loaded.export,
            ExportSettings::Jpeg {
                quality: JpegQuality::new(85),
            }
        );
        assert_eq!(loaded.brightness, Brightness::MAX);
        assert_eq!(loaded.contrast, Contrast::MAX);
        assert_eq!(loaded.saturation, Saturation::new(-0.5));
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
        let proj_path = tmp.path().join("future.recto");
        std::fs::write(&proj_path, serde_json::to_string(&json).unwrap()).unwrap();
        let err = load_project(&proj_path).unwrap_err();
        assert!(matches!(
            err,
            Error::UnsupportedProjectVersion {
                found: 99,
                supported: _
            }
        ));
    }

    #[test]
    fn version_defaults_to_1() {
        let tmp = tempfile::tempdir().unwrap();
        let json = r#"{"pages":[],"crop_presets":[],"export":{"format":"png"},"output_dir":"","prefix":"p"}"#;
        let proj_path = tmp.path().join("old.recto");
        std::fs::write(&proj_path, json).unwrap();
        let project = load_project(&proj_path).unwrap();
        assert_eq!(project.version, 1);
    }
}
