//! Named native windows for multi-window profiling. Keep only handles for
//! inactive windows: retaining their views would invalidate close/leak tests.
use super::*;

fn main_windows(cx: &App) -> Vec<AnyWindowHandle> {
    cx.windows()
        .into_iter()
        .filter(|window| window.downcast::<GitCometView>().is_some())
        .collect()
}

impl Driver {
    pub(super) async fn new_window(&mut self, name: &str, cx: &mut AsyncApp) -> Result<(), String> {
        if name.is_empty() || self.windows.contains_key(name) {
            return Err(format!("window name must be new and nonempty: {name:?}"));
        }
        if self.operation.is_some() {
            return Err("finish the active operation before changing target windows".into());
        }
        let previous = cx.update(|cx| main_windows(cx));
        let view = self.view.clone();
        self.window
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.execute_command("new-window", Some(window), cx)
                });
            })
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
        let window = loop {
            let opened = cx.update(|cx| {
                main_windows(cx)
                    .into_iter()
                    .find(|window| !previous.contains(window))
            });
            if let Some(window) = opened {
                break window;
            }
            if Instant::now() >= deadline {
                return Err(format!("new window {name:?} never opened"));
            }
            self.sleep(WITNESS_POLL, cx).await;
        };
        self.windows.insert(name.into(), window);
        self.switch_window(name, cx).await
    }

    pub(super) async fn switch_window(
        &mut self,
        name: &str,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        if self.operation.is_some() {
            return Err("finish the active operation before changing target windows".into());
        }
        let window = *self
            .windows
            .get(name)
            .ok_or_else(|| format!("unknown window {name:?}"))?;
        let view = cx.update(|cx| {
            window
                .downcast::<GitCometView>()
                .ok_or_else(|| "target is not a main window".to_string())?
                .entity(cx)
                .map_err(|e| e.to_string())
        })?;
        let (changed, observers) = Self::observe_view(&view, cx);
        self.window = window;
        self.view = view;
        self.changed = changed;
        self._observers = observers;
        self.pointer_expected = None;
        window
            .update(cx, |_, window, _| window.activate())
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
        while !window
            .update(cx, |_, window, _| window.is_window_active())
            .map_err(|e| e.to_string())?
        {
            if Instant::now() >= deadline {
                return Err(format!("window {name:?} did not receive native focus"));
            }
            self.sleep(WITNESS_POLL, cx).await;
        }
        record(
            "scenario_window_target",
            json!({"name": name, "window": format!("{:?}", window.window_id())}),
        );
        self.record_windows(cx)?;
        Ok(())
    }

    pub(super) async fn close_window(
        &mut self,
        name: &str,
        cx: &mut AsyncApp,
    ) -> Result<(), String> {
        let window = *self
            .windows
            .get(name)
            .ok_or_else(|| format!("unknown window {name:?}"))?;
        if window == self.window {
            return Err("switch to a surviving window before closing the current target".into());
        }
        window
            .update(cx, |_, window, cx| {
                crate::app::close_window_or_warn(window, cx);
            })
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + DEFAULT_WITNESS_TIMEOUT;
        while cx.update(|cx| main_windows(cx).contains(&window)) {
            if Instant::now() >= deadline {
                return Err(format!("window {name:?} did not close; check close guards"));
            }
            self.sleep(WITNESS_POLL, cx).await;
        }
        self.windows.remove(name);
        self.record_windows(cx)?;
        Ok(())
    }

    pub(super) fn record_windows(&self, cx: &mut AsyncApp) -> Result<usize, String> {
        let handles = cx.update(|cx| main_windows(cx));
        let mut windows = Vec::with_capacity(handles.len());
        for handle in handles {
            let name = self
                .windows
                .iter()
                .find_map(|(name, window)| (*window == handle).then_some(name.as_str()));
            let state = handle
                .update(cx, |root, window, cx| {
                    let view = root.downcast::<GitCometView>().expect("main window");
                    let view = view.read(cx);
                    let repo = view.active_repo();
                    json!({
                        "name": name,
                        "window": format!("{:?}", handle.window_id()),
                        "active": window.is_window_active(),
                        "repository": repo.map(|r| &r.spec.workdir),
                        "selected_commit": repo.and_then(|r| r.history_state.selected_commit.as_ref()).map(|id| id.as_ref()),
                        "ready": repo.is_some_and(|r| matches!(r.open, Loadable::Ready(_))
                            && matches!(r.status, Loadable::Ready(_))
                            && matches!(r.history_state.log, Loadable::Ready(_))),
                        "indexed_window": view.main_pane.read(cx).history_view.read(cx).scenario_indexed_window(),
                    })
                })
                .map_err(|e| e.to_string())?;
            windows.push(state);
        }
        let count = windows.len();
        record("scenario_windows", json!({"windows": windows}));
        Ok(count)
    }
}
