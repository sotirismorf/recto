pub mod event;
pub mod state;

pub use event::AppEvent;
pub use state::AppState;

use crate::domain::crop::{CropBox, CropPreset};
use crate::domain::export::{ExportSettings, OutputSize};
use crate::domain::values::{Brightness, Contrast, Rotation, Saturation, Scale};
use std::path::PathBuf;

/// Every mutation to the project is expressed as a [`Command`].
///
/// Commands carry all the data needed to apply **and** reverse the mutation,
/// making them the foundation for undo/redo (see [`AppState`]).
#[derive(Clone, Debug)]
pub enum Command {
    AddPages(Vec<PathBuf>),
    RemovePages(Vec<usize>),
    ReorderPages {
        from: usize,
        to: usize,
    },
    SetRotation {
        index: usize,
        rotation: Rotation,
    },
    SetCrop {
        index: usize,
        crop: Option<CropBox>,
    },
    SetCropPreset {
        index: usize,
        preset: Option<usize>,
    },
    SetOutputSize {
        index: usize,
        size: Option<OutputSize>,
    },
    SetBrightness(Brightness),
    SetContrast(Contrast),
    SetSaturation(Saturation),
    SetExportScale(Scale),
    SetExportSettings(ExportSettings),
    SetOutputDir(PathBuf),
    SetPrefix(String),
    AddCropPreset(CropPreset),
    RemoveCropPreset(usize),
    SetCropPresetLocked {
        index: usize,
        locked: bool,
    },
    SetCropPresetSize {
        index: usize,
        w: u32,
        h: u32,
    },
}
