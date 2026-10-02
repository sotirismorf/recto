//! Small widget builders shared by the workspace modes.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};

/// A zoomable, pannable preview pane (left button pans).
pub(super) fn make_preview() -> (PreviewCanvas, Rc<ZoomPanController>) {
    let preview = PreviewCanvas::new();
    preview.set_hexpand(true);
    preview.set_vexpand(true);
    preview.set_overflow(gtk::Overflow::Hidden);
    preview.add_css_class("view");
    let zoom = ZoomPanController::attach(
        &preview,
        &preview,
        ZoomPanConfig {
            pan_button: gdk::BUTTON_PRIMARY,
            ..Default::default()
        },
    );
    (preview, zoom)
}

pub(super) fn make_mode_button(
    icon: &str,
    label: &str,
    group: Option<&gtk::ToggleButton>,
) -> gtk::ToggleButton {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    content.append(&gtk::Image::builder().icon_name(icon).pixel_size(16).build());
    content.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );

    let mut builder = gtk::ToggleButton::builder()
        .child(&content)
        .has_frame(false);
    if let Some(g) = group {
        builder = builder.group(g);
    }
    let btn = builder.build();
    btn.add_css_class("mode-button");
    btn
}

pub(super) fn make_section(title: &str, body: &impl IsA<gtk::Widget>) -> gtk::Box {
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
