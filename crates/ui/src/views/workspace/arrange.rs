//! Arrange mode: add, rotate, reorder and delete pages, with a preview of the
//! first selected page.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::app::State;
use crate::latest::{PreviewService, RequestDedup};
use crate::pixbuf::PixelData;
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::ZoomPanController;
use recto_core::{Command, Rotation};

use super::helpers::{make_preview, make_section};
use super::store::selected_positions;

/// Where a reorder action sends the selected pages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MoveTarget {
    Start,
    Back,
    Forward,
    End,
}

/// The reorder buttons drive `gio::SimpleAction`s rather than click handlers,
/// so the sidebar, the keyboard shortcuts, and the button sensitivity all
/// share one definition.
const REORDER_BUTTONS: [(&str, &str, &str, MoveTarget); 4] = [
    (
        "go-first-symbolic",
        "move-start",
        "Move to start (Ctrl+Shift+Home)",
        MoveTarget::Start,
    ),
    (
        "go-previous-symbolic",
        "move-back",
        "Move back (Ctrl+Shift+Left)",
        MoveTarget::Back,
    ),
    (
        "go-next-symbolic",
        "move-forward",
        "Move forward (Ctrl+Shift+Right)",
        MoveTarget::Forward,
    ),
    (
        "go-last-symbolic",
        "move-end",
        "Move to end (Ctrl+Shift+End)",
        MoveTarget::End,
    ),
];

/// Name of the action group the reorder actions are published under.
pub const REORDER_GROUP: &str = "arrange";

pub(super) struct ArrangeView {
    state: State,
    selection: gtk::MultiSelection,
    pub(super) controls: gtk::Box,
    pub(super) preview: PreviewCanvas,
    zoom: Rc<ZoomPanController>,
    preview_req: RequestDedup<(PathBuf, u32)>,
    /// Path and rotation of the page currently previewed (or loading).
    shown: RefCell<Option<(PathBuf, u32)>>,
    /// Buttons that need a selection: rotate ×3 and delete.
    selection_buttons: Vec<gtk::Button>,
    /// Reorder actions, published by the workspace under [`REORDER_GROUP`].
    pub(super) actions: gio::SimpleActionGroup,
    reorder: Vec<(gio::SimpleAction, MoveTarget)>,
}

impl ArrangeView {
    pub(super) fn new(
        state: State,
        selection: gtk::MultiSelection,
        spinner: &gtk::Spinner,
        count: &gtk::Label,
        load_images: Rc<dyn Fn(Vec<PathBuf>)>,
    ) -> Rc<Self> {
        let (preview, zoom) = make_preview();
        let (preview_svc, preview_req, preview_rx) =
            PreviewService::<(PathBuf, u32), PixelData>::new(|_id, (path, rotation)| {
                crate::pixbuf::load_page(&path, rotation, None)
                    .map(|pb| PixelData::from_pixbuf(&pb))
            });

        let add_btn = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("list-add-symbolic")
                    .label("Add Images")
                    .build(),
            )
            .css_classes(["suggested-action", "pill"])
            .build();

        let rotate_ccw = icon_button(
            "object-rotate-left-symbolic",
            "Rotate 90° counter-clockwise",
        );
        let rotate_180 = icon_button("object-flip-vertical-symbolic", "Rotate 180°");
        let rotate_cw = icon_button("object-rotate-right-symbolic", "Rotate 90° clockwise");
        let rotate_box = linked_row();
        rotate_box.append(&rotate_ccw);
        rotate_box.append(&rotate_180);
        rotate_box.append(&rotate_cw);

        let actions = gio::SimpleActionGroup::new();
        let reorder_box = linked_row();
        for (icon, action, tooltip, _) in REORDER_BUTTONS {
            reorder_box.append(
                &gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(tooltip)
                    .action_name(format!("{REORDER_GROUP}.{action}"))
                    .build(),
            );
        }

        let delete_btn = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("user-trash-symbolic")
                    .label("Delete")
                    .build(),
            )
            .tooltip_text("Remove selected pages")
            .css_classes(["destructive-action", "flat"])
            .sensitive(false)
            .build();

        let status_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        status_box.append(spinner);
        status_box.append(count);

        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(10)
            .margin_end(10)
            .build();
        controls.append(&add_btn);
        controls.append(&make_section("Rotate", &rotate_box));
        controls.append(&make_section("Reorder", &reorder_box));
        controls.append(&delete_btn);
        controls.append(&gtk::Box::builder().vexpand(true).build());
        controls.append(&status_box);

        let view = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let reorder = REORDER_BUTTONS
                .iter()
                .map(|&(_, name, _, target)| {
                    let action = gio::SimpleAction::new(name, None);
                    action.set_enabled(false);
                    let weak = weak.clone();
                    action.connect_activate(move |_, _| {
                        if let Some(view) = weak.upgrade() {
                            view.move_selection(target);
                        }
                    });
                    actions.add_action(&action);
                    (action, target)
                })
                .collect();
            Self {
                state,
                selection,
                controls,
                preview,
                zoom,
                preview_req,
                shown: RefCell::new(None),
                selection_buttons: vec![
                    rotate_ccw.clone(),
                    rotate_180.clone(),
                    rotate_cw.clone(),
                    delete_btn.clone(),
                ],
                actions,
                reorder,
            }
        });

        add_btn.connect_clicked(move |btn| choose_images(btn, load_images.clone()));
        for (btn, delta) in [(&rotate_ccw, -90), (&rotate_180, 180), (&rotate_cw, 90)] {
            let weak = Rc::downgrade(&view);
            btn.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.rotate_selection(delta);
                }
            });
        }
        delete_btn.connect_clicked({
            let weak = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.delete_selection();
                }
            }
        });

        glib::spawn_future_local({
            let weak = Rc::downgrade(&view);
            async move {
                let _svc = preview_svc;
                while let Ok((id, pixels)) = preview_rx.recv().await {
                    let Some(view) = weak.upgrade() else { break };
                    if view.preview_req.is_current(id) {
                        view.preview.set_texture(Some(pixels.to_texture()));
                        view.zoom.refit_after_texture_change();
                    }
                }
            }
        });

        view
    }

    /// Preview the first selected page, decoding only when it changed.
    pub(super) fn refresh(&self) {
        let first = selected_positions(&self.selection).first().and_then(|&i| {
            let project = self.state.project();
            let page = project.pages.get(i)?;
            Some((page.path.clone(), page.rotation.as_degrees() as u32))
        });
        if *self.shown.borrow() == first {
            return;
        }
        self.shown.replace(first.clone());
        match first {
            Some(source) => {
                self.preview.set_texture(None);
                self.preview_req.send(source);
            }
            None => self.clear(),
        }
    }

    pub(super) fn clear(&self) {
        self.shown.replace(None);
        self.preview_req.send_dummy();
        self.preview.set_texture(None);
    }

    /// Enable the controls that need a selection, and each reorder action
    /// only when it would actually move something.
    pub(super) fn sync_actions(&self) {
        let positions = selected_positions(&self.selection);
        let page_count = self.selection.n_items() as usize;
        for btn in &self.selection_buttons {
            btn.set_sensitive(!positions.is_empty());
        }
        let (back, forward) = match (positions.first(), positions.last()) {
            (Some(&first), Some(&last)) => (first > 0, last + 1 < page_count),
            _ => (false, false),
        };
        for (action, target) in &self.reorder {
            action.set_enabled(match target {
                MoveTarget::Start | MoveTarget::Back => back,
                MoveTarget::Forward | MoveTarget::End => forward,
            });
        }
    }

    fn rotate_selection(&self, delta: i32) {
        let positions = selected_positions(&self.selection);
        self.state.batch(|| {
            for pos in positions {
                let rotation = {
                    let project = self.state.project();
                    let Some(page) = project.pages.get(pos) else {
                        continue;
                    };
                    Rotation::new((page.rotation.as_degrees() as i32 + delta).rem_euclid(360) as u16)
                };
                self.state.dispatch(Command::SetRotation {
                    index: pos,
                    rotation,
                });
            }
        });
    }

    /// Move the selected pages as one contiguous block, as a single
    /// [`Command::MovePages`] (one undo step). The selection follows the
    /// pages, see `store::reorder_pages`.
    fn move_selection(&self, target: MoveTarget) {
        let indices = selected_positions(&self.selection);
        let (Some(&first), Some(&last)) = (indices.first(), indices.last()) else {
            return;
        };
        let len = self.state.project().pages.len();
        let before = match target {
            MoveTarget::Start => 0,
            MoveTarget::Back => first.saturating_sub(1),
            // Past the page after the last selected one, so the block ends up
            // one position further along.
            MoveTarget::Forward => (last + 2).min(len),
            MoveTarget::End => len,
        };
        self.state.dispatch(Command::MovePages { indices, before });
    }

    fn delete_selection(&self) {
        let indices = selected_positions(&self.selection);
        if indices.is_empty() {
            return;
        }
        let n = indices.len();
        self.state.dispatch(Command::RemovePages(indices));
        crate::window::show_toast(&format!(
            "Deleted {n} page{}",
            if n == 1 { "" } else { "s" }
        ));
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .sensitive(false)
        .build()
}

fn linked_row() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .homogeneous(true)
        .css_classes(["linked"])
        .build()
}

fn choose_images(btn: &gtk::Button, load_images: Rc<dyn Fn(Vec<PathBuf>)>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    for mime in [
        "image/jpeg",
        "image/png",
        "image/tiff",
        "image/webp",
        "image/bmp",
    ] {
        filter.add_mime_type(mime);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Add page images")
        .modal(true)
        .filters(&filters)
        .build();
    let parent = btn.root().and_downcast::<gtk::Window>();
    dialog.open_multiple(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(files) = result else { return };
        let paths = (0..files.n_items())
            .filter_map(|i| files.item(i).and_downcast::<gio::File>())
            .filter_map(|f| f.path())
            .collect();
        load_images(paths);
    });
}
