use super::*;
use crate::view::large_file_tools_probe::settings_tool_probe_generation;
use gitcomet_core::large_file_tools::{LargeFileToolsState, ToolAvailability};

fn tools(version: &str) -> LargeFileToolsState {
    LargeFileToolsState {
        git_lfs: ToolAvailability::Available {
            version: Some(version.into()),
        },
        git_annex: ToolAvailability::Unknown,
    }
}

fn fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<GitCometView>,
    gpui::WindowHandle<SettingsWindowView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    crate::view::test_support::drain_store_worker(&main, cx);
    cx.update(|_, app| open_settings_window(app));
    cx.run_until_parked();
    let settings = cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    (main, settings, cx)
}

fn stored_tools(
    main: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> LargeFileToolsState {
    cx.run_until_parked();
    crate::view::test_support::drain_store_worker(main, cx);
    cx.update(|_, app| main.read(app).store.snapshot().large_file_tools.clone())
}

#[gpui::test]
fn large_file_probe_settings_completed_publication_survives_window_close(
    cx: &mut gpui::TestAppContext,
) {
    let (main, settings, cx) = fixture(cx);
    let worker_token = settings
        .update(cx, |settings, window, cx| {
            let token = settings.large_file_tools_cancellation.clone();
            settings.apply_large_file_tools_probe(tools("completed before close"), cx);
            // The detached broadcast has not run yet; this destroys its sender first.
            window.remove_window();
            token
        })
        .unwrap();
    assert!(
        worker_token.is_cancelled(),
        "Settings must actually have dropped"
    );
    assert_eq!(stored_tools(&main, cx), tools("completed before close"));
}

#[gpui::test]
fn large_file_probe_settings_forced_recheck_supersedes_queued_publication(
    cx: &mut gpui::TestAppContext,
) {
    let (main, settings, cx) = fixture(cx);
    settings
        .update(cx, |settings, _, cx| {
            settings.apply_large_file_tools_probe(tools("old completed request"), cx);
            crate::view::runtime_probe::request(cx, true);
        })
        .unwrap();
    assert_eq!(stored_tools(&main, cx), LargeFileToolsState::default());
    settings
        .update(cx, |settings, _, cx| {
            settings.apply_large_file_tools_probe(tools("new completed request"), cx);
        })
        .unwrap();
    assert_eq!(stored_tools(&main, cx), tools("new completed request"));
}

#[gpui::test]
fn large_file_probe_reopened_settings_supersedes_queued_publication(cx: &mut gpui::TestAppContext) {
    let (main, settings, cx) = fixture(cx);
    settings
        .update(cx, |settings, window, cx| {
            settings.apply_large_file_tools_probe(tools("closed Settings request"), cx);
            window.remove_window();
        })
        .unwrap();
    cx.update(|_, app| open_settings_window(app));
    let replacement = cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    replacement
        .update(cx, |settings, _, cx| {
            settings.refresh_large_file_tools(cx);
        })
        .unwrap();
    assert_eq!(stored_tools(&main, cx), LargeFileToolsState::default());
    replacement
        .update(cx, |settings, _, cx| {
            settings.apply_large_file_tools_probe(tools("reopened Settings request"), cx);
        })
        .unwrap();
    assert_eq!(stored_tools(&main, cx), tools("reopened Settings request"));
}

#[gpui::test]
fn large_file_probe_settings_return_to_same_runtime_rejects_old_completion(
    cx: &mut gpui::TestAppContext,
) {
    let (main, settings, cx) = fixture(cx);
    let old_token = settings
        .update(cx, |settings, _, cx| {
            let runtime_a = current_git_runtime();
            let generation = settings_tool_probe_generation(cx);
            let token = settings.large_file_tools_cancellation.clone();
            let mut runtime_b = runtime_a.clone();
            runtime_b.preference = GitExecutablePreference::Custom(PathBuf::from("fixture-B"));
            settings.sync_git_runtime_state(runtime_b, cx);
            settings.sync_git_runtime_state(runtime_a.clone(), cx);
            // Release the original worker only after A -> B -> A. Runtime value
            // equality cannot distinguish it from the replacement request.
            settings.apply_large_file_tools_probe_for_request(
                runtime_a,
                generation,
                tools("obsolete A worker"),
                cx,
            );
            token
        })
        .unwrap();
    assert_eq!(stored_tools(&main, cx), LargeFileToolsState::default());
    assert!(
        old_token.is_cancelled(),
        "Settings left the old worker running"
    );
    settings
        .update(cx, |settings, _, _| {
            assert_eq!(
                settings.runtime_info.large_file_tools,
                Some(LargeFileToolsState::default())
            );
        })
        .unwrap();
}

#[gpui::test]
fn large_file_probe_settings_generation_starts_fresh_for_each_app(cx: &mut gpui::TestAppContext) {
    assert_eq!(cx.update(|app| settings_tool_probe_generation(app)), 0);
    cx.update(|app| crate::view::runtime_probe::request(app, true));
    assert_eq!(cx.update(|app| settings_tool_probe_generation(app)), 1);
}

#[gpui::test]
fn large_file_probe_settings_close_cancels_while_entity_is_retained(cx: &mut gpui::TestAppContext) {
    let (main, settings, cx) = fixture(cx);
    let (retained, token, generation) = settings
        .update(cx, |settings, _, cx| {
            // Ordinary Settings tests intentionally do not spawn workers. This is
            // its actual cancellation token, used by the production worker.
            (
                cx.entity(),
                settings.large_file_tools_cancellation.clone(),
                settings_tool_probe_generation(cx),
            )
        })
        .unwrap();
    assert!(!token.is_cancelled());
    settings
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    assert!(
        token.is_cancelled(),
        "closed Settings left its worker token live"
    );
    cx.update(|_, app| {
        retained.update(app, |settings, cx| {
            settings.refresh_large_file_tools(cx);
            settings.apply_probed_runtime(current_git_runtime(), cx);
            settings.apply_large_file_tools_probe(tools("closed Settings completion"), cx);
            assert!(settings.large_file_tools_probe.is_none());
        });
        assert_eq!(
            settings_tool_probe_generation(app),
            generation,
            "closed Settings restarted discovery"
        );
    });
    assert_eq!(stored_tools(&main, cx), LargeFileToolsState::default());
}
