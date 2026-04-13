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
