//! The page thumbnail grid shared by every workspace mode: the card factory,
//! the grid widget and its wiring to reordering and crop overlays.

use gtk::glib;
use gtk::prelude::*;

use std::rc::Rc;

use recto_core::Command;

use super::reorder::ReorderDnd;
use super::{Mode, Workspace};
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

/// Factory for the page grid. Each card has a transparent DrawingArea over
/// its thumbnail (the crop step draws the page's crop there).
///
/// `on_setup` runs once per *recycled card widget*, before any item is bound,
/// with the card, its overlay and its list item: the place to attach event
/// controllers and draw functions, which outlive binding. A draw function can
/// find its page through `ListItem::item` / `ListItem::position`.
fn page_grid_factory<S>(on_setup: S) -> gtk::SignalListItemFactory
where
    S: Fn(&gtk::Box, &gtk::DrawingArea, &gtk::ListItem) + 'static,
{
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
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
        on_setup(&card, &da, item);
    });
    factory.connect_bind(|_, list_item| {
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
        bind_props(item, &page, &image, &label);
        // Bound to another page: redraw its crop overlay.
        if let Some(da) = overlay.last_child() {
            da.queue_draw();
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
fn build_grid_scroll(
    selection: &gtk::MultiSelection,
    factory: &gtk::SignalListItemFactory,
) -> (gtk::GridView, gtk::ScrolledWindow) {
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
    // No horizontal scrolling: the scroller's minimum width becomes the
    // grid's, i.e. one full column, so the panel can't be squeezed below it.
    let scroll = gtk::ScrolledWindow::builder()
        .child(&grid_view)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .build();
    (grid_view, scroll)
}

impl Workspace {
    /// The page thumbnail grid, shared by all modes. Reordering by drag
    /// belongs to Arrange, so drags are gated on the mode.
    pub(super) fn build_grid(self: &Rc<Self>) -> gtk::ScrolledWindow {
        let reorder_dnd = ReorderDnd {
            enabled: Rc::new({
                let mode = self.mode.clone();
                move || mode.get() == Mode::Arrange
            }),
            selection: self.selection.clone(),
            on_move: Rc::new({
                let state = self.state.clone();
                move |indices, before| state.dispatch(Command::MovePages { indices, before })
            }),
        };
        let factory = page_grid_factory({
            let dnd = reorder_dnd.clone();
            let crop = Rc::downgrade(&self.crop);
            move |card, overlay, list_item| {
                dnd.install_card(card, list_item);
                if let Some(crop) = crop.upgrade() {
                    crop.attach_thumb_overlay(overlay, list_item);
                }
            }
        });
        let (grid_view, grid_scroll) = build_grid_scroll(&self.selection, &factory);
        reorder_dnd.install_grid(&grid_view, &grid_scroll);
        reorder_dnd.install_background(&grid_scroll);

        // A click on empty grid space clears the selection.
        let click = gtk::GestureClick::new();
        let selection = self.selection.clone();
        click.connect_pressed(move |gesture, n_press, x, y| {
            let on_background = gesture
                .widget()
                .and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT))
                .is_some_and(|target| {
                    target.is::<gtk::GridView>()
                        || target.is::<gtk::Viewport>()
                        || target.is::<gtk::ScrolledWindow>()
                });
            if n_press == 1 && on_background {
                selection.unselect_all();
            }
        });
        grid_scroll.add_controller(click);
        grid_scroll
    }
}
