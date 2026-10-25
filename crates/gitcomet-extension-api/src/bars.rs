//! Placement of window controls, independent of the code that owns them.

/// A row in the repository window's chrome. Items retain their owner's
/// ordering, state, eligibility and handlers when moved to another row.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
#[non_exhaustive]
pub enum BarItemLocation {
    ActionBarStart,
    ActionBarEnd,
    StatusBarStart,
    #[default]
    StatusBarEnd,
}

impl BarItemLocation {
    pub const fn is_action_bar(self) -> bool {
        matches!(self, Self::ActionBarStart | Self::ActionBarEnd)
    }
}

/// A host-owned control or related group of controls. Moving a group keeps
/// its interactions together (for example, branch tracking and Pull/Push).
/// Extensions choose placement with [`crate::Registrar::place_bar_item`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[non_exhaustive]
pub enum BuiltinBarItem {
    Navigation,
    Worktree,
    Tracking,
    HistoricalBrowse,
    OperationControls,
    /// The selected repository view's own context, in place of History's.
    ViewContext,
    RepositoryViews,
    Terminal,
    CreateBranch,
    Stash,
    SidebarToggle,
    FilesystemProgress,
    DetailsToggle,
    HookActivity,
    Documents,
    Zoom,
    /// The edition strip, or the host's community, edition, brand and version links.
    Branding,
}

impl BuiltinBarItem {
    /// Upstream placement, also used when no extension overrides this item.
    pub const fn default_location(self) -> BarItemLocation {
        use BarItemLocation::*;
        match self {
            Self::Navigation
            | Self::Worktree
            | Self::Tracking
            | Self::HistoricalBrowse
            | Self::OperationControls
            | Self::ViewContext => ActionBarStart,
            Self::RepositoryViews | Self::Terminal | Self::CreateBranch | Self::Stash => {
                ActionBarEnd
            }
            Self::SidebarToggle | Self::FilesystemProgress => StatusBarStart,
            Self::DetailsToggle
            | Self::HookActivity
            | Self::Documents
            | Self::Zoom
            | Self::Branding => StatusBarEnd,
        }
    }
}
