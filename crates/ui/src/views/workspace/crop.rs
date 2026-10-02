//! Crop mode: the crop picker for the first selected page, crop presets
//! shown as chips, margin and bleed controls, page auto-detection, and the
//! crop rectangles drawn over every thumbnail.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::State;
use crate::widgets::crop_picker::{preset_color, CropPicker};
use crate::widgets::page_item::PageItem;
use recto_core::{Bleed, Command, CropBox, CropPreset, Project, Rect, Rotation};

use super::store::selected_positions;

pub(super) struct CropView {
    state: State,
    selection: gtk::MultiSelection,
    pub(super) controls: gtk::Box,
    pub(super) picker: Rc<CropPicker>,
    chips: gtk::Box,
    status: gtk::Label,
    margin_spin: gtk::SpinButton,
    /// The margin spin is relative: it starts at 0 for each page and every
    /// change grows or shrinks the preset by the difference.
    last_margin: Cell<f64>,
    /// The page the margin spin was last reset for.
    margin_page: Cell<Option<usize>>,
    bleed_spin: gtk::SpinButton,
    btn_auto: gtk::Button,
    /// Set while controls are moved to match the project, so their handlers
    /// don't dispatch the values straight back.
    syncing: Cell<bool>,
    /// Whether crop mode is showing (thumbnail overlays draw only then).
    active: Cell<bool>,
    /// Presets are seeded from page sizes the first time crop mode opens.
    presets_seeded: Cell<bool>,
    thumb_overlays: RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>,
}

impl CropView {
    pub(super) fn new(state: State, selection: gtk::MultiSelection) -> Rc<Self> {
        let chips = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .build();
        let presets_section = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .vexpand(true)
            .build();
        presets_section.append(
            &gtk::Label::builder()
                .label("Presets")
                .xalign(0.0)
                .css_classes(["heading", "caption"])
                .build(),
        );
        presets_section.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&chips)
                .vexpand(true)
                .build(),
        );

        let margin_spin = spin_button(
            gtk::Adjustment::new(0.0, -200.0, 200.0, 1.0, 10.0, 0.0),
            "Shrink or grow crop by this many pixels on all sides (applies to all pages in the preset)",
        );
        margin_spin.set_sensitive(false);
        let bleed_spin = spin_button(
            gtk::Adjustment::new(
                state.project().bleed.as_u32() as f64,
                0.0,
                2000.0,
                1.0,
                10.0,
                0.0,
            ),
            "Extra pixels kept around each crop in PDF export only. Hidden by the PDF \
             page crop box; does not affect image exports or the crop rectangles.",
        );

        let btn_auto = flat_button(
            "image-adjust-colors-symbolic",
            "Auto Detect",
            "Automatically detect page boundaries using OpenCV",
        );
        let btn_new = flat_button(
            "list-add-symbolic",
            "New Preset",
            "New preset from current crop",
        );
        let status = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .wrap(true)
            .build();

        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(10)
            .margin_end(10)
            .build();
        controls.append(&presets_section);
        controls.append(&labelled("Crop Margin", &margin_spin));
        controls.append(&labelled("PDF Bleed (px)", &bleed_spin));
        controls.append(&btn_auto);
        controls.append(&btn_new);

        let view = Rc::new(Self {
            state,
            selection,
            controls,
            picker: CropPicker::new(),
            chips,
            status,
            margin_spin,
            last_margin: Cell::new(0.0),
            margin_page: Cell::new(None),
            bleed_spin,
            btn_auto,
            syncing: Cell::new(false),
            active: Cell::new(false),
            presets_seeded: Cell::new(false),
            thumb_overlays: RefCell::new(Vec::new()),
        });

        let weak = Rc::downgrade(&view);
        let on = move |f: fn(&Rc<Self>)| {
            let weak = weak.clone();
            move || {
                if let Some(view) = weak.upgrade() {
                    f(&view);
                }
            }
        };
        view.picker.connect_changed(on(|v| v.refresh()));
        btn_new.connect_clicked({
            let f = on(|v| v.new_preset_from_current());
            move |_| f()
        });
        view.btn_auto.connect_clicked({
            let f = on(|v| v.run_autodetect());
            move |_| f()
        });
        view.margin_spin.connect_value_changed({
            let f = on(|v| v.on_margin_changed());
            move |_| f()
        });
        view.bleed_spin.connect_value_changed({
            let f = on(|v| v.on_bleed_changed());
            move |_| f()
        });
        view
    }

    /// The page shown in the picker: the first selected one.
    fn current_index(&self) -> Option<usize> {
        selected_positions(&self.selection).first().copied()
    }

    pub(super) fn activate(self: &Rc<Self>) {
        self.active.set(true);
        if !self.presets_seeded.replace(true) {
            seed_presets_from_sizes(&self.state);
        }
        if self.selection.selection().is_empty() && self.selection.n_items() > 0 {
            // Re-enters through the workspace's selection handler.
            self.selection.select_item(0, true);
        }
        self.refresh();
    }

    pub(super) fn deactivate(&self) {
        self.active.set(false);
        self.queue_thumb_overlays();
    }

    /// Bring everything in crop mode up to date with the project and the
    /// selection: picker, preset chips, status line, margin spin and the
    /// thumbnail overlays.
    pub(super) fn refresh(self: &Rc<Self>) {
        match self.current_index() {
            Some(idx) => self.picker.bind(self.state.clone(), idx),
            None => self.picker.clear(),
        }
        self.rebuild_chips();
        let idx = self.current_index();
        self.status
            .set_label(&status_text(&self.state.project(), idx));
        self.reset_margin_spin(idx);
        self.picker.queue_redraw();
        self.queue_thumb_overlays();
    }

    /// Move the controls to the project's values (load, undo, clear).
    pub(super) fn sync_from_project(&self) {
        let bleed = self.state.project().bleed.as_u32() as f64;
        self.syncing.set(true);
        self.bleed_spin.set_value(bleed);
        self.syncing.set(false);
    }

    /// Draw the page's crop over a grid thumbnail while crop mode is active.
    /// Called once per recycled card; the card's list item tells which page
    /// it currently shows.
    pub(super) fn attach_thumb_overlay(
        self: &Rc<Self>,
        da: &gtk::DrawingArea,
        item: &gtk::ListItem,
    ) {
        let view = Rc::downgrade(self);
        let item = item.downgrade();
        da.set_draw_func(move |_, cr, w, h| {
            let (Some(view), Some(item)) = (view.upgrade(), item.upgrade()) else {
                return;
            };
            let Some(page_item) = item.item().and_downcast::<PageItem>() else {
                return;
            };
            if view.active.get() && item.position() != gtk::INVALID_LIST_POSITION {
                draw_thumb_crop(
                    cr,
                    (w as f64, h as f64),
                    &view.state.project(),
                    item.position() as usize,
                    &page_item,
                );
            }
        });
        self.thumb_overlays.borrow_mut().push(da.downgrade());
    }

    fn queue_thumb_overlays(&self) {
        self.thumb_overlays
            .borrow_mut()
            .retain(|w| match w.upgrade() {
                Some(da) => {
                    da.queue_draw();
                    true
                }
                None => false,
            });
    }

    fn rebuild_chips(self: &Rc<Self>) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        let project = self.state.project();
        let current_preset = self
            .current_index()
            .and_then(|idx| project.pages.get(idx))
            .and_then(|p| p.crop_preset);
        for (i, preset) in project.crop_presets.iter().enumerate() {
            let in_use = project.pages.iter().any(|p| p.crop_preset == Some(i));
            self.chips
                .append(&self.make_chip(i, preset, current_preset == Some(i), in_use));
        }
    }

    fn make_chip(
        self: &Rc<Self>,
        i: usize,
        preset: &CropPreset,
        current: bool,
        in_use: bool,
    ) -> gtk::Box {
        let chip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["preset-chip", &format!("preset-c{}", i % 8)])
            .build();
        if current {
            chip.add_css_class("preset-active");
        }
        let pointer = gdk::Cursor::from_name("pointer", None);

        let label = gtk::Button::builder()
            .label(&preset.name)
            .tooltip_text(format!("{} ({}×{})", preset.name, preset.w, preset.h))
            .css_classes(["preset-label"])
            .build();
        label.set_cursor(pointer.as_ref());
        let weak = Rc::downgrade(self);
        label.connect_clicked(move |_| with(&weak, |v| v.apply_preset(i)));
        chip.append(&label);

        let (icon, tooltip) = if preset.locked {
            (
                "changes-prevent-symbolic",
                "Locked — drag won't change the preset size",
            )
        } else {
            (
                "changes-allow-symbolic",
                "Unlocked — drag updates the preset for all pages in this group",
            )
        };
        let lock = gtk::ToggleButton::builder()
            .active(preset.locked)
            .icon_name(icon)
            .tooltip_text(tooltip)
            .has_frame(false)
            .css_classes(["preset-lock"])
            .build();
        lock.set_cursor(pointer.as_ref());
        let weak = Rc::downgrade(self);
        lock.connect_toggled(move |btn| {
            let locked = btn.is_active();
            with(&weak, |v| {
                v.state
                    .dispatch(Command::SetCropPresetLocked { index: i, locked })
            });
        });
        chip.append(&lock);

        if !in_use {
            let close = gtk::Button::builder()
                .label("\u{00d7}")
                .has_frame(false)
                .tooltip_text("Delete this preset permanently")
                .css_classes(["preset-close"])
                .build();
            close.set_cursor(pointer.as_ref());
            let weak = Rc::downgrade(self);
            close.connect_clicked(move |_| {
                with(&weak, |v| v.state.dispatch(Command::RemoveCropPreset(i)))
            });
            chip.append(&close);
        }
        chip
    }

    /// Apply preset `pi` to every selected page: each keeps its crop's
    /// corner and is clamped to its own image.
    fn apply_preset(&self, pi: usize) {
        let crops: Vec<(usize, CropBox)> = {
            let project = self.state.project();
            let Some(preset) = project.crop_presets.get(pi) else {
                return;
            };
            selected_positions(&self.selection)
                .into_iter()
                .filter_map(|idx| {
                    let page = project.pages.get(idx)?;
                    let dims = recto_core::io::page_dimensions(page)?;
                    let crop = recto_core::resize_crop(page.crop, preset.w, preset.h, dims);
                    Some((idx, crop))
                })
                .collect()
        };
        self.state.batch(|| {
            for (index, crop) in crops {
                self.state.dispatch(Command::SetCropPreset {
                    index,
                    preset: Some(pi),
                });
                self.state.dispatch(Command::SetCrop {
                    index,
                    crop: Some(crop),
                });
            }
        });
    }

    fn new_preset_from_current(&self) {
        let Some(idx) = self.current_index() else {
            return;
        };
        let (w, h, pi) = {
            let project = self.state.project();
            let Some(page) = project.pages.get(idx) else {
                return;
            };
            let (w, h) = match page.crop {
                Some(c) => (c.w, c.h),
                None => {
                    let (iw, ih) = self.picker.image_dims();
                    (iw.max(1.0) as u32, ih.max(1.0) as u32)
                }
            };
            (w, h, project.crop_presets.len())
        };
        self.state.batch(|| {
            self.state.dispatch(Command::AddCropPreset(CropPreset {
                name: format!("Preset {}", pi + 1),
                w,
                h,
                locked: false,
            }));
            self.state.dispatch(Command::SetCropPreset {
                index: idx,
                preset: Some(pi),
            });
            self.state.dispatch(Command::SetCrop {
                index: idx,
                crop: Some(CropBox { x: 0, y: 0, w, h }),
            });
        });
    }

    /// The margin spin is enabled for pages in an unlocked preset, and goes
    /// back to 0 whenever another page is shown.
    fn reset_margin_spin(&self, idx: Option<usize>) {
        let unlocked_preset = {
            let project = self.state.project();
            idx.and_then(|i| project.pages.get(i))
                .and_then(|p| p.crop_preset)
                .and_then(|pi| project.crop_presets.get(pi))
                .is_some_and(|p| !p.locked)
        };
        self.margin_spin.set_sensitive(unlocked_preset);
        if self.margin_page.replace(idx) != idx {
            self.syncing.set(true);
            self.margin_spin.set_value(0.0);
            self.last_margin.set(0.0);
            self.syncing.set(false);
        }
    }

    /// Grow or shrink the current page's preset, and every page using it,
    /// by the change in margin on each side.
    fn on_margin_changed(&self) {
        if self.syncing.get() {
            return;
        }
        let margin = self.margin_spin.value();
        let delta = (margin - self.last_margin.replace(margin)) as i32;
        let Some(idx) = self.current_index() else {
            return;
        };
        let (pi, w, h, pages) = {
            let project = self.state.project();
            let Some(pi) = project.pages.get(idx).and_then(|p| p.crop_preset) else {
                return;
            };
            let Some(preset) = project.crop_presets.get(pi).filter(|p| !p.locked) else {
                return;
            };
            let w = (preset.w as i32 + delta * 2).max(1) as u32;
            let h = (preset.h as i32 + delta * 2).max(1) as u32;
            let pages: Vec<(usize, CropBox)> = project
                .pages
                .iter()
                .enumerate()
                .filter(|(_, p)| p.crop_preset == Some(pi))
                .map(|(i, p)| {
                    let dims = recto_core::io::page_dimensions(p).unwrap_or((u32::MAX, u32::MAX));
                    let crop = match p.crop {
                        Some(c) => recto_core::resize_crop_centered(c, w, h, dims),
                        None => recto_core::resize_crop(None, w, h, dims),
                    };
                    (i, crop)
                })
                .collect();
            (pi, w, h, pages)
        };
        if delta == 0 {
            return;
        }
        // Resizing the preset and every page using it is one undo step.
        self.state.batch(|| {
            self.state
                .dispatch(Command::SetCropPresetSize { index: pi, w, h });
            for (index, crop) in pages {
                self.state.dispatch(Command::SetCrop {
                    index,
                    crop: Some(crop),
                });
            }
        });
    }

    fn on_bleed_changed(&self) {
        if self.syncing.get() {
            return;
        }
        let bleed = Bleed::new(self.bleed_spin.value() as u32);
        if self.state.project().bleed != bleed {
            self.state.dispatch(Command::SetBleed(bleed));
        }
    }

    /// Detect the page in every image off the main thread, then replace the
    /// presets with one per cluster of similar pages, each page's crop being
    /// its cluster's median size centred on its own detection.
    fn run_autodetect(self: &Rc<Self>) {
        let pages: Vec<(usize, std::path::PathBuf, Rotation)> = self
            .state
            .project()
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.path.clone(), p.rotation))
            .collect();
        if pages.is_empty() {
            return;
        }
        self.set_detecting(true);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                let input: Vec<_> = pages
                    .iter()
                    .map(|(i, p, r)| (*i, p.as_path(), *r))
                    .collect();
                let results = recto_core::autodetect::detect_all_pages(&input);
                let detections: Vec<(usize, CropBox)> =
                    results.iter().map(|&(idx, cb, _, _)| (idx, cb)).collect();
                let clusters = recto_core::autodetect::cluster_by_similarity(&detections);
                (results, clusters)
            })
            .await;
            let Some(view) = weak.upgrade() else { return };
            view.set_detecting(false);
            match result {
                Ok((results, clusters)) if !results.is_empty() => {
                    view.apply_detections(results, &clusters)
                }
                Ok(_) => crate::window::show_toast("Auto-detection failed: no pages detected"),
                Err(_) => crate::window::show_toast("Auto-detection failed"),
            }
        });
    }

    fn set_detecting(&self, detecting: bool) {
        self.btn_auto.set_sensitive(!detecting);
        if let Some(content) = self.btn_auto.child().and_downcast::<adw::ButtonContent>() {
            content.set_label(if detecting {
                "Detecting…"
            } else {
                "Auto Detect"
            });
        }
    }

    fn apply_detections(
        &self,
        results: Vec<(usize, CropBox, u32, u32)>,
        clusters: &[recto_core::autodetect::CropCluster],
    ) {
        let detections: HashMap<usize, (CropBox, u32, u32)> = results
            .into_iter()
            .map(|(idx, cb, w, h)| (idx, (cb, w, h)))
            .collect();
        // Replacing the presets and every page's crop is one undo step.
        self.state.batch(|| {
            let n = self.state.project().crop_presets.len();
            for i in (0..n).rev() {
                self.state.dispatch(Command::RemoveCropPreset(i));
            }
            for (pi, cluster) in clusters.iter().enumerate() {
                self.state
                    .dispatch(Command::AddCropPreset(cluster.preset.clone()));
                for &index in &cluster.page_indices {
                    let Some(&(detection, w, h)) = detections.get(&index) else {
                        continue;
                    };
                    let crop = recto_core::centered_crop(
                        cluster.preset.w,
                        cluster.preset.h,
                        detection,
                        (w, h),
                    );
                    self.state.dispatch(Command::SetCropPreset {
                        index,
                        preset: Some(pi),
                    });
                    self.state.dispatch(Command::SetCrop {
                        index,
                        crop: Some(crop),
                    });
                }
            }
        });
        let pages: usize = clusters.iter().map(|c| c.page_indices.len()).sum();
        crate::window::show_toast(&format!(
            "Detected {} preset{} across {} page{}",
            clusters.len(),
            if clusters.len() == 1 { "" } else { "s" },
            pages,
            if pages == 1 { "" } else { "s" },
        ));
    }
}

/// Run `f` on the view if it is still alive.
fn with(view: &Weak<CropView>, f: impl FnOnce(&Rc<CropView>)) {
    if let Some(view) = view.upgrade() {
        f(&view);
    }
}

/// Give a project without presets one preset per distinct page size, each
/// covering its pages fully. Sizes come from the image headers, so this
/// works before thumbnails have loaded.
fn seed_presets_from_sizes(state: &State) {
    let sizes: Vec<(usize, (u32, u32))> = {
        let project = state.project();
        if !project.crop_presets.is_empty() {
            return;
        }
        project
            .pages
            .iter()
            .enumerate()
            .filter_map(|(i, page)| Some((i, recto_core::io::page_dimensions(page)?)))
            .collect()
    };
    if sizes.is_empty() {
        return;
    }
    state.batch(|| {
        for (pi, ((w, h), pages)) in recto_core::group_by_size(&sizes).into_iter().enumerate() {
            state.dispatch(Command::AddCropPreset(CropPreset {
                name: format!("Preset {}", pi + 1),
                w,
                h,
                locked: false,
            }));
            for index in pages {
                state.dispatch(Command::SetCropPreset {
                    index,
                    preset: Some(pi),
                });
                state.dispatch(Command::SetCrop {
                    index,
                    crop: Some(CropBox { x: 0, y: 0, w, h }),
                });
            }
        }
    });
}

fn status_text(project: &Project, index: Option<usize>) -> String {
    let Some((idx, page)) = index.and_then(|i| Some((i, project.pages.get(i)?))) else {
        return "No page selected".into();
    };
    let mut text = format!("Page {}/{}", idx + 1, project.pages.len());
    if let Some(p) = page.crop_preset.and_then(|pi| project.crop_presets.get(pi)) {
        let lock = if p.locked { " \u{1f512}" } else { " \u{1f513}" };
        text.push_str(&format!(" | Preset: {} ({}×{}{lock})", p.name, p.w, p.h));
    }
    if let Some(c) = page.crop {
        text.push_str(&format!(" | Crop: {}×{} @ ({}, {})", c.w, c.h, c.x, c.y));
    }
    text
}

/// Draw page `index`'s crop, in its preset colour, over its thumbnail drawn
/// to fit a `size` area.
fn draw_thumb_crop(
    cr: &gtk::cairo::Context,
    size: (f64, f64),
    project: &Project,
    index: usize,
    item: &PageItem,
) {
    let (width, height) = size;
    let Some(page) = project.pages.get(index) else {
        return;
    };
    let Some(crop) = page.crop.filter(|c| c.w > 0 && c.h > 0) else {
        return;
    };
    let (bw, bh) = (item.base_width() as f64, item.base_height() as f64);
    let (iw, ih) = match (bw > 0.0 && bh > 0.0, page.rotation.as_degrees() % 180) {
        (true, 90) => (bh, bw),
        (true, _) => (bw, bh),
        // Image size unknown yet: assume the crop reaches the image edges.
        (false, _) => ((crop.x + crop.w) as f64, (crop.y + crop.h) as f64),
    };
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let s = (width / iw).min(height / ih);
    let ox = (width - iw * s) / 2.0;
    let oy = (height - ih * s) / 2.0;
    let r = Rect::from(crop);
    let (red, green, blue) = page
        .crop_preset
        .map(preset_color)
        .unwrap_or((0.5, 0.5, 0.5));
    cr.rectangle(ox + r.x * s, oy + r.y * s, r.w * s, r.h * s);
    cr.set_source_rgba(red, green, blue, 0.25);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(red, green, blue, 0.8);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
}

fn spin_button(adjustment: gtk::Adjustment, tooltip: &str) -> gtk::SpinButton {
    gtk::SpinButton::builder()
        .adjustment(&adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .update_policy(gtk::SpinButtonUpdatePolicy::IfValid)
        .tooltip_text(tooltip)
        .build()
}

fn flat_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name(icon)
                .label(label)
                .build(),
        )
        .tooltip_text(tooltip)
        .css_classes(["flat"])
        .build()
}

fn labelled(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    section.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .css_classes(["caption"])
            .build(),
    );
    section.append(widget);
    section
}
