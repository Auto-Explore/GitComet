use super::*;

#[gpui::test]
fn operation_menu_dismissal_clears_invoker_and_restores_focus_consistently(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    for kind in [PopoverKind::PushPicker, PopoverKind::PullPicker] {
        for escape in [true, false] {
            let (store, events) = AppStore::new_test(Arc::new(TestBackend));
            let (view, cx) =
                cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
            let source = cx.update(|window, app| {
                let focus = app.focus_handle();
                window.focus(&focus, app);
                view.update(app, |view, cx| {
                    view.open_popover_at(
                        kind.clone().invoked_by("operation_menu".into()),
                        point(px(72.0), px(72.0)),
                        window,
                        cx,
                    )
                });
                focus
            });
            cx.run_until_parked();
            crate::test_support::refresh_and_draw(cx);
            cx.update(|_, app| {
                assert_eq!(
                    view.read(app).active_context_menu_invoker.as_deref(),
                    Some("operation_menu")
                )
            });
            if escape {
                cx.simulate_keystrokes("escape");
            } else {
                cx.simulate_click(point(px(2.0), px(2.0)), gpui::Modifiers::default());
            }
            cx.run_until_parked();
            cx.update(|window, app| {
                assert!(!view.read(app).popover_host.read(app).is_open());
                assert!(
                    source.is_focused(window),
                    "all operation menus restore their source"
                );
                assert_eq!(view.read(app).active_context_menu_invoker, None);
                assert!(!crate::view::tooltip::tooltips_suppressed_by_overlay(app));
            });
        }
    }
}

#[gpui::test]
fn replacing_menu_in_one_update_publishes_only_the_current_invoker(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.open_popover_at(
                PopoverKind::PushPicker.invoked_by("push".into()),
                point(px(72.0), px(72.0)),
                window,
                cx,
            );
            view.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
            view.open_popover_at(
                PopoverKind::PullPicker.invoked_by("pull".into()),
                point(px(72.0), px(72.0)),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).active_context_menu_invoker.as_deref(),
            Some("pull")
        );
        assert!(crate::view::tooltip::tooltips_suppressed_by_overlay(app));
    });
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.open_popover_at(
                PopoverKind::UiScalePicker,
                point(px(72.0), px(72.0)),
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| assert_eq!(view.read(app).active_context_menu_invoker, None));
}

#[gpui::test]
fn inline_prompt_dismissal_releases_tooltip_suppression(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |view, cx| {
            view.open_popover_at(
                PopoverKind::StashPrompt.invoked_by("stash".into()),
                point(px(72.0), px(72.0)),
                window,
                cx,
            )
        });
    });
    cx.run_until_parked();
    crate::test_support::refresh_and_draw(cx);
    cx.update(|_, app| assert!(crate::view::tooltip::tooltips_suppressed_by_overlay(app)));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(!view.read(app).popover_host.read(app).is_open());
        assert_eq!(view.read(app).active_context_menu_invoker, None);
        assert!(
            !crate::view::tooltip::tooltips_suppressed_by_overlay(app),
            "closing the prompt must let subsequent tooltips render"
        );
    });
}

#[gpui::test]
fn explicit_popover_focus_return_overrides_the_previously_focused_input(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    for escape in [false, true] {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let target = cx.update(|window, app| {
            let source = app.focus_handle();
            let target = app.focus_handle();
            window.focus(&source, app);
            view.update(app, |view, cx| {
                view.open_popover_at(
                    PopoverKind::PushPicker
                        .invoked_by("explicit_focus_menu".into())
                        .returning_focus_to(target.clone()),
                    point(px(72.0), px(72.0)),
                    window,
                    cx,
                )
            });
            target
        });
        cx.run_until_parked();
        crate::test_support::refresh_and_draw(cx);
        if escape {
            cx.simulate_keystrokes("escape");
        } else {
            cx.simulate_click(point(px(2.0), px(2.0)), gpui::Modifiers::default());
        }
        cx.run_until_parked();
        crate::test_support::refresh_and_draw(cx);
        cx.update(|window, app| {
            assert!(!view.read(app).popover_host.read(app).is_open());
            assert!(target.is_focused(window));
        });
    }
}
