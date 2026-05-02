use gtk::gio;
use recto_core::project::Project;
use std::cell::RefCell;
use std::rc::Rc;

use crate::steps::page_item::PageItem;

pub type State = Rc<RefCell<Project>>;

pub fn new_state() -> State {
    Rc::new(RefCell::new(Project::default()))
}

pub fn new_store() -> gio::ListStore {
    gio::ListStore::new::<PageItem>()
}
