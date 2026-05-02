use std::cell::Cell;
use std::path::PathBuf;

use adw::prelude::*;
use gtk::gio;

use crate::app::State;
use crate::widgets::preview_canvas::PreviewCanvas;
use recto_core::Command;

use crate::widgets::page_item::PageItem;

/// Sync existing page-store items with the project state — only updates
/// metadata (rotation, …) without reloading thumbnails from disk.
pub(crate) fn sync_page_metadata(store: &gio::ListStore, state: &State) {
    let project = state.project();
    let store_len = store.n_items() as usize;
    let project_len = project.pages.len();

    // Update rotation on existing items only when it changed.
    let n = store_len.min(project_len);
    for i in 0..n {
        if let Some(item) = store.item(i as u32).and_downcast::<PageItem>() {
            let expected = project.pages[i].rotation.as_degrees() as u32;
            if item.rotation() != expected {
                item.set_rotation_absolute(expected);
            }
        }
    }
    // Remove excess items (pages were deleted).
    while store.n_items() > project_len as u32 {
        store.remove(store.n_items() - 1);
    }
    // Add placeholders for any new pages.
    while (store.n_items() as usize) < project_len {
        let i = store.n_items() as usize;
        let item = PageItem::new_placeholder(project.pages[i].path.clone());
        item.set_rotation(project.pages[i].rotation.as_degrees() as u32);
        store.append(&item);
    }
}

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
        state.dispatch(Command::SetRotation {
            index: pos as usize,
            rotation: recto_core::Rotation::new(page.rotation() as u16),
        });
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
