#[cfg(test)]
use std::cell::Cell;
use std::time::Duration;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiRuntimeMode {
    Live,
    Deterministic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UiRuntime {
    mode: UiRuntimeMode,
}

impl UiRuntime {
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) const fn live() -> Self {
        Self {
            mode: UiRuntimeMode::Live,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn deterministic() -> Self {
        Self {
            mode: UiRuntimeMode::Deterministic,
        }
    }

    pub(crate) const fn uses_live_store_poller(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn uses_background_compute(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn uses_tooltip_delay(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn uses_toast_ttl(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn uses_cursor_blink(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn uses_pane_animations(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn uses_repo_tab_spinner_delay(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn persists_ui_settings(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn auto_restores_session(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub(crate) const fn diff_syntax_foreground_parse_budget(self) -> Duration {
        match self.mode {
            UiRuntimeMode::Live => Duration::from_millis(1),
            UiRuntimeMode::Deterministic => Duration::from_millis(2),
        }
    }
}

#[cfg(test)]
thread_local! {
    static UI_RUNTIME_OVERRIDE: Cell<Option<UiRuntime>> = const { Cell::new(None) };
}

/// Runs `compute` on a background thread, or inline when the runtime is
/// deterministic, then calls `apply` with the result inside a view update.
///
/// The `apply` closure owns the site's staleness checks (generation, repo,
/// revision) and any re-issue, so the snapshot/compute/stale/apply skeleton
/// stays in one place while each caller keeps its own guards.
pub(crate) fn run_background_compute<V, O, F, A>(
    cx: &mut gpui::Context<V>,
    compute: F,
    apply: A,
) -> gpui::Task<()>
where
    V: 'static,
    O: Send + 'static,
    F: FnOnce() -> O + Send + 'static,
    A: FnOnce(&mut V, &mut gpui::Context<V>, O) + 'static,
{
    cx.spawn(
        async move |view: gpui::WeakEntity<V>, cx: &mut gpui::AsyncApp| {
            let output = if current().uses_background_compute() {
                smol::unblock(compute).await
            } else {
                compute()
            };
            let _ = view.update(cx, move |view, cx| apply(view, cx, output));
        },
    )
}

pub(crate) fn current() -> UiRuntime {
    #[cfg(test)]
    {
        UI_RUNTIME_OVERRIDE.with(|cell| cell.get().unwrap_or_else(UiRuntime::deterministic))
    }

    #[cfg(not(test))]
    {
        UiRuntime::live()
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn with_override<T>(runtime: UiRuntime, f: impl FnOnce() -> T) -> T {
    UI_RUNTIME_OVERRIDE.with(|cell| {
        let prev = cell.replace(Some(runtime));
        let result = f();
        cell.set(prev);
        result
    })
}
