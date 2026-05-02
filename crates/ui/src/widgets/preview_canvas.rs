use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    type ResizeCallback = Box<dyn Fn(i32, i32)>;

    #[derive(Default)]
    pub struct PreviewCanvas {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub zoom: Cell<f64>,
        pub pan: Cell<(f64, f64)>,
        pub cursor_anchor: RefCell<Option<(f64, f64, f64, f64)>>, // (ix, iy, cx, cy)
        pub on_resize: RefCell<Option<ResizeCallback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PreviewCanvas {
        const NAME: &'static str = "RectoPreviewCanvas";
        type Type = super::PreviewCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PreviewCanvas {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![glib::ParamSpecDouble::builder("zoom")
                    .minimum(0.0)
                    .default_value(1.0)
                    .build()]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            if pspec.name() == "zoom" {
                self.zoom
                    .set(value.get().expect("zoom property value must be f64"));
                self.obj().queue_draw();
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "zoom" => self.zoom.get().to_value(),
                _ => unreachable!(),
            }
        }
    }

    impl WidgetImpl for PreviewCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(tex) = self.texture.borrow().clone() else {
                return;
            };
            let z = self.zoom.get();
            let (px, py) = self.pan.get();
            let w = tex.width() as f32 * z as f32;
            let h = tex.height() as f32 * z as f32;
            let filter = if z < 1.0 {
                gsk::ScalingFilter::Trilinear
            } else if z > 1.5 {
                gsk::ScalingFilter::Nearest
            } else {
                gsk::ScalingFilter::Linear
            };
            snapshot.append_scaled_texture(
                &tex,
                filter,
                &graphene::Rect::new(px as f32, py as f32, w, h),
            );
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if let Some(cb) = self.on_resize.borrow().as_ref() {
                cb(width, height);
            }
        }
    }
}

glib::wrapper! {
    pub struct PreviewCanvas(ObjectSubclass<imp::PreviewCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PreviewCanvas {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_texture(&self, t: Option<gdk::Texture>) {
        self.imp().texture.replace(t);
        self.queue_draw();
    }

    pub fn set_transform(&self, zoom: f64, pan: (f64, f64)) {
        self.imp().zoom.set(zoom);
        self.imp().pan.set(pan);
        self.queue_draw();
    }

    pub fn set_pan(&self, px: f64, py: f64) {
        self.imp().pan.set((px, py));
        self.queue_draw();
    }

    pub fn set_on_resize<F: Fn(i32, i32) + 'static>(&self, f: F) {
        self.imp().on_resize.replace(Some(Box::new(f)));
    }

    pub fn texture_size(&self) -> (f64, f64) {
        if let Some(tex) = self.imp().texture.borrow().as_ref() {
            (tex.width() as f64, tex.height() as f64)
        } else {
            (0.0, 0.0)
        }
    }

    pub fn zoom(&self) -> f64 {
        self.imp().zoom.get()
    }

    pub fn pan(&self) -> (f64, f64) {
        self.imp().pan.get()
    }

    pub fn transform(&self) -> (f64, f64, f64) {
        let (px, py) = self.imp().pan.get();
        (self.imp().zoom.get(), px, py)
    }

    pub fn set_cursor_anchor(&self, ix: f64, iy: f64, cx: f64, cy: f64) {
        self.imp().cursor_anchor.replace(Some((ix, iy, cx, cy)));
    }

    pub fn cursor_anchor(&self) -> Option<(f64, f64, f64, f64)> {
        *self.imp().cursor_anchor.borrow()
    }
}
