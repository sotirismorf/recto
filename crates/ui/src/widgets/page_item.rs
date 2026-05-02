use gtk::{gdk, gdk_pixbuf, glib, prelude::*, subclass::prelude::*};
use recto_core::Rotation;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use crate::types::PageId;

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::PageItem)]
    pub struct PageItem {
        #[property(get, set, nullable)]
        pub thumbnail: RefCell<Option<gdk::Paintable>>,
        #[property(get, set)]
        pub filename: RefCell<String>,
        #[property(get, set)]
        pub rotation: Cell<u32>,

        pub base_pixbuf: RefCell<Option<gdk_pixbuf::Pixbuf>>,
        pub path: RefCell<PathBuf>,
        pub base_width: Cell<u32>,
        pub base_height: Cell<u32>,
        pub stable_id: Cell<PageId>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageItem {
        const NAME: &'static str = "RectoPageItem";
        type Type = super::PageItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PageItem {}
}

glib::wrapper! {
    pub struct PageItem(ObjectSubclass<imp::PageItem>);
}

impl PageItem {
    pub fn new(path: PathBuf, base: gdk_pixbuf::Pixbuf) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().stable_id.set(PageId::new());
        {
            let i = obj.imp();
            *i.path.borrow_mut() = path.clone();
            *i.base_pixbuf.borrow_mut() = Some(base.clone());
        }
        let texture = gdk::Texture::for_pixbuf(&base);
        obj.set_thumbnail(Some(texture.upcast::<gdk::Paintable>()));
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        obj.set_filename(name);
        obj
    }

    pub fn new_placeholder(path: PathBuf) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().stable_id.set(PageId::new());
        {
            let i = obj.imp();
            *i.path.borrow_mut() = path.clone();
            *i.base_pixbuf.borrow_mut() = None;
        }
        obj.set_thumbnail(None::<gdk::Paintable>);
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        obj.set_filename(name);
        obj
    }

    pub fn stable_id(&self) -> PageId {
        self.imp().stable_id.get()
    }

    pub fn set_image(&self, base: gdk_pixbuf::Pixbuf) {
        {
            let i = self.imp();
            *i.base_pixbuf.borrow_mut() = Some(base.clone());
        }
        let current_rotation = self.rotation();
        let rotated = rotate(&base, current_rotation);
        let texture = gdk::Texture::for_pixbuf(&rotated);
        self.set_thumbnail(Some(texture.upcast::<gdk::Paintable>()));
    }

    pub fn set_dims(&self, w: u32, h: u32) {
        let i = self.imp();
        i.base_width.set(w);
        i.base_height.set(h);
    }

    pub fn base_width(&self) -> u32 {
        self.imp().base_width.get()
    }

    pub fn base_height(&self) -> u32 {
        self.imp().base_height.get()
    }

    /// Set rotation to an absolute value and re-render the thumbnail.
    pub fn set_rotation_absolute(&self, deg: u32) {
        self.set_rotation(deg);
        let base = self.imp().base_pixbuf.borrow().clone();
        if let Some(base) = base {
            let rotated = rotate(&base, deg);
            let texture = gdk::Texture::for_pixbuf(&rotated);
            self.set_thumbnail(Some(texture.upcast::<gdk::Paintable>()));
        }
    }

    pub fn path(&self) -> PathBuf {
        self.imp().path.borrow().clone()
    }

    pub fn rotation_value(&self) -> Rotation {
        Rotation::new(self.rotation() as u16)
    }

    pub fn set_rotation_value(&self, r: Rotation) {
        self.set_rotation_absolute(r.as_degrees() as u32);
    }
}

pub fn rotate(p: &gdk_pixbuf::Pixbuf, deg: u32) -> gdk_pixbuf::Pixbuf {
    match deg % 360 {
        90 => p
            .rotate_simple(gdk_pixbuf::PixbufRotation::Clockwise)
            .unwrap_or_else(|| p.clone()),
        180 => p
            .rotate_simple(gdk_pixbuf::PixbufRotation::Upsidedown)
            .unwrap_or_else(|| p.clone()),
        270 => p
            .rotate_simple(gdk_pixbuf::PixbufRotation::Counterclockwise)
            .unwrap_or_else(|| p.clone()),
        _ => p.clone(),
    }
}
