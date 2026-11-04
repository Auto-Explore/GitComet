//! Which open tab is a first-level submodule of which, and the tab order that
//! keeps each submodule tab directly after its parent worktree's tab.

use crate::model::{AppState, RepoId, RepoState, SubmoduleParentLink};
use crate::msg::{Effect, PersistenceEffect};
use gitcomet_core::domain::{SubmoduleParent, SubmoduleStatus};
use std::path::{Path, PathBuf};

/// Record what the open task read about the repository, link it to its
/// parent's tab (or link waiting children to it), and regroup the tabs.
pub(super) fn submodule_parent_resolved(
    state: &mut AppState,
    repo_id: RepoId,
    canonical_workdir: PathBuf,
    parent: Option<SubmoduleParent>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    repo.canonical_workdir = Some(canonical_workdir);
    repo.submodule_parent =
        parent
            .filter(|parent| !parent.nested)
            .map(|parent| SubmoduleParentLink {
                workdir: parent.workdir,
                path: parent.path,
                repo_id: None,
            });
    relink_and_group(state)
}

/// Re-resolve every link against the open tabs, then regroup. Returns the
/// session write a changed order needs.
pub(super) fn relink_and_group(state: &mut AppState) -> Vec<Effect> {
    relink(state);
    if group(state) {
        vec![Effect::Persistence(PersistenceEffect::PersistSession {
            repo_id: state.active_repo,
            action: "grouping submodule tabs",
        })]
    } else {
        Vec::new()
    }
}

fn repo_matches_workdir(repo: &RepoState, workdir: &Path) -> bool {
    repo.canonical_workdir.as_deref() == Some(workdir) || repo.spec.workdir == workdir
}

/// Point each link at its parent's open tab, or at none.
pub(super) fn relink(state: &mut AppState) {
    let parents: Vec<(usize, Option<RepoId>)> = state
        .repos
        .iter()
        .enumerate()
        .filter_map(|(ix, repo)| {
            let link = repo.submodule_parent.as_ref()?;
            let parent_id = state
                .repos
                .iter()
                .find(|candidate| {
                    candidate.id != repo.id
                        // A parent that is itself linked would make this a
                        // second-level submodule; the backend already says so,
                        // this keeps a stale link from chaining.
                        && candidate.submodule_parent.is_none()
                        && repo_matches_workdir(candidate, &link.workdir)
                })
                .map(|candidate| candidate.id);
            Some((ix, parent_id))
        })
        .collect();
    for (ix, parent_id) in parents {
        if let Some(link) = state.repos[ix].submodule_parent.as_mut()
            && link.repo_id != parent_id
        {
            link.repo_id = parent_id;
        }
    }
}

/// Put each linked submodule tab right after its parent's tab, keeping the
/// relative order of everything else. Returns whether the order changed.
pub(super) fn group(state: &mut AppState) -> bool {
    let order: Vec<RepoId> = state.repos.iter().map(|repo| repo.id).collect();
    let mut grouped: Vec<RepoId> = Vec::with_capacity(order.len());
    for repo in &state.repos {
        if repo.submodule_parent_tab().is_some() {
            continue;
        }
        grouped.push(repo.id);
        grouped.extend(
            state
                .repos
                .iter()
                .filter(|child| child.submodule_parent_tab() == Some(repo.id))
                .map(|child| child.id),
        );
    }
    // A child whose parent id no longer names an open tab keeps its place.
    for repo in &state.repos {
        if !grouped.contains(&repo.id) {
            grouped.push(repo.id);
        }
    }
    if grouped == order {
        return false;
    }
    state.repos.sort_by_key(|repo| {
        grouped
            .iter()
            .position(|id| *id == repo.id)
            .unwrap_or(usize::MAX)
    });
    true
}

/// When `path` is a loaded, first-level submodule of an open worktree tab,
/// the link a new tab for it starts with, so it opens next to its parent
/// before the backend confirms the relationship.
pub(super) fn provisional_link_for_open(
    state: &AppState,
    path: &Path,
) -> Option<SubmoduleParentLink> {
    state.repos.iter().find_map(|parent| {
        if parent.submodule_parent.is_some() {
            return None;
        }
        let relative = path.strip_prefix(&parent.spec.workdir).ok()?;
        if relative.as_os_str().is_empty() {
            return None;
        }
        let gitcomet_base::Loadable::Ready(submodules) = &parent.submodules else {
            return None;
        };
        submodules
            .iter()
            .any(|submodule| {
                !submodule.nested
                    && submodule.path == relative
                    && submodule.status != SubmoduleStatus::NotInitialized
            })
            .then(|| SubmoduleParentLink {
                workdir: parent
                    .canonical_workdir
                    .clone()
                    .unwrap_or_else(|| parent.spec.workdir.clone()),
                path: relative.to_path_buf(),
                repo_id: Some(parent.id),
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;

    fn repo(id: u64, path: &str) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(id),
            RepoSpec {
                workdir: PathBuf::from(path),
            },
        );
        repo.canonical_workdir = Some(PathBuf::from(path));
        repo
    }

    fn state_with(repos: Vec<RepoState>) -> AppState {
        let mut state = AppState::default();
        state.repos = repos;
        state
    }

    fn order(state: &AppState) -> Vec<u64> {
        state.repos.iter().map(|repo| repo.id.0).collect()
    }

    fn parent(path: &str, sub: &str) -> SubmoduleParent {
        SubmoduleParent {
            workdir: PathBuf::from(path),
            path: PathBuf::from(sub),
            nested: false,
        }
    }

    #[test]
    fn a_submodule_opened_after_its_parent_moves_next_to_it() {
        let mut state = state_with(vec![
            repo(1, "/w/app"),
            repo(2, "/w/other"),
            repo(3, "/w/app/vendor/lib"),
        ]);
        let effects = submodule_parent_resolved(
            &mut state,
            RepoId(3),
            PathBuf::from("/w/app/vendor/lib"),
            Some(parent("/w/app", "vendor/lib")),
        );
        assert_eq!(order(&state), vec![1, 3, 2]);
        assert_eq!(effects.len(), 1, "the new order is persisted");
        assert_eq!(state.repos[1].submodule_parent_tab(), Some(RepoId(1)));
    }

    #[test]
    fn a_parent_opened_after_its_submodule_pulls_the_submodule_along() {
        let mut state = state_with(vec![repo(3, "/w/app/vendor/lib"), repo(2, "/w/other")]);
        submodule_parent_resolved(
            &mut state,
            RepoId(3),
            PathBuf::from("/w/app/vendor/lib"),
            Some(parent("/w/app", "vendor/lib")),
        );
        assert_eq!(order(&state), vec![3, 2], "no parent tab yet: order kept");
        assert_eq!(state.repos[0].submodule_parent_tab(), None);

        state.repos.push(repo(1, "/w/app"));
        submodule_parent_resolved(&mut state, RepoId(1), PathBuf::from("/w/app"), None);
        assert_eq!(order(&state), vec![2, 1, 3]);
        assert_eq!(state.repos[2].submodule_parent_tab(), Some(RepoId(1)));
    }

    #[test]
    fn closing_the_parent_unlinks_and_reopening_relinks() {
        let mut state = state_with(vec![repo(1, "/w/app"), repo(3, "/w/app/vendor/lib")]);
        submodule_parent_resolved(
            &mut state,
            RepoId(3),
            PathBuf::from("/w/app/vendor/lib"),
            Some(parent("/w/app", "vendor/lib")),
        );
        assert_eq!(state.repos[1].submodule_parent_tab(), Some(RepoId(1)));

        state.repos.retain(|repo| repo.id != RepoId(1));
        relink_and_group(&mut state);
        assert_eq!(state.repos[0].submodule_parent_tab(), None);
        assert!(state.repos[0].submodule_parent.is_some(), "the link waits");

        state.repos.push(repo(4, "/w/app"));
        submodule_parent_resolved(&mut state, RepoId(4), PathBuf::from("/w/app"), None);
        assert_eq!(order(&state), vec![4, 3]);
        assert_eq!(state.repos[1].submodule_parent_tab(), Some(RepoId(4)));
    }

    #[test]
    fn a_second_level_submodule_gets_no_link() {
        let mut state = state_with(vec![
            repo(1, "/w/app/vendor/lib"),
            repo(2, "/w/x"),
            repo(3, "/w/app/vendor/lib/deep"),
        ]);
        submodule_parent_resolved(
            &mut state,
            RepoId(3),
            PathBuf::from("/w/app/vendor/lib/deep"),
            Some(SubmoduleParent {
                workdir: PathBuf::from("/w/app/vendor/lib"),
                path: PathBuf::from("deep"),
                nested: true,
            }),
        );
        assert!(state.repos[2].submodule_parent.is_none());
        assert_eq!(order(&state), vec![1, 2, 3]);
    }

    #[test]
    fn siblings_keep_their_relative_order() {
        let mut state = state_with(vec![
            repo(1, "/w/app"),
            repo(5, "/w/x"),
            repo(3, "/w/app/b"),
            repo(2, "/w/app/a"),
        ]);
        for (id, path, sub) in [(3, "/w/app/b", "b"), (2, "/w/app/a", "a")] {
            submodule_parent_resolved(
                &mut state,
                RepoId(id),
                PathBuf::from(path),
                Some(parent("/w/app", sub)),
            );
        }
        assert_eq!(order(&state), vec![1, 3, 2, 5]);
        assert_eq!(
            state.submodule_children(RepoId(1)),
            vec![RepoId(3), RepoId(2)]
        );
    }

    #[test]
    fn a_provisional_link_needs_a_loaded_first_level_row() {
        let mut parent_repo = repo(1, "/w/app");
        let mut row = gitcomet_core::domain::Submodule::new(
            PathBuf::from("vendor/lib"),
            gitcomet_core::domain::CommitId("a".into()),
            Some(gitcomet_core::domain::CommitId("a".into())),
            SubmoduleStatus::UpToDate,
        );
        parent_repo.set_submodules(gitcomet_base::Loadable::Ready(vec![row.clone()]));
        let state = state_with(vec![parent_repo]);
        let link = provisional_link_for_open(&state, Path::new("/w/app/vendor/lib"))
            .expect("a loaded first-level submodule opens next to its parent");
        assert_eq!(link.repo_id, Some(RepoId(1)));
        assert_eq!(link.path, PathBuf::from("vendor/lib"));
        assert!(provisional_link_for_open(&state, Path::new("/w/app/src")).is_none());

        row.nested = true;
        let mut parent_repo = repo(1, "/w/app");
        parent_repo.set_submodules(gitcomet_base::Loadable::Ready(vec![row]));
        let state = state_with(vec![parent_repo]);
        assert!(provisional_link_for_open(&state, Path::new("/w/app/vendor/lib")).is_none());
    }
}
