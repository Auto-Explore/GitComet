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

    // Five autoscroll ticks with no further mouse input.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(150));
    cx.run_until_parked();

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
