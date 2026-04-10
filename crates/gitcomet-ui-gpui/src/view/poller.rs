use super::*;

pub(super) struct Poller {
    _task: gpui::Task<()>,
}

impl Poller {
    pub(super) fn start(
        store: Arc<AppStore>,
        events: smol::channel::Receiver<StoreEvent>,
        model: WeakEntity<AppUiModel>,
        window: &mut Window,
        cx: &mut gpui::Context<GitCometView>,
    ) -> Poller {
        let task = window.spawn(cx, async move |cx| {
            loop {
                if events.recv().await.is_err() {
                    break;
                }
                while events.try_recv().is_ok() {}

                // Keep the store lock/read work off the UI thread.
                let snapshot = if cfg!(test) {
                    store.snapshot()
                } else {
                    smol::unblock({
                        let store = Arc::clone(&store);
                        move || store.snapshot()
                    })
                    .await
                };

                let _ = model.update(cx, |model, cx| model.set_state(snapshot, cx));
            }
        });

        Poller { _task: task }
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Poller {
        Poller {
            _task: gpui::Task::ready(()),
        }
    }
}
