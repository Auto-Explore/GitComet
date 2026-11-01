//! Per-repository virtual branches state: the virtual branches, their stacked branches, and
//! which branch each changed file belongs to.
//!
//! This is a cache of what `gitcomet/virtual.json` holds, plus the stacked branches
//! derived from it. It is loaded lazily — a repository that never opens the
//! Virtual branches view never reads or writes a virtual branches — and every field is a
//! `Loadable` so the UI can show the state it has while a reload is in flight
//! rather than blanking out.

use gitcomet_core::virtual_branches::{
    AssignmentIndex, BranchApplyState, FileAssignment, StackedBranch, VirtualBranch,
    VirtualBranchesState,
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
    /// Stacked branches derived from `state`, computed once per load so rendering a long
    /// stacked branch list does not re-sort per frame.
    pub stacked_branches: Arc<Vec<StackedBranch>>,
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
    /// incompatible version) still renders: the stacked branches are empty and the raw
    /// state is kept for the error message, rather than the view refusing to
    /// draw at all.
    pub fn set_state(&mut self, state: VirtualBranchesState) {
        self.stacked_branches = Arc::new(state.stacked_branches().unwrap_or_default());
        self.state = Loadable::Ready(Arc::new(state));
        self.conflict = None;
        self.bump_rev();
    }

    pub fn clear(&mut self) {
        self.state = Loadable::NotLoaded;
        self.stacked_branches = Arc::new(Vec::new());
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
        self.stacked_branches
            .iter()
            .map(StackedBranch::applied_count)
            .sum()
    }

    pub fn total_count(&self) -> usize {
        self.stacked_branches
            .iter()
            .map(|stacked_branch| stacked_branch.branches.len())
            .sum()
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
    /// Slot a new branch into a stacked branch next to `anchor`.
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
    /// Move a branch into a stacked branch, above or below a sibling.
    MoveToStackedBranch {
        name: String,
        stacked_branch_base: String,
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
            | Self::MoveToStackedBranch { name, .. }
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
    /// Inserting *above* an anchor stacked branches the new branch on it, so nothing that
    /// existed changes base. Inserting *below* puts the new branch in the
    /// anchor's place, which leaves the anchor sitting on it, so the anchor's
    /// commits have to be replayed on the branch that displaced them.
    pub fn rewrites_history(&self) -> bool {
        match self {
            Self::SetParent { .. } | Self::MoveToStackedBranch { .. } | Self::SetTarget { .. } => {
                true
            }
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
mod tests;
