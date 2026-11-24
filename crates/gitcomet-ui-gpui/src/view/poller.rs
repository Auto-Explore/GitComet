use super::*;

#[derive(Debug, Default)]
pub(super) struct PendingPublication {
    snapshot: Option<(Arc<AppState>, u64)>,
    waiting_for_frame: bool,
    timer: Option<gpui::Task<()>>,
}

// A selection and its cached details usually publish within a millisecond.
// Deliver that pair together, well before the next normal display interval.
const INITIAL_BATCH_INTERVAL: Duration = Duration::from_millis(1);

// A compositor can stop delivering frames to a hidden window. Its document saves
// and repository feedback still need acknowledgement while the window is hidden.
const FRAME_STALL_TIMEOUT: Duration = Duration::from_millis(250);

fn ready_selected_commit(repo: &RepoState) -> Option<&CommitId> {
    match &repo.history_state.commit_details {
        Loadable::Ready(details)
            if repo.history_state.selected_commit.as_ref() == Some(&details.id) =>
        {
            Some(&details.id)
        }
        _ => None,
    }
}

impl AppUiModel {
    fn should_present_selection(&self, next: &AppState) -> bool {
        if self.state.active_repo != next.active_repo {
            return true;
        }
        let Some(ready) = self
            .active_repo_index
            .get(next)
            .and_then(ready_selected_commit)
        else {
            return false;
        };
        self.active_repo_index
            .get(&self.state)
            .and_then(ready_selected_commit)
            != Some(ready)
    }

    fn queue_publication(
        &mut self,
        snapshot: (Arc<AppState>, u64),
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let stalled = self.publication.waiting_for_frame;
        let present_selection = !stalled && self.should_present_selection(&snapshot.0);
        let pending = self.publication.snapshot.replace(snapshot).is_some();
        if present_selection {
            self.publication.timer.take();
            if self.apply_publication(window.window_handle().window_id(), cx) {
                self.wait_for_frame(window, cx);
            }
            return;
        }
        if pending {
            return;
        }
        let delay = if stalled {
            FRAME_STALL_TIMEOUT
        } else {
            INITIAL_BATCH_INTERVAL
        };
        let timer = cx.background_executor().timer(delay);
        self.publication.timer = Some(cx.spawn_in(window, async move |model, cx| {
            timer.await;
            let _ = cx.update(|window, cx| {
                model.update(cx, |model, cx| {
                    if model.apply_publication(window.window_handle().window_id(), cx) && !stalled {
                        model.wait_for_frame(window, cx);
                    }
                })
            });
        }));
    }

    fn wait_for_frame(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.publication.waiting_for_frame = true;
        let model = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            // GPUI runs this callback immediately before painting. The next
            // foreground turn delivers the pending state after this frame.
            window
                .spawn(cx, async move |cx| {
                    let _ = cx.update(|window, cx| {
                        model.update(cx, |model, cx| {
                            model.publication.waiting_for_frame = false;
                            model.publication.timer.take();
                            if model.apply_publication(window.window_handle().window_id(), cx) {
                                model.wait_for_frame(window, cx);
                            }
                        })
                    });
                })
                .detach();
        });
    }

    fn apply_publication(&mut self, window: gpui::WindowId, cx: &mut gpui::Context<Self>) -> bool {
        let Some((snapshot, publication)) = self.publication.snapshot.take() else {
            return false;
        };
        if Arc::ptr_eq(&snapshot, &self.state) {
            return false;
        }
        let applying = gitcomet_core::op_trace::enabled().then(Instant::now);
        self.set_state(snapshot, cx);
        if let Some(applying) = applying {
            cx.defer(move |_| {
                let elapsed = gitcomet_core::op_trace::duration_ns(applying.elapsed());
                gitcomet_core::op_trace::record(
                    gitcomet_core::op_trace::Stage::Applied,
                    0,
                    "set_state",
                    publication,
                    elapsed,
                );
                if crate::ui_probe::jsonl_enabled() {
                    crate::ui_probe::scenario_record(
                        "state_applied",
                        serde_json::json!({"window": format!("{window:?}"), "publication": publication, "duration_ns": elapsed}),
                    );
                }
            });
        }
        true
    }
}

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

                // The read is a lock plus an `Arc` clone; only when the reducer
                // holds the write lock is it worth a thread hop to keep the UI
                // thread from waiting on it.
                let snapshot = if let Some(snapshot) = store.try_snapshot_with_publication() {
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

                let _ = cx.update(|window, cx| {
                    model.update(cx, |model, cx| {
                        model.queue_publication(snapshot, window, cx)
                    })
                });
            }
        });

        Poller {
            _task: task,
            _held_events: None,
        }
    }
}
