use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::super::page_item::PageItem;
use crate::app::State;

pub(crate) fn queue_all_overlays(
    overlays: &Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>,
) {
    let mut list = overlays.borrow_mut();
    list.retain(|w| w.upgrade().is_some());
    for weak in list.iter() {
        if let Some(da) = weak.upgrade() {
            da.queue_draw();
        }
    }
}

pub(crate) fn preset_color_rgb(i: usize) -> (f64, f64, f64) {
    #[rustfmt::skip]
    const C: [(f64, f64, f64); 8] = [
        (0x35_u8 as f64 / 255.0, 0x84_u8 as f64 / 255.0, 0xe4_u8 as f64 / 255.0),
        (0x33_u8 as f64 / 255.0, 0xd1_u8 as f64 / 255.0, 0x7a_u8 as f64 / 255.0),
        (0xff_u8 as f64 / 255.0, 0x78_u8 as f64 / 255.0, 0x00_u8 as f64 / 255.0),
        (0x91_u8 as f64 / 255.0, 0x41_u8 as f64 / 255.0, 0xac_u8 as f64 / 255.0),
        (0xed_u8 as f64 / 255.0, 0x33_u8 as f64 / 255.0, 0x3b_u8 as f64 / 255.0),
        (0x1c_u8 as f64 / 255.0, 0x71_u8 as f64 / 255.0, 0xd8_u8 as f64 / 255.0),
        (0xc0_u8 as f64 / 255.0, 0x61_u8 as f64 / 255.0, 0xcb_u8 as f64 / 255.0),
        (0x98_u8 as f64 / 255.0, 0x6a_u8 as f64 / 255.0, 0x44_u8 as f64 / 255.0),
    ];
    C[i % 8]
}

pub(crate) fn draw_crop_overlay(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    state: &State,
    page_store: &gio::ListStore,
    page_index: usize,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let project = state.borrow();
    let Some(page) = project.pages.get(page_index) else {
        return;
    };
    let Some(crop) = page.crop else { return };

    let cw = crop.w as f64;
    let ch = crop.h as f64;
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }

    let (iw, ih) = match page_store
        .item(page_index as u32)
        .and_downcast::<PageItem>()
    {
        Some(item) => {
            let w = item.base_width() as f64;
            let h = item.base_height() as f64;
            if w > 0.0 && h > 0.0 {
                let rot = page.rotation as u32 % 360;
                if rot == 90 || rot == 270 {
                    (h, w)
                } else {
                    (w, h)
                }
            } else {
                (
                    (crop.x + crop.w).max(1) as f64,
                    (crop.y + crop.h).max(1) as f64,
                )
            }
        }
        None => (
            (crop.x + crop.w).max(1) as f64,
            (crop.y + crop.h).max(1) as f64,
        ),
    };

    let s = (width / iw).min(height / ih);
    let rendered_w = iw * s;
    let rendered_h = ih * s;
    let ox = (width - rendered_w) / 2.0;
    let oy = (height - rendered_h) / 2.0;

    let rx = ox + crop.x as f64 * s;
    let ry = oy + crop.y as f64 * s;
    let rw = cw * s;
    let rh = ch * s;

    let (r, g, b) = match page.crop_preset {
        Some(pi) => preset_color_rgb(pi),
        None => (0.5, 0.5, 0.5),
    };

    cr.set_source_rgba(r, g, b, 0.25);
    cr.rectangle(rx, ry, rw, rh);
    let _ = cr.fill();

    cr.set_source_rgba(r, g, b, 0.8);
    cr.set_line_width(2.0);
    cr.rectangle(rx, ry, rw, rh);
    let _ = cr.stroke();
}

pub(crate) fn update_status_label(lbl: &gtk::Label, state: &State, index: i32) {
    let project = state.borrow();
    let text = if index >= 0 && (index as usize) < project.pages.len() {
        let page = &project.pages[index as usize];
        let mut t = format!("Page {}/{}", index + 1, project.pages.len());
        if let Some(pi) = page.crop_preset {
            if let Some(p) = project.crop_presets.get(pi) {
                let locked = if p.locked { " \u{1f512}" } else { " \u{1f513}" };
                t.push_str(&format!(
                    " | Preset: {} ({}×{}{})",
                    p.name, p.w, p.h, locked
                ));
            }
        }
        if let Some(c) = page.crop {
            t.push_str(&format!(" | Crop: {}×{} @ ({}, {})", c.w, c.h, c.x, c.y));
        }
        t
    } else {
        String::from("No page selected")
    };
    lbl.set_label(&text);
}
