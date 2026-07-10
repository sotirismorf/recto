use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::app::State;
use recto_core::{Brightness, Command, Contrast, Saturation};

use super::helpers::make_section;

#[derive(Clone)]
pub struct ColorSidebar {
    pub controls: gtk::Box,
    pub brightness_scale: gtk::Scale,
    pub contrast_scale: gtk::Scale,
    pub saturation_scale: gtk::Scale,
    pub reset_btn: gtk::Button,
    pub updating: Rc<Cell<bool>>,
}

fn make_adjustment_scale(initial: f64) -> gtk::Scale {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.01);
    scale.set_value(initial);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_digits(2);
    scale.set_hexpand(true);
    scale.add_mark(0.0, gtk::PositionType::Bottom, None);
    scale
}

pub fn build_color_sidebar(state: &State) -> ColorSidebar {
    let project = state.project();
    let brightness_scale = make_adjustment_scale(project.brightness.as_f32() as f64);
    let contrast_scale = make_adjustment_scale(project.contrast.as_f32() as f64);
    let saturation_scale = make_adjustment_scale(project.saturation.as_f32() as f64);
    drop(project);

    let reset_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("edit-undo-symbolic")
                .label("Reset")
                .build(),
        )
        .tooltip_text("Reset brightness, contrast, and saturation")
        .build();
    reset_btn.add_css_class("flat");

    let brightness_section = make_section("Brightness", &brightness_scale);
    let contrast_section = make_section("Contrast", &contrast_scale);
    let saturation_section = make_section("Saturation", &saturation_scale);

    let color_spacer = gtk::Box::builder().vexpand(true).build();

    let color_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    color_controls.append(&brightness_section);
    color_controls.append(&contrast_section);
    color_controls.append(&saturation_section);
    color_controls.append(&color_spacer);
    color_controls.append(&reset_btn);

    ColorSidebar {
        controls: color_controls,
        brightness_scale,
        contrast_scale,
        saturation_scale,
        reset_btn,
        updating: Rc::new(Cell::new(false)),
    }
}

/// Sync the scales with the project without re-dispatching commands.
/// Called on project load and undo/redo.
pub(crate) fn sync_color_scales(sidebar: &ColorSidebar, state: &State) {
    let (b, c, s) = {
        let project = state.project();
        (
            project.brightness.as_f32() as f64,
            project.contrast.as_f32() as f64,
            project.saturation.as_f32() as f64,
        )
    };
    sidebar.updating.set(true);
    sidebar.brightness_scale.set_value(b);
    sidebar.contrast_scale.set_value(c);
    sidebar.saturation_scale.set_value(s);
    sidebar.updating.set(false);
}

/// Wire the color-mode signal handlers.
pub fn wire_color_handlers(sidebar: &ColorSidebar, state: State) {
    let s = state;
    let bs = sidebar.brightness_scale.clone();
    let cs = sidebar.contrast_scale.clone();
    let ss = sidebar.saturation_scale.clone();
    let updating = sidebar.updating.clone();

    sidebar.brightness_scale.connect_value_changed(glib::clone!(
        #[strong]
        s,
        #[strong]
        updating,
        move |scale| {
            if updating.get() {
                return;
            }
            s.dispatch(Command::SetBrightness(
                Brightness::new(scale.value() as f32),
            ));
        }
    ));

    sidebar.contrast_scale.connect_value_changed(glib::clone!(
        #[strong]
        s,
        #[strong]
        updating,
        move |scale| {
            if updating.get() {
                return;
            }
            s.dispatch(Command::SetContrast(Contrast::new(scale.value() as f32)));
        }
    ));

    sidebar.saturation_scale.connect_value_changed(glib::clone!(
        #[strong]
        s,
        #[strong]
        updating,
        move |scale| {
            if updating.get() {
                return;
            }
            s.dispatch(Command::SetSaturation(
                Saturation::new(scale.value() as f32),
            ));
        }
    ));

    sidebar.reset_btn.connect_clicked(glib::clone!(
        #[strong]
        s,
        #[strong]
        updating,
        #[weak]
        bs,
        #[weak]
        cs,
        #[weak]
        ss,
        move |_| {
            s.dispatch(Command::SetBrightness(Brightness::ZERO));
            s.dispatch(Command::SetContrast(Contrast::ZERO));
            s.dispatch(Command::SetSaturation(Saturation::ZERO));
            updating.set(true);
            bs.set_value(0.0);
            cs.set_value(0.0);
            ss.set_value(0.0);
            updating.set(false);
        }
    ));
}
