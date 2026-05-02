//! # pagecutter-core
//!
//! The headless image-processing engine behind pagecutter2.
//!
//! ## Pipeline
//!
//! Every page flows through a fixed six-step transform:
//!   load → orient (EXIF) → rotate (user) → crop → colours → resize
//!
//! After transforms, pages are exported to PNG, JPEG, TIFF, or collated into a
//! single PDF via [`export_to_pdf`].
//!
//! ## Feature flags
//!
//! - `autodetect` — enables OpenCV-based crop presets via `opencv`.

pub mod error;
pub mod exif;
pub mod geometry;
pub mod io;
pub mod pipeline;
pub mod project;

pub use pipeline::{export_to_pdf, run_batch};
pub use project::{
    load_pdf_meta, load_project, save_pdf_meta, save_project, CropBox, CropPreset, ExportSettings,
    Page, PdfMeta, Project,
};
