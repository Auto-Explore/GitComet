use super::*;
use gitcomet_core::services::InteractiveRebaseAction;

const COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX: f32 = 240.0;
const COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX: f32 = 200.0;

#[derive(Clone)]
pub(in crate::view) enum AppMenuAction {
    CommandPalette,
    /// Reveal the open file in the sidebar's file explorer. Like every other
    /// variant here, whether it can run is carried by the menu item's own
    /// `disabled` flag rather than duplicated in the payload.
    LocateFileInExplorer,
    Settings,
    OpenInCodeEditor {
        path: Option<std::path::PathBuf>,
    },
    ApplyPatch {
        repo_id: Option<RepoId>,
    },
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    InstallDesktopIntegration,
    Quit,
    CloseWindow,
}

#[derive(Clone, Copy)]
pub(in crate::view) enum AddRepoMenuAction {
    Open,
    Clone,
    Initialize,
}

#[derive(Clone)]
pub(in crate::view) enum ContextMenuAction {
    AppMenu(AppMenuAction),
    AddRepoMenu(AddRepoMenuAction),
    SelectDiff {
        repo_id: RepoId,
        target: DiffTarget,
    },
    SelectConflictDiff {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    OpenFile {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    OpenFileLocation {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    OpenRepositoryLocation {
        path: std::path::PathBuf,
    },
    OpenInCodeEditor {
        repo_id: Option<RepoId>,
        path: std::path::PathBuf,
    },
    OpenFileContent {
        repo_id: RepoId,
        source: gitcomet_core::domain::FileSource,
        path: std::path::PathBuf,
    },
    /// Open the working-tree file in GitComet's own editor. Carries no source:
    /// editing is always of the workspace copy, whatever view it was invoked
    /// from.
    EditFile {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    /// Throw away the editor's unsaved buffer for this file and reload it from
    /// disk. Handled in the view rather than dispatched: the buffer lives in
    /// `MainPaneView`, and the store has no message for it.
    DiscardFileEdits {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    BrowseRepositoryAtCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    ResetBrowseToLive {
        repo_id: RepoId,
    },
    OpenRepo {
        path: std::path::PathBuf,
    },
    ActivateRepo {
        repo_id: RepoId,
    },
    CloseRepo {
        repo_id: RepoId,
    },
    CloseRepos {
        repo_ids: Vec<RepoId>,
        activate_after: Option<RepoId>,
    },
    OpenSubmoduleDiffInTab {
        path: std::path::PathBuf,
        target: DiffTarget,
    },
    ExportPatch {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    MarkForComparison {
        repo_id: RepoId,
        commit_id: CommitId,
        label: String,
    },
    CompareWithMarked {
        repo_id: RepoId,
        commit_id: CommitId,
        label: String,
    },
    CompareWithWorkingTree {
        repo_id: RepoId,
        commit_id: CommitId,
        label: String,
    },
    ClearComparisonMark {
        repo_id: RepoId,
    },
    CheckoutCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    CherryPickCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    RevertCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    /// Opens the squash confirmation prompt for the current multi-selection.
    SquashSelectedCommits {
        repo_id: RepoId,
    },
    CheckoutBranch {
        repo_id: RepoId,
        name: String,
    },
    DeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    /// Pins/unpins a branch in the sidebar's dedicated "Pinned" section.
    ToggleBranchPin {
        repo_id: RepoId,
        section: BranchSection,
        name: String,
    },
    SetHistoryScope {
        repo_id: RepoId,
        scope: gitcomet_core::domain::LogScope,
    },
    SetDiffContentMode {
        mode: DiffContentMode,
    },
    SetDiffWhitespaceMode {
        mode: DiffWhitespaceMode,
    },
    SetDiffRevealWhitespaceChars {
        enabled: bool,
    },
    SetDiffWordWrap {
        enabled: bool,
    },
    SetDiffShowLineNumbers {
        enabled: bool,
    },
    SetChangeTrackingView {
        view: ChangeTrackingView,
    },
    SetCommitAmendEnabled {
        enabled: bool,
    },
    SetCommitPushAfterEnabled {
        enabled: bool,
    },
    UseCommitMessage {
        message: String,
    },
    SetUiScale {
        percent: u32,
    },
    StageSelectionOrPath {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
    },
    UnstageSelectionOrPath {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
    },
    DiscardWorktreeChangesSelectionOrPath {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
    },
    AddToGitignoreSelectionOrPath {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
    },
    CheckoutConflictSideSelectionOrPath {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
        side: gitcomet_core::services::ConflictSide,
    },
    LaunchMergetool {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    FetchAll {
        repo_id: RepoId,
    },
    PruneMergedBranches {
        repo_id: RepoId,
    },
    PruneLocalTags {
        repo_id: RepoId,
    },
    UpdateSubmodules {
        repo_id: RepoId,
    },
    LoadSubmodule {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    LoadWorktrees {
        repo_id: RepoId,
    },
    Pull {
        repo_id: RepoId,
        mode: PullMode,
    },
    PullBranch {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    MergeRef {
        repo_id: RepoId,
        reference: String,
    },
    SquashRef {
        repo_id: RepoId,
        reference: String,
    },
    ApplyStash {
        repo_id: RepoId,
        index: usize,
    },
    PopStash {
        repo_id: RepoId,
        index: usize,
    },
    DropStashConfirm {
        repo_id: RepoId,
        index: usize,
        message: String,
    },
    Push {
        repo_id: RepoId,
    },
    SetUpstreamBranch {
        repo_id: RepoId,
        branch: String,
        upstream: String,
    },
    UnsetUpstreamBranch {
        repo_id: RepoId,
        branch: String,
    },
    OpenPopover {
        kind: PopoverKind,
    },
    LoadInteractiveRebaseSetup {
        repo_id: RepoId,
        base: String,
    },
    OpenInteractiveCherryPickSetup {
        repo_id: RepoId,
        entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
        source_colors: Vec<(String, u8)>,
    },
    SetInteractiveRebaseAction {
        ix: usize,
        action: InteractiveRebaseAction,
    },
    SetInteractiveRebaseAutosquashMode {
        mode: AutosquashMode,
    },
    ConflictResolverPick {
        target: ResolverPickTarget,
    },
    ConflictResolverUnresolve {
        conflict_ix: usize,
    },
    ConflictResolverSplitSelection,
    /// kdiff3 manual diff help: pin the marked lines onto one another.
    ConflictResolverAlignManually,
    /// kdiff3 manual diff help: drop every pin and replan automatically.
    ConflictResolverClearManualAlignments,
    ConflictResolverJoinRegions {
        target: ConflictResolverJoinTarget,
    },
    SetMergetoolAutoAdvance {
        enabled: bool,
    },
    ToggleMergetoolCollapseUnchanged,
    SetMergetoolOutputScrollSync {
        enabled: bool,
    },
    SetMergetoolShowLineNumbers {
        enabled: bool,
    },
    SetMergetoolThreeWayView {
        enabled: bool,
    },
    ConflictResolverOutputCut {
        text: String,
    },
    ConflictResolverOutputPaste,
    CopyText {
        text: String,
    },
    /// Copy a link's destination, and say so.
    ///
    /// Separate from [`ContextMenuAction::CopyText`] because a link's address
    /// is never on screen — the document shows its text — so the reader has no
    /// way to tell the copy happened without being told.
    CopyLinkAddress {
        url: String,
    },
    OpenWebUrl {
        url: String,
    },
    CopyDiffSelection {
        text: String,
    },
    CopyDiffText {
        visible_ix: usize,
        region: DiffTextRegion,
    },
    TerminalCopy {
        repo_id: RepoId,
    },
    TerminalPaste {
        repo_id: RepoId,
    },
    TerminalSelectAll {
        repo_id: RepoId,
    },
    TerminalClear {
        repo_id: RepoId,
    },
    TerminalOpenExternal {
        repo_id: RepoId,
    },
    ApplyIndexPatch {
        repo_id: RepoId,
        patch: String,
        reverse: bool,
    },
    ApplyWorktreePatch {
        repo_id: RepoId,
        patch: String,
        reverse: bool,
    },
    StageHunk {
        repo_id: RepoId,
        src_ix: usize,
    },
    UnstageHunk {
        repo_id: RepoId,
        src_ix: usize,
    },
    DeleteTag {
        repo_id: RepoId,
        name: String,
    },
    PushTag {
        repo_id: RepoId,
        remote: String,
        name: String,
    },
    DeleteRemoteTag {
        repo_id: RepoId,
        remote: String,
        name: String,
    },
}

#[derive(Clone)]
enum ContextMenuItem {
    Separator,
    Header(components::ContextMenuText),
    /// Muted helper text placed directly under the menu's header.
    Description(components::ContextMenuText),
    Label(components::ContextMenuText),
    Entry {
        label: SharedString,
        icon: Option<SharedString>,
        shortcut: Option<SharedString>,
        disabled: bool,
        action: Box<ContextMenuAction>,
    },
    /// A caption plus a segmented control, for settings whose options are
    /// mutually exclusive and read better side by side than as a checked list
    /// (the merge tool's view mode). Segments are clicked, not
    /// keyboard-selected, so the row is skipped by arrow navigation.
    Segmented {
        label: SharedString,
        segments: Vec<ContextMenuSegment>,
    },
}

/// One option inside a [`ContextMenuItem::Segmented`] row.
#[derive(Clone)]
struct ContextMenuSegment {
    /// Stable element id, also used as the debug selector.
    id: SharedString,
    label: SharedString,
    tooltip: Option<SharedString>,
    selected: bool,
    action: ContextMenuAction,
}

#[derive(Clone)]
struct ContextMenuModel {
    items: Vec<ContextMenuItem>,
    /// Render shortcut labels as individual keycaps, matching the Command
    /// Palette. Most context menus use shortcuts as single-key mnemonics, so
    /// this remains opt-in.
    shortcut_keycaps: bool,
    /// Optional hover tooltip per entry index (e.g. the full commit message in the
    /// browse-history menu). Sparse — most menus leave this empty.
    entry_tooltips: std::collections::HashMap<usize, SharedString>,
    /// Stable debug selectors for menus whose entries predate the shared context-menu
    /// renderer. Sparse so ordinary menus continue deriving selectors from labels.
    entry_debug_selectors: std::collections::HashMap<usize, SharedString>,
}

impl ContextMenuModel {
    fn new(items: Vec<ContextMenuItem>) -> Self {
        Self {
            items,
            shortcut_keycaps: false,
            entry_tooltips: std::collections::HashMap::new(),
            entry_debug_selectors: std::collections::HashMap::new(),
        }
    }

    fn with_shortcut_keycaps(mut self) -> Self {
        self.shortcut_keycaps = true;
        self
    }

    fn with_entry_tooltips(
        mut self,
        entry_tooltips: std::collections::HashMap<usize, SharedString>,
    ) -> Self {
        self.entry_tooltips = entry_tooltips;
        self
    }

    fn with_entry_debug_selectors(
        mut self,
        entry_debug_selectors: std::collections::HashMap<usize, SharedString>,
    ) -> Self {
        self.entry_debug_selectors = entry_debug_selectors;
        self
    }

    fn is_selectable(&self, ix: usize) -> bool {
        matches!(
            self.items.get(ix),
            Some(ContextMenuItem::Entry { disabled, .. }) if !*disabled
        )
    }

    fn first_selectable(&self) -> Option<usize> {
        (0..self.items.len()).find(|&ix| self.is_selectable(ix))
    }

    fn last_selectable(&self) -> Option<usize> {
        (0..self.items.len())
            .rev()
            .find(|&ix| self.is_selectable(ix))
    }

    fn next_selectable(&self, from: Option<usize>, dir: isize) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let Some(mut ix) = from else {
            return if dir >= 0 {
                self.first_selectable()
            } else {
                self.last_selectable()
            };
        };

        let n = self.items.len() as isize;
        for _ in 0..self.items.len() {
            ix = ((ix as isize + dir).rem_euclid(n)) as usize;
            if self.is_selectable(ix) {
                return Some(ix);
            }
        }
        None
    }
}

// HistoryColResizeDragGhost moved to view/mod.rs for accessibility from panes::HistoryView.

mod action_bar;
mod bars;
mod bottom_status_bar;
mod layout;
mod main;
mod popover;
mod repo_tabs_bar;

pub(super) use action_bar::{ActionBarView, action_bar_height};
pub(super) use bottom_status_bar::BottomStatusBarView;
pub(super) use popover::PopoverHost;
#[cfg(feature = "benchmarks")]
pub(in crate::view) use popover::{benchmark_branch_checkout_rows, benchmark_workspace_rows};
/// Layout guards outside this module assert against the tab padding, so they
/// follow the constant instead of hardcoding the current value.
#[cfg(test)]
pub(in crate::view) use repo_tabs_bar::REPO_TAB_SIDE_PADDING_PX;
pub(super) use repo_tabs_bar::RepoTabsBarView;
#[allow(unused_imports)]
pub(in crate::view) use repo_tabs_bar::repo_tab_insert_before_for_drag_cursor;

#[cfg(test)]
mod tests;
