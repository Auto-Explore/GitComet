use super::*;

fn open_app_menu(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::AppMenu,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (view, cx)
}

#[gpui::test]
fn app_menu_offers_close_window_rather_than_a_dismiss_entry(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    assert!(
        cx.debug_bounds("app_menu_close_window").is_some(),
        "app menu should offer Close Window"
    );
    assert!(
        cx.debug_bounds("app_menu_quit").is_some(),
        "app menu should offer Quit"
    );
    assert!(
        cx.debug_bounds("app_menu_close").is_none(),
        "the old popover-dismiss entry should be gone"
    );
}

#[gpui::test]
fn app_menu_hides_desktop_integration_on_unsupported_platforms(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    let entry = cx.debug_bounds("app_menu_install_desktop");
    if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        assert!(
            entry.is_some(),
            "desktop integration should be offered where it is implemented"
        );
    } else {
        assert!(
            entry.is_none(),
            "desktop integration should not render where it cannot run"
        );
    }
}

#[gpui::test]
fn app_menu_shortcuts_use_command_palette_keycaps(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_app_menu(cx);

    assert!(
        cx.debug_bounds("shortcut_keycaps").is_some(),
        "App-menu shortcuts should use the shared Command Palette keycap badges"
    );
}

#[gpui::test]
fn app_menu_owns_keyboard_focus_and_escape_dismisses_it(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_app_menu(cx);

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.context_menu_focus_handle.is_focused(window),
            "opening the app menu should move keyboard focus into the menu"
        );
        assert_eq!(
            host.context_menu_selected_ix,
            Some(1),
            "the first selectable row follows the Application header"
        );
    });

    simulate_key_press(cx, "tab");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .popover_host
                .read(app)
                .context_menu_selected_ix,
            Some(2),
            "Tab should select Settings"
        );
    });

    simulate_key_press(cx, "shift-tab");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .popover_host
                .read(app)
                .context_menu_selected_ix,
            Some(1),
            "Shift+Tab should select Command Palette"
        );
    });

    simulate_key_press(cx, "escape");
    cx.update(|_window, app| {
        assert!(
            !view.read(app).popover_host.read(app).is_open(),
            "Escape should dismiss the app menu"
        );
    });
}
