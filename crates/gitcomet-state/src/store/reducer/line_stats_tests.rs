use super::*;
use crate::model::{RepoLoadsInFlight, RepoState};
use gitcomet_core::domain::{RepoSpec, RepoStatus};
use gitcomet_core::error::{Error, ErrorKind};
use std::path::PathBuf;

fn fixture() -> (AppState, RepoId) {
    let id = RepoId(901);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        id,
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    ));
    (state, id)
}

fn snapshot() -> RepoStatus {
    RepoStatus {
        staged: Arc::new(vec![]),
        unstaged: Arc::new(vec![gitcomet_core::domain::FileStatus {
            path: PathBuf::from("changed.txt"),
            kind: gitcomet_core::domain::FileStatusKind::Modified,
            conflict: None,
        }]),
    }
}

fn stats_job(effects: &[Effect]) -> (u64, Arc<RepoStatus>) {
    let jobs: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadUncommittedLineStats {
                generation, status, ..
            } => Some((*generation, Arc::clone(status))),
            _ => None,
        })
        .collect();
    assert_eq!(jobs.len(), 1);
    jobs[0].clone()
}

fn no_stats(effects: &[Effect]) {
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadUncommittedLineStats { .. }))
    );
}

#[test]
fn initial_refresh_publishes_status_before_counts_and_reuses_its_arcs() {
    for full in [false, true] {
        let (mut state, id) = fixture();
        let requested = if full {
            util::refresh_full_effects(&mut state.repos[0], Default::default())
        } else {
            util::refresh_primary_effects(&mut state.repos[0])
        };
        no_stats(&requested);
        assert_eq!(
            requested
                .iter()
                .filter(|e| matches!(e, Effect::LoadStatus { .. }))
                .count(),
            1
        );
        let status = snapshot();
        let (generation, supplied) =
            stats_job(&effects::status_loaded(&mut state, id, Ok(status.clone())));
        assert_eq!(supplied.as_ref(), &status);
        assert!(Arc::ptr_eq(&supplied.unstaged, &status.unstaged));
        assert!(matches!(state.repos[0].worktree_status, Loadable::Ready(_)));
        assert!(
            state.repos[0]
                .loads_in_flight
                .is_in_flight(RepoLoadsInFlight::UNCOMMITTED_LINE_STATS)
        );
        assert!(
            effects::uncommitted_line_stats_loaded(
                &mut state,
                id,
                generation,
                Ok(Default::default())
            )
            .is_empty()
        );
        assert!(
            !state.repos[0]
                .loads_in_flight
                .is_in_flight(RepoLoadsInFlight::UNCOMMITTED_LINE_STATS)
        );
    }
}

#[test]
fn changes_during_status_wait_for_both_replays_even_when_paths_are_unchanged() {
    let (mut state, id) = fixture();
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let replay = effects::status_loaded(&mut state, id, Ok(snapshot()));
    no_stats(&replay);
    assert_eq!(replay.len(), 2);
    no_stats(&effects::worktree_status_loaded(
        &mut state,
        id,
        Ok(snapshot().unstaged.as_ref().clone()),
    ));
    let (generation, _) = stats_job(&effects::staged_status_loaded(&mut state, id, Ok(vec![])));
    effects::uncommitted_line_stats_loaded(&mut state, id, generation, Ok(Default::default()));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
}

#[test]
fn same_paths_new_content_discards_old_counts_and_coalesces_the_latest_snapshot() {
    let (mut state, id) = fixture();
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (old_generation, _) = stats_job(&effects::status_loaded(&mut state, id, Ok(snapshot())));
    let revision = state.repos[0].worktree_status_rev;
    for _ in 0..2 {
        util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
        no_stats(&effects::status_loaded(&mut state, id, Ok(snapshot())));
    }
    assert_eq!(state.repos[0].worktree_status_rev, revision);
    let (new_generation, _) = stats_job(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old_generation,
        Ok(Default::default()),
    ));
    assert_ne!(old_generation, new_generation);
    assert!(matches!(
        state.repos[0].uncommitted_line_stats,
        Loadable::NotLoaded
    ));
    // A duplicate old completion must not release the new job.
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old_generation,
        Ok(Default::default()),
    ));
    assert!(state.repos[0].loads_in_flight.any_in_flight());
    effects::uncommitted_line_stats_loaded(&mut state, id, new_generation, Ok(Default::default()));
    assert!(matches!(
        state.repos[0].uncommitted_line_stats,
        Loadable::Ready(_)
    ));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
}

#[test]
fn worktree_only_change_updates_counts_and_failed_lane_never_uses_combined_fallback() {
    let (mut state, id) = fixture();
    state.repos[0].set_status(Loadable::Ready(Arc::new(snapshot())));
    let previous_counts = Arc::new(gitcomet_core::domain::UncommittedLineStats::default());
    state.repos[0].set_uncommitted_line_stats(Loadable::Ready(Arc::clone(&previous_counts)));
    let mut repos = FxHashMap::default();
    let ids = AtomicU64::new(902);
    let change = crate::msg::RepoExternalChange {
        worktree: true,
        ..Default::default()
    };
    let requested = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: id,
            change,
        },
    );
    no_stats(&requested);
    assert!(
        requested
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktreeStatus { .. }))
    );
    no_stats(&effects::worktree_status_loaded(
        &mut state,
        id,
        Err(Error::new(ErrorKind::Backend("failed".into()))),
    ));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
    // Full success with an unchanged combined payload must restore the failed lane.
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (generation, _) = stats_job(&effects::status_loaded(&mut state, id, Ok(snapshot())));
    effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        generation,
        Err(Error::new(ErrorKind::Backend("count failed".into()))),
    );
    assert!(matches!(&state.repos[0].uncommitted_line_stats,
        Loadable::Ready(counts) if Arc::ptr_eq(counts, &previous_counts)));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
    let requested = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: id,
            change,
        },
    );
    no_stats(&requested);
    stats_job(&effects::worktree_status_loaded(
        &mut state,
        id,
        Ok(snapshot().unstaged.as_ref().clone()),
    ));
}

#[test]
fn cancellation_clears_queued_counts_and_old_completion_cannot_release_new_job() {
    let (mut state, id) = fixture();
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (old, _) = stats_job(&effects::status_loaded(&mut state, id, Ok(snapshot())));
    state.repos[0].loads_in_flight.invalidate_line_stats();
    state.repos[0].loads_in_flight.clear();
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old,
        Ok(Default::default()),
    ));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (new, _) = stats_job(&effects::status_loaded(&mut state, id, Ok(snapshot())));
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old,
        Ok(Default::default()),
    ));
    assert!(state.repos[0].loads_in_flight.any_in_flight());
    effects::uncommitted_line_stats_loaded(&mut state, id, new, Ok(Default::default()));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
}

#[test]
fn already_up_to_date_pull_still_refreshes_once_and_counts_settle_after_status() {
    use gitcomet_core::services::{CommandOutput, PullMode};
    let (mut state, id) = fixture();
    state.repos[0].pull_in_flight = 1;
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
    let mut repos = FxHashMap::default();
    let ids = AtomicU64::new(902);
    let mut output = CommandOutput::empty_success("git pull");
    output.stdout = "Already up to date.\n".into();
    let requested = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Ok(output),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 0);
    assert!(state.repos[0].loads_in_flight.any_in_flight());
    no_stats(&requested);
    assert_eq!(
        requested
            .iter()
            .filter(|e| matches!(e, Effect::LoadStatus { .. }))
            .count(),
        1
    );
    let (generation, supplied) = stats_job(&effects::status_loaded(
        &mut state,
        id,
        Ok(RepoStatus::default()),
    ));
    assert!(supplied.unstaged.is_empty());
    effects::uncommitted_line_stats_loaded(&mut state, id, generation, Ok(Default::default()));
    assert!(!state.repos[0].loads_in_flight.is_in_flight(
        RepoLoadsInFlight::WORKTREE_STATUS
            | RepoLoadsInFlight::STAGED_STATUS
            | RepoLoadsInFlight::UNCOMMITTED_LINE_STATS
    ));
}
