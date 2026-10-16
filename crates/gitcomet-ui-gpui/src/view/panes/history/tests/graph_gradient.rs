use super::*;
use gitcomet_core::domain::{FileStatus, FileStatusKind, RepoStatus, WorktreeDirtySummary};

#[derive(Debug, PartialEq)]
struct GraphQuad {
    bounds: Bounds<gpui::ScaledPixels>,
    background: gpui::Background,
    border_color: gpui::Background,
    corner_radii: gpui::Corners<gpui::ScaledPixels>,
    border_widths: gpui::Edges<gpui::ScaledPixels>,
}

struct GraphPaint {
    quads: Vec<GraphQuad>,
    fade_opacities: Vec<f32>,
}

fn graph_paint(cx: &mut gpui::VisualTestContext, area: Bounds<Pixels>) -> GraphPaint {
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        let scale = window.scale_factor();
        let mut paint = GraphPaint {
            quads: Vec::new(),
            fade_opacities: Vec::new(),
        };
        for quad in window.painted_quads() {
            let center = quad.bounds.center();
            let center = point(px(center.x.0 / scale), px(center.y.0 / scale));
            if !area.contains(&center) {
                continue;
            }
            if let gpui::BackgroundKind::LinearGradient { stops, .. } = quad.background.kind() {
                let start: gpui::Hsla = stops[0].color.into();
                let end: gpui::Hsla = stops[1].color.into();
                assert_eq!(start.alpha, 0.0);
                paint.fade_opacities.push(end.alpha);
            } else {
                paint.quads.push(GraphQuad {
                    bounds: quad.bounds,
                    background: quad.background,
                    border_color: quad.border_color,
                    corner_radii: quad.corner_radii,
                    border_widths: quad.border_widths,
                });
            }
        }
        paint
    })
}

fn assert_graph_gradient_changes_only_the_fade(cx: &mut gpui::TestAppContext, indexed: bool) {
    cx.update(crate::appearance::pin_compact_for_test);
    let (index, commits) = refresh::indexed_fixture(6);
    let (view, cx, mut state, store) =
        refresh::mount(cx, Arc::new(log_page(commits.clone(), None)));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    let repo = &mut state.repos[0];
    repo.status = Loadable::Ready(Arc::new(RepoStatus {
        unstaged: Arc::new(vec![FileStatus {
            path: PathBuf::from("changed.txt"),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        ..Default::default()
    }));
    repo.status_rev += 1;
    repo.worktree_dirty = Loadable::Ready(Arc::new(vec![WorktreeDirtySummary {
        path: PathBuf::from("/tmp/gradient-worktree"),
        head: Some(commits[1].id.clone()),
        branch: Some("side".into()),
        detached: false,
        added: 1,
        modified: 0,
        deleted: 0,
        staged: Vec::new(),
        unstaged: Vec::new(),
        line_stats: Default::default(),
    }]));
    repo.worktree_dirty_rev += 1;
    if indexed {
        install_index(&mut state, index.clone());
        let history = &mut state.repos[0].history_state.indexed;
        history.range_index = Some(index.clone());
        history.ranges.insert(
            0,
            Arc::new(gitcomet_core::history_index::HistoryRange {
                snapshot: index.snapshot.clone(),
                start: 0,
                commits,
            }),
        );
    }
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    wait_until(cx, "gradient fixture rows", |cx| {
        cx.debug_bounds("history_row_1").is_some()
            && (!indexed || cx.debug_bounds("indexed_history_viewport").is_some())
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let (graph_rows, indexed_window) = cx.update(|_, app| {
        history.update(app, |history, cx| {
            history.set_history_show_graph_gradient(true, cx);
            assert!(history.ensure_history_worktree_summary_cache().0);
            let plan = if indexed {
                history.indexed.plan.clone()
            } else {
                history.ensure_history_list_plan()
            };
            assert!(matches!(
                plan.row_at(0),
                Some(crate::view::caches::HistoryListRow::WorkingTreeSummary)
            ));
            assert!(matches!(
                plan.row_at(2),
                Some(crate::view::caches::HistoryListRow::WorktreeUncommitted { .. })
            ));
            let graph_rows = if indexed {
                history
                    .indexed
                    .window
                    .as_ref()
                    .unwrap()
                    .cache
                    .base
                    .graph_rows
                    .clone()
            } else {
                history
                    .history_cache
                    .as_ref()
                    .unwrap()
                    .base
                    .graph_rows
                    .clone()
            };
            (graph_rows, history.indexed.window.clone())
        })
    });
    cx.run_until_parked();
    let row = cx.debug_bounds("history_row_1").unwrap();
    let graph = cx.debug_bounds("history_graph_header_cell").unwrap();
    let area = Bounds::new(
        point(graph.left(), row.top() - row.size.height),
        size(
            graph.size.width + px(HISTORY_MESSAGE_BORDER_W_PX),
            row.size.height * 8.0,
        ),
    );
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    cx.update(|window, _| window.refresh());
    let baseline = graph_paint(cx, area);
    assert_eq!(baseline.fade_opacities, vec![0.10; 8]);
    assert!(
        baseline
            .quads
            .iter()
            .any(|quad| quad.corner_radii.top_left.0 > 0.0)
    );

    for (enabled, opacity, expected) in [
        (false, 0.10, 0.0),
        (true, 0.10, 0.10),
        (false, 0.35, 0.0),
        (true, 0.35, 0.35),
        (true, 0.0, 0.0),
        (true, 0.10, 0.10),
    ] {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.set_history_show_graph_gradient(enabled, cx)
            });
            history.update(app, |history, cx| {
                if history.theme.graph_fade_opacity != opacity {
                    history.set_theme(
                        AppTheme {
                            graph_fade_opacity: opacity,
                            ..history.theme
                        },
                        cx,
                    );
                }
            });
        });
        let paint = graph_paint(cx, area);
        assert_eq!(
            paint.fade_opacities,
            if expected == 0.0 {
                Vec::new()
            } else {
                vec![expected; 8]
            }
        );
        let mut remaining = paint.quads;
        assert_eq!(remaining.len(), baseline.quads.len());
        for expected in &baseline.quads {
            let index = remaining
                .iter()
                .position(|quad| quad == expected)
                .unwrap_or_else(|| panic!("unchanged graph primitive missing: {expected:?}"));
            remaining.swap_remove(index);
        }
        cx.update(|_, app| {
            let history = history.read(app);
            if let Some(window) = &indexed_window {
                assert!(std::rc::Rc::ptr_eq(
                    window,
                    history.indexed.window.as_ref().unwrap()
                ));
            } else {
                assert!(Arc::ptr_eq(
                    &graph_rows,
                    &history.history_cache.as_ref().unwrap().base.graph_rows
                ));
            }
            assert_eq!(
                history.active_repo().unwrap().history_state.history_scope,
                LogScope::AllBranches
            );
        });
    }
}

#[gpui::test]
fn graph_gradient_toggle_repaints_cached_rows_without_changing_the_graph(
    cx: &mut gpui::TestAppContext,
) {
    assert_graph_gradient_changes_only_the_fade(cx, false);
}

#[gpui::test]
fn indexed_graph_gradient_toggle_repaints_cached_rows_without_changing_the_graph(
    cx: &mut gpui::TestAppContext,
) {
    assert_graph_gradient_changes_only_the_fade(cx, true);
}
