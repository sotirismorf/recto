use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::app::{new_state, new_store, Session};
use crate::types::AppError;
use crate::views;
use recto_core::AppEvent;

thread_local! {
    static TOAST_OVERLAY: RefCell<Option<glib::WeakRef<adw::ToastOverlay>>> = const { RefCell::new(None) };
}

pub fn show_toast(text: &str) {
    TOAST_OVERLAY.with_borrow(|cell| {
        if let Some(overlay) = cell.as_ref().and_then(|w| w.upgrade()) {
            let toast = adw::Toast::new(text);
            toast.set_timeout(3);
            overlay.add_toast(toast);
        }
    });
}

pub fn build(app: &adw::Application, project_path: Option<PathBuf>) {
    let (state, event_rx) = new_state();
    let session = Session::new(state.clone());
    let page_store = new_store();
    // Shared selection — every tab's grid view models on the same selection,
    // so picking pages on import carries over to crop and colors.
    let selection = gtk::MultiSelection::new(Some(page_store.clone().upcast::<gio::ListModel>()));
    let paned_sync = views::PanedSync::new();

    // Dirty tracker: subscribes to mutation events and marks the session dirty.
    // ProjectLoaded / ProjectCleared are excluded: they start a clean session,
    // and their initiators manage the dirty flag synchronously — marking here
    // would race and re-dirty the session right after open/new.
    {
        let session = session.clone();
        let dirty_rx = state.subscribe();
        glib::spawn_future_local(async move {
            while let Ok(event) = dirty_rx.recv().await {
                match event {
                    AppEvent::PagesAdded(_)
                    | AppEvent::PagesRemoved(_)
                    | AppEvent::PagesReordered(_)
                    | AppEvent::PageChanged(_)
                    | AppEvent::GlobalSettingsChanged
                    | AppEvent::PresetsChanged
                    | AppEvent::ProjectChanged => {
                        session.mark_dirty();
                    }
                    AppEvent::ProjectLoaded | AppEvent::ProjectCleared => {}
                }
            }
        });
    }

    let spinner = gtk::Spinner::builder().visible(false).build();
    let count = gtk::Label::builder().label("0 images").build();

    // ---- Loaders ----------------------------------------------------------

    let load_images: Rc<dyn Fn(Vec<PathBuf>)> = {
        let state = state.clone();
        Rc::new(move |paths: Vec<PathBuf>| {
            if paths.is_empty() {
                return;
            }
            let n = paths.len();
            state.dispatch(recto_core::Command::AddPages(paths));
            show_toast(&format!(
                "Added {} image{}",
                n,
                if n == 1 { "" } else { "s" }
            ));
        })
    };

    let load_project: Rc<dyn Fn(recto_core::Project)> = {
        let state = state.clone();
        let page_store = page_store.clone();
        Rc::new(move |project: recto_core::Project| {
            let n = project.pages.len();
            page_store.remove_all();
            state.load_project(project);
            show_toast(&format!(
                "Opened project with {} page{}",
                n,
                if n == 1 { "" } else { "s" }
            ));
        })
    };

    // ---- Workspace (single unified view) ---------------------------------

    let workspace = crate::views::workspace::build(
        state.clone(),
        page_store.clone(),
        selection.clone(),
        paned_sync.clone(),
        spinner.clone(),
        count.clone(),
        Rc::clone(&load_images),
        event_rx,
    );

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
        section.append(Some("Export\u{2026}"), Some("win.export"));
        menu_model.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some("About Recto"), Some("win.about"));
        menu_model.append_section(None, &section);
    }
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu_model)
        .primary(true)
        .visible(false)
        .build();
    header.pack_end(&menu_btn);

    // ---- Stack: start page vs main work view ------------------------------

    let main_stack = gtk::Stack::new();
    main_stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    // Show menu and switch to the work view.
    let enter_main = {
        let main_stack = main_stack.clone();
        let menu_btn = menu_btn.clone();
        Rc::new(move || {
            menu_btn.set_visible(true);
            main_stack.set_visible_child_name("main");
        })
    };

    // Reset back to the start page.
    let enter_start = {
        let main_stack = main_stack.clone();
        let menu_btn = menu_btn.clone();
        Rc::new(move || {
            menu_btn.set_visible(false);
            main_stack.set_visible_child_name("start");
        })
    };

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1100)
        .default_height(720)
        .build();

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
        let window = window.clone();
        Rc::new(
            move |path: PathBuf| match recto_core::io::project::load_project(&path) {
                Ok(project) => {
                    load(project);
                    session.set_path(Some(&path));
                    session.clear_dirty();
                    enter_main();
                }
                Err(e) => {
                    tracing::error!("open project: {e}");
                    show_error_dialog(&window, &AppError::ProjectLoad(e.into()));
                }
            },
        )
    };

    let start_page = crate::views::start::build(on_files.clone(), on_project.clone());
    main_stack.add_named(&start_page, Some("start"));
    main_stack.add_named(&workspace, Some("main"));
    main_stack.set_visible_child_name("start");

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&main_stack));
    TOAST_OVERLAY.with_borrow_mut(|cell| *cell = Some(toast_overlay.downgrade()));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toast_overlay));
    window.set_content(Some(&toolbar));

    // ---- Title sync -------------------------------------------------------

    {
        let title_widget = title_widget.clone();
        let window = window.clone();
        session.connect_notify_local(Some("display-name"), move |s, _| {
            let name = s.display_name();
            title_widget.set_title(&name);
            title_widget.set_subtitle(if s.dirty() { "Unsaved changes" } else { "" });
            window.set_title(Some(&format!("{} — Recto", name)));
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
                path = path.with_extension("recto");
            }
            let project = session.state().project().clone();
            match recto_core::io::project::save_project(&project, &path) {
                Ok(()) => {
                    session.set_path(Some(&path));
                    session.clear_dirty();
                    show_toast("Project saved");
                    true
                }
                Err(e) => {
                    tracing::error!("save project: {e}");
                    show_toast("Failed to save project");
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
            if !session.dirty() {
                after();
                return;
            }
            let dialog = adw::AlertDialog::new(
                Some("Save changes?"),
                Some("Your project has unsaved changes."),
            );
            dialog.add_response("discard", "Discard");
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("save", "Save");
            dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("save"));
            dialog.set_close_response("cancel");
            let save_now = save_now.clone();
            dialog.connect_response(None, move |_dlg, response| match response {
                "discard" => after(),
                "save" => save_now(Some(after.clone())),
                _ => {}
            });
            dialog.present(Some(&window));
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
        let enter_start = Rc::clone(&enter_start);
        act_new.connect_activate(move |_, _| {
            let session = session.clone();
            let enter_start = Rc::clone(&enter_start);
            confirm_discard(Rc::new(move || {
                session.reset(); // → state.clear() → ProjectCleared → projector clears store
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

    // win.export — open export dialog
    let act_export = gio::SimpleAction::new("export", None);
    {
        let state = state.clone();
        let window = window.clone();
        act_export.connect_activate(move |_, _| {
            let jq = Rc::new(crate::worker::JobQueue::new());
            crate::views::export::show_export_dialog(
                state.clone(),
                window.upcast_ref::<gtk::Window>(),
                jq,
            );
        });
    }

    // win.undo
    let act_undo = gio::SimpleAction::new("undo", None);
    {
        let state = state.clone();
        act_undo.connect_activate(move |_, _| {
            state.undo();
        });
    }

    // win.redo
    let act_redo = gio::SimpleAction::new("redo", None);
    {
        let state = state.clone();
        act_redo.connect_activate(move |_, _| {
            state.redo();
        });
    }

    // win.about
    let act_about = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        act_about.connect_activate(move |_, _| {
            let about = adw::AboutWindow::builder()
                .application_name("Recto")
                .application_icon("io.github.sotirismorf.Recto")
                .version("0.1.0")
                .developer_name("Sotiris Morfakidis")
                .comments("The powerful book scanning post-processor")
                .license_type(gtk::License::Gpl30)
                .website("https://github.com/sotirismorf/recto")
                .issue_url("https://github.com/sotirismorf/recto/issues")
                .developers(["Sotiris Morfakidis"].as_slice())
                .copyright("© 2026 Sotiris Morfakidis")
                .build();
            about.set_transient_for(Some(&window));
            about.present();
        });
    }

    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&act_new);
    actions.add_action(&act_open);
    actions.add_action(&act_save);
    actions.add_action(&act_save_as);
    actions.add_action(&act_export);
    actions.add_action(&act_undo);
    actions.add_action(&act_redo);
    actions.add_action(&act_about);
    window.insert_action_group("win", Some(&actions));

    // ---- Keyboard shortcuts (ShortcutController) --------------------------
    let sc = gtk::ShortcutController::new();
    sc.add_shortcut(
        gtk::Shortcut::builder()
            .trigger(&gtk::ShortcutTrigger::parse_string("<Control>s").unwrap())
            .action(&gtk::ShortcutAction::parse_string("action(win.save)").unwrap())
            .build(),
    );
    sc.add_shortcut(
        gtk::Shortcut::builder()
            .trigger(&gtk::ShortcutTrigger::parse_string("<Control><Shift>s").unwrap())
            .action(&gtk::ShortcutAction::parse_string("action(win.save-as)").unwrap())
            .build(),
    );
    sc.add_shortcut(
        gtk::Shortcut::builder()
            .trigger(&gtk::ShortcutTrigger::parse_string("<Control>n").unwrap())
            .action(&gtk::ShortcutAction::parse_string("action(win.new)").unwrap())
            .build(),
    );
    sc.add_shortcut(
        gtk::Shortcut::builder()
            .trigger(&gtk::ShortcutTrigger::parse_string("<Control>o").unwrap())
            .action(&gtk::ShortcutAction::parse_string("action(win.open)").unwrap())
            .build(),
    );
    sc.add_shortcut(
        gtk::Shortcut::builder()
            .trigger(&gtk::ShortcutTrigger::parse_string("<Control>z").unwrap())
            .action(&gtk::ShortcutAction::parse_string("action(win.undo)").unwrap())
            .build(),
    );
    sc.add_shortcut(
        gtk::Shortcut::builder()
            .trigger(&gtk::ShortcutTrigger::parse_string("<Control><Shift>z").unwrap())
            .action(&gtk::ShortcutAction::parse_string("action(win.redo)").unwrap())
            .build(),
    );
    // Page reordering. The actions live on the workspace and are disabled
    // whenever the move would be a no-op, so these are inert outside Arrange
    // work without needing a mode check here.
    for (trigger, action) in [
        ("<Control><Shift>Left", "arrange.move-back"),
        ("<Control><Shift>Right", "arrange.move-forward"),
        ("<Control><Shift>Home", "arrange.move-start"),
        ("<Control><Shift>End", "arrange.move-end"),
    ] {
        sc.add_shortcut(
            gtk::Shortcut::builder()
                .trigger(&gtk::ShortcutTrigger::parse_string(trigger).unwrap())
                .action(&gtk::ShortcutAction::parse_string(&format!("action({action})")).unwrap())
                .build(),
        );
    }
    window.add_controller(sc);

    // ---- Close confirmation ----------------------------------------------

    {
        let session = session.clone();
        let confirm_discard = confirm_discard.clone();
        let window_weak = window.downgrade();
        let force_close = Rc::new(Cell::new(false));
        let fc = force_close.clone();
        window.connect_close_request(move |w| {
            if fc.get() || !session.dirty() {
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
        on_project(path);
    }
}

fn show_error_dialog(parent: &impl IsA<gtk::Widget>, err: &AppError) {
    let dialog = adw::AlertDialog::new(Some("Error"), Some(&err.to_string()));
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(Some(parent));
}
