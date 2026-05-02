pub mod colors;
pub mod crop;
pub mod export;
pub mod import;
pub mod page_item;
pub mod start;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::{glib, prelude::WidgetExt, Paned};

/// Minimum right-panel width for one column of images (px).
const MIN_GRID_W: i32 = 175;

struct PanedSyncInner {
    position: Cell<i32>,
    paneds: RefCell<Vec<glib::WeakRef<Paned>>>,
    updating: Cell<bool>,
}

/// Shared paned-position state for the import/crop/colors steps.
/// Cloning this is cheap — all clones refer to the same inner state.
#[derive(Clone)]
pub struct PanedSync(Rc<PanedSyncInner>);

impl PanedSync {
    pub fn new() -> Self {
        Self(Rc::new(PanedSyncInner {
            position: Cell::new(560),
            paneds: RefCell::new(Vec::new()),
            updating: Cell::new(false),
        }))
    }

    /// Register a paned with this sync group: sets its initial position and
    /// keeps it in sync with the other paneds in the group.
    pub fn register(&self, paned: &Paned) {
        paned.set_position(self.0.position.get());
        let weak = glib::WeakRef::new();
        weak.set(Some(paned));
        self.0.paneds.borrow_mut().push(weak);

        paned.connect_position_notify({
            let sync = self.clone();
            move |paned| {
                if sync.0.updating.get() {
                    return;
                }
                sync.0.updating.set(true);

                let pos = paned.position();
                let total_w = paned.width();
                let clamped = if total_w > MIN_GRID_W {
                    pos.min(total_w - MIN_GRID_W)
                } else {
                    pos
                };

                if clamped != pos {
                    paned.set_position(clamped);
                }
                sync.0.position.set(clamped);

                let guard = sync.0.paneds.borrow();
                for weak in guard.iter() {
                    if let Some(other) = weak.upgrade() {
                        if other != *paned {
                            other.set_position(clamped);
                        }
                    }
                }
                drop(guard);
                sync.0.updating.set(false);
            }
        });

        // Re-apply the shared position whenever this tab becomes visible.
        // This catches any case where the position drifted while the paned
        // was unmapped (e.g. the user dragged another tab's divider).
        paned.connect_map({
            let sync = self.clone();
            move |paned| {
                let pos = sync.0.position.get();
                if paned.position() != pos {
                    paned.set_position(pos);
                }
            }
        });
    }
}
