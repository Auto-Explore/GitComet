use super::*;

#[test]
fn reducer_diagnostics_track_dispatches_and_clone_on_write() {
    let before = AppStore::reducer_diagnostics();

    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, event_rx) = AppStore::new(backend);
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
