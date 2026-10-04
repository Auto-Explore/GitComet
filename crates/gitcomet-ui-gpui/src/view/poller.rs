use super::*;

pub(super) struct Poller {
    _task: gpui::Task<()>,
    _held_events: Option<smol::channel::Receiver<StoreEvent>>,
}

impl Poller {
    pub(super) fn start(
        store: Arc<AppStore>,
        events: smol::channel::Receiver<StoreEvent>,
        model: WeakEntity<AppUiModel>,
        window: &mut Window,
        cx: &mut gpui::Context<GitCometView>,
    ) -> Poller {
        let runtime = crate::ui_runtime::current();
        if !runtime.uses_live_store_poller() {
            // GPUI's test scheduler does not allow cross-thread wakes into a foreground task.
            // Keep the receiver alive so store notifications stay coalesced, but require tests
            // to pull snapshots explicitly via `crate::view::test_support::sync_store_snapshot`.
            return Poller {
                _task: gpui::Task::ready(()),
                _held_events: Some(events),
            };
        }

        let probe_window = crate::ui_probe::jsonl_enabled()
            .then(|| format!("{:?}", window.window_handle().window_id()));
        let task = window.spawn(cx, async move |cx| {
            loop {
                if events.recv().await.is_err() {
                    break;
                }
                while events.try_recv().is_ok() {}

                // The read is a lock plus an `Arc` clone; only when the reducer
                // holds the write lock is it worth a thread hop to keep the UI
                // thread from waiting on it.
                let (snapshot, publication) =
                    if let Some(snapshot) = store.try_snapshot_with_publication() {
                        snapshot
                    } else if runtime.uses_background_compute() {
                        smol::unblock({
                            let store = Arc::clone(&store);
                            move || store.snapshot_with_publication()
                        })
                        .await
                    } else {
                        store.snapshot_with_publication()
                    };

                let applying = gitcomet_core::op_trace::enabled().then(Instant::now);
                let _ = model.update(cx, |model, cx| model.set_state(snapshot, cx));
                if let Some(applying) = applying {
                    // Observers of the model run inside this update, so the
                    // span covers every pane's synchronous state application.
                    let elapsed = gitcomet_core::op_trace::duration_ns(applying.elapsed());
                    gitcomet_core::op_trace::record(
                        gitcomet_core::op_trace::Stage::Applied,
                        0,
                        "set_state",
                        publication,
                        elapsed,
                    );
                    if let Some(window) = &probe_window {
                        // Publication sequences are local to a store. A different
                        // window can have the same sequence, so the profiler must
                        // identify the window before matching an applied state.
                        crate::ui_probe::scenario_record(
                            "state_applied",
                            serde_json::json!({"window": window, "publication": publication, "duration_ns": elapsed}),
                        );
                    }
                }
            }
        });

        Poller {
            _task: task,
            _held_events: None,
        }
    }
}
