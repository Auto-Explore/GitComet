use super::*;

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

#[gpui::test]
fn history_branch_names_repositions_headers_and_separates_hover_regions(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(BlockingBackend));
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
            ..Default::default()
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
