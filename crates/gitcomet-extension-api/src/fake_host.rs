//! A stand-in window host for extension tests (`test-support` feature).
//!
//! It keeps workspace state in memory and records what an extension asked the
//! host to show or do. Requests that need a real window, such as diff panes
//! and file lists, fail with [`HostError::Unsupported`]; dialogs, popovers,
//! menus and pop-out windows are recorded by title without building content.

use crate::host::{
    DialogContent, DialogHandle, HostError, OnWindowClosed, PopOutImpl, PopOutWindow,
    RepositoryHandle, RepositoryWatch, StateObserver, WindowContent, WindowHost, WindowHostImpl,
};
use crate::id::{ContributionId, ExtensionId};
use crate::storage::StorageError;
use crate::{HostNotifier, HostedAction, HostedMenuItem, NotificationKind, RepositoryReader};
use crate::{SettingsTarget, ViewTarget};
use gitcomet_core::identity::WindowKind;
use gitcomet_state::model::AppState;
use gitcomet_state::msg::Msg;
use gitcomet_ui_kit::gpui::{App, HighlightStyle, Pixels, Point, SharedString, WindowId};
use gitcomet_ui_kit::theme::AppTheme;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Something an extension asked the host to show or do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HostRequest {
    Navigate {
        repository: RepositoryHandle,
        target: ViewTarget,
    },
    OpenSettings(SettingsTarget),
    SetCommitMessage {
        repository: RepositoryHandle,
        message: String,
    },
    OpenBottomPanel {
        repository: RepositoryHandle,
        panel: ContributionId,
    },
    CloseBottomPanel {
        repository: RepositoryHandle,
        panel: ContributionId,
    },
    ShowCanvasOverlay {
        repository: RepositoryHandle,
        overlay: ContributionId,
    },
    HideCanvasOverlay {
        repository: RepositoryHandle,
        overlay: ContributionId,
    },
    DropCanvasOverlay {
        repository: RepositoryHandle,
        overlay: ContributionId,
    },
    Dialog {
        title: SharedString,
    },
    Popover {
        title: SharedString,
    },
    Menu {
        items: usize,
    },
    Toast {
        kind: NotificationKind,
        message: SharedString,
    },
    Window {
        title: SharedString,
    },
    Notify(SharedString),
    OpenUrl(String),
    OpenPath(PathBuf),
    OpenTerminal {
        repository: RepositoryHandle,
        cwd: PathBuf,
        command: Option<String>,
    },
    RevealCommit {
        repository: RepositoryHandle,
        commit: gitcomet_core::domain::CommitId,
    },
}

/// A window host for tests. Pass [`FakeWindowHost::host`] to extension code
/// and read back what it did with [`FakeWindowHost::requests`],
/// [`FakeWindowHost::take_dispatched`] and the workspace state accessors.
pub struct FakeWindowHost {
    window: WindowId,
    kind: WindowKind,
    theme: AppTheme,
    state: RefCell<Arc<AppState>>,
    active: RefCell<Option<RepositoryHandle>>,
    open: RefCell<Vec<RepositoryHandle>>,
    workspace: RefCell<BTreeMap<ExtensionId, serde_json::Value>>,
    open_panels: RefCell<HashSet<(RepositoryHandle, ContributionId)>>,
    /// The canvas overlay shown over each repository.
    shown_overlays: RefCell<HashMap<RepositoryHandle, ContributionId>>,
    requests: RefCell<Vec<HostRequest>>,
    dispatched: RefCell<Vec<Msg>>,
    observers: RefCell<Vec<(u64, StateObserver)>>,
    next_observer: Cell<u64>,
    next_repo: Cell<u64>,
    /// What each menu still open runs once closed.
    menus_open: RefCell<Vec<crate::OnMenuClosed>>,
}

impl FakeWindowHost {
    /// A main window with the default dark theme, an empty state and no
    /// active repository.
    pub fn new(window: WindowId) -> Rc<Self> {
        Rc::new(Self {
            window,
            kind: WindowKind::Main,
            theme: AppTheme::gitcomet_dark(),
            state: RefCell::default(),
            active: RefCell::default(),
            open: RefCell::default(),
            workspace: RefCell::default(),
            open_panels: RefCell::default(),
            shown_overlays: RefCell::default(),
            requests: RefCell::default(),
            dispatched: RefCell::default(),
            observers: RefCell::default(),
            next_observer: Cell::new(1),
            next_repo: Cell::new(1),
            menus_open: RefCell::default(),
        })
    }

    /// The handle extension code receives.
    pub fn host(self: &Rc<Self>) -> WindowHost {
        WindowHost::new(self.clone())
    }

    /// A repository handle in this window, made the active repository. Those
    /// opened before stay open, as tabs do.
    pub fn open_repository(&self, workdir: impl Into<PathBuf>) -> RepositoryHandle {
        let repo_id = gitcomet_state::model::RepoId(self.next_repo.get());
        self.next_repo.set(repo_id.0 + 1);
        let handle = RepositoryHandle::new(self.window, repo_id, 1, workdir.into());
        self.open.borrow_mut().push(handle.clone());
        *self.active.borrow_mut() = Some(handle.clone());
        handle
    }

    /// Makes an open repository the active one, as selecting its tab does.
    pub fn select_repository(&self, repository: &RepositoryHandle) {
        assert!(
            self.open.borrow().contains(repository),
            "select_repository: {repository:?} is not open in this fake window"
        );
        *self.active.borrow_mut() = Some(repository.clone());
    }

    /// Closes a repository: its handle stops being current, and no repository
    /// is active if it was.
    pub fn close_repository(&self, repository: &RepositoryHandle) {
        self.open.borrow_mut().retain(|open| open != repository);
        let mut active = self.active.borrow_mut();
        if active.as_ref() == Some(repository) {
            *active = None;
        }
    }

    /// Replaces the state snapshot and runs the registered observers, as the
    /// host does after a store update.
    pub fn set_state(self: &Rc<Self>, state: Arc<AppState>, cx: &mut App) {
        *self.state.borrow_mut() = state;
        let observers: Vec<StateObserver> = self
            .observers
            .borrow()
            .iter()
            .map(|(_, observer)| observer.clone())
            .collect();
        let host = self.host();
        for observer in observers {
            observer(&host, cx);
        }
    }

    /// Everything requested so far, oldest first.
    /// Closes every menu opened with a close callback, running each.
    pub fn close_menus(&self, cx: &mut App) {
        let menus = std::mem::take(&mut *self.menus_open.borrow_mut());
        for on_closed in menus {
            on_closed(cx);
        }
    }

    pub fn requests(&self) -> Vec<HostRequest> {
        self.requests.borrow().clone()
    }

    /// The messages dispatched since the last call.
    pub fn take_dispatched(&self) -> Vec<Msg> {
        std::mem::take(&mut self.dispatched.borrow_mut())
    }

    /// The saved state of `extension` for this window's workspace.
    pub fn saved_workspace_state(&self, extension: &ExtensionId) -> Option<serde_json::Value> {
        self.workspace.borrow().get(extension).cloned()
    }

    fn record(&self, request: HostRequest) {
        self.requests.borrow_mut().push(request);
    }

    fn forget_overlay(&self, repository: &RepositoryHandle, overlay: &ContributionId) {
        let mut shown = self.shown_overlays.borrow_mut();
        if shown.get(repository) == Some(overlay) {
            shown.remove(repository);
        }
    }
}

struct FakePopOut(Cell<bool>);

impl PopOutImpl for FakePopOut {
    fn close(&self, _cx: &mut App) {
        self.0.set(false);
    }

    fn is_open(&self, _cx: &App) -> bool {
        self.0.get()
    }
}

impl WindowHostImpl for FakeWindowHost {
    fn repository_reader(&self) -> RepositoryReader {
        RepositoryReader::new(self.window, |_, _| None)
    }

    fn kind(&self) -> WindowKind {
        self.kind
    }

    fn notifier(&self) -> HostNotifier {
        HostNotifier::new(|| {})
    }

    fn navigate(
        &self,
        repository: &RepositoryHandle,
        target: ViewTarget,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.record(HostRequest::Navigate {
            repository: repository.clone(),
            target,
        });
        Ok(())
    }

    fn open_settings_at(&self, target: SettingsTarget, _cx: &mut App) -> Result<(), HostError> {
        self.record(HostRequest::OpenSettings(target));
        Ok(())
    }

    fn set_commit_message(
        &self,
        repository: &RepositoryHandle,
        message: String,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.record(HostRequest::SetCommitMessage {
            repository: repository.clone(),
            message,
        });
        Ok(())
    }

    fn reveal_commit(
        &self,
        repository: &RepositoryHandle,
        commit: &gitcomet_core::domain::CommitId,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.record(HostRequest::RevealCommit {
            repository: repository.clone(),
            commit: commit.clone(),
        });
        Ok(())
    }

    fn window_id(&self) -> WindowId {
        self.window
    }

    fn is_open(&self, _cx: &App) -> bool {
        true
    }

    fn active_repository(&self, _cx: &App) -> Result<Option<RepositoryHandle>, HostError> {
        Ok(self.active.borrow().clone())
    }

    fn state(&self, _cx: &App) -> Result<Arc<AppState>, HostError> {
        Ok(self.state.borrow().clone())
    }

    fn theme(&self, _cx: &App) -> AppTheme {
        self.theme
    }

    fn observe_state(&self, observer: StateObserver) -> Result<u64, HostError> {
        let id = self.next_observer.get();
        self.next_observer.set(id + 1);
        self.observers.borrow_mut().push((id, observer));
        Ok(id)
    }

    fn unobserve_state(&self, id: u64) {
        self.observers
            .borrow_mut()
            .retain(|(observer, _)| *observer != id);
    }

    fn create_diff_pane(
        &self,
        _repository: &RepositoryHandle,
        _target: gitcomet_core::domain::DiffTarget,
        _options: crate::panes::DiffPaneOptions,
        _cx: &mut App,
    ) -> Result<crate::panes::DiffPane, HostError> {
        Err(HostError::Unsupported)
    }

    fn create_snapshot_pane(
        &self,
        _snapshot: crate::panes::DiffSnapshot,
        _options: crate::panes::DiffPaneOptions,
        _cx: &mut App,
    ) -> Result<crate::panes::DiffPane, HostError> {
        Err(HostError::Unsupported)
    }

    fn create_file_list(
        &self,
        _repository: &RepositoryHandle,
        _source: gitcomet_state::diff_session::ChangeSource,
        _on_select: crate::panes::FileSelected,
        _cx: &mut App,
    ) -> Result<crate::panes::FileList, HostError> {
        Err(HostError::Unsupported)
    }

    fn open_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.open_panels
            .borrow_mut()
            .insert((repository.clone(), panel.clone()));
        self.record(HostRequest::OpenBottomPanel {
            repository: repository.clone(),
            panel: panel.clone(),
        });
        Ok(())
    }

    fn close_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.open_panels
            .borrow_mut()
            .remove(&(repository.clone(), panel.clone()));
        self.record(HostRequest::CloseBottomPanel {
            repository: repository.clone(),
            panel: panel.clone(),
        });
        Ok(())
    }

    fn is_bottom_panel_open(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        _cx: &App,
    ) -> bool {
        self.open_panels
            .borrow()
            .contains(&(repository.clone(), panel.clone()))
    }

    fn show_canvas_overlay(
        &self,
        repository: &RepositoryHandle,
        overlay: &ContributionId,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.shown_overlays
            .borrow_mut()
            .insert(repository.clone(), overlay.clone());
        self.record(HostRequest::ShowCanvasOverlay {
            repository: repository.clone(),
            overlay: overlay.clone(),
        });
        Ok(())
    }

    fn hide_canvas_overlay(
        &self,
        repository: &RepositoryHandle,
        overlay: &ContributionId,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.forget_overlay(repository, overlay);
        self.record(HostRequest::HideCanvasOverlay {
            repository: repository.clone(),
            overlay: overlay.clone(),
        });
        Ok(())
    }

    fn drop_canvas_overlay(
        &self,
        repository: &RepositoryHandle,
        overlay: &ContributionId,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.forget_overlay(repository, overlay);
        self.record(HostRequest::DropCanvasOverlay {
            repository: repository.clone(),
            overlay: overlay.clone(),
        });
        Ok(())
    }

    fn is_canvas_overlay_shown(
        &self,
        repository: &RepositoryHandle,
        overlay: &ContributionId,
        cx: &App,
    ) -> bool {
        self.is_current(repository, cx)
            && self.shown_overlays.borrow().get(repository) == Some(overlay)
    }

    fn watch_repository(
        &self,
        _repository: &RepositoryHandle,
        _cx: &App,
    ) -> Result<RepositoryWatch, HostError> {
        Ok(RepositoryWatch::new(Box::new(())))
    }

    fn watch_worktree(
        &self,
        _repository: &RepositoryHandle,
        _path: &Path,
        _cx: &App,
    ) -> Result<RepositoryWatch, HostError> {
        Ok(RepositoryWatch::new(Box::new(())))
    }

    fn highlight_line(
        &self,
        _path: &Path,
        _text: &str,
        _cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        Vec::new()
    }

    fn is_current(&self, repository: &RepositoryHandle, _cx: &App) -> bool {
        self.open.borrow().contains(repository)
    }

    fn dispatch(&self, msg: Msg, _cx: &mut App) -> Result<(), HostError> {
        self.dispatched.borrow_mut().push(msg);
        Ok(())
    }

    fn open_dialog(
        &self,
        title: SharedString,
        _content: DialogContent,
        _cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.record(HostRequest::Dialog { title });
        Ok(DialogHandle::new(|_| {}))
    }

    fn open_popover(
        &self,
        title: SharedString,
        _anchor: Point<Pixels>,
        _content: DialogContent,
        _cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.record(HostRequest::Popover { title });
        Ok(DialogHandle::new(|_| {}))
    }

    fn open_menu(
        &self,
        _anchor: Point<Pixels>,
        items: Vec<HostedMenuItem>,
        _cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.record(HostRequest::Menu { items: items.len() });
        Ok(DialogHandle::new(|_| {}))
    }

    fn open_menu_with_on_closed(
        &self,
        _anchor: Point<Pixels>,
        items: Vec<HostedMenuItem>,
        on_closed: crate::OnMenuClosed,
        _cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.record(HostRequest::Menu { items: items.len() });
        self.menus_open.borrow_mut().push(on_closed);
        Ok(DialogHandle::new(|_| {}))
    }

    fn toast(
        &self,
        kind: NotificationKind,
        message: SharedString,
        _actions: Vec<HostedAction>,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.record(HostRequest::Toast { kind, message });
        Ok(())
    }

    fn open_window(
        &self,
        title: SharedString,
        _content: WindowContent,
        _on_closed: OnWindowClosed,
        _cx: &mut App,
    ) -> Result<PopOutWindow, HostError> {
        self.record(HostRequest::Window { title });
        Ok(PopOutWindow::new(Rc::new(FakePopOut(Cell::new(true)))))
    }

    fn notify(&self, message: SharedString, _cx: &mut App) -> Result<(), HostError> {
        self.record(HostRequest::Notify(message));
        Ok(())
    }

    fn open_url(&self, url: &str, _cx: &mut App) -> Result<(), HostError> {
        self.record(HostRequest::OpenUrl(url.to_string()));
        Ok(())
    }

    fn open_path(&self, path: &Path, _cx: &mut App) -> Result<(), HostError> {
        self.record(HostRequest::OpenPath(path.to_path_buf()));
        Ok(())
    }

    fn open_terminal(
        &self,
        repository: &RepositoryHandle,
        cwd: &Path,
        command: Option<&str>,
        _cx: &mut App,
    ) -> Result<(), HostError> {
        self.record(HostRequest::OpenTerminal {
            repository: repository.clone(),
            cwd: cwd.to_path_buf(),
            command: command.map(str::to_owned),
        });
        Ok(())
    }

    fn workspace_state(
        &self,
        extension: &ExtensionId,
        _cx: &App,
    ) -> Result<Option<serde_json::Value>, HostError> {
        Ok(self.saved_workspace_state(extension))
    }

    fn set_workspace_state(
        &self,
        extension: &ExtensionId,
        value: serde_json::Value,
        _cx: &mut App,
    ) -> Result<Result<(), StorageError>, HostError> {
        self.workspace.borrow_mut().insert(extension.clone(), value);
        Ok(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn repositories_stay_current_until_closed(cx: &mut gpui::TestAppContext) {
        let fake = FakeWindowHost::new(WindowId::from(3));
        let host = fake.host();
        let first = fake.open_repository("/tmp/first");
        let second = fake.open_repository("/tmp/second");
        cx.update(|cx| {
            assert_eq!(host.active_repository(cx), Ok(Some(second.clone())));
            assert_eq!(
                host.set_commit_message(&first, "Off screen".into(), cx),
                Ok(())
            );

            fake.select_repository(&first);
            assert_eq!(host.active_repository(cx), Ok(Some(first.clone())));

            fake.close_repository(&first);
            assert_eq!(host.active_repository(cx), Ok(None));
            assert_eq!(
                host.set_commit_message(&first, "Too late".into(), cx),
                Err(HostError::RepositoryClosed)
            );
            assert_eq!(host.check(&second, cx), Ok(()));
        });
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fake.select_repository(&first)
        }));
        assert!(refused.is_err(), "a closed repository cannot be selected");
        assert_eq!(
            fake.requests(),
            [HostRequest::SetCommitMessage {
                repository: first,
                message: "Off screen".into(),
            }]
        );
    }

    #[gpui::test]
    fn canvas_overlays_are_one_per_repository_and_recorded(cx: &mut gpui::TestAppContext) {
        let fake = FakeWindowHost::new(WindowId::from(3));
        let host = fake.host();
        let extension = ExtensionId::new("com.example.overlays").unwrap();
        let first = extension.contribution("first").unwrap();
        let second = extension.contribution("second").unwrap();
        let repository = fake.open_repository("/tmp/repository");
        cx.update(|cx| {
            assert_eq!(host.show_canvas_overlay(&repository, &first, cx), Ok(()));
            assert_eq!(host.show_canvas_overlay(&repository, &second, cx), Ok(()));
            assert!(!host.is_canvas_overlay_shown(&repository, &first, cx));
            assert!(host.is_canvas_overlay_shown(&repository, &second, cx));

            assert_eq!(host.hide_canvas_overlay(&repository, &first, cx), Ok(()));
            assert!(
                host.is_canvas_overlay_shown(&repository, &second, cx),
                "hiding an overlay that is not shown leaves the shown one"
            );
            assert_eq!(host.hide_canvas_overlay(&repository, &second, cx), Ok(()));
            assert!(!host.is_canvas_overlay_shown(&repository, &second, cx));

            assert_eq!(host.show_canvas_overlay(&repository, &first, cx), Ok(()));
            assert_eq!(host.drop_canvas_overlay(&repository, &first, cx), Ok(()));
            assert!(!host.is_canvas_overlay_shown(&repository, &first, cx));

            assert_eq!(host.show_canvas_overlay(&repository, &second, cx), Ok(()));
        });
        cx.update(|cx| {
            assert_eq!(host.show_canvas_overlay(&repository, &second, cx), Ok(()));
        });
        let next = fake.open_repository("/tmp/next");
        cx.update(|cx| {
            assert!(!host.is_canvas_overlay_shown(&next, &second, cx));
            assert_eq!(host.show_canvas_overlay(&next, &first, cx), Ok(()));
            assert!(
                host.is_canvas_overlay_shown(&repository, &second, cx),
                "each repository has its own overlay"
            );
        });
        fake.close_repository(&repository);
        cx.update(|cx| {
            assert!(
                !host.is_canvas_overlay_shown(&repository, &second, cx),
                "a closed repository shows no overlay"
            );
            assert_eq!(
                host.show_canvas_overlay(&repository, &first, cx),
                Err(HostError::RepositoryClosed)
            );
            assert!(host.is_canvas_overlay_shown(&next, &first, cx));
        });

        let show = |overlay: &ContributionId| HostRequest::ShowCanvasOverlay {
            repository: repository.clone(),
            overlay: overlay.clone(),
        };
        assert_eq!(
            fake.requests(),
            [
                show(&first),
                show(&second),
                HostRequest::HideCanvasOverlay {
                    repository: repository.clone(),
                    overlay: first.clone(),
                },
                HostRequest::HideCanvasOverlay {
                    repository: repository.clone(),
                    overlay: second.clone(),
                },
                show(&first),
                HostRequest::DropCanvasOverlay {
                    repository: repository.clone(),
                    overlay: first.clone(),
                },
                show(&second),
                show(&second),
                HostRequest::ShowCanvasOverlay {
                    repository: next,
                    overlay: first,
                },
            ]
        );
    }
}
