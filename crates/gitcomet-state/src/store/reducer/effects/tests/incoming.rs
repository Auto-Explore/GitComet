use super::*;
use crate::model::{RepoIncomingSnapshot, RepoIncomingState};
use gitcomet_core::domain::{HeadTracking, Upstream, UpstreamDivergence};
use std::time::UNIX_EPOCH;

fn tracking(branch: &str, behind: usize) -> HeadTracking {
    HeadTracking::Tracking {
        branch: branch.into(),
        upstream: Upstream {
            remote: "origin".into(),
            branch: branch.into(),
        },
        divergence: Some(UpstreamDivergence { ahead: 0, behind }),
    }
}

fn state() -> AppState {
    let mut state = AppState::test_default();
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/repo"),
        },
    );
    repo.set_open(Loadable::Ready(()));
    repo.set_head_branch(Loadable::Ready("main".into()));
    repo.incoming = RepoIncomingState::Current(RepoIncomingSnapshot {
        tracking: tracking("main", 3),
        checked_at: UNIX_EPOCH,
    });
    state.repos.push(repo);
    state.active_repo = Some(RepoId(1));
    state
}

#[test]
fn incoming_tracking_publishes_one_branch_identity_and_count_snapshot() {
    let mut state = state();
    let inputs = crate::store::reducer::test_inputs::clock();
    state.repos[0]
        .loads_in_flight
        .request(LoadKind::UpstreamDivergence);
    super::super::head_tracking_loaded(&inputs, &mut state, RepoId(1), Ok(tracking("other", 0)));
    let RepoIncomingState::Current(snapshot) = &state.repos[0].incoming else {
        panic!("successful tracking read must publish a current snapshot");
    };
    assert_eq!(snapshot.tracking, tracking("other", 0));
    assert_eq!(
        state.repos[0].upstream_divergence,
        Loadable::Ready(Some(UpstreamDivergence {
            ahead: 0,
            behind: 0,
        }))
    );
    super::super::head_branch_loaded(&inputs, &mut state, RepoId(1), Ok("main".into()));
    let RepoIncomingState::Current(snapshot) = &state.repos[0].incoming else {
        panic!("older head replies cannot invalidate authoritative tracking");
    };
    assert_eq!(snapshot.tracking, tracking("other", 0));
}

#[test]
fn incoming_tracking_discards_a_completed_read_when_replacement_is_pending() {
    let mut state = state();
    let inputs = crate::store::reducer::test_inputs::clock();
    let repo = &mut state.repos[0];
    assert!(repo.loads_in_flight.request(LoadKind::UpstreamDivergence));
    assert!(!repo.loads_in_flight.request(LoadKind::UpstreamDivergence));
    let effects =
        super::super::head_tracking_loaded(&inputs, &mut state, RepoId(1), Ok(tracking("main", 2)));
    assert!(matches!(
        &state.repos[0].incoming,
        RepoIncomingState::Stale(snapshot) if snapshot.tracking == tracking("main", 3)
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::References(ReferenceEffect::LoadUpstreamDivergence { repo_id: RepoId(1) })
    )));
    super::super::head_tracking_loaded(&inputs, &mut state, RepoId(1), Ok(tracking("other", 0)));
    assert!(matches!(
        &state.repos[0].incoming,
        RepoIncomingState::Current(snapshot) if snapshot.tracking == tracking("other", 0)
    ));
}

#[test]
fn incoming_tracking_read_failure_keeps_known_count_stale_and_quiet() {
    let mut state = state();
    let inputs = crate::store::reducer::test_inputs::clock();
    state.repos[0]
        .loads_in_flight
        .request_background(LoadKind::UpstreamDivergence);
    super::super::head_tracking_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Err(Error::new(ErrorKind::Backend(
            "cannot read references".into(),
        ))),
    );
    assert!(matches!(
        &state.repos[0].incoming,
        RepoIncomingState::Stale(snapshot) if snapshot.tracking == tracking("main", 3)
    ));
    assert!(state.repos[0].incoming_is_stale());
    assert!(state.repos[0].feedback.diagnostics.is_empty());
    assert!(state.notifications.is_empty());
}

#[test]
fn incoming_tracking_cached_reference_success_does_not_hide_fetch_failure() {
    let mut state = state();
    let inputs = crate::store::reducer::test_inputs::clock();
    state.repos[0].auto_fetch.status = crate::model::AutoFetchStatus::Failed {
        at: UNIX_EPOCH,
        error: Error::new(ErrorKind::Backend("remote unavailable".into())),
    };
    state.repos[0]
        .loads_in_flight
        .request_background(LoadKind::UpstreamDivergence);
    super::super::head_tracking_loaded(&inputs, &mut state, RepoId(1), Ok(tracking("main", 4)));
    assert!(state.repos[0].incoming_is_stale());
    let snapshot = match &state.repos[0].incoming {
        RepoIncomingState::Current(snapshot) | RepoIncomingState::Stale(snapshot) => snapshot,
        RepoIncomingState::Unknown => panic!("partial fetch must retain useful current refs"),
    };
    assert_eq!(snapshot.tracking, tracking("main", 4));
}

#[test]
fn incoming_tracking_cancelled_refresh_remains_stale_after_tab_deactivation() {
    let mut state = state();
    state.repos[0]
        .loads_in_flight
        .request_background(LoadKind::UpstreamDivergence);
    let mut effects = Vec::new();
    crate::store::reducer::repo_management::append_cancel_repo_loads_effect_for_repo(
        &mut state,
        Some(RepoId(1)),
        &mut effects,
    );
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
    assert!(matches!(
        state.repos[0].incoming,
        RepoIncomingState::Stale(_)
    ));
    assert!(state.repos[0].incoming_is_stale());
}

#[test]
fn incoming_tracking_scalar_fallback_preserves_counts_without_inventing_identity() {
    let mut state = state();
    let inputs = crate::store::reducer::test_inputs::clock();
    state.repos[0]
        .loads_in_flight
        .request(LoadKind::UpstreamDivergence);
    super::super::upstream_divergence_loaded(
        &inputs,
        &mut state,
        RepoId(1),
        Ok(Some(UpstreamDivergence {
            ahead: 2,
            behind: 4,
        })),
    );
    assert_eq!(state.repos[0].incoming, RepoIncomingState::Unknown);
    assert_eq!(
        state.repos[0].upstream_divergence,
        Loadable::Ready(Some(UpstreamDivergence {
            ahead: 2,
            behind: 4
        }))
    );
}
