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
    let rows = match &repo.log {
        Loadable::Ready(page) => page.commits.len(),
        _ => 0,
    };
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
            cx.debug_bounds("history_row_0").is_some()
                && cx.update(|_window, app| {
                    history_view(&view, app)
                        .read(app)
                        .history_cache
                        .as_ref()
                        .is_some_and(|cache| cache.base.row_vms.len() == rows)
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
