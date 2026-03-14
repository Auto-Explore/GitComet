pub(crate) fn lock_clipboard_test() -> std::sync::MutexGuard<'static, ()> {
    static CLIPBOARD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    match CLIPBOARD_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
