//! The project state: [`AppState`] owns the project and applies every
//! [`Command`] to it, then emits an [`AppEvent`] describing the change.

mod app_state;
mod command;
mod event;

pub use app_state::AppState;
pub use command::Command;
pub use event::AppEvent;
