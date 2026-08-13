use super::*;
use gpui::{Bounds, Pixels, Window, fill, point, px, size};

pub(super) fn paint_history_graph(
    theme: AppTheme,
    row: &history_graph::GraphRow,
    connect_from_top_col: Option<usize>,
    is_stash_node: bool,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    use gpui::PathBuilder;

    if row.lanes_now.is_empty() {
        return;
    }

    let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
    let scaled_px = |value| px(value * design_scale_factor);
    let stroke_width = scaled_px(1.6);
    let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
    let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
    let node_radius = if row.is_merge {
        scaled_px(3.9)
    } else {
        scaled_px(3.4)
    };
    let node_corner_radius = scaled_px(2.0);

    let elbow_radius = scaled_px(HISTORY_GRAPH_ELBOW_RADIUS_PX);

    let y_top = bounds.top();
    let y_center = bounds.top() + bounds.size.height / 2.0;
    let y_bottom = bounds.bottom();

    let x_for_col = |col: usize| margin_x + col_gap * (col as f32);
    let left = bounds.left();

    let node_x = x_for_col(usize::from(row.node_col));

    // Whether column `col` draws a vertical down from the top edge of this row.
    let has_incoming_vertical = |col: usize| {
        row.lanes_now
            .get(col)
            .is_some_and(|lane| lane.is_active() && lane.incoming())
            || connect_from_top_col == Some(col)
    };
    // A join whose source column also has an incoming vertical is drawn as one
    // continuous elbow, so the plain vertical pass must not draw it twice.
    let joins_out_of = |col: usize| {
        row.joins_in
            .iter()
            .any(|edge| usize::from(edge.from_col) == col && edge.from_col != edge.to_col)
    };

    // Incoming vertical segments.
    for (col, lane) in row.lanes_now.iter().enumerate() {
        if !lane.is_active() {
            continue;
        }
        if !(lane.incoming() || connect_from_top_col == Some(col)) {
            continue;
        }
        if joins_out_of(col) {
            continue;
        }
        let x = x_for_col(col);
        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(left + x, y_top));
        path.line_to(point(left + x, y_center));
        if let Ok(p) = path.build() {
            window.paint_path(p, history_graph::lane_color(theme, lane.color_ix));
        }
    }

    // Incoming join edges into the node (used both for merge commits and fork points).
    for edge in row.joins_in.iter() {
        if edge.from_col == edge.to_col {
            continue;
        }
        let from = usize::from(edge.from_col);
        let color = history_graph::lane_color(theme, edge.color_ix);
        if has_incoming_vertical(from) {
            paint_lane_to_node(
                left,
                x_for_col(from),
                x_for_col(usize::from(edge.to_col)),
                y_top,
                y_center,
                elbow_radius,
                stroke_width,
                color,
                window,
            );
        } else {
            // A fork whisker has nothing above it, so it stays a bare stub.
            let mut path = PathBuilder::stroke(stroke_width);
            path.move_to(point(left + x_for_col(from), y_center));
            path.line_to(point(left + x_for_col(usize::from(edge.to_col)), y_center));
            if let Ok(p) = path.build() {
                window.paint_path(p, color);
            }
        }
    }

    // Continuations from current row to next row.
    for (out_col, lane) in row.lanes_next.iter().enumerate() {
        if !lane.is_active() {
            continue;
        }
        let x_out = x_for_col(out_col);
        let color = history_graph::lane_color(theme, lane.color_ix);
        if lane.starts_at_node() {
            paint_node_to_lane(
                left,
                node_x,
                x_out,
                y_center,
                y_bottom,
                elbow_radius,
                stroke_width,
                color,
                window,
            );
        } else {
            let mut path = PathBuilder::stroke(stroke_width);
            path.move_to(point(left + x_out, y_center));
            path.line_to(point(left + x_out, y_bottom));
            if let Ok(p) = path.build() {
                window.paint_path(p, color);
            }
        }
    }

    // Additional merge edges from the node into lanes that were re-targeted to secondary parents.
    for edge in row.edges_out.iter() {
        if edge.from_col == edge.to_col {
            continue;
        }
        paint_node_to_lane(
            left,
            node_x,
            x_for_col(usize::from(edge.to_col)),
            y_center,
            y_bottom,
            elbow_radius,
            stroke_width,
            history_graph::lane_color(theme, edge.color_ix),
            window,
        );
    }

    let node_color = history_graph::lane_color(theme, row.node_color_ix);

    // Within one paint layer gpui draws all quads before any path, so the
    // node (a quad) would sit under the lane lines no matter the call
    // order. A nested layer gives the node a strictly higher draw order;
    // its bounds are generous enough for the stash node's handle.
    let node_layer_half = scaled_px(10.0);
    let node_layer_bounds = Bounds::new(
        point(
            bounds.left() + node_x - node_layer_half,
            y_center - node_layer_half,
        ),
        size(node_layer_half * 2.0, node_layer_half * 2.0),
    );
    window.paint_layer(node_layer_bounds, |window| {
        if is_stash_node {
            paint_stash_node(
                bounds.left() + node_x,
                y_center,
                theme.colors.window_bg,
                node_color,
                window,
            );
        } else {
            paint_commit_node(
                bounds.left() + node_x,
                y_center,
                node_radius,
                node_corner_radius,
                node_color,
                window,
            );
        }
    });
}

/// Control-point ratio for approximating a circular quarter-arc with a cubic
/// Bezier: `4/3 * (sqrt(2) - 1)`.
const ELBOW_K: f32 = 0.552_284_7;

/// Radius actually usable for a corner turning `dx` horizontally with `vertical`
/// pixels of room. Clamped so a short jog or a small UI scale degrades into a
/// tighter corner instead of overshooting past its own endpoints.
fn elbow_radius(preferred: Pixels, dx: Pixels, vertical: Pixels) -> Pixels {
    preferred.min(dx.abs()).min(vertical.max(px(0.0)))
}

/// Leaves the node horizontally, turns through a rounded corner, then runs
/// straight down to the bottom of the row.
#[allow(clippy::too_many_arguments)]
fn paint_node_to_lane(
    left: Pixels,
    x_from: Pixels,
    x_to: Pixels,
    y_center: Pixels,
    y_bottom: Pixels,
    preferred_radius: Pixels,
    stroke_width: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
) {
    use gpui::PathBuilder;

    let mut path = PathBuilder::stroke(stroke_width);
    path.move_to(point(left + x_from, y_center));

    let dx = x_to - x_from;
    if dx.abs() < px(0.5) {
        path.line_to(point(left + x_to, y_bottom));
    } else {
        let dir = if dx > px(0.0) { 1.0 } else { -1.0 };
        let r = elbow_radius(preferred_radius, dx, y_bottom - y_center);
        let turn_x = x_to - r * dir;
        if (turn_x - x_from).abs() > px(0.05) {
            path.line_to(point(left + turn_x, y_center));
        }
        path.cubic_bezier_to(
            point(left + x_to, y_center + r),
            point(left + turn_x + r * (dir * ELBOW_K), y_center),
            point(left + x_to, y_center + r * (1.0 - ELBOW_K)),
        );
        path.line_to(point(left + x_to, y_bottom));
    }

    if let Ok(p) = path.build() {
        window.paint_path(p, color);
    }
}

/// Runs straight down the lane's own column from the top of the row, turns
/// through a rounded corner, then runs horizontally into the node.
#[allow(clippy::too_many_arguments)]
fn paint_lane_to_node(
    left: Pixels,
    x_from: Pixels,
    x_to: Pixels,
    y_top: Pixels,
    y_center: Pixels,
    preferred_radius: Pixels,
    stroke_width: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
) {
    use gpui::PathBuilder;

    let dx = x_to - x_from;
    let dir = if dx > px(0.0) { 1.0 } else { -1.0 };
    let r = elbow_radius(preferred_radius, dx, y_center - y_top);

    let mut path = PathBuilder::stroke(stroke_width);
    path.move_to(point(left + x_from, y_top));
    path.line_to(point(left + x_from, y_center - r));
    path.cubic_bezier_to(
        point(left + x_from + r * dir, y_center),
        point(left + x_from, y_center - r * (1.0 - ELBOW_K)),
        point(left + x_from + r * (dir * ELBOW_K), y_center),
    );
    path.line_to(point(left + x_to, y_center));

    if let Ok(p) = path.build() {
        window.paint_path(p, color);
    }
}

fn paint_commit_node(
    x_center: Pixels,
    y_center: Pixels,
    node_radius: Pixels,
    corner_radius: Pixels,
    node_color: gpui::Rgba,
    window: &mut Window,
) {
    window.paint_quad(
        fill(
            gpui::Bounds::new(
                point(x_center - node_radius, y_center - node_radius),
                size(node_radius * 2.0, node_radius * 2.0),
            ),
            node_color,
        )
        .corner_radii(node_radius.min(corner_radius)),
    );
}

fn paint_stash_node(
    x_center: Pixels,
    y_center: Pixels,
    fill_color: gpui::Rgba,
    border_color: gpui::Rgba,
    window: &mut Window,
) {
    use gpui::PathBuilder;

    let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
    let scaled_px = |value| px(value * design_scale_factor);
    let border = scaled_px(1.0);
    let body_w = scaled_px(11.2);
    let body_h = scaled_px(5.8);
    let outer_w = body_w + border * 2.0;
    let outer_h = body_h + border * 2.0;
    let body_y_offset = scaled_px(1.15);
    let body_radius = scaled_px(1.25);

    let outer = Bounds::new(
        point(
            x_center - outer_w * 0.5,
            y_center - outer_h * 0.5 + body_y_offset,
        ),
        size(outer_w, outer_h),
    );
    let inner = Bounds::new(
        point(outer.left() + border, outer.top() + border),
        size(body_w, body_h),
    );

    window.paint_quad(fill(outer, border_color).corner_radii(body_radius.min(scaled_px(3.0))));
    window.paint_quad(
        fill(inner, fill_color).corner_radii(
            (body_radius - scaled_px(0.6))
                .max(px(0.0))
                .min(scaled_px(1.5)),
        ),
    );

    let zipper = Bounds::new(
        point(x_center - scaled_px(4.2), inner.top() + scaled_px(1.3)),
        size(scaled_px(8.4), scaled_px(1.0)),
    );
    window.paint_quad(fill(zipper, with_alpha(border_color, 0.72)).corner_radii(scaled_px(0.5)));

    let handle_attach_y = outer.top() + scaled_px(0.15);
    let handle_apex_y = outer.top() - scaled_px(1.95);
    let handle_half_width = scaled_px(1.95);

    let mut handle = PathBuilder::stroke(border);
    handle.move_to(point(x_center - handle_half_width, handle_attach_y));
    handle.cubic_bezier_to(
        point(x_center + handle_half_width, handle_attach_y),
        point(x_center - scaled_px(1.9), handle_apex_y),
        point(x_center + scaled_px(1.9), handle_apex_y),
    );
    if let Ok(path) = handle.build() {
        window.paint_path(path, border_color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Design geometry the radius has to sit inside: 16px column pitch, 14px
    /// half-row.
    const COL_GAP: f32 = HISTORY_GRAPH_COL_GAP_PX;
    const HALF_ROW: f32 = 14.0;

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
}
