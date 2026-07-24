use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use adw::prelude::*;
use gtk::gio;

use crate::app::State;
use crate::latest::RequestDedup;
use crate::widgets::page_item::PageItem;
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::thumbnail_loader::ThumbReq;
use recto_core::{Command, Rotation};

/// Rebuild the page store so it matches the project page-for-page, reusing the
/// existing [`PageItem`]s wherever possible.
///
/// This runs after undo/redo, where the project may have been reordered,
/// truncated, or grown in ways no single fine-grained event describes. Items
/// are matched to pages by path: two pages sharing a path are interchangeable
/// here, because a thumbnail is derived purely from the path and rotation is
/// re-synced below.
///
/// Returns a request for every page that had no item to reuse, so the caller
/// can enqueue the missing thumbnails with the thumbnail service.
pub(crate) fn reconcile_store(store: &gio::ListStore, state: &State) -> Vec<ThumbReq> {
    let project = state.project();

    let mut spare: HashMap<PathBuf, VecDeque<PageItem>> = HashMap::new();
    for i in 0..store.n_items() {
        let Some(item) = store.item(i).and_downcast::<PageItem>() else {
            continue;
        };
        spare.entry(item.path()).or_default().push_back(item);
    }

    let mut items: Vec<PageItem> = Vec::with_capacity(project.pages.len());
    let mut missing = Vec::new();
    for page in &project.pages {
        let item = match spare.get_mut(&page.path).and_then(VecDeque::pop_front) {
            Some(item) => item,
            None => {
                let item = PageItem::new_placeholder(page.path.clone());
                missing.push(ThumbReq {
                    id: item.stable_id(),
                    path: page.path.clone(),
                });
                item
            }
        };
        let rotation = page.rotation.as_degrees() as u32;
        if item.rotation() != rotation {
            item.set_rotation_absolute(rotation);
        }
        items.push(item);
    }

    store.splice(0, store.n_items(), &items);
    missing
}

pub(crate) fn update_arrange_preview(
    sel: &gtk::MultiSelection,
    preview: &PreviewCanvas,
    preview_req: &RequestDedup<(PathBuf, u32)>,
) {
    let positions = selected_positions(sel);
    if let Some(&first) = positions.first() {
        if let Some(page) = sel.item(first).and_downcast::<PageItem>() {
            preview_req.send((page.path(), page.rotation()));
            return;
        }
    }
    preview_req.send_dummy();
    preview.set_texture(None);
}

pub(crate) fn rotate_selected(sel: &gtk::MultiSelection, state: &State, delta: i32) {
    for pos in selected_positions(sel) {
        let new_deg = {
            let project = state.project();
            let Some(page) = project.pages.get(pos as usize) else {
                continue;
            };
            (page.rotation.as_degrees() as i32 + delta).rem_euclid(360) as u16
        };
        state.dispatch(Command::SetRotation {
            index: pos as usize,
            rotation: Rotation::new(new_deg),
        });
    }
}

/// Where a [`move_selection`] call sends the selected pages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MoveTarget {
    Start,
    Back,
    Forward,
    End,
}

/// Move the selected pages as one contiguous block.
///
/// The whole gesture is a single [`Command::MovePages`], so it costs exactly
/// one undo step no matter how many pages are selected. The grid's selection
/// follows the pages — see the `PagesReordered` arm of the projector.
pub(crate) fn move_selection(sel: &gtk::MultiSelection, state: &State, target: MoveTarget) {
    let indices: Vec<usize> = selected_positions(sel)
        .into_iter()
        .map(|p| p as usize)
        .collect();
    let (Some(&first), Some(&last)) = (indices.first(), indices.last()) else {
        return;
    };
    let len = state.project().pages.len();

    let before = match target {
        MoveTarget::Start => 0,
        MoveTarget::Back => first.saturating_sub(1),
        // Past the page after the last selected one, so the block ends up
        // one position further along.
        MoveTarget::Forward => (last + 2).min(len),
        MoveTarget::End => len,
    };

    state.dispatch(Command::MovePages { indices, before });
}

/// Whether the selection can still move towards the start / the end. Drives
/// the sensitivity of the reorder buttons and actions.
pub(crate) fn can_move(sel: &gtk::MultiSelection, page_count: usize) -> (bool, bool) {
    let positions = selected_positions(sel);
    let (Some(&first), Some(&last)) = (positions.first(), positions.last()) else {
        return (false, false);
    };
    (first > 0, (last as usize) + 1 < page_count)
}

pub(crate) fn project_page_into_store(store: &gio::ListStore, state: &State, idx: usize) {
    let project = state.project();
    let Some(page) = project.pages.get(idx) else {
        return;
    };
    let Some(item) = store.item(idx as u32).and_downcast::<PageItem>() else {
        return;
    };
    let expected = page.rotation.as_degrees() as u32;
    if item.rotation() != expected {
        item.set_rotation_absolute(expected);
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
