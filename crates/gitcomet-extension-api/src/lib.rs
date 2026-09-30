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

pub mod contributions;
pub mod host;
pub mod id;
pub mod panes;
pub mod registry;
pub mod storage;

pub use contributions::{
    BottomPanelDescriptor, CloseDecision, CloseGuard, CloseRequest, CloseScope, CommandContext,
    CommandDescriptor, CommandHandler, DetailsTabDescriptor, EntryOrigin, GateDecision,
    MenuLocation, RepositoryEntryGate, RepositoryEntryRequest, RepositoryViewContext,
    RepositoryViewDescriptor, SettingsPageDescriptor, SidebarSectionDescriptor,
    StatusItemDescriptor, ViewBuilder,
};
/// Revision-pinned state types hosted panes take.
pub use gitcomet_state::diff_session::ChangeSource;
pub use host::{
    DialogContent, DialogHandle, HostError, RepositoryHandle, RepositoryWatch, StateObserver,
    StateSubscription, WindowHost, WindowHostImpl,
};
pub use id::{ContributionId, ExtensionId, IdError};
pub use panes::{
    DiffAnnotation, DiffAnnotations, DiffGutterAction, DiffInset, DiffLegendItem, DiffLineRange,
    DiffLineSide, DiffPane, DiffPaneImpl, DiffPaneOptions, DiffPanePolicy, DiffRowDecor,
    DiffRowDecorProvider, DiffRowStyle, DiffSelectionAction, DiffSelectionRun, DiffSnapshot,
    FileList, FileListImpl, FileListMode, FileSelected,
};
pub use registry::{Registrar, RegistrationError, Registry};

/// A compiled-in extension. The host calls [`Extension::register`] once, before
/// any window opens.
pub trait Extension: 'static {
    fn id(&self) -> ExtensionId;

    fn register(&self, registrar: &mut Registrar);
}
