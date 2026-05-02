/// Events emitted by [`AppState::dispatch`](crate::command::AppState::dispatch) after a command is applied.
///
/// UI components subscribe to these instead of polling state.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    PagesAdded(Vec<usize>),
    PagesRemoved(Vec<usize>),
    PageChanged(usize),
    GlobalSettingsChanged,
    PresetsChanged,
    ProjectLoaded,
    ProjectCleared,
    ProjectChanged,
}
