use super::*;
use gpui::{App, Bounds, Pixels, Window, fill, point, px, size};
use smallvec::SmallVec;

pub(super) fn paint_history_graph(
    theme: AppTheme,
    row: &history_graph::GraphRow,
    connect_from_top_col: Option<usize>,
    is_stash_node: bool,
    // Colour index of the lane the selected commit sits on. Every colour the
    // graph draws goes through `lane`, so that one lane stays saturated along its
    // whole run and every other lane recedes along its whole run -- a property of
    // the lane, not of which rows happen to connect to the selection.
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui::PathBuilder;

    if row.lanes_now.is_empty() {
        return;
    }

    let lane = |color_ix| lane_wash_color(theme, color_ix, selected_lane_color_ix);

    let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
    let scaled_px = |value| px(value * design_scale_factor);
    let stroke_width = scaled_px(1.6);
    let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
    let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
    let node_radius = scaled_px(3.4);
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
    for (col, lane_paint) in row.lanes_now.iter().enumerate() {
        if !lane_paint.is_active() {
            continue;
        }
        if !(lane_paint.incoming() || connect_from_top_col == Some(col)) {
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
            window.paint_path(p, lane(lane_paint.color_ix));
        }
    }

    // Incoming join edges into the node (used both for merge commits and fork points).
    for edge in row.joins_in.iter() {
        if edge.from_col == edge.to_col {
            continue;
        }
        let from = usize::from(edge.from_col);
        let color = lane(edge.color_ix);
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
    for (out_col, lane_paint) in row.lanes_next.iter().enumerate() {
        if !lane_paint.is_active() {
            continue;
        }
        let x_out = x_for_col(out_col);
        let color = lane(lane_paint.color_ix);
        if lane_paint.starts_at_node() {
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
            lane(edge.color_ix),
            window,
        );
    }

    let node_color = lane(row.node_color_ix);

    // Within one paint layer gpui draws all quads before any path, so the
    // node (a quad) would sit under the lane lines no matter the call
    // order. A nested layer gives the node a strictly higher draw order;
    // its bounds are generous enough for the 16px icon nodes.
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
            paint_icon_node(
                bounds.left() + node_x,
                y_center,
                icons::GIT_STASH_NODE_ICON_PATH,
                theme.colors.surface.canvas,
                node_color,
                window,
                cx,
            );
        } else if row.is_merge {
            paint_icon_node(
                bounds.left() + node_x,
                y_center,
                icons::GIT_MERGE_ICON_PATH,
                theme.colors.surface.canvas,
                node_color,
                window,
                cx,
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

/// The lane-coloured wash down the right edge of the graph column, tying a row's
/// node to the border on its message cell. Shared by the commit rows and the two
/// uncommitted-changes rows so all three fade identically.
pub(super) fn paint_graph_fade(
    color: gpui::Rgba,
    graph_bounds: Bounds<Pixels>,
    fade_width: Pixels,
    window: &mut Window,
) {
    if graph_bounds.size.width <= px(0.0) {
        return;
    }
    let fade_w = graph_bounds.size.width.min(fade_width);
    window.paint_quad(fill(
        Bounds::new(
            point(graph_bounds.right() - fade_w, graph_bounds.top()),
            size(fade_w, graph_bounds.size.height),
        ),
        gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(with_alpha(color, 0.0), 0.0),
            gpui::linear_color_stop(with_alpha(color, HISTORY_GRAPH_FADE_ALPHA), 1.0),
        ),
    ));
}

/// The column the row directly above draws its connector down from, or `None`
/// when nothing above it connects.
///
/// Shared by the worktree bands and the commit rows: both draw the matching stub
/// upwards, and both have to agree on the column or the two rows show a seam.
///
/// Only the two synthetic row kinds connect downwards: the pinned working-tree
/// row, which always sits on column 0, and a worktree band. A band's connector
/// leaves on its node's `exit_col`, not the column the node is drawn on — when the
/// node is pushed out to a free column it elbows back across before the row ends
/// (see [`band_node_for`]) — and it is that landing column the row below must
/// match. Two bands can also share an anchor commit without sharing a column, a
/// detached worktree and one on a branch that has fallen behind resolving
/// differently on the very same row, so the answer always comes from the row
/// above's own summary, never from the row asking.
pub(in crate::view) fn worktree_band_connect_from_top_col(
    plan: &crate::view::caches::HistoryListPlan,
    graph_rows: &[history_graph::GraphRow],
    worktree_dirty: &[gitcomet_core::domain::WorktreeDirtySummary],
    list_ix: usize,
) -> Option<usize> {
    use crate::view::caches::HistoryListRow;

    match plan.row_at(list_ix.checked_sub(1)?) {
        Some(HistoryListRow::WorkingTreeSummary) => Some(0),
        Some(HistoryListRow::WorktreeUncommitted {
            visible_ix,
            worktree_ix,
        }) => {
            let above_row = graph_rows.get(visible_ix)?;
            let above = worktree_dirty.get(worktree_ix)?;
            let on_branch = above.branch.is_some() && !above.detached;
            Some(usize::from(band_node_for(above_row, on_branch).exit_col))
        }
        _ => None,
    }
}

/// A lane's colour, washed out unless it is the lane the selection sits on.
///
/// The wash is a property of the *lane*: a lane keeps its colour index for its
/// whole lifetime and `pick_lane_color_ix` avoids collisions between lanes that
/// are alive at the same time, so the index identifies a lane across every row it
/// spans. That is what keeps a washed lane washed from top to bottom instead of
/// flickering wherever it happens to touch the selected chain.
///
/// `None` -- nothing selected -- leaves every lane at full strength.
pub(super) fn lane_wash_color(
    theme: AppTheme,
    color_ix: history_graph::LaneColorIx,
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
) -> gpui::Rgba {
    let full = history_graph::lane_color(theme, color_ix);
    match selected_lane_color_ix {
        // The same mix the unrelated-row dimming uses, so the two read alike.
        Some(selected) if selected != color_ix => {
            history_canvas::selection_related_lane_color(theme, full, Some(false))
        }
        _ => full,
    }
}

/// How a band's node is painted: the column it sits on, its resolved colour, and
/// where its connector leaves the band when that column is one of its own.
#[derive(Clone, Copy, Debug)]
pub(super) struct BandNodePaint {
    pub(super) col: u16,
    pub(super) color: gpui::Rgba,
    pub(super) exit_col: Option<u16>,
}

/// The column and colour a band's node sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct BandNode {
    pub(in crate::view) col: u16,
    /// Column the node's connector leaves the band's bottom edge on. Differs
    /// from `col` when the node had to take a column of its own, in which case
    /// the band elbows across into the commit below.
    pub(in crate::view) exit_col: u16,
    pub(in crate::view) color_ix: history_graph::LaneColorIx,
}

/// Where a worktree's uncommitted node belongs on the row it sits above.
///
/// A branch that has fallen behind does not own the lane its head commit is
/// drawn on — the graph gives it a lane of its own, born at that commit and
/// drawn as a whisker into the node (`history_graph.rs`, the `force_branch_head_lane`
/// fork). A worktree checked out on such a branch belongs on *that* lane, in its
/// colour; putting it on the commit's lane claims the work sits on the branch
/// that happens to own the column, which is a different branch entirely.
///
/// A fork lane is recognised the same way the painter recognises it: a join edge
/// whose source lane is born on this row rather than carried into it. There is
/// at most one such lane per row. `on_branch` is false for a detached worktree,
/// which has no branch to claim the fork.
pub(in crate::view) fn band_node_for(row: &history_graph::GraphRow, on_branch: bool) -> BandNode {
    let fork = on_branch.then(|| {
        row.joins_in.iter().find(|edge| {
            edge.from_col != edge.to_col
                && row
                    .lanes_now
                    .get(usize::from(edge.from_col))
                    .is_some_and(|lane| lane.is_active() && !lane.incoming())
        })
    });
    let (natural_col, color_ix) = match fork.flatten() {
        Some(edge) => (edge.from_col, edge.color_ix),
        None => (row.node_col, row.node_color_ix),
    };

    // Uncommitted changes are not a commit: nothing descends from them, so the
    // node must never sit on a lane that runs *past* it. On a lane carried in
    // from the row above it would read as a link in that lane's chain -- as if
    // the changes were an ancestor of whatever is above. A lane born at the
    // commit below has nothing above it and is safe to sit on; anything else
    // pushes the node out to a column of its own.
    let passes_through = row
        .lanes_now
        .get(usize::from(natural_col))
        .is_some_and(|lane| lane.is_active() && lane.incoming());
    if passes_through {
        BandNode {
            // One past the last lane: always free, and the graph column's
            // trailing margin leaves room for it.
            col: row.lanes_now.len() as u16,
            exit_col: row.node_col,
            color_ix,
        }
    } else {
        BandNode {
            col: natural_col,
            exit_col: natural_col,
            color_ix,
        }
    }
}

/// Which halves of a band row a lane occupies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BandLaneSegment {
    pub(super) col: usize,
    pub(super) color_ix: history_graph::LaneColorIx,
    /// Top edge down to the row's centre.
    pub(super) has_top: bool,
    /// Centre down to the bottom edge.
    pub(super) has_bottom: bool,
}

/// Decides what a band row draws, given the `lanes_now` of the commit below it.
///
/// The band's top edge has to match the bottom edge of whatever sits above, and
/// its bottom edge has to match the commit's top edge. `paint_history_graph`
/// only draws a top-half segment for lanes that are `incoming()` (or that
/// `connect_from_top_col` names), because `lanes_now` also carries lanes that are
/// *born* at that commit — a newborn lane, or a branch head forking off. Those
/// have nothing above them, so the band must not draw through them either.
///
/// The one exception is the band's own column: our node connects down into the
/// commit's node, so it always gets a bottom half even when the lane starts there.
pub(super) fn band_lane_segments(
    lanes: &[history_graph::LanePaint],
    node_col: usize,
    connect_from_top_col: Option<usize>,
) -> SmallVec<[BandLaneSegment; 8]> {
    lanes
        .iter()
        .enumerate()
        .filter_map(|(col, lane)| {
            if !lane.is_active() {
                return None;
            }
            let passes_through = lane.incoming() || connect_from_top_col == Some(col);
            let segment = BandLaneSegment {
                col,
                color_ix: lane.color_ix,
                has_top: passes_through,
                has_bottom: passes_through || col == node_col,
            };
            (segment.has_top || segment.has_bottom).then_some(segment)
        })
        .collect()
}

/// Paints a synthetic row that sits *between* two commits: lanes that flow past
/// run straight through, with an uncommitted-changes node on the band's own
/// column connecting down into the commit below.
///
/// Takes the commit's `lanes` and the band's own `node_col` rather than a
/// `GraphRow`: a band has nothing to elbow and no secondary-parent edges, so the
/// rest of a row -- `lanes_next`, `joins_in`, `edges_out` -- would be a per-row,
/// per-frame clone of data this never reads.
pub(super) fn paint_history_graph_band(
    theme: AppTheme,
    lanes: &[history_graph::LanePaint],
    connect_from_top_col: Option<usize>,
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
    node: BandNodePaint,
    show_graph_color_marker: bool,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui::PathBuilder;

    if lanes.is_empty() {
        return;
    }

    let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
    let scaled_px = |value| px(value * design_scale_factor);
    let stroke_width = scaled_px(1.6);
    let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
    let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
    let elbow_radius = scaled_px(HISTORY_GRAPH_ELBOW_RADIUS_PX);

    let left = bounds.left();
    let y_top = bounds.top();
    let y_center = bounds.top() + bounds.size.height / 2.0;
    let y_bottom = bounds.bottom();
    let x_for_col = |col: usize| margin_x + col_gap * (col as f32);

    // The same wash the commit rows carry into their message border, painted
    // before the lanes so the strokes stay crisp on top of it.
    if show_graph_color_marker {
        paint_graph_fade(
            node.color,
            bounds,
            scaled_px(HISTORY_GRAPH_FADE_WIDTH_PX),
            window,
        );
    }

    for segment in band_lane_segments(lanes, usize::from(node.col), connect_from_top_col) {
        let x = left + x_for_col(segment.col);
        let from_y = if segment.has_top { y_top } else { y_center };
        let to_y = if segment.has_bottom {
            y_bottom
        } else {
            y_center
        };
        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(x, from_y));
        path.line_to(point(x, to_y));
        if let Ok(p) = path.build() {
            // The lane's own colour, not the node's: a branch head keeps the
            // descendant lane's colour above the node, and the commit below
            // paints its matching stub the same way -- including its wash, or the
            // seam between the two rows reappears.
            window.paint_path(
                p,
                lane_wash_color(theme, segment.color_ix, selected_lane_color_ix),
            );
        }
    }

    // The node sits on a column no lane runs through, so it reaches the commit
    // below by leaving horizontally and turning down -- the same shape a branch
    // head's whisker takes.
    if let Some(exit_col) = node.exit_col {
        paint_node_to_lane(
            left,
            x_for_col(usize::from(node.col)),
            x_for_col(usize::from(exit_col)),
            y_center,
            y_bottom,
            elbow_radius,
            stroke_width,
            node.color,
            window,
        );
    }

    // Same nested-layer trick the commit nodes use: quads otherwise draw under
    // every path in the layer regardless of call order.
    let node_x = left + x_for_col(usize::from(node.col));
    let node_layer_half = scaled_px(10.0);
    let node_layer_bounds = Bounds::new(
        point(node_x - node_layer_half, y_center - node_layer_half),
        size(node_layer_half * 2.0, node_layer_half * 2.0),
    );
    window.paint_layer(node_layer_bounds, |window| {
        paint_ring_icon_node(
            node_x,
            y_center,
            icons::UNCOMMITTED_NODE_ICON_PATH,
            node.color,
            theme.colors.surface.canvas,
            window,
            cx,
        );
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

/// Nodes that carry a glyph — merges and stashes — read as a solid disc in the
/// lane colour with the icon knocked out of it in the background colour. Sized
/// to the full lane pitch, so in dense multi-lane regions the disc touches its
/// neighbours.
pub(super) fn paint_icon_node(
    x_center: Pixels,
    y_center: Pixels,
    icon_path: &'static str,
    glyph_color: gpui::Rgba,
    disc_color: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) {
    let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
    let scaled_px = |value| px(value * design_scale_factor);
    let diameter = scaled_px(16.0);
    let glyph = scaled_px(10.5);

    let disc = Bounds::new(
        point(x_center - diameter * 0.5, y_center - diameter * 0.5),
        size(diameter, diameter),
    );

    window.paint_quad(fill(disc, disc_color).corner_radii(diameter * 0.5));

    // gpui orders primitives within a layer by kind, and the sprite `paint_svg`
    // emits sorts after quads, so the glyph lands on top of the disc without
    // needing a layer of its own.
    super::diff_canvas::paint_centered_svg_icon(icon_path, disc, glyph, glyph_color, window, cx);
}

/// The inverse of [`paint_icon_node`]: an outlined circle with the glyph drawn
/// solid, over an opaque middle.
///
/// The middle is filled rather than left transparent so the lane running through
/// the node's column does not show through the hole. `background` is the history
/// list's own surface; a row that paints a tint over it (hover, selection, the
/// browsing-commit highlight) will not match exactly.
pub(super) fn paint_ring_icon_node(
    x_center: Pixels,
    y_center: Pixels,
    icon_path: &'static str,
    ring_color: gpui::Rgba,
    background: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) {
    let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
    let scaled_px = |value| px(value * design_scale_factor);
    let diameter = scaled_px(16.0);
    let ring_width = scaled_px(1.5);
    let glyph = scaled_px(10.5);

    let disc = Bounds::new(
        point(x_center - diameter * 0.5, y_center - diameter * 0.5),
        size(diameter, diameter),
    );

    window.paint_quad(gpui::quad(
        disc,
        diameter * 0.5,
        background,
        gpui::Edges::all(ring_width),
        ring_color,
        gpui::BorderStyle::Solid,
    ));

    super::diff_canvas::paint_centered_svg_icon(icon_path, disc, glyph, ring_color, window, cx);
}

#[cfg(test)]
mod band_tests {
    use super::*;
    use crate::view::caches::{HistoryListPlan, HistoryWorktreeRowAnchor};
    use crate::view::history_graph::{GraphRow, LanePaint};
    use gitcomet_core::domain::WorktreeDirtySummary;

    fn row_anchor(visible_ix: usize, worktree_ix: usize) -> HistoryWorktreeRowAnchor {
        HistoryWorktreeRowAnchor {
            visible_ix,
            worktree_ix,
        }
    }

    /// A band takes the `lanes_now` of the commit it sits above.
    fn band(lanes: &[LanePaint], node_col: u16) -> GraphRow {
        GraphRow {
            lanes_now: lanes.iter().copied().collect(),
            lanes_next: lanes.iter().copied().collect(),
            joins_in: Default::default(),
            edges_out: Default::default(),
            node_col,
            node_color_ix: 0,
            is_merge: false,
        }
    }

    fn incoming(color_ix: u8) -> LanePaint {
        LanePaint::lane(color_ix, true, false)
    }

    /// Born at the commit below -- a new lane, or a branch head forking off.
    /// Nothing exists above it.
    fn born(color_ix: u8) -> LanePaint {
        LanePaint::lane(color_ix, false, false)
    }

    fn segment(row: &GraphRow, connect: Option<usize>, col: usize) -> Option<BandLaneSegment> {
        band_lane_segments(&row.lanes_now, usize::from(row.node_col), connect)
            .into_iter()
            .find(|segment| segment.col == col)
    }

    fn edge(from_col: u16, to_col: u16, color_ix: u8) -> crate::view::history_graph::GraphEdge {
        crate::view::history_graph::GraphEdge {
            from_col,
            to_col,
            color_ix,
        }
    }

    /// A branch that has fallen behind is drawn as a lane born at its head
    /// commit, whiskered into a node the *other* branch owns. The worktree sits
    /// on the branch, so it belongs on that born lane, in its colour.
    #[test]
    fn a_behind_branchs_fork_lane_claims_the_node() {
        let mut row = band(&[incoming(1), born(7)], 0);
        row.joins_in = [edge(1, 0, 7)].into_iter().collect();

        let node = band_node_for(&row, true);
        assert_eq!(node.col, 1, "the node belongs on the branch's own lane");
        assert_eq!(node.color_ix, 7, "and takes that lane's colour");
    }

    /// Selecting a worktree row highlights its *branch's* lane. For a branch
    /// that has fallen behind, that is the fork lane beside the commit — not the
    /// lane the commit itself is drawn on, which belongs to its descendant.
    #[test]
    fn a_behind_branchs_highlight_follows_the_fork_lane_not_the_commit() {
        let mut row = band(&[incoming(1), born(7)], 0);
        row.joins_in = [edge(1, 0, 7)].into_iter().collect();

        let highlighted = band_node_for(&row, true).color_ix;
        assert_eq!(highlighted, 7, "the branch's own lane is what lights up");
        assert_ne!(
            highlighted, row.node_color_ix,
            "the commit's lane belongs to whatever descends from it"
        );
    }

    /// A branch with commits of its own owns the lane its head is drawn on, so
    /// highlighting it lights that lane the whole way down.
    #[test]
    fn a_branch_that_owns_its_lane_highlights_that_lane() {
        let row = band(&[incoming(4)], 0);
        assert_eq!(band_node_for(&row, true).color_ix, row.node_color_ix);
    }

    /// A detached worktree has no branch, so a fork lane on the row belongs to
    /// somebody else and must not be claimed.
    #[test]
    fn a_detached_worktree_does_not_claim_the_fork_lane() {
        let mut row = band(&[incoming(1), born(7)], 0);
        row.joins_in = [edge(1, 0, 7)].into_iter().collect();

        let node = band_node_for(&row, false);
        assert_ne!(
            node.col, 1,
            "the fork lane belongs to the branch, not to us"
        );
        assert_eq!(
            node.color_ix, row.node_color_ix,
            "it takes the colour of the commit it sits on"
        );
    }

    /// A merge's incoming lanes are carried in from above, not born here, so
    /// they are not fork lanes and must not steal the node.
    #[test]
    fn a_merges_incoming_lanes_are_not_fork_lanes() {
        let mut row = band(&[incoming(1), incoming(7)], 0);
        row.joins_in = [edge(1, 0, 7)].into_iter().collect();

        assert_ne!(
            band_node_for(&row, true).col,
            1,
            "a lane carried in from above is not a branch head's fork"
        );
    }

    /// Without a fork the node takes the commit's colour, and — because that
    /// commit's lane is carried in from above — a column of its own.
    #[test]
    fn without_a_fork_the_node_takes_the_commits_colour() {
        let row = band(&[incoming(4)], 0);
        let node = band_node_for(&row, true);
        assert_eq!(node.color_ix, row.node_color_ix);
        assert_ne!(node.col, row.node_col);
    }

    /// Uncommitted changes are not a commit, so nothing may appear to descend
    /// through them. On a lane carried in from the row above they would read as
    /// a link in that lane's chain — as if the merge above had them as an
    /// ancestor — so the node moves to a free column and elbows into its commit.
    #[test]
    fn a_lane_running_past_the_row_pushes_the_node_to_its_own_column() {
        let row = band(&[incoming(1), incoming(7)], 1);

        let node = band_node_for(&row, true);
        assert_eq!(node.col, 2, "one past the last lane, which is always free");
        assert_eq!(
            node.exit_col, row.node_col,
            "and it elbows across into the commit it sits on"
        );
        assert_ne!(node.col, node.exit_col, "so the band draws that elbow");
    }

    /// A lane born at the commit below has nothing above it, so the node can sit
    /// on it directly and simply run straight down.
    #[test]
    fn a_lane_born_below_needs_no_column_of_its_own() {
        let mut row = band(&[incoming(1), born(7)], 0);
        row.joins_in = [edge(1, 0, 7)].into_iter().collect();

        let node = band_node_for(&row, true);
        assert_eq!(node.col, 1);
        assert_eq!(node.exit_col, node.col, "a straight connector, no elbow");
    }

    fn worktree(branch: Option<&str>, detached: bool) -> WorktreeDirtySummary {
        WorktreeDirtySummary {
            path: std::path::PathBuf::from("/tmp/worktree"),
            head: None,
            branch: branch.map(str::to_string),
            detached,
            added: 1,
            modified: 0,
            deleted: 0,
            staged: Vec::new(),
            unstaged: Vec::new(),
        }
    }

    /// Two dirty worktrees on one commit stack into two bands above it. They
    /// share the anchor row but not necessarily the column, so the lower band's
    /// connector has to be read off the *upper* band's own worktree.
    #[test]
    fn a_stacked_band_connects_from_the_band_above_not_from_itself() {
        let mut row = band(&[incoming(1), born(7)], 0);
        row.joins_in = [edge(1, 0, 7)].into_iter().collect();

        // Detached above, on a behind branch below: `band_node_for` puts them on
        // different columns for the same row.
        let dirty = [worktree(None, true), worktree(Some("behind"), false)];
        let plan = HistoryListPlan::new(false, vec![row_anchor(0, 0), row_anchor(0, 1)]);
        let rows = [row.clone()];

        let detached = band_node_for(&row, false);
        let on_branch = band_node_for(&row, true);
        assert_ne!(
            detached.exit_col, on_branch.exit_col,
            "fixture must actually put the two worktrees on different columns"
        );
        assert_ne!(
            detached.col, detached.exit_col,
            "and the detached node must be the pushed-out kind, so col != exit_col"
        );

        assert_eq!(
            worktree_band_connect_from_top_col(&plan, &rows, &dirty, 1),
            Some(usize::from(detached.exit_col)),
            "the lower band meets the column the band above actually lands on, \
             not the one its node is drawn on"
        );
        assert_eq!(
            worktree_band_connect_from_top_col(&plan, &rows, &dirty, 0),
            None,
            "the top band has nothing above it"
        );
    }

    /// The pinned working-tree row always draws its connector straight down
    /// column 0, whatever the band below resolves to.
    #[test]
    fn a_band_under_the_working_tree_row_connects_from_column_zero() {
        let row = band(&[incoming(1), born(7)], 0);
        let dirty = [worktree(Some("behind"), false)];
        let plan = HistoryListPlan::new(true, vec![row_anchor(0, 0)]);

        assert_eq!(
            worktree_band_connect_from_top_col(&plan, &[row], &dirty, 1),
            Some(0)
        );
    }

    /// A commit row above draws nothing down into the band: the band sits on top
    /// of its own commit, and the commit above it is a separate lane run.
    #[test]
    fn a_band_under_a_commit_row_has_no_connector() {
        let row = band(&[incoming(1), born(7)], 0);
        let dirty = [worktree(Some("behind"), false)];
        // One anchor on the *second* visible commit, so a commit row sits above it.
        let plan = HistoryListPlan::new(false, vec![row_anchor(1, 0)]);

        assert_eq!(
            worktree_band_connect_from_top_col(&plan, &[row.clone(), row], &dirty, 1),
            None
        );
    }

    /// The wash is a property of the lane, not of the row: the painter routes
    /// every stroke and node fill through this, so a regression here either
    /// un-washes the whole graph or washes the selected lane along with the rest.
    #[test]
    fn only_the_selected_lane_keeps_its_colour() {
        let theme = AppTheme::gitcomet_dark();
        let selected = 3u8;
        let other = 5u8;

        assert_eq!(
            lane_wash_color(theme, other, None),
            history_graph::lane_color(theme, other),
            "with nothing selected every lane stays at full strength"
        );
        assert_eq!(
            lane_wash_color(theme, selected, Some(selected)),
            history_graph::lane_color(theme, selected),
            "the selected commit's own lane is never washed"
        );

        let washed = lane_wash_color(theme, other, Some(selected));
        assert_ne!(
            washed,
            history_graph::lane_color(theme, other),
            "every other lane recedes"
        );
        assert_eq!(
            washed,
            history_canvas::selection_related_lane_color(
                theme,
                history_graph::lane_color(theme, other),
                Some(false)
            ),
            "reusing the row dimming's mix keeps the two reading alike"
        );
        assert_eq!(
            washed.a, 1.0,
            "opaque on purpose: lanes are stroked over the graph fade wash"
        );
    }

    /// A node takes its own lane's colour index, so "nodes follow their lane"
    /// needs no separate rule -- but it does need the node to go through the
    /// same lookup, which this pins.
    #[test]
    fn a_node_is_washed_with_the_lane_it_sits_on() {
        let theme = AppTheme::gitcomet_dark();
        let row = band(&[incoming(7)], 0);

        assert_eq!(
            lane_wash_color(theme, row.node_color_ix, Some(row.node_color_ix)),
            history_graph::lane_color(theme, row.node_color_ix)
        );
        assert_ne!(
            lane_wash_color(theme, row.node_color_ix, Some(row.node_color_ix + 1)),
            history_graph::lane_color(theme, row.node_color_ix)
        );
    }

    #[test]
    fn an_incoming_lane_runs_the_full_height() {
        let row = band(&[incoming(3)], 0);
        let segment = segment(&row, None, 0).expect("the lane is drawn");
        assert!(segment.has_top && segment.has_bottom);
        assert_eq!(segment.color_ix, 3);
    }

    /// The regression this rule exists for: drawing a full-height line here put a
    /// stray segment above a lane that does not exist yet, and left the node with
    /// nothing to connect to.
    #[test]
    fn a_lane_born_at_the_commit_below_is_drawn_only_under_the_node() {
        let row = band(&[born(1), born(2)], 0);

        let node_column = segment(&row, None, 0).expect("the node column is drawn");
        assert!(
            !node_column.has_top,
            "nothing exists above a lane that starts at the commit below"
        );
        assert!(
            node_column.has_bottom,
            "the node still has to reach the commit below it"
        );

        assert_eq!(
            segment(&row, None, 1),
            None,
            "a born lane away from the node column is not drawn at all"
        );
    }

    #[test]
    fn holes_are_never_drawn() {
        let row = band(&[LanePaint::HOLE, incoming(4)], 1);
        assert_eq!(segment(&row, None, 0), None);
        assert!(segment(&row, None, 1).is_some());
    }

    /// The working-tree row above, or a second worktree band, connects down into
    /// this one, so the named column has to pass through even when the lane below
    /// is born rather than carried in.
    #[test]
    fn the_connect_override_restores_the_top_half() {
        let row = band(&[born(1)], 0);
        assert!(!segment(&row, None, 0).expect("drawn").has_top);

        let connected = segment(&row, Some(0), 0).expect("drawn");
        assert!(connected.has_top && connected.has_bottom);
    }

    #[test]
    fn the_connect_override_only_applies_to_the_column_it_names() {
        let row = band(&[born(1), born(2)], 0);
        assert_eq!(segment(&row, Some(0), 1), None);
    }

    /// Lanes flowing past a band keep running while the node sits on its own
    /// column -- the common shape when a worktree is checked out on a side branch.
    #[test]
    fn a_pass_through_lane_and_a_born_node_column_coexist() {
        let row = band(&[incoming(1), born(2)], 1);
        let passing = segment(&row, None, 0).expect("drawn");
        assert!(passing.has_top && passing.has_bottom);

        let node_column = segment(&row, None, 1).expect("drawn");
        assert!(!node_column.has_top && node_column.has_bottom);
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
