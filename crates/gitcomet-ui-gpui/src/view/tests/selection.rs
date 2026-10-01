//! Selection reconciliation and home-screen selection.

use super::*;

#[test]
fn reconcile_status_multi_selection_prunes_missing_paths_and_anchors() {
    let a = PathBuf::from("a.txt");
    let b = PathBuf::from("b.txt");
    let c = PathBuf::from("c.txt");

    let status = RepoStatus {
        staged: std::sync::Arc::new(vec![]),
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: a.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
    };

    let mut selection = StatusMultiSelection {
        explicit_section: Some(StatusSection::CombinedUnstaged),
        untracked: vec![],
        untracked_anchor: None,
        unstaged: vec![a.clone(), b.clone()],
        unstaged_anchor: Some(b),
        unstaged_anchor_index: None,
        unstaged_anchor_order_rev: None,
        staged: vec![c.clone()],
        staged_anchor: Some(c),
        staged_anchor_index: None,
        staged_anchor_order_rev: None,
    };

    reconcile_status_multi_selection(&mut selection, &status);

    assert_eq!(selection.unstaged, vec![a]);
    assert!(selection.unstaged_anchor.is_none());
    assert!(selection.staged.is_empty());
    assert!(selection.staged_anchor.is_none());
}

#[gpui::test]
fn home_search_filters_workspaces_and_repositories(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let alpha = gitcomet_state::session::Workspace::new(vec![PathBuf::from("/work/alpha")]);
    let beta = gitcomet_state::session::Workspace::new(vec![PathBuf::from("/work/beta")]);
    let alpha_row: &'static str = format!("home_workspace_{}", alpha.id).leak();
    let beta_row: &'static str = format!("home_workspace_{}", beta.id).leak();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![alpha, beta]);
        view.update(app, |view, _cx| {
            view.home_recent_repos = vec![PathBuf::from("/work/gamma")];
            view.home_pinned_repos.clear();
        });
        let _ = window.draw(app);
    });
    let gamma_row: &'static str = format!(
        "home_recent_{}",
        gitcomet_state::session::path_storage_key(Path::new("/work/gamma"))
    )
    .leak();
    assert!(cx.debug_bounds(alpha_row).is_some());
    assert!(cx.debug_bounds(beta_row).is_some());
    assert!(cx.debug_bounds(gamma_row).is_some());

    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.home_search_input
                .update(cx, |input, cx| input.set_text("ALPHA", cx));
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds(alpha_row).is_some(),
        "matches by name, ignoring case"
    );
    assert!(cx.debug_bounds(beta_row).is_none());
    assert!(cx.debug_bounds(gamma_row).is_none());
}

fn home_selected(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Option<usize> {
    cx.update(|_window, app| view.read(app).home_selected)
}

#[gpui::test]
fn new_window_focuses_the_home_search(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = home_view_with(cx, Vec::new(), Vec::new());
    let focused = cx.update(|window, app| {
        view.read(app)
            .home_search_input
            .read(app)
            .focus_handle()
            .is_focused(window)
    });
    assert!(focused, "a new window on Home is ready to type into");
}

#[gpui::test]
fn home_selects_the_first_row_and_arrows_walk_both_columns(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = home_view_with(
        cx,
        vec![
            named_saved_workspace("Alpha", "/work/a"),
            named_saved_workspace("Beta", "/work/b"),
        ],
        vec![PathBuf::from("/work/c"), PathBuf::from("/work/d")],
    );
    assert_eq!(
        home_selected(&view, cx),
        Some(0),
        "the first row starts selected"
    );

    press(cx, "down down");
    assert_eq!(
        home_selected(&view, cx),
        Some(2),
        "Down continues into the repositories"
    );
    press(cx, "up");
    assert_eq!(
        home_selected(&view, cx),
        Some(1),
        "Up returns to the last workspace"
    );
    press(cx, "down down down");
    assert_eq!(home_selected(&view, cx), Some(0), "the run wraps around");
}

#[gpui::test]
fn home_left_and_right_jump_between_columns(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = home_view_with(
        cx,
        vec![
            named_saved_workspace("Alpha", "/work/a"),
            named_saved_workspace("Beta", "/work/b"),
        ],
        vec![PathBuf::from("/work/c"), PathBuf::from("/work/d")],
    );
    press(cx, "down");
    assert_eq!(home_selected(&view, cx), Some(1));
    press(cx, "right");
    assert_eq!(
        home_selected(&view, cx),
        Some(3),
        "Right keeps the row position"
    );
    press(cx, "left");
    assert_eq!(home_selected(&view, cx), Some(1), "Left jumps back");

    // With text and the caret mid-text, Left edits the query instead.
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.home_search_input
                .update(cx, |input, cx| input.set_text("work", cx));
        });
    });
    cx.run_until_parked();
    let selected = home_selected(&view, cx);
    press(cx, "left");
    assert_eq!(
        home_selected(&view, cx),
        selected,
        "the caret moves, not the selection"
    );
}

#[gpui::test]
fn home_typing_reselects_the_first_match_and_enter_opens_it(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut empty = gitcomet_state::session::Workspace::new(Vec::new());
    empty.custom_name = Some("Later".to_string());
    empty.restore_on_launch = false;
    let id = empty.id;
    let (view, cx) = home_view_with(
        cx,
        vec![named_saved_workspace("Alpha", "/work/a"), empty],
        vec![PathBuf::from("/work/c")],
    );
    press(cx, "down down");
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            view.home_search_input
                .update(cx, |input, cx| input.set_text("later", cx));
        });
    });
    cx.run_until_parked();
    assert_eq!(
        home_selected(&view, cx),
        Some(0),
        "a new query selects its first match"
    );

    press(cx, "enter");
    let window_id = cx.update(|window, _app| window.window_handle().window_id());
    assert_eq!(
        cx.update(|_window, app| {
            crate::workspaces::workspace_for_window(app, window_id).map(|workspace| workspace.id)
        }),
        Some(id),
        "Enter opens the selected workspace in this window"
    );
}

#[gpui::test]
fn home_lists_are_virtualized_and_capped(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let recents = (0..50)
        .map(|ix| PathBuf::from(format!("/work/repo-{ix:02}")))
        .collect::<Vec<_>>();
    let row_selector = |ix: usize| -> &'static str {
        format!(
            "home_recent_{}",
            gitcomet_state::session::path_storage_key(Path::new(&format!("/work/repo-{ix:02}")))
        )
        .leak()
    };
    let (view, cx) = home_view_with(cx, Vec::new(), recents);

    let first = cx
        .debug_bounds(row_selector(0))
        .expect("first row rendered");
    assert!(
        cx.debug_bounds(row_selector(49)).is_none(),
        "rows far below the fold are not rendered"
    );
    let frame = cx.debug_bounds("home_recent_list").expect("list frame");
    let cap = first.size.height * crate::view::home::HOME_LIST_MAX_ROWS as f32;
    assert!(
        frame.size.height <= cap + px(16.0),
        "the list stops growing at {} rows ({:?} > {:?})",
        crate::view::home::HOME_LIST_MAX_ROWS,
        frame.size.height,
        cap
    );

    press(cx, "up");
    assert_eq!(
        home_selected(&view, cx),
        Some(49),
        "Up from the first row wraps to the last"
    );
    assert!(
        cx.debug_bounds(row_selector(49)).is_some(),
        "the selection scrolls into view"
    );
}

#[gpui::test]
fn home_selected_row_shows_the_enter_hint(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let alpha = named_saved_workspace("Alpha", "/work/a");
    let alpha_row: &'static str = format!("home_workspace_{}", alpha.id).leak();
    let repo_row: &'static str = format!(
        "home_recent_{}",
        gitcomet_state::session::path_storage_key(Path::new("/work/c"))
    )
    .leak();
    let (_view, cx) = home_view_with(cx, vec![alpha], vec![PathBuf::from("/work/c")]);

    let hint = cx
        .debug_bounds("home_enter_hint")
        .expect("Enter hint on the selection");
    let row = cx.debug_bounds(alpha_row).expect("first row");
    assert!(
        row.contains(&hint.center()),
        "the hint sits on the selected row"
    );

    press(cx, "down");
    let hint = cx
        .debug_bounds("home_enter_hint")
        .expect("hint follows the selection");
    assert!(
        cx.debug_bounds(repo_row)
            .expect("repo row")
            .contains(&hint.center())
    );
}

#[gpui::test]
fn home_cross_removes_a_recent_repository(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx) = home_view_with(
        cx,
        Vec::new(),
        vec![PathBuf::from("/work/c"), PathBuf::from("/work/d")],
    );
    // The selected row keeps its cross visible without hovering.
    let cross = cx
        .debug_bounds("home_recent_remove_0")
        .expect("remove cross on the selected repository");
    cx.simulate_click(cross.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    let recents = cx.update(|_window, app| view.read(app).home_recent_repos.clone());
    assert_eq!(recents, vec![PathBuf::from("/work/d")]);
    assert_eq!(home_selected(&view, cx), Some(0), "a row stays selected");
}

#[gpui::test]
fn home_cross_deletes_a_saved_workspace_but_not_an_open_one(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let saved = named_saved_workspace("Saved", "/work/a");
    let saved_id = saved.id;
    let mut open = named_saved_workspace("Open elsewhere", "/work/b");
    open.restore_on_launch = true;
    let open_id = open.id;
    let (_view, cx) = home_view_with(cx, vec![open, saved], Vec::new());
    let other = cx.cx.add_window(|_, _| gpui::Empty);
    cx.cx.update(|app| {
        crate::workspaces::sync_window(
            app,
            other.window_id(),
            Some(open_id),
            vec!["/work/b".into()],
            None,
        );
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    // Rows: the open workspace first (index 0, selected), then the saved one.
    assert!(
        cx.debug_bounds("home_workspace_remove_0").is_none(),
        "a workspace open in another window cannot be removed from here"
    );
    press(cx, "down");
    let cross = cx
        .debug_bounds("home_workspace_remove_1")
        .expect("remove cross on the saved workspace");
    cx.simulate_click(cross.center(), gpui::Modifiers::default());
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(crate::workspaces::workspace(app, saved_id).is_none());
        assert!(crate::workspaces::workspace(app, open_id).is_some());
    });
}
