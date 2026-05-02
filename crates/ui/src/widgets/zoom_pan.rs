//! Reusable zoom + pan + fit-to-window controller for [`PreviewCanvas`].
//!
//! Wires up:
//! - pointer tracking,
//! - mouse-wheel and touchpad scroll → animated/instant zoom,
//! - drag-to-pan (configurable button),
//! - double-click toggles fit ↔ 1:1 (or 2× fit if 1:1 is below fit),
//! - on-resize re-fit that preserves user zoom when set.
//!
//! Both `import` and `crop` steps use one of these instead of re-implementing
//! ~250 lines of identical plumbing each.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::preview_canvas::PreviewCanvas;

#[derive(Copy, Clone)]
pub struct ZoomPanConfig {
    /// Button that initiates drag-to-pan. Defaults to middle button.
    pub pan_button: u32,
    /// Maximum zoom factor (multiplier on fit-zoom). Defaults to 32.0.
    pub max_zoom: f64,
    /// Animation duration for wheel zoom, in ms.
    pub anim_ms: u32,
}

impl Default for ZoomPanConfig {
    fn default() -> Self {
        Self {
            pan_button: gdk::BUTTON_MIDDLE,
            max_zoom: 32.0,
            anim_ms: 150,
        }
    }
}

pub struct ZoomPanController {
    canvas: glib::WeakRef<PreviewCanvas>,
    cfg: ZoomPanConfig,
    target_zoom: Cell<f64>,
    target_pan: Cell<(f64, f64)>,
    fit_zoom: Cell<f64>,
    user_zoomed: Cell<bool>,
    pointer: Cell<(f64, f64)>,
    anim: RefCell<Option<adw::TimedAnimation>>,
    on_changed: RefCell<Vec<Box<dyn Fn()>>>,
}

impl ZoomPanController {
    /// Attach a controller to `canvas`. Input controllers are added to
    /// `host` — usually the canvas itself, or an `Overlay` sitting on top.
    pub fn attach(
        canvas: &PreviewCanvas,
        host: &impl IsA<gtk::Widget>,
        cfg: ZoomPanConfig,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            canvas: canvas.downgrade(),
            cfg,
            target_zoom: Cell::new(1.0),
            target_pan: Cell::new((0.0, 0.0)),
            fit_zoom: Cell::new(1.0),
            user_zoomed: Cell::new(false),
            pointer: Cell::new((0.0, 0.0)),
            anim: RefCell::new(None),
            on_changed: RefCell::new(Vec::new()),
        });

        // Re-fit on widget resize.
        canvas.set_on_resize({
            let this = this.clone();
            move |w, h| this.recompute_fit(w, h, false)
        });

        // Pointer tracking for zoom anchor.
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let this = this.clone();
            move |_, x, y| this.pointer.set((x, y))
        });
        host.add_controller(motion);

        // notify::zoom — keep pan in sync with anchor while the property animates.
        canvas.connect_notify_local(Some("zoom"), {
            let this = this.clone();
            move |c, _| {
                if let Some((ix, iy, cx, cy)) = c.cursor_anchor() {
                    let z = c.zoom();
                    let (img_w, img_h) = c.texture_size();
                    if img_w > 0.0 && img_h > 0.0 {
                        let p = clamp_pan(
                            cx - ix * z,
                            cy - iy * z,
                            z,
                            img_w,
                            img_h,
                            c.width() as f64,
                            c.height() as f64,
                        );
                        c.set_pan(p.0, p.1);
                    }
                }
                this.emit_changed();
            }
        });

        // Wheel + touchpad scroll → zoom.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        scroll.connect_scroll({
            let this = this.clone();
            move |ctrl, _dx, dy| {
                let is_wheel = matches!(
                    ctrl.current_event()
                        .as_ref()
                        .and_then(|e| e.downcast_ref::<gdk::ScrollEvent>())
                        .map(|e| e.unit()),
                    Some(gdk::ScrollUnit::Wheel)
                );
                let factor = (-dy * if is_wheel { 0.30 } else { 0.10 }).exp();
                let new_t =
                    (this.target_zoom.get() * factor).clamp(this.fit_zoom.get(), this.cfg.max_zoom);
                if (new_t - this.target_zoom.get()).abs() < 1e-9 {
                    return glib::Propagation::Stop;
                }
                let (cx, cy) = this.pointer.get();
                if is_wheel {
                    this.zoom_to_animated(new_t, cx, cy);
                } else {
                    this.zoom_to_instant(new_t, cx, cy);
                }
                glib::Propagation::Stop
            }
        });
        host.add_controller(scroll);

        // Drag-to-pan.
        let pan_start = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
        let pan_drag = gtk::GestureDrag::new();
        pan_drag.set_button(cfg.pan_button);
        pan_drag.connect_drag_begin({
            let this = this.clone();
            let pan_start = pan_start.clone();
            move |_, _, _| {
                let Some(c) = this.canvas.upgrade() else {
                    return;
                };
                this.pause_anim();
                let z = c.zoom();
                let p = c.pan();
                this.target_zoom.set(z);
                this.target_pan.set(p);
                pan_start.set(p);
            }
        });
        pan_drag.connect_drag_update({
            let this = this.clone();
            move |_, dx, dy| {
                let Some(c) = this.canvas.upgrade() else {
                    return;
                };
                let (sx, sy) = pan_start.get();
                let mut p = (sx + dx, sy + dy);
                let (img_w, img_h) = c.texture_size();
                if img_w > 0.0 && img_h > 0.0 {
                    p = clamp_pan(
                        p.0,
                        p.1,
                        c.zoom(),
                        img_w,
                        img_h,
                        c.width() as f64,
                        c.height() as f64,
                    );
                }
                this.target_pan.set(p);
                this.user_zoomed.set(true);
                c.set_transform(c.zoom(), p);
                this.emit_changed();
            }
        });
        host.add_controller(pan_drag);

        // Double-click toggles fit ↔ 1:1 (or 2× fit if 1:1 < fit).
        let click = gtk::GestureClick::builder().build();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.connect_pressed({
            let this = this.clone();
            move |g, n_press, x, y| {
                if n_press != 2 {
                    return;
                }
                g.set_state(gtk::EventSequenceState::Claimed);
                let fz = this.fit_zoom.get();
                let target = if this.target_zoom.get() > fz * 1.05 {
                    fz
                } else {
                    1.0_f64.max(fz * 2.0)
                };
                this.zoom_to_animated(target, x, y);
            }
        });
        host.add_controller(click);

        this
    }

    /// Subscribe to transform-changed events. Fired when zoom or pan changes.
    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_changed.borrow_mut().push(Box::new(f));
    }

    /// Re-fit after a texture swap. Pauses any in-flight zoom animation,
    /// preserves user-zoom if set (just clamps into new bounds), otherwise
    /// snaps to fit.
    pub fn refit_after_texture_change(&self) {
        self.pause_anim();
        let Some(c) = self.canvas.upgrade() else {
            return;
        };
        self.recompute_fit(c.width(), c.height(), false);
    }

    fn pause_anim(&self) {
        if let Some(a) = self.anim.borrow_mut().take() {
            a.pause();
        }
    }

    fn emit_changed(&self) {
        for cb in self.on_changed.borrow().iter() {
            cb();
        }
    }

    fn recompute_fit(&self, alloc_w: i32, alloc_h: i32, force_reset: bool) {
        let Some(c) = self.canvas.upgrade() else {
            return;
        };
        let (iw, ih) = c.texture_size();
        let aw = alloc_w as f64;
        let ah = alloc_h as f64;
        if iw <= 0.0 || ih <= 0.0 || aw <= 0.0 || ah <= 0.0 {
            return;
        }
        let fz = (aw / iw).min(ah / ih);
        self.fit_zoom.set(fz);
        if force_reset || !self.user_zoomed.get() {
            let p = ((aw - iw * fz) * 0.5, (ah - ih * fz) * 0.5);
            self.target_zoom.set(fz);
            self.target_pan.set(p);
            c.set_transform(fz, p);
        } else {
            let z = c.zoom();
            let cur = c.pan();
            let np = clamp_pan(cur.0, cur.1, z, iw, ih, aw, ah);
            c.set_pan(np.0, np.1);
            let tz = self.target_zoom.get();
            let tcur = self.target_pan.get();
            let tnp = clamp_pan(tcur.0, tcur.1, tz, iw, ih, aw, ah);
            self.target_pan.set(tnp);
            if tz < fz {
                self.target_zoom.set(fz);
            }
            if z < fz {
                c.set_transform(fz, c.pan());
            }
        }
        self.emit_changed();
    }

    fn zoom_to_animated(self: &Rc<Self>, new_z: f64, cx: f64, cy: f64) {
        let Some(c) = self.canvas.upgrade() else {
            return;
        };
        let z0 = c.zoom();
        let (px0, py0) = c.pan();
        if z0 <= 0.0 || new_z <= 0.0 {
            return;
        }
        let ix = (cx - px0) / z0;
        let iy = (cy - py0) / z0;

        let (mut tpx, mut tpy) = (cx - ix * new_z, cy - iy * new_z);
        let (img_w, img_h) = c.texture_size();
        if img_w > 0.0 && img_h > 0.0 {
            let p = clamp_pan(
                tpx,
                tpy,
                new_z,
                img_w,
                img_h,
                c.width() as f64,
                c.height() as f64,
            );
            tpx = p.0;
            tpy = p.1;
        }
        self.target_zoom.set(new_z);
        self.target_pan.set((tpx, tpy));

        c.set_cursor_anchor(ix, iy, cx, cy);
        self.pause_anim();
        self.user_zoomed
            .set((new_z - self.fit_zoom.get()).abs() > 1e-4);

        let target = adw::PropertyAnimationTarget::new(&c, "zoom");
        let a = adw::TimedAnimation::builder()
            .widget(&c)
            .value_from(z0)
            .value_to(new_z)
            .duration(self.cfg.anim_ms)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target)
            .build();
        a.play();
        *self.anim.borrow_mut() = Some(a);
    }

    fn zoom_to_instant(&self, new_z: f64, cx: f64, cy: f64) {
        let Some(c) = self.canvas.upgrade() else {
            return;
        };
        let z0 = c.zoom();
        let (px0, py0) = c.pan();
        if z0 <= 0.0 {
            return;
        }
        let ix = (cx - px0) / z0;
        let iy = (cy - py0) / z0;

        let (mut tpx, mut tpy) = (cx - ix * new_z, cy - iy * new_z);
        let (img_w, img_h) = c.texture_size();
        if img_w > 0.0 && img_h > 0.0 {
            let p = clamp_pan(
                tpx,
                tpy,
                new_z,
                img_w,
                img_h,
                c.width() as f64,
                c.height() as f64,
            );
            tpx = p.0;
            tpy = p.1;
        }
        self.user_zoomed
            .set((new_z - self.fit_zoom.get()).abs() > 1e-4);
        self.pause_anim();
        self.target_zoom.set(new_z);
        self.target_pan.set((tpx, tpy));
        c.set_transform(new_z, (tpx, tpy));
        self.emit_changed();
    }
}

/// If the scaled image is larger than the container on an axis, clamp pan so
/// the image edges can't cross the container edges. If smaller, force-center.
pub fn clamp_pan(px: f64, py: f64, z: f64, img_w: f64, img_h: f64, cw: f64, ch: f64) -> (f64, f64) {
    let iw = img_w * z;
    let ih = img_h * z;
    let nx = if iw >= cw {
        px.clamp(cw - iw, 0.0)
    } else {
        (cw - iw) * 0.5
    };
    let ny = if ih >= ch {
        py.clamp(ch - ih, 0.0)
    } else {
        (ch - ih) * 0.5
    };
    (nx, ny)
}
