mod css;
mod helpers;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};

use crate::app::{MarkDirty, State};
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};
use recto_core::project::Project;

use super::crop::CropPicker;
use super::import::ThumbData;
use super::page_item::PageItem;

use css::{load_preset_css, load_sidebar_css};
use helpers::{
    make_mode_button, make_section, rotate_selected, selected_positions,
    update_arrange_preview,
};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Arrange,
    Crop,
    Color,
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    state: State,
    page_store: gio::ListStore,
    selection: gtk::MultiSelection,
    paned_sync: super::PanedSync,
    spinner: gtk::Spinner,
    count: gtk::Label,
    load_images: Rc<dyn Fn(Vec<PathBuf>)>,
    _load_project: Rc<dyn Fn(Project)>,
    mark_dirty: MarkDirty,
) -> gtk::Widget {
    let current_mode: Rc<Cell<Mode>> = Rc::new(Cell::new(Mode::Arrange));

    let current_index: Rc<Cell<i32>> = Rc::new(Cell::new(-1));
    let crop_initialised: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let selected_indices: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let overlays: Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>> =
        Rc::new(RefCell::new(Vec::new()));

    let (tx_prev, rx_prev) = async_channel::unbounded::<(u64, PathBuf, u32)>();
    let (tx_prev_res, rx_prev_res) = async_channel::unbounded::<(u64, ThumbData)>();
    let preview_req_id: Rc<Cell<u64>> = Rc::new(Cell::new(0));

    std::thread::spawn(move || {
        while let Ok((id, path, rotation)) = rx_prev.recv_blocking() {
            let mut latest = (id, path, rotation);
            while let Ok(next) = rx_prev.try_recv() {
                latest = next;
            }
            let (id, path, rotation) = latest;
            if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file(&path) {
                let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                let pb = super::page_item::rotate(&pb, rotation);
                let bytes = pb.read_pixel_bytes();
                let data = ThumbData {
                    bytes,
                    width: pb.width(),
                    height: pb.height(),
                    rowstride: pb.rowstride(),
                    has_alpha: pb.has_alpha(),
                    orig_width: pb.width() as u32,
                    orig_height: pb.height() as u32,
                };
                let _ = tx_prev_res.send_blocking((id, data));
            }
        }
    });

    let color_req_id: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let (tx_color, rx_color) = async_channel::unbounded::<super::colors::ColorReq>();
    let (tx_color_res, rx_color_res) = async_channel::unbounded::<super::colors::ColorResult>();

    std::thread::spawn(move || {
        while let Ok(mut req) = rx_color.recv_blocking() {
            while let Ok(next) = rx_color.try_recv() {
                req = next;
            }
            if let Some(result) = super::colors::render_preview(&req) {
                let _ = tx_color_res.send_blocking(result);
            }
        }
    });

    load_preset_css();
    load_sidebar_css();

    // ---- Sidebar ----
    let btn_arrange = make_mode_button("view-grid-symbolic", "Arrange", None);
    btn_arrange.set_active(true);
    let btn_crop = make_mode_button("edit-cut-symbolic", "Crop", Some(&btn_arrange));
    let btn_color = make_mode_button("preferences-color-symbolic", "Color", Some(&btn_arrange));

    let mode_btn_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();
    mode_btn_box.append(&btn_arrange);
    mode_btn_box.append(&btn_crop);
    mode_btn_box.append(&btn_color);

    // Arrange controls
    let add_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label("Add Images")
                .build(),
        )
        .build();
    add_btn.add_css_class("suggested-action");
    add_btn.add_css_class("pill");

    let rotate_ccw = gtk::Button::builder()
        .icon_name("object-rotate-left-symbolic")
        .tooltip_text("Rotate 90° counter-clockwise")
        .sensitive(false)
        .build();
    let rotate_180 = gtk::Button::builder()
        .icon_name("object-flip-vertical-symbolic")
        .tooltip_text("Rotate 180°")
        .sensitive(false)
        .build();
    let rotate_cw = gtk::Button::builder()
        .icon_name("object-rotate-right-symbolic")
        .tooltip_text("Rotate 90° clockwise")
        .sensitive(false)
        .build();

    let rotate_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .homogeneous(true)
        .build();
    rotate_box.add_css_class("linked");
    rotate_box.append(&rotate_ccw);
    rotate_box.append(&rotate_180);
    rotate_box.append(&rotate_cw);

    let rotate_section = make_section("Rotate", &rotate_box);

    let delete_btn = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("user-trash-symbolic")
                .label("Delete")
                .build(),
        )
        .tooltip_text("Remove selected pages")
        .sensitive(false)
        .build();
    delete_btn.add_css_class("destructive-action");
    delete_btn.add_css_class("flat");

    let arrange_spacer = gtk::Box::builder().vexpand(true).build();

    count.set_visible(false);
    let status_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    status_box.append(&spinner);
    status_box.append(&count);

    let arrange_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    arrange_controls.append(&add_btn);
    arrange_controls.append(&rotate_section);
    arrange_controls.append(&delete_btn);
    arrange_controls.append(&arrange_spacer);
    arrange_controls.append(&status_box);

    // Crop controls
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
    crop_controls.append(&btn_new);

    // Color controls
    let init_brightness = state.borrow().brightness as f64;
    let init_contrast = state.borrow().contrast as f64;

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

    // Mode controls stack
    let mode_controls_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::None)
        .vexpand(true)
        .build();
    mode_controls_stack.add_named(&arrange_controls, Some("arrange"));
    mode_controls_stack.add_named(&crop_controls, Some("crop"));
    mode_controls_stack.add_named(&color_controls, Some("color"));

    // Sidebar assembly
    let sidebar = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(180)
        .hexpand(false)
        .build();
    sidebar.add_css_class("sidebar-pane");
    sidebar.append(&mode_btn_box);
    sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    sidebar.append(&mode_controls_stack);

    // ---- Preview area ----
    let preview_arrange = PreviewCanvas::new();
    preview_arrange.set_hexpand(true);
    preview_arrange.set_vexpand(true);
    preview_arrange.add_css_class("view");

    let zoom_arrange = ZoomPanController::attach(
        &preview_arrange,
        &preview_arrange,
        ZoomPanConfig {
            pan_button: gdk::BUTTON_PRIMARY,
            ..Default::default()
        },
    );

    let preview_color = PreviewCanvas::new();
    preview_color.set_hexpand(true);
    preview_color.set_vexpand(true);
    preview_color.add_css_class("view");

    let zoom_color = ZoomPanController::attach(
        &preview_color,
        &preview_color,
        ZoomPanConfig {
            pan_button: gdk::BUTTON_PRIMARY,
            ..Default::default()
        },
    );

    let picker = CropPicker::new();

    let mode_preview_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::None)
        .hexpand(true)
        .vexpand(true)
        .build();
    mode_preview_stack.add_named(&preview_arrange, Some("arrange"));
    mode_preview_stack.add_named(picker.overlay(), Some("crop"));
    mode_preview_stack.add_named(&preview_color, Some("color"));

    // ---- Grid ----
    let factory = {
        let state = state.clone();
        let store = page_store.clone();
        let overlays = overlays.clone();
        let mode_ref = current_mode.clone();
        super::grid::overlay_factory(move |stable_id, da| {
            let st = state.clone();
            let sto = store.clone();
            let m = mode_ref.clone();
            da.set_draw_func(move |_, cr, width, height| {
                if m.get() == Mode::Crop {
                    let current_pos = (0..sto.n_items()).find(|&i| {
                        sto.item(i)
                            .and_downcast::<PageItem>()
                            .is_some_and(|item| item.stable_id() == stable_id)
                    });
                    if let Some(pos) = current_pos {
                        super::crop::draw_crop_overlay(
                            cr,
                            width as f64,
                            height as f64,
                            &st,
                            &sto,
                            pos as usize,
                        );
                    }
                }
            });
            let mut list = overlays.borrow_mut();
            list.retain(|w| w.upgrade().is_some());
            let da_ptr = da.as_ptr() as usize;
            let already = list
                .iter()
                .any(|w| w.upgrade().map(|x| x.as_ptr() as usize == da_ptr).unwrap_or(false));
            if !already {
                let w = glib::WeakRef::new();
                w.set(Some(da));
                list.push(w);
            }
        })
    };

    let (_grid_view, grid_scroll) = super::grid::build_grid_scroll(&selection, &factory);

    let click_gesture = gtk::GestureClick::builder().build();
    click_gesture.connect_pressed(glib::clone!(
        #[weak]
        selection,
        move |gesture, n_press, x, y| {
            if n_press == 1 {
                if let Some(widget) = gesture.widget() {
                    if let Some(target) = widget.pick(x, y, gtk::PickFlags::DEFAULT) {
                        if target.is::<gtk::GridView>()
                            || target.is::<gtk::Viewport>()
                            || target.is::<gtk::ScrolledWindow>()
                        {
                            selection.unselect_all();
                        }
                    }
                }
            }
        }
    ));
    grid_scroll.add_controller(click_gesture);

    // ---- Layout ----
    let preview_area = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    preview_area.append(&mode_preview_stack);

    let left_panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    left_panel.append(&sidebar);
    left_panel.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    left_panel.append(&preview_area);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left_panel)
        .end_child(&grid_scroll)
        .resize_start_child(false)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .vexpand(true)
        .build();
    paned_sync.register(&paned);

    // ---- Async receivers ----
    glib::MainContext::default().spawn_local({
        let preview_weak = preview_arrange.downgrade();
        let preview_req_id = preview_req_id.clone();
        let zoom_arrange = zoom_arrange.clone();
        async move {
            while let Ok((id, data)) = rx_prev_res.recv().await {
                if id != preview_req_id.get() {
                    continue;
                }
                let pb = gdk_pixbuf::Pixbuf::from_bytes(
                    &data.bytes,
                    gdk_pixbuf::Colorspace::Rgb,
                    data.has_alpha,
                    8,
                    data.width,
                    data.height,
                    data.rowstride,
                );
                let Some(preview) = preview_weak.upgrade() else {
                    continue;
                };
                preview.set_texture(Some(gdk::Texture::for_pixbuf(&pb)));
                zoom_arrange.refit_after_texture_change();
            }
        }
    });

    glib::MainContext::default().spawn_local({
        let preview_weak = preview_color.downgrade();
        let color_req_id = color_req_id.clone();
        let zoom_color = zoom_color.clone();
        async move {
            while let Ok(result) = rx_color_res.recv().await {
                if result.id != color_req_id.get() {
                    continue;
                }
                let pb = gdk_pixbuf::Pixbuf::from_bytes(
                    &result.bytes,
                    gdk_pixbuf::Colorspace::Rgb,
                    result.has_alpha,
                    8,
                    result.width,
                    result.height,
                    result.rowstride,
                );
                let Some(preview) = preview_weak.upgrade() else {
                    continue;
                };
                preview.set_texture(Some(gdk::Texture::for_pixbuf(&pb)));
                zoom_color.refit_after_texture_change();
            }
        }
    });

    // ---- Selection handler ----
    selection.connect_selection_changed(glib::clone!(
        #[strong]
        current_mode,
        #[strong]
        state,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req_id,
        #[strong]
        tx_prev,
        #[strong]
        color_req_id,
        #[strong]
        tx_color,
        #[strong]
        picker,
        #[strong]
        current_index,
        #[strong]
        selected_indices,
        #[strong]
        overlays,
        #[strong]
        preset_chips,
        #[weak]
        status_lbl,
        #[strong]
        mark_dirty,
        #[weak]
        rotate_ccw,
        #[weak]
        rotate_cw,
        #[weak]
        rotate_180,
        #[weak]
        delete_btn,
        #[weak]
        page_store,
        move |sel, _, _| {
            let bitset = sel.selection();
            let n = page_store.n_items();
            let mut indices: Vec<usize> = Vec::new();
            for i in 0..n {
                if bitset.contains(i) {
                    indices.push(i as usize);
                }
            }
            *selected_indices.borrow_mut() = indices;

            let any = !bitset.is_empty();
            rotate_ccw.set_sensitive(any);
            rotate_cw.set_sensitive(any);
            rotate_180.set_sensitive(any);
            delete_btn.set_sensitive(any);

            match current_mode.get() {
                Mode::Arrange => {
                    update_arrange_preview(sel, &preview_arrange, &preview_req_id, &tx_prev);
                }
                Mode::Crop => {
                    let first = bitset.minimum();
                    if first == u32::MAX {
                        current_index.set(-1);
                        picker.clear();
                        super::crop::refresh_preset_chips(
                            &preset_chips,
                            &state,
                            current_index.clone(),
                            &picker,
                            &overlays,
                            &selected_indices,
                            &mark_dirty,
                        );
                        status_lbl.set_label("No page selected");
                    } else {
                        current_index.set(first as i32);
                        picker.bind(state.clone(), first as usize);
                        super::crop::refresh_preset_chips(
                            &preset_chips,
                            &state,
                            current_index.clone(),
                            &picker,
                            &overlays,
                            &selected_indices,
                            &mark_dirty,
                        );
                        super::crop::update_status_label(&status_lbl, &state, first as i32);
                    }
                }
                Mode::Color => {
                    super::colors::send_preview_req(&state, sel, &color_req_id, &tx_color);
                }
            }
        }
    ));

    // ---- Crop picker on_changed ----
    picker.connect_changed({
        let state = state.clone();
        let chips = preset_chips.clone();
        let sl = status_lbl.clone();
        let ci = current_index.clone();
        let p = picker.clone();
        let ov = overlays.clone();
        let sel_indices = selected_indices.clone();
        let mark_dirty = mark_dirty.clone();
        move || {
            let idx = ci.get();
            if idx < 0 {
                return;
            }
            super::crop::refresh_preset_chips(
                &chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty,
            );
            super::crop::update_status_label(&sl, &state, idx);
            super::crop::queue_all_overlays(&ov);
            mark_dirty();
        }
    });

    // ---- New-preset button ----
    btn_new.connect_clicked({
        let state = state.clone();
        let picker = picker.clone();
        let chips = preset_chips.clone();
        let sl = status_lbl.clone();
        let ci = current_index.clone();
        let p = picker.clone();
        let ov = overlays.clone();
        let sel_indices = selected_indices.clone();
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
            project.crop_presets.push(recto_core::project::CropPreset {
                name,
                w,
                h,
                locked: false,
            });
            if let Some(page) = project.pages.get_mut(idx as usize) {
                page.crop_preset = Some(pi);
                page.crop = Some(recto_core::project::CropBox { x: 0, y: 0, w, h });
            }
            drop(project);
            picker.set_crop(Some(recto_core::geometry::Rect::new(
                0.0, 0.0, w as f64, h as f64,
            )));
            super::crop::refresh_preset_chips(
                &chips, &state, ci.clone(), &p, &ov, &sel_indices, &mark_dirty,
            );
            super::crop::update_status_label(&sl, &state, idx);
            mark_dirty();
        }
    });

    // ---- Mode switch handlers ----
    btn_arrange.connect_toggled(glib::clone!(
        #[strong]
        current_mode,
        #[weak]
        mode_preview_stack,
        #[weak]
        mode_controls_stack,
        #[strong]
        selection,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req_id,
        #[strong]
        tx_prev,
        #[strong]
        overlays,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            current_mode.set(Mode::Arrange);
            mode_preview_stack.set_visible_child_name("arrange");
            mode_controls_stack.set_visible_child_name("arrange");
            update_arrange_preview(&selection, &preview_arrange, &preview_req_id, &tx_prev);
            super::crop::queue_all_overlays(&overlays);
        }
    ));

    btn_crop.connect_toggled(glib::clone!(
        #[strong]
        current_mode,
        #[weak]
        mode_preview_stack,
        #[weak]
        mode_controls_stack,
        #[strong]
        state,
        #[strong]
        page_store,
        #[strong]
        selection,
        #[strong]
        picker,
        #[strong]
        current_index,
        #[strong]
        selected_indices,
        #[strong]
        preset_chips,
        #[weak]
        status_lbl,
        #[strong]
        overlays,
        #[strong]
        crop_initialised,
        #[strong]
        mark_dirty,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            current_mode.set(Mode::Crop);
            mode_preview_stack.set_visible_child_name("crop");
            mode_controls_stack.set_visible_child_name("crop");

            if !crop_initialised.get() {
                crop_initialised.set(true);
                super::crop::auto_detect_presets(&state, &page_store);
            }

            let bs = selection.selection();
            let first = if !bs.is_empty() {
                bs.minimum()
            } else if page_store.n_items() > 0 {
                selection.select_item(0, true);
                0
            } else {
                u32::MAX
            };

            if first == u32::MAX {
                current_index.set(-1);
                picker.clear();
                super::crop::refresh_preset_chips(
                    &preset_chips,
                    &state,
                    current_index.clone(),
                    &picker,
                    &overlays,
                    &selected_indices,
                    &mark_dirty,
                );
                status_lbl.set_label("No page selected");
            } else {
                current_index.set(first as i32);
                picker.bind(state.clone(), first as usize);
                super::crop::refresh_preset_chips(
                    &preset_chips,
                    &state,
                    current_index.clone(),
                    &picker,
                    &overlays,
                    &selected_indices,
                    &mark_dirty,
                );
                super::crop::update_status_label(&status_lbl, &state, first as i32);
            }
            super::crop::queue_all_overlays(&overlays);
        }
    ));

    btn_color.connect_toggled(glib::clone!(
        #[strong]
        current_mode,
        #[weak]
        mode_preview_stack,
        #[weak]
        mode_controls_stack,
        #[strong]
        state,
        #[strong]
        selection,
        #[strong]
        color_req_id,
        #[strong]
        tx_color,
        #[strong]
        overlays,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            current_mode.set(Mode::Color);
            mode_preview_stack.set_visible_child_name("color");
            mode_controls_stack.set_visible_child_name("color");
            super::colors::send_preview_req(&state, &selection, &color_req_id, &tx_color);
            super::crop::queue_all_overlays(&overlays);
        }
    ));

    // ---- Arrange action handlers ----
    add_btn.connect_clicked(glib::clone!(
        #[strong]
        load_images,
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Add page images")
                .modal(true)
                .build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for mime in ["image/jpeg", "image/png", "image/tiff", "image/webp", "image/bmp"] {
                filter.add_mime_type(mime);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let parent = btn.root().and_downcast::<gtk::Window>();
            dialog.open_multiple(
                parent.as_ref(),
                gio::Cancellable::NONE,
                glib::clone!(
                    #[strong]
                    load_images,
                    move |result| {
                        let Ok(files) = result else { return };
                        let mut paths = Vec::new();
                        for i in 0..files.n_items() {
                            let Some(file) = files.item(i).and_downcast::<gio::File>() else {
                                continue;
                            };
                            if let Some(path) = file.path() {
                                paths.push(path);
                            }
                        }
                        load_images(paths);
                    }
                ),
            );
        }
    ));

    let drop_target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop_target.connect_drop(glib::clone!(
        #[strong]
        load_images,
        move |_, value, _, _| {
            let Ok(file_list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let paths: Vec<_> = file_list.files().iter().filter_map(|f| f.path()).collect();
            load_images(paths);
            true
        }
    ));
    paned.add_controller(drop_target);

    rotate_ccw.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[weak]
        page_store,
        #[strong]
        selection,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req_id,
        #[strong]
        tx_prev,
        #[strong]
        mark_dirty,
        move |_| {
            rotate_selected(&selection, &page_store, &state, -90);
            update_arrange_preview(&selection, &preview_arrange, &preview_req_id, &tx_prev);
            mark_dirty();
        }
    ));

    rotate_cw.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[weak]
        page_store,
        #[strong]
        selection,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req_id,
        #[strong]
        tx_prev,
        #[strong]
        mark_dirty,
        move |_| {
            rotate_selected(&selection, &page_store, &state, 90);
            update_arrange_preview(&selection, &preview_arrange, &preview_req_id, &tx_prev);
            mark_dirty();
        }
    ));

    rotate_180.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[weak]
        page_store,
        #[strong]
        selection,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req_id,
        #[strong]
        tx_prev,
        #[strong]
        mark_dirty,
        move |_| {
            rotate_selected(&selection, &page_store, &state, 180);
            update_arrange_preview(&selection, &preview_arrange, &preview_req_id, &tx_prev);
            mark_dirty();
        }
    ));

    delete_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[weak]
        page_store,
        #[strong]
        selection,
        #[weak]
        count,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req_id,
        #[strong]
        tx_prev,
        #[strong]
        mark_dirty,
        move |_| {
            let mut positions = selected_positions(&selection);
            positions.sort_unstable_by(|a, b| b.cmp(a));
            let removed = !positions.is_empty();
            for pos in positions {
                page_store.remove(pos);
                let mut p = state.borrow_mut();
                if (pos as usize) < p.pages.len() {
                    p.pages.remove(pos as usize);
                }
            }
            super::import::update_count(&count, &state);
            update_arrange_preview(&selection, &preview_arrange, &preview_req_id, &tx_prev);
            if removed {
                mark_dirty();
            }
        }
    ));

    // ---- Color slider handlers ----
    brightness_scale.connect_value_changed(glib::clone!(
        #[strong]
        state,
        #[strong]
        selection,
        #[strong]
        color_req_id,
        #[strong]
        tx_color,
        #[strong]
        mark_dirty,
        move |scale| {
            let v = scale.value() as f32;
            let mut p = state.borrow_mut();
            let changed = p.brightness != v;
            if changed {
                p.brightness = v;
            }
            drop(p);
            if changed {
                mark_dirty();
            }
            super::colors::send_preview_req(&state, &selection, &color_req_id, &tx_color);
        }
    ));

    contrast_scale.connect_value_changed(glib::clone!(
        #[strong]
        state,
        #[strong]
        selection,
        #[strong]
        color_req_id,
        #[strong]
        tx_color,
        #[strong]
        mark_dirty,
        move |scale| {
            let v = scale.value() as f32;
            let mut p = state.borrow_mut();
            let changed = p.contrast != v;
            if changed {
                p.contrast = v;
            }
            drop(p);
            if changed {
                mark_dirty();
            }
            super::colors::send_preview_req(&state, &selection, &color_req_id, &tx_color);
        }
    ));

    reset_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        selection,
        #[strong]
        color_req_id,
        #[strong]
        tx_color,
        #[strong]
        mark_dirty,
        #[weak]
        brightness_scale,
        #[weak]
        contrast_scale,
        move |_| {
            let mut p = state.borrow_mut();
            let changed = p.brightness != 0.0 || p.contrast != 0.0;
            p.brightness = 0.0;
            p.contrast = 0.0;
            drop(p);
            brightness_scale.set_value(0.0);
            contrast_scale.set_value(0.0);
            if changed {
                mark_dirty();
            }
            super::colors::send_preview_req(&state, &selection, &color_req_id, &tx_color);
        }
    ));

    paned.upcast()
}
