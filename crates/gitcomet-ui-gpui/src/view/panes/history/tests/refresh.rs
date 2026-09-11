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

fn indexed_fixture(
    count: usize,
) -> (
    gitcomet_core::history_index::HistoryIndexHandle,
    Vec<Commit>,
) {
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot(format!("fixture-{count}").into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap();
    let raw = |number: usize| {
        let mut id = [0u8; 20];
        id[..8].copy_from_slice(&(number as u64).to_be_bytes());
        id
    };
    let mut commits = Vec::new();
    for row in 0..count {
        let id = raw(count - row);
        let parents: Vec<_> = (row + 1 < count)
            .then(|| raw(count - row - 1))
            .into_iter()
            .collect();
        builder
            .push(&id, parents.iter().map(|id| id.as_slice()), 0, false)
            .unwrap();
        commits.push(Commit {
            id: CommitId(gitcomet_core::hex::encode(&id).into()),
            parent_ids: parents
                .iter()
                .map(|id| CommitId(gitcomet_core::hex::encode(id).into()))
                .collect(),
            author: "author".into(),
            summary: "commit".into(),
            time: SystemTime::UNIX_EPOCH,
        });
    }
    (builder.finish(&CancellationToken::new()).unwrap(), commits)
}

fn install_index(state: &mut AppState, index: gitcomet_core::history_index::HistoryIndexHandle) {
    let history = &mut state.repos[0].history_state;
    history.log_snapshot = Some(index.snapshot.clone());
    history.indexed.requested = Some(index.snapshot.clone());
    history.indexed.index = Some(index);
    history.indexed.rev += 1;
}

fn logical_top(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
) -> (usize, f64, usize) {
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.read(app);
        let scroll = history.scroll_interaction.borrow();
        let logical = scroll.logical.as_ref().unwrap();
        (logical.top, logical.within, logical.total)
    })
}

#[gpui::test]
fn indexed_history_real_wheel_and_thumb_keep_position_during_refresh(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = indexed_fixture(5000);
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits[..200].to_vec(),
            next_cursor: None,
        }),
    );
    install_index(&mut state, index.clone());
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    wait_until(cx, "indexed viewport", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
    });
    let bounds = cx.debug_bounds("indexed_history_viewport").unwrap();
    cx.update(|_, app| {
        let entity = view.read(app).main_pane.read(app).history_view.clone();
        entity.update(app, |history, _| {
            history.pending_history_reveal = Some(PendingHistoryReveal {
                repo_id: RepoId(1),
                commit_id: index.commit_id(0).unwrap(),
                fallback_scope: None,
                worktree_path: None,
            })
        });
    });
    for _ in 0..25 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-2000.25))),
            ..Default::default()
        });
        cx.run_until_parked();
    }
    let after_wheel = logical_top(cx, &view);
    assert!(after_wheel.0 > 1000);
    assert_eq!(after_wheel.2, 5000);
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .main_pane
                .read(app)
                .history_view
                .read(app)
                .pending_history_reveal
                .is_none()
        )
    });
    // A late block response hydrates the current viewport without touching it.
    let block = after_wheel.0 / 256 * 256;
    state.repos[0].history_state.indexed.range_index = Some(index.clone());
    state.repos[0].history_state.indexed.ranges.insert(
        block,
        Arc::new(gitcomet_core::history_index::HistoryRange {
            snapshot: index.snapshot.clone(),
            start: block,
            commits: commits[block..(block + 256).min(commits.len())].to_vec(),
        }),
    );
    state.repos[0].history_state.indexed.rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    cx.run_until_parked();
    assert_eq!(logical_top(cx, &view), after_wheel);

    // Drag the actual thumb using the production f64 logical metrics.
    let track = f32::from(bounds.size.height) - 8.0;
    let height = cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.read(app);
        let scroll = history.scroll_interaction.borrow();
        let logical = scroll.logical.as_ref().unwrap();
        (logical.position() / logical.max()) as f32
    });
    let thumb = point(
        bounds.right() - px(8.0),
        bounds.top() + px(4.0 + (track - 24.0) * height + 12.0),
    );
    cx.simulate_mouse_down(thumb, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        point(thumb.x, bounds.bottom() - px(40.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    let during = logical_top(cx, &view);
    assert!(during.0 > after_wheel.0);
    let (next, _) = indexed_fixture(5010);
    install_index(&mut state, next);
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    // Let the background graph complete while the mouse remains down.
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    assert_eq!(
        logical_top(cx, &view),
        during,
        "refresh must not resize a dragged history"
    );
    cx.simulate_mouse_up(
        point(thumb.x, bounds.bottom() - px(40.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    wait_until(cx, "new indexed snapshot", |cx| {
        logical_top(cx, &view).2 == 5010
    });
    let after = logical_top(cx, &view);
    assert_eq!(after.0, during.0 + 10, "the same commit remains at the top");
    assert!((after.1 - during.1).abs() < 0.001);

    // With no explicit selection, arrows start at HEAD even when another
    // branch has thousands of newer commits above it.
    let head = state.repos[0]
        .history_state
        .indexed
        .index
        .as_ref()
        .unwrap()
        .commit_id(4000)
        .unwrap();
    state.repos[0].head_branch = Loadable::Ready("main".to_owned());
    state.repos[0].head_branch_rev += 1;
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_owned(),
        target: head.clone(),
        upstream: None,
        divergence: None,
    }]));
    state.repos[0].branches_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state));
    wait_until(cx, "HEAD attribution", |cx| {
        cx.update(|_, app| {
            view.read(app)
                .main_pane
                .read(app)
                .history_view
                .read(app)
                .indexed
                .presentation
                .as_ref()
                .is_some_and(|shown| shown.head.as_deref() == Some(head.as_ref()))
        })
    });
    cx.update(|window, app| {
        let entity = view.read(app).main_pane.read(app).history_view.clone();
        let focus = entity.read(app).history_panel_focus_handle.clone();
        window.focus(&focus, app);
    });
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(logical_top(cx, &view).0, 4001);
}

#[gpui::test]
fn indexed_history_bootstrap_preserves_worktree_row_anchor(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = indexed_fixture(5000);
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits[..200].to_vec(),
            next_cursor: None,
        }),
    );
    state.repos[0].worktree_dirty = Loadable::Ready(Arc::new(vec![dirty(
        "/tmp/indexed-worktree",
        commits[50].id.as_ref(),
    )]));
    state.repos[0].worktree_dirty_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    scroll(cx, &view, Some(50));
    install_index(&mut state, index);
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state));
    wait_until(cx, "indexed viewport", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
    });
    assert_eq!(logical_top(cx, &view), (50, 7.0, 5001));
}

#[gpui::test]
fn indexed_history_handoff_never_replaces_visible_commits_with_placeholders(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = indexed_fixture(5000);
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits[..200].to_vec(),
            next_cursor: None,
        }),
    );
    scroll(cx, &view, Some(40));
    for (total, next) in [(5000, index), (5010, indexed_fixture(5010).0)] {
        install_index(&mut state, next);
        store.replace_snapshot_for_test(Arc::new(state.clone()));
        set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
        wait_until(cx, "hydrated atomic handoff", |cx| {
            cx.update(|_, app| {
                let history = view.read(app).main_pane.read(app).history_view.read(app);
                if history.indexed.presentation.is_none() {
                    return false;
                }
                let logical = history.scroll_interaction.borrow().logical.clone().unwrap();
                let window = history
                    .indexed
                    .window
                    .as_ref()
                    .expect("published index must include a window");
                for row in logical.visible_range() {
                    if let Some(crate::view::caches::HistoryListRow::Commit { visible_ix }) =
                        history.indexed.plan.row_at(row)
                    {
                        let ix = visible_ix - window.start;
                        assert!(
                            window.loaded[ix],
                            "visible commit {row} became a placeholder"
                        );
                        assert_eq!(window.cache.page.commits[ix].summary.as_ref(), "commit");
                    }
                }
                logical.total == total
            })
        });
    }
    assert_eq!(logical_top(cx, &view), (50, 7.0, 5010));
}

#[gpui::test]
fn indexed_history_skeleton_waits_300ms_and_hydrates_without_a_minimum_dwell(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = indexed_fixture(5000);
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits[..200].to_vec(),
            next_cursor: None,
        }),
    );
    install_index(&mut state, index.clone());
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    wait_until(cx, "indexed viewport", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
    });
    cx.update(|window, app| {
        let entity = view.read(app).main_pane.read(app).history_view.clone();
        entity.update(app, |history, cx| {
            let mut scroll = history.scroll_interaction.borrow_mut();
            let logical = scroll.logical.as_mut().unwrap();
            logical.set_position(650.0 * logical.height + 7.0);
            cx.notify();
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("history_skeleton_650").is_none());
    cx.executor().advance_clock(Duration::from_millis(299));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_skeleton_650").is_none());
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let placeholder = cx
        .debug_bounds("history_skeleton_650")
        .expect("skeleton deadline must repaint without more input");
    state.repos[0]
        .history_state
        .indexed
        .range_errors
        .insert(512, "read failed".to_owned());
    state.repos[0].history_state.indexed.rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    assert!(cx.debug_bounds("history_loading_error_650").is_some());
    assert!(cx.debug_bounds("history_skeleton_650").is_none());
    state.repos[0].history_state.indexed.range_errors.clear();
    let before = logical_top(cx, &view);
    state.repos[0].history_state.indexed.range_index = Some(index.clone());
    state.repos[0].history_state.indexed.ranges.insert(
        512,
        Arc::new(gitcomet_core::history_index::HistoryRange {
            snapshot: index.snapshot.clone(),
            start: 512,
            commits: commits[512..768].to_vec(),
        }),
    );
    state.repos[0].history_state.indexed.rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    wait_until(cx, "commit replaces skeleton immediately", |cx| {
        cx.debug_bounds("history_row_650").is_some()
    });
    let real = cx.debug_bounds("history_row_650").unwrap();
    assert_eq!(real.size.height, placeholder.size.height);
    assert_eq!(real.origin.y, placeholder.origin.y);
    assert!(cx.debug_bounds("history_skeleton_650").is_none());
    assert_eq!(logical_top(cx, &view), before);
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        cx.debug_bounds("history_skeleton_650").is_none(),
        "obsolete timer resurrected loading rows"
    );
}

#[gpui::test]
fn history_startup_keeps_recent_rows_interactive_with_a_capped_draggable_thumb(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = indexed_fixture(5000);
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits[..200].to_vec(),
            next_cursor: Some(LogCursor {
                last_seen: commits[199].id.clone(),
                resume_from: None,
                resume_token: None,
            }),
        }),
    );
    state.repos[0].history_state.indexed.loading = true;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    let first = cx
        .debug_bounds("history_row_0")
        .expect("first page is already interactive");
    let thumb = point(first.right() + px(8.0), first.top() + px(28.0));
    cx.simulate_mouse_down(thumb, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .main_pane
                .read(app)
                .history_view
                .read(app)
                .scroll_interaction
                .borrow()
                .dragging
        );
    });
    cx.simulate_mouse_move(
        point(thumb.x, thumb.y + px(100.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    let before = top(cx, &view);
    assert_ne!(before.0, commits[0].id);
    install_index(&mut state, index);
    state.repos[0].history_state.indexed.loading = false;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state));
    cx.run_until_parked();
    assert_eq!(
        top(cx, &view),
        before,
        "held thumb cannot publish a new extent"
    );
    cx.simulate_mouse_up(
        point(thumb.x, thumb.y + px(100.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    wait_until(cx, "full range after releasing thumb", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
    });
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.read(app);
        let scroll = history.scroll_interaction.borrow();
        let logical = scroll.logical.as_ref().unwrap();
        assert_eq!(
            history
                .indexed
                .presentation
                .as_ref()
                .unwrap()
                .graph
                .projection
                .commit_id(logical.top),
            Some(before.0.clone())
        );
    });
}

#[gpui::test]
fn history_initial_skeleton_is_decorative_and_old_query_timers_are_cancelled(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits(100),
            next_cursor: None,
        }),
    );
    state.repos[0].log = Loadable::Loading;
    state.repos[0].history_state.log = Loadable::Loading;
    state.repos[0].log_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    assert!(cx.debug_bounds("history_skeleton_0").is_none());
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_skeleton_0").is_some());
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.read(app);
        assert!(history.history_cache.is_none());
        assert!(
            history.scroll_interaction.borrow().logical.is_none(),
            "decorations must not invent an indexed extent"
        );
    });
    state.repos[0].history_state.history_author_filter = Some("another author".to_owned());
    state.repos[0].log_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
    assert!(
        cx.debug_bounds("history_skeleton_0").is_none(),
        "a new query starts its own grace period"
    );
    cx.executor().advance_clock(Duration::from_millis(299));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_skeleton_0").is_none());
    let page = Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    });
    state.repos[0].log = Loadable::Ready(page.clone());
    state.repos[0].history_state.log = Loadable::Ready(page);
    state.repos[0].log_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("history_skeleton_0").is_none());
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.read(app);
        assert!(
            !history
                .loading
                .status_visible(app.background_executor().now())
        );
    });
}

#[gpui::test]
fn graph_column_width_holds_through_bootstrap_and_indexed_publish(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (index, commits) = indexed_fixture(5000);
    let (view, cx, mut state, store) = mount(
        cx,
        Arc::new(LogPage {
            commits: commits[..200].to_vec(),
            next_cursor: None,
        }),
    );
    let graph_width = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            view.read(app)
                .main_pane
                .read(app)
                .history_view
                .read(app)
                .history_col_graph_design
        })
    };
    assert_eq!(
        graph_width(cx),
        HISTORY_COL_GRAPH_PX,
        "the bootstrap page must not resize the graph column"
    );
    install_index(&mut state, index);
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    set_history_view_state_for_tests(cx, &view, Arc::new(state));
    wait_until(cx, "indexed presentation", |cx| {
        cx.debug_bounds("indexed_history_viewport").is_some()
            && cx.update(|_, app| {
                view.read(app)
                    .main_pane
                    .read(app)
                    .history_view
                    .read(app)
                    .indexed
                    .presentation
                    .is_some()
            })
    });
    assert_eq!(
        graph_width(cx),
        HISTORY_COL_GRAPH_PX,
        "the indexed publish must not resize the graph column"
    );
}
