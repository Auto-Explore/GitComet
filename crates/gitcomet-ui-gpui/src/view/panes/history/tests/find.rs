//! The history find bar (Cmd-F over the history list), driven through the
//! real key bindings. `BlockingBackend` gives the store no history index, so
//! the bar runs in its paged mode and matches the loaded log page directly;
//! selections still round-trip through the store as `Msg::SelectCommit`.

use super::*;

const FIND_REPO_ID: RepoId = RepoId(1);

fn authored(id: &str, summary: &str, author: &str) -> Commit {
    Commit {
        author: author.into(),
        ..commit(id, &[], summary)
    }
}

/// Top to bottom as the list shows them. "fix" matches rows 0, 2 and 4 (in
/// three different cases); "bob" matches rows 1 and 5 by author only; the
/// SHA prefix "dddd3" matches row 3 and no summary or author.
fn find_fixture_commits() -> Vec<Commit> {
    vec![
        authored("aaaa0000", "Fix login bug", "Alice"),
        authored("bbbb1111", "Add feature", "Bob"),
        authored("cccc2222", "fix typo in README", "Carol"),
        authored("dddd3333", "Refactor parser", "Alice"),
        authored("eeee4444", "FIX crash on start", "Dave"),
        authored("ffff5555", "Docs", "Bob"),
    ]
}

fn find_fixture_repo(commits: Vec<Commit>) -> RepoState {
    let page = Arc::new(log_page(commits, None));
    let workdir = PathBuf::from(format!(
        "/tmp/history-find-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut repo = RepoState::new_opening(FIND_REPO_ID, RepoSpec { workdir });
    // Everything the panes read is already loaded, so rendering never has
    // to ask the store (and its worker threads) for data.
    repo.open = Loadable::Ready(());
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "aaaa0000")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    repo
}

/// Mounts `repo` in both the view and the store (the rows dispatch into the
/// store, and the reducer mutates exactly this state), installs the app and
/// text-input key bindings in production order, and focuses the history list
/// when it is showing.
fn mount_find_fixture(
    cx: &mut gpui::TestAppContext,
    repo: RepoState,
) -> (
    gpui::Entity<GitCometView>,
    AppStore,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let history_showing = repo.diff_state.diff_target.is_none();
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(FIND_REPO_ID),
        ..AppState::test_default()
    });
    store_for_assert.replace_snapshot_for_test(Arc::clone(&state));
    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| model.set_state(Arc::clone(&state), cx));
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    if history_showing {
        ensure_history_cache_for_tests(cx, &view, state);
        wait_until(cx, "history rows", |cx| {
            cx.update(|_window, app| {
                let history = history_view(&view, app).read(app);
                history.history_cache.is_some()
                    && history.history_scroll.0.borrow().last_item_size.is_some()
            })
        });
    }

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::bind_text_input_keys_for_test(app);
        if history_showing {
            let focus = history_view(&view, app)
                .read(app)
                .history_panel_focus_handle
                .clone();
            window.focus(&focus, app);
        }
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    (view, store_for_assert, cx)
}

fn history_view(view: &gpui::Entity<GitCometView>, app: &gpui::App) -> gpui::Entity<HistoryView> {
    view.read(app).main_pane.read(app).history_view.clone()
}

fn draw_and_park(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn find_is_open(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> bool {
    cx.update(|_window, app| history_view(view, app).read(app).history_find_is_open())
}

fn find_input(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> gpui::Entity<components::TextInput> {
    cx.update(|_window, app| {
        history_view(view, app)
            .read(app)
            .find
            .as_ref()
            .expect("the find bar should have been created")
            .input
            .clone()
    })
}

fn find_input_is_focused(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> bool {
    let input = find_input(cx, view);
    cx.update(|window, app| input.read(app).focus_handle().is_focused(window))
}

fn history_panel_is_focused(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> bool {
    cx.update(|window, app| {
        history_view(view, app)
            .read(app)
            .history_panel_focus_handle
            .is_focused(window)
    })
}

/// The selection the store holds.
fn store_selected(store: &AppStore) -> Option<String> {
    store
        .snapshot()
        .repos
        .iter()
        .find(|repo| repo.id == FIND_REPO_ID)
        .and_then(|repo| repo.history_state.selected_commit.as_ref())
        .map(|id| id.as_ref().to_string())
}

/// The selection the history view has seen, which is what stepping starts
/// from and what the match label reads.
fn view_selected(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<String> {
    cx.update(|_window, app| {
        history_view(view, app)
            .read(app)
            .active_repo()
            .and_then(|repo| repo.history_state.selected_commit.as_ref())
            .map(|id| id.as_ref().to_string())
    })
}

/// The test runtime has no live store poller, so the view only sees what the
/// reducer did once a test pulls the store's snapshot into it.
fn sync_view_with_store(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

/// Waits for `expected` to be selected in the store and for the history view
/// to have caught up with it.
fn wait_for_selection(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    expected: &str,
) {
    wait_until(cx, &format!("{expected} to be selected"), |cx| {
        sync_view_with_store(cx, view);
        store_selected(store).as_deref() == Some(expected)
            && view_selected(cx, view).as_deref() == Some(expected)
    });
}

/// What the match label is computed from: the matching visible rows, whether
/// the scan finished, and the 1-based position of the selected commit among
/// the matches (`k` in "k of N").
#[derive(Debug, PartialEq, Eq)]
struct FindStatus {
    matches: Vec<usize>,
    complete: bool,
    current: Option<usize>,
}

fn find_status(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<FindStatus> {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| {
            let matches = history.history_find_matches()?;
            let selected = history
                .active_repo()
                .and_then(|repo| repo.history_state.selected_commit.clone());
            let cache = history.history_cache.as_ref()?;
            let selected_visible = selected.and_then(|selected| {
                cache.base.visible_indices.iter().position(|commit_ix| {
                    cache
                        .page
                        .commits
                        .get(commit_ix)
                        .is_some_and(|commit| commit.id == selected)
                })
            });
            let current = selected_visible
                .and_then(|visible| matches.visible.binary_search(&visible).ok())
                .map(|ix| ix + 1);
            Some(FindStatus {
                matches: matches.visible.clone(),
                complete: matches.complete,
                current,
            })
        })
    })
}

fn assert_find_status(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    matches: &[usize],
    current: Option<usize>,
    context: &str,
) {
    assert_eq!(
        find_status(cx, view),
        Some(FindStatus {
            matches: matches.to_vec(),
            complete: true,
            current,
        }),
        "{context}"
    );
}

/// Opens the bar with Cmd-F from the focused history list.
fn open_find_with_shortcut(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.simulate_keystrokes("secondary-f");
    draw_and_park(cx);
    assert!(
        find_is_open(cx, view),
        "secondary-f over the history list must open its find bar"
    );
}

fn type_query(cx: &mut gpui::VisualTestContext, query: &str) {
    cx.simulate_input(query);
    draw_and_park(cx);
    settle_typing(cx);
}

/// Lets the find bar's post-keystroke quiet period elapse.
fn settle_typing(cx: &mut gpui::VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(
        crate::view::panes::history::find::HISTORY_FIND_SETTLE_MS + 30,
    ));
    draw_and_park(cx);
}

/// Replaces the whole query, the way a user retyping it would.
fn retype_query(cx: &mut gpui::VisualTestContext, query: &str) {
    cx.simulate_keystrokes("secondary-a");
    type_query(cx, query);
}

/// Lets any queued store dispatches land, then checks the selection held.
fn assert_selection_settles_on(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    expected: &str,
    context: &str,
) {
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_with_store(cx, view);
        assert_eq!(
            store_selected(store).as_deref(),
            Some(expected),
            "{context}"
        );
    }
    assert_eq!(
        view_selected(cx, view).as_deref(),
        Some(expected),
        "{context}"
    );
}

#[gpui::test]
fn secondary_f_over_history_opens_the_find_bar_and_focuses_its_input(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, _store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    assert!(!find_is_open(cx, &view));
    assert!(cx.debug_bounds("history_find").is_none());
    assert!(history_panel_is_focused(cx, &view));

    open_find_with_shortcut(cx, &view);

    for selector in [
        "history_find",
        "history_find_input_slot",
        "history_find_match_label",
        "history_find_prev",
        "history_find_next",
        "history_find_close",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "expected {selector} to be rendered once the bar opens"
        );
    }
    assert!(
        find_input_is_focused(cx, &view),
        "opening the find bar must focus its input"
    );
    assert!(
        !cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_active),
        "the history find bar is not the diff search"
    );
    assert_eq!(
        find_status(cx, &view),
        None,
        "an empty query matches nothing (the label reads \"0 results\")"
    );
}

#[gpui::test]
fn secondary_f_with_a_diff_visible_opens_diff_search_not_history_find(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut repo = find_fixture_repo(find_fixture_commits());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(
        gitcomet_core::domain::Diff {
            target,
            lines: Vec::new(),
        }
        .into(),
    );
    repo.diff_state.diff_rev = 1;
    let (view, _store, cx) = mount_find_fixture(cx, repo);
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-f");
    draw_and_park(cx);

    cx.update(|window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_active,
            "secondary-f with a diff visible must still open the diff search"
        );
        assert!(
            pane.diff_search_input
                .read(app)
                .focus_handle()
                .is_focused(window),
            "and focus the diff search input"
        );
    });
    assert!(
        !find_is_open(cx, &view),
        "secondary-f with a diff visible must not open the history find bar"
    );
    assert!(cx.debug_bounds("history_find").is_none());
}

#[gpui::test]
fn history_find_query_selects_the_first_match(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    assert_eq!(store_selected(&store), None);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");

    assert_eq!(
        find_input(cx, &view).read_with(cx, |input, _| input.text().to_owned()),
        "fix"
    );
    wait_for_selection(cx, &view, &store, "aaaa0000");
    assert_find_status(
        cx,
        &view,
        &[0, 2, 4],
        Some(1),
        "\"fix\" matches three rows regardless of case, and the top one is selected (\"1 of 3\")",
    );
    assert!(
        find_input_is_focused(cx, &view),
        "selecting the first match must leave focus in the find input"
    );
}

#[gpui::test]
fn history_find_enter_and_shift_enter_step_through_matches_and_wrap(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");

    // Enter steps down through the matches and wraps back to the top.
    for (expected, position) in [("cccc2222", 2), ("eeee4444", 3), ("aaaa0000", 1)] {
        cx.simulate_keystrokes("enter");
        draw_and_park(cx);
        wait_for_selection(cx, &view, &store, expected);
        assert_find_status(
            cx,
            &view,
            &[0, 2, 4],
            Some(position),
            &format!("enter should move to match {position} of 3 ({expected})"),
        );
        assert!(find_input_is_focused(cx, &view));
        assert_eq!(
            find_input(cx, &view).read_with(cx, |input, _| input.text().to_owned()),
            "fix",
            "enter must not edit the single-line query"
        );
    }

    // Shift-Enter resolves to `HistoryFindPrevious` inside the bar and steps
    // back up, wrapping from the first match to the last.
    for (expected, position) in [("eeee4444", 3), ("cccc2222", 2), ("aaaa0000", 1)] {
        cx.simulate_keystrokes("shift-enter");
        draw_and_park(cx);
        wait_for_selection(cx, &view, &store, expected);
        assert_find_status(
            cx,
            &view,
            &[0, 2, 4],
            Some(position),
            &format!("shift-enter should move to match {position} of 3 ({expected})"),
        );
        assert!(find_input_is_focused(cx, &view));
        assert_eq!(
            find_input(cx, &view).read_with(cx, |input, _| input.text().to_owned()),
            "fix",
            "shift-enter must not insert a line break into the query"
        );
    }
}

#[gpui::test]
fn history_find_f3_and_f2_step_through_matches(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");

    cx.simulate_keystrokes("f3");
    draw_and_park(cx);
    wait_for_selection(cx, &view, &store, "cccc2222");

    cx.simulate_keystrokes("f2");
    draw_and_park(cx);
    wait_for_selection(cx, &view, &store, "aaaa0000");

    cx.simulate_keystrokes("f2");
    draw_and_park(cx);
    wait_for_selection(cx, &view, &store, "eeee4444");
    assert!(find_input_is_focused(cx, &view));
}

#[gpui::test]
fn history_find_matches_sha_prefix_and_author(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);

    // A hex query of four or more characters matches the start of the SHA.
    type_query(cx, "DDDD3");
    wait_for_selection(cx, &view, &store, "dddd3333");
    assert_find_status(
        cx,
        &view,
        &[3],
        Some(1),
        "a SHA prefix matches only the commit whose id starts with it (\"1 of 1\")",
    );

    // An author name matches every commit by that author.
    retype_query(cx, "bob");
    wait_for_selection(cx, &view, &store, "bbbb1111");
    assert_find_status(
        cx,
        &view,
        &[1, 5],
        Some(1),
        "an author query matches that author's commits (\"1 of 2\")",
    );

    retype_query(cx, "Carol");
    wait_for_selection(cx, &view, &store, "cccc2222");
    assert_find_status(cx, &view, &[2], Some(1), "author match is case-insensitive");
}

#[gpui::test]
fn history_find_without_matches_reports_none_and_keeps_the_selection(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    type_query(cx, "refactor");
    wait_for_selection(cx, &view, &store, "dddd3333");

    retype_query(cx, "zzz");
    assert_find_status(
        cx,
        &view,
        &[],
        None,
        "a query nothing matches has a complete, empty result (\"0 results\")",
    );
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        "dddd3333",
        "a query with no matches must leave the selection alone",
    );

    // Nothing to step to: the keys fall through without moving the selection.
    cx.simulate_keystrokes("enter");
    draw_and_park(cx);
    cx.simulate_keystrokes("shift-enter");
    draw_and_park(cx);
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        "dddd3333",
        "stepping with no matches must leave the selection alone",
    );
}

#[gpui::test]
fn history_find_escape_closes_and_secondary_f_restores_the_query(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");

    // Escape in the input closes the bar and hands focus back to the list.
    cx.simulate_keystrokes("escape");
    draw_and_park(cx);
    assert!(!find_is_open(cx, &view), "escape must close the find bar");
    assert!(
        cx.debug_bounds("history_find").is_none(),
        "the closed bar must not be rendered"
    );
    assert!(
        history_panel_is_focused(cx, &view),
        "closing the find bar must return focus to the history list"
    );
    assert_eq!(
        find_status(cx, &view),
        None,
        "a closed bar reports no matches"
    );

    // Cmd-F brings the bar back with the previous query selected.
    open_find_with_shortcut(cx, &view);
    assert!(cx.debug_bounds("history_find").is_some());
    assert!(find_input_is_focused(cx, &view));
    let input = find_input(cx, &view);
    assert_eq!(
        input.read_with(cx, |input, _| input.text().to_owned()),
        "fix"
    );
    assert_eq!(
        input.read_with(cx, |input, _| input.selected_range()),
        0..3,
        "reopening must select the whole previous query"
    );
    draw_and_park(cx);
    assert_eq!(
        find_status(cx, &view).map(|status| status.matches),
        Some(vec![0, 2, 4]),
        "the restored query must be searched again"
    );

    // Escape with the history list focused also closes the bar.
    cx.update(|window, app| {
        let focus = history_view(&view, app)
            .read(app)
            .history_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    assert!(find_is_open(cx, &view));
    cx.simulate_keystrokes("escape");
    draw_and_park(cx);
    assert!(
        !find_is_open(cx, &view),
        "escape on the history list must close an open find bar"
    );
    assert!(cx.debug_bounds("history_find").is_none());
    assert!(history_panel_is_focused(cx, &view));
}

#[gpui::test]
fn history_find_buttons_step_and_close(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");

    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let at = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
            .center();
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        cx.simulate_click(at, gpui::Modifiers::default());
        draw_and_park(cx);
    };

    click(cx, "history_find_prev");
    wait_for_selection(cx, &view, &store, "eeee4444");
    click(cx, "history_find_next");
    wait_for_selection(cx, &view, &store, "aaaa0000");
    click(cx, "history_find_next");
    wait_for_selection(cx, &view, &store, "cccc2222");

    click(cx, "history_find_close");
    assert!(!find_is_open(cx, &view), "× must close the find bar");
    assert!(cx.debug_bounds("history_find").is_none());
    assert!(
        history_panel_is_focused(cx, &view),
        "closing with × must return focus to the history list"
    );
}

/// The text the bar's count label shows.
fn find_label(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> String {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| history.history_find_label().to_string())
    })
}

/// Visible rows drawn faded, decided by the same rule the row renderer uses.
fn dimmed_rows(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> Vec<usize> {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| {
            let query = history.history_find_query().cloned();
            let selected = history
                .active_repo()
                .and_then(|repo| repo.history_state.selected_commit.clone());
            let Some(cache) = history.history_cache.as_ref() else {
                return Vec::new();
            };
            cache
                .base
                .visible_indices
                .iter()
                .enumerate()
                .filter_map(|(visible_ix, commit_ix)| {
                    let commit = cache.page.commits.get(commit_ix)?;
                    let is_selected = Some(&commit.id) == selected.as_ref();
                    crate::view::panes::history::find::history_find_row_dimmed(
                        query.as_ref(),
                        commit,
                        is_selected,
                    )
                    .then_some(visible_ix)
                })
                .collect()
        })
    })
}

/// Moves the selection one row down, the way the Down key on the list would.
fn select_next_row(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, cx| {
            history.history_select_adjacent_commit(1, cx);
        })
    });
    draw_and_park(cx);
}

#[gpui::test]
fn history_find_selects_the_first_match_only_once_typing_settles(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    cx.simulate_input("fix");
    draw_and_park(cx);
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_with_store(cx, &view);
        assert_eq!(
            store_selected(&store),
            None,
            "nothing is selected while the query is still being typed"
        );
    }
    assert_eq!(
        dimmed_rows(cx, &view),
        vec![1, 3, 5],
        "rows fade with every keystroke, before the query settles"
    );

    settle_typing(cx);
    wait_for_selection(cx, &view, &store, "aaaa0000");
}

#[gpui::test]
fn history_find_enter_while_typing_jumps_to_the_first_match(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    cx.simulate_input("fix");
    cx.simulate_keystrokes("enter");
    draw_and_park(cx);
    wait_for_selection(cx, &view, &store, "aaaa0000");
}

#[gpui::test]
fn history_find_label_reports_position_count_and_no_results(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    assert_eq!(find_label(cx, &view), "0 results", "an empty query");

    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");
    assert_eq!(find_label(cx, &view), "1 of 3");

    cx.simulate_keystrokes("enter");
    draw_and_park(cx);
    wait_for_selection(cx, &view, &store, "cccc2222");
    assert_eq!(find_label(cx, &view), "2 of 3");

    // Off a match, the label counts the matches instead of placing one.
    select_next_row(cx, &view);
    wait_for_selection(cx, &view, &store, "dddd3333");
    assert_eq!(find_label(cx, &view), "3 results");

    retype_query(cx, "dddd3");
    assert_eq!(
        find_label(cx, &view),
        "1 of 1",
        "the only match is already selected"
    );
    select_next_row(cx, &view);
    wait_for_selection(cx, &view, &store, "eeee4444");
    assert_eq!(find_label(cx, &view), "1 result");

    retype_query(cx, "zzz");
    assert_eq!(find_label(cx, &view), "0 results");
}

#[gpui::test]
fn history_find_dims_misses_but_not_matches_or_the_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    assert_eq!(
        dimmed_rows(cx, &view),
        Vec::<usize>::new(),
        "an empty query dims nothing"
    );

    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");
    assert_eq!(
        dimmed_rows(cx, &view),
        vec![1, 3, 5],
        "every row that misses \"fix\" fades; the matches stay bright"
    );

    // A selected miss stays readable; the match it left is still bright.
    select_next_row(cx, &view);
    wait_for_selection(cx, &view, &store, "bbbb1111");
    assert_eq!(dimmed_rows(cx, &view), vec![3, 5]);

    retype_query(cx, "zzz");
    assert_eq!(
        dimmed_rows(cx, &view),
        vec![0, 2, 3, 4, 5],
        "with no matches everything but the selection fades"
    );

    cx.simulate_keystrokes("escape");
    draw_and_park(cx);
    assert!(!find_is_open(cx, &view));
    assert_eq!(
        dimmed_rows(cx, &view),
        Vec::<usize>::new(),
        "closing the bar brings every row back"
    );
}

#[test]
fn row_dimming_follows_the_rows_own_text() {
    use crate::view::panes::history::find::history_find_row_dimmed;
    use gitcomet_core::history_find::HistoryFindQuery;

    let query = HistoryFindQuery::new("fix");
    let hit = authored("aaaa0000", "Fix login bug", "Alice");
    let miss = authored("bbbb1111", "Add feature", "Bob");

    assert!(!history_find_row_dimmed(query.as_ref(), &hit, false));
    assert!(history_find_row_dimmed(query.as_ref(), &miss, false));
    assert!(
        !history_find_row_dimmed(query.as_ref(), &miss, true),
        "the selected row stays bright even when it misses"
    );
    assert!(
        !history_find_row_dimmed(None, &miss, false),
        "no query, nothing fades"
    );
}

// ---------------------------------------------------------------------------
// Indexed history
// ---------------------------------------------------------------------------
//
// With a history index the bar stops matching loaded rows itself: it asks the
// store to scan every indexed row (`HistoryFindMsg::Find`) and maps the raw
// rows the scan reports, chunk by chunk, onto the displayed list. These tests
// hand the store a fake repository so that request reaches the real scan
// effect, and then either let the scan run (`ScanMode::Serve`, `Fail`) or hold
// it and report its chunks by hand (`ScanMode::Hold`), so each stage of a
// streaming scan can be looked at.

use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::history_index::{HistoryIndexBuilder, HistoryIndexHandle, HistoryRange};
use gitcomet_core::services::{CancellationToken, HistorySnapshot};
use gitcomet_state::history_find::{HistoryFindChunk, HistoryFindMsg, HistoryFindState};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Long enough that the scan spans several blocks and a match can sit far
/// below the viewport.
const INDEXED_ROWS: usize = 600;
/// A probable stash. Its second parent is the stash's index commit, which the
/// list hides.
const STASH_ROW: usize = 20;
/// The hidden stash helper. Its summary matches "fix", so a scan reports it.
const STASH_HELPER_ROW: usize = 21;
/// A "fix" match far below the first screen.
const FAR_FIX_ROW: usize = 300;

/// Raw index rows matching "fix": 0, 5, the hidden helper, and the far row.
/// "bob" matches rows 2 and 7 by author.
fn indexed_find_commit_text(row: usize) -> (String, &'static str) {
    match row {
        0 => ("Fix login bug".into(), "Alice"),
        2 => ("Add feature".into(), "Bob"),
        5 => ("fix typo in README".into(), "Carol"),
        7 => ("Docs".into(), "Bob"),
        STASH_ROW => ("WIP on main: 1234567 wip".into(), "author"),
        STASH_HELPER_ROW => ("index on main: 1234567 fix helper".into(), "author"),
        FAR_FIX_ROW => ("FIX crash far below".into(), "Dave"),
        _ => (format!("commit {row}"), "author"),
    }
}

fn indexed_find_raw_id(row: usize) -> [u8; 20] {
    let mut id = [0u8; 20];
    id[..8].copy_from_slice(&((INDEXED_ROWS - row) as u64).to_be_bytes());
    id
}

/// The commit id of raw index row `row`.
fn indexed_find_id(row: usize) -> String {
    gitcomet_core::hex::encode(&indexed_find_raw_id(row))
}

/// Where raw row `row` shows in the list: rows below the hidden stash helper
/// move up by one.
fn indexed_visible(row: usize) -> usize {
    assert_ne!(row, STASH_HELPER_ROW, "the stash helper is not shown");
    row - usize::from(row > STASH_HELPER_ROW)
}

/// A linear history, except that the stash row also has the helper as its
/// second parent.
fn indexed_find_history() -> (HistoryIndexHandle, Vec<Commit>) {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("history-find-indexed".into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap();
    let mut commits = Vec::with_capacity(INDEXED_ROWS);
    for row in 0..INDEXED_ROWS {
        let parents: Vec<[u8; 20]> = match row {
            STASH_ROW => vec![
                indexed_find_raw_id(STASH_HELPER_ROW + 1),
                indexed_find_raw_id(STASH_HELPER_ROW),
            ],
            _ if row + 1 < INDEXED_ROWS => vec![indexed_find_raw_id(row + 1)],
            _ => Vec::new(),
        };
        builder
            .push(
                &indexed_find_raw_id(row),
                parents.iter().map(|id| id.as_slice()),
                row == STASH_ROW,
            )
            .unwrap();
        let (summary, author) = indexed_find_commit_text(row);
        commits.push(Commit {
            id: CommitId(indexed_find_id(row).into()),
            parent_ids: parents
                .iter()
                .map(|id| CommitId(gitcomet_core::hex::encode(id).into()))
                .collect(),
            summary: summary.into(),
            author: author.into(),
            time: SystemTime::UNIX_EPOCH,
        });
    }
    (builder.finish(&CancellationToken::new()).unwrap(), commits)
}

/// What the find scan does once it reaches the repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanMode {
    /// Read the fixture's commits, so the real scan runs to the end.
    Serve,
    /// Fail the first read.
    Fail,
    /// Block until cancelled; the test reports the scan's chunks itself.
    Hold,
}

/// Serves the fixture's commits to the viewport's range loads, and to the
/// find scan as `ScanMode` says.
struct IndexedFindRepo {
    spec: RepoSpec,
    commits: Vec<Commit>,
    mode: ScanMode,
    scans_started: AtomicUsize,
    released: AtomicBool,
}

/// The viewport's range loads run on the store's named test pools; the find
/// scan has its own unnamed worker (see `store/effects/history_find.rs`).
fn on_find_scan_thread() -> bool {
    std::thread::current()
        .name()
        .is_none_or(|name| !name.starts_with("gitcomet-test-store"))
}

fn not_needed<T>() -> Result<T> {
    Err(Error::new(ErrorKind::Unsupported(
        "not needed by the history find tests",
    )))
}

impl GitRepository for IndexedFindRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn read_history_range(
        &self,
        index: &HistoryIndexHandle,
        range: std::ops::Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<HistoryRange> {
        cancellation.check_cancelled()?;
        if on_find_scan_thread() {
            if range.start == 0 {
                self.scans_started.fetch_add(1, Ordering::SeqCst);
            }
            match self.mode {
                ScanMode::Serve => {}
                ScanMode::Fail => {
                    return Err(Error::new(ErrorKind::Backend("history read failed".into())));
                }
                ScanMode::Hold => {
                    // The scan executor is one thread shared by every test,
                    // so a held scan must end with its search or its test.
                    while !self.released.load(Ordering::SeqCst) {
                        cancellation.check_cancelled()?;
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    return Err(Error::new(ErrorKind::Cancelled));
                }
            }
        }
        let commits = self
            .commits
            .get(range.clone())
            .ok_or_else(|| Error::new(ErrorKind::Backend("range out of bounds".into())))?
            .to_vec();
        Ok(HistoryRange {
            snapshot: index.snapshot.clone(),
            start: range.start,
            commits,
        })
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<Arc<LogPage>> {
        not_needed()
    }
    fn commit_details(&self, _id: &CommitId) -> Result<gitcomet_core::domain::CommitDetails> {
        not_needed()
    }
    fn reflog_head(&self, _limit: usize) -> Result<Vec<gitcomet_core::domain::ReflogEntry>> {
        not_needed()
    }
    fn current_branch(&self) -> Result<String> {
        not_needed()
    }
    fn list_branches(&self) -> Result<Vec<Branch>> {
        not_needed()
    }
    fn list_remotes(&self) -> Result<Vec<gitcomet_core::domain::Remote>> {
        not_needed()
    }
    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        not_needed()
    }
    fn status(&self) -> Result<gitcomet_core::domain::RepoStatus> {
        not_needed()
    }
    fn diff_unified(&self, _target: &gitcomet_core::domain::DiffTarget) -> Result<String> {
        not_needed()
    }
    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        not_needed()
    }
    fn delete_branch(&self, _name: &str) -> Result<()> {
        not_needed()
    }
    fn checkout_branch(&self, _name: &str) -> Result<()> {
        not_needed()
    }
    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        not_needed()
    }
    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        not_needed()
    }
    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        not_needed()
    }
    fn stash_list(&self) -> Result<Vec<gitcomet_core::domain::StashEntry>> {
        not_needed()
    }
    fn stash_apply(&self, _index: usize) -> Result<()> {
        not_needed()
    }
    fn stash_drop(&self, _index: usize) -> Result<()> {
        not_needed()
    }
    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        not_needed()
    }
    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        not_needed()
    }
    fn commit(&self, _message: &str) -> Result<()> {
        not_needed()
    }
    fn fetch_all(&self) -> Result<()> {
        not_needed()
    }
    fn pull(&self, _mode: gitcomet_core::services::PullMode) -> Result<()> {
        not_needed()
    }
    fn push(&self) -> Result<()> {
        not_needed()
    }
    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        not_needed()
    }
}

/// The fake repository, releasing any held scan when the test ends.
struct IndexedFindBackend(Arc<IndexedFindRepo>);

impl IndexedFindBackend {
    /// How many scans have reached the repository.
    fn scans_started(&self) -> usize {
        self.0.scans_started.load(Ordering::SeqCst)
    }
}

impl Drop for IndexedFindBackend {
    fn drop(&mut self) {
        self.0.released.store(true, Ordering::SeqCst);
    }
}

/// [`mount_find_fixture`], then the index is installed and the list switches
/// to its indexed mode.
fn mount_indexed_find_fixture(
    cx: &mut gpui::TestAppContext,
    mode: ScanMode,
) -> (
    gpui::Entity<GitCometView>,
    AppStore,
    &mut gpui::VisualTestContext,
    IndexedFindBackend,
) {
    let (index, commits) = indexed_find_history();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(commits.clone()));
    let repo = Arc::new(IndexedFindRepo {
        spec: store.snapshot().repos[0].spec.clone(),
        commits,
        mode,
        scans_started: AtomicUsize::new(0),
        released: AtomicBool::new(false),
    });
    store.insert_repo_for_test(FIND_REPO_ID, repo.clone());
    let backend = IndexedFindBackend(repo);

    let mut state = (*store.snapshot()).clone();
    install_index(&mut state, index.clone());
    let state = Arc::new(state);
    store.replace_snapshot_for_test(Arc::clone(&state));
    set_history_view_state_for_tests(cx, &view, state);
    wait_until(cx, "indexed history", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
            && cx.update(|_window, app| {
                history_view(&view, app)
                    .read(app)
                    .indexed
                    .presentation
                    .as_ref()
                    .is_some_and(|shown| Arc::ptr_eq(&shown.graph.projection.index, &index))
            })
    });
    // The store selects rows only against the index the view published.
    wait_until(cx, "published index", |_| {
        store.snapshot().repos[0]
            .history_state
            .indexed
            .displayed_index
            .as_ref()
            .is_some_and(|shown| Arc::ptr_eq(shown, &index))
    });
    cx.update(|window, app| {
        let focus = history_view(&view, app)
            .read(app)
            .history_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    (view, store, cx, backend)
}

fn store_find(store: &AppStore) -> HistoryFindState {
    store.snapshot().repos[0].history_state.find.clone()
}

/// Waits for the view's request for `query` to reach the store and returns
/// the search's generation.
fn wait_for_find_request(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    query: &str,
) -> u64 {
    let query = HistoryFindQuery::new(query);
    wait_until(cx, "the search request", |cx| {
        sync_view_with_store(cx, view);
        store_find(store).query == query
    });
    store_find(store).generation()
}

/// Reports what a scan would, for the current search, and lets the view see it.
fn report_scan(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    result: Result<HistoryFindChunk>,
) {
    let before = store_find(store);
    store.dispatch(Msg::HistoryFind(HistoryFindMsg::Found {
        repo_id: FIND_REPO_ID,
        seq: before.generation(),
        result,
    }));
    wait_until(cx, "the scan report", |cx| {
        sync_view_with_store(cx, view);
        store_find(store).rev != before.rev
    });
    draw_and_park(cx);
}

fn report_matches(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    raw_rows: &[usize],
    done: bool,
) {
    report_scan(
        cx,
        view,
        store,
        Ok(HistoryFindChunk {
            matches: raw_rows.iter().map(|&row| row as u32).collect(),
            done,
        }),
    );
}

#[derive(Debug, PartialEq, Eq)]
struct IndexedFindStatus {
    matches: Vec<usize>,
    complete: bool,
    pending: bool,
    failed: bool,
}

fn indexed_find_status(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<IndexedFindStatus> {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| {
            assert!(
                history.indexed.presentation.is_some(),
                "the list must still be in its indexed mode"
            );
            let matches = history.history_find_matches()?;
            Some(IndexedFindStatus {
                matches: matches.visible.clone(),
                complete: matches.complete,
                pending: matches.pending,
                failed: matches.failed,
            })
        })
    })
}

fn found(matches: &[usize], complete: bool) -> Option<IndexedFindStatus> {
    Some(IndexedFindStatus {
        matches: matches.to_vec(),
        complete,
        pending: false,
        failed: false,
    })
}

/// Loaded indexed rows in `rows` drawn faded, by the rule the row renderer uses.
fn indexed_dimmed_rows(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    rows: std::ops::Range<usize>,
) -> Vec<usize> {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| {
            let query = history.history_find_query().cloned();
            let selected = history
                .active_repo()
                .and_then(|repo| repo.history_state.selected_commit.clone());
            let window = history
                .indexed
                .window
                .as_ref()
                .expect("the indexed rows should be built");
            let dimmed: Vec<usize> = window
                .cache
                .page
                .commits
                .iter()
                .enumerate()
                .filter(|(ix, _)| window.loaded.get(*ix).copied().unwrap_or(false))
                .map(|(ix, commit)| (window.start + ix, commit))
                .filter(|(visible_ix, _)| rows.contains(visible_ix))
                .filter(|(_, commit)| {
                    crate::view::panes::history::find::history_find_row_dimmed(
                        query.as_ref(),
                        commit,
                        selected.as_ref() == Some(&commit.id),
                    )
                })
                .map(|(visible_ix, _)| visible_ix)
                .collect();
            dimmed
        })
    })
}

/// The indexed list's scroll position, viewport height and row height.
fn indexed_viewport(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> (f64, f64, f64) {
    cx.update(|_window, app| {
        let history = history_view(view, app);
        let history = history.read(app);
        let scroll = history.scroll_interaction.borrow();
        let logical = scroll
            .logical
            .as_ref()
            .expect("the indexed list should have a viewport");
        (logical.position(), logical.viewport, logical.height)
    })
}

/// The top of visible row `visible_ix`, in the indexed list's coordinates.
fn indexed_row_top(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    visible_ix: usize,
) -> f64 {
    let list_ix = cx.update(|_window, app| {
        history_view(view, app)
            .read(app)
            .indexed
            .plan
            .list_ix_for_visible(visible_ix)
    });
    let (_, _, height) = indexed_viewport(cx, view);
    list_ix as f64 * height
}

fn indexed_row_in_view(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    visible_ix: usize,
) -> bool {
    let top = indexed_row_top(cx, view, visible_ix);
    let (position, viewport, height) = indexed_viewport(cx, view);
    top >= position && top + height <= position + viewport
}

fn assert_indexed_row_centred(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    visible_ix: usize,
    context: &str,
) {
    let top = indexed_row_top(cx, view, visible_ix);
    let (position, viewport, height) = indexed_viewport(cx, view);
    let centre = position + viewport / 2.0;
    assert!(
        (top + height / 2.0 - centre).abs() <= height,
        "{context}: row {visible_ix} (top {top}) should be centred in {position}..{}",
        position + viewport
    );
}

/// Presses `keys` in the find input and waits for raw row `row` to be selected.
fn step_to(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    keys: &str,
    row: usize,
) {
    cx.simulate_keystrokes(keys);
    draw_and_park(cx);
    wait_for_selection(cx, view, store, &indexed_find_id(row));
}

#[gpui::test]
fn indexed_history_find_scans_the_whole_index_and_selects_the_first_match(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Serve);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_until(cx, "the scan to finish", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).done
    });
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));

    let results = store_find(&store);
    assert!(results.error.is_none(), "{:?}", results.error);
    assert_eq!(
        results.match_rows().collect::<Vec<_>>(),
        vec![0, 5, STASH_HELPER_ROW, FAR_FIX_ROW],
        "the scan reads every indexed row, the hidden stash helper included"
    );
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[0, 5, indexed_visible(FAR_FIX_ROW)], true),
        "the hidden stash helper is not a match in the list"
    );
    assert_eq!(find_label(cx, &view), "1 of 3");
    assert_eq!(backend.scans_started(), 1);

    // An answered search is not asked for again on later frames.
    let generation = results.generation();
    for _ in 0..5 {
        sync_view_with_store(cx, &view);
    }
    assert_eq!(store_find(&store).generation(), generation);
    assert_eq!(backend.scans_started(), 1, "the finished scan was repeated");

    step_to(cx, &view, &store, "enter", 5);
    assert_eq!(find_label(cx, &view), "2 of 3");
    step_to(cx, &view, &store, "enter", FAR_FIX_ROW);
    assert_eq!(find_label(cx, &view), "3 of 3");
    assert_indexed_row_centred(
        cx,
        &view,
        indexed_visible(FAR_FIX_ROW),
        "a match below the viewport is scrolled to the middle",
    );
    step_to(cx, &view, &store, "enter", 0);
    assert_eq!(find_label(cx, &view), "1 of 3", "enter wraps to the top");
    assert!(indexed_row_in_view(cx, &view, 0));
    assert!(find_input_is_focused(cx, &view));
}

#[gpui::test]
fn indexed_history_find_selects_the_first_match_from_the_first_chunk(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, "fix");
    wait_until(cx, "the scan to start", |_| backend.scans_started() == 1);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[], false),
        "nothing is known until the scan reports"
    );
    assert_eq!(store_selected(&store), None);

    report_matches(cx, &view, &store, &[0, 5, STASH_HELPER_ROW], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[0, 5], false),
        "the stash helper the scan reported is hidden, so it is not counted"
    );
    assert_eq!(
        find_label(cx, &view),
        "1 of 2+",
        "the scan is still running"
    );

    report_matches(cx, &view, &store, &[FAR_FIX_ROW], true);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[0, 5, indexed_visible(FAR_FIX_ROW)], true)
    );
    assert_eq!(find_label(cx, &view), "1 of 3", "the scan is done");
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        &indexed_find_id(0),
        "later chunks do not move the selection",
    );
}

#[gpui::test]
fn indexed_history_find_steps_across_streamed_chunks_and_wraps(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5, STASH_HELPER_ROW], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));

    // A match already on screen is selected without scrolling the list.
    assert!(indexed_row_in_view(cx, &view, 5));
    let position = indexed_viewport(cx, &view).0;
    step_to(cx, &view, &store, "enter", 5);
    assert_eq!(find_label(cx, &view), "2 of 2+");
    assert_eq!(indexed_viewport(cx, &view).0, position);
    // Only the first chunk is known yet, so stepping wraps within it.
    step_to(cx, &view, &store, "enter", 0);
    assert_eq!(find_label(cx, &view), "1 of 2+");

    let first_chunk = indexed_find_status(cx, &view).unwrap().matches;
    report_matches(cx, &view, &store, &[FAR_FIX_ROW], true);
    let status = indexed_find_status(cx, &view).unwrap();
    assert_eq!(
        status.matches[..first_chunk.len()],
        first_chunk[..],
        "a later chunk extends the matches already mapped"
    );
    assert_eq!(
        status,
        found(&[0, 5, indexed_visible(FAR_FIX_ROW)], true).unwrap()
    );
    assert_eq!(find_label(cx, &view), "1 of 3");

    step_to(cx, &view, &store, "f3", 5);
    assert_eq!(find_label(cx, &view), "2 of 3");
    step_to(cx, &view, &store, "f3", FAR_FIX_ROW);
    assert_eq!(
        find_label(cx, &view),
        "3 of 3",
        "stepping reaches the match from the second chunk"
    );
    assert_indexed_row_centred(
        cx,
        &view,
        indexed_visible(FAR_FIX_ROW),
        "an off-screen match is centred",
    );
    step_to(cx, &view, &store, "f3", 0);
    assert_eq!(find_label(cx, &view), "1 of 3", "f3 wraps to the top");
    assert!(indexed_row_in_view(cx, &view, 0));

    step_to(cx, &view, &store, "shift-enter", FAR_FIX_ROW);
    assert_eq!(
        find_label(cx, &view),
        "3 of 3",
        "shift-enter wraps to the end"
    );
    assert!(indexed_row_in_view(cx, &view, indexed_visible(FAR_FIX_ROW)));
    step_to(cx, &view, &store, "f2", 5);
    assert_eq!(find_label(cx, &view), "2 of 3");
    step_to(cx, &view, &store, "shift-enter", 0);
    assert_eq!(find_label(cx, &view), "1 of 3");
    assert!(find_input_is_focused(cx, &view));
}

#[gpui::test]
fn indexed_history_find_ignores_matches_on_hidden_stash_helper_rows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    cx.update(|_window, app| {
        let history = history_view(&view, app);
        let projection = &history
            .read(app)
            .indexed
            .presentation
            .as_ref()
            .unwrap()
            .graph
            .projection;
        assert_eq!(projection.visible_position(STASH_HELPER_ROW), None);
        assert_eq!(projection.visible_position(STASH_ROW), Some(STASH_ROW));
    });

    open_find_with_shortcut(cx, &view);
    type_query(cx, "helper");
    wait_for_find_request(cx, &view, &store, "helper");
    report_matches(cx, &view, &store, &[STASH_HELPER_ROW], true);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[], true),
        "a match on a hidden row is not shown"
    );
    assert_eq!(find_label(cx, &view), "0 results");
    assert_selection_settles_on_nothing(cx, &view, &store);

    // Rows below the hidden helper are counted at their shown positions.
    retype_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[STASH_HELPER_ROW, FAR_FIX_ROW], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(FAR_FIX_ROW));
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[FAR_FIX_ROW - 1], true)
    );
    assert_eq!(find_label(cx, &view), "1 of 1");
}

fn assert_selection_settles_on_nothing(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
) {
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_with_store(cx, view);
        assert_eq!(store_selected(store), None, "nothing should be selected");
    }
}

#[gpui::test]
fn indexed_history_find_edit_holds_the_count_and_dims_rows_at_once(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, "fix");
    report_matches(
        cx,
        &view,
        &store,
        &[0, 5, STASH_HELPER_ROW, FAR_FIX_ROW],
        true,
    );
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(find_label(cx, &view), "1 of 3");
    assert_eq!(
        indexed_dimmed_rows(cx, &view, 0..12),
        vec![1, 2, 3, 4, 6, 7, 8, 9, 10, 11]
    );

    // Still typing: the store has not been asked, so its results are for the
    // old query.
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("bob");
    draw_and_park(cx);
    sync_view_with_store(cx, &view);
    assert_eq!(store_find(&store).query, HistoryFindQuery::new("fix"));
    assert_eq!(
        indexed_find_status(cx, &view).map(|status| status.pending),
        Some(true)
    );
    assert_eq!(
        find_label(cx, &view),
        "1 of 3",
        "the last count is held instead of flashing \"Searching…\""
    );
    assert_eq!(
        indexed_dimmed_rows(cx, &view, 0..12),
        vec![1, 3, 4, 5, 6, 8, 9, 10, 11],
        "rows fade by the new query at once; the selected row stays bright"
    );

    settle_typing(cx);
    wait_for_find_request(cx, &view, &store, "bob");
    report_matches(cx, &view, &store, &[2, 7], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(2));
    assert_eq!(indexed_find_status(cx, &view), found(&[2, 7], true));
    assert_eq!(find_label(cx, &view), "1 of 2");
}

#[gpui::test]
fn indexed_history_find_holds_the_count_until_the_new_scan_answers(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, "fix");
    report_matches(
        cx,
        &view,
        &store,
        &[0, 5, STASH_HELPER_ROW, FAR_FIX_ROW],
        true,
    );
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(find_label(cx, &view), "1 of 3");

    // The store has taken the new query, but its scan has not reported yet.
    retype_query(cx, "bob");
    wait_for_find_request(cx, &view, &store, "bob");
    draw_and_park(cx);
    assert_eq!(
        find_label(cx, &view),
        "1 of 3",
        "the last count is held until the new scan answers, not \"Searching…\""
    );

    report_matches(cx, &view, &store, &[2], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(2));
    assert_eq!(find_label(cx, &view), "1 of 1+");
}

/// `HistoryFindState::interrupt` (run when the repository's loads are
/// cancelled) is crate-private to the store, and every message that reaches
/// it also reloads or switches the repository. Clearing the search from the
/// store's side, which cancels the scan and bumps its generation just as the
/// interrupt does, leaves the view in the same place: its search unanswered.
#[gpui::test]
fn indexed_history_find_asks_again_when_an_unfinished_scan_is_dropped(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    let generation = wait_for_find_request(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    step_to(cx, &view, &store, "enter", 5);
    assert_eq!(find_label(cx, &view), "2 of 2+");
    wait_until(cx, "the first scan", |_| backend.scans_started() == 1);

    store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
        repo_id: FIND_REPO_ID,
        query: None,
        index: None,
    }));
    wait_until(cx, "the search to be asked for again", |cx| {
        sync_view_with_store(cx, &view);
        let results = store_find(&store);
        results.query == HistoryFindQuery::new("fix") && results.generation() == generation + 2
    });
    wait_until(cx, "the scan to restart", |_| backend.scans_started() == 2);
    assert!(store_find(&store).matches.is_empty());

    report_matches(
        cx,
        &view,
        &store,
        &[0, 5, STASH_HELPER_ROW, FAR_FIX_ROW],
        true,
    );
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[0, 5, indexed_visible(FAR_FIX_ROW)], true)
    );
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        &indexed_find_id(5),
        "the restarted search does not jump back to the first match",
    );
    assert_eq!(find_label(cx, &view), "2 of 3");
}

#[gpui::test]
fn indexed_history_find_reports_a_failed_scan(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Fail);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_until(cx, "the scan to fail", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).error.is_some()
    });
    draw_and_park(cx);
    assert_eq!(backend.scans_started(), 1);
    assert_eq!(
        indexed_find_status(cx, &view),
        Some(IndexedFindStatus {
            matches: Vec::new(),
            complete: true,
            pending: false,
            failed: true,
        })
    );
    assert_eq!(find_label(cx, &view), "Search failed");
    assert_selection_settles_on_nothing(cx, &view, &store);
}

#[gpui::test]
fn indexed_history_find_keeps_partial_matches_after_a_failure(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));

    report_scan(
        cx,
        &view,
        &store,
        Err(Error::new(ErrorKind::Backend("history read failed".into()))),
    );
    assert_eq!(
        indexed_find_status(cx, &view),
        Some(IndexedFindStatus {
            matches: vec![0, 5],
            complete: true,
            pending: false,
            failed: true,
        })
    );
    assert_eq!(find_label(cx, &view), "Search failed");
    step_to(cx, &view, &store, "enter", 5);
    assert_eq!(
        find_label(cx, &view),
        "Search failed",
        "the matches found before the failure can still be stepped through"
    );
}
