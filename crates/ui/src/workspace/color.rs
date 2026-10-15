//! Color mode: exposure, black level, white balance and saturation sliders
//! with a live preview of the first selected page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::app::State;
use crate::background::color_preview::{BaseCache, ColorReq};
use crate::background::preview_service::{PreviewService, RequestDedup};
use crate::pixbuf::PixelData;
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::ZoomPanController;
use recto_core::{BlackLevel, Command, Exposure, Project, Saturation, Temperature, Tint};

use super::builders::{make_preview, make_section};
use super::store::selected_positions;

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

pub(super) struct ColorView {
    state: State,
    selection: gtk::MultiSelection,
    pub(super) controls: gtk::Box,
    pub(super) preview: PreviewCanvas,
    zoom: Rc<ZoomPanController>,
    scales: Vec<(Slider, gtk::Scale)>,
    /// Set while the scales are moved to match the project, so their
    /// handlers don't dispatch the values straight back.
    syncing: Cell<bool>,
    preview_req: RequestDedup<ColorReq>,
}

impl ColorView {
    pub(super) fn new(state: State, selection: gtk::MultiSelection) -> Rc<Self> {
        let (preview, zoom) = make_preview();
        let (preview_svc, preview_req, preview_rx) = PreviewService::<ColorReq, PixelData>::new({
            // Lives on the single preview worker thread.
            let cache = RefCell::new(BaseCache::default());
            move |_id, req| {
                crate::background::color_preview::render_preview(&req, &mut cache.borrow_mut())
            }
        });

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
            .css_classes(["flat"])
            .build();

        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(10)
            .margin_end(10)
            .build();
        for (slider, scale) in &scales {
            controls.append(&make_section(slider.title(), scale));
        }
        controls.append(&gtk::Box::builder().vexpand(true).build());
        controls.append(&reset_btn);

        let view = Rc::new(Self {
            state,
            selection,
            controls,
            preview,
            zoom,
            scales,
            syncing: Cell::new(false),
            preview_req,
        });

        for &(slider, ref scale) in &view.scales {
            let weak = Rc::downgrade(&view);
            scale.connect_value_changed(move |scale| {
                if let Some(view) = weak.upgrade().filter(|v| !v.syncing.get()) {
                    view.state.dispatch(slider.command(scale.value()));
                }
            });
        }
        reset_btn.connect_clicked({
            let weak = Rc::downgrade(&view);
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.state.batch(|| {
                        for (slider, _) in &view.scales {
                            view.state.dispatch(slider.command(0.0));
                        }
                    });
                }
            }
        });

        glib::spawn_future_local({
            let weak = Rc::downgrade(&view);
            async move {
                let _svc = preview_svc;
                while let Ok((id, result)) = preview_rx.recv().await {
                    let Some(view) = weak.upgrade() else { break };
                    if view.preview_req.is_current(id) {
                        view.preview.set_texture(Some(result.to_texture()));
                        view.zoom.refit_after_texture_change();
                    }
                }
            }
        });

        view
    }

    /// Re-render the preview with the current settings.
    pub(super) fn refresh(&self) {
        let req = selected_positions(&self.selection)
            .first()
            .and_then(|&i| ColorReq::for_page(&self.state.project(), i));
        match req {
            Some(req) => {
                self.preview_req.send(req);
            }
            None => self.clear(),
        }
    }

    pub(super) fn clear(&self) {
        self.preview_req.send_dummy();
        self.preview.set_texture(None);
    }

    /// Move the scales to the project's values (load, undo, reset).
    pub(super) fn sync_from_project(&self) {
        let values: Vec<f64> = {
            let project = self.state.project();
            self.scales.iter().map(|(s, _)| s.get(&project)).collect()
        };
        self.syncing.set(true);
        for ((_, scale), v) in self.scales.iter().zip(values) {
            scale.set_value(v);
        }
        self.syncing.set(false);
    }
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
