use super::*;
use std::rc::Rc;

/// Within a device pixel of the design size and even, so the node's edges
/// land on pixels around its lane's centre line.
fn assert_even_device_diameter(node: Bounds<Pixels>, design: Pixels, device_scale: f32) {
    assert_eq!(node.size.width, node.size.height);
    let half = f32::from(node.size.width) * device_scale / 2.0;
    assert!((half - half.round()).abs() < 1e-3, "{half} half device px");
    assert!(
        (half * 2.0 - f32::from(design) * device_scale).abs() <= 1.0,
        "{node:?} for {design:?} at {device_scale}x"
    );
}

type GraphPaint = Rc<dyn Fn(&mut Window, &mut App)>;

struct GraphPaintView(GraphPaint);

impl gpui::Render for GraphPaintView {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let paint = self.0.clone();
        gpui::canvas(|_, _, _| (), move |_, (), window, cx| paint(window, cx)).size_full()
    }
}

/// Each node is one round quad of even device size, centred on its lane.
#[gpui::test]
fn history_nodes_paint_as_round_quads_centred_on_their_lane(cx: &mut gpui::TestAppContext) {
    use crate::appearance::{Appearance, UiDensity};
    use gitcomet_core::history_perf::{self, Work};
    let (view, cx) = cx.add_window_view(|_, _| GraphPaintView(Rc::new(|_, _| {})));
    for (device_scale, ring) in [(1.0, 2.0), (1.25, 2.0), (1.5, 2.0), (2.0, 3.0)] {
        for (ui_font_size_px, density) in [14, 15]
            .into_iter()
            .flat_map(|font| UiDensity::ALL.map(|density| (font, density)))
        {
            for (is_merge, reveal) in [(false, 1.0), (false, 0.0), (true, 1.0)] {
                let case = format!(
                    "{device_scale}x font={ui_font_size_px} {density:?} merge={is_merge} reveal={reveal}"
                );
                let _capture = history_perf::capture();
                let quads = cx.update(|window, app| {
                    window.set_scale_factor(device_scale);
                    view.update(app, |view, cx| {
                        view.0 = Rc::new(move |window, app| {
                            let mut theme = AppTheme::gitcomet_dark();
                            theme.metrics = Appearance {
                                density,
                                ui_font_size_px,
                                ..Appearance::default()
                            };
                            let scale = ui_scale::UiScale::from_window(window)
                                .with_appearance(theme.metrics);
                            let lanes_now = [history_graph::LanePaint::lane(0, true, false)];
                            let lanes_next = [history_graph::LanePaint::lane(0, false, false)];
                            let row = history_graph::GraphRow {
                                lanes_now: lanes_now.into_iter().collect(),
                                lanes_next: lanes_next.into_iter().collect(),
                                joins_in: Default::default(),
                                edges_out: Default::default(),
                                node_col: 0,
                                node_color_ix: 0,
                                is_merge,
                                from_node_cols: history_graph::from_node_cols_of(&lanes_next),
                            };
                            let cell = Bounds::new(
                                point(px(40.25), px(10.5)),
                                size(
                                    scale.px(HISTORY_COL_GRAPH_PX),
                                    super::super::history::history_row_height(scale),
                                ),
                            );
                            let grid = GraphGrid::new(window, scale);
                            paint_history_graph(
                                theme,
                                &row,
                                0,
                                None,
                                true,
                                false,
                                None,
                                history_canvas::HistoryRowVisual::new(
                                    theme,
                                    &row,
                                    0,
                                    None,
                                    false,
                                    theme.colors.surface.canvas,
                                ),
                                &HistoryAuthorVm::new("Ada Lovelace".into()),
                                reveal,
                                0.0,
                                cell,
                                grid,
                                graph_node_bounds(&row, false, reveal, cell, grid, scale),
                                window,
                                app,
                            );
                        });
                        cx.notify();
                    });
                    let _ = window.draw(app);
                    window.painted_quads()
                });
                assert_eq!(history_perf::count(Work::PaintPath), 0, "{case}");
                let (nodes, lanes): (Vec<&gpui::Quad>, Vec<&gpui::Quad>) = quads
                    .iter()
                    .partition(|quad| quad.corner_radii.top_left.0 > 0.0);
                assert_eq!(nodes.len(), 1, "{case}");
                let node = nodes[0].bounds;
                let diameter = node.size.width.0;
                assert_eq!(node.size.width, node.size.height, "{case}");
                let half = diameter / 2.0;
                assert!(
                    (half - half.round()).abs() < 1e-3,
                    "{case}: {diameter} device px"
                );
                assert!(
                    (nodes[0].corner_radii.top_left.0 * 2.0 - diameter).abs() < 1e-3,
                    "{case}: radius {:?} for {diameter}",
                    nodes[0].corner_radii,
                );
                let expected_ring = if reveal > 0.0 { ring } else { 0.0 };
                assert_eq!(nodes[0].border_widths.top.0, expected_ring, "{case}");
                assert_eq!(lanes.len(), 2, "{case}: incoming and continuing runs");
                for lane in lanes {
                    assert!(
                        (lane.bounds.center().x.0 - node.center().x.0).abs() < 1e-3,
                        "{case}: lane {:?} off node {node:?}",
                        lane.bounds
                    );
                }
            }
        }
    }
}

#[test]
fn author_nodes_fit_resized_graphs_at_fractional_scales() {
    use crate::appearance::{Appearance, UiDensity};

    let mut row = history_graph::GraphRow {
        lanes_now: Default::default(),
        lanes_next: Default::default(),
        joins_in: Default::default(),
        edges_out: Default::default(),
        node_col: 0,
        node_color_ix: 0,
        is_merge: false,
        from_node_cols: Default::default(),
    };
    for (density, author_size, icon_size) in [
        (UiDensity::Compact, 22.0, 18.0),
        (UiDensity::Comfortable, 24.0, 20.0),
        (UiDensity::Spacious, 26.0, 22.0),
    ] {
        for (percent, ui_font_size_px) in [80, 100, 125, 150, 200]
            .into_iter()
            .flat_map(|percent| [10, 14, 18, 24].map(|font| (percent, font)))
        {
            let scale = ui_scale::UiScale::from_percent(percent).with_appearance(Appearance {
                density,
                ui_font_size_px,
                ..Appearance::default()
            });
            for device_scale in [1.0, 1.25, 1.5, 2.0] {
                let grid = GraphGrid::for_scale(device_scale, scale);
                for width in [
                    HISTORY_COL_GRAPH_MIN_PX + (scale.appearance.ui_text(20.0) - 20.0).max(0.0),
                    HISTORY_COL_GRAPH_PX,
                    HISTORY_COL_GRAPH_MAX_PX,
                ] {
                    let cell = Bounds::new(
                        point(px(40.0), px(10.0)),
                        size(
                            scale.px(width),
                            super::super::history::history_row_height(scale),
                        ),
                    );
                    // Column 14 is past every width's full-pitch capacity.
                    for col in [0, 1, 14, 5000] {
                        row.node_col = col;
                        let node = graph_node_bounds(&row, false, 1.0, cell, grid, scale);
                        assert_even_device_diameter(
                            node,
                            scale.ui_text(20.0) + scale.px(author_size - 20.0),
                            device_scale,
                        );
                        assert!(node.left() >= cell.left());
                        assert!(node.right() <= cell.right());
                        assert!(node.top() >= cell.top());
                        assert!(node.bottom() <= cell.bottom());
                        if col >= 14 {
                            let edge = grid.snap(cell.left()) + grid.edge_x(cell.size.width);
                            assert!(
                                (node.center().x - edge).abs() < px(0.001),
                                "the node center must stay on the pinned edge"
                            );
                        }
                        assert!(grid.col_gap > node.size.width);
                    }
                    let stash = graph_node_bounds(&row, true, 1.0, cell, grid, scale);
                    assert_even_device_diameter(
                        stash,
                        scale.ui_text(16.0) + scale.px(icon_size - 16.0),
                        device_scale,
                    );
                    row.is_merge = true;
                    assert_eq!(
                        graph_node_bounds(&row, false, 1.0, cell, grid, scale),
                        stash
                    );
                    row.is_merge = false;
                }
            }
        }
    }
}
