use gtk::gio;
use recto_core::project::Project;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::steps::page_item::PageItem;

pub type State = Rc<RefCell<Project>>;

/// Callback that marks the current session as having unsaved changes.
pub type MarkDirty = Rc<dyn Fn()>;

pub fn new_state() -> State {
    Rc::new(RefCell::new(Project::default()))
}

pub fn new_store() -> gio::ListStore {
    gio::ListStore::new::<PageItem>()
}

/// Session-level state surrounding the loaded project: where it came from on
/// disk (if anywhere), whether it has unsaved edits, and a callback list so
/// the window chrome can react to changes.
pub struct Session {
    pub state: State,
    path: RefCell<Option<PathBuf>>,
    dirty: Cell<bool>,
    listeners: RefCell<Vec<Box<dyn Fn(&Session)>>>,
}

impl Session {
    pub fn new(state: State) -> Rc<Self> {
        Rc::new(Self {
            state,
            path: RefCell::new(None),
            dirty: Cell::new(false),
            listeners: RefCell::new(Vec::new()),
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.path.borrow().clone()
    }

    /// Display name for the title bar: file stem if saved, "Untitled" otherwise.
    pub fn display_name(&self) -> String {
        match self.path.borrow().as_ref() {
            Some(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_string()),
            None => "Untitled".to_string(),
        }
    }

    pub fn mark_dirty(&self) {
        if !self.dirty.get() {
            self.dirty.set(true);
            self.emit();
        }
    }

    pub fn clear_dirty(&self) {
        if self.dirty.get() {
            self.dirty.set(false);
            self.emit();
        }
    }

    pub fn set_path(&self, p: Option<&Path>) {
        *self.path.borrow_mut() = p.map(Path::to_path_buf);
        self.emit();
    }

    /// Reset to a fresh untitled project: clears state, path and dirty.
    pub fn reset(&self) {
        *self.state.borrow_mut() = Project::default();
        *self.path.borrow_mut() = None;
        self.dirty.set(false);
        self.emit();
    }

    pub fn connect_changed(&self, f: impl Fn(&Session) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    fn emit(&self) {
        let listeners = self.listeners.borrow();
        for l in listeners.iter() {
            l(self);
        }
    }
}
