pub mod command;
pub mod domain;
pub mod error;
pub mod export;
pub mod io;
pub mod transform;

pub use command::{AppEvent, AppState, Command};
pub use domain::crop::{CropBox, CropPreset};
pub use domain::export::{ExportSettings, OutputSize};
pub use domain::geometry::Rect;
pub use domain::project::{Page, Project};
pub use domain::values::{Brightness, Contrast, Dpi, JpegQuality, Rotation};
pub use export::{export_to_pdf, run_batch};
pub use io::config::{load_pdf_meta, save_pdf_meta, PdfMeta};
pub use io::project::{load_project, save_project};
pub use transform::PipelineContext;

// Backward-compat re-exports so existing import paths still work.
pub mod project {
    pub use crate::domain::crop::{CropBox, CropPreset};
    pub use crate::domain::export::{ExportSettings, OutputSize};
    pub use crate::domain::project::{Page, Project, CURRENT_SCHEMA};
    pub use crate::io::project::{load_project, save_project};
}

pub mod geometry {
    pub use crate::domain::geometry::*;
}

pub mod pipeline {
    pub use crate::export::{export_to_pdf, run_batch};
    pub use crate::transform::PipelineContext;
}

pub mod config {
    pub use crate::io::config::{load_pdf_meta, save_pdf_meta, PdfMeta};
}

pub mod exif {
    pub use crate::io::exif::{read_orientation, Orientation};
}
