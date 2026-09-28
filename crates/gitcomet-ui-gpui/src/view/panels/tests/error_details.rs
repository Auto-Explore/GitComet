//! Errors as toasts that stay until closed, and the details dialog behind
//! their Show button.

use super::*;

const PUSH_FAILED: &str =
    "Push failed:\n\n    git push origin main\n\n     ! [rejected] main -> main (fetch first)";

fn open_view(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        // Escape for dialogs is bound with the text-input keys.
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
        window.activate_window();
    });
    (view, cx)
}

fn report(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>, message: &str) {
    let report = ErrorReport::message(None, message);
    cx.update(|_, app| view.update(app, |this, cx| this.report_error(report, cx)));
    draw_and_drain_test_window(cx);
}

/// `(toast id, message, count)`, newest first.
fn errors(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
) -> Vec<(u64, String, u32)> {
    cx.update(|_, app| {
        view.read(app)
            .toast_host
            .read(app)
            .error_notices()
            .into_iter()
            .map(|(id, notice)| (id, notice.message.clone(), notice.count))
            .collect()
    })
}

fn popover(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>) -> Option<PopoverKind> {
    cx.update(|_, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

/// Test-only selectors are built at runtime; `debug_bounds` wants `'static`.
fn selector(name: impl Into<String>) -> &'static str {
    Box::leak(name.into().into_boxed_str())
}

fn click(cx: &mut gpui::VisualTestContext, name: impl Into<String>) {
    let name = selector(name);
    let bounds = cx
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{name} must be drawn"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);
}

#[gpui::test]
fn an_error_stays_merges_repeats_and_shows_its_details(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = open_view(cx);
    report(cx, &view, PUSH_FAILED);
    report(cx, &view, PUSH_FAILED);
    let [(id, message, count)] = errors(cx, &view).try_into().expect("one error toast");
    assert_eq!(
        (message.as_str(), count),
        (PUSH_FAILED, 2),
        "a repeat bumps, never stacks"
    );
    assert!(
        cx.debug_bounds(selector(format!("toast_error_count_{id}")))
            .is_some()
    );

    click(cx, format!("toast_error_show_{id}"));
    assert_eq!(
        popover(cx, &view),
        Some(PopoverKind::ErrorDetails { toast_id: id })
    );
    let command = cx.update(|_, app| {
        view.read(app)
            .popover_host
            .read(app)
            .error_details_text_for_test(&format!("{id}_command"))
            .map(|input| input.read(app).text().to_string())
    });
    assert_eq!(command.as_deref(), Some("git push origin main"));
    assert!(
        cx.debug_bounds("error_details_rail").is_none(),
        "one error needs no list"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert_eq!(popover(cx, &view), None, "Escape closes the dialog");
    assert_eq!(
        errors(cx, &view).len(),
        1,
        "closing the dialog keeps the error"
    );

    click(cx, format!("toast_error_show_{id}"));
    click(cx, "error_details_dismiss");
    assert!(errors(cx, &view).is_empty());
    assert_eq!(popover(cx, &view), None, "nothing left to show");
}

#[gpui::test]
fn errors_pushed_out_of_the_stack_stay_reachable(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = open_view(cx);
    for ix in 0..5 {
        report(cx, &view, &format!("Fetch {ix} failed"));
    }
    assert_eq!(errors(cx, &view).len(), 5);
    let newest = errors(cx, &view)[0].0;
    assert!(
        cx.debug_bounds(selector(format!("toast_error_show_{newest}")))
            .is_some()
    );
    let oldest = errors(cx, &view)[4].0;
    assert!(
        cx.debug_bounds(selector(format!("toast_error_show_{oldest}")))
            .is_none(),
        "the stack shows three"
    );

    click(cx, "toast_more_errors");
    assert!(matches!(
        popover(cx, &view),
        Some(PopoverKind::ErrorDetails { .. })
    ));
    assert!(cx.debug_bounds("error_details_rail").is_some());

    // Picking another error in the list shows it.
    click(cx, format!("error_details_row_{oldest}"));
    let summary = cx.update(|_, app| {
        view.read(app)
            .popover_host
            .read(app)
            .error_details_text_for_test(&format!("{oldest}_summary"))
            .map(|input| input.read(app).text().to_string())
    });
    assert_eq!(summary.as_deref(), Some("Fetch 0 failed"));

    // Dismissing one keeps the dialog on the rest.
    click(cx, "error_details_dismiss");
    assert_eq!(errors(cx, &view).len(), 4);
    assert!(matches!(
        popover(cx, &view),
        Some(PopoverKind::ErrorDetails { .. })
    ));

    click(cx, "error_details_dismiss_all");
    assert!(errors(cx, &view).is_empty());
    assert_eq!(popover(cx, &view), None);
}

#[gpui::test]
fn closing_a_repo_drops_its_errors_only(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = open_view(cx);
    let (first_id, second_id) = (RepoId(9601), RepoId(9602));
    let mut first = opening_repo_state(first_id, Path::new("/tmp/error-repo-a"));
    first.open = Loadable::Ready(());
    let mut second = opening_repo_state(second_id, Path::new("/tmp/error-repo-b"));
    second.open = Loadable::Ready(());
    super::shortcuts::apply_state(
        cx,
        &view,
        Arc::new(AppState {
            repos: vec![first.clone(), second],
            active_repo: Some(first_id),
            ..AppState::test_default()
        }),
    );
    for (repo_id, message) in [
        (Some(first_id), "first repo failed"),
        (Some(second_id), "second repo failed"),
        (None, "app failed"),
    ] {
        let report = ErrorReport::message(repo_id, message);
        cx.update(|_, app| view.update(app, |this, cx| this.report_error(report, cx)));
    }
    super::shortcuts::apply_state(
        cx,
        &view,
        Arc::new(AppState {
            repos: vec![first],
            active_repo: Some(first_id),
            ..AppState::test_default()
        }),
    );
    let mut left = errors(cx, &view)
        .into_iter()
        .map(|(_, message, _)| message)
        .collect::<Vec<_>>();
    left.sort();
    assert_eq!(left, vec!["app failed", "first repo failed"]);
}
