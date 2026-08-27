use super::*;

#[test]
fn dropped_existing_repo_focuses_its_tab_without_creating_a_duplicate() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(PathBuf::from("/tmp/repo1")),
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        state
            .repos
            .iter()
            .all(|repo| !repo.is_provisional_external_drop_open())
    );
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
}

#[test]
fn diagnostics_are_capped() {
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );

    for i in 0..205 {
        super::reducer::push_diagnostic(&mut repo_state, DiagnosticKind::Error, format!("err-{i}"));
    }

    assert_eq!(repo_state.diagnostics.len(), 200);
    assert_eq!(repo_state.diagnostics[0].message, "err-5");
    assert_eq!(repo_state.diagnostics.last().unwrap().message, "err-204");
}

#[test]
fn session_persist_error_reports_notification_and_repo_diagnostic() {
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    super::reducer::handle_session_persist_result(
        &mut state,
        Some(RepoId(1)),
        "opening a repository",
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )),
    );

    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("Failed to persist session state"))
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("permission denied"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("permission denied"))
    );
}

#[test]
fn session_persist_error_without_repo_still_reports_notification() {
    let mut state = AppState::default();

    super::reducer::handle_session_persist_result(
        &mut state,
        Some(RepoId(999)),
        "closing a repository",
        Err(std::io::Error::other("disk full")),
    );

    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("disk full"))
    );
    assert!(state.repos.is_empty());
}

#[test]
fn session_persist_failed_msg_reports_notification_and_repo_diagnostic() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
            repo_id: Some(RepoId(1)),
            action: "opening a repository",
            error: "disk full".to_string(),
        }),
    );

    assert!(effects.is_empty());
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("Failed to persist session state"))
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("disk full"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("disk full"))
    );
}

#[test]
fn recursive_expand_opens_the_folder_and_every_directory_under_it() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    // The invoked folder itself has to open too, or "Expand all under here"
    // would leave the subtree it just expanded hidden behind a closed row.
    assert!(expanded.contains(&Arc::new(PathBuf::from("src"))));
    assert!(expanded.contains(&Arc::new(PathBuf::from("src/nested"))));
    // Siblings outside the subtree are untouched, and files are not directories.
    assert!(!expanded.contains(&Arc::new(PathBuf::from("other"))));
    assert!(!expanded.contains(&Arc::new(PathBuf::from("src/a.rs"))));
    assert_eq!(expanded.len(), 2);
    assert_ne!(
        state.repos[0].file_browser.file_browser_rev, rev_before,
        "the tree has to repaint after a recursive expand"
    );
}

#[test]
fn recursive_collapse_closes_exactly_the_subtree_it_opened() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    for path in ["other", "src", "src/nested"] {
        state.repos[0]
            .file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from(path)));
    }

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: false,
        },
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    assert_eq!(
        expanded,
        &[Arc::new(PathBuf::from("other"))]
            .into_iter()
            .collect::<FxHashSet<_>>(),
        "collapsing a subtree must not disturb folders outside it"
    );
}

/// `starts_with` on a `Path` compares whole components, so a sibling whose name
/// merely begins with the same characters is a different folder. String-prefix
/// matching here would collapse `src_generated` along with `src`.
#[test]
fn recursive_expand_does_not_match_name_prefixes_of_sibling_folders() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: gitcomet_core::domain::FileEntryKind::Directory,
            depth: 0,
        },
        gitcomet_core::domain::FileEntry {
            name: "src_generated".to_string(),
            path: Arc::new(PathBuf::from("src_generated")),
            kind: gitcomet_core::domain::FileEntryKind::Directory,
            depth: 0,
        },
    ]));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    assert!(expanded.contains(&Arc::new(PathBuf::from("src"))));
    assert!(!expanded.contains(&Arc::new(PathBuf::from("src_generated"))));
}

/// `Path::starts_with("")` is true of every path, so an empty path would sweep
/// the whole tree — a collapse would wipe `expanded_dirs` outright rather than
/// touching one subtree.
#[test]
fn recursive_collapse_of_an_empty_path_leaves_the_tree_alone() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    for path in ["other", "src", "src/nested"] {
        state.repos[0]
            .file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from(path)));
    }
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::new(),
            expanded: false,
        },
    );

    assert_eq!(
        state.repos[0].file_browser.expanded_dirs.len(),
        3,
        "an empty path names no folder, so it may not collapse every folder"
    );
    assert_eq!(state.repos[0].file_browser.file_browser_rev, rev_before);
}

/// A no-op must not bump the rev: the file browser's row cache is keyed on it,
/// so a bump on every right-click would throw away the memoized row list for
/// nothing.
#[test]
fn recursive_expand_of_an_already_expanded_subtree_does_not_bump_the_rev() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    for path in ["src", "src/nested"] {
        state.repos[0]
            .file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from(path)));
    }
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    );

    assert_eq!(state.repos[0].file_browser.file_browser_rev, rev_before);
}

/// A filtered tree force-expands every directory and never reads
/// `expanded_dirs`, so a toggle would move nothing on screen and then reshape
/// the tree the moment the search was cleared.
#[test]
fn folder_toggles_are_frozen_while_a_search_filters_the_tree() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    state.repos[0].file_browser.search_query = "a.rs".to_string();
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    for msg in [
        Msg::ToggleFileBrowserDir {
            repo_id,
            path: PathBuf::from("src"),
        },
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path: PathBuf::from("src"),
            expanded: true,
        },
    ] {
        reduce(&mut repos, &id_alloc, &mut state, msg);
    }

    assert!(
        state.repos[0].file_browser.expanded_dirs.is_empty(),
        "a filtered tree must keep the shape the user left it in"
    );
    assert_eq!(state.repos[0].file_browser.file_browser_rev, rev_before);
}

/// The search input is multiline and stores what was typed verbatim, so a lone
/// space is a non-empty query that filters nothing — the toggles stay live.
#[test]
fn folder_toggles_stay_live_for_a_whitespace_only_query() {
    let (mut repos, id_alloc, mut state, repo_id) = state_with_file_browser_tree();
    state.repos[0].file_browser.search_query = "   \n".to_string();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ToggleFileBrowserDir {
            repo_id,
            path: PathBuf::from("src"),
        },
    );

    assert!(
        state.repos[0]
            .file_browser
            .expanded_dirs
            .contains(&Arc::new(PathBuf::from("src")))
    );
}

#[test]
fn delete_branches_emits_one_effect_carrying_every_name() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);

    let names = vec!["feat/a".to_string(), "feat/b".to_string()];
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteBranches {
            repo_id,
            names: names.clone(),
            force: true,
        },
    );

    let batched = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DeleteBranches {
                repo_id: candidate,
                names,
                force,
            } if *candidate == repo_id => Some((names.clone(), *force)),
            _ => None,
        })
        .expect("expected a batched delete effect");
    // One effect for the whole batch, not one per branch: the scheduler needs
    // the full list to summarise partial failures.
    assert_eq!(batched.0, names);
    assert!(batched.1, "the force choice has to survive to the effect");
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::DeleteBranches { .. }))
            .count(),
        1
    );
}

#[test]
fn delete_branches_with_an_empty_list_does_nothing() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);
    let busy_before = state.repos[0].local_actions_in_flight;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteBranches {
            repo_id,
            names: Vec::new(),
            force: false,
        },
    );

    assert!(effects.is_empty());
    // Crucially it must not mark the repo busy, or the UI would sit disabled
    // waiting on an action that never runs.
    assert_eq!(state.repos[0].local_actions_in_flight, busy_before);
}

#[test]
fn delete_remote_branches_keeps_the_batch_under_one_remote() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo_id = RepoId(1);
    mark_repo_open_ready(&mut repos, &mut state, repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteBranches {
            repo_id,
            remote: "origin".to_string(),
            branches: vec!["feat/a".to_string(), "feat/b".to_string()],
        },
    );

    let batched = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DeleteRemoteBranches {
                remote, branches, ..
            } => Some((remote.clone(), branches.clone())),
            _ => None,
        })
        .expect("expected a batched remote delete effect");
    assert_eq!(batched.0, "origin");
    assert_eq!(batched.1.len(), 2);
}
