//! The main work view: a sidebar with the mode switch (Arrange / Crop /
//! Color) and the active mode's controls, the active mode's preview, and the
//! page thumbnail grid.
//!
//! The modes are not tabs. The grid, its split position and the selection
//! are single shared widgets owned by [`Workspace`], identical in every mode;
//! only the sidebar controls and the preview switch. Each mode is a view
//! struct ([`ArrangeView`], [`CropView`], [`ColorView`]) that the workspace
//! forwards selection changes and project events to.

mod arrange;
mod color;
mod crop;
mod css;
mod helpers;
mod store;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::State;
use crate::widgets::grid_reorder::ReorderDnd;
use crate::widgets::page_item::PageItem;
use crate::widgets::thumbnail_loader::{LoadMsg, ThumbReq, ThumbnailService};
use recto_core::{AppEvent, Command};

use arrange::ArrangeView;
pub use arrange::REORDER_GROUP;
use color::ColorView;
use crop::CropView;
use helpers::make_mode_button;

/// Width the thumbnail grid starts at: about two columns.
const INITIAL_GRID_WIDTH: i32 = 420;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Arrange,
    Crop,
    Color,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Arrange => "arrange",
            Mode::Crop => "crop",
            Mode::Color => "color",
        }
    }
}

struct Workspace {
    state: State,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    /// Shared with the grid's drag-and-drop, which only reorders in Arrange.
    mode: Rc<Cell<Mode>>,
    controls_stack: gtk::Stack,
    preview_stack: gtk::Stack,
    /// Shows the empty-project status page instead of the previews.
    content_stack: gtk::Stack,
    arrange: Rc<ArrangeView>,
    crop: Rc<CropView>,
    color: Rc<ColorView>,
    thumbs: ThumbnailService,
    pending_thumbs: Cell<usize>,
    spinner: gtk::Spinner,
    count: gtk::Label,
}

/// Build the workspace and return its root widget. The workspace lives as
/// long as the project event channel, which its event loop owns it through.
pub fn build(
    state: State,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    spinner: gtk::Spinner,
    count: gtk::Label,
    load_images: Rc<dyn Fn(Vec<PathBuf>)>,
    event_rx: async_channel::Receiver<AppEvent>,
) -> gtk::Widget {
    css::load_sidebar_css();
    css::load_preset_css();

    let arrange = ArrangeView::new(
        state.clone(),
        selection.clone(),
        &spinner,
        &count,
        load_images.clone(),
    );
    let crop = CropView::new(state.clone(), selection.clone());
    let color = ColorView::new(state.clone(), selection.clone());
    let (thumbs, thumb_rx) = ThumbnailService::new();

    // ---- Sidebar: mode switch + per-mode controls ---------------------------
    let btn_arrange = make_mode_button("view-grid-symbolic", "Arrange", None);
    btn_arrange.set_active(true);
    let btn_crop = make_mode_button("edit-cut-symbolic", "Crop", Some(&btn_arrange));
    let btn_color = make_mode_button("preferences-color-symbolic", "Color", Some(&btn_arrange));
    let mode_buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    for btn in [&btn_arrange, &btn_crop, &btn_color] {
        mode_buttons.append(btn);
    }

    let controls_stack = gtk::Stack::builder().vexpand(true).build();
    controls_stack.add_named(&arrange.controls, Some(Mode::Arrange.name()));
    controls_stack.add_named(&crop.controls, Some(Mode::Crop.name()));
    controls_stack.add_named(&color.controls, Some(Mode::Color.name()));

    let sidebar = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(180)
        // Children like the colour sliders expand; without this their
        // expansion propagates up and the sidebar takes a share of all extra
        // window width.
        .hexpand(false)
        .css_classes(["sidebar-pane"])
        .build();
    sidebar.append(&mode_buttons);
    sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    sidebar.append(&controls_stack);

    // ---- Preview area -------------------------------------------------------
    let preview_stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    preview_stack.add_named(&arrange.preview, Some(Mode::Arrange.name()));
    preview_stack.add_named(crop.picker.overlay(), Some(Mode::Crop.name()));
    preview_stack.add_named(&color.preview, Some(Mode::Color.name()));

    let content_stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        // Keep the preview usable; once it hits this width, further window
        // shrinking comes out of the thumbnail grid (down to one column).
        .width_request(400)
        .build();
    content_stack.add_named(&preview_stack, Some("preview"));
    content_stack.add_named(
        &adw::StatusPage::builder()
            .icon_name("image-missing-symbolic")
            .title("No images")
            .description("Add images or open a project to get started")
            .build(),
        Some("empty"),
    );
    content_stack.set_visible_child_name("empty");

    let ws = Rc::new(Workspace {
        state,
        store,
        selection,
        mode: Rc::new(Cell::new(Mode::Arrange)),
        controls_stack,
        preview_stack,
        content_stack,
        arrange,
        crop,
        color,
        thumbs,
        pending_thumbs: Cell::new(0),
        spinner,
        count,
    });

    // ---- Grid ---------------------------------------------------------------
    let grid_scroll = ws.build_grid();

    // ---- Layout -------------------------------------------------------------
    let left_panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    left_panel.append(&sidebar);
    left_panel.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    left_panel.append(&ws.content_stack);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left_panel)
        .end_child(&grid_scroll)
        // Window resizes go to the preview; the thumbnail grid keeps the
        // width set by dragging the handle, down to one column.
        .resize_start_child(true)
        .resize_end_child(false)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .vexpand(true)
        .build();
    // Start the grid at about two columns, whatever the window size: the
    // position is measured from the left, so it can only be set once the
    // paned knows its width. Later window resizes go to the preview.
    let initial_split = Cell::new(false);
    paned.connect_max_position_notify(move |paned| {
        if paned.width() > INITIAL_GRID_WIDTH && !initial_split.replace(true) {
            paned.set_position(paned.width() - INITIAL_GRID_WIDTH);
        }
    });
    // Published here so the window's keyboard shortcuts reach them.
    paned.insert_action_group(REORDER_GROUP, Some(&ws.arrange.actions));
    paned.add_controller(file_drop_target(load_images));

    // ---- Wiring -------------------------------------------------------------
    for (btn, mode) in [
        (&btn_arrange, Mode::Arrange),
        (&btn_crop, Mode::Crop),
        (&btn_color, Mode::Color),
    ] {
        let weak = Rc::downgrade(&ws);
        btn.connect_toggled(move |btn| {
            if let Some(ws) = weak.upgrade().filter(|_| btn.is_active()) {
                ws.set_mode(mode);
            }
        });
    }

    ws.selection.connect_selection_changed({
        let weak = Rc::downgrade(&ws);
        move |_, _, _| {
            if let Some(ws) = weak.upgrade() {
                ws.arrange.sync_actions();
                ws.refresh_active();
            }
        }
    });
    // Whether a page can move also depends on the page count, which a
    // selection change alone does not cover.
    ws.selection.connect_items_changed({
        let weak = Rc::downgrade(&ws);
        move |_, _, _, _| {
            if let Some(ws) = weak.upgrade() {
                ws.arrange.sync_actions();
            }
        }
    });

    glib::spawn_future_local({
        let weak = Rc::downgrade(&ws);
        async move {
            while let Ok(msg) = thumb_rx.recv().await {
                let Some(ws) = weak.upgrade() else { break };
                ws.on_thumbnail(msg);
            }
        }
    });

    // The event loop owns the workspace: it lives as long as the state.
    glib::spawn_future_local({
        let ws = ws.clone();
        async move {
            while let Ok(event) = event_rx.recv().await {
                ws.on_event(&event);
            }
        }
    });

    paned.upcast()
}

impl Workspace {
    fn set_mode(&self, mode: Mode) {
        self.mode.set(mode);
        self.controls_stack.set_visible_child_name(mode.name());
        self.preview_stack.set_visible_child_name(mode.name());
        if mode == Mode::Crop {
            self.crop.activate();
        } else {
            self.crop.deactivate();
            self.refresh_active();
        }
    }

    /// Bring the active mode's view up to date with the project and the
    /// selection.
    fn refresh_active(&self) {
        match self.mode.get() {
            Mode::Arrange => self.arrange.refresh(),
            Mode::Crop => self.crop.refresh(),
            Mode::Color => self.color.refresh(),
        }
    }

    /// Project the event onto the page store and the views.
    fn on_event(&self, event: &AppEvent) {
        let missing = match event {
            AppEvent::PagesAdded(indices) => store::add_pages(&self.store, &self.state, indices),
            AppEvent::PagesRemoved(indices) => {
                store::remove_pages(&self.store, indices);
                Vec::new()
            }
            AppEvent::PagesReordered(permutation) => {
                store::reorder_pages(&self.store, &self.state, &self.selection, permutation)
            }
            AppEvent::PageChanged(idx) => {
                store::sync_page(&self.store, &self.state, *idx);
                Vec::new()
            }
            AppEvent::ProjectLoaded => {
                let all: Vec<usize> = (0..self.state.project().pages.len()).collect();
                store::add_pages(&self.store, &self.state, &all)
            }
            AppEvent::ProjectCleared => {
                self.store.remove_all();
                self.selection.unselect_all();
                self.arrange.clear();
                self.color.clear();
                Vec::new()
            }
            // Undo/redo can change the project arbitrarily.
            AppEvent::ProjectChanged => store::reconcile(&self.store, &self.state),
            AppEvent::GlobalSettingsChanged | AppEvent::PresetsChanged => Vec::new(),
        };
        self.load_thumbnails(missing);

        if matches!(
            event,
            AppEvent::ProjectLoaded | AppEvent::ProjectCleared | AppEvent::ProjectChanged
        ) {
            self.crop.sync_from_project();
            self.color.sync_from_project();
        }
        let n = self.state.project().pages.len();
        self.count.set_label(&format!("{n} images"));
        self.content_stack
            .set_visible_child_name(if n == 0 { "empty" } else { "preview" });
        self.refresh_active();
    }

    fn load_thumbnails(&self, requests: Vec<ThumbReq>) {
        if requests.is_empty() {
            return;
        }
        self.pending_thumbs
            .set(self.pending_thumbs.get() + requests.len());
        self.spinner.set_visible(true);
        self.spinner.start();
        self.thumbs.enqueue_batch(requests);
    }

    fn on_thumbnail(&self, msg: LoadMsg) {
        match msg {
            LoadMsg::Progress(id, data) => {
                let item = (0..self.store.n_items())
                    .filter_map(|i| self.store.item(i).and_downcast::<PageItem>())
                    .find(|item| item.stable_id() == id);
                if let Some(item) = item {
                    item.set_image(data.to_pixbuf());
                    item.set_dims(data.orig_width, data.orig_height);
                }
            }
            LoadMsg::Finished => {
                let left = self.pending_thumbs.get().saturating_sub(1);
                self.pending_thumbs.set(left);
                if left == 0 {
                    self.spinner.stop();
                    self.spinner.set_visible(false);
                }
            }
        }
    }

    /// The page thumbnail grid, shared by all modes. Reordering by drag
    /// belongs to Arrange, so drags are gated on the mode.
    fn build_grid(self: &Rc<Self>) -> gtk::ScrolledWindow {
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
        let factory = crate::widgets::thumbnail_grid::page_grid_factory({
            let dnd = reorder_dnd.clone();
            let crop = Rc::downgrade(&self.crop);
            move |card, overlay, list_item| {
                dnd.install_card(card, list_item);
                if let Some(crop) = crop.upgrade() {
                    crop.attach_thumb_overlay(overlay, list_item);
                }
            }
        });
        let (grid_view, grid_scroll) =
            crate::widgets::thumbnail_grid::build_grid_scroll(&self.selection, &factory);
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

/// Dropping image files anywhere on the workspace adds them as pages.
fn file_drop_target(load_images: Rc<dyn Fn(Vec<PathBuf>)>) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    target.connect_drop(move |_, value, _, _| {
        let Ok(files) = value.get::<gdk::FileList>() else {
            return false;
        };
        load_images(files.files().iter().filter_map(|f| f.path()).collect());
        true
    });
    target
}
