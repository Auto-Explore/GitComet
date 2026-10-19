use super::*;

#[test]
fn reducer_diagnostics_track_dispatches_and_clone_on_write() {
    let before = AppStore::reducer_diagnostics();

    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, event_rx) = AppStore::new_test(backend);
    let held_snapshot = store.snapshot();

    store.dispatch(Msg::RestoreSession {
        open_repos: Vec::new(),
        active_repo: None,
    });
    wait_for_state_changed(&event_rx);
    drop(held_snapshot);

    let after = AppStore::reducer_diagnostics();
    assert!(after.dispatch_count > before.dispatch_count);
    assert!(after.reducer_total_nanos >= before.reducer_total_nanos);
    assert!(after.reducer_max_nanos >= before.reducer_max_nanos);
    assert!(after.clone_on_write_count > before.clone_on_write_count);
    assert!(after.clone_on_write_total_nanos >= before.clone_on_write_total_nanos);
    assert!(after.clone_on_write_max_nanos >= before.clone_on_write_max_nanos);
    assert!(after.max_shared_state_handles >= before.max_shared_state_handles.max(1));
    assert!(after.average_reducer_nanos() <= after.reducer_total_nanos);
    assert!(after.average_clone_on_write_nanos() <= after.clone_on_write_total_nanos);
}

/// How long one dispatch's copy-on-write clone of the whole state takes with
/// several repositories whose bounded logs are full. Reports timings rather
/// than asserting; run with `-- --ignored --nocapture`.
#[test]
#[ignore = "timing probe"]
fn timing_state_clone_with_full_repository_logs() {
    use crate::model::{CommandLogEntry, DiagnosticEntry, DiagnosticKind};

    fn full_repo(id: u64) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(id),
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::from(format!("/work/repo-{id}")),
            },
        );
        for ix in 0..200 {
            repo.feedback_mut().diagnostics.push(DiagnosticEntry {
                time: std::time::SystemTime::UNIX_EPOCH,
                kind: DiagnosticKind::Info,
                message: format!("diagnostic {ix} for repository {id}"),
            });
            repo.feedback_mut().command_log.push(CommandLogEntry {
                time: std::time::SystemTime::UNIX_EPOCH,
                ok: true,
                command: format!("git fetch origin refs/heads/feature-{ix}"),
                summary: format!("Fetched feature-{ix}"),
                stdout: Arc::from(""),
                stderr: Arc::from(""),
                announce_success: false,
                handled_by_auth_prompt: false,
                failure_id: None,
                hook_operation_id: None,
            });
        }
        repo
    }
    for repos in [1usize, 4, 8] {
        let state = Arc::new(AppState {
            repos: (1..=repos as u64).map(full_repo).collect(),
            ..AppState::default()
        });
        let rounds = 200;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            let mut shared = Arc::clone(&state);
            std::hint::black_box(Arc::make_mut(&mut shared));
        }
        let per_clone = start.elapsed() / rounds;
        eprintln!("{repos} repositories with full logs: {per_clone:?} per dispatch clone");
    }
}
