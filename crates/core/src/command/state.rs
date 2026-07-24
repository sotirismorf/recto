use std::cell::{Ref, RefCell};

use crate::command::event::AppEvent;
use crate::command::Command;
use crate::domain::order::plan_move;
use crate::domain::project::{Page, Project};
use crate::io::config::load_config;

/// Maximum number of undo entries kept in the ring buffer.
const MAX_UNDO: usize = 50;

/// Owns the [`Project`] behind a [`RefCell`] and is the single point of
/// mutation.  Uses interior mutability so it can be shared behind an `Rc`
/// while still offering `dispatch(&self, …)`.
///
/// Undo/redo is snapshot-based: before every mutating operation the current
/// project is pushed onto an undo stack (capped at [`MAX_UNDO`]).
///
/// ```no_run
/// use recto_core::command::{AppState, Command};
/// use recto_core::domain::project::Project;
/// use recto_core::domain::values::Brightness;
///
/// let project = Project::default();
/// let (state, _rx) = AppState::new(project);
/// state.dispatch(Command::SetBrightness(Brightness::new(0.5)));
/// ```
pub struct AppState {
    project: RefCell<Project>,
    undo_stack: RefCell<Vec<Project>>,
    redo_stack: RefCell<Vec<Project>>,
    event_txs: RefCell<Vec<async_channel::Sender<AppEvent>>>,
}

impl AppState {
    pub fn new(project: Project) -> (Self, async_channel::Receiver<AppEvent>) {
        let (tx, rx) = async_channel::unbounded();
        (
            Self {
                project: RefCell::new(project),
                undo_stack: RefCell::new(Vec::with_capacity(MAX_UNDO)),
                redo_stack: RefCell::new(Vec::new()),
                event_txs: RefCell::new(vec![tx]),
            },
            rx,
        )
    }

    /// Immutable access (panics if already mutably borrowed).
    pub fn project(&self) -> Ref<'_, Project> {
        self.project.borrow()
    }

    /// Apply a command to the project and emit resulting events.
    /// Pushes the current state to the undo stack before mutating.
    pub fn dispatch(&self, cmd: Command) {
        self.push_undo();
        self.redo_stack.borrow_mut().clear();
        let events = self.apply(&cmd);
        for event in events {
            self.emit(event);
        }
    }

    /// Apply a command without recording it in the undo stack (useful for
    /// internal state sync that should not be independently undoable).
    pub fn dispatch_without_undo(&self, cmd: Command) {
        let events = self.apply(&cmd);
        for event in events {
            self.emit(event);
        }
    }

    /// Replace the entire project (used for load / reset).
    /// Undo history is discarded: undoing across a project boundary would
    /// resurrect the previous project under the new session path.
    pub fn load_project(&self, project: Project) {
        self.undo_stack.borrow_mut().clear();
        self.redo_stack.borrow_mut().clear();
        *self.project.borrow_mut() = project;
        self.emit(AppEvent::ProjectLoaded);
    }

    /// Reset to a fresh project. Discards undo history like [`load_project`].
    ///
    /// [`load_project`]: Self::load_project
    pub fn clear(&self) {
        self.undo_stack.borrow_mut().clear();
        self.redo_stack.borrow_mut().clear();
        let mut project = Project::default();
        let config = load_config();
        project.export = config.export;
        project.export_scale = config.export_scale;
        *self.project.borrow_mut() = project;
        self.emit(AppEvent::ProjectCleared);
    }

    /// Undo the last [`dispatch`].
    /// Returns `true` if an undo step was available.
    pub fn undo(&self) -> bool {
        let snapshot = match self.undo_stack.borrow_mut().pop() {
            Some(s) => s,
            None => return false,
        };
        let current = self.project.borrow().clone();
        let mut redo = self.redo_stack.borrow_mut();
        if redo.len() >= MAX_UNDO {
            redo.remove(0);
        }
        redo.push(current);
        drop(redo);
        *self.project.borrow_mut() = snapshot;
        self.emit(AppEvent::ProjectChanged);
        true
    }

    /// Redo the last undone operation.
    /// Returns `true` if a redo step was available.
    pub fn redo(&self) -> bool {
        let snapshot = match self.redo_stack.borrow_mut().pop() {
            Some(s) => s,
            None => return false,
        };
        self.undo_stack
            .borrow_mut()
            .push(self.project.borrow().clone());
        *self.project.borrow_mut() = snapshot;
        self.emit(AppEvent::ProjectChanged);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.borrow().is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.borrow().is_empty()
    }

    /// Create a new event subscriber. Events are cloned to every subscriber.
    pub fn subscribe(&self) -> async_channel::Receiver<AppEvent> {
        let (tx, rx) = async_channel::unbounded();
        self.event_txs.borrow_mut().push(tx);
        rx
    }

    fn emit(&self, event: AppEvent) {
        let txs = self.event_txs.borrow();
        for tx in txs.iter() {
            let _ = tx.try_send(event.clone());
        }
    }

    fn push_undo(&self) {
        let mut undo = self.undo_stack.borrow_mut();
        undo.push(self.project.borrow().clone());
        if undo.len() > MAX_UNDO {
            undo.remove(0);
        }
    }

    fn apply(&self, cmd: &Command) -> Vec<AppEvent> {
        let mut p = self.project.borrow_mut();
        match cmd {
            Command::AddPages(paths) => {
                let start = p.pages.len();
                for path in paths {
                    p.pages.push(Page::new(path.clone()));
                }
                let indices: Vec<usize> = (start..p.pages.len()).collect();
                if indices.is_empty() {
                    vec![]
                } else {
                    vec![AppEvent::PagesAdded(indices)]
                }
            }

            Command::RemovePages(indices) => {
                let mut sorted: Vec<usize> = indices.clone();
                sorted.sort_unstable();
                for &i in sorted.iter().rev() {
                    if i < p.pages.len() {
                        p.pages.remove(i);
                    }
                }
                if indices.is_empty() {
                    vec![]
                } else {
                    vec![AppEvent::PagesRemoved(indices.clone())]
                }
            }

            Command::MovePages { indices, before } => {
                match plan_move(p.pages.len(), indices, *before) {
                    Some(plan) => {
                        plan.apply(&mut p.pages);
                        vec![AppEvent::PagesReordered(plan.permutation)]
                    }
                    None => vec![],
                }
            }

            Command::SetRotation { index, rotation } => {
                if let Some(page) = p.pages.get_mut(*index) {
                    page.rotation = *rotation;
                    vec![AppEvent::PageChanged(*index)]
                } else {
                    vec![]
                }
            }

            Command::SetCrop { index, crop } => {
                if let Some(page) = p.pages.get_mut(*index) {
                    page.crop = *crop;
                    vec![AppEvent::PageChanged(*index)]
                } else {
                    vec![]
                }
            }

            Command::SetCropPreset { index, preset } => {
                if let Some(page) = p.pages.get_mut(*index) {
                    page.crop_preset = *preset;
                    vec![AppEvent::PageChanged(*index)]
                } else {
                    vec![]
                }
            }

            Command::SetOutputSize { index, size } => {
                if let Some(page) = p.pages.get_mut(*index) {
                    page.output = *size;
                    vec![AppEvent::PageChanged(*index)]
                } else {
                    vec![]
                }
            }

            Command::SetBrightness(val) => {
                p.brightness = *val;
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetContrast(val) => {
                p.contrast = *val;
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetSaturation(val) => {
                p.saturation = *val;
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetExportScale(scale) => {
                p.export_scale = *scale;
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetBleed(val) => {
                p.bleed = *val;
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetExportSettings(settings) => {
                p.export = settings.clone();
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetOutputDir(dir) => {
                p.output_dir = dir.clone();
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::SetPrefix(prefix) => {
                p.prefix = prefix.clone();
                vec![AppEvent::GlobalSettingsChanged]
            }

            Command::AddCropPreset(preset) => {
                p.crop_presets.push(preset.clone());
                vec![AppEvent::PresetsChanged]
            }

            Command::RemoveCropPreset(index) => {
                if *index >= p.crop_presets.len() {
                    return vec![];
                }
                p.crop_presets.remove(*index);
                for page in p.pages.iter_mut() {
                    match page.crop_preset {
                        Some(pi) if pi == *index => {
                            page.crop_preset = None;
                            page.crop = None;
                        }
                        Some(pi) if pi > *index => page.crop_preset = Some(pi - 1),
                        _ => {}
                    }
                }
                for (i, preset) in p.crop_presets.iter_mut().enumerate() {
                    preset.name = format!("Preset {}", i + 1);
                }
                vec![AppEvent::PresetsChanged]
            }

            Command::SetCropPresetLocked { index, locked } => {
                if let Some(preset) = p.crop_presets.get_mut(*index) {
                    preset.locked = *locked;
                }
                vec![AppEvent::PresetsChanged]
            }

            Command::SetCropPresetSize { index, w, h } => {
                let Some(preset) = p.crop_presets.get_mut(*index) else {
                    return vec![];
                };
                if preset.locked {
                    return vec![];
                }
                preset.w = *w;
                preset.h = *h;
                for page in p.pages.iter_mut() {
                    if page.crop_preset == Some(*index) {
                        if let Some(c) = &mut page.crop {
                            c.w = *w;
                            c.h = *h;
                        }
                    }
                }
                vec![AppEvent::PresetsChanged]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::{Bleed, Brightness, Rotation};
    use std::path::PathBuf;

    fn project_with_page() -> Project {
        let mut p = Project::default();
        p.pages.push(Page::new(PathBuf::from("a.png")));
        p.pages.push(Page::new(PathBuf::from("b.png")));
        p
    }

    #[test]
    fn add_pages_emits_indices() {
        let (state, rx) = AppState::new(Project::default());
        state.dispatch(Command::AddPages(vec![
            PathBuf::from("1.png"),
            PathBuf::from("2.png"),
        ]));
        assert_eq!(state.project().pages.len(), 2);
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::PagesAdded(vec![0, 1]));
    }

    #[test]
    fn set_bleed_emits_global_settings_and_undoes() {
        let (state, rx) = AppState::new(Project::default());
        state.dispatch(Command::SetBleed(Bleed::new(20)));
        assert_eq!(state.project().bleed, Bleed::new(20));
        assert_eq!(rx.try_recv().unwrap(), AppEvent::GlobalSettingsChanged);
        assert!(state.undo());
        assert_eq!(state.project().bleed, Bleed::ZERO);
    }

    #[test]
    fn remove_pages_reverse_order() {
        let (state, rx) = AppState::new(project_with_page());
        state.dispatch(Command::RemovePages(vec![1, 0]));
        assert!(state.project().pages.is_empty());
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::PagesRemoved(vec![1, 0]));
    }

    #[test]
    fn set_rotation_updates_page() {
        let (state, rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        assert_eq!(state.project().pages[0].rotation, Rotation::DEG90);
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::PageChanged(0));
    }

    #[test]
    fn set_rotation_270() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG270,
        });
        assert_eq!(state.project().pages[0].rotation, Rotation::DEG270);
    }

    #[test]
    fn set_brightness_clamps_to_range() {
        let (state, rx) = AppState::new(Project::default());
        state.dispatch(Command::SetBrightness(Brightness::new(2.0)));
        assert_eq!(state.project().brightness, Brightness::MAX);
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::GlobalSettingsChanged);
    }

    #[test]
    fn oob_index_is_noop() {
        let (state, _rx) = AppState::new(Project::default());
        state.dispatch(Command::SetRotation {
            index: 99,
            rotation: Rotation::DEG90,
        });
        assert!(state.project().pages.is_empty());
    }

    #[test]
    fn load_project_emits_event_and_replaces() {
        let (state, rx) = AppState::new(project_with_page());
        let mut new_project = Project::default();
        new_project.pages.push(Page::new(PathBuf::from("c.png")));
        state.load_project(new_project);
        assert_eq!(state.project().pages.len(), 1);
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::ProjectLoaded);
    }

    #[test]
    fn clear_resets_project() {
        let (state, rx) = AppState::new(project_with_page());
        state.clear();
        assert!(state.project().pages.is_empty());
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::ProjectCleared);
    }

    #[test]
    fn undo_reverts_last_dispatch() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        assert_eq!(state.project().pages[0].rotation, Rotation::DEG90);
        drop(state.project());
        assert!(state.can_undo());
        assert!(state.undo());
        assert_eq!(state.project().pages[0].rotation, Rotation::ZERO);
    }

    #[test]
    fn redo_reapplies_undone_command() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        assert!(state.undo());
        assert!(state.can_redo());
        assert!(state.redo());
        assert_eq!(state.project().pages[0].rotation, Rotation::DEG90);
    }

    #[test]
    fn undo_emits_project_changed() {
        let (state, rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetBrightness(Brightness::new(0.3)));
        let _ = rx.try_recv();
        assert!(state.undo());
        let event = rx.try_recv().unwrap();
        assert_eq!(event, AppEvent::ProjectChanged);
    }

    #[test]
    fn redo_after_new_dispatch_is_noop() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        assert!(state.undo());
        state.dispatch(Command::SetBrightness(Brightness::new(0.1)));
        assert!(!state.can_redo());
        assert!(!state.redo());
    }

    #[test]
    fn clear_discards_undo_history() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        state.clear();
        assert!(state.project().pages.is_empty());
        assert!(!state.can_undo());
        assert!(!state.undo());
    }

    #[test]
    fn load_project_discards_undo_history() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        state.load_project(Project::default());
        assert!(!state.can_undo());
        assert!(!state.redo());
    }

    fn project_with_pages(names: &[&str]) -> Project {
        let mut p = Project::default();
        for name in names {
            p.pages.push(Page::new(PathBuf::from(name)));
        }
        p
    }

    fn page_names(state: &AppState) -> Vec<String> {
        state
            .project()
            .pages
            .iter()
            .map(|p| p.path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn move_pages_moves_page_and_emits_permutation() {
        let (state, rx) = AppState::new(project_with_page());
        state.dispatch(Command::MovePages {
            indices: vec![0],
            before: 2,
        });
        assert_eq!(state.project().pages[1].path, PathBuf::from("a.png"));
        assert_eq!(rx.try_recv().unwrap(), AppEvent::PagesReordered(vec![1, 0]));
    }

    #[test]
    fn move_pages_gathers_non_contiguous_selection() {
        let (state, _rx) = AppState::new(project_with_pages(&["a", "b", "c", "d", "e", "f"]));
        state.dispatch(Command::MovePages {
            indices: vec![4, 1],
            before: 3,
        });
        assert_eq!(page_names(&state), ["a", "c", "b", "e", "d", "f"]);
    }

    #[test]
    fn move_pages_to_end_appends() {
        let (state, _rx) = AppState::new(project_with_pages(&["a", "b", "c"]));
        state.dispatch(Command::MovePages {
            indices: vec![0],
            before: 3,
        });
        assert_eq!(page_names(&state), ["b", "c", "a"]);
    }

    #[test]
    fn move_pages_noop_emits_nothing() {
        let (state, rx) = AppState::new(project_with_page());
        state.dispatch(Command::MovePages {
            indices: vec![0],
            before: 1,
        });
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn move_pages_oob_emits_nothing() {
        let (state, rx) = AppState::new(project_with_page());
        state.dispatch(Command::MovePages {
            indices: vec![0],
            before: 5,
        });
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn move_pages_undoes_in_a_single_step() {
        let (state, _rx) = AppState::new(project_with_pages(&["a", "b", "c", "d"]));
        state.dispatch(Command::MovePages {
            indices: vec![0, 1],
            before: 4,
        });
        assert_eq!(page_names(&state), ["c", "d", "a", "b"]);
        assert!(state.undo());
        assert_eq!(page_names(&state), ["a", "b", "c", "d"]);
        assert!(!state.can_undo());
    }

    #[test]
    fn dispatch_without_undo_skips_stack() {
        let (state, _rx) = AppState::new(project_with_page());
        state.dispatch_without_undo(Command::SetRotation {
            index: 0,
            rotation: Rotation::DEG90,
        });
        assert_eq!(state.project().pages[0].rotation, Rotation::DEG90);
        assert!(!state.can_undo());
    }

    #[test]
    fn empty_undo_is_noop() {
        let (state, _rx) = AppState::new(project_with_page());
        assert!(!state.can_undo());
        assert!(!state.undo());
        assert!(!state.can_redo());
        assert!(!state.redo());
    }
}
