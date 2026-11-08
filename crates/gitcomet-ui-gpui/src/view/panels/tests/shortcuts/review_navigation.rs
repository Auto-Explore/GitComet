use super::*;
use crate::view::rows::FileListId;

#[path = "review_regressions.rs"]
mod regressions;

fn review_repo() -> RepoState {
    let id = CommitId("abcdef0123456789".into());
    let mut repo = shortcut_fixture_repo(
        RepoId(9071),
        &std::env::temp_dir().join("gitcomet-review-keys"),
        &id,
    );
    repo.history_state.selected_commit = Some(id.clone());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id,
        message: "Review navigation".into(),
        author_name: "Alice".into(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: String::new(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: ["a.rs", "b.rs", "c.png"]
            .into_iter()
            .map(|path| CommitFileChange::new(path.into(), FileStatusKind::Modified))
            .collect(),
    }));
    repo.history_state.commit_details_rev = 1;
    repo
}

fn prepare(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>) {
    apply_state(cx, view, app_state_with_active_repo(review_repo()));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    cx.update(|_, app| crate::app::bind_text_input_keys_for_test(app));
    cx.update(|window, app| {
        window.activate();
        let root = view.read(app);
        let details = root.details_pane.clone();
        let history = root.main_pane.read(app).history_view.clone();
        details.update(app, |pane, cx| {
            pane.file_list_layout = FileListLayout::Flat;
            cx.notify();
        });
        let focus = history.read(app).history_panel_focus_handle.clone();
        window.focus(&focus, app);
    });
    draw_and_drain_test_window(cx);
}

fn files_focused(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>) -> bool {
    cx.update(|window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .review_file_focus(FileListId::CommitFiles)
            .is_focused(window)
    })
}

fn graph_focused(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>) -> bool {
    cx.update(|window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_panel_focus_handle
            .is_focused(window)
    })
}

fn selected_path(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
) -> Option<std::path::PathBuf> {
    cx.update(|_, app| {
        view.read(app).store.snapshot().repos[0]
            .diff_state
            .diff_target
            .as_ref()
            .and_then(DiffTarget::file_path)
            .map(std::path::Path::to_owned)
    })
}

fn open_file(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>, path: &str) {
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, view, std::path::Path::new(path));
    sync_store_snapshot(cx, view);
    assert!(files_focused(cx, view));
    cx.simulate_keystrokes("enter");
    draw_and_drain_test_window(cx);
    assert!(diff_panel_is_focused(cx, view));
}

fn close_commit_details(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    area: Option<DiffArea>,
) {
    prepare(cx, view);
    let mut repo = review_repo();
    let file = gitcomet_core::domain::FileStatus {
        path: "working.rs".into(),
        kind: FileStatusKind::Modified,
        conflict: None,
    };
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: Arc::new(if area == Some(DiffArea::Staged) {
                vec![file.clone()]
            } else {
                vec![]
            }),
            unstaged: Arc::new(if area == Some(DiffArea::Unstaged) {
                vec![file]
            } else {
                vec![]
            }),
        }
        .into(),
    );
    apply_state(cx, view, app_state_with_active_repo(repo));
    cx.simulate_keystrokes("f6");
    draw_and_drain_test_window(cx);
    assert!(files_focused(cx, view));
    let close = cx.debug_bounds("commit_details_close_icon").unwrap();
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    wait_until(cx, "commit details closed", |cx| {
        sync_store_snapshot(cx, view);
        cx.update(|_, app| {
            view.read(app).state.repos[0]
                .history_state
                .selected_commit
                .is_none()
        })
    });
    draw_and_drain_test_window(cx);
}

#[gpui::test]
fn review_close_commit_details_focuses_graph_when_status_is_empty(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    close_commit_details(cx, &view, None);
    assert!(graph_focused(cx, &view));
    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    assert!(cx.debug_bounds("history_find").is_some());
}

#[gpui::test]
fn review_close_commit_details_preserves_and_focuses_open_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    let target = cx.update(|_, app| {
        view.read(app).store.snapshot().repos[0]
            .diff_state
            .diff_target
            .clone()
    });
    let close = cx.debug_bounds("commit_details_close_icon").unwrap();
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    wait_until(cx, "commit details closed over diff", |cx| {
        sync_store_snapshot(cx, &view);
        cx.update(|_, app| {
            view.read(app).state.repos[0]
                .history_state
                .selected_commit
                .is_none()
        })
    });
    assert!(diff_panel_is_focused(cx, &view));
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).store.snapshot().repos[0]
                .diff_state
                .diff_target,
            target
        )
    });
    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    assert!(diff_search_input_is_focused(cx, &view));
}

fn assert_close_focuses_status(
    cx: &mut gpui::TestAppContext,
    area: DiffArea,
    section: StatusSection,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    close_commit_details(cx, &view, Some(area));
    assert!(cx.update(|window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .review_file_focus(FileListId::Status(section))
            .is_focused(window)
    }));
    cx.simulate_keystrokes("down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("working.rs"));
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).store.snapshot().repos[0]
                .diff_state
                .diff_target,
            Some(DiffTarget::working_tree("working.rs".into(), area))
        )
    });
}

#[gpui::test]
fn review_close_commit_details_focuses_unstaged_rows(cx: &mut gpui::TestAppContext) {
    assert_close_focuses_status(cx, DiffArea::Unstaged, StatusSection::CombinedUnstaged);
}

#[gpui::test]
fn review_close_commit_details_focuses_staged_rows(cx: &mut gpui::TestAppContext) {
    assert_close_focuses_status(cx, DiffArea::Staged, StatusSection::Staged);
}

fn working_copy_repo(staged: &[&str], unstaged: &[&str]) -> RepoState {
    let mut repo = review_repo();
    repo.history_state.selected_commit = None;
    let files = |paths: &[&str]| {
        Arc::new(
            paths
                .iter()
                .map(|path| gitcomet_core::domain::FileStatus {
                    path: (*path).into(),
                    kind: FileStatusKind::Modified,
                    conflict: None,
                })
                .collect(),
        )
    };
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: files(staged),
            unstaged: files(unstaged),
        }
        .into(),
    );
    repo
}

fn focus_status(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    section: StatusSection,
) {
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .details_pane
            .read(app)
            .review_file_focus(FileListId::Status(section))
            .clone();
        window.focus(&focus, app);
    });
    draw_and_drain_test_window(cx);
}

#[gpui::test]
fn review_return_to_files_preserves_staged_area(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = working_copy_repo(&["both.rs"], &["both.rs"]);
    let target = DiffTarget::working_tree("both.rs".into(), DiffArea::Staged);
    repo.diff_state.diff_target = Some(target.clone());
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        view.update(app, |root, cx| {
            root.navigate_review(
                crate::view::review_navigation::ReviewNavigation::Files { from_graph: false },
                window,
                cx,
            )
        });
    });
    draw_and_drain_test_window(cx);
    assert!(cx.update(|window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .review_file_focus(FileListId::Status(StatusSection::Staged))
            .is_focused(window)
    }));
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).store.snapshot().repos[0]
                .diff_state
                .diff_target,
            Some(target)
        )
    });
}

#[gpui::test]
fn review_mouse_selection_replaces_previous_keyboard_cursor(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = working_copy_repo(&[], &["a.rs", "b.rs", "c.rs"]);
    repo.diff_state.diff_target = Some(DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_status(cx, &view, StatusSection::CombinedUnstaged);
    cx.simulate_keystrokes("down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    let row = cx.debug_bounds("status_row_9071_unstaged_2").unwrap();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("c.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("up");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
}

#[gpui::test]
fn review_clicking_previewed_file_clears_directory_cursor(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(working_copy_repo(&[], &["src/a.rs", "src/b.rs"])),
    );
    cx.update(|_, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.file_list_layout = FileListLayout::Tree;
            cx.notify();
        });
    });
    focus_status(cx, &view, StatusSection::CombinedUnstaged);
    cx.simulate_keystrokes("down down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("src/a.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("left");
    draw_and_drain_test_window(cx);
    let row = cx.debug_bounds("status_row_9071_unstaged_1").unwrap();
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("enter");
    draw_and_drain_test_window(cx);
    assert!(diff_panel_is_focused(cx, &view));
    assert_eq!(
        selected_path(cx, &view).as_deref(),
        Some(std::path::Path::new("src/a.rs"))
    );
}

#[gpui::test]
fn review_staged_search_returns_focus_to_staged_results(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = working_copy_repo(&["a.rs", "b.rs"], &["unstaged.rs"]);
    repo.diff_state.diff_target = Some(DiffTarget::working_tree("a.rs".into(), DiffArea::Staged));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_status(cx, &view, StatusSection::Staged);
    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    cx.simulate_input("b.rs");
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    assert!(cx.update(|window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .review_file_focus(FileListId::Status(StatusSection::Staged))
            .is_focused(window)
    }));
}

#[gpui::test]
fn review_worktree_files_can_be_focused_from_root_navigation(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = working_copy_repo(&[], &[]);
    let linked = tempfile::tempdir().unwrap();
    repo.history_state.worktree_selection = Some(linked.path().to_owned());
    repo.worktree_dirty = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::WorktreeDirtySummary {
            path: linked.path().to_owned(),
            head: None,
            branch: Some("linked".into()),
            detached: false,
            added: 0,
            modified: 1,
            deleted: 0,
            staged: vec![],
            unstaged: vec![gitcomet_core::domain::FileStatus {
                path: "a.rs".into(),
                kind: FileStatusKind::Modified,
                conflict: None,
            }],
            line_stats: Default::default(),
        },
    ]));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        view.update(app, |root, cx| {
            root.navigate_review(
                crate::view::review_navigation::ReviewNavigation::Files { from_graph: true },
                window,
                cx,
            )
        });
    });
    draw_and_drain_test_window(cx);
    assert!(cx.update(|window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .review_file_focus(FileListId::WorktreeFiles)
            .is_focused(window)
    }));
}

#[gpui::test]
fn review_search_does_not_leave_file_jump_pending(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    cx.simulate_input("new");
    draw_and_drain_test_window(cx);
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .diff_renderer
            .read(app)
            .diff_panel_focus_handle
            .clone();
        window.focus(&focus, app);
    });
    cx.simulate_keystrokes("alt-down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, false, false);
    cx.simulate_keystrokes("escape alt-down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("c.png"));
}

#[gpui::test]
fn review_cycle_focuses_the_mounted_document_canvas(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    cx.update(|window, app| {
        view.update(app, |root, cx| {
            root.set_sidebar_collapsed(false, cx);
            root.documents_active = true;
            cx.notify();
        });
        let focus = view.read(app).sidebar_pane.read(app).sidebar_focus.clone();
        window.focus(&focus, app);
    });
    draw_and_drain_test_window(cx);
    assert!(cx.debug_bounds("documents_canvas").is_some());
    assert!(cx.update(|window, app| {
        view.read(app)
            .sidebar_pane
            .read(app)
            .sidebar_focus
            .is_focused(window)
    }));
    cx.simulate_keystrokes("f6");
    draw_and_drain_test_window(cx);
    let focus_state = cx.update(|window, app| {
        let root = view.read(app);
        (
            root.repository_view_focus.is_focused(window),
            root.documents_active,
            root.sidebar_pane.read(app).sidebar_focus.is_focused(window),
            root.background_hovers_blocked(app),
            root.details_pane
                .read(app)
                .visible_review_focus_handles()
                .iter()
                .any(|handle| handle.is_focused(window)),
        )
    });
    assert!(focus_state.0, "document focus state: {focus_state:?}");
    assert!(!graph_focused(cx, &view));
}

fn diff_content(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    loading: bool,
    empty: bool,
) {
    let mut state = cx.update(|_, app| (*view.read(app).store.snapshot()).clone());
    let repo = &mut state.repos[0];
    let target = repo.diff_state.diff_target.clone().unwrap();
    repo.diff_state.diff = if loading {
        Loadable::Loading
    } else {
        let path = target.file_path().unwrap().to_string_lossy().into_owned();
        let mut diff = two_hunk_diff(target);
        for line in &mut diff.lines {
            line.text = line.text.replace("src/lib.rs", &path).into();
        }
        if empty {
            diff.lines.clear();
        }
        Loadable::Ready(Arc::new(diff))
    };
    repo.diff_state.diff_file = if loading {
        Loadable::Loading
    } else {
        if empty {
            Loadable::Ready(None)
        } else {
            Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
                repo.diff_state.diff_target.as_ref().unwrap().file_path().unwrap().to_owned(),
                Some("old one\ncontext1\ncontext2\ncontext3\ncontext4\ncontext5\ncontext6\ncontext7\ncontext8\nold two\n".into()),
                Some("new one\ncontext1\ncontext2\ncontext3\ncontext4\ncontext5\ncontext6\ncontext7\ncontext8\nnew two\n".into()),
            ))))
        }
    };
    repo.diff_state.diff_file_rev += 1;
    repo.diff_state.diff_rev += 1;
    repo.diff_state.diff_state_rev += 1;
    apply_state(cx, view, Arc::new(state));
    if !loading && !empty {
        wait_until(cx, "review diff blocks", |cx| {
            cx.update(|_, app| {
                let pane = view.read(app).diff_renderer.read(app);
                pane.diff_nav_entries().len() == 2 && !pane.review_diff_is_loading()
            })
        });
    }
}

#[gpui::test]
fn review_enter_escape_restores_each_pane_and_keeps_graph_selection(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    cx.simulate_keystrokes("down");
    draw_and_drain_test_window(cx);
    let anchor = diff_selection_anchor(cx, &view);
    assert!(anchor.is_some());
    assert!(
        cx.debug_bounds("review_file_focus").is_none(),
        "the file preview must not look focused while reviewing the diff"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(files_focused(cx, &view));
    let outline = cx
        .debug_bounds("review_file_focus")
        .expect("selected file outline");
    let row_height = cx.update(|_, app| {
        let pane = view.read(app).details_pane.read(app);
        crate::view::rows::sidebar::sidebar_list_row_height(pane.theme, pane.ui_scale_percent)
    });
    assert_eq!(
        outline.size.height, row_height,
        "the outline encloses one file row, not the list container"
    );
    assert_eq!(
        selected_path(cx, &view).as_deref(),
        Some(std::path::Path::new("a.rs"))
    );
    cx.simulate_keystrokes("escape");
    wait_until(cx, "graph restored", |cx| {
        sync_store_snapshot(cx, &view);
        selected_path(cx, &view).is_none()
    });
    assert!(graph_focused(cx, &view));
    assert!(
        cx.debug_bounds("review_file_focus").is_none(),
        "the file outline disappears when focus returns to the graph"
    );
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("a.rs"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, false, false);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        anchor,
        "return to the same block"
    );
    cx.simulate_keystrokes("escape");
    wait_until(cx, "graph restored again", |cx| {
        sync_store_snapshot(cx, &view);
        selected_path(cx, &view).is_none()
    });
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    cx.update(|_, app| {
        let root = view.read(app);
        assert!(root.details_pane.read(app).review_details_dismissed);
        assert!(!root.details_collapsed);
        assert!(root.state.repos[0].history_state.selected_commit.is_some());
    });
    let mut state = cx.update(|_, app| (*view.read(app).store.snapshot()).clone());
    state.repos[0].history_state.selected_commit_rev += 1;
    apply_state(cx, &view, Arc::new(state));
    assert!(!cx.update(|_, app| {
        view.read(app)
            .details_pane
            .read(app)
            .review_details_dismissed
    }));
}

#[gpui::test]
fn review_arrows_cross_blocks_and_files_once_and_wait_for_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    let first = diff_selection_anchor(cx, &view).unwrap();
    cx.simulate_keystrokes("down");
    draw_and_drain_test_window(cx);
    let last = diff_selection_anchor(cx, &view).unwrap();
    assert!(
        last > first,
        "Down moves to a block, not the next line or file"
    );
    assert_eq!(
        selected_path(cx, &view).as_deref(),
        Some(std::path::Path::new("a.rs"))
    );
    cx.simulate_keystrokes("down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, true, false);
    cx.simulate_keystrokes("down down alt-down");
    draw_and_drain_test_window(cx);
    assert_eq!(
        selected_path(cx, &view).as_deref(),
        Some(std::path::Path::new("b.rs"))
    );
    diff_content(cx, &view, false, false);
    assert_eq!(diff_selection_anchor(cx, &view), Some(first));
    cx.simulate_keystrokes("up");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("a.rs"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, false, false);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(last),
        "backward crossing lands on the last block"
    );
    cx.simulate_keystrokes("alt-down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, false, false);
    cx.simulate_keystrokes("alt-down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("c.png"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, false, true);
    cx.simulate_keystrokes("down");
    draw_and_drain_test_window(cx);
    assert_eq!(
        selected_path(cx, &view).as_deref(),
        Some(std::path::Path::new("c.png")),
        "outer edge never wraps"
    );
    cx.simulate_keystrokes("up");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    diff_content(cx, &view, false, false);
    assert_eq!(diff_selection_anchor(cx, &view), Some(last));
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(files_focused(cx, &view));
    assert_eq!(
        selected_path(cx, &view).as_deref(),
        Some(std::path::Path::new("b.rs")),
        "Escape follows the diff's current file"
    );
}

#[gpui::test]
fn review_file_filter_and_context_menu_keep_keyboard_focus(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("a.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("review_file_search").is_some(),
        "file search is visible"
    );
    cx.simulate_input("B.RS");
    draw_and_drain_test_window(cx);
    cx.update(|_, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.active_commit_file_source_indices(RepoId(9071))
                .unwrap()
                .len(),
            1
        );
    });
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    assert!(files_focused(cx, &view));
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(files_focused(cx, &view));
    cx.simulate_keystrokes("shift-f10");
    draw_and_drain_test_window(cx);
    assert!(popover_is_open(cx, &view));
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(!popover_is_open(cx, &view));
    assert!(files_focused(cx, &view));
}

#[gpui::test]
fn review_tree_and_pane_cycle_respect_focus_and_modals(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = review_repo();
    if let Loadable::Ready(details) = &mut repo.history_state.commit_details {
        Arc::make_mut(details).files = ["docs/a.md", "src/a.rs", "src/b.rs"]
            .into_iter()
            .map(|path| CommitFileChange::new(path.into(), FileStatusKind::Modified))
            .collect();
    }
    repo.history_state.commit_details_rev += 1;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|_, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.file_list_layout = FileListLayout::Tree;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);
    assert!(graph_focused(cx, &view), "graph before F6");
    cx.simulate_keystrokes("f6");
    draw_and_drain_test_window(cx);
    assert!(
        files_focused(cx, &view),
        "F6 must focus files; graph still focused: {}",
        graph_focused(cx, &view)
    );
    cx.simulate_keystrokes("shift-f6");
    draw_and_drain_test_window(cx);
    assert!(graph_focused(cx, &view));
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("docs/a.md"));
    sync_store_snapshot(cx, &view);
    // Left selects the parent, then collapses it. Down moves to the next
    // folder; Right enters its first file without moving focus to the diff.
    cx.simulate_keystrokes("left left down right");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("src/a.rs"));
    sync_store_snapshot(cx, &view);
    assert!(files_focused(cx, &view));
    cx.simulate_keystrokes("down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("src/b.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("secondary-p");
    draw_and_drain_test_window(cx);
    assert!(cx.update(|_, app| view.read(app).command_palette_open));
    let focus = cx.update(|window, app| window.focused(app));
    cx.simulate_keystrokes("f6 shift-f6");
    draw_and_drain_test_window(cx);
    assert_eq!(cx.update(|window, app| window.focused(app)), focus);
}
