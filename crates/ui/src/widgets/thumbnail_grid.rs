//! Shared building blocks for the page-thumbnail grids used by the import,
//! crop, and colors steps. Centralising this keeps card sizing, CSS, and
//! property bindings consistent across the three views.

use std::rc::Rc;
use std::sync::OnceLock;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::types::PageId;
use crate::widgets::page_item::PageItem;

pub const THUMB_PX: i32 = 150;
pub const CARD_CHARS: i32 = 18;

/// Key used to store the binding vector on each list item via GObject data.
const BINDINGS_KEY: &str = "recto-prop-bindings";

/// Attach two property bindings (thumbnail + filename) from `page` to the
/// card widgets, storing them under `BINDINGS_KEY` on the list item so
/// [`unbind_props`] can find and clean them up.
fn bind_props(item: &gtk::ListItem, page: &PageItem, image: &gtk::Image, label: &gtk::Label) {
    let b1 = page
        .bind_property("thumbnail", image, "paintable")
        .sync_create()
        .build();
    let b2 = page
        .bind_property("filename", label, "label")
        .sync_create()
        .build();
    let bindings = vec![b1, b2];
    unsafe {
        item.set_data(BINDINGS_KEY, bindings);
    }
}

/// Release all property bindings for this list item.
fn unbind_props(item: &gtk::ListItem) {
    unsafe {
        if let Some(bindings) = item.steal_data::<Vec<glib::Binding>>(BINDINGS_KEY) {
            for b in bindings {
                b.unbind();
            }
        }
    }
}

/// One-time install of the photo-grid CSS for cell margin/padding.
pub fn install_grid_css() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let provider = gtk::CssProvider::new();
        // The cell's inner padding belongs to `.photo-card`, not to the `child`
        // node. Together with the card's Fill alignment (see `make_card`) that
        // makes the card's allocation exactly the highlighted rounded
        // rectangle, so every pixel a user reads as "the card" is a pixel the
        // card's own gestures receive. Were the padding left on `child`, its
        // ring would be part of the highlight but belong to the list item,
        // where a press falls through to the grid's rubberband instead of
        // starting a reorder.
        //
        // `margin` stays on `child`: it is outside the border box, so it is
        // excluded from hit-testing and forms the gap that rubberbands.
        provider.load_from_string(
            "\
            .photo-grid > child { margin: 6px; border-radius: 8px; }\
            .photo-card { padding: 8px; }\
            .photo-card.drop-before { box-shadow: inset 3px 0 0 0 @accent_bg_color; }\
            .photo-card.drop-after { box-shadow: inset -3px 0 0 0 @accent_bg_color; }\
            .photo-card.dnd-source { opacity: 0.35; }\
            ",
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

fn make_card() -> (gtk::Box, gtk::Image, gtk::Label) {
    // Fill, not Center/Start: the card must span its whole cell so that its
    // allocation coincides with the highlighted rounded rectangle, making the
    // entire highlight draggable. The children below keep their own centering,
    // so this changes hit-testing rather than layout. Expand stays off — a
    // GridView hands its child the full cell regardless, and expand would only
    // propagate up.
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .hexpand(false)
        .vexpand(false)
        .css_classes(["photo-card"])
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

/// Factory for a grid where each thumbnail has a transparent overlay on top
/// (used by the crop step to draw the crop rectangle). The caller's
/// `set_draw` closure is invoked once per bind with the page's stable_id and
/// the per-card DrawingArea, and is expected to install a draw_func on it.
///
/// `on_setup` runs once per *recycled card widget*, before any item is bound —
/// the place to attach event controllers, which outlive binding.
pub fn overlay_factory<F, S>(set_draw: F, on_setup: S) -> gtk::SignalListItemFactory
where
    F: Fn(PageId, &gtk::DrawingArea) + 'static,
    S: Fn(&gtk::Box, &gtk::ListItem) + 'static,
{
    let set_draw = Rc::new(set_draw);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("ListItem expected");
        let (card, image, label) = make_card();
        on_setup(&card, item);
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
            set_draw(page.stable_id(), &da);
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
