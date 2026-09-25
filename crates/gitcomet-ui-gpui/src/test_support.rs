/// Force layout and paint even when only a scroll handle or test fixture changed.
pub(crate) fn refresh_and_draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
}

/// Every `.rs` file below `dir`, for guards that scan the crate's own source.
pub(crate) fn rust_sources_under(dir: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            rust_sources_under(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
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
