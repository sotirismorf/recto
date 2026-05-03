use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib};
use recto_core::{Command, CropBox, Rect};

use crate::app::State;
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};

const HANDLE_HIT_PX: f64 = 12.0;
const HANDLE_DRAW_PX: f64 = 7.0;
const MIN_CROP_PX: f64 = 16.0;

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
        let overlay = gtk::Overlay::builder().hexpand(true).vexpand(true).build();
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

        this.zoom_pan.connect_changed({
            let weak = Rc::downgrade(&this);
            move || {
                if let Some(p) = weak.upgrade() {
                    p.area.queue_draw();
                }
            }
        });

        this.area.set_draw_func({
            let weak = Rc::downgrade(&this);
            move |_, cr, _, _| {
                if let Some(p) = weak.upgrade() {
                    p.draw_overlay(cr);
                }
            }
        });

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
            let project = state.project();
            let Some(page) = project.pages.get(index) else {
                self.clear();
                return;
            };
            (
                page.path.clone(),
                page.rotation.as_degrees() as u32,
                page.crop.map(Rect::from),
            )
        };

        let pb = gdk_pixbuf::Pixbuf::from_file(&path)
            .ok()
            .map(|pb| pb.apply_embedded_orientation().unwrap_or(pb))
            .map(|pb| crate::widgets::page_item::rotate(&pb, rotation));

        {
            let mut s = self.state.borrow_mut();
            s.image_dims = pb
                .as_ref()
                .map(|pb| (pb.width() as f64, pb.height() as f64));
            s.crop = crop;
            s.binding = Some((state, index));
        }

        self.canvas
            .set_texture(pb.as_ref().map(gdk::Texture::for_pixbuf));
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
        let Some(img_pos) = self.widget_to_image(x, y) else {
            return;
        };
        let (z, _, _) = self.canvas.transform();
        let mut s = self.state.borrow_mut();

        let locked = s
            .binding
            .as_ref()
            .map(|(state, index)| {
                let p = state.project();
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
        let Some(end_img) = self.widget_to_image(end_wx, end_wy) else {
            return;
        };
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
        
        let (rect, locked) = {
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
                    let p = state.project();
                    p.pages
                        .get(*index)
                        .and_then(|pg| pg.crop_preset)
                        .and_then(|pi| p.crop_presets.get(pi))
                        .map(|pr| pr.locked)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            (rect, locked)
        };

        let (z, _, _) = self.canvas.transform();
        let h = if locked {
            if rect.contains(img_pos) {
                Some(Handle::Move)
            } else {
                None
            }
        } else {
            Handle::pick(rect, img_pos, z).or_else(|| {
                if rect.contains(img_pos) {
                    Some(Handle::Move)
                } else {
                    None
                }
            })
        };
        let cursor = h.and_then(|h| gdk::Cursor::from_name(h.cursor_name(), None));
        self.area.set_cursor(cursor.as_ref());
    }

    fn persist_crop(&self) {
        let (state, index, crop) = {
            let s = self.state.borrow();
            let Some((state, index)) = s.binding.clone() else {
                return;
            };
            let crop = s.crop.map(CropBox::from);
            (state, index, crop)
        };
        state.dispatch(Command::SetCrop { index, crop });
    }

    fn sync_preset_from_drag(&self) {
        let (state, pi, w, h) = {
            let s = self.state.borrow();
            let Some((state, index)) = s.binding.as_ref() else {
                return;
            };
            let Some(rect) = s.crop else { return };
            let cb = CropBox::from(rect);
            let project = state.project();
            let Some(pi) = project.pages.get(*index).and_then(|p| p.crop_preset) else {
                return;
            };
            (state.clone(), pi, cb.w, cb.h)
        };

        state.dispatch(Command::SetCropPresetSize {
            index: pi,
            w,
            h,
        });
    }

    fn draw_overlay(&self, cr: &gtk::cairo::Context) {
        let s = self.state.borrow();
        let Some((iw, ih)) = s.image_dims else { return };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let Some(rect) = s.crop else { return };

        let (cr_r, cr_g, cr_b) = s
            .binding
            .as_ref()
            .and_then(|(state, index)| {
                let project = state.project();
                project
                    .pages
                    .get(*index)
                    .and_then(|p| p.crop_preset)
                    .map(crate::widgets::crop_overlay::preset_color_rgb)
            })
            .unwrap_or((0.18, 0.6, 1.0));

        let (z, px, py) = self.canvas.transform();

        let ix = px;
        let iy = py;
        let iw_w = iw * z;
        let ih_w = ih * z;
        let rx = px + rect.x * z;
        let ry = py + rect.y * z;
        let rw = rect.w * z;
        let rh = rect.h * z;

        cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
        cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
        cr.rectangle(ix, iy, iw_w, ih_w);
        cr.rectangle(rx, ry, rw, rh);
        let _ = cr.fill();

        cr.set_source_rgba(cr_r, cr_g, cr_b, 1.0);
        cr.set_line_width(2.0);
        cr.rectangle(rx, ry, rw, rh);
        let _ = cr.stroke();

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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(100.0, 100.0, 200.0, 200.0)
    }

    #[test]
    fn handle_hit_is_zoom_invariant() {
        let r = rect();
        for z in [0.25_f64, 1.0, 4.0] {
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
        let r = Handle::Corner(Corner::SE).apply(
            initial,
            (100.0, 100.0),
            (0.0, 0.0),
            (200.0, 200.0),
        );
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
        assert!((r.y + r.h - 150.0).abs() < 1e-6);
        assert!((r.y - 20.0).abs() < 1e-6);
    }
}
