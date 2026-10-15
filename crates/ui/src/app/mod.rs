//! The application shell: the main window, its actions and the project
//! session it edits.

mod actions;
mod session;
mod window;

pub use session::State;
pub use window::{build, error_chain, show_toast};
