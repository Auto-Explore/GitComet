use super::*;
use gitcomet_core::domain::CommitDetails;
use gitcomet_state::model::RangeSelection;
use std::path::PathBuf;

fn repo_state(id: RepoId, path: &str) -> RepoState {
    RepoState::new_opening(
        id,
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from(path),
        },
    )
}

#[test]
fn notify_fingerprint_ignores_inactive_repo_revisions() {
    let active = repo_state(RepoId(1), "/tmp/active");
    let inactive = repo_state(RepoId(2), "/tmp/inactive");
    let mut state = AppState {
        repos: vec![active, inactive],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = DetailsPaneView::notify_fingerprint(&state);

    state.repos[1].worktree_status_rev = 1;
    state.repos[1].staged_status_rev = 1;
    state.repos[1].ops_rev = 1;
    state.repos[1].history_state.selected_commit_rev = 1;
    state.repos[1].history_state.commit_details_rev = 1;
    state.repos[1].merge_message_rev = 1;

    assert_eq!(DetailsPaneView::notify_fingerprint(&state), initial);
}

#[test]
fn indexed_regression_details_fingerprint_tracks_comparison_metadata() {
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        repos: vec![repo_state(RepoId(1), "/tmp/indexed-comparison")],
        ..AppState::test_default()
    };
    let (a, b) = (
        CommitId("a".repeat(40).into()),
        CommitId("b".repeat(40).into()),
    );
    let range = RangeSelection::new(a.clone(), Some(b.clone()), "a".into(), "b".into());
    for multi in [false, true] {
        let history = &mut state.repos[0].history_state;
        if multi {
            history.range_selection = None;
            history.multi_selection.commits = Arc::new(vec![a.clone(), b.clone()]);
        } else {
            history.range_selection = Some(range.clone());
        }
        let before = DetailsPaneView::notify_fingerprint(&state);
        state.repos[0].history_state.indexed.rev += 1;
        let after_index = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(before, after_index, "multi={multi}");
        state.repos[0].log_rev += 1;
        assert_ne!(
            after_index,
            DetailsPaneView::notify_fingerprint(&state),
            "multi={multi}"
        );
    }
}

/// History pages load and signatures stream in while the user scrolls the
/// log; outside a comparison the pane shows neither.
#[test]
fn details_fingerprint_ignores_history_it_does_not_show() {
    let shown = CommitId("c".repeat(40).into());
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        repos: vec![repo_state(RepoId(1), "/tmp/details-history")],
        ..AppState::test_default()
    };
    let history = &mut state.repos[0].history_state;
    history.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: shown.clone(),
        message: "shown".into(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: String::new(),
        committed_at_unix: 0,
        parent_ids: Vec::new(),
        files: Vec::new(),
    }));
    let signature = |status| gitcomet_core::domain::CommitSignature {
        status,
        format: gitcomet_core::domain::SignatureFormat::OpenPgp,
        signer: None,
        key_id: None,
    };
    let before = DetailsPaneView::notify_fingerprint(&state);

    let history = &mut state.repos[0].history_state;
    history.indexed.rev += 1;
    history.commit_signatures_rev += 1;
    history.commit_signatures = Arc::new(
        [(
            CommitId("d".repeat(40).into()),
            signature(gitcomet_core::domain::SignatureStatus::Good),
        )]
        .into_iter()
        .collect(),
    );
    state.repos[0].log_rev += 1;
    assert_eq!(DetailsPaneView::notify_fingerprint(&state), before);

    state.repos[0].history_state.commit_signatures_rev += 1;
    state.repos[0].history_state.commit_signatures = Arc::new(
        [(
            shown,
            signature(gitcomet_core::domain::SignatureStatus::Bad),
        )]
        .into_iter()
        .collect(),
    );
    assert_ne!(DetailsPaneView::notify_fingerprint(&state), before);
}

#[test]
fn notify_fingerprint_tracks_active_repo_relevant_revisions() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = DetailsPaneView::notify_fingerprint(&state);

    state.repos[0].worktree_status_rev = 1;
    let after_status = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_status, initial);

    state.repos[0].ops_rev = 1;
    let after_ops = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_ops, after_status);

    state.repos[0].history_state.selected_commit_rev = 1;
    let after_selected = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_selected, after_ops);

    state.repos[0].history_state.commit_details_rev = 1;
    let after_details = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_details, after_selected);

    state.repos[0].history_state.range_files_rev = 1;
    let after_range_files = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_range_files, after_details);

    state.repos[0].merge_message_rev = 1;
    let after_merge = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_merge, after_range_files);

    state.repos[0].suggested_commit_message_rev = 1;
    assert_ne!(DetailsPaneView::notify_fingerprint(&state), after_merge);
}

#[test]
fn notify_fingerprint_tracks_amend_availability_revisions() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = DetailsPaneView::notify_fingerprint(&state);

    state.repos[0].head_branch_rev = 1;
    let after_head_branch = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_head_branch, initial);

    state.repos[0].branches_rev = 1;
    assert_ne!(
        DetailsPaneView::notify_fingerprint(&state),
        after_head_branch
    );
}
