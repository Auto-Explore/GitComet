//! Per-repository workspace state: the virtual branches, their stacks, and
//! which branch each changed file belongs to.
//!
//! This is a cache of what `gitcomet/workspace.json` holds, plus the stacks
//! derived from it. It is loaded lazily — a repository that never opens the
//! Workspace view never reads or writes a workspace — and every field is a
//! `Loadable` so the UI can show the state it has while a reload is in flight
//! rather than blanking out.

use gitcomet_core::workspace::{
    AssignmentIndex, BranchApplyState, FileAssignment, Stack, VirtualBranch, WorkspaceState,
};
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::Loadable;

/// A workspace action running on the backend.
///
/// Workspace edits mutate the repository, so they are tracked like other Git
/// work: the UI disables conflicting actions and shows progress rather than
/// letting two of them race.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceBusy {
    pub loading: bool,
    pub applying: bool,
    pub committing: bool,
    /// Any operation that changes branches or the workspace branch, including
    /// apply, unapply, restack, and a target change.
    pub mutating: bool,
}

impl WorkspaceBusy {
    pub fn any(&self) -> bool {
        self.loading || self.applying || self.committing || self.mutating
    }
}

/// Why the last workspace rebuild did not produce a clean result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceConflict {
    /// The branch being applied when the merge failed.
    pub branch: String,
    /// The branch it conflicted with, when the failure names one.
    pub against: Option<String>,
    /// The paths git reported as conflicting, when it named any.
    pub paths: Arc<Vec<PathBuf>>,
    pub message: String,
}

impl WorkspaceConflict {
    /// The conflict described in terms of the branches involved, which is what
    /// the user can act on — a bare checkout/rebase conflict message names
    /// files but not the virtual branches that produced them.
    pub fn summary(&self) -> String {
        match &self.against {
            Some(against) => format!("{} conflicts with {}", self.branch, against),
            None => format!("{} could not be applied", self.branch),
        }
    }
}

/// The workspace of one open repository.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceRepoState {
    /// The stored workspace: target branch plus virtual branches.
    pub state: Loadable<Arc<WorkspaceState>>,
    /// Stacks derived from `state`, computed once per load so rendering a long
    /// stack list does not re-sort per frame.
    pub stacks: Arc<Vec<Stack>>,
    /// Which virtual branch each changed file belongs to.
    pub assignments: Arc<AssignmentIndex>,
    /// Paths with no branch assigned, kept beside the index so the UI can show
    /// them without scanning every status row.
    pub unassigned: Arc<Vec<PathBuf>>,
    /// The tip of `gitcomet/workspace`, when it has been built.
    pub workspace_commit: Loadable<Option<gitcomet_core::domain::CommitId>>,
    pub busy: WorkspaceBusy,
    /// The conflict from the last failed apply, cleared by the next successful
    /// one so a resolved conflict does not linger.
    pub conflict: Option<WorkspaceConflict>,
    /// Bumped on every change to the workspace, for cache keys.
    pub rev: u64,
}

impl WorkspaceRepoState {
    /// Recompute the derived fields from a freshly loaded state.
    ///
    /// A workspace that fails to validate (hand-edited, or written by an
    /// incompatible version) still renders: the stacks are empty and the raw
    /// state is kept for the error message, rather than the view refusing to
    /// draw at all.
    pub fn set_state(&mut self, state: WorkspaceState) {
        self.stacks = Arc::new(state.stacks().unwrap_or_default());
        self.state = Loadable::Ready(Arc::new(state));
        self.conflict = None;
        self.bump_rev();
    }

    pub fn clear(&mut self) {
        self.state = Loadable::NotLoaded;
        self.stacks = Arc::new(Vec::new());
        self.assignments = Arc::new(AssignmentIndex::default());
        self.unassigned = Arc::new(Vec::new());
        self.workspace_commit = Loadable::NotLoaded;
        self.busy = WorkspaceBusy::default();
        self.conflict = None;
        self.bump_rev();
    }

    pub fn bump_rev(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// The loaded workspace state, if there is one.
    pub fn workspace(&self) -> Option<&WorkspaceState> {
        match &self.state {
            Loadable::Ready(state) => Some(state),
            _ => None,
        }
    }

    pub fn target(&self) -> Option<&str> {
        self.workspace().map(|state| state.target.as_str())
    }

    pub fn branch(&self, name: &str) -> Option<&VirtualBranch> {
        self.workspace()?.get(name)
    }

    pub fn applied_count(&self) -> usize {
        self.stacks.iter().map(Stack::applied_count).sum()
    }

    pub fn total_count(&self) -> usize {
        self.stacks.iter().map(|stack| stack.branches.len()).sum()
    }

    /// Changed files grouped by the branch they are assigned to, plus the
    /// unassigned remainder, in path order within each group.
    ///
    /// Paths whose branch no longer exists are reported as unassigned rather
    /// than dropped: the file is still changed and still needs a home.
    pub fn files_by_branch(&self, changed: &[PathBuf]) -> Vec<FileAssignment> {
        let state = match self.workspace() {
            Some(state) => state,
            None => return Vec::new(),
        };
        changed
            .iter()
            .map(|path| FileAssignment {
                path: path.clone(),
                branch: self.assignments.resolve(path, state).map(str::to_string),
            })
            .collect()
    }

    /// The counts the workspace header shows: assigned per branch, unassigned,
    /// and unapplied branches that hold no files right now.
    pub fn summary_counts(&self, changed: &[PathBuf]) -> WorkspaceSummary {
        let assignments = self.files_by_branch(changed);
        let mut per_branch: FxHashMap<String, usize> = FxHashMap::default();
        let mut unassigned = 0;
        for assignment in &assignments {
            match &assignment.branch {
                Some(branch) => *per_branch.entry(branch.clone()).or_default() += 1,
                None => unassigned += 1,
            }
        }
        WorkspaceSummary {
            per_branch,
            unassigned,
            applied: self.applied_count(),
            total: self.total_count(),
        }
    }

    /// Whether committing is possible: something is applied, something is
    /// changed, and no other workspace work is running.
    pub fn can_commit(&self, changed_count: usize) -> bool {
        changed_count > 0 && self.applied_count() > 0 && !self.busy.any()
    }
}

/// The counts shown above the workspace file list.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceSummary {
    pub per_branch: FxHashMap<String, usize>,
    pub unassigned: usize,
    pub applied: usize,
    pub total: usize,
}

/// An edit to the workspace, as the UI requests it.
///
/// The reducer turns one of these into a state change plus the backend calls
/// needed to realize it, so a single message covers create/apply/restack rather
/// than one message per button.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceEdit {
    /// Add a branch on the target.
    Create {
        name: String,
    },
    /// Add a branch stacked on another, or directly on the target when
    /// `parent` is `None`.
    CreateStacked {
        name: String,
        parent: Option<String>,
    },
    SetApplied {
        name: String,
        applied: BranchApplyState,
    },
    /// Change a branch's base, rebasing it there.
    SetParent {
        name: String,
        parent: Option<String>,
    },
    /// Move a branch into a stack, above or below a sibling.
    MoveToStack {
        name: String,
        stack_base: String,
        relative_to: Option<String>,
        below: bool,
    },
    /// Swap two siblings' positions.
    Reorder {
        first: String,
        second: String,
    },
    Remove {
        name: String,
    },
    /// Point the workspace at a different base and rebase onto it.
    SetTarget {
        target: String,
    },
    /// Assign a file to a branch, or to none.
    AssignFile {
        path: PathBuf,
        branch: Option<String>,
    },
    /// Commit the given files to a branch rather than to the workspace.
    CommitPaths {
        name: String,
        message: String,
        paths: Vec<PathBuf>,
    },
}

impl WorkspaceEdit {
    /// The branch the edit acts on, for progress reporting and for deciding
    /// which repository rows to invalidate.
    pub fn branch_name(&self) -> Option<&str> {
        match self {
            Self::Create { name }
            | Self::CreateStacked { name, .. }
            | Self::SetApplied { name, .. }
            | Self::SetParent { name, .. }
            | Self::MoveToStack { name, .. }
            | Self::Remove { name }
            | Self::CommitPaths { name, .. } => Some(name),
            Self::Reorder { first, .. } => Some(first),
            Self::SetTarget { .. } => None,
            Self::AssignFile { branch, .. } => branch.as_deref(),
        }
    }

    /// Whether the set of applied branches moved, which is what forces the
    /// workspace branch to be replayed.
    ///
    /// Creating and removing count: a new branch is applied by default and a
    /// removed one stops contributing, so both change what the working tree
    /// should contain.
    pub fn changes_applied_set(&self) -> bool {
        matches!(
            self,
            Self::SetApplied { .. }
                | Self::SetTarget { .. }
                | Self::Create { .. }
                | Self::CreateStacked { .. }
                | Self::Remove { .. }
        )
    }

    /// Whether a branch's base moved, which is the only edit that rewrites
    /// branch history.
    pub fn rewrites_history(&self) -> bool {
        matches!(
            self,
            Self::SetParent { .. } | Self::MoveToStack { .. } | Self::SetTarget { .. }
        )
    }

    /// Whether applying this edit produced a commit on a branch.
    pub fn produces_commit(&self) -> bool {
        matches!(self, Self::CommitPaths { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(names: &[(&str, Option<&str>, BranchApplyState)]) -> WorkspaceState {
        WorkspaceState::new("main").with_branches(
            names
                .iter()
                .map(|(name, parent, applied)| {
                    let mut branch = VirtualBranch::new(*name);
                    if let Some(parent) = parent {
                        branch = branch.with_parent(*parent);
                    }
                    branch.applied = *applied;
                    branch
                })
                .collect(),
        )
    }

    fn loaded(names: &[(&str, Option<&str>, BranchApplyState)]) -> WorkspaceRepoState {
        let mut workspace = WorkspaceRepoState::default();
        workspace.set_state(state_with(names));
        workspace
    }

    #[test]
    fn set_state_derives_stacks() {
        let workspace = loaded(&[("api", None, BranchApplyState::Applied), ("ui", Some("api"), BranchApplyState::Applied)]);
        assert_eq!(workspace.stacks.len(), 1);
        assert_eq!(workspace.total_count(), 2);
        assert_eq!(workspace.applied_count(), 2);
    }

    #[test]
    fn unapplied_branches_are_counted_separately() {
        let workspace = loaded(&[("api", None, BranchApplyState::Applied), ("parked", None, BranchApplyState::Unapplied)]);
        assert_eq!(workspace.total_count(), 2);
        assert_eq!(workspace.applied_count(), 1);
    }

    #[test]
    fn rev_advances_on_every_change() {
        let mut workspace = WorkspaceRepoState::default();
        assert_eq!(workspace.rev, 0);
        workspace.set_state(WorkspaceState::new("main"));
        assert_eq!(workspace.rev, 1);
        workspace.bump_rev();
        assert_eq!(workspace.rev, 2);
    }

    #[test]
    fn files_are_grouped_by_their_branch() {
        let mut workspace = loaded(&[("api", None, BranchApplyState::Applied)]);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("src/a.rs"), Some("api".into()));
        index.set(PathBuf::from("src/b.rs"), None);
        workspace.assignments = Arc::new(index);

        let files = workspace.files_by_branch(&[
            PathBuf::from("src/a.rs"),
            PathBuf::from("src/b.rs"),
        ]);
        assert_eq!(files[0].branch.as_deref(), Some("api"));
        assert_eq!(files[1].branch, None);
    }

    #[test]
    fn a_file_assigned_to_a_deleted_branch_reads_as_unassigned() {
        let mut workspace = loaded(&[("api", None, BranchApplyState::Applied)]);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("src/a.rs"), Some("gone".into()));
        workspace.assignments = Arc::new(index);

        let files = workspace.files_by_branch(&[PathBuf::from("src/a.rs")]);
        assert_eq!(files[0].branch, None);
    }

    #[test]
    fn summary_counts_cover_assigned_and_unassigned_files() {
        let mut workspace = loaded(&[("api", None, BranchApplyState::Applied), ("ui", None, BranchApplyState::Applied)]);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        index.set(PathBuf::from("b.rs"), Some("api".into()));
        index.set(PathBuf::from("c.rs"), Some("ui".into()));
        index.set(PathBuf::from("d.rs"), None);
        workspace.assignments = Arc::new(index);

        let summary = workspace.summary_counts(&[
            PathBuf::from("a.rs"),
            PathBuf::from("b.rs"),
            PathBuf::from("c.rs"),
            PathBuf::from("d.rs"),
        ]);
        assert_eq!(summary.per_branch.get("api"), Some(&2));
        assert_eq!(summary.per_branch.get("ui"), Some(&1));
        assert_eq!(summary.unassigned, 1);
        assert_eq!(summary.applied, 2);
    }

    #[test]
    fn can_commit_requires_changes_applied_branches_and_idle_state() {
        let workspace = loaded(&[("api", None, BranchApplyState::Applied)]);
        assert!(workspace.can_commit(1));
        assert!(!workspace.can_commit(0));

        let mut busy = workspace.clone();
        busy.busy.mutating = true;
        assert!(!busy.can_commit(1));

        let nothing_applied = loaded(&[("api", None, BranchApplyState::Unapplied)]);
        assert!(!nothing_applied.can_commit(1));
    }

    #[test]
    fn clearing_resets_every_field() {
        let mut workspace = loaded(&[("api", None, BranchApplyState::Applied)]);
        workspace.busy.applying = true;
        workspace.clear();
        assert!(workspace.workspace().is_none());
        assert!(workspace.stacks.is_empty());
        assert!(workspace.assignments.is_empty());
        assert!(!workspace.busy.any());
    }

    #[test]
    fn a_corrupt_workspace_still_renders_empty_stacks() {
        let mut workspace = WorkspaceRepoState::default();
        // A cycle cannot produce stacks, but it must not panic the view.
        let cyclic = WorkspaceState::new("main").with_branches(vec![
            VirtualBranch::new("a").with_parent("b"),
            VirtualBranch::new("b").with_parent("a"),
        ]);
        workspace.set_state(cyclic);
        assert!(workspace.stacks.is_empty());
        assert!(workspace.workspace().is_some());
    }

    #[test]
    fn edits_report_which_branch_they_act_on() {
        assert_eq!(
            WorkspaceEdit::Create { name: "a".into() }.branch_name(),
            Some("a")
        );
        assert_eq!(
            WorkspaceEdit::AssignFile {
                path: PathBuf::from("a.rs"),
                branch: Some("api".into()),
            }
            .branch_name(),
            Some("api")
        );
        assert_eq!(WorkspaceEdit::SetTarget { target: "x".into() }.branch_name(), None);
    }

    #[test]
    fn edits_that_move_the_applied_set_are_marked() {
        assert!(WorkspaceEdit::SetApplied {
            name: "a".into(),
            applied: BranchApplyState::Applied,
        }
        .changes_applied_set());
        assert!(WorkspaceEdit::SetTarget { target: "x".into() }.changes_applied_set());
        // A new branch is applied by default, and a removed one stops
        // contributing, so both have to be replayed into the workspace branch.
        assert!(WorkspaceEdit::Create { name: "a".into() }.changes_applied_set());
        assert!(WorkspaceEdit::Remove { name: "a".into() }.changes_applied_set());
        assert!(!WorkspaceEdit::Reorder {
            first: "a".into(),
            second: "b".into(),
        }
        .changes_applied_set());
        assert!(!WorkspaceEdit::AssignFile {
            path: PathBuf::from("a.rs"),
            branch: Some("a".into()),
        }
        .changes_applied_set());
    }

    #[test]
    fn only_base_moves_rewrite_history() {
        assert!(WorkspaceEdit::SetParent {
            name: "a".into(),
            parent: None,
        }
        .rewrites_history());
        assert!(!WorkspaceEdit::Create { name: "a".into() }.rewrites_history());
        assert!(!WorkspaceEdit::AssignFile {
            path: PathBuf::from("a.rs"),
            branch: None,
        }
        .rewrites_history());
    }

    #[test]
    fn only_committing_produces_a_commit() {
        assert!(WorkspaceEdit::CommitPaths {
            name: "a".into(),
            message: "m".into(),
            paths: vec![PathBuf::from("a.rs")],
        }
        .produces_commit());
        assert!(!WorkspaceEdit::SetApplied {
            name: "a".into(),
            applied: BranchApplyState::Applied,
        }
        .produces_commit());
        assert_eq!(
            WorkspaceEdit::CommitPaths {
                name: "a".into(),
                message: "m".into(),
                paths: Vec::new(),
            }
            .branch_name(),
            Some("a")
        );
    }

    #[test]
    fn a_conflict_summary_names_both_branches() {
        let conflict = WorkspaceConflict {
            branch: "feature/ui".into(),
            against: Some("feature/api".into()),
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        };
        assert_eq!(
            conflict.summary(),
            "feature/ui conflicts with feature/api"
        );

        let alone = WorkspaceConflict {
            branch: "feature/ui".into(),
            against: None,
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        };
        assert_eq!(alone.summary(), "feature/ui could not be applied");
    }
}
