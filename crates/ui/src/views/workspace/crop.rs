use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::glib;

use crate::app::State;
use crate::widgets::crop_picker::CropPicker;
use crate::widgets::preset_chips::PresetCallbacks;
use recto_core::{Command, CropBox, CropPreset, Rect};

pub struct CropSidebar {
    pub controls: gtk::Box,
    pub preset_chips: gtk::Box,
    pub btn_new: gtk::Button,
    pub status_lbl: gtk::Label,
}

pub fn build_crop_sidebar() -> CropSidebar {
    let preset_chips = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    let preset_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&preset_chips)
        .vexpand(true)
        .build();

    let btn_new = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label("New Preset")
                .build(),
        )
        .tooltip_text("New preset from current crop")
        .build();
    btn_new.add_css_class("flat");

    let presets_section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .vexpand(true)
        .build();
    let presets_header = gtk::Label::builder()
        .label("Presets")
        .xalign(0.0)
        .css_classes(["heading", "caption"])
        .build();
    presets_section.append(&presets_header);
    presets_section.append(&preset_scroll);

    let status_lbl = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .wrap(true)
        .build();

    let crop_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    crop_controls.append(&presets_section);
    crop_controls.append(&btn_new);

    CropSidebar {
        controls: crop_controls,
        preset_chips,
        btn_new,
        status_lbl,
    }
}

/// Wire the crop-mode signal handlers.
pub fn wire_crop_handlers(
    sidebar: &CropSidebar,
    state: State,
    callbacks: Rc<PresetCallbacks>,
    picker: Rc<CropPicker>,
    current_index: Rc<std::cell::Cell<Option<usize>>>,
    overlays: Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>,
    selected_indices: Rc<RefCell<Vec<usize>>>,
) {
    let s = state;
    let cb = callbacks;
    let pk = picker;
    let chips = sidebar.preset_chips.clone();
    let sl = sidebar.status_lbl.clone();
    let ci = current_index;
    let p = pk.clone();
    let ov = overlays;
    let si = selected_indices;

    sidebar.btn_new.connect_clicked(move |_| {
        let Some(idx) = ci.get() else {
            return;
        };
        let (w, h) = {
            let project = s.project();
            match project.pages.get(idx) {
                Some(page) => match page.crop {
                    Some(c) => (c.w, c.h),
                    None => {
                        let (iw, ih) = pk.image_dims();
                        (iw.max(1.0) as u32, ih.max(1.0) as u32)
                    }
                },
                None => return,
            }
        };
        let pi = s.project().crop_presets.len();
        let name = format!("Preset {}", pi + 1);
        s.dispatch(Command::AddCropPreset(CropPreset {
            name,
            w,
            h,
            locked: false,
        }));
        s.dispatch(Command::SetCropPreset {
            index: idx,
            preset: Some(pi),
        });
        s.dispatch(Command::SetCrop {
            index: idx,
            crop: Some(CropBox { x: 0, y: 0, w, h }),
        });
        pk.set_crop(Some(Rect::new(
            0.0, 0.0, w as f64, h as f64,
        )));
        {
            let project = s.project();
            crate::widgets::preset_chips::refresh_preset_chips(
                &chips, &project, ci.clone(), &p, &ov, &si, &cb,
            );
        }
        crate::widgets::crop_overlay::update_status_label(&sl, &s, Some(idx));
    });
}
