/// Force layout and paint even when only a scroll handle or test fixture changed.
pub(crate) fn refresh_and_draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
}

pub(crate) fn lock_clipboard_test() -> std::sync::MutexGuard<'static, ()> {
    static CLIPBOARD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    match CLIPBOARD_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn lock_visual_test() -> std::sync::MutexGuard<'static, ()> {
    static VISUAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    match VISUAL_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
