use gtk::glib;
use gtk::prelude::*;

use crate::app::State;
use recto_core::{Brightness, Command, Contrast};

use super::helpers::make_section;

pub struct ColorSidebar {
    pub controls: gtk::Box,
    pub brightness_scale: gtk::Scale,
    pub contrast_scale: gtk::Scale,
    pub reset_btn: gtk::Button,
}

pub fn build_color_sidebar(state: &State) -> ColorSidebar {
    let init_brightness = state.project().brightness.as_f32() as f64;
    let init_contrast = state.project().contrast.as_f32() as f64;

    let brightness_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.01);
    brightness_scale.set_value(init_brightness);
    brightness_scale.set_draw_value(true);
    brightness_scale.set_value_pos(gtk::PositionType::Right);
    brightness_scale.set_digits(2);
    brightness_scale.set_hexpand(true);
    brightness_scale.add_mark(0.0, gtk::PositionType::Bottom, None);

    let contrast_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.01);
    contrast_scale.set_value(init_contrast);
    contrast_scale.set_draw_value(true);
    contrast_scale.set_value_pos(gtk::PositionType::Right);
    contrast_scale.set_digits(2);
    contrast_scale.set_hexpand(true);
    contrast_scale.add_mark(0.0, gtk::PositionType::Bottom, None);

    let reset_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("edit-undo-symbolic")
                .label("Reset")
                .build(),
        )
        .tooltip_text("Reset brightness and contrast")
        .build();
    reset_btn.add_css_class("flat");

    let brightness_section = make_section("Brightness", &brightness_scale);
    let contrast_section = make_section("Contrast", &contrast_scale);

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
    color_controls.append(&color_spacer);
    color_controls.append(&reset_btn);

    ColorSidebar {
        controls: color_controls,
        brightness_scale,
        contrast_scale,
        reset_btn,
    }
}

/// Wire the color-mode signal handlers.
pub fn wire_color_handlers(sidebar: &ColorSidebar, state: State) {
    let s = state;
    let bs = sidebar.brightness_scale.clone();
    let cs = sidebar.contrast_scale.clone();

    sidebar.brightness_scale.connect_value_changed(glib::clone!(
        #[strong]
        s,
        move |scale| {
            s.dispatch(Command::SetBrightness(
                Brightness::new(scale.value() as f32),
            ));
        }
    ));

    sidebar.contrast_scale.connect_value_changed(glib::clone!(
        #[strong]
        s,
        move |scale| {
            s.dispatch(Command::SetContrast(Contrast::new(scale.value() as f32)));
        }
    ));

    sidebar.reset_btn.connect_clicked(glib::clone!(
        #[strong]
        s,
        #[weak]
        bs,
        #[weak]
        cs,
        move |_| {
            s.dispatch(Command::SetBrightness(Brightness::ZERO));
            s.dispatch(Command::SetContrast(Contrast::ZERO));
            bs.set_value(0.0);
            cs.set_value(0.0);
        }
    ));
}
