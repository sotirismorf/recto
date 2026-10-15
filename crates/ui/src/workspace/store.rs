//! Keeps the page store (one [`PageItem`] per project page, backing the
//! thumbnail grid) in step with the project.
//!
//! Every function returns thumbnail requests for the pages that need one;
//! the caller hands them to the thumbnail service.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::*;

use crate::app::State;
use crate::background::thumbnails::ThumbReq;
use crate::widgets::page_item::PageId;
use crate::widgets::page_item::PageItem;

/// Grid positions of the selected pages, ascending. Positions equal project
/// page indices because the store mirrors the project.
pub(super) fn selected_positions(sel: &gtk::MultiSelection) -> Vec<usize> {
    let bs = sel.selection();
    let mut out = Vec::new();
    if let Some((iter, first)) = gtk::BitsetIter::init_first(&bs) {
        out.push(first as usize);
        out.extend(iter.map(|p| p as usize));
    }
    out
}

fn placeholder(path: PathBuf, rotation: u32, missing: &mut Vec<ThumbReq>) -> PageItem {
    let item = PageItem::new_placeholder(path.clone());
    item.set_rotation(rotation);
    missing.push(ThumbReq {
        id: item.stable_id(),
        path,
    });
    item
}

/// Append items for newly added pages.
pub(super) fn add_pages(store: &gio::ListStore, state: &State, indices: &[usize]) -> Vec<ThumbReq> {
    let mut missing = Vec::new();
    let project = state.project();
    for page in indices.iter().filter_map(|&i| project.pages.get(i)) {
        let item = placeholder(
            page.path.clone(),
            page.rotation.as_degrees() as u32,
            &mut missing,
        );
        store.append(&item);
    }
    missing
}

/// Remove the items of removed pages.
pub(super) fn remove_pages(store: &gio::ListStore, indices: &[usize]) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    for idx in sorted {
        if (idx as u32) < store.n_items() {
            store.remove(idx as u32);
        }
    }
}

/// Apply a reorder permutation by moving the existing items, so thumbnails
/// and stable ids survive, and carry the selection along with its pages.
pub(super) fn reorder_pages(
    store: &gio::ListStore,
    state: &State,
    selection: &gtk::MultiSelection,
    permutation: &[usize],
) -> Vec<ThumbReq> {
    let items: Vec<PageItem> = (0..store.n_items())
        .filter_map(|i| store.item(i).and_downcast::<PageItem>())
        .collect();
    if items.len() != permutation.len() {
        // The permutation describes a project the store never caught up with.
        // Store positions are read as page indices, so rebuild instead.
        tracing::warn!(
            "reorder permutation of {} does not match store of {}; rebuilding the store",
            permutation.len(),
            items.len()
        );
        return reconcile(store, state);
    }

    let selected: Vec<PageId> = selected_positions(selection)
        .into_iter()
        .filter_map(|p| items.get(p).map(PageItem::stable_id))
        .collect();

    let reordered: Vec<PageItem> = permutation.iter().map(|&old| items[old].clone()).collect();
    // A splice is one items-changed signal, but it also drops the selection,
    // hence the explicit restore below.
    store.splice(0, store.n_items(), &reordered);

    selection.unselect_all();
    for (pos, item) in reordered.iter().enumerate() {
        if selected.contains(&item.stable_id()) {
            selection.select_item(pos as u32, false);
        }
    }
    Vec::new()
}

/// Bring one item's rotation in line with its page.
pub(super) fn sync_page(store: &gio::ListStore, state: &State, idx: usize) {
    let project = state.project();
    let (Some(page), Some(item)) = (
        project.pages.get(idx),
        store.item(idx as u32).and_downcast::<PageItem>(),
    ) else {
        return;
    };
    let rotation = page.rotation.as_degrees() as u32;
    if item.rotation() != rotation {
        item.set_rotation_absolute(rotation);
    }
}

/// Rebuild the store to match the project page for page, reusing existing
/// items wherever possible.
///
/// Used after undo/redo, where the project may have been reordered, truncated
/// or grown in ways no fine-grained event describes. Items are matched by
/// path: pages sharing a path are interchangeable here, since a thumbnail
/// depends only on the path and rotation is re-synced.
pub(super) fn reconcile(store: &gio::ListStore, state: &State) -> Vec<ThumbReq> {
    let project = state.project();
    let mut spare: HashMap<PathBuf, VecDeque<PageItem>> = HashMap::new();
    for item in (0..store.n_items()).filter_map(|i| store.item(i).and_downcast::<PageItem>()) {
        spare.entry(item.path()).or_default().push_back(item);
    }

    let mut missing = Vec::new();
    let items: Vec<PageItem> = project
        .pages
        .iter()
        .map(|page| {
            let rotation = page.rotation.as_degrees() as u32;
            match spare.get_mut(&page.path).and_then(VecDeque::pop_front) {
                Some(item) => {
                    if item.rotation() != rotation {
                        item.set_rotation_absolute(rotation);
                    }
                    item
                }
                None => placeholder(page.path.clone(), rotation, &mut missing),
            }
        })
        .collect();
    store.splice(0, store.n_items(), &items);
    missing
}
