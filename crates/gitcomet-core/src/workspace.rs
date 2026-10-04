//! The workspace model: virtual branches, stacks, and the target branch.
//!
//! A *virtual branch* is an independent unit of work that lives as an ordinary
//! Git branch under `refs/heads/`. Nothing about it requires server support:
//! it can be pushed, reviewed, and merged like any other branch. What GitComet
//! adds on top is a dependency — a branch may name another branch as its
//! *parent*, which makes it a *stacked* branch whose pull request targets that
//! parent rather than the target branch.
//!
//! The interesting design decision is that "stacked" and "independent" are not
//! two types. A branch is independent when [`VirtualBranch::parent`] is `None`,
//! and stacked when it is `Some`. Moving a branch between stacks, reordering a
//! stack, and promoting a branch to independent are all the same edit: change
//! one field. That keeps the data model small enough to serialize to a file and
//! reason about in tests.
//!
//! The combined state of every *applied* branch is materialized as a single
//! branch, [`WORKSPACE_BRANCH`]. It is an implementation detail of the model,
//! not a branch a user works on: commits belong to virtual branches, and the
//! workspace branch is regenerated from them whenever the applied set changes.
//!
//! Everything here is pure data plus validation. Nothing in this module talks
//! to Git; see [`crate::services`] for the operations that do.

use crate::error::{Error, ErrorKind, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The branch holding the combined tree of every applied virtual branch.
///
/// It is managed internally: GitComet rewrites it whenever the applied set
/// changes, and direct user commits to it are refused. It is named under a
/// `gitcomet/` prefix so it is visually distinct from a user's own branches and
/// so it can be filtered out of ordinary branch listings.
pub const WORKSPACE_BRANCH: &str = "gitcomet/workspace";

/// The ref namespace the workspace owns. Branches under it are GitComet's own
/// bookkeeping and are never presented as user branches.
pub const WORKSPACE_REF_PREFIX: &str = "gitcomet/";

/// Whether a virtual branch contributes its tree to the workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchApplyState {
    /// Contributes to the workspace branch and can take assigned changes.
    Applied,
    /// Exists as a branch but contributes nothing to the workspace.
    Unapplied,
}

impl BranchApplyState {
    pub const fn is_applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

/// One unit of work in the workspace.
///
/// `parent` is the whole of the stacking model: `None` means the branch is
/// stacked directly on the workspace target, `Some(name)` means it is stacked
/// on another virtual branch, and that branch's pull request targets `name`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VirtualBranch {
    /// The Git branch name. This is the branch's identity everywhere else:
    /// pushing `refs/heads/<name>` is what publishes the work.
    pub name: String,
    /// The branch this one is stacked on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default = "default_apply_state")]
    pub applied: BranchApplyState,
    /// A short note the user keeps alongside the branch. Never sent to Git.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Position among siblings stacked on the same parent. Only meaningful
    /// between branches that share a parent; independent branches are ordered
    /// by name in the UI and this is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
}

fn default_apply_state() -> BranchApplyState {
    BranchApplyState::Applied
}

impl VirtualBranch {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parent: None,
            applied: BranchApplyState::Applied,
            description: None,
            order: None,
        }
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    pub fn with_applied(mut self, applied: BranchApplyState) -> Self {
        self.applied = applied;
        self
    }

    pub fn with_order(mut self, order: u32) -> Self {
        self.order = Some(order);
        self
    }

    pub fn is_applied(&self) -> bool {
        self.applied.is_applied()
    }

    /// Whether this branch sits directly on the target rather than on another
    /// virtual branch.
    pub fn is_independent(&self) -> bool {
        self.parent.is_none()
    }

    /// The branch whose pull request this one targets: its parent if it is
    /// stacked, otherwise the workspace target.
    pub fn base_branch<'a>(&'a self, target: &'a str) -> &'a str {
        self.parent.as_deref().unwrap_or(target)
    }
}

/// One column of the workspace: a parent branch and everything stacked on it,
/// in application order.
///
/// Stacks are a view, not stored state. They are derived from `parent` links by
/// [`WorkspaceState::stacks`], so a stack can never disagree with the branches
/// that make it up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stack {
    /// The target branch, or the name of the bottom branch of the stack.
    pub base: String,
    /// Members from the bottom of the stack upward, so `stacked_on` is
    /// unambiguous when rendering indentation.
    pub branches: Vec<StackMember>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackMember {
    pub branch: VirtualBranch,
    /// Depth within the stack: 0 for the branch that sits on the base.
    pub depth: usize,
}

impl Stack {
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.branches.iter().any(|m| m.branch.name == name)
    }

    pub fn applied_count(&self) -> usize {
        self.branches.iter().filter(|m| m.branch.is_applied()).count()
    }
}

/// The target branch plus every virtual branch, as one unit.
///
/// This is the structure the workspace UI reads and the structure that gets
/// written back after an edit. It carries no Git state: nothing here knows
/// whether a branch exists yet or what its tip is.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// The base every independent branch is built on, e.g. `main` or
    /// `origin/main`. A revision, not necessarily a local branch.
    #[serde(default = "default_target_branch")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<VirtualBranch>,
}

fn default_target_branch() -> String {
    "main".to_string()
}

impl WorkspaceState {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            branches: Vec::new(),
        }
    }

    pub fn with_branches(mut self, branches: Vec<VirtualBranch>) -> Self {
        self.branches = branches;
        self
    }

    pub fn get(&self, name: &str) -> Option<&VirtualBranch> {
        self.branches.iter().find(|b| b.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut VirtualBranch> {
        self.branches.iter_mut().find(|b| b.name == name)
    }

    pub fn applied(&self) -> impl Iterator<Item = &VirtualBranch> {
        self.branches.iter().filter(|b| b.is_applied())
    }

    pub fn applied_names(&self) -> Vec<&str> {
        self.applied().map(|b| b.name.as_str()).collect()
    }

    /// Names that must be merged into the workspace branch, in an order where
    /// a stacked branch always follows its parent.
    ///
    /// The order matters: applying `feature/ui` before `feature/api` would merge
    /// a commit whose parent is not yet present and fail to resolve. Independent
    /// branches keep their declared order so a workspace rebuild is
    /// deterministic and a diff of two rebuilds is empty.
    pub fn application_order(&self) -> Result<Vec<&VirtualBranch>> {
        let ordered = self.topological_order()?;
        Ok(ordered
            .into_iter()
            .filter(|name| self.get(name).is_some_and(VirtualBranch::is_applied))
            .filter_map(|name| self.get(name))
            .collect())
    }

    /// Every branch name ordered so a child never precedes its parent.
    ///
    /// Roots come first in declaration order; each root is followed by the
    /// branches stacked on it, depth-first. A cycle is not reachable from a
    /// root, so it is left out and reported by [`Self::validate`].
    fn topological_order(&self) -> Result<Vec<String>> {
        let mut ordered = Vec::with_capacity(self.branches.len());
        let mut roots: Vec<&VirtualBranch> =
            self.branches.iter().filter(|b| b.is_independent()).collect();
        roots.sort_by_key(|b| b.order.unwrap_or(u32::MAX));

        let mut remaining: Vec<&VirtualBranch> =
            self.branches.iter().filter(|b| !b.is_independent()).collect();
        remaining.sort_by_key(|b| b.order.unwrap_or(u32::MAX));

        for root in roots {
            ordered.push(root.name.clone());
            let mut frontier = vec![root.name.as_str()];
            while let Some(parent) = frontier.pop() {
                let mut children: Vec<&VirtualBranch> = remaining
                    .iter()
                    .copied()
                    .filter(|b| b.parent.as_deref() == Some(parent))
                    .collect();
                children.sort_by_key(|b| b.order.unwrap_or(u32::MAX));
                for child in children {
                    remaining.retain(|b| b.name != child.name);
                    ordered.push(child.name.clone());
                    frontier.push(child.name.as_str());
                }
            }
        }

        if let Some(orphan) = remaining.first() {
            return Err(workspace_error(format!(
                "branch '{}' is stacked on '{}', which is not a workspace branch",
                orphan.name,
                orphan.parent.as_deref().unwrap_or_default(),
            )));
        }
        Ok(ordered)
    }

    /// The branches grouped into stacks, each with its depth.
    ///
    /// Unreachable branches (cycles) do not appear; [`Self::validate`] is what
    /// surfaces them, so a corrupt state degrades to a missing row rather than
    /// an infinite loop in the renderer.
    pub fn stacks(&self) -> Result<Vec<Stack>> {
        let order = self.topological_order()?;
        let mut stacks: Vec<Stack> = Vec::new();

        for name in order {
            let Some(branch) = self.get(&name) else {
                continue;
            };
            match &branch.parent {
                None => stacks.push(Stack {
                    base: self.target.clone(),
                    branches: vec![StackMember {
                        branch: branch.clone(),
                        depth: 0,
                    }],
                }),
                Some(parent) => {
                    let depth = match stacks.iter_mut().rev().find(|s| s.contains(parent)) {
                        Some(stack) => {
                            let depth = stack
                                .branches
                                .iter()
                                .find(|m| m.branch.name == *parent)
                                .map_or(0, |m| m.depth + 1);
                            stack.branches.push(StackMember {
                                branch: branch.clone(),
                                depth,
                            });
                            depth
                        }
                        None => {
                            stacks.push(Stack {
                                base: parent.clone(),
                                branches: vec![StackMember {
                                    branch: branch.clone(),
                                    depth: 0,
                                }],
                            });
                            0
                        }
                    };
                    let _ = depth;
                }
            }
        }

        Ok(stacks)
    }

    /// Everything that would make this state unusable to render or apply.
    ///
    /// Called before an edit is written, so a rejected edit leaves the
    /// persisted state exactly as it was rather than half-applied.
    pub fn validate(&self) -> Result<()> {
        for branch in &self.branches {
            validate_branch_name(&branch.name)?;
        }

        for (index, branch) in self.branches.iter().enumerate() {
            if self.branches[..index]
                .iter()
                .any(|other| other.name == branch.name)
            {
                return Err(workspace_error(format!(
                    "duplicate workspace branch '{}'",
                    branch.name
                )));
            }
        }

        for branch in &self.branches {
            let Some(parent) = &branch.parent else {
                continue;
            };
            if *parent == branch.name {
                return Err(workspace_error(format!(
                    "branch '{}' is stacked on itself",
                    branch.name
                )));
            }
            if !self.branches.iter().any(|b| &b.name == parent) {
                return Err(workspace_error(format!(
                    "branch '{}' is stacked on '{}', which is not a workspace branch",
                    branch.name, parent
                )));
            }
            if self.reaches(branch, parent) {
                return Err(workspace_error(format!(
                    "branch '{}' is stacked on '{}', which already depends on it",
                    branch.name, parent
                )));
            }
        }

        if self.branches.iter().any(|b| b.name == self.target) {
            return Err(workspace_error(format!(
                "'{}' is the workspace target and cannot also be a workspace branch",
                self.target
            )));
        }
        for branch in &self.branches {
            if branch.name.starts_with(WORKSPACE_REF_PREFIX) {
                return Err(workspace_error(format!(
                    "'{}' is inside the reserved '{}' namespace",
                    branch.name, WORKSPACE_REF_PREFIX
                )));
            }
        }

        self.topological_order().map(|_| ())
    }

    /// Whether `branch`'s parent chain reaches `target`, i.e. stacking `target`
    /// on `branch` would close a loop.
    fn reaches(&self, branch: &VirtualBranch, target: &str) -> bool {
        let mut cursor = branch.parent.as_deref();
        // Bounded by branch count: a cycle is rejected by `validate`, and a
        // walk that long means one is already present.
        for _ in 0..=self.branches.len() {
            let Some(name) = cursor else {
                return false;
            };
            if name == target {
                return true;
            }
            cursor = self.get(name).and_then(|b| b.parent.as_deref());
        }
        false
    }

    /// The parent `branch` would have after `name` is moved above or below it,
    /// or `None` if it becomes independent.
    ///
    /// Used by the reorder and move actions so the caller can render the
    /// resulting shape without applying the edit first.
    pub fn parent_after_move(
        &self,
        name: &str,
        target_name: Option<&str>,
        below: bool,
    ) -> Option<String> {
        let moved = self.get(name)?;
        match target_name {
            None => None,
            Some(target) if target == name => moved.parent.clone(),
            Some(target) => {
                let target_parent = self
                    .get(target)
                    .and_then(|t| t.parent.clone())
                    .unwrap_or_else(|| self.target.clone());
                if below {
                    Some(target.to_string())
                } else {
                    Some(target_parent)
                }
            }
        }
    }

    /// Insert a branch above `name`, taking over `name`'s parent.
    pub fn insert_above(&mut self, branch: VirtualBranch, name: &str) -> Result<()> {
        let displaced = self
            .get(name)
            .ok_or_else(|| workspace_error(format!("no workspace branch '{name}'")))?
            .parent
            .clone();
        self.push_branch(branch)?;
        self.set_parent(name, displaced.as_deref())?;
        self.set_parent(&branch.name, Some(name))?;
        Ok(())
    }

    /// Insert a branch below `name`, stacked on it.
    pub fn insert_below(&mut self, branch: VirtualBranch, name: &str) -> Result<()> {
        if self.get(name).is_none() {
            return Err(workspace_error(format!("no workspace branch '{name}'")));
        }
        self.push_branch(branch)?;
        self.set_parent(&branch.name, Some(name))?;
        Ok(())
    }

    /// Move `name` into `stack_base`'s stack, optionally above or below
    /// `relative_to`.
    pub fn move_to_stack(
        &mut self,
        name: &str,
        stack_base: &str,
        relative_to: Option<&str>,
        below: bool,
    ) -> Result<()> {
        if self.get(name).is_none() {
            return Err(workspace_error(format!("no workspace branch '{name}'")));
        }
        let parent = match relative_to {
            None => Some(stack_base.to_string()),
            Some(relative) if relative == name => self.get(name).and_then(|b| b.parent.clone()),
            Some(relative) => {
                if below {
                    Some(relative.to_string())
                } else {
                    Some(
                        self.get(relative)
                            .and_then(|b| b.parent.clone())
                            .unwrap_or_else(|| stack_base.to_string()),
                    )
                }
            }
        };
        self.set_parent(name, parent.as_deref())
    }

    /// Swap two branches' positions, keeping each one's place in its stack.
    pub fn reorder(&mut self, first: &str, second: &str) -> Result<()> {
        if first == second {
            return Ok(());
        }
        let left = self
            .get(first)
            .ok_or_else(|| workspace_error(format!("no workspace branch '{first}'")))?
            .clone();
        let right = self
            .get(second)
            .ok_or_else(|| workspace_error(format!("no workspace branch '{second}'")))?
            .clone();

        self.swap_positions(first, second, &left, &right)
    }

    fn swap_positions(
        &mut self,
        first: &str,
        second: &str,
        left: &VirtualBranch,
        right: &VirtualBranch,
    ) -> Result<()> {
        // Reordering only makes sense between siblings: two branches with
        // different parents are in different stacks, and "swap" has no meaning
        // without first choosing where the second one goes.
        if left.parent != right.parent {
            return Err(workspace_error(format!(
                "'{first}' and '{second}' are in different stacks; move one into the other's stack instead"
            )));
        }

        match (left.order, right.order) {
            (Some(a), Some(b)) => {
                if let Some(branch) = self.get_mut(first) {
                    branch.order = Some(b);
                }
                if let Some(branch) = self.get_mut(second) {
                    branch.order = Some(a);
                }
            }
            (Some(_), None) | (None, Some(_)) => {
                // One side carries an explicit position and the other does not;
                // both need a position now or the pair has no stable order.
                let base = left.order.or(right.order).unwrap_or_default();
                if let Some(branch) = self.get_mut(first) {
                    branch.order = Some(base);
                }
                if let Some(branch) = self.get_mut(second) {
                    branch.order = Some(base + 1);
                }
            }
            (None, None) => {
                for (index, name) in [first, second].into_iter().enumerate() {
                    if let Some(branch) = self.get_mut(name) {
                        branch.order = Some(index as u32);
                    }
                }
            }
        }
        Ok(())
    }

    /// Add a branch, rejecting a name that is already taken.
    pub fn push_branch(&mut self, branch: VirtualBranch) -> Result<()> {
        validate_branch_name(&branch.name)?;
        if branch.name.starts_with(WORKSPACE_REF_PREFIX) {
            return Err(workspace_error(format!(
                "'{}' is inside the reserved '{}' namespace",
                branch.name, WORKSPACE_REF_PREFIX
            )));
        }
        if self.get(&branch.name).is_some() {
            return Err(workspace_error(format!(
                "'{}' is already a workspace branch",
                branch.name
            )));
        }
        self.branches.push(branch);
        Ok(())
    }

    /// Remove a branch, re-parenting whatever was stacked on it so the stack
    /// does not develop a hole.
    pub fn remove_branch(&mut self, name: &str) -> Result<VirtualBranch> {
        let index = self
            .branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| workspace_error(format!("no workspace branch '{name}'")))?;
        let removed = self.branches.remove(index);
        let parent = removed.parent.clone();
        for branch in &mut self.branches {
            if branch.parent.as_deref() == Some(name) {
                branch.parent = parent.clone();
            }
        }
        Ok(removed)
    }

    /// Point `name` at `parent`, where `None` makes it independent.
    ///
    /// The resulting state is validated before it is committed, so a move that
    /// would create a cycle is refused with the state left untouched.
    pub fn set_parent(&mut self, name: &str, parent: Option<&str>) -> Result<()> {
        if self.get(name).is_none() {
            return Err(workspace_error(format!("no workspace branch '{name}'")));
        }
        let previous = self.branches.clone();
        if let Some(branch) = self.get_mut(name) {
            branch.parent = parent.map(str::to_string);
        }
        if let Err(error) = self.validate() {
            self.branches = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Set whether a branch contributes to the workspace branch.
    pub fn set_applied(&mut self, name: &str, applied: BranchApplyState) -> Result<()> {
        let Some(branch) = self.get_mut(name) else {
            return Err(workspace_error(format!("no workspace branch '{name}'")));
        };
        branch.applied = applied;
        Ok(())
    }

    /// Point the workspace at a different target branch.
    ///
    /// Independent branches keep their names; only their base moves. Callers
    /// rebase them onto the new target with
    /// [`crate::services::WorkspaceRepository::update_workspace_target`].
    pub fn set_target(&mut self, target: impl Into<String>) -> Result<()> {
        let target = target.into();
        if target.is_empty() {
            return Err(workspace_error("the workspace target cannot be empty"));
        }
        if self.branches.iter().any(|b| b.name == target) {
            return Err(workspace_error(format!(
                "'{target}' is already a workspace branch and cannot also be the target"
            )));
        }
        self.target = target;
        Ok(())
    }
}

fn workspace_error(message: String) -> Error {
    Error::new(ErrorKind::Backend(message))
}

/// Reject names Git would refuse anyway, before they reach a command that
/// fails with a less specific message.
fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(workspace_error("a workspace branch needs a name".into()));
    }
    if name.starts_with('-') {
        return Err(workspace_error(format!(
            "'{name}' cannot start with '-'"
        )));
    }
    if !name.is_ascii() {
        return Err(workspace_error(format!(
            "'{name}' is not a valid branch name"
        )));
    }
    if name.contains("..")
        || name.contains("//")
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains('~')
        || name.contains('^')
        || name.contains(':')
        || name.contains('?')
        || name.contains('*')
        || name.contains('[')
        || name.contains('\\')
        || name.contains(' ')
        || name.contains('\0')
    {
        return Err(workspace_error(format!(
            "'{name}' is not a valid branch name"
        )));
    }
    for part in name.split('/') {
        if part.is_empty() || part.starts_with('.') || part.ends_with(".lock") {
            return Err(workspace_error(format!(
                "'{name}' is not a valid branch name"
            )));
        }
    }
    Ok(())
}

/// A file in the working tree and the virtual branches its changes belong to.
///
/// Assignment is recorded per repository rather than derived from the diff, so
/// a file keeps its branch across edits made while it is unmodified.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileAssignment {
    pub path: PathBuf,
    /// The branch the whole file is on, or `None` for a split or unassigned
    /// file. `None` alone does not mean unassigned — see [`Self::split`].
    pub branch: Option<String>,
    /// The branches a *split* file's hunks are on, deduplicated and sorted.
    ///
    /// A split file is assigned to nobody in particular and to everybody in
    /// here: every branch in this list commits part of it. Reporting it as
    /// unassigned because `branch` is `None` would be a lie the user acts on,
    /// so the list is what the view reads to say "this file belongs to both".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split: Vec<String>,
}

impl FileAssignment {
    pub fn assigned(path: impl Into<PathBuf>, branch: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            branch: Some(branch.into()),
            split: Vec::new(),
        }
    }

    pub fn unassigned(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            branch: None,
            split: Vec::new(),
        }
    }

    /// Whether anything at all is assigned, whole file or hunk.
    pub fn is_assigned(&self) -> bool {
        self.branch.is_some() || !self.split.is_empty()
    }
}

/// The assignment map, plus the paths that have nothing assigned at all.
///
/// Kept sorted by path so the list the UI renders is stable without a sort on
/// every frame, and so a persisted file stays diffable.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentIndex {
    entries: BTreeMap<PathBuf, FileAssignments>,
}

/// One hunk of a file's diff, addressed by where it sits in the **base**.
/// Stable identity of one change in a file, independent of what it is diffed
/// against.
///
/// A line range cannot be this: the view diffs `HEAD` against the working tree
/// while a commit diffs the *branch's* base against the same working tree, so
/// the same change has different ranges on the two sides. What both sides agree
/// on is the change itself, so that is what is stored — how many lines it adds,
/// how many it removes, and a hash of the text it produces.
///
/// The trade is that two hunks adding and removing exactly the same text are
/// the same fingerprint, and both follow the assignment. That is deliberate:
/// they are indistinguishable to a user reading the file, and picking one would
/// be a coin flip dressed up as precision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct HunkFingerprint {
    /// FNV-1a over the hunk's new-side lines.
    ///
    /// FNV rather than the standard library's hasher because this is written to
    /// disk: a hasher that changes between Rust releases would silently
    /// invalidate every assignment a user ever made.
    content: u64,
    /// Lines the hunk produces on the new side.
    new_lines: u32,
    /// Base lines the hunk removes.
    base_lines: u32,
}

impl HunkFingerprint {
    /// Identify a change from what it adds and what it removes.
    pub fn of(new_side: &[&str], base_lines: usize) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in new_side.iter().flat_map(|line| line.as_bytes()) {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self {
            content: hash,
            new_lines: new_side.len() as u32,
            base_lines: base_lines as u32,
        }
    }
}

/// A hunk together with the ranges it covers on each side.
///
/// The fingerprint is what an assignment is keyed by; the ranges are what
/// applies one right now. They come out of the same diff pass, which is the
/// point: taking them from two passes would let a file that changed between
/// them produce a range belonging to a different change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HunkSpan {
    pub fingerprint: HunkFingerprint,
    /// Base lines the hunk replaces, 0-based half-open.
    pub base_range: std::ops::Range<usize>,
    /// New lines the hunk produces, 0-based half-open.
    pub new_range: std::ops::Range<usize>,
}

/// Unchanged lines git tolerates between two change groups before starting a
/// new `@@` hunk instead of extending the current one.
///
/// A hunk assignment is keyed by the *grouping* of changes, not by the changes
/// themselves: the user picks one of the hunks `git diff` showed them, so the
/// hunks the model computes have to be the ones `git diff` would show. xdiff
/// merges two change groups into a single hunk while the gap between them is at
/// most twice the context, and `git diff` is invoked without `-U`
/// (`GixRepo::build_unified_diff_command`), so the context is its default of 3.
pub const HUNK_CONTEXT_LINES: usize = 3;

/// Every hunk between two versions of a file, in base order.
///
/// "Hunk" here means the same thing it means in `git diff`: not every maximal
/// run of changed lines, but the runs git would print under one `@@` header.
/// See [`HUNK_CONTEXT_LINES`].
pub fn hunk_spans(old: &str, new: &str) -> Vec<HunkSpan> {
    use crate::file_diff::{side_by_side_plan, FileDiffPlanRun};

    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let plan = side_by_side_plan(old, new);
    let mut spans: Vec<HunkSpan> = Vec::new();
    // The hunk the previous change run opened, if any. Adjacent change runs —
    // no context between them at all — are unambiguously one hunk; runs with a
    // little context between them are folded together later, once the gap is
    // known.
    let mut open: Option<(usize, usize, usize, usize)> = None;
    // The base line the next change starts at, 1-based. Context runs move it;
    // insertions do not, which is what anchors a pure insertion to the line it
    // sits in front of.
    let mut base_pos = 1usize;

    for run in &plan.runs {
        let (start, base_lines, new_start, new_lines) = match run {
            FileDiffPlanRun::Context { old_start, len, .. } => {
                close_span(open.take(), &mut spans, &new_lines);
                base_pos = old_start + len;
                continue;
            }
            FileDiffPlanRun::Remove { old_start, len } => {
                base_pos = old_start + len;
                (*old_start, *len, 0, 0)
            }
            FileDiffPlanRun::Add { new_start, len } => (base_pos, 0, *new_start, *len),
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                base_pos = old_start + len;
                (*old_start, *len, *new_start, *len)
            }
        };
        open = Some(match open {
            // Consecutive runs continue both sides: only the first one says
            // where the hunk's new lines begin, and the base range starts there.
            Some((base_start, base_count, new_at, new_count)) => (
                base_start,
                base_count + base_lines,
                if new_at == 0 { new_start } else { new_at },
                new_count + new_lines,
            ),
            None => (start, base_lines, new_start, new_lines),
        });
    }
    close_span(open.take(), &mut spans, &new_lines);
    merge_neighbouring_changes(spans, &new_lines)
}

/// Fold change runs git would have printed as one `@@` into a single span.
///
/// Without this the spans are *finer* than the hunks the view showed: two edits
/// three lines apart are one hunk to the user and two spans here, so their
/// fingerprint matches neither of them and the assignment would silently apply
/// to nothing. Merging by the same rule the diff uses keeps the two views of a
/// hunk the same hunk.
///
/// The fingerprint is rebuilt over the group's *changed* lines only, never over
/// the context the merge folded in: context is by definition what the two
/// versions agree on, so hashing it would make a change's identity depend on its
/// neighbours — and the view, which can only read the `+` lines out of a git
/// hunk, could never reproduce that hash.
fn merge_neighbouring_changes(spans: Vec<HunkSpan>, new_lines: &[&str]) -> Vec<HunkSpan> {
    let max_gap = 2 * HUNK_CONTEXT_LINES;
    let mut groups: Vec<Vec<HunkSpan>> = Vec::new();
    for span in spans {
        let gap = groups.last().and_then(|group| group.last()).map_or(
            max_gap + 1,
            |prev| span.base_range.start.saturating_sub(prev.base_range.end),
        );
        if gap <= max_gap {
            if let Some(group) = groups.last_mut() {
                group.push(span);
                continue;
            }
        }
        groups.push(vec![span]);
    }

    groups
        .into_iter()
        .map(|group| {
            let mut merged = group[0].clone();
            for span in &group[1..] {
                merged.base_range.end = merged.base_range.end.max(span.base_range.end);
                merged.new_range.end = merged.new_range.end.max(span.new_range.end);
            }
            let produced: Vec<&str> = group
                .iter()
                .flat_map(|span| {
                    new_lines
                        .get(span.new_range.clone())
                        .map(<[&str]>::to_vec)
                        .unwrap_or_default()
                })
                .collect();
            let base_lines: usize = group.iter().map(|span| span.base_range.len()).sum();
            merged.fingerprint = HunkFingerprint::of(&produced, base_lines);
            merged
        })
        .collect()
}

fn close_span(
    open: Option<(usize, usize, usize, usize)>,
    spans: &mut Vec<HunkSpan>,
    file_lines: &[&str],
) {
    let Some((base_start, base_lines, new_start, new_lines)) = open else {
        return;
    };
    // Pure insertions start one line past what they follow, which is exactly
    // what makes the range the empty one at the line they sit in front of.
    let base_range = if base_lines == 0 {
        base_start..base_start
    } else {
        (base_start - 1)..(base_start - 1 + base_lines)
    };
    let new_range = if new_lines == 0 {
        new_start..new_start
    } else {
        (new_start - 1)..(new_start - 1 + new_lines)
    };
    let produced: Vec<&str> = file_lines
        .get(new_range.clone())
        .map(<[&str]>::to_vec)
        .unwrap_or_default();
    spans.push(HunkSpan {
        // Provisional: `merge_neighbouring_changes` rehashes once the runs git
        // would have printed as one hunk have been folded together.
        fingerprint: HunkFingerprint::of(&produced, base_lines),
        base_range,
        new_range,
    });
}

/// Build the version of a file that belongs to one branch.
///
/// Walks the base lines and takes, for each hunk, either the new lines — when
/// that hunk is on `keep` — or the base lines it replaced. Hunk assignments are
/// therefore the only thing that decides what a branch's commit contains, and a
/// file the user never split comes out byte-identical to the working tree.
///
/// Line endings are the base's: the working tree's would silently rewrite every
/// untouched line of a CRLF file on a commit that only moves one hunk.
pub fn synthesize_for_branch(
    base: &str,
    working: &str,
    spans: &[HunkSpan],
    keep: &dyn Fn(HunkFingerprint) -> bool,
) -> String {
    let base_lines: Vec<&str> = base.split_inclusive('\n').collect();
    let working_lines: Vec<&str> = working.split_inclusive('\n').collect();
    let mut out = String::with_capacity(base.len());
    let mut base_ix = 0usize;

    for span in spans {
        // Everything between the previous hunk and this one is untouched.
        while base_ix < span.base_range.start {
            out.push_str(base_lines.get(base_ix).copied().unwrap_or(""));
            base_ix += 1;
        }
        if keep(span.fingerprint) {
            for line in &working_lines[span.new_range.clone()] {
                out.push_str(line);
            }
        } else {
            for line in &base_lines[span.base_range.clone()] {
                out.push_str(line);
            }
        }
        base_ix = span.base_range.end.max(span.base_range.start);
    }
    while base_ix < base_lines.len() {
        out.push_str(base_lines[base_ix]);
        base_ix += 1;
    }
    out
}

/// What belongs to which branch, for one file.
///
/// A file is either whole or split. `whole` and `hunks` are never both set:
/// assigning a hunk to a whole-assigned file clears the whole-file assignment,
/// because the two would otherwise disagree about the same lines.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileAssignments {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    whole: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    hunks: BTreeMap<HunkFingerprint, String>,
}

impl FileAssignments {
    /// The whole file belongs to `branch`.
    pub fn whole(branch: Option<String>) -> Self {
        Self {
            whole: branch,
            hunks: BTreeMap::new(),
        }
    }

    pub fn is_whole(&self) -> bool {
        self.whole.is_some() || (self.whole.is_none() && self.hunks.is_empty())
    }

    /// The branch the whole file belongs to, `None` when it is split or unassigned.
    pub fn branch(&self) -> Option<&str> {
        self.whole.as_deref()
    }

    pub fn hunks(&self) -> impl Iterator<Item = (HunkFingerprint, &str)> {
        self.hunks.iter().map(|(key, branch)| (*key, branch.as_str()))
    }

    pub fn hunk_branches(&self) -> impl Iterator<Item = &str> {
        self.hunks.values().map(String::as_str)
    }

    /// The branch one hunk is on, if any.
    pub fn hunk_branch(&self, fingerprint: HunkFingerprint) -> Option<&str> {
        self.hunks.get(&fingerprint).map(String::as_str)
    }

    /// Assign one hunk, or clear it when `branch` is `None`.
    pub fn set_hunk(&mut self, fingerprint: HunkFingerprint, branch: Option<String>) {
        match branch {
            Some(branch) => {
                self.hunks.insert(fingerprint, branch);
            }
            None => {
                self.hunks.remove(&fingerprint);
            }
        }
    }

    /// Drop hunk assignments whose change the current diff no longer contains.
///
/// Identity is by content, so there is nothing to carry over: a hunk that
/// still exists has the same fingerprint and stays; one that was reverted or
/// rewritten no longer matches and goes.
    pub fn retain_hunks(&mut self, current: &[HunkSpan], state: &WorkspaceState) {
        self.hunks.retain(|fingerprint, branch| {
            state.get(branch).is_some() && current.iter().any(|span| span.fingerprint == *fingerprint)
        });
    }

    /// Whether the file has no assignment at all, whole or per hunk.
    pub fn is_empty(&self) -> bool {
        self.whole.is_none() && self.hunks.is_empty()
    }
}

impl AssignmentIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_pairs<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (PathBuf, Option<String>)>,
    {
        Self {
            entries: pairs
                .into_iter()
                .map(|(path, branch)| (path, FileAssignments::whole(branch)))
                .collect(),
        }
    }

    /// Assign the whole file, dropping any per-hunk split.
    pub fn set(&mut self, path: impl Into<PathBuf>, branch: Option<String>) {
        self.entries.insert(path.into(), FileAssignments::whole(branch));
    }

    pub fn remove(&mut self, path: &Path) {
        self.entries.remove(path);
    }

    /// Assign one hunk of a file, creating the split if the file had none.
    ///
    /// Assigning a hunk to a file that was assigned whole **splits** it: the
    /// whole-file branch does not carry over to the other hunks. Doing that
    /// silently would be worse than not carrying it over — the user would see
    /// hunks they thought were on a branch quietly land on none — so the prompt
    /// that offers a hunk assignment says so, and assigning the whole file is
    /// still one click away.
    ///
    /// There is no attempt to seed the rest with the old branch because the
    /// hunk list depends on which base the diff is taken against, and the
    /// assignment happens without knowing the branch that will commit it.
    pub fn set_hunk(
        &mut self,
        path: impl Into<PathBuf>,
        key: HunkFingerprint,
        branch: Option<String>,
    ) {
        let path = path.into();
        let entry = self.entries.entry(path).or_default();
        if entry.is_whole() {
            *entry = FileAssignments::default();
        }
        entry.set_hunk(key, branch);
    }

    pub fn branch_of(&self, path: &Path) -> Option<&str> {
        self.entries.get(path).and_then(FileAssignments::branch)
    }

    /// The per-hunk assignments of a file, empty when it is not split.
    pub fn file(&self, path: &Path) -> Option<&FileAssignments> {
        self.entries.get(path)
    }

    /// The branch `path` belongs to, if that branch still exists.
    ///
    /// A branch deleted elsewhere must not leave the map pointing at nothing,
    /// so the answer is filtered through the workspace state. A split file has
    /// no single branch and so has no answer here; callers that care read
    /// [`AssignmentIndex::file`].
    pub fn resolve(&self, path: &Path, state: &WorkspaceState) -> Option<&str> {
        let branch = self.branch_of(path)?;
        state.get(branch).map(|b| b.name.as_str())
    }

    pub fn paths(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
        self.entries
            .iter()
            .map(|(path, assignments)| (path.as_path(), assignments.branch()))
    }

    /// Every path with something assigned to `branch`, in path order.
    ///
    /// A split file counts when any of its hunks is on the branch, because
    /// committing that branch has to produce a version of the file either way.
    pub fn paths_for(&self, branch: &str) -> Vec<&Path> {
        self.entries
            .iter()
            .filter(|(_, assignments)| {
                assignments.branch() == Some(branch)
                    || assignments.hunk_branches().any(|assigned| assigned == branch)
            })
            .map(|(path, _)| path.as_path())
            .collect()
    }

    /// Drop assignments naming branches that are no longer in `state`, and any
    /// entry for a path Git no longer reports as changed.
    ///
    /// `hunks_of` supplies the hunks a path currently has, so hunk assignments
    /// can be dropped when the change they named is gone. Without it hunk
    /// assignments are kept, which is the right answer for a caller that has
    /// no diff to hand.
    pub fn retain(
        &mut self,
        state: &WorkspaceState,
        changed: &dyn Fn(&Path) -> bool,
        hunks_of: Option<&dyn Fn(&Path) -> Vec<HunkSpan>>,
    ) {
        self.entries.retain(|path, assignments| {
            if !changed(path) {
                return false;
            }
            if let Some(hunks_of) = hunks_of
                && !assignments.hunks.is_empty()
            {
                assignments.retain_hunks(&hunks_of(path), state);
            }
            let Some(branch) = assignments.whole.as_ref() else {
                // A split file survives while it still has a hunk on a live
                // branch.
                return assignments.hunk_branches().any(|b| state.get(b).is_some());
            };
            state.get(branch).is_some()
        });
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vb(name: &str) -> VirtualBranch {
        VirtualBranch::new(name)
    }

    fn workspace() -> WorkspaceState {
        WorkspaceState::new("main")
    }

    #[test]
    fn independent_branches_are_ordered_by_declaration() {
        let state = workspace()
            .with_branches(vec![vb("a"), vb("b"), vb("c")]);
        assert_eq!(state.application_order().unwrap().len(), 3);
        let names: Vec<_> = state.application_order().unwrap().iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn a_stacked_branch_is_applied_after_its_parent() {
        let state = workspace().with_branches(vec![
            vb("feature/ui").with_parent("feature/api"),
            vb("feature/api"),
        ]);
        let names: Vec<_> = state
            .application_order()
            .unwrap()
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(names, ["feature/api", "feature/ui"]);
    }

    #[test]
    fn unapplied_branches_are_excluded_from_the_application_order() {
        let state = workspace().with_branches(vec![
            vb("a"),
            vb("b").with_applied(BranchApplyState::Unapplied),
        ]);
        let names: Vec<_> = state
            .application_order()
            .unwrap()
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(names, ["a"]);
    }

    #[test]
    fn a_stacked_branch_applies_after_its_unapplied_parent() {
        // The parent contributes nothing, but the child still has to be merged
        // last or its commits have no ancestor in the workspace branch.
        let state = workspace().with_branches(vec![
            vb("parent").with_applied(BranchApplyState::Unapplied),
            vb("child").with_parent("parent"),
        ]);
        let names: Vec<_> = state
            .application_order()
            .unwrap()
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(names, ["child"]);
    }

    #[test]
    fn stacks_group_independent_branches_separately() {
        let state = workspace().with_branches(vec![
            vb("api"),
            vb("ui").with_parent("api"),
            vb("e2e").with_parent("ui"),
            vb("fix"),
        ]);
        let stacks = state.stacks().unwrap();
        assert_eq!(stacks.len(), 2);

        let first = &stacks[0];
        assert_eq!(first.base, "main");
        assert_eq!(
            first
                .branches
                .iter()
                .map(|m| (m.branch.name.as_str(), m.depth))
                .collect::<Vec<_>>(),
            [("api", 0), ("ui", 1), ("e2e", 2)]
        );
        assert_eq!(first.applied_count(), 3);

        let second = &stacks[1];
        assert_eq!(second.base, "main");
        assert_eq!(second.branches[0].branch.name, "fix");
    }

    #[test]
    fn order_controls_sibling_ordering() {
        let state = workspace().with_branches(vec![
            vb("second").with_order(1),
            vb("first").with_order(0),
        ]);
        let stacks = state.stacks().unwrap();
        assert_eq!(stacks[0].branches[0].branch.name, "first");
        assert_eq!(stacks[0].branches[1].branch.name, "second");
    }

    #[test]
    fn a_cycle_is_rejected_by_validate() {
        let state = workspace().with_branches(vec![
            vb("a").with_parent("b"),
            vb("b").with_parent("a"),
        ]);
        assert!(state.validate().is_err());
    }

    #[test]
    fn a_branch_stacked_on_itself_is_rejected() {
        let state = workspace().with_branches(vec![vb("a").with_parent("a")]);
        assert!(state.validate().is_err());
    }

    #[test]
    fn a_branch_stacked_on_a_missing_branch_is_rejected() {
        let state = workspace().with_branches(vec![vb("a").with_parent("ghost")]);
        assert!(state.validate().is_err());
    }

    #[test]
    fn duplicate_branch_names_are_rejected() {
        let state = workspace().with_branches(vec![vb("a"), vb("a")]);
        assert!(state.validate().is_err());
    }

    #[test]
    fn the_workspace_reserved_namespace_is_rejected() {
        let state = workspace().with_branches(vec![vb("gitcomet/workspace")]);
        assert!(state.validate().is_err());
        assert!(state.push_branch(vb("gitcomet/other")).is_err());
    }

    #[test]
    fn a_branch_named_like_the_target_is_rejected() {
        let state = workspace().with_branches(vec![vb("main")]);
        assert!(state.validate().is_err());
    }

    #[test]
    fn invalid_branch_names_are_rejected() {
        for name in [
            "",
            "-leading",
            "has space",
            "trailing/",
            "double//slash",
            "dots..here",
            "tilde~1",
            "caret^",
            "colon:name",
            "question?",
            "star*",
            "bracket[",
            "back\\slash",
            ".hidden",
        ] {
            let state = workspace().with_branches(vec![vb(name)]);
            assert!(
                state.validate().is_err(),
                "expected '{name}' to be rejected"
            );
        }
    }

    #[test]
    fn valid_branch_names_are_accepted() {
        for name in [
            "main",
            "feature/api",
            "feature/api/v2",
            "fix-1.2",
            "release-2024",
        ] {
            let state = workspace().with_branches(vec![vb(name)]);
            assert!(state.validate().is_ok(), "expected '{name}' to be valid");
        }
    }

    #[test]
    fn a_refused_parent_change_leaves_the_state_untouched() {
        let mut state = workspace().with_branches(vec![vb("a"), vb("b")]);
        state.set_parent("a", Some("b")).unwrap();
        let before = state.clone();
        assert!(state.set_parent("b", Some("a")).is_err());
        assert_eq!(state, before);
    }

    #[test]
    fn moving_a_branch_above_another_takes_over_its_parent() {
        let mut state = workspace().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
        state.insert_above(vb("ui2"), "ui").unwrap();
        assert_eq!(state.get("ui").unwrap().parent.as_deref(), Some("api"));
        assert_eq!(state.get("ui2").unwrap().parent.as_deref(), Some("ui"));
    }

    #[test]
    fn moving_a_branch_below_another_stacks_it_on_that_branch() {
        let mut state = workspace().with_branches(vec![vb("api")]);
        state.insert_below(vb("ui"), "api").unwrap();
        assert_eq!(state.get("ui").unwrap().parent.as_deref(), Some("api"));
    }

    #[test]
    fn removing_a_branch_reparents_its_children() {
        let mut state = workspace().with_branches(vec![
            vb("api"),
            vb("ui").with_parent("api"),
            vb("e2e").with_parent("ui"),
        ]);
        state.remove_branch("ui").unwrap();
        assert_eq!(state.get("e2e").unwrap().parent.as_deref(), Some("api"));
        assert!(state.get("ui").is_none());
    }

    #[test]
    fn removing_an_unknown_branch_is_an_error() {
        let mut state = workspace();
        assert!(state.remove_branch("ghost").is_err());
    }

    #[test]
    fn reorder_swaps_sibling_positions() {
        let mut state = workspace().with_branches(vec![
            vb("a").with_order(0),
            vb("b").with_order(1),
            vb("c").with_order(2),
        ]);
        state.reorder("a", "c").unwrap();
        assert_eq!(state.get("a").unwrap().order, Some(2));
        assert_eq!(state.get("c").unwrap().order, Some(0));
    }

    #[test]
    fn reorder_across_stacks_is_refused() {
        let mut state = workspace().with_branches(vec![
            vb("api"),
            vb("ui").with_parent("api"),
        ]);
        assert!(state.reorder("api", "ui").is_err());
    }

    #[test]
    fn reorder_gives_both_branches_a_position_when_neither_has_one() {
        let mut state = workspace().with_branches(vec![vb("a"), vb("b")]);
        state.reorder("a", "b").unwrap();
        let orders: Vec<_> = [state.get("a").unwrap().order, state.get("b").unwrap().order]
            .into_iter()
            .collect();
        assert!(orders.iter().all(Option::is_some));
        assert_ne!(orders[0], orders[1]);
    }

    #[test]
    fn move_to_stack_places_a_branch_under_the_new_base() {
        let mut state = workspace().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
        state.move_to_stack("ui", "main", None, false).unwrap();
        assert_eq!(state.get("ui").unwrap().parent, None);
    }

    #[test]
    fn move_to_stack_below_a_sibling_stacks_on_that_sibling() {
        let mut state =
            workspace().with_branches(vec![vb("api"), vb("e2e").with_parent("api")]);
        state.move_to_stack("e2e", "main", Some("api"), true).unwrap();
        assert_eq!(state.get("e2e").unwrap().parent.as_deref(), Some("api"));
    }

    #[test]
    fn move_to_stack_above_a_sibling_takes_its_place() {
        let mut state = workspace().with_branches(vec![
            vb("api"),
            vb("e2e").with_parent("api"),
            vb("other"),
        ]);
        state.move_to_stack("other", "main", Some("api"), false).unwrap();
        assert_eq!(state.get("other").unwrap().parent, None);
        assert_eq!(state.get("e2e").unwrap().parent.as_deref(), Some("api"));
    }

    #[test]
    fn a_move_that_would_create_a_cycle_is_refused() {
        let mut state = workspace().with_branches(vec![
            vb("api"),
            vb("ui").with_parent("api"),
            vb("e2e").with_parent("ui"),
        ]);
        assert!(state.move_to_stack("api", "main", Some("e2e"), true).is_err());
        assert_eq!(state.get("api").unwrap().parent, None);
    }

    #[test]
    fn base_branch_follows_the_parent_or_the_target() {
        let stacked = vb("ui").with_parent("api");
        assert_eq!(stacked.base_branch("main"), "api");
        assert_eq!(vb("api").base_branch("main"), "main");
    }

    #[test]
    fn set_applied_toggles_a_branch_without_disturbing_the_stack() {
        let mut state = workspace().with_branches(vec![vb("api"), vb("ui").with_parent("api")]);
        state.set_applied("ui", BranchApplyState::Unapplied).unwrap();
        assert!(!state.get("ui").unwrap().is_applied());
        assert_eq!(state.get("ui").unwrap().parent.as_deref(), Some("api"));
        state.set_applied("ui", BranchApplyState::Applied).unwrap();
        assert!(state.get("ui").unwrap().is_applied());
    }

    #[test]
    fn set_applied_rejects_an_unknown_branch() {
        let mut state = workspace();
        assert!(state.set_applied("ghost", BranchApplyState::Applied).is_err());
    }

    #[test]
    fn set_target_refuses_an_empty_or_conflicting_target() {
        let mut state = workspace().with_branches(vec![vb("api")]);
        assert!(state.set_target("").is_err());
        assert!(state.set_target("api").is_err());
        state.set_target("develop").unwrap();
        assert_eq!(state.target, "develop");
    }

    #[test]
    fn workspace_state_round_trips_through_serde() {
        let state = workspace().with_branches(vec![
            vb("api"),
            vb("ui").with_parent("api").with_order(1),
            vb("parked").with_applied(BranchApplyState::Unapplied),
        ]);
        let json = serde_json::to_string(&state).unwrap();
        let parsed: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn workspace_state_deserializes_a_minimal_document() {
        // Older or hand-edited files may omit the optional fields entirely.
        let parsed: WorkspaceState = serde_json::from_str(r#"{"target":"main"}"#).unwrap();
        assert_eq!(parsed.target, "main");
        assert!(parsed.branches.is_empty());
        assert!(state_defaults_to_applied(&parsed));
    }

    fn state_defaults_to_applied(state: &WorkspaceState) -> bool {
        let mut probe = state.clone();
        probe.push_branch(vb("probe")).unwrap();
        probe.get("probe").unwrap().is_applied()
    }

    #[test]
    fn assignment_index_resolves_only_existing_branches() {
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("src/a.rs"), Some("api".into()));
        index.set(PathBuf::from("src/b.rs"), None);
        index.set(PathBuf::from("src/c.rs"), Some("deleted".into()));

        let state = workspace().with_branches(vec![vb("api")]);
        assert_eq!(index.branch_of(Path::new("src/a.rs")), Some("api"));
        assert_eq!(index.branch_of(Path::new("src/b.rs")), None);
        assert_eq!(index.resolve(Path::new("src/a.rs"), &state), Some("api"));
        assert_eq!(index.resolve(Path::new("src/c.rs"), &state), None);
    }

    #[test]
    fn assignment_paths_for_returns_paths_in_path_order() {
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("z.rs"), Some("api".into()));
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        index.set(PathBuf::from("m.rs"), Some("ui".into()));
        let paths: Vec<_> = index
            .paths_for("api")
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(paths, ["a.rs", "z.rs"]);
    }

    #[test]
    fn retain_drops_assignments_for_deleted_branches_and_unchanged_paths() {
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("kept.rs"), Some("api".into()));
        index.set(PathBuf::from("gone.rs"), Some("deleted".into()));
        index.set(PathBuf::from("clean.rs"), Some("api".into()));

        let state = workspace().with_branches(vec![vb("api")]);
        let mut changed = |path: &Path| path == Path::new("kept.rs");
        index.retain(&state, &mut changed, None);

        assert_eq!(index.len(), 1);
        assert_eq!(index.branch_of(Path::new("kept.rs")), Some("api"));
    }

    #[test]
    fn a_separate_edit_produces_its_own_hunk() {
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\n";
        let working =
            "one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\neleven\n";
        let spans = hunk_spans(base, working);
        assert_eq!(spans.len(), 2, "two edits further apart than git's context, two hunks");
        assert_eq!(spans[0].base_range, 1..2, "line two");
        assert_eq!(spans[1].base_range, 9..10, "line ten");
    }

    #[test]
    fn an_insert_anchors_to_the_line_it_precedes() {
        let spans = hunk_spans("one\ntwo\n", "one\nnew\ntwo\n");
        assert_eq!(spans.len(), 1);
        assert!(
            spans[0].base_range.is_empty(),
            "an insertion replaces no base line"
        );
        assert_eq!(
            spans[0].base_range,
            1..1,
            "empty range at the line the new text sits in front of"
        );
        assert_eq!(spans[0].new_range, 1..2);
    }

    #[test]
    fn editing_a_file_above_a_hunk_leaves_it_alone() {
        // This is the property the whole model rests on: a change made at the
        // top of a file must not move the assignment of a change further down.
        let base = "one\ntwo\nthree\n";
        let working = "one\ntwo\nTHREE\n";
        let before = hunk_spans(base, working)[0].fingerprint;

        let extended = "ZERO\none\ntwo\nTHREE\n";
        let after = hunk_spans(base, extended)[0].fingerprint;
        assert_eq!(
            before, after,
            "what a change is does not depend on what sits above it"
        );
    }

    #[test]
    fn an_unchanged_hunk_keeps_its_assignment_across_a_reload() {
        let state = workspace().with_branches(vec![vb("api")]);
        let base = "one\ntwo\nthree\n";
        let working = "ONE\ntwo\nthree\n";
        let fingerprint = hunk_spans(base, working)[0].fingerprint;

        let mut index = AssignmentIndex::new();
        index.set_hunk(PathBuf::from("a.rs"), fingerprint, Some("api".into()));
        index.retain(&state, &|_| true, Some(&|_| hunk_spans(base, working)));

        assert_eq!(
            index.file(Path::new("a.rs")).unwrap().hunks().next(),
            Some((fingerprint, "api")),
            "the assignment follows the hunk rather than being dropped"
        );
    }

    #[test]
    fn a_change_that_no_longer_exists_loses_its_assignment() {
        let state = workspace().with_branches(vec![vb("api")]);
        let fingerprint = hunk_spans("one\ntwo\n", "ONE\ntwo\n")[0].fingerprint;
        let mut index = AssignmentIndex::new();
        index.set_hunk(PathBuf::from("a.rs"), fingerprint, Some("api".into()));
        // The change was reverted, so the file has no hunk to attach it to.
        index.retain(&state, &|_| true, Some(&|_| Vec::new()));
        assert_eq!(
            index.file(Path::new("a.rs")).unwrap().hunks().count(),
            0,
            "an assignment for a change that is gone has nothing to act on"
        );
    }

    #[test]
    fn a_fingerprint_survives_a_different_base() {
        // The property that makes hunk assignment work at all: the view diffs
        // against HEAD and the commit diffs against the branch's base, so the
        // same change has to carry the same identity through both.
        let working = "one\nTWO\nthree\n";
        let from_head = hunk_spans("one\ntwo\nthree\n", working)[0].fingerprint;
        let from_branch = hunk_spans("one\ntwo\nthree\nfour\n", working)[0].fingerprint;
        assert_eq!(
            from_head, from_branch,
            "what the change adds and removes does not depend on what is around it"
        );
    }

    #[test]
    fn two_different_changes_do_not_share_a_fingerprint() {
        // Far enough apart that git prints them as two hunks.
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        let working = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
        let spans = hunk_spans(base, working);
        assert_eq!(spans.len(), 2);
        assert_ne!(spans[0].fingerprint, spans[1].fingerprint);
    }

    #[test]
    fn changes_within_gits_context_are_one_hunk() {
        // Two edits three lines apart are a single `@@` to the user, so they have
        // to be a single span here too: an assignment is keyed by the hunk the
        // view showed, and a fingerprint matching neither span would assign the
        // change to nothing at all.
        let base = "one\ntwo\nthree\nfour\n";
        let working = "ONE\ntwo\nTHREE\nfour\n";
        let spans = hunk_spans(base, working);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].base_range, 0..3);
        assert_eq!(spans[0].new_range, 0..3);
    }

    #[test]
    fn a_merged_hunk_is_fingerprinted_by_the_lines_it_changes_only() {
        // The view can only read the `+` lines out of a git hunk, so the context
        // a merge folds in must stay out of the hash — otherwise the same change
        // would hash differently depending on what was edited nearby.
        let base = "one\ntwo\nthree\nfour\n";
        let working = "ONE\ntwo\nTHREE\nfour\n";
        let spans = hunk_spans(base, working);
        assert_eq!(
            spans[0].fingerprint,
            HunkFingerprint::of(&["ONE\n", "THREE\n"], 2),
            "the hash covers what the hunk adds and removes, not the two lines between"
        );
    }

    #[test]
    fn a_change_beyond_gits_context_stays_its_own_hunk() {
        // xdiff merges change groups while at most two contexts separate them;
        // this pins the boundary at 6 lines, the value `git diff` was observed
        // to use at its default context.
        let base: String = (1..=20).map(|n| format!("line{n}\n")).collect();
        let close = base.replacen("line1\n", "LINE1\n", 1).replacen("line8\n", "LINE8\n", 1);
        let spans = hunk_spans(&base, &close);
        assert_eq!(spans.len(), 1, "6 unchanged lines between them is one hunk");

        let far = base.replacen("line1\n", "LINE1\n", 1).replacen("line9\n", "LINE9\n", 1);
        let spans = hunk_spans(&base, &far);
        assert_eq!(spans.len(), 2, "7 unchanged lines between them is two hunks");
    }

    #[test]
    fn assigning_a_hunk_splits_the_file_and_leaves_the_rest_unassigned() {
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        let working = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
        let spans = hunk_spans(base, working);
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        index.set_hunk(PathBuf::from("a.rs"), spans[0].fingerprint, Some("ui".into()));

        let file = index.file(Path::new("a.rs")).unwrap();
        assert_eq!(file.branch(), None, "the file is split, not whole");
        let assigned: Vec<_> = file.hunks().collect();
        assert_eq!(
            assigned,
            [(spans[0].fingerprint, "ui")],
            "only the hunk that was named; the other is nobody's until it is named"
        );
    }

    #[test]
    fn a_branch_commit_takes_only_its_own_hunks() {
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        let working = "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
        let spans = hunk_spans(base, working);

        let ui = synthesize_for_branch(base, working, &spans, &|fingerprint| {
            fingerprint == spans[1].fingerprint
        });
        assert_eq!(
            ui, "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
            "the hunk that is not this branch's is reverted to the base"
        );

        let api = synthesize_for_branch(base, working, &spans, &|_| true);
        assert_eq!(api, working, "keeping every hunk reproduces the working tree");

        let neither = synthesize_for_branch(base, working, &spans, &|_| false);
        assert_eq!(neither, base, "keeping none reproduces the base");
    }

    #[test]
    fn an_unsplit_file_commits_unchanged() {
        // The overwhelmingly common case must not be perturbed by any of this.
        let base = "one\ntwo\n";
        let working = "one\nTWO\n";
        let spans = hunk_spans(base, working);
        assert_eq!(
            synthesize_for_branch(base, working, &spans, &|_| true),
            working
        );
    }

    #[test]
    fn an_insertion_kept_for_one_branch_is_reverted_for_the_other() {
        let base = "one\ntwo\n";
        let working = "one\nnew\ntwo\n";
        let spans = hunk_spans(base, working);
        assert_eq!(
            synthesize_for_branch(base, working, &spans, &|_| false),
            base,
            "an insertion that is not this branch's must not be left behind"
        );
        assert_eq!(
            synthesize_for_branch(base, working, &spans, &|_| true),
            working
        );
    }
}
