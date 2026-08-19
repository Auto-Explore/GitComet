use super::branch_sidebar::{self, BranchSidebarRow};
use super::caches::{
    BranchSidebarCache, BranchSidebarFingerprint, branch_sidebar_cache_lookup,
    branch_sidebar_cache_lookup_by_cached_source, branch_sidebar_cache_lookup_by_source,
    branch_sidebar_cache_store,
};
use super::*;
use gitcomet_state::model::SidebarDataRequest;
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct WorkspaceBadgeIndex {
    listed_paths_by_branch: Arc<FxHashMap<String, PathBuf>>,
    active_paths_by_branch: Arc<FxHashMap<String, PathBuf>>,
}

impl WorkspaceBadgeIndex {
    fn for_state(repo: &RepoState, open_repos: &[RepoState]) -> Self {
        Self {
            listed_paths_by_branch: Arc::new(crate::view::rows::listed_workspace_paths_by_branch(
                repo,
            )),
            active_paths_by_branch: Arc::new(crate::view::rows::active_workspace_paths_by_branch(
                repo, open_repos,
            )),
        }
    }

    pub(in crate::view) fn listed_path(&self, branch: &str) -> Option<&PathBuf> {
        self.listed_paths_by_branch.get(branch)
    }

    pub(in crate::view) fn active_path(&self, branch: &str) -> Option<&PathBuf> {
        self.active_paths_by_branch.get(branch)
    }
}

#[derive(Clone)]
pub(in crate::view) struct SidebarPresentation {
    pub(in crate::view) rows: Rc<[BranchSidebarRow]>,
    pub(in crate::view) workspace_badges: WorkspaceBadgeIndex,
}

#[derive(Default)]
pub(in crate::view) struct SidebarPresentationCache {
    branch_rows: Option<BranchSidebarCache>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct SidebarRequestFingerprint {
    active_repo_id: Option<RepoId>,
    request: Option<SidebarDataRequest>,
}

pub(in crate::view) fn active_sidebar_data_request(
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
) -> Option<(RepoId, SidebarDataRequest)> {
    let repo_id = state.active_repo?;
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    let empty = BTreeSet::new();
    let collapsed_items = collapsed_items_by_repo
        .get(&repo.spec.workdir)
        .unwrap_or(&empty);
    Some((
        repo_id,
        SidebarDataRequest {
            worktrees: true,
            submodules: !branch_sidebar::is_collapsed(
                collapsed_items,
                branch_sidebar::submodules_section_storage_key(),
            ),
            stashes: !branch_sidebar::is_collapsed(
                collapsed_items,
                branch_sidebar::stash_section_storage_key(),
            ),
        },
    ))
}

pub(in crate::view) fn sidebar_request_fingerprint(
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
) -> SidebarRequestFingerprint {
    let (active_repo_id, request) = active_sidebar_data_request(state, collapsed_items_by_repo)
        .map_or((state.active_repo, None), |(repo_id, request)| {
            (Some(repo_id), Some(request))
        });
    SidebarRequestFingerprint {
        active_repo_id,
        request,
    }
}

pub(in crate::view) fn build_sidebar_presentation(
    cache: &mut SidebarPresentationCache,
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    pinned_branches_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    branch_filter: &str,
) -> Option<SidebarPresentation> {
    let repo_id = state.active_repo?;
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    let empty = BTreeSet::new();
    let collapsed_items = collapsed_items_by_repo
        .get(&repo.spec.workdir)
        .unwrap_or(&empty);
    let pinned_branches = pinned_branches_by_repo
        .get(&repo.spec.workdir)
        .unwrap_or(&empty);

    Some(SidebarPresentation {
        rows: branch_sidebar_rows_cached(
            &mut cache.branch_rows,
            repo,
            collapsed_items,
            pinned_branches,
            branch_filter,
        ),
        workspace_badges: WorkspaceBadgeIndex::for_state(repo, state.repos.as_slice()),
    })
}

fn branch_sidebar_rows_cached(
    cache: &mut Option<BranchSidebarCache>,
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
    pinned_branches: &BTreeSet<String>,
    branch_filter: &str,
) -> Rc<[BranchSidebarRow]> {
    // A live filter query changes rows independently of the cached repo/source
    // fingerprints, so bypass the cache entirely while filtering (and don't
    // pollute it with filtered results).
    if !branch_filter.trim().is_empty() {
        return branch_sidebar::branch_sidebar_rows(
            repo,
            collapsed_items,
            pinned_branches,
            branch_filter,
        )
        .into();
    }

    let fingerprint = BranchSidebarFingerprint::from_repo(repo);

    if let Some(rows) = branch_sidebar_cache_lookup(cache, repo.id, fingerprint) {
        return rows;
    }

    if let Some(rows) = branch_sidebar_cache_lookup_by_cached_source(cache, repo, fingerprint) {
        return rows;
    }

    let (source_fingerprint, source_parts) = {
        let cached_source_parts = cache
            .as_ref()
            .filter(|cached| cached.repo_id == repo.id)
            .map(|cached| &cached.source_parts);
        branch_sidebar::branch_sidebar_source_fingerprint(repo, cached_source_parts)
    };

    if let Some(rows) = branch_sidebar_cache_lookup_by_source(
        cache,
        repo.id,
        fingerprint,
        source_fingerprint,
        &source_parts,
    ) {
        return rows;
    }

    let rows: Rc<[BranchSidebarRow]> =
        branch_sidebar::branch_sidebar_rows(repo, collapsed_items, pinned_branches, "").into();

    branch_sidebar_cache_store(
        cache,
        repo.id,
        fingerprint,
        source_fingerprint,
        source_parts,
        Rc::clone(&rows),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_state(id: RepoId, path: &str) -> RepoState {
        RepoState::new_opening(
            id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from(path),
            },
        )
    }

    fn worktree_branch_for_path(rows: &[BranchSidebarRow], path: &str) -> Option<String> {
        rows.iter().find_map(|row| match row {
            BranchSidebarRow::WorktreeItem {
                path: row_path,
                branch: Some(branch),
                ..
            } if row_path == &PathBuf::from(path) => Some(branch.to_string()),
            _ => None,
        })
    }

    #[test]
    fn active_sidebar_data_request_always_requests_worktrees() {
        let state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            ..Default::default()
        };

        let (_, request) =
            active_sidebar_data_request(&state, &BTreeMap::new()).expect("request exists");

        assert!(request.worktrees);
        assert!(!request.submodules);
        assert!(!request.stashes);
    }

    #[test]
    fn active_sidebar_data_request_respects_repo_collapse_state() {
        let state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            ..Default::default()
        };
        let collapsed_items = BTreeMap::from([(
            PathBuf::from("/tmp/repo"),
            BTreeSet::from([
                branch_sidebar::expanded_default_section_storage_key(
                    branch_sidebar::submodules_section_storage_key(),
                )
                .expect("submodules should support explicit expansion"),
                branch_sidebar::expanded_default_section_storage_key(
                    branch_sidebar::stash_section_storage_key(),
                )
                .expect("stash should support explicit expansion"),
            ]),
        )]);

        let (_, request) =
            active_sidebar_data_request(&state, &collapsed_items).expect("request exists");

        assert!(request.worktrees);
        assert!(request.submodules);
        assert!(request.stashes);
    }

    #[test]
    fn build_sidebar_presentation_reloads_worktree_row_branch_after_worktree_refresh() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;
        repo.branch_sidebar_rev = 1;
        let mut state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..Default::default()
        };
        let expanded_worktrees = branch_sidebar::expanded_default_section_storage_key(
            branch_sidebar::worktrees_section_storage_key(),
        )
        .expect("worktrees should support explicit expansion");
        let collapsed_items = BTreeMap::from([(
            PathBuf::from("/tmp/repo"),
            BTreeSet::from([expanded_worktrees]),
        )]);
        let mut cache = SidebarPresentationCache::default();

        let initial =
            build_sidebar_presentation(&mut cache, &state, &collapsed_items, &BTreeMap::new(), "")
                .expect("initial sidebar presentation");
        assert_eq!(
            worktree_branch_for_path(initial.rows.as_ref(), "/tmp/repo-feature"),
            Some("feature/old".to_string())
        );

        state.repos[0].worktrees =
            Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
                path: PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature/new".to_string()),
                detached: false,
            }]));
        state.repos[0].worktrees_rev = state.repos[0].worktrees_rev.wrapping_add(1);
        state.repos[0].branch_sidebar_rev = state.repos[0].branch_sidebar_rev.wrapping_add(1);

        let refreshed =
            build_sidebar_presentation(&mut cache, &state, &collapsed_items, &BTreeMap::new(), "")
                .expect("refreshed sidebar presentation");
        assert_eq!(
            worktree_branch_for_path(refreshed.rows.as_ref(), "/tmp/repo-feature"),
            Some("feature/new".to_string())
        );
    }

    #[test]
    fn workspace_badge_index_returns_none_for_unknown_branch() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let index = WorkspaceBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("nonexistent").is_none());
        assert!(index.active_path("nonexistent").is_none());
    }

    #[test]
    fn workspace_badge_index_returns_path_for_listed_worktree() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let index = WorkspaceBadgeIndex::for_state(&repo, &[]);

        assert_eq!(
            index.listed_path("feature"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn workspace_badge_index_active_path_returns_none_when_no_open_repo_matches() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let index = WorkspaceBadgeIndex::for_state(&repo, &[]);

        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn workspace_badge_index_active_path_returns_when_open_repo_matches_workdir() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/listed".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let mut open_repo = repo_state(RepoId(2), "/tmp/repo-feature");
        open_repo.head_branch = Loadable::Ready("feature/listed".to_string());
        open_repo.head_branch_rev = 1;

        let index = WorkspaceBadgeIndex::for_state(&repo, &[open_repo]);

        assert_eq!(
            index.active_path("feature/listed"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn workspace_badge_index_built_with_error_worktrees_returns_empty_maps() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Error("failed to load".into());

        let index = WorkspaceBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("feature").is_none());
        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn workspace_badge_index_built_with_loading_worktrees_returns_empty_maps() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Loading;

        let index = WorkspaceBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("feature").is_none());
        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn workspace_badge_index_built_with_not_loaded_worktrees_returns_empty_maps() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::NotLoaded;

        let index = WorkspaceBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("feature").is_none());
        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn build_sidebar_presentation_includes_workspace_badges_when_worktrees_loaded() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;
        repo.branch_sidebar_rev = 1;
        let state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..Default::default()
        };
        let mut cache = SidebarPresentationCache::default();

        let presentation =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("sidebar presentation");

        assert_eq!(
            presentation.workspace_badges.listed_path("feature"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn build_sidebar_presentation_clears_workspace_badges_when_worktrees_become_error() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;
        repo.branch_sidebar_rev = 1;
        let mut state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..Default::default()
        };
        let mut cache = SidebarPresentationCache::default();

        state.repos[0].worktrees = Loadable::Error("failed to load".into());
        state.repos[0].worktrees_rev = state.repos[0].worktrees_rev.wrapping_add(1);
        state.repos[0].branch_sidebar_rev = state.repos[0].branch_sidebar_rev.wrapping_add(1);

        let presentation =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("sidebar presentation");

        assert!(
            presentation
                .workspace_badges
                .listed_path("feature")
                .is_none()
        );
    }

    #[test]
    fn build_sidebar_presentation_updates_workspace_badges_after_worktree_change() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;
        repo.branch_sidebar_rev = 1;
        let mut state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..Default::default()
        };
        let mut cache = SidebarPresentationCache::default();

        let initial =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("initial sidebar presentation");
        assert_eq!(
            initial.workspace_badges.listed_path("feature/old"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );

        state.repos[0].worktrees =
            Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
                path: PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature/new".to_string()),
                detached: false,
            }]));
        state.repos[0].worktrees_rev = state.repos[0].worktrees_rev.wrapping_add(1);
        state.repos[0].branch_sidebar_rev = state.repos[0].branch_sidebar_rev.wrapping_add(1);

        let refreshed =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("refreshed sidebar presentation");

        assert!(
            refreshed
                .workspace_badges
                .listed_path("feature/old")
                .is_none()
        );
        assert_eq!(
            refreshed.workspace_badges.listed_path("feature/new"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn build_sidebar_presentation_removes_badge_when_worktree_becomes_detached() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;
        repo.branch_sidebar_rev = 1;
        let mut state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..Default::default()
        };
        let mut cache = SidebarPresentationCache::default();

        let initial =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("initial sidebar presentation");
        assert!(initial.workspace_badges.listed_path("feature").is_some());

        state.repos[0].worktrees =
            Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
                path: PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: None,
                detached: true,
            }]));
        state.repos[0].worktrees_rev = state.repos[0].worktrees_rev.wrapping_add(1);
        state.repos[0].branch_sidebar_rev = state.repos[0].branch_sidebar_rev.wrapping_add(1);

        let refreshed =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("refreshed sidebar presentation");

        assert!(refreshed.workspace_badges.listed_path("feature").is_none());
    }
}
