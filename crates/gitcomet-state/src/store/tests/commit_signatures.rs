use super::*;
use gitcomet_core::domain::{
    CommitDetails, CommitId, CommitSignature, SignatureFormat, SignatureStatus,
};

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
