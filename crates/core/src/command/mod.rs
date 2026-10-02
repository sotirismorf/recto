pub mod event;
pub mod state;

pub use event::AppEvent;
pub use state::AppState;

use crate::domain::crop::{CropBox, CropPreset};
use crate::domain::export::OutputSize;
use crate::domain::values::{BlackLevel, Bleed, Exposure, Rotation, Saturation, Temperature, Tint};
use std::path::PathBuf;

/// Every mutation to the project is expressed as a [`Command`] and applied
/// by [`AppState::dispatch`]. Undo is snapshot-based, so commands only need
/// to describe the forward change.
#[derive(Clone, Debug)]
pub enum Command {
    AddPages(Vec<PathBuf>),
    RemovePages(Vec<usize>),
    /// Move the pages at `indices` so they sit as one contiguous block
    /// immediately before the page currently at `before` (`before == len`
    /// appends). See [`plan_move`](crate::domain::order::plan_move) for the
    /// exact semantics.
    MovePages {
        indices: Vec<usize>,
        before: usize,
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
    SetExposure(Exposure),
    SetBlackLevel(BlackLevel),
    SetTemperature(Temperature),
    SetTint(Tint),
    SetSaturation(Saturation),
    SetBleed(Bleed),
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

impl Command {
    /// Continuous edits (slider drags, spin buttons) of the same setting share
    /// a key; consecutive dispatches with equal keys in quick succession
    /// coalesce into one undo step.
    pub(crate) fn coalesce_key(&self) -> Option<std::mem::Discriminant<Self>> {
        match self {
            Self::SetExposure(_)
            | Self::SetBlackLevel(_)
            | Self::SetTemperature(_)
            | Self::SetTint(_)
            | Self::SetSaturation(_)
            | Self::SetBleed(_) => Some(std::mem::discriminant(self)),
            _ => None,
        }
    }
}
