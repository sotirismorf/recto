use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib};

use crate::app::{MarkDirty, State};
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};
use recto_core::project::CropBox;

struct ColorReq {
    id: u64,
    path: PathBuf,
    rotation: u32,
    crop: Option<CropBox>,
    brightness: f32,
    contrast: f32,
}

struct ColorResult {
    id: u64,
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    rowstride: i32,
    has_alpha: bool,
}

pub fn build(
    state: State,
    selection: gtk::MultiSelection,
    paned_sync: super::PanedSync,
    mark_dirty: MarkDirty,
) -> gtk::Widget {
    let req_id: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let (tx_req, rx_req) = async_channel::unbounded::<ColorReq>();
    let (tx_res, rx_res) = async_channel::unbounded::<ColorResult>();

    std::thread::spawn(move || {
        while let Ok(mut req) = rx_req.recv_blocking() {
            while let Ok(next) = rx_req.try_recv() {
                req = next;
            }
            let Some(result) = render_preview(&req) else { continue };
            let _ = tx_res.send_blocking(result);
        }
    });

    let init_brightness = state.borrow().brightness as f64;
    let init_contrast = state.borrow().contrast as f64;

    let brightness_lbl = gtk::Label::new(Some("Brightness"));
    let brightness_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.01);
    brightness_scale.set_width_request(180);
    brightness_scale.set_value(init_brightness);
    brightness_scale.set_draw_value(true);
    brightness_scale.set_digits(2);

    let contrast_lbl = gtk::Label::new(Some("Contrast"));
    let contrast_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.01);
    contrast_scale.set_width_request(180);
    contrast_scale.set_value(init_contrast);
    contrast_scale.set_draw_value(true);
    contrast_scale.set_digits(2);

    let reset_btn = gtk::Button::builder()
        .label("Reset")
        .tooltip_text("Reset brightness and contrast to zero")
        .build();

    let spacer = gtk::Box::builder().hexpand(true).build();
    let count_lbl = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .margin_end(4)
        .build();

    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(8)
        .margin_end(8)
        .build();
    toolbar.append(&brightness_lbl);
    toolbar.append(&brightness_scale);
    toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    toolbar.append(&contrast_lbl);
    toolbar.append(&contrast_scale);
    toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    toolbar.append(&reset_btn);
    toolbar.append(&spacer);
    toolbar.append(&count_lbl);

    let preview = PreviewCanvas::new();
    preview.set_hexpand(true);
    preview.set_vexpand(true);
    preview.add_css_class("view");

    let zoom_pan = ZoomPanController::attach(
        &preview,
        &preview,
        ZoomPanConfig {
            pan_button: gdk::BUTTON_PRIMARY,
            ..Default::default()
        },
    );

    let factory = super::grid::simple_factory();
    let (_grid_view, grid_scroll) = super::grid::build_grid_scroll(&selection, &factory);

    let left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    left.append(&preview);
    left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    left.append(&toolbar);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&grid_scroll)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .vexpand(true)
        .build();
    paned_sync.register(&paned);

    glib::MainContext::default().spawn_local({
        let preview_weak = preview.downgrade();
        let req_id = req_id.clone();
        async move {
            while let Ok(result) = rx_res.recv().await {
                if result.id != req_id.get() { continue; }
                let pb = gdk_pixbuf::Pixbuf::from_bytes(
                    &result.bytes,
                    gdk_pixbuf::Colorspace::Rgb,
                    result.has_alpha,
                    8,
                    result.width,
                    result.height,
                    result.rowstride,
                );
                let Some(preview) = preview_weak.upgrade() else { continue };
                preview.set_texture(Some(gdk::Texture::for_pixbuf(&pb)));
                zoom_pan.refit_after_texture_change();
            }
        }
    });

    selection.connect_selection_changed({
        let state = state.clone();
        let selection = selection.clone();
        let req_id = req_id.clone();
        let tx_req = tx_req.clone();
        let count_lbl = count_lbl.clone();
        move |_, _, _| {
            let n = selection.selection().size();
            count_lbl.set_label(&format!("{} selected", n));
            send_preview_req(&state, &selection, &req_id, &tx_req);
        }
    });

    brightness_scale.connect_value_changed({
        let state = state.clone();
        let selection = selection.clone();
        let req_id = req_id.clone();
        let tx_req = tx_req.clone();
        let mark_dirty = mark_dirty.clone();
        move |scale| {
            let v = scale.value() as f32;
            let mut p = state.borrow_mut();
            if p.brightness != v {
                p.brightness = v;
                drop(p);
                mark_dirty();
            } else {
                drop(p);
            }
            send_preview_req(&state, &selection, &req_id, &tx_req);
        }
    });

    contrast_scale.connect_value_changed({
        let state = state.clone();
        let selection = selection.clone();
        let req_id = req_id.clone();
        let tx_req = tx_req.clone();
        let mark_dirty = mark_dirty.clone();
        move |scale| {
            let v = scale.value() as f32;
            let mut p = state.borrow_mut();
            if p.contrast != v {
                p.contrast = v;
                drop(p);
                mark_dirty();
            } else {
                drop(p);
            }
            send_preview_req(&state, &selection, &req_id, &tx_req);
        }
    });

    reset_btn.connect_clicked({
        let state = state.clone();
        let brightness_scale = brightness_scale.clone();
        let contrast_scale = contrast_scale.clone();
        let mark_dirty = mark_dirty.clone();
        move |_| {
            let mut p = state.borrow_mut();
            let changed = p.brightness != 0.0 || p.contrast != 0.0;
            p.brightness = 0.0;
            p.contrast = 0.0;
            drop(p);
            brightness_scale.set_value(0.0);
            contrast_scale.set_value(0.0);
            if changed {
                mark_dirty();
            }
        }
    });

    paned.upcast()
}

fn send_preview_req(
    state: &State,
    selection: &gtk::MultiSelection,
    req_id: &Rc<Cell<u64>>,
    tx: &async_channel::Sender<ColorReq>,
) {
    let bs = selection.selection();
    let first = match gtk::BitsetIter::init_first(&bs) {
        Some((_, idx)) => idx as usize,
        None => {
            req_id.set(req_id.get().wrapping_add(1));
            return;
        }
    };
    let project = state.borrow();
    let Some(page) = project.pages.get(first) else { return };
    let id = req_id.get().wrapping_add(1);
    req_id.set(id);
    let req = ColorReq {
        id,
        path: page.path.clone(),
        rotation: page.rotation as u32,
        crop: page.crop,
        brightness: project.brightness,
        contrast: project.contrast,
    };
    drop(project);
    let _ = tx.send_blocking(req);
}

fn render_preview(req: &ColorReq) -> Option<ColorResult> {
    let pb = gdk_pixbuf::Pixbuf::from_file(&req.path).ok()?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let pb = super::page_item::rotate(&pb, req.rotation);
    let pb = match req.crop {
        Some(c) => {
            let x = (c.x as i32).min(pb.width().saturating_sub(1));
            let y = (c.y as i32).min(pb.height().saturating_sub(1));
            let w = (c.w as i32).min(pb.width() - x).max(1);
            let h = (c.h as i32).min(pb.height() - y).max(1);
            pb.new_subpixbuf(x, y, w, h)
        }
        None => pb,
    };
    let pb = scale_down(pb, 2048);
    let pb = adjust_colors(pb, req.brightness, req.contrast);
    Some(ColorResult {
        id: req.id,
        bytes: pb.read_pixel_bytes(),
        width: pb.width(),
        height: pb.height(),
        rowstride: pb.rowstride(),
        has_alpha: pb.has_alpha(),
    })
}

fn scale_down(pb: gdk_pixbuf::Pixbuf, max_px: i32) -> gdk_pixbuf::Pixbuf {
    if pb.width() <= max_px && pb.height() <= max_px {
        return pb;
    }
    let s = (max_px as f64 / pb.width() as f64).min(max_px as f64 / pb.height() as f64);
    let nw = ((pb.width() as f64 * s).round() as i32).max(1);
    let nh = ((pb.height() as f64 * s).round() as i32).max(1);
    pb.scale_simple(nw, nh, gdk_pixbuf::InterpType::Bilinear).unwrap_or(pb)
}

fn adjust_colors(pb: gdk_pixbuf::Pixbuf, brightness: f32, contrast: f32) -> gdk_pixbuf::Pixbuf {
    if brightness == 0.0 && contrast == 0.0 {
        return pb;
    }
    let c = (contrast + 1.0_f32).max(0.0);
    let b = (brightness * 128.0_f32).round() as i32;
    let channels = if pb.has_alpha() { 4 } else { 3 } as usize;
    let w = pb.width() as usize;
    let h = pb.height() as usize;
    let src_stride = pb.rowstride() as usize;
    let src_bytes = pb.read_pixel_bytes();
    let src = src_bytes.as_ref();
    let dst_stride = w * channels;
    let mut dst = vec![0u8; h * dst_stride];
    for row in 0..h {
        for col in 0..w {
            let si = row * src_stride + col * channels;
            let di = row * dst_stride + col * channels;
            for ch in 0..3_usize {
                let v = ((src[si + ch] as f32 - 128.0) * c + 128.0).round() as i32 + b;
                dst[di + ch] = v.clamp(0, 255) as u8;
            }
            if channels == 4 {
                dst[di + 3] = src[si + 3];
            }
        }
    }
    let bytes = glib::Bytes::from(dst.as_slice());
    gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        pb.colorspace(),
        pb.has_alpha(),
        pb.bits_per_sample(),
        pb.width(),
        pb.height(),
        dst_stride as i32,
    )
}
