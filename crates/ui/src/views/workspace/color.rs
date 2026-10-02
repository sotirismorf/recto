use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::app::State;
use recto_core::{BlackLevel, Command, Exposure, Project, Saturation, Temperature, Tint};

use super::helpers::make_section;

/// One colour adjustment and how its slider maps onto the project.
#[derive(Copy, Clone)]
enum Slider {
    Exposure,
    BlackLevel,
    Temperature,
    Tint,
    Saturation,
}

impl Slider {
    const ALL: [Self; 5] = [
        Self::Exposure,
        Self::BlackLevel,
        Self::Temperature,
        Self::Tint,
        Self::Saturation,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Exposure => "Exposure (EV)",
            Self::BlackLevel => "Black level (%)",
            Self::Temperature => "Temperature",
            Self::Tint => "Tint",
            Self::Saturation => "Saturation",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Exposure => {
                "Brighten or darken the light itself, in stops; ink and paper scale together"
            }
            Self::BlackLevel => "Remove glare that lifts the blacks, so ink regains its depth",
            Self::Temperature => "Cooler (blue) ↔ warmer (yellow)",
            Self::Tint => "Greener ↔ more magenta",
            Self::Saturation => "Leave at 0 for accurate colour",
        }
    }

    /// Slider (min, max, step, digits). Black level is shown in percent.
    fn range(self) -> (f64, f64, f64, i32) {
        match self {
            Self::Exposure => (
                Exposure::MIN.as_f32() as f64,
                Exposure::MAX.as_f32() as f64,
                0.05,
                2,
            ),
            Self::BlackLevel => (0.0, BlackLevel::MAX.as_f32() as f64 * 100.0, 0.1, 1),
            Self::Temperature | Self::Tint | Self::Saturation => (-1.0, 1.0, 0.01, 2),
        }
    }

    fn get(self, p: &Project) -> f64 {
        match self {
            Self::Exposure => p.exposure.as_f32() as f64,
            Self::BlackLevel => p.black_level.as_f32() as f64 * 100.0,
            Self::Temperature => p.temperature.as_f32() as f64,
            Self::Tint => p.tint.as_f32() as f64,
            Self::Saturation => p.saturation.as_f32() as f64,
        }
    }

    fn command(self, value: f64) -> Command {
        let v = value as f32;
        match self {
            Self::Exposure => Command::SetExposure(Exposure::new(v)),
            Self::BlackLevel => Command::SetBlackLevel(BlackLevel::new(v / 100.0)),
            Self::Temperature => Command::SetTemperature(Temperature::new(v)),
            Self::Tint => Command::SetTint(Tint::new(v)),
            Self::Saturation => Command::SetSaturation(Saturation::new(v)),
        }
    }
}

#[derive(Clone)]
pub struct ColorSidebar {
    pub controls: gtk::Box,
    scales: Vec<(Slider, gtk::Scale)>,
    pub reset_btn: gtk::Button,
    pub updating: Rc<Cell<bool>>,
}

fn make_adjustment_scale(slider: Slider, initial: f64) -> gtk::Scale {
    let (min, max, step, digits) = slider.range();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.set_value(initial);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_digits(digits);
    scale.set_hexpand(true);
    scale.set_tooltip_text(Some(slider.tooltip()));
    scale.add_mark(0.0, gtk::PositionType::Bottom, None);
    scale
}

pub fn build_color_sidebar(state: &State) -> ColorSidebar {
    let scales: Vec<(Slider, gtk::Scale)> = {
        let project = state.project();
        Slider::ALL
            .iter()
            .map(|&s| (s, make_adjustment_scale(s, s.get(&project))))
            .collect()
    };

    let reset_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("edit-undo-symbolic")
                .label("Reset")
                .build(),
        )
        .tooltip_text("Reset all colour adjustments")
        .build();
    reset_btn.add_css_class("flat");

    let color_spacer = gtk::Box::builder().vexpand(true).build();

    let color_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    for (slider, scale) in &scales {
        color_controls.append(&make_section(slider.title(), scale));
    }
    color_controls.append(&color_spacer);
    color_controls.append(&reset_btn);

    ColorSidebar {
        controls: color_controls,
        scales,
        reset_btn,
        updating: Rc::new(Cell::new(false)),
    }
}

/// Sync the scales with the project without re-dispatching commands.
/// Called on project load and undo/redo.
pub(crate) fn sync_color_scales(sidebar: &ColorSidebar, state: &State) {
    let values: Vec<f64> = {
        let project = state.project();
        sidebar
            .scales
            .iter()
            .map(|(s, _)| s.get(&project))
            .collect()
    };
    sidebar.updating.set(true);
    for ((_, scale), v) in sidebar.scales.iter().zip(values) {
        scale.set_value(v);
    }
    sidebar.updating.set(false);
}

/// Wire the color-mode signal handlers.
pub fn wire_color_handlers(sidebar: &ColorSidebar, state: State) {
    let s = state;
    let updating = sidebar.updating.clone();

    for &(slider, ref scale) in &sidebar.scales {
        scale.connect_value_changed(glib::clone!(
            #[strong]
            s,
            #[strong]
            updating,
            move |scale| {
                if updating.get() {
                    return;
                }
                s.dispatch(slider.command(scale.value()));
            }
        ));
    }

    sidebar.reset_btn.connect_clicked({
        let sidebar = sidebar.clone();
        move |_| {
            for (slider, _) in &sidebar.scales {
                s.dispatch(slider.command(0.0));
            }
            sync_color_scales(&sidebar, &s);
        }
    });
}
