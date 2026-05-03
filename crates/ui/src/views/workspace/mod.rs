mod arrange;
mod color;
mod crop;
mod css;
pub mod helpers;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};

use crate::app::State;
use crate::latest::PreviewService;
use crate::widgets::color_preview::ColorReq;
use crate::widgets::crop_picker::CropPicker;
use crate::widgets::page_item::PageItem;
use crate::widgets::preset_chips::PresetCallbacks;
use crate::widgets::preview_canvas::PreviewCanvas;
use crate::widgets::thumbnail_loader::{
    decrement_pending, update_count, LoadMsg, ThumbData, ThumbReq, ThumbnailService,
};
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};
use recto_core::{AppEvent, Command};

use crate::views::workspace::crop::update_margin_spin;
#[cfg(feature = "autodetect")]
use crate::worker::JobQueue;
use css::{load_preset_css, load_sidebar_css};
use helpers::{
    make_mode_button, project_page_into_store, sync_page_metadata, update_arrange_preview,
};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Arrange,
    Crop,
    Color,
}

fn projector_on_pages_added(
    store: &gio::ListStore,
    state: &State,
    indices: &[usize],
) -> Vec<ThumbReq> {
    let paths: Vec<(PathBuf, u32)> = {
        let project = state.project();
        indices
            .iter()
            .filter_map(|&i| {
                let page = project.pages.get(i)?;
                Some((page.path.clone(), page.rotation.as_degrees() as u32))
            })
            .collect()
    };
    paths
        .into_iter()
        .map(|(path, rotation)| {
            let item = PageItem::new_placeholder(path.clone());
            item.set_rotation(rotation);
            store.append(&item);
            ThumbReq {
                id: item.stable_id(),
                path,
            }
        })
        .collect()
}

fn projector_on_pages_removed(store: &gio::ListStore, indices: &[usize]) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    for idx in sorted {
        if (idx as u32) < store.n_items() {
            store.remove(idx as u32);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    state: State,
    page_store: gio::ListStore,
    selection: gtk::MultiSelection,
    paned_sync: crate::views::PanedSync,
    spinner: gtk::Spinner,
    count: gtk::Label,
    load_images: Rc<dyn Fn(Vec<PathBuf>)>,
    event_rx: async_channel::Receiver<AppEvent>,
) -> gtk::Widget {
    let current_mode: Rc<Cell<Mode>> = Rc::new(Cell::new(Mode::Arrange));
    let current_index: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let crop_initialised: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let selected_indices: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let overlays: Rc<RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>> =
        Rc::new(RefCell::new(Vec::new()));

    let preset_callbacks: Rc<PresetCallbacks> = {
        let s = state.clone();
        Rc::new(PresetCallbacks {
            get_project: Rc::new({
                let s = s.clone();
                move || s.project().clone()
            }),
            on_set_crop_preset: Rc::new({
                let s = s.clone();
                move |idx, preset| s.dispatch(Command::SetCropPreset { index: idx, preset })
            }),
            on_set_crop: Rc::new({
                let s = s.clone();
                move |idx, crop| s.dispatch(Command::SetCrop { index: idx, crop })
            }),
            on_set_crop_preset_locked: Rc::new({
                let s = s.clone();
                move |idx, locked| s.dispatch(Command::SetCropPresetLocked { index: idx, locked })
            }),
            on_remove_crop_preset: Rc::new({
                let s = s.clone();
                move |pi| s.dispatch(Command::RemoveCropPreset(pi))
            }),
            on_add_crop_preset: Rc::new({
                let s = s.clone();
                move |preset| s.dispatch(Command::AddCropPreset(preset))
            }),
        })
    };

    // ---- Thumbnail service --------------------------------------------------
    let (thumb_svc, thumb_msg_rx) = ThumbnailService::new();
    let pending_tasks: Rc<Cell<usize>> = Rc::new(Cell::new(0));

    // ---- Preview services ---------------------------------------------------
    let (arrange_preview_svc, preview_req, rx_prev_res) =
        PreviewService::<(PathBuf, u32), ThumbData>::new(|_id, (path, rotation)| {
            let pb = gdk_pixbuf::Pixbuf::from_file(&path).ok()?;
            let pb = pb.apply_embedded_orientation().unwrap_or(pb);
            let pb = crate::widgets::page_item::rotate(&pb, rotation);
            let bytes = pb.read_pixel_bytes();
            Some(ThumbData {
                bytes,
                width: pb.width(),
                height: pb.height(),
                rowstride: pb.rowstride(),
                has_alpha: pb.has_alpha(),
                orig_width: pb.width() as u32,
                orig_height: pb.height() as u32,
            })
        });

    let (color_preview_svc, color_req, rx_color_res) =
        PreviewService::<ColorReq, crate::widgets::color_preview::ColorResult>::new(|_id, req| {
            crate::widgets::color_preview::render_preview(&req)
        });

    load_preset_css();
    load_sidebar_css();

    // ---- Sidebar — mode buttons ------------------------------------------
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

    // ---- Sidebar — per-mode controls --------------------------------------
    let arrange_sidebar = arrange::build_arrange_sidebar(&spinner, &count);
    let crop_sidebar = crop::build_crop_sidebar();
    let color_sidebar = color::build_color_sidebar(&state);

    let mode_controls_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::None)
        .vexpand(true)
        .build();
    mode_controls_stack.add_named(&arrange_sidebar.controls, Some("arrange"));
    mode_controls_stack.add_named(&crop_sidebar.controls, Some("crop"));
    mode_controls_stack.add_named(&color_sidebar.controls, Some("color"));

    let sidebar = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(180)
        .hexpand(false)
        .build();
    sidebar.add_css_class("sidebar-pane");
    sidebar.append(&mode_btn_box);
    sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    sidebar.append(&mode_controls_stack);

    // ---- Preview area ------------------------------------------------------
    let preview_arrange = PreviewCanvas::new();
    preview_arrange.set_hexpand(true);
    preview_arrange.set_vexpand(true);
    preview_arrange.set_overflow(gtk::Overflow::Hidden);
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
    preview_color.set_overflow(gtk::Overflow::Hidden);
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

    // ---- Grid --------------------------------------------------------------
    let factory = {
        let state = state.clone();
        let store = page_store.clone();
        let overlays = overlays.clone();
        let mode_ref = current_mode.clone();
        crate::widgets::thumbnail_grid::overlay_factory(move |stable_id, da| {
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
                        crate::widgets::crop_overlay::draw_crop_overlay(
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
            let already = list.iter().any(|w| {
                w.upgrade()
                    .map(|x| x.as_ptr() as usize == da_ptr)
                    .unwrap_or(false)
            });
            if !already {
                let w = glib::WeakRef::new();
                w.set(Some(da));
                list.push(w);
            }
        })
    };

    let (_grid_view, grid_scroll) =
        crate::widgets::thumbnail_grid::build_grid_scroll(&selection, &factory);

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

    // ---- Layout ------------------------------------------------------------
    let empty_page = adw::StatusPage::builder()
        .icon_name("image-missing-symbolic")
        .title("No images")
        .description("Add images or open a project to get started")
        .vexpand(true)
        .hexpand(true)
        .build();

    let preview_stack = gtk::Stack::new();
    preview_stack.add_named(&mode_preview_stack, Some("preview"));
    preview_stack.add_named(&empty_page, Some("empty"));
    preview_stack.set_visible_child_name("empty");
    preview_stack.set_vexpand(true);
    preview_stack.set_hexpand(true);

    let left_panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    left_panel.append(&sidebar);
    left_panel.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    left_panel.append(&preview_stack);

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

    // ---- Async receivers ---------------------------------------------------
    glib::MainContext::default().spawn_local({
        let preview_weak = preview_arrange.downgrade();
        let preview_req = preview_req.clone();
        let zoom_arrange = zoom_arrange.clone();
        async move {
            let _svc = arrange_preview_svc;
            while let Ok((id, data)) = rx_prev_res.recv().await {
                if !preview_req.is_current(id) {
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
        let color_req = color_req.clone();
        let zoom_color = zoom_color.clone();
        async move {
            let _svc = color_preview_svc;
            while let Ok((id, result)) = rx_color_res.recv().await {
                if !color_req.is_current(id) {
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

    // ---- Thumbnail receiver -------------------------------------------------
    glib::MainContext::default().spawn_local({
        let page_store = page_store.clone();
        let spinner = spinner.clone();
        let pending_tasks = pending_tasks.clone();
        async move {
            while let Ok(msg) = thumb_msg_rx.recv().await {
                match msg {
                    LoadMsg::Progress(id, data) => {
                        let pb = gdk_pixbuf::Pixbuf::from_bytes(
                            &data.bytes,
                            gdk_pixbuf::Colorspace::Rgb,
                            data.has_alpha,
                            8,
                            data.width,
                            data.height,
                            data.rowstride,
                        );
                        let item = (0..page_store.n_items())
                            .filter_map(|i| page_store.item(i).and_downcast::<PageItem>())
                            .find(|item| item.stable_id() == id);
                        if let Some(item) = item {
                            item.set_image(pb);
                            item.set_dims(data.orig_width, data.orig_height);
                        }
                    }
                    LoadMsg::Finished => {
                        decrement_pending(&pending_tasks, &spinner);
                    }
                }
            }
        }
    });

    // ---- AppEvent subscriber (projector) ------------------------------------
    glib::MainContext::default().spawn_local({
        let state = state.clone();
        let selection = selection.clone();
        let color_req = color_req.clone();
        let page_store = page_store.clone();
        let preview_arrange = preview_arrange.downgrade();
        let preview_req = preview_req.clone();
        let current_mode = current_mode.clone();
        let count = count.clone();
        let spinner = spinner.clone();
        let pending_tasks = pending_tasks.clone();
        let preview_stack = preview_stack.clone();
        async move {
            while let Ok(event) = event_rx.recv().await {
                match event {
                    AppEvent::PagesAdded(ref indices) => {
                        let items = projector_on_pages_added(&page_store, &state, indices);
                        if !items.is_empty() {
                            pending_tasks.set(pending_tasks.get() + items.len());
                            spinner.set_visible(true);
                            spinner.start();
                            thumb_svc.enqueue_batch(items);
                        }
                        update_count(&count, &state);
                        preview_stack.set_visible_child_name("preview");
                    }
                    AppEvent::PagesRemoved(ref indices) => {
                        projector_on_pages_removed(&page_store, indices);
                        update_count(&count, &state);
                        if current_mode.get() == Mode::Arrange {
                            if let Some(preview) = preview_arrange.upgrade() {
                                update_arrange_preview(&selection, &preview, &preview_req);
                            }
                        }
                        if page_store.n_items() == 0 {
                            preview_stack.set_visible_child_name("empty");
                        }
                    }
                    AppEvent::PageChanged(idx) => {
                        project_page_into_store(&page_store, &state, idx);
                        if current_mode.get() == Mode::Arrange {
                            if let Some(preview) = preview_arrange.upgrade() {
                                update_arrange_preview(&selection, &preview, &preview_req);
                            }
                        }
                    }
                    AppEvent::PresetsChanged | AppEvent::GlobalSettingsChanged => {
                        if current_mode.get() == Mode::Color {
                            let project = state.project();
                            crate::widgets::color_preview::send_preview_req(
                                &project, &selection, &color_req,
                            );
                        }
                    }
                    AppEvent::ProjectLoaded => {
                        let n = state.project().pages.len();
                        let indices: Vec<usize> = (0..n).collect();
                        let items = projector_on_pages_added(&page_store, &state, &indices);
                        if !items.is_empty() {
                            pending_tasks.set(pending_tasks.get() + items.len());
                            spinner.set_visible(true);
                            spinner.start();
                            thumb_svc.enqueue_batch(items);
                        }
                        update_count(&count, &state);
                        if current_mode.get() == Mode::Arrange {
                            if let Some(preview) = preview_arrange.upgrade() {
                                update_arrange_preview(&selection, &preview, &preview_req);
                            }
                        }
                        preview_stack.set_visible_child_name("preview");
                    }
                    AppEvent::ProjectCleared => {
                        page_store.remove_all();
                        update_count(&count, &state);
                        if let Some(preview) = preview_arrange.upgrade() {
                            preview_req.send_dummy();
                            preview.set_texture(None);
                        }
                        preview_stack.set_visible_child_name("empty");
                    }
                    AppEvent::ProjectChanged => {
                        sync_page_metadata(&page_store, &state);
                        update_count(&count, &state);
                        if current_mode.get() == Mode::Arrange {
                            if let Some(preview) = preview_arrange.upgrade() {
                                update_arrange_preview(&selection, &preview, &preview_req);
                            }
                        }
                    }
                }
            }
        }
    });

    // ---- Selection handler -------------------------------------------------
    selection.connect_selection_changed(glib::clone!(
        #[strong]
        current_mode,
        #[strong]
        state,
        #[weak]
        preview_arrange,
        #[strong]
        preview_req,
        #[strong]
        color_req,
        #[strong]
        picker,
        #[strong]
        current_index,
        #[strong]
        selected_indices,
        #[strong]
        overlays,
        #[strong(rename_to = chips)]
        crop_sidebar.preset_chips,
        #[weak(rename_to = sl)]
        crop_sidebar.status_lbl,
        #[strong]
        crop_sidebar,
        #[weak(rename_to = rotate_ccw)]
        arrange_sidebar.rotate_ccw,
        #[weak(rename_to = rotate_cw)]
        arrange_sidebar.rotate_cw,
        #[weak(rename_to = rotate_180)]
        arrange_sidebar.rotate_180,
        #[weak(rename_to = delete_btn)]
        arrange_sidebar.delete_btn,
        #[weak]
        page_store,
        #[strong]
        preset_callbacks,
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
                    update_arrange_preview(sel, &preview_arrange, &preview_req);
                }
                Mode::Crop => {
                    let first = bitset.minimum();
                    if first == u32::MAX {
                        current_index.set(None);
                        picker.clear();
                        {
                            let project = state.project();
                            crate::widgets::preset_chips::refresh_preset_chips(
                                &chips,
                                &project,
                                current_index.clone(),
                                &picker,
                                &overlays,
                                &selected_indices,
                                &preset_callbacks,
                            );
                        }
                        sl.set_label("No page selected");
                        update_margin_spin(&crop_sidebar, &state, &current_index);
                    } else {
                        current_index.set(Some(first as usize));
                        picker.bind(state.clone(), first as usize);
                        {
                            let project = state.project();
                            crate::widgets::preset_chips::refresh_preset_chips(
                                &chips,
                                &project,
                                current_index.clone(),
                                &picker,
                                &overlays,
                                &selected_indices,
                                &preset_callbacks,
                            );
                        }
                        crate::widgets::crop_overlay::update_status_label(
                            &sl,
                            &state,
                            Some(first as usize),
                        );
                        update_margin_spin(&crop_sidebar, &state, &current_index);
                    }
                }
                Mode::Color => {
                    let project = state.project();
                    crate::widgets::color_preview::send_preview_req(&project, sel, &color_req);
                }
            }
        }
    ));

    // ---- Crop picker on_changed --------------------------------------------
    picker.connect_changed({
        let state = state.clone();
        let cb = preset_callbacks.clone();
        let chips = crop_sidebar.preset_chips.clone();
        let sl = crop_sidebar.status_lbl.clone();
        let ci = current_index.clone();
        let p = picker.clone();
        let ov = overlays.clone();
        let sel_indices = selected_indices.clone();
        let crop_sidebar = crop_sidebar.clone();
        move || {
            let Some(idx) = ci.get() else {
                return;
            };
            {
                let project = state.project();
                crate::widgets::preset_chips::refresh_preset_chips(
                    &chips,
                    &project,
                    ci.clone(),
                    &p,
                    &ov,
                    &sel_indices,
                    &cb,
                );
            }
            crate::widgets::crop_overlay::update_status_label(&sl, &state, Some(idx));
            crate::widgets::crop_overlay::queue_all_overlays(&ov);
            update_margin_spin(&crop_sidebar, &state, &ci);
        }
    });

    // ---- Mode switch handlers ----------------------------------------------
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
        preview_req,
        #[strong]
        overlays,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            current_mode.set(Mode::Arrange);
            mode_preview_stack.set_visible_child_name("arrange");
            mode_controls_stack.set_visible_child_name("arrange");
            update_arrange_preview(&selection, &preview_arrange, &preview_req);
            crate::widgets::crop_overlay::queue_all_overlays(&overlays);
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
        #[strong(rename_to = chips)]
        crop_sidebar.preset_chips,
        #[weak(rename_to = sl)]
        crop_sidebar.status_lbl,
        #[strong]
        crop_sidebar,
        #[strong]
        overlays,
        #[strong]
        crop_initialised,
        #[strong]
        preset_callbacks,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            current_mode.set(Mode::Crop);
            mode_preview_stack.set_visible_child_name("crop");
            mode_controls_stack.set_visible_child_name("crop");

            if !crop_initialised.get() {
                crop_initialised.set(true);
                let project = state.project().clone();
                crate::widgets::preset_chips::auto_detect_presets(
                    &project,
                    &page_store,
                    &preset_callbacks,
                );
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
                current_index.set(None);
                picker.clear();
                {
                    let project = state.project();
                    crate::widgets::preset_chips::refresh_preset_chips(
                        &chips,
                        &project,
                        current_index.clone(),
                        &picker,
                        &overlays,
                        &selected_indices,
                        &preset_callbacks,
                    );
                }
                sl.set_label("No page selected");
                update_margin_spin(&crop_sidebar, &state, &current_index);
            } else {
                current_index.set(Some(first as usize));
                picker.bind(state.clone(), first as usize);
                {
                    let project = state.project();
                    crate::widgets::preset_chips::refresh_preset_chips(
                        &chips,
                        &project,
                        current_index.clone(),
                        &picker,
                        &overlays,
                        &selected_indices,
                        &preset_callbacks,
                    );
                }
                crate::widgets::crop_overlay::update_status_label(
                    &sl,
                    &state,
                    Some(first as usize),
                );
                update_margin_spin(&crop_sidebar, &state, &current_index);
            }
            crate::widgets::crop_overlay::queue_all_overlays(&overlays);
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
        color_req,
        #[strong]
        overlays,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            current_mode.set(Mode::Color);
            mode_preview_stack.set_visible_child_name("color");
            mode_controls_stack.set_visible_child_name("color");
            {
                let project = state.project();
                crate::widgets::color_preview::send_preview_req(&project, &selection, &color_req);
            }
            crate::widgets::crop_overlay::queue_all_overlays(&overlays);
        }
    ));

    // ---- Wire per-mode handlers --------------------------------------------
    arrange::wire_arrange_handlers(
        &arrange_sidebar,
        state.clone(),
        selection.clone(),
        load_images,
        paned.clone(),
    );

    crop::wire_crop_handlers(
        &crop_sidebar,
        state.clone(),
        preset_callbacks.clone(),
        picker.clone(),
        current_index.clone(),
        overlays.clone(),
        selected_indices.clone(),
    );

    color::wire_color_handlers(&color_sidebar, state.clone());

    // ---- Auto Detect button (OpenCV) ---------------------------------------
    #[cfg(feature = "autodetect")]
    {
        use crate::window::show_toast;
        use recto_core::{CropBox, Rotation};

        let state = state.clone();
        let spinner_w = spinner.downgrade();
        let ci = current_index.clone();
        let ov = overlays.clone();
        let si = selected_indices.clone();
        let chips_w = crop_sidebar.preset_chips.downgrade();
        let sl_w = crop_sidebar.status_lbl.downgrade();
        let cb_cl = preset_callbacks.clone();
        let btn_w = crop_sidebar.btn_auto.downgrade();
        let cs = crop_sidebar.clone();

        crop_sidebar.btn_auto.clone().connect_clicked(move |_| {
            let Some(btn) = btn_w.upgrade() else { return };
            let Some(spinner) = spinner_w.upgrade() else {
                return;
            };
            let cs = cs.clone();

            let pages: Vec<(usize, std::path::PathBuf, Rotation)> = {
                let project = state.project();
                project
                    .pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i, p.path.clone(), p.rotation))
                    .collect()
            };
            if pages.is_empty() {
                return;
            }

            spinner.set_visible(true);
            spinner.start();
            btn.set_sensitive(false);

            {
                let n = state.project().crop_presets.len();
                for i in (0..n).rev() {
                    state.dispatch_without_undo(Command::RemoveCropPreset(i));
                }
            }

            let (tx, rx) = async_channel::unbounded();

            // Clone everything the async block will need
            let state2 = state.clone();
            let btn2 = btn.clone();
            let spinner2 = spinner.clone();
            let ci = ci.clone();
            let ov = ov.clone();
            let si = si.clone();
            let cb_cl = cb_cl.clone();
            let picker_w2 = Rc::downgrade(&picker);
            let chips_w2 = chips_w.clone();
            let sl_w2 = sl_w.clone();

            glib::MainContext::default().spawn_local(async move {
                let jq = JobQueue::new();

                jq.spawn(move || {
                    let paths: Vec<(usize, &std::path::Path, Rotation)> = pages
                        .iter()
                        .map(|(i, p, r)| (*i, p.as_path(), *r))
                        .collect();
                    let results = recto_core::autodetect::detect_all_pages(&paths);
                    tracing::info!("autodetect worker: {} page(s) detected", results.len());
                    let detections: Vec<(usize, CropBox)> =
                        results.iter().map(|(idx, cb, _, _)| (*idx, *cb)).collect();
                    let clusters = recto_core::autodetect::cluster_by_similarity(&detections);
                    tracing::info!("autodetect worker: {} cluster(s)", clusters.len());
                    match tx.send_blocking((results, clusters)) {
                        Ok(()) => tracing::info!("autodetect worker: result sent OK"),
                        Err(e) => tracing::error!("autodetect worker: send failed: {:?}", e),
                    }
                });

                match rx.recv().await {
                    Ok((results, clusters)) => {
                        if results.is_empty() {
                            spinner2.stop();
                            spinner2.set_visible(false);
                            btn2.set_sensitive(true);
                            show_toast("Auto-detection failed: no pages detected");
                            return;
                        }

                        let info_map: std::collections::HashMap<usize, (CropBox, u32, u32)> =
                            results
                                .into_iter()
                                .map(|(idx, cb, w, h)| (idx, (cb, w, h)))
                                .collect();

                        for cluster in &clusters {
                            state2.dispatch(Command::AddCropPreset(cluster.preset.clone()));
                            let pi = state2.project().crop_presets.len() - 1;
                            for &page_idx in &cluster.page_indices {
                                let (detection, img_w, img_h) =
                                    info_map.get(&page_idx).copied().unwrap_or((
                                        CropBox {
                                            x: 0,
                                            y: 0,
                                            w: cluster.preset.w,
                                            h: cluster.preset.h,
                                        },
                                        cluster.preset.w.max(1),
                                        cluster.preset.h.max(1),
                                    ));

                                // Center the median preset size on the individual detection
                                let cx = detection.x as i32 + (detection.w as i32 / 2);
                                let cy = detection.y as i32 + (detection.h as i32 / 2);

                                let mut final_w = cluster.preset.w;
                                let mut final_h = cluster.preset.h;

                                // Proportionally scale down if preset is larger than image
                                if img_w > 0 && img_h > 0 && (final_w > img_w || final_h > img_h) {
                                    let scale = (img_w as f32 / final_w as f32)
                                        .min(img_h as f32 / final_h as f32);
                                    final_w = (final_w as f32 * scale) as u32;
                                    final_h = (final_h as f32 * scale) as u32;
                                }

                                let mut final_x = (cx - (final_w as i32 / 2)).max(0) as u32;
                                let mut final_y = (cy - (final_h as i32 / 2)).max(0) as u32;

                                // Ensure x+w and y+h don't exceed image boundaries
                                if img_w > 0 && final_x + final_w > img_w {
                                    final_x = img_w.saturating_sub(final_w);
                                }
                                if img_h > 0 && final_y + final_h > img_h {
                                    final_y = img_h.saturating_sub(final_h);
                                }

                                let crop = CropBox {
                                    x: final_x,
                                    y: final_y,
                                    w: final_w.min(img_w).max(1),
                                    h: final_h.min(img_h).max(1),
                                };

                                state2.dispatch_without_undo(Command::SetCropPreset {
                                    index: page_idx,
                                    preset: Some(pi),
                                });
                                state2.dispatch_without_undo(Command::SetCrop {
                                    index: page_idx,
                                    crop: Some(crop),
                                });
                            }
                        }

                        spinner2.stop();
                        spinner2.set_visible(false);
                        btn2.set_sensitive(true);

                        let Some(chips) = chips_w2.upgrade() else {
                            return;
                        };
                        let Some(picker) = picker_w2.upgrade() else {
                            return;
                        };
                        let Some(sl) = sl_w2.upgrade() else { return };

                        {
                            let project = state2.project();
                            crate::widgets::preset_chips::refresh_preset_chips(
                                &chips,
                                &project,
                                ci.clone(),
                                &picker,
                                &ov,
                                &si,
                                &cb_cl,
                            );
                        }
                        if let Some(idx) = ci.get() {
                            crate::widgets::crop_overlay::update_status_label(
                                &sl,
                                &state2,
                                Some(idx),
                            );
                            if let Some(r) = {
                                let project = state2.project();
                                project
                                    .pages
                                    .get(idx)
                                    .and_then(|p| p.crop)
                                    .map(recto_core::Rect::from)
                            } {
                                picker.set_crop(Some(r));
                            }
                            update_margin_spin(&cs, &state2, &ci);
                        }
                        crate::widgets::crop_overlay::queue_all_overlays(&ov);

                        let total_pages: usize =
                            clusters.iter().map(|c| c.page_indices.len()).sum();
                        show_toast(&format!(
                            "Detected {} preset{} across {} page{}",
                            clusters.len(),
                            if clusters.len() == 1 { "" } else { "s" },
                            total_pages,
                            if total_pages == 1 { "" } else { "s" },
                        ));
                    }
                    Err(_) => {
                        tracing::error!("autodetect: channel closed unexpectedly");
                        spinner2.stop();
                        spinner2.set_visible(false);
                        btn2.set_sensitive(true);
                        show_toast("Auto-detection failed");
                    }
                }
            });
        });
    }

    paned.upcast()
}
