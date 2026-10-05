/// Keep independent DX12 test devices from overlapping creation and teardown
/// on Windows. CPU tests still run in parallel, and runtime contexts are unchanged.
pub(crate) fn guard() -> Option<std::sync::MutexGuard<'static, ()>> {
    #[cfg(target_os = "windows")]
    {
        static GPU_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        Some(
            GPU_TESTS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
    #[cfg(not(target_os = "windows"))]
    None
}
