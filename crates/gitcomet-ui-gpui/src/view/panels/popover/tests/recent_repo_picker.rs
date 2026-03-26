use super::*;
use gitcomet_core::process::background_command as no_window_command;
use std::time::{Duration, Instant};

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";

fn wait_until(cx: &mut gpui::VisualTestContext, description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn normalize_existing_path(path: std::path::PathBuf) -> std::path::PathBuf {
    strip_windows_verbatim_prefix(std::fs::canonicalize(&path).unwrap_or(path))
}

#[cfg(windows)]
fn strip_windows_verbatim_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };

    let mut out = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => {
            std::path::PathBuf::from(format!("{}:", char::from(letter)))
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut out = std::path::PathBuf::from(r"\\");
            out.push(server);
            out.push(share);
            out
        }
        Prefix::Verbatim(raw) => std::path::PathBuf::from(raw),
        _ => return path,
    };

    for component in components {
        out.push(component.as_os_str());
    }
    out
}

#[cfg(not(windows))]
fn strip_windows_verbatim_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    path
}

#[gpui::test]
fn recent_repository_picker_opens_and_initializes_search_input(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_recent_repository_picker(window, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("app_popover").is_some());

    cx.update(|_window, app| {
        let popover_host = { view.read(app).popover_host.clone() };
        assert!(view.read(app).is_popover_open(app));

        let host = popover_host.read(app);
        assert!(matches!(
            host.popover,
            Some(PopoverKind::RecentRepositoryPicker)
        ));

        let input = host
            .recent_repo_picker_search_input
            .clone()
            .expect("recent repository picker should create a search input");
        assert_eq!(input.read(app).text().to_string(), "");
    });
}

#[gpui::test]
fn recent_repository_picker_reopen_clears_previous_search_text(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_recent_repository_picker(window, cx);
        });
    });

    cx.update(|_window, app| {
        let popover_host = { view.read(app).popover_host.clone() };
        let input = popover_host
            .read(app)
            .recent_repo_picker_search_input
            .clone()
            .expect("recent repository picker should create a search input");
        input.update(app, |input, cx| input.set_text("repo", cx));
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_recent_repository_picker(window, cx);
        });
    });

    cx.update(|_window, app| {
        let popover_host = { view.read(app).popover_host.clone() };
        let input = popover_host
            .read(app)
            .recent_repo_picker_search_input
            .clone()
            .expect("recent repository picker should reuse its search input");
        assert_eq!(input.read(app).text().to_string(), "");
    });
}

#[test]
fn recent_repository_picker_selecting_recent_repo_does_not_panic_wrapper() {
    if std::env::var_os(SESSION_FILE_ENV).is_some() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo-a");
    std::fs::create_dir_all(&repo_path).expect("create recent repo dir");
    let session_file = dir.path().join("session.json");
    gitcomet_state::session::persist_recent_repo_to_path(&repo_path, &session_file)
        .expect("seed recent repo session");

    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = no_window_command(current_exe)
        .arg("recent_repository_picker_selecting_recent_repo_does_not_panic_subprocess")
        .arg("--nocapture")
        .env(SESSION_FILE_ENV, &session_file)
        .output()
        .expect("spawn subtest process");
    assert!(
        output.status.success(),
        "subtest failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[gpui::test]
fn recent_repository_picker_selecting_recent_repo_does_not_panic_subprocess(
    cx: &mut gpui::TestAppContext,
) {
    if std::env::var_os(SESSION_FILE_ENV).is_none() {
        return;
    }

    let _visual_guard = crate::test_support::lock_visual_test();
    let expected_path = gitcomet_state::session::load()
        .recent_repos
        .into_iter()
        .next()
        .expect("seeded recent repo");
    let expected_path = normalize_existing_path(expected_path);

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_recent_repository_picker(window, cx);
        });
        let _ = window.draw(app);
        window.activate_window();
    });

    let item_bounds = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected first recent repository picker item");
    cx.simulate_mouse_move(item_bounds.center(), None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        item_bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        item_bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert!(
            !view.read(app).is_popover_open(app),
            "expected recent repository picker to close after selection"
        );
    });
    wait_until(cx, "selected recent repository to open", || {
        store_for_assert
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == expected_path)
    });
}
