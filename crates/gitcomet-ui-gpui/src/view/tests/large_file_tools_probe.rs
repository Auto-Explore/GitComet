use super::*;
use crate::view::large_file_tools_probe::{
    HeldLargeFileToolsDetector, LargeFileToolsProbeRequest, LargeFileToolsPublication,
    settings_tool_probe_generation,
};
use crate::view::preference_change::PreferenceChange;
use gitcomet_core::large_file_tools::{LargeFileToolsState, ToolAvailability};

fn runtime(name: &str) -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::Custom(PathBuf::from(name)),
        availability: GitExecutableAvailability::Available {
            version_output: "git version 2.50.0".into(),
        },
    }
}

fn tools(version: &str) -> LargeFileToolsState {
    LargeFileToolsState {
        git_lfs: ToolAvailability::Available {
            version: Some(version.into()),
        },
        git_annex: ToolAvailability::Unknown,
    }
}

fn unpresented_fixture(
    cx: &mut gpui::TestAppContext,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    store.dispatch(Msg::SetGitRuntimeState(runtime("A")));
    // A repository keeps these request-lifecycle fixtures eligible for automatic
    // discovery; the separate Home tests cover deferred startup.
    store.dispatch(Msg::OpenRepo(
        std::env::temp_dir().join("tool-probe-lifecycle"),
    ));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::drain_store_worker(&view, cx);
    view.update(cx, |view, cx| {
        view.large_file_tools_detector_for_test = Some(HeldLargeFileToolsDetector {
            runtime: runtime("A"),
            requests: Vec::new(),
        });
        view.apply_state_snapshot(view.store.snapshot(), cx);
    });
    assert_eq!(requests(&view, cx).len(), 0);
    (view, cx)
}

fn next_frame(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
}

fn fixture(cx: &mut gpui::TestAppContext) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (view, cx) = unpresented_fixture(cx);
    next_frame(cx);
    next_frame(cx);
    assert_eq!(requests(&view, cx).len(), 1);
    (view, cx)
}

fn requests(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<LargeFileToolsProbeRequest> {
    cx.update(|_, app| {
        view.read(app)
            .large_file_tools_detector_for_test
            .as_ref()
            .unwrap()
            .requests
            .clone()
    })
}

fn change_runtime(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    runtime: GitRuntimeState,
) {
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.large_file_tools_detector_for_test
                .as_mut()
                .unwrap()
                .runtime = runtime.clone();
            view.apply_preference_change(&PreferenceChange::GitRuntime(runtime), window, cx);
        });
    });
}

fn publish_store(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    test_support::drain_store_worker(view, cx);
    view.update(cx, |view, cx| {
        view.apply_state_snapshot(view.store.snapshot(), cx);
    });
}

fn accept_settings_tools(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    tools: LargeFileToolsState,
) {
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            let publication = LargeFileToolsPublication {
                runtime: view
                    .large_file_tools_detector_for_test
                    .as_ref()
                    .unwrap()
                    .runtime
                    .clone(),
                generation: settings_tool_probe_generation(cx),
                tools,
            };
            view.apply_preference_change(
                &PreferenceChange::LargeFileTools(publication),
                window,
                cx,
            );
        });
    });
}

fn stored_tools(
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> LargeFileToolsState {
    test_support::drain_store_worker(view, cx);
    cx.update(|_, app| view.read(app).store.snapshot().large_file_tools.clone())
}

#[gpui::test]
fn large_file_probe_waits_for_initial_platform_frame_without_another_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = unpresented_fixture(cx);
    publish_store(&view, cx);
    let handle = cx.update(|window, _| window.window_handle());
    let platform_window = cx.cx.test_window(handle);
    assert_eq!(platform_window.presents(), 0);
    assert!(requests(&view, cx).is_empty());
    platform_window.simulate_frame_request(gpui::RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(platform_window.presents(), 1);
    assert!(
        requests(&view, cx).is_empty(),
        "the first callback runs before the platform presents"
    );
    platform_window.simulate_frame_request(gpui::RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(requests(&view, cx).len(), 1);
    next_frame(cx);
    publish_store(&view, cx);
    assert_eq!(requests(&view, cx).len(), 1);
}

#[gpui::test]
fn large_file_probe_frame_gate_uses_the_latest_runtime(cx: &mut gpui::TestAppContext) {
    let (view, cx) = unpresented_fixture(cx);
    next_frame(cx);
    change_runtime(&view, cx, runtime("B"));
    publish_store(&view, cx);
    assert!(requests(&view, cx).is_empty());
    next_frame(cx);
    let pending = requests(&view, cx);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].runtime, runtime("B"));
}

#[gpui::test]
fn large_file_probe_settings_result_before_frame_prevents_automatic_work(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = unpresented_fixture(cx);
    let queued = cx.update(|_, app| view.read(app).state.clone());
    accept_settings_tools(&view, cx, tools("installed since startup"));
    // Even the stale NotChecked snapshot must not restart the settled runtime.
    view.update(cx, |view, cx| {
        view.apply_state_snapshot(queued, cx);
    });
    next_frame(cx);
    next_frame(cx);
    assert!(requests(&view, cx).is_empty());
    assert_eq!(stored_tools(&view, cx), tools("installed since startup"));
}

#[gpui::test]
fn large_file_probe_closed_owner_cannot_start_when_frame_gate_opens(cx: &mut gpui::TestAppContext) {
    let (view, cx) = unpresented_fixture(cx);
    next_frame(cx);
    view.update(cx, |view, _| view.close_large_file_tools_probe());
    next_frame(cx);
    publish_store(&view, cx);
    assert!(requests(&view, cx).is_empty());
}

#[gpui::test]
fn large_file_probe_frame_callbacks_do_not_retain_a_closed_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let window = cx.add_window(|window, cx| GitCometView::new(store, events, None, window, cx));
    let weak = window
        .update(cx, |_, window, cx| {
            let weak = cx.weak_entity();
            window.remove_window();
            weak
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|_| {});
    assert!(
        weak.upgrade().is_none(),
        "pending frame callbacks must not retain the closed view"
    );
}

#[gpui::test]
fn large_file_probe_runtime_change_cancels_the_owned_request(cx: &mut gpui::TestAppContext) {
    let (view, cx) = fixture(cx);
    let old = requests(&view, cx)[0].clone();
    change_runtime(&view, cx, runtime("B"));
    assert!(
        old.cancellation.is_cancelled(),
        "Git changed but A still runs"
    );
}

#[gpui::test]
fn large_file_probe_new_runtime_starts_before_old_completion(cx: &mut gpui::TestAppContext) {
    let (view, cx) = fixture(cx);
    change_runtime(&view, cx, runtime("B"));
    publish_store(&view, cx);
    let requests = requests(&view, cx);
    assert_eq!(requests.len(), 2, "B must not wait for A's timeout");
    assert_eq!(requests[1].runtime, runtime("B"));
    assert!(!requests[1].cancellation.is_cancelled());
}

#[gpui::test]
fn large_file_probe_return_to_same_runtime_rejects_old_generation(cx: &mut gpui::TestAppContext) {
    let (view, cx) = fixture(cx);
    let old = requests(&view, cx)[0].clone();
    // Both preferences reach the view before its store snapshots are applied.
    change_runtime(&view, cx, runtime("B"));
    change_runtime(&view, cx, runtime("A"));
    publish_store(&view, cx);
    view.update(cx, |view, _| {
        view.complete_large_file_tools_probe(&old, tools("obsolete A"));
    });
    assert_eq!(stored_tools(&view, cx), LargeFileToolsState::default());
    let pending = requests(&view, cx);
    assert_eq!(pending.len(), 2);
    assert!(!pending[1].cancellation.is_cancelled());
    view.update(cx, |view, _| {
        view.complete_large_file_tools_probe(&pending[1], tools("current A"));
    });
    assert_eq!(stored_tools(&view, cx), tools("current A"));
}

#[gpui::test]
fn large_file_probe_settings_result_survives_old_completion(cx: &mut gpui::TestAppContext) {
    let (view, cx) = fixture(cx);
    let old = requests(&view, cx)[0].clone();
    accept_settings_tools(&view, cx, tools("fresh Settings"));
    view.update(cx, |view, _| {
        view.complete_large_file_tools_probe(&old, tools("obsolete main window"));
    });
    assert_eq!(stored_tools(&view, cx), tools("fresh Settings"));
    assert!(old.cancellation.is_cancelled());
}

#[gpui::test]
fn large_file_probe_settings_result_blocks_queued_not_checked_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let queued = cx.update(|_, app| view.read(app).state.clone());
    accept_settings_tools(&view, cx, tools("fresh Settings"));
    view.update(cx, |view, cx| {
        view.apply_state_snapshot(queued, cx);
    });
    assert_eq!(
        requests(&view, cx).len(),
        1,
        "stale snapshot restarted discovery"
    );
    publish_store(&view, cx);
    assert_eq!(stored_tools(&view, cx), tools("fresh Settings"));
}

#[gpui::test]
fn large_file_probe_new_runtime_settings_result_survives_old_runtime_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let queued_a = cx.update(|_, app| view.read(app).state.clone());
    change_runtime(&view, cx, runtime("B"));
    accept_settings_tools(&view, cx, tools("fresh Settings B"));
    view.update(cx, |view, cx| {
        view.apply_state_snapshot(queued_a, cx);
    });
    publish_store(&view, cx);
    assert_eq!(requests(&view, cx).len(), 1, "settled B was restarted");
    assert_eq!(stored_tools(&view, cx), tools("fresh Settings B"));
}

#[gpui::test]
fn large_file_probe_window_close_cancels_even_while_view_is_retained(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let request = requests(&view, cx)[0].clone();
    let store = cx.update(|_, app| view.read(app).store.clone());
    cx.update(|window, _| window.remove_window());
    assert!(
        request.cancellation.is_cancelled(),
        "closed window still owns a running probe"
    );
    // A retained view and an already queued snapshot must not restart discovery.
    cx.cx.update(|app| {
        view.update(app, |view, cx| {
            view.complete_large_file_tools_probe(&request, tools("closed window completion"));
            view.apply_state_snapshot(view.state.clone(), cx);
            assert_eq!(
                view.large_file_tools_detector_for_test
                    .as_ref()
                    .unwrap()
                    .requests
                    .len(),
                1
            );
        })
    });
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        store.snapshot().large_file_tools,
        LargeFileToolsState::default()
    );
}

#[gpui::test]
fn large_file_probe_drop_cancels_the_owned_request(cx: &mut gpui::TestAppContext) {
    // Keep only a WindowHandle so no test-owned Entity survives window close.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    store.dispatch(Msg::SetGitRuntimeState(runtime("A")));
    // A repository keeps these request-lifecycle fixtures eligible for automatic
    // discovery; the separate Home tests cover deferred startup.
    store.dispatch(Msg::OpenRepo(
        std::env::temp_dir().join("tool-probe-lifecycle"),
    ));
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    let window = cx.add_window(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
        window.simulate_next_frame(cx);
    })
    .unwrap();
    let (request, weak) = window
        .update(cx, |view, _, cx| {
            view.large_file_tools_detector_for_test = Some(HeldLargeFileToolsDetector {
                runtime: runtime("A"),
                requests: Vec::new(),
            });
            view.apply_state_snapshot(view.store.snapshot(), cx);
            let request = view
                .large_file_tools_detector_for_test
                .as_ref()
                .unwrap()
                .requests[0]
                .clone();
            (request, cx.weak_entity())
        })
        .unwrap();
    window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update(|_| {});
    assert!(weak.upgrade().is_none(), "fixture still retains the view");
    assert!(
        request.cancellation.is_cancelled(),
        "closed window left its probe running"
    );
}

#[gpui::test]
fn large_file_probe_delayed_settings_result_cannot_settle_another_runtime(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let publication = cx.update(|_, app| LargeFileToolsPublication {
        runtime: runtime("A"),
        generation: settings_tool_probe_generation(app),
        tools: tools("old Settings A"),
    });
    change_runtime(&view, cx, runtime("B"));
    publish_store(&view, cx);
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.apply_preference_change(
                &PreferenceChange::LargeFileTools(publication),
                window,
                cx,
            );
        })
    });
    assert_eq!(stored_tools(&view, cx), LargeFileToolsState::default());
    let pending = requests(&view, cx);
    assert_eq!(pending.len(), 2);
    assert!(!pending[1].cancellation.is_cancelled());
    view.update(cx, |view, _| {
        view.complete_large_file_tools_probe(&pending[1], tools("current B"));
    });
    assert_eq!(stored_tools(&view, cx), tools("current B"));
}

#[gpui::test]
fn large_file_probe_repository_and_repeated_snapshots_keep_one_request(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| {
        assert!(!view.state.repos.is_empty());
        for _ in 0..3 {
            view.apply_state_snapshot(view.state.clone(), cx);
        }
    });
    assert_eq!(requests(&view, cx).len(), 1);
    let request = requests(&view, cx)[0].clone();
    view.update(cx, |view, _| {
        view.complete_large_file_tools_probe(&request, tools("current"));
    });
    publish_store(&view, cx);
    publish_store(&view, cx);
    assert_eq!(requests(&view, cx).len(), 1);
}

#[gpui::test]
fn large_file_probe_unavailable_runtime_waits_for_recovery(cx: &mut gpui::TestAppContext) {
    let (view, cx) = fixture(cx);
    let mut unavailable = runtime("B");
    unavailable.availability = GitExecutableAvailability::Unavailable {
        detail: "fixture unavailable".into(),
    };
    change_runtime(&view, cx, unavailable);
    publish_store(&view, cx);
    assert_eq!(requests(&view, cx).len(), 1);
    change_runtime(&view, cx, runtime("B"));
    publish_store(&view, cx);
    assert_eq!(requests(&view, cx).len(), 2);
}

fn native_fixture_command(root: &Path, mode: &str) -> std::process::Command {
    let mut command = gitcomet_core::process::background_command(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "view::tests::large_file_tools_probe::large_file_probe_native_child",
            "--nocapture",
            "--",
        ])
        .env("GITCOMET_LARGE_FILE_PROBE_FIXTURE", mode)
        .env("GITCOMET_LARGE_FILE_PROBE_ROOT", root);
    command
}

#[test]
#[ignore = "native subprocess fixture for the real large-file detector"]
#[allow(clippy::zombie_processes)] // The descendant deliberately retains the parent's pipes.
fn large_file_probe_native_child() {
    let root = PathBuf::from(std::env::var_os("GITCOMET_LARGE_FILE_PROBE_ROOT").unwrap());
    let mode = std::env::var("GITCOMET_LARGE_FILE_PROBE_FIXTURE").unwrap();
    if mode == "parent" {
        // libtest accepts multiple exact filters. The detector's appended Git
        // arguments remain visible here without a shell or a second executable.
        let tool = std::env::args()
            .find(|arg| arg == "lfs" || arg == "annex")
            .unwrap();
        std::fs::write(root.join(format!("{tool}.invoked")), b"invoked").unwrap();
        let _child = native_fixture_command(&root, "descendant")
            .env("GITCOMET_LARGE_FILE_PROBE_TOOL", &tool)
            .spawn()
            .unwrap();
        return;
    }
    let tool = std::env::var("GITCOMET_LARGE_FILE_PROBE_TOOL").unwrap();
    std::fs::write(root.join(format!("{tool}.started")), b"started").unwrap();
    std::thread::sleep(Duration::from_secs(10));
    std::fs::write(root.join(format!("{tool}.escaped")), b"escaped").unwrap();
}

#[gpui::test]
fn large_file_probe_owned_token_stops_both_native_detectors_and_descendants(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let request = requests(&view, cx)[0].clone();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let token = request.cancellation.clone();
    let worker = std::thread::spawn(move || {
        gitcomet_core::large_file_tools::detect_large_file_tools_with(
            &|| native_fixture_command(&root, "parent"),
            &token,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    let both_started = loop {
        if ["lfs", "annex"]
            .iter()
            .all(|tool| dir.path().join(format!("{tool}.started")).exists())
        {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    change_runtime(&view, cx, runtime("B"));
    let owner_cancelled = request.cancellation.is_cancelled();
    // Always clean up the native fixture, including on the expected-red baseline.
    request.cancellation.cancel();
    let result = worker.join().unwrap();
    assert!(
        both_started,
        "both detector children must reach the fixture barrier"
    );
    for tool in ["lfs", "annex"] {
        assert!(dir.path().join(format!("{tool}.invoked")).exists());
        assert!(!dir.path().join(format!("{tool}.escaped")).exists());
    }
    assert_eq!(result.git_lfs, ToolAvailability::Unknown);
    assert_eq!(result.git_annex, ToolAvailability::Unknown);
    assert!(
        owner_cancelled,
        "the view did not cancel the token used by the real detector"
    );
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let result = gitcomet_core::large_file_tools::detect_large_file_tools_with(
        &|| {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            native_fixture_command(dir.path(), "parent")
        },
        &request.cancellation,
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(result, LargeFileToolsState::default());
}
