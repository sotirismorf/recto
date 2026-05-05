use crate::domain::export::ExportSettings;
use crate::domain::project::Project;
use crate::domain::values::JpegQuality;
use crate::error::{Error, Result};
use crate::io;
use crate::io::config::PdfMeta;
use crate::transform::{transform_page, PipelineContext};
use image::{codecs::jpeg::JpegEncoder, DynamicImage};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Export all pages as a single PDF with JPEG-encoded images.
///
/// The `on_progress` callback receives `(done, total)` after each page.
/// Metadata from [`PdfMeta`] is written into the PDF document info dictionary.
/// Set `cancel` to `true` from another thread to abort the export early.
pub fn export_to_pdf<F>(
    project: &Project,
    out_path: &Path,
    meta: &PdfMeta,
    jpeg_quality: JpegQuality,
    cancel: Arc<AtomicBool>,
    on_progress: F,
) -> Result<()>
where
    F: Fn(usize, usize) + Sync + Send,
{
    use pdf_writer::{Filter, Name, Pdf, Rect, Ref, TextStr};

    let total = project.pages.len();
    let done = AtomicUsize::new(0);
    let quality = jpeg_quality.as_u8();
    on_progress(0, total);

    let jpeg_data: Vec<Result<(Vec<u8>, u32, u32)>> = project
        .pages
        .par_iter()
        .map(|page| {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let ctx = PipelineContext::from_page(project, page);
            let img = transform_page(&ctx)?;
            let (w, h) = (img.width(), img.height());
            let mut jpeg = Vec::new();
            DynamicImage::ImageRgb8(img.into_rgb8())
                .write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, quality))?;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            on_progress(n, total);
            Ok((jpeg, w, h))
        })
        .collect();

    for result in &jpeg_data {
        if let Err(Error::Cancelled) = result {
            return Err(Error::Cancelled);
        }
    }

    let pages: Vec<(Vec<u8>, u32, u32)> = jpeg_data.into_iter().collect::<Result<_>>()?;
    let n = pages.len();

    let page_height_mm = match project.export {
        ExportSettings::Pdf { page_height_mm, .. } => page_height_mm,
        _ => 0,
    };

    let mut pdf = Pdf::new();
    let mut alloc = Ref::new(1);
    let catalog_ref = alloc.bump();
    let page_tree_ref = alloc.bump();

    let refs: Vec<[Ref; 3]> = (0..n)
        .map(|_| [alloc.bump(), alloc.bump(), alloc.bump()])
        .collect();

    for (i, (jpeg, w, h)) in pages.iter().enumerate() {
        let [img_ref, content_ref, page_ref] = refs[i];
        let mut w_pt = *w as f64 * 72.0 / meta.dpi.as_f64();
        let mut h_pt = *h as f64 * 72.0 / meta.dpi.as_f64();

        if page_height_mm > 0 {
            let target_h_pt = (page_height_mm as f64 * 72.0) / 25.4;
            let scale = target_h_pt / h_pt;
            h_pt = target_h_pt;
            w_pt *= scale;
        }

        pdf.image_xobject(img_ref, jpeg)
            .width(*w as i32)
            .height(*h as i32)
            .color_space_name(Name(b"DeviceRGB"))
            .bits_per_component(8)
            .filter(Filter::DctDecode);

        let content = format!("q\n{w_pt:.4} 0 0 {h_pt:.4} 0 0 cm\n/Im Do\nQ\n");
        pdf.stream(content_ref, content.as_bytes());

        let mut page = pdf.page(page_ref);
        page.media_box(Rect::new(0.0, 0.0, w_pt as f32, h_pt as f32));
        page.parent(page_tree_ref);
        page.contents(content_ref);
        page.resources().x_objects().pair(Name(b"Im"), img_ref);
    }

    pdf.pages(page_tree_ref)
        .count(n as i32)
        .kids(refs.iter().map(|r| r[2]));

    pdf.catalog(catalog_ref).pages(page_tree_ref);

    {
        let info_ref = alloc.bump();
        let mut info = pdf.document_info(info_ref);
        info.producer(TextStr("Recto"));
        if !meta.creator.is_empty() {
            info.creator(TextStr(&meta.creator));
        }
        if !meta.title.is_empty() {
            info.title(TextStr(&meta.title));
        }
        if !meta.author.is_empty() {
            info.author(TextStr(&meta.author));
        }
        if !meta.subject.is_empty() {
            info.subject(TextStr(&meta.subject));
        }
        if !meta.keywords.is_empty() {
            info.keywords(TextStr(&meta.keywords));
        }
    }

    let pdf_bytes = pdf.finish();
    io::write_atomic(out_path, &pdf_bytes)
}
