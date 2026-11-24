use super::*;
use crate::model::{AutoFetchStatus, RepoState};
use gitcomet_core::domain::RepoSpec;
use gitcomet_core::error::{Error, ErrorKind};
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

fn state() -> AppState {
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    state
}

#[test]
fn automatic_failure_records_a_quiet_result_without_clearing_manual_errors() {
    let mut state = state();
    let repo = &mut state.repos[0];
    let lifetime = repo.lifetime_id();
    repo.feedback_mut().last_error = Some("manual push rejected".into());
    let previous = UNIX_EPOCH + Duration::from_secs(10);
    let at = previous + Duration::from_secs(1800);
    let error = Error::new(ErrorKind::Unsupported("test offline"));
    let inputs = super::super::test_inputs::clock();
    let effects = updated(
        &inputs,
        &mut state,
        RepoId(1),
        lifetime,
        1,
        RepoAutoFetchState {
            last_success_at: Some(previous),
            status: AutoFetchStatus::Failed {
                at,
                error: error.clone(),
            },
            ..RepoAutoFetchState::default()
        },
        Some(Arc::new(Err(error))),
    );
    assert!(effects.is_empty());
    let repo = &state.repos[0];
    assert_eq!(
        repo.feedback.last_error.as_deref(),
        Some("manual push rejected")
    );
    assert!(repo.feedback.diagnostics.is_empty());
    assert!(repo.feedback.hook_activity.is_empty());
    assert!(state.notifications.is_empty());
    assert!(state.auth_prompt.is_none());
    assert_eq!(repo.pull_in_flight, 0);
    assert_eq!(repo.auto_fetch.last_success_at, Some(previous));
    assert!(matches!(
        repo.auto_fetch.status,
        AutoFetchStatus::Failed { .. }
    ));
    let entry = repo.feedback.command_log.last().unwrap();
    assert_eq!(entry.source, CommandSource::Background);
    assert!(!entry.ok);
    assert!(!entry.announce_success);
}

#[test]
fn stale_automatic_messages_cannot_change_a_reopened_tab_or_newer_success() {
    let mut state = state();
    let lifetime = state.repos[0].lifetime_id();
    let inputs = super::super::test_inputs::clock();
    let success = RepoAutoFetchState {
        last_success_at: Some(UNIX_EPOCH),
        status: AutoFetchStatus::Succeeded { at: UNIX_EPOCH },
        ..RepoAutoFetchState::default()
    };
    updated(&inputs, &mut state, RepoId(1), lifetime, 10, success, None);
    updated(
        &inputs,
        &mut state,
        RepoId(1),
        lifetime,
        9,
        RepoAutoFetchState::default(),
        None,
    );
    updated(
        &inputs,
        &mut state,
        RepoId(1),
        RepoLifetime::allocate(),
        11,
        RepoAutoFetchState::default(),
        None,
    );
    assert!(matches!(
        state.repos[0].auto_fetch.status,
        AutoFetchStatus::Succeeded { .. }
    ));
    assert_eq!(state.repos[0].auto_fetch_revision, 10);
}

#[test]
fn automatic_refresh_keeps_unselected_restored_tabs_lazy() {
    let mut state = state();
    state.repos[0].set_open(Loadable::NotLoaded);
    let lifetime = state.repos[0].lifetime_id();
    let effects = refresh(&mut state, RepoId(1), lifetime);
    assert!(matches!(
        effects.as_slice(),
        [Effect::References(
            ReferenceEffect::LoadUpstreamDivergence { repo_id: RepoId(1) }
        )]
    ));
    assert!(matches!(state.repos[0].open, Loadable::NotLoaded));
    assert!(matches!(
        state.repos[0].history_state.log,
        Loadable::NotLoaded
    ));
    assert!(matches!(state.repos[0].status, Loadable::NotLoaded));
}

#[test]
fn automatic_refresh_keeps_parked_visited_tabs_lazy() {
    let mut state = state();
    let repo = &mut state.repos[0];
    repo.set_open(Loadable::Ready(()));
    repo.backend_parked = true;
    repo.auto_fetch.status = AutoFetchStatus::Succeeded { at: UNIX_EPOCH };
    let lifetime = repo.lifetime_id();
    let effects = refresh(&mut state, RepoId(1), lifetime);
    assert!(matches!(
        effects.as_slice(),
        [Effect::References(
            ReferenceEffect::LoadUpstreamDivergence { repo_id: RepoId(1) }
        )]
    ));
    assert!(state.repos[0].is_backend_parked());
    assert!(matches!(
        state.repos[0].auto_fetch.status,
        AutoFetchStatus::Succeeded { .. }
    ));
}

#[test]
fn automatic_refresh_requests_local_references_without_worktree_or_remote_network_loads() {
    let mut state = state();
    state.repos[0].set_open(Loadable::Ready(()));
    let lifetime = state.repos[0].lifetime_id();
    let effects = refresh(&mut state, RepoId(1), lifetime);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::References(ReferenceEffect::LoadRemoteBranches { .. })
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::References(ReferenceEffect::LoadUpstreamDivergence { .. })
    )));
    assert!(effects.iter().all(|effect| matches!(
        effect,
        Effect::References(
            ReferenceEffect::LoadBranches { .. }
                | ReferenceEffect::LoadRemoteBranches { .. }
                | ReferenceEffect::LoadRemotes { .. }
                | ReferenceEffect::LoadUpstreamDivergence { .. }
        )
    )));
}

#[test]
fn automatic_refresh_failures_are_quiet_and_retain_cached_references_and_history() {
    use gitcomet_core::domain::{
        Branch, CommitId, LogPage, Remote, RemoteBranch, Tag, UpstreamDivergence,
    };
    let mut state = state();
    let repo = &mut state.repos[0];
    repo.set_open(Loadable::Ready(()));
    repo.set_branches(Loadable::Ready(vec![Branch {
        name: "main".into(),
        target: CommitId("before".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.set_remote_branches(Loadable::Ready(vec![RemoteBranch {
        remote: "origin".into(),
        name: "main".into(),
        target: CommitId("before".into()),
    }]));
    repo.set_remotes(Loadable::Ready(vec![Remote {
        name: "origin".into(),
        url: Some("/remote".into()),
        default_branch: None,
    }]));
    repo.set_tags(Loadable::Ready(vec![Tag {
        name: "v1".into(),
        target: CommitId("before".into()),
    }]));
    repo.set_upstream_divergence(Loadable::Ready(Some(UpstreamDivergence {
        ahead: 1,
        behind: 2,
    })));
    repo.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    })));
    repo.auto_fetch.last_success_at = Some(UNIX_EPOCH);
    let cached = repo.clone();
    let lifetime = repo.lifetime_id();
    let inputs = super::super::test_inputs::clock();
    refresh(&mut state, RepoId(1), lifetime);
    let seq = state.repos[0].loads_in_flight.active_log_seq().unwrap();
    let scope = state.repos[0].history_state.history_scope;
    let error = Error::new(ErrorKind::Backend("repository no longer exists".into()));
    super::super::effects::branches_loaded(&inputs, &mut state, RepoId(1), Err(error.clone()));
    super::super::effects::remote_branches_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Err(error.clone()),
    );
    super::super::effects::remotes_loaded(&inputs, &mut state, RepoId(1), Err(error.clone()));
    super::super::effects::tags_loaded(&inputs, &mut state, RepoId(1), Err(error.clone()));
    super::super::effects::upstream_divergence_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Err(error.clone()),
    );
    super::super::external_and_history::log_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        seq,
        scope,
        None,
        Err(error),
    );
    let repo = &state.repos[0];
    assert_eq!(repo.branches, cached.branches);
    assert_eq!(repo.remote_branches, cached.remote_branches);
    assert_eq!(repo.remotes, cached.remotes);
    assert_eq!(repo.tags, cached.tags);
    assert_eq!(repo.upstream_divergence, cached.upstream_divergence);
    assert_eq!(repo.history_state.log, cached.history_state.log);
    assert!(repo.feedback.diagnostics.is_empty());
    assert!(state.notifications.is_empty());
    assert!(matches!(
        repo.auto_fetch.status,
        AutoFetchStatus::Failed { .. }
    ));
    assert_eq!(repo.auto_fetch.last_success_at, Some(UNIX_EPOCH));
    assert!(!repo.loads_in_flight.any_in_flight());

    state.repos[0].loads_in_flight.request(LoadKind::Branches);
    super::super::effects::branches_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Err(Error::new(ErrorKind::Backend(
            "manual refresh failed".into(),
        ))),
    );
    assert!(matches!(state.repos[0].branches, Loadable::Error(_)));
    assert!(!state.repos[0].feedback.diagnostics.is_empty());
}

#[test]
fn unsupported_optional_tags_do_not_turn_a_successful_automatic_fetch_into_a_failure() {
    use gitcomet_core::domain::Tag;

    let mut state = state();
    state.repos[0].set_open(Loadable::Ready(()));
    state.repos[0].set_tags(Loadable::Ready(Vec::<Tag>::new()));
    state.repos[0].auto_fetch.last_success_at = Some(UNIX_EPOCH);
    state.repos[0].auto_fetch.status = AutoFetchStatus::Succeeded { at: UNIX_EPOCH };
    state.repos[0]
        .loads_in_flight
        .request_background(LoadKind::Tags);
    let inputs = super::super::test_inputs::clock();
    super::super::effects::tags_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Err(Error::new(ErrorKind::Unsupported(
            "test backend has no tag listing",
        ))),
    );
    let repo = &state.repos[0];
    assert!(matches!(
        repo.auto_fetch.status,
        AutoFetchStatus::Succeeded { .. }
    ));
    assert_eq!(repo.auto_fetch.last_success_at, Some(UNIX_EPOCH));
    assert!(matches!(&repo.tags, Loadable::Ready(tags) if tags.is_empty()));
    assert!(repo.feedback.diagnostics.is_empty());
    assert!(!repo.loads_in_flight.any_in_flight());
}

#[test]
fn a_failed_background_read_keeps_the_tab_due_for_a_reload_after_the_next_fetch() {
    use gitcomet_core::domain::{HeadTracking, Tag};

    let mut state = state();
    let repo = &mut state.repos[0];
    repo.set_open(Loadable::Ready(()));
    repo.set_tags(Loadable::Ready(vec![Tag {
        name: "v1".into(),
        target: gitcomet_core::domain::CommitId("before".into()),
    }]));
    let lifetime = repo.lifetime_id();
    let inputs = super::super::test_inputs::clock();
    let effects = refresh(&mut state, RepoId(1), lifetime);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::References(ReferenceEffect::LoadTags { .. })))
    );
    super::super::effects::head_tracking_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Ok(HeadTracking::Untracked {
            branch: "main".into(),
        }),
    );
    super::super::effects::tags_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Err(Error::new(ErrorKind::Backend(
            "disk briefly unreadable".into(),
        ))),
    );
    // The next fetch runs and moves nothing.
    for (revision, status) in [
        (1, AutoFetchStatus::Running),
        (2, AutoFetchStatus::Succeeded { at: UNIX_EPOCH }),
    ] {
        updated(
            &inputs,
            &mut state,
            RepoId(1),
            lifetime,
            revision,
            RepoAutoFetchState {
                status,
                ..RepoAutoFetchState::default()
            },
            None,
        );
        assert!(
            state.repos[0].needs_auto_fetch_refresh(),
            "the failed tags read is retried even though tracking loaded"
        );
    }
    refresh(&mut state, RepoId(1), lifetime);
    super::super::effects::tags_loaded(&inputs, &mut state, RepoId(1), Ok(Vec::new()));
    super::super::effects::head_tracking_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Ok(HeadTracking::Untracked {
            branch: "main".into(),
        }),
    );
    assert!(!state.repos[0].needs_auto_fetch_refresh());
}

#[test]
fn a_count_whose_remote_the_fetch_skipped_stays_stale() {
    use gitcomet_core::domain::{HeadTracking, Upstream, UpstreamDivergence};

    let mut state = state();
    let lifetime = state.repos[0].lifetime_id();
    let inputs = super::super::test_inputs::clock();
    // origin is paused; the fetch reached only the other remote.
    updated(
        &inputs,
        &mut state,
        RepoId(1),
        lifetime,
        1,
        RepoAutoFetchState {
            status: AutoFetchStatus::Succeeded { at: UNIX_EPOCH },
            paused_remotes: vec!["origin".into()],
            ..RepoAutoFetchState::default()
        },
        None,
    );
    refresh(&mut state, RepoId(1), lifetime);
    let tracking = |remote: &str| HeadTracking::Tracking {
        branch: "main".into(),
        upstream: Upstream {
            remote: remote.into(),
            branch: "main".into(),
        },
        divergence: Some(UpstreamDivergence {
            ahead: 0,
            behind: 1,
        }),
    };
    super::super::effects::head_tracking_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Ok(tracking("origin")),
    );
    assert!(
        state.repos[0].incoming_is_stale(),
        "nothing checked origin, so its ↓1 may be out of date"
    );
    refresh(&mut state, RepoId(1), lifetime);
    super::super::effects::head_tracking_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Ok(tracking("other")),
    );
    assert!(!state.repos[0].incoming_is_stale(), "other was fetched");
    // origin's server waits out a failure, so the next fetch skips it.
    updated(
        &inputs,
        &mut state,
        RepoId(1),
        lifetime,
        2,
        RepoAutoFetchState {
            status: AutoFetchStatus::Succeeded { at: UNIX_EPOCH },
            skipped_remotes: vec!["other".into()],
            ..RepoAutoFetchState::default()
        },
        None,
    );
    assert!(
        state.repos[0].incoming_is_stale(),
        "the count other's fetch left unchecked greys at once"
    );
}

#[test]
fn failed_automatic_fetch_still_applies_successful_partial_reference_refreshes() {
    use gitcomet_core::domain::{CommitId, RemoteBranch};
    let mut state = state();
    state.repos[0].set_open(Loadable::Ready(()));
    let lifetime = state.repos[0].lifetime_id();
    let inputs = super::super::test_inputs::clock();
    updated(
        &inputs,
        &mut state,
        RepoId(1),
        lifetime,
        1,
        RepoAutoFetchState {
            last_success_at: Some(UNIX_EPOCH),
            status: AutoFetchStatus::Failed {
                at: UNIX_EPOCH,
                error: Error::new(ErrorKind::Backend("second remote unavailable".into())),
            },
            ..RepoAutoFetchState::default()
        },
        None,
    );
    refresh(&mut state, RepoId(1), lifetime);
    let branches = vec![RemoteBranch {
        remote: "origin".into(),
        name: "main".into(),
        target: CommitId("new-remote-tip".into()),
    }];
    super::super::effects::remote_branches_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Ok(branches.clone()),
    );
    assert_eq!(
        state.repos[0].remote_branches,
        Loadable::Ready(Arc::new(branches))
    );
    assert!(state.repos[0].feedback.diagnostics.is_empty());
    assert!(matches!(
        state.repos[0].auto_fetch.status,
        AutoFetchStatus::Failed { .. }
    ));
}
