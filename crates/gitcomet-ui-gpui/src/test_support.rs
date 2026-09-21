/// Force layout and paint even when only a scroll handle or test fixture changed.
pub(crate) fn refresh_and_draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
}

/// Keep the lock alive for the entire measured hold interval. Timing is opt-in
/// and uses the same per-test records as fixture and UI wait diagnostics.
pub(crate) struct TestLockGuard {
    _hold: gitcomet_core::test_support::git_fixture::FixtureTimer,
    _guard: std::sync::MutexGuard<'static, ()>,
}

fn lock_test(lock: &'static std::sync::Mutex<()>, name: &str) -> TestLockGuard {
    use gitcomet_core::test_support::git_fixture::FixtureTimer;
    let wait = FixtureTimer::new("ui-lock-wait", name);
    let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(wait);
    TestLockGuard {
        _hold: FixtureTimer::new("ui-lock-held", name),
        _guard: guard,
    }
}

pub(crate) fn lock_clipboard_test() -> TestLockGuard {
    static CLIPBOARD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    lock_test(&CLIPBOARD_TEST_LOCK, "clipboard")
}

pub(crate) fn lock_visual_test() -> TestLockGuard {
    static VISUAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    lock_test(&VISUAL_TEST_LOCK, "visual")
}

/// Compare the real fill and border quad, rather than inspecting which style
/// methods the renderer called. Text and independently painted inset rings are
/// intentionally outside the background assertion.
pub(crate) fn painted_control_quads(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> Vec<(gpui::Background, gpui::Background)> {
    let bounds = cx.debug_bounds(selector).expect("control must be drawn");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                let rect = quad.bounds;
                (rect.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 1.0
                    && (rect.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 1.0
                    && (rect.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.0
                    && (rect.size.height.0 - f32::from(bounds.size.height) * scale).abs() < 1.0
            })
            .map(|quad| (quad.background, quad.border_color))
            .collect()
    })
}
