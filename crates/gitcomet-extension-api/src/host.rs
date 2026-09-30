//! Handles to the host: weak references that fail cleanly once their window or
//! repository is gone.

use crate::id::ExtensionId;
use crate::storage::StorageError;
use gitcomet_state::model::{AppState, RepoId};
use gitcomet_state::msg::Msg;
use gitcomet_ui_kit::gpui::{AnyView, App, SharedString, Window, WindowId};
use gitcomet_ui_kit::theme::AppTheme;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Why a handle could not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The handle's window has closed.
    WindowClosed,
    /// The handle's repository has closed, or its tab now holds another
    /// repository.
    RepositoryClosed,
    /// This window cannot do it (for example a focused mergetool window).
    Unsupported,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WindowClosed => "the window has closed",
            Self::RepositoryClosed => "the repository has closed",
            Self::Unsupported => "this window does not support the request",
        })
    }
}

impl std::error::Error for HostError {}

/// A repository in one window. `RepoId`s are per window and may be reused,
/// so the handle also carries the repository's lifetime token; a handle
/// never silently points at a different repository.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepositoryHandle {
    window: WindowId,
    repo_id: RepoId,
    lifetime: u64,
    workdir: PathBuf,
}

impl RepositoryHandle {
    /// For hosts; extensions receive handles from the host.
    pub fn new(window: WindowId, repo_id: RepoId, lifetime: u64, workdir: PathBuf) -> Self {
        Self {
            window,
            repo_id,
            lifetime,
            workdir,
        }
    }

    pub fn window(&self) -> WindowId {
        self.window
    }

    /// Meaningful only together with [`Self::window`] and [`Self::lifetime`].
    pub fn repo_id(&self) -> RepoId {
        self.repo_id
    }

    pub fn lifetime(&self) -> u64 {
        self.lifetime
    }

    pub fn workdir(&self) -> &std::path::Path {
        &self.workdir
    }
}

/// A dialog an extension opened; closing it returns focus to where it was.
pub struct DialogHandle {
    close: Box<dyn FnOnce(&mut App)>,
}

impl DialogHandle {
    pub fn new(close: impl FnOnce(&mut App) + 'static) -> Self {
        Self {
            close: Box::new(close),
        }
    }

    pub fn close(self, cx: &mut App) {
        (self.close)(cx)
    }
}

/// Builds a dialog's content once the host is ready to show it.
pub type DialogContent = Box<dyn FnOnce(&mut Window, &mut App) -> AnyView>;

/// Called after the window's state changes.
pub type StateObserver = Rc<dyn Fn(&WindowHost, &mut App)>;

/// Keeps a repository's file watcher running while it is not the active one
/// (a view of a linked worktree, say); dropping it releases the watch.
/// Watches count, so several views can hold one repository.
#[must_use = "dropping the watch releases it"]
pub struct RepositoryWatch {
    _lease: Box<dyn std::any::Any>,
}

impl RepositoryWatch {
    /// For hosts: wraps whatever keeps the watch alive.
    pub fn new(lease: Box<dyn std::any::Any>) -> Self {
        Self { _lease: lease }
    }
}

/// Keeps a state observer registered; dropping it unregisters the observer.
#[must_use = "dropping the subscription unregisters the observer"]
pub struct StateSubscription {
    host: WindowHost,
    id: u64,
}

impl Drop for StateSubscription {
    fn drop(&mut self) {
        self.host.0.unobserve_state(self.id);
    }
}

/// What the host implements behind a [`WindowHost`]. Every method is weak:
/// a closed window answers [`HostError::WindowClosed`].
pub trait WindowHostImpl {
    fn window_id(&self) -> WindowId;

    fn is_open(&self, cx: &App) -> bool;

    /// The active repository, if the window shows one.
    fn active_repository(&self, cx: &App) -> Result<Option<RepositoryHandle>, HostError>;

    /// The window's current state snapshot (revision-pinned).
    fn state(&self, cx: &App) -> Result<Arc<AppState>, HostError>;

    /// The window's theme; read it while rendering so views follow changes.
    fn theme(&self, cx: &App) -> AppTheme;

    /// Registers `observer`, returning its id. The host calls observers after
    /// state changes, at most once per update cycle however many changes
    /// land in it, and never polls while none are registered.
    fn observe_state(&self, observer: StateObserver) -> Result<u64, HostError>;

    fn unobserve_state(&self, id: u64);

    /// Watches `repository` until the returned value drops.
    fn watch_repository(
        &self,
        repository: &RepositoryHandle,
        cx: &App,
    ) -> Result<RepositoryWatch, HostError>;

    /// Whether `repository` is still the repository it named when issued.
    fn is_current(&self, repository: &RepositoryHandle, cx: &App) -> bool;

    /// Dispatches an existing message to the window's store.
    fn dispatch(&self, msg: Msg, cx: &mut App) -> Result<(), HostError>;

    /// Shows `content` in a modal dialog titled `title`, deferred to the next
    /// update so it never re-enters the host's render.
    fn open_dialog(
        &self,
        title: SharedString,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError>;

    /// Shows a transient notification.
    fn notify(&self, message: SharedString, cx: &mut App) -> Result<(), HostError>;

    /// The extension's saved state for this window's workspace.
    fn workspace_state(
        &self,
        extension: &ExtensionId,
        cx: &App,
    ) -> Result<Option<serde_json::Value>, HostError>;

    /// Saves the extension's state for this window's workspace.
    fn set_workspace_state(
        &self,
        extension: &ExtensionId,
        value: serde_json::Value,
        cx: &mut App,
    ) -> Result<Result<(), StorageError>, HostError>;
}

/// A weak handle to one window of the host.
#[derive(Clone)]
pub struct WindowHost(Rc<dyn WindowHostImpl>);

impl WindowHost {
    pub fn new(host: Rc<dyn WindowHostImpl>) -> Self {
        Self(host)
    }

    pub fn id(&self) -> WindowId {
        self.0.window_id()
    }

    pub fn is_open(&self, cx: &App) -> bool {
        self.0.is_open(cx)
    }

    pub fn active_repository(&self, cx: &App) -> Result<Option<RepositoryHandle>, HostError> {
        self.0.active_repository(cx)
    }

    pub fn state(&self, cx: &App) -> Result<Arc<AppState>, HostError> {
        self.0.state(cx)
    }

    /// The window's theme, or its last one once the window has closed.
    pub fn theme(&self, cx: &App) -> AppTheme {
        self.0.theme(cx)
    }

    /// Fails with [`HostError::RepositoryClosed`] when `repository` no longer
    /// names what it named when issued.
    pub fn check(&self, repository: &RepositoryHandle, cx: &App) -> Result<(), HostError> {
        if repository.window() != self.id() {
            return Err(HostError::Unsupported);
        }
        if !self.is_open(cx) {
            return Err(HostError::WindowClosed);
        }
        if self.0.is_current(repository, cx) {
            Ok(())
        } else {
            Err(HostError::RepositoryClosed)
        }
    }

    pub fn dispatch(&self, msg: Msg, cx: &mut App) -> Result<(), HostError> {
        self.0.dispatch(msg, cx)
    }

    /// Keeps `repository`'s file watcher running (and its changes delivered)
    /// while it is not the active repository, until the watch drops.
    pub fn watch_repository(
        &self,
        repository: &RepositoryHandle,
        cx: &App,
    ) -> Result<RepositoryWatch, HostError> {
        self.check(repository, cx)?;
        self.0.watch_repository(repository, cx)
    }

    /// Calls `observer` after this window's state changes, coalesced to once
    /// per update cycle, until the subscription is dropped.
    pub fn observe_state(
        &self,
        observer: impl Fn(&WindowHost, &mut App) + 'static,
    ) -> Result<StateSubscription, HostError> {
        let id = self.0.observe_state(Rc::new(observer))?;
        Ok(StateSubscription {
            host: self.clone(),
            id,
        })
    }

    pub fn open_dialog(
        &self,
        title: impl Into<SharedString>,
        content: impl FnOnce(&mut Window, &mut App) -> AnyView + 'static,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.0.open_dialog(title.into(), Box::new(content), cx)
    }

    pub fn notify(&self, message: impl Into<SharedString>, cx: &mut App) -> Result<(), HostError> {
        self.0.notify(message.into(), cx)
    }

    pub fn workspace_state(
        &self,
        extension: &ExtensionId,
        cx: &App,
    ) -> Result<Option<serde_json::Value>, HostError> {
        self.0.workspace_state(extension, cx)
    }

    pub fn set_workspace_state(
        &self,
        extension: &ExtensionId,
        value: serde_json::Value,
        cx: &mut App,
    ) -> Result<Result<(), StorageError>, HostError> {
        self.0.set_workspace_state(extension, value, cx)
    }
}

impl fmt::Debug for WindowHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("WindowHost").field(&self.id()).finish()
    }
}

impl PartialEq for WindowHost {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}
