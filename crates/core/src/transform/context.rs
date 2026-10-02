use crate::domain::crop::CropBox;
use crate::domain::export::OutputSize;
use crate::domain::values::{Rotation, Scale};
use crate::transform::color::ColorAdjust;
use std::path::Path;

/// Immutable snapshot of all parameters for one page's transform.
/// Constructed before any work begins. Once created, project mutations
/// cannot affect an in-flight transform.
#[derive(Clone)]
pub struct PipelineContext<'a> {
    pub source_path: &'a Path,
    pub rotation: Rotation,
    pub crop: Option<CropBox>,
    pub output: Option<OutputSize>,
    pub color: ColorAdjust,
    pub scale: Scale,
}

impl<'a> PipelineContext<'a> {
    pub fn from_page(
        project: &'a crate::domain::project::Project,
        page: &'a crate::domain::project::Page,
    ) -> Self {
        Self {
            source_path: &page.path,
            rotation: page.rotation,
            crop: page.crop,
            output: page.output,
            color: ColorAdjust::from_project(project),
            scale: project.export_scale,
        }
    }
}
