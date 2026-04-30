use crate::exif;
use crate::io as pio;
use crate::project::{ExportSettings, Page, Project};
use anyhow::Result;
use image::{imageops::FilterType, DynamicImage};
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn run_batch<F>(project: &Project, on_progress: F) -> Vec<Result<PathBuf>>
where
    F: Fn(usize, usize) + Sync + Send,
{
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

fn process_one(project: &Project, page: &Page, index: usize) -> Result<PathBuf> {
    let img = pio::load(&page.path)?;
    let img = exif::read_orientation(&page.path).apply(img);
    let img = apply_rotation(img, page.rotation);
    let img = apply_crop(img, page);
    let img = apply_colors(img, project.brightness, project.contrast);
    let img = apply_resize(img, page);

    let ext = match project.export {
        ExportSettings::Png => "png",
        ExportSettings::Jpeg { .. } => "jpg",
        ExportSettings::Tiff => "tiff",
    };
    let filename = format!("{}_{:02}.{}", project.prefix, index, ext);
    let out_path = project.output_dir.join(filename);

    match project.export {
        ExportSettings::Png => pio::save_png(&img, &out_path)?,
        ExportSettings::Jpeg { quality } => pio::save_jpeg(&img, &out_path, quality)?,
        ExportSettings::Tiff => pio::save_tiff(&img, &out_path)?,
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
    let mut rgb = img.into_rgb8();
    let c = (contrast + 1.0_f32).max(0.0);
    let b = (brightness * 128.0_f32).round() as i32;
    for pixel in rgb.pixels_mut() {
        for ch in pixel.0.iter_mut() {
            let v = ((*ch as f32 - 128.0) * c + 128.0).round() as i32 + b;
            *ch = v.clamp(0, 255) as u8;
        }
    }
    DynamicImage::ImageRgb8(rgb)
}

fn apply_resize(img: DynamicImage, page: &Page) -> DynamicImage {
    let Some(o) = page.output else { return img };
    if o.w == img.width() && o.h == img.height() {
        return img;
    }
    img.resize_exact(o.w, o.h, FilterType::Lanczos3)
}

