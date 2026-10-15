//! The window's `win.*` actions, the main menu that lists them and their
//! keyboard shortcuts.

use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gio;

use super::window::MainWindow;

/// Keyboard shortcuts for the `win.*` actions.
const ACCELS: &[(&str, &[&str])] = &[
    ("win.new", &["<Control>n"]),
    ("win.open", &["<Control>o"]),
    ("win.save", &["<Control>s"]),
    ("win.save-as", &["<Control><Shift>s"]),
    ("win.undo", &["<Control>z"]),
    ("win.redo", &["<Control><Shift>z"]),
];

/// The main (hamburger) menu.
pub(super) fn menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append(Some("New Project"), Some("win.new"));
    section.append(Some("Open Project…"), Some("win.open"));
    menu.append_section(None, &section);
    let section = gio::Menu::new();
    section.append(Some("Save"), Some("win.save"));
    section.append(Some("Save As…"), Some("win.save-as"));
    section.append(Some("Export…"), Some("win.export"));
    menu.append_section(None, &section);
    let section = gio::Menu::new();
    section.append(Some("About Recto"), Some("win.about"));
    menu.append_section(None, &section);
    menu
}

/// Add the `win.*` actions to the window and register their shortcuts.
pub(super) fn install(app: &adw::Application, win: &Rc<MainWindow>) {
    let weak = Rc::downgrade(win);
    let entries = [
        entry(&weak, "new", |win| win.new_project()),
        entry(&weak, "open", |win| win.choose_project()),
        entry(&weak, "save", |win| win.save(None)),
        entry(&weak, "save-as", |win| win.save_as(None)),
        entry(&weak, "export", |win| {
            crate::dialogs::export::show_export_dialog(
                win.state.clone(),
                win.window.upcast_ref::<gtk::Window>(),
                Rc::new(crate::background::job_queue::JobQueue::new()),
            );
        }),
        entry(&weak, "undo", |win| {
            win.state.undo();
        }),
        entry(&weak, "redo", |win| {
            win.state.redo();
        }),
        entry(&weak, "about", |win| show_about(&win.window)),
    ];
    win.window.add_action_entries(entries);
    for (action, accels) in ACCELS {
        app.set_accels_for_action(action, accels);
    }
}

fn entry(
    win: &Weak<MainWindow>,
    name: &str,
    activate: impl Fn(&Rc<MainWindow>) + 'static,
) -> gio::ActionEntry<adw::ApplicationWindow> {
    let win = win.clone();
    gio::ActionEntry::builder(name)
        .activate(move |_, _, _| {
            if let Some(win) = win.upgrade() {
                activate(&win);
            }
        })
        .build()
}

fn show_about(window: &adw::ApplicationWindow) {
    let about = adw::AboutWindow::builder()
        .application_name("Recto")
        .application_icon("io.github.sotirismorf.Recto")
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Sotiris Morfakidis")
        .comments("The powerful book scanning post-processor")
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/sotirismorf/recto")
        .issue_url("https://github.com/sotirismorf/recto/issues")
        .developers(["Sotiris Morfakidis"].as_slice())
        .copyright("© 2026 Sotiris Morfakidis")
        .transient_for(window)
        .build();
    about.present();
}
