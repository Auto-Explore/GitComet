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

    let stroke_width = px(1.6);
    let col_gap = px(HISTORY_GRAPH_COL_GAP_PX);
    let margin_x = px(HISTORY_GRAPH_MARGIN_X_PX);
    let node_radius = if row.is_merge { px(3.9) } else { px(3.4) };

    let y_top = bounds.top();
    let y_center = bounds.top() + bounds.size.height / 2.0;
    let y_bottom = bounds.bottom();

    let x_for_col = |col: usize| margin_x + col_gap * (col as f32);
    let node_x = x_for_col(usize::from(row.node_col));

    // Incoming vertical segments.
    for (col, lane) in row.lanes_now.iter().enumerate() {
        let incoming = lane.incoming;
        if !(incoming || connect_from_top_col == Some(col)) {
            continue;
        }
        let x = x_for_col(col);
        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(bounds.left() + x, y_top));
        path.line_to(point(bounds.left() + x, y_center));
        if let Ok(p) = path.build() {
            window.paint_path(p, history_graph::lane_color(theme, lane.color_ix));
        }
    }

    // Incoming join edges into the node (used both for merge commits and fork points).
    for edge in row.joins_in.iter() {
        if edge.from_col == edge.to_col {
            continue;
        }
        let x_from = x_for_col(edge.from_col as usize);
        let x_to = x_for_col(edge.to_col as usize);
        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(bounds.left() + x_from, y_center));
        if (x_from - x_to).abs() < px(0.5) {
            path.line_to(point(bounds.left() + x_to, y_center));
        } else {
            let ctrl = px(8.0);
            path.cubic_bezier_to(
                point(bounds.left() + x_to, y_center),
                point(bounds.left() + x_from + ctrl, y_center),
                point(bounds.left() + x_to - ctrl, y_center),
            );
        }
        if let Ok(p) = path.build() {
            window.paint_path(p, history_graph::lane_color(theme, edge.color_ix));
        }
    }

    // Continuations from current row to next row.
    for (out_col, lane) in row.lanes_next.iter().enumerate() {
        let x_out = x_for_col(out_col);

        let x_from = lane
            .from_col
            .map(|col| x_for_col(col as usize))
            .unwrap_or(node_x);

        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(bounds.left() + x_from, y_center));
        if (x_from - x_out).abs() < px(0.5) {
            path.line_to(point(bounds.left() + x_out, y_bottom));
        } else {
            let y_mid = y_center + (y_bottom - y_center) * 0.5;
            path.cubic_bezier_to(
                point(bounds.left() + x_out, y_bottom),
                point(bounds.left() + x_from, y_mid),
                point(bounds.left() + x_out, y_mid),
            );
        }
        if let Ok(p) = path.build() {
            window.paint_path(p, history_graph::lane_color(theme, lane.color_ix));
        }
    }

    // Additional merge edges from the node into lanes that were re-targeted to secondary parents.
    for edge in row.edges_out.iter() {
        if edge.from_col == edge.to_col {
            continue;
        }
        let x_to = x_for_col(edge.to_col as usize);
        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(bounds.left() + node_x, y_center));
        if (node_x - x_to).abs() < px(0.5) {
            path.line_to(point(bounds.left() + x_to, y_bottom));
        } else {
            let y_mid = y_center + (y_bottom - y_center) * 0.5;
            path.cubic_bezier_to(
                point(bounds.left() + x_to, y_bottom),
                point(bounds.left() + node_x, y_mid),
                point(bounds.left() + x_to, y_mid),
            );
        }
        if let Ok(p) = path.build() {
            window.paint_path(p, history_graph::lane_color(theme, edge.color_ix));
        }
    }

    let node_color = row
        .lanes_now
        .get(usize::from(row.node_col))
        .map(|l| history_graph::lane_color(theme, l.color_ix))
        .unwrap_or(theme.colors.text_muted);

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
            node_color,
            window,
        );
    }
}

fn paint_commit_node(
    x_center: Pixels,
    y_center: Pixels,
    node_radius: Pixels,
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
        .corner_radii(node_radius.min(px(2.0))),
    );
}

fn paint_stash_node(
    x_center: Pixels,
    y_center: Pixels,
    fill_color: gpui::Rgba,
    border_color: gpui::Rgba,
    window: &mut Window,
) {
    let border = px(1.0);
    let box_w = px(9.0);
    let box_h = px(8.0);
    let outer_w = box_w + border * 2.0;
    let outer_h = box_h + border * 2.0;
    let r = px(1.8);

    let outer = Bounds::new(
        point(x_center - outer_w * 0.5, y_center - outer_h * 0.5),
        size(outer_w, outer_h),
    );
    let inner = Bounds::new(
        point(outer.left() + border, outer.top() + border),
        size(box_w, box_h),
    );

    window.paint_quad(fill(outer, border_color).corner_radii(r.min(px(2.0))));
    window
        .paint_quad(fill(inner, fill_color).corner_radii((r - px(0.4)).max(px(0.0)).min(px(2.0))));

    // Simple "lid" line to make it read as a stash/box.
    let lid_y = inner.top() + px(2.4);
    let lid = Bounds::new(
        point(inner.left() + px(1.0), lid_y),
        size((inner.size.width - px(2.0)).max(px(0.0)), px(1.0)),
    );
    window.paint_quad(fill(lid, with_alpha(border_color, 0.65)));
}
