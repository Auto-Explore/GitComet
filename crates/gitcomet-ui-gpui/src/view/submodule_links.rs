//! Which open repository tab is a first-level submodule of which other tab.
//!
//! Detection reads the checkout's `.git` file and the parent's `.gitmodules`
//! once per working tree and remembers the answer, so tab rendering can ask
//! every frame.

use gitcomet_core::submodule_link::{SubmoduleParent, detect_submodule_parent};
use gitcomet_state::model::{AppState, RepoId, RepoState};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static PARENTS: RefCell<FxHashMap<PathBuf, Option<SubmoduleParent>>> =
        RefCell::new(FxHashMap::default());
}

/// The repository `workdir` is a submodule of, if any.
pub(crate) fn submodule_parent(workdir: &Path) -> Option<SubmoduleParent> {
    PARENTS.with(|parents| {
        parents
            .borrow_mut()
            .entry(workdir.to_path_buf())
            .or_insert_with(|| detect_submodule_parent(workdir))
            .clone()
    })
}

/// A submodule tab's link to its parent repository's open tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParentTab {
    pub(crate) repo_id: RepoId,
    /// The submodule's path inside the parent.
    pub(crate) path: PathBuf,
}

/// The open tab of the repository `repo` is a submodule of.
pub(crate) fn parent_tab(state: &AppState, repo: &RepoState) -> Option<ParentTab> {
    let parent = submodule_parent(&repo.spec.workdir)?;
    let tab = state
        .repos
        .iter()
        .find(|candidate| candidate.spec.workdir == parent.workdir)?;
    Some(ParentTab {
        repo_id: tab.id,
        path: parent.path,
    })
}

/// For each tab in order, the tab it is drawn attached to: its parent, when
/// the parent's tab sits right before it or before its sibling submodule
/// tabs. A detached submodule tab (dragged away, or opened before the parent
/// was) is drawn as an ordinary tab.
pub(crate) fn attached_parents(state: &AppState) -> Vec<Option<RepoId>> {
    let mut attached: Vec<Option<RepoId>> = Vec::with_capacity(state.repos.len());
    for (ix, repo) in state.repos.iter().enumerate() {
        let link = parent_tab(state, repo).and_then(|parent| {
            let previous = ix.checked_sub(1).and_then(|prev| state.repos.get(prev))?;
            let follows_parent = previous.id == parent.repo_id
                || attached
                    .last()
                    .copied()
                    .flatten()
                    .is_some_and(|previous_parent| previous_parent == parent.repo_id);
            follows_parent.then_some(parent.repo_id)
        });
        attached.push(link);
    }
    attached
}
