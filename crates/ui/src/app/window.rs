use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use super::actions;
use super::session::{new_state, new_store, Session, State};
use recto_core::{AppEvent, Command, Project};

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("Failed to load project")]
    ProjectLoad(#[source] anyhow::Error),

    #[error("Failed to save project")]
    ProjectSave(#[source] anyhow::Error),

    #[error("Failed to export")]
    Export(#[source] anyhow::Error),
}

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

/// A continuation run once a dialog flow (save, discard) has completed.
pub(super) type After = Rc<dyn Fn()>;

/// The main window: the start page or the workspace, the project session
/// they edit, and the file operations the window's actions run.
pub(super) struct MainWindow {
    pub(super) window: adw::ApplicationWindow,
    pub(super) state: State,
    session: Rc<Session>,
    page_store: gio::ListStore,
    main_stack: gtk::Stack,
    menu_btn: gtk::MenuButton,
    /// Set once the user has agreed to close despite unsaved changes.
    force_close: Cell<bool>,
}

/// Build the main window. `initial_files` come from the command line or the
/// file manager: a `.recto` file is opened as a project, anything else is
/// added as page images.
pub fn build(app: &adw::Application, initial_files: Vec<PathBuf>) {
    let (state, event_rx) = new_state();
    let session = Session::new(state.clone());
    let page_store = new_store();
    // Shared selection — every mode's grid models on the same selection,
    // so picking pages in one mode carries over to the others.
    let selection = gtk::MultiSelection::new(Some(page_store.clone().upcast::<gio::ListModel>()));
    track_dirty(&state, &session);

    let add_images: Rc<dyn Fn(Vec<PathBuf>)> = {
        let state = state.clone();
        Rc::new(move |paths| add_images(&state, paths))
    };
    let workspace = crate::workspace::build(
        state.clone(),
        page_store.clone(),
        selection,
        add_images,
        event_rx,
    );

    // ---- Header bar -------------------------------------------------------

    let title_widget = adw::WindowTitle::new("Recto", "");
    let header = adw::HeaderBar::builder()
        .title_widget(&title_widget)
        .build();
    // Hidden until a project session is active.
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&actions::menu())
        .primary(true)
        .visible(false)
        .build();
    header.pack_end(&menu_btn);

    // ---- Stack: start page vs main work view ------------------------------

    let main_stack = gtk::Stack::new();
    main_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    // Size to the visible page only, so the start page's minimum width
    // doesn't stop the workspace from shrinking.
    main_stack.set_hhomogeneous(false);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1100)
        .default_height(720)
        .build();

    let win = Rc::new(MainWindow {
        window: window.clone(),
        state,
        session,
        page_store,
        main_stack: main_stack.clone(),
        menu_btn,
        force_close: Cell::new(false),
    });

    let start_page = crate::start_page::build(
        {
            let weak = Rc::downgrade(&win);
            Rc::new(move |paths| {
                if let Some(win) = weak.upgrade() {
                    win.start_with_images(paths);
                }
            })
        },
        {
            let weak = Rc::downgrade(&win);
            Rc::new(move |path| {
                if let Some(win) = weak.upgrade() {
                    win.open_project(path);
                }
            })
        },
    );
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
        win.session
            .connect_notify_local(Some("display-name"), move |s, _| {
                let name = s.display_name();
                title_widget.set_title(&name);
                title_widget.set_subtitle(if s.dirty() { "Unsaved changes" } else { "" });
                window.set_title(Some(&format!("{} — Recto", name)));
            });
    }
    // Prime once.
    {
        let name = win.session.display_name();
        title_widget.set_title(&name);
        window.set_title(Some(&format!("{} — Recto", name)));
    }

    actions::install(app, &win);

    // ---- Close confirmation ----------------------------------------------

    // The handler holds the window's only strong reference to `win`, so the
    // shell lives exactly as long as the window.
    let shell = win.clone();
    window.connect_close_request(move |_| {
        if shell.force_close.get() || !shell.session.dirty() {
            return glib::Propagation::Proceed;
        }
        let weak = Rc::downgrade(&shell);
        shell.confirm_discard(Rc::new(move || {
            if let Some(win) = weak.upgrade() {
                win.force_close.set(true);
                win.window.close();
            }
        }));
        glib::Propagation::Stop
    });

    window.present();

    let (projects, images): (Vec<PathBuf>, Vec<PathBuf>) = initial_files
        .into_iter()
        .partition(|p| p.extension().is_some_and(|e| e == "recto"));
    if let Some(path) = projects.into_iter().next() {
        win.open_project(path);
    } else if !images.is_empty() {
        win.start_with_images(images);
    }
}

/// Mark the session dirty on every project mutation.
///
/// ProjectLoaded / ProjectCleared are excluded: they start a clean session,
/// and their initiators manage the dirty flag synchronously — marking here
/// would race and re-dirty the session right after open/new.
fn track_dirty(state: &State, session: &Rc<Session>) {
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

fn add_images(state: &State, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let n = paths.len();
    state.dispatch(Command::AddPages(paths));
    show_toast(&format!(
        "Added {} image{}",
        n,
        if n == 1 { "" } else { "s" }
    ));
}

fn project_file_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Recto project (*.recto)"));
    filter.add_pattern("*.recto");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    filters
}

impl MainWindow {
    /// Show the menu and switch to the work view.
    fn enter_main(&self) {
        self.menu_btn.set_visible(true);
        self.main_stack.set_visible_child_name("main");
    }

    /// Reset back to the start page.
    fn enter_start(&self) {
        self.menu_btn.set_visible(false);
        self.main_stack.set_visible_child_name("start");
    }

    fn start_with_images(&self, paths: Vec<PathBuf>) {
        add_images(&self.state, paths);
        self.enter_main();
    }

    fn load_project(&self, project: Project) {
        let n = project.pages.len();
        self.page_store.remove_all();
        self.state.load_project(project);
        show_toast(&format!(
            "Opened project with {} page{}",
            n,
            if n == 1 { "" } else { "s" }
        ));
    }

    fn open_project(&self, path: PathBuf) {
        match recto_core::io::project::load_project(&path) {
            Ok(project) => {
                self.load_project(project);
                self.session.set_path(Some(&path));
                self.session.clear_dirty();
                self.enter_main();
            }
            Err(e) => {
                tracing::error!("open project: {e}");
                show_error_dialog(&self.window, &AppError::ProjectLoad(e.into()));
            }
        }
    }

    /// Write the project to `path`; returns whether it succeeded.
    fn save_to(&self, mut path: PathBuf) -> bool {
        if path.extension().is_none() {
            path = path.with_extension("recto");
        }
        let project = self.state.project().clone();
        match recto_core::io::project::save_project(&project, &path) {
            Ok(()) => {
                self.session.set_path(Some(&path));
                self.session.clear_dirty();
                show_toast("Project saved");
                true
            }
            Err(e) => {
                tracing::error!("save project: {e}");
                show_toast("Failed to save project");
                false
            }
        }
    }

    /// Pick a path and save there. `after` runs only on success, so
    /// Save-and-Close can be chained.
    pub(super) fn save_as(self: &Rc<Self>, after: Option<After>) {
        let dialog = gtk::FileDialog::builder()
            .title("Save Project As")
            .modal(true)
            .filters(&project_file_filters())
            .build();
        let initial_name = self
            .session
            .path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "project.recto".to_string());
        dialog.set_initial_name(Some(&initial_name));
        let weak = Rc::downgrade(self);
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(win), Ok(file)) = (weak.upgrade(), result) else {
                return;
            };
            let Some(path) = file.path() else { return };
            if win.save_to(path) {
                if let Some(after) = after {
                    after();
                }
            }
        });
    }

    /// Save to the project's path, or ask for one if it has none yet.
    pub(super) fn save(self: &Rc<Self>, after: Option<After>) {
        match self.session.path() {
            Some(path) => {
                if self.save_to(path) {
                    if let Some(after) = after {
                        after();
                    }
                }
            }
            None => self.save_as(after),
        }
    }

    /// Run `after`, first offering to save if the project has unsaved
    /// changes. Used by New Project, Open Project, and closing the window.
    fn confirm_discard(self: &Rc<Self>, after: After) {
        if !self.session.dirty() {
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
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_dlg, response| match response {
            "discard" => after(),
            "save" => {
                if let Some(win) = weak.upgrade() {
                    win.save(Some(after.clone()));
                }
            }
            _ => {}
        });
        dialog.present(Some(&self.window));
    }

    /// Discard the project (after confirming) and return to the start page.
    pub(super) fn new_project(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.confirm_discard(Rc::new(move || {
            if let Some(win) = weak.upgrade() {
                // → state.clear() → ProjectCleared → the workspace clears its store
                win.session.reset();
                win.enter_start();
            }
        }));
    }

    /// Pick a project file (after confirming discard) and open it.
    pub(super) fn choose_project(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.confirm_discard(Rc::new(move || {
            let Some(win) = weak.upgrade() else { return };
            let dialog = gtk::FileDialog::builder()
                .title("Open Project")
                .modal(true)
                .filters(&project_file_filters())
                .build();
            let weak = weak.clone();
            dialog.open(Some(&win.window), gio::Cancellable::NONE, move |result| {
                let (Some(win), Ok(file)) = (weak.upgrade(), result) else {
                    return;
                };
                if let Some(path) = file.path() {
                    win.open_project(path);
                }
            });
        }));
    }
}

fn show_error_dialog(parent: &impl IsA<gtk::Widget>, err: &AppError) {
    let dialog = adw::AlertDialog::new(Some("Error"), Some(&err.to_string()));
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(Some(parent));
}
