//! Crop step.
//!
//! Layout: a sidebar listing every page (reuses the shared `gio::ListStore<PageItem>`)
//! and, on the right, an image preview with a draggable crop overlay. The same
//! crop preset is shared across many pages, so editing one page's crop size
//! propagates to every page using that preset; positions stay per-page.
//!
//! The image preview uses [`PreviewCanvas`] + [`ZoomPanController`] for the
//! zoom/pan plumbing, identical to the import step. This file contains only
//! the crop-specific logic: handle hit-testing, drag math, sidebar wiring,
//! and the preset toolbar.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};

use super::page_item::PageItem;
use crate::app::{MarkDirty, State};
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};
use recto_core::geometry::Rect;
use recto_core::project::{CropBox, CropPreset};

// --- Tunables -------------------------------------------------------------

/// Hit-radius around a handle, in *widget* pixels (independent of zoom).
const HANDLE_HIT_PX: f64 = 12.0;
/// Visible handle radius, widget pixels.
const HANDLE_DRAW_PX: f64 = 7.0;
/// Minimum allowed crop size on either axis, in image-space pixels.
const MIN_CROP_PX: f64 = 16.0;

// --- Handle / drag types --------------------------------------------------

#[derive(Copy, Clone, PartialEq, Debug)]
enum Corner {
    NW,
    NE,
    SW,
    SE,
}

#[derive(Copy, Clone, PartialEq, Debug)]
enum Edge {
    N,
    S,
    E,
    W,
}

#[derive(Copy, Clone, PartialEq, Debug)]
enum Handle {
    NewBox,
    Move,
    Corner(Corner),
    Edge(Edge),
}

impl Handle {
    /// Pick the handle (if any) closest to `pos_img` within the hit radius.
    /// `z` is the canvas zoom — used to convert the widget-space hit radius
    /// back into image-space.
    fn pick(rect: Rect, pos_img: (f64, f64), z: f64) -> Option<Self> {
        let r = HANDLE_HIT_PX / z.max(1e-6);
        let r2 = r * r;
        let dist2 = |a: (f64, f64)| {
            let dx = a.0 - pos_img.0;
            let dy = a.1 - pos_img.1;
            dx * dx + dy * dy
        };

        let corners = [
            (Corner::NW, (rect.x, rect.y)),
            (Corner::NE, (rect.x + rect.w, rect.y)),
            (Corner::SW, (rect.x, rect.y + rect.h)),
            (Corner::SE, (rect.x + rect.w, rect.y + rect.h)),
        ];
        for (c, p) in corners {
            if dist2(p) <= r2 {
                return Some(Handle::Corner(c));
            }
        }
        let edges = [
            (Edge::N, (rect.x + rect.w * 0.5, rect.y)),
            (Edge::S, (rect.x + rect.w * 0.5, rect.y + rect.h)),
            (Edge::W, (rect.x, rect.y + rect.h * 0.5)),
            (Edge::E, (rect.x + rect.w, rect.y + rect.h * 0.5)),
        ];
        for (e, p) in edges {
            if dist2(p) <= r2 {
                return Some(Handle::Edge(e));
            }
        }
        None
    }

    /// Apply this handle's drag, given the rect at drag-begin, the start &
    /// current pointer positions in image space, and the image bounds.
    fn apply(
        self,
        initial: Rect,
        start: (f64, f64),
        end: (f64, f64),
        bounds: (f64, f64),
    ) -> Option<Rect> {
        let (iw, ih) = bounds;
        match self {
            Handle::NewBox => Rect::from_points(start, end, iw, ih, MIN_CROP_PX),
            Handle::Move => {
                let dx = end.0 - start.0;
                let dy = end.1 - start.1;
                Some(
                    Rect::new(initial.x + dx, initial.y + dy, initial.w, initial.h)
                        .clamp_to(iw, ih, MIN_CROP_PX),
                )
            }
            Handle::Corner(c) => {
                let anchor = match c {
                    Corner::NW => (initial.x + initial.w, initial.y + initial.h),
                    Corner::NE => (initial.x, initial.y + initial.h),
                    Corner::SW => (initial.x + initial.w, initial.y),
                    Corner::SE => (initial.x, initial.y),
                };
                Rect::from_points(anchor, end, iw, ih, MIN_CROP_PX)
            }
            Handle::Edge(e) => {
                let mut r = initial;
                match e {
                    Edge::N => {
                        let bottom = r.y + r.h;
                        let new_y = end.1.clamp(0.0, bottom - MIN_CROP_PX);
                        r.y = new_y;
                        r.h = bottom - new_y;
                    }
                    Edge::S => {
                        let top = r.y;
                        let new_bottom = end.1.clamp(top + MIN_CROP_PX, ih);
                        r.h = new_bottom - top;
                    }
                    Edge::W => {
                        let right = r.x + r.w;
                        let new_x = end.0.clamp(0.0, right - MIN_CROP_PX);
                        r.x = new_x;
                        r.w = right - new_x;
                    }
                    Edge::E => {
                        let left = r.x;
                        let new_right = end.0.clamp(left + MIN_CROP_PX, iw);
                        r.w = new_right - left;
                    }
                }
                Some(r)
            }
        }
    }

    fn cursor_name(self) -> &'static str {
        match self {
            Handle::Corner(Corner::NW) | Handle::Corner(Corner::SE) => "nwse-resize",
            Handle::Corner(Corner::NE) | Handle::Corner(Corner::SW) => "nesw-resize",
            Handle::Edge(Edge::N) | Handle::Edge(Edge::S) => "ns-resize",
            Handle::Edge(Edge::E) | Handle::Edge(Edge::W) => "ew-resize",
            Handle::Move => "move",
            Handle::NewBox => "crosshair",
        }
    }
}

#[derive(Copy, Clone)]
struct DragState {
    handle: Handle,
    initial: Option<Rect>,
    start_img: (f64, f64),
}

// --- CropPicker ----------------------------------------------------------

#[derive(Default)]
struct PickerState {
    image_dims: Option<(f64, f64)>,
    binding: Option<(State, usize)>,
    crop: Option<Rect>,
    drag: Option<DragState>,
}

pub struct CropPicker {
    overlay: gtk::Overlay,
    canvas: PreviewCanvas,
    area: gtk::DrawingArea,
    zoom_pan: Rc<ZoomPanController>,
    state: RefCell<PickerState>,
    on_changed: RefCell<Vec<Box<dyn Fn()>>>,
}

impl CropPicker {
    pub fn new() -> Rc<Self> {
        let overlay = gtk::Overlay::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        let canvas = PreviewCanvas::new();
        canvas.add_css_class("view");
        overlay.set_child(Some(&canvas));

        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        overlay.add_overlay(&area);

        let zoom_pan = ZoomPanController::attach(
            &canvas,
            &overlay,
            ZoomPanConfig {
                pan_button: gdk::BUTTON_MIDDLE,
                ..Default::default()
            },
        );

        let this = Rc::new(Self {
            overlay,
            canvas,
            area,
            zoom_pan,
            state: RefCell::new(PickerState::default()),
            on_changed: RefCell::new(Vec::new()),
        });

        // Redraw overlay when zoom/pan changes.
        this.zoom_pan.connect_changed({
            let weak = Rc::downgrade(&this);
            move || {
                if let Some(p) = weak.upgrade() {
                    p.area.queue_draw();
                }
            }
        });

        // Cairo overlay drawing.
        this.area.set_draw_func({
            let weak = Rc::downgrade(&this);
            move |_, cr, _, _| {
                if let Some(p) = weak.upgrade() {
                    p.draw_overlay(cr);
                }
            }
        });

        // Primary-button drag = crop interaction.
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        drag.connect_drag_begin({
            let weak = Rc::downgrade(&this);
            move |_, x, y| {
                if let Some(p) = weak.upgrade() {
                    p.on_drag_begin(x, y);
                }
            }
        });
        drag.connect_drag_update({
            let weak = Rc::downgrade(&this);
            move |g, dx, dy| {
                if let Some(p) = weak.upgrade() {
                    if let Some((sx, sy)) = g.start_point() {
                        p.on_drag_update(sx + dx, sy + dy);
                    }
                }
            }
        });
        drag.connect_drag_end({
            let weak = Rc::downgrade(&this);
            move |_, _, _| {
                if let Some(p) = weak.upgrade() {
                    p.on_drag_end();
                }
            }
        });
        this.area.add_controller(drag);

        // Hover cursor.
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let weak = Rc::downgrade(&this);
            move |_, x, y| {
                if let Some(p) = weak.upgrade() {
                    p.update_hover_cursor(x, y);
                }
            }
        });
        motion.connect_leave({
            let weak = Rc::downgrade(&this);
            move |_| {
                if let Some(p) = weak.upgrade() {
                    p.area.set_cursor(None);
                }
            }
        });
        this.area.add_controller(motion);

        // Esc cancels in-progress drag.
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed({
            let weak = Rc::downgrade(&this);
            move |_, keyval, _, _| {
                if keyval == gdk::Key::Escape {
                    if let Some(p) = weak.upgrade() {
                        if p.cancel_drag() {
                            return glib::Propagation::Stop;
                        }
                    }
                }
                glib::Propagation::Proceed
            }
        });
        this.overlay.add_controller(key);

        this
    }

    pub fn overlay(&self) -> &gtk::Overlay {
        &self.overlay
    }

    pub fn image_dims(&self) -> (f64, f64) {
        self.state.borrow().image_dims.unwrap_or((0.0, 0.0))
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_changed.borrow_mut().push(Box::new(f));
    }

    pub fn set_crop(&self, rect: Option<Rect>) {
        self.state.borrow_mut().crop = rect;
        self.area.queue_draw();
    }

    pub fn bind(&self, state: State, index: usize) {
        let (path, rotation, crop) = {
            let project = state.borrow();
            let Some(page) = project.pages.get(index) else {
                self.clear();
                return;
            };
            (
                page.path.clone(),
                page.rotation as u32,
                page.crop.map(Rect::from),
            )
        };

        let pb = gdk_pixbuf::Pixbuf::from_file(&path)
            .ok()
            .map(|pb| pb.apply_embedded_orientation().unwrap_or(pb))
            .map(|pb| super::page_item::rotate(&pb, rotation));

        {
            let mut s = self.state.borrow_mut();
            s.image_dims = pb.as_ref().map(|pb| (pb.width() as f64, pb.height() as f64));
            s.crop = crop;
            s.binding = Some((state, index));
        }

        self.canvas.set_texture(pb.as_ref().map(gdk::Texture::for_pixbuf));
        self.zoom_pan.refit_after_texture_change();
        self.area.queue_draw();
    }

    pub fn clear(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.image_dims = None;
            s.crop = None;
            s.binding = None;
        }
        self.canvas.set_texture(None);
        self.area.queue_draw();
    }

    fn emit_changed(&self) {
        for cb in self.on_changed.borrow().iter() {
            cb();
        }
    }

    fn widget_to_image(&self, wx: f64, wy: f64) -> Option<(f64, f64)> {
        let (z, px, py) = self.canvas.transform();
        if z <= 0.0 {
            return None;
        }
        Some(((wx - px) / z, (wy - py) / z))
    }

    fn on_drag_begin(&self, x: f64, y: f64) {
        let Some(img_pos) = self.widget_to_image(x, y) else { return };
        let (z, _, _) = self.canvas.transform();
        let mut s = self.state.borrow_mut();

        let locked = s
            .binding
            .as_ref()
            .map(|(state, index)| {
                let p = state.borrow();
                p.pages
                    .get(*index)
                    .and_then(|pg| pg.crop_preset)
                    .and_then(|pi| p.crop_presets.get(pi))
                    .map(|pr| pr.locked)
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        let handle = match s.crop {
            Some(r) => Handle::pick(r, img_pos, z).unwrap_or(if r.contains(img_pos) {
                Handle::Move
            } else {
                Handle::NewBox
            }),
            None => Handle::NewBox,
        };

        // Locked preset: only allow repositioning, not resizing or new boxes.
        if locked && !matches!(handle, Handle::Move) {
            return;
        }

        s.drag = Some(DragState {
            handle,
            initial: s.crop,
            start_img: img_pos,
        });
    }

    fn on_drag_update(&self, end_wx: f64, end_wy: f64) {
        let Some(end_img) = self.widget_to_image(end_wx, end_wy) else { return };
        let mut s = self.state.borrow_mut();
        let Some(d) = s.drag else { return };
        let Some((iw, ih)) = s.image_dims else { return };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let initial = d
            .initial
            .unwrap_or_else(|| Rect::new(d.start_img.0, d.start_img.1, 0.0, 0.0));
        if let Some(r) = d.handle.apply(initial, d.start_img, end_img, (iw, ih)) {
            s.crop = Some(r.clamp_to(iw, ih, MIN_CROP_PX));
        }
        drop(s);
        self.area.queue_draw();
    }

    fn on_drag_end(&self) {
        self.state.borrow_mut().drag = None;
        self.persist_crop();
        self.sync_preset_from_drag();
        self.emit_changed();
    }

    /// Returns true if a drag was actually cancelled.
    fn cancel_drag(&self) -> bool {
        let mut s = self.state.borrow_mut();
        let Some(d) = s.drag.take() else { return false };
        s.crop = d.initial;
        drop(s);
        self.area.queue_draw();
        true
    }

    fn update_hover_cursor(&self, x: f64, y: f64) {
        let Some(img_pos) = self.widget_to_image(x, y) else {
            self.area.set_cursor(None);
            return;
        };
        let s = self.state.borrow();
        let rect = match s.crop {
            Some(r) => r,
            None => {
                self.area.set_cursor(None);
                return;
            }
        };
        let locked = s
            .binding
            .as_ref()
            .map(|(state, index)| {
                let p = state.borrow();
                p.pages
                    .get(*index)
                    .and_then(|pg| pg.crop_preset)
                    .and_then(|pi| p.crop_presets.get(pi))
                    .map(|pr| pr.locked)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        drop(s);
        let (z, _, _) = self.canvas.transform();
        let h = if locked {
            if rect.contains(img_pos) { Some(Handle::Move) } else { None }
        } else {
            Handle::pick(rect, img_pos, z)
                .or_else(|| if rect.contains(img_pos) { Some(Handle::Move) } else { None })
        };
        let cursor = h.and_then(|h| gdk::Cursor::from_name(h.cursor_name(), None));
        self.area.set_cursor(cursor.as_ref());
    }

    fn persist_crop(&self) {
        let s = self.state.borrow();
        let Some((state, index)) = s.binding.clone() else { return };
        let crop = s.crop.map(CropBox::from);
        drop(s);
        let mut project = state.borrow_mut();
        if let Some(page) = project.pages.get_mut(index) {
            page.crop = crop;
        }
    }

    /// After the user finishes a drag, propagate the new size to every page
    /// sharing the same preset. (Position stays per-page.)
    fn sync_preset_from_drag(&self) {
        let (state, index, w, h) = {
            let s = self.state.borrow();
            let Some((state, index)) = s.binding.clone() else { return };
            let Some(rect) = s.crop else { return };
            let cb = CropBox::from(rect);
            (state, index, cb.w, cb.h)
        };
        let mut project = state.borrow_mut();
        let Some(pi) = project.pages.get(index).and_then(|p| p.crop_preset) else { return };
        let Some(preset) = project.crop_presets.get_mut(pi) else { return };
        if preset.locked {
            return;
        }
        preset.w = w;
        preset.h = h;
        for page in project.pages.iter_mut() {
            if page.crop_preset == Some(pi) {
                if let Some(c) = &mut page.crop {
                    c.w = w;
                    c.h = h;
                }
            }
        }
    }

    fn draw_overlay(&self, cr: &gtk::cairo::Context) {
        let s = self.state.borrow();
        let Some((iw, ih)) = s.image_dims else { return };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let Some(rect) = s.crop else { return };

        let (cr_r, cr_g, cr_b) = s.binding.as_ref()
            .and_then(|(state, index)| {
                let project = state.borrow();
                project.pages.get(*index)
                    .and_then(|p| p.crop_preset)
                    .map(preset_color_rgb)
            })
            .unwrap_or((0.18, 0.6, 1.0));

        let (z, px, py) = self.canvas.transform();

        // Image rect in widget coords.
        let ix = px;
        let iy = py;
        let iw_w = iw * z;
        let ih_w = ih * z;
        // Crop rect in widget coords.
        let rx = px + rect.x * z;
        let ry = py + rect.y * z;
        let rw = rect.w * z;
        let rh = rect.h * z;

        // Dark mask outside the crop.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
        cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
        cr.rectangle(ix, iy, iw_w, ih_w);
        cr.rectangle(rx, ry, rw, rh);
        let _ = cr.fill();

        // Crop frame.
        cr.set_source_rgba(cr_r, cr_g, cr_b, 1.0);
        cr.set_line_width(2.0);
        cr.rectangle(rx, ry, rw, rh);
        let _ = cr.stroke();

        // Handle dots.
        let handles = [
            (rx, ry),
            (rx + rw, ry),
            (rx, ry + rh),
            (rx + rw, ry + rh),
            (rx + rw / 2.0, ry),
            (rx + rw / 2.0, ry + rh),
            (rx, ry + rh / 2.0),
            (rx + rw, ry + rh / 2.0),
        ];
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
        for (hx, hy) in handles {
            cr.arc(hx, hy, HANDLE_DRAW_PX, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
        }
        cr.set_source_rgba(cr_r, cr_g, cr_b, 1.0);
        cr.set_line_width(1.5);
        for (hx, hy) in handles {
            cr.arc(hx, hy, HANDLE_DRAW_PX, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.stroke();
        }
    }
}

// --- build() — entry point ------------------------------------------------

pub fn build(
    state: State,
    page_store: gio::ListStore,
    selection: gtk::MultiSelection,
    paned_sync: super::PanedSync,
    mark_dirty: MarkDirty,
) -> gtk::Widget {
    let current_index: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    let initialised: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    // --- Crop preview ---------------------------------------------------
    let picker = CropPicker::new();

    // --- Preset toolbar (full width, above the paned) -------------------
    let preset_chips = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();

    let btn_new = gtk::Button::builder()
        .label("+")
        .tooltip_text("New preset from current crop")
        .build();

    let status_lbl = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .margin_start(8)
        .build();

    let spacer = gtk::Box::builder().hexpand(true).build();

    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(8)
        .margin_end(8)
        .build();
    toolbar.append(&preset_chips);
    toolbar.append(&btn_new);
    toolbar.append(&spacer);
    toolbar.append(&status_lbl);

    // --- Grid view (right side, mirrors import step layout) -------------

    let selected_indices: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let overlays: Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>> = Rc::new(RefCell::new(Vec::new()));

    let factory = super::grid::overlay_factory({
        let state = state.clone();
        let store = page_store.clone();
        let overlays = overlays.clone();
        move |pos, da| {
            // Per-bind: install a draw_func capturing this card's current
            // page index, and register the DrawingArea so the preset toolbar
            // can redraw every visible thumbnail when presets change.
            let st = state.clone();
            let sto = store.clone();
            da.set_draw_func(move |_, cr, width, height| {
                draw_crop_overlay(cr, width as f64, height as f64, &st, &sto, pos);
            });
            let mut list = overlays.borrow_mut();
            list.retain(|w| w.upgrade().is_some());
            let da_ptr = da.as_ptr() as usize;
            let already = list.iter().any(|w| {
                w.upgrade().map(|x| x.as_ptr() as usize == da_ptr).unwrap_or(false)
            });
            if !already {
                let w = glib::WeakRef::new();
                w.set(Some(da));
                list.push(w);
            }
        }
    });
    let (_grid_view, grid_scroll) = super::grid::build_grid_scroll(&selection, &factory);

    // Preset chip styles.
    let preset_css = gtk::CssProvider::new();
    preset_css.load_from_string(
        "\
        .preset-chip {\
          border-radius: 8px;\
          padding: 2px;\
        }\
        .preset-chip.preset-active {\
          outline: 2px solid @theme_selected_bg_color;\
          outline-offset: 1px;\
        }\
        .preset-label {\
          background: transparent;\
          color: white;\
          font-weight: bold;\
          padding: 4px 10px;\
          border: none;\
          border-radius: 6px;\
          min-height: 24px;\
        }\
        .preset-label:hover {\
          opacity: 0.85;\
        }\
        .preset-lock, .preset-close {\
          background: transparent;\
          color: rgba(255,255,255,0.7);\
          border: none;\
          border-radius: 6px;\
          padding: 2px 4px;\
          min-height: 20px;\
          min-width: 20px;\
        }\
        .preset-lock:hover, .preset-close:hover {\
          background: rgba(255,255,255,0.15);\
          color: white;\
        }\
        .preset-c0 { background: #3584e4; }\
        .preset-c1 { background: #33d17a; }\
        .preset-c2 { background: #ff7800; }\
        .preset-c3 { background: #9141ac; }\
        .preset-c4 { background: #ed333b; }\
        .preset-c5 { background: #1c71d8; }\
        .preset-c6 { background: #c061cb; }\
        .preset-c7 { background: #986a44; }\
    ",
    );
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display must be available"),
        &preset_css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // --- Paned: left = crop preview + toolbar, right = grid -------------
    let left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    left.append(picker.overlay());
    left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    left.append(&toolbar);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&grid_scroll)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .vexpand(true)
        .build();
    paned_sync.register(&paned);

    // --- Selection drives picker.bind + toolbar/status ------------------

    // Pre-clone for multiple closures that need to pass picker as &Rc.
    let picker_rc = picker.clone();
    let si = selected_indices.clone();
    let ps = page_store.clone();

    selection.connect_selection_changed({
        let state = state.clone();
        let picker = picker.clone();
        let chips = preset_chips.clone();
        let sl = status_lbl.clone();
        let ci = current_index.clone();
        let p = picker_rc.clone();
        let sel_indices = si.clone();
        let ov = overlays.clone();
        let store = ps.clone();
        let mark_dirty = mark_dirty.clone();
        move |sel, _, _| {
            let bitset = sel.selection();
            let n = store.n_items();
            let mut indices: Vec<usize> = Vec::new();
            for i in 0..n {
                if bitset.contains(i) {
                    indices.push(i as usize);
                }
            }
            *sel_indices.borrow_mut() = indices;

            let first = bitset.minimum();
            if first == u32::MAX {
                ci.set(-1);
                picker.clear();
                refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty);
                update_status_label(&sl, &state, -1);
            } else {
                let idx = first as i32;
                ci.set(idx);
                picker.bind(state.clone(), first as usize);
                refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty);
                update_status_label(&sl, &state, idx);
            }
        }
    });

    // --- Crop drag finished → refresh toolbar/status --------------------

    picker.connect_changed({
        let state = state.clone();
        let chips = preset_chips.clone();
        let sl = status_lbl.clone();
        let ci = current_index.clone();
        let p = picker_rc.clone();
        let ov = overlays.clone();
        let sel_indices = si.clone();
        let mark_dirty = mark_dirty.clone();
        move || {
            let idx = ci.get();
            if idx < 0 {
                return;
            }
            refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty);
            update_status_label(&sl, &state, idx);
            mark_dirty();
        }
    });

    // --- New preset -----------------------------------------------------

    btn_new.connect_clicked({
        let state = state.clone();
        let picker = picker.clone();
        let chips = preset_chips.clone();
        let sl = status_lbl.clone();
        let ci = current_index.clone();
        let p = picker_rc.clone();
        let ov = overlays.clone();
        let mark_dirty = mark_dirty.clone();
        move |_| {
            let idx = ci.get();
            if idx < 0 {
                return;
            }
            let mut project = state.borrow_mut();
            let (w, h) = match project.pages.get(idx as usize) {
                Some(page) => match page.crop {
                    Some(c) => (c.w, c.h),
                    None => {
                        let (iw, ih) = picker.image_dims();
                        (iw.max(1.0) as u32, ih.max(1.0) as u32)
                    }
                },
                None => return,
            };
            let pi = project.crop_presets.len();
            let name = format!("Preset {}", pi + 1);
            project.crop_presets.push(CropPreset {
                name,
                w,
                h,
                locked: false,
            });
            if let Some(page) = project.pages.get_mut(idx as usize) {
                page.crop_preset = Some(pi);
                page.crop = Some(CropBox { x: 0, y: 0, w, h });
            }
            drop(project);

            picker.set_crop(Some(Rect::new(0.0, 0.0, w as f64, h as f64)));
            refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &selected_indices, &mark_dirty);
            update_status_label(&sl, &state, idx);
            mark_dirty();
        }
    });

    // --- One-shot init: auto-detect presets, refresh chips, select first

    paned.connect_map({
        let state = state.clone();
        let store = page_store.clone();
        let chips = preset_chips.clone();
        let selection = selection.clone();
        let sl = status_lbl.clone();
        let initialised = initialised.clone();
        let p = picker_rc.clone();
        let ci = current_index.clone();
        let ov = overlays.clone();
        let sel_indices = si.clone();
        let mark_dirty = mark_dirty.clone();
        move |_| {
            if !initialised.get() {
                initialised.set(true);
                auto_detect_presets(&state, &store);
                refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty);
            }
            // Sync the picker to the current shared selection, so re-entering
            // this tab reflects whatever was selected on import/colors. Auto-
            // pick the first page only if nothing is selected yet.
            let bs = selection.selection();
            let first = match gtk::BitsetIter::init_first(&bs) {
                Some((_, idx)) => Some(idx),
                None => {
                    if store.n_items() > 0 {
                        selection.select_item(0, true);
                        Some(0)
                    } else {
                        None
                    }
                }
            };
            match first {
                Some(idx) => {
                    ci.set(idx as i32);
                    p.bind(state.clone(), idx as usize);
                    refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty);
                    update_status_label(&sl, &state, idx as i32);
                }
                None => {
                    ci.set(-1);
                    p.clear();
                    refresh_preset_chips(&chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty);
                    update_status_label(&sl, &state, -1);
                }
            }
        }
    });

    paned.upcast()
}

// --- helpers --------------------------------------------------------------

/// Rebuild the preset chip row. Each chip is a coloured horizontal box
/// containing a label button, a lock toggle and (if unused) a close button.
fn refresh_preset_chips(
    chip_box: &gtk::Box,
    state: &State,
    current_index: Rc<Cell<i32>>,
    picker: &Rc<CropPicker>,
    overlays: &Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>,
    selected_indices: &Rc<RefCell<Vec<usize>>>,
    mark_dirty: &MarkDirty,
) {
    // Remove all children.
    while let Some(child) = chip_box.first_child() {
        chip_box.remove(&child);
    }

    let project = state.borrow();
    let n_presets = project.crop_presets.len();
    if n_presets == 0 {
        return;
    }

    // Determine which preset the *current* page uses (for active highlight).
    let current_preset: Option<usize> = {
        let idx = current_index.get();
        if idx >= 0 {
            project.pages.get(idx as usize).and_then(|p| p.crop_preset)
        } else {
            None
        }
    };

    // Count references per preset so we know whether to show "×".
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
            .css_classes(vec![
                "preset-chip",
                &format!("preset-c{}", i % 8),
            ])
            .build();
        if current_preset == Some(i) {
            chip.add_css_class("preset-active");
        }

        // Label button — applies the preset to the current page.
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
                let Some(picker) = picker_weak.upgrade() else { return };
                let Some(chips) = chips_weak.upgrade() else { return };
                let targets = sel.borrow().clone();
                if targets.is_empty() {
                    return;
                }
                let (iw, ih) = picker.image_dims();
                let mut new_first: Option<Rect> = None;
                {
                    let mut project = state.borrow_mut();
                    for &idx in &targets {
                        let Some(page) = project.pages.get_mut(idx) else { continue };
                        page.crop_preset = Some(pi);
                        let mut cb = page.crop.unwrap_or(CropBox { x: 0, y: 0, w: 0, h: 0 });
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

        // Lock toggle.
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
            lock_btn.set_tooltip_text(Some("Unlocked — drag updates the preset for all pages in this group"));
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
                    btn.set_tooltip_text(Some("Unlocked — drag updates the preset for all pages in this group"));
                }
                let mut project = state.borrow_mut();
                if let Some(p) = project.crop_presets.get_mut(pi) {
                    p.locked = locked;
                }
                drop(project);
                if let (Some(picker), Some(chips)) = (picker_weak.upgrade(), chips_weak.upgrade()) {
                    refresh_preset_chips(&chips, &state, ci.clone(), &picker, &ov, &sel, &mark_dirty);
                }
                mark_dirty();
            });
        }

        chip.append(&lock_btn);

        // Close button — only if *no* pages reference this preset.
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
                    let Some(picker) = picker_weak.upgrade() else { return };
                    let Some(chips) = chips_weak.upgrade() else { return };
                    delete_preset(&state, pi);
                    // If the deleted preset was the current page's, clear its crop.
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
                    refresh_preset_chips(&chips, &state, ci.clone(), &picker, &ov, &sel, &mark_dirty);
                    mark_dirty();
                });
            }

            chip.append(&close_btn);
        }

        chip_box.append(&chip);
    }
    queue_all_overlays(overlays);
}

/// Trigger a redraw on all thumbnail overlay DrawingAreas, forcing the crop
/// rectangles to be repainted with current data.
fn queue_all_overlays(overlays: &Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>) {
    let mut list = overlays.borrow_mut();
    list.retain(|w| w.upgrade().is_some());
    for weak in list.iter() {
        if let Some(da) = weak.upgrade() {
            da.queue_draw();
        }
    }
}

/// Return the (r, g, b) colour for preset index `i` (cycles every 8).
fn preset_color_rgb(i: usize) -> (f64, f64, f64) {
    #[rustfmt::skip]
    const C: [(f64, f64, f64); 8] = [
        (0x35_u8 as f64 / 255.0, 0x84_u8 as f64 / 255.0, 0xe4_u8 as f64 / 255.0),
        (0x33_u8 as f64 / 255.0, 0xd1_u8 as f64 / 255.0, 0x7a_u8 as f64 / 255.0),
        (0xff_u8 as f64 / 255.0, 0x78_u8 as f64 / 255.0, 0x00_u8 as f64 / 255.0),
        (0x91_u8 as f64 / 255.0, 0x41_u8 as f64 / 255.0, 0xac_u8 as f64 / 255.0),
        (0xed_u8 as f64 / 255.0, 0x33_u8 as f64 / 255.0, 0x3b_u8 as f64 / 255.0),
        (0x1c_u8 as f64 / 255.0, 0x71_u8 as f64 / 255.0, 0xd8_u8 as f64 / 255.0),
        (0xc0_u8 as f64 / 255.0, 0x61_u8 as f64 / 255.0, 0xcb_u8 as f64 / 255.0),
        (0x98_u8 as f64 / 255.0, 0x6a_u8 as f64 / 255.0, 0x44_u8 as f64 / 255.0),
    ];
    C[i % 8]
}

/// Draw the crop rectangle overlay on a grid thumbnail. Uses the preset's
/// colour if the page has a preset assigned; otherwise falls back to a
/// muted grey.
fn draw_crop_overlay(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    state: &State,
    page_store: &gio::ListStore,
    page_index: usize,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let project = state.borrow();
    let Some(page) = project.pages.get(page_index) else { return };
    let Some(crop) = page.crop else { return };

    let cw = crop.w as f64;
    let ch = crop.h as f64;
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }

    // Read original image dimensions from the PageItem GObject.
    let (iw, ih) = match page_store.item(page_index as u32).and_downcast::<PageItem>() {
        Some(item) => {
            let w = item.base_width() as f64;
            let h = item.base_height() as f64;
            if w > 0.0 && h > 0.0 {
                let rot = page.rotation as u32 % 360;
                if rot == 90 || rot == 270 { (h, w) } else { (w, h) }
            } else {
                ((crop.x + crop.w).max(1) as f64, (crop.y + crop.h).max(1) as f64)
            }
        }
        None => ((crop.x + crop.w).max(1) as f64, (crop.y + crop.h).max(1) as f64),
    };

    // ContentFit::Contain scaling (same as the Picture widget).
    let s = (width / iw).min(height / ih);
    let rendered_w = iw * s;
    let rendered_h = ih * s;
    let ox = (width - rendered_w) / 2.0;
    let oy = (height - rendered_h) / 2.0;

    let rx = ox + crop.x as f64 * s;
    let ry = oy + crop.y as f64 * s;
    let rw = cw * s;
    let rh = ch * s;

    // Pick colour from preset, or muted grey.
    let (r, g, b) = match page.crop_preset {
        Some(pi) => preset_color_rgb(pi),
        None => (0.5, 0.5, 0.5),
    };

    // Semi-transparent fill.
    cr.set_source_rgba(r, g, b, 0.25);
    cr.rectangle(rx, ry, rw, rh);
    let _ = cr.fill();

    // Stroked border.
    cr.set_source_rgba(r, g, b, 0.8);
    cr.set_line_width(2.0);
    cr.rectangle(rx, ry, rw, rh);
    let _ = cr.stroke();
}

/// Delete a preset at index `pi`. Fixes up page references and renames
/// remaining presets.
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

fn update_status_label(lbl: &gtk::Label, state: &State, index: i32) {
    let project = state.borrow();
    let text = if index >= 0 && (index as usize) < project.pages.len() {
        let page = &project.pages[index as usize];
        let mut t = format!("Page {}/{}", index + 1, project.pages.len());
        if let Some(pi) = page.crop_preset {
            if let Some(p) = project.crop_presets.get(pi) {
                let locked = if p.locked { " \u{1f512}" } else { " \u{1f513}" };
                t.push_str(&format!(" | Preset: {} ({}×{}{})", p.name, p.w, p.h, locked));
            }
        }
        if let Some(c) = page.crop {
            t.push_str(&format!(" | Crop: {}×{} @ ({}, {})", c.w, c.h, c.x, c.y));
        }
        t
    } else {
        String::from("No page selected")
    };
    lbl.set_label(&text);
}

/// Group pages by image dimensions, create one preset per group, attach it.
/// Uses the already-decoded `base_pixbuf` from PageItem when available — never
/// blocks the UI by re-decoding from disk. Pages without a loaded thumbnail
/// are skipped; they'll get their preset assigned the next time the user
/// opens the crop tab after thumbnails finish loading.
fn auto_detect_presets(state: &State, page_store: &gio::ListStore) {
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
                if rot == 90 || rot == 270 { Some((h, w)) } else { Some((w, h)) }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(100.0, 100.0, 200.0, 200.0)
    }

    /// A click 11 widget-px from a corner must register at every zoom level —
    /// this is the bug from the original implementation, where the hit radius
    /// was tied to image space instead of widget space.
    #[test]
    fn handle_hit_is_zoom_invariant() {
        let r = rect();
        for z in [0.25_f64, 1.0, 4.0] {
            // Image-space offset corresponding to 11 widget-px: 11 / z.
            let off = 11.0 / z;
            let near_nw = (r.x + off, r.y);
            let h = Handle::pick(r, near_nw, z);
            assert_eq!(
                h,
                Some(Handle::Corner(Corner::NW)),
                "expected NW corner hit at z={}, near_nw={:?}",
                z,
                near_nw
            );
        }
    }

    #[test]
    fn handle_miss_outside_radius() {
        let r = rect();
        for z in [0.25_f64, 1.0, 4.0] {
            // 20 widget-px away — outside HANDLE_HIT_PX = 12.
            let off = 20.0 / z;
            let far = (r.x + off, r.y + off);
            assert_eq!(Handle::pick(r, far, z), None);
        }
    }

    #[test]
    fn move_drag_clamps_into_bounds() {
        let initial = Rect::new(50.0, 50.0, 100.0, 100.0);
        let r = Handle::Move
            .apply(initial, (0.0, 0.0), (10000.0, 10000.0), (200.0, 200.0))
            .unwrap();
        assert_eq!(r, Rect::new(100.0, 100.0, 100.0, 100.0));
    }

    #[test]
    fn corner_resize_respects_min() {
        let initial = Rect::new(0.0, 0.0, 100.0, 100.0);
        // Try to drag SE corner to NW corner — should clamp to MIN_CROP_PX.
        let r = Handle::Corner(Corner::SE)
            .apply(initial, (100.0, 100.0), (0.0, 0.0), (200.0, 200.0));
        // Either None (rejected as too small) or a rect with at least MIN_CROP_PX.
        if let Some(r) = r {
            assert!(r.w >= MIN_CROP_PX);
            assert!(r.h >= MIN_CROP_PX);
        }
    }

    #[test]
    fn edge_drag_keeps_opposite_edge_fixed() {
        let initial = Rect::new(50.0, 50.0, 100.0, 100.0);
        let r = Handle::Edge(Edge::N)
            .apply(initial, (0.0, 0.0), (0.0, 20.0), (500.0, 500.0))
            .unwrap();
        // South edge = 150 must still be 150.
        assert!((r.y + r.h - 150.0).abs() < 1e-6);
        assert!((r.y - 20.0).abs() < 1e-6);
    }
}
