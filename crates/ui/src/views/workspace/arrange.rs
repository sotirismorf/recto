use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::State;
use crate::views::workspace::helpers::{
    can_move, move_selection, rotate_selected, selected_positions, MoveTarget,
};
use recto_core::Command;

use super::helpers::make_section;

pub struct ArrangeSidebar {
    pub controls: gtk::Box,
    pub add_btn: gtk::Button,
    pub rotate_ccw: gtk::Button,
    pub rotate_cw: gtk::Button,
    pub rotate_180: gtk::Button,
    pub delete_btn: gtk::Button,
}

/// The reorder buttons drive `gio::SimpleAction`s rather than click handlers,
/// so the sidebar, the keyboard shortcuts, and the button sensitivity all
/// share one definition. See [`wire_reorder_actions`].
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

pub fn build_arrange_sidebar(spinner: &gtk::Spinner, count: &gtk::Label) -> ArrangeSidebar {
    let add_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label("Add Images")
                .build(),
        )
        .build();
    add_btn.add_css_class("suggested-action");
    add_btn.add_css_class("pill");

    let rotate_ccw = gtk::Button::builder()
        .icon_name("object-rotate-left-symbolic")
        .tooltip_text("Rotate 90° counter-clockwise")
        .sensitive(false)
        .build();
    let rotate_180 = gtk::Button::builder()
        .icon_name("object-flip-vertical-symbolic")
        .tooltip_text("Rotate 180°")
        .sensitive(false)
        .build();
    let rotate_cw = gtk::Button::builder()
        .icon_name("object-rotate-right-symbolic")
        .tooltip_text("Rotate 90° clockwise")
        .sensitive(false)
        .build();

    let rotate_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .homogeneous(true)
        .build();
    rotate_box.add_css_class("linked");
    rotate_box.append(&rotate_ccw);
    rotate_box.append(&rotate_180);
    rotate_box.append(&rotate_cw);

    let rotate_section = make_section("Rotate", &rotate_box);

    let reorder_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .homogeneous(true)
        .build();
    reorder_box.add_css_class("linked");
    for (icon, action, tooltip, _) in REORDER_BUTTONS {
        let btn = gtk::Button::builder()
            .icon_name(icon)
            .tooltip_text(tooltip)
            .action_name(format!("{REORDER_GROUP}.{action}"))
            .build();
        reorder_box.append(&btn);
    }

    let reorder_section = make_section("Reorder", &reorder_box);

    let delete_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("user-trash-symbolic")
                .label("Delete")
                .build(),
        )
        .tooltip_text("Remove selected pages")
        .sensitive(false)
        .build();
    delete_btn.add_css_class("destructive-action");
    delete_btn.add_css_class("flat");

    let arrange_spacer = gtk::Box::builder().vexpand(true).build();

    let status_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    status_box.append(spinner);
    status_box.append(count);

    let arrange_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    arrange_controls.append(&add_btn);
    arrange_controls.append(&rotate_section);
    arrange_controls.append(&reorder_section);
    arrange_controls.append(&delete_btn);
    arrange_controls.append(&arrange_spacer);
    arrange_controls.append(&status_box);

    ArrangeSidebar {
        controls: arrange_controls,
        add_btn,
        rotate_ccw,
        rotate_cw,
        rotate_180,
        delete_btn,
    }
}

/// The reorder actions, held so their sensitivity can be refreshed whenever
/// the selection changes.
pub struct ReorderActions {
    actions: Vec<(gio::SimpleAction, MoveTarget)>,
}

impl ReorderActions {
    /// Enable each action only when it would actually move something: the
    /// backwards actions need a selection that is not already at the start,
    /// the forwards ones a selection not already at the end.
    pub fn sync(&self, sel: &gtk::MultiSelection, page_count: usize) {
        let (back, forward) = can_move(sel, page_count);
        for (action, target) in &self.actions {
            action.set_enabled(match target {
                MoveTarget::Start | MoveTarget::Back => back,
                MoveTarget::Forward | MoveTarget::End => forward,
            });
        }
    }
}

/// Publish the reorder actions on `widget` under [`REORDER_GROUP`], where both
/// the sidebar buttons and the window's keyboard shortcuts can reach them.
pub fn wire_reorder_actions(
    widget: &impl IsA<gtk::Widget>,
    state: State,
    selection: gtk::MultiSelection,
) -> ReorderActions {
    let group = gio::SimpleActionGroup::new();
    let actions = REORDER_BUTTONS
        .iter()
        .map(|&(_, name, _, target)| {
            let action = gio::SimpleAction::new(name, None);
            action.set_enabled(false);
            action.connect_activate(glib::clone!(
                #[strong]
                state,
                #[strong]
                selection,
                move |_, _| move_selection(&selection, &state, target)
            ));
            group.add_action(&action);
            (action, target)
        })
        .collect();
    widget
        .as_ref()
        .insert_action_group(REORDER_GROUP, Some(&group));
    ReorderActions { actions }
}

/// Wire the arrange-mode signal handlers.
pub fn wire_arrange_handlers(
    sidebar: &ArrangeSidebar,
    state: State,
    selection: gtk::MultiSelection,
    load_images: Rc<dyn Fn(Vec<std::path::PathBuf>)>,
    paned: gtk::Paned,
) {
    let s = state;
    let sel = selection;

    sidebar.add_btn.connect_clicked(glib::clone!(
        #[strong]
        load_images,
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Add page images")
                .modal(true)
                .build();
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
            dialog.set_filters(Some(&filters));
            let parent = btn.root().and_downcast::<gtk::Window>();
            dialog.open_multiple(
                parent.as_ref(),
                gio::Cancellable::NONE,
                glib::clone!(
                    #[strong]
                    load_images,
                    move |result| {
                        let Ok(files) = result else { return };
                        let mut paths = Vec::new();
                        for i in 0..files.n_items() {
                            let Some(file) = files.item(i).and_downcast::<gio::File>() else {
                                continue;
                            };
                            if let Some(path) = file.path() {
                                paths.push(path);
                            }
                        }
                        load_images(paths);
                    }
                ),
            );
        }
    ));

    let drop_target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop_target.connect_drop(glib::clone!(
        #[strong]
        load_images,
        move |_, value, _, _| {
            let Ok(file_list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let paths: Vec<_> = file_list.files().iter().filter_map(|f| f.path()).collect();
            load_images(paths);
            true
        }
    ));
    paned.add_controller(drop_target);

    sidebar.rotate_ccw.connect_clicked(glib::clone!(
        #[strong]
        s,
        #[strong]
        sel,
        move |_| rotate_selected(&sel, &s, -90)
    ));

    sidebar.rotate_cw.connect_clicked(glib::clone!(
        #[strong]
        s,
        #[strong]
        sel,
        move |_| rotate_selected(&sel, &s, 90)
    ));

    sidebar.rotate_180.connect_clicked(glib::clone!(
        #[strong]
        s,
        #[strong]
        sel,
        move |_| rotate_selected(&sel, &s, 180)
    ));

    sidebar.delete_btn.connect_clicked(glib::clone!(
        #[strong]
        s,
        #[strong]
        sel,
        move |_| {
            let positions = selected_positions(&sel);
            if positions.is_empty() {
                return;
            }
            let indices: Vec<usize> = positions.iter().map(|&p| p as usize).collect();
            let n = indices.len();
            s.dispatch(Command::RemovePages(indices));
            crate::window::show_toast(&format!(
                "Deleted {} page{}",
                n,
                if n == 1 { "" } else { "s" }
            ));
        }
    ));
}
