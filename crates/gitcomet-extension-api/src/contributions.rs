//! What an extension can contribute. Every descriptor is plain data plus the
//! callbacks the host invokes on the UI thread.
//!
//! Views are GPUI entities the extension owns: when an extension's state
//! changes it calls `cx.notify()` on its own entity, and the host redraws that
//! view. No host render is needed for a contribution to update.
//!
//! Descriptors are built with `new` and `with_*`, so fields added later do not
//! break an extension.

use crate::host::{RepositoryHandle, WindowHost};
use gitcomet_ui_kit::gpui::{AnyView, App, FocusHandle, Focusable, SharedString, Window};
use gitcomet_ui_kit::theme::AppTheme;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Builds a contribution's view for one window or repository.
pub type ViewBuilder<C> = Rc<dyn Fn(C, &mut Window, &mut App) -> AnyView>;

/// The keyboard scope of a built repository view.
pub type ViewFocus = Rc<dyn Fn(&AnyView, &App) -> Option<FocusHandle>>;

/// Runs a command.
pub type CommandHandler = Rc<dyn Fn(CommandContext, &mut Window, &mut App)>;

/// What a repository view is built for: its window and repository.
#[derive(Clone, Debug)]
pub struct RepositoryViewContext {
    pub window: WindowHost,
    pub repository: RepositoryHandle,
}

/// A view of a repository, selectable next to History from the tabs in the
/// action bar. The view is built when first selected in a window and kept
/// while the repository stays open there.
#[derive(Clone)]
#[non_exhaustive]
pub struct RepositoryViewDescriptor {
    pub title: SharedString,
    pub icon: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
    /// Focused by the host when the user switches to this view.
    pub focus: Option<ViewFocus>,
    /// Navigation belongs to the selected view, including mouse side buttons.
    pub navigation: Option<ViewNavigation>,
    /// The action bar's context while the view is selected: shown after
    /// Back/Forward in place of History's branch, tracking and merge
    /// controls, which a selected view always hides. Built with the view.
    pub action_bar: Option<ViewBuilder<RepositoryViewContext>>,
    /// Listed in the action bar's More menu instead of having a tab of its
    /// own, for a view used less often. While it is selected, More names it.
    pub under_more: bool,
    /// The content offered in each surrounding panel while this view is active.
    pub panels: BTreeMap<PanelArea, PanelContentPolicy>,
}

impl RepositoryViewDescriptor {
    pub fn new(
        title: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        build: impl Fn(RepositoryViewContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            icon: icon.into(),
            build: Rc::new(build),
            focus: None,
            navigation: None,
            action_bar: None,
            under_more: false,
            panels: BTreeMap::new(),
        }
    }

    pub fn with_navigation(mut self, navigation: ViewNavigation) -> Self {
        self.navigation = Some(navigation);
        self
    }

    /// Give the keyboard to the built view of type `V` on selection. The
    /// host does this once per view switch, never on redraw or activation.
    pub fn with_focus<V: Focusable + 'static>(mut self) -> Self {
        self.focus = Some(Rc::new(|view, cx| {
            view.clone()
                .downcast::<V>()
                .ok()
                .map(|view| view.read(cx).focus_handle(cx))
        }));
        self
    }

    pub fn with_action_bar(
        mut self,
        build: impl Fn(RepositoryViewContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.action_bar = Some(Rc::new(build));
        self
    }

    /// Lists the view in the action bar's More menu rather than as a tab.
    pub fn under_more(mut self) -> Self {
        self.under_more = true;
        self
    }

    /// Sets which content a surrounding panel offers for this view.
    pub fn with_panel_content(mut self, area: PanelArea, policy: PanelContentPolicy) -> Self {
        self.panels.insert(area, policy);
        self
    }

    pub fn panel_content(&self, area: PanelArea) -> PanelContentPolicy {
        self.panels.get(&area).copied().unwrap_or_default()
    }
}

pub type NavigationAvailability = Rc<dyn Fn(&RepositoryViewContext, &App) -> bool>;
pub type Navigate = Rc<dyn Fn(RepositoryViewContext, &mut App)>;

#[derive(Clone)]
#[non_exhaustive]
pub struct ViewNavigation {
    pub can_back: NavigationAvailability,
    pub can_forward: NavigationAvailability,
    pub back: Navigate,
    pub forward: Navigate,
}

impl ViewNavigation {
    pub fn new(
        can_back: impl Fn(&RepositoryViewContext, &App) -> bool + 'static,
        can_forward: impl Fn(&RepositoryViewContext, &App) -> bool + 'static,
        back: impl Fn(RepositoryViewContext, &mut App) + 'static,
        forward: impl Fn(RepositoryViewContext, &mut App) + 'static,
    ) -> Self {
        Self {
            can_back: Rc::new(can_back),
            can_forward: Rc::new(can_forward),
            back: Rc::new(back),
            forward: Rc::new(forward),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ViewTarget {
    History,
    Extension(crate::ContributionId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingsTarget {
    /// A built-in page's stable name, such as `general`, `diff`, or `git-log`.
    Builtin(SharedString),
    Extension(crate::ContributionId),
}

/// A panel in the repository's bottom area beside the terminal and reflog,
/// opened with [`WindowHost::open_bottom_panel`](crate::WindowHost::open_bottom_panel).
/// Built on open and dropped on close.
#[derive(Clone)]
#[non_exhaustive]
pub struct BottomPanelDescriptor {
    pub title: SharedString,
    pub icon: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
}

impl BottomPanelDescriptor {
    pub fn new(
        title: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        build: impl Fn(RepositoryViewContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            icon: icon.into(),
            build: Rc::new(build),
        }
    }
}

/// A surrounding panel that accepts tabs. This identifies placement,
/// independent of the extension or workflow that supplies its content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[non_exhaustive]
pub enum PanelArea {
    Details,
    Sidebar,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PanelContentPolicy {
    /// The host's tabs and the contributions listed for the active view.
    #[default]
    HostAndExtensions,
    /// Only the contributions listed for the active view. The first tab is
    /// selected if none requests `opens_with_view`. Empty panels retain the
    /// host's content so a view cannot leave an unusable blank panel.
    ExtensionsOnly,
}

/// A tab in a repository panel. Built when first selected in
/// a window and kept while the repository stays open there.
#[derive(Clone)]
#[non_exhaustive]
pub struct PanelTabDescriptor {
    pub title: SharedString,
    /// Shown before the title in the tab strip.
    pub icon: Option<SharedString>,
    pub build: ViewBuilder<RepositoryViewContext>,
    /// Lists the tab only while this repository view is selected; `None`
    /// lists it in every view.
    pub view: Option<ViewTarget>,
    /// Selects the tab whenever its view is selected. Leaving the view
    /// brings back the tab that was selected before.
    pub opens_with_view: bool,
}

impl PanelTabDescriptor {
    pub fn new(
        title: impl Into<SharedString>,
        build: impl Fn(RepositoryViewContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            icon: None,
            build: Rc::new(build),
            view: None,
            opens_with_view: false,
        }
    }

    pub fn with_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Lists the tab only while `view` is selected.
    pub fn with_view(mut self, view: ViewTarget) -> Self {
        self.view = Some(view);
        self
    }

    /// Selects the tab when its view is selected; see [`Self::opens_with_view`].
    pub fn opens_with_view(mut self) -> Self {
        self.opens_with_view = true;
        self
    }
}

/// A panel tab placed in the details area.
pub type DetailsTabDescriptor = PanelTabDescriptor;
/// A panel tab placed in the sidebar.
pub type SidebarTabDescriptor = PanelTabDescriptor;

/// A section below the sidebar's own, for the active repository. Built once
/// per repository in a window and kept while it stays open there.
#[derive(Clone)]
#[non_exhaustive]
pub struct SidebarSectionDescriptor {
    pub title: SharedString,
    pub build: ViewBuilder<RepositoryViewContext>,
}

impl SidebarSectionDescriptor {
    pub fn new(
        title: impl Into<SharedString>,
        build: impl Fn(RepositoryViewContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            build: Rc::new(build),
        }
    }
}

/// An item in the window's status bar, built once per window.
#[derive(Clone)]
#[non_exhaustive]
pub struct StatusItemDescriptor {
    /// Restricts the item to a repository view; `None` shows it in every view.
    pub view: Option<ViewTarget>,
    pub build: ViewBuilder<WindowHost>,
}

impl StatusItemDescriptor {
    /// An item shown in every repository view.
    pub fn new(build: impl Fn(WindowHost, &mut Window, &mut App) -> AnyView + 'static) -> Self {
        Self {
            view: None,
            build: Rc::new(build),
        }
    }

    pub fn with_view(mut self, view: ViewTarget) -> Self {
        self.view = Some(view);
        self
    }
}

/// What a settings page is built with: the Settings window's host, for
/// dialogs, menus, and toasts, and its theme at that moment.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SettingsPageContext {
    pub host: WindowHost,
    pub theme: AppTheme,
}

impl SettingsPageContext {
    #[doc(hidden)]
    pub fn new(host: WindowHost, theme: AppTheme) -> Self {
        Self { host, theme }
    }
}

/// A page in the Settings window. Settings builds only the selected page,
/// each time it is selected.
#[derive(Clone)]
#[non_exhaustive]
pub struct SettingsPageDescriptor {
    pub title: SharedString,
    pub icon: SharedString,
    /// Extra search terms matched by the Settings search.
    pub keywords: SharedString,
    pub build: ViewBuilder<SettingsPageContext>,
}

impl SettingsPageDescriptor {
    pub fn new(
        title: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        build: impl Fn(SettingsPageContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            icon: icon.into(),
            keywords: SharedString::default(),
            build: Rc::new(build),
        }
    }

    pub fn with_keywords(mut self, keywords: impl Into<SharedString>) -> Self {
        self.keywords = keywords.into();
        self
    }
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
#[non_exhaustive]
pub struct CommandDescriptor {
    pub label: SharedString,
    pub category: SharedString,
    pub keywords: SharedString,
    /// Hidden while no repository is active.
    pub requires_repository: bool,
    pub run: CommandHandler,
}

impl CommandDescriptor {
    pub fn new(
        label: impl Into<SharedString>,
        category: impl Into<SharedString>,
        run: impl Fn(CommandContext, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            category: category.into(),
            keywords: SharedString::default(),
            requires_repository: false,
            run: Rc::new(run),
        }
    }

    pub fn with_keywords(mut self, keywords: impl Into<SharedString>) -> Self {
        self.keywords = keywords.into();
        self
    }

    /// Hides the command while no repository is active.
    pub fn requiring_repository(mut self) -> Self {
        self.requires_repository = true;
        self
    }
}

/// Menus that accept extension entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MenuLocation {
    /// The application menu (macOS menu bar, and the in-window app menu).
    Application,
    /// A repository tab's context menu.
    RepositoryTab,
}

/// How a repository is being opened. Gates should deny origins they do not
/// recognise rather than allow them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryOrigin {
    /// The command line or a request forwarded by another process.
    CommandLine,
    /// The Home screen, the open-repository dialog, or a submodule opened
    /// from its parent.
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

/// What is being closed. Guards should ask about scopes they do not
/// recognise rather than allow them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct WindowGateDescriptor {
    pub signal: crate::SlotSignal,
    pub active: WindowGatePredicate,
    pub build: ViewBuilder<WindowHost>,
}

impl WindowGateDescriptor {
    pub fn new(
        signal: crate::SlotSignal,
        active: impl Fn(&WindowHost, &App) -> bool + 'static,
        build: impl Fn(WindowHost, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            signal,
            active: Rc::new(active),
            build: Rc::new(build),
        }
    }
}

/// Replaces one of the host's static chrome slots for the life of a window.
#[derive(Clone)]
#[non_exhaustive]
pub struct ChromeDescriptor {
    pub build: ViewBuilder<WindowHost>,
}

impl ChromeDescriptor {
    pub fn new(build: impl Fn(WindowHost, &mut Window, &mut App) -> AnyView + 'static) -> Self {
        Self {
            build: Rc::new(build),
        }
    }
}
