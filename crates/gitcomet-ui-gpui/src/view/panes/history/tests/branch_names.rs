use super::*;

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

/// Inspect the production canvas output: a leader must use the node's actual
/// colour and facing edge, and initials must move out of the author column.
fn assert_ref_node_paint(
    cx: &mut gpui::VisualTestContext,
    mode: HistoryBranchNamesMode,
    graph_visible: bool,
) {
    let row = cx.debug_bounds("history_row_0").unwrap();
    let author = cx
        .debug_bounds("history_author_header_cell")
        .expect("visible author column");
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
        let scale = window.scale_factor();
        let quads = window.painted_quads();
        let at_row = |q: &&gpui::Quad| {
            (q.bounds.center().y.0 - f32::from(row.center().y) * scale).abs() < 0.6
        };
        let node = quads.iter().filter(at_row).find(|q| {
            q.corner_radii.top_left.0 > 0.0
                && !q.background.is_transparent()
                && (q.bounds.size.width.0 - 22.0 * scale).abs() < 0.1
                && (q.bounds.size.height.0 - 22.0 * scale).abs() < 0.1
        });
        assert_eq!(node.is_some(), graph_visible);
        if let Some(node) = node {
            let dots: Vec<_> = quads
                .iter()
                .filter(at_row)
                .filter(|q| {
                    q.bounds.size.width == q.bounds.size.height
                        && q.bounds.size.width.0 <= 1.5 * scale + 1.0
                        && q.background == node.border_color
                })
                .collect();
            assert!(!dots.is_empty(), "the ref group has a leader");
            match mode {
                HistoryBranchNamesMode::SeparateColumn => {
                    let nearest = dots.iter().map(|dot| dot.bounds.right()).max().unwrap();
                    assert!(
                        dots.iter()
                            .all(|dot| dot.bounds.right() <= node.bounds.left())
                    );
                    assert!((nearest.0 - node.bounds.left().0).abs() < 0.6);
                }
                HistoryBranchNamesMode::Inline => {
                    let nearest = dots.iter().map(|dot| dot.bounds.left()).min().unwrap();
                    assert!(
                        dots.iter()
                            .all(|dot| dot.bounds.left() >= node.bounds.right())
                    );
                    assert!((nearest.0 - node.bounds.right().0).abs() < 0.6);
                }
            }
            assert!(
                dots.iter().all(|dot| dot.order < node.order),
                "lane node must cover the leader"
            );
        }
        {
            let avatars = quads
                .iter()
                .filter(at_row)
                .filter(|q| {
                    q.bounds.left().0 >= f32::from(author.left()) * scale
                        && q.bounds.right().0 <= f32::from(author.right()) * scale
                        && (q.bounds.size.width.0 - 16.0 * scale).abs() < 0.1
                        && (q.bounds.size.height.0 - 16.0 * scale).abs() < 0.1
                })
                .count();
            assert_eq!(avatars, usize::from(!graph_visible));
        }
    });
}

/// Tests and benchmarks settle frames under the deterministic runtime, whose
/// clock only moves on demand, so a selection must not keep a frame pending
/// there. Pane animations follow the same runtime policy.
#[gpui::test]
fn history_selection_settles_frames_in_the_deterministic_runtime(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    let page = Arc::new(log_page(
        (0..5)
            .map(|ix| commit(&format!("c{ix}"), &[&format!("c{}", ix + 1)], "Same author"))
            .collect(),
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-author-deterministic"),
        },
    );
    // An opening repository's tab spinner requests frames of its own.
    repo.open = Loadable::Ready(());
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let mut state = AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_4").is_some()
    });
    // Other panes' spinners request frames too; count the history view's own.
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let _observer = cx.update(|_, app| {
        let notified = notified.clone();
        app.observe(&history, move |_, _| notified.set(notified.get() + 1))
    });
    let frame_notifies = |cx: &mut gpui::VisualTestContext| {
        notified.set(0);
        // One update per frame: gpui merges a view's notifies until a flush.
        for _ in 0..20 {
            cx.update(|window, app| {
                window.simulate_next_frame(app);
                let _ = window.draw(app);
            });
        }
        notified.get()
    };
    assert_eq!(frame_notifies(cx), 0);
    for selected in ["c2", "c3"] {
        state.repos[0].history_state.selected_commit = Some(CommitId(selected.into()));
        set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
        redraw(cx);
        assert_eq!(
            frame_notifies(cx),
            0,
            "selecting {selected} kept requesting history frames"
        );
    }
}

#[gpui::test]
fn history_repeated_author_dot_animates_with_cached_selection_frames(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    cx.update(crate::appearance::pin_compact_for_test);
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    let page = Arc::new(log_page(
        (0..5)
            .map(|ix| commit(&format!("c{ix}"), &[&format!("c{}", ix + 1)], "Same author"))
            .collect(),
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-author-animation"),
        },
    );
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    });
    ensure_history_cache_for_tests(cx, &view, state.clone());
    wait_until(cx, "author run", |cx| {
        cx.debug_bounds("history_row_4").is_some()
    });
    // Cached views may skip selector registration during animation-only frames.
    let base_row = cx.debug_bounds("history_row_2").unwrap();
    let graph_right = cx
        .debug_bounds("history_message_header_cell")
        .unwrap()
        .left();
    let node_width = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        let mut row = base_row;
        row.origin.y += row.size.height * (row_ix as f32 - 2.0);
        cx.update(|window, _| {
            let scale = window.scale_factor();
            // Cached frames replay their quads, so the scene holds the node a
            // reused row last painted; a progressing animation must replace it.
            let node = window
                .painted_quads()
                .into_iter()
                .find(|q| {
                    q.corner_radii.top_left.0 > 0.0
                        && !q.background.is_transparent()
                        && (q.bounds.center().y.0 / scale - f32::from(row.center().y)).abs() < 0.6
                        && q.bounds.right().0 / scale < f32::from(graph_right)
                        && q.bounds.size.width == q.bounds.size.height
                        && q.bounds.size.width.0 / scale >= 7.0
                })
                .expect("author dot/disc");
            node.bounds.size.width.0 / scale
        })
    };
    assert_eq!(node_width(cx, 2), 7.0);
    // Pane animations only run under the live runtime.
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let _cache_guard = crate::view::enable_stable_cached_views_for_test();
        let mut selected = (*state).clone();
        selected.repos[0].history_state.selected_commit = Some(CommitId("c2".into()));
        set_history_view_state_for_tests(cx, &view, Arc::new(selected.clone()));
        redraw(cx);
        assert_eq!(node_width(cx, 2), 7.0, "selection starts from the dot");
        let mut row_ix = 2;
        for (frame, elapsed) in [90, 90].into_iter().enumerate() {
            cx.executor().advance_clock(Duration::from_millis(elapsed));
            cx.update(|window, app| {
                window.simulate_next_frame(app);
            });
            redraw(cx);
            let width = node_width(cx, row_ix);
            assert!(width > 7.0 && width <= 22.0);
            if frame == 0 {
                selected.repos[0].worktree_status =
                    Loadable::Ready(Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: PathBuf::from("README.md"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]));
                selected.repos[0].worktree_status_rev += 1;
                set_history_view_state_for_tests(cx, &view, Arc::new(selected.clone()));
                redraw(cx);
                row_ix = 3;
                assert_eq!(
                    node_width(cx, row_ix),
                    width,
                    "inserting WIP must preserve the animation"
                );
            }
        }
        assert_eq!(node_width(cx, row_ix), 22.0);
        selected.repos[0].worktree_status = Loadable::Ready(Arc::new(Vec::new()));
        selected.repos[0].worktree_status_rev += 1;
        set_history_view_state_for_tests(cx, &view, Arc::new(selected));
        redraw(cx);
        assert_eq!(
            node_width(cx, 2),
            22.0,
            "removing WIP retains the selected node"
        );
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    // The live runtime builds the cache on a thread. The clock stands still
    // meanwhile, so a node that inherited a transition would stay mid-way.
    cx.executor().allow_parking();
    let show_repo = |cx: &mut gpui::VisualTestContext, state: AppState| {
        let repo_id = state.active_repo.unwrap();
        ensure_history_cache_for_tests(cx, &view, Arc::new(state));
        wait_until(cx, "repository history cache", |cx| {
            cx.update(|_, app| {
                history
                    .read(app)
                    .history_cache
                    .as_ref()
                    .is_some_and(|cache| cache.base.request.repo_id == repo_id)
            })
        });
        redraw(cx);
    };
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let mut other_repo = state.repos[0].clone();
        other_repo.id = RepoId(2);
        other_repo.spec.workdir = PathBuf::from("/tmp/history-author-other-worktree");
        show_repo(
            cx,
            AppState {
                repos: vec![other_repo],
                active_repo: Some(RepoId(2)),
                ..(*state).clone()
            },
        );
        assert_eq!(
            node_width(cx, 2),
            7.0,
            "another repository must not inherit selection for the same commit"
        );
        let mut returning = (*state).clone();
        returning.repos[0].history_state.selected_commit = Some(CommitId("c2".into()));
        show_repo(cx, returning);
        assert_eq!(node_width(cx, 2), 22.0);
        cx.update(|_, app| app.set_reduce_motion(true));
        set_history_view_state_for_tests(cx, &view, state);
        redraw(cx);
        assert_eq!(node_width(cx, 2), 7.0, "reduced motion settles immediately");
    });
}

#[gpui::test]
fn history_branch_names_repositions_headers_and_separates_hover_regions(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    cx.update(crate::appearance::pin_compact_for_test);
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    let page = Arc::new(log_page(
        vec![
            commit("tip", &["base"], "Tip message"),
            commit("base", &[], "Base message"),
        ],
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-inline-refs"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".into());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(
        (0..12)
            .map(|ix| gitcomet_core::domain::Tag {
                name: format!("release-{ix}"),
                target: CommitId("tip".into()),
            })
            .collect(),
    ));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    ensure_history_cache_for_tests(
        cx,
        &view,
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..AppState::test_default()
        }),
    );
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_1").is_some()
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let graph = cx.update(|_, app| {
        history.update(app, |history, cx| {
            history.history_col_branch = px(174.0);
            cx.notify();
            history
                .history_cache
                .as_ref()
                .unwrap()
                .base
                .graph_rows
                .clone()
        })
    });
    redraw(cx);
    assert_ref_node_paint(cx, HistoryBranchNamesMode::SeparateColumn, true);
    let original_left = cx.debug_bounds("history_ref_header_cell").unwrap().left();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_history_branch_names(HistoryBranchNamesMode::Inline, cx)
        })
    });

    for (show_graph, graph_width) in [(true, 80.0), (true, 28.0), (false, 28.0)] {
        cx.update(|_, app| {
            history.update(app, |history, cx| {
                history.history_col_graph = px(graph_width);
                history.set_history_column_preferences(show_graph, true, true, false, cx);
                cx.notify();
            })
        });
        redraw(cx);
        assert_ref_node_paint(cx, HistoryBranchNamesMode::Inline, show_graph);
        assert!(cx.debug_bounds("history_ref_header_cell").is_none());
        let message = cx.debug_bounds("history_message_header_cell").unwrap();
        let control = cx.debug_bounds("history_mode_header").unwrap();
        if show_graph {
            let graph_cell = cx.debug_bounds("history_graph_header_cell").unwrap();
            assert_eq!(graph_cell.left(), original_left);
            assert_eq!(graph_cell.right(), message.left());
            assert!(control.left() >= graph_cell.left());
            assert!(control.right() <= graph_cell.right());
        } else {
            assert!(cx.debug_bounds("history_graph_header_cell").is_none());
            assert_eq!(message.left(), original_left);
            assert!(control.left() >= message.left());
            assert!(control.right() < message.right());
        }
        crate::view::tooltip::clear_visible_tooltip_text_for_test();
        cx.simulate_mouse_move(control.center(), None, gpui::Modifiers::default());
        crate::view::test_support::wait_for_native_tooltip(cx);
        assert_eq!(
            crate::view::test_support::tooltip_text(cx, &view).as_deref(),
            Some("History mode: All branches")
        );
        cx.simulate_click(control.center(), gpui::Modifiers::default());
        redraw(cx);
        cx.update(|_, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::HistoryBranchFilter { repo_id: RepoId(1) })
            );
            let host = view.read(app).popover_host.clone();
            host.update(app, |host, cx| host.close_popover(cx));
        });
        redraw(cx);
        let row = cx.debug_bounds("history_row_0").unwrap();
        cx.simulate_mouse_move(
            point(message.left() + px(16.0), row.center().y),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        redraw(cx);
        assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
        assert!(cx.debug_bounds("commit_message_hover").is_none());
        cx.simulate_mouse_move(
            point(message.left() + message.size.width * 0.75, row.center().y),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        redraw(cx);
        assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
        assert!(cx.debug_bounds("commit_message_hover").is_some());
        cx.update(|_, app| view.update(app, |view, cx| view.dismiss_commit_message_hover(cx)));
        redraw(cx);
        // A row with no refs uses the beginning of the cell for its message.
        let base = cx.debug_bounds("history_row_1").unwrap();
        cx.simulate_mouse_move(
            point(message.left() + px(16.0), base.center().y),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        redraw(cx);
        assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
        assert!(cx.debug_bounds("commit_message_hover").is_some());
        cx.update(|_, app| view.update(app, |view, cx| view.dismiss_commit_message_hover(cx)));
    }

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_history_branch_names(HistoryBranchNamesMode::SeparateColumn, cx)
        })
    });
    redraw(cx);
    assert_eq!(
        cx.debug_bounds("history_ref_header_cell")
            .unwrap()
            .size
            .width,
        px(174.0)
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
    assert!(cx.debug_bounds("commit_message_hover").is_none());
    cx.update(|_, app| {
        let history = history.read(app);
        assert_eq!(history.history_ref_column_width(), px(174.0));
        assert!(Arc::ptr_eq(
            &graph,
            &history.history_cache.as_ref().unwrap().base.graph_rows
        ));
    });
}
