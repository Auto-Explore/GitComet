//! The virtual branches model: virtual branches, stacked branches, and the target branch.
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
//! and stacked when it is `Some`. Moving a branch between stacked branches, reordering a
//! stacked branch, and promoting a branch to independent are all the same edit: change
//! one field. That keeps the data model small enough to serialize to a file and
//! reason about in tests.
//!
//! The combined state of every *applied* branch is materialized as a single
//! branch, [`VIRTUAL_BRANCHES_BRANCH`]. It is an implementation detail of the model,
//! not a branch a user works on: commits belong to virtual branches, and the
//! virtual-branches branch is regenerated from them whenever the applied set changes.
//!
//! Everything here is pure data plus validation. Nothing in this module talks
//! to Git; see [`crate::services`] for the operations that do.

use crate::error::{Error, ErrorKind};
use crate::services::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The branch holding the combined tree of every applied virtual branch.
///
/// It is managed internally: GitComet rewrites it whenever the applied set
/// changes, and direct user commits to it are refused. It is named under a
/// `gitcomet/` prefix so it is visually distinct from a user's own branches and
/// so it can be filtered out of ordinary branch listings.
pub const VIRTUAL_BRANCHES_BRANCH: &str = "gitcomet/virtual";

/// The ref namespace the virtual branches owns. Branches under it are GitComet's own
/// bookkeeping and are never presented as user branches.
pub const VIRTUAL_BRANCHES_REF_PREFIX: &str = "gitcomet/";

/// Whether a virtual branch contributes its tree to the virtual branches.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchApplyState {
    /// Contributes to the virtual-branches branch and can take assigned changes.
    Applied,
    /// Exists as a branch but contributes nothing to the virtual branches.
    Unapplied,
}

impl BranchApplyState {
    pub const fn is_applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

/// One unit of work in the virtual branches.
///
/// `parent` is the whole of the stacking model: `None` means the branch is
/// stacked directly on the virtual branches target, `Some(name)` means it is stacked
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
    /// stacked, otherwise the virtual branches target.
    pub fn base_branch<'a>(&'a self, target: &'a str) -> &'a str {
        self.parent.as_deref().unwrap_or(target)
    }
}

/// One column of the virtual branches: a parent branch and everything stacked on it,
/// in application order.
///
/// Stacked branches are a view, not stored state. They are derived from `parent` links by
/// [`VirtualBranchesState::stacked_branches`], so a stacked branch can never disagree with the branches
/// that make it up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackedBranch {
    /// The target branch, or the name of the bottom branch of the stacked branch.
    pub base: String,
    /// Members from the bottom of the stacked branch upward, so `stacked_on` is
    /// unambiguous when rendering indentation.
    pub branches: Vec<StackedBranchMember>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackedBranchMember {
    pub branch: VirtualBranch,
    /// Depth within the stacked_branch: 0 for the branch that sits on the base.
    pub depth: usize,
}

impl StackedBranch {
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.branches.iter().any(|m| m.branch.name == name)
    }

    pub fn applied_count(&self) -> usize {
        self.branches
            .iter()
            .filter(|m| m.branch.is_applied())
            .count()
    }
}

/// The target branch plus every virtual branch, as one unit.
///
/// This is the structure the virtual branches UI reads and the structure that gets
/// written back after an edit. It carries no Git state: nothing here knows
/// whether a branch exists yet or what its tip is.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct VirtualBranchesState {
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

impl VirtualBranchesState {
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

    /// Whether a virtual branch of that name exists.
    ///
    /// The target is deliberately not counted: it is a branch the user already
    /// has, not one of the virtual branches's, and `validate` refuses a virtual branch
    /// that reuses the name. Callers asking "can I create this?" get the
    /// answer from this and from the repository's own branch list; a name that
    /// is only the target is a refusal the user can read, not a silent one.
    pub fn contains(&self, name: &str) -> bool {
        self.branches.iter().any(|b| b.name == name)
    }

    pub fn applied(&self) -> impl Iterator<Item = &VirtualBranch> {
        self.branches.iter().filter(|b| b.is_applied())
    }

    pub fn applied_names(&self) -> Vec<&str> {
        self.applied().map(|b| b.name.as_str()).collect()
    }

    /// Names that must be merged into the virtual-branches branch, in an order where
    /// a stacked branch always follows its parent.
    ///
    /// The order matters: applying `feature/ui` before `feature/api` would merge
    /// a commit whose parent is not yet present and fail to resolve. Independent
    /// branches keep their declared order so a virtual branches rebuild is
    /// deterministic and a diff of two rebuilds is empty.
    pub fn application_order(&self) -> Result<Vec<&VirtualBranch>> {
        let ordered = self.topological_order()?;
        Ok(ordered
            .into_iter()
            .filter(|name| self.get(name).is_some_and(VirtualBranch::is_applied))
            .filter_map(|name| self.get(&name))
            .collect())
    }

    /// Every branch name ordered so a child never precedes its parent.
    ///
    /// Roots come first in declaration order; each root is followed by the
    /// branches stacked on it, depth-first. A cycle is not reachable from a
    /// root, so it is left out and reported by [`Self::validate`].
    fn topological_order(&self) -> Result<Vec<String>> {
        let mut ordered = Vec::with_capacity(self.branches.len());
        let mut roots: Vec<&VirtualBranch> = self
            .branches
            .iter()
            .filter(|b| b.is_independent())
            .collect();
        roots.sort_by_key(|b| b.order.unwrap_or(u32::MAX));

        let mut remaining: Vec<&VirtualBranch> = self
            .branches
            .iter()
            .filter(|b| !b.is_independent())
            .collect();
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
            return Err(virtual_branches_error(format!(
                "branch '{}' is stacked on '{}', which is not a virtual-branches branch",
                orphan.name,
                orphan.parent.as_deref().unwrap_or_default(),
            )));
        }
        Ok(ordered)
    }

    /// The branches grouped into stacked branches, each with its depth.
    ///
    /// Unreachable branches (cycles) do not appear; [`Self::validate`] is what
    /// surfaces them, so a corrupt state degrades to a missing row rather than
    /// an infinite loop in the renderer.
    pub fn stacked_branches(&self) -> Result<Vec<StackedBranch>> {
        let order = self.topological_order()?;
        let mut stacked_branches: Vec<StackedBranch> = Vec::new();

        for name in order {
            let Some(branch) = self.get(&name) else {
                continue;
            };
            match &branch.parent {
                None => stacked_branches.push(StackedBranch {
                    base: self.target.clone(),
                    branches: vec![StackedBranchMember {
                        branch: branch.clone(),
                        depth: 0,
                    }],
                }),
                Some(parent) => {
                    let depth = match stacked_branches
                        .iter_mut()
                        .rev()
                        .find(|s| s.contains(parent))
                    {
                        Some(stacked_branch) => {
                            let depth = stacked_branch
                                .branches
                                .iter()
                                .find(|m| m.branch.name == *parent)
                                .map_or(0, |m| m.depth + 1);
                            stacked_branch.branches.push(StackedBranchMember {
                                branch: branch.clone(),
                                depth,
                            });
                            depth
                        }
                        None => {
                            stacked_branches.push(StackedBranch {
                                base: parent.clone(),
                                branches: vec![StackedBranchMember {
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

        Ok(stacked_branches)
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
                return Err(virtual_branches_error(format!(
                    "duplicate virtual-branches branch '{}'",
                    branch.name
                )));
            }
        }

        for branch in &self.branches {
            let Some(parent) = &branch.parent else {
                continue;
            };
            if *parent == branch.name {
                return Err(virtual_branches_error(format!(
                    "branch '{}' is stacked on itself",
                    branch.name
                )));
            }
            if !self.branches.iter().any(|b| &b.name == parent) {
                return Err(virtual_branches_error(format!(
                    "branch '{}' is stacked on '{}', which is not a virtual-branches branch",
                    branch.name, parent
                )));
            }
            // A loop exists when `parent`'s own ancestor chain comes back
            // around to `branch` — walking `branch`'s chain instead would find
            // the edge just added and reject every stacking edit.
            if self
                .get(parent)
                .is_some_and(|parent_branch| self.reaches(parent_branch, &branch.name))
            {
                return Err(virtual_branches_error(format!(
                    "branch '{}' is stacked on '{}', which already depends on it",
                    branch.name, parent
                )));
            }
        }

        if self.branches.iter().any(|b| b.name == self.target) {
            return Err(virtual_branches_error(format!(
                "'{}' is the virtual branches target and cannot also be a virtual-branches branch",
                self.target
            )));
        }
        for branch in &self.branches {
            if branch.name.starts_with(VIRTUAL_BRANCHES_REF_PREFIX) {
                return Err(virtual_branches_error(format!(
                    "'{}' is inside the reserved '{}' namespace",
                    branch.name, VIRTUAL_BRANCHES_REF_PREFIX
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

    /// Insert a branch above `name`: the new branch is stacked on `name`.
    pub fn insert_above(&mut self, branch: VirtualBranch, name: &str) -> Result<()> {
        if self.get(name).is_none() {
            return Err(virtual_branches_error(format!(
                "no virtual-branches branch '{name}'"
            )));
        }
        let previous = self.branches.clone();
        let result = self
            .push_branch(branch.clone())
            .and_then(|()| self.set_parent(&branch.name, Some(name)));
        if result.is_err() {
            self.branches = previous;
        }
        result
    }

    /// Insert a branch below `name`: it takes `name`'s place in the stacked branch, so
    /// `name` ends up sitting on it and everything above follows along.
    ///
    /// The mirror of [`Self::insert_above`], and deliberately not the same
    /// operation. A branch *above* an anchor is stacked on it; a branch *below*
    /// one displaces it. Doing the first for both would make the two buttons the
    /// same edit, and neither would mean what the label on it says.
    pub fn insert_below(&mut self, branch: VirtualBranch, name: &str) -> Result<()> {
        let displaced = self
            .get(name)
            .ok_or_else(|| virtual_branches_error(format!("no virtual-branches branch '{name}'")))?
            .parent
            .clone();
        let previous = self.branches.clone();
        // The new branch inherits the anchor's old parent, so the stacked branch keeps
        // its depth and everything that was above the anchor stays above it.
        let result = self
            .push_branch(branch.clone())
            .and_then(|()| self.set_parent(name, Some(&branch.name)))
            .and_then(|()| self.set_parent(&branch.name, displaced.as_deref()));
        if result.is_err() {
            self.branches = previous;
        }
        result
    }

    /// Move `name` into `stacked_branch_base`'s stacked branch, optionally above or below
    /// `relative_to`.
    pub fn move_to_stacked_branch(
        &mut self,
        name: &str,
        stacked_branch_base: &str,
        relative_to: Option<&str>,
        below: bool,
    ) -> Result<()> {
        if self.get(name).is_none() {
            return Err(virtual_branches_error(format!(
                "no virtual-branches branch '{name}'"
            )));
        }
        let parent = match relative_to {
            None => Some(stacked_branch_base.to_string()),
            Some(relative) if relative == name => self.get(name).and_then(|b| b.parent.clone()),
            Some(relative) => {
                // A relative branch that is not in the virtual branches cannot be
                // reasoned about. Falling back to `stacked_branch_base` would land the
                // branch somewhere the user never asked for and report success,
                // which is the same failure `insert_above`/`insert_below` refuse.
                if self.get(relative).is_none() {
                    return Err(virtual_branches_error(format!(
                        "no virtual-branches branch '{relative}'"
                    )));
                }
                if below {
                    Some(relative.to_string())
                } else {
                    Some(
                        self.get(relative)
                            .and_then(|b| b.parent.clone())
                            .unwrap_or_else(|| stacked_branch_base.to_string()),
                    )
                }
            }
        };
        // Stacking onto the target means independent: the target is not a
        // virtual-branches branch and `validate` would refuse it as a parent.
        let parent = parent.filter(|parent| parent != &self.target);
        self.set_parent(name, parent.as_deref())
    }

    /// Swap two branches' positions, keeping each one's place in its stacked branch.
    pub fn reorder(&mut self, first: &str, second: &str) -> Result<()> {
        if first == second {
            return Ok(());
        }
        let left = self
            .get(first)
            .ok_or_else(|| virtual_branches_error(format!("no virtual-branches branch '{first}'")))?
            .clone();
        let right = self
            .get(second)
            .ok_or_else(|| {
                virtual_branches_error(format!("no virtual-branches branch '{second}'"))
            })?
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
        // different parents are in different stacked branches, and "swap" has no meaning
        // without first choosing where the second one goes.
        if left.parent != right.parent {
            return Err(virtual_branches_error(format!(
                "'{first}' and '{second}' are in different stacked branches; move one into the other's stacked branch instead"
            )));
        }

        // Siblings are ordered by their position, with unpositioned ones after
        // the positioned ones in declaration order. Swapping two of them means
        // swapping where they sit in that sequence, whatever mix of positioned
        // and unpositioned branches they are, so every sibling is given its
        // current place explicitly and the pair then trades places.
        let mut siblings: Vec<String> = {
            let mut siblings: Vec<&VirtualBranch> = self
                .branches
                .iter()
                .filter(|b| b.parent == left.parent)
                .collect();
            siblings.sort_by_key(|b| b.order.unwrap_or(u32::MAX));
            siblings.into_iter().map(|b| b.name.clone()).collect()
        };
        let first_index = siblings.iter().position(|name| name == first);
        let second_index = siblings.iter().position(|name| name == second);
        if let (Some(first_index), Some(second_index)) = (first_index, second_index) {
            siblings.swap(first_index, second_index);
        }
        for (index, name) in siblings.iter().enumerate() {
            if let Some(branch) = self.get_mut(name) {
                branch.order = Some(index as u32);
            }
        }
        Ok(())
    }

    /// Add a branch, rejecting a name that is already taken.
    pub fn push_branch(&mut self, branch: VirtualBranch) -> Result<()> {
        validate_branch_name(&branch.name)?;
        if branch.name.starts_with(VIRTUAL_BRANCHES_REF_PREFIX) {
            return Err(virtual_branches_error(format!(
                "'{}' is inside the reserved '{}' namespace",
                branch.name, VIRTUAL_BRANCHES_REF_PREFIX
            )));
        }
        if self.get(&branch.name).is_some() {
            return Err(virtual_branches_error(format!(
                "'{}' is already a virtual-branches branch",
                branch.name
            )));
        }
        self.branches.push(branch);
        if let Err(error) = self.validate() {
            self.branches.pop();
            return Err(error);
        }
        Ok(())
    }

    /// Remove a branch, re-parenting whatever was stacked on it so the stacked branch
    /// does not develop a hole.
    pub fn remove_branch(&mut self, name: &str) -> Result<VirtualBranch> {
        let index = self
            .branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| {
                virtual_branches_error(format!("no virtual-branches branch '{name}'"))
            })?;
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
            return Err(virtual_branches_error(format!(
                "no virtual-branches branch '{name}'"
            )));
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

    /// Set whether a branch contributes to the virtual-branches branch.
    pub fn set_applied(&mut self, name: &str, applied: BranchApplyState) -> Result<()> {
        let Some(branch) = self.get_mut(name) else {
            return Err(virtual_branches_error(format!(
                "no virtual-branches branch '{name}'"
            )));
        };
        branch.applied = applied;
        Ok(())
    }

    /// Point the virtual branches at a different target branch.
    ///
    /// Independent branches keep their names; only their base moves. Callers
    /// rebase them onto the new target with
    /// [`crate::services::VirtualBranchesRepository::update_virtual_branches_target`].
    pub fn set_target(&mut self, target: impl Into<String>) -> Result<()> {
        let target = target.into();
        if target.is_empty() {
            return Err(virtual_branches_error(
                "the virtual branches target cannot be empty".into(),
            ));
        }
        if self.branches.iter().any(|b| b.name == target) {
            return Err(virtual_branches_error(format!(
                "'{target}' is already a virtual-branches branch and cannot also be the target"
            )));
        }
        self.target = target;
        Ok(())
    }
}

fn virtual_branches_error(message: String) -> Error {
    Error::new(ErrorKind::Backend(message))
}

/// Reject names Git would refuse anyway, before they reach a command that
/// fails with a less specific message.
fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(virtual_branches_error(
            "a virtual-branches branch needs a name".into(),
        ));
    }
    if name.starts_with('-') {
        return Err(virtual_branches_error(format!(
            "'{name}' cannot start with '-'"
        )));
    }
    if !name.is_ascii() {
        return Err(virtual_branches_error(format!(
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
        return Err(virtual_branches_error(format!(
            "'{name}' is not a valid branch name"
        )));
    }
    for part in name.split('/') {
        if part.is_empty() || part.starts_with('.') || part.ends_with(".lock") {
            return Err(virtual_branches_error(format!(
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
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize,
)]
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
        for line in new_side {
            // The line's length goes in first. Hashing the concatenation alone
            // cannot tell two lines from one, so `["X", "\nY"]` and
            // `["X\n", "Y"]` — different changes that happen to produce the same
            // bytes — would share a fingerprint and one would drag the other
            // along when the user assigned it.
            hash ^= line.len() as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            for byte in line.as_bytes() {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
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
    use crate::file_diff::{FileDiffPlanRun, side_by_side_plan};

    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let plan = side_by_side_plan(old, new);
    let mut spans: Vec<HunkSpan> = Vec::new();
    // The hunk the previous change run opened, if any. Adjacent change runs —
    // no context between them at all — are unambiguously one hunk; runs with a
    // little context between them are folded together later, once the gap is
    // known.
    let mut open: Option<(usize, usize, usize, usize)> = None;
    // The base line the next change starts at, 0-based (plan runs are
    // 0-based). Context runs move it; insertions do not, which is what anchors
    // a pure insertion to the line it sits in front of.
    let mut base_pos = 0usize;

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
///
/// The same reasoning extends to a pure insertion or removal the merge folded
/// in beside a replacement: whether the diff also happened to show a line added
/// above depends on which base the two sides are diffing (the view diffs `HEAD`,
/// a commit diffs the branch's base), so a hunk containing both is fingerprinted
/// by its replacements alone. A group with no replacement at all — a purely
/// added or purely removed hunk — is fingerprinted by everything it contains,
/// because there is nothing else to key on. The view applies the same rule to
/// the change groups inside the `@@` it rendered.
fn merge_neighbouring_changes(spans: Vec<HunkSpan>, new_lines: &[&str]) -> Vec<HunkSpan> {
    let max_gap = 2 * HUNK_CONTEXT_LINES;
    let mut groups: Vec<Vec<HunkSpan>> = Vec::new();
    for span in spans {
        let gap = groups
            .last()
            .and_then(|group| group.last())
            .map_or(max_gap + 1, |prev| {
                span.base_range.start.saturating_sub(prev.base_range.end)
            });
        if gap <= max_gap
            && let Some(group) = groups.last_mut()
        {
            group.push(span);
            continue;
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
            // A replacement touches both sides; a pure insertion or removal
            // touches one. See the function docs for why only replacements
            // contribute when the group has any.
            let is_replacement =
                |span: &HunkSpan| !span.base_range.is_empty() && !span.new_range.is_empty();
            let has_replacement = group.iter().any(is_replacement);
            let contributing: Vec<&HunkSpan> = group
                .iter()
                .filter(|span| !has_replacement || is_replacement(span))
                .collect();
            let produced: Vec<&str> = contributing
                .iter()
                .flat_map(|span| {
                    new_lines
                        .get(span.new_range.clone())
                        .map(<[&str]>::to_vec)
                        .unwrap_or_default()
                })
                .collect();
            let base_lines: usize = contributing.iter().map(|span| span.base_range.len()).sum();
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
    // A pure insertion is an empty range at the line it sits in front of;
    // plan runs are 0-based, so the ranges are taken as-is.
    let base_range = base_start..base_start + base_lines;
    let new_range = new_start..new_start + new_lines;
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
            // `.get(..)` rather than indexing: a span whose range runs past the
            // end would panic, and this runs inside a commit the user is waiting
            // on. Everything else here degrades instead of dying, and a hunk
            // that contributes nothing is recoverable in a way a panic is not.
            for line in working_lines
                .get(span.new_range.clone())
                .unwrap_or_default()
                .iter()
                .copied()
            {
                out.push_str(line);
            }
        } else {
            for line in base_lines
                .get(span.base_range.clone())
                .unwrap_or_default()
                .iter()
                .copied()
            {
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
    // A fingerprint is a struct, and serde_json only accepts string map keys:
    // serializing the derive directly fails with "key must be a string",
    // which would lose every split assignment the moment the file is written.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "hunks_as_string_keys"
    )]
    hunks: BTreeMap<HunkFingerprint, String>,
}

/// `hunks` as a JSON object keyed by `"<content>:<new_lines>:<base_lines>"`.
///
/// The on-disk assignment file is JSON, whose map keys must be strings, while
/// [`HunkFingerprint`] is a three-field struct. Encoding the key as text keeps
/// the file an object a human can read and round-trips exactly: all three
/// fields are part of the identity, so nothing is lost in the encoding.
mod hunks_as_string_keys {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::HunkFingerprint;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<HunkFingerprint, String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_map(Some(map.len()))?;
        for (fingerprint, branch) in map {
            let key = format!(
                "{}:{}:{}",
                fingerprint.content, fingerprint.new_lines, fingerprint.base_lines
            );
            out.serialize_entry(&key, branch)?;
        }
        out.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<HunkFingerprint, String>, D::Error> {
        let raw = BTreeMap::<String, String>::deserialize(deserializer)?;
        raw.into_iter()
            .map(|(key, branch)| {
                let fingerprint = decode(&key).ok_or_else(|| {
                    D::Error::custom(format!(
                        "hunk fingerprint key {key:?} is not content:new:base"
                    ))
                })?;
                Ok((fingerprint, branch))
            })
            .collect()
    }

    fn decode(key: &str) -> Option<HunkFingerprint> {
        let (content, rest) = key.split_once(':')?;
        let (new_lines, base_lines) = rest.split_once(':')?;
        Some(HunkFingerprint {
            content: content.parse().ok()?,
            new_lines: new_lines.parse().ok()?,
            base_lines: base_lines.parse().ok()?,
        })
    }
}

/// The optional hunk lookup [`FileAssignments::retain`] takes: the hunks a
/// path currently has, or nothing when the caller has no diff to hand.
type HunkLookup<'a> = Option<&'a dyn Fn(&Path) -> Vec<HunkSpan>>;

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
        self.hunks
            .iter()
            .map(|(key, branch)| (*key, branch.as_str()))
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
    pub fn retain_hunks(&mut self, current: &[HunkSpan], state: &VirtualBranchesState) {
        self.hunks.retain(|fingerprint, branch| {
            state.get(branch).is_some()
                && current.iter().any(|span| span.fingerprint == *fingerprint)
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
        self.entries
            .insert(path.into(), FileAssignments::whole(branch));
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
    /// so the answer is filtered through the virtual branches state. A split file has
    /// no single branch and so has no answer here; callers that care read
    /// [`AssignmentIndex::file`].
    pub fn resolve<'a>(&self, path: &Path, state: &'a VirtualBranchesState) -> Option<&'a str> {
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
                    || assignments
                        .hunk_branches()
                        .any(|assigned| assigned == branch)
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
    ///
    /// A split file whose hunks were pruned stays as a row with nothing
    /// assigned when at least one of its hunks named a branch that still
    /// exists — the path is still changed, it just has nothing to act on. It
    /// leaves with its branches when every branch it named is gone.
    pub fn retain(
        &mut self,
        state: &VirtualBranchesState,
        changed: &dyn Fn(&Path) -> bool,
        hunks_of: HunkLookup<'_>,
    ) {
        self.entries.retain(|path, assignments| {
            if !changed(path) {
                return false;
            }
            // Judged before pruning: once the hunks are gone the reason they
            // were dropped — a dead branch or a reverted change — is no longer
            // visible, and the two have to end up in different places.
            let had_live_hunk = assignments
                .hunk_branches()
                .any(|branch| state.get(branch).is_some());
            if !assignments.hunks.is_empty() {
                match hunks_of {
                    Some(hunks_of) => assignments.retain_hunks(&hunks_of(path), state),
                    // Without a diff to hand, the only thing a hunk can be
                    // judged by is the branch it names. Keeping one whose branch
                    // is gone would make the assignment outlive the branch it was
                    // made for, and it would be written back to disk every load.
                    None => assignments
                        .hunks
                        .retain(|_, branch| state.get(branch).is_some()),
                }
            }
            let Some(branch) = assignments.whole.as_ref() else {
                return had_live_hunk;
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
mod tests;
