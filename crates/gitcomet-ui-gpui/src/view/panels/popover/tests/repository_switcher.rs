use super::*;
use gitcomet_core::path_utils::canonicalize_or_original;
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
    canonicalize_or_original(path)
}

#[gpui::test]
fn repository_switcher_opens_the_repo_picker_with_a_fresh_search_input(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.toggle_repository_switcher(window, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("app_popover").is_some());

    cx.update(|_window, app| {
        let popover_host = { view.read(app).popover_host.clone() };
        assert!(crate::view::test_support::popover_is_open(
            view.read(app),
            app
        ));

        let host = popover_host.read(app);
        assert!(matches!(host.popover, Some(PopoverKind::RepoPicker)));

        let input = host
            .repo_picker_search_input
            .clone()
            .expect("repository switcher should create a search input");
        assert_eq!(input.read(app).text().to_string(), "");
    });
}

#[gpui::test]
fn repository_switcher_reopen_clears_previous_search_text(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.toggle_repository_switcher(window, cx);
        });
    });

    cx.update(|_window, app| {
        let popover_host = { view.read(app).popover_host.clone() };
        let input = popover_host
            .read(app)
            .repo_picker_search_input
            .clone()
            .expect("repository switcher should create a search input");
        input.update(app, |input, cx| input.set_text("repo", cx));
    });

    // The shortcut toggles, so reopening means closing first.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.toggle_repository_switcher(window, cx);
            this.toggle_repository_switcher(window, cx);
        });
    });

    cx.update(|_window, app| {
        let popover_host = { view.read(app).popover_host.clone() };
        let input = popover_host
            .read(app)
            .repo_picker_search_input
            .clone()
            .expect("repository switcher should reuse its search input");
        assert_eq!(input.read(app).text().to_string(), "");
    });
}

#[test]
fn repository_switcher_selecting_recent_repo_opens_it_wrapper() {
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
        .arg("repository_switcher_selecting_recent_repo_opens_it_subprocess")
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
fn repository_switcher_selecting_recent_repo_opens_it_subprocess(cx: &mut gpui::TestAppContext) {
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
            this.toggle_repository_switcher(window, cx);
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
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected the repository switcher to close after selection"
        );
    });
    wait_until(cx, "selected repository to open", || {
        store_for_assert
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == expected_path)
    });
}

#[test]
fn repository_switcher_removes_a_recent_repo_wrapper() {
    if std::env::var_os(SESSION_FILE_ENV).is_some() {
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");
    for name in ["repo-a", "repo-b"] {
        let repo_path = dir.path().join(name);
        std::fs::create_dir_all(&repo_path).expect("create recent repo dir");
        gitcomet_state::session::persist_recent_repo_to_path(&repo_path, &session_file)
            .expect("seed recent repo session");
    }

    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = no_window_command(current_exe)
        .arg("repository_switcher_removes_a_recent_repo_subprocess")
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
fn repository_switcher_removes_a_recent_repo_subprocess(cx: &mut gpui::TestAppContext) {
    if std::env::var_os(SESSION_FILE_ENV).is_none() {
        return;
    }

    let _visual_guard = crate::test_support::lock_visual_test();
    let seeded = gitcomet_state::session::load().recent_repos;
    assert_eq!(seeded.len(), 2, "expected two seeded recent repositories");
    let removed = seeded[0].clone();
    let kept = seeded[1].clone();

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.toggle_repository_switcher(window, cx);
        });
        let _ = window.draw(app);
        window.activate_window();
    });

    // The remove button only appears once its row is hovered.
    let row_bounds = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected the first recent repository picker item");
    cx.simulate_mouse_move(row_bounds.center(), None, gpui::Modifiers::default());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let remove_bounds = cx
        .debug_bounds("picker_prompt_item_remove_0")
        .expect("expected a remove button on the recent repository row");
    cx.simulate_mouse_move(remove_bounds.center(), None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        remove_bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        remove_bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "removing a recent repository should leave the picker open"
        );
        let host = view.read(app).popover_host.read(app);
        assert_eq!(
            host.cached_recent_repos,
            vec![kept.clone()],
            "expected the removed repository to leave the picker list"
        );
    });
    assert_eq!(
        gitcomet_state::session::load().recent_repos,
        vec![kept],
        "expected the removal to persist to the session file"
    );
    assert!(
        !gitcomet_state::session::load()
            .recent_repos
            .contains(&removed),
        "expected the removed repository to stay gone"
    );
}

#[gpui::test]
fn repository_switcher_shortcut_toggles_the_picker_closed(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let toggle = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.toggle_repository_switcher(window, cx);
            });
            let _ = window.draw(app);
        });
    };
    let is_open = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| crate::view::test_support::popover_is_open(view.read(app), app))
    };

    toggle(cx);
    assert!(is_open(cx), "expected the shortcut to open the picker");

    toggle(cx);
    assert!(
        !is_open(cx),
        "expected the shortcut to close the picker it opened"
    );

    toggle(cx);
    assert!(is_open(cx), "expected the shortcut to reopen the picker");
}

/// The shortcut anchors to the titlebar chevron; only the command palette
/// centres the picker.
#[gpui::test]
fn repository_switcher_anchors_to_the_toggle_but_the_palette_centres(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events, _repo, _workdir) = super::branch::create_tracking_store("switcher-anchor");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    // Draw once so the titlebar chevron reports painted bounds.
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let toggle_bounds = cx
        .debug_bounds("repo_picker_toggle")
        .expect("expected the titlebar repository switcher toggle");

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.toggle_repository_switcher(window, cx);
        });
        let _ = window.draw(app);
    });
    let shortcut_bounds = cx
        .debug_bounds("app_popover")
        .expect("expected the picker to open from the shortcut");
    assert!(
        (shortcut_bounds.origin.x - toggle_bounds.origin.x).abs() < gpui::px(40.0),
        "expected the shortcut to anchor near the toggle at {:?}, got {:?}",
        toggle_bounds.origin,
        shortcut_bounds.origin
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
            this.open_repository_switcher_centered(window, cx);
        });
        let _ = window.draw(app);
    });
    let centered_bounds = cx
        .debug_bounds("app_popover")
        .expect("expected the picker to open from the palette");
    assert!(
        centered_bounds.origin.x > shortcut_bounds.origin.x,
        "expected the palette to centre the picker (shortcut at {:?}, palette at {:?})",
        shortcut_bounds.origin,
        centered_bounds.origin
    );
}
