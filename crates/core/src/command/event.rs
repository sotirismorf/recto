/// Events emitted by [`AppState::dispatch`](crate::command::AppState::dispatch) after a command is applied.
///
/// UI components subscribe to these instead of polling state.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    PagesAdded(Vec<usize>),
    PagesRemoved(Vec<usize>),
    /// Pages were reordered. The payload is a permutation where element `i` is
    /// the *old* index of the page now at position `i`, letting subscribers
    /// rearrange their own views without rebuilding them.
    PagesReordered(Vec<usize>),
    PageChanged(usize),
    GlobalSettingsChanged,
    PresetsChanged,
    ProjectLoaded,
    ProjectCleared,
    ProjectChanged,
}
