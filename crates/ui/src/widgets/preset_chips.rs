use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::widgets::crop_overlay::queue_all_overlays;
use crate::widgets::crop_picker::CropPicker;
use crate::widgets::page_item::PageItem;
use recto_core::{CropBox, CropPreset, Project, Rect};

pub(crate) struct PresetCallbacks {
    pub get_project: Rc<dyn Fn() -> Project>,
    pub on_set_crop_preset: Rc<dyn Fn(usize, Option<usize>)>,
    pub on_set_crop: Rc<dyn Fn(usize, Option<CropBox>)>,
    pub on_set_crop_preset_locked: Rc<dyn Fn(usize, bool)>,
    pub on_remove_crop_preset: Rc<dyn Fn(usize)>,
    pub on_add_crop_preset: Rc<dyn Fn(CropPreset)>,
}

pub(crate) fn refresh_preset_chips(
    chip_box: &gtk::Box,
    project: &Project,
    current_index: Rc<Cell<Option<usize>>>,
    picker: &Rc<CropPicker>,
    overlays: &Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>,
    selected_indices: &Rc<RefCell<Vec<usize>>>,
    callbacks: &PresetCallbacks,
) {
    while let Some(child) = chip_box.first_child() {
        chip_box.remove(&child);
    }

    let n_presets = project.crop_presets.len();
    if n_presets == 0 {
        return;
    }

    let current_preset: Option<usize> = current_index
        .get()
        .and_then(|idx| project.pages.get(idx))
        .and_then(|p| p.crop_preset);

    let mut refs: Vec<usize> = vec![0; n_presets];
    for page in project.pages.iter() {
        if let Some(pi) = page.crop_preset {
            if pi < n_presets {
                refs[pi] += 1;
            }
        }
    }

    for (i, preset) in project.crop_presets.iter().enumerate() {
        let chip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(0)
            .css_classes(vec!["preset-chip", &format!("preset-c{}", i % 8)])
            .build();
        if current_preset == Some(i) {
            chip.add_css_class("preset-active");
        }

        let label_str = format!("Preset {}", i + 1);
        let label_btn = gtk::Button::builder()
            .label(&label_str)
            .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
            .build();
        label_btn.add_css_class("preset-label");
        label_btn.set_tooltip_text(Some(&format!("{} ({}×{})", label_str, preset.w, preset.h)));

        {
            let ci = current_index.clone();
            let picker_weak = Rc::downgrade(picker);
            let chips_weak = chip_box.downgrade();
            let sel = selected_indices.clone();
            let ov = overlays.clone();
            let pi = i;
            let pw = preset.w;
            let ph = preset.h;
            let cb_clone = PresetCallbacks {
                get_project: callbacks.get_project.clone(),
                on_set_crop_preset: callbacks.on_set_crop_preset.clone(),
                on_set_crop: callbacks.on_set_crop.clone(),
                on_set_crop_preset_locked: callbacks.on_set_crop_preset_locked.clone(),
                on_remove_crop_preset: callbacks.on_remove_crop_preset.clone(),
                on_add_crop_preset: callbacks.on_add_crop_preset.clone(),
            };
            label_btn.connect_clicked(move |_| {
                let Some(picker) = picker_weak.upgrade() else {
                    return;
                };
                let Some(chips) = chips_weak.upgrade() else {
                    return;
                };
                let targets = sel.borrow().clone();
                if targets.is_empty() {
                    return;
                }
                let (iw, ih) = picker.image_dims();
                let mut new_first: Option<Rect> = None;
                let mut crops: Vec<(usize, CropBox)> = Vec::new();
                {
                    let project = (cb_clone.get_project)();
                    for &idx in &targets {
                        let Some(page) = project.pages.get(idx) else {
                            continue;
                        };
                        let mut cb = page.crop.unwrap_or(CropBox {
                            x: 0,
                            y: 0,
                            w: 0,
                            h: 0,
                        });
                        cb.w = pw;
                        cb.h = ph;
                        if iw > 0.0 && cb.x as f64 + pw as f64 > iw {
                            cb.x = (iw as u32).saturating_sub(pw);
                        }
                        if ih > 0.0 && cb.y as f64 + ph as f64 > ih {
                            cb.y = (ih as u32).saturating_sub(ph);
                        }
                        if new_first.is_none() {
                            new_first = Some(Rect::from(cb));
                        }
                        crops.push((idx, cb));
                    }
                }
                for (idx, cb) in crops {
                    (cb_clone.on_set_crop_preset)(idx, Some(pi));
                    (cb_clone.on_set_crop)(idx, Some(cb));
                }
                if let Some(r) = new_first {
                    picker.set_crop(Some(r));
                }
                let project = (cb_clone.get_project)();
                refresh_preset_chips(&chips, &project, ci.clone(), &picker, &ov, &sel, &cb_clone);
            });
        }

        chip.append(&label_btn);

        let lock_btn = gtk::ToggleButton::builder()
            .active(preset.locked)
            .has_frame(false)
            .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
            .build();
        lock_btn.add_css_class("preset-lock");
        if preset.locked {
            lock_btn.set_icon_name("changes-prevent-symbolic");
            lock_btn.set_tooltip_text(Some("Locked — drag won't change the preset size"));
        } else {
            lock_btn.set_icon_name("changes-allow-symbolic");
            lock_btn.set_tooltip_text(Some(
                "Unlocked — drag updates the preset for all pages in this group",
            ));
        }

        {
            let chips_weak = chip_box.downgrade();
            let ci = current_index.clone();
            let picker_weak = Rc::downgrade(picker);
            let ov = overlays.clone();
            let sel = selected_indices.clone();
            let pi = i;
            let cb_clone = PresetCallbacks {
                get_project: callbacks.get_project.clone(),
                on_set_crop_preset: callbacks.on_set_crop_preset.clone(),
                on_set_crop: callbacks.on_set_crop.clone(),
                on_set_crop_preset_locked: callbacks.on_set_crop_preset_locked.clone(),
                on_remove_crop_preset: callbacks.on_remove_crop_preset.clone(),
                on_add_crop_preset: callbacks.on_add_crop_preset.clone(),
            };
            lock_btn.connect_toggled(move |btn| {
                let locked = btn.is_active();
                if locked {
                    btn.set_icon_name("changes-prevent-symbolic");
                    btn.set_tooltip_text(Some("Locked — drag won't change the preset size"));
                } else {
                    btn.set_icon_name("changes-allow-symbolic");
                    btn.set_tooltip_text(Some(
                        "Unlocked — drag updates the preset for all pages in this group",
                    ));
                }
                (cb_clone.on_set_crop_preset_locked)(pi, locked);
                if let (Some(picker), Some(chips)) = (picker_weak.upgrade(), chips_weak.upgrade()) {
                    let project = (cb_clone.get_project)();
                    refresh_preset_chips(
                        &chips,
                        &project,
                        ci.clone(),
                        &picker,
                        &ov,
                        &sel,
                        &cb_clone,
                    );
                }
            });
        }

        chip.append(&lock_btn);

        if refs[i] == 0 {
            let close_btn = gtk::Button::builder()
                .label("\u{00d7}")
                .has_frame(false)
                .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
                .tooltip_text("Delete this preset permanently")
                .build();
            close_btn.add_css_class("preset-close");

            {
                let chips_weak = chip_box.downgrade();
                let ci = current_index.clone();
                let picker_weak = Rc::downgrade(picker);
                let ov = overlays.clone();
                let sel = selected_indices.clone();
                let pi = i;
                let cb_clone = PresetCallbacks {
                    get_project: callbacks.get_project.clone(),
                    on_set_crop_preset: callbacks.on_set_crop_preset.clone(),
                    on_set_crop: callbacks.on_set_crop.clone(),
                    on_set_crop_preset_locked: callbacks.on_set_crop_preset_locked.clone(),
                    on_remove_crop_preset: callbacks.on_remove_crop_preset.clone(),
                    on_add_crop_preset: callbacks.on_add_crop_preset.clone(),
                };
                close_btn.connect_clicked(move |_| {
                    let Some(picker) = picker_weak.upgrade() else {
                        return;
                    };
                    let Some(chips) = chips_weak.upgrade() else {
                        return;
                    };
                    (cb_clone.on_remove_crop_preset)(pi);
                    if let Some(idx) = ci.get() {
                        let project = (cb_clone.get_project)();
                        if let Some(page) = project.pages.get(idx) {
                            if page.crop_preset.is_none() {
                                picker.set_crop(None);
                            }
                        }
                    }
                    let project = (cb_clone.get_project)();
                    refresh_preset_chips(
                        &chips,
                        &project,
                        ci.clone(),
                        &picker,
                        &ov,
                        &sel,
                        &cb_clone,
                    );
                });
            }

            chip.append(&close_btn);
        }

        chip_box.append(&chip);
    }
    queue_all_overlays(overlays);
}

pub(crate) fn auto_detect_presets(
    project: &Project,
    page_store: &gio::ListStore,
    callbacks: &PresetCallbacks,
) {
    if !project.crop_presets.is_empty() {
        return;
    }
    let n = project.pages.len();
    if n == 0 {
        return;
    }
    let page_info: Vec<_> = (0..n)
        .map(|i| {
            let item = page_store.item(i as u32).and_downcast::<PageItem>();
            let rot = project
                .pages
                .get(i)
                .map(|p| p.rotation.as_degrees() % 360)
                .unwrap_or(0);
            (item, rot)
        })
        .collect();

    let mut dim_groups: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, (item, rot)) in page_info.iter().enumerate() {
        let Some(item) = item else { continue };
        let dims = {
            let w = item.base_width();
            let h = item.base_height();
            if w > 0 && h > 0 {
                if *rot == 90 || *rot == 270 {
                    Some((h, w))
                } else {
                    Some((w, h))
                }
            } else {
                None
            }
        };
        if let Some(d) = dims {
            dim_groups.entry(d).or_default().push(i);
        }
    }

    let mut sorted: Vec<((u32, u32), Vec<usize>)> = dim_groups.into_iter().collect();
    sorted.sort_by_key(|(_, idxs)| -(idxs.len() as i64));

    let base = project.crop_presets.len();
    for (pi, ((w, h), indices)) in (base..).zip(sorted.iter()) {
        (callbacks.on_add_crop_preset)(CropPreset {
            name: format!("Preset {}", pi + 1),
            w: *w,
            h: *h,
            locked: false,
        });
        for &idx in indices {
            (callbacks.on_set_crop_preset)(idx, Some(pi));
            (callbacks.on_set_crop)(
                idx,
                Some(CropBox {
                    x: 0,
                    y: 0,
                    w: *w,
                    h: *h,
                }),
            );
        }
    }
}
