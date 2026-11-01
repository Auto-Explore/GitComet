use super::*;
use crate::view::large_file_tools_probe::{
    HeldLargeFileToolsDetector, LargeFileToolsPublication, NativeLargeFileToolsDetectorForTest,
    settings_tool_probe_generation,
};
use crate::view::preference_change::PreferenceChange;
use gitcomet_core::large_file_tools::{LargeFileToolsState, ToolAvailability};
use std::sync::atomic::{AtomicUsize, Ordering};

fn runtime() -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::Custom(PathBuf::from("home-probe-fixture")),
        availability: GitExecutableAvailability::Available {
            version_output: "git version 2.50.0".into(),
        },
    }
}

struct NativeFixture {
    root: tempfile::TempDir,
    calls: Arc<AtomicUsize>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl Drop for NativeFixture {
    fn drop(&mut self) {
        let (released, wake) = self.release.as_ref();
        *released.lock().unwrap() = true;
        wake.notify_all();
    }
}

fn fixture(
    cx: &mut gpui::TestAppContext,
    initially_opening: bool,
) -> (
    NativeFixture,
    Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let native = NativeFixture {
        root: tempfile::tempdir().unwrap(),
        calls: Arc::new(AtomicUsize::new(0)),
        release: Arc::new((Mutex::new(false), Condvar::new())),
    };
    let (store, events) = AppStore::new_test(Arc::new(BlockingFailingBackend {
        release: Arc::clone(&native.release),
    }));
    store.dispatch(Msg::SetGitRuntimeState(runtime()));
    if initially_opening {
        store.dispatch(Msg::OpenRepo(native.root.path().join("repository")));
    }
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::drain_store_worker(&view, cx);
    let root = native.root.path().to_path_buf();
    let calls = Arc::clone(&native.calls);
    cx.update(|_, app| {
        app.set_global(NativeLargeFileToolsDetectorForTest(Arc::new(move || {
            let invocation = calls.fetch_add(1, Ordering::Relaxed);
            let mut command =
                gitcomet_core::process::background_command(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    "view::tests::large_file_tools_home::large_file_probe_home_native_child",
                    "--nocapture",
                    "--",
                ])
                .env("GITCOMET_HOME_PROBE_ROOT", &root)
                .env("GITCOMET_HOME_PROBE_INVOCATION", invocation.to_string());
            command
        })));
    });
    view.update(cx, |view, _| {
        view.large_file_tools_detector_for_test = Some(HeldLargeFileToolsDetector {
            runtime: runtime(),
            requests: Vec::new(),
        });
    });
    (native, view, cx)
}

fn apply_snapshots(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    test_support::drain_store_worker(view, cx);
    view.update(cx, |view, cx| {
        for _ in 0..3 {
            view.apply_state_snapshot(view.store.snapshot(), cx);
        }
    });
    cx.run_until_parked();
    test_support::drain_store_worker(view, cx);
}

fn request_count(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> usize {
    cx.update(|_, app| {
        view.read(app)
            .large_file_tools_detector_for_test
            .as_ref()
            .unwrap()
            .requests
            .len()
    })
}

fn witness(
    scenario: &str,
    native: &NativeFixture,
    view: &Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> usize {
    let requests = request_count(view, cx);
    let mut tools = std::fs::read_dir(native.root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "invoked")
        })
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();
    tools.sort();
    let calls = native.calls.load(Ordering::Relaxed);
    println!(
        "HOME_PROBE_WORK scenario={scenario} requests={requests} commands={calls} child_markers={}",
        tools.len(),
    );
    // Both native children must actually run, not merely reach the factory.
    assert_eq!(calls, requests * 2);
    assert_eq!(tools.len(), calls);
    assert_eq!(
        tools.iter().filter(|tool| tool.as_str() == "lfs").count(),
        requests
    );
    assert_eq!(
        tools.iter().filter(|tool| tool.as_str() == "annex").count(),
        requests
    );
    calls
}

fn assert_published(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.simulate_next_frame(cx);
    });
    pump_until(cx, "native large-file result publication", |cx| {
        cx.update(|_, app| {
            let state = view.read(app).store.snapshot();
            matches!(
                state.large_file_tools.git_lfs,
                ToolAvailability::Available { .. }
            ) && matches!(
                state.large_file_tools.git_annex,
                ToolAvailability::Available { .. }
            )
        })
    });
}

#[gpui::test]
fn large_file_probe_native_commands_wait_until_after_initial_platform_frame(
    cx: &mut gpui::TestAppContext,
) {
    let (native, view, cx) = fixture(cx, true);
    apply_snapshots(&view, cx);
    let handle = cx.update(|window, _| window.window_handle());
    let platform_window = cx.cx.test_window(handle);
    assert_eq!(platform_window.presents(), 0);
    assert_eq!(witness("before-platform-frame", &native, &view, cx), 0);
    platform_window.simulate_frame_request(gpui::RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_eq!(platform_window.presents(), 1);
    apply_snapshots(&view, cx);
    assert_eq!(witness("after-first-present", &native, &view, cx), 0);
    // No repository completion is needed: this fixture's open stays blocked.
    platform_window.simulate_frame_request(gpui::RequestFrameOptions {
        require_presentation: true,
        ..Default::default()
    });
    assert_published(&view, cx);
    assert_eq!(witness("following-platform-turn", &native, &view, cx), 2);
}

#[gpui::test]
fn large_file_probe_home_defers_automatic_native_detectors(cx: &mut gpui::TestAppContext) {
    let (native, view, cx) = fixture(cx, false);
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.simulate_next_frame(cx);
    });
    apply_snapshots(&view, cx);
    assert!(cx.update(|_, app| view.read(app).state.repos.is_empty()));
    if request_count(&view, cx) != 0 {
        // The unchanged baseline must demonstrate both real processes before
        // failing the new requirement, rather than failing a fixture precondition.
        assert_published(&view, cx);
    }
    assert_eq!(witness("empty-home", &native, &view, cx), 0);
    assert_eq!(request_count(&view, cx), 0);
    assert_eq!(
        cx.update(|_, app| view.read(app).store.snapshot().large_file_tools.clone()),
        LargeFileToolsState::default(),
    );
}

#[gpui::test]
fn large_file_probe_home_first_opening_repo_starts_one_native_pair(cx: &mut gpui::TestAppContext) {
    let (native, view, cx) = fixture(cx, false);
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.simulate_next_frame(cx);
    });
    apply_snapshots(&view, cx);
    if request_count(&view, cx) != 0 {
        assert_published(&view, cx);
    }
    let home_calls = witness("before-first-repository", &native, &view, cx);
    view.update(cx, |view, _| {
        view.store
            .dispatch(Msg::OpenRepo(native.root.path().join("repository")));
    });
    apply_snapshots(&view, cx);
    assert!(cx.update(|_, app| {
        let state = view.read(app).store.snapshot();
        state.repos.len() == 1 && matches!(state.repos[0].open, Loadable::Loading)
    }));
    assert_published(&view, cx);
    apply_snapshots(&view, cx);
    assert_eq!(witness("first-opening-repository", &native, &view, cx), 2);
    assert_eq!(request_count(&view, cx), 1);
    assert_eq!(
        home_calls, 0,
        "discovery must wait for the first repository"
    );
}

#[gpui::test]
fn large_file_probe_home_restored_opening_repo_starts_one_native_pair(
    cx: &mut gpui::TestAppContext,
) {
    // A restored repository already exists in the first snapshot, but its backend
    // open remains blocked. Detection must not wait for repository loading.
    let (native, view, cx) = fixture(cx, true);
    apply_snapshots(&view, cx);
    assert!(cx.update(|_, app| {
        let state = view.read(app).store.snapshot();
        state.repos.len() == 1 && matches!(state.repos[0].open, Loadable::Loading)
    }));
    assert_published(&view, cx);
    apply_snapshots(&view, cx);
    assert_eq!(witness("initial-opening-repository", &native, &view, cx), 2);
    assert_eq!(request_count(&view, cx), 1);
}

#[gpui::test]
fn large_file_probe_home_settings_publication_prevents_redundant_detection(
    cx: &mut gpui::TestAppContext,
) {
    let (native, view, cx) = fixture(cx, false);
    let tools = LargeFileToolsState {
        git_lfs: ToolAvailability::Available {
            version: Some("Settings LFS".into()),
        },
        git_annex: ToolAvailability::Available {
            version: Some("Settings annex".into()),
        },
    };
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            assert!(view.state.repos.is_empty());
            view.apply_preference_change(
                &PreferenceChange::LargeFileTools(LargeFileToolsPublication {
                    runtime: runtime(),
                    generation: settings_tool_probe_generation(cx),
                    tools: tools.clone(),
                }),
                window,
                cx,
            );
        });
    });
    apply_snapshots(&view, cx);
    assert_eq!(
        cx.update(|_, app| view.read(app).store.snapshot().large_file_tools.clone()),
        tools
    );
    view.update(cx, |view, _| {
        view.store
            .dispatch(Msg::OpenRepo(native.root.path().join("repository")));
    });
    apply_snapshots(&view, cx);
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.simulate_next_frame(cx);
    });
    assert_eq!(
        witness("settings-then-first-repository", &native, &view, cx),
        0
    );
}

#[test]
#[ignore = "native subprocess fixture for automatic discovery resource counts"]
fn large_file_probe_home_native_child() {
    let root = PathBuf::from(std::env::var_os("GITCOMET_HOME_PROBE_ROOT").unwrap());
    let invocation = std::env::var("GITCOMET_HOME_PROBE_INVOCATION").unwrap();
    let args = std::env::args().collect::<Vec<_>>();
    let tool = args
        .iter()
        .find(|arg| *arg == "lfs" || *arg == "annex")
        .unwrap();
    assert!(args.iter().any(|arg| arg == "version"));
    assert_eq!(args.iter().any(|arg| arg == "--raw"), tool == "annex");
    std::fs::write(root.join(format!("{invocation}-{tool}.invoked")), tool).unwrap();
    println!("fixture {tool} 1.0");
}
