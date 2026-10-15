pub mod domain;
pub mod error;
pub mod export;
pub mod io;
pub mod state;
pub mod transform;

pub mod autodetect;

pub use domain::crop::{
    centered_crop, expand_crop, group_by_size, resize_crop, resize_crop_centered, BleedInsets,
    CropBox, CropPreset,
};
pub use domain::export::{ExportSettings, ImageFormat, PdfCompression, PdfOptions};
pub use domain::geometry::Rect;
pub use domain::order::{plan_move, MovePlan};
pub use domain::project::{Page, Project};
pub use domain::values::{
    BlackLevel, Bleed, Dpi, Exposure, JpegQuality, Rotation, Saturation, Scale, Temperature, Tint,
};
pub use export::{export_to_pdf, run_batch};
pub use io::camera::{estimate_page_height_mm, CameraGeometry};
pub use io::config::{load_config, save_config, AppConfig, PdfMeta};
pub use io::project::{load_project, save_project};
pub use state::{AppEvent, AppState, Command};
pub use transform::PipelineContext;
