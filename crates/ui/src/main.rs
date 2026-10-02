mod app;
mod latest;
mod pixbuf;
mod types;
mod views;
mod widgets;
mod window;
mod worker;

use adw::prelude::*;
use clap::Parser;
use gtk::gio;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Parser, Debug)]
#[command(name = "recto", version)]
struct Cli {
    #[arg(long)]
    project: Option<std::path::PathBuf>,
    #[arg(long)]
    headless: bool,
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
    if cli.headless {
        tracing::info!(
            "headless mode: not yet implemented (project={:?})",
            cli.project
        );
        return glib::ExitCode::SUCCESS;
    }

    let mut initial = cli.files;
    initial.extend(cli.project);
    let initial_files = Rc::new(Cell::new(initial));

    let app = adw::Application::builder()
        .application_id("io.github.sotirismorf.Recto")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

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
            window::build(app, initial_files.take());
        }
    ));

    app.run()
}
