use crate::domain::export::ExportSettings;
use crate::domain::project::Project;
use crate::domain::project::Page;
use crate::error::{Result, Error};
use crate::io;
use crate::transform::{PipelineContext, transform_page};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Process every page in parallel, saving individual image files according to
/// the project's [`ExportSettings`].
///
/// The `on_progress` callback receives `(done, total)` after each page
/// completes.  Returns one [`Result`] per page in order.
/// Set `cancel` to `true` from another thread to abort the export early;
/// pages already saved to disk before the cancel flag is observed will
/// remain on disk.
#[must_use]
pub fn run_batch<F>(project: &Project, cancel: Arc<AtomicBool>, on_progress: F) -> Vec<Result<PathBuf>>
where
    F: Fn(usize, usize) + Sync + Send,
{
    let total = project.pages.len();
    let done = AtomicUsize::new(0);
    on_progress(0, total);

    project
        .pages
        .par_iter()
        .enumerate()
        .map(|(i, page)| {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let r = process_one(project, page, i + 1);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            on_progress(n, total);
            r
        })
        .collect()
}

/// Export a single page to an image file, returning the output path.
fn process_one(project: &Project, page: &Page, index: usize) -> Result<PathBuf> {
    let ctx = PipelineContext::from_page(project, page);
    let img = transform_page(&ctx)?;

    let ext = match project.export {
        ExportSettings::Png { .. } => "png",
        ExportSettings::Jpeg { .. } => "jpg",
        ExportSettings::Tiff => "tiff",
        ExportSettings::Pdf { .. } => unreachable!(),
    };
    let filename = format!("{}_{:02}.{}", project.prefix, index, ext);
    let out_path = project.output_dir.join(filename);

    match &project.export {
        ExportSettings::Png { compression } => io::save_png(&img, &out_path, *compression)?,
        ExportSettings::Jpeg { quality } => io::save_jpeg(&img, &out_path, *quality)?,
        ExportSettings::Tiff => io::save_tiff(&img, &out_path)?,
        ExportSettings::Pdf { .. } => unreachable!(),
    }

    Ok(out_path)
}
