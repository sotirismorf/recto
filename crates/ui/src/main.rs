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
    /// Open a project file (passed by file manager when double-clicked)
    #[arg()]
    file: Option<std::path::PathBuf>,
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

    let project_path = Rc::new(Cell::new(cli.project.or(cli.file)));

    let app = adw::Application::builder()
        .application_id("io.github.sotirismorf.Recto")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_open(glib::clone!(
        #[strong]
        project_path,
        move |app, files, _hint| {
            for file in files {
                let maybe_path = file.path().or_else(|| {
                    let uri = file.uri();
                    tracing::info!("Open URI (not a local path): {uri}");
                    None
                });
                if let Some(path) = maybe_path {
                    tracing::info!("Opening project from file association: {path:?}");
                    project_path.set(Some(path));
                    break;
                }
            }
            app.activate();
        }
    ));

    app.connect_activate(glib::clone!(
        #[strong]
        project_path,
        move |app| {
            window::build(app, project_path.take());
        }
    ));

    app.run()
}
