//! The file-history picker, which is the newly-windowed list with a real bound
//! on it: the first page of the history load is 200 commits, which is some
//! sixteen viewports, and the rest of the history is appended behind it.

use super::super::file_history as history;
use super::*;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state, push_test_state};

const COMMIT_COUNT: usize = 200;

/// A named change to one of the inputs the rows are built from, and the label
/// the assertion reports it under.
type RowsInputBump = (&'static str, fn(&mut PopoverHost));

fn commit(ix: usize) -> gitcomet_core::domain::Commit {
    gitcomet_core::domain::Commit {
        // Commit ids are content hashes, so a distinct one per row is what a
        // real page looks like — and what the cache signature reads.
        id: CommitId(format!("{ix:0>40}").into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: format!("Commit {ix:03} touching the file").into(),
        author: "Alice".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

fn repo_with_file_history(
    repo_id: RepoId,
    path: &std::path::Path,
    next_cursor: Option<gitcomet_core::domain::LogCursor>,
) -> RepoState {
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_file_history_{}", std::process::id()));
    let mut repo = opening_repo_state(repo_id, &workdir);
    repo.history_state.file_history_path = Some(path.to_path_buf());
    repo.history_state.file_history = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits: (0..COMMIT_COUNT).map(commit).collect(),
            next_cursor,
        }
        .into(),
    );
    repo
}

/// Seeds a repository with a complete page of [`COMMIT_COUNT`] commits and
/// opens the file-history popover over it.
fn open_file_history(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> gpui::Entity<PopoverHost> {
    open_file_history_with(view, cx, None)
}

/// Like [`open_file_history`], with the page reporting `next_cursor` — `Some`
/// is a first page whose remainder is still on its way.
fn open_file_history_with(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    next_cursor: Option<gitcomet_core::domain::LogCursor>,
) -> gpui::Entity<PopoverHost> {
    let repo_id = RepoId(1);
    let path = std::path::PathBuf::from("src/main.rs");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_file_history(repo_id, &path, next_cursor);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::FileHistory {
                        repo_id,
                        path: path.clone(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
    cx.update(|_window, app| view.read(app).popover_host.clone())
}

/// Boilerplate every test below opens with. Expands to statements rather than a
/// block so that rebinding `cx` to the visual context reaches the test body.
macro_rules! file_history_picker {
    ($cx:ident, $host:ident) => {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, $cx) =
            $cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let $host = open_file_history(&view, $cx);
    };
}

/// The first page is on screen before the rest of the history has been
/// fetched. Searching that list would silently miss older commits, so the
/// picker says the rest is coming — and stops once the page is complete.
#[gpui::test]
fn file_history_reports_older_commits_still_loading(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pending = Some(gitcomet_core::domain::LogCursor {
        last_seen: commit(COMMIT_COUNT - 1).id,
        resume_from: None,
        resume_token: None,
    });
    let _popover_host = open_file_history_with(&view, cx, pending);
    assert!(
        cx.debug_bounds("file_history_loading_older").is_some(),
        "a page with a next cursor must say older commits are loading"
    );
    assert!(
        cx.debug_bounds("picker_prompt_item_0").is_some(),
        "the first page stays browsable while the rest loads"
    );

    let _popover_host = open_file_history_with(&view, cx, None);
    assert!(
        cx.debug_bounds("file_history_loading_older").is_none(),
        "a complete page has nothing left to load"
    );
}

/// Windowing is no longer opted into picker by picker, so this list gets it by
/// being long: 200 commits in a 340px viewport is some sixteen viewports of
/// content, and every frame — including the ones a hover between rows causes —
/// used to build an element for each.
#[gpui::test]
fn a_long_file_history_renders_only_the_rows_in_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    let matched = cx.update(|_window, app| {
        super::super::file_history::cached(popover_host.read(app), "")
            .layout
            .item_indices
            .len()
    });
    assert_eq!(matched, COMMIT_COUNT);

    assert!(
        cx.debug_bounds("picker_prompt_item_0").is_some(),
        "the first row is in view"
    );
    assert!(
        cx.debug_bounds("picker_prompt_item_150").is_none(),
        "a row 150 places down must not be built until it is scrolled to"
    );
}

/// Arrowing up from nothing selects the last row, far outside the window. There
/// is no element there for `ScrollHandle::scroll_to_item` to find, so this only
/// works if the picker scrolls by its row geometry.
#[gpui::test]
fn arrowing_to_the_last_file_history_row_scrolls_it_into_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    // `debug_bounds` takes a `&'static str`, so the last row is named by literal.
    const LAST_ROW: &str = "picker_prompt_item_199";
    assert!(
        cx.debug_bounds(LAST_ROW).is_none(),
        "the row this arrows to must start outside the window"
    );

    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert_eq!(
        cx.update(|_window, app| popover_host.read(app).file_history_selected_index),
        Some(COMMIT_COUNT - 1),
        "arrowing up from nothing selects the last row"
    );
    assert!(
        cx.debug_bounds(LAST_ROW).is_some(),
        "the selected row has to be scrolled into the window, not left unbuilt"
    );
}

/// The windowed list stands spacers in for the rows it does not render, sized
/// from the geometry alone. If that arithmetic drifted from what rows really
/// paint at, scrolling would drift with it.
#[gpui::test]
fn file_history_row_geometry_matches_the_height_rows_paint_at(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    let geometry = cx.update(|_window, app| {
        let rows = super::super::file_history::cached(popover_host.read(app), "");
        components::PickerPromptGeometry::new(&rows.items, &rows.layout, 100u32)
    });
    let first = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected the first row to render");
    let second = cx
        .debug_bounds("picker_prompt_item_1")
        .expect("expected the second row to render");

    assert_eq!(
        first.size.height,
        geometry.row_height(0),
        "a painted row must be exactly as tall as the geometry says"
    );
    assert_eq!(
        second.origin.y - first.origin.y,
        geometry.row_top(1) - geometry.row_top(0),
        "the stride between rows must match the geometry"
    );
}

/// `PopoverHost` is an uncached overlay view, so a hover moving between rows
/// re-renders the whole picker. Rebuilding 200 rows on each of those frames is
/// what the cache is here to avoid — and every input the rows read has to be in
/// its signature, or the list goes stale with nothing on screen to say so.
#[gpui::test]
fn file_history_rows_are_reused_until_their_data_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    let rows = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| super::super::file_history::cached(popover_host.read(app), ""))
    };

    let first = rows(cx);
    assert!(
        std::rc::Rc::ptr_eq(&first, &rows(cx)),
        "an unchanged page must hand back the very same rows"
    );

    let bumps: [RowsInputBump; 5] = [
        ("relative dates", |host| {
            host.history_relative_dates = !host.history_relative_dates
        }),
        ("date format", |host| {
            host.date_time_format = DateTimeFormat::DmyHm
        }),
        // Same number of commits, different ones: a signature that only counted
        // the rows would call this unchanged and keep showing the old page.
        ("the commits", |host| {
            let mut state = (*host.state).clone();
            state.repos[0].history_state.file_history = Loadable::Ready(
                gitcomet_core::domain::LogPage {
                    commits: (1_000..1_000 + COMMIT_COUNT).map(commit).collect(),
                    next_cursor: None,
                }
                .into(),
            );
            host.state = Arc::new(state);
        }),
        ("the file", |host| {
            let mut state = (*host.state).clone();
            state.repos[0].history_state.file_history_path =
                Some(std::path::PathBuf::from("src/other.rs"));
            host.state = Arc::new(state);
        }),
        ("the commit being viewed", |host| {
            let mut state = (*host.state).clone();
            state.repos[0].diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit(1).id,
                path: Some(std::path::PathBuf::from("src/main.rs")),
            });
            host.state = Arc::new(state);
        }),
    ];

    let mut previous = rows(cx);
    for (label, bump) in bumps {
        cx.update(|_window, app| {
            popover_host.update(app, |host, _cx| bump(host));
        });
        let rebuilt = rows(cx);
        assert!(
            !std::rc::Rc::ptr_eq(&previous, &rebuilt),
            "changing {label} must rebuild the rows"
        );
        previous = rebuilt;
    }
}

fn draw_picker(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

#[gpui::test]
fn file_history_keyboard_jumps_scroll_to_both_ends_and_page_by_viewport(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(crate::app::bind_text_input_keys_for_test);
    file_history_picker!(cx, host);
    cx.simulate_keystrokes("ctrl-end");
    draw_picker(cx);
    assert_eq!(
        cx.update(|_, app| host.read(app).file_history_selected_index),
        Some(COMMIT_COUNT - 1)
    );
    assert!(cx.debug_bounds("picker_prompt_item_199").is_some());
    cx.simulate_keystrokes("ctrl-home");
    draw_picker(cx);
    assert_eq!(
        cx.update(|_, app| host.read(app).file_history_selected_index),
        Some(0)
    );
    assert!(cx.debug_bounds("picker_prompt_item_0").is_some());
    let page_rows = cx.update(|_, app| {
        let viewport = host.read(app).picker_prompt_scroll.bounds().size.height;
        (f32::from(viewport) / f32::from(components::picker_row_height(100u32.into(), true)))
            .floor() as usize
    });
    cx.simulate_keystrokes("pagedown");
    draw_picker(cx);
    assert_eq!(
        cx.update(|_, app| host.read(app).file_history_selected_index),
        Some(page_rows)
    );
    cx.simulate_keystrokes("pageup");
    draw_picker(cx);
    assert_eq!(
        cx.update(|_, app| host.read(app).file_history_selected_index),
        Some(0)
    );
}

#[gpui::test]
fn file_history_rows_show_monospace_sha_author_dates_and_filter_in_history_order(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, host);
    cx.update(|_, app| {
        host.update(app, |host, cx| {
            let first = history::cached(host, "");
            assert_eq!(
                first.items[0].debug_display_text(),
                "Commit 000 touching the file"
            );
            assert_eq!(
                first.items[0].debug_secondary_part_font_families()[1],
                Some(crate::view::UI_MONOSPACE_FONT_FAMILY)
            );
            assert!(
                first.items[0]
                    .debug_secondary_text()
                    .contains("00000000  •  Alice  •  ")
            );
            assert!(first.items[0].debug_secondary_text().ends_with("ago"));
            let filtered = history::cached(host, "1");
            assert!(std::rc::Rc::ptr_eq(&first.items, &filtered.items));
            assert!(
                filtered
                    .layout
                    .item_indices
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
            );
            assert_eq!(history::cached(host, "Alice").filtered_len(), COMMIT_COUNT);
            host.set_history_relative_dates(false, cx);
            host.set_timezone(Timezone::Utc, cx);
            let absolute = history::cached(host, "");
            assert!(
                absolute.items[0]
                    .debug_secondary_text()
                    .contains("1970-01-01")
            );
            let mut state = (*host.state).clone();
            let Loadable::Ready(page) = &mut state.repos[0].history_state.file_history else {
                panic!()
            };
            let page = Arc::make_mut(page);
            for commit in &mut page.commits {
                commit.summary = "summary".into();
            }
            host.state = Arc::new(state);
            // The blank marker, layout separators, and dates are not search fields.
            assert_eq!(history::cached(host, "  ").filtered_len(), 0);
        })
    });
}

#[gpui::test]
fn file_history_ignores_a_page_for_a_different_file(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, host);
    cx.update(|_, app| {
        host.update(app, |host, cx| {
            host.popover = Some(PopoverKind::FileHistory {
                repo_id: RepoId(1),
                path: "src/other.rs".into(),
            });
            assert_eq!(history::cached(host, "").filtered_len(), 0);
            cx.notify();
        })
    });
    draw_picker(cx);
    assert!(cx.debug_bounds("picker_prompt_item_0").is_none());
}

fn right_click_history_row(cx: &mut gpui::VisualTestContext) {
    let center = cx.debug_bounds("picker_prompt_item_1").unwrap().center();
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    draw_picker(cx);
}

#[gpui::test]
fn file_history_row_menu_offers_file_and_commit_actions_and_routes_keyboard(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(crate::app::bind_text_input_keys_for_test);
    file_history_picker!(cx, host);
    right_click_history_row(cx);
    cx.update(|_, app| {
        host.update(app, |host, cx| {
            let model = host
                .picker_row_menu
                .as_ref()
                .unwrap()
                .model_for_test(host, cx);
            let labels: Vec<_> = model
                .items
                .iter()
                .filter_map(|item| match item {
                    ContextMenuItem::Entry { label, .. } => Some(label.as_ref()),
                    _ => None,
                })
                .collect();
            for expected in [
                "Open file at this commit",
                "Open file at parent",
                "Show changes to this file",
                "Reveal in history",
                "Copy SHA",
                "Checkout (detached)",
                "Create branch from this commit",
            ] {
                assert!(labels.contains(&expected), "missing {expected}");
            }
        })
    });
    cx.simulate_keystrokes("down");
    draw_picker(cx);
    assert_eq!(
        cx.update(|_, app| host.read(app).file_history_selected_index),
        Some(0)
    );
    cx.simulate_keystrokes("escape");
    draw_picker(cx);
    cx.update(|_, app| {
        let host = host.read(app);
        assert!(host.picker_row_menu.is_none());
        assert!(host.is_open());
        assert_eq!(host.file_history_selected_index, Some(1));
    });
    right_click_history_row(cx);
    cx.simulate_keystrokes("A");
    draw_picker(cx);
    assert!(cx.update(|_, app| host.read(app).picker_row_menu.is_none()));
}

#[gpui::test]
fn copying_file_history_sha_keeps_picker_open(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let _clipboard_guard = crate::test_support::lock_clipboard_test();
    file_history_picker!(cx, host);
    right_click_history_row(cx);
    cx.update(|window, app| host.update(app, |host, cx| {
        let action = picker_row_menu::nav_actions(host, cx).unwrap().into_iter().find(|action| matches!(action, ContextMenuAction::CopyText { text } if text == commit(1).id.as_ref())).unwrap();
        picker_row_menu::activate(host, action, window, cx);
        assert!(host.is_open());
        assert!(host.picker_row_menu.is_none());
        assert_eq!(crate::clipboard::read_text(cx).as_deref(), Some(commit(1).id.as_ref()));
    }));
}

#[gpui::test]
fn file_history_enter_activates_row_and_closes_picker(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(crate::app::bind_text_input_keys_for_test);
    file_history_picker!(cx, host);
    let store = cx.update(|_, app| {
        let host = host.read(app);
        let store = host.store.clone();
        store.replace_snapshot_for_test(Arc::clone(&host.state));
        store.insert_repo_for_test(
            RepoId(1),
            Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                "/tmp/file-history-enter",
            )),
        );
        store
    });
    cx.simulate_keystrokes("down enter");
    draw_picker(cx);
    assert!(cx.update(|_, app| !host.read(app).is_open()));
    super::branch::wait_until("file content preview", || {
        store.snapshot().repos[0].diff_state.content_preview
    });
    let state = store.snapshot();
    assert_eq!(
        state.repos[0].diff_state.diff_target,
        Some(DiffTarget::Commit {
            commit_id: commit(0).id,
            path: Some("src/main.rs".into())
        })
    );
    assert_eq!(state.repos[0].navigation.view_history.entries.len(), 1);
}

#[gpui::test]
fn file_history_show_changes_opens_a_file_diff_and_closes_picker(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, host);
    let store = cx.update(|_, app| {
        let host = host.read(app);
        let store = host.store.clone();
        store.replace_snapshot_for_test(Arc::clone(&host.state));
        store.insert_repo_for_test(
            RepoId(1),
            Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                "/tmp/file-history-changes",
            )),
        );
        store
    });
    right_click_history_row(cx);
    cx.update(|window, app| {
        host.update(app, |host, cx| {
            let action = picker_row_menu::nav_actions(host, cx)
                .unwrap()
                .into_iter()
                .find(|action| matches!(action, ContextMenuAction::ShowFileChangesAtCommit { .. }))
                .unwrap();
            picker_row_menu::activate(host, action, window, cx);
            assert!(!host.is_open());
        })
    });
    let expected = DiffTarget::Commit {
        commit_id: commit(1).id,
        path: Some("src/main.rs".into()),
    };
    super::branch::wait_until("file changes", || {
        store.snapshot().repos[0].diff_state.diff_target.as_ref() == Some(&expected)
    });
    assert!(!store.snapshot().repos[0].diff_state.content_preview);
}
