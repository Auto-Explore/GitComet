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

        let task = window.spawn(cx, async move |cx| {
            loop {
                if events.recv().await.is_err() {
                    break;
                }
                while events.try_recv().is_ok() {}

                // Keep the store lock/read work off the UI thread.
                let snapshot = if runtime.uses_background_compute() {
                    smol::unblock({
                        let store = Arc::clone(&store);
                        move || store.snapshot()
                    })
                    .await
                } else {
                    store.snapshot()
                };

                let _ = model.update(cx, |model, cx| model.set_state(snapshot, cx));
            }
        });

        Poller {
            _task: task,
            _held_events: None,
        }
    }
}
