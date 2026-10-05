//! Shared Git discovery. All process creation and waiting happens on workers.
use super::*;
use gitcomet_core::process::{
    GitRuntimeState, begin_git_runtime_probe, current_git_runtime, wait_for_git_runtime_probe,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Longer than the probe's own `git --version` timeout.
const ADOPT_TIMEOUT: Duration = Duration::from_secs(10);

/// Tasks that will report a probe result to the windows.
static REPORTERS: AtomicUsize = AtomicUsize::new(0);

/// A pending report visits every window that exists when the result arrives.
/// A routine request (a new window, an activation while Git is missing) leaves
/// the work to it: two reporters would both see stores still `Checking`, so
/// both would dispatch and re-check signing tools. A forced request (Settings)
/// always reports.
struct Reporter;

impl Reporter {
    fn claim(force: bool) -> Option<Self> {
        if !force && REPORTERS.load(Ordering::Acquire) > 0 {
            return None;
        }
        REPORTERS.fetch_add(1, Ordering::AcqRel);
        Some(Self)
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        REPORTERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Whether a report refreshes a store and its signing tools. A started or
/// forced probe refreshes every store; an adopted result for a routine request
/// fills in only the stores that missed it.
fn reports_to_store(refresh_all: bool, store: &GitRuntimeState, runtime: &GitRuntimeState) -> bool {
    refresh_all || store != runtime
}

pub(super) fn request(cx: &mut gpui::App, force: bool) {
    if cfg!(test) {
        return;
    }
    let Some(reporter) = Reporter::claim(force) else {
        return;
    };
    let (work, adopted) = match begin_git_runtime_probe(force) {
        Some(probe) => (
            cx.background_executor().spawn(async move { probe.run() }),
            false,
        ),
        // Nothing to start: a probe is running or has settled. The browser
        // probes before its first window exists, and that probe reports to no
        // store, so a window created meanwhile adopts its result.
        None => (
            cx.background_executor()
                .spawn(async move { wait_for_git_runtime_probe(ADOPT_TIMEOUT) }),
            true,
        ),
    };
    let refresh_all = force || !adopted;
    cx.spawn(async move |cx| {
        let _reporter = reporter;
        let Some(runtime) = work.await else {
            return;
        };
        cx.update(|cx| {
            // A path may have changed between worker publication and this UI turn.
            if current_git_runtime() != runtime {
                return;
            }
            crate::environment::refresh_git(cx);
            for handle in cx.windows() {
                if let Some(handle) = handle.downcast::<GitCometView>() {
                    let _ = handle.update(cx, |view, _, cx| {
                        if !reports_to_store(
                            refresh_all,
                            &view.store.snapshot().git_runtime,
                            &runtime,
                        ) {
                            return;
                        }
                        view.store
                            .dispatch(Msg::SetGitRuntimeState(runtime.clone()));
                        view.refresh_signing_tools(true, cx);
                    });
                } else if refresh_all
                    && let Some(handle) = handle.downcast::<settings_window::SettingsWindowView>()
                {
                    let _ = handle.update(cx, |view, _, cx| {
                        view.apply_probed_runtime(runtime.clone(), cx);
                    });
                }
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference};

    #[test]
    fn routine_requests_leave_reporting_to_a_pending_report() {
        let routine = Reporter::claim(false).expect("first report");
        assert!(
            Reporter::claim(false).is_none(),
            "a second routine report would dispatch twice"
        );
        let forced = Reporter::claim(true).expect("Settings always reports");
        drop(routine);
        assert!(
            Reporter::claim(false).is_none(),
            "the forced report is still pending"
        );
        drop(forced);
        assert!(
            Reporter::claim(false).is_some(),
            "a settled report releases the claim"
        );
    }

    #[test]
    fn adopted_results_fill_in_only_stores_that_missed_them() {
        let state = |availability| GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability,
        };
        let checking = state(GitExecutableAvailability::Checking);
        let missing = state(GitExecutableAvailability::Unavailable {
            detail: "missing".into(),
        });
        assert!(reports_to_store(false, &checking, &missing));
        assert!(!reports_to_store(false, &missing, &missing));
        assert!(
            reports_to_store(true, &missing, &missing),
            "a recheck refreshes signing tools"
        );
    }
}
