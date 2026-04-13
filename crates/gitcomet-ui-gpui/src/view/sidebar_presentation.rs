use super::branch_sidebar::{self, BranchSidebarRow};
use super::caches::{
    BranchSidebarCache, BranchSidebarFingerprint, branch_sidebar_cache_lookup,
    branch_sidebar_cache_lookup_by_cached_source, branch_sidebar_cache_lookup_by_source,
    branch_sidebar_cache_store,
};
use super::*;
use gitcomet_state::model::SidebarDataRequest;
use rustc_hash::FxHashMap as HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct WorkspaceBadgeIndex {
    listed_paths_by_branch: Arc<HashMap<String, PathBuf>>,
    active_paths_by_branch: Arc<HashMap<String, PathBuf>>,
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
            subtrees: !branch_sidebar::is_collapsed(
                collapsed_items,
                branch_sidebar::subtrees_section_storage_key(),
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
) -> Option<SidebarPresentation> {
    let repo_id = state.active_repo?;
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    let empty = BTreeSet::new();
    let collapsed_items = collapsed_items_by_repo
        .get(&repo.spec.workdir)
        .unwrap_or(&empty);

    Some(SidebarPresentation {
        rows: branch_sidebar_rows_cached(&mut cache.branch_rows, repo, collapsed_items),
        workspace_badges: WorkspaceBadgeIndex::for_state(repo, state.repos.as_slice()),
    })
}

fn branch_sidebar_rows_cached(
    cache: &mut Option<BranchSidebarCache>,
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
) -> Rc<[BranchSidebarRow]> {
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
        branch_sidebar::branch_sidebar_rows(repo, collapsed_items).into();

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
        assert!(!request.subtrees);
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
                branch_sidebar::expanded_default_section_storage_key(
                    branch_sidebar::subtrees_section_storage_key(),
                )
                .expect("subtrees should support explicit expansion"),
            ]),
        )]);

        let (_, request) =
            active_sidebar_data_request(&state, &collapsed_items).expect("request exists");

        assert!(request.worktrees);
        assert!(request.submodules);
        assert!(request.subtrees);
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

        let initial = build_sidebar_presentation(&mut cache, &state, &collapsed_items)
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

        let refreshed = build_sidebar_presentation(&mut cache, &state, &collapsed_items)
            .expect("refreshed sidebar presentation");
        assert_eq!(
            worktree_branch_for_path(refreshed.rows.as_ref(), "/tmp/repo-feature"),
            Some("feature/new".to_string())
        );
    }
}
