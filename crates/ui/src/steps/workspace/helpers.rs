use std::cell::Cell;
use std::path::PathBuf;

use adw::prelude::*;
use gtk::gio;

use crate::app::State;
use crate::widgets::preview_canvas::PreviewCanvas;

use super::super::page_item::PageItem;

pub(crate) fn update_arrange_preview(
    sel: &gtk::MultiSelection,
    preview: &PreviewCanvas,
    preview_req_id: &Cell<u64>,
    tx_prev: &async_channel::Sender<(u64, PathBuf, u32)>,
) {
    let positions = selected_positions(sel);
    if let Some(&first) = positions.first() {
        if let Some(page) = sel.item(first).and_downcast::<PageItem>() {
            let id = preview_req_id.get() + 1;
            preview_req_id.set(id);
            let _ = tx_prev.send_blocking((id, page.path(), page.rotation()));
            return;
        }
    }
    preview_req_id.set(preview_req_id.get() + 1);
    preview.set_texture(None);
}

pub(crate) fn rotate_selected(
    sel: &gtk::MultiSelection,
    store: &gio::ListStore,
    state: &State,
    delta: i32,
) {
    for pos in selected_positions(sel) {
        let Some(page) = store.item(pos).and_downcast::<PageItem>() else {
            continue;
        };
        page.apply_rotation_delta(delta);
        let mut p = state.borrow_mut();
        if let Some(stored) = p.pages.get_mut(pos as usize) {
            stored.rotation = page.rotation() as u16;
        }
    }
}

pub(crate) fn selected_positions(sel: &gtk::MultiSelection) -> Vec<u32> {
    let bs = sel.selection();
    let mut out = Vec::new();
    if let Some((iter, first)) = gtk::BitsetIter::init_first(&bs) {
        out.push(first);
        out.extend(iter);
    }
    out
}

pub(crate) fn make_mode_button(
    icon: &str,
    label: &str,
    group: Option<&gtk::ToggleButton>,
) -> gtk::ToggleButton {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    content.append(
        &gtk::Image::builder()
            .icon_name(icon)
            .pixel_size(16)
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );

    let mut builder = gtk::ToggleButton::builder().child(&content).has_frame(false);
    if let Some(g) = group {
        builder = builder.group(g);
    }
    let btn = builder.build();
    btn.add_css_class("mode-button");
    btn
}

pub(crate) fn make_section(title: &str, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    section.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .css_classes(["heading", "caption"])
            .build(),
    );
    section.append(body);
    section
}
