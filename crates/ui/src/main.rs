mod app;
mod steps;
mod widgets;
mod window;

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

    let project_path = Rc::new(Cell::new(cli.project));

    let app = adw::Application::builder()
        .application_id("io.github.sotirismorf.Recto")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_open(glib::clone!(
        #[strong] project_path,
        move |_app, files, _hint| {
            if let Some(file) = files.first() {
                if let Some(path) = file.path() {
                    project_path.set(Some(path));
                }
            }
        }
    ));

    app.connect_activate(glib::clone!(
        #[strong] project_path,
        move |app| {
            window::build(app, project_path.take());
        }
    ));

    app.run()
}
