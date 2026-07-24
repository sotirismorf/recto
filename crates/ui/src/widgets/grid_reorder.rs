//! Drag-and-drop reordering for the page-thumbnail grid.
//!
//! GTK's `GridView` has no built-in reordering, so this wires it up by hand:
//! a [`gtk::DragSource`] and a [`gtk::DropTarget`] on every card, plus a
//! catch-all drop target on the scrolled window for dropping past the last
//! thumbnail, plus a press-time arbiter on the grid itself that decides
//! whether a drag reorders or rubber-band selects (see
//! [`ReorderDnd::install_grid`]). The grid factory only has to call
//! [`ReorderDnd::install_card`] once per recycled card widget.
//!
//! Everything is expressed in *grid positions*, which the shared
//! [`gtk::MultiSelection`] keeps in lockstep with project page indices, so a
//! drop resolves straight into a `MovePages` command.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::widgets::page_item::PageItem;

/// CSS classes marking where a drop would land. See `install_grid_css`.
const DROP_BEFORE: &str = "drop-before";
const DROP_AFTER: &str = "drop-after";
const DRAG_SOURCE: &str = "dnd-source";

/// Distance from a scroll edge, in pixels, at which a hovering drag starts
/// auto-scrolling the grid.
const AUTOSCROLL_MARGIN: f64 = 48.0;
/// Pixels scrolled per auto-scroll tick.
const AUTOSCROLL_STEP: f64 = 18.0;
const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(16);

/// Longest side of the pointer-following drag icon, in logical pixels — small
/// enough to keep the grid readable while dragging, as file managers do.
const DRAG_ICON_MAX: f64 = 64.0;
/// Corner radius of the drag icon.
const DRAG_ICON_RADIUS: f32 = 6.0;
/// Gap between the pointer and the drag icon's top-left corner.
const DRAG_ICON_OFFSET: i32 = 8;

/// The value carried by a reorder drag. It is never read — the dragged pages
/// are taken from the live selection — but a `GType` is needed to distinguish
/// our own drags from unrelated ones (notably the file drops the arrange view
/// already accepts).
type DragPayload = u64;

/// Wiring for one grid's drag-to-reorder behaviour.
#[derive(Clone)]
pub struct ReorderDnd {
    /// Gate consulted before every drag begins, so reordering can be limited
    /// to the mode that owns it.
    pub enabled: Rc<dyn Fn() -> bool>,
    /// The grid's selection model, and the source of truth for positions.
    pub selection: gtk::MultiSelection,
    /// Called with the positions to move and the insertion point, both in
    /// pre-move coordinates — exactly `Command::MovePages`.
    pub on_move: Rc<dyn Fn(Vec<usize>, usize)>,
}

impl ReorderDnd {
    /// Positions to drag when the gesture starts on `position`: the whole
    /// selection if that row is part of it, otherwise just that row (which
    /// also becomes the new selection, matching how file managers behave).
    fn drag_set(&self, position: u32) -> Vec<usize> {
        let bitset = self.selection.selection();
        if bitset.contains(position) {
            let mut out = Vec::new();
            if let Some((iter, first)) = gtk::BitsetIter::init_first(&bitset) {
                out.push(first as usize);
                out.extend(iter.map(|p| p as usize));
            }
            out
        } else {
            self.selection.select_item(position, true);
            vec![position as usize]
        }
    }

    /// Whether a drag starting on this row should reorder rather than select.
    /// Used both to decide the sequence's owner and to produce the drag's
    /// content, so the two can never disagree.
    fn can_drag(&self, list_item: &gtk::ListItem) -> bool {
        (self.enabled)() && list_item.position() != gtk::INVALID_LIST_POSITION
    }

    /// Decide, at press time, whether the grid may rubberband — the file
    /// manager rule: a drag from a card reorders, a drag from the gap
    /// rubber-band selects. Call once on the grid the cards live in.
    ///
    /// The decision cannot be left to the gestures themselves. GTK targets a
    /// button press at the widget under the *last delivered motion event*, not
    /// at the press coordinates (`handle_pointing_event` in gtkmain.c), and
    /// motion is compressed to frame rate — so a press that lands on a card
    /// while the pointer is still moving is routinely delivered to whatever
    /// the previous frame was over. Any card-side gesture then never sees the
    /// sequence at all, and the grid's rubberband wins by default.
    ///
    /// So the grid's owner decides: a capture-phase handler hit-tests the
    /// press's true coordinates itself. `set_enable_rubberband(false)` removes
    /// the grid's internal rubberband gesture
    /// (`gtk_list_base_set_enable_rubberband`), so flipping it during capture
    /// — before propagation reaches the grid — deterministically keeps that
    /// gesture from ever receiving a press that landed on a card. No
    /// sequence-claiming race is left to lose: the card's drag source is the
    /// only contender. Clicks are untouched — selection belongs to the list
    /// items' own click gestures, not to the rubberband.
    ///
    /// The handler must live on `scroll`, an *ancestor* — never on the grid:
    /// toggling the property mutates the grid's controller list, and doing
    /// that from a controller on the grid itself means mutating the very list
    /// GTK is iterating, freeing an entry under the iterator's feet. From the
    /// ancestor's capture phase the grid's list is only touched between its
    /// own per-event iterations.
    ///
    /// The press handler only ever disables; release/reset re-enable, so the
    /// steady state is "rubberband on" and the next gap press finds the
    /// gesture already installed and eligible for the new sequence.
    pub fn install_grid(&self, grid_view: &gtk::GridView, scroll: &gtk::ScrolledWindow) {
        let press = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        press.connect_pressed(glib::clone!(
            #[strong(rename_to = this)]
            self,
            #[weak]
            grid_view,
            #[weak]
            scroll,
            move |_, _, x, y| {
                if (this.enabled)() && pick_is_card(&scroll, x, y) {
                    grid_view.set_enable_rubberband(false);
                }
            }
        ));
        let restore = glib::clone!(
            #[weak]
            grid_view,
            move || grid_view.set_enable_rubberband(true)
        );
        press.connect_released(glib::clone!(
            #[strong]
            restore,
            move |_, _, _, _| restore()
        ));
        // `stopped` covers every way the sequence can end without a release
        // reaching us (denial, cancellation, grab breaks).
        press.connect_stopped(move |_| restore());
        scroll.add_controller(press);
    }

    /// Attach the drag source and drop target to one card. Call this from the
    /// factory's `setup` handler: the controllers read the row's live position
    /// from `list_item` each time they fire, so recycling needs no rebinding.
    pub fn install_card(&self, card: &gtk::Box, list_item: &gtk::ListItem) {
        let source = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();

        source.connect_prepare(glib::clone!(
            #[strong(rename_to = this)]
            self,
            #[weak]
            card,
            #[weak]
            list_item,
            #[upgrade_or]
            None,
            move |_, x, y| {
                if !this.can_drag(&list_item) {
                    return None;
                }
                // The stale-focus misrouting described on `install_grid` can
                // also deliver a *gap* press to this card. The press point is
                // in card coordinates, so an out-of-bounds start exposes it:
                // decline, and let the rubberband have the sequence.
                if x < 0.0 || y < 0.0 || x > f64::from(card.width()) || y > f64::from(card.height())
                {
                    return None;
                }
                let position = list_item.position();
                // Selecting here (rather than in drag_begin) keeps the grid in
                // the state the drop will act on.
                this.drag_set(position);
                Some(gdk::ContentProvider::for_value(
                    &(position as DragPayload).to_value(),
                ))
            }
        ));

        source.connect_drag_begin(glib::clone!(
            #[weak]
            card,
            #[weak]
            list_item,
            move |source, _| {
                card.add_css_class(DRAG_SOURCE);
                if let Some(icon) = list_item
                    .item()
                    .and_downcast::<PageItem>()
                    .and_then(|page| page.thumbnail())
                    .and_then(|thumb| drag_icon(&thumb))
                {
                    // A negative hotspot hangs the icon below and to the right
                    // of the pointer, so it never hides the drop caret.
                    source.set_icon(Some(&icon), -DRAG_ICON_OFFSET, -DRAG_ICON_OFFSET);
                }
            }
        ));
        let clear_source = glib::clone!(
            #[weak]
            card,
            move || card.remove_css_class(DRAG_SOURCE)
        );
        source.connect_drag_end(glib::clone!(
            #[strong]
            clear_source,
            move |_, _, _| clear_source()
        ));
        source.connect_drag_cancel(move |_, _, _| {
            clear_source();
            false
        });
        card.add_controller(source);

        let target = gtk::DropTarget::new(DragPayload::static_type(), gdk::DragAction::MOVE);

        // Which side of the hovered card the caret is on. Kept per-card so the
        // drop handler agrees with what was last drawn.
        let after: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        target.connect_motion(glib::clone!(
            #[weak]
            card,
            #[strong]
            after,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |_, x, _| {
                let is_after = x > f64::from(card.width()) / 2.0;
                if after.replace(is_after) != is_after || !has_caret(&card) {
                    set_caret(&card, Some(is_after));
                }
                gdk::DragAction::MOVE
            }
        ));
        target.connect_leave(glib::clone!(
            #[weak]
            card,
            move |_| set_caret(&card, None)
        ));
        target.connect_drop(glib::clone!(
            #[strong(rename_to = this)]
            self,
            #[weak]
            card,
            #[weak]
            list_item,
            #[strong]
            after,
            #[upgrade_or]
            false,
            move |_, _, _, _| {
                set_caret(&card, None);
                let position = list_item.position();
                if position == gtk::INVALID_LIST_POSITION {
                    return false;
                }
                let before = position as usize + usize::from(after.get());
                this.drop_at(before)
            }
        ));
        card.add_controller(target);
    }

    /// Accept drops on the empty space around the cards as "move to the end",
    /// and auto-scroll the grid while a drag hovers near its edges.
    pub fn install_background(&self, scroll: &gtk::ScrolledWindow) {
        let target = gtk::DropTarget::new(DragPayload::static_type(), gdk::DragAction::MOVE);
        target.connect_drop(glib::clone!(
            #[strong(rename_to = this)]
            self,
            move |_, _, _, _| {
                let end = this.selection.n_items() as usize;
                this.drop_at(end)
            }
        ));
        scroll.add_controller(target);

        install_autoscroll(scroll);
    }

    /// Turn an insertion point into a move, dropping no-ops on the floor.
    fn drop_at(&self, before: usize) -> bool {
        let indices: Vec<usize> = {
            let bitset = self.selection.selection();
            let mut out = Vec::new();
            if let Some((iter, first)) = gtk::BitsetIter::init_first(&bitset) {
                out.push(first as usize);
                out.extend(iter.map(|p| p as usize));
            }
            out
        };
        if indices.is_empty() {
            return false;
        }
        (self.on_move)(indices, before);
        true
    }
}

/// A small rounded-corner rendition of `thumb` for use as the drag icon,
/// scaled down (never up) so its longest side is [`DRAG_ICON_MAX`].
fn drag_icon(thumb: &gdk::Paintable) -> Option<gdk::Paintable> {
    let (w, h) = (
        f64::from(thumb.intrinsic_width()),
        f64::from(thumb.intrinsic_height()),
    );
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let scale = (DRAG_ICON_MAX / w.max(h)).min(1.0);
    let (w, h) = (w * scale, h * scale);
    let snapshot = gtk::Snapshot::new();
    snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(
        gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32),
        DRAG_ICON_RADIUS,
    ));
    thumb.snapshot(&snapshot, w, h);
    snapshot.pop();
    snapshot.to_paintable(Some(&gtk::graphene::Size::new(w as f32, h as f32)))
}

/// Whether the point (in `root` coordinates) falls on a card — its padding
/// included, its margin not, exactly the highlighted rounded rectangle (see
/// the CSS notes in `thumbnail_grid`). The pick can land on a descendant
/// (image, label), so the card is searched for among ancestors, up to `root`.
fn pick_is_card(root: &impl IsA<gtk::Widget>, x: f64, y: f64) -> bool {
    let root = root.as_ref();
    let mut widget = root.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(w) = widget {
        if w.has_css_class("photo-card") {
            return true;
        }
        if w == *root {
            return false;
        }
        widget = w.parent();
    }
    false
}

fn has_caret(card: &gtk::Box) -> bool {
    card.has_css_class(DROP_BEFORE) || card.has_css_class(DROP_AFTER)
}

/// Draw the insertion caret on `card`, or clear it with `None`.
fn set_caret(card: &gtk::Box, side: Option<bool>) {
    card.remove_css_class(DROP_BEFORE);
    card.remove_css_class(DROP_AFTER);
    match side {
        Some(true) => card.add_css_class(DROP_AFTER),
        Some(false) => card.add_css_class(DROP_BEFORE),
        None => {}
    }
}

/// Scroll the grid while a drag hovers near its top or bottom edge — without
/// it, reordering across a few hundred pages means dropping and re-dragging.
fn install_autoscroll(scroll: &gtk::ScrolledWindow) {
    let motion = gtk::DropControllerMotion::new();
    // Signed pixels-per-tick; zero parks the timer's work.
    let velocity: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
    let running: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    motion.connect_motion(glib::clone!(
        #[weak]
        scroll,
        #[strong]
        velocity,
        #[strong]
        running,
        move |_, _, y| {
            let height = f64::from(scroll.height());
            velocity.set(if y < AUTOSCROLL_MARGIN {
                -AUTOSCROLL_STEP
            } else if y > height - AUTOSCROLL_MARGIN {
                AUTOSCROLL_STEP
            } else {
                0.0
            });

            if velocity.get() == 0.0 || running.replace(true) {
                return;
            }
            glib::timeout_add_local(
                AUTOSCROLL_INTERVAL,
                glib::clone!(
                    #[weak]
                    scroll,
                    #[strong]
                    velocity,
                    #[strong]
                    running,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        let delta = velocity.get();
                        if delta == 0.0 {
                            running.set(false);
                            return glib::ControlFlow::Break;
                        }
                        let adj = scroll.vadjustment();
                        let max = adj.upper() - adj.page_size();
                        adj.set_value((adj.value() + delta).clamp(adj.lower(), max.max(0.0)));
                        glib::ControlFlow::Continue
                    }
                ),
            );
        }
    ));
    motion.connect_leave(glib::clone!(
        #[strong]
        velocity,
        move |_| velocity.set(0.0)
    ));
    scroll.add_controller(motion);
}
