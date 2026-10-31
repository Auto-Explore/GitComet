use super::*;
use gitcomet_core::identity::HistoryRefPreset;
use gitcomet_state::model::{HiddenRefPrefix, HiddenRefPrefixError};

fn settings_window_for(cx: &mut gpui::VisualTestContext) -> gpui::WindowHandle<SettingsWindowView> {
    cx.update(|_, app| open_settings_window(app));
    cx.run_until_parked();
    cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window")
    })
}

fn draw_frames(cx: &mut gpui::VisualTestContext) {
    let handle = cx.update(|window, _| window.window_handle());
    let platform_window = cx.test_window(handle);
    for _ in 0..8 {
        if !platform_window.simulate_scheduled_frame() {
            break;
        }
        cx.run_until_parked();
    }
}

#[gpui::test]
fn a_window_starts_with_the_default_hidden_refs(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (_main, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    wait_for_store(cx, &store, "hidden refs bootstrapped", |state| {
        !state.history_hidden_refs.is_empty()
    });
    let filter = store.snapshot().history_hidden_refs.clone();
    for hidden in [
        "refs/t3/checkpoints/thread/turn/1",
        "refs/claude/checkpoint-1",
        "refs/pull/7/head",
        "refs/notes/commits",
    ] {
        assert!(filter.excludes(hidden), "{hidden}");
    }
    assert!(!filter.excludes("refs/heads/main"));
    assert!(!filter.excludes("refs/remotes/origin/main"));
}

#[gpui::test]
fn hidden_ref_settings_reach_the_store_and_rules_apply_only_on_enter(
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
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    let settings = settings_window_for(cx);
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings.deref(), cx);
    settings
        .update(&mut settings_cx, |settings, window, cx| {
            settings.select_category(SettingsCategory::GitLog, cx);
            settings.toggle_section(SettingsSection::GitLogHiddenRefs, cx);
            settings.set_hidden_ref_preset(&HistoryRefPreset::T3_CODE, false, cx);
            settings.hidden_ref_prefix_input.update(cx, |input, cx| {
                input.set_text("refs/review/", cx);
                window.focus(&input.focus_handle(), cx);
            });
        })
        .expect("edit hidden refs");
    wait_for_store(&mut settings_cx, &store, "t3 shown", |state| {
        !state.history_hidden_refs.is_empty() && !state.history_hidden_refs.excludes("refs/t3/x")
    });
    assert!(
        !store
            .snapshot()
            .history_hidden_refs
            .excludes("refs/review/1"),
        "a typed rule applies nothing before Enter"
    );

    settings_cx.update(|window, app| {
        window.activate();
        let _ = window.draw(app);
    });
    settings_cx.simulate_keystrokes("enter");
    wait_for_store(&mut settings_cx, &store, "custom rule applied", |state| {
        state.history_hidden_refs.excludes("refs/review/1")
    });
    settings
        .read_with(&settings_cx, |settings, cx| {
            assert_eq!(
                settings.hidden_ref_settings.custom_prefixes(),
                [HiddenRefPrefix::parse("refs/review").unwrap()]
            );
            assert_eq!(settings.hidden_ref_prefix_input.read(cx).text(), "");
            let saved = settings.preference_settings();
            assert_eq!(
                saved.history_hidden_ref_prefixes,
                Some(vec!["refs/review".to_string()])
            );
            assert_eq!(
                saved
                    .history_hidden_ref_presets
                    .as_ref()
                    .and_then(|presets| presets.get("t3-code").copied()),
                Some(false)
            );
        })
        .expect("rule saved");

    settings
        .update(&mut settings_cx, |settings, _, cx| {
            settings.remove_hidden_ref_prefix(&HiddenRefPrefix::parse("refs/review").unwrap(), cx);
        })
        .expect("remove rule");
    wait_for_store(&mut settings_cx, &store, "custom rule removed", |state| {
        !state.history_hidden_refs.excludes("refs/review/1")
    });
}

#[gpui::test]
fn an_invalid_custom_ref_prefix_shows_why_and_keeps_the_saved_rules(cx: &mut gpui::TestAppContext) {
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    settings.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::GitLog, cx);
        settings.toggle_section(SettingsSection::GitLogHiddenRefs, cx);
        settings.hidden_ref_prefix_input.update(cx, |input, cx| {
            input.set_text("refs/heads/wip", cx);
        });
        settings.add_hidden_ref_prefix(cx);
    });
    draw_frames(cx);
    assert!(
        cx.debug_bounds("settings_window_git_log_hidden_ref_error")
            .is_some()
    );
    settings.read_with(cx, |settings, cx| {
        assert_eq!(
            settings.hidden_ref_prefix_error,
            Some(HiddenRefPrefixError::OwnedNamespace)
        );
        assert!(settings.hidden_ref_settings.custom_prefixes().is_empty());
        assert_eq!(
            settings.hidden_ref_prefix_input.read(cx).text(),
            "refs/heads/wip",
            "the draft stays for correction"
        );
    });
}

#[gpui::test]
fn the_hidden_refs_section_lists_every_preset_when_expanded(cx: &mut gpui::TestAppContext) {
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    settings.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::GitLog, cx);
    });
    draw_frames(cx);
    assert!(
        cx.debug_bounds("settings_window_git_log_hidden_refs")
            .is_some()
    );
    assert!(
        cx.debug_bounds("settings_window_git_log_hidden_ref_preset_t3-code")
            .is_none(),
        "collapsed by default"
    );
    settings.update(cx, |settings, cx| {
        settings.toggle_section(SettingsSection::GitLogHiddenRefs, cx);
    });
    draw_frames(cx);
    for preset in gitcomet_core::identity::current().history_ref_presets() {
        // `debug_bounds` takes a static selector.
        let id = format!("settings_window_git_log_hidden_ref_preset_{}", preset.id).leak();
        assert!(cx.debug_bounds(id).is_some(), "{id}");
    }

    settings.update(cx, |settings, cx| {
        let mut off = settings.hidden_ref_settings.clone();
        off.enabled = false;
        settings.set_hidden_ref_settings(off, cx);
    });
    draw_frames(cx);
    assert!(
        cx.debug_bounds("settings_window_git_log_hidden_ref_preset_t3-code")
            .is_none(),
        "switched off, the presets are not offered"
    );
}

#[test]
fn hidden_refs_are_discoverable_in_settings_search() {
    for query in ["hidden refs", "checkpoints", "pull requests", "notes"] {
        assert!(SettingsCategory::GitLog.matches_query(query), "{query}");
    }
}

#[gpui::test]
fn hidden_ref_rows_explain_what_the_refs_are_and_why_to_hide_them(cx: &mut gpui::TestAppContext) {
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    settings.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::GitLog, cx);
        settings.toggle_section(SettingsSection::GitLogHiddenRefs, cx);
    });
    draw_frames(cx);

    for (selector, starts_with) in [
        (
            "settings_window_git_log_hidden_refs_enabled",
            "Refs that coding agents, review systems and Git's own bookkeeping",
        ),
        (
            "settings_window_git_log_hidden_ref_preset_t3-code",
            "Snapshots of the working tree T3 Code saves on every agent turn.\n",
        ),
        (
            "settings_window_git_log_hidden_ref_preset_github-pull-requests",
            "GitHub pull request heads and test merges, when fetched into this clone.\n",
        ),
    ] {
        let center = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
        crate::view::tooltip::clear_visible_tooltip_text_for_test();
        crate::view::test_support::wait_for_native_tooltip(cx);
        let text = crate::view::tooltip::tooltip_text_for_test().unwrap_or_default();
        assert!(text.starts_with(starts_with), "{selector}: {text:?}");
    }
}
