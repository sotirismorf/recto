use crate::domain::crop::BleedInsets;
use crate::domain::export::{PdfCompression, PdfOptions};
use crate::domain::project::Project;
use crate::domain::values::Scale;
use crate::error::{Error, Result};
use crate::io;
use crate::io::config::PdfMeta;
use crate::transform::{transform_page_bleed, PipelineContext};
use image::{codecs::jpeg::JpegEncoder, DynamicImage};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// One page's encoded image plus the geometry needed to place it.
struct PdfPage {
    data: Vec<u8>,
    filter: pdf_writer::Filter,
    w: u32,
    h: u32,
    insets: BleedInsets,
    expanded: (u32, u32),
}

/// Encode a page image for embedding: JPEG (DCTDecode) or lossless zlib
/// (FlateDecode) over the raw 8-bit RGB samples.
fn encode_image(img: DynamicImage, opts: &PdfOptions) -> Result<(Vec<u8>, pdf_writer::Filter)> {
    let rgb = img.into_rgb8();
    Ok(match opts.compression {
        PdfCompression::Jpeg => {
            let mut jpeg = Vec::new();
            DynamicImage::ImageRgb8(rgb).write_with_encoder(JpegEncoder::new_with_quality(
                &mut jpeg,
                opts.quality.as_u8(),
            ))?;
            (jpeg, pdf_writer::Filter::DctDecode)
        }
        PdfCompression::Flate => (
            miniz_oxide::deflate::compress_to_vec_zlib(rgb.as_raw(), 6),
            pdf_writer::Filter::FlateDecode,
        ),
    })
}

/// The visible page height in millimetres: the EXIF estimate from the first
/// page when requested and available, otherwise the configured height.
fn page_height_mm(project: &Project, opts: &PdfOptions) -> u32 {
    if !opts.estimate_page_height {
        return opts.page_height_mm;
    }
    project
        .pages
        .first()
        .and_then(io::estimate_page_height_mm)
        .map(|mm| mm.round() as u32)
        .unwrap_or_else(|| {
            tracing::warn!(
                "cannot estimate page height from EXIF; using {} mm",
                opts.page_height_mm
            );
            opts.page_height_mm
        })
}

/// Export all pages as a single PDF, encoding images per `opts`.
///
/// The `on_progress` callback receives `(done, total)` after each page.
/// Metadata from [`PdfMeta`] is written into the PDF document info dictionary.
/// Set `cancel` to `true` from another thread to abort the export early.
pub fn export_to_pdf<F>(
    project: &Project,
    out_path: &Path,
    meta: &PdfMeta,
    opts: &PdfOptions,
    scale: Scale,
    cancel: Arc<AtomicBool>,
    on_progress: F,
) -> Result<()>
where
    F: Fn(usize, usize) + Sync + Send,
{
    use pdf_writer::{Name, Pdf, Rect, Ref, TextStr};

    let total = project.pages.len();
    let done = AtomicUsize::new(0);
    on_progress(0, total);

    let encoded: Vec<Result<PdfPage>> = project
        .pages
        .par_iter()
        .map(|page| {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let ctx = PipelineContext::from_page(project, page, scale);
            let (img, insets, expanded) = transform_page_bleed(&ctx, project.bleed.as_u32())?;
            let (w, h) = (img.width(), img.height());
            let (data, filter) = encode_image(img, opts)?;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            on_progress(n, total);
            Ok(PdfPage {
                data,
                filter,
                w,
                h,
                insets,
                expanded,
            })
        })
        .collect();

    for result in &encoded {
        if let Err(Error::Cancelled) = result {
            return Err(Error::Cancelled);
        }
    }

    let pages: Vec<PdfPage> = encoded.into_iter().collect::<Result<_>>()?;
    let n = pages.len();
    let page_height_mm = page_height_mm(project, opts);

    let mut pdf = Pdf::new();
    let mut alloc = Ref::new(1);
    let catalog_ref = alloc.bump();
    let page_tree_ref = alloc.bump();

    let refs: Vec<[Ref; 3]> = (0..n)
        .map(|_| [alloc.bump(), alloc.bump(), alloc.bump()])
        .collect();

    for (i, page_img) in pages.iter().enumerate() {
        let [img_ref, content_ref, page_ref] = refs[i];
        let ((w_pt, h_pt), inner) = page_boxes(
            page_img.w,
            page_img.h,
            &page_img.insets,
            page_img.expanded,
            meta.dpi.as_f64(),
            page_height_mm,
        );

        pdf.image_xobject(img_ref, &page_img.data)
            .width(page_img.w as i32)
            .height(page_img.h as i32)
            .color_space_name(Name(b"DeviceRGB"))
            .bits_per_component(8)
            .filter(page_img.filter);

        let content = format!("q\n{w_pt:.4} 0 0 {h_pt:.4} 0 0 cm\n/Im Do\nQ\n");
        pdf.stream(content_ref, content.as_bytes());

        let mut page = pdf.page(page_ref);
        page.media_box(Rect::new(0.0, 0.0, w_pt as f32, h_pt as f32));
        if let Some([x0, y0, x1, y1]) = inner {
            page.crop_box(Rect::new(x0, y0, x1, y1));
            page.trim_box(Rect::new(x0, y0, x1, y1));
        }
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

/// Compute the page media box size and the inner (visible) box in points.
///
/// `w`/`h` are the final embedded-image pixels; `insets`/`expanded` describe
/// the bleed in pre-resize pixels (converted via fractions, which survive
/// resizing). `inner` is `None` when there is no bleed. When `page_height_mm`
/// is set, it fixes the height of the *visible* box, not the media box, so
/// the displayed page matches the requested size regardless of bleed.
fn page_boxes(
    w: u32,
    h: u32,
    insets: &BleedInsets,
    expanded: (u32, u32),
    dpi: f64,
    page_height_mm: u32,
) -> ((f64, f64), Option<[f32; 4]>) {
    let mut w_pt = w as f64 * 72.0 / dpi;
    let mut h_pt = h as f64 * 72.0 / dpi;

    if insets.is_zero() {
        if page_height_mm > 0 {
            let target_h_pt = (page_height_mm as f64 * 72.0) / 25.4;
            let scale = target_h_pt / h_pt;
            h_pt = target_h_pt;
            w_pt *= scale;
        }
        return ((w_pt, h_pt), None);
    }

    let (fl, ft, fr, fb) = insets.fractions(expanded.0, expanded.1);
    // PDF y-axis points up: the top inset subtracts from the high-y edge.
    let mut x0 = w_pt * fl;
    let mut x1 = w_pt * (1.0 - fr);
    let mut y0 = h_pt * fb;
    let mut y1 = h_pt * (1.0 - ft);

    if page_height_mm > 0 {
        let target_h_pt = (page_height_mm as f64 * 72.0) / 25.4;
        let scale = target_h_pt / (y1 - y0);
        w_pt *= scale;
        h_pt *= scale;
        x0 *= scale;
        x1 *= scale;
        y0 *= scale;
        y1 *= scale;
    }

    (
        (w_pt, h_pt),
        Some([x0 as f32, y0 as f32, x1 as f32, y1 as f32]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flate_embeds_exact_pixels() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("p.png");
        let src =
            image::RgbImage::from_fn(7, 5, |x, y| image::Rgb([x as u8 * 30, y as u8 * 50, 9]));
        src.save(&img_path).unwrap();
        let mut project = Project::default();
        project
            .pages
            .push(crate::domain::project::Page::new(img_path));
        let out = tmp.path().join("out.pdf");
        let opts = PdfOptions {
            compression: PdfCompression::Flate,
            ..Default::default()
        };
        export_to_pdf(
            &project,
            &out,
            &PdfMeta::default(),
            &opts,
            Scale::FULL,
            Arc::new(AtomicBool::new(false)),
            |_, _| {},
        )
        .unwrap();

        let pdf = std::fs::read(&out).unwrap();
        let find = |needle: &[u8]| pdf.windows(needle.len()).position(|w| w == needle);
        assert!(find(b"/FlateDecode").is_some());
        let start = find(b"stream\n").unwrap() + b"stream\n".len();
        let end = start + find(b"\nendstream").unwrap() - start;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&pdf[start..end]).unwrap();
        assert_eq!(raw, src.into_raw());
    }

    const EPS: f64 = 1e-6;

    #[test]
    fn page_boxes_without_bleed_has_no_inner_box() {
        let ((w_pt, h_pt), inner) =
            page_boxes(3000, 4500, &BleedInsets::default(), (3000, 4500), 300.0, 0);
        assert!((w_pt - 720.0).abs() < EPS);
        assert!((h_pt - 1080.0).abs() < EPS);
        assert!(inner.is_none());
    }

    #[test]
    fn page_boxes_symmetric_bleed_centers_inner_box() {
        let insets = BleedInsets {
            left: 30,
            top: 30,
            right: 30,
            bottom: 30,
        };
        // 300 px inner crop + 30 px bleed on each side, at 300 DPI.
        let ((w_pt, h_pt), inner) = page_boxes(360, 360, &insets, (360, 360), 300.0, 0);
        assert!((w_pt - 86.4).abs() < EPS);
        assert!((h_pt - 86.4).abs() < EPS);
        let [x0, y0, x1, y1] = inner.unwrap();
        assert!((x0 as f64 - 7.2).abs() < 1e-4);
        assert!((y0 as f64 - 7.2).abs() < 1e-4);
        assert!((x1 as f64 - 79.2).abs() < 1e-4);
        assert!((y1 as f64 - 79.2).abs() < 1e-4);
    }

    #[test]
    fn page_boxes_asymmetric_top_inset_subtracts_from_high_y() {
        let insets = BleedInsets {
            left: 0,
            top: 50,
            right: 0,
            bottom: 0,
        };
        let ((_, h_pt), inner) = page_boxes(100, 200, &insets, (100, 200), 72.0, 0);
        let [_, y0, _, y1] = inner.unwrap();
        assert!((y0 as f64 - 0.0).abs() < EPS);
        assert!((y1 as f64 - (h_pt - 50.0)).abs() < 1e-4);
    }

    #[test]
    fn page_height_pins_visible_box_height() {
        let insets = BleedInsets {
            left: 20,
            top: 20,
            right: 20,
            bottom: 20,
        };
        let ((_, h_pt), inner) = page_boxes(240, 240, &insets, (240, 240), 300.0, 200);
        let [_, y0, _, y1] = inner.unwrap();
        let target = 200.0 * 72.0 / 25.4;
        assert!((y1 as f64 - y0 as f64 - target).abs() < 1e-3);
        // Media box is taller than the visible box by the bleed fractions.
        assert!(h_pt > target);
    }

    #[test]
    fn page_height_without_bleed_matches_previous_behavior() {
        let ((w_pt, h_pt), inner) = page_boxes(
            1000,
            2000,
            &BleedInsets::default(),
            (1000, 2000),
            300.0,
            254,
        );
        assert!(inner.is_none());
        assert!((h_pt - 720.0).abs() < EPS);
        assert!((w_pt - 360.0).abs() < EPS);
    }
}
