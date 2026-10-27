use super::*;

#[gpui::test]
fn automatic_fetch_blur_repaints_validation_without_another_input_event(
    cx: &mut gpui::TestAppContext,
) {
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    settings.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::Remotes, cx);
        settings.auto_fetch_interval_input.update(cx, |input, cx| {
            input.set_text("0", cx);
        });
        settings.commit_auto_fetch_interval(cx);
    });
    settings.update_in(cx, |settings, window, cx| {
        window.activate();
        window.focus(
            &settings.auto_fetch_interval_input.read(cx).focus_handle(),
            cx,
        );
    });
    cx.run_until_parked();
    let handle = cx.update(|window, _| window.window_handle());
    let platform_window = cx.test_window(handle);
    for _ in 0..8 {
        if !platform_window.simulate_scheduled_frame() {
            break;
        }
        cx.run_until_parked();
    }
    assert!(
        cx.debug_bounds("settings_window_auto_fetch_interval_error")
            .is_some()
    );

    settings.update_in(cx, |settings, window, cx| {
        settings.auto_fetch_interval_input.update(cx, |input, cx| {
            input.set_text("1440", cx);
        });
        window.focus(&settings.search_input.read(cx).focus_handle(), cx);
    });
    cx.run_until_parked();
    for _ in 0..8 {
        if !platform_window.simulate_scheduled_frame() {
            break;
        }
        cx.run_until_parked();
    }
    assert!(!platform_window.frame_scheduled());
    settings.read_with(cx, |settings, _| {
        assert_eq!(settings.auto_fetch.interval.minutes(), 1440);
        assert!(settings.auto_fetch_interval_error.is_none());
    });
    assert!(
        cx.debug_bounds("settings_window_auto_fetch_interval_error")
            .is_none()
    );
}

#[gpui::test]
fn automatic_fetch_interval_commits_on_enter_or_blur_and_rejects_invalid_values(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (_main, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| open_settings_window(app));
    cx.run_until_parked();
    let settings = cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window")
    });
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings.deref(), cx);
    settings
        .update(&mut settings_cx, |settings, window, cx| {
            settings.select_category(SettingsCategory::Remotes, cx);
            assert!(!settings.auto_fetch.enabled);
            assert_eq!(settings.auto_fetch.interval.minutes(), 30);
            settings.auto_fetch_interval_input.update(cx, |input, cx| {
                input.set_text("7", cx);
                window.focus(&input.focus_handle(), cx);
            });
        })
        .expect("edit interval");
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        window.activate();
        let _ = window.draw(app);
    });
    settings
        .read_with(&settings_cx, |settings, _| {
            assert_eq!(settings.auto_fetch.interval.minutes(), 30);
        })
        .expect("uncommitted interval");
    settings_cx.simulate_keystrokes("enter");
    settings_cx.run_until_parked();
    settings
        .read_with(&settings_cx, |settings, _| {
            assert_eq!(settings.auto_fetch.interval.minutes(), 7);
            assert!(settings.auto_fetch_interval_error.is_none());
        })
        .expect("committed interval");

    for invalid in ["0", "4", "1441", "1.5", "", "minutes"] {
        settings
            .update(&mut settings_cx, |settings, window, cx| {
                settings.auto_fetch_interval_input.update(cx, |input, cx| {
                    input.set_text(invalid, cx);
                    window.focus(&input.focus_handle(), cx);
                });
            })
            .expect("edit invalid interval");
        settings_cx.run_until_parked();
        settings_cx.simulate_keystrokes("enter");
        settings_cx.run_until_parked();
        settings
            .read_with(&settings_cx, |settings, _| {
                assert_eq!(settings.auto_fetch.interval.minutes(), 7);
                assert_eq!(
                    settings.auto_fetch_interval_error,
                    Some("Enter a whole number from 5 to 1,440 minutes.")
                );
                assert_eq!(
                    settings
                        .preference_settings()
                        .auto_fetch_interval_minutes
                        .map(AutoFetchInterval::minutes),
                    Some(7)
                );
            })
            .expect("invalid draft preserves saved interval");
    }

    settings
        .update(&mut settings_cx, |settings, _, cx| {
            settings.auto_fetch_interval_input.update(cx, |input, cx| {
                input.set_text("1440", cx);
            });
        })
        .expect("edit interval before blur");
    settings_cx.run_until_parked();
    settings
        .update(&mut settings_cx, |settings, window, cx| {
            window.focus(&settings.search_input.read(cx).focus_handle(), cx);
        })
        .expect("blur interval");
    settings_cx.run_until_parked();
    settings
        .read_with(&settings_cx, |settings, _| {
            assert_eq!(settings.auto_fetch.interval.minutes(), 1440);
            assert!(!settings.auto_fetch.enabled);
            assert!(settings.auto_fetch_interval_error.is_none());
        })
        .expect("blur commits valid interval while disabled");
}

#[gpui::test]
fn automatic_fetch_preferences_reach_every_window_and_preserve_pruning(
    cx: &mut gpui::TestAppContext,
) {
    let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
    let (_, first_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(first_store.clone(), first_events, None, window, cx)
    });
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window")
    });
    let (second_store, second_events) = AppStore::new_test(Arc::new(TestBackend));
    let (_, second_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(second_store.clone(), second_events, None, window, cx)
    });
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings.deref(), second_cx);
    settings
        .update(&mut settings_cx, |settings, _, cx| {
            settings.set_prune_deleted_remote_branches_on_fetch(false, cx);
            settings.auto_fetch_interval_input.update(cx, |input, cx| {
                input.set_text("9", cx);
            });
            settings.commit_auto_fetch_interval(cx);
            settings.set_auto_fetch_enabled(true, cx);
        })
        .expect("change remote preferences");
    settings_cx.run_until_parked();
    for store in [&first_store, &second_store] {
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .expect("remote preferences reduced");
        let snapshot = store.snapshot();
        assert!(snapshot.remote_settings.auto_fetch.enabled);
        assert_eq!(snapshot.remote_settings.auto_fetch.interval.minutes(), 9);
        assert!(
            !snapshot
                .remote_settings
                .prune_deleted_remote_branches_on_fetch
        );
    }
    settings
        .update(&mut settings_cx, |settings, _, cx| {
            settings.set_auto_fetch_enabled(false, cx);
            settings.set_prune_deleted_remote_branches_on_fetch(true, cx);
        })
        .expect("disable automatic fetch and change pruning");
    settings_cx.run_until_parked();
    for store in [&first_store, &second_store] {
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .expect("disabled preference reduced");
        let snapshot = store.snapshot();
        assert!(!snapshot.remote_settings.auto_fetch.enabled);
        assert_eq!(snapshot.remote_settings.auto_fetch.interval.minutes(), 9);
        assert!(
            snapshot
                .remote_settings
                .prune_deleted_remote_branches_on_fetch
        );
    }
}

#[test]
fn automatic_fetch_is_discoverable_in_settings_search() {
    for query in ["automatic fetch", "background", "interval", "minutes"] {
        assert!(SettingsCategory::Remotes.matches_query(query), "{query}");
    }
}
