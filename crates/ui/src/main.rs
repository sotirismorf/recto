mod app;
mod steps;
mod widgets;
mod window;

use adw::prelude::*;
use clap::Parser;

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

    let app = adw::Application::builder()
        .application_id("io.github.sotirismorf.Recto")
        .build();
    app.connect_activate(window::build);
    app.run()
}
