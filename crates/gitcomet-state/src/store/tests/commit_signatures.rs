use super::*;
use gitcomet_core::domain::{
    CommitDetails, CommitId, CommitSignature, SignatureFormat, SignatureFormats, SignatureStatus,
};
use gitcomet_core::signing_tools::{SigningTool, SigningToolAvailability, SigningToolsState};

fn repo_with_selected_commit(
    commit_id: &CommitId,
) -> (AppState, FxHashMap<RepoId, Arc<dyn GitRepository>>) {
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_selected_commit(Some(commit_id.clone()));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    (state, repos)
}

fn commit_details_for(commit_id: &CommitId) -> CommitDetails {
    CommitDetails {
        id: commit_id.clone(),
        message: "subject".to_string(),
        author_name: "Ada".to_string(),
        author_email: "ada@example.com".to_string(),
        authored_at_unix: 0,
        committed_at: "2026-04-07T12:00:00Z".to_string(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: vec![],
    }
}

fn good_signature() -> CommitSignature {
    CommitSignature {
        status: SignatureStatus::Good,
        format: SignatureFormat::OpenPgp,
        signer: Some(Arc::from("Ada <ada@example.com>")),
        key_id: Some(Arc::from("DEADBEEF")),
    }
}

#[test]
fn loading_commit_details_asks_for_its_signature() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(commit_details_for(&commit_id)),
        }),
    );

    let requested = effects.iter().find_map(|effect| match effect {
        Effect::VerifyCommitSignatures { commit_ids, .. } => Some(commit_ids.clone()),
        _ => None,
    });
    assert_eq!(
        requested.as_deref(),
        Some([commit_id].as_slice()),
        "got {effects:?}"
    );
}

#[test]
fn the_preference_suppresses_verification_entirely() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.git_log_settings.verify_commit_signatures = false;
    let id_alloc = AtomicU64::new(2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(commit_details_for(&commit_id)),
        }),
    );

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::VerifyCommitSignatures { .. })),
        "got {effects:?}"
    );
}

#[test]
fn a_known_verdict_is_not_requested_again() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let repo_state = state.repos.first_mut().expect("repo state");
    repo_state.merge_commit_signatures(vec![(commit_id.clone(), good_signature())]);
    let id_alloc = AtomicU64::new(2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(commit_details_for(&commit_id)),
        }),
    );

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::VerifyCommitSignatures { .. })),
        "got {effects:?}"
    );
}

#[test]
fn verified_signatures_merge_into_state_and_bump_the_rev() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);
    let before = state.repos[0].history_state.commit_signatures_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            epoch: 0,
            repo_id: RepoId(1),
            result: Ok(vec![(commit_id.clone(), good_signature())]),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(
        history.commit_signatures.get(&commit_id).map(|s| s.status),
        Some(SignatureStatus::Good)
    );
    assert_ne!(history.commit_signatures_rev, before);
}

#[test]
fn a_later_batch_adds_to_the_map_rather_than_replacing_it() {
    let first = CommitId("aaaa".into());
    let second = CommitId("bbbb".into());
    let (mut state, mut repos) = repo_with_selected_commit(&first);
    let id_alloc = AtomicU64::new(2);

    for id in [&first, &second] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
                epoch: 0,
                repo_id: RepoId(1),
                result: Ok(vec![(id.clone(), good_signature())]),
            }),
        );
    }

    let signatures = &state.repos[0].history_state.commit_signatures;
    assert!(signatures.contains_key(&first), "first batch was dropped");
    assert!(signatures.contains_key(&second));
}

#[test]
fn a_reply_for_an_unknown_repo_is_dropped() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            epoch: 0,
            repo_id: RepoId(99),
            result: Ok(vec![(commit_id.clone(), good_signature())]),
        }),
    );

    assert!(state.repos[0].history_state.commit_signatures.is_empty());
}

#[test]
fn a_verification_failure_leaves_the_map_untouched() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);
    let before = state.repos[0].history_state.commit_signatures_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            epoch: 0,
            repo_id: RepoId(1),
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("gpg exploded".to_string()),
            )),
        }),
    );

    let history = &state.repos[0].history_state;
    assert!(history.commit_signatures.is_empty());
    assert_eq!(history.commit_signatures_rev, before);
}

fn log_page_with(ids: &[&str]) -> gitcomet_core::domain::LogPage {
    gitcomet_core::domain::LogPage {
        commits: ids
            .iter()
            .map(|id| gitcomet_core::domain::Commit {
                id: CommitId((*id).into()),
                parent_ids: Default::default(),
                summary: Arc::from("subject"),
                author: Arc::from("Ada"),
                time: std::time::UNIX_EPOCH,
            })
            .collect(),
        next_cursor: None,
    }
}

fn set_verification(state: &mut AppState, enabled: bool) -> Msg {
    let _ = state;
    Msg::SetGitLogSettings {
        show_history_tags: true,
        tag_fetch_mode: crate::model::GitLogTagFetchMode::OnRepositoryActivation,
        verify_commit_signatures: enabled,
    }
}

#[test]
fn turning_verification_off_clears_the_badges_immediately() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.repos[0].merge_commit_signatures(vec![(commit_id.clone(), good_signature())]);
    let before = state.repos[0].history_state.commit_signatures_rev;
    let id_alloc = AtomicU64::new(2);

    let msg = set_verification(&mut state, false);
    let effects = reduce(&mut repos, &id_alloc, &mut state, msg);

    let history = &state.repos[0].history_state;
    assert!(
        history.commit_signatures.is_empty(),
        "verdicts must be dropped so the badges clear on the next paint"
    );
    assert_ne!(
        history.commit_signatures_rev, before,
        "the rev has to move or the panes never repaint"
    );
    assert!(effects.is_empty(), "got {effects:?}");
}

#[test]
fn turning_verification_back_on_rechecks_the_loaded_page() {
    let commit_id = CommitId("aaaa".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(&["aaaa", "bbbb"]))));
    state.git_log_settings.verify_commit_signatures = false;
    let id_alloc = AtomicU64::new(2);

    let msg = set_verification(&mut state, true);
    let effects = reduce(&mut repos, &id_alloc, &mut state, msg);

    let requested = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::VerifyCommitSignatures { commit_ids, .. } => Some(commit_ids.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a re-verification effect, got {effects:?}"));
    assert_eq!(
        requested.as_ref(),
        [CommitId("aaaa".into()), CommitId("bbbb".into())].as_slice()
    );
}

#[test]
fn re_enabling_also_covers_a_selected_commit_outside_the_page() {
    let revealed = CommitId("cccc".into());
    let (mut state, mut repos) = repo_with_selected_commit(&revealed);
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(&["aaaa"]))));
    state.git_log_settings.verify_commit_signatures = false;
    let id_alloc = AtomicU64::new(2);

    let msg = set_verification(&mut state, true);
    let effects = reduce(&mut repos, &id_alloc, &mut state, msg);

    let requested = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::VerifyCommitSignatures { commit_ids, .. } => Some(commit_ids.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a re-verification effect, got {effects:?}"));
    assert!(
        requested.contains(&revealed),
        "a revealed commit outside the page still needs its badge, got {requested:?}"
    );
}

#[test]
fn re_applying_the_same_setting_changes_nothing() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.repos[0].merge_commit_signatures(vec![(commit_id.clone(), good_signature())]);
    let before = state.repos[0].history_state.commit_signatures_rev;
    let id_alloc = AtomicU64::new(2);

    // Already enabled by default; re-sending must not clear or re-request.
    let msg = set_verification(&mut state, true);
    let effects = reduce(&mut repos, &id_alloc, &mut state, msg);

    let history = &state.repos[0].history_state;
    assert!(!history.commit_signatures.is_empty());
    assert_eq!(history.commit_signatures_rev, before);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::VerifyCommitSignatures { .. })),
        "got {effects:?}"
    );
}

#[test]
fn a_running_verification_reply_is_discarded_after_disabling() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(commit_details_for(&commit_id)),
        }),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::VerifyCommitSignatures { .. }))
    );
    let msg = set_verification(&mut state, false);
    reduce(&mut repos, &id_alloc, &mut state, msg);
    let before = state.repos[0].history_state.commit_signatures_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            epoch: 0,
            repo_id: RepoId(1),
            result: Ok(vec![(commit_id, good_signature())]),
        }),
    );
    assert!(state.repos[0].history_state.commit_signatures.is_empty());
    assert_eq!(state.repos[0].history_state.commit_signatures_rev, before);
}

#[test]
fn a_reply_from_before_disabling_is_discarded_after_re_enabling() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);
    for enabled in [false, true] {
        let msg = set_verification(&mut state, enabled);
        reduce(&mut repos, &id_alloc, &mut state, msg);
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            epoch: 0,
            repo_id: RepoId(1),
            result: Ok(vec![(commit_id, good_signature())]),
        }),
    );
    assert!(
        state.repos[0].history_state.commit_signatures.is_empty(),
        "a batch started before the preference changed is stale even after re-enabling"
    );
}

#[test]
fn a_resolved_reveal_verifies_the_full_commit_outside_the_loaded_page() {
    for enabled in [true, false] {
        let reference = CommitId("dead".into());
        let commit_id = CommitId("deadbeef".into());
        let (mut state, mut repos) = repo_with_selected_commit(&CommitId("aaaa".into()));
        state.git_log_settings.verify_commit_signatures = enabled;
        state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(&["aaaa"]))));
        let id_alloc = AtomicU64::new(2);
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::RevealCommit {
                repo_id: RepoId(1),
                reference: reference.clone(),
            },
        );
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
                repo_id: RepoId(1),
                reference,
                result: Ok(commit_details_for(&commit_id)),
            }),
        );
        assert_eq!(
            state.repos[0].history_state.selected_commit.as_ref(),
            Some(&commit_id)
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadCommitDetails { .. }))
        );
        let requested = effects.iter().find_map(|e| match e {
            Effect::VerifyCommitSignatures { commit_ids, .. } => Some(commit_ids.as_ref()),
            _ => None,
        });
        assert_eq!(
            requested,
            enabled.then_some([commit_id].as_slice()),
            "got {effects:?}"
        );
    }
}

#[test]
fn replacing_history_rechecks_verdicts_and_discards_old_replies() {
    let commit_id = CommitId("aaaa".into());
    let revealed = CommitId("cccc".into());
    let (mut state, mut repos) = repo_with_selected_commit(&revealed);
    let page = Arc::new(log_page_with(&["aaaa", "bbbb"]));
    let repo = &mut state.repos[0];
    repo.set_log(Loadable::Ready(page.clone()));
    repo.merge_commit_signatures(vec![
        (commit_id, good_signature()),
        (revealed.clone(), good_signature()),
    ]);
    let previous_epoch = repo.history_state.commit_signatures_epoch;
    let cancellation = repo.history_state.commit_signatures_cancellation.clone();
    let seq = repo
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope: gitcomet_core::domain::LogScope::AllBranches,
            author: None,
            limit: 200,
            cursor: None,
        })
        .expect("start refresh");
    let result = gitcomet_core::services::HistoryReadResult::Page {
        page,
        snapshot: None,
    };
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: gitcomet_core::domain::LogScope::AllBranches,
            cursor: None,
            result: Ok(result),
        }),
    );
    assert!(
        state.repos[0].history_state.commit_signatures.is_empty(),
        "stale badges must be invalidated"
    );
    assert!(cancellation.is_cancelled());
    let requested = effects
        .iter()
        .find_map(|e| match e {
            Effect::VerifyCommitSignatures { commit_ids, .. } => Some(commit_ids.as_ref()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("refresh must reverify, got {effects:?}"));
    assert_eq!(
        requested,
        [CommitId("aaaa".into()), CommitId("bbbb".into()), revealed].as_slice()
    );
    let epoch = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::VerifyCommitSignatures { epoch, .. } => Some(*epoch),
            _ => None,
        })
        .unwrap();
    assert_ne!(epoch, previous_epoch);
    // A successful refresh can now return no badge; an earlier good verdict
    // must neither survive nor be restored by an older batch finishing last.
    reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch,
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch: previous_epoch,
            result: Ok(vec![(CommitId("aaaa".into()), good_signature())]),
        }),
    );
    assert!(state.repos[0].history_state.commit_signatures.is_empty());
    let mut bad = good_signature();
    bad.status = SignatureStatus::Bad;
    reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch,
            result: Ok(vec![(CommitId("aaaa".into()), bad)]),
        }),
    );
    assert_eq!(
        state.repos[0].history_state.commit_signatures[&CommitId("aaaa".into())].status,
        SignatureStatus::Bad
    );
}

fn assert_history_check_preserves_verification(
    result: Result<gitcomet_core::services::HistoryReadResult>,
) {
    // Complete one batch with a badge and unsigned commits, leave another in
    // flight, and keep a third queued when the history check finishes.
    let unsigned = CommitId("0001".into());
    let (mut state, mut repos) = repo_with_selected_commit(&unsigned);
    let ids: Vec<_> = (0..34).map(|ix| format!("{ix:04x}")).collect();
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    ))));
    state.git_log_settings.verify_commit_signatures = false;
    let id_alloc = AtomicU64::new(2);
    let msg = set_verification(&mut state, true);
    let effects = reduce(&mut repos, &id_alloc, &mut state, msg);
    assert!(matches!(
        effects.as_slice(),
        [Effect::VerifyCommitSignatures { .. }]
    ));
    let epoch = state.repos[0].history_state.commit_signatures_epoch;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch,
            result: Ok(vec![(CommitId("0000".into()), good_signature())]),
        }),
    );
    let [Effect::VerifyCommitSignatures { commit_ids, .. }] = effects.as_slice() else {
        panic!("expected the second batch, got {effects:?}");
    };
    let running_commit = commit_ids[0].clone();
    let repo = &mut state.repos[0];
    let history = &repo.history_state;
    let signatures = history.commit_signatures.clone();
    let rev = history.commit_signatures_rev;
    let cancellation = history.commit_signatures_cancellation.clone();
    let requested = history.commit_signatures_requested.clone();
    let queue = history.commit_signatures_queue.clone();
    assert!(!queue.is_empty());
    let seq = repo
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope: gitcomet_core::domain::LogScope::AllBranches,
            author: None,
            limit: 200,
            cursor: None,
        })
        .expect("start refresh");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: gitcomet_core::domain::LogScope::AllBranches,
            cursor: None,
            result,
        }),
    );
    assert!(effects.is_empty(), "unexpected refresh work: {effects:?}");
    let history = &state.repos[0].history_state;
    assert!(Arc::ptr_eq(&history.commit_signatures, &signatures));
    assert_eq!(history.commit_signatures_rev, rev);
    assert_eq!(history.commit_signatures_epoch, epoch);
    assert!(!cancellation.is_cancelled());
    assert!(history.commit_signatures_in_flight);
    assert_eq!(history.commit_signatures_requested, requested);
    assert_eq!(history.commit_signatures_queue, queue);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch,
            result: Ok(vec![(running_commit.clone(), good_signature())]),
        }),
    );
    assert_eq!(
        state.repos[0]
            .history_state
            .commit_signatures
            .get(&running_commit),
        Some(&good_signature()),
        "the batch started before the history check must still be accepted"
    );
    let [Effect::VerifyCommitSignatures { commit_ids, .. }] = effects.as_slice() else {
        panic!("expected the queued batch to resume, got {effects:?}");
    };
    assert_eq!(commit_ids, &queue[0]);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch,
            result: Ok(Vec::new()),
        }),
    );
    assert!(effects.is_empty());
    assert!(!state.repos[0].history_state.commit_signatures_in_flight);
    let effects = load_selected_details(&mut state, &mut repos, &unsigned);
    assert!(
        effects.is_empty(),
        "unsigned commits must stay cached: {effects:?}"
    );
}

#[test]
fn unchanged_history_preserves_badges_and_running_verification() {
    assert_history_check_preserves_verification(Ok(
        gitcomet_core::services::HistoryReadResult::Unchanged,
    ));
}

#[test]
fn failed_history_refresh_preserves_badges_and_running_verification() {
    assert_history_check_preserves_verification(Err(gitcomet_core::error::Error::new(
        gitcomet_core::error::ErrorKind::Backend("history refresh failed".into()),
    )));
}

#[test]
fn loading_more_history_preserves_badges_and_verifies_only_new_commits() {
    let commit_id = CommitId("aaaa".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let cursor = gitcomet_core::domain::LogCursor {
        last_seen: commit_id.clone(),
        resume_from: None,
        resume_token: None,
    };
    let mut page = log_page_with(&["aaaa"]);
    page.next_cursor = Some(cursor.clone());
    let repo = &mut state.repos[0];
    repo.set_log(Loadable::Ready(Arc::new(page)));
    repo.merge_commit_signatures(vec![(commit_id.clone(), good_signature())]);
    let signatures = repo.history_state.commit_signatures.clone();
    let rev = repo.history_state.commit_signatures_rev;
    let epoch = repo.history_state.commit_signatures_epoch;
    let cancellation = repo.history_state.commit_signatures_cancellation.clone();
    let seq = repo
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope: gitcomet_core::domain::LogScope::AllBranches,
            author: None,
            limit: 200,
            cursor: Some(cursor.clone()),
        })
        .expect("start loading more");
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(2),
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: gitcomet_core::domain::LogScope::AllBranches,
            cursor: Some(cursor),
            result: Ok(Arc::new(log_page_with(&["bbbb", "cccc"])).into()),
        }),
    );
    let [Effect::VerifyCommitSignatures { commit_ids, .. }] = effects.as_slice() else {
        panic!("expected verification of the added page, got {effects:?}");
    };
    assert_eq!(
        commit_ids.as_ref(),
        &[CommitId("bbbb".into()), CommitId("cccc".into())]
    );
    let history = &state.repos[0].history_state;
    assert!(Arc::ptr_eq(&history.commit_signatures, &signatures));
    assert_eq!(history.commit_signatures_rev, rev);
    assert_eq!(history.commit_signatures_epoch, epoch);
    assert!(!cancellation.is_cancelled());
}

#[test]
fn repeated_details_while_verification_is_running_do_not_start_overlapping_batches() {
    let commit_id = CommitId("aaaa".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);
    for attempt in 0..2 {
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
                repo_id: RepoId(1),
                commit_id: commit_id.clone(),
                result: Ok(commit_details_for(&commit_id)),
            }),
        );
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::VerifyCommitSignatures { .. }))
                .count(),
            usize::from(attempt == 0)
        );
    }
}

#[test]
fn a_completed_no_badge_result_is_not_repeated_until_refresh() {
    let commit_id = CommitId("aaaa".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    let id_alloc = AtomicU64::new(2);
    let details = || {
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(commit_details_for(&commit_id)),
        })
    };
    let effects = reduce(&mut repos, &id_alloc, &mut state, details());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::VerifyCommitSignatures { .. }))
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id: RepoId(1),
            epoch: 0,
            result: Ok(Vec::new()),
        }),
    );
    let effects = reduce(&mut repos, &id_alloc, &mut state, details());
    assert!(
        effects.is_empty(),
        "an unsigned or unverifiable commit must be remembered, got {effects:?}"
    );
}

#[test]
fn enabling_verification_starts_only_a_bounded_batch() {
    let selected = CommitId("0000".into());
    let (mut state, mut repos) = repo_with_selected_commit(&selected);
    let ids: Vec<_> = (0..5000).map(|ix| format!("{ix:04x}")).collect();
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    ))));
    state.git_log_settings.verify_commit_signatures = false;
    let msg = set_verification(&mut state, true);
    let effects = reduce(&mut repos, &AtomicU64::new(2), &mut state, msg);
    let batches: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::VerifyCommitSignatures { commit_ids, .. } => Some(commit_ids),
            _ => None,
        })
        .collect();
    assert_eq!(batches.len(), 1);
    assert!(
        batches[0].len() <= 32,
        "verification must not monopolize a worker with the whole history"
    );
}

#[test]
fn signature_batches_drain_without_rechecking_no_badge_commits() {
    let selected = CommitId("0000".into());
    let (mut state, mut repos) = repo_with_selected_commit(&selected);
    let ids: Vec<_> = (0..70).map(|ix| format!("{ix:04x}")).collect();
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    ))));
    state.git_log_settings.verify_commit_signatures = false;
    let id_alloc = AtomicU64::new(2);
    let msg = set_verification(&mut state, true);
    let mut effects = reduce(&mut repos, &id_alloc, &mut state, msg);
    let mut requested = Vec::new();
    while !effects.is_empty() {
        let [
            Effect::VerifyCommitSignatures {
                epoch, commit_ids, ..
            },
        ] = effects.as_slice()
        else {
            panic!("{effects:?}");
        };
        assert!(commit_ids.len() <= 16);
        requested.extend(commit_ids.iter().cloned());
        assert!(requested.len() <= ids.len());
        let epoch = *epoch;
        effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
                repo_id: RepoId(1),
                epoch,
                result: Ok(Vec::new()),
            }),
        );
    }
    assert_eq!(
        requested.iter().map(|id| id.as_ref()).collect::<Vec<_>>(),
        ids.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn leaving_a_revealed_commit_prunes_its_badge_outside_the_log() {
    let first = CommitId("aaaa".into());
    let second = CommitId("bbbb".into());
    let (mut state, _) = repo_with_selected_commit(&first);
    let repo = &mut state.repos[0];
    repo.set_log(Loadable::Ready(Arc::new(log_page_with(&["cccc"]))));
    repo.merge_commit_signatures(vec![(first.clone(), good_signature())]);
    repo.set_selected_commit(Some(second));
    assert!(
        repo.history_state.commit_signatures.is_empty(),
        "an off-page badge must not reserve the author gutter forever"
    );
    repo.merge_commit_signatures(vec![(first, good_signature())]);
    assert!(
        repo.history_state.commit_signatures.is_empty(),
        "a late batch must not restore an off-page badge"
    );
}

#[test]
fn revisiting_a_pruned_reveal_can_verify_its_signature_again() {
    for reply_before_leaving in [false, true] {
        let revealed = CommitId("cccc".into());
        let (mut state, mut repos) = repo_with_selected_commit(&revealed);
        state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(&["aaaa"]))));
        let id_alloc = AtomicU64::new(2);
        let load_details = || {
            Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
                repo_id: RepoId(1),
                commit_id: revealed.clone(),
                result: Ok(commit_details_for(&revealed)),
            })
        };
        let first = reduce(&mut repos, &id_alloc, &mut state, load_details());
        let epoch = match first.as_slice() {
            [Effect::VerifyCommitSignatures { epoch, .. }] => *epoch,
            _ => panic!("expected verification, got {first:?}"),
        };
        if !reply_before_leaving {
            state.repos[0].set_selected_commit(Some(CommitId("aaaa".into())));
        }
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
                repo_id: RepoId(1),
                epoch,
                result: Ok(vec![(revealed.clone(), good_signature())]),
            }),
        );
        if reply_before_leaving {
            state.repos[0].set_selected_commit(Some(CommitId("aaaa".into())));
        }
        assert!(
            !state.repos[0]
                .history_state
                .commit_signatures
                .contains_key(&revealed)
        );
        state.repos[0].set_selected_commit(Some(revealed.clone()));
        let effects = reduce(&mut repos, &id_alloc, &mut state, load_details());
        assert!(
            matches!(effects.as_slice(), [Effect::VerifyCommitSignatures { commit_ids, .. }] if commit_ids.as_ref() == [revealed.clone()]),
            "a pruned badge must be recoverable when revisiting the commit (reply before leaving: {reply_before_leaving}), got {effects:?}"
        );
    }
}

#[test]
fn merging_one_badge_does_not_clone_every_previous_signature() {
    let first = CommitId("0000".into());
    let (mut state, _) = repo_with_selected_commit(&first);
    let signer: Arc<str> = "test signer".into();
    let mut signature = good_signature();
    signature.signer = Some(signer.clone());
    let repo = &mut state.repos[0];
    repo.merge_commit_signatures(
        (0..1024)
            .map(|ix| (CommitId(format!("{ix:04x}").into()), signature.clone()))
            .collect(),
    );
    let previous = repo.history_state.commit_signatures.clone();
    let before = Arc::strong_count(&signer);
    repo.merge_commit_signatures(vec![(CommitId("ffff".into()), signature)]);
    let cloned = Arc::strong_count(&signer) - before;
    assert!(
        cloned < 64,
        "adding one badge cloned {cloned} existing signatures"
    );
    assert_eq!(
        previous.len(),
        1024,
        "published snapshots must remain immutable"
    );
    assert_eq!(repo.history_state.commit_signatures.len(), 1025);
}

#[test]
fn closing_repositories_cancels_their_signature_work() {
    for bulk in [false, true] {
        let (mut state, mut repos) = repo_with_selected_commit(&CommitId("aaaa".into()));
        let cancellation = state.repos[0]
            .history_state
            .commit_signatures_cancellation
            .clone();
        let msg = if bulk {
            Msg::CloseRepos {
                repo_ids: vec![RepoId(1)],
                activate_after: None,
            }
        } else {
            Msg::CloseRepo { repo_id: RepoId(1) }
        };
        reduce(&mut repos, &AtomicU64::new(2), &mut state, msg);
        assert!(cancellation.is_cancelled());
    }
}

fn signing_tools(gpg_found: bool, ssh_keygen_found: bool) -> SigningToolsState {
    let tool = |program: &str, found: bool| SigningTool {
        program: program.to_string(),
        availability: if found {
            SigningToolAvailability::Available {
                version: Some(format!("{program} 1.0")),
            }
        } else {
            SigningToolAvailability::NotFound {
                detail: format!("`{program}` was not found on Git's PATH."),
            }
        },
    };
    SigningToolsState {
        gpg: tool("gpg", gpg_found),
        ssh_keygen: tool("ssh-keygen", ssh_keygen_found),
    }
}

fn requested_formats(effects: &[Effect]) -> Option<SignatureFormats> {
    effects.iter().find_map(|effect| match effect {
        Effect::VerifyCommitSignatures { formats, .. } => Some(*formats),
        _ => None,
    })
}

fn load_selected_details(
    state: &mut AppState,
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    commit_id: &CommitId,
) -> Vec<Effect> {
    let id_alloc = AtomicU64::new(2);
    reduce(
        repos,
        &id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(commit_details_for(commit_id)),
        }),
    )
}

#[test]
fn unprobed_verifiers_do_not_hold_verification_back() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);

    let effects = load_selected_details(&mut state, &mut repos, &commit_id);

    assert_eq!(
        requested_formats(&effects),
        Some(SignatureFormats::ALL),
        "got {effects:?}"
    );
}

#[test]
fn no_installed_verifier_suppresses_verification_entirely() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.signing_tools = signing_tools(false, false);

    let effects = load_selected_details(&mut state, &mut repos, &commit_id);

    assert_eq!(requested_formats(&effects), None, "got {effects:?}");
    assert!(
        state.repos[0]
            .history_state
            .commit_signatures_requested
            .is_empty(),
        "nothing may be queued for a later batch either"
    );
}

#[test]
fn verification_is_limited_to_formats_with_an_installed_verifier() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.signing_tools = signing_tools(false, true);

    let effects = load_selected_details(&mut state, &mut repos, &commit_id);

    let formats = requested_formats(&effects).expect("SSH signatures are still verifiable");
    assert!(formats.contains(SignatureFormat::Ssh));
    assert!(!formats.contains(SignatureFormat::OpenPgp));
    assert!(!formats.contains(SignatureFormat::X509));
}

#[test]
fn losing_every_verifier_clears_the_badges_and_cancels_running_work() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.signing_tools = signing_tools(true, true);
    state.repos[0].merge_commit_signatures(vec![(commit_id.clone(), good_signature())]);
    let history = &state.repos[0].history_state;
    let (rev, epoch) = (
        history.commit_signatures_rev,
        history.commit_signatures_epoch,
    );
    let running = history.commit_signatures_cancellation.clone();
    let id_alloc = AtomicU64::new(2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSigningToolsState(signing_tools(false, false)),
    );

    let history = &state.repos[0].history_state;
    assert!(history.commit_signatures.is_empty());
    assert_ne!(history.commit_signatures_rev, rev);
    assert_ne!(history.commit_signatures_epoch, epoch);
    assert!(running.is_cancelled());
    assert!(effects.is_empty(), "got {effects:?}");
}

#[test]
fn installing_a_verifier_rechecks_the_loaded_page() {
    let commit_id = CommitId("aaaa".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(&["aaaa", "bbbb"]))));
    state.signing_tools = signing_tools(false, false);
    let id_alloc = AtomicU64::new(2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSigningToolsState(signing_tools(true, false)),
    );

    let (commit_ids, formats) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::VerifyCommitSignatures {
                commit_ids,
                formats,
                ..
            } => Some((commit_ids.clone(), *formats)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a re-verification effect, got {effects:?}"));
    assert_eq!(
        commit_ids.as_ref(),
        [CommitId("aaaa".into()), CommitId("bbbb".into())].as_slice()
    );
    assert!(formats.contains(SignatureFormat::OpenPgp));
    assert!(!formats.contains(SignatureFormat::Ssh));
}

#[test]
fn a_new_verifier_version_alone_does_not_recheck() {
    let commit_id = CommitId("deadbeef".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.signing_tools = signing_tools(true, true);
    state.repos[0].merge_commit_signatures(vec![(commit_id.clone(), good_signature())]);
    let mut upgraded = signing_tools(true, true);
    upgraded.gpg.availability = SigningToolAvailability::Available {
        version: Some("gpg 2.0".to_string()),
    };
    let id_alloc = AtomicU64::new(2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSigningToolsState(upgraded.clone()),
    );

    assert!(effects.is_empty(), "got {effects:?}");
    assert_eq!(state.signing_tools, upgraded);
    assert!(
        state.repos[0]
            .history_state
            .commit_signatures
            .contains_key(&commit_id)
    );
}

#[test]
fn enabling_the_preference_without_any_verifier_requests_nothing() {
    let commit_id = CommitId("aaaa".into());
    let (mut state, mut repos) = repo_with_selected_commit(&commit_id);
    state.repos[0].set_log(Loadable::Ready(Arc::new(log_page_with(&["aaaa", "bbbb"]))));
    state.git_log_settings.verify_commit_signatures = false;
    state.signing_tools = signing_tools(false, false);
    let id_alloc = AtomicU64::new(2);

    let msg = set_verification(&mut state, true);
    let effects = reduce(&mut repos, &id_alloc, &mut state, msg);

    assert!(effects.is_empty(), "got {effects:?}");
}
