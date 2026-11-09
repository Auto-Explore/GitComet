use super::*;

#[gpui::test]
fn straight_connectors_and_node_exits_do_not_tessellate(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::history_perf::{self, Work};
    let cx = cx.add_empty_window();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let _capture = history_perf::capture();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(
                gpui::AvailableSpace::Definite(px(120.0)),
                gpui::AvailableSpace::Definite(px(80.0)),
            ),
            |_, _| {
                gpui::canvas(
                    |_, _, _| (),
                    move |_, (), window, _| {
                        let grid = GraphGrid::for_scale(
                            window.scale_factor(),
                            ui_scale::UiScale::from_percent((scale * 100.0) as u32),
                        );
                        let p = |x, y| point(grid.snap(px(x * scale)), grid.snap(px(y * scale)));
                        let stroke = grid.stroke_width;
                        let colour = gpui::rgb(0xabcdef);
                        // Both fork directions, both vertical directions and the node exit.
                        for (from, to) in [
                            (p(10.0, 20.0), p(30.0, 20.0)),
                            (p(30.0, 25.0), p(10.0, 25.0)),
                            (p(10.0, 10.0), p(10.0, 30.0)),
                            (p(20.0, 30.0), p(20.0, 10.0)),
                        ] {
                            paint_straight_connector(from, to, stroke, colour, window);
                        }
                        paint_node_to_lane(
                            px(0.0),
                            grid.snap(px(40.0 * scale)),
                            grid.snap(px(40.0 * scale)),
                            grid.snap(px(10.0 * scale)),
                            grid.snap(px(30.0 * scale)),
                            px(4.0 * scale),
                            stroke,
                            colour,
                            window,
                        );
                        // Degenerate strokes contribute no geometry.
                        paint_straight_connector(p(5.0, 5.0), p(5.0, 5.0), px(1.6), colour, window);
                    },
                )
                .w(px(120.0))
                .h(px(80.0))
            },
        );
        assert_eq!(history_perf::count(Work::PaintPath), 0, "scale={scale}");
        assert_eq!(
            history_perf::count(Work::PaintSegmentQuad),
            5,
            "scale={scale}"
        );
    }
}

#[gpui::test]
fn connectors_off_the_pixel_grid_or_at_a_mask_edge_stay_paths(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::history_perf::{self, Work};
    let cx = cx.add_empty_window();
    let canvas = Bounds::new(point(px(0.0), px(0.0)), size(px(60.0), px(40.0)));
    let unmasked = gpui::ContentMask {
        bounds: canvas,
        ..Default::default()
    };
    let clipped = gpui::ContentMask {
        bounds: Bounds::new(canvas.origin, size(px(20.0), px(40.0))),
        ..Default::default()
    };
    let faded = gpui::ContentMask {
        bounds: canvas,
        fade_out: gpui::Edges {
            right: px(8.0),
            ..Default::default()
        },
    };
    // A 2 px stroke at whole pixels has whole-pixel edges.
    for (case, mask, from_x, quads) in [
        ("aligned", unmasked, 10.0, 1),
        ("fractional", unmasked, 10.3, 0),
        ("clipped", clipped, 10.0, 0),
        ("faded", faded, 10.0, 0),
    ] {
        let _capture = history_perf::capture();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(
                gpui::AvailableSpace::Definite(px(60.0)),
                gpui::AvailableSpace::Definite(px(40.0)),
            ),
            |_, _| {
                gpui::canvas(
                    |_, _, _| (),
                    move |_, (), window, _| {
                        window.with_content_mask(Some(mask), |window| {
                            paint_straight_connector(
                                point(px(from_x), px(20.0)),
                                point(px(from_x + 20.0), px(20.0)),
                                px(2.0),
                                gpui::rgb(0xabcdef),
                                window,
                            );
                        });
                    },
                )
                .w(px(60.0))
                .h(px(40.0))
            },
        );
        assert_eq!(history_perf::count(Work::PaintSegmentQuad), quads, "{case}");
        assert_eq!(history_perf::count(Work::PaintPath), 1 - quads, "{case}");
    }
}

#[gpui::test]
fn nearly_collinear_connectors_keep_their_endpoints(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::history_perf::{self, Work};
    let cx = cx.add_empty_window();
    let _capture = history_perf::capture();
    cx.draw(
        point(px(0.0), px(0.0)),
        size(
            gpui::AvailableSpace::Definite(px(60.0)),
            gpui::AvailableSpace::Definite(px(40.0)),
        ),
        |_, _| {
            gpui::canvas(
                |_, _, _| (),
                |_, (), window, _| {
                    paint_node_to_lane(
                        px(0.0),
                        px(20.0),
                        px(20.25),
                        px(10.0),
                        px(30.0),
                        px(4.0),
                        px(1.6),
                        gpui::rgb(0xabcdef),
                        window,
                    );
                },
            )
            .w(px(60.0))
            .h(px(40.0))
        },
    );
    assert_eq!(history_perf::count(Work::PaintPath), 1);
    assert_eq!(history_perf::count(Work::PaintSegmentQuad), 0);
}

#[gpui::test]
fn indexed_history_actual_paint_paths_are_bounded_by_displayed_columns(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::history_perf::{self, Work};
    let cx = cx.add_empty_window();
    for pixels in [32.0, 80.0, 240.0] {
        for band in [false, true] {
            let mut counts = Vec::new();
            for width in [64, 512, 5261] {
                let row = history_graph::GraphRow {
                    lanes_now: (0..width)
                        .map(|col| history_graph::LanePaint::lane((col % 12) as u8, true, false))
                        .collect(),
                    lanes_next: (0..width)
                        .map(|col| history_graph::LanePaint::lane((col % 12) as u8, false, false))
                        .collect(),
                    joins_in: Default::default(),
                    edges_out: Default::default(),
                    node_col: (width - 1) as u16,
                    node_color_ix: 0,
                    is_merge: false,
                    from_node_cols: Default::default(),
                };
                let _capture = history_perf::capture();
                cx.draw(
                    point(px(0.0), px(0.0)),
                    size(
                        gpui::AvailableSpace::Definite(px(pixels)),
                        gpui::AvailableSpace::Definite(px(28.0)),
                    ),
                    |_, _| {
                        gpui::canvas(
                            |_, _, _| (),
                            move |bounds, (), window, app| {
                                let theme = AppTheme::gitcomet_dark();
                                let background = gpui::rgba(0x202020ff);
                                let selected = Some(SelectedLane::span(1, 0, 100));
                                if band {
                                    paint_history_graph_band(
                                        theme,
                                        &row.lanes_now,
                                        10,
                                        None,
                                        true,
                                        selected,
                                        BandNodePaint {
                                            col: row.node_col,
                                            color: lane_wash_color(
                                                theme,
                                                row.node_color_ix,
                                                10,
                                                selected,
                                            ),
                                            exit_col: None,
                                        },
                                        false,
                                        theme.graph_fade_opacity,
                                        background,
                                        0.0,
                                        bounds,
                                        window,
                                        app,
                                    );
                                } else {
                                    paint_history_graph(
                                        theme,
                                        &row,
                                        10,
                                        None,
                                        true,
                                        false,
                                        selected,
                                        history_canvas::HistoryRowVisual::new(
                                            theme, &row, 10, selected, false, background,
                                        ),
                                        &HistoryAuthorVm::new("Ada Lovelace".into()),
                                        1.0,
                                        0.0,
                                        bounds,
                                        GraphGrid::new(
                                            window,
                                            ui_scale::UiScale::from_window(window),
                                        ),
                                        graph_node_bounds(
                                            &row,
                                            false,
                                            1.0,
                                            bounds,
                                            GraphGrid::new(
                                                window,
                                                ui_scale::UiScale::from_window(window),
                                            ),
                                            ui_scale::UiScale::from_window(window),
                                        ),
                                        window,
                                        app,
                                    );
                                }
                            },
                        )
                        .w(px(pixels))
                        .h(px(28.0))
                    },
                );
                // Straight lanes and the node are quads.
                assert_eq!(
                    history_perf::count(Work::PaintPath),
                    0,
                    "pixels={pixels} band={band} width={width}"
                );
                counts.push(history_perf::count(Work::PaintSegmentQuad));
            }
            assert!(counts[0] > 0);
            assert!(
                counts.iter().all(|count| *count == counts[0]),
                "pixels={pixels} band={band}: {counts:?}"
            );
            assert!(counts[0] <= 40, "bounded displayed geometry: {counts:?}");
        }
    }
}

#[test]
fn indexed_history_coincident_paths_keep_last_winning_color_in_each_pass() {
    for lanes in [64usize, 512, 5261] {
        for displayed in [1usize, 8, 35] {
            let items = (0..lanes).map(|col| (col, col as u8 % 8));
            let geometry = |(col, _): (usize, u8)| col.min(displayed - 1);
            let selected = |(col, color)| col >= displayed - 1 && color == 3;
            let winners = coalesced(items.clone(), geometry, selected);
            let mut original: Vec<_> = items.collect();
            original.sort_by_key(|item| selected(*item));
            let mut expected = rustc_hash::FxHashMap::default();
            for item in original {
                expected.insert(geometry(item), item);
            }
            assert_eq!(winners.len(), displayed);
            for item in winners {
                assert_eq!(Some(&item), expected.get(&geometry(item)));
            }
        }
    }
}
