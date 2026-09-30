//! What an extension can contribute. Every descriptor is plain data plus the
//! callbacks the host invokes on the UI thread.
//!
//! Views are GPUI entities the extension owns: when an extension's state
//! changes it calls `cx.notify()` on its own entity, and the host redraws that
//! view. No host render is needed for a contribution to update.

use crate::host::{RepositoryHandle, WindowHost};
use gitcomet_ui_kit::gpui::{AnyView, App, SharedString, Window};
use gitcomet_ui_kit::theme::AppTheme;
use std::path::PathBuf;
use std::rc::Rc;

/// Builds a contribution's view for one window or repository.
pub type ViewBuilder<C> = Rc<dyn Fn(C, &mut Window, &mut App) -> AnyView>;

/// Runs a command.
pub type CommandHandler = Rc<dyn Fn(CommandContext, &mut Window, &mut App)>;

/// What a repository view is built for: its window and repository.
#[derive(Clone, Debug)]
pub struct RepositoryViewContext {
    pub window: WindowHost,
    pub repository: RepositoryHandle,
}

/// A view of a repository, selectable next to History in the repository's
/// navigation. The view is built when first selected in a window and kept
/// while the repository stays open there.
#[derive(Clone)]
pub struct RepositoryViewDescriptor {
    pub title: SharedString,
    pub icon: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
    /// Navigation belongs to the selected view, including mouse side buttons.
    pub navigation: Option<ViewNavigation>,
}

pub type NavigationAvailability = Rc<dyn Fn(&RepositoryViewContext, &App) -> bool>;
pub type Navigate = Rc<dyn Fn(RepositoryViewContext, &mut App)>;

#[derive(Clone)]
pub struct ViewNavigation {
    pub can_back: NavigationAvailability,
    pub can_forward: NavigationAvailability,
    pub back: Navigate,
    pub forward: Navigate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewTarget {
    History,
    Extension(crate::ContributionId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsTarget {
    /// A built-in page's stable name, such as `general`, `diff`, or `git-log`.
    Builtin(SharedString),
    Extension(crate::ContributionId),
}

/// A panel in the repository's bottom area beside the terminal and reflog,
/// opened with [`WindowHost::open_bottom_panel`](crate::WindowHost::open_bottom_panel).
/// Built on open and dropped on close.
#[derive(Clone)]
pub struct BottomPanelDescriptor {
    pub title: SharedString,
    pub icon: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
}

/// A tab beside the details pane's own content. Built when first selected in
/// a window and kept while the repository stays open there.
#[derive(Clone)]
pub struct DetailsTabDescriptor {
    pub title: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
}

/// A section below the sidebar's own, for the active repository. Built once
/// per repository in a window and kept while it stays open there.
#[derive(Clone)]
pub struct SidebarSectionDescriptor {
    pub title: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
}

/// An item in the window's status bar, built once per window.
#[derive(Clone)]
pub struct StatusItemDescriptor {
    /// Restricts the item to a repository view; `None` shows it in every view.
    pub view: Option<ViewTarget>,
    pub build: ViewBuilder<WindowHost>,
}

/// A page in the Settings window. Settings builds only the selected page,
/// each time it is selected, with the window's theme at that moment.
#[derive(Clone)]
pub struct SettingsPageDescriptor {
    pub title: SharedString,
    pub icon: SharedString,
    /// Extra search terms matched by the Settings search.
    pub keywords: SharedString,
    pub build: ViewBuilder<AppTheme>,
}

/// Where a command runs: the window, and its active repository if any.
#[derive(Clone)]
pub struct CommandContext {
    pub window: WindowHost,
    pub repository: Option<RepositoryHandle>,
}

/// A command in the command palette, the target of key bindings and menu
/// items.
#[derive(Clone)]
pub struct CommandDescriptor {
    pub label: SharedString,
    pub category: SharedString,
    pub keywords: SharedString,
    /// Hidden while no repository is active.
    pub requires_repository: bool,
    pub run: CommandHandler,
}

/// Menus that accept extension entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuLocation {
    /// The application menu (macOS menu bar, and the in-window app menu).
    Application,
    /// A repository tab's context menu.
    RepositoryTab,
}

/// How a repository is being opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryOrigin {
    /// The command line or a request forwarded by another process.
    CommandLine,
    /// The Home screen or the open-repository dialog.
    Chooser,
    /// A folder dropped on a window.
    Drop,
    /// A saved workspace being restored.
    WorkspaceRestore,
}

#[derive(Clone, Debug)]
pub struct RepositoryEntryRequest {
    pub path: PathBuf,
    pub origin: EntryOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    /// Refuse the entry; the host shows `reason`.
    Deny {
        reason: SharedString,
    },
}

/// Decides whether a repository may open. Gates run in registration order;
/// the first denial wins.
pub type RepositoryEntryGate = Rc<dyn Fn(&RepositoryEntryRequest, &App) -> GateDecision>;

/// What is being closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseScope {
    Application,
    Window,
    Repository,
}

#[derive(Clone, Debug)]
pub struct CloseRequest {
    pub scope: CloseScope,
    pub window: WindowHost,
    pub repository: Option<RepositoryHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseDecision {
    Allow,
    /// Ask before closing; the host shows `reason` with a way to proceed.
    Confirm {
        reason: SharedString,
    },
}

/// Runs before a close, after the host's own unsaved-edit, terminal, and Git
/// operation guards.
pub type CloseGuard = Rc<dyn Fn(&CloseRequest, &App) -> CloseDecision>;

/// A full-window gate. Bump `signal` and notify `Slot::Gate` when its condition changes.
/// Gates are tested in registration order; only the first active gate is built.
pub type WindowGatePredicate = Rc<dyn Fn(&WindowHost, &App) -> bool>;

#[derive(Clone)]
pub struct WindowGateDescriptor {
    pub signal: crate::SlotSignal,
    pub active: WindowGatePredicate,
    pub build: ViewBuilder<WindowHost>,
}

/// Replaces one of the host's static chrome slots for the life of a window.
#[derive(Clone)]
pub struct ChromeDescriptor {
    pub build: ViewBuilder<WindowHost>,
}
