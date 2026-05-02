use crate::error::{Error, Result};
use crate::exif;
use crate::io as pio;
use crate::project::{ExportSettings, Page, PdfMeta, Project};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn transform_page(project: &Project, page: &Page) -> Result<DynamicImage> {
    let img = pio::load(&page.path)?;
    let img = exif::read_orientation(&page.path).apply(img);
    let img = apply_rotation(img, page.rotation);
    let img = apply_crop(img, page);
    let img = apply_colors(img, project.brightness, project.contrast);
    let img = apply_resize(img, page);
    Ok(img)
}

pub fn run_batch<F>(project: &Project, on_progress: F) -> Vec<Result<PathBuf>>
where
    F: Fn(usize, usize) + Sync + Send,
{
    if matches!(project.export, ExportSettings::Pdf { .. }) {
        return vec![Err(Error::PdfNotSupportedInBatch)];
    }

    let total = project.pages.len();
    let done = AtomicUsize::new(0);

    project
        .pages
        .par_iter()
        .enumerate()
        .map(|(i, page)| {
            let r = process_one(project, page, i + 1);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            on_progress(n, total);
            r
        })
        .collect()
}

pub fn export_to_pdf<F>(
    project: &Project,
    out_path: &Path,
    meta: &PdfMeta,
    jpeg_quality: u8,
    on_progress: F,
) -> Result<()>
where
    F: Fn(usize, usize) + Sync + Send,
{
    use pdf_writer::{Filter, Name, Pdf, Rect, Ref, TextStr};

    let total = project.pages.len();
    let done = AtomicUsize::new(0);

    let jpeg_data: Vec<Result<(Vec<u8>, u32, u32)>> = project
        .pages
        .par_iter()
        .map(|page| {
            let img = transform_page(project, page)?;
            let (w, h) = (img.width(), img.height());
            let mut jpeg = Vec::new();
            DynamicImage::ImageRgb8(img.into_rgb8())
                .write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, jpeg_quality))?;
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            on_progress(n, total);
            Ok((jpeg, w, h))
        })
        .collect();

    let pages: Vec<(Vec<u8>, u32, u32)> = jpeg_data.into_iter().collect::<Result<_>>()?;
    let n = pages.len();

    let mut pdf = Pdf::new();
    let mut alloc = Ref::new(1);
    let catalog_ref = alloc.bump();
    let page_tree_ref = alloc.bump();

    let refs: Vec<[Ref; 3]> = (0..n)
        .map(|_| [alloc.bump(), alloc.bump(), alloc.bump()])
        .collect();

    for (i, (jpeg, w, h)) in pages.iter().enumerate() {
        let [img_ref, content_ref, page_ref] = refs[i];
        let w_pt = *w as f64 * 72.0 / meta.dpi;
        let h_pt = *h as f64 * 72.0 / meta.dpi;

        pdf.image_xobject(img_ref, jpeg)
            .width(*w as i32)
            .height(*h as i32)
            .color_space_name(Name(b"DeviceRGB"))
            .bits_per_component(8)
            .filter(Filter::DctDecode);

        let content =
            format!("q\n{w_pt:.4} 0 0 {h_pt:.4} 0 0 cm\n/Im Do\nQ\n");
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
        info.producer(TextStr("pagecutter"));
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
    pio::write_atomic(out_path, &pdf_bytes)
}

fn process_one(project: &Project, page: &Page, index: usize) -> Result<PathBuf> {
    let img = transform_page(project, page)?;

    let ext = match project.export {
        ExportSettings::Png => "png",
        ExportSettings::Jpeg { .. } => "jpg",
        ExportSettings::Tiff => "tiff",
        ExportSettings::Pdf { .. } => unreachable!(),
    };
    let filename = format!("{}_{:02}.{}", project.prefix, index, ext);
    let out_path = project.output_dir.join(filename);

    match project.export {
        ExportSettings::Png => pio::save_png(&img, &out_path)?,
        ExportSettings::Jpeg { quality } => pio::save_jpeg(&img, &out_path, quality)?,
        ExportSettings::Tiff => pio::save_tiff(&img, &out_path)?,
        ExportSettings::Pdf { .. } => unreachable!(),
    }

    Ok(out_path)
}

fn apply_rotation(img: DynamicImage, rotation: u16) -> DynamicImage {
    match rotation % 360 {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    }
}

fn apply_crop(img: DynamicImage, page: &Page) -> DynamicImage {
    let Some(c) = page.crop else { return img };
    let (iw, ih) = (img.width(), img.height());
    let x = c.x.min(iw.saturating_sub(1));
    let y = c.y.min(ih.saturating_sub(1));
    let w = c.w.min(iw - x);
    let h = c.h.min(ih - y);
    img.crop_imm(x, y, w, h)
}

fn apply_colors(img: DynamicImage, brightness: f32, contrast: f32) -> DynamicImage {
    if brightness == 0.0 && contrast == 0.0 {
        return img;
    }
    let c = (contrast + 1.0).max(0.0) as f64;
    let b = (brightness as f64 * 128.0).round() as i32;
    let mut rgb = img.into_rgb8();
    rgb.as_mut().par_iter_mut().for_each(|ch| {
        let v = ((*ch as f64 - 128.0) * c + 128.0).round() as i32 + b;
        *ch = v.clamp(0, 255) as u8;
    });
    DynamicImage::ImageRgb8(rgb)
}

fn apply_resize(img: DynamicImage, page: &Page) -> DynamicImage {
    let Some(o) = page.output else { return img };
    if o.w == img.width() && o.h == img.height() {
        return img;
    }
    img.resize_exact(o.w, o.h, FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::CropBox;

    #[test]
    fn transform_page_smoke() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test.png");
        let test_img = image::ImageBuffer::from_fn(64, 48, |x, y| {
            image::Rgb([x as u8, y as u8, 128u8])
        });
        test_img.save(&img_path).unwrap();

        let project = Project::default();
        let page = Page {
            path: img_path,
            rotation: 0,
            crop: None,
            crop_preset: None,
            output: None,
        };

        let result = transform_page(&project, &page).unwrap();
        assert_eq!(result.width(), 64);
        assert_eq!(result.height(), 48);
    }

    #[test]
    fn transform_page_with_crop_and_resize() {
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test.png");
        let test_img = image::ImageBuffer::from_fn(128, 128, |x, y| {
            image::Rgb([x as u8, y as u8, 255u8])
        });
        test_img.save(&img_path).unwrap();

        let project = Project {
            brightness: 10.0,
            contrast: 5.0,
            ..Project::default()
        };
        let page = Page {
            path: img_path,
            rotation: 0,
            crop: Some(CropBox {
                x: 16,
                y: 16,
                w: 64,
                h: 64,
            }),
            crop_preset: None,
            output: Some(crate::project::OutputSize { w: 32, h: 32 }),
        };

        let result = transform_page(&project, &page).unwrap();
        assert_eq!(result.width(), 32);
        assert_eq!(result.height(), 32);
    }
}
