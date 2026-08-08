use super::*;

#[gpui::test]
fn submodule_add_advanced_toggle_keeps_its_geometry_when_focused(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::submodule(RepoId(1), SubmodulePopoverKind::AddPrompt),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    let unfocused = cx
        .debug_bounds("submodule_add_advanced_toggle")
        .expect("expected the Advanced toggle before focus");
    let unfocused_label = cx
        .debug_bounds("submodule_add_advanced_label")
        .expect("expected the Advanced label before focus");

    cx.update(|window, app| {
        let focus_handle = view
            .read(app)
            .popover_host
            .read(app)
            .submodule_advanced_focus_handle
            .clone();
        window.focus(&focus_handle, app);
        let _ = window.draw(app);
    });

    let focused = cx
        .debug_bounds("submodule_add_advanced_toggle")
        .expect("expected the Advanced toggle after focus");
    let focused_label = cx
        .debug_bounds("submodule_add_advanced_label")
        .expect("expected the Advanced label after focus");
    assert_eq!(
        focused.size, unfocused.size,
        "the reserved focus border must keep the Advanced row geometry stable",
    );
    assert_eq!(
        focused_label, unfocused_label,
        "the reserved focus border must keep the Advanced label in place",
    );
}

#[gpui::test]
fn submodule_add_popover_tabs_through_advanced_fields_and_wraps(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::submodule(RepoId(1), SubmodulePopoverKind::AddPrompt),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_url_input.read(app).focus_handle(),
            "expected submodule add to focus the URL input first",
        );
    });

    // Fill the required fields so the Add button is enabled and participates
    // in the tab order.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.submodule_url_input.update(cx, |input, cx| {
                    input.set_text("https://example.com/org/repo.git", cx);
                });
                host.submodule_path_input.update(cx, |input, cx| {
                    input.set_text("vendor/repo", cx);
                });
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_path_input.read(app).focus_handle(),
            "expected Tab to move from submodule URL to path",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_branch_input.read(app).focus_handle(),
            "expected Tab to move from path to branch",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_advanced_focus_handle.clone(),
            "expected Tab to move from branch to Advanced",
        );
    });

    simulate_key_press(cx, "enter");
    cx.update(|window, app| {
        let _ = window.draw(app);
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.submodule_add_advanced_expanded,
            "expected Enter to expand Advanced fields"
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_name_input.read(app).focus_handle(),
            "expected Tab to move from Advanced to the logical name input",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_force_focus_handle.clone(),
            "expected Tab to move from the logical name input to Force reuse",
        );
    });

    simulate_key_press(cx, "space");
    cx.update(|window, app| {
        let _ = window.draw(app);
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.submodule_force_enabled,
            "expected Space to toggle Force reuse on"
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_focus.cancel.clone(),
            "expected Tab to move from Force reuse to Cancel",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_focus.submit.clone(),
            "expected Tab to move from Cancel to Add",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_url_input.read(app).focus_handle(),
            "expected Tab to wrap from Add back to URL",
        );
    });

    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.submodule_focus.submit.clone(),
            "expected Shift-Tab to wrap from URL back to Add",
        );
    });
}

#[gpui::test]
fn submodule_add_popover_escape_closes_from_advanced_toggle(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::submodule(RepoId(1), SubmodulePopoverKind::AddPrompt),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                window.focus(&host.submodule_advanced_focus_handle, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    simulate_key_press(cx, "escape");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        !is_open,
        "expected Escape to close submodule popover from Advanced"
    );
}
