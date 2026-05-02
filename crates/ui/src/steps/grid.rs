//! Shared building blocks for the page-thumbnail grids used by the import,
//! crop, and colors steps. Centralising this keeps card sizing, CSS, and
//! property bindings consistent across the three views.

use std::rc::Rc;
use std::sync::OnceLock;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::page_item::PageItem;

pub const THUMB_PX: i32 = 150;
pub const CARD_CHARS: i32 = 18;

/// One-time install of the photo-grid CSS for cell margin/padding.
pub fn install_grid_css() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let provider = gtk::CssProvider::new();
        provider.load_from_string(
            ".photo-grid > child { margin: 6px; padding: 8px; border-radius: 8px; }",
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

fn make_card() -> (gtk::Box, gtk::Image, gtk::Label) {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .hexpand(false)
        .vexpand(false)
        .build();
    // gtk::Image with pixel_size clamps its natural size to a fixed square —
    // unlike gtk::Picture, whose natural size grows with the paintable. That
    // keeps cells stable and lets only the inter-card gap fluctuate as the
    // grid reflows columns.
    let image = gtk::Image::builder()
        .pixel_size(THUMB_PX)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let label = gtk::Label::builder()
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(CARD_CHARS)
        .width_chars(CARD_CHARS)
        .css_classes(["caption"])
        .build();
    (card, image, label)
}

fn bind_props(item: &gtk::ListItem, page: &PageItem, image: &gtk::Image, label: &gtk::Label) {
    let b1 = page
        .bind_property("thumbnail", image, "paintable")
        .sync_create()
        .build();
    let b2 = page
        .bind_property("filename", label, "label")
        .sync_create()
        .build();
    // SAFETY: keys are unique to this module; bindings are released in
    // unbind_props which GTK guarantees runs exactly once per item.
    unsafe {
        item.set_data("__b_thumb", b1);
        item.set_data("__b_label", b2);
    }
}

fn unbind_props(item: &gtk::ListItem) {
    // SAFETY: keys match those set in bind_props; steal_data hands us
    // ownership of the stored value, so dropping it is safe.
    unsafe {
        if let Some(b) = item.steal_data::<glib::Binding>("__b_thumb") {
            b.unbind();
        }
        if let Some(b) = item.steal_data::<glib::Binding>("__b_label") {
            b.unbind();
        }
    }
}

/// Factory for a plain thumbnail grid (image + filename label). Used by the
/// import and colors steps.
pub fn simple_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("ListItem expected");
        let (card, image, label) = make_card();
        card.append(&image);
        card.append(&label);
        item.set_child(Some(&card));
    });
    factory.connect_bind(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("ListItem expected");
        let page: PageItem = item.item().and_downcast().expect("PageItem expected");
        let card: gtk::Box = item.child().and_downcast().expect("Box card expected");
        let image: gtk::Image = card
            .first_child()
            .and_downcast()
            .expect("first card child must be Image");
        let label: gtk::Label = image
            .next_sibling()
            .and_downcast()
            .expect("second card child must be Label");
        bind_props(item, &page, &image, &label);
    });
    factory.connect_unbind(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("ListItem expected");
        unbind_props(item);
    });
    factory
}

/// Factory for a grid where each thumbnail has a transparent overlay on top
/// (used by the crop step to draw the crop rectangle). The caller's
/// `set_draw` closure is invoked once per bind with the page index and the
/// per-card DrawingArea, and is expected to install a draw_func on it.
pub fn overlay_factory<F>(set_draw: F) -> gtk::SignalListItemFactory
where
    F: Fn(usize, &gtk::DrawingArea) + 'static,
{
    let set_draw = Rc::new(set_draw);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("ListItem expected");
        let (card, image, label) = make_card();
        let overlay = gtk::Overlay::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(false)
            .vexpand(false)
            .build();
        overlay.set_child(Some(&image));
        let da = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        overlay.add_overlay(&da);
        card.append(&overlay);
        card.append(&label);
        item.set_child(Some(&card));
    });
    factory.connect_bind({
        let set_draw = set_draw.clone();
        move |_, list_item| {
            let item = list_item
                .downcast_ref::<gtk::ListItem>()
                .expect("ListItem expected");
            let page: PageItem = item.item().and_downcast().expect("PageItem expected");
            let card: gtk::Box = item.child().and_downcast().expect("Box card expected");
            let overlay: gtk::Overlay = card
                .first_child()
                .and_downcast()
                .expect("first card child must be Overlay");
            let image: gtk::Image = overlay
                .first_child()
                .and_downcast()
                .expect("first overlay child must be Image");
            let label: gtk::Label = overlay
                .next_sibling()
                .and_downcast()
                .expect("second card child must be Label");
            // The only widget added via add_overlay is our DrawingArea, so it
            // is the overlay's last child.
            let da: gtk::DrawingArea = overlay
                .last_child()
                .and_downcast()
                .expect("overlay must have a DrawingArea on top");
            bind_props(item, &page, &image, &label);
            set_draw(item.position() as usize, &da);
        }
    });
    factory.connect_unbind(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("ListItem expected");
        unbind_props(item);
    });
    factory
}

/// Build the GridView + ScrolledWindow used by all three steps. The selection
/// model is supplied by the caller so multiple tabs can share one.
pub fn build_grid_scroll(
    selection: &gtk::MultiSelection,
    factory: &gtk::SignalListItemFactory,
) -> (gtk::GridView, gtk::ScrolledWindow) {
    install_grid_css();
    let grid_view = gtk::GridView::builder()
        .model(selection)
        .factory(factory)
        .min_columns(1)
        .max_columns(8)
        .enable_rubberband(true)
        .vexpand(true)
        .hexpand(true)
        .build();
    grid_view.add_css_class("photo-grid");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&grid_view)
        .vexpand(true)
        .hexpand(true)
        .build();
    (grid_view, scroll)
}
