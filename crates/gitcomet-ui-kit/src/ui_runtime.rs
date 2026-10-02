//! Runtime policy: whether timers, animations, background work, and
//! persistence run live or deterministically.
//!
//! The policy is explicit rather than tied to `cfg(test)`, which a dependency
//! never sees: the application installs [`UiRuntime::live`] at launch, and
//! everything else — this kit's tests, a host's tests, benchmarks — runs
//! deterministically unless it installs or overrides a policy.

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiRuntimeMode {
    Live,
    Deterministic,
    /// Deterministic, but restoring saved sessions like the live app.
    DeterministicAutoRestore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiRuntime {
    mode: UiRuntimeMode,
}

impl UiRuntime {
    pub const fn live() -> Self {
        Self {
            mode: UiRuntimeMode::Live,
        }
    }

    pub const fn deterministic() -> Self {
        Self {
            mode: UiRuntimeMode::Deterministic,
        }
    }

    pub const fn deterministic_auto_restore() -> Self {
        Self {
            mode: UiRuntimeMode::DeterministicAutoRestore,
        }
    }

    pub const fn mode(self) -> UiRuntimeMode {
        self.mode
    }

    pub const fn uses_live_store_poller(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn uses_background_compute(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn uses_tooltip_delay(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn uses_toast_ttl(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    /// Repaints the elapsed time on progress cards once a second.
    pub const fn uses_progress_ticker(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn uses_cursor_blink(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn uses_pane_animations(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn uses_repo_tab_spinner_delay(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn persists_ui_settings(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    /// Copies probe the platform clipboard (WSLg's X11 bridge) and leave a
    /// crash diagnostic; otherwise they use GPUI's clipboard only.
    pub const fn uses_clipboard_diagnostics(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    /// Opening a URL or file starts its default application from a worker
    /// thread; otherwise the request goes to GPUI's platform, which a test
    /// platform only records.
    pub const fn launches_applications(self) -> bool {
        matches!(self.mode, UiRuntimeMode::Live)
    }

    pub const fn auto_restores_session(self) -> bool {
        match self.mode {
            UiRuntimeMode::Live | UiRuntimeMode::DeterministicAutoRestore => true,
            UiRuntimeMode::Deterministic => false,
        }
    }

    pub const fn diff_syntax_foreground_parse_budget(self) -> Duration {
        match self.mode {
            UiRuntimeMode::Live => Duration::from_millis(1),
            UiRuntimeMode::Deterministic | UiRuntimeMode::DeterministicAutoRestore => {
                Duration::from_millis(2)
            }
        }
    }

    const fn code(self) -> u8 {
        match self.mode {
            UiRuntimeMode::Live => 1,
            UiRuntimeMode::Deterministic => 2,
            UiRuntimeMode::DeterministicAutoRestore => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::live()),
            2 => Some(Self::deterministic()),
            3 => Some(Self::deterministic_auto_restore()),
            _ => None,
        }
    }
}

/// The process-wide policy; 0 means none was installed.
static INSTALLED: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static OVERRIDE: Cell<Option<UiRuntime>> = const { Cell::new(None) };
}

/// Sets the process-wide policy. The application calls this with
/// [`UiRuntime::live`] before opening its first window.
pub fn install(runtime: UiRuntime) {
    INSTALLED.store(runtime.code(), Ordering::Release);
}

/// This thread's override, else the installed policy, else deterministic.
pub fn current() -> UiRuntime {
    OVERRIDE.with(Cell::get).unwrap_or_else(|| {
        UiRuntime::from_code(INSTALLED.load(Ordering::Acquire))
            .unwrap_or_else(UiRuntime::deterministic)
    })
}

/// Runs `f` with `runtime` as this thread's policy.
pub fn with_override<T>(runtime: UiRuntime, f: impl FnOnce() -> T) -> T {
    OVERRIDE.with(|cell| {
        let prev = cell.replace(Some(runtime));
        let result = f();
        cell.set(prev);
        result
    })
}

/// Runs `compute` on a background thread, or inline when the runtime is
/// deterministic, then calls `apply` with the result inside a view update.
///
/// The `apply` closure owns the site's staleness checks (generation, repo,
/// revision) and any re-issue, so the snapshot/compute/stale/apply skeleton
/// stays in one place while each caller keeps its own guards.
pub fn run_background_compute<V, O, F, A>(
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

/// Filesystem and native payload jobs use the same deterministic test runtime
/// as other view computations; production work always runs off the UI thread.
pub async fn background_compute<O: Send + 'static>(
    compute: impl FnOnce() -> O + Send + 'static,
) -> O {
    if current().uses_background_compute() {
        smol::unblock(compute).await
    } else {
        compute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_override_wins_and_is_restored() {
        let before = current();
        with_override(UiRuntime::live(), || {
            assert_eq!(current(), UiRuntime::live());
            with_override(UiRuntime::deterministic_auto_restore(), || {
                assert!(current().auto_restores_session());
            });
            assert_eq!(current(), UiRuntime::live());
        });
        assert_eq!(current(), before);
    }

    #[test]
    fn codes_round_trip_every_mode() {
        for runtime in [
            UiRuntime::live(),
            UiRuntime::deterministic(),
            UiRuntime::deterministic_auto_restore(),
        ] {
            assert_eq!(UiRuntime::from_code(runtime.code()), Some(runtime));
        }
        assert_eq!(UiRuntime::from_code(0), None);
    }
}
