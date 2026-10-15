//! Image transforms: the per-page export pipeline and the colour math it
//! shares with the on-screen preview.

pub mod color;
mod pipeline;

pub use pipeline::{transform_page, transform_page_bleed, PipelineContext};
