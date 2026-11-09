use super::*;

/// Design geometry the radius has to sit inside: the column pitch and a 14px
/// half-row.
const COL_GAP: f32 = HISTORY_GRAPH_COL_GAP_PX;
const HALF_ROW: f32 = 14.0;
/// Half a 16px node icon.
const ICON_HALF: Pixels = px(8.0);

fn edge_x(width: Pixels) -> Pixels {
    graph_edge_x(
        px(HISTORY_GRAPH_MARGIN_X_PX),
        px(HISTORY_GRAPH_MARGIN_RIGHT_PX),
        width,
    )
}

/// Design-scale x of column `col` in a graph cell `width` wide.
fn col_x(col: usize, width: Pixels) -> Pixels {
    graph_col_x(
        col,
        px(HISTORY_GRAPH_MARGIN_X_PX),
        px(HISTORY_GRAPH_COL_GAP_PX),
        edge_x(width),
    )
}

/// Widening the lane pitch for author discs must not cut how many lanes the
/// widest column can separate; 14 was the capacity at the 16px pitch.
#[test]
fn the_widest_graph_column_separates_fourteen_lanes() {
    let width = px(crate::view::HISTORY_COL_GRAPH_MAX_PX);
    // Lanes past the edge pin to it, so only these keep the full pitch.
    let full_pitch = (0..64)
        .take_while(|&col| {
            px(HISTORY_GRAPH_MARGIN_X_PX + HISTORY_GRAPH_COL_GAP_PX * col as f32) <= edge_x(width)
        })
        .count();
    assert_eq!(full_pitch, 14);
}

/// In a column narrower than the graph, the node's natural column is outside
/// a cell that clips rather than overflows, so it would not be drawn at all.
/// It has to come back inside.
#[test]
fn a_pushed_out_node_stays_inside_a_column_too_narrow_for_it() {
    let clamped_width = px(crate::view::HISTORY_COL_GRAPH_MAX_PX);

    // 20 lanes want an x past the widest column the clamp allows.
    let offset = col_x(20, clamped_width);
    assert!(
        offset < clamped_width,
        "the node must stay inside the clipped cell, got {offset:?}"
    );
    assert_eq!(offset, clamped_width - px(HISTORY_GRAPH_MARGIN_RIGHT_PX));

    // A column dragged narrower than one margin still yields a drawable
    // offset rather than a negative one.
    let offset = col_x(3, px(4.0));
    assert!(offset >= px(0.0), "got {offset:?}");
}

/// The connector's far end is clamped like the node's, or the elbow -- whose
/// direction is the sign of `x_to - x_from` -- turns away from the lane and
/// leaves through the clipped edge, deleting the only thing joining the band
/// to the commit below.
#[test]
fn a_narrow_column_clamps_the_connector_as_well_as_the_node() {
    // Wide enough for both: the node sits right of its exit lane, so the
    // connector runs leftwards into it.
    let wide = px(400.0);
    let node_x = col_x(6, wide);
    let exit_x = col_x(2, wide);
    assert!(
        exit_x < node_x,
        "the connector should still run left, got {exit_x:?} vs {node_x:?}"
    );

    // Narrower than one margin: both ends land on the same drawable x, which
    // `paint_node_to_lane` renders as a straight drop rather than an elbow
    // aimed off-screen.
    let narrow = px(4.0);
    let node_x = col_x(6, narrow);
    let exit_x = col_x(2, narrow);
    assert_eq!(exit_x, node_x);
    assert!(same_x(exit_x, node_x), "must not turn at all");
}

/// Columns that fit keep their place; every one past the edge shares the
/// edge x, far enough in that a 16px icon there keeps 8px clear of the
/// message border.
#[test]
fn columns_past_the_edge_share_one_x_inside_the_cell() {
    let margin = px(HISTORY_GRAPH_MARGIN_X_PX);
    let gap = px(HISTORY_GRAPH_COL_GAP_PX);
    let width = px(crate::view::HISTORY_COL_GRAPH_PX);
    let edge = edge_x(width);

    let xs: Vec<_> = (0..20).map(|col| col_x(col, width)).collect();
    for (col, &x) in xs.iter().enumerate() {
        let natural = margin + gap * (col as f32);
        if natural <= edge {
            assert_eq!(x, natural, "column {col} fits and must not move");
        } else {
            assert_eq!(x, edge, "column {col} must pin to the edge");
        }
        assert!(
            x + ICON_HALF + px(8.0) <= width,
            "a 16px icon on column {col} crowds the message border"
        );
    }
    assert!(xs.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(xs.iter().filter(|&&x| x == edge).count() > 10);
}

/// At its minimum the column shows exactly one lane: every column lands on
/// column 0's x, with a 16px icon clear of both sides.
#[test]
fn the_minimum_graph_column_draws_one_lane() {
    let margin = px(HISTORY_GRAPH_MARGIN_X_PX);
    let width = px(crate::view::HISTORY_COL_GRAPH_MIN_PX);
    for col in 0..20 {
        assert_eq!(col_x(col, width), margin, "column {col}");
    }
    assert!(
        margin - ICON_HALF >= px(2.0),
        "a 16px icon touches the left edge"
    );
    assert!(
        margin + ICON_HALF + px(8.0) <= width,
        "a 16px icon crowds the message border"
    );
}

/// On the edge line the selected lane is painted last; lanes elsewhere keep
/// column order.
#[test]
fn the_edge_line_paints_the_selected_lane_last() {
    // (col, on_edge, selected), in the column order painted.
    let mut lanes = [
        (1, false, true),
        (2, false, false),
        (4, true, true),
        (5, true, false),
        (6, true, false),
    ];
    lanes.sort_by_key(|&(_, on_edge, selected)| edge_paint_last(on_edge, || selected));
    let order: Vec<_> = lanes.iter().map(|&(col, ..)| col).collect();
    assert_eq!(order, [1, 2, 5, 6, 4]);
}

/// Beside a node on the edge line the line takes the node's colour: in a
/// one-lane column that is every row, so the line matches each row's dot.
/// A node that still has a column of its own never recolours the edge.
#[test]
fn a_node_on_the_edge_line_colours_the_line_beside_it() {
    let takes = |node: usize, segment: usize, width: Pixels| {
        edge_takes_node_colour(col_x(node, width), col_x(segment, width), edge_x(width))
    };

    let one_lane = px(crate::view::HISTORY_COL_GRAPH_MIN_PX);
    for node in 0..20 {
        for segment in 0..20 {
            assert!(
                takes(node, segment, one_lane),
                "node {node}, segment {segment}"
            );
        }
    }

    // Four lanes' room: column 6 is pinned, column 1 is not.
    let wide = px(80.0);
    assert!(takes(6, 8, wide), "a pinned node colours the edge line");
    assert!(!takes(6, 1, wide), "but not a lane that has its own column");
    assert!(
        !takes(1, 8, wide),
        "an on-screen node leaves the edge line alone"
    );
}

/// gpui snaps the quads that draw straight runs but not the paths that draw
/// elbows, so an off-grid lane drew 4px solid in one row and 3.2px
/// antialiased in the next. Every lane edge has to land on a device pixel.
#[test]
fn lanes_sit_on_the_device_pixel_grid() {
    let whole = |value: f32| (value - value.round()).abs() < 1e-3;
    for scale_factor in [1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
        let device = |value: Pixels| f32::from(value) * scale_factor;
        let from_device = |value: f32| px(value / scale_factor);
        for &percent in ui_scale::UI_SCALE_PRESETS {
            let grid = GraphGrid::for_scale(scale_factor, ui_scale::UiScale::from_percent(percent));
            let label = format!("scale {scale_factor}, zoom {percent}%");

            let stroke = device(grid.stroke_width);
            let raw = LANE_STROKE_PX * percent as f32 / 100.0 * scale_factor;
            assert!(
                whole(stroke) && stroke >= 2.0 && (stroke.round() as u32).is_multiple_of(2),
                "{label}: stroke {stroke} is not an even device width"
            );
            assert!(
                (stroke - raw).abs() <= 1.0,
                "{label}: stroke {stroke} for {raw}"
            );

            // Graph cells are laid out on whole device pixels.
            for width in [41.0, 160.0, 333.0] {
                let edge = grid.edge_x(from_device(width * scale_factor));
                for col in 0..24 {
                    let x = device(grid.col_x(col, edge));
                    assert!(
                        whole(x - stroke / 2.0) && whole(x + stroke / 2.0),
                        "{label}: column {col} at {x} with stroke {stroke}"
                    );
                }
            }

            for height in [37.0, 56.0, 75.0] {
                let bounds = Bounds::new(
                    point(px(0.0), from_device(height * 3.0 + 7.0)),
                    size(px(100.0), from_device(height)),
                );
                let (top, center, bottom) = grid.row_ys(bounds);
                for y in [top, center, bottom] {
                    assert!(whole(device(y)), "{label}: row y {} off grid", device(y));
                }
            }
        }
    }
    // Today's straight-lane weight: what a quad snapped to 1.6px drew.
    for (scale_factor, expected) in [(1.0, 2.0), (1.5, 2.0), (2.0, 4.0), (3.0, 4.0)] {
        let grid = GraphGrid::for_scale(scale_factor, ui_scale::UiScale::from_percent(100));
        assert_eq!(f32::from(grid.stroke_width) * scale_factor, expected);
    }
}

#[test]
fn elbow_radius_fits_a_one_column_jog_at_normal_scale() {
    let r = elbow_radius(px(HISTORY_GRAPH_ELBOW_RADIUS_PX), px(COL_GAP), px(HALF_ROW));
    // Neither clamp binds, so the corner keeps its designed radius and
    // leaves straight runs on both sides of the turn.
    assert_eq!(r, px(HISTORY_GRAPH_ELBOW_RADIUS_PX));
    assert!(r < px(COL_GAP));
    assert!(r < px(HALF_ROW));
}

#[test]
fn elbow_radius_clamps_to_a_short_horizontal_run() {
    let r = elbow_radius(px(HISTORY_GRAPH_ELBOW_RADIUS_PX), px(2.0), px(HALF_ROW));
    assert_eq!(r, px(2.0));
}

#[test]
fn elbow_radius_clamps_to_a_short_vertical_run() {
    let r = elbow_radius(px(HISTORY_GRAPH_ELBOW_RADIUS_PX), px(COL_GAP), px(3.0));
    assert_eq!(r, px(3.0));
}

#[test]
fn elbow_radius_is_direction_agnostic_and_never_negative() {
    let right = elbow_radius(px(HISTORY_GRAPH_ELBOW_RADIUS_PX), px(COL_GAP), px(HALF_ROW));
    let left = elbow_radius(
        px(HISTORY_GRAPH_ELBOW_RADIUS_PX),
        px(-COL_GAP),
        px(HALF_ROW),
    );
    assert_eq!(right, left);

    // A degenerate row would otherwise produce a corner bulging the wrong way.
    assert_eq!(
        elbow_radius(px(HISTORY_GRAPH_ELBOW_RADIUS_PX), px(COL_GAP), px(-1.0)),
        px(0.0)
    );
}
