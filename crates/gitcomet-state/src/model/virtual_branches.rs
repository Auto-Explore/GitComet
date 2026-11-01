//! Per-repository virtual branches state: the virtual branches, their stacks, and
//! which branch each changed file belongs to.
//!
//! This is a cache of what `gitcomet/virtual.json` holds, plus the stacks
//! derived from it. It is loaded lazily — a repository that never opens the
//! Virtual branches view never reads or writes a virtual branches — and every field is a
//! `Loadable` so the UI can show the state it has while a reload is in flight
//! rather than blanking out.

use gitcomet_core::virtual_branches::{
    AssignmentIndex, BranchApplyState, FileAssignment, Stack, VirtualBranch, VirtualBranchesState,
};
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::Loadable;

/// A virtual branches action running on the backend.
///
/// Virtual branches edits mutate the repository, so they are tracked like other Git
/// work: the UI disables conflicting actions and shows progress rather than
/// letting two of them race.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VirtualBranchesBusy {
    pub loading: bool,
    pub applying: bool,
    pub committing: bool,
    /// Any operation that changes branches or the virtual-branches branch, including
    /// apply, unapply, restack, and a target change.
    pub mutating: bool,
    /// Switching the working directory into or out of the virtual branches.
    ///
    /// Separate from `mutating` because this one moves HEAD under the user: it
    /// needs its own progress state so nothing else offers to run at the same
    /// time, and so a failure can be reported as "could not switch" rather than
    /// as a broken apply.
    pub switching: bool,
}

impl VirtualBranchesBusy {
    pub fn any(&self) -> bool {
        self.loading || self.applying || self.committing || self.mutating || self.switching
    }
}

/// Why the last virtual branches rebuild did not produce a clean result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualBranchesConflict {
    /// The branch being applied when the merge failed.
    pub branch: String,
    /// The branch it conflicted with, when the failure names one.
    pub against: Option<String>,
    /// The paths git reported as conflicting, when it named any.
    pub paths: Arc<Vec<PathBuf>>,
    pub message: String,
}

impl VirtualBranchesConflict {
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

/// The virtual branches of one open repository.
#[derive(Clone, Debug, Default)]
pub struct VirtualBranchesRepoState {
    /// The stored virtual branches: target branch plus virtual branches.
    pub state: Loadable<Arc<VirtualBranchesState>>,
    /// Stacks derived from `state`, computed once per load so rendering a long
    /// stack list does not re-sort per frame.
    pub stacks: Arc<Vec<Stack>>,
    /// Which virtual branch each changed file — or each of its hunks — belongs
    /// to.
    pub assignments: Arc<AssignmentIndex>,
    /// The tip of `gitcomet/virtual`, when it has been built.
    pub virtual_branches_commit: Loadable<Option<gitcomet_core::domain::CommitId>>,
    pub busy: VirtualBranchesBusy,
    /// The conflict from the last failed apply, cleared by the next successful
    /// one so a resolved conflict does not linger.
    pub conflict: Option<VirtualBranchesConflict>,
    /// Bumped on every change to the virtual branches, for cache keys.
    pub rev: u64,
    /// Whether the working directory is currently on `gitcomet/virtual`.
    ///
    /// The virtual branches only owns the working directory while the user is looking
    /// at it. A rebuild while this is `false` updates the ref and nothing else,
    /// which is what stops a virtual branches nobody has entered from changing files.
    pub active: bool,
    /// The branch to put the working directory back on when the user leaves.
    ///
    /// Captured on entry rather than derived, because by the time they leave
    /// there is no way to tell what they were on. Kept in memory rather than on
    /// disk: after a restart the user is either already on the virtual branches (and
    /// this is `None`, meaning "no idea") or not on it at all.
    pub checkout_base: Option<String>,
}

impl VirtualBranchesRepoState {
    /// Recompute the derived fields from a freshly loaded state.
    ///
    /// A virtual branches that fails to validate (hand-edited, or written by an
    /// incompatible version) still renders: the stacks are empty and the raw
    /// state is kept for the error message, rather than the view refusing to
    /// draw at all.
    pub fn set_state(&mut self, state: VirtualBranchesState) {
        self.stacks = Arc::new(state.stacks().unwrap_or_default());
        self.state = Loadable::Ready(Arc::new(state));
        self.conflict = None;
        self.bump_rev();
    }

    pub fn clear(&mut self) {
        self.state = Loadable::NotLoaded;
        self.stacks = Arc::new(Vec::new());
        self.assignments = Arc::new(AssignmentIndex::default());
        self.virtual_branches_commit = Loadable::NotLoaded;
        self.busy = VirtualBranchesBusy::default();
        self.conflict = None;
        self.active = false;
        self.checkout_base = None;
        self.bump_rev();
    }

    pub fn bump_rev(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// The loaded virtual branches state, if there is one.
    pub fn virtual_branches(&self) -> Option<&VirtualBranchesState> {
        match &self.state {
            Loadable::Ready(state) => Some(state),
            _ => None,
        }
    }

    pub fn target(&self) -> Option<&str> {
        self.virtual_branches().map(|state| state.target.as_str())
    }

    pub fn branch(&self, name: &str) -> Option<&VirtualBranch> {
        self.virtual_branches()?.get(name)
    }

    pub fn applied_count(&self) -> usize {
        self.stacks.iter().map(Stack::applied_count).sum()
    }

    pub fn total_count(&self) -> usize {
        self.stacks.iter().map(|stack| stack.branches.len()).sum()
    }

    /// Changed files with the branch — or the branches — they are assigned to.
    ///
    /// Paths whose branch no longer exists are reported as unassigned rather
    /// than dropped: the file is still changed and still needs a home.
    pub fn files_by_branch(&self, changed: &[PathBuf]) -> Vec<FileAssignment> {
        let state = match self.virtual_branches() {
            Some(state) => state,
            None => return Vec::new(),
        };
        changed
            .iter()
            .map(|path| {
                let file = self.assignments.file(path);
                let mut split: Vec<String> = file
                    .map(|file| file.hunk_branches())
                    .into_iter()
                    .flatten()
                    .filter(|branch| state.get(branch).is_some())
                    .map(str::to_string)
                    .collect();
                // The map is keyed by hunk, so the same branch appears once per
                // hunk it owns; the view shows a file, not its hunks.
                split.sort_unstable();
                split.dedup();
                FileAssignment {
                    path: path.clone(),
                    branch: self.assignments.resolve(path, state).map(str::to_string),
                    split,
                }
            })
            .collect()
    }

    /// The counts the virtual branches header shows: assigned per branch, unassigned,
    /// and unapplied branches that hold no files right now.
    ///
    /// A split file counts once for every branch that commits part of it, so
    /// the per-branch numbers answer "what will this branch commit" rather than
    /// "how many files sum to the list above" — and the two do not have to
    /// agree, which is the point of a file being split.
    pub fn summary_counts(&self, changed: &[PathBuf]) -> VirtualBranchesSummary {
        let assignments = self.files_by_branch(changed);
        let mut per_branch: FxHashMap<String, usize> = FxHashMap::default();
        let mut unassigned = 0;
        for assignment in &assignments {
            if let Some(branch) = &assignment.branch {
                *per_branch.entry(branch.clone()).or_default() += 1;
            } else {
                for branch in &assignment.split {
                    *per_branch.entry(branch.clone()).or_default() += 1;
                }
            }
            if !assignment.is_assigned() {
                unassigned += 1;
            }
        }
        VirtualBranchesSummary {
            per_branch,
            unassigned,
            applied: self.applied_count(),
            total: self.total_count(),
        }
    }

    /// Whether committing is possible: something is applied, something is
    /// changed, and no other virtual branches work is running.
    pub fn can_commit(&self, changed_count: usize) -> bool {
        changed_count > 0 && self.applied_count() > 0 && !self.busy.any()
    }
}

/// The counts shown above the virtual branches file list.
#[derive(Clone, Debug, Default)]
pub struct VirtualBranchesSummary {
    pub per_branch: FxHashMap<String, usize>,
    pub unassigned: usize,
    pub applied: usize,
    pub total: usize,
}

/// An edit to the virtual branches, as the UI requests it.
///
/// The reducer turns one of these into a state change plus the backend calls
/// needed to realize it, so a single message covers create/apply/restack rather
/// than one message per button.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VirtualBranchesEdit {
    /// Add a branch on the target.
    Create {
        name: String,
    },
    /// Slot a new branch into a stack next to `anchor`.
    ///
    /// `below` is the whole difference: below means stacking on the anchor,
    /// above means taking the anchor's place and pushing it — and whatever was
    /// above it — down one.
    InsertRelativeTo {
        name: String,
        anchor: String,
        below: bool,
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
    /// Point the virtual branches at a different base and rebase onto it.
    SetTarget {
        target: String,
    },
    /// Assign a file to a branch, or to none.
    AssignFile {
        path: PathBuf,
        branch: Option<String>,
    },
    /// Assign one hunk of a file to a branch, or back to no branch.
    ///
    /// Splitting a file that was assigned whole keeps the old branch on every
    /// other hunk, so this moves the hunk rather than orphaning the rest.
    AssignHunk {
        path: PathBuf,
        hunk: gitcomet_core::virtual_branches::HunkFingerprint,
        branch: Option<String>,
    },
    /// Commit the given files to a branch rather than to the virtual branches.
    CommitPaths {
        name: String,
        message: String,
        paths: Vec<PathBuf>,
    },
}

impl VirtualBranchesEdit {
    /// The branch the edit acts on, for progress reporting and for deciding
    /// which repository rows to invalidate.
    pub fn branch_name(&self) -> Option<&str> {
        match self {
            Self::Create { name }
            | Self::InsertRelativeTo { name, .. }
            | Self::SetApplied { name, .. }
            | Self::SetParent { name, .. }
            | Self::MoveToStack { name, .. }
            | Self::Remove { name }
            | Self::CommitPaths { name, .. } => Some(name),
            Self::Reorder { first, .. } => Some(first),
            Self::SetTarget { .. } => None,
            Self::AssignFile { branch, .. } | Self::AssignHunk { branch, .. } => branch.as_deref(),
        }
    }

    /// Whether the set of applied branches moved, which is what forces the
    /// virtual-branches branch to be replayed.
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
                | Self::InsertRelativeTo { .. }
                | Self::Remove { .. }
        )
    }

    /// Whether a branch's base moved, which is the only edit that rewrites
    /// branch history.
    ///
    /// Inserting *above* an anchor stacks the new branch on it, so nothing that
    /// existed changes base. Inserting *below* puts the new branch in the
    /// anchor's place, which leaves the anchor sitting on it, so the anchor's
    /// commits have to be replayed on the branch that displaced them.
    pub fn rewrites_history(&self) -> bool {
        match self {
            Self::SetParent { .. } | Self::MoveToStack { .. } | Self::SetTarget { .. } => true,
            Self::InsertRelativeTo { below, .. } => *below,
            _ => false,
        }
    }

    /// Whether applying this edit produced a commit on a branch.
    pub fn produces_commit(&self) -> bool {
        matches!(self, Self::CommitPaths { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(names: &[(&str, Option<&str>, BranchApplyState)]) -> VirtualBranchesState {
        VirtualBranchesState::new("main").with_branches(
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

    fn loaded(names: &[(&str, Option<&str>, BranchApplyState)]) -> VirtualBranchesRepoState {
        let mut virtual_branches = VirtualBranchesRepoState::default();
        virtual_branches.set_state(state_with(names));
        virtual_branches
    }

    #[test]
    fn set_state_derives_stacks() {
        let virtual_branches = loaded(&[
            ("api", None, BranchApplyState::Applied),
            ("ui", Some("api"), BranchApplyState::Applied),
        ]);
        assert_eq!(virtual_branches.stacks.len(), 1);
        assert_eq!(virtual_branches.total_count(), 2);
        assert_eq!(virtual_branches.applied_count(), 2);
    }

    #[test]
    fn unapplied_branches_are_counted_separately() {
        let virtual_branches = loaded(&[
            ("api", None, BranchApplyState::Applied),
            ("parked", None, BranchApplyState::Unapplied),
        ]);
        assert_eq!(virtual_branches.total_count(), 2);
        assert_eq!(virtual_branches.applied_count(), 1);
    }

    #[test]
    fn rev_advances_on_every_change() {
        let mut virtual_branches = VirtualBranchesRepoState::default();
        assert_eq!(virtual_branches.rev, 0);
        virtual_branches.set_state(VirtualBranchesState::new("main"));
        assert_eq!(virtual_branches.rev, 1);
        virtual_branches.bump_rev();
        assert_eq!(virtual_branches.rev, 2);
    }

    #[test]
    fn files_are_grouped_by_their_branch() {
        let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("src/a.rs"), Some("api".into()));
        index.set(PathBuf::from("src/b.rs"), None);
        virtual_branches.assignments = Arc::new(index);

        let files =
            virtual_branches.files_by_branch(&[PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")]);
        assert_eq!(files[0].branch.as_deref(), Some("api"));
        assert_eq!(files[1].branch, None);
    }

    #[test]
    fn a_file_assigned_to_a_deleted_branch_reads_as_unassigned() {
        let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("src/a.rs"), Some("gone".into()));
        virtual_branches.assignments = Arc::new(index);

        let files = virtual_branches.files_by_branch(&[PathBuf::from("src/a.rs")]);
        assert_eq!(files[0].branch, None);
    }

    #[test]
    fn summary_counts_cover_assigned_and_unassigned_files() {
        let mut virtual_branches = loaded(&[
            ("api", None, BranchApplyState::Applied),
            ("ui", None, BranchApplyState::Applied),
        ]);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        index.set(PathBuf::from("b.rs"), Some("api".into()));
        index.set(PathBuf::from("c.rs"), Some("ui".into()));
        index.set(PathBuf::from("d.rs"), None);
        virtual_branches.assignments = Arc::new(index);

        let summary = virtual_branches.summary_counts(&[
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
    fn a_split_file_reads_as_belonging_to_every_branch_that_has_part_of_it() {
        // A split file has no single branch, so `branch` is `None` — but
        // reporting that as unassigned would send the user looking for a file
        // that two branches are already committing.
        let mut virtual_branches = loaded(&[
            ("api", None, BranchApplyState::Applied),
            ("ui", None, BranchApplyState::Applied),
        ]);
        let mut index = AssignmentIndex::new();
        let first = gitcomet_core::virtual_branches::HunkFingerprint::of(&["api\n"], 1);
        let second = gitcomet_core::virtual_branches::HunkFingerprint::of(&["ui\n"], 1);
        index.set_hunk(PathBuf::from("a.rs"), first, Some("api".into()));
        index.set_hunk(PathBuf::from("a.rs"), second, Some("ui".into()));
        virtual_branches.assignments = Arc::new(index);

        let files = virtual_branches.files_by_branch(&[PathBuf::from("a.rs")]);
        assert_eq!(files[0].branch, None, "it is on no single branch");
        assert_eq!(files[0].split, ["api", "ui"], "it is on both");
        assert!(files[0].is_assigned());

        let summary = virtual_branches.summary_counts(&[PathBuf::from("a.rs")]);
        assert_eq!(summary.per_branch.get("api"), Some(&1));
        assert_eq!(summary.per_branch.get("ui"), Some(&1));
        assert_eq!(summary.unassigned, 0, "nothing here will fail to commit");
    }

    #[test]
    fn a_hunk_on_a_deleted_branch_does_not_keep_a_file_assigned() {
        let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
        let mut index = AssignmentIndex::new();
        index.set_hunk(
            PathBuf::from("a.rs"),
            gitcomet_core::virtual_branches::HunkFingerprint::of(&["new\n"], 1),
            Some("gone".into()),
        );
        virtual_branches.assignments = Arc::new(index);

        let files = virtual_branches.files_by_branch(&[PathBuf::from("a.rs")]);
        assert!(files[0].split.is_empty());
        assert!(!files[0].is_assigned());
    }

    #[test]
    fn switching_the_working_directory_counts_as_busy() {
        // Entering the virtual branches moves HEAD under the user. Nothing else may
        // offer to run while that is in flight.
        let mut virtual_branches = VirtualBranchesRepoState::default();
        virtual_branches.busy.switching = true;
        assert!(virtual_branches.busy.any());
        assert!(!virtual_branches.can_commit(1));
    }

    #[test]
    fn clearing_forgets_that_the_working_directory_was_in_the_virtual_branches() {
        let mut virtual_branches = VirtualBranchesRepoState::default();
        virtual_branches.active = true;
        virtual_branches.checkout_base = Some("main".into());
        virtual_branches.clear();
        assert!(!virtual_branches.active);
        assert!(
            virtual_branches.checkout_base.is_none(),
            "a remembered branch must not outlive the session that remembered it"
        );
    }

    #[test]
    fn can_commit_requires_changes_applied_branches_and_idle_state() {
        let virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
        assert!(virtual_branches.can_commit(1));
        assert!(!virtual_branches.can_commit(0));

        let mut busy = virtual_branches.clone();
        busy.busy.mutating = true;
        assert!(!busy.can_commit(1));

        let nothing_applied = loaded(&[("api", None, BranchApplyState::Unapplied)]);
        assert!(!nothing_applied.can_commit(1));
    }

    #[test]
    fn clearing_resets_every_field() {
        let mut virtual_branches = loaded(&[("api", None, BranchApplyState::Applied)]);
        virtual_branches.busy.applying = true;
        virtual_branches.clear();
        assert!(virtual_branches.virtual_branches().is_none());
        assert!(virtual_branches.stacks.is_empty());
        assert!(virtual_branches.assignments.is_empty());
        assert!(!virtual_branches.busy.any());
    }

    #[test]
    fn a_corrupt_virtual_branches_still_renders_empty_stacks() {
        let mut virtual_branches = VirtualBranchesRepoState::default();
        // A cycle cannot produce stacks, but it must not panic the view.
        let cyclic = VirtualBranchesState::new("main").with_branches(vec![
            VirtualBranch::new("a").with_parent("b"),
            VirtualBranch::new("b").with_parent("a"),
        ]);
        virtual_branches.set_state(cyclic);
        assert!(virtual_branches.stacks.is_empty());
        assert!(virtual_branches.virtual_branches().is_some());
    }

    #[test]
    fn edits_report_which_branch_they_act_on() {
        assert_eq!(
            VirtualBranchesEdit::Create { name: "a".into() }.branch_name(),
            Some("a")
        );
        assert_eq!(
            VirtualBranchesEdit::AssignFile {
                path: PathBuf::from("a.rs"),
                branch: Some("api".into()),
            }
            .branch_name(),
            Some("api")
        );
        assert_eq!(
            VirtualBranchesEdit::SetTarget { target: "x".into() }.branch_name(),
            None
        );
    }

    #[test]
    fn edits_that_move_the_applied_set_are_marked() {
        assert!(
            VirtualBranchesEdit::SetApplied {
                name: "a".into(),
                applied: BranchApplyState::Applied,
            }
            .changes_applied_set()
        );
        assert!(VirtualBranchesEdit::SetTarget { target: "x".into() }.changes_applied_set());
        // A new branch is applied by default, and a removed one stops
        // contributing, so both have to be replayed into the virtual-branches branch.
        assert!(VirtualBranchesEdit::Create { name: "a".into() }.changes_applied_set());
        assert!(VirtualBranchesEdit::Remove { name: "a".into() }.changes_applied_set());
        assert!(
            !VirtualBranchesEdit::Reorder {
                first: "a".into(),
                second: "b".into(),
            }
            .changes_applied_set()
        );
        assert!(
            !VirtualBranchesEdit::AssignFile {
                path: PathBuf::from("a.rs"),
                branch: Some("a".into()),
            }
            .changes_applied_set()
        );
    }

    #[test]
    fn only_base_moves_rewrite_history() {
        assert!(
            VirtualBranchesEdit::SetParent {
                name: "a".into(),
                parent: None,
            }
            .rewrites_history()
        );
        assert!(!VirtualBranchesEdit::Create { name: "a".into() }.rewrites_history());
        assert!(
            !VirtualBranchesEdit::AssignFile {
                path: PathBuf::from("a.rs"),
                branch: None,
            }
            .rewrites_history()
        );
    }

    #[test]
    fn only_inserting_below_replays_the_anchor() {
        let insert = |below| VirtualBranchesEdit::InsertRelativeTo {
            name: "b".into(),
            anchor: "a".into(),
            below,
        };
        // Above stacks on the anchor: nothing that existed changes base.
        assert!(!insert(false).rewrites_history());
        // Below takes the anchor's place, so the anchor is replayed on it.
        assert!(insert(true).rewrites_history());
    }

    #[test]
    fn inserting_a_branch_moves_the_applied_set() {
        // A new branch is applied by default, like any other creation.
        assert!(
            VirtualBranchesEdit::InsertRelativeTo {
                name: "b".into(),
                anchor: "a".into(),
                below: true,
            }
            .changes_applied_set()
        );
        assert_eq!(
            VirtualBranchesEdit::InsertRelativeTo {
                name: "b".into(),
                anchor: "a".into(),
                below: true,
            }
            .branch_name(),
            Some("b"),
            "the created branch is the one to report progress on"
        );
    }

    #[test]
    fn only_committing_produces_a_commit() {
        assert!(
            VirtualBranchesEdit::CommitPaths {
                name: "a".into(),
                message: "m".into(),
                paths: vec![PathBuf::from("a.rs")],
            }
            .produces_commit()
        );
        assert!(
            !VirtualBranchesEdit::SetApplied {
                name: "a".into(),
                applied: BranchApplyState::Applied,
            }
            .produces_commit()
        );
        assert_eq!(
            VirtualBranchesEdit::CommitPaths {
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
        let conflict = VirtualBranchesConflict {
            branch: "feature/ui".into(),
            against: Some("feature/api".into()),
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        };
        assert_eq!(conflict.summary(), "feature/ui conflicts with feature/api");

        let alone = VirtualBranchesConflict {
            branch: "feature/ui".into(),
            against: None,
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        };
        assert_eq!(alone.summary(), "feature/ui could not be applied");
    }
}
