use adw::prelude::*;
use gtk::{gdk_pixbuf, gio, glib};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::app::{new_state, new_store, MarkDirty, Session, State};
use crate::steps;

pub fn build(app: &adw::Application, project_path: Option<PathBuf>) {
    let state: State = new_state();
    let session = Session::new(state.clone());
    let page_store = new_store();
    // Shared selection — every tab's grid view models on the same selection,
    // so picking pages on import carries over to crop and colors.
    let selection = gtk::MultiSelection::new(Some(page_store.clone().upcast::<gio::ListModel>()));
    let paned_sync = steps::PanedSync::new();

    let mark_dirty: MarkDirty = {
        let session = session.clone();
        Rc::new(move || session.mark_dirty())
    };

    let (tx, rx) = async_channel::unbounded::<steps::import::LoadMsg>();
    let pending_tasks = Rc::new(Cell::new(0));
    let spinner = gtk::Spinner::builder().visible(false).build();
    let count = gtk::Label::builder().label("0 images").build();

    // ---- Loaders ----------------------------------------------------------

    let load_images: Rc<dyn Fn(Vec<PathBuf>)> = Rc::new(glib::clone!(
        #[weak]
        page_store,
        #[strong]
        state,
        #[weak]
        count,
        #[weak]
        spinner,
        #[strong]
        pending_tasks,
        #[strong]
        tx,
        #[strong]
        mark_dirty,
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
            mark_dirty();
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
                paths_with_indices
                    .into_par_iter()
                    .for_each(|(index, path)| {
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
        #[weak]
        page_store,
        #[strong]
        state,
        #[weak]
        count,
        #[weak]
        spinner,
        #[strong]
        pending_tasks,
        #[strong]
        tx,
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
                pages_info
                    .into_par_iter()
                    .for_each(|(index, path, _rotation)| {
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
        #[weak]
        page_store,
        #[weak]
        spinner,
        #[strong]
        pending_tasks,
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

    // ---- Step pages -------------------------------------------------------

    let work_stack = adw::ViewStack::new();
    work_stack.add_titled_with_icon(
        &steps::import::build(
            state.clone(),
            page_store.clone(),
            selection.clone(),
            paned_sync.clone(),
            load_env,
            Rc::clone(&load_images),
            Rc::clone(&load_project),
            Rc::clone(&mark_dirty),
        ),
        Some("import"),
        "Import",
        "document-open-symbolic",
    );
    work_stack.add_titled_with_icon(
        &steps::crop::build(
            state.clone(),
            page_store.clone(),
            selection.clone(),
            paned_sync.clone(),
            Rc::clone(&mark_dirty),
        ),
        Some("crop"),
        "Crop",
        "edit-cut-symbolic",
    );
    work_stack.add_titled_with_icon(
        &steps::colors::build(
            state.clone(),
            selection.clone(),
            paned_sync.clone(),
            Rc::clone(&mark_dirty),
        ),
        Some("colors"),
        "Colors",
        "preferences-color-symbolic",
    );
    work_stack.add_titled_with_icon(
        &steps::export::build(state.clone(), Rc::clone(&mark_dirty)),
        Some("export"),
        "Export",
        "document-save-symbolic",
    );

    let switcher = adw::ViewSwitcher::builder()
        .stack(&work_stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    switcher.set_visible(false);

    // ---- Header bar -------------------------------------------------------

    let title_widget = adw::WindowTitle::new("Recto", "");
    let header = adw::HeaderBar::builder()
        .title_widget(&title_widget)
        .build();

    // Hamburger menu (top-left). Hidden until a project session is active.
    let menu_model = gio::Menu::new();
    {
        let section = gio::Menu::new();
        section.append(Some("New Project"), Some("win.new"));
        section.append(Some("Open Project…"), Some("win.open"));
        menu_model.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some("Save"), Some("win.save"));
        section.append(Some("Save As…"), Some("win.save-as"));
        menu_model.append_section(None, &section);
    }
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu_model)
        .primary(true)
        .visible(false)
        .build();
    header.pack_start(&menu_btn);

    // ---- Stack: start page vs main work view ------------------------------

    let main_stack = gtk::Stack::new();
    main_stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    // Show chrome (switcher + menu) and switch to the work view.
    let enter_main = {
        let main_stack = main_stack.clone();
        let switcher = switcher.clone();
        let menu_btn = menu_btn.clone();
        let header = header.clone();
        Rc::new(move || {
            header.set_title_widget(Some(&switcher));
            switcher.set_visible(true);
            menu_btn.set_visible(true);
            main_stack.set_visible_child_name("main");
        })
    };

    // Reset back to the start page.
    let enter_start = {
        let main_stack = main_stack.clone();
        let switcher = switcher.clone();
        let menu_btn = menu_btn.clone();
        let header = header.clone();
        let title_widget = title_widget.clone();
        Rc::new(move || {
            switcher.set_visible(false);
            menu_btn.set_visible(false);
            header.set_title_widget(Some(&title_widget));
            main_stack.set_visible_child_name("start");
        })
    };

    let on_files: Rc<dyn Fn(Vec<PathBuf>)> = {
        let load = Rc::clone(&load_images);
        let enter_main = Rc::clone(&enter_main);
        Rc::new(move |paths: Vec<PathBuf>| {
            load(paths);
            enter_main();
        })
    };

    let on_project: Rc<dyn Fn(PathBuf)> = {
        let load = Rc::clone(&load_project);
        let enter_main = Rc::clone(&enter_main);
        let session = session.clone();
        let page_store = page_store.clone();
        Rc::new(
            move |path: PathBuf| match recto_core::project::load_project(&path) {
                Ok(project) => {
                    page_store.remove_all();
                    load(project);
                    session.set_path(Some(&path));
                    session.clear_dirty();
                    enter_main();
                }
                Err(e) => tracing::error!("open project: {e}"),
            },
        )
    };

    let start_page = steps::start::build(on_files.clone(), on_project.clone());
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

    // ---- Title sync -------------------------------------------------------

    {
        let title_widget = title_widget.clone();
        let window = window.clone();
        session.connect_changed(move |s| {
            let name = s.display_name();
            let dirty_mark = if s.is_dirty() { " •" } else { "" };
            title_widget.set_title(&format!("{}{}", name, dirty_mark));
            title_widget.set_subtitle(if s.is_dirty() { "Unsaved changes" } else { "" });
            window.set_title(Some(&format!("{}{} — Recto", name, dirty_mark)));
        });
    }
    // Prime once.
    {
        let name = session.display_name();
        title_widget.set_title(&name);
        window.set_title(Some(&format!("{} — Recto", name)));
    }

    // ---- Actions ----------------------------------------------------------

    let save_to = {
        let session = session.clone();
        Rc::new(move |path: PathBuf| -> bool {
            let mut path = path;
            if path.extension().is_none() {
                path = path.with_extension("pcut");
            }
            let project = session.state.borrow().clone();
            match recto_core::project::save_project(&project, &path) {
                Ok(()) => {
                    session.set_path(Some(&path));
                    session.clear_dirty();
                    true
                }
                Err(e) => {
                    tracing::error!("save project: {e}");
                    false
                }
            }
        })
    };

    // Save As: pick a path, write, store path, clear dirty. After-save hook
    // runs only on success, so we can chain Save-and-Close.
    let save_as = {
        let window = window.clone();
        let save_to = save_to.clone();
        let session = session.clone();
        Rc::new(move |after: Option<Rc<dyn Fn()>>| {
            let dialog = gtk::FileDialog::builder()
                .title("Save Project As")
                .modal(true)
                .build();
            if let Some(p) = session.path() {
                if let Some(name) = p.file_name() {
                    dialog.set_initial_name(Some(&name.to_string_lossy()));
                }
            } else {
                dialog.set_initial_name(Some("project.recto"));
            }
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Recto project (*.recto)"));
            filter.add_pattern("*.recto");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let save_to = save_to.clone();
            dialog.save(Some(&window), gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                if save_to(path) {
                    if let Some(after) = after {
                        after();
                    }
                }
            });
        }) as Rc<dyn Fn(Option<Rc<dyn Fn()>>)>
    };

    // Save: write to existing path; if none, fall through to Save As.
    let save_now = {
        let session = session.clone();
        let save_to = save_to.clone();
        let save_as = save_as.clone();
        Rc::new(move |after: Option<Rc<dyn Fn()>>| match session.path() {
            Some(p) => {
                if save_to(p) {
                    if let Some(after) = after {
                        after();
                    }
                }
            }
            None => save_as(after),
        }) as Rc<dyn Fn(Option<Rc<dyn Fn()>>)>
    };

    // Confirm-if-dirty and then run a continuation. Used by New Project,
    // Open Project, and the close handler.
    let confirm_discard = {
        let window = window.clone();
        let session = session.clone();
        let save_now = save_now.clone();
        Rc::new(move |after: Rc<dyn Fn()>| {
            if !session.is_dirty() {
                after();
                return;
            }
            let dialog = adw::MessageDialog::builder()
                .transient_for(&window)
                .modal(true)
                .heading("Save changes?")
                .body("Your project has unsaved changes.")
                .build();
            dialog.add_response("discard", "Discard");
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("save", "Save");
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("save"));
            dialog.set_close_response("cancel");
            let save_now = save_now.clone();
            dialog.connect_response(None, move |dlg, response| {
                match response {
                    "discard" => after(),
                    "save" => save_now(Some(after.clone())),
                    _ => {}
                }
                dlg.close();
            });
            dialog.present();
        }) as Rc<dyn Fn(Rc<dyn Fn()>)>
    };

    // win.save
    let act_save = gio::SimpleAction::new("save", None);
    {
        let save_now = save_now.clone();
        act_save.connect_activate(move |_, _| save_now(None));
    }

    // win.save-as
    let act_save_as = gio::SimpleAction::new("save-as", None);
    {
        let save_as = save_as.clone();
        act_save_as.connect_activate(move |_, _| save_as(None));
    }

    // win.new — confirm-if-dirty, then reset and return to start page.
    let act_new = gio::SimpleAction::new("new", None);
    {
        let confirm_discard = confirm_discard.clone();
        let session = session.clone();
        let page_store = page_store.clone();
        let count = count.clone();
        let state = state.clone();
        let enter_start = Rc::clone(&enter_start);
        act_new.connect_activate(move |_, _| {
            let session = session.clone();
            let page_store = page_store.clone();
            let count = count.clone();
            let state = state.clone();
            let enter_start = Rc::clone(&enter_start);
            confirm_discard(Rc::new(move || {
                page_store.remove_all();
                session.reset();
                steps::import::update_count(&count, &state);
                enter_start();
            }));
        });
    }

    // win.open — confirm-if-dirty, then run the existing on_project flow.
    let act_open = gio::SimpleAction::new("open", None);
    {
        let confirm_discard = confirm_discard.clone();
        let on_project = on_project.clone();
        let window = window.clone();
        act_open.connect_activate(move |_, _| {
            let on_project = on_project.clone();
            let window = window.clone();
            confirm_discard(Rc::new(move || {
                let dialog = gtk::FileDialog::builder()
                    .title("Open Project")
                    .modal(true)
                    .build();
                let filter = gtk::FileFilter::new();
                filter.set_name(Some("Recto project (*.recto)"));
                filter.add_pattern("*.recto");
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));
                let on_project = on_project.clone();
                dialog.open(Some(&window), gio::Cancellable::NONE, move |result| {
                    let Ok(file) = result else { return };
                    let Some(path) = file.path() else { return };
                    on_project(path);
                });
            }));
        });
    }

    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&act_new);
    actions.add_action(&act_open);
    actions.add_action(&act_save);
    actions.add_action(&act_save_as);
    window.insert_action_group("win", Some(&actions));

    app.set_accels_for_action("win.save", &["<Primary>s"]);
    app.set_accels_for_action("win.save-as", &["<Primary><Shift>s"]);
    app.set_accels_for_action("win.new", &["<Primary>n"]);
    app.set_accels_for_action("win.open", &["<Primary>o"]);

    // ---- Close confirmation ----------------------------------------------

    {
        let session = session.clone();
        let confirm_discard = confirm_discard.clone();
        let window_weak = window.downgrade();
        let force_close = Rc::new(Cell::new(false));
        let fc = force_close.clone();
        window.connect_close_request(move |w| {
            if fc.get() || !session.is_dirty() {
                return glib::Propagation::Proceed;
            }
            let force_close = fc.clone();
            let window_weak = window_weak.clone();
            confirm_discard(Rc::new(move || {
                force_close.set(true);
                if let Some(w) = window_weak.upgrade() {
                    w.close();
                }
            }));
            let _ = w;
            glib::Propagation::Stop
        });
    }

    window.present();

    if let Some(path) = project_path {
        match recto_core::project::load_project(&path) {
            Ok(project) => {
                page_store.remove_all();
                load_project(project);
                session.set_path(Some(&path));
                session.clear_dirty();
                enter_main();
            }
            Err(e) => tracing::error!("Failed to load project {:?}: {}", path, e),
        }
    }
}
