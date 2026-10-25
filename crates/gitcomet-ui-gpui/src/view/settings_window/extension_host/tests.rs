use super::*;

struct MenuExtension;

impl Extension for MenuExtension {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.settings-menu").unwrap()
    }

    fn register(&self, _: &mut Registrar) {}
}

#[gpui::test]
fn settings_menu_preserves_disabled_release_and_close_before_callback_ordering(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|app| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(MenuExtension)]).unwrap(),
            app,
        );
    });
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.run_until_parked();
    let previous_focus = cx.update(|window, app| {
        let focus = app.focus_handle();
        window.focus(&focus, app);
        focus
    });
    let calls = Rc::new(Cell::new(0));
    let action_calls = calls.clone();
    let parent = settings.clone();
    let window_handle = cx.window_handle();
    let expected_focus = previous_focus.clone();
    let action = HostedAction::new("Remove review", move |app| {
        assert!(parent.read(app).extension_dialog.is_none());
        window_handle
            .update(app, |_, window, _| {
                assert!(expected_focus.is_focused(window));
            })
            .unwrap();
        action_calls.set(action_calls.get() + 1);
    });
    let disabled_calls = calls.clone();
    cx.update(|_, app| {
        let host = settings.read(app).extension_window.as_ref().unwrap().host();
        host.open_menu(
            gpui::point(px(100.0), px(100.0)),
            vec![
                HostedMenuItem::Header("Review".into()),
                HostedMenuItem::Separator,
                HostedMenuItem::action(HostedAction::new("Disabled", move |_| {
                    disabled_calls.set(disabled_calls.get() + 1);
                }))
                .disabled(true),
                HostedMenuItem::action(action).with_icon("icons/terminal.svg"),
            ],
            app,
        )
        .unwrap();
    });
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    let disabled = cx
        .debug_bounds("settings_hosted_action_2")
        .unwrap()
        .center();
    let enabled = cx
        .debug_bounds("settings_hosted_action_3")
        .unwrap()
        .center();
    cx.simulate_click(disabled, gpui::Modifiers::default());
    assert_eq!(calls.get(), 0);
    cx.update(|_, app| assert!(settings.read(app).extension_dialog.is_some()));

    cx.simulate_mouse_down(enabled, gpui::MouseButton::Left, gpui::Modifiers::default());
    assert_eq!(calls.get(), 0);
    cx.update(|_, app| assert!(settings.read(app).extension_dialog.is_some()));
    cx.simulate_mouse_up(
        disabled,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    assert_eq!(calls.get(), 0);

    cx.simulate_click(enabled, gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    cx.update(|_, app| assert!(settings.read(app).extension_dialog.is_none()));
    cx.simulate_mouse_up(enabled, gpui::MouseButton::Left, gpui::Modifiers::default());
    assert_eq!(calls.get(), 1);
}

#[gpui::test]
fn settings_menu_entries_show_their_tooltips_on_hover(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(MenuExtension)]).unwrap(),
            app,
        );
    });
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.run_until_parked();
    cx.update(|_, app| {
        let host = settings.read(app).extension_window.as_ref().unwrap().host();
        host.open_menu(
            gpui::point(px(100.0), px(100.0)),
            vec![
                HostedMenuItem::action(HostedAction::new("Re-run failed jobs", |_| {}))
                    .with_tooltip("Runs the failed jobs again"),
                HostedMenuItem::action(HostedAction::new("Open run", |_| {})),
                HostedMenuItem::action(HostedAction::new("Cancel run", |_| {}))
                    .disabled(true)
                    .with_tooltip("The run has finished"),
            ],
            app,
        )
        .unwrap();
    });
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);

    for (selector, expected) in [
        (
            "settings_hosted_action_0",
            Some("Runs the failed jobs again"),
        ),
        ("settings_hosted_action_1", None),
        ("settings_hosted_action_2", Some("The run has finished")),
    ] {
        let center = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
        crate::view::tooltip::clear_visible_tooltip_text_for_test();
        crate::view::test_support::wait_for_native_tooltip(cx);
        assert_eq!(
            crate::view::tooltip::tooltip_text_for_test().as_deref(),
            expected,
            "{selector}"
        );
    }
}
