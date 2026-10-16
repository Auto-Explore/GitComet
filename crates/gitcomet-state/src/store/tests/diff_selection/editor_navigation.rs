use super::*;

#[test]
fn status_actions_retire_only_the_affected_editor_return_diff() {
    use gitcomet_core::domain::DiffArea;
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        for preview in [false, true] {
            for paths in [
                vec![],
                vec![PathBuf::from("shown.rs")],
                vec![PathBuf::from("other.rs")],
            ] {
                for succeeds in [false, true] {
                    let origin = DiffTarget::working_tree("shown.rs".into(), area);
                    let mut state = status_action_state(origin.clone());
                    state.repos[0].diff_state.content_preview = preview;
                    reduce(
                        &mut Default::default(),
                        &AtomicU64::new(3),
                        &mut state,
                        Msg::OpenFileEditor {
                            repo_id: RepoId(1),
                            path: "shown.rs".into(),
                        },
                    );
                    finish_status_action(
                        &mut state,
                        RepoId(1),
                        RepoActionKind::StagePaths,
                        paths.clone(),
                        succeeds,
                    );
                    let retired = succeeds
                        && !preview
                        && area == DiffArea::Unstaged
                        && (paths.is_empty() || paths.contains(&PathBuf::from("shown.rs")));
                    assert!(state.repos[0].diff_state.edit_mode);
                    assert_eq!(
                        state.repos[0].diff_state.edit_return_view.is_none(),
                        retired
                    );
                    reduce(
                        &mut Default::default(),
                        &AtomicU64::new(3),
                        &mut state,
                        Msg::ExitDiffEditMode { repo_id: RepoId(1) },
                    );
                    assert!(!state.repos[0].diff_state.edit_mode);
                    assert_eq!(
                        state.repos[0].diff_state.content_preview,
                        retired || preview
                    );
                    assert_eq!(state.repos[0].diff_state.diff_target, Some(origin));
                }
            }
        }
    }
}

#[test]
fn open_file_content_sets_diff_target_and_content_preview() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let path = PathBuf::from("src/main.rs");
    let source = FileSource::WorkingDirectory;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id: RepoId(1),
            source,
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged))
    );
    assert!(repo_state.diff_state.content_preview);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            ..
        }
    )));

    // Test with Commit source
    let commit_path = PathBuf::from("src/commit.rs");
    let commit_id = CommitId("deadbeef".into());
    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id: RepoId(1),
            source: FileSource::Commit(commit_id.clone()),
            path: commit_path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::commit(commit_id, commit_path))
    );
    assert!(repo_state.diff_state.content_preview);
    assert!(repo_state.diff_state.diff_preview_text_file.is_loading());
    // Opening a file for reading never turns editing on.
    assert!(!repo_state.diff_state.edit_mode);
}

#[test]
fn open_file_editor_targets_the_working_tree_from_any_source() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");

    // Start on a commit's copy of the file...
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(CommitId("deadbeef".into())),
            path: path.clone(),
        },
    );

    // ...and editing still opens the file on disk, not the historical blob.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged))
    );
    assert!(repo_state.diff_state.content_preview);
    assert!(repo_state.diff_state.edit_mode);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            ..
        }
    )));
}

#[test]
fn selecting_another_view_leaves_edit_mode() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.edit_mode);

    // A plain diff selection is not an editing view.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: DiffTarget::working_tree(PathBuf::from("other.rs"), DiffArea::Unstaged),
        },
    );
    assert!(!state.repos[0].diff_state.edit_mode);
    assert!(!state.repos[0].diff_state.content_preview);

    // Neither is a read-only content view.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.edit_mode);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::WorkingDirectory,
            path,
        },
    );
    assert!(!state.repos[0].diff_state.edit_mode);
    assert!(state.repos[0].diff_state.content_preview);
}

#[test]
fn exiting_edit_mode_keeps_the_file_on_screen() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.edit_mode);
    let rev_before = state.repos[0].diff_state.diff_state_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert!(!state.repos[0].diff_state.edit_mode);
    assert!(
        state.repos[0].diff_state.content_preview,
        "leaving the editor lands on the read-only view of the same file"
    );
    assert_ne!(state.repos[0].diff_state.diff_state_rev, rev_before);

    // Idempotent: nothing to leave, nothing to repaint.
    let rev = state.repos[0].diff_state.diff_state_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert_eq!(state.repos[0].diff_state.diff_state_rev, rev);
}

#[test]
fn exiting_edit_mode_restores_the_originating_diff_or_content_preview() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");
    let commit_id = CommitId("deadbeef".into());
    let commit_target = DiffTarget::commit(commit_id.clone(), path.clone());

    // A diff remains a diff, including its historical target, after editing
    // the working-tree copy.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: commit_target.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert_eq!(state.repos[0].diff_state.diff_target, Some(commit_target));
    assert!(!state.repos[0].diff_state.content_preview);
    assert!(!state.repos[0].diff_state.edit_mode);

    // A full-content preview returns to that preview and its original commit,
    // rather than being forced into the working-tree preview.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(commit_id.clone()),
            path: path.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor { repo_id, path },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert_eq!(
        state.repos[0].diff_state.diff_target,
        Some(DiffTarget::commit(commit_id, PathBuf::from("src/main.rs")))
    );
    assert!(state.repos[0].diff_state.content_preview);
    assert!(!state.repos[0].diff_state.edit_mode);
}

#[test]
fn global_nav_realigns_viewer_history_onto_restored_file_view() {
    // Regression: a global (mouse) navigation that lands on a file-content view
    // must reposition the in-viewer file-version history (`view_history`) onto
    // the file now shown, so the viewer's prev/next-version buttons step relative
    // to it rather than a stale cursor left behind by earlier file opens.
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // Open three files in the viewer: view_history = [a, b, c], cursor = 2.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::WorkingDirectory,
            path: PathBuf::from("a.rs"),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(CommitId("c1".into())),
            path: PathBuf::from("b.rs"),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(CommitId("c2".into())),
            path: PathBuf::from("c.rs"),
        },
    );
    assert_eq!(state.repos[0].navigation.view_history.cursor, 2);

    // Leave the viewer for a full-tree diff (not a file-content view): the
    // global stack records it, but view_history stops tracking and stays at c.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: DiffTarget::commit_range(
                CommitId("c2".into()),
                Some(CommitId("c3".into())),
                None,
            ),
        },
    );
    assert_eq!(
        state.repos[0].navigation.view_history.cursor, 2,
        "a non-content diff view must not record viewer history"
    );

    // Mouse-back twice: lands on c's content view, then b's content view.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );

    // The viewer-history cursor now points at b (the file shown), not stale c, so
    // "previous version" would step to a and "next version" to c.
    let view_history = &state.repos[0].navigation.view_history;
    assert_eq!(view_history.cursor, 1);
    let current = &view_history.entries[view_history.cursor];
    assert_eq!(current.source, FileSource::Commit(CommitId("c1".into())));
    assert_eq!(current.path, PathBuf::from("b.rs"));
    assert!(view_history.can_back());
    assert!(view_history.can_forward());
}

#[test]
fn global_nav_reloads_commit_details_when_a_stale_load_is_in_flight() {
    // Regression: navigating back to a snapshot whose commit is already selected
    // makes `select_commit` no-op. If commit_details is still `Loading` for a
    // *different* (stale/cancelled) commit, the result will be dropped by the
    // id-guard, so trusting `is_loading()` would leave the details pane stuck
    // forever. global_nav must reload because the details shown are not for this
    // commit.
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let commit_y = CommitId("yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy".into());
    // Two back/forward entries for the SAME commit (two of its files), so
    // stepping back keeps `selected_commit` == Y and `select_commit` no-ops.
    let snap = |path: PathBuf| crate::model::MainViewSnapshot {
        diff_target: Some(DiffTarget::commit(commit_y.clone(), path)),
        edit_mode: false,
        content_preview: false,
        selected_commit: Some(commit_y.clone()),
        range_selection: None,
        worktree_selection: None,
    };
    state.repos[0]
        .navigation
        .main_history
        .record(snap(PathBuf::from("src/main.rs")));
    state.repos[0]
        .navigation
        .main_history
        .record(snap(PathBuf::from("src/lib.rs")));
    // Live view matches the nav tail; commit Y is selected but its details are
    // stuck Loading (the relevant load was cancelled / is for another commit).
    state.repos[0].diff_state.diff_target = Some(DiffTarget::commit(
        commit_y.clone(),
        PathBuf::from("src/lib.rs"),
    ));
    state.repos[0].set_selected_commit(Some(commit_y.clone()));
    state.repos[0].set_commit_details(Loadable::Loading);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadCommitDetails { repo_id: id, commit_id }
                if *id == repo_id && *commit_id == commit_y
        )),
        "stuck-Loading details for the restored commit must be reloaded, not skipped"
    );
    assert!(
        matches!(
            state.repos[0].history_state.commit_details,
            Loadable::NotLoaded
        ),
        "details are reset to NotLoaded before the reload"
    );
}

/// A renamed or copied file's target carries where the commit took it from,
/// so the viewer's history must carry it too: replaying the entry without it
/// loads the old side from the new path and shows a whole-file addition.
#[test]
fn viewer_history_replays_a_renamed_file_with_its_source() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let commit = CommitId("cccccccccccccccccccccccccccccccccccccccc".into());
    let renamed = DiffTarget::commit(commit.clone(), PathBuf::from("new.rs"))
        .with_old_path(Some(PathBuf::from("old.rs")));
    let snap = |target: Option<DiffTarget>| crate::model::MainViewSnapshot {
        diff_target: target,
        edit_mode: false,
        content_preview: true,
        selected_commit: Some(commit.clone()),
        range_selection: None,
        worktree_selection: None,
    };
    // The file was previewed, then the log shown.
    state.repos[0]
        .navigation
        .main_history
        .record(snap(Some(renamed.clone())));
    state.repos[0].navigation.main_history.record(snap(None));
    state.repos[0].set_selected_commit(Some(commit.clone()));

    // Global back lands on the preview and aligns the viewer's history to it.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert_eq!(state.repos[0].navigation.view_history.entries.len(), 1);
    // Another file is opened, then the viewer steps back to the renamed one.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::WorkingDirectory,
            path: PathBuf::from("x.rs"),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ViewerNavBack { repo_id },
    );
    let target = state.repos[0]
        .diff_state
        .diff_target
        .as_ref()
        .expect("the renamed file is shown again");
    assert_eq!(target.file_path(), Some(std::path::Path::new("new.rs")));
    assert_eq!(
        target.old_file_path(),
        Some(std::path::Path::new("old.rs")),
        "the replayed diff must pair the file with its source"
    );
}

/// The comparison view takes precedence over both commit-detail views, so a
/// back/forward step that doesn't carry the comparison would restore a target
/// and selection the pane never gets around to showing. Stepping back out of a
/// comparison must leave it, and stepping forward into one must re-enter it —
/// including re-issuing the file-list load, since leaving dropped the list.
#[test]
fn global_nav_enters_and_leaves_a_range_comparison() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let from = CommitId("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let to = CommitId("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
    let range = crate::model::RangeSelection {
        from: from.clone(),
        to: Some(to.clone()),
        from_label: "base".into(),
        to_label: "tip".into(),
        options: gitcomet_core::services::ComparisonOptions::merge_base().with_untracked(true),
        base: None,
    };

    // Step 0: the history log. Step 1: a comparison (which clears the diff pane).
    state.repos[0]
        .navigation
        .main_history
        .record(crate::model::MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            edit_mode: false,
            selected_commit: None,
            range_selection: None,
            worktree_selection: None,
        });
    state.repos[0]
        .navigation
        .main_history
        .record(crate::model::MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            edit_mode: false,
            selected_commit: None,
            range_selection: Some(range.clone()),
            worktree_selection: None,
        });
    // Live view matches the nav tail so the reduce-wrapper's reconcile no-ops.
    state.repos[0].set_range_selection(Some(range.clone()));

    // Back to the history log: the comparison must dissolve, not linger.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert!(
        state.repos[0].history_state.range_selection.is_none(),
        "stepping back past a comparison must leave it"
    );

    // Forward into it again: restored, and its file list re-requested because
    // leaving dropped it.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavForward { repo_id },
    );
    assert_eq!(
        state.repos[0].history_state.range_selection,
        Some(range),
        "stepping forward into a comparison must restore its endpoints"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRangeFiles { from: f, to: t, .. } if *f == from && *t == Some(to.clone())
        )),
        "the restored comparison must reload its changed-file list"
    );
}

#[test]
fn global_nav_reuses_a_comparison_whose_base_loaded_after_the_snapshot() {
    let mut repos = RepositoryRuntimes::default();
    let ids = AtomicU64::new(2);
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/tmp/repo".into(),
        },
    );
    let range = crate::model::RangeSelection::new(
        CommitId("base".into()),
        None,
        "base".into(),
        "Working tree".into(),
    );
    let snapshot = |path: &str| crate::model::MainViewSnapshot {
        diff_target: Some(DiffTarget::commit_range(
            range.from.clone(),
            None,
            Some(path.into()),
        )),
        content_preview: false,
        edit_mode: false,
        selected_commit: None,
        range_selection: Some(range.clone()),
        worktree_selection: None,
    };
    repo.navigation.main_history.record(snapshot("a.rs"));
    repo.navigation.main_history.record(snapshot("b.rs"));
    repo.set_range_selection(Some(crate::model::RangeSelection {
        base: Some(range.from.clone()),
        ..range
    }));
    repo.set_range_files(Loadable::Ready(Arc::new(Vec::new())));
    let mut state = AppState::test_default();
    state.repos.push(repo);
    let effects = reduce(&mut repos, &ids, &mut state, Msg::GlobalNavBack { repo_id });
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadRangeFiles { .. }))
    );
    assert!(matches!(
        state.repos[0].history_state.range_files,
        Loadable::Ready(_)
    ));
}

#[test]
fn open_file_content_skips_conflict_path_during_browse() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    // Set up a conflict so that normal SelectDiff would enter conflict-loading
    let path = PathBuf::from("conflicted.txt");
    repo_state.staged_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: path.clone(),
        kind: FileStatusKind::Conflicted,
        conflict: Some(FileConflictKind::BothModified),
    }]));
    repo_state.worktree_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: path.clone(),
        kind: FileStatusKind::Conflicted,
        conflict: Some(FileConflictKind::BothModified),
    }]));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id: RepoId(1),
            source: FileSource::WorkingDirectory,
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::working_tree(path, DiffArea::Unstaged))
    );
    assert!(repo_state.diff_state.content_preview);
    // effects should contain LoadSelectedDiff, NOT conflict-loading effects
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
    );
}

#[test]
fn global_nav_steps_into_and_out_of_edit_mode() {
    // Opening the editor on the file already on screen changes nothing the
    // snapshot used to record, so it deduped away and back/forward could not
    // cross the boundary in either direction.
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/lib.rs");

    // Read-only content view, then the editor on the same file, then back out.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: gitcomet_core::domain::FileSource::WorkingDirectory,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.content_preview);
    assert!(!state.repos[0].diff_state.edit_mode);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(
        state.repos[0].diff_state.edit_mode,
        "the editor is open on the file"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert!(!state.repos[0].diff_state.edit_mode);

    // Back returns to the editor rather than skipping over it.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert!(
        state.repos[0].diff_state.edit_mode,
        "Back from the read-only view must land in the editor it was left from"
    );

    // Back again leaves the editor for the read-only view it was entered from.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert!(
        !state.repos[0].diff_state.edit_mode,
        "Back out of the editor must return to the read-only content view"
    );
    assert!(
        state.repos[0].diff_state.content_preview,
        "and that view is the file's content, not a diff"
    );

    // Forward walks the same boundary in the other direction.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavForward { repo_id },
    );
    assert!(
        state.repos[0].diff_state.edit_mode,
        "Forward must step back into the editor"
    );
}

#[test]
fn file_history_actions_resolve_paths_before_opening_content_or_changes() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/file-history-actions"),
        },
    ));
    state.active_repo = Some(repo_id);
    let commit_id = CommitId("abcdef".into());
    let path = PathBuf::from("src/main.rs");
    for content_preview in [true, false] {
        let msg = if content_preview {
            Msg::OpenFileAtCommit {
                repo_id,
                commit_id: commit_id.clone(),
                path: path.clone(),
            }
        } else {
            Msg::ShowFileChangesAtCommit {
                repo_id,
                commit_id: commit_id.clone(),
                path: path.clone(),
            }
        };
        let effects = reduce(&mut repos, &id_alloc, &mut state, msg);
        assert!(
            matches!(effects.as_slice(), [Effect::OpenFileAtCommit { repo_id: id, commit_id: commit, path: file, content_preview: mode }] if *id == repo_id && commit == &commit_id && file == &path && *mode == content_preview)
        );
        assert!(
            state.repos[0].diff_state.diff_target.is_none(),
            "navigation waits for path resolution"
        );
    }
}
