use super::*;
use crate::model::{AppState, Loadable, RepoState};
use crate::msg::{CommitSelectMode, Effect};
use gitcomet_core::domain::{
    Commit, CommitFileChange, CommitId, FileStatusKind, LogPage, RepoSpec,
};
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use std::sync::atomic::AtomicU64;

fn commit(id: &str, parent: &str) -> Commit {
    Commit {
        id: CommitId(id.into()),
        parent_ids: smallvec::smallvec![CommitId(parent.into())],
        summary: id.into(),
        author: "Tester".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

/// A repo whose loaded log is newest-first `c3, c2, c1` (so `c1` is oldest).
/// Every commit has a parent — including the oldest, whose parent `c0` is
/// simply older than the loaded page — so the merged-diff base is a real
/// parent rather than the root-commit fallback.
fn state_with_log(repo_id: RepoId) -> AppState {
    let mut state = AppState {
        git_runtime: GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability: GitExecutableAvailability::Available {
                version_output: "git version 2.0.0".to_string(),
            },
        },
        ..Default::default()
    };
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.history_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![commit("c3", "c2"), commit("c2", "c1"), commit("c1", "c0")],
        next_cursor: None,
    }));
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);
    state
}

fn dispatch_effects(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(99);
    reduce(&mut repos, &id_alloc, state, msg)
}

fn repo(state: &AppState, repo_id: RepoId) -> &RepoState {
    state.repos.iter().find(|r| r.id == repo_id).unwrap()
}

fn select(state: &mut AppState, repo_id: RepoId, id: &str, mode: CommitSelectMode) -> Vec<Effect> {
    dispatch_effects(
        state,
        Msg::SelectCommitMulti {
            request_id: None,
            repo_id,
            commit_id: CommitId(id.into()),
            mode,
            clicked_index: None,
            visible_order: None,
        },
    )
}

#[test]
fn selecting_two_commits_enters_ordered_range_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let c0 = CommitId("c0".into());
    let c3 = CommitId("c3".into());

    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    let effects = select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);

    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("two selected commits should start a comparison");
    // The base is the *parent* of the oldest selected commit, regardless of
    // click order, so the merged diff includes that commit's own changes.
    assert_eq!(range.from, c0);
    assert_eq!(range.to, Some(c3.clone()));

    // The diff pane stays empty: the comparison presents the file
    // side-selection first, and the user opens a file to view its diff.
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
    assert!(matches!(
        repo(&state, repo_id).history_state.range_files,
        Loadable::Loading
    ));
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRangeFiles { from, to, .. } if *from == c0 && *to == Some(c3.clone())
        )),
        "a LoadRangeFiles effect for c0->c3 should be issued"
    );
}

#[test]
fn range_files_loaded_populates_only_the_current_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    let effects = select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    let request = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadRangeFiles { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a range-file load should be issued");

    let files = vec![
        CommitFileChange::new(std::path::PathBuf::from("a.rs"), FileStatusKind::Modified)
            .with_line_counts(Some(1), Some(0)),
    ];

    // A stale result (wrong `from`) is dropped.
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c9".into()),
            to: Some(CommitId("c3".into())),
            request,
            result: Ok(gitcomet_core::services::Comparison::new(
                CommitId("c9".into()),
                files.clone(),
            )),
        }),
    );
    assert!(matches!(
        repo(&state, repo_id).history_state.range_files,
        Loadable::Loading
    ));

    // A reply from an *overtaken* load for the very same endpoints is
    // dropped too. This is the case `(from, to)` cannot catch: a
    // commit↔working-tree comparison keeps its pair across every refresh, so
    // only the request id distinguishes a current reply from a late one.
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c0".into()),
            to: Some(CommitId("c3".into())),
            request: request.wrapping_sub(1),
            result: Ok(gitcomet_core::services::Comparison::new(
                CommitId("c0".into()),
                files.clone(),
            )),
        }),
    );
    assert!(matches!(
        repo(&state, repo_id).history_state.range_files,
        Loadable::Loading
    ));

    // The matching result populates the list.
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c0".into()),
            to: Some(CommitId("c3".into())),
            request,
            result: Ok(gitcomet_core::services::Comparison::new(
                CommitId("c0".into()),
                files.clone(),
            )),
        }),
    );
    match &repo(&state, repo_id).history_state.range_files {
        Loadable::Ready(loaded) => assert_eq!(loaded.as_ref(), &files),
        other => panic!("expected loaded range files, got {other:?}"),
    }
}

#[test]
fn single_selection_clears_an_active_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );

    select(&mut state, repo_id, "c2", CommitSelectMode::Single);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_none(),
        "collapsing to a single commit ends the comparison"
    );
}

fn loaded_details(id: &str) -> gitcomet_core::domain::CommitDetails {
    gitcomet_core::domain::CommitDetails {
        id: CommitId(id.into()),
        message: format!("{id} message"),
        author_name: "Tester".into(),
        author_email: "t@example.com".into(),
        authored_at_unix: 0,
        committed_at: String::new(),
        committed_at_unix: 0,
        parent_ids: Vec::new(),
        files: Vec::new(),
    }
}

/// Entering a comparison moves `selected_commit` to the focused commit
/// without loading its details — the comparison view owns the pane, so that
/// load would be wasted. Leaving the comparison is therefore the moment the
/// details pane has to be put back in sync, or it keeps rendering whichever
/// commit's details were loaded last under a different commit's selection.
#[test]
fn closing_a_comparison_reloads_the_focused_commits_details() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);

    // c3 selected, its details loaded.
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    state.repos[0].history_state.commit_details = Loadable::Ready(Arc::new(loaded_details("c3")));

    // Ctrl-click c1: comparison mode, focus moves to c1, details stay c3's.
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        Some(CommitId("c1".into()))
    );

    let effects = dispatch_effects(&mut state, Msg::ClearComparison { repo_id });

    let r = repo(&state, repo_id);
    assert!(
        !matches!(&r.history_state.commit_details, Loadable::Ready(d) if d.id == CommitId("c3".into())),
        "c3's details must not stay on screen under c1's selection"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadCommitDetails { commit_id, .. } if *commit_id == CommitId("c1".into())
        )),
        "closing the comparison should load the still-selected commit's details"
    );
}

/// Every plain history click leaves a commit in `multi_selection`, so a
/// comparison started from a context menu finds a stale one sitting there.
/// It describes a different comparison, so it must not survive to name this
/// one or supply its preview cards.
#[test]
fn an_explicit_comparison_drops_a_stale_multi_selection() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .multi_selection
            .is_multi(),
        "precondition: a multi-selection comparison is active"
    );

    dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: CommitId("c2".into()),
            from_label: "main".into(),
        },
    );

    let r = repo(&state, repo_id);
    assert!(
        r.history_state.multi_selection.commits.is_empty(),
        "the previous selection is not part of this comparison"
    );
    let range = r
        .history_state
        .range_selection
        .clone()
        .expect("the explicit comparison replaces the previous one");
    assert_eq!(range.from, CommitId("c2".into()));
    assert_eq!(range.to, None);
}

/// A multi-selection comparison keeps its selection: there, the selection
/// *is* what is being compared, and the UI names the comparison after it.
#[test]
fn a_multi_selection_comparison_keeps_its_selection() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);

    let r = repo(&state, repo_id);
    assert!(r.history_state.range_selection.is_some());
    assert!(r.history_state.multi_selection.is_multi());
}

#[test]
fn clear_comparison_dismisses_selection_and_diff() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );

    dispatch_effects(&mut state, Msg::ClearComparison { repo_id });
    let r = repo(&state, repo_id);
    assert!(r.history_state.range_selection.is_none());
    assert!(!r.history_state.multi_selection.is_multi());
    assert_eq!(r.diff_state.diff_target, None);
}

#[test]
fn mark_then_compare_with_marked_builds_the_range() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let c1 = CommitId("c1".into());
    let c3 = CommitId("c3".into());

    // Nothing marked yet: comparing is a no-op.
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithMarked {
            repo_id,
            commit_id: c3.clone(),
            label: "c3".into(),
        },
    );
    assert!(effects.is_empty());
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_none()
    );

    // Mark c1 (base), then compare c3 against it.
    dispatch_effects(
        &mut state,
        Msg::MarkForComparison {
            repo_id,
            commit_id: c1.clone(),
            label: "main".into(),
        },
    );
    dispatch_effects(
        &mut state,
        Msg::CompareWithMarked {
            repo_id,
            commit_id: c3.clone(),
            label: "feature".into(),
        },
    );
    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("compare with marked should start a comparison");
    assert_eq!(range.from, c1);
    assert_eq!(range.to, Some(c3.clone()));
    assert_eq!(range.from_label, "main");
    assert_eq!(range.to_label, "feature");
}

#[test]
fn compare_commit_range_message_orders_via_labels() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareCommitRange {
            repo_id,
            from: CommitId("c1".into()),
            to: CommitId("c3".into()),
            from_label: "main".into(),
            to_label: "feature".into(),
        },
    );
    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("explicit compare should set a comparison");
    assert_eq!(range.from_label, "main");
    assert_eq!(range.to_label, "feature");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );
}

#[test]
fn compare_with_working_tree_starts_a_worktree_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let from = CommitId("c2".into());

    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: from.clone(),
            from_label: "main".into(),
        },
    );

    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("compare with working tree should start a comparison");
    assert_eq!(range.from, from);
    // The tip is the working tree, not a commit.
    assert_eq!(range.to, None);
    assert_eq!(range.to_label, "Working tree");
    // A worktree-tip file list load is issued, and the diff pane is cleared.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadRangeFiles { from: f, to: None, .. } if *f == from
    )));
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
}

/// A refresh means two full-tree `git diff` calls, so changes arriving while
/// one is running must fold into it rather than each starting their own —
/// and the fold must still end with a run that sees the final state.
#[test]
fn external_worktree_changes_refresh_a_worktree_comparison_one_at_a_time() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let from = CommitId("c2".into());
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: from.clone(),
            from_label: "main".into(),
        },
    );
    let load_request = |effects: &[Effect]| {
        effects.iter().find_map(|e| match e {
            Effect::LoadRangeFiles {
                from: f,
                to: None,
                request,
                ..
            } if *f == from => Some(*request),
            _ => None,
        })
    };
    let first = load_request(&effects).expect("the comparison issues a file-list load");

    // Two changes land while that load is still running: neither starts its
    // own, they collapse into the one already in flight.
    for _ in 0..2 {
        let effects = dispatch_effects(
            &mut state,
            Msg::RepoExternallyChanged {
                repo_id,
                change: crate::msg::RepoExternalChange::Worktree,
            },
        );
        assert!(
            load_request(&effects).is_none(),
            "a refresh must not stack on top of one already in flight"
        );
    }

    // When it lands, the folded changes are honoured by exactly one re-run,
    // so the list ends up describing the worktree as it is now.
    let effects = dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: from.clone(),
            to: None,
            request: first,
            result: Ok(gitcomet_core::services::Comparison::new(
                from.clone(),
                Vec::new(),
            )),
        }),
    );
    let second = load_request(&effects).expect("the folded refresh runs once the load lands");
    assert_ne!(first, second, "the re-run is a new request, not a replay");

    // Nothing further is queued, so a quiet worktree stops the chain.
    let effects = dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: from.clone(),
            to: None,
            request: second,
            result: Ok(gitcomet_core::services::Comparison::new(
                from.clone(),
                Vec::new(),
            )),
        }),
    );
    assert!(load_request(&effects).is_none());

    // And with nothing in flight, the next change refreshes immediately.
    let effects = dispatch_effects(
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        load_request(&effects).is_some(),
        "expected the worktree comparison file list to refresh"
    );
}

#[test]
fn external_change_does_not_refresh_a_commit_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    // Two-commit (immutable) comparison.
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );

    let effects = dispatch_effects(
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. })),
        "a commit↔commit comparison is immutable and must not refresh"
    );
}
