use super::*;

#[gpui::test]
fn popover_is_clickable_above_content(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_worktree_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-popover-click-test"),
    );

    // Open the repo picker dropdown in the action bar, which should overlay the rest of the UI.
    let picker_bounds = cx
        .debug_bounds("repo_picker_toggle")
        .expect("expected repo_picker_toggle in debug bounds");
    cx.simulate_mouse_move(picker_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let popover_bounds = cx
        .debug_bounds("app_popover")
        .expect("expected repository picker popover bounds");
    assert_eq!(
        popover_bounds.left(),
        picker_bounds.left(),
        "expected repository picker to stay aligned to the chevron's left edge"
    );
    assert_eq!(
        popover_bounds.top(),
        picker_bounds.bottom() + px(1.0),
        "expected repository picker to stay attached below the chevron"
    );

    let close_bounds = cx
        .debug_bounds("repo_popover_close")
        .expect("expected repo_popover_close in debug bounds");
    cx.simulate_mouse_move(close_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        close_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        close_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected popover to close on click"
        );
    });
}

#[gpui::test]
fn popover_closes_when_clicking_outside(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_worktree_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-popover-outside-test"),
    );

    let picker_bounds = cx
        .debug_bounds("repo_picker_toggle")
        .expect("expected repo_picker_toggle in debug bounds");
    cx.simulate_mouse_move(picker_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "expected popover to open"
        );
    });

    // Click somewhere in the main content area (outside the popover).
    let outside = gpui::point(px(900.0), px(700.0));
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected popover to close when clicking outside"
        );
    });
}

#[gpui::test]
fn titlebar_hamburger_opens_app_menu(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_worktree_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-titlebar-menu-test"),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let menu_bounds = cx
        .debug_bounds("app_menu")
        .expect("expected app menu hamburger bounds");
    cx.simulate_mouse_move(menu_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        menu_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        menu_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "expected hamburger click to open the app menu"
        );
    });
}

#[gpui::test]
fn titlebar_hamburger_opens_from_keyboard_and_restores_focus_on_escape(
    cx: &mut gpui::TestAppContext,
) {
    if cfg!(target_os = "macos") {
        return;
    }
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_worktree_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-titlebar-menu-keyboard-test"),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let app_menu_focus = cx.update(|_window, app| {
        crate::view::test_support::app_menu_focus_handle(view.read(app), app)
    });
    cx.update(|window, app| window.focus(&app_menu_focus, app));

    simulate_key_press(cx, "enter");
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "Enter on the shared titlebar Button should open the app menu"
        );
    });

    simulate_key_press(cx, "escape");
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "Escape should close the keyboard-opened app menu"
        );
        assert!(
            app_menu_focus.is_focused(window),
            "closing the app menu should restore focus to its shared Button"
        );
    });
}

#[gpui::test]
fn titlebar_window_controls_update_tooltip_on_hover(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        // The custom Min/Max/Close controls are only rendered on non-macOS.
        return;
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let min_bounds = cx
        .debug_bounds("titlebar_win_min")
        .expect("expected titlebar min control bounds");
    cx.simulate_mouse_move(min_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Minimize window".into())
    );

    let max_bounds = cx
        .debug_bounds("titlebar_win_max")
        .expect("expected titlebar max control bounds");
    let expected_max = cx.update(|window, _app| {
        if window.is_maximized() {
            "Restore window".into()
        } else {
            "Maximize window".into()
        }
    });
    cx.simulate_mouse_move(max_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some(expected_max)
    );

    let close_bounds = cx
        .debug_bounds("titlebar_win_close")
        .expect("expected titlebar close control bounds");
    cx.simulate_mouse_move(close_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Close window".into())
    );

    cx.simulate_mouse_move(gpui::point(px(120.0), px(18.0)), None, Modifiers::default());
    assert_eq!(crate::view::test_support::tooltip_text(cx, &view), None);
}

fn simulate_key_press(cx: &mut gpui::VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key)
        .unwrap_or_else(|err| panic!("failed to parse test keystroke `{key}`: {err}"))
        .with_simulated_ime();
    cx.update(|window, app| {
        let _ = window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            app,
        );
        let _ = window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }),
            app,
        );
    });
    cx.run_until_parked();
}
