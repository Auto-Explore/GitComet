use super::*;

const COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX: f32 = 240.0;
const COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX: f32 = 200.0;

#[derive(Clone)]
enum ContextMenuAction {
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
    OpenRepo {
        path: std::path::PathBuf,
    },
    ExportPatch {
        repo_id: RepoId,
        commit_id: CommitId,
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
    CheckoutBranch {
        repo_id: RepoId,
        name: String,
    },
    DeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    SetHistoryScope {
        repo_id: RepoId,
        scope: gitcomet_core::domain::LogScope,
    },
    SetHistoryColumns {
        show_author: bool,
        show_date: bool,
        show_sha: bool,
    },
    ResetHistoryColumnWidths,
    SetThemeMode {
        mode: ThemeMode,
    },
    SetDateTimeFormat {
        format: DateTimeFormat,
    },
    SetTimezone {
        timezone: Timezone,
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
    ConflictResolverPick {
        target: ResolverPickTarget,
    },
    ConflictResolverOutputCut {
        text: String,
    },
    ConflictResolverOutputPaste,
    CopyText {
        text: String,
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
    Header(SharedString),
    Label(SharedString),
    Entry {
        label: SharedString,
        icon: Option<SharedString>,
        shortcut: Option<SharedString>,
        disabled: bool,
        action: Box<ContextMenuAction>,
    },
}

#[derive(Clone)]
struct ContextMenuModel {
    items: Vec<ContextMenuItem>,
}

impl ContextMenuModel {
    fn new(items: Vec<ContextMenuItem>) -> Self {
        Self { items }
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
mod layout;
mod main;
mod popover;
mod repo_tabs_bar;

pub(super) use action_bar::ActionBarView;
pub(super) use popover::PopoverHost;
pub(super) use repo_tabs_bar::RepoTabsBarView;

#[cfg(test)]
mod tests;
