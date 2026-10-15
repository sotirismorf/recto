mod app;
mod background;
mod dialogs;
mod pixbuf;
mod start_page;
mod widgets;
mod workspace;

use adw::prelude::*;
use clap::Parser;
use gtk::{gdk, gio};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Parser, Debug)]
#[command(name = "recto", version)]
struct Cli {
    /// A project file to open, or images to start a new project with
    /// (passed by the file manager when double-clicked)
    #[arg()]
    files: Vec<std::path::PathBuf>,
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let initial_files = Rc::new(Cell::new(cli.files));

    let app = adw::Application::builder()
        .application_id("io.github.sotirismorf.Recto")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|_| load_css());

    app.connect_open(glib::clone!(
        #[strong]
        initial_files,
        move |app, files, _hint| {
            let paths = files
                .iter()
                .filter_map(|file| {
                    let path = file.path();
                    if path.is_none() {
                        tracing::info!("Open URI (not a local path): {}", file.uri());
                    }
                    path
                })
                .collect();
            initial_files.set(paths);
            app.activate();
        }
    ));

    app.connect_activate(glib::clone!(
        #[strong]
        initial_files,
        move |app| {
            app::build(app, initial_files.take());
        }
    ));

    app.run()
}

/// Install the app stylesheet for the whole display.
fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display must be available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
