use super::*;
use gitcomet_core::domain::Branch;
use std::path::PathBuf;

/// Three copies of "is the tree filtered" exist: this one, the reducer's
/// `file_browser_is_filtered`, and the renderer's `file_search_matchers`.
/// They have to agree or the reducer freezes toggles the tree still honours.
#[test]
fn file_browser_search_predicate_agrees_with_the_renderers_matchers() {
    let options = TextSearchOptions::default();
    for query in [
        "", " ", "\n", "  \n \t ", "a", " a ", "a\nb", "\na", "#comment",
    ] {
        assert_eq!(
            file_browser_search_is_active(query),
            !file_search_matchers(query, options).is_empty(),
            "predicates disagree for {query:?}"
        );
    }
}

#[test]
fn active_local_branch_target_requires_matching_ready_branch_data() {
    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.head_branch = Loadable::Ready("feature/current".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/current".to_string(),
        target: CommitId("current-tip".into()),
        upstream: None,
        divergence: None,
    }]));

    assert_eq!(
        active_local_branch_target(&repo).map(|(name, tip)| (name, tip.0.as_ref())),
        Some(("feature/current", "current-tip"))
    );

    repo.head_branch = Loadable::Ready("HEAD".to_string());
    assert_eq!(active_local_branch_target(&repo), None);

    repo.head_branch = Loadable::Ready("missing".to_string());
    assert_eq!(active_local_branch_target(&repo), None);
}

#[test]
fn expanding_a_local_branch_path_preserves_unrelated_collapsed_groups() {
    let local_section = branch_sidebar::local_section_storage_key().to_string();
    let feature = branch_sidebar::local_group_storage_key("feature");
    let feature_deep = branch_sidebar::local_group_storage_key("feature/deep");
    let feature_deep_topic = branch_sidebar::local_group_storage_key("feature/deep/topic");
    let unrelated = branch_sidebar::local_group_storage_key("release");
    let mut collapsed = BTreeSet::from([
        local_section.clone(),
        feature.clone(),
        feature_deep.clone(),
        feature_deep_topic.clone(),
        unrelated.clone(),
    ]);

    assert!(expand_local_branch_path(
        &mut collapsed,
        "feature/deep/topic"
    ));
    for expanded in [local_section, feature, feature_deep, feature_deep_topic] {
        assert!(
            !collapsed.contains(&expanded),
            "{expanded} stayed collapsed"
        );
    }
    assert!(collapsed.contains(&unrelated));
    assert!(!expand_local_branch_path(
        &mut collapsed,
        "feature/deep/topic"
    ));
}

#[test]
fn selected_file_rows_use_the_branch_selection_foreground() {
    for theme in [AppTheme::gitcomet_light(), AppTheme::gitcomet_dark()] {
        assert_eq!(
            file_browser_row_label_color(theme, true),
            selected_branch_label_color(theme)
        );
        assert_eq!(
            file_browser_row_label_color(theme, false),
            theme.colors.foreground.primary
        );
    }
}

fn repo_state(id: RepoId, path: &str) -> RepoState {
    RepoState::new_opening(
        id,
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from(path),
        },
    )
}

#[test]
fn sidebar_repaints_when_either_status_lane_changes() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        sidebar_mode: SidebarMode::Files,
        ..AppState::test_default()
    };
    let initial = SidebarNotifyFingerprint::from_state(&state);
    state.repos[0].worktree_status_rev += 1;
    let worktree = SidebarNotifyFingerprint::from_state(&state);
    assert_ne!(initial, worktree);
    state.repos[0].staged_status_rev += 1;
    assert_ne!(worktree, SidebarNotifyFingerprint::from_state(&state));
}

#[test]
fn sidebar_notify_fingerprint_tracks_open_repo_workdirs() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos.push(repo_state(RepoId(2), "/tmp/repo-wt"));

    assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn sidebar_notify_fingerprint_tracks_sidebar_mode() {
    let mut state = AppState::test_default();
    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.sidebar_mode = SidebarMode::Files;

    assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn bare_mode_change_repaints_the_explorer_without_a_directory_reload() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/bare-explorer-fingerprint")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    let normal = SidebarNotifyFingerprint::from_state(&state);
    state.repos[0].repository_mode = Some(gitcomet_core::domain::RepositoryMode::Bare);
    assert_ne!(normal, SidebarNotifyFingerprint::from_state(&state));
}

#[test]
fn sidebar_notify_fingerprint_tracks_live_worktree_badge_branch_changes() {
    let mut active = repo_state(RepoId(1), "/tmp/repo");
    active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
        path: PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature/old".to_string()),
        detached: false,
    }]));

    let mut worktree_repo = repo_state(RepoId(2), "/tmp/repo-feature");
    worktree_repo.head_branch = Loadable::Ready("feature/old".to_string());
    let mut state = AppState {
        repos: vec![active, worktree_repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos[1].head_branch = Loadable::Ready("feature/new".to_string());
    state.repos[1].head_branch_rev = 1;

    assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn sidebar_notify_fingerprint_tracks_worktree_badge_removal_when_tab_closes() {
    let mut active = repo_state(RepoId(1), "/tmp/repo");
    active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
        path: PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature".to_string()),
        detached: false,
    }]));

    let mut worktree_repo = repo_state(RepoId(2), "/tmp/repo-feature");
    worktree_repo.head_branch = Loadable::Ready("feature".to_string());
    let mut state = AppState {
        repos: vec![active, worktree_repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos.pop();

    assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn sidebar_notify_fingerprint_tracks_worktree_badge_removal_when_worktree_detaches() {
    let mut active = repo_state(RepoId(1), "/tmp/repo");
    active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
        path: PathBuf::from("/tmp/repo-feature"),
        head: None,
        branch: Some("feature".to_string()),
        detached: false,
    }]));

    let mut worktree_repo = repo_state(RepoId(2), "/tmp/repo-feature");
    worktree_repo.head_branch = Loadable::Ready("feature".to_string());
    let mut state = AppState {
        repos: vec![active, worktree_repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos[1].head_branch = Loadable::Ready("HEAD".to_string());
    state.repos[1].head_branch_rev = 1;
    state.repos[1].detached_head_commit = Some(CommitId("deadbeef".into()));

    assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn sidebar_notify_fingerprint_ignores_repo_tab_order() {
    let state_a = AppState {
        repos: vec![
            repo_state(RepoId(1), "/tmp/repo"),
            repo_state(RepoId(2), "/tmp/repo-wt"),
        ],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let state_b = AppState {
        repos: vec![
            repo_state(RepoId(2), "/tmp/repo-wt"),
            repo_state(RepoId(1), "/tmp/repo"),
        ],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    assert_eq!(
        SidebarNotifyFingerprint::from_state(&state_a),
        SidebarNotifyFingerprint::from_state(&state_b)
    );
}

#[test]
fn toggling_default_closed_sections_persists_expanded_overrides() {
    let mut collapsed_items = BTreeSet::new();

    branch_sidebar::toggle_collapse_state(
        &mut collapsed_items,
        branch_sidebar::worktrees_section_storage_key(),
    );

    assert!(
        !branch_sidebar::is_collapsed(
            &collapsed_items,
            branch_sidebar::worktrees_section_storage_key(),
        ),
        "opening a default-closed section should persist an expanded override"
    );
    assert_eq!(
        collapsed_items,
        BTreeSet::from([branch_sidebar::expanded_default_section_storage_key(
            branch_sidebar::worktrees_section_storage_key(),
        )
        .expect("worktrees should support explicit expansion")])
    );

    branch_sidebar::toggle_collapse_state(
        &mut collapsed_items,
        branch_sidebar::worktrees_section_storage_key(),
    );

    assert!(
        branch_sidebar::is_collapsed(
            &collapsed_items,
            branch_sidebar::worktrees_section_storage_key(),
        ),
        "closing a default-closed section should drop the override"
    );
    assert!(collapsed_items.is_empty());
}

#[test]
fn sidebar_notify_fingerprint_ignores_inactive_repo_changes() {
    let active = repo_state(RepoId(1), "/tmp/active");
    let inactive = repo_state(RepoId(2), "/tmp/inactive");
    let mut state = AppState {
        repos: vec![active, inactive],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos[1].head_branch_rev = 1;
    state.repos[1].branches_rev = 1;
    state.repos[1].remote_branches_rev = 1;
    state.repos[1].worktrees_rev = 1;
    state.repos[1].submodules_rev = 1;
    state.repos[1].stashes_rev = 1;
    state.repos[1].branch_sidebar_rev = 1;

    assert_eq!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn sidebar_notify_fingerprint_ignores_unrelated_open_repo_branch_changes() {
    let mut active = repo_state(RepoId(1), "/tmp/active");
    active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
        bare: false,
        path: PathBuf::from("/tmp/active-feature"),
        head: None,
        branch: Some("feature".to_string()),
        detached: false,
    }]));
    let related = repo_state(RepoId(2), "/tmp/active-feature");
    let unrelated = repo_state(RepoId(3), "/tmp/unrelated");
    let mut state = AppState {
        repos: vec![active, related, unrelated],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos[2].head_branch = Loadable::Ready("other".to_string());
    state.repos[2].head_branch_rev = 1;

    assert_eq!(SidebarNotifyFingerprint::from_state(&state), initial);
}

#[test]
fn sidebar_notify_fingerprint_tracks_active_repo_branch_sidebar_changes() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos[0].head_branch_rev = 1;
    let after_head = SidebarNotifyFingerprint::from_state(&state);
    assert_ne!(after_head, initial);

    state.repos[0].branches_rev = 1;
    let after_branches = SidebarNotifyFingerprint::from_state(&state);
    assert_ne!(after_branches, after_head);

    state.repos[0].branch_sidebar_rev = 42;
    assert_ne!(SidebarNotifyFingerprint::from_state(&state), after_branches);
}

#[test]
fn sidebar_notify_fingerprint_tracks_file_browser_rev() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    state.repos[0].file_browser.file_browser_rev = 1;
    let after_bump = SidebarNotifyFingerprint::from_state(&state);
    assert_ne!(after_bump, initial);

    state.repos[0].file_browser.file_browser_rev = 99;
    assert_ne!(SidebarNotifyFingerprint::from_state(&state), after_bump);
}

#[test]
fn sidebar_notify_fingerprint_ignores_inactive_file_browser_rev() {
    let mut state = AppState {
        repos: vec![
            repo_state(RepoId(1), "/tmp/active"),
            repo_state(RepoId(2), "/tmp/inactive"),
        ],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = SidebarNotifyFingerprint::from_state(&state);

    // Only change the INACTIVE repo's file_browser_rev
    state.repos[1].file_browser.file_browser_rev = 42;
    assert_eq!(SidebarNotifyFingerprint::from_state(&state), initial);
}

fn repo_with_local_and_remote_branches() -> RepoState {
    let mut repo = repo_state(RepoId(1), "/tmp/repo");
    repo.branches = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::Branch {
            name: "feature/alpha".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        gitcomet_core::domain::Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::RemoteBranch {
            remote: "origin".to_string(),
            name: "feature/beta".to_string(),
            target: CommitId("deadbeef".into()),
        },
        gitcomet_core::domain::RemoteBranch {
            remote: "origin".to_string(),
            name: "release".to_string(),
            target: CommitId("deadbeef".into()),
        },
    ]));
    repo
}

fn filter_rows(query: &str, section: CollapsedSidebarSection) -> Vec<BranchSidebarRow> {
    let repo = repo_with_local_and_remote_branches();
    let state = AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..AppState::test_default()
    };
    sidebar_presentation::build_sidebar_presentation_scoped(
        &mut SidebarPresentationCache::default(),
        &state,
        &BTreeMap::new(),
        &BTreeMap::new(),
        query,
        Default::default(),
        section.storage_key(),
    )
    .unwrap()
    .rows
    .to_vec()
}

fn branch_names(rows: &[BranchSidebarRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn collapsed_popover_search_stays_within_the_open_section() {
    assert_eq!(
        branch_names(&filter_rows("feature", CollapsedSidebarSection::Local)),
        ["feature/alpha"]
    );
    assert_eq!(
        branch_names(&filter_rows("feature", CollapsedSidebarSection::Remote)),
        ["origin/feature/beta"]
    );
    assert_eq!(
        branch_names(&filter_rows("main", CollapsedSidebarSection::Local)),
        ["main"]
    );
    assert!(branch_names(&filter_rows("release", CollapsedSidebarSection::Local)).is_empty());
}

/// The accent row colours, the eye column and the "Focusing x/y" banner are
/// computed while rendering, not stored in the cached rows, so without this
/// the sidebar keeps painting the pre-focus rows until something else happens
/// to repaint it.
#[test]
fn sidebar_notify_fingerprint_tracks_the_history_focus() {
    use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};

    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::default()
    };
    let unfocused = SidebarNotifyFingerprint::from_state(&state);

    state.repos[0].history_state.history_focus =
        HistoryFocusSet::from_iter([HistoryFocus::local_branch("feature")]);
    let focused = SidebarNotifyFingerprint::from_state(&state);
    assert_ne!(
        focused, unfocused,
        "focusing a ref must repaint the sidebar"
    );

    // Adding a second ref is also a repaint: the banner's count changes.
    state.repos[0].history_state.history_focus = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("feature"),
        HistoryFocus::local_branch("main"),
    ]);
    assert_ne!(SidebarNotifyFingerprint::from_state(&state), focused);

    state.repos[0].history_state.history_focus = HistoryFocusSet::default();
    assert_eq!(
        SidebarNotifyFingerprint::from_state(&state),
        unfocused,
        "clearing the focus returns the sidebar to its unfocused painting"
    );
}
