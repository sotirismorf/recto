use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::app::State;
use crate::widgets::crop_picker::CropPicker;
use crate::widgets::preset_chips::PresetCallbacks;
use recto_core::{Command, CropBox, CropPreset, Rect};

#[derive(Clone)]
pub struct CropSidebar {
    pub controls: gtk::Box,
    pub preset_chips: gtk::Box,
    pub btn_new: gtk::Button,
    pub btn_auto: gtk::Button,
    pub margin_spin: gtk::SpinButton,
    pub last_margin: std::rc::Rc<std::cell::Cell<f64>>,
    pub is_updating: std::rc::Rc<std::cell::Cell<bool>>,
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

    let btn_auto = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("image-adjust-colors-symbolic")
                .label("Auto Detect")
                .build(),
        )
        .tooltip_text("Automatically detect page boundaries using OpenCV")
        .build();
    btn_auto.add_css_class("flat");
    btn_auto.set_sensitive(true);

    let margin_spin = gtk::SpinButton::builder()
        .adjustment(&gtk::Adjustment::new(0.0, -200.0, 200.0, 1.0, 10.0, 0.0))
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .update_policy(gtk::SpinButtonUpdatePolicy::IfValid)
        .tooltip_text("Shrink or grow crop by this many pixels on all sides (applies to all pages in the preset)")
        .build();
    margin_spin.set_sensitive(false);

    let last_margin = std::rc::Rc::new(std::cell::Cell::new(0.0));
    let is_updating = std::rc::Rc::new(std::cell::Cell::new(false));

    let margin_label = gtk::Label::builder()
        .label("Crop Margin")
        .xalign(0.0)
        .css_classes(["caption"])
        .build();

    let margin_section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    margin_section.append(&margin_label);
    margin_section.append(&margin_spin);

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
    crop_controls.append(&margin_section);
    crop_controls.append(&btn_auto);
    crop_controls.append(&btn_new);

    CropSidebar {
        controls: crop_controls,
        preset_chips,
        btn_new,
        btn_auto,
        margin_spin,
        last_margin,
        is_updating,
        status_lbl,
    }
}

/// Set the margin spin sensitivity based on the current selection state.
/// Should be called whenever the selected preset changes.
pub(crate) fn update_margin_spin(
    sidebar: &CropSidebar,
    state: &State,
    current_index: &std::cell::Cell<Option<usize>>,
) {
    let spin = &sidebar.margin_spin;
    let last_margin = &sidebar.last_margin;
    let is_updating = &sidebar.is_updating;

    let preset_info = {
        let Some(idx) = current_index.get() else {
            return;
        };
        let project = state.project();
        let page = project.pages.get(idx);
        let pi = page.and_then(|p| p.crop_preset);
        let preset = pi.and_then(|i| project.crop_presets.get(i));
        preset.map(|p| (p.locked, 0.0))
    };

    is_updating.set(true);
    match preset_info {
        Some((locked, value)) => {
            spin.set_sensitive(!locked);
            if (spin.value() - value).abs() > f64::EPSILON {
                spin.set_value(value);
            }
            last_margin.set(value);
        }
        None => {
            spin.set_sensitive(false);
            if (spin.value() - 0.0).abs() > f64::EPSILON {
                spin.set_value(0.0);
            }
            last_margin.set(0.0);
        }
    }
    is_updating.set(false);
}

/// Wire the crop-mode signal handlers.
#[allow(clippy::too_many_arguments)]
pub fn wire_crop_handlers(
    sidebar: &CropSidebar,
    state: State,
    callbacks: Rc<PresetCallbacks>,
    picker: Rc<CropPicker>,
    current_index: Rc<Cell<Option<usize>>>,
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

    // Pre-clone for the margin spin handler (must happen before btn_new moves them)
    let s_m = s.clone();
    let cb_m = cb.clone();
    let pk_m = pk.clone();
    let chips_m = chips.clone();
    let sl_m = sl.clone();
    let ci_m = ci.clone();
    let p_m = p.clone();
    let ov_m = ov.clone();
    let si_m = si.clone();

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
        pk.set_crop(Some(Rect::new(0.0, 0.0, w as f64, h as f64)));
        {
            let project = s.project();
            crate::widgets::preset_chips::refresh_preset_chips(
                &chips,
                &project,
                ci.clone(),
                &p,
                &ov,
                &si,
                &cb,
            );
        }
        crate::widgets::crop_overlay::update_status_label(&sl, &s, Some(idx));
    });

    // ---- Crop margin handler ------------------------------------------------
    {
        let spin = sidebar.margin_spin.clone();
        let last_margin = sidebar.last_margin.clone();
        let is_updating = sidebar.is_updating.clone();

        spin.connect_value_changed(move |spin| {
            if is_updating.get() {
                return;
            }
            is_updating.set(true);

            let current_margin = spin.value();
            let delta = (current_margin - last_margin.get()) as i32;
            last_margin.set(current_margin);

            if delta == 0 {
                is_updating.set(false);
                return;
            }

            let Some(idx) = ci_m.get() else {
                is_updating.set(false);
                return;
            };

            let (pi, pw, ph, pages_to_update) = {
                let project = s_m.project();
                let page = match project.pages.get(idx) {
                    Some(p) => p,
                    None => {
                        is_updating.set(false);
                        return;
                    }
                };
                let Some(pi) = page.crop_preset else {
                    is_updating.set(false);
                    return;
                };
                let Some(preset) = project.crop_presets.get(pi) else {
                    is_updating.set(false);
                    return;
                };
                if preset.locked {
                    is_updating.set(false);
                    return;
                }

                // pw and ph are the NEW preset dimensions
                let pw = (preset.w as i32 + delta * 2).max(1) as u32;
                let ph = (preset.h as i32 + delta * 2).max(1) as u32;

                let mut pages_to_update = Vec::new();
                for (page_idx, p) in project.pages.iter().enumerate() {
                    if p.crop_preset == Some(pi) {
                        pages_to_update.push((page_idx, p.crop));
                    }
                }

                (pi, pw, ph, pages_to_update)
            };

            s_m.dispatch(Command::SetCropPresetSize {
                index: pi,
                w: pw,
                h: ph,
            });

            let (iw, ih) = pk_m.image_dims();
            for (page_idx, old_crop) in pages_to_update {
                let mut cb = old_crop.unwrap_or(recto_core::CropBox {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                });

                // Centering logic: move x and y in the opposite direction of the margin change
                cb.x = (cb.x as i32 - delta).max(0) as u32;
                cb.y = (cb.y as i32 - delta).max(0) as u32;
                cb.w = pw;
                cb.h = ph;

                // Clamp to image boundaries
                if iw > 0.0 && cb.x as f64 + pw as f64 > iw {
                    cb.x = (iw as u32).saturating_sub(pw);
                }
                if ih > 0.0 && cb.y as f64 + ph as f64 > ih {
                    cb.y = (ih as u32).saturating_sub(ph);
                }

                s_m.dispatch(Command::SetCrop {
                    index: page_idx,
                    crop: Some(cb),
                });

                // If this is the currently viewed page, update the picker widget immediately
                if Some(page_idx) == ci_m.get() {
                    pk_m.set_crop(Some(recto_core::Rect::from(cb)));
                }
            }

            let project_after = s_m.project();
            crate::widgets::preset_chips::refresh_preset_chips(
                &chips_m,
                &project_after,
                ci_m.clone(),
                &p_m,
                &ov_m,
                &si_m,
                &cb_m,
            );
            crate::widgets::crop_overlay::update_status_label(&sl_m, &s_m, Some(idx));
            crate::widgets::crop_overlay::queue_all_overlays(&ov_m);

            is_updating.set(false);
        });
    }
}
