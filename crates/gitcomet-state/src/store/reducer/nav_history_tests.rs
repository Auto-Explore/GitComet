use super::*;
use crate::model::{AppState, RepoState};
use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, RepoSpec};
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use std::sync::atomic::AtomicU64;

fn available_state_with_repo(repo_id: RepoId) -> AppState {
    let mut state = AppState {
        git_runtime: GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability: GitExecutableAvailability::Available {
                version_output: "git version 2.0.0".to_string(),
            },
        },
        ..Default::default()
    };
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state
}

fn dispatch(state: &mut AppState, msg: Msg) {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(99);
    let _ = reduce(&mut repos, &id_alloc, state, msg);
}

fn repo(state: &AppState, repo_id: RepoId) -> &RepoState {
    state.repos.iter().find(|r| r.id == repo_id).unwrap()
}

#[test]
fn repo_watch_degraded_pushes_warning_notification() {
    let mut state = AppState::default();
    dispatch(
        &mut state,
        Msg::RepoWatchDegraded {
            repo_id: RepoId(1),
            reason: crate::msg::RepoWatchDegradedReason::TooManyFolders { dir_count: 9000 },
        },
    );
    assert_eq!(state.notifications.len(), 1);
    let note = &state.notifications[0];
    assert_eq!(note.kind, crate::model::AppNotificationKind::Warning);
    assert!(
        note.message.contains("9000"),
        "warning should mention the folder count: {}",
        note.message
    );

    // A partial watch failure surfaces a (distinct) warning too — not just the stderr log.
    dispatch(
        &mut state,
        Msg::RepoWatchDegraded {
            repo_id: RepoId(1),
            reason: crate::msg::RepoWatchDegradedReason::WatchLimitReached { unwatched_dirs: 42 },
        },
    );
    assert_eq!(state.notifications.len(), 2);
    let note = &state.notifications[1];
    assert_eq!(note.kind, crate::model::AppNotificationKind::Warning);
    assert!(
        note.message.contains("42"),
        "partial-watch warning should mention the unwatched count: {}",
        note.message
    );
}

/// A linked-worktree row is a third kind of history selection, and selecting
/// one clears the commit selection. Left out of the navigation machinery it
/// read as "the view went back to the log": the entry for the commit the user
/// came from was overwritten in place, so Back skipped it, and no snapshot
/// could reproduce the worktree row on the way forward.
#[test]
fn selecting_a_worktree_row_is_a_navigation_step_of_its_own() {
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);
    let commit = CommitId("abc".into());
    let worktree = std::path::PathBuf::from("/tmp/wt/a");

    dispatch(
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id,
            commit_id: commit.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectWorktreeUncommitted {
            request_id: None,
            repo_id,
            path: worktree.clone(),
        },
    );
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        None,
        "the worktree row displaces the commit selection"
    );

    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit.as_ref(),
        Some(&commit),
        "back must return to the commit the worktree row was selected from"
    );
    assert_eq!(repo(&state, repo_id).history_state.worktree_selection, None);

    dispatch(&mut state, Msg::GlobalNavForward { repo_id });
    assert_eq!(
        repo(&state, repo_id)
            .history_state
            .worktree_selection
            .as_ref(),
        Some(&worktree),
        "forward must reproduce the worktree row, not just clear the commit"
    );
    assert_eq!(repo(&state, repo_id).history_state.selected_commit, None);
}

#[test]
fn opening_a_file_diff_is_recorded_and_back_restores_the_log() {
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);
    let target = DiffTarget::working_tree(std::path::PathBuf::from("a.txt"), DiffArea::Unstaged);

    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: target.clone(),
        },
    );
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, Some(target));
    // Origin (history log) seeded + the diff.
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.entries.len(),
        2
    );

    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    assert_eq!(
        repo(&state, repo_id).diff_state.diff_target,
        None,
        "back closes the file diff and shows the history log"
    );

    dispatch(&mut state, Msg::GlobalNavForward { repo_id });
    assert!(
        repo(&state, repo_id).diff_state.diff_target.is_some(),
        "forward reopens the file diff"
    );
}

#[test]
fn commit_then_file_diffs_are_all_remembered() {
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);
    let commit_a = CommitId("aaa".into());
    let file1 = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("file1.rs"));
    let file2 = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("file2.rs"));

    dispatch(
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id,
            commit_id: commit_a.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file1.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file2.clone(),
        },
    );

    let entries = &repo(&state, repo_id).navigation.main_history.entries;
    assert!(entries.iter().any(|e| e.diff_target == Some(file1.clone())));
    assert!(entries.iter().any(|e| e.diff_target == Some(file2.clone())));

    // Back must step one-by-one: file2 diff -> file1 diff -> commit details
    // (commit selected, no diff) -> history log.
    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    assert_eq!(
        repo(&state, repo_id).diff_state.diff_target,
        Some(file1.clone())
    );

    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(r.diff_state.diff_target, None, "should show commit details");
    assert_eq!(
        r.history_state.selected_commit.as_ref(),
        Some(&commit_a),
        "commit should still be selected at the details step"
    );

    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        None,
        "final back returns to the history log with no commit selected"
    );
}

#[test]
fn view_navigation_messages_push_others_fold_in_place() {
    // User navigations create a new global back/forward step.
    assert!(is_view_navigation(&Msg::SelectDiff {
        repo_id: RepoId(1),
        target: DiffTarget::working_tree(std::path::PathBuf::from("a.txt"), DiffArea::Unstaged),
    }));
    assert!(is_view_navigation(&Msg::SelectCommit {
        request_id: None,
        repo_id: RepoId(1),
        commit_id: CommitId("a".into()),
    }));
    // The file-content viewer's own back/forward does NOT land a global
    // step — it operates on a separate viewer-level stack so it does not
    // pollute the global back/forward history.
    assert!(!is_view_navigation(&Msg::ViewerNavBack {
        repo_id: RepoId(1)
    }));
    // Background / non-navigation messages do not push a step (they are
    // folded into the current entry in place, so they can't pollute history).
    assert!(!is_view_navigation(&Msg::ReportError {
        repo_id: None,
        message: String::new(),
    }));
}

#[test]
fn closure_and_replay_messages_are_not_view_navigations() {
    assert!(!is_view_navigation(&Msg::ClearDiffSelection {
        repo_id: RepoId(1),
    }));
    assert!(!is_view_navigation(&Msg::ClearCommitSelection {
        request_id: None,
        repo_id: RepoId(1),
    }));
    assert!(!is_view_navigation(&Msg::ViewerNavBack {
        repo_id: RepoId(1),
    }));
    assert!(!is_view_navigation(&Msg::ViewerNavForward {
        repo_id: RepoId(1),
    }));
    assert!(!is_view_navigation(&Msg::CloseInlineSubmoduleDiff {
        repo_id: RepoId(1),
    }));
    assert!(is_view_navigation(&Msg::OpenInlineSubmoduleDiff {
        origin: crate::model::ForeignDiffOrigin::Submodule,
        repo_id: RepoId(1),
        submodule_repo_path: std::path::PathBuf::from("/tmp/sub"),
        parent_submodule_path: std::path::PathBuf::from("sub"),
        entries: vec![].into(),
        selected_ix: 0,
    }));
}

#[test]
fn close_inline_submodule_diff_folds_in_place_and_does_not_bloat_nav_history() {
    // Closing a sub-view must fold in-place: if the snapshot after
    // closing matches a previous entry, it should collapse back to that
    // entry rather than pushing a duplicate.
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);

    // Seed: select a working tree diff (entries: [origin, diff], cursor=1).
    let target = DiffTarget::working_tree(std::path::PathBuf::from("a.txt"), DiffArea::Unstaged);
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: target.clone(),
        },
    );
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.entries.len(),
        2
    );
    assert_eq!(repo(&state, repo_id).navigation.main_history.cursor, 1);

    // Open inline submodule diff.
    dispatch(
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id,
            submodule_repo_path: std::path::PathBuf::from("/tmp/repo/vendor/first"),
            parent_submodule_path: std::path::PathBuf::from("vendor/first"),
            entries: vec![].into(),
            selected_ix: 0,
        },
    );

    // Close inline submodule diff — must fold, not push.
    dispatch(&mut state, Msg::CloseInlineSubmoduleDiff { repo_id });
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.entries.len(),
        2,
        "close must not add a new nav entry"
    );
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.cursor,
        1,
        "cursor must not advance past the parent diff"
    );
}

#[test]
fn clearing_diff_folds_in_place_and_single_back_goes_to_commit_details() {
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);
    let commit_a = CommitId("aaa".into());
    let file = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("file1.rs"));

    dispatch(
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id,
            commit_id: commit_a.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file.clone(),
        },
    );
    // User clicks the same committed file again, which dispatches
    // ClearDiffSelection to close the diff view.
    dispatch(&mut state, Msg::ClearDiffSelection { repo_id });

    let entries = &repo(&state, repo_id).navigation.main_history.entries;
    // After folding in-place, no duplicate entry remains—the file
    // diff entry is collapsed back into the commit-details entry.
    assert_eq!(
        entries.len(),
        2,
        "fold-and-collapse must not create a new entry"
    );
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.cursor,
        1,
        "cursor should be back at the commit-details step"
    );

    // One GlobalNavBack from the commit-details view goes to the
    // history log (origin), confirming the stack did not bloat.
    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(r.diff_state.diff_target, None);
    assert_eq!(r.history_state.selected_commit, None);
    assert!(!r.navigation.main_history.can_back());
}

#[test]
fn clearing_diff_without_folding_previous_allows_correct_back() {
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);
    let commit_a = CommitId("aaa".into());
    let commit_b = CommitId("bbb".into());
    let file = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("file1.rs"));

    dispatch(
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id,
            commit_id: commit_a.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file.clone(),
        },
    );
    // Switch to a different commit (no fold-collapse because the
    // new state differs from the previous entry).
    dispatch(
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id,
            commit_id: commit_b.clone(),
        },
    );

    let r = repo(&state, repo_id);
    assert_eq!(
        r.navigation.main_history.entries.len(),
        4,
        "select-commit pushes a new entry when the commit changes"
    );
    assert_eq!(r.navigation.main_history.cursor, 3);
    assert_eq!(r.history_state.selected_commit.as_ref(), Some(&commit_b));

    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(
        r.diff_state.diff_target,
        Some(file),
        "back should reopen the file diff"
    );
    assert_eq!(r.history_state.selected_commit.as_ref(), Some(&commit_a));
}

#[test]
fn browsing_committed_files_within_a_commit_keeps_commit_selected_on_back() {
    let repo_id = RepoId(1);
    let mut state = available_state_with_repo(repo_id);
    let commit_a = CommitId("aaa".into());
    let file_a = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("src/a.rs"));
    let file_b = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("src/b.rs"));
    let file_c = DiffTarget::commit(commit_a.clone(), std::path::PathBuf::from("src/c.rs"));

    dispatch(
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id,
            commit_id: commit_a.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file_a.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file_b.clone(),
        },
    );
    dispatch(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: file_c.clone(),
        },
    );

    let r = repo(&state, repo_id);
    // Origin + commit details + three file diffs = 5 entries.
    assert_eq!(
        r.navigation.main_history.entries.len(),
        5,
        "each file selection must push a distinct history entry"
    );
    assert_eq!(r.navigation.main_history.cursor, 4);
    assert_eq!(r.diff_state.diff_target, Some(file_c.clone()));

    // ── Back 1: file_c → file_b ──
    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(
        r.diff_state.diff_target,
        Some(file_b.clone()),
        "first back must return to the previously viewed file (b)"
    );
    assert_eq!(
        r.history_state.selected_commit.as_ref(),
        Some(&commit_a),
        "commit must remain selected while browsing files"
    );
    assert_eq!(r.navigation.main_history.cursor, 3);

    // ── Back 2: file_b → file_a ──
    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(
        r.diff_state.diff_target,
        Some(file_a.clone()),
        "second back must return to the first opened file (a)"
    );
    assert_eq!(r.history_state.selected_commit.as_ref(), Some(&commit_a));
    assert_eq!(r.navigation.main_history.cursor, 2);

    // ── Back 3: file_a → commit details (no diff, commit still selected) ──
    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(
        r.diff_state.diff_target, None,
        "third back closes the last file diff and shows commit details"
    );
    assert_eq!(
        r.history_state.selected_commit.as_ref(),
        Some(&commit_a),
        "commit must still be selected — back must not deselect the commit"
    );
    assert_eq!(r.navigation.main_history.cursor, 1);

    // ── Back 4: commit details → history log ──
    dispatch(&mut state, Msg::GlobalNavBack { repo_id });
    let r = repo(&state, repo_id);
    assert_eq!(r.diff_state.diff_target, None);
    assert_eq!(
        r.history_state.selected_commit, None,
        "only the fourth back returns to the history log"
    );
    assert_eq!(r.navigation.main_history.cursor, 0);
    assert!(!r.navigation.main_history.can_back());
}
