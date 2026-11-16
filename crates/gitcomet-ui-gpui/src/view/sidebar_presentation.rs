use super::branch_sidebar::{self, BranchSidebarRow};
use super::caches::{
    BranchSidebarCache, BranchSidebarFingerprint, branch_sidebar_cache_lookup,
    branch_sidebar_cache_lookup_by_cached_source, branch_sidebar_cache_lookup_by_source,
    branch_sidebar_cache_store,
};
use super::sidebar_search::SidebarSearch;
use super::*;
use gitcomet_core::text_search::TextSearchOptions;
use gitcomet_state::model::SidebarDataRequest;
use rustc_hash::{FxHashMap, FxHasher};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct WorktreeBadgeIndex {
    listed_paths_by_branch: Arc<FxHashMap<String, PathBuf>>,
    active_paths_by_branch: Arc<FxHashMap<String, PathBuf>>,
    active_fingerprint: (usize, u64),
}

impl WorktreeBadgeIndex {
    fn for_state(repo: &RepoState, open_repos: &[RepoState]) -> Self {
        let active_paths = crate::view::rows::active_worktree_paths_by_branch(repo, open_repos);
        let mut badges = active_paths.iter().collect::<Vec<_>>();
        badges.sort_unstable();
        let mut hasher = FxHasher::default();
        badges.len().hash(&mut hasher);
        for (branch, path) in &badges {
            branch.hash(&mut hasher);
            path.hash(&mut hasher);
        }
        let active_fingerprint = (badges.len(), hasher.finish());
        Self {
            listed_paths_by_branch: Arc::new(crate::view::rows::listed_worktree_paths_by_branch(
                repo,
            )),
            active_paths_by_branch: Arc::new(active_paths),
            active_fingerprint,
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
    pub(in crate::view) worktree_badges: WorktreeBadgeIndex,
    pub(in crate::view) pins: Rc<[BranchSidebarRow]>,
    pub(in crate::view) structure: Rc<super::sidebar_sticky::SidebarStructure>,
    pub(in crate::view) search: Rc<SidebarSearch>,
    pub(in crate::view) match_count: usize,
    pub(in crate::view) row_keys: Rc<[SharedString]>,
}

#[derive(Default)]
pub(in crate::view) struct SidebarPresentationCache {
    branch_rows: Option<BranchSidebarCache>,
    worktree_badges: Option<WorktreeBadgeCache>,
    search: Option<Rc<SidebarSearch>>,
    // Pin/collapse mutations explicitly invalidate this view-owned cache.
    // Comparing entire pin sets here would make every scroll frame O(pins).
    projection: Option<SidebarProjection>,
}

struct SidebarProjection {
    tree: Rc<[BranchSidebarRow]>,
    pins: Rc<[BranchSidebarRow]>,
    scope: Option<String>,
    presentation: SidebarPresentation,
}

impl SidebarPresentationCache {
    pub(in crate::view) fn active_worktree_badges_fingerprint(
        &mut self,
        state: &AppState,
    ) -> (usize, u64) {
        let Some(repo) = state
            .repos
            .iter()
            .find(|repo| Some(repo.id) == state.active_repo)
        else {
            return (0, 0);
        };
        worktree_badges_cached(&mut self.worktree_badges, repo, &state.repos).active_fingerprint
    }
}

struct OpenWorktreeSource {
    path: PathBuf,
    repository_mode: Option<gitcomet_core::domain::RepositoryMode>,
    head: Loadable<String>,
    detached: Option<CommitId>,
}

struct WorktreeBadgeCache {
    repo_id: RepoId,
    worktrees: Option<Arc<Vec<gitcomet_core::domain::Worktree>>>,
    open_repos: Vec<OpenWorktreeSource>,
    index: WorktreeBadgeIndex,
}

impl WorktreeBadgeCache {
    fn matches(&self, repo: &RepoState, open_repos: &[RepoState]) -> bool {
        let same_worktrees = match (&self.worktrees, &repo.worktrees) {
            (Some(cached), Loadable::Ready(current)) => Arc::ptr_eq(cached, current),
            (None, Loadable::NotLoaded | Loadable::Loading | Loadable::Error(_)) => true,
            _ => false,
        };
        self.repo_id == repo.id
            && same_worktrees
            && self.open_repos.len() == open_repos.len()
            && self
                .open_repos
                .iter()
                .zip(open_repos)
                .all(|(cached, current)| {
                    cached.path == current.spec.workdir
                        && cached.repository_mode == current.repository_mode
                        && cached.head == current.head_branch
                        && cached.detached == current.detached_head_commit
                })
    }
}

fn worktree_badges_cached(
    cache: &mut Option<WorktreeBadgeCache>,
    repo: &RepoState,
    open_repos: &[RepoState],
) -> WorktreeBadgeIndex {
    if let Some(cached) = cache
        .as_ref()
        .filter(|cached| cached.matches(repo, open_repos))
    {
        return cached.index.clone();
    }
    let index = WorktreeBadgeIndex::for_state(repo, open_repos);
    *cache = Some(WorktreeBadgeCache {
        repo_id: repo.id,
        worktrees: match &repo.worktrees {
            Loadable::Ready(worktrees) => Some(Arc::clone(worktrees)),
            _ => None,
        },
        open_repos: open_repos
            .iter()
            .map(|repo| OpenWorktreeSource {
                path: repo.spec.workdir.clone(),
                repository_mode: repo.repository_mode,
                head: repo.head_branch.clone(),
                detached: repo.detached_head_commit.clone(),
            })
            .collect(),
        index: index.clone(),
    });
    index
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct SidebarRequestFingerprint {
    active_repo_id: Option<RepoId>,
    request: Option<SidebarDataRequest>,
}

pub(in crate::view) fn active_sidebar_data_request(
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    expanded_visible: bool,
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
            submodules: expanded_visible
                || !branch_sidebar::is_collapsed(
                    collapsed_items,
                    branch_sidebar::submodules_section_storage_key(),
                ),
            stashes: expanded_visible
                || !branch_sidebar::is_collapsed(
                    collapsed_items,
                    branch_sidebar::stash_section_storage_key(),
                ),
            tags: expanded_visible
                || !branch_sidebar::is_collapsed(
                    collapsed_items,
                    branch_sidebar::tags_section_storage_key(),
                ),
        },
    ))
}

pub(in crate::view) fn sidebar_request_fingerprint(
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    expanded_visible: bool,
) -> SidebarRequestFingerprint {
    let (active_repo_id, request) =
        active_sidebar_data_request(state, collapsed_items_by_repo, expanded_visible)
            .map_or((state.active_repo, None), |(repo_id, request)| {
                (Some(repo_id), Some(request))
            });
    SidebarRequestFingerprint {
        active_repo_id,
        request,
    }
}

#[cfg(test)]
pub(in crate::view) fn build_sidebar_presentation(
    cache: &mut SidebarPresentationCache,
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    pinned_branches_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    branch_filter: &str,
) -> Option<SidebarPresentation> {
    build_sidebar_presentation_scoped(
        cache,
        state,
        collapsed_items_by_repo,
        pinned_branches_by_repo,
        branch_filter,
        TextSearchOptions::default(),
        None,
    )
}

pub(in crate::view) fn build_sidebar_presentation_scoped(
    cache: &mut SidebarPresentationCache,
    state: &AppState,
    collapsed_items_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    pinned_branches_by_repo: &BTreeMap<PathBuf, BTreeSet<String>>,
    branch_filter: &str,
    options: TextSearchOptions,
    scope: Option<&str>,
) -> Option<SidebarPresentation> {
    if !cache.search.as_ref().is_some_and(|search| {
        search.query == branch_filter.trim() && search.matcher.options() == options
    }) {
        let search = Rc::new(SidebarSearch::new(branch_filter, options));
        if cache
            .search
            .as_ref()
            .is_none_or(|old| old.matcher.is_empty() != search.matcher.is_empty())
        {
            cache.branch_rows = None;
        }
        cache.search = Some(search);
    }
    let search = Rc::clone(cache.search.as_ref().unwrap());
    let repo_id = state.active_repo?;
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    let empty = BTreeSet::new();
    let collapsed_items = collapsed_items_by_repo
        .get(&repo.spec.workdir)
        .unwrap_or(&empty);
    let pinned_branches = pinned_branches_by_repo
        .get(&repo.spec.workdir)
        .unwrap_or(&empty);

    // Search temporarily reveals ancestors without changing saved collapse state.
    let collapsed_items = if search.matcher.is_empty() {
        collapsed_items
    } else {
        &empty
    };
    let tree = branch_sidebar_rows_cached(&mut cache.branch_rows, repo, collapsed_items);
    // The base tree already tracks content, including identical repo refreshes.
    // Its identity also tells us whether the unfiltered pinned rows can be reused.
    let raw_pins = cache
        .projection
        .as_ref()
        .filter(|cached| Rc::ptr_eq(&cached.tree, &tree))
        .map(|cached| Rc::clone(&cached.pins))
        .unwrap_or_else(|| {
            branch_sidebar::expanded_pinned_rows(repo, pinned_branches, collapsed_items).into()
        });
    let badges = worktree_badges_cached(&mut cache.worktree_badges, repo, state.repos.as_slice());
    if let Some(cached) = &cache.projection
        && Rc::ptr_eq(&tree, &cached.tree)
        && Rc::ptr_eq(&raw_pins, &cached.pins)
        && Rc::ptr_eq(&search, &cached.presentation.search)
        && cached.scope.as_deref() == scope
    {
        return Some(SidebarPresentation {
            worktree_badges: badges,
            ..cached.presentation.clone()
        });
    }
    let mut pins = search.project(&raw_pins);
    let mut rows = search.project(&tree);
    if let Some(scope) = scope {
        pins.retain(|row| match row {
            BranchSidebarRow::Branch { section, .. }
            | BranchSidebarRow::GroupHeader { section, .. } => match section {
                branch_sidebar::BranchSection::Local => {
                    scope == branch_sidebar::local_section_storage_key()
                }
                branch_sidebar::BranchSection::Remote => {
                    scope == branch_sidebar::remote_section_storage_key()
                }
            },
            _ => false,
        });
        rows = rows
            .iter()
            .position(|row| super::sidebar_sticky::header_key(row).is_some_and(|key| key == scope))
            .map(|start| {
                let end = rows[start + 1..]
                    .iter()
                    .position(|row| {
                        super::sidebar_sticky::header_key(row)
                            .is_some_and(|key| branch_sidebar::is_top_level_collapse_key(key))
                    })
                    .map_or(rows.len(), |end| start + 1 + end);
                rows[start + 1..end].to_vec()
            })
            .unwrap_or_default();
    }
    let pins: Rc<[BranchSidebarRow]> = pins.into();
    let mut combined = pins.to_vec();
    for row in rows {
        // One blank slot before each section. Stuck headers stack over it.
        if scope.is_none()
            && !combined.is_empty()
            && super::sidebar_sticky::header_key(&row)
                .is_some_and(|key| branch_sidebar::is_top_level_collapse_key(key))
        {
            combined.push(BranchSidebarRow::SectionSpacer);
        }
        combined.push(row);
    }
    let rows: Rc<[BranchSidebarRow]> = combined.into();
    let structure = Rc::new(super::sidebar_sticky::SidebarStructure::with_pins(
        &rows,
        pins.len(),
    ));
    let mut pin_root = String::new();
    let row_keys = rows
        .iter()
        .enumerate()
        .map(|(ix, row)| {
            if matches!(row, BranchSidebarRow::SectionSpacer) {
                let next = rows.get(ix + 1).map(super::sidebar_sticky::row_key);
                return format!("gap:{}", next.unwrap_or_default()).into();
            }
            let key = super::sidebar_sticky::row_key(row);
            if ix < pins.len() {
                if matches!(
                    row,
                    BranchSidebarRow::Branch { depth: 0, .. }
                        | BranchSidebarRow::GroupHeader { depth: 0, .. }
                ) {
                    pin_root = key.to_string();
                }
                format!("pin:{}:{pin_root}:{key}", pin_root.len()).into()
            } else {
                format!("tree:{key}").into()
            }
        })
        .collect::<Vec<SharedString>>()
        .into();
    let match_count = rows
        .iter()
        .filter(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch { .. }
                    | BranchSidebarRow::WorktreeItem { .. }
                    | BranchSidebarRow::SubmoduleItem { .. }
                    | BranchSidebarRow::AnnexRepositoryItem { .. }
                    | BranchSidebarRow::StashItem { .. }
                    | BranchSidebarRow::TagItem { .. }
            )
        })
        .count();
    let presentation = SidebarPresentation {
        rows,
        pins,
        structure,
        row_keys,
        search,
        match_count,
        worktree_badges: badges,
    };
    cache.projection = Some(SidebarProjection {
        tree,
        pins: raw_pins,
        scope: scope.map(str::to_owned),
        presentation: presentation.clone(),
    });
    Some(presentation)
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
        branch_sidebar::expanded_sidebar_rows(repo, collapsed_items, "").into();

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
mod tests;
