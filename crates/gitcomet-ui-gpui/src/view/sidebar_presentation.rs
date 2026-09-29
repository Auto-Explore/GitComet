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
                    | BranchSidebarRow::StashItem { .. }
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
mod tests {
    use super::*;

    fn review_cache_fixture() -> (AppState, BTreeMap<PathBuf, BTreeSet<String>>) {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.branches = Loadable::Ready(Arc::new(
            ["feat/A", "feat/B"]
                .map(|name| gitcomet_core::domain::Branch {
                    name: name.into(),
                    target: CommitId("a".into()),
                    upstream: None,
                    divergence: None,
                })
                .to_vec(),
        ));
        let pins = BTreeMap::from([(
            repo.spec.workdir.clone(),
            BTreeSet::from(["group:local:feat".into()]),
        )]);
        (
            AppState {
                active_repo: Some(repo.id),
                repos: vec![repo],
                ..AppState::test_default()
            },
            pins,
        )
    }

    #[test]
    fn review_search_edits_reuse_unfiltered_tree_and_pins() {
        let (state, pins) = review_cache_fixture();
        let collapsed = BTreeMap::from([(
            state.repos[0].spec.workdir.clone(),
            BTreeSet::from(["group:local:feat".into()]),
        )]);
        let mut cache = SidebarPresentationCache::default();
        let closed = build_sidebar_presentation(&mut cache, &state, &collapsed, &pins, "").unwrap();
        let first =
            build_sidebar_presentation(&mut cache, &state, &collapsed, &pins, "feat").unwrap();
        assert!(first.rows.len() > closed.rows.len());
        let tree = cache.projection.as_ref().unwrap().tree.clone();
        let raw_pins = cache.projection.as_ref().unwrap().pins.clone();
        let second =
            build_sidebar_presentation(&mut cache, &state, &collapsed, &pins, "feat/A").unwrap();
        assert!(second.match_count < first.match_count);
        assert!(
            Rc::ptr_eq(&tree, &cache.projection.as_ref().unwrap().tree),
            "query edits rebuilt the branch tree"
        );
        assert!(
            Rc::ptr_eq(&raw_pins, &cache.projection.as_ref().unwrap().pins),
            "query edits rebuilt pinned groups"
        );
        let restored =
            build_sidebar_presentation(&mut cache, &state, &collapsed, &pins, "").unwrap();
        assert_eq!(restored.rows, closed.rows);
    }

    #[test]
    fn review_identical_repo_refresh_reuses_pins_and_row_keys() {
        let (mut state, pins) = review_cache_fixture();
        let empty = BTreeMap::new();
        let mut cache = SidebarPresentationCache::default();
        let first = build_sidebar_presentation(&mut cache, &state, &empty, &pins, "").unwrap();
        let tree = cache.projection.as_ref().unwrap().tree.clone();
        state.repos[0].branches_rev += 1;
        state.repos[0].branch_sidebar_rev += 1;
        if let Loadable::Ready(branches) = &mut state.repos[0].branches {
            *branches = Arc::new(branches.as_ref().clone());
        }
        let second = build_sidebar_presentation(&mut cache, &state, &empty, &pins, "").unwrap();
        assert!(Rc::ptr_eq(&tree, &cache.projection.as_ref().unwrap().tree));
        assert!(
            Rc::ptr_eq(&first.pins, &second.pins),
            "unchanged refresh rebuilt pinned rows"
        );
        assert!(
            Rc::ptr_eq(&first.row_keys, &second.row_keys),
            "unchanged refresh reformatted every row key"
        );
    }

    #[test]
    fn section_gaps_precede_every_section_except_the_first_row() {
        let (state, pins) = review_cache_fixture();
        let empty = BTreeMap::new();
        let gaps = |p: &SidebarPresentation| {
            let gaps: Vec<_> = p
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| matches!(row, BranchSidebarRow::SectionSpacer))
                .map(|(ix, _)| ix)
                .collect();
            let expected: Vec<_> = p
                .structure
                .sections
                .iter()
                .filter(|ix| **ix > 0)
                .map(|ix| ix - 1)
                .collect();
            assert_eq!(gaps, expected);
            assert_eq!(
                p.row_keys.iter().collect::<BTreeSet<_>>().len(),
                p.row_keys.len()
            );
            gaps
        };
        let plain = build_sidebar_presentation(
            &mut SidebarPresentationCache::default(),
            &state,
            &empty,
            &empty,
            "",
        )
        .unwrap();
        assert!(matches!(
            plain.rows[0],
            BranchSidebarRow::SectionHeader { .. }
        ));
        assert!(plain.structure.sections.len() > 1);
        assert_eq!(gaps(&plain).len(), plain.structure.sections.len() - 1);
        assert!(!matches!(
            plain.rows.last(),
            Some(BranchSidebarRow::SectionSpacer)
        ));

        let pinned = build_sidebar_presentation(
            &mut SidebarPresentationCache::default(),
            &state,
            &empty,
            &pins,
            "",
        )
        .unwrap();
        assert!(!pinned.pins.is_empty());
        assert_eq!(pinned.structure.sections[0], pinned.pins.len() + 1);
        assert_eq!(gaps(&pinned).len(), pinned.structure.sections.len());

        let filtered = build_sidebar_presentation(
            &mut SidebarPresentationCache::default(),
            &state,
            &empty,
            &empty,
            "feat/A",
        )
        .unwrap();
        gaps(&filtered);

        let rail = build_sidebar_presentation_scoped(
            &mut SidebarPresentationCache::default(),
            &state,
            &empty,
            &pins,
            "",
            Default::default(),
            Some(branch_sidebar::local_section_storage_key()),
        )
        .unwrap();
        assert!(
            !rail
                .rows
                .iter()
                .any(|row| matches!(row, BranchSidebarRow::SectionSpacer))
        );
    }

    #[test]
    fn review_sidebar_requests_follow_the_visible_top_level_sections() {
        let (state, _) = review_cache_fixture();
        let empty = BTreeMap::new();
        let (_, hidden) = active_sidebar_data_request(&state, &empty, false).unwrap();
        assert!(!hidden.submodules && !hidden.stashes);
        let (_, visible) = active_sidebar_data_request(&state, &empty, true).unwrap();
        let rows = branch_sidebar::expanded_sidebar_rows(&state.repos[0], &BTreeSet::new(), "");
        assert!(rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SubmodulesHeader {
                collapsed: false,
                ..
            }
        )));
        assert!(rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::StashHeader {
                collapsed: false,
                ..
            }
        )));
        assert!(
            visible.submodules && visible.stashes,
            "the expanded sidebar must load its open sections"
        );
    }

    #[test]
    fn scoped_flyout_keeps_nested_folder_state_and_shares_section_rows() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.branches = Loadable::Ready(Arc::new(
            ["main", "feat/A"]
                .map(|name| gitcomet_core::domain::Branch {
                    name: name.into(),
                    target: CommitId("a".into()),
                    upstream: None,
                    divergence: None,
                })
                .to_vec(),
        ));
        let state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..AppState::test_default()
        };
        let scope = branch_sidebar::local_section_storage_key();
        let collapsed = BTreeMap::from([(
            PathBuf::from("/tmp/repo"),
            BTreeSet::from([scope.to_owned(), "group:local:feat".into()]),
        )]);
        let empty = BTreeMap::new();
        let mut cache = SidebarPresentationCache::default();
        let expanded =
            build_sidebar_presentation(&mut cache, &state, &collapsed, &empty, "").unwrap();
        assert!(
            expanded
                .rows
                .iter()
                .any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "main"))
        );
        let scoped = build_sidebar_presentation_scoped(
            &mut cache,
            &state,
            &collapsed,
            &empty,
            "",
            Default::default(),
            Some(scope),
        )
        .unwrap();
        assert!(
            scoped
                .rows
                .iter()
                .any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "main"))
        );
        assert!(scoped.rows.iter().any(|row| matches!(row, BranchSidebarRow::GroupHeader { path, collapsed: true, .. } if path == "feat")));
        assert!(
            !scoped.rows.iter().any(
                |row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "feat/A")
            )
        );
        let cached = build_sidebar_presentation_scoped(
            &mut cache,
            &state,
            &collapsed,
            &empty,
            "",
            Default::default(),
            Some(scope),
        )
        .unwrap();
        assert!(Rc::ptr_eq(&scoped.rows, &cached.rows));
        let expanded =
            build_sidebar_presentation(&mut cache, &state, &collapsed, &empty, "").unwrap();
        assert!(
            expanded
                .rows
                .iter()
                .any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "main"))
        );
        assert!(
            !expanded.rows.iter().any(
                |row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "feat/A")
            )
        );
    }

    #[test]
    fn combined_pins_keep_distinct_identity_and_share_filtered_group_context() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.branches = Loadable::Ready(Arc::new(
            ["feat/A", "feat/B"]
                .map(|name| gitcomet_core::domain::Branch {
                    name: name.into(),
                    target: CommitId("a".into()),
                    upstream: None,
                    divergence: None,
                })
                .to_vec(),
        ));
        let state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..AppState::test_default()
        };
        let pins = BTreeMap::from([(
            PathBuf::from("/tmp/repo"),
            BTreeSet::from(["local:feat/A".into(), "group:local:feat".into()]),
        )]);
        let empty = BTreeMap::new();
        let mut cache = SidebarPresentationCache::default();
        let p = build_sidebar_presentation(&mut cache, &state, &empty, &pins, "").unwrap();
        assert_eq!(p.pins.len(), 4);
        assert_eq!(p.structure.pin_roots.len(), 2);
        let copies: Vec<_> = p
            .rows
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| match row {
                BranchSidebarRow::Branch { name, .. } if name == "feat/A" => {
                    Some(p.row_keys[ix].clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(copies.len(), 3);
        assert_eq!(copies.iter().collect::<BTreeSet<_>>().len(), 3);
        assert!(p.structure.headers["group:local:feat"] >= p.pins.len());
        let options = TextSearchOptions {
            regex: true,
            match_case: true,
            ..Default::default()
        };
        let filtered = build_sidebar_presentation_scoped(
            &mut cache, &state, &empty, &pins, "A$", options, None,
        )
        .unwrap();
        assert_eq!(filtered.pins.len(), 3);
        assert_eq!(filtered.match_count, 3);
        let again = build_sidebar_presentation_scoped(
            &mut cache, &state, &empty, &pins, "A$", options, None,
        )
        .unwrap();
        assert!(Rc::ptr_eq(&filtered.rows, &again.rows));
        assert!(Rc::ptr_eq(&filtered.search, &again.search));
        let closed = BTreeMap::from([(
            PathBuf::from("/tmp/repo"),
            BTreeSet::from(["group:local:feat".into()]),
        )]);
        let closed = build_sidebar_presentation(
            &mut SidebarPresentationCache::default(),
            &state,
            &closed,
            &pins,
            "",
        )
        .unwrap();
        assert_eq!(closed.pins.len(), 2);
        assert_eq!(
            closed.structure.pin_roots.len(),
            2,
            "closed groups remain pinned roots"
        );
    }

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
    fn filtered_rows_and_worktree_badges_reuse_data_until_their_sources_change() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/feature"),
            head: None,
            branch: Some("feature/old".into()),
            detached: false,
        }]));
        let mut open = repo_state(RepoId(2), "/tmp/feature");
        open.head_branch = Loadable::Ready("feature/old".into());
        let mut state = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo, open],
            ..AppState::test_default()
        };
        let empty = BTreeMap::new();
        let mut cache = SidebarPresentationCache::default();
        let initial =
            build_sidebar_presentation(&mut cache, &state, &empty, &empty, "feature").unwrap();
        // Diff and diagnostic updates must not rebuild long sidebar lists or
        // re-index all worktrees, including when a filter is active.
        state.repos[0].diff_state.diff_state_rev += 1;
        let unchanged =
            build_sidebar_presentation(&mut cache, &state, &empty, &empty, "feature").unwrap();
        assert!(Rc::ptr_eq(&initial.rows, &unchanged.rows));
        assert!(Arc::ptr_eq(
            &initial.worktree_badges.active_paths_by_branch,
            &unchanged.worktree_badges.active_paths_by_branch,
        ));
        assert!(Arc::ptr_eq(
            &initial.worktree_badges.listed_paths_by_branch,
            &unchanged.worktree_badges.listed_paths_by_branch,
        ));

        state.repos[1].head_branch = Loadable::Ready("feature/new".into());
        let changed_head =
            build_sidebar_presentation(&mut cache, &state, &empty, &empty, "feature").unwrap();
        assert!(Rc::ptr_eq(&initial.rows, &changed_head.rows));
        assert!(
            changed_head
                .worktree_badges
                .active_path("feature/old")
                .is_none()
        );
        assert_eq!(
            changed_head.worktree_badges.active_path("feature/new"),
            Some(&PathBuf::from("/tmp/feature"))
        );

        state.repos[0].worktrees = Loadable::Ready(Arc::new(Vec::new()));
        state.repos[0].worktrees_rev += 1;
        let refreshed =
            build_sidebar_presentation(&mut cache, &state, &empty, &empty, "feature").unwrap();
        assert!(!Rc::ptr_eq(&initial.rows, &refreshed.rows));
        assert!(
            refreshed
                .worktree_badges
                .listed_path("feature/old")
                .is_none()
        );
        assert!(
            refreshed
                .worktree_badges
                .active_path("feature/new")
                .is_none()
        );

        let different_query =
            build_sidebar_presentation(&mut cache, &state, &empty, &empty, "another").unwrap();
        assert!(!Rc::ptr_eq(&refreshed.rows, &different_query.rows));
        state.active_repo = Some(RepoId(2));
        let different_repo =
            build_sidebar_presentation(&mut cache, &state, &empty, &empty, "another").unwrap();
        assert!(!Rc::ptr_eq(&different_query.rows, &different_repo.rows));
    }

    #[test]
    fn active_sidebar_data_request_always_requests_worktrees() {
        let state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            ..AppState::test_default()
        };

        let (_, request) =
            active_sidebar_data_request(&state, &BTreeMap::new(), false).expect("request exists");

        assert!(request.worktrees);
        assert!(!request.submodules);
        assert!(!request.stashes);
    }

    #[test]
    fn active_sidebar_data_request_respects_repo_collapse_state() {
        let state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            ..AppState::test_default()
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
            active_sidebar_data_request(&state, &collapsed_items, false).expect("request exists");

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
            ..AppState::test_default()
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

    /// A repository's own worktree is the one row the badge index leaves out, so
    /// the index changes meaning when `set_spec` moves `spec.workdir` under it.
    #[test]
    fn worktree_badge_cache_follows_a_repositorys_workdir_moving() {
        let worktrees = Arc::new(vec![
            gitcomet_core::domain::Worktree {
                path: PathBuf::from("/tmp/repo"),
                head: None,
                branch: Some("main".to_string()),
                detached: false,
            },
            gitcomet_core::domain::Worktree {
                path: PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature".to_string()),
                detached: false,
            },
        ]);
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::clone(&worktrees));
        repo.worktrees_rev = 1;
        let mut state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo],
            ..AppState::test_default()
        };
        let mut cache = SidebarPresentationCache::default();

        let badges = worktree_badges_cached(
            &mut cache.worktree_badges,
            &state.repos[0],
            state.repos.as_slice(),
        );
        assert!(badges.listed_path("main").is_none(), "the repo's own row");
        assert_eq!(
            badges.listed_path("feature"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
        let fingerprint = cache.active_worktree_badges_fingerprint(&state);

        // The same worktree list, the same open repositories -- only the
        // repository's own path moved.
        state.repos[0].spec.workdir = PathBuf::from("/tmp/repo-feature");
        assert!(matches!(
            &state.repos[0].worktrees,
            Loadable::Ready(current) if Arc::ptr_eq(current, &worktrees)
        ));

        let badges = worktree_badges_cached(
            &mut cache.worktree_badges,
            &state.repos[0],
            state.repos.as_slice(),
        );
        assert_eq!(
            badges.listed_path("main"),
            Some(&PathBuf::from("/tmp/repo")),
            "the old workdir's row is a listed workspace now"
        );
        assert!(
            badges.listed_path("feature").is_none(),
            "and the new one is the repository itself"
        );
        assert_ne!(
            fingerprint,
            cache.active_worktree_badges_fingerprint(&state),
            "the sidebar has to repaint for it"
        );
    }

    #[test]
    fn worktree_badge_index_returns_none_for_unknown_branch() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let index = WorktreeBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("nonexistent").is_none());
        assert!(index.active_path("nonexistent").is_none());
    }

    #[test]
    fn worktree_badge_index_returns_path_for_listed_worktree() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let index = WorktreeBadgeIndex::for_state(&repo, &[]);

        assert_eq!(
            index.listed_path("feature"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn worktree_badge_index_active_path_returns_none_when_no_open_repo_matches() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        repo.worktrees_rev = 1;

        let index = WorktreeBadgeIndex::for_state(&repo, &[]);

        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn worktree_badge_index_active_path_returns_when_open_repo_matches_workdir() {
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

        let index = WorktreeBadgeIndex::for_state(&repo, &[open_repo]);

        assert_eq!(
            index.active_path("feature/listed"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn worktree_badge_index_built_with_error_worktrees_returns_empty_maps() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Error("failed to load".into());

        let index = WorktreeBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("feature").is_none());
        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn worktree_badge_index_built_with_loading_worktrees_returns_empty_maps() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::Loading;

        let index = WorktreeBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("feature").is_none());
        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn worktree_badge_index_built_with_not_loaded_worktrees_returns_empty_maps() {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.worktrees = Loadable::NotLoaded;

        let index = WorktreeBadgeIndex::for_state(&repo, &[]);

        assert!(index.listed_path("feature").is_none());
        assert!(index.active_path("feature").is_none());
    }

    #[test]
    fn build_sidebar_presentation_includes_worktree_badges_when_worktrees_loaded() {
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
            ..AppState::test_default()
        };
        let mut cache = SidebarPresentationCache::default();

        let presentation =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("sidebar presentation");

        assert_eq!(
            presentation.worktree_badges.listed_path("feature"),
            Some(&PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn build_sidebar_presentation_clears_worktree_badges_when_worktrees_become_error() {
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
            ..AppState::test_default()
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
                .worktree_badges
                .listed_path("feature")
                .is_none()
        );
    }

    #[test]
    fn build_sidebar_presentation_updates_worktree_badges_after_worktree_change() {
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
            ..AppState::test_default()
        };
        let mut cache = SidebarPresentationCache::default();

        let initial =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("initial sidebar presentation");
        assert_eq!(
            initial.worktree_badges.listed_path("feature/old"),
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
                .worktree_badges
                .listed_path("feature/old")
                .is_none()
        );
        assert_eq!(
            refreshed.worktree_badges.listed_path("feature/new"),
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
            ..AppState::test_default()
        };
        let mut cache = SidebarPresentationCache::default();

        let initial =
            build_sidebar_presentation(&mut cache, &state, &BTreeMap::new(), &BTreeMap::new(), "")
                .expect("initial sidebar presentation");
        assert!(initial.worktree_badges.listed_path("feature").is_some());

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

        assert!(refreshed.worktree_badges.listed_path("feature").is_none());
    }
}
