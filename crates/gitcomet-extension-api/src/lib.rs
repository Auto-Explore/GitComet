//! The contract between the GitComet host and compiled-in extensions.
//!
//! An [`Extension`] declares what it contributes through a [`Registrar`]:
//! repository views, status items, settings pages, commands and their key
//! bindings and menu entries, assets, repository-entry gates, and close
//! guards. The host validates every declaration and freezes the set before
//! the first window opens, so contributions never change while windows run.
//!
//! At run time an extension reaches the host only through handles:
//! [`WindowHost`] for a window and [`RepositoryHandle`] for a repository in a
//! window. Handles are weak: once their window closes or their repository
//! closes they report [`HostError`] instead of keeping anything alive.
//!
//! Core and state types (snapshots, `Msg` dispatch) are available as
//! revision-pinned integration APIs: extensions may read existing state and
//! dispatch existing messages, but reducers stay the host's.

mod annotations;
pub mod contributions;
pub use annotations::*;
pub mod files;
pub use files::*;
pub mod sidebar;
pub use sidebar::*;
pub mod host;
pub mod id;
pub mod lifecycle;
pub mod panes;
pub mod presentation;
pub use presentation::{HostedAction, HostedMenuItem, NotificationKind};
pub mod registry;
pub mod services;
pub mod storage;
pub use services::{RepositoryReader, StoreView, SyntaxService};

pub use contributions::{
    BottomPanelDescriptor, ChromeDescriptor, CloseDecision, CloseGuard, CloseRequest, CloseScope,
    CommandContext, CommandDescriptor, CommandHandler, DetailsTabDescriptor, EntryOrigin,
    GateDecision, MenuLocation, Navigate, NavigationAvailability, RepositoryEntryGate,
    RepositoryEntryRequest, RepositoryViewContext, RepositoryViewDescriptor, SettingsPageContext,
    SettingsPageDescriptor, SettingsTarget, SidebarSectionDescriptor, StatusItemDescriptor,
    ViewBuilder, ViewNavigation, ViewTarget, WindowGateDescriptor, WindowGatePredicate,
};
/// Revision-pinned state types hosted panes take.
pub use gitcomet_state::diff_session::ChangeSource;
pub use host::{
    DialogContent, DialogHandle, HostError, OnWindowClosed, PopOutWindow, RepositoryHandle,
    RepositoryWatch, StateObserver, StateSubscription, WindowContent, WindowHost,
};
pub use id::{ContributionId, ExtensionId, IdError};
pub use lifecycle::{HostNotifier, ShellEvent, Slot, SlotSignal, WindowExtension};
pub use panes::{
    DiffAnnotation, DiffAnnotations, DiffFileNavigation, DiffGutterAction, DiffInset, DiffLayout,
    DiffLegendItem, DiffLineRange, DiffLineSide, DiffPane, DiffPaneEvent, DiffPaneEventHandler,
    DiffPaneOptions, DiffPanePolicy, DiffRowDecor, DiffRowDecorProvider, DiffRowStyle,
    DiffScrollAnchor, DiffSelectionAction, DiffSelectionRun, DiffSnapshot, FileList, FileListMode,
    FileSelected,
};
pub use registry::{
    KeyBindingDeclaration, MenuItemDeclaration, Registrar, RegistrationError, Registry,
};
pub use storage::StorageError;

/// A compiled-in extension. The host calls [`Extension::register`] once, before
/// any window opens.
pub trait Extension: 'static {
    fn id(&self) -> ExtensionId;

    fn register(&self, registrar: &mut Registrar);

    /// Called once per window after it opens; [`WindowHost::kind`] says which.
    /// The returned state receives that window's [`ShellEvent`]s and is
    /// dropped with it.
    fn window_opened(
        &self,
        _host: WindowHost,
        _window: &mut gitcomet_ui_kit::gpui::Window,
        _cx: &mut gitcomet_ui_kit::gpui::App,
    ) -> Option<Box<dyn WindowExtension>> {
        None
    }
}
