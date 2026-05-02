use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use super::picker::CropPicker;
use super::overlay::queue_all_overlays;
use super::super::page_item::PageItem;
use crate::app::{MarkDirty, State};
use recto_core::geometry::Rect;
use recto_core::project::{CropBox, CropPreset};

pub(crate) fn refresh_preset_chips(
    chip_box: &gtk::Box,
    state: &State,
    current_index: Rc<Cell<i32>>,
    picker: &Rc<CropPicker>,
    overlays: &Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>,
    selected_indices: &Rc<RefCell<Vec<usize>>>,
    mark_dirty: &MarkDirty,
) {
    while let Some(child) = chip_box.first_child() {
        chip_box.remove(&child);
    }

    let project = state.borrow();
    let n_presets = project.crop_presets.len();
    if n_presets == 0 {
        return;
    }

    let current_preset: Option<usize> = {
        let idx = current_index.get();
        if idx >= 0 {
            project.pages.get(idx as usize).and_then(|p| p.crop_preset)
        } else {
            None
        }
    };

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
            let state = state.clone();
            let ci = current_index.clone();
            let picker_weak = Rc::downgrade(picker);
            let chips_weak = chip_box.downgrade();
            let sel = selected_indices.clone();
            let ov = overlays.clone();
            let mark_dirty = mark_dirty.clone();
            let pi = i;
            let pw = preset.w;
            let ph = preset.h;
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
                {
                    let mut project = state.borrow_mut();
                    for &idx in &targets {
                        let Some(page) = project.pages.get_mut(idx) else {
                            continue;
                        };
                        page.crop_preset = Some(pi);
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
                        page.crop = Some(cb);
                        if new_first.is_none() {
                            new_first = Some(Rect::from(cb));
                        }
                    }
                }
                if let Some(r) = new_first {
                    picker.set_crop(Some(r));
                }
                refresh_preset_chips(&chips, &state, ci.clone(), &picker, &ov, &sel, &mark_dirty);
                mark_dirty();
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
            let state = state.clone();
            let chips_weak = chip_box.downgrade();
            let ci = current_index.clone();
            let picker_weak = Rc::downgrade(picker);
            let ov = overlays.clone();
            let sel = selected_indices.clone();
            let mark_dirty = mark_dirty.clone();
            let pi = i;
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
                let mut project = state.borrow_mut();
                if let Some(p) = project.crop_presets.get_mut(pi) {
                    p.locked = locked;
                }
                drop(project);
                if let (Some(picker), Some(chips)) = (picker_weak.upgrade(), chips_weak.upgrade()) {
                    refresh_preset_chips(
                        &chips,
                        &state,
                        ci.clone(),
                        &picker,
                        &ov,
                        &sel,
                        &mark_dirty,
                    );
                }
                mark_dirty();
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
                let state = state.clone();
                let chips_weak = chip_box.downgrade();
                let ci = current_index.clone();
                let picker_weak = Rc::downgrade(picker);
                let ov = overlays.clone();
                let sel = selected_indices.clone();
                let mark_dirty = mark_dirty.clone();
                let pi = i;
                close_btn.connect_clicked(move |_| {
                    let Some(picker) = picker_weak.upgrade() else {
                        return;
                    };
                    let Some(chips) = chips_weak.upgrade() else {
                        return;
                    };
                    delete_preset(&state, pi);
                    {
                        let project = state.borrow();
                        let idx = ci.get();
                        if idx >= 0 {
                            if let Some(page) = project.pages.get(idx as usize) {
                                if page.crop_preset.is_none() {
                                    drop(project);
                                    picker.set_crop(None);
                                }
                            }
                        }
                    }
                    refresh_preset_chips(
                        &chips,
                        &state,
                        ci.clone(),
                        &picker,
                        &ov,
                        &sel,
                        &mark_dirty,
                    );
                    mark_dirty();
                });
            }

            chip.append(&close_btn);
        }

        chip_box.append(&chip);
    }
    queue_all_overlays(overlays);
}

fn delete_preset(state: &State, pi: usize) {
    let mut project = state.borrow_mut();
    if pi >= project.crop_presets.len() {
        return;
    }
    project.crop_presets.remove(pi);
    for page in project.pages.iter_mut() {
        match page.crop_preset {
            Some(p) if p == pi => {
                page.crop_preset = None;
                page.crop = None;
            }
            Some(p) if p > pi => page.crop_preset = Some(p - 1),
            _ => {}
        }
    }
    for (i, preset) in project.crop_presets.iter_mut().enumerate() {
        preset.name = format!("Preset {}", i + 1);
    }
}

pub(crate) fn auto_detect_presets(state: &State, page_store: &gio::ListStore) {
    let mut project = state.borrow_mut();
    if !project.crop_presets.is_empty() {
        return;
    }
    let n = project.pages.len();
    if n == 0 {
        return;
    }

    let mut dim_groups: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for i in 0..n {
        let item = match page_store.item(i as u32).and_downcast::<PageItem>() {
            Some(it) => it,
            None => continue,
        };
        let dims = {
            let w = item.base_width();
            let h = item.base_height();
            if w > 0 && h > 0 {
                let rot = project.pages.get(i).map(|p| p.rotation % 360).unwrap_or(0);
                if rot == 90 || rot == 270 {
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

    for ((w, h), indices) in &sorted {
        let pi = project.crop_presets.len();
        project.crop_presets.push(CropPreset {
            name: format!("Preset {}", pi + 1),
            w: *w,
            h: *h,
            locked: false,
        });
        for &idx in indices {
            if let Some(page) = project.pages.get_mut(idx) {
                page.crop_preset = Some(pi);
                page.crop = Some(CropBox {
                    x: 0,
                    y: 0,
                    w: *w,
                    h: *h,
                });
            }
        }
    }
}
