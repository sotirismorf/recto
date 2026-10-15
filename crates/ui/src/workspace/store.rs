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
    let item = PageItem::new(path.clone());
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

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use recto_core::state::AppState;
    use recto_core::{Command, Project, Rotation};

    fn state_with(paths: &[&str]) -> State {
        let (state, _rx) = AppState::new(Project::default());
        state.dispatch(Command::AddPages(paths.iter().map(PathBuf::from).collect()));
        Rc::new(state)
    }

    fn store_paths(store: &gio::ListStore) -> Vec<PathBuf> {
        items(store).iter().map(PageItem::path).collect()
    }

    fn items(store: &gio::ListStore) -> Vec<PageItem> {
        (0..store.n_items())
            .filter_map(|i| store.item(i).and_downcast::<PageItem>())
            .collect()
    }

    #[test]
    fn add_pages_appends_items_and_requests_thumbnails() {
        let state = state_with(&["a.jpg", "b.jpg"]);
        let store = gio::ListStore::new::<PageItem>();
        let requests = add_pages(&store, &state, &[0, 1]);
        assert_eq!(
            store_paths(&store),
            [PathBuf::from("a.jpg"), "b.jpg".into()]
        );
        let ids: Vec<PageId> = items(&store).iter().map(PageItem::stable_id).collect();
        let requested: Vec<PageId> = requests.iter().map(|r| r.id).collect();
        assert_eq!(ids, requested);
    }

    #[test]
    fn remove_pages_handles_unsorted_indices() {
        let state = state_with(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let store = gio::ListStore::new::<PageItem>();
        add_pages(&store, &state, &[0, 1, 2, 3]);
        remove_pages(&store, &[1, 3]);
        assert_eq!(
            store_paths(&store),
            [PathBuf::from("a.jpg"), "c.jpg".into()]
        );
    }

    #[test]
    fn reconcile_reuses_items_and_syncs_rotation() {
        let state = state_with(&["a.jpg", "b.jpg"]);
        let store = gio::ListStore::new::<PageItem>();
        add_pages(&store, &state, &[0, 1]);
        let id_of = |store: &gio::ListStore, path: &str| {
            items(store)
                .into_iter()
                .find(|item| item.path() == std::path::Path::new(path))
                .map(|item| item.stable_id())
        };
        let a_id = id_of(&store, "a.jpg");

        // Reorder, rotate and grow the project behind the store's back.
        state.dispatch(Command::MovePages {
            indices: vec![1],
            before: 0,
        });
        state.dispatch(Command::SetRotation {
            index: 1,
            rotation: Rotation::DEG90,
        });
        state.dispatch(Command::AddPages(vec!["c.jpg".into()]));

        let requests = reconcile(&store, &state);
        assert_eq!(
            store_paths(&store),
            [PathBuf::from("b.jpg"), "a.jpg".into(), "c.jpg".into()]
        );
        assert_eq!(id_of(&store, "a.jpg"), a_id, "existing items are reused");
        assert_eq!(items(&store)[1].rotation(), 90);
        // Only the new page needs a thumbnail.
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, PathBuf::from("c.jpg"));
    }
}
