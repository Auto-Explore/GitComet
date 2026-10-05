#![cfg(feature = "test-support")]

use gpui::{App, AppContext, Context, Render, TestAppContext, Window, WindowOptions, div};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CleanupCounts {
    test_quits: usize,
    app_quits: usize,
    window_drops: usize,
}

thread_local! {
    static COUNTS: Cell<CleanupCounts> = Cell::default();
}

fn record(update: impl FnOnce(&mut CleanupCounts)) {
    COUNTS.with(|counts| {
        let mut value = counts.get();
        update(&mut value);
        counts.set(value);
    });
}

struct WindowRoot;

impl Render for WindowRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
    }
}

impl Drop for WindowRoot {
    fn drop(&mut self) {
        record(|counts| counts.window_drops += 1);
    }
}

fn observe_app_quit(cx: &App) {
    cx.on_app_quit(|_| async { record(|counts| counts.app_quits += 1) })
        .detach();
}

fn retain_visual_context(cx: &mut TestAppContext) {
    cx.on_quit(|| record(|counts| counts.test_quits += 1));
    cx.update(|cx| observe_app_quit(cx));
    // add_window_view retains a context until TestAppContext::quit runs.
    let _ = cx.add_window_view(|_, _| WindowRoot);
}

// These generated functions are also called below so the assertions run after
// the macro has torn down its contexts, on the same thread as the callbacks.
#[gpui::test]
fn sync_context(cx: &mut TestAppContext) {
    retain_visual_context(cx);
}

#[gpui::test]
async fn async_context(cx: &mut TestAppContext) {
    retain_visual_context(cx);
}

#[gpui::test]
fn app_reference(cx: &mut App) {
    observe_app_quit(cx);
    cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| WindowRoot))
        .unwrap();
}

fn assert_cleanup(test: fn(), test_quits: usize) {
    COUNTS.set(CleanupCounts::default());
    test();
    assert_eq!(
        COUNTS.get(),
        CleanupCounts {
            test_quits,
            app_quits: 1,
            window_drops: 1,
        },
        "the test macro must run cleanup exactly once and release its windows",
    );
}

#[test]
fn sync_test_context_runs_cleanup_and_releases_windows() {
    assert_cleanup(sync_context, 1);
}

#[test]
fn async_test_context_runs_cleanup_and_releases_windows() {
    assert_cleanup(async_context, 1);
}

#[test]
fn app_reference_runs_cleanup_and_releases_windows() {
    assert_cleanup(app_reference, 0);
}
