use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

fn exif_corrected_dims(file_dims: (u32, u32), thumb_w: i32, thumb_h: i32) -> (u32, u32) {
    let (fw, fh) = file_dims;
    if fw == 0 || fh == 0 || thumb_w == 0 || thumb_h == 0 {
        return file_dims;
    }
    let (fw, fh) = (fw as f64, fh as f64);
    let (tw, th) = (thumb_w as f64, thumb_h as f64);
    if (tw * fh - th * fw).abs() <= (tw * fw - th * fh).abs() {
        (fw as u32, fh as u32)
    } else {
        (fh as u32, fw as u32)
    }
}

use super::page_item::PageItem;
use crate::app::State;
use crate::widgets::zoom_pan::{ZoomPanConfig, ZoomPanController};
use pagecutter_core::project::Page;

struct ThumbData {
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    rowstride: i32,
    has_alpha: bool,
    orig_width: u32,
    orig_height: u32,
}

enum LoadMsg {
    Progress(usize, ThumbData),
    Finished,
}

pub fn build(state: State, page_store: gio::ListStore) -> gtk::Widget {
    let store = page_store;
    let selection = gtk::MultiSelection::new(Some(store.clone()));

    let pending_tasks = Rc::new(Cell::new(0));
    let (tx, rx) = async_channel::unbounded::<LoadMsg>();

    let (tx_prev, rx_prev) = async_channel::unbounded::<(u64, PathBuf, u32)>();
    let (tx_prev_res, rx_prev_res) = async_channel::unbounded::<(u64, ThumbData)>();

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

    let preview_req_id = Rc::new(Cell::new(0u64));

    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();

    let add_btn = gtk::Button::builder()
        .label("Add Images")
        .icon_name("list-add-symbolic")
        .build();
    add_btn.add_css_class("suggested-action");

    let open_project_btn = gtk::Button::builder()
        .label("Open Project…")
        .icon_name("document-open-symbolic")
        .tooltip_text("Open a saved .pcut project file")
        .build();

    let spinner = gtk::Spinner::builder().visible(false).build();

    let rotate_ccw = gtk::Button::builder()
        .icon_name("object-rotate-left-symbolic")
        .tooltip_text("Rotate selected 90° counter-clockwise")
        .sensitive(false)
        .build();
    let rotate_cw = gtk::Button::builder()
        .icon_name("object-rotate-right-symbolic")
        .tooltip_text("Rotate selected 90° clockwise")
        .sensitive(false)
        .build();
    let rotate_180 = gtk::Button::builder()
        .label("180°")
        .tooltip_text("Rotate selected 180°")
        .sensitive(false)
        .build();
    let delete_btn = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Remove selected")
        .sensitive(false)
        .build();
    delete_btn.add_css_class("destructive-action");

    let spacer = gtk::Box::builder().hexpand(true).build();

    let count = gtk::Label::builder()
        .label("0 images")
        .build();

    toolbar.append(&add_btn);
    toolbar.append(&open_project_btn);
    toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    toolbar.append(&rotate_ccw);
    toolbar.append(&rotate_cw);
    toolbar.append(&rotate_180);
    toolbar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    toolbar.append(&delete_btn);
    toolbar.append(&spacer);
    toolbar.append(&spinner);
    toolbar.append(&count);

    glib::MainContext::default().spawn_local(glib::clone!(
        #[weak]
        store,
        #[weak]
        spinner,
        #[strong]
        pending_tasks,
        async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    LoadMsg::Progress(index, data) => {
                        let pb = gdk_pixbuf::Pixbuf::from_bytes(
                            &data.bytes,
                            gdk_pixbuf::Colorspace::Rgb,
                            data.has_alpha,
                            8,
                            data.width,
                            data.height,
                            data.rowstride,
                        );
                        if let Some(item) = store.item(index as u32).and_downcast::<PageItem>() {
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
    ));

use rayon::prelude::*;

    let start_loading = glib::clone!(
        #[weak] store,
        #[strong] state,
        #[weak] count,
        #[weak] spinner,
        #[strong] pending_tasks,
        #[strong] tx,
        move |paths: Vec<PathBuf>| {
            if paths.is_empty() { return; }
            
            let start_index = store.n_items() as usize;

            for path in &paths {
                let item = PageItem::new_placeholder(path.clone());
                store.append(&item);
                state.borrow_mut().pages.push(Page {
                    path: path.clone(),
                    rotation: 0,
                    crop: None,
                    crop_preset: None,
                    output: None,
                });
            }
            update_count(&count, &state);

            pending_tasks.set(pending_tasks.get() + paths.len());
            spinner.set_visible(true);
            spinner.start();

            let paths_with_indices: Vec<(usize, PathBuf)> = paths
                .into_iter()
                .enumerate()
                .map(|(i, p)| (start_index + i, p))
                .collect();

            let tx = tx.clone();
            std::thread::spawn(move || {
                let total_start = std::time::Instant::now();
                paths_with_indices.into_par_iter().for_each(|(index, path)| {
                    // Read original image dimensions from the file header
                    // (non-decoding, fast) so crop auto-detection uses real
                    // sizes, not the 256 px thumbnail dimensions.
                    let file_dims = gdk_pixbuf::Pixbuf::file_info(&path)
                        .map(|(_, w, h)| (w as u32, h as u32))
                        .unwrap_or((0, 0));
                    if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true) {
                        let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                        let (orig_width, orig_height) = exif_corrected_dims(file_dims, pb.width(), pb.height());
                        let bytes = pb.read_pixel_bytes();
                        let data = ThumbData {
                            bytes,
                            width: pb.width(),
                            height: pb.height(),
                            rowstride: pb.rowstride(),
                            has_alpha: pb.has_alpha(),
                            orig_width,
                            orig_height,
                        };
                        let _ = tx.send_blocking(LoadMsg::Progress(index, data));
                    }
                    let _ = tx.send_blocking(LoadMsg::Finished);
                });
                tracing::info!(elapsed = ?total_start.elapsed(), "all thumbnails loaded");
            });
        }
    );

    // Loads a complete saved project: resets state+store, creates placeholders
    // with the saved rotation already applied, then loads thumbnails.
    let start_loading_project = glib::clone!(
        #[weak] store,
        #[strong] state,
        #[weak] count,
        #[weak] spinner,
        #[strong] pending_tasks,
        #[strong] tx,
        move |project: pagecutter_core::project::Project| {
            store.remove_all();
            let pages_info: Vec<(usize, PathBuf, u32)> = project
                .pages
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.path.clone(), p.rotation as u32))
                .collect();
            *state.borrow_mut() = project;

            for (_, path, rotation) in &pages_info {
                let item = PageItem::new_placeholder(path.clone());
                item.set_rotation(*rotation);
                store.append(&item);
            }
            update_count(&count, &state);

            if pages_info.is_empty() { return; }

            pending_tasks.set(pending_tasks.get() + pages_info.len());
            spinner.set_visible(true);
            spinner.start();

            let tx = tx.clone();
            std::thread::spawn(move || {
                pages_info.into_par_iter().for_each(|(index, path, _rotation)| {
                    let file_dims = gdk_pixbuf::Pixbuf::file_info(&path)
                        .map(|(_, w, h)| (w as u32, h as u32))
                        .unwrap_or((0, 0));
                    if let Ok(pb) = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true) {
                        let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                        let (orig_width, orig_height) =
                            exif_corrected_dims(file_dims, pb.width(), pb.height());
                        let bytes = pb.read_pixel_bytes();
                        let data = ThumbData {
                            bytes,
                            width: pb.width(),
                            height: pb.height(),
                            rowstride: pb.rowstride(),
                            has_alpha: pb.has_alpha(),
                            orig_width,
                            orig_height,
                        };
                        let _ = tx.send_blocking(LoadMsg::Progress(index, data));
                    }
                    let _ = tx.send_blocking(LoadMsg::Finished);
                });
            });
        }
    );

    open_project_btn.connect_clicked(glib::clone!(
        #[strong] start_loading_project,
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Open Project")
                .modal(true)
                .build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("pagecutter project (*.pcut)"));
            filter.add_pattern("*.pcut");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let parent = btn.root().and_downcast::<gtk::Window>();
            dialog.open(
                parent.as_ref(),
                gio::Cancellable::NONE,
                glib::clone!(
                    #[strong] start_loading_project,
                    move |result| {
                        let Ok(file) = result else { return };
                        let Some(path) = file.path() else { return };
                        match pagecutter_core::project::load_project(&path) {
                            Ok(project) => start_loading_project(project),
                            Err(e) => tracing::error!("open project: {e}"),
                        }
                    }
                ),
            );
        }
    ));

    // Preview surface: a custom widget that renders a gdk::Texture via GPU.
    let preview = crate::widgets::preview_canvas::PreviewCanvas::new();
    preview.set_hexpand(true);
    preview.set_vexpand(true);
    preview.add_css_class("view");

    let zoom_pan = ZoomPanController::attach(
        &preview,
        &preview,
        ZoomPanConfig {
            pan_button: gdk::BUTTON_PRIMARY,
            ..Default::default()
        },
    );

    glib::MainContext::default().spawn_local({
        let preview_weak = preview.downgrade();
        let preview_req_id = preview_req_id.clone();
        let zoom_pan = zoom_pan.clone();
        async move {
            while let Ok((id, data)) = rx_prev_res.recv().await {
                if id != preview_req_id.get() { continue; }
                let pb = gdk_pixbuf::Pixbuf::from_bytes(
                    &data.bytes,
                    gdk_pixbuf::Colorspace::Rgb,
                    data.has_alpha,
                    8,
                    data.width,
                    data.height,
                    data.rowstride,
                );
                let tex = gdk::Texture::for_pixbuf(&pb);
                let Some(preview) = preview_weak.upgrade() else { continue; };
                preview.set_texture(Some(tex));
                zoom_pan.refit_after_texture_change();
            }
        }
    });

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("SignalListItemFactory must provide gtk::ListItem");
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        let pic = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .width_request(150)
            .height_request(150)
            .build();
        let lbl = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(18)
            .css_classes(["caption"])
            .build();
        card.append(&pic);
        card.append(&lbl);
        item.set_child(Some(&card));
    });
    factory.connect_bind(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("SignalListItemFactory must provide gtk::ListItem");
        let page: PageItem = item
            .item()
            .and_downcast()
            .expect("ListStore item must be PageItem");
        let card: gtk::Box = item.child().and_downcast().expect("child must be gtk::Box");
        let pic: gtk::Picture = card.first_child().and_downcast().expect("first child must be gtk::Picture");
        let lbl: gtk::Label = pic.next_sibling().and_downcast().expect("next sibling must be gtk::Label");

        let b1 = page
            .bind_property("thumbnail", &pic, "paintable")
            .sync_create()
            .build();
        let b2 = page
            .bind_property("filename", &lbl, "label")
            .sync_create()
            .build();
        // SAFETY: set_data stores glib::Binding references under unique
        // keys that no other code uses. The bindings are tied to this
        // ListItem's lifetime: they are cleaned up in connect_unbind below
        // which GTK guarantees is called exactly once before the item is
        // recycled or dropped.
        unsafe {
            item.set_data("__b_thumb", b1);
            item.set_data("__b_label", b2);
        }
    });
    factory.connect_unbind(|_, list_item| {
        let item = list_item
            .downcast_ref::<gtk::ListItem>()
            .expect("SignalListItemFactory must provide gtk::ListItem");
        // SAFETY: steal_data takes ownership of the raw pointer stored by
        // set_data in connect_bind. connect_unbind fires exactly once per
        // item, so the pointer is valid and will not be accessed again.
        unsafe {
            if let Some(b) = item.steal_data::<glib::Binding>("__b_thumb") {
                b.unbind();
            }
            if let Some(b) = item.steal_data::<glib::Binding>("__b_label") {
                b.unbind();
            }
        }
    });

    let grid_view = gtk::GridView::builder()
        .model(&selection)
        .factory(&factory)
        .min_columns(2)
        .max_columns(8)
        .enable_rubberband(true)
        .vexpand(true)
        .hexpand(true)
        .build();
    grid_view.add_css_class("photo-grid");

    let provider = gtk::CssProvider::new();
    provider.load_from_string(".photo-grid > child { margin: 8px; border-radius: 8px; }");
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display must be available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let grid_scroll = gtk::ScrolledWindow::builder()
        .child(&grid_view)
        .vexpand(true)
        .hexpand(true)
        .build();

    let click_gesture = gtk::GestureClick::builder().build();
    click_gesture.connect_pressed(glib::clone!(
        #[weak] selection,
        move |gesture, n_press, x, y| {
            if n_press == 1 {
                if let Some(widget) = gesture.widget() {
                    if let Some(target) = widget.pick(x, y, gtk::PickFlags::DEFAULT) {
                        if target.is::<gtk::GridView>() || target.is::<gtk::Viewport>() || target.is::<gtk::ScrolledWindow>() {
                            selection.unselect_all();
                        }
                    }
                }
            }
        }
    ));
    grid_scroll.add_controller(click_gesture);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&preview)
        .end_child(&grid_scroll)
        .resize_start_child(true)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .position(560)
        .vexpand(true)
        .build();

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    root.append(&toolbar);
    root.append(&paned);

    selection.connect_selection_changed(glib::clone!(
        #[strong] selection,
        #[weak] preview,
        #[strong] preview_req_id,
        #[strong] tx_prev,
        #[weak] rotate_ccw,
        #[weak] rotate_cw,
        #[weak] rotate_180,
        #[weak] delete_btn,
        move |_, _, _| {
            let positions = selected_positions(&selection);
            let any = !positions.is_empty();
            rotate_ccw.set_sensitive(any);
            rotate_cw.set_sensitive(any);
            rotate_180.set_sensitive(any);
            delete_btn.set_sensitive(any);
            update_preview(&selection, &preview, &preview_req_id, &tx_prev);
        }
    ));

    add_btn.connect_clicked(glib::clone!(
        #[strong]
        start_loading,
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Add page images")
                .modal(true)
                .build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for mime in [
                "image/jpeg",
                "image/png",
                "image/tiff",
                "image/webp",
                "image/bmp",
            ] {
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
                    start_loading,
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
                        start_loading(paths);
                    }
                ),
            );
        }
    ));

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop.connect_drop(glib::clone!(
        #[strong]
        start_loading,
        move |_, value, _, _| {
            let Ok(file_list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let mut paths = Vec::new();
            for file in file_list.files() {
                if let Some(path) = file.path() {
                    paths.push(path);
                }
            }
            start_loading(paths);
            true
        }
    ));
    root.add_controller(drop);

    rotate_ccw.connect_clicked(glib::clone!(
        #[strong] state,
        #[weak] store,
        #[strong] selection,
        #[weak] preview,

        #[strong] preview_req_id,
        #[strong] tx_prev,
        move |_| {
            rotate_selected(&selection, &store, &state, -90);
            update_preview(&selection, &preview, &preview_req_id, &tx_prev);
        }
    ));
    rotate_cw.connect_clicked(glib::clone!(
        #[strong] state,
        #[weak] store,
        #[strong] selection,
        #[weak] preview,

        #[strong] preview_req_id,
        #[strong] tx_prev,
        move |_| {
            rotate_selected(&selection, &store, &state, 90);
            update_preview(&selection, &preview, &preview_req_id, &tx_prev);
        }
    ));
    rotate_180.connect_clicked(glib::clone!(
        #[strong] state,
        #[weak] store,
        #[strong] selection,
        #[weak] preview,

        #[strong] preview_req_id,
        #[strong] tx_prev,
        move |_| {
            rotate_selected(&selection, &store, &state, 180);
            update_preview(&selection, &preview, &preview_req_id, &tx_prev);
        }
    ));

    delete_btn.connect_clicked(glib::clone!(
        #[strong] state,
        #[weak] store,
        #[strong] selection,
        #[weak] count,
        #[weak] preview,

        #[strong] preview_req_id,
        #[strong] tx_prev,
        move |_| {
            let mut positions = selected_positions(&selection);
            positions.sort_unstable_by(|a, b| b.cmp(a));
            for pos in positions {
                store.remove(pos);
                let mut p = state.borrow_mut();
                if (pos as usize) < p.pages.len() {
                    p.pages.remove(pos as usize);
                }
            }
            update_count(&count, &state);
            update_preview(&selection, &preview, &preview_req_id, &tx_prev);
        }
    ));

    root.upcast()
}

fn rotate_selected(sel: &gtk::MultiSelection, store: &gio::ListStore, state: &State, delta: i32) {
    for pos in selected_positions(sel) {
        let Some(page) = store.item(pos).and_downcast::<PageItem>() else {
            continue;
        };
        page.apply_rotation_delta(delta);
        let mut p = state.borrow_mut();
        if let Some(stored) = p.pages.get_mut(pos as usize) {
            stored.rotation = page.rotation() as u16;
        }
    }
}

fn decrement_pending(counter: &Rc<Cell<usize>>, spinner: &gtk::Spinner) {
    let val = counter.get();
    if val > 0 {
        counter.set(val - 1);
    }
    if counter.get() == 0 {
        spinner.set_visible(false);
        spinner.stop();
    }
}

fn selected_positions(sel: &gtk::MultiSelection) -> Vec<u32> {
    let bs = sel.selection();
    let mut out = Vec::new();
    if let Some((iter, first)) = gtk::BitsetIter::init_first(&bs) {
        out.push(first);
        out.extend(iter);
    }
    out
}

fn update_count(count: &gtk::Label, state: &State) {
    let n = state.borrow().pages.len();
    count.set_label(&format!("{} images", n));
}

fn update_preview(
    sel: &gtk::MultiSelection,
    preview: &crate::widgets::preview_canvas::PreviewCanvas,
    preview_req_id: &Rc<Cell<u64>>,
    tx_prev: &async_channel::Sender<(u64, PathBuf, u32)>,
) {
    let positions = selected_positions(sel);
    if let Some(&first) = positions.first() {
        if let Some(page) = sel.item(first).and_downcast::<PageItem>() {
            let id = preview_req_id.get() + 1;
            preview_req_id.set(id);
            let _ = tx_prev.send_blocking((id, page.path(), page.rotation()));
            return;
        }
    }
    preview_req_id.set(preview_req_id.get() + 1);
    preview.set_texture(None);
}
