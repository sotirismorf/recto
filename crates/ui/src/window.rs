use adw::prelude::*;
use gtk::{gdk_pixbuf, glib};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::app::{new_state, new_store, State};
use crate::steps;

pub fn build(app: &adw::Application) {
    let state: State = new_state();
    let page_store = new_store();
    let paned_sync = steps::PanedSync::new();

    let (tx, rx) = async_channel::unbounded::<steps::import::LoadMsg>();
    let pending_tasks = Rc::new(Cell::new(0));
    let spinner = gtk::Spinner::builder().visible(false).build();
    let count = gtk::Label::builder().label("0 images").build();

    let load_images: Rc<dyn Fn(Vec<PathBuf>)> = Rc::new(glib::clone!(
        #[weak] page_store,
        #[strong] state,
        #[weak] count,
        #[weak] spinner,
        #[strong] pending_tasks,
        #[strong] tx,
        move |paths: Vec<PathBuf>| {
            use rayon::prelude::*;
            if paths.is_empty() {
                return;
            }
            let start_index = page_store.n_items() as usize;
            for path in &paths {
                let item = steps::page_item::PageItem::new_placeholder(path.clone());
                page_store.append(&item);
                state.borrow_mut().pages.push(recto_core::project::Page {
                    path: path.clone(),
                    rotation: 0,
                    crop: None,
                    crop_preset: None,
                    output: None,
                });
            }
            steps::import::update_count(&count, &state);
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
                paths_with_indices.into_par_iter().for_each(|(index, path)| {
                    let file_dims = gdk_pixbuf::Pixbuf::file_info(&path)
                        .map(|(_, w, h)| (w as u32, h as u32))
                        .unwrap_or((0, 0));
                    if let Ok(pb) =
                        gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true)
                    {
                        let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                        let (orig_width, orig_height) = steps::import::exif_corrected_dims(
                            file_dims,
                            pb.width(),
                            pb.height(),
                        );
                        let bytes = pb.read_pixel_bytes();
                        let data = steps::import::ThumbData {
                            bytes,
                            width: pb.width(),
                            height: pb.height(),
                            rowstride: pb.rowstride(),
                            has_alpha: pb.has_alpha(),
                            orig_width,
                            orig_height,
                        };
                        let _ = tx.send_blocking(steps::import::LoadMsg::Progress(index, data));
                    }
                    let _ = tx.send_blocking(steps::import::LoadMsg::Finished);
                });
            });
        }
    ));

    let load_project: Rc<dyn Fn(recto_core::project::Project)> = Rc::new(glib::clone!(
        #[weak] page_store,
        #[strong] state,
        #[weak] count,
        #[weak] spinner,
        #[strong] pending_tasks,
        #[strong] tx,
        move |project: recto_core::project::Project| {
            use rayon::prelude::*;
            page_store.remove_all();
            let pages_info: Vec<(usize, PathBuf, u32)> = project
                .pages
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.path.clone(), p.rotation as u32))
                .collect();
            *state.borrow_mut() = project;
            for (_, path, rotation) in &pages_info {
                let item = steps::page_item::PageItem::new_placeholder(path.clone());
                item.set_rotation(*rotation);
                page_store.append(&item);
            }
            steps::import::update_count(&count, &state);
            if pages_info.is_empty() {
                return;
            }
            pending_tasks.set(pending_tasks.get() + pages_info.len());
            spinner.set_visible(true);
            spinner.start();
            let tx = tx.clone();
            std::thread::spawn(move || {
                pages_info.into_par_iter().for_each(|(index, path, _rotation)| {
                    let file_dims = gdk_pixbuf::Pixbuf::file_info(&path)
                        .map(|(_, w, h)| (w as u32, h as u32))
                        .unwrap_or((0, 0));
                    if let Ok(pb) =
                        gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 256, 256, true)
                    {
                        let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                        let (orig_width, orig_height) = steps::import::exif_corrected_dims(
                            file_dims,
                            pb.width(),
                            pb.height(),
                        );
                        let bytes = pb.read_pixel_bytes();
                        let data = steps::import::ThumbData {
                            bytes,
                            width: pb.width(),
                            height: pb.height(),
                            rowstride: pb.rowstride(),
                            has_alpha: pb.has_alpha(),
                            orig_width,
                            orig_height,
                        };
                        let _ = tx.send_blocking(steps::import::LoadMsg::Progress(index, data));
                    }
                    let _ = tx.send_blocking(steps::import::LoadMsg::Finished);
                });
            });
        }
    ));

    glib::MainContext::default().spawn_local(glib::clone!(
        #[weak] page_store,
        #[weak] spinner,
        #[strong] pending_tasks,
        async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    steps::import::LoadMsg::Progress(index, data) => {
                        let pb = gdk_pixbuf::Pixbuf::from_bytes(
                            &data.bytes,
                            gdk_pixbuf::Colorspace::Rgb,
                            data.has_alpha,
                            8,
                            data.width,
                            data.height,
                            data.rowstride,
                        );
                        if let Some(item) = page_store
                            .item(index as u32)
                            .and_downcast::<steps::page_item::PageItem>()
                        {
                            item.set_image(pb);
                            item.set_dims(data.orig_width, data.orig_height);
                        }
                    }
                    steps::import::LoadMsg::Finished => {
                        steps::import::decrement_pending(&pending_tasks, &spinner);
                    }
                }
            }
        }
    ));

    let load_env = steps::import::LoadEnv {
        spinner: spinner.clone(),
        count: count.clone(),
    };

    let work_stack = adw::ViewStack::new();
    work_stack.add_titled_with_icon(
        &steps::import::build(
            state.clone(),
            page_store.clone(),
            paned_sync.clone(),
            load_env,
            Rc::clone(&load_images),
            Rc::clone(&load_project),
        ),
        Some("import"),
        "Import",
        "document-open-symbolic",
    );
    work_stack.add_titled_with_icon(
        &steps::crop::build(state.clone(), page_store.clone(), paned_sync.clone()),
        Some("crop"),
        "Crop",
        "edit-cut-symbolic",
    );
    work_stack.add_titled_with_icon(
        &steps::colors::build(state.clone(), page_store.clone(), paned_sync.clone()),
        Some("colors"),
        "Colors",
        "preferences-color-symbolic",
    );
    work_stack.add_titled_with_icon(
        &steps::export::build(state.clone()),
        Some("export"),
        "Export",
        "document-save-symbolic",
    );

    let switcher = adw::ViewSwitcher::builder()
        .stack(&work_stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    switcher.set_visible(false);

    let header = adw::HeaderBar::builder().build();

    let main_stack = gtk::Stack::new();
    main_stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let on_files: Rc<dyn Fn(Vec<PathBuf>)> = {
        let load = Rc::clone(&load_images);
        let main_stack = main_stack.clone();
        let header = header.clone();
        let switcher = switcher.clone();
        Rc::new(move |paths: Vec<PathBuf>| {
            load(paths);
            header.set_title_widget(Some(&switcher));
            switcher.set_visible(true);
            main_stack.set_visible_child_name("main");
        })
    };

    let on_project: Rc<dyn Fn(PathBuf)> = {
        let load = Rc::clone(&load_project);
        let main_stack = main_stack.clone();
        let header = header.clone();
        let switcher = switcher.clone();
        Rc::new(move |path: PathBuf| {
            match recto_core::project::load_project(&path) {
                Ok(project) => {
                    load(project);
                    header.set_title_widget(Some(&switcher));
                    switcher.set_visible(true);
                    main_stack.set_visible_child_name("main");
                }
                Err(e) => tracing::error!("open project: {e}"),
            }
        })
    };

    let start_page = steps::start::build(on_files, on_project);
    main_stack.add_named(&start_page, Some("start"));
    main_stack.add_named(&work_stack, Some("main"));
    main_stack.set_visible_child_name("start");

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&main_stack));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1100)
        .default_height(720)
        .content(&toolbar)
        .build();
    window.present();
}
