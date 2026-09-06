use super::*;
use gitcomet_core::domain::{LogPage, WorktreeDirtySummary};
use gitcomet_core::services::{CancellationToken, HistoryReadRequest, HistoryReadResult};

fn mount(
    cx: &mut gpui::TestAppContext,
    page: Arc<LogPage>,
) -> (
    Entity<GitCometView>,
    &mut gpui::VisualTestContext,
    AppState,
    AppStore,
) {
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
    let store_for_test = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-refresh-viewport"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.history_state.log = Loadable::Ready(page);
    repo.log_rev = 1;
    let state = AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..Default::default()
    };
    store_for_test.replace_snapshot_for_test(Arc::new(state.clone()));
    cx.update(|_, app| {
        view.read(app).ui_model.clone().update(app, |model, cx| {
            model.set_state(Arc::new(state.clone()), cx)
        });
    });
    ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
    wait_until(cx, "history layout", |cx| {
        cx.update(|_, app| {
            let history = view.read(app).main_pane.read(app).history_view.read(app);
            history.history_cache.is_some()
                && history.history_scroll.0.borrow().last_item_size.is_some()
        })
    });
    (view, cx, state, store_for_test)
}

fn commits(count: usize) -> Vec<Commit> {
    (0..count)
        .map(|ix| commit(&format!("c{ix}"), &[], &format!("commit {ix}")))
        .collect()
}

fn top(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>) -> (CommitId, Pixels) {
    cx.update(|_, app| {
        let entity = view.read(app).main_pane.read(app).history_view.clone();
        entity.update(app, |history, _| {
            let plan = history.ensure_history_list_plan();
            let offset = history.history_scroll.0.borrow().base_handle.offset().y;
            let height = crate::view::rows::history_row_height(history.ui_scale());
            let list_ix = (-offset / height).floor() as usize;
            let crate::view::caches::HistoryListRow::Commit { visible_ix } =
                plan.row_at(list_ix).unwrap()
            else {
                panic!("expected commit at viewport top")
            };
            let cache = history.history_cache.as_ref().unwrap();
            (
                cache.page.commits[cache.base.visible_indices.get(visible_ix).unwrap()]
                    .id
                    .clone(),
                offset + height * list_ix as f32,
            )
        })
    })
}

fn scroll(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>, row: Option<usize>) {
    cx.update(|window, app| {
        let history = view.read(app).main_pane.read(app).history_view.read(app);
        let handle = history.history_scroll.0.borrow();
        let height = crate::view::rows::history_row_height(history.ui_scale());
        let y = row.map_or(-handle.base_handle.max_offset().y, |row| {
            -(height * row as f32 + px(7.0))
        });
        handle.base_handle.set_offset(point(px(0.0), y));
        window.refresh();
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

/// Park the cache request without completing it, so a real frame is rendered
/// after state changes but before the replacement graph exists.
fn hold_rebuild(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>, state: &AppState) {
    cx.update(|window, app| {
        let entity = view.read(app).main_pane.read(app).history_view.clone();
        entity.update(app, |history, cx| {
            history.state = Arc::new(state.clone());
            let repo = history.active_repo().unwrap();
            let page = HistoryView::display_log_page_for_repo(repo).unwrap();
            history.history_cache_inflight = Some(HistoryCacheBuildRequest {
                base_request: history.history_base_cache_request_for_repo(repo, &page),
                decoration_request: history.history_decoration_cache_request_for_repo(repo, &page),
            });
            cx.notify();
        });
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn release_rebuild(cx: &mut gpui::VisualTestContext, view: &Entity<GitCometView>) {
    cx.update(|_, app| {
        let entity = view.read(app).main_pane.read(app).history_view.clone();
        entity.update(app, |history, cx| {
            history.history_cache_inflight = None;
            history.ensure_history_cache(cx);
            cx.notify();
        });
    });
    wait_until(cx, "replacement displayed", |cx| {
        cx.update(|_, app| {
            let history = view.read(app).main_pane.read(app).history_view.read(app);
            history.history_cache_inflight.is_none()
                && history.pending_history_cache.is_none()
                && history.history_cache.as_ref().is_some_and(|cache| {
                    history
                        .active_repo()
                        .and_then(HistoryView::display_log_page_for_repo)
                        .is_some_and(|page| Arc::ptr_eq(&cache.page, &page))
                })
        })
    });
}

#[gpui::test]
fn deep_refresh_keeps_the_displayed_page_and_respects_scrolling_during_the_build(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let page = Arc::new(log_page(commits(50_000), None));
    let (view, cx, mut state, _) = mount(cx, Arc::clone(&page));
    scroll(cx, &view, None);
    let before = top(cx, &view);
    let updated = (0..601)
        .map(|i| commit(&format!("new{i}"), &[], "new"))
        .chain(page.commits.iter().cloned())
        .collect();
    state.repos[0].log = Loadable::Ready(Arc::new(log_page(updated, None)));
    state.repos[0].log_rev += 1;
    hold_rebuild(cx, &view, &state);
    assert_eq!(
        top(cx, &view),
        before,
        "the pending frame still shows the old source"
    );
    scroll(cx, &view, Some(49_950));
    let moved = top(cx, &view);
    release_rebuild(cx, &view);
    assert_eq!(
        top(cx, &view),
        moved,
        "restoration must use the latest user scroll"
    );
}

#[gpui::test]
fn interior_reorder_updates_the_graph_and_clicks_use_the_displayed_source(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let page = Arc::new(log_page(commits(600), None));
    let (view, cx, mut state, store) = mount(cx, Arc::clone(&page));
    scroll(cx, &view, Some(450));
    let before = top(cx, &view);
    let mut reordered = page.commits.clone();
    reordered.swap(300, 450); // Same length and the same first/last three IDs.
    state.repos[0].log = Loadable::Ready(Arc::new(log_page(reordered, None)));
    state.repos[0].log_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    hold_rebuild(cx, &view, &state);
    let bounds = cx.debug_bounds("history_row_450").unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    wait_until(cx, "displayed commit selected", |_| {
        store.snapshot().repos[0]
            .history_state
            .selected_commit
            .as_ref()
            == Some(&before.0)
    });
    release_rebuild(cx, &view);
    assert_eq!(top(cx, &view), before);
}

fn dirty(path: &str, head: &str) -> WorktreeDirtySummary {
    WorktreeDirtySummary {
        path: PathBuf::from(path),
        head: Some(CommitId(head.into())),
        branch: Some("side".into()),
        detached: false,
        added: 1,
        modified: 0,
        deleted: 0,
        staged: Vec::new(),
        unstaged: Vec::new(),
    }
}

#[gpui::test]
fn pagination_waits_for_the_new_source_and_uses_its_current_extent(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx, mut state, store) = mount(cx, Arc::new(log_page(commits(600), None)));
    scroll(cx, &view, None);
    state.repos[0].log = Loadable::Ready(Arc::new(log_page(commits(1_000), Some("c999"))));
    state.repos[0].log_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    let before = state.repos[0].loads_in_flight.clone();
    hold_rebuild(cx, &view, &state);
    for (release, id) in [(false, "c1"), (true, "c2")] {
        if release {
            release_rebuild(cx, &view);
        }
        // A user message is an ordering barrier after the frame's dispatches.
        store.dispatch(Msg::SelectCommit {
            repo_id: RepoId(1),
            commit_id: CommitId(id.into()),
        });
        wait_until(cx, "frame dispatches consumed", |_| {
            store.snapshot().repos[0]
                .history_state
                .selected_commit
                .as_ref()
                .is_some_and(|selected| selected.as_ref() == id)
        });
        assert_eq!(
            store.snapshot().repos[0].loads_in_flight,
            before,
            "the previous layout's bottom must not trigger pagination for the new source"
        );
    }
}

#[gpui::test]
fn synthetic_rows_and_deleted_anchors_preserve_the_nearest_surviving_commit(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let page = Arc::new(log_page(commits(600), None));
    let (view, cx, mut state, _) = mount(cx, Arc::clone(&page));
    scroll(cx, &view, Some(450));
    let before = top(cx, &view);
    state.repos[0].worktree_dirty = Loadable::Ready(Arc::new(vec![dirty("/wt/one", "c10")]));
    state.repos[0].worktree_dirty_rev += 1;
    ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
    assert_eq!(
        top(cx, &view),
        before,
        "adding a worktree row above must preserve the commit"
    );
    state.repos[0].worktree_dirty = Loadable::Ready(Arc::new(Vec::new()));
    state.repos[0].worktree_dirty_rev += 1;
    ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
    assert_eq!(top(cx, &view), before);
    let mut removed = page.commits.clone();
    removed.remove(450);
    state.repos[0].log = Loadable::Ready(Arc::new(log_page(removed, None)));
    state.repos[0].log_rev += 1;
    hold_rebuild(cx, &view, &state);
    release_rebuild(cx, &view);
    // c451 keeps its prior screen coordinate; c449 becomes the partial top row.
    assert_eq!(top(cx, &view), (CommitId("c449".into()), before.1));
}

fn activate_and_check(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    store: &AppStore,
    expected: &(CommitId, Pixels),
) {
    let before = store.snapshot().repos[0].loads_in_flight.clone();
    store.dispatch(Msg::RepoActivated { repo_id: RepoId(1) });
    wait_until(cx, "activation refresh completed", |cx| {
        let state = store.snapshot();
        ensure_history_cache_for_tests(cx, view, Arc::clone(&state));
        assert_eq!(
            &top(cx, view),
            expected,
            "every refresh frame must retain its viewport"
        );
        let loads = &state.repos[0].loads_in_flight;
        loads != &before && !loads.any_in_flight()
    });
}

#[gpui::test]
fn worktree_viewport_anchors_follow_paths_when_status_reorders_and_moves_rows(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx, mut state, _) = mount(cx, Arc::new(log_page(commits(600), None)));
    state.repos[0].worktree_dirty = Loadable::Ready(Arc::new(vec![
        dirty("/wt/a", "c300"),
        dirty("/wt/b", "c450"),
    ]));
    state.repos[0].worktree_dirty_rev += 1;
    ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
    scroll(cx, &view, Some(451));
    for head in ["c450", "c200"] {
        state.repos[0].worktree_dirty =
            Loadable::Ready(Arc::new(vec![dirty("/wt/b", head), dirty("/wt/a", "c300")]));
        state.repos[0].worktree_dirty_rev += 1;
        ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
        cx.update(|_, app| {
            let entity = view.read(app).main_pane.read(app).history_view.clone();
            entity.update(app, |history, _| {
                let plan = history.ensure_history_list_plan();
                let height = crate::view::rows::history_row_height(history.ui_scale());
                let offset = history.history_scroll.0.borrow().base_handle.offset().y;
                let ix = (-offset / height).floor() as usize;
                let crate::view::caches::HistoryListRow::WorktreeUncommitted {
                    worktree_ix, ..
                } = plan.row_at(ix).unwrap()
                else {
                    panic!("worktree row should stay at the top")
                };
                let Loadable::Ready(dirty) = &history.active_repo().unwrap().worktree_dirty else {
                    unreachable!()
                };
                assert_eq!(dirty[worktree_ix].path, PathBuf::from("/wt/b"));
                assert_eq!(offset + height * ix as f32, px(-7.0));
            });
        });
    }
}

#[gpui::test]
fn activation_through_store_to_render_preserves_the_oldest_commit(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    let mut stream = String::new();
    for i in 0..6_000 {
        stream.push_str(&format!("commit refs/heads/main\nmark :{}\ncommitter Test <test@example.com> {} +0000\ndata 6\ncommit\n", i + 1, 1_600_000_000 + i));
        if i > 0 {
            stream.push_str(&format!("from :{i}\n"));
        }
        stream.push('\n');
    }
    stream.push_str("done\n");
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["fast-import", "--quiet", "--done"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), stream.as_bytes()).unwrap();
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    let repo = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let HistoryReadResult::Page { page, snapshot } = repo
        .read_history(
            LogScope::AllBranches,
            None,
            &HistoryReadRequest::Page {
                limit: 6_000,
                cursor: None,
                snapshot: None,
            },
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap()
    else {
        panic!("initial page")
    };
    let (view, cx, mut state, store) = mount(cx, Arc::clone(&page));
    state.repos[0].spec = repo.spec().clone();
    state.repos[0].history_state.log_snapshot = snapshot;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    store.insert_repo_for_test(RepoId(1), repo);
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    scroll(cx, &view, None);
    let expected = top(cx, &view);
    for _ in 0..3 {
        activate_and_check(cx, &view, &store, &expected);
        assert!(
            matches!(&store.snapshot().repos[0].log, Loadable::Ready(current) if Arc::ptr_eq(current, &page))
        );
    }
}
