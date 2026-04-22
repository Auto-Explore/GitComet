use super::branch::wait_until;
use super::*;

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
}

fn unique_parent_dir(label: &str) -> std::path::PathBuf {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gitcomet-ui-clone-popover-{label}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("test parent directory to be created");
    path
}

#[gpui::test]
fn clone_repo_popover_renders_shortcut_hints_and_separators(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CloneRepo,
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

    cx.debug_bounds("clone_repo_cancel_hint")
        .expect("expected clone Cancel shortcut hint");
    cx.debug_bounds("clone_repo_go_hint")
        .expect("expected clone shortcut hint");
    cx.debug_bounds("clone_repo_cancel_end_slot_separator")
        .expect("expected clone Cancel shortcut separator");
    cx.debug_bounds("clone_repo_go_end_slot_separator")
        .expect("expected clone shortcut separator");
}

#[gpui::test]
fn clone_repo_popover_escape_closes_from_parent_input(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CloneRepo,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                let focus = host
                    .clone_repo_parent_dir_input
                    .read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close clone popover");
    assert!(
        store.snapshot().clone.is_none(),
        "expected Escape to avoid starting a clone"
    );
}

#[gpui::test]
fn clone_repo_popover_escape_closes_from_browse_button(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CloneRepo,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                window.focus(&host.clone_repo_browse_focus_handle, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    simulate_key_press(cx, "escape");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        !is_open,
        "expected Escape to close clone popover from Browse"
    );
    assert!(
        store.snapshot().clone.is_none(),
        "expected Escape from Browse to avoid starting a clone"
    );
}

#[gpui::test]
fn clone_repo_popover_tabs_between_inputs_buttons_and_wraps(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CloneRepo,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.clone_repo_url_input.update(cx, |input, cx| {
                    input.set_text("http://example.com/org/repo.git", cx)
                });
                host.clone_repo_parent_dir_input
                    .update(cx, |input, cx| input.set_text("/tmp", cx));
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.clone_repo_url_input.read(app).focus_handle(),
            "expected clone popover to focus the URL input first",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.clone_repo_parent_dir_input.read(app).focus_handle(),
            "expected Tab to move from clone URL to parent dir",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.clone_repo_browse_focus_handle.clone(),
            "expected Tab to move from clone parent dir to Browse",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.clone_repo_submit_focus_handle.clone(),
            "expected Tab to move from Browse to Clone",
        );
    });

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.clone_repo_url_input.read(app).focus_handle(),
            "expected Tab to wrap from Clone back to URL",
        );
    });

    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert_window_focus(
            window,
            app,
            host.clone_repo_submit_focus_handle.clone(),
            "expected Shift-Tab to wrap from URL back to Clone",
        );
    });
}

#[gpui::test]
fn clone_repo_popover_enter_from_parent_input_submits_and_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let url = "http://example.com/org/repo.git";
    let parent = unique_parent_dir("enter");
    let expected_dest = parent.join("repo");

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CloneRepo,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.clone_repo_url_input
                    .update(cx, |input, cx| input.set_text(url, cx));
                host.clone_repo_parent_dir_input.update(cx, |input, cx| {
                    input.set_text(parent.display().to_string(), cx);
                });
                let focus = host
                    .clone_repo_parent_dir_input
                    .read_with(cx, |input, _| input.focus_handle());
                window.focus(&focus, cx);
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Enter to close clone popover");

    // AppStore dispatch is asynchronous, so wait until the reducer records the clone request.
    wait_until("clone op to be recorded", || {
        let snapshot = store.snapshot();
        snapshot
            .clone
            .as_ref()
            .is_some_and(|op| &*op.url == url && op.dest.as_ref() == &expected_dest)
    });

    let snapshot = store.snapshot();
    let op = snapshot
        .clone
        .as_ref()
        .expect("expected clone op to be recorded");
    assert_eq!(&*op.url, url);
    assert_eq!(op.dest.as_ref(), &expected_dest);
}

#[gpui::test]
fn clone_repo_popover_clone_button_requires_parent_path(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let url = "http://example.com/org/repo.git";

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CloneRepo,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.clone_repo_url_input
                    .update(cx, |input, cx| input.set_text(url, cx));
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    click_debug_selector(cx, "clone_repo_go_hint");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        is_open,
        "expected incomplete clone form to keep the popover open"
    );
    assert!(
        store.snapshot().clone.is_none(),
        "expected missing parent path to keep Clone disabled"
    );
}
