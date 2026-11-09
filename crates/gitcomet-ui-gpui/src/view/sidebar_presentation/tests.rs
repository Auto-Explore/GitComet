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
    let first = build_sidebar_presentation(&mut cache, &state, &collapsed, &pins, "feat").unwrap();
    // Empty-section cards are not search results, so leave them out of the
    // comparison between the collapsed tree and the filtered one.
    let listed = |rows: &[BranchSidebarRow]| {
        rows.iter()
            .filter(|row| !matches!(row, BranchSidebarRow::EmptyCard { .. }))
            .count()
    };
    assert!(listed(&first.rows) > listed(&closed.rows));
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
    let restored = build_sidebar_presentation(&mut cache, &state, &collapsed, &pins, "").unwrap();
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
    let expanded = build_sidebar_presentation(&mut cache, &state, &collapsed, &empty, "").unwrap();
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
        !scoped
            .rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "feat/A"))
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
    let expanded = build_sidebar_presentation(&mut cache, &state, &collapsed, &empty, "").unwrap();
    assert!(
        expanded
            .rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "main"))
    );
    assert!(
        !expanded
            .rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::Branch { name, .. } if name == "feat/A"))
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
    let filtered =
        build_sidebar_presentation_scoped(&mut cache, &state, &empty, &pins, "A$", options, None)
            .unwrap();
    assert_eq!(filtered.pins.len(), 3);
    assert_eq!(filtered.match_count, 3);
    let again =
        build_sidebar_presentation_scoped(&mut cache, &state, &empty, &pins, "A$", options, None)
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
        bare: false,
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
        bare: false,
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

    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
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
            bare: false,
            path: PathBuf::from("/tmp/repo"),
            head: None,
            branch: Some("main".to_string()),
            detached: false,
        },
        gitcomet_core::domain::Worktree {
            bare: false,
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
        bare: false,
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
        bare: false,
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
        bare: false,
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
        bare: false,
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
fn bare_mode_changes_invalidate_cached_active_checkout_badges() {
    let mut repo = repo_state(RepoId(1), "/tmp/bare-badge-cache");
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.head_branch = Loadable::Ready("main".into());
    repo.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
        path: repo.spec.workdir.clone(),
        head: None,
        branch: Some("main".into()),
        detached: false,
    }]));
    let mut cache = None;
    assert!(
        worktree_badges_cached(&mut cache, &repo, &[repo.clone()])
            .active_path("main")
            .is_some()
    );
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::Bare);
    assert!(
        worktree_badges_cached(&mut cache, &repo, &[repo.clone()])
            .active_path("main")
            .is_none()
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
        bare: false,
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
        bare: false,
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
        bare: false,
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

    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
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
        bare: false,
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

    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
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
