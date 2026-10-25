use super::*;
use crate::model::LoadKind;
use crate::model::RepoState;
use crate::msg::RepoCommandKind;
use crate::msg::StatusEffect;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{RepoSpec, RepoStatus};
use gitcomet_core::error::{Error, ErrorKind};
use std::path::PathBuf;

fn fixture() -> (AppState, RepoId) {
    let id = RepoId(901);
    let mut state = AppState::test_default();
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

fn stats_job(effects: &[Effect]) -> (crate::model::LineStatsGeneration, Arc<RepoStatus>) {
    let jobs: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Status(StatusEffect::LoadUncommittedLineStats {
                generation, status, ..
            }) => Some((*generation, Arc::clone(status))),
            _ => None,
        })
        .collect();
    assert_eq!(jobs.len(), 1);
    jobs[0].clone()
}

fn no_stats(effects: &[Effect]) {
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::Status(StatusEffect::LoadUncommittedLineStats { .. })
    )));
}

fn previous_counts() -> Arc<gitcomet_core::domain::UncommittedLineStats> {
    let mut counts = gitcomet_core::domain::UncommittedLineStats::default();
    counts.staged.insert(
        PathBuf::from("staged.txt"),
        gitcomet_core::domain::LineStats::UNKNOWN,
    );
    counts.unstaged.insert(
        PathBuf::from("changed.txt"),
        gitcomet_core::domain::LineStats::UNKNOWN,
    );
    Arc::new(counts)
}

fn assert_empty_counts_settled(repo: &RepoState) {
    assert!(matches!(&repo.uncommitted_line_stats,
        Loadable::Ready(counts) if counts.staged.is_empty() && counts.unstaged.is_empty()));
    assert!(repo.uncommitted_large_files.is_empty());
    assert!(!repo.loads_in_flight.line_stats_in_flight());
}

#[test]
fn clean_status_settles_counts_and_large_file_rows_without_a_backend_job() {
    for large_files in [false, true] {
        let (mut state, id) = fixture();
        if large_files {
            effects::large_file_support_loaded(
                &crate::store::reducer::test_inputs::clock(),
                &mut state,
                id,
                Ok(lfs_support()),
            );
        }
        // Initial clean loading and subsequent dirty-to-clean refreshes both
        // settle immediately, including stale LFS rows from the earlier view.
        for after_dirty in [false, true] {
            if after_dirty {
                state.repos[0].set_uncommitted_line_stats(Loadable::Ready(previous_counts()));
                if large_files {
                    state.repos[0]
                        .set_uncommitted_large_files(Arc::new(large_files_for("changed.txt")));
                }
            }
            let mut requested = Vec::new();
            util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
            no_stats(&effects::status_loaded(
                &crate::store::reducer::test_inputs::clock(),
                &mut state,
                id,
                Ok(RepoStatus::default()),
            ));
            assert_empty_counts_settled(&state.repos[0]);
            assert!(!state.repos[0].loads_in_flight.any_in_flight());
        }
    }
}

#[test]
fn clean_counts_wait_for_both_status_replays_and_failed_lane_recovery() {
    let (mut state, id) = fixture();
    let previous = previous_counts();
    state.repos[0].set_uncommitted_line_stats(Loadable::Ready(Arc::clone(&previous)));
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let replay = effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(RepoStatus::default()),
    );
    no_stats(&replay);
    assert_eq!(replay.len(), 2);
    assert!(matches!(&state.repos[0].uncommitted_line_stats,
        Loadable::Ready(counts) if Arc::ptr_eq(counts, &previous)));
    no_stats(&effects::worktree_status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(vec![]),
    ));
    assert!(matches!(&state.repos[0].uncommitted_line_stats,
        Loadable::Ready(counts) if Arc::ptr_eq(counts, &previous)));
    no_stats(&effects::staged_status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Err(Error::new(ErrorKind::Backend("index unavailable".into()))),
    ));
    assert!(matches!(&state.repos[0].uncommitted_line_stats,
        Loadable::Ready(counts) if Arc::ptr_eq(counts, &previous)));

    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    no_stats(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(RepoStatus::default()),
    ));
    assert_empty_counts_settled(&state.repos[0]);
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
}

#[test]
fn clean_counts_after_an_obsolete_job_reject_late_results_and_allow_new_dirty_counts() {
    let (mut state, id) = fixture();
    effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(lfs_support()),
    );
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (obsolete, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    no_stats(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(RepoStatus::default()),
    ));
    assert!(state.repos[0].loads_in_flight.line_stats_in_flight());
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        obsolete,
        Ok(previous_counts().as_ref().clone()),
        Some(Ok(large_files_for("obsolete.bin"))),
    ));
    assert_empty_counts_settled(&state.repos[0]);

    // The empty generation must not leave the lane busy or let an old reply
    // release the new dirty job. A staged-only snapshot still needs that job.
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let staged = RepoStatus {
        staged: snapshot().unstaged,
        unstaged: Arc::default(),
    };
    let (current, supplied) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(staged.clone()),
    ));
    assert_ne!(current, obsolete);
    assert_eq!(*supplied, staged);
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        obsolete,
        Ok(previous_counts().as_ref().clone()),
        Some(Ok(large_files_for("obsolete.bin"))),
    ));
    assert!(state.repos[0].loads_in_flight.line_stats_in_flight());
    assert!(state.repos[0].uncommitted_large_files.is_empty());
    let next = previous_counts();
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        current,
        Ok(next.as_ref().clone()),
        Some(Ok(large_files_for("current.bin"))),
    ));
    assert!(matches!(&state.repos[0].uncommitted_line_stats,
        Loadable::Ready(counts) if **counts == *next));
    assert!(!state.repos[0].uncommitted_large_files.is_empty());
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
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
                .filter(|e| matches!(e, Effect::Status(StatusEffect::LoadStatus { .. })))
                .count(),
            1
        );
        let status = snapshot();
        let (generation, supplied) = stats_job(&effects::status_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            id,
            Ok(status.clone()),
        ));
        assert_eq!(supplied.as_ref(), &status);
        assert!(Arc::ptr_eq(&supplied.unstaged, &status.unstaged));
        assert!(matches!(state.repos[0].worktree_status, Loadable::Ready(_)));
        assert!(state.repos[0].loads_in_flight.line_stats_in_flight());
        assert!(
            effects::uncommitted_line_stats_loaded(
                &mut state,
                id,
                generation,
                Ok(Default::default()),
                None
            )
            .is_empty()
        );
        assert!(!state.repos[0].loads_in_flight.line_stats_in_flight());
    }
}

#[test]
fn changes_during_status_wait_for_both_replays_even_when_paths_are_unchanged() {
    let (mut state, id) = fixture();
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let replay = effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    );
    no_stats(&replay);
    assert_eq!(replay.len(), 2);
    no_stats(&effects::worktree_status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot().unstaged.as_ref().clone()),
    ));
    let (generation, _) = stats_job(&effects::staged_status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(vec![]),
    ));
    effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        generation,
        Ok(Default::default()),
        None,
    );
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
}

#[test]
fn same_paths_new_content_discards_old_counts_and_coalesces_the_latest_snapshot() {
    let (mut state, id) = fixture();
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (old_generation, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    let revision = state.repos[0].worktree_status_rev;
    for _ in 0..2 {
        util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
        no_stats(&effects::status_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            id,
            Ok(snapshot()),
        ));
    }
    assert_eq!(state.repos[0].worktree_status_rev, revision);
    let (new_generation, _) = stats_job(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old_generation,
        Ok(Default::default()),
        None,
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
        None,
    ));
    assert!(state.repos[0].loads_in_flight.any_in_flight());
    effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        new_generation,
        Ok(Default::default()),
        None,
    );
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
    let mut repos = RepositoryRuntimes::default();
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
            change: change.clone(),
        },
    );
    no_stats(&requested);
    assert!(
        requested
            .iter()
            .any(|e| matches!(e, Effect::Status(StatusEffect::LoadWorktreeStatus { .. })))
    );
    no_stats(&effects::worktree_status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Err(Error::new(ErrorKind::Backend("failed".into()))),
    ));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
    // Full success with an unchanged combined payload must restore the failed lane.
    let mut requested = Vec::new();
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (generation, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        generation,
        Err(Error::new(ErrorKind::Backend("count failed".into()))),
        None,
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
        &crate::store::reducer::test_inputs::clock(),
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
    let (old, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    state.repos[0].loads_in_flight.invalidate_line_stats();
    state.repos[0].loads_in_flight.clear();
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old,
        Ok(Default::default()),
        None,
    ));
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
    util::append_requested_status_refresh_effects(&mut state.repos[0], &mut requested);
    let (new, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    no_stats(&effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        old,
        Ok(Default::default()),
        None,
    ));
    assert!(state.repos[0].loads_in_flight.any_in_flight());
    effects::uncommitted_line_stats_loaded(&mut state, id, new, Ok(Default::default()), None);
    assert!(!state.repos[0].loads_in_flight.any_in_flight());
}

#[test]
fn already_up_to_date_pull_still_refreshes_once_and_counts_settle_after_status() {
    use gitcomet_core::services::{CommandOutput, PullMode};
    let (mut state, id) = fixture();
    state.repos[0].pull_in_flight = 1;
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
    let mut repos = RepositoryRuntimes::default();
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
            .filter(|e| matches!(e, Effect::Status(StatusEffect::LoadStatus { .. })))
            .count(),
        1
    );
    no_stats(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(RepoStatus::default()),
    ));
    assert_empty_counts_settled(&state.repos[0]);
    let loads = &state.repos[0].loads_in_flight;
    assert!(!loads.is_in_flight(LoadKind::WorktreeStatus));
    assert!(!loads.is_in_flight(LoadKind::StagedStatus));
    assert!(!loads.line_stats_in_flight());
}

fn lfs_support() -> gitcomet_core::large_files::LargeFileSupport {
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.lfs.has_local_store = true;
    support
}

fn large_files_for(path: &str) -> gitcomet_core::large_files::UncommittedLargeFiles {
    let oid = gitcomet_core::lfs::LfsOid([7; 32]);
    let mut files = gitcomet_core::large_files::UncommittedLargeFiles::default();
    files.unstaged.insert(
        PathBuf::from(path),
        gitcomet_core::large_files::LargeFileState {
            pointer: gitcomet_core::large_files::LargeFilePointer::Lfs(
                gitcomet_core::lfs::LfsPointer { oid, size: 9 },
            ),
            in_local_store: Some(false),
            worktree: Some(gitcomet_core::large_files::LargeFileWorktree::Pointer),
            lockable: false,
        },
    );
    files
}

fn stats_job_large_files(effects: &[Effect]) -> bool {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Status(StatusEffect::LoadUncommittedLineStats { large_files, .. }) => {
                Some(*large_files)
            }
            _ => None,
        })
        .expect("a line-stats job")
}

/// Rows counted before the repo was known to use LFS carry no chips, so the
/// first active support load must recount them, this time with large files.
#[test]
fn becoming_active_recounts_rows_with_large_files() {
    let (mut state, id) = fixture();
    util::refresh_primary_effects(&mut state.repos[0]);
    let (generation, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        generation,
        Ok(Default::default()),
        None,
    );

    let recount = effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(lfs_support()),
    );
    assert!(state.repos[0].large_file_support_active());
    assert!(stats_job_large_files(&recount), "{recount:?}");

    // A second identical load changes nothing and schedules nothing.
    let rev = state.repos[0].large_file_support_rev;
    no_stats(&effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(lfs_support()),
    ));
    assert_eq!(state.repos[0].large_file_support_rev, rev);
}

#[test]
fn large_file_rows_follow_the_line_stats_generation() {
    let (mut state, id) = fixture();
    effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(lfs_support()),
    );
    util::refresh_primary_effects(&mut state.repos[0]);
    let (stale, _) = stats_job(&effects::status_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(snapshot()),
    ));
    // Invalidated mid-scan: the stale reply must not publish its rows.
    state.repos[0].loads_in_flight.invalidate_line_stats();
    let replay = effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        stale,
        Ok(Default::default()),
        Some(Ok(large_files_for("stale.bin"))),
    );
    assert!(state.repos[0].uncommitted_large_files.is_empty());

    let (current, _) = stats_job(&replay);
    let rev = state.repos[0].large_files_rev;
    effects::uncommitted_line_stats_loaded(
        &mut state,
        id,
        current,
        Ok(Default::default()),
        Some(Ok(large_files_for("a.bin"))),
    );
    assert_eq!(state.repos[0].large_files_rev, rev.wrapping_add(1));
    let row = state.repos[0]
        .large_file_state(
            gitcomet_core::domain::DiffArea::Unstaged,
            std::path::Path::new("a.bin"),
        )
        .expect("published row");
    assert!(row.content_missing());
}

#[test]
fn losing_support_clears_rows_and_failures_keep_what_was_known() {
    let (mut state, id) = fixture();
    effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(lfs_support()),
    );
    state.repos[0].set_uncommitted_large_files(Arc::new(large_files_for("a.bin")));

    effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Err(Error::new(ErrorKind::Backend("config unreadable".into()))),
    );
    assert!(
        state.repos[0].large_file_support_active(),
        "a failed refresh keeps facts"
    );
    assert!(
        state.repos[0].feedback.diagnostics.is_empty(),
        "detection never raises a banner"
    );

    effects::large_file_support_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        id,
        Ok(Default::default()),
    );
    assert!(!state.repos[0].large_file_support_active());
    assert!(state.repos[0].uncommitted_large_files.is_empty());
}
