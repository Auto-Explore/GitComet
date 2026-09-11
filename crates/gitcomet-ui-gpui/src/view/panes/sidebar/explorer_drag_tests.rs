use super::*;
use crate::view::test_support::{self, TestBackend};
use gitcomet_core::domain::RepoSpec;
use std::path::PathBuf;

fn dir(name: &str, path: &str, depth: usize) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: Arc::new(PathBuf::from(path)),
        kind: FileEntryKind::Directory,
        depth,
    }
}

fn file(name: &str, path: &str, depth: usize) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: Arc::new(PathBuf::from(path)),
        kind: FileEntryKind::File,
        depth,
    }
}

/// Two expanded folders so the row under the pointer and the last painted row
/// resolve to different drop destinations.
fn explorer_state() -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(7),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-explorer-drag"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.file_browser.active = true;
    repo.file_browser.entries = Loadable::Ready(Arc::new(vec![
        dir("alpha", "alpha", 0),
        file("one.rs", "alpha/one.rs", 1),
        dir("zulu", "zulu", 0),
        file("two.rs", "zulu/two.rs", 1),
    ]));
    repo.file_browser
        .expanded_dirs
        .insert(Arc::new(PathBuf::from("alpha")));
    repo.file_browser
        .expanded_dirs
        .insert(Arc::new(PathBuf::from("zulu")));
    repo.file_browser.bump_rev();
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        sidebar_mode: gitcomet_state::model::SidebarMode::Files,
        ..Default::default()
    })
}

#[gpui::test]
fn explorer_drag_highlights_the_folder_under_the_pointer(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    let state = explorer_state();
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);

    let alpha_row = cx
        .debug_bounds("file_browser_row_0")
        .expect("alpha folder row");
    let dragged_row = cx
        .debug_bounds("file_browser_row_1")
        .expect("alpha/one.rs row");

    // Pick up alpha/one.rs, then park the pointer over the alpha folder row.
    cx.simulate_mouse_move(
        dragged_row.center(),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseDownEvent {
        position: dragged_row.center(),
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        dragged_row.center() + gpui::point(px(0.0), px(-6.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_move(
        alpha_row.center(),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    let target = cx.update(|_window, app| pane.read(app).explorer_drop_target.clone());
    assert_eq!(
        target,
        Some(PathBuf::from("alpha")),
        "the drop target must be the folder under the pointer"
    );

    // Check later rows too: using the viewport height as the row height
    // incorrectly resolves every pointer position to the first folder.
    for (selector, path) in [
        ("file_browser_row_2", "zulu"),
        ("file_browser_row_3", "zulu/two.rs"),
    ] {
        let row = cx.debug_bounds(selector).unwrap();
        cx.simulate_mouse_move(
            row.center(),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        test_support::redraw(cx);
        cx.update(|_, app| {
            let pane = pane.read(app);
            assert_eq!(
                pane.explorer_drop_row.as_deref(),
                Some(std::path::Path::new(path))
            );
            assert_eq!(
                pane.explorer_drop_target.as_deref(),
                Some(std::path::Path::new("zulu"))
            );
        });
    }
}

/// A list long enough to scroll, so drag autoscroll has somewhere to go.
fn long_explorer_state(count: usize) -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(8),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-explorer-autoscroll"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.file_browser.active = true;
    repo.file_browser.entries = Loadable::Ready(Arc::new(
        (0..count)
            .map(|ix| file(&format!("file_{ix:04}.rs"), &format!("file_{ix:04}.rs"), 0))
            .collect(),
    ));
    repo.file_browser.bump_rev();
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        sidebar_mode: gitcomet_state::model::SidebarMode::Files,
        ..Default::default()
    })
}

#[gpui::test]
fn explorer_drag_autoscroll_repaints_while_the_pointer_is_parked(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
    let state = long_explorer_state(400);
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);

    let container = cx
        .debug_bounds("file_browser_scroll_container")
        .expect("file browser viewport");
    let dragged_row = cx.debug_bounds("file_browser_row_1").expect("a file row");

    cx.simulate_mouse_move(
        dragged_row.center(),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseDownEvent {
        position: dragged_row.center(),
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        dragged_row.center() + gpui::point(px(0.0), px(-6.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    // Park the pointer in the bottom autoscroll band and stop moving it.
    let parked = gpui::point(container.center().x, container.bottom() - px(8.0));
    cx.simulate_mouse_move(
        parked,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    let offset_before = cx.update(|_window, app| {
        pane.read(app)
            .file_browser_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    let painted_before = cx
        .debug_bounds("file_browser_row_1")
        .expect("row before autoscroll")
        .top();

    // Deliver display frames with no further mouse input.
    for _ in 0..5 {
        drag_frame(cx, std::time::Duration::from_millis(16));
    }

    let offset_after = cx.update(|_window, app| {
        pane.read(app)
            .file_browser_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    let painted_after = cx
        .debug_bounds("file_browser_row_1")
        .expect("row after autoscroll")
        .top();

    assert!(
        offset_after.y < offset_before.y,
        "autoscroll must advance the scroll offset (before={offset_before:?}, after={offset_after:?})"
    );
    assert!(
        painted_after < painted_before,
        "autoscroll must reach the screen, not just the scroll handle \
         (offset moved {:?} -> {:?} but the painted row stayed at {painted_before:?})",
        offset_before.y,
        offset_after.y
    );
}

/// The store reducer runs on its own OS thread, which `run_until_parked` does
/// not drive; poll the published snapshot instead.
fn wait_for_expanded(store: &AppStore, repo_id: RepoId, path: &str) -> bool {
    let wanted = Arc::new(PathBuf::from(path));
    for _ in 0..200 {
        let snapshot = store.snapshot();
        if snapshot
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .is_some_and(|r| r.file_browser.expanded_dirs.contains(&wanted))
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// A collapsed folder that is not the last visible row.
fn collapsed_folder_state() -> Arc<AppState> {
    let mut repo = RepoState::new_opening(
        RepoId(9),
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-explorer-expand"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.file_browser.active = true;
    repo.file_browser.entries = Loadable::Ready(Arc::new(vec![
        dir("alpha", "alpha", 0),
        file("hidden.rs", "alpha/hidden.rs", 1),
        dir("zulu", "zulu", 0),
        file("two.rs", "zulu/two.rs", 1),
    ]));
    repo.file_browser
        .expanded_dirs
        .insert(Arc::new(PathBuf::from("zulu")));
    repo.file_browser.bump_rev();
    Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        sidebar_mode: gitcomet_state::model::SidebarMode::Files,
        ..Default::default()
    })
}

#[gpui::test]
fn hovering_a_collapsed_folder_during_a_drag_expands_it(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_handle = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = collapsed_folder_state();
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);

    let alpha_row = cx
        .debug_bounds("file_browser_row_0")
        .expect("collapsed alpha row");
    // zulu/two.rs is the last visible row; drag it onto the collapsed alpha.
    let dragged_row = cx.debug_bounds("file_browser_row_2").expect("zulu/two.rs");

    cx.simulate_mouse_move(
        dragged_row.center(),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseDownEvent {
        position: dragged_row.center(),
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        dragged_row.center() + gpui::point(px(0.0), px(-6.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_move(
        alpha_row.center(),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    // Hold still over alpha for longer than the 600ms hover-expand delay.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(900));
    cx.run_until_parked();
    test_support::redraw(cx);

    let expanded = wait_for_expanded(&store_handle, RepoId(9), "alpha");
    assert!(
        expanded,
        "hovering a collapsed folder during a drag must expand it"
    );
}

#[gpui::test]
fn drawing_the_tree_does_not_read_the_platform_clipboard_each_frame(cx: &mut gpui::TestAppContext) {
    // On Linux/X11 a clipboard read is a synchronous selection transfer, and
    // gpui re-renders on every mouse move while a drag is in flight.
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = long_explorer_state(120);
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);

    crate::clipboard::FILE_CLIPBOARD_READS.with(|reads| reads.set(0));
    for _ in 0..5 {
        test_support::redraw(cx);
    }
    let reads = crate::clipboard::FILE_CLIPBOARD_READS.with(|reads| reads.get());
    assert_eq!(
        reads, 0,
        "five redraws with an unchanged clipboard must not re-read it"
    );
}

fn drag_frame(cx: &mut gpui::VisualTestContext, elapsed: std::time::Duration) {
    cx.run_until_parked();
    cx.executor().advance_clock(elapsed);
    cx.update(|window, app| {
        window.simulate_next_frame(app);
    });
    cx.run_until_parked();
    test_support::redraw(cx);
}

fn start_file_drag(cx: &mut gpui::VisualTestContext, start: gpui::Point<Pixels>) {
    cx.simulate_mouse_move(start, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        start + gpui::point(px(6.0), px(0.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    assert!(cx.update(|_, app| app.has_active_drag()));
}

#[gpui::test]
fn explorer_preview_tracks_cursor_from_any_grab_position(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = explorer_state();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let row = cx.debug_bounds("file_browser_row_1").unwrap();
    for x in [
        row.left() + px(25.0),
        row.center().x,
        row.right() - px(15.0),
    ] {
        start_file_drag(cx, gpui::point(x, row.center().y));
        for delta in [15.0, 30.0, 50.0] {
            let cursor = gpui::point(x + px(delta), row.center().y + px(30.0));
            cx.simulate_mouse_move(
                cursor,
                Some(gpui::MouseButton::Left),
                gpui::Modifiers::default(),
            );
            test_support::redraw(cx);
            let preview = cx.debug_bounds("explorer_drag_preview").unwrap();
            let expected = cursor + gpui::point(px(12.0), px(16.0));
            assert!(
                (preview.origin.x - expected.x).abs() <= px(1.0),
                "{preview:?} vs {expected:?}"
            );
            assert!(
                (preview.origin.y - expected.y).abs() <= px(1.0),
                "{preview:?} vs {expected:?}"
            );
            assert!(preview.size.width <= px(320.0));
        }
        assert!(cx.debug_bounds("explorer_drag_copy_badge").is_none());
        let mut modifiers = gpui::Modifiers::default();
        if cfg!(target_os = "macos") {
            modifiers.alt = true;
        } else {
            modifiers.control = true;
        }
        cx.simulate_event(gpui::ModifiersChangedEvent {
            modifiers,
            capslock: Default::default(),
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("explorer_drag_copy_badge").is_some());
        cx.simulate_event(gpui::ModifiersChangedEvent {
            modifiers: Default::default(),
            capslock: Default::default(),
        });
        test_support::redraw(cx);
        assert!(cx.debug_bounds("explorer_drag_copy_badge").is_none());
        cx.update(|window, app| window.cancel_drag(app));
        test_support::redraw(cx);
        assert!(cx.debug_bounds("explorer_drag_preview").is_none());
    }
}

#[gpui::test]
fn explorer_autoscroll_survives_continuous_motion_and_cancels(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = long_explorer_state(400);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let row = cx.debug_bounds("file_browser_row_1").unwrap();
    start_file_drag(cx, row.center());
    let bounds = cx.update(|_, app| {
        pane.read(app)
            .file_browser_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
    });
    let offset = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            pane.read(app)
                .file_browser_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y
        })
    };
    for frame in 0..6 {
        // 1 kHz movement within the band must not postpone the scroll step.
        for ix in 0..16 {
            cx.simulate_mouse_move(
                gpui::point(
                    bounds.center().x + px((ix % 2) as f32),
                    bounds.bottom() - px(8.0),
                ),
                Some(gpui::MouseButton::Left),
                Default::default(),
            );
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(1));
        }
        drag_frame(cx, std::time::Duration::ZERO);
        assert!(
            offset(cx) < px(-4.0 * (frame + 1) as f32),
            "scroll stalled at {:?}",
            offset(cx)
        );
    }
    let scrolled = offset(cx);
    cx.simulate_mouse_move(
        bounds.center(),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    drag_frame(cx, std::time::Duration::from_millis(32));
    assert_eq!(offset(cx), scrolled);
    cx.update(|window, app| window.cancel_drag(app));
    test_support::redraw(cx);
    drag_frame(cx, std::time::Duration::from_millis(32));
    cx.update(|_, app| {
        let pane = pane.read(app);
        assert!(pane.explorer_scroll_task.is_none());
        assert!(pane.explorer_hover_task.is_none());
        assert!(pane.explorer_drop_target.is_none());
    });
}

#[gpui::test]
fn pointer_only_drag_reuses_the_cached_explorer(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = long_explorer_state(40);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let center = cx.debug_bounds("file_browser_row_1").unwrap().center();
    let other_rows = [
        "file_browser_row_2",
        "file_browser_row_3",
        "file_browser_row_4",
    ]
    .map(|selector| cx.debug_bounds(selector).unwrap().center());
    start_file_drag(cx, center);
    // Resolve the initial test coordinates before enabling cached mounts:
    // GPUI does not replay debug bounds from its cached paint ranges.
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    cx.simulate_mouse_move(center, Some(gpui::MouseButton::Left), Default::default());
    test_support::redraw(cx);
    let before = cx.update(|_, app| pane.read(app).render_count);
    for dx in 1..10 {
        cx.simulate_mouse_move(
            center + gpui::point(px(dx as f32), px(0.0)),
            Some(gpui::MouseButton::Left),
            Default::default(),
        );
        test_support::redraw(cx);
    }
    for position in other_rows {
        cx.simulate_mouse_move(position, Some(gpui::MouseButton::Left), Default::default());
        test_support::redraw(cx);
    }
    let after = cx.update(|_, app| pane.read(app).render_count);
    assert_eq!(
        before, after,
        "motion within and across rows sharing a destination must not rebuild the explorer"
    );
}

#[gpui::test]
fn explorer_drag_in_collapsed_files_popover_targets_rows_and_escape_cancels(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = explorer_state();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, Arc::clone(&state), cx);
            view.set_sidebar_collapsed(true, cx);
            view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
        })
    });
    for _ in 0..25 {
        drag_frame(cx, std::time::Duration::from_millis(16));
    }
    let source = cx.debug_bounds("file_browser_row_1").unwrap();
    let destination = cx.debug_bounds("file_browser_row_2").unwrap();
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    start_file_drag(cx, source.center());
    cx.simulate_mouse_move(
        destination.center(),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    test_support::redraw(cx);
    assert_eq!(
        cx.update(|_, app| pane.read(app).explorer_drop_target.clone()),
        Some(PathBuf::from("zulu"))
    );
    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    assert!(!cx.update(|_, app| app.has_active_drag()));
    assert!(cx.update(|_, app| pane.read(app).explorer_drop_target.is_none()));
}

#[gpui::test]
fn explorer_external_drag_highlights_subtrees_root_and_excludes_controls(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = (*explorer_state()).clone();
    if let Loadable::Ready(entries) = &mut state.repos[0].file_browser.entries {
        let entries = Arc::make_mut(entries);
        entries.insert(2, dir("nested", "alpha/nested", 1));
        entries.insert(3, file("child.txt", "alpha/nested/child.txt", 2));
    }
    state.repos[0]
        .file_browser
        .expanded_dirs
        .insert(Arc::new(PathBuf::from("alpha/nested")));
    let state = Arc::new(state);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    let first = cx.debug_bounds("file_browser_row_0").unwrap();
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Entered {
                position: first.center(),
                paths: gpui::ExternalPaths(
                    [PathBuf::from("/tmp/external.txt")].into_iter().collect(),
                ),
            }),
            app,
        );
    });
    for (row, target, expected) in [
        (0, "alpha", 0..4),
        (1, "alpha", 0..4),
        (2, "alpha/nested", 2..4),
        (3, "alpha/nested", 2..4),
        (5, "zulu", 4..6),
    ] {
        let bounds = cx
            .debug_bounds(
                [
                    "file_browser_row_0",
                    "file_browser_row_1",
                    "file_browser_row_2",
                    "file_browser_row_3",
                    "file_browser_row_4",
                    "file_browser_row_5",
                ][row],
            )
            .unwrap();
        cx.simulate_mouse_move(
            bounds.center(),
            Some(gpui::MouseButton::Left),
            Default::default(),
        );
        test_support::redraw(cx);
        cx.update(|_, app| {
            let pane = pane.read(app);
            assert_eq!(pane.explorer_drop_target, Some(PathBuf::from(target)));
            assert_eq!(
                pane.explorer_drop_range(&pane.file_browser_visible_rows(app)),
                expected
            );
        });
    }
    let last = cx.debug_bounds("file_browser_row_5").unwrap();
    cx.simulate_mouse_move(
        last.center() + gpui::point(px(0.0), first.size.height * 2.0),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    test_support::redraw(cx);
    cx.update(|window, app| {
        pane.update(app, |pane, cx| {
            assert_eq!(
                pane.explorer_pointer_target(window, cx),
                Some(PathBuf::new())
            )
        });
        let pane = pane.read(app);
        assert_eq!(pane.explorer_drop_target, Some(PathBuf::new()));
        assert_eq!(
            pane.explorer_drop_range(&pane.file_browser_visible_rows(app)),
            0..6
        );
    });
    cx.simulate_mouse_move(
        first.center() - gpui::point(px(0.0), first.size.height * 2.0),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    test_support::redraw(cx);
    assert!(cx.update(|_, app| pane.read(app).explorer_drop_target.is_none()));
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Exited),
            app,
        );
    });
}

#[gpui::test]
fn explorer_inline_editor_fits_rows_and_focus_does_not_shift_labels(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    for scale in [80, 100, 150, 200] {
        let state = explorer_state();
        cx.update(|window, app| {
            ui_scale::set_current(app, scale);
            ui_scale::apply_to_window(window, scale);
            pane.update(app, |pane, _| pane.explorer_name_edit = None);
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state.clone(), cx);
                view.set_sidebar_collapsed(false, cx);
            });
            window.refresh();
        });
        test_support::redraw(cx);
        let label = cx.debug_bounds("explorer_label_1").unwrap();
        let mut focused = (*state).clone();
        focused.repos[0].file_browser.selection.focused = Some(PathBuf::from("alpha/one.rs"));
        focused.repos[0]
            .file_browser
            .selection
            .paths
            .insert(PathBuf::from("alpha/one.rs"));
        let focused = Arc::new(focused);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(focused.clone());
                test_support::push_test_state(view, focused.clone(), cx);
            })
        });
        test_support::redraw(cx);
        assert_eq!(
            cx.debug_bounds("explorer_label_1").unwrap(),
            label,
            "focus must only affect paint at {scale}%"
        );
        for action in [
            ExplorerAction::NewFile,
            ExplorerAction::NewFolder,
            ExplorerAction::Rename,
        ] {
            cx.update(|window, app| {
                pane.update(app, |pane, cx| {
                    pane.explorer_action(
                        action,
                        Some(PathBuf::from(if action == ExplorerAction::Rename {
                            "alpha/one.rs"
                        } else {
                            "alpha"
                        })),
                        window,
                        cx,
                    );
                })
            });
            test_support::redraw(cx);
            let row = cx.debug_bounds("explorer_inline_row").unwrap();
            let field = cx.debug_bounds("explorer_inline_name").unwrap();
            let previous = cx.debug_bounds("file_browser_row_0").unwrap();
            let next = cx.debug_bounds("file_browser_row_2").unwrap();
            assert_eq!(row.size.height, previous.size.height);
            assert!(
                field.top() >= row.top() && field.bottom() <= row.bottom(),
                "field must fit at {scale}%: {field:?} in {row:?}"
            );
            assert!(row.top() >= previous.bottom() && row.bottom() <= next.top());
            assert_eq!(
                field.left(),
                label.left(),
                "text columns must align at {scale}%"
            );
            cx.simulate_keystrokes("escape");
            test_support::redraw(cx);
            assert!(cx.update(|_, app| pane.read(app).explorer_name_edit.is_none()));
        }
    }
}

#[gpui::test]
fn explorer_root_creation_follows_pinned_rows_and_pinned_rows_reject_drops(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("alpha")).unwrap();
    std::fs::write(directory.path().join("alpha/one.rs"), "fn one() {}\n").unwrap();
    let mut state = (*explorer_state()).clone();
    let repo = &mut state.repos[0];
    repo.spec.workdir = directory.path().to_path_buf();
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("alpha/one.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = true;
    let state = Arc::new(state);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
            view.main_pane
                .update(cx, |pane, cx| pane.ensure_file_editor_loaded(cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.main_pane.update(cx, |pane, cx| {
                pane.file_editor_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, "// unsaved\n", cx)
                });
            });
        })
    });
    cx.run_until_parked();
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    for popup in [false, true] {
        if popup {
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.set_sidebar_collapsed(true, cx);
                    view.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
                })
            });
            for _ in 0..25 {
                drag_frame(cx, std::time::Duration::from_millis(16));
            }
        }
        cx.update(|window, app| {
            pane.update(app, |pane, cx| {
                pane.explorer_action(ExplorerAction::NewFile, Some(PathBuf::new()), window, cx);
            })
        });
        test_support::redraw(cx);
        cx.update(|_, app| {
            let rows = pane.read(app).file_browser_visible_rows(app);
            assert!(matches!(
                rows[0],
                FileBrowserVisibleRow::UnsavedHeader { .. }
            ));
            assert!(matches!(rows[1], FileBrowserVisibleRow::UnsavedFile { .. }));
            assert!(matches!(
                rows[2],
                FileBrowserVisibleRow::NameEntry { depth: 0 }
            ));
            assert!(matches!(
                rows[3],
                FileBrowserVisibleRow::Entry { depth: 0, .. }
            ));
        });
        let inline = cx.debug_bounds("explorer_inline_row").unwrap();
        let first = cx.debug_bounds("file_browser_row_3").unwrap();
        assert!(inline.bottom() <= first.top());
        let header = cx.debug_bounds("file_browser_unsaved_header").unwrap();
        for point in [
            header.center(),
            header.center() + gpui::point(px(0.0), header.size.height),
            inline.center(),
        ] {
            cx.simulate_mouse_move(point, None, Default::default());
            cx.update(|window, app| {
                pane.update(app, |pane, cx| {
                    assert!(pane.explorer_pointer_target(window, cx).is_none());
                })
            });
        }
        cx.simulate_keystrokes("escape");
        test_support::redraw(cx);
    }
}

#[gpui::test]
fn explorer_empty_repository_accepts_root_focus_and_external_highlight(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = long_explorer_state(0);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    let area = cx.debug_bounds("explorer_empty_area").unwrap();
    cx.simulate_mouse_down(area.center(), gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_up(area.center(), gpui::MouseButton::Left, Default::default());
    cx.update(|window, app| assert!(pane.read(app).explorer_focus.is_focused(window)));
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Entered {
                position: area.center(),
                paths: gpui::ExternalPaths(
                    [PathBuf::from("/tmp/external.txt")].into_iter().collect(),
                ),
            }),
            app,
        );
    });
    cx.simulate_mouse_move(
        area.center(),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    test_support::redraw(cx);
    assert_eq!(
        cx.update(|_, app| pane.read(app).explorer_drop_target.clone()),
        Some(PathBuf::new())
    );
}
