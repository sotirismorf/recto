pub mod command;
pub mod domain;
pub mod error;
pub mod export;
pub mod io;
pub mod transform;

pub mod autodetect;

pub use command::{AppEvent, AppState, Command};
pub use domain::crop::{CropBox, CropPreset};
pub use domain::export::{ExportSettings, OutputSize, PdfCompression};
pub use domain::geometry::Rect;
pub use domain::project::{Page, Project};
pub use domain::values::{Brightness, Contrast, Dpi, JpegQuality, Rotation, Saturation, Scale};
pub use export::{export_to_pdf, run_batch};
pub use io::config::{load_config, save_config, AppConfig, PdfMeta};
pub use io::project::{load_project, save_project};
pub use transform::PipelineContext;
