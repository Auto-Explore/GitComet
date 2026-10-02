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
        ignored: false,
    }
}

fn file(name: &str, path: &str, depth: usize) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: Arc::new(PathBuf::from(path)),
        kind: FileEntryKind::File,
        depth,
        ignored: false,
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    for scale in [80, 100, 150, 200] {
        let state = explorer_state();
        cx.update(|window, app| {
            ui_scale::set_default(app, scale);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("alpha")).unwrap();
    std::fs::write(directory.path().join("alpha/one.rs"), "fn one() {}\n").unwrap();
    let mut state = (*explorer_state()).clone();
    let repo = &mut state.repos[0];
    repo.spec.workdir = directory.path().to_path_buf();
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("alpha/one.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
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
                FileBrowserVisibleRow::FileSetHeader { .. }
            ));
            assert!(matches!(rows[1], FileBrowserVisibleRow::FileSetFile { .. }));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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

/// `explorer_state()` over a real directory, so inline edits reach the disk.
fn explorer_state_in(workdir: &std::path::Path) -> Arc<AppState> {
    for (path, contents) in [
        ("alpha/one.rs", "fn one() {}\n"),
        ("zulu/two.rs", "fn two() {}\n"),
    ] {
        let path = workdir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    let mut state = (*explorer_state()).clone();
    state.repos[0].spec.workdir = workdir.to_path_buf();
    Arc::new(state)
}

/// Mounts `state` in an active window with the app's text-input keymap, so
/// Enter reaches the name field the way it does in the real app.
fn explorer_window(
    cx: &mut gpui::TestAppContext,
    state: Arc<AppState>,
) -> (
    gpui::Entity<GitCometView>,
    gpui::Entity<SidebarPaneView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(Arc::clone(&state));
            test_support::push_test_state(view, state, cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    (view, pane, cx)
}

fn start_inline_edit(
    cx: &mut gpui::VisualTestContext,
    pane: &gpui::Entity<SidebarPaneView>,
    action: ExplorerAction,
    path: &str,
) {
    cx.update(|window, app| {
        pane.update(app, |pane, cx| {
            pane.explorer_action(action, Some(PathBuf::from(path)), window, cx)
        })
    });
    test_support::redraw(cx);
}

fn set_inline_name(
    cx: &mut gpui::VisualTestContext,
    pane: &gpui::Entity<SidebarPaneView>,
    name: &str,
) {
    cx.update(|_, app| {
        let input = pane.read(app).explorer_name_input.clone();
        input.update(app, |input, cx| input.set_text(name.to_string(), cx));
    });
}

fn inline_edit_focused(
    cx: &mut gpui::VisualTestContext,
    pane: &gpui::Entity<SidebarPaneView>,
) -> bool {
    cx.update(|window, app| pane.read(app).explorer_inline_edit_owns_focus(window, app))
}

/// Focus something outside the explorer, as a click elsewhere would.
fn move_focus_elsewhere(cx: &mut gpui::VisualTestContext) -> gpui::FocusHandle {
    let other = cx.update(|window, app| {
        let other = app.focus_handle();
        window.focus(&other, app);
        other
    });
    // Blur listeners run during the next draw.
    test_support::redraw(cx);
    other
}

/// The filesystem worker runs on its own thread; poll the disk.
fn wait_for_path(cx: &mut gpui::VisualTestContext, path: &std::path::Path, exists: bool) {
    for _ in 0..300 {
        if path.exists() == exists {
            return;
        }
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("{} never became exists={exists}", path.display());
}

#[gpui::test]
fn explorer_inline_edit_enter_creates_the_file_and_returns_focus_to_the_tree(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let (_view, pane, cx) = explorer_window(cx, explorer_state_in(&root));

    start_inline_edit(cx, &pane, ExplorerAction::NewFile, "");
    assert!(
        inline_edit_focused(cx, &pane),
        "the name field must take the caret"
    );
    set_inline_name(cx, &pane, "new.txt");
    cx.simulate_keystrokes("enter");
    wait_for_path(cx, &root.join("new.txt"), true);
    cx.update(|window, app| {
        let pane = pane.read(app);
        assert!(pane.explorer_name_edit.is_none());
        assert!(
            pane.explorer_focus.is_focused(window),
            "Enter hands focus back to the tree"
        );
    });

    start_inline_edit(cx, &pane, ExplorerAction::NewFolder, "zulu");
    assert!(inline_edit_focused(cx, &pane));
    set_inline_name(cx, &pane, "nested");
    cx.simulate_keystrokes("enter");
    wait_for_path(cx, &root.join("zulu/nested"), true);
    assert!(root.join("zulu/nested").is_dir());

    start_inline_edit(cx, &pane, ExplorerAction::Rename, "alpha/one.rs");
    assert!(inline_edit_focused(cx, &pane));
    set_inline_name(cx, &pane, "renamed.rs");
    cx.simulate_keystrokes("enter");
    wait_for_path(cx, &root.join("alpha/renamed.rs"), true);
    assert!(!root.join("alpha/one.rs").exists());
}

#[gpui::test]
fn explorer_inline_edit_commits_on_focus_out_and_cancels_empty_or_unchanged_names(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let (view, pane, cx) = explorer_window(cx, explorer_state_in(&root));

    // A typed name is applied when focus leaves the field, and focus stays put.
    start_inline_edit(cx, &pane, ExplorerAction::Rename, "alpha/one.rs");
    set_inline_name(cx, &pane, "moved.rs");
    let other = move_focus_elsewhere(cx);
    wait_for_path(cx, &root.join("alpha/moved.rs"), true);
    cx.update(|window, app| {
        assert!(pane.read(app).explorer_name_edit.is_none());
        assert!(
            other.is_focused(window),
            "focus-out must not pull focus back"
        );
    });

    // Empty New folder and unchanged Rename end silently.
    start_inline_edit(cx, &pane, ExplorerAction::NewFolder, "");
    move_focus_elsewhere(cx);
    start_inline_edit(cx, &pane, ExplorerAction::Rename, "zulu/two.rs");
    move_focus_elsewhere(cx);
    cx.update(|_, app| assert!(pane.read(app).explorer_name_edit.is_none()));
    let store = cx.update(|_, app| view.read(app).store.clone());
    let entries = std::fs::read_dir(&root).unwrap().count();
    assert_eq!(entries, 2, "nothing new may appear in the root");
    assert!(root.join("zulu/two.rs").exists());

    // An invalid name on Enter is reported and the edit stays open to fix it.
    start_inline_edit(cx, &pane, ExplorerAction::NewFile, "");
    set_inline_name(cx, &pane, "a/b");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(inline_edit_focused(cx, &pane), "the field keeps the caret");
    let mut reported = false;
    for _ in 0..200 {
        reported = store.snapshot().repos[0]
            .feedback
            .diagnostics
            .iter()
            .any(|entry| entry.message.contains("file name"));
        if reported {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(reported, "an invalid name must be reported");

    // Escape cancels without creating anything.
    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    cx.update(|window, app| {
        let pane = pane.read(app);
        assert!(pane.explorer_name_edit.is_none());
        assert!(pane.explorer_focus.is_focused(window));
    });
    assert!(!root.join("a").exists());
}

#[gpui::test]
fn explorer_inline_edit_survives_window_deactivation(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let (_view, pane, cx) = explorer_window(cx, explorer_state_in(&root));

    start_inline_edit(cx, &pane, ExplorerAction::Rename, "alpha/one.rs");
    set_inline_name(cx, &pane, "kept.rs");
    cx.deactivate_window();
    test_support::redraw(cx);
    cx.update(|_, app| assert!(pane.read(app).explorer_name_edit.is_some()));
    assert!(
        root.join("alpha/one.rs").exists(),
        "switching windows is not a commit"
    );

    cx.update(|window, _| window.activate());
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(
        inline_edit_focused(cx, &pane),
        "the caret returns with the window"
    );
    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    cx.update(|_, app| assert!(pane.read(app).explorer_name_edit.is_none()));
    assert!(root.join("alpha/one.rs").exists());
}

#[gpui::test]
fn explorer_menu_rename_keeps_focus_in_the_name_field(cx: &mut gpui::TestAppContext) {
    use crate::view::panels::ContextMenuAction;

    let _guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let (view, pane, cx) = explorer_window(cx, explorer_state_in(&root));
    let host = cx.update(|_, app| view.read(app).popover_host.clone());
    for (action, editing) in [
        (ExplorerAction::Rename, true),
        (ExplorerAction::Copy, false),
    ] {
        // A real right-click, so the menu records the tree as its invoker.
        let row = cx
            .debug_bounds("file_browser_row_1")
            .expect("alpha/one.rs row");
        cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Default::default());
        test_support::redraw(cx);
        assert!(
            cx.update(|_, app| host.read(app).popover_kind_for_tests().is_some()),
            "the row menu opens"
        );
        cx.update(|window, app| {
            host.update(app, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::Explorer {
                        repo_id: RepoId(7),
                        path: PathBuf::from("alpha/one.rs"),
                        action,
                    },
                    window,
                    cx,
                )
            })
        });
        test_support::redraw(cx);
        cx.update(|window, app| {
            assert!(host.read(app).popover_kind_for_tests().is_none());
            let pane = pane.read(app);
            assert_eq!(
                pane.explorer_inline_edit_owns_focus(window, app),
                editing,
                "{action:?}: closing the menu must leave an inline edit focused"
            );
            if !editing {
                assert!(
                    pane.explorer_focus.is_focused(window),
                    "other actions hand focus back to the tree"
                );
            }
        });
        cx.simulate_keystrokes("escape");
        test_support::redraw(cx);
    }
}

#[gpui::test]
fn dragging_a_folder_over_itself_or_its_descendants_offers_no_target(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (_view, pane, cx) = explorer_window(cx, explorer_state());
    let alpha = cx.debug_bounds("file_browser_row_0").expect("alpha row");
    start_file_drag(cx, alpha.center());
    for (selector, expected) in [
        ("file_browser_row_0", None),
        ("file_browser_row_1", None),
        ("file_browser_row_2", Some(PathBuf::from("zulu"))),
        ("file_browser_row_3", Some(PathBuf::from("zulu"))),
    ] {
        let row = cx.debug_bounds(selector).unwrap();
        cx.simulate_mouse_move(
            row.center(),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        test_support::redraw(cx);
        cx.update(|window, app| {
            pane.update(app, |pane, cx| {
                assert_eq!(
                    pane.explorer_drop_target, expected,
                    "{selector}: dragged alpha may only highlight elsewhere"
                );
                assert_eq!(pane.explorer_pointer_target(window, cx), expected);
            })
        });
    }
}

#[gpui::test]
fn hovering_a_dragged_collapsed_folder_does_not_expand_it(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, _pane, cx) = explorer_window(cx, collapsed_folder_state());
    let store = cx.update(|_, app| view.read(app).store.clone());
    let alpha = cx
        .debug_bounds("file_browser_row_0")
        .expect("collapsed alpha row");
    start_file_drag(cx, alpha.center());
    cx.simulate_mouse_move(
        alpha.center(),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(900));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !store.snapshot().repos[0]
            .file_browser
            .expanded_dirs
            .contains(&PathBuf::from("alpha")),
        "a folder cannot open under its own drag"
    );
}

#[gpui::test]
fn dropping_a_folder_onto_itself_or_its_parent_is_a_silent_no_op(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let (view, pane, cx) = explorer_window(cx, explorer_state_in(&root));
    for target in ["alpha", ""] {
        cx.update(|window, app| {
            pane.update(app, |pane, cx| {
                pane.explorer_drop(
                    vec![root.join("alpha")],
                    Some(PathBuf::from(target)),
                    false,
                    window,
                    cx,
                )
            });
            assert!(
                !view.read(app).file_operations.has_pending(),
                "dropping alpha onto {target:?} must not start an operation"
            );
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(
                view.read(app)
                    .toast_host
                    .read(app)
                    .error_notices()
                    .is_empty()
            );
        });
        assert!(root.join("alpha/one.rs").exists());
    }
    // A real destination still moves.
    cx.update(|window, app| {
        pane.update(app, |pane, cx| {
            pane.explorer_drop(
                vec![root.join("alpha")],
                Some(PathBuf::from("zulu")),
                false,
                window,
                cx,
            )
        });
        assert!(view.read(app).file_operations.has_pending());
    });
    wait_for_path(cx, &root.join("zulu/alpha/one.rs"), true);
}
