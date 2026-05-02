use gtk::subclass::prelude::ObjectSubclassIsExt;
use gtk::{gio, glib};
use recto_core::command::{AppEvent, AppState};
use recto_core::project::Project;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::widgets::page_item::PageItem;

pub type State = Rc<AppState>;

pub type MarkDirty = Rc<dyn Fn()>;

pub fn new_state() -> (State, async_channel::Receiver<AppEvent>) {
    let (state, rx) = AppState::new(Project::default());
    (Rc::new(state), rx)
}

pub fn new_store() -> gio::ListStore {
    gio::ListStore::new::<PageItem>()
}

mod imp {
    use super::*;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::Session)]
    pub struct Session {
        #[property(get, set)]
        dirty: Cell<bool>,

        #[property(get, set)]
        display_name: RefCell<String>,

        pub path: RefCell<Option<PathBuf>>,
        pub state: RefCell<Option<State>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Session {
        const NAME: &'static str = "RectoSession";
        type Type = super::Session;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Session {}
}

glib::wrapper! {
    pub struct Session(ObjectSubclass<imp::Session>);
}

impl Session {
    pub fn new(state: State) -> Rc<Self> {
        let session: Session = glib::Object::builder().build();
        session.imp().state.replace(Some(state));
        session.sync_display_name();
        Rc::new(session)
    }

    pub fn state(&self) -> State {
        self.imp().state.borrow().clone().expect("Session state not set")
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.imp().path.borrow().clone()
    }

    pub fn set_path(&self, p: Option<&Path>) {
        self.imp().path.replace(p.map(Path::to_path_buf));
        self.sync_display_name();
    }

    pub fn mark_dirty(&self) {
        if !self.dirty() {
            self.set_dirty(true);
            self.sync_display_name();
        }
    }

    pub fn clear_dirty(&self) {
        if self.dirty() {
            self.set_dirty(false);
            self.sync_display_name();
        }
    }

    pub fn reset(&self) {
        self.state().clear();
        self.imp().path.replace(None);
        self.set_dirty(false);
        self.sync_display_name();
    }

    fn sync_display_name(&self) {
        let name = match self.imp().path.borrow().as_ref() {
            Some(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_string()),
            None => "Untitled".to_string(),
        };
        let labeled = if self.dirty() {
            let mut d = name;
            d.push_str(" •");
            d
        } else {
            name
        };
        self.set_display_name(labeled);
    }
}
