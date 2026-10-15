use crate::domain::project::{Project, CURRENT_SCHEMA};
use crate::error::{Error, Result};
use crate::io;
use std::path::Path;

/// Save `project` as JSON. Page paths under the project file's directory are
/// stored relative to it, so a project folder can be moved as a whole.
pub fn save_project(project: &Project, file_path: &Path) -> Result<()> {
    let base = file_path.parent().unwrap_or(Path::new("."));
    let mut saved = project.clone();
    saved.version = CURRENT_SCHEMA;
    for page in &mut saved.pages {
        if let Ok(relative) = page.path.strip_prefix(base) {
            page.path = relative.to_path_buf();
        }
    }
    let json = serde_json::to_string_pretty(&saved)?;
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
    use crate::domain::project::Page;
    use crate::domain::values::{
        BlackLevel, Bleed, Exposure, Rotation, Saturation, Temperature, Tint,
    };

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
            }],
            crop_presets: vec![CropPreset {
                name: "test".into(),
                w: 100,
                h: 200,
                locked: false,
            }],
            output_dir: tmp.path().join("out"),
            prefix: "img".into(),
            exposure: Exposure::new(-0.5),
            black_level: BlackLevel::new(0.03),
            temperature: Temperature::new(0.2),
            tint: Tint::new(-0.1),
            saturation: Saturation::new(-0.5),
            bleed: Bleed::new(24),
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
        assert_eq!(loaded.exposure, Exposure::new(-0.5));
        assert_eq!(loaded.black_level, BlackLevel::new(0.03));
        assert_eq!(loaded.temperature, Temperature::new(0.2));
        assert_eq!(loaded.tint, Tint::new(-0.1));
        assert_eq!(loaded.saturation, Saturation::new(-0.5));
        assert_eq!(loaded.bleed, Bleed::new(24));
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
        assert_eq!(project.bleed, Bleed::ZERO);
    }

    #[test]
    fn saves_page_paths_relative_to_the_project_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut project = Project::default();
        project
            .pages
            .push(Page::new(tmp.path().join("scans/p1.jpg")));
        let proj_path = tmp.path().join("book.recto");
        save_project(&project, &proj_path).unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&proj_path).unwrap()).unwrap();
        assert_eq!(json["pages"][0]["path"], "scans/p1.jpg");
        assert!(json["pages"][0].get("crop").is_none());
        assert_eq!(project.pages[0].path, tmp.path().join("scans/p1.jpg"));
    }
}
