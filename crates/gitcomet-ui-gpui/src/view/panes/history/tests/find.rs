//! The history find bar (Cmd-F over the history list), driven through the
//! real key bindings. `BlockingBackend` gives the store no history index, so
//! the bar runs in its paged mode and matches the loaded log page directly;
//! selections still round-trip through the store as `Msg::SelectCommit`.

use super::*;
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::text_search::TextSearchOptions;

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
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    draw_and_park(cx);

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
    draw_and_park(cx);

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
        window.simulate_next_frame(app);
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

fn find_text(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> String {
    find_input(cx, view).read_with(cx, |input, _| input.text().to_owned())
}

fn find_input_is_focused(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> bool {
    let input = find_input(cx, view);
    cx.update(|window, app| input.read(app).focus_handle().is_focused(window))
}

fn focus_history_panel(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|window, app| {
        let focus = history_view(view, app)
            .read(app)
            .history_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
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

/// Lets any queued store dispatches land, then checks the selection held.
fn assert_selection_settles_on(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    expected: Option<&str>,
    context: &str,
) {
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_with_store(cx, view);
        assert_eq!(store_selected(store).as_deref(), expected, "{context}");
    }
    assert_eq!(view_selected(cx, view).as_deref(), expected, "{context}");
}

/// The visible rows the current query matches (none without a query).
fn find_matches(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> Vec<usize> {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| {
            history
                .history_find_matches()
                .map(|matches| matches.visible.clone())
                .unwrap_or_default()
        })
    })
}

fn find_status(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> components::QuickSearchStatus {
    cx.update(|_window, app| {
        history_view(view, app).update(app, |history, _cx| history.history_find_status())
    })
}

/// The text the bar's match label shows.
fn find_label(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) -> String {
    find_status(cx, view).label().to_string()
}

/// Opens the bar with Cmd-F from the focused history list.
fn open_find_with_shortcut(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.simulate_keystrokes("secondary-f");
    draw_and_park(cx);
    // Deliver the mount animation's frames before typing into its input.
    cx.executor().advance_clock(Duration::from_millis(150));
    draw_and_park(cx);
    assert!(find_input_is_focused(cx, view));
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

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
        .center();
    cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
    cx.simulate_click(at, gpui::Modifiers::default());
    draw_and_park(cx);
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
                    let row = cache.base.row_vms.get(visible_ix)?;
                    let is_selected = Some(&commit.id) == selected.as_ref();
                    crate::view::panes::history::find::history_find_row_marks(
                        query.as_ref(),
                        commit,
                        is_selected,
                        row.summary.as_ref(),
                        row.author.as_ref(),
                        "",
                    )
                    .0
                    .then_some(visible_ix)
                })
                .collect()
        })
    })
}

const MATCH_CASE: &str = "history_find_match_case";
const WHOLE_WORD: &str = "history_find_whole_word";
const REGEX: &str = "history_find_regex";

/// One user action in a [`run_script`] step.
#[derive(Clone, Copy, Debug)]
enum Act {
    /// Replace the query and let typing settle.
    Type(&'static str),
    Key(&'static str),
    Click(&'static str),
    /// Move the selection one row down, as the Down key on the list does.
    NextRow,
}

/// Plays `script` on an open bar. After each act it checks the matching
/// visible rows, the selected commit and the match label, and that the input
/// kept focus and the query text. An act expected to leave the selection
/// alone is checked to keep leaving it alone.
fn run_script(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    script: &[(Act, &[usize], &str, &str)],
) {
    let mut text = find_text(cx, view);
    for &(act, matches, selected, label) in script {
        let before = view_selected(cx, view);
        match act {
            Act::Type(query) => {
                retype_query(cx, query);
                text = query.to_owned();
            }
            Act::Key(keys) => {
                cx.simulate_keystrokes(keys);
                draw_and_park(cx);
            }
            Act::Click(selector) => click(cx, selector),
            Act::NextRow => select_next_row(cx, view),
        }
        let context = format!("after {act:?}");
        if before.as_deref() == Some(selected) {
            assert_selection_settles_on(cx, view, store, Some(selected), &context);
        } else {
            wait_for_selection(cx, view, store, selected);
        }
        assert_eq!(find_matches(cx, view), matches, "{context}");
        assert_eq!(find_label(cx, view), label, "{context}");
        assert!(find_input_is_focused(cx, view), "{context}: input focus");
        assert_eq!(find_text(cx, view), text, "{context}: query text");
    }
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
        MATCH_CASE,
        WHOLE_WORD,
        REGEX,
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
        cx.debug_bounds("history_find_newline").is_none(),
        "the query is a single line"
    );
    assert!(
        find_input_is_focused(cx, &view),
        "opening the find bar must focus its input"
    );
    assert!(
        !cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_active),
        "the history find bar is not the diff search"
    );
    assert_eq!(find_matches(cx, &view), Vec::<usize>::new());
    assert_eq!(find_label(cx, &view), "Type to search");
}

#[gpui::test]
fn secondary_f_with_a_diff_visible_opens_diff_search_not_history_find(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut repo = find_fixture_repo(find_fixture_commits());
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
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

/// Typing selects the first match; Enter/F3/↓ step down and Shift-Enter
/// (`HistoryFindPrevious` inside the bar)/F2/↑ step up, wrapping at either
/// end, without editing the query or leaving the input; × closes the bar.
#[gpui::test]
fn history_find_selects_the_first_match_and_steps_with_keys_and_buttons(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    assert_eq!(store_selected(&store), None);

    open_find_with_shortcut(cx, &view);
    const FIX: &[usize] = &[0, 2, 4];
    run_script(
        cx,
        &view,
        &store,
        &[
            (Act::Type("fix"), FIX, "aaaa0000", "1/3"),
            (Act::Key("enter"), FIX, "cccc2222", "2/3"),
            (Act::Key("enter"), FIX, "eeee4444", "3/3"),
            (Act::Key("enter"), FIX, "aaaa0000", "1/3"),
            (Act::Key("shift-enter"), FIX, "eeee4444", "3/3"),
            (Act::Key("shift-enter"), FIX, "cccc2222", "2/3"),
            (Act::Key("f3"), FIX, "eeee4444", "3/3"),
            (Act::Key("f3"), FIX, "aaaa0000", "1/3"),
            (Act::Key("f2"), FIX, "eeee4444", "3/3"),
            (Act::Key("f2"), FIX, "cccc2222", "2/3"),
            (Act::Click("history_find_prev"), FIX, "aaaa0000", "1/3"),
            (Act::Click("history_find_prev"), FIX, "eeee4444", "3/3"),
            (Act::Click("history_find_next"), FIX, "aaaa0000", "1/3"),
            (Act::Click("history_find_next"), FIX, "cccc2222", "2/3"),
        ],
    );

    click(cx, "history_find_close");
    assert!(!find_is_open(cx, &view), "× must close the find bar");
    assert!(cx.debug_bounds("history_find").is_none());
    assert!(
        history_panel_is_focused(cx, &view),
        "closing with × must return focus to the history list"
    );
}

/// The summary, the author (ignoring case) and a SHA prefix all match; off a
/// match the label counts instead of placing; with no matches, stepping
/// leaves the selection alone.
#[gpui::test]
fn history_find_matches_and_labels(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    run_script(
        cx,
        &view,
        &store,
        &[
            (Act::Type("fix"), &[0, 2, 4], "aaaa0000", "1/3"),
            (Act::NextRow, &[0, 2, 4], "bbbb1111", "3 matches"),
            (Act::Type("bob"), &[1, 5], "bbbb1111", "1/2"),
            (Act::Type("Carol"), &[2], "cccc2222", "1/1"),
            (Act::Type("DDDD3"), &[3], "dddd3333", "1/1"),
            (Act::NextRow, &[3], "eeee4444", "1 match"),
            (Act::Type("zzz"), &[], "eeee4444", "No matches"),
            (Act::Key("enter"), &[], "eeee4444", "No matches"),
            (Act::Key("shift-enter"), &[], "eeee4444", "No matches"),
        ],
    );
    cx.simulate_keystrokes("secondary-a backspace");
    settle_typing(cx);
    assert_eq!(find_label(cx, &view), "Type to search");
    assert_eq!(find_matches(cx, &view), Vec::<usize>::new());
}

/// Find matches what a row shows. A stash tip missing from the stash list
/// shows its summary after the "WIP on main:" prefix, so the hidden prefix
/// neither counts it nor leaves it bright without a visible match.
#[allow(clippy::single_range_in_vec_init)]
#[gpui::test]
fn history_find_matches_stash_rows_on_what_they_show(cx: &mut gpui::TestAppContext) {
    use crate::view::panes::history::find::history_find_row_marks;

    let _visual_guard = crate::test_support::lock_visual_test();
    let mut commits = find_fixture_commits();
    commits.insert(
        1,
        Commit {
            parent_ids: [CommitId("bbbb1111".into()), CommitId("9999ffff".into())].into(),
            ..authored("abab7777", "WIP on main: bbbb111 Add feature", "Alice")
        },
    );
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(commits));
    open_find_with_shortcut(cx, &view);
    let stash_row = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            history_view(&view, app).update(app, |history, _cx| {
                let query = history.history_find_query().cloned();
                let cache = history.history_cache.as_ref().expect("the list is built");
                let commit = &cache.page.commits[cache.base.visible_indices.get(1).unwrap()];
                let row = &cache.base.row_vms[1];
                assert!(row.is_stash, "the row is drawn as a stash");
                assert_eq!(row.summary.as_ref(), "bbbb111 Add feature");
                history_find_row_marks(
                    query.as_ref(),
                    commit,
                    false,
                    row.summary.as_ref(),
                    row.author.as_ref(),
                    "abab7777",
                )
            })
        })
    };

    retype_query(cx, "main");
    assert_eq!(find_matches(cx, &view), Vec::<usize>::new());
    assert_eq!(stash_row(cx), (true, None), "the prefix is not shown");

    retype_query(cx, "add feature");
    wait_for_selection(cx, &view, &store, "abab7777");
    assert_eq!(find_matches(cx, &view), vec![1, 2]);
    let (dimmed, highlights) = stash_row(cx);
    assert!(!dimmed);
    assert_eq!(highlights.map(|found| found.summary), Some(vec![8..19]));
}

/// Before its index is built (a scope change, a first load) the list is one
/// page with more history behind it: the bar cannot say "No matches" or
/// give a final count yet.
#[gpui::test]
fn history_find_over_a_partial_page_is_not_final_while_the_index_builds(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut repo = find_fixture_repo(find_fixture_commits());
    let page = Arc::new(log_page(find_fixture_commits(), Some("ffff5555")));
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.history_state.log = Loadable::Ready(page);
    let snapshot = HistorySnapshot("history-find-building".into());
    repo.history_state.log_snapshot = Some(snapshot.clone());
    repo.history_state.indexed.requested = Some(snapshot);
    repo.history_state.indexed.epoch = repo.load_epoch;
    repo.history_state.indexed.loading = true;
    let (view, store, cx) = mount_find_fixture(cx, repo);

    open_find_with_shortcut(cx, &view);
    run_script(
        cx,
        &view,
        &store,
        &[
            (Act::Type("fix"), &[0, 2, 4], "aaaa0000", "1/3+"),
            (Act::Type("zzz"), &[], "aaaa0000", "Searching…"),
        ],
    );

    // A failed build leaves the page as the whole answer.
    let mut state = (*store.snapshot()).clone();
    state.repos[0].history_state.indexed.loading = false;
    state.repos[0].history_state.indexed.error = Some("index failed".into());
    let state = Arc::new(state);
    store.replace_snapshot_for_test(Arc::clone(&state));
    sync_view_with_store(cx, &view);
    draw_and_park(cx);
    assert_eq!(find_label(cx, &view), "No matches");
}

/// Each toggle rebuilds the query at once, selects its first match and keeps
/// the input focused. The SHA prefix does not apply to a regex.
#[gpui::test]
fn history_find_option_toggles_change_the_matches(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    run_script(
        cx,
        &view,
        &store,
        &[
            (Act::Type("fix"), &[0, 2, 4], "aaaa0000", "1/3"),
            (Act::Click(MATCH_CASE), &[2], "cccc2222", "1/1"),
            (Act::Click(MATCH_CASE), &[0, 2, 4], "aaaa0000", "1/3"),
            (Act::Type("ix"), &[0, 2, 4], "aaaa0000", "1/3"),
            (Act::Click(WHOLE_WORD), &[], "aaaa0000", "No matches"),
            (Act::Click(WHOLE_WORD), &[0, 2, 4], "aaaa0000", "1/3"),
            (Act::Type("^(add|docs)"), &[], "aaaa0000", "No matches"),
            (Act::Click(REGEX), &[1, 5], "bbbb1111", "1/2"),
            (Act::Type("dddd3"), &[], "bbbb1111", "No matches"),
            (Act::Click(REGEX), &[3], "dddd3333", "1/1"),
        ],
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
    assert!(cx.debug_bounds("history_find").is_none());
    assert!(
        history_panel_is_focused(cx, &view),
        "closing the find bar must return focus to the history list"
    );
    assert_eq!(find_matches(cx, &view), Vec::<usize>::new());

    // Cmd-F brings the bar back with the previous query selected.
    open_find_with_shortcut(cx, &view);
    assert!(cx.debug_bounds("history_find").is_some());
    assert!(find_input_is_focused(cx, &view));
    assert_eq!(find_text(cx, &view), "fix");
    assert_eq!(
        find_input(cx, &view).read_with(cx, |input, _| input.selected_range()),
        0..3,
        "reopening must select the whole previous query"
    );
    draw_and_park(cx);
    assert_eq!(
        find_matches(cx, &view),
        vec![0, 2, 4],
        "the restored query must be searched again"
    );

    // Escape with the history list focused also closes the bar.
    focus_history_panel(cx, &view);
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

/// Rows fade with every keystroke, but the scan and the first-match jump
/// wait for typing to settle, unless Enter asks for them at once.
#[gpui::test]
fn history_find_selects_the_first_match_once_typing_settles_or_on_enter(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));

    open_find_with_shortcut(cx, &view);
    cx.simulate_input("fix");
    draw_and_park(cx);
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        None,
        "nothing is selected while the query is still being typed",
    );
    assert_eq!(
        dimmed_rows(cx, &view),
        vec![1, 3, 5],
        "rows fade with every keystroke, before the query settles"
    );
    settle_typing(cx);
    wait_for_selection(cx, &view, &store, "aaaa0000");

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("bob");
    cx.simulate_keystrokes("enter");
    draw_and_park(cx);
    wait_for_selection(cx, &view, &store, "bbbb1111");
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

    // An invalid regex is an error, not a query that misses every row.
    click(cx, REGEX);
    retype_query(cx, "fix(");
    assert_eq!(find_label(cx, &view), "Invalid regex");
    assert!(
        find_status(cx, &view).is_error(),
        "the label reads as an error"
    );
    assert_eq!(dimmed_rows(cx, &view), Vec::<usize>::new());
    assert_eq!(find_matches(cx, &view), Vec::<usize>::new());
    assert!(
        cx.update(|_window, app| {
            history_view(&view, app).update(app, |history, cx| {
                !history.history_find_step(true, cx) && !history.history_find_step(false, cx)
            })
        }),
        "an invalid regex has nothing to step to"
    );
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some("bbbb1111"),
        "an invalid regex does not move the selection",
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

#[allow(clippy::single_range_in_vec_init)]
#[test]
fn row_highlights_show_why_the_row_matched() {
    use crate::view::panes::history::find::{HistoryFindHighlights, history_find_highlights};

    let query = |text: &str| HistoryFindQuery::new(text, TextSearchOptions::default());
    let commit = authored("abcd1234ffff", "Fix login fix", "Alice Fixer");
    let highlights = |query: Option<HistoryFindQuery>, summary: &str| {
        history_find_highlights(query.as_ref(), &commit, summary, "Alice Fixer", "abcd1234")
    };

    assert_eq!(
        highlights(query("fix"), "Fix login fix"),
        Some(HistoryFindHighlights {
            summary: vec![0..3, 10..13],
            author: vec![6..9],
            sha: 0,
        }),
        "every match in the summary and the author"
    );
    for prefix in ["abcd", "ABCD1234FF"] {
        assert_eq!(
            highlights(query(prefix), "Fix login fix"),
            Some(HistoryFindHighlights {
                sha: 8,
                ..HistoryFindHighlights::default()
            }),
            "a SHA prefix ({prefix}) lights up the whole SHA shown, never past its end"
        );
    }
    assert_eq!(
        highlights(query("login"), "stash message").map(|found| found.summary),
        None,
        "ranges index the text shown, which a stash row replaces"
    );
    assert_eq!(highlights(query("zzz"), "Fix login fix"), None);
    assert_eq!(highlights(None, "Fix login fix"), None);
}

#[test]
fn row_marks_fade_misses_and_highlight_only_matches() {
    use crate::view::panes::history::find::{history_find_highlights, history_find_row_marks};

    let query = HistoryFindQuery::new("fix", TextSearchOptions::default());
    let hit = authored("aaaa0000", "Fix login bug", "Alice");
    let miss = authored("bbbb1111", "Add feature", "Bob");
    let marks = |commit: &Commit, selected| {
        history_find_row_marks(
            query.as_ref(),
            commit,
            selected,
            &commit.summary,
            &commit.author,
            "aaaa0000",
        )
    };

    assert_eq!(
        marks(&hit, false),
        (
            false,
            history_find_highlights(query.as_ref(), &hit, "Fix login bug", "Alice", "aaaa0000")
        )
    );
    assert_eq!(marks(&miss, false), (true, None));
    assert_eq!(
        marks(&miss, true),
        (false, None),
        "the selection stays bright"
    );
    assert_eq!(
        history_find_row_marks(None, &hit, false, "Fix login bug", "Alice", "aaaa0000"),
        (false, None)
    );
}

/// Timing probe: a screen of rows, mostly misses, under a regex query. The
/// "before" arm is how rows were marked before `history_find_row_marks`.
#[test]
#[ignore]
fn history_find_row_marks_timing() {
    use crate::view::panes::history::find::{history_find_highlights, history_find_row_marks};

    let query = HistoryFindQuery::new(
        "fix(ed)? (crash|leak)",
        TextSearchOptions {
            regex: true,
            ..TextSearchOptions::default()
        },
    );
    let rows: Vec<Commit> = (0..60)
        .map(|row| {
            let summary = if row % 12 == 0 {
                format!("fixed crash in parser {row}")
            } else {
                format!("Refactor the widget layout for row {row} and tidy up")
            };
            authored(&format!("{row:08x}"), &summary, "Alice Example")
        })
        .collect();
    let frames = 2_000;
    let started = std::time::Instant::now();
    let mut marked = 0usize;
    for _ in 0..frames {
        for commit in &rows {
            let dimmed = query.as_ref().is_some_and(|query| !query.matches(commit));
            let highlights = history_find_highlights(
                query.as_ref(),
                commit,
                &commit.summary,
                &commit.author,
                &commit.id.as_ref()[..8],
            );
            marked += usize::from(dimmed) + usize::from(highlights.is_some());
        }
    }
    let before = started.elapsed();
    let started = std::time::Instant::now();
    let mut marked_after = 0usize;
    for _ in 0..frames {
        for commit in &rows {
            let (dimmed, highlights) = history_find_row_marks(
                query.as_ref(),
                commit,
                false,
                &commit.summary,
                &commit.author,
                &commit.id.as_ref()[..8],
            );
            marked_after += usize::from(dimmed) + usize::from(highlights.is_some());
        }
    }
    let after = started.elapsed();
    assert_eq!(marked, marked_after);
    eprintln!(
        "row marks per frame of 60 rows: before {:?}, after {:?}",
        before / frames,
        after / frames
    );
}

#[allow(clippy::single_range_in_vec_init)]
#[test]
fn detail_highlights_follow_the_fields_a_row_matches_on() {
    use crate::view::panes::history::find::{
        CommitDetailsFindHighlights, history_find_detail_highlights,
    };

    let query = |text: &str| HistoryFindQuery::new(text, TextSearchOptions::default());
    let id = "abcd1234ffff0000111122223333444455556666";
    let message = "Fix login fix\n\nThe fix is in the body.";
    let highlights = |query: Option<HistoryFindQuery>| {
        history_find_detail_highlights(query.as_ref(), id, message, "Alice Fixer", "Fix login fix")
    };

    assert_eq!(
        highlights(query("fix")),
        Some(CommitDetailsFindHighlights {
            summary: vec![0..3, 10..13],
            author: vec![6..9],
            ..CommitDetailsFindHighlights::default()
        }),
        "the summary line and the author, never the body"
    );
    assert_eq!(highlights(query("body")), None, "the body is not searched");
    assert_eq!(
        highlights(query("ABCD")),
        Some(CommitDetailsFindHighlights {
            sha: true,
            short_sha_len: Some(8),
            ..CommitDetailsFindHighlights::default()
        }),
        "an abbreviation lights up the whole SHA and shows the list's short form"
    );
    assert_eq!(
        highlights(query("abcd1234ffff")).and_then(|found| found.short_sha_len),
        Some(12),
        "a longer abbreviation is shown as typed"
    );
    assert_eq!(
        highlights(query(id)),
        Some(CommitDetailsFindHighlights {
            sha: true,
            ..CommitDetailsFindHighlights::default()
        }),
        "the full id needs no short form"
    );
    let regex = TextSearchOptions {
        regex: true,
        ..TextSearchOptions::default()
    };
    assert_eq!(
        highlights(HistoryFindQuery::new("abcd", regex)),
        None,
        "a regex never matches the SHA"
    );
    assert_eq!(highlights(None), None);
}

/// Mounts the find fixture with its top row selected and that commit's
/// details loaded. The selection is the first match of every query the
/// details tests type, so the bar's jump to it keeps the loaded details.
fn mount_details_find_fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
    String,
) {
    let id = format!("aaaa0000{}", "1".repeat(32));
    let mut commits = find_fixture_commits();
    commits[0] = authored(&id, "Fix login bug", "Alice Fixer");
    let mut repo = find_fixture_repo(commits);
    repo.history_state.selected_commit = Some(CommitId(id.clone().into()));
    repo.history_state.selected_commit_rev = 1;
    repo.history_state.commit_details =
        Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
            id: CommitId(id.clone().into()),
            message: "Fix login bug\n\nThe fix needs a test.".to_string(),
            author_name: "Alice Fixer".to_string(),
            author_email: "alice@fix.example".to_string(),
            authored_at_unix: 0,
            committed_at: String::new(),
            committed_at_unix: 0,
            parent_ids: Vec::new(),
            files: Vec::new(),
        }));
    repo.history_state.commit_details_rev = 1;
    let (view, _store, cx) = mount_find_fixture(cx, repo);
    (view, cx, id)
}

/// Ranges of the details message (or SHA) field washed as find matches.
fn details_find_washes(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    sha: bool,
) -> Vec<std::ops::Range<usize>> {
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let input = if sha {
            pane.commit_details_sha_input.clone()
        } else {
            pane.commit_details_message_input.clone()
        };
        input.update(app, |input, _| {
            let len = input.text().len();
            input
                .debug_effective_highlights_for_range(0..len)
                .into_iter()
                .filter(|(_, style)| style.background_color.is_some())
                .map(|(range, _)| range)
                .collect()
        })
    })
}

/// The details pane follows the find bar: the summary match, the whole SHA
/// for an abbreviation (shown beside it) or the full id, and nothing once the
/// bar closes.
#[gpui::test]
fn history_find_highlights_the_selected_commit_details(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx, id) = mount_details_find_fixture(cx);
    let short_shown = |cx: &mut gpui::VisualTestContext| {
        cx.debug_bounds("commit_details_sha_find_short").is_some()
    };
    assert!(details_find_washes(cx, &view, false).is_empty());
    assert!(details_find_washes(cx, &view, true).is_empty());

    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    assert_eq!(
        details_find_washes(cx, &view, false),
        vec![0..3],
        "the summary match, not the body's"
    );
    assert!(details_find_washes(cx, &view, true).is_empty());
    assert!(!short_shown(cx));

    retype_query(cx, "AAAA0000");
    assert_eq!(
        details_find_washes(cx, &view, true),
        vec![0..40],
        "an abbreviation washes the whole id"
    );
    assert!(details_find_washes(cx, &view, false).is_empty());
    assert!(short_shown(cx), "the abbreviation shows beside the full id");

    retype_query(cx, &id);
    assert_eq!(details_find_washes(cx, &view, true), vec![0..40]);
    assert!(!short_shown(cx), "the full id needs no short form");

    cx.simulate_keystrokes("escape");
    draw_and_park(cx);
    assert!(!find_is_open(cx, &view));
    assert!(
        details_find_washes(cx, &view, true).is_empty(),
        "closing the bar clears the wash"
    );
    assert!(!short_shown(cx));
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
    indexed_find_history_with_top("history-find-indexed", None)
}

/// The same history after a fetch put a commit summarised `summary` on top,
/// so every earlier row moves down by one.
fn indexed_find_history_after_fetch(summary: &str) -> (HistoryIndexHandle, Vec<Commit>) {
    indexed_find_history_with_top("history-find-fetched", Some(summary))
}

fn indexed_find_history_with_top(
    snapshot: &str,
    top: Option<&str>,
) -> (HistoryIndexHandle, Vec<Commit>) {
    let mut builder =
        HistoryIndexBuilder::new(HistorySnapshot(snapshot.into()), LogScope::AllBranches, 20)
            .unwrap();
    let mut commits = Vec::with_capacity(INDEXED_ROWS + 1);
    if let Some(summary) = top {
        let id = [0xff; 20];
        let parent = indexed_find_raw_id(0);
        builder.push(&id, [parent.as_slice()], false).unwrap();
        commits.push(Commit {
            id: CommitId(gitcomet_core::hex::encode(&id).into()),
            parent_ids: std::iter::once(CommitId(indexed_find_id(0).into())).collect(),
            summary: summary.into(),
            author: "Erin".into(),
            time: SystemTime::UNIX_EPOCH,
        });
    }
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
    /// Fail every scan's first read.
    Fail,
    /// Fail only the first scan, then serve a retry.
    FailOnce,
    PanicOnce,
    /// Block until cancelled; the test reports the scan's chunks itself.
    Hold,
}

/// Serves the fixture's commits to the viewport's range loads, and to the
/// find scan as `ScanMode` says.
struct IndexedFindRepo {
    spec: RepoSpec,
    /// Replaced when a test moves the fixture to a newer history.
    commits: std::sync::RwLock<Vec<Commit>>,
    mode: ScanMode,
    scans_started: AtomicUsize,
    released: AtomicBool,
}

/// The find scan runs on its own named worker; the viewport's range loads run
/// on the store's repo-load pool, whose threads are unnamed.
fn on_find_scan_thread() -> bool {
    std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with(gitcomet_state::history_find::HISTORY_FIND_THREAD))
}

/// Repository methods the find tests never reach.
macro_rules! not_needed {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $ret:ty;)*) => {
        $(fn $name(&self $(, $arg: $ty)*) -> Result<$ret> {
            $(let _ = $arg;)*
            Err(Error::new(ErrorKind::Unsupported(
                "not needed by the history find tests",
            )))
        })*
    };
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
                ScanMode::PanicOnce if self.scans_started.load(Ordering::SeqCst) > 1 => {}
                ScanMode::PanicOnce => panic!("deliberate history find panic"),
                ScanMode::FailOnce if self.scans_started.load(Ordering::SeqCst) > 1 => {}
                ScanMode::Fail | ScanMode::FailOnce => {
                    return Err(Error::new(ErrorKind::Backend("history read failed".into())));
                }
                ScanMode::Hold => {
                    // A held scan must end with its search or its test.
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
            .read()
            .unwrap()
            .get(range.clone())
            .ok_or_else(|| Error::new(ErrorKind::Backend("range out of bounds".into())))?
            .to_vec();
        Ok(HistoryRange {
            snapshot: index.snapshot.clone(),
            start: range.start,
            commits,
        })
    }

    not_needed! {
        fn log_head_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Arc<LogPage>;
        fn commit_details(&self, id: &CommitId) -> gitcomet_core::domain::CommitDetails;
        fn reflog_head(&self, limit: usize) -> Vec<gitcomet_core::domain::ReflogEntry>;
        fn current_branch(&self) -> String;
        fn list_branches(&self) -> Vec<Branch>;
        fn list_remotes(&self) -> Vec<gitcomet_core::domain::Remote>;
        fn list_remote_branches(&self) -> Vec<RemoteBranch>;
        fn status(&self) -> gitcomet_core::domain::RepoStatus;
        fn diff_unified(&self, target: &gitcomet_core::domain::DiffTarget) -> String;
        fn create_branch(&self, name: &str, target: &CommitId) -> ();
        fn delete_branch(&self, name: &str) -> ();
        fn checkout_branch(&self, name: &str) -> ();
        fn checkout_commit(&self, id: &CommitId) -> ();
        fn cherry_pick(&self, id: &CommitId) -> ();
        fn stash_create(&self, message: &str, include_untracked: bool) -> ();
        fn stash_list(&self) -> Vec<gitcomet_core::domain::StashEntry>;
        fn stash_apply(&self, index: usize) -> ();
        fn stash_drop(&self, index: usize) -> ();
        fn stage(&self, paths: &[&Path]) -> ();
        fn unstage(&self, paths: &[&Path]) -> ();
        fn commit(&self, message: &str) -> ();
        fn fetch_all(&self) -> ();
        fn pull(&self, mode: gitcomet_core::services::PullMode) -> ();
        fn push(&self) -> ();
        fn discard_worktree_changes(&self, paths: &[&Path]) -> ();
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
        commits: commits.into(),
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
    focus_history_panel(cx, &view);
    (view, store, cx, backend)
}

fn store_find(store: &AppStore) -> HistoryFindState {
    store.snapshot().repos[0].history_state.find.clone()
}

/// `text` with the default options, as the store would hold it.
fn plain_query(text: &str) -> Option<HistoryFindQuery> {
    HistoryFindQuery::new(text, TextSearchOptions::default())
}

/// Waits for the view's request for `query` to reach the store and returns
/// the search's generation.
fn wait_for_find_request(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    query: Option<HistoryFindQuery>,
) -> u64 {
    wait_until(cx, "the search request", |cx| {
        sync_view_with_store(cx, view);
        store_find(store).query == query
    });
    store_find(store).generation()
}

/// Opens the bar, types `text` and waits for the store to be asked for it.
fn open_and_find(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    text: &str,
) -> u64 {
    open_find_with_shortcut(cx, view);
    type_query(cx, text);
    wait_for_find_request(cx, view, store, plain_query(text))
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

/// Every raw row matching "fix", as a finished scan reports them.
const ALL_FIX_ROWS: [usize; 4] = [0, 5, STASH_HELPER_ROW, FAR_FIX_ROW];

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

fn failed(matches: &[usize]) -> Option<IndexedFindStatus> {
    Some(IndexedFindStatus {
        matches: matches.to_vec(),
        complete: true,
        pending: false,
        failed: true,
    })
}

/// The visible rows of every "fix" match.
fn all_fix_visible() -> Vec<usize> {
    vec![0, 5, indexed_visible(FAR_FIX_ROW)]
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
            window
                .cache
                .page
                .commits
                .iter()
                .zip(window.cache.base.row_vms.iter())
                .enumerate()
                .filter(|(ix, _)| window.loaded.get(*ix).copied().unwrap_or(false))
                .map(|(ix, row)| (window.start + ix, row))
                .filter(|(visible_ix, (commit, row))| {
                    rows.contains(visible_ix)
                        && crate::view::panes::history::find::history_find_row_marks(
                            query.as_ref(),
                            commit,
                            selected.as_ref() == Some(&commit.id),
                            row.summary.as_ref(),
                            row.author.as_ref(),
                            "",
                        )
                        .0
                })
                .map(|(visible_ix, _)| visible_ix)
                .collect()
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

/// Presses `keys` in the find input, waits for raw row `row` to be selected
/// and checks the match label.
fn step_to(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    store: &AppStore,
    keys: &str,
    row: usize,
    label: &str,
) {
    cx.simulate_keystrokes(keys);
    draw_and_park(cx);
    wait_for_selection(cx, view, store, &indexed_find_id(row));
    assert_eq!(find_label(cx, view), label, "after {keys} to row {row}");
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
        ALL_FIX_ROWS,
        "the scan reads every indexed row, the hidden stash helper included"
    );
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&all_fix_visible(), true),
        "the hidden stash helper is not a match in the list"
    );
    assert_eq!(find_label(cx, &view), "1/3");
    assert_eq!(backend.scans_started(), 1);

    // An answered search is not asked for again on later frames.
    let generation = results.generation();
    for _ in 0..5 {
        sync_view_with_store(cx, &view);
    }
    assert_eq!(store_find(&store).generation(), generation);
    assert_eq!(backend.scans_started(), 1, "the finished scan was repeated");

    step_to(cx, &view, &store, "enter", 5, "2/3");
    step_to(cx, &view, &store, "enter", FAR_FIX_ROW, "3/3");
    assert_indexed_row_centred(
        cx,
        &view,
        indexed_visible(FAR_FIX_ROW),
        "a match below the viewport is scrolled to the middle",
    );
    step_to(cx, &view, &store, "enter", 0, "1/3");
    assert!(indexed_row_in_view(cx, &view, 0), "enter wraps to the top");
    assert!(find_input_is_focused(cx, &view));
}

/// The first chunk selects the first match; a later chunk extends the
/// matches without moving the selection, and stepping reaches it.
#[gpui::test]
fn indexed_history_find_selects_from_the_first_chunk_and_steps_across_chunks(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_and_find(cx, &view, &store, "fix");
    wait_until(cx, "the scan to start", |_| backend.scans_started() == 1);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[], false),
        "nothing is known until the scan reports"
    );
    assert_eq!(find_label(cx, &view), "Searching…");
    assert_eq!(store_selected(&store), None);

    report_matches(cx, &view, &store, &[0, 5, STASH_HELPER_ROW], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[0, 5], false),
        "the stash helper the scan reported is hidden, so it is not counted"
    );
    assert_eq!(find_label(cx, &view), "1/2+", "the scan is still running");

    // A match already on screen is selected without scrolling the list.
    assert!(indexed_row_in_view(cx, &view, 5));
    let position = indexed_viewport(cx, &view).0;
    step_to(cx, &view, &store, "enter", 5, "2/2+");
    assert_eq!(indexed_viewport(cx, &view).0, position);
    // Previous matches in this chunk are available while the scan runs.
    step_to(cx, &view, &store, "shift-enter", 0, "1/2+");

    report_matches(cx, &view, &store, &[FAR_FIX_ROW], true);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&all_fix_visible(), true)
    );
    assert_eq!(find_label(cx, &view), "1/3", "the scan is done");
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some(&indexed_find_id(0)),
        "later chunks do not move the selection",
    );

    step_to(cx, &view, &store, "f3", 5, "2/3");
    step_to(cx, &view, &store, "f3", FAR_FIX_ROW, "3/3");
    assert_indexed_row_centred(
        cx,
        &view,
        indexed_visible(FAR_FIX_ROW),
        "an off-screen match from the second chunk is centred",
    );
    step_to(cx, &view, &store, "f3", 0, "1/3");
    assert!(indexed_row_in_view(cx, &view, 0), "f3 wraps to the top");
    step_to(cx, &view, &store, "shift-enter", FAR_FIX_ROW, "3/3");
    assert!(
        indexed_row_in_view(cx, &view, indexed_visible(FAR_FIX_ROW)),
        "shift-enter wraps to the end"
    );
    step_to(cx, &view, &store, "f2", 5, "2/3");
    step_to(cx, &view, &store, "shift-enter", 0, "1/3");
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

    open_and_find(cx, &view, &store, "helper");
    report_matches(cx, &view, &store, &[STASH_HELPER_ROW], true);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[], true),
        "a match on a hidden row is not shown"
    );
    assert_eq!(find_label(cx, &view), "No matches");
    assert_selection_settles_on(cx, &view, &store, None, "nothing to select");

    // Rows below the hidden helper are counted at their shown positions.
    retype_query(cx, "fix");
    wait_for_find_request(cx, &view, &store, plain_query("fix"));
    report_matches(cx, &view, &store, &[STASH_HELPER_ROW, FAR_FIX_ROW], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(FAR_FIX_ROW));
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&[FAR_FIX_ROW - 1], true)
    );
    assert_eq!(find_label(cx, &view), "1/1");
}

/// The stash row is not in the fixture's (empty) stash list, so it shows its
/// summary after the "WIP on main:" prefix. The scan matches that text, as
/// the row's fading does, rather than the hidden prefix.
#[gpui::test]
fn indexed_history_find_matches_stash_rows_on_what_they_show(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Serve);

    open_find_with_shortcut(cx, &view);
    for (query, rows) in [
        ("main", vec![STASH_HELPER_ROW]),
        ("1234567 wip", vec![STASH_ROW]),
    ] {
        retype_query(cx, query);
        wait_until(cx, "the scan to finish", |cx| {
            sync_view_with_store(cx, &view);
            let results = store_find(&store);
            results.query == plain_query(query) && results.done
        });
        assert_eq!(
            store_find(&store).match_rows().collect::<Vec<_>>(),
            rows,
            "{query}"
        );
    }
    assert_eq!(
        indexed_dimmed_rows(cx, &view, STASH_ROW..STASH_ROW + 1),
        Vec::<usize>::new(),
        "the row showing the match stays bright"
    );
}

/// A query edit fades rows immediately and clears the previous query's count.
#[gpui::test]
fn indexed_history_find_edit_dims_at_once_and_resets_the_label_until_answered(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &ALL_FIX_ROWS, true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(find_label(cx, &view), "1/3");
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
    assert_eq!(store_find(&store).query, plain_query("fix"));
    assert_eq!(
        indexed_find_status(cx, &view).map(|status| status.pending),
        Some(true)
    );
    assert_eq!(
        find_label(cx, &view),
        "Searching…",
        "a new query must not show the old query's count"
    );
    assert_eq!(
        indexed_dimmed_rows(cx, &view, 0..12),
        vec![1, 3, 4, 5, 6, 8, 9, 10, 11],
        "rows fade by the new query at once; the selected row stays bright"
    );

    // The store has taken the new query, but its scan has not reported yet.
    settle_typing(cx);
    wait_for_find_request(cx, &view, &store, plain_query("bob"));
    draw_and_park(cx);
    assert_eq!(
        find_label(cx, &view),
        "Searching…",
        "the new scan has not answered"
    );

    report_matches(cx, &view, &store, &[2], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(2));
    assert_eq!(find_label(cx, &view), "1/1+");
    report_matches(cx, &view, &store, &[7], true);
    assert_eq!(indexed_find_status(cx, &view), found(&[2, 7], true));
    assert_eq!(find_label(cx, &view), "1/2");
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

    let generation = open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    step_to(cx, &view, &store, "enter", 5, "2/2+");
    wait_until(cx, "the first scan", |_| backend.scans_started() == 1);

    store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
        repo_id: FIND_REPO_ID,
        query: None,
        index: None,
    }));
    wait_until(cx, "the search to be asked for again", |cx| {
        sync_view_with_store(cx, &view);
        let results = store_find(&store);
        results.query == plain_query("fix") && results.generation() == generation + 2
    });
    wait_until(cx, "the scan to restart", |_| backend.scans_started() == 2);
    assert!(store_find(&store).matches.is_empty());

    report_matches(cx, &view, &store, &ALL_FIX_ROWS, true);
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&all_fix_visible(), true)
    );
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some(&indexed_find_id(5)),
        "the restarted search does not jump back to the first match",
    );
    assert_eq!(find_label(cx, &view), "2/3");
}

/// Toggling an option is a new query: the running scan is cancelled and a
/// new one starts with the new options, and its first match is selected.
#[gpui::test]
fn indexed_history_find_option_toggles_restart_the_scan(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    let generation = open_and_find(cx, &view, &store, "fix");
    wait_until(cx, "the first scan", |_| backend.scans_started() == 1);
    report_matches(cx, &view, &store, &[0, 5], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));

    let match_case = TextSearchOptions {
        match_case: true,
        ..TextSearchOptions::default()
    };
    click(cx, MATCH_CASE);
    assert!(find_input_is_focused(cx, &view), "the toggle keeps focus");
    let restarted =
        wait_for_find_request(cx, &view, &store, HistoryFindQuery::new("fix", match_case));
    assert!(restarted > generation);
    // The scan worker is a single thread, and a held scan ends only once
    // cancelled, so a second scan starting means the first was cancelled.
    wait_until(cx, "the scan to restart", |_| backend.scans_started() == 2);
    assert_eq!(
        find_label(cx, &view),
        "Searching…",
        "the old option's count must not survive a new search"
    );

    report_matches(cx, &view, &store, &[5], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(5));
    assert_eq!(find_label(cx, &view), "1/1");

    // Whole word and regex restart it too.
    for (toggle, options) in [
        (
            WHOLE_WORD,
            TextSearchOptions {
                whole_word: true,
                ..match_case
            },
        ),
        (
            REGEX,
            TextSearchOptions {
                whole_word: true,
                regex: true,
                ..match_case
            },
        ),
    ] {
        let scans = backend.scans_started();
        click(cx, toggle);
        wait_for_find_request(cx, &view, &store, HistoryFindQuery::new("fix", options));
        wait_until(cx, "the scan to restart", |_| {
            backend.scans_started() == scans + 1
        });
    }
}

/// An invalid regex is reported as such and never searched for: no request
/// reaches the store, no scan starts, nothing fades and nothing is selected.
#[gpui::test]
fn indexed_history_find_does_not_search_an_invalid_regex(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_find_with_shortcut(cx, &view);
    let before = store_find(&store).rev;
    click(cx, REGEX);
    type_query(cx, "fix(");
    for _ in 0..5 {
        sync_view_with_store(cx, &view);
    }
    assert_eq!(find_label(cx, &view), "Invalid regex");
    assert!(find_status(cx, &view).is_error());
    assert_eq!(indexed_find_status(cx, &view), None);
    assert_eq!(store_find(&store).query, None, "no search was asked for");
    assert_eq!(store_find(&store).rev, before, "no Find reached the store");
    assert_eq!(backend.scans_started(), 0);
    assert_eq!(indexed_dimmed_rows(cx, &view, 0..12), Vec::<usize>::new());
    cx.simulate_keystrokes("enter");
    draw_and_park(cx);
    assert_selection_settles_on(cx, &view, &store, None, "no jump, no step");

    // Completing the pattern searches it.
    cx.simulate_input(")");
    settle_typing(cx);
    let regex = TextSearchOptions {
        regex: true,
        ..TextSearchOptions::default()
    };
    wait_for_find_request(cx, &view, &store, HistoryFindQuery::new("fix()", regex));
    wait_until(cx, "the scan to start", |_| backend.scans_started() == 1);
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
    assert_eq!(indexed_find_status(cx, &view), failed(&[]));
    assert_eq!(find_label(cx, &view), "Search failed");
    assert!(find_status(cx, &view).is_error());
    assert_selection_settles_on(cx, &view, &store, None, "nothing to select");
}

#[gpui::test]
fn indexed_history_find_keeps_partial_matches_after_a_failure(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);

    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], false);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));

    report_scan(
        cx,
        &view,
        &store,
        Err(Error::new(ErrorKind::Backend("history read failed".into()))),
    );
    assert_eq!(indexed_find_status(cx, &view), failed(&[0, 5]));
    assert_eq!(find_label(cx, &view), "Search failed");
    // The matches found before the failure can still be stepped through.
    step_to(cx, &view, &store, "enter", 5, "Search failed");
}

#[gpui::test]
fn history_find_input_tracks_theme_changes_while_open_and_closed(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, _store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    open_find_with_shortcut(cx, &view);
    let input = find_input(cx, &view);
    for theme in [AppTheme::gitcomet_light(), AppTheme::gitcomet_dark()] {
        cx.update(|_window, app| {
            history_view(&view, app).update(app, |history, cx| history.set_theme(theme, cx));
            let actual = input.read(app).theme_for_test();
            assert_eq!(
                actual.colors.editor.foreground,
                theme.colors.editor.foreground
            );
            assert_eq!(actual.colors.editor.cursor, theme.colors.editor.cursor);
        });
        cx.simulate_keystrokes("escape");
    }
}

#[gpui::test]
fn indexed_history_find_manual_navigation_cancels_the_first_match_jump(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    select_next_row(cx, &view);
    wait_for_selection(cx, &view, &store, &indexed_find_id(1));
    report_matches(cx, &view, &store, &[0, 5], true);
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some(&indexed_find_id(1)),
        "the user's row wins",
    );
}

#[gpui::test]
fn history_find_two_steps_before_a_store_snapshot_reach_two_matches(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    wait_for_selection(cx, &view, &store, "aaaa0000");
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            assert!(history.history_find_step(true, cx));
            assert!(history.history_find_step(true, cx));
        });
    });
    wait_for_selection(cx, &view, &store, "eeee4444");
}

#[gpui::test]
fn indexed_history_find_step_waits_for_the_next_chunk_before_wrapping(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], false);
    step_to(cx, &view, &store, "enter", 5, "2/2+");
    cx.simulate_keystrokes("enter");
    draw_and_park(cx);
    report_matches(cx, &view, &store, &[FAR_FIX_ROW], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(FAR_FIX_ROW));
}

#[gpui::test]
fn history_find_paged_matches_are_reused_until_the_projection_changes(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, _store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    open_find_with_shortcut(cx, &view);
    type_query(cx, "fix");
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, _cx| {
            // Row text is built per visible row, so it follows the indices.
            let cache = history.history_cache.as_mut().unwrap();
            let rows = cache.base.row_vms.clone();
            cache.base.visible_indices =
                HistoryVisibleIndices::Filtered(Arc::from([0, 1, 2, 3, 4]));
            cache.base.row_vms = rows[0..5].to_vec();
            cache.base.request.stashes_rev += 1;
            let first = history.history_find_matches().unwrap();
            assert_eq!(first.visible, vec![0, 2, 4]);
            for _ in 0..20 {
                let again = history.history_find_matches().unwrap();
                assert!(
                    std::rc::Rc::ptr_eq(&first, &again),
                    "unchanged frames must reuse matches"
                );
            }
            // The same page now hides a different stash helper, with the same visible count.
            let cache = history.history_cache.as_mut().unwrap();
            cache.base.visible_indices =
                HistoryVisibleIndices::Filtered(Arc::from([1, 2, 3, 4, 5]));
            cache.base.row_vms = rows[1..6].to_vec();
            cache.base.request.stashes_rev += 1;
            assert_eq!(history.history_find_matches().unwrap().visible, vec![1, 3]);
        });
    });
}

#[gpui::test]
fn history_find_quick_down_up_keeps_the_last_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    store.dispatch(Msg::SelectCommit {
        request_id: None,
        repo_id: FIND_REPO_ID,
        commit_id: CommitId("aaaa0000".into()),
    });
    wait_for_selection(cx, &view, &store, "aaaa0000");
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            assert!(history.history_select_adjacent_commit(1, cx));
            assert!(history.history_select_adjacent_commit(-1, cx));
        });
    });
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some("aaaa0000"),
        "Down then Up returns to the original row",
    );
}

/// A fetch or a commit replaces the index while the bar is open. The same
/// query is searched again over the new index; meanwhile the bar keeps its
/// answer, moved onto the new rows, instead of blanking to "Searching…".
#[gpui::test]
fn indexed_history_find_keeps_its_matches_while_a_refreshed_index_is_searched(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &ALL_FIX_ROWS, true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(find_label(cx, &view), "1/3");

    let (fetched, commits) = indexed_find_history_after_fetch("Add a feature on top");
    *backend.0.commits.write().unwrap() = commits;
    let mut state = (*store.snapshot()).clone();
    install_index(&mut state, fetched.clone());
    let state = Arc::new(state);
    store.replace_snapshot_for_test(Arc::clone(&state));
    set_history_view_state_for_tests(cx, &view, state);
    wait_until(cx, "the fetched history to show", |cx| {
        sync_view_with_store(cx, &view);
        cx.update(|_window, app| {
            history_view(&view, app)
                .read(app)
                .indexed
                .presentation
                .as_ref()
                .is_some_and(|shown| Arc::ptr_eq(&shown.graph.projection.index, &fetched))
        }) && store_find(&store)
            .index
            .is_some_and(|index| index.ptr_eq(&Arc::downgrade(&fetched)))
    });

    // Every old row moved down by one under the new commit.
    let moved: Vec<usize> = all_fix_visible().iter().map(|row| row + 1).collect();
    assert_eq!(
        indexed_find_status(cx, &view),
        found(&moved, false),
        "the old answer, on the new rows, until the new search reports"
    );
    assert_eq!(find_label(cx, &view), "1/3+");
    step_to(cx, &view, &store, "enter", 5, "2/3+");

    let fetched_rows: Vec<usize> = ALL_FIX_ROWS.iter().map(|row| row + 1).collect();
    report_matches(cx, &view, &store, &fetched_rows, true);
    assert_eq!(indexed_find_status(cx, &view), found(&moved, true));
    assert_eq!(find_label(cx, &view), "2/3");
}

/// Each window has its own store. A whole-history scan in one window must not
/// hold up find in another.
#[test]
fn history_find_in_one_store_does_not_wait_for_another_stores_scan() {
    let (index, commits) = indexed_find_history();
    let start = |mode| {
        let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
        let mut state = AppState {
            repos: vec![find_fixture_repo(commits.clone())],
            active_repo: Some(FIND_REPO_ID),
            ..AppState::test_default()
        };
        install_index(&mut state, index.clone());
        store.replace_snapshot_for_test(Arc::new(state));
        let repo = Arc::new(IndexedFindRepo {
            spec: store.snapshot().repos[0].spec.clone(),
            commits: commits.clone().into(),
            mode,
            scans_started: AtomicUsize::new(0),
            released: AtomicBool::new(false),
        });
        store.insert_repo_for_test(FIND_REPO_ID, repo.clone());
        store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
            repo_id: FIND_REPO_ID,
            query: plain_query("fix"),
            index: Some(index.clone()),
        }));
        (store, events, IndexedFindBackend(repo))
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let (held, _held_events, held_backend) = start(ScanMode::Hold);
    while held_backend.scans_started() == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the held scan never started"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let (other, _other_events, _other_backend) = start(ScanMode::Serve);
    while !store_find(&other).done {
        assert!(
            std::time::Instant::now() < deadline,
            "the other store's search waited for the held scan"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        store_find(&other).match_rows().collect::<Vec<_>>(),
        ALL_FIX_ROWS
    );
    drop(held);
}

/// The store drops a selection naming a presentation it no longer shows. The
/// next Down must start from the row still selected, not from the dropped one.
#[gpui::test]
fn indexed_history_rejected_selections_do_not_move_where_down_starts(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            history.select_indexed_commit_row(FIND_REPO_ID, 0, false, cx);
        });
    });
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));

    let shown = store.snapshot().repos[0]
        .history_state
        .indexed
        .displayed_index
        .clone();
    let set_displayed = |index: Option<HistoryIndexHandle>| {
        let mut state = (*store.snapshot()).clone();
        state.repos[0].history_state.indexed.displayed_index = index;
        store.replace_snapshot_for_test(Arc::new(state));
    };
    set_displayed(None);
    for _ in 0..2 {
        cx.update(|_window, app| {
            history_view(&view, app).update(app, |history, cx| {
                assert!(history.history_select_adjacent_commit(1, cx));
            });
        });
        sync_view_with_store(cx, &view);
    }
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some(&indexed_find_id(0)),
        "the store dropped both selections",
    );

    set_displayed(shown);
    sync_view_with_store(cx, &view);
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            assert!(history.history_select_adjacent_commit(1, cx));
        });
    });
    wait_until(cx, "the next selection", |cx| {
        sync_view_with_store(cx, &view);
        store_selected(&store).as_deref() != Some(&indexed_find_id(0))
    });
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some(&indexed_find_id(1)),
        "Down starts from the row still selected",
    );
}

#[gpui::test]
fn indexed_history_find_quick_down_up_keeps_the_last_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            history.select_indexed_commit_row(FIND_REPO_ID, 0, false, cx);
        });
    });
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            assert!(history.history_select_adjacent_commit(1, cx));
            assert!(history.history_select_adjacent_commit(-1, cx));
        });
    });
    assert_selection_settles_on(
        cx,
        &view,
        &store,
        Some(&indexed_find_id(0)),
        "Down then Up returns to the original row",
    );
}

#[gpui::test]
fn indexed_history_find_does_not_resend_while_the_snapshot_is_pending(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_find_with_shortcut(cx, &view);
    let (unrelated, _unrelated_events) = AppStore::new_test(Arc::new(BlockingBackend));
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            history.set_history_find_query("fix", false);
            let before = store.history_find_dispatch_count_for_test();
            for _ in 0..20 {
                unrelated.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
                    repo_id: FIND_REPO_ID,
                    query: None,
                    index: None,
                }));
                history.sync_history_find(cx);
            }
            // Count sends synchronously, scoped to this store. No worker timing
            // or traffic from another test can influence this assertion.
            let dispatched = store.history_find_dispatch_count_for_test() - before;
            assert_eq!(
                dispatched, 1,
                "renders with the same unanswered snapshot must share a request"
            );
        });
    });
}

#[gpui::test]
fn indexed_history_find_reuses_commit_text_between_queries(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Serve);
    open_and_find(cx, &view, &store, "fix");
    wait_until(cx, "first scan", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).done
    });
    let reads = backend.scans_started();
    retype_query(cx, "fix typo");
    wait_until(cx, "narrower scan", |cx| {
        sync_view_with_store(cx, &view);
        let find = store_find(&store);
        find.query == plain_query("fix typo") && find.done
    });
    assert_eq!(store_find(&store).match_rows().collect::<Vec<_>>(), vec![5]);
    assert_eq!(
        backend.scans_started(),
        reads,
        "a second query must reuse decoded text"
    );
    retype_query(cx, "bob");
    wait_until(cx, "unrelated query", |cx| {
        sync_view_with_store(cx, &view);
        let find = store_find(&store);
        find.query == plain_query("bob") && find.done
    });
    assert_eq!(
        store_find(&store).match_rows().collect::<Vec<_>>(),
        vec![2, 7]
    );
    assert_eq!(
        backend.scans_started(),
        reads,
        "an unrelated query also reuses decoded text"
    );
}

#[gpui::test]
fn history_find_reselecting_a_commit_leaves_range_comparison(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut repo = find_fixture_repo(find_fixture_commits());
    repo.history_state.selected_commit = Some(CommitId("aaaa0000".into()));
    repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection {
        from: CommitId("cccc2222".into()),
        to: Some(CommitId("aaaa0000".into())),
        from_label: "base".into(),
        to_label: "tip".into(),
        options: Default::default(),
        base: None,
    });
    let (view, store, cx) = mount_find_fixture(cx, repo);
    open_find_with_shortcut(cx, &view);
    type_query(cx, "Fix login");
    wait_until(
        cx,
        "single commit details instead of the range comparison",
        |cx| {
            sync_view_with_store(cx, &view);
            store.snapshot().repos[0]
                .history_state
                .range_selection
                .is_none()
        },
    );
}

#[gpui::test]
fn indexed_history_find_jump_during_render_requests_another_frame(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[FAR_FIX_ROW], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(FAR_FIX_ROW));
    cx.update(|window, app| {
        app.set_reduce_motion(true);
        history_view(&view, app).update(app, |history, cx| {
            history.scroll_indexed_to(0, false);
            cx.notify();
        });
        let _ = window.draw(app);
        window.simulate_next_frame(app);
    });
    // A different query has the already selected commit as its first match.
    retype_query(cx, "fix an offscreen");
    wait_for_find_request(cx, &view, &store, plain_query("fix an offscreen"));
    let before = store_find(&store);
    store.dispatch(Msg::HistoryFind(HistoryFindMsg::Found {
        repo_id: FIND_REPO_ID,
        seq: before.generation(),
        result: Ok(HistoryFindChunk {
            matches: vec![FAR_FIX_ROW as u32],
            done: true,
        }),
    }));
    // Wait for the store without drawing. The next draw performs the jump.
    for _ in 0..100 {
        if store_find(&store).done {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(store_find(&store).done);
    cx.update(|window, app| {
        window.simulate_next_frame(app);
        history_view(&view, app).update(app, |history, cx| {
            history.state = store.snapshot();
            cx.notify();
        });
        let _ = window.draw(app);
        assert!(
            window.simulate_next_frame(app) > 0,
            "a scroll during drawing needs a scheduled frame"
        );
    });
}

#[gpui::test]
fn history_find_input_observes_typed_text(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, _store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    open_find_with_shortcut(cx, &view);
    cx.simulate_input("fix");
    draw_and_park(cx);
    assert_eq!(
        find_text(cx, &view),
        "fix",
        "the focused input receives text"
    );
    let query = cx.update(|_window, app| {
        history_view(&view, app)
            .read(app)
            .history_find_query()
            .map(|q| q.text().to_owned())
    });
    assert_eq!(
        query.as_deref(),
        Some("fix"),
        "the history view observes its input"
    );
}

#[gpui::test]
fn indexed_history_find_enter_retries_a_transient_failure(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::FailOnce);
    open_and_find(cx, &view, &store, "fix");
    wait_until(cx, "transient failure", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).error.is_some()
    });
    assert_eq!(find_label(cx, &view), "Search failed");
    for _ in 0..5 {
        sync_view_with_store(cx, &view);
    }
    assert_eq!(
        backend.scans_started(),
        1,
        "failure must not cause a busy retry loop"
    );
    cx.simulate_keystrokes("enter");
    wait_until(cx, "retry to finish", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).done
    });
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(backend.scans_started(), 2);
    assert_eq!(find_label(cx, &view), "1/3");
}

#[gpui::test]
fn indexed_history_find_click_cancels_a_pending_jump_and_step(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    // Both an initial jump and a step waiting on the scan yield to a click.
    for step in [false, true] {
        if step {
            cx.simulate_keystrokes("enter");
        }
        click(cx, "history_row_8");
        report_matches(cx, &view, &store, &[0, 5], true);
        assert_selection_settles_on(
            cx,
            &view,
            &store,
            Some(&indexed_find_id(8)),
            "the clicked row wins",
        );
        if !step {
            open_find_with_shortcut(cx, &view);
            retype_query(cx, "bob");
            wait_for_find_request(cx, &view, &store, plain_query("bob"));
        }
    }
}

#[gpui::test]
fn indexed_history_find_clearing_and_reopening_do_not_restore_a_stale_count(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(find_label(cx, &view), "1/2");
    cx.simulate_keystrokes("secondary-a backspace");
    draw_and_park(cx);
    assert_eq!(find_label(cx, &view), "Type to search");
    type_query(cx, "bob");
    wait_for_find_request(cx, &view, &store, plain_query("bob"));
    assert_eq!(find_label(cx, &view), "Searching…");
    report_matches(cx, &view, &store, &[2, 7], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(2));
    cx.simulate_keystrokes("escape");
    wait_until(cx, "closed search", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).query.is_none()
    });
    open_find_with_shortcut(cx, &view);
    wait_for_find_request(cx, &view, &store, plain_query("bob"));
    assert_eq!(find_label(cx, &view), "Searching…");
}

#[gpui::test]
fn indexed_history_find_switching_repos_releases_the_previous_search(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    report_matches(cx, &view, &store, &[0, 5], true);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    assert_eq!(find_label(cx, &view), "1/2");
    let mut state = (*store.snapshot()).clone();
    let mut second = state.repos[0].clone();
    second.id = RepoId(2);
    second.history_state.find = Default::default();
    second.history_state.selected_commit = None;
    state.repos.push(second);
    state.active_repo = Some(RepoId(2));
    store.insert_repo_for_test(RepoId(2), backend.0.clone());
    let state = Arc::new(state);
    store.replace_snapshot_for_test(state.clone());
    cx.update(|_window, app| {
        let model = view.read(app).ui_model.clone();
        model.update(app, |model, cx| model.set_state(state, cx));
    });
    cx.run_until_parked();
    wait_until(cx, "search in the second repo", |cx| {
        sync_view_with_store(cx, &view);
        let state = store.snapshot();
        state.repos[0].history_state.find.query.is_none()
            && state.repos[1].history_state.find.query == plain_query("fix")
    });
    assert_eq!(find_label(cx, &view), "Searching…");
    cx.update(|window, app| {
        history_view(&view, app).update(app, |history, cx| history.close_history_find(window, cx));
    });
    wait_until(cx, "all searches released", |_| {
        store
            .snapshot()
            .repos
            .iter()
            .all(|repo| repo.history_state.find.query.is_none())
    });
}

#[gpui::test]
fn history_find_repeated_destinations_acknowledge_only_the_request_that_arrived(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx) = mount_find_fixture(cx, find_fixture_repo(find_fixture_commits()));
    store.dispatch(Msg::SelectCommit {
        request_id: None,
        repo_id: FIND_REPO_ID,
        commit_id: CommitId("aaaa0000".into()),
    });
    wait_for_selection(cx, &view, &store, "aaaa0000");
    select_next_row(cx, &view);
    wait_until(cx, "first Down reduced", |_| {
        store_selected(&store).as_deref() == Some("bbbb1111")
    });
    let first_reply = store.snapshot();
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            assert!(history.history_select_adjacent_commit(1, cx));
            assert!(history.history_select_adjacent_commit(-1, cx));
            assert_eq!(history.pending_history_selections.len(), 3);
        });
        let model = view.read(app).ui_model.clone();
        model.update(app, |model, cx| model.set_state(first_reply, cx));
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            assert_eq!(
                history.pending_history_selections.len(),
                2,
                "the first Down must not acknowledge the later Up to the same row"
            );
            assert!(history.history_select_adjacent_commit(1, cx));
        });
    });
    wait_for_selection(cx, &view, &store, "cccc2222");
}

#[gpui::test]
fn indexed_history_find_store_selection_preserves_pending_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_and_find(cx, &view, &store, "fix");
    for step in [false, true] {
        if step {
            retype_query(cx, "bob");
            wait_for_find_request(cx, &view, &store, plain_query("bob"));
            cx.simulate_keystrokes("enter");
        }
        // A store-driven reconciliation/reveal, with no history input.
        let mut state = (*store.snapshot()).clone();
        state.repos[0].history_state.selected_commit = Some(CommitId(indexed_find_id(8).into()));
        state.repos[0].history_state.selected_commit_rev += 1;
        store.replace_snapshot_for_test(Arc::new(state));
        sync_view_with_store(cx, &view);
        let rows = if step { [2, 7] } else { [0, 5] };
        report_matches(cx, &view, &store, &rows, true);
        wait_for_selection(cx, &view, &store, &indexed_find_id(rows[0]));
    }
}

#[gpui::test]
fn history_find_toggle_deselect_uses_the_remaining_focus(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut repo = find_fixture_repo(find_fixture_commits());
    repo.history_state.selected_commit = Some(CommitId("cccc2222".into()));
    repo.history_state.multi_selection.commits = Arc::new(vec![
        CommitId("bbbb1111".into()),
        CommitId("cccc2222".into()),
    ]);
    let (view, store, cx) = mount_find_fixture(cx, repo);
    let at = cx.debug_bounds("history_row_2").unwrap().center();
    cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
    cx.simulate_click(
        at,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    );
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, _cx| {
            assert_eq!(
                history.history_navigation_selection(history.active_repo().unwrap(), true),
                Some(HistoryPrimarySelection::Commit(CommitId("bbbb1111".into())))
            );
        });
    });
    wait_for_selection(cx, &view, &store, "bbbb1111");
}

#[gpui::test]
fn history_find_stash_details_highlight_only_the_row_summary(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let id = "abab7777";
    let message = "WIP on main: saved work\n\nmain in the body";
    let parents = vec![CommitId("bbbb1111".into()), CommitId("9999ffff".into())];
    let mut commits = find_fixture_commits();
    commits[0] = Commit {
        parent_ids: parents.clone().into(),
        ..authored(id, "WIP on main: saved work", "Alice")
    };
    let mut repo = find_fixture_repo(commits);
    repo.history_state.selected_commit = Some(CommitId(id.into()));
    repo.history_state.commit_details =
        Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
            id: CommitId(id.into()),
            message: message.into(),
            author_name: "Alice".into(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: String::new(),
            committed_at_unix: 0,
            parent_ids: parents,
            files: vec![],
        }));
    let (view, _store, cx) = mount_find_fixture(cx, repo);
    open_find_with_shortcut(cx, &view);
    type_query(cx, "main");
    settle_typing(cx);
    assert!(
        details_find_washes(cx, &view, false).is_empty(),
        "the hidden stash prefix is not a match"
    );
    retype_query(cx, "saved");
    settle_typing(cx);
    assert_eq!(details_find_washes(cx, &view, false), vec![13..18]);
}

#[gpui::test]
fn indexed_history_find_panic_reports_failure_and_can_retry(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::PanicOnce);
    open_and_find(cx, &view, &store, "fix");
    wait_until(cx, "panic reported as search failure", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).error.is_some()
    });
    assert_eq!(find_label(cx, &view), "Search failed");
    cx.simulate_keystrokes("enter");
    wait_until(cx, "retry after panic", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).done
    });
    assert_eq!(backend.scans_started(), 2);
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
}

#[gpui::test]
fn history_find_step_after_tab_switch_uses_the_active_repos_page(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    open_find_with_shortcut(cx, &view);
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            let mut cache = history.indexed.window.as_ref().unwrap().cache.clone();
            cache.base.request.repo_id = RepoId(2);
            let mut repo = find_fixture_repo(cache.page.commits.clone());
            repo.id = RepoId(2);
            let mut next = (*history.state).clone();
            next.repos.push(repo);
            next.active_repo = Some(RepoId(2));
            history.state = Arc::new(next);
            store.replace_snapshot_for_test(Arc::clone(&history.state));
            // The new tab has a page, while the indexed presentation still
            // belongs to the old tab for this transition frame.
            history.history_cache = Some(cache);
            history.set_history_find_query("fix", false);
            assert!(history.history_find_step(true, cx));
            assert!(
                !history.pending_history_selections.is_empty(),
                "F3 must dispatch through the active page"
            );
        });
    });
    wait_until(cx, "F3 in the second repository", |_| {
        store.snapshot().repos[1]
            .history_state
            .selected_commit
            .as_ref()
            .is_some_and(|id| id.as_ref() == indexed_find_id(0))
    });
}

#[gpui::test]
fn indexed_history_find_loading_stashes_restarts_the_same_query(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, backend) = mount_indexed_find_fixture(cx, ScanMode::Serve);
    open_and_find(cx, &view, &store, "main");
    wait_until(cx, "initial search", |cx| {
        sync_view_with_store(cx, &view);
        store_find(&store).done
    });
    assert!(find_matches(cx, &view).is_empty());
    let old_index = store.snapshot().repos[0]
        .history_state
        .indexed
        .index
        .clone()
        .unwrap();
    let scans = backend.scans_started();
    let mut state = (*store.snapshot()).clone();
    state.repos[0].stashes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::StashEntry {
        index: 0,
        id: CommitId(indexed_find_id(STASH_ROW).into()),
        message: "On main: listed work".into(),
        created_at: None,
    }]));
    state.repos[0].stashes_rev += 1;
    let revision = state.repos[0].stashes_rev;
    store.replace_snapshot_for_test(Arc::new(state));
    wait_until(cx, "search with loaded stashes", |cx| {
        sync_view_with_store(cx, &view);
        let found = store_find(&store);
        found.done
            && found.stashes_rev == revision
            && cx.update(|_window, app| {
                history_view(&view, app)
                    .read(app)
                    .active_repo()
                    .is_some_and(|repo| repo.history_state.find.rev == found.rev)
            })
    });
    assert_eq!(find_matches(cx, &view), vec![STASH_ROW]);
    assert!(Arc::ptr_eq(
        &old_index,
        store.snapshot().repos[0]
            .history_state
            .indexed
            .index
            .as_ref()
            .unwrap()
    ));
    assert_eq!(
        backend.scans_started(),
        scans,
        "the new stash search reuses decoded commit text"
    );
}

#[gpui::test]
fn indexed_history_find_range_then_toggle_predicts_the_store_focus(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, store, cx, _backend) = mount_indexed_find_fixture(cx, ScanMode::Hold);
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            history.select_indexed_commit_row(FIND_REPO_ID, 0, false, cx);
        });
    });
    wait_for_selection(cx, &view, &store, &indexed_find_id(0));
    cx.update(|_window, app| {
        history_view(&view, app).update(app, |history, cx| {
            use gitcomet_state::msg::CommitSelectMode;
            history.select_history_commit(
                FIND_REPO_ID,
                CommitId(indexed_find_id(5).into()),
                CommitSelectMode::Range,
                None,
            );
            history.select_history_commit(
                FIND_REPO_ID,
                CommitId(indexed_find_id(5).into()),
                CommitSelectMode::Toggle,
                None,
            );
            assert_eq!(
                history.history_navigation_selection(history.active_repo().unwrap(), true),
                Some(HistoryPrimarySelection::Commit(CommitId(
                    indexed_find_id(4).into()
                )))
            );
            assert!(history.history_select_adjacent_commit(1, cx));
        });
    });
    wait_for_selection(cx, &view, &store, &indexed_find_id(5));
}
