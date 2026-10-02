//! Handles to the host: weak references that fail cleanly once their window or
//! repository is gone.

use crate::id::{ContributionId, ExtensionId};
use crate::storage::StorageError;
use gitcomet_state::model::{AppState, RepoId};
use gitcomet_state::msg::Msg;
use gitcomet_ui_kit::gpui::{AnyView, App, HighlightStyle, SharedString, Window, WindowId};
use gitcomet_ui_kit::theme::AppTheme;
use std::cell::RefCell;
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Why a handle could not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostError {
    /// The handle's window has closed.
    WindowClosed,
    /// The handle's repository has closed, or its tab now holds another
    /// repository.
    RepositoryClosed,
    /// This window cannot do it (for example a focused mergetool window).
    Unsupported,
    /// The request itself was refused, such as a URL scheme that may not be
    /// opened.
    InvalidRequest(SharedString),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WindowClosed => "the window has closed",
            Self::RepositoryClosed => "the repository has closed",
            Self::Unsupported => "this window does not support the request",
            Self::InvalidRequest(reason) => reason,
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
    #[doc(hidden)]
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
    #[doc(hidden)]
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
    #[doc(hidden)]
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
#[doc(hidden)]
pub trait WindowHostImpl {
    fn repository_reader(&self) -> crate::RepositoryReader;
    fn kind(&self) -> gitcomet_core::identity::WindowKind;
    fn notifier(&self) -> crate::HostNotifier;
    fn navigate(
        &self,
        repository: &RepositoryHandle,
        target: crate::ViewTarget,
        cx: &mut App,
    ) -> Result<(), HostError>;
    fn open_settings_at(
        &self,
        target: crate::SettingsTarget,
        cx: &mut App,
    ) -> Result<(), HostError>;
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

    /// A diff pane on `target` in `repository`, owned by the returned handle.
    fn create_diff_pane(
        &self,
        repository: &RepositoryHandle,
        target: gitcomet_core::domain::DiffTarget,
        options: crate::panes::DiffPaneOptions,
        cx: &mut App,
    ) -> Result<crate::panes::DiffPane, HostError>;

    /// A diff pane over `snapshot`, with no repository behind it.
    fn create_snapshot_pane(
        &self,
        snapshot: crate::panes::DiffSnapshot,
        options: crate::panes::DiffPaneOptions,
        cx: &mut App,
    ) -> Result<crate::panes::DiffPane, HostError>;

    /// A file list of `source`'s changes in `repository`, owned by the
    /// returned handle; `on_select` runs when the user picks a file.
    fn create_file_list(
        &self,
        repository: &RepositoryHandle,
        source: gitcomet_state::diff_session::ChangeSource,
        on_select: crate::panes::FileSelected,
        cx: &mut App,
    ) -> Result<crate::panes::FileList, HostError>;

    /// Opens bottom panel `panel` for `repository` and shows it; records the
    /// open at once and builds the view after the current update.
    fn open_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        cx: &mut App,
    ) -> Result<(), HostError>;

    fn close_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        cx: &mut App,
    ) -> Result<(), HostError>;

    fn is_bottom_panel_open(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        cx: &App,
    ) -> bool;

    /// Watches `repository` until the returned value drops.
    fn watch_repository(
        &self,
        repository: &RepositoryHandle,
        cx: &App,
    ) -> Result<RepositoryWatch, HostError>;

    fn watch_worktree(
        &self,
        repository: &RepositoryHandle,
        path: &Path,
        cx: &App,
    ) -> Result<RepositoryWatch, HostError>;

    /// Syntax highlights for one line of `path`'s text in the window's
    /// theme; empty when the language is unknown.
    fn highlight_line(
        &self,
        path: &Path,
        text: &str,
        cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)>;

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

    fn open_popover(
        &self,
        title: SharedString,
        anchor: gitcomet_ui_kit::gpui::Point<gitcomet_ui_kit::gpui::Pixels>,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError>;

    fn open_menu(
        &self,
        anchor: gitcomet_ui_kit::gpui::Point<gitcomet_ui_kit::gpui::Pixels>,
        items: Vec<crate::HostedMenuItem>,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError>;

    fn toast(
        &self,
        kind: crate::NotificationKind,
        message: SharedString,
        actions: Vec<crate::HostedAction>,
        cx: &mut App,
    ) -> Result<(), HostError>;

    /// Opens a window of its own showing `content`, titled `title`.
    fn open_window(
        &self,
        title: SharedString,
        content: WindowContent,
        on_closed: OnWindowClosed,
        cx: &mut App,
    ) -> Result<PopOutWindow, HostError>;

    /// Shows a transient notification.
    fn notify(&self, message: SharedString, cx: &mut App) -> Result<(), HostError>;

    fn open_url(&self, url: &str, cx: &mut App) -> Result<(), HostError>;

    fn open_path(&self, path: &Path, cx: &mut App) -> Result<(), HostError>;

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

/// Builds a pop-out window's content.
pub type WindowContent = Box<dyn FnOnce(&mut Window, &mut App) -> AnyView>;

/// Runs once when a pop-out window closes, however it closes.
pub type OnWindowClosed = Box<dyn FnOnce(&mut App)>;

/// What the host implements behind a [`PopOutWindow`].
#[doc(hidden)]
pub trait PopOutImpl {
    fn close(&self, cx: &mut App);
    fn is_open(&self, cx: &App) -> bool;
}

/// A window opened with [`WindowHost::open_window`]. Dropping the handle
/// leaves it open; it closes with the window that opened it.
#[derive(Clone)]
pub struct PopOutWindow(Rc<dyn PopOutImpl>);

impl PopOutWindow {
    #[doc(hidden)]
    pub fn new(window: Rc<dyn PopOutImpl>) -> Self {
        Self(window)
    }

    pub fn close(&self, cx: &mut App) {
        self.0.close(cx)
    }

    pub fn is_open(&self, cx: &App) -> bool {
        self.0.is_open(cx)
    }
}

/// A weak handle to one window of the host.
///
/// A Settings window ([`WindowKind::Settings`](gitcomet_core::identity::WindowKind))
/// has no repository: its state is empty, and navigation, repository panes
/// and file lists, bottom panels, watches, `dispatch`, and workspace state
/// answer [`HostError::Unsupported`]. Dialogs, popovers, menus, toasts,
/// snapshot panes, pop-out windows, and syntax highlighting work there.
#[derive(Clone)]
pub struct WindowHost(Rc<dyn WindowHostImpl>);

impl WindowHost {
    #[doc(hidden)]
    pub fn new(host: Rc<dyn WindowHostImpl>) -> Self {
        Self(host)
    }

    pub fn repository_reader(&self) -> crate::RepositoryReader {
        self.0.repository_reader()
    }

    pub fn store_view(&self) -> crate::StoreView {
        crate::StoreView(self.clone())
    }

    pub fn syntax(&self) -> crate::SyntaxService {
        crate::SyntaxService(self.clone())
    }

    pub fn id(&self) -> WindowId {
        self.0.window_id()
    }

    pub fn kind(&self) -> gitcomet_core::identity::WindowKind {
        self.0.kind()
    }

    pub fn notifier(&self) -> crate::HostNotifier {
        self.0.notifier()
    }

    pub fn navigate(
        &self,
        repository: &RepositoryHandle,
        target: crate::ViewTarget,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.check(repository, cx)?;
        self.0.navigate(repository, target, cx)
    }

    pub fn open_settings_at(
        &self,
        target: crate::SettingsTarget,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.0.open_settings_at(target, cx)
    }

    /// Invalidates only the owner of the given slot, after this update.
    pub fn invalidate(&self, slot: crate::Slot, _cx: &mut App) {
        self.0.notifier().notify(slot);
    }

    pub fn storage_dir(&self, extension: &ExtensionId) -> Option<PathBuf> {
        crate::storage::storage_dir(extension)
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

    /// A diff pane on `target`, independent of History and every other pane.
    pub fn create_diff_pane(
        &self,
        repository: &RepositoryHandle,
        target: gitcomet_core::domain::DiffTarget,
        options: crate::panes::DiffPaneOptions,
        cx: &mut App,
    ) -> Result<crate::panes::DiffPane, HostError> {
        self.check(repository, cx)?;
        self.0.create_diff_pane(repository, target, options, cx)
    }

    /// A diff pane over two texts rather than a repository; blame is
    /// unavailable.
    pub fn create_snapshot_pane(
        &self,
        snapshot: crate::panes::DiffSnapshot,
        options: crate::panes::DiffPaneOptions,
        cx: &mut App,
    ) -> Result<crate::panes::DiffPane, HostError> {
        self.0.create_snapshot_pane(snapshot, options, cx)
    }

    /// A file list of `source`'s changes; `on_select` gets each pick's target.
    pub fn create_file_list(
        &self,
        repository: &RepositoryHandle,
        source: gitcomet_state::diff_session::ChangeSource,
        on_select: impl Fn(
            &gitcomet_core::domain::CommitFileChange,
            gitcomet_core::domain::DiffTarget,
            &mut App,
        ) + 'static,
        cx: &mut App,
    ) -> Result<crate::panes::FileList, HostError> {
        self.check(repository, cx)?;
        self.0
            .create_file_list(repository, source, Rc::new(on_select), cx)
    }

    /// Opens this extension's bottom panel `panel` for `repository` and
    /// brings it to the front. [`HostError::Unsupported`] when no bottom
    /// panel has that id.
    pub fn open_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.check(repository, cx)?;
        self.0.open_bottom_panel(repository, panel, cx)
    }

    /// Closes the panel and drops its view.
    pub fn close_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.check(repository, cx)?;
        self.0.close_bottom_panel(repository, panel, cx)
    }

    pub fn is_bottom_panel_open(
        &self,
        repository: &RepositoryHandle,
        panel: &ContributionId,
        cx: &App,
    ) -> bool {
        self.0.is_bottom_panel_open(repository, panel, cx)
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

    /// Watches a linked worktree until the returned lease drops.
    pub fn watch_worktree(
        &self,
        repository: &RepositoryHandle,
        path: &Path,
        cx: &App,
    ) -> Result<RepositoryWatch, HostError> {
        self.check(repository, cx)?;
        self.0.watch_worktree(repository, path, cx)
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

    /// Like [`Self::observe_state`], but calls `observer` only when
    /// `select`'s result differs from the one it last saw (the current state's
    /// at subscription), and hands it that result.
    pub fn observe_selected<K: PartialEq + 'static>(
        &self,
        select: impl Fn(&AppState) -> K + 'static,
        observer: impl Fn(&WindowHost, &K, &mut App) + 'static,
        cx: &App,
    ) -> Result<StateSubscription, HostError> {
        let last = RefCell::new(select(self.state(cx)?.as_ref()));
        self.observe_state(move |host, cx| {
            let Ok(state) = host.state(cx) else {
                return;
            };
            let next = select(&state);
            if *last.borrow() == next {
                return;
            }
            observer(host, &next, cx);
            *last.borrow_mut() = next;
        })
    }

    /// Syntax highlights for one line of `path`'s text in the window's
    /// theme, for views that draw file text themselves.
    pub fn highlight_line(
        &self,
        path: &Path,
        text: &str,
        cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.0.highlight_line(path, text, cx)
    }

    pub fn open_dialog(
        &self,
        title: impl Into<SharedString>,
        content: impl FnOnce(&mut Window, &mut App) -> AnyView + 'static,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.0.open_dialog(title.into(), Box::new(content), cx)
    }

    pub fn open_popover(
        &self,
        title: impl Into<SharedString>,
        anchor: gitcomet_ui_kit::gpui::Point<gitcomet_ui_kit::gpui::Pixels>,
        content: impl FnOnce(&mut Window, &mut App) -> AnyView + 'static,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.0
            .open_popover(title.into(), anchor, Box::new(content), cx)
    }

    pub fn open_menu(
        &self,
        anchor: gitcomet_ui_kit::gpui::Point<gitcomet_ui_kit::gpui::Pixels>,
        items: Vec<crate::HostedMenuItem>,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.0.open_menu(anchor, items, cx)
    }

    pub fn toast(
        &self,
        kind: crate::NotificationKind,
        message: impl Into<SharedString>,
        actions: Vec<crate::HostedAction>,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.0.toast(kind, message.into(), actions, cx)
    }

    pub fn report_error(
        &self,
        message: impl Into<SharedString>,
        actions: Vec<crate::HostedAction>,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.toast(crate::NotificationKind::Error, message, actions, cx)
    }

    /// Opens a window of its own showing `content`, such as a pane's view
    /// popped out of the extension's layout (mount a view in one window at a
    /// time). It closes with this window; `on_closed` runs once when it
    /// closes, however it closes.
    pub fn open_window(
        &self,
        title: impl Into<SharedString>,
        content: impl FnOnce(&mut Window, &mut App) -> AnyView + 'static,
        on_closed: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> Result<PopOutWindow, HostError> {
        self.0
            .open_window(title.into(), Box::new(content), Box::new(on_closed), cx)
    }

    pub fn notify(&self, message: impl Into<SharedString>, cx: &mut App) -> Result<(), HostError> {
        self.0.notify(message.into(), cx)
    }

    /// Opens `url` in the default browser once this update ends, off the UI
    /// thread. Schemes that run code or reach local files are refused with
    /// [`HostError::InvalidRequest`]; a failed launch is reported in the
    /// window.
    pub fn open_url(&self, url: &str, cx: &mut App) -> Result<(), HostError> {
        self.0.open_url(url, cx)
    }

    /// Opens `path` with its default application, like [`Self::open_url`].
    pub fn open_path(&self, path: &Path, cx: &mut App) -> Result<(), HostError> {
        self.0.open_path(path, cx)
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
