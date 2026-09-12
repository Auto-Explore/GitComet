use super::*;
use gpui::{App, Bounds, Pixels, Window, fill, point, px, size};
use smallvec::SmallVec;

/// Keep the last winner for each displayed geometry, visiting the selected
/// pass first in reverse order. Reversing the survivors preserves paint order
/// at intersections while retained storage is bounded by displayed geometry.
fn coalesced<T: Copy, K: Eq + std::hash::Hash>(
    items: impl DoubleEndedIterator<Item = T> + Clone,
    geometry: impl Fn(T) -> K,
    selected: impl Fn(T) -> bool,
) -> SmallVec<[T; 8]> {
    let mut seen = rustc_hash::FxHashSet::default();
    let mut winners = SmallVec::new();
    for pass in [true, false] {
        for item in items.clone().rev() {
            if selected(item) == pass && seen.insert(geometry(item)) {
                winners.push(item);
            }
        }
    }
    winners.reverse();
    winners
}

fn x_key(x: Pixels) -> u32 {
    f32::from(x).to_bits()
}

/// The first column [`graph_col_x`] pins to the edge x; every column from here
/// on draws at that one x.
fn pinned_from_col(margin_x: Pixels, col_gap: Pixels, edge_x: Pixels) -> usize {
    let natural = |col: usize| margin_x + col_gap * (col as f32);
    if col_gap <= px(0.0) {
        return if natural(0) >= edge_x { 0 } else { usize::MAX };
    }
    let mut col = ((edge_x - margin_x) / col_gap).max(0.0).ceil() as usize;
    while col > 0 && natural(col - 1) >= edge_x {
        col -= 1;
    }
    while col < usize::from(u16::MAX) && natural(col) < edge_x {
        col += 1;
    }
    col
}

type LaneItem = (usize, history_graph::LanePaint);

/// [`coalesced`] over a dense lane array without visiting every lane.
///
/// Columns before `pin_col` each draw at their own x, so every included lane
/// there wins outright. Columns from `pin_col` on all draw at the edge x, where
/// one geometry wants the last selected lane, or else the last lane: a backward
/// scan finds it without hashing the whole frontier, and stops as soon as it
/// has it. `extra_tail` carries the (selected, plain) winners of a second edge
/// geometry the caller resolved from row summaries. Winners come out in
/// `coalesced`'s order: plain ones by column, then selected ones by column.
fn coalesced_lanes(
    lanes: &[history_graph::LanePaint],
    pin_col: usize,
    selection_possible: bool,
    include: impl Fn(usize, history_graph::LanePaint) -> bool,
    selected: impl Fn(usize, history_graph::LanePaint) -> bool,
    extra_tail: (Option<LaneItem>, Option<LaneItem>),
) -> SmallVec<[LaneItem; 8]> {
    let mut winners: SmallVec<[LaneItem; 8]> = SmallVec::new();
    let mut chosen: SmallVec<[LaneItem; 2]> = SmallVec::new();
    let displayed = pin_col.min(lanes.len());
    for (col, &lane) in lanes[..displayed].iter().enumerate() {
        if include(col, lane) {
            if selected(col, lane) {
                chosen.push((col, lane));
            } else {
                winners.push((col, lane));
            }
        }
    }
    let mut tail_chosen = None;
    let mut tail_plain = None;
    for col in (displayed..lanes.len()).rev() {
        let lane = lanes[col];
        if !include(col, lane) {
            continue;
        }
        if selection_possible && selected(col, lane) {
            tail_chosen = Some((col, lane));
            break;
        }
        if tail_plain.is_none() {
            tail_plain = Some((col, lane));
            if !selection_possible {
                break;
            }
        }
    }
    let mut tail: SmallVec<[LaneItem; 2]> = SmallVec::new();
    if tail_chosen.is_none() {
        tail.extend(tail_plain);
    }
    if extra_tail.0.is_none() {
        tail.extend(extra_tail.1);
    }
    tail.sort_unstable_by_key(|(col, _)| *col);
    winners.extend(tail);
    winners.extend(chosen);
    let mut tail: SmallVec<[LaneItem; 2]> = SmallVec::new();
    tail.extend(tail_chosen);
    tail.extend(extra_tail.0);
    tail.sort_unstable_by_key(|(col, _)| *col);
    winners.extend(tail);
    winners
}

/// The row's continuations, coalesced like [`coalesced`] keyed on x and on
/// whether the lane elbows out of the node. Lanes born at the node that land on
/// the edge line come from the row's summary, so an unpinned node never forces a
/// scan of the pinned columns.
fn continuing_winners(
    row: &history_graph::GraphRow,
    pin_col: usize,
    node_pinned: bool,
    selection_possible: bool,
    selected: impl Fn(usize, history_graph::LanePaint) -> bool,
) -> SmallVec<[LaneItem; 8]> {
    let mut extra = (None, None);
    if !node_pinned {
        for &col in row.from_node_cols.iter().rev() {
            let col = usize::from(col);
            if col < pin_col {
                break;
            }
            let Some(&lane) = row.lanes_next.get(col) else {
                continue;
            };
            if !(lane.is_active() && lane.starts_at_node()) {
                continue;
            }
            if selection_possible && selected(col, lane) {
                extra.0 = Some((col, lane));
                break;
            }
            if extra.1.is_none() {
                extra.1 = Some((col, lane));
                if !selection_possible {
                    break;
                }
            }
        }
    }
    coalesced_lanes(
        &row.lanes_next,
        pin_col,
        selection_possible,
        |col, lane| lane.is_active() && !(col >= pin_col && !node_pinned && lane.starts_at_node()),
        selected,
        extra,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_history_graph(
    theme: AppTheme,
    row: &history_graph::GraphRow,
    // This row's index into `graph_rows`, so a lane can be told from an
    // unrelated one elsewhere on the page that recycled its colour.
    row_ix: usize,
    connect_from_top_col: Option<usize>,
    is_stash_node: bool,
    // The lane the selected commit sits on. Every colour the graph draws goes
    // through `lane`, so that one lane stays saturated along its whole run and
    // every other lane recedes along its whole run -- a property of the lane, not
    // of which rows happen to connect to the selection.
    selected_lane: Option<SelectedLane>,
    // What the row is actually painted over, tints included. The icon nodes knock
    // their glyphs out in it, and the list's untinted surface is the wrong answer
    // on any row carrying a hover, selection or browse tint.
    row_background: gpui::Rgba,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui::PathBuilder;

    if row.lanes_now.is_empty() {
        return;
    }

    let lane = |color_ix| lane_wash_color(theme, color_ix, row_ix, selected_lane);

    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
    let stroke_width = scaled_px(1.6);
    let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
    let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
    let margin_right = scaled_px(HISTORY_GRAPH_MARGIN_RIGHT_PX);
    let node_radius = scaled_px(3.4);
    let node_corner_radius = scaled_px(2.0);

    let elbow_radius = scaled_px(HISTORY_GRAPH_ELBOW_RADIUS_PX);

    let y_top = bounds.top();
    let y_center = bounds.top() + bounds.size.height / 2.0;
    let y_bottom = bounds.bottom();

    // Columns past the edge all land on the edge x, icons included.
    let edge_x = graph_edge_x(margin_x, margin_right, bounds.size.width);
    let x_for_col = |col: usize| graph_col_x(col, margin_x, col_gap, edge_x);
    let left = bounds.left();

    let node_x = x_for_col(usize::from(row.node_col));
    let node_color = lane(row.node_color_ix);
    // Beside a node on the edge line, the line matches the node's dot and
    // gradient; elsewhere on it the selected lane is painted on top.
    let segment_color = |x: Pixels, color_ix| {
        if edge_takes_node_colour(node_x, x, edge_x) {
            node_color
        } else {
            lane(color_ix)
        }
    };
    let paints_last = |col: usize, color_ix| {
        edge_paint_last(same_x(x_for_col(col), edge_x), || {
            selected_lane.is_some_and(|lane| lane.covers(theme, row_ix, color_ix))
        })
    };
    let pin_col = pinned_from_col(margin_x, col_gap, edge_x);
    let selection_possible = selected_lane.is_some_and(|lane| lane.covers_row(row_ix));

    // Whether column `col` draws a vertical down from the top edge of this row.
    let has_incoming_vertical = |col: usize| {
        row.lanes_now
            .get(col)
            .is_some_and(|lane| lane.is_active() && lane.incoming())
            || connect_from_top_col == Some(col)
    };
    // A join whose source column also has an incoming vertical is drawn as one
    // continuous elbow, so the plain vertical pass must not draw it twice. A join
    // collapsed onto one x is no elbow; the vertical pass draws its lane.
    let joins_out_of = |col: usize| {
        row.joins_in.iter().any(|edge| {
            usize::from(edge.from_col) == col
                && !same_x(x_for_col(col), x_for_col(usize::from(edge.to_col)))
        })
    };

    // Incoming vertical segments.
    let incoming = coalesced_lanes(
        &row.lanes_now,
        pin_col,
        selection_possible,
        |col, lane| {
            lane.is_active()
                && (lane.incoming() || connect_from_top_col == Some(col))
                && !joins_out_of(col)
        },
        |col, lane| paints_last(col, lane.color_ix),
        (None, None),
    );
    for (col, lane_paint) in incoming {
        let x = x_for_col(col);
        let mut path = PathBuilder::stroke(stroke_width);
        path.move_to(point(left + x, y_top));
        path.line_to(point(left + x, y_center));
        if let Ok(p) = path.build() {
            gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
            window.paint_path(p, segment_color(x, lane_paint.color_ix));
        }
    }

    // Incoming join edges into the node (used both for merge commits and fork points).
    for edge in coalesced(
        row.joins_in.iter().copied(),
        |edge| {
            (
                x_key(x_for_col(usize::from(edge.from_col))),
                x_key(x_for_col(usize::from(edge.to_col))),
                has_incoming_vertical(usize::from(edge.from_col)),
            )
        },
        |_| false,
    ) {
        let from = usize::from(edge.from_col);
        if same_x(x_for_col(from), x_for_col(usize::from(edge.to_col))) {
            continue;
        }
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
                gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
                window.paint_path(p, color);
            }
        }
    }

    // Continuations from current row to next row.
    let continuing = continuing_winners(
        row,
        pin_col,
        same_x(node_x, edge_x),
        selection_possible,
        |col, lane| paints_last(col, lane.color_ix),
    );
    for (out_col, lane_paint) in continuing {
        let x_out = x_for_col(out_col);
        let color = segment_color(x_out, lane_paint.color_ix);
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
                gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
                window.paint_path(p, color);
            }
        }
    }

    // Additional merge edges from the node into lanes that were re-targeted to secondary parents.
    // Collapsed onto the node's x, the target lane's own continuation covers it.
    for edge in coalesced(
        row.edges_out.iter().copied(),
        |edge| x_key(x_for_col(usize::from(edge.to_col))),
        |_| false,
    ) {
        let x_to = x_for_col(usize::from(edge.to_col));
        if same_x(node_x, x_to) {
            continue;
        }
        paint_node_to_lane(
            left,
            node_x,
            x_to,
            y_center,
            y_bottom,
            elbow_radius,
            stroke_width,
            lane(edge.color_ix),
            window,
        );
    }

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
                row_background,
                node_color,
                window,
                cx,
            );
        } else if row.is_merge {
            paint_icon_node(
                bounds.left() + node_x,
                y_center,
                icons::GIT_MERGE_ICON_PATH,
                row_background,
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
#[cfg(test)]
pub(in crate::view) fn worktree_band_connect_from_top_col(
    plan: &crate::view::caches::HistoryListPlan,
    graph_rows: &[history_graph::GraphRow],
    worktree_dirty: &[gitcomet_core::domain::WorktreeDirtySummary],
    list_ix: usize,
) -> Option<usize> {
    worktree_band_connect_from_top_col_in_window(plan, graph_rows, worktree_dirty, list_ix, 0)
}

pub(in crate::view) fn worktree_band_connect_from_top_col_in_window(
    plan: &crate::view::caches::HistoryListPlan,
    graph_rows: &[history_graph::GraphRow],
    worktree_dirty: &[gitcomet_core::domain::WorktreeDirtySummary],
    list_ix: usize,
    graph_start: usize,
) -> Option<usize> {
    use crate::view::caches::HistoryListRow;

    match plan.row_at(list_ix.checked_sub(1)?) {
        Some(HistoryListRow::WorkingTreeSummary) => Some(0),
        Some(HistoryListRow::WorktreeUncommitted {
            visible_ix,
            worktree_ix,
        }) => {
            let above_row = graph_rows.get(visible_ix.checked_sub(graph_start)?)?;
            let above = worktree_dirty.get(worktree_ix)?;
            let on_branch = above.branch.is_some() && !above.detached;
            Some(usize::from(band_node_for(above_row, on_branch).exit_col))
        }
        _ => None,
    }
}

/// The lane the selection sits on: the one lane that keeps full colour.
///
/// A colour index alone does not identify a lane. `pick_lane_color_ix` only
/// avoids collisions between lanes that are alive *at the same time*, and it
/// recycles freely after that, so a lane that ended near the top of the page and
/// one born near the bottom routinely share an index. Matching on the index alone
/// lit both, and the highlight read as two disjoint chains with washed-out rows
/// between them. The row span pins it to the one lane: a lane's lifetime is a
/// contiguous run of rows, so the span plus the colour is unambiguous — no other
/// live lane can hold that colour anywhere inside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct SelectedLane {
    pub(in crate::view) color_ix: history_graph::LaneColorIx,
    /// First visible row the lane occupies in `lanes_now`.
    first_row: usize,
    /// Last such row, inclusive.
    last_row: usize,
}

impl SelectedLane {
    pub(in crate::view) fn relative_to(self, start: usize) -> Self {
        if self.last_row < start {
            Self::span(self.color_ix, usize::MAX - 1, usize::MAX - 1)
        } else {
            Self::span(
                self.color_ix,
                self.first_row.saturating_sub(start),
                self.last_row - start,
            )
        }
    }

    pub(in crate::view) fn span(
        color_ix: history_graph::LaneColorIx,
        first_row: usize,
        last_row: usize,
    ) -> Self {
        Self {
            color_ix,
            first_row,
            last_row,
        }
    }
    /// Whether visible row `row_ix` lies in the lane's span at all. False means
    /// no colour on the row can be the selected lane.
    pub(in crate::view) fn covers_row(self, row_ix: usize) -> bool {
        // `row_ix + 1 >= first_row` rather than `row_ix >= first_row - 1`, which
        // underflows at the top of the page. The slack row is the lane's birth
        // row: it draws the lane's lower half out of `lanes_next` one row above
        // the first row that carries it in `lanes_now`.
        row_ix + 1 >= self.first_row && row_ix <= self.last_row
    }
    /// Whether the lane drawn in `color_ix` on visible row `row_ix` is this one.
    pub(in crate::view) fn covers(
        self,
        theme: AppTheme,
        row_ix: usize,
        color_ix: history_graph::LaneColorIx,
    ) -> bool {
        // Resolved colours, not indices. `GraphLanePalette::color_at` wraps at the
        // palette's real length, so a theme supplying fewer colours than
        // `GRAPH_LANE_PALETTE_SIZE` maps distinct indices onto one RGB. Two lanes
        // the user cannot tell apart must not be drawn at two strengths on the
        // same row; washing them together is the only reading that holds up.
        if history_graph::lane_color(theme, color_ix)
            != history_graph::lane_color(theme, self.color_ix)
        {
            return false;
        }
        self.covers_row(row_ix)
    }
}

/// The lane `color_ix` occupies on `row`, whether it is carried into the row or
/// starts at its node. Live lanes hold distinct colours, so this is exact.
///
/// `lanes_next` is consulted first because it is the column the lane *continues*
/// in, and continuing is what [`selected_lane_at`] walks. `lanes_now` can hold
/// the same colour in a paint-only column: a branch head that has fallen behind
/// gets a fork whisker beside its node (`adopt_fork_color`), which exists on that
/// one row and carries the same colour as the real lane starting below it.
/// Resolving to the whisker collapsed the span to the anchor row alone, and every
/// row of the branch the highlight was meant to light washed out instead.
pub(in crate::view) fn lane_col_for_color(
    row: &history_graph::GraphRow,
    color_ix: history_graph::LaneColorIx,
) -> Option<usize> {
    let matching = |lanes: &[history_graph::LanePaint]| {
        lanes
            .iter()
            .position(|lane| lane.is_active() && lane.color_ix == color_ix)
    };
    matching(&row.lanes_next).or_else(|| matching(&row.lanes_now))
}

/// Resolves the lane drawn in `color_ix` at `anchor_row` to its full row span.
///
/// Walks outwards while the lane's column keeps carrying its colour. A lane holds
/// one column for its whole life and a new lane may not reuse a colour that ended
/// on the row above it (`ended_colors` in `compute_graph`), so the run this finds
/// starts and stops exactly where the lane does.
pub(in crate::view) fn selected_lane_at(
    graph_rows: &[history_graph::GraphRow],
    anchor_row: usize,
    color_ix: history_graph::LaneColorIx,
) -> Option<SelectedLane> {
    let col = lane_col_for_color(graph_rows.get(anchor_row)?, color_ix)?;
    let occupies = |row_ix: usize| {
        graph_rows
            .get(row_ix)
            .and_then(|row| row.lanes_now.get(col))
            .is_some_and(|lane| lane.is_active() && lane.color_ix == color_ix)
    };

    // A lane that starts at this row's node is only in `lanes_next` here; the
    // first row carrying it is the next one down.
    let start = if occupies(anchor_row) {
        anchor_row
    } else {
        anchor_row + 1
    };
    if !occupies(start) {
        // Nothing below carries it -- the lane ends here, or the page does.
        return Some(SelectedLane {
            color_ix,
            first_row: anchor_row,
            last_row: anchor_row,
        });
    }

    let mut first_row = start;
    while first_row > 0 && occupies(first_row - 1) {
        first_row -= 1;
    }
    let mut last_row = start;
    while occupies(last_row + 1) {
        last_row += 1;
    }
    Some(SelectedLane {
        color_ix,
        first_row,
        last_row,
    })
}

/// A lane's colour on row `row_ix`, washed out unless it is the selected lane.
///
/// The wash is a property of the *lane*, so a washed lane stays washed from top
/// to bottom rather than flickering wherever it happens to touch the selected
/// chain. `None` -- nothing selected -- leaves every lane at full strength.
pub(super) fn lane_wash_color(
    theme: AppTheme,
    color_ix: history_graph::LaneColorIx,
    row_ix: usize,
    selected: Option<SelectedLane>,
) -> gpui::Rgba {
    let full = history_graph::lane_color(theme, color_ix);
    match selected {
        // The same mix the unrelated-row dimming uses, so the two read alike.
        Some(selected) if !selected.covers(theme, row_ix, color_ix) => {
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

/// Which halves of a band row a lane occupies. The painter resolves the same
/// halves lane by lane; this stays as the reference for its tests.
#[cfg(test)]
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
#[cfg(test)]
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
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_history_graph_band(
    theme: AppTheme,
    lanes: &[history_graph::LanePaint],
    // Index of the commit row *below* the band, whose lanes it draws.
    row_ix: usize,
    connect_from_top_col: Option<usize>,
    selected_lane: Option<SelectedLane>,
    node: BandNodePaint,
    show_graph_color_marker: bool,
    // What the row is painted over, so the node's middle -- which has to be
    // opaque, or the lane through its column shows inside it -- matches. The
    // band's hover tint comes from a `div().hover()` the canvas cannot observe,
    // so this covers the row's persistent state (selection) only.
    row_background: gpui::Rgba,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui::PathBuilder;

    if lanes.is_empty() {
        return;
    }

    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
    let stroke_width = scaled_px(1.6);
    let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
    let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
    let elbow_radius = scaled_px(HISTORY_GRAPH_ELBOW_RADIUS_PX);

    let left = bounds.left();
    let y_top = bounds.top();
    let y_center = bounds.top() + bounds.size.height / 2.0;
    let y_bottom = bounds.bottom();
    // Same edge pinning as the commit rows, or the edge line breaks at a band.
    let edge_x = graph_edge_x(
        margin_x,
        scaled_px(HISTORY_GRAPH_MARGIN_RIGHT_PX),
        bounds.size.width,
    );
    let x_for_col = |col: usize| graph_col_x(col, margin_x, col_gap, edge_x);

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

    // A pushed-out node sits one column past the last lane, which in a narrow
    // column is the edge x.
    let node_x_offset = x_for_col(usize::from(node.col));

    // The same halves `band_lane_segments` describes, resolved lane by lane so
    // a wide frontier costs the band only its displayed columns.
    let pin_col = pinned_from_col(margin_x, col_gap, edge_x);
    let selection_possible = selected_lane.is_some_and(|lane| lane.covers_row(row_ix));
    let node_col = usize::from(node.col);
    let passes_through = |col: usize, lane: history_graph::LanePaint| {
        lane.incoming() || connect_from_top_col == Some(col)
    };
    for top in [true, false] {
        let winners = coalesced_lanes(
            lanes,
            pin_col,
            selection_possible,
            |col, lane| {
                lane.is_active() && (passes_through(col, lane) || (!top && col == node_col))
            },
            |col, lane| {
                edge_paint_last(same_x(x_for_col(col), edge_x), || {
                    selected_lane
                        .is_some_and(|lane_sel| lane_sel.covers(theme, row_ix, lane.color_ix))
                })
            },
            (None, None),
        );
        for (col, lane) in winners {
            let x = x_for_col(col);
            let from_y = if top { y_top } else { y_center };
            let to_y = if !top { y_bottom } else { y_center };
            let mut path = PathBuilder::stroke(stroke_width);
            path.move_to(point(left + x, from_y));
            path.line_to(point(left + x, to_y));
            if let Ok(p) = path.build() {
                // The lane's own colour, not the node's: a branch head keeps the
                // descendant lane's colour above the node, and the commit below
                // paints its matching stub the same way -- including its wash, or the
                // seam between the two rows reappears. The exception is the edge line
                // beside a node on it, which matches the node like a commit row's.
                let color = if edge_takes_node_colour(node_x_offset, x, edge_x) {
                    node.color
                } else {
                    lane_wash_color(theme, lane.color_ix, row_ix, selected_lane)
                };
                gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
                window.paint_path(p, color);
            }
        }
    }

    // The node sits on a column no lane runs through, so it reaches the commit
    // below by leaving horizontally and turning down -- the same shape a branch
    // head's whisker takes. Both ends are pinned, so one past the edge collapses
    // into a straight drop instead of turning out through the clipped edge.
    if let Some(exit_col) = node.exit_col {
        paint_node_to_lane(
            left,
            node_x_offset,
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
    let node_x = left + node_x_offset;
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
            row_background,
            window,
            cx,
        );
    });
}

/// Where column `col` is drawn, in pixels from the graph cell's left edge.
///
/// The column's width is fixed rather than fitted to the lanes, and the cell
/// clips, so every column past `edge_x` (see [`graph_edge_x`]) is pinned to it:
/// its lanes merge into one edge line and its nodes sit on it instead of
/// vanishing.
fn graph_col_x(col: usize, margin_x: Pixels, col_gap: Pixels, edge_x: Pixels) -> Pixels {
    (margin_x + col_gap * (col as f32)).min(edge_x)
}

/// The right-most x a lane or node is drawn at: `margin_right` in from the edge,
/// clear of the message border, but never left of column 0.
fn graph_edge_x(margin_x: Pixels, margin_right: Pixels, width: Pixels) -> Pixels {
    (width - margin_right).max(margin_x)
}

/// Whether two x-offsets draw as one line. A connector between them is no elbow.
fn same_x(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(0.5)
}

/// Paint order key for a lane. Lanes pinned to the edge share one x and the last
/// one painted wins, so there the selected lane goes last. Everywhere else lanes
/// keep column order.
fn edge_paint_last(on_edge: bool, selected: impl FnOnce() -> bool) -> bool {
    on_edge && selected()
}

/// Whether a segment at `x` takes the node's colour instead of its lane's: both
/// sit on the edge line, so the collapsed line matches the node's dot and
/// gradient rather than whichever lane was painted last.
fn edge_takes_node_colour(node_x: Pixels, x: Pixels, edge_x: Pixels) -> bool {
    same_x(node_x, edge_x) && same_x(x, edge_x)
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
    if same_x(x_from, x_to) {
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
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
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
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
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
    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
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
/// the node's column does not show through the hole. `background` must therefore
/// be what the row is actually painted over, tints included -- the list's own
/// surface is only right on an untinted row.
pub(super) fn paint_ring_icon_node(
    x_center: Pixels,
    y_center: Pixels,
    icon_path: &'static str,
    ring_color: gpui::Rgba,
    background: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) {
    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
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
            from_node_cols: history_graph::from_node_cols_of(lanes),
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

    /// The fork whisker beside a behind-branch's node carries the branch's colour
    /// on that row alone, while the lane the colour really belongs to starts one
    /// column over in `lanes_next`. Resolving the highlight to the whisker pinned
    /// it to a single row, so `covers` said no for every row below and the whole
    /// branch washed out -- the failure `SelectedLane` exists to prevent.
    #[test]
    fn a_fork_whisker_does_not_collapse_the_highlight_to_one_row() {
        let theme = AppTheme::gitcomet_dark();
        // The head commit: its own lane (colour 1) runs on to its descendant,
        // the branch's new lane (colour 7) is born at the node in that same
        // column, and the whisker marking the head sits beside it.
        let anchor = GraphRow {
            lanes_now: [incoming(1), born(7)].into_iter().collect(),
            lanes_next: [LanePaint::lane(7, false, true)].into_iter().collect(),
            joins_in: [edge(1, 0, 7)].into_iter().collect(),
            edges_out: Default::default(),
            node_col: 0,
            node_color_ix: 7,
            is_merge: false,
            from_node_cols: [0].into_iter().collect(),
        };
        let below = || GraphRow {
            lanes_now: [incoming(7)].into_iter().collect(),
            lanes_next: [incoming(7)].into_iter().collect(),
            joins_in: Default::default(),
            edges_out: Default::default(),
            node_col: 0,
            node_color_ix: 7,
            is_merge: false,
            from_node_cols: Default::default(),
        };
        let rows = [anchor, below(), below()];

        let selected = selected_lane_at(&rows, 0, 7).expect("the lane resolves");
        assert_eq!(
            (selected.first_row, selected.last_row),
            (1, 2),
            "the span must follow the continuing lane, not the one-row whisker"
        );
        for row_ix in 0..rows.len() {
            assert!(
                selected.covers(theme, row_ix, 7),
                "row {row_ix} is on the selected branch and must stay lit"
            );
        }
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
            line_stats: Default::default(),
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

    /// A lane spanning every row of a short page, for wash tests that are about
    /// the colour rather than the span.
    fn whole_page_lane(color_ix: history_graph::LaneColorIx) -> SelectedLane {
        SelectedLane {
            color_ix,
            first_row: 0,
            last_row: usize::MAX,
        }
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
            lane_wash_color(theme, other, 0, None),
            history_graph::lane_color(theme, other),
            "with nothing selected every lane stays at full strength"
        );
        assert_eq!(
            lane_wash_color(theme, selected, 0, Some(whole_page_lane(selected))),
            history_graph::lane_color(theme, selected),
            "the selected commit's own lane is never washed"
        );

        let washed = lane_wash_color(theme, other, 0, Some(whole_page_lane(selected)));
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
            washed.alpha, 1.0,
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
            lane_wash_color(
                theme,
                row.node_color_ix,
                0,
                Some(whole_page_lane(row.node_color_ix))
            ),
            history_graph::lane_color(theme, row.node_color_ix)
        );
        assert_ne!(
            lane_wash_color(
                theme,
                row.node_color_ix,
                0,
                Some(whole_page_lane(row.node_color_ix + 1))
            ),
            history_graph::lane_color(theme, row.node_color_ix)
        );
    }

    /// The colour index is not a lane id. `pick_lane_color_ix` only avoids
    /// collisions between lanes alive at the same time and recycles freely after
    /// that, so a lane that ended near the top of a page and one born near the
    /// bottom routinely share an index. Matching on the index alone lit both, and
    /// the highlight read as two disjoint chains.
    #[test]
    fn a_lane_that_recycled_the_selected_colour_still_washes() {
        let theme = AppTheme::gitcomet_dark();
        let color_ix = 4u8;
        let selected = SelectedLane {
            color_ix,
            first_row: 10,
            last_row: 20,
        };

        for row_ix in [9usize, 10, 15, 20] {
            assert_eq!(
                lane_wash_color(theme, color_ix, row_ix, Some(selected)),
                history_graph::lane_color(theme, color_ix),
                "row {row_ix} is inside the selected lane's run (plus its birth row)"
            );
        }
        for row_ix in [0usize, 8, 21, 400] {
            assert_ne!(
                lane_wash_color(theme, color_ix, row_ix, Some(selected)),
                history_graph::lane_color(theme, color_ix),
                "row {row_ix} is a different lane that happens to share the colour"
            );
        }
    }

    /// `GraphLanePalette::color_at` wraps at the palette's real length, so a theme
    /// supplying fewer colours than `GRAPH_LANE_PALETTE_SIZE` maps distinct lane
    /// indices onto one RGB. Washing one and not the other put the same hue on the
    /// same row at two strengths, which reads as a rendering fault rather than a
    /// highlight.
    #[test]
    fn lanes_a_short_palette_paints_alike_wash_alike() {
        let red = gpui::rgba(0xff0000ff);
        let blue = gpui::rgba(0x0000ffff);
        let mut theme = AppTheme::gitcomet_dark();
        theme.graph_lane_palette = crate::theme::GraphLanePalette::leaked_for_test(&[red, blue]);

        // Index 2 wraps back onto index 0's colour.
        assert_eq!(
            history_graph::lane_color(theme, 0),
            history_graph::lane_color(theme, 2),
            "fixture must actually produce a collision"
        );

        let selected = SelectedLane {
            color_ix: 0,
            first_row: 0,
            last_row: 10,
        };
        assert_eq!(
            lane_wash_color(theme, 2, 5, Some(selected)),
            history_graph::lane_color(theme, 2),
            "a lane the theme paints identically to the selected one must not be \
             drawn at a second strength"
        );
        assert_ne!(
            lane_wash_color(theme, 1, 5, Some(selected)),
            history_graph::lane_color(theme, 1),
            "a lane the theme really does paint differently still washes"
        );
    }

    /// A lane's lifetime is a contiguous run of rows at one column, so the span
    /// is found by walking outwards from the anchor. Two same-coloured lanes with
    /// a gap between them must resolve to the one the anchor is on.
    #[test]
    fn a_lanes_span_stops_where_the_lane_does() {
        let color_ix = 6u8;
        let rows = vec![
            band(&[incoming(color_ix)], 0),
            band(&[incoming(color_ix)], 0),
            // The lane ends; the column is a hole for one row.
            band(&[history_graph::LanePaint::HOLE], 0),
            // A different lane recycles the colour further down.
            band(&[incoming(color_ix)], 0),
        ];

        let top = selected_lane_at(&rows, 0, color_ix).expect("the anchor is on a lane");
        assert_eq!((top.first_row, top.last_row), (0, 1));
        let bottom = selected_lane_at(&rows, 3, color_ix).expect("the anchor is on a lane");
        assert_eq!((bottom.first_row, bottom.last_row), (3, 3));

        let theme = AppTheme::gitcomet_dark();
        assert_ne!(
            lane_wash_color(theme, color_ix, 3, Some(top)),
            history_graph::lane_color(theme, color_ix),
            "selecting the top lane must not light the recycled one below"
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

    /// In a column narrower than the graph, the node's natural column is outside
    /// a cell that clips rather than overflows, so it would not be drawn at all.
    /// It has to come back inside.
    #[test]
    fn a_pushed_out_node_stays_inside_a_column_too_narrow_for_it() {
        let clamped_width = px(crate::view::HISTORY_COL_GRAPH_MAX_PX);

        // 20 lanes want x = 332 in a column the clamp holds at 240.
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

#[cfg(test)]
mod coalescing_regressions {
    use super::*;

    #[gpui::test]
    fn indexed_history_actual_paint_paths_are_bounded_by_displayed_columns(
        cx: &mut gpui::TestAppContext,
    ) {
        use gitcomet_core::history_perf::{self, Work};
        let _guard = crate::test_support::lock_visual_test();
        let cx = cx.add_empty_window();
        for pixels in [32.0, 80.0, 240.0] {
            for band in [false, true] {
                let mut counts = Vec::new();
                for width in [64, 512, 5261] {
                    let row = history_graph::GraphRow {
                        lanes_now: (0..width)
                            .map(|col| {
                                history_graph::LanePaint::lane((col % 12) as u8, true, false)
                            })
                            .collect(),
                        lanes_next: (0..width)
                            .map(|col| {
                                history_graph::LanePaint::lane((col % 12) as u8, false, false)
                            })
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
                                            background,
                                            bounds,
                                            window,
                                            app,
                                        );
                                    } else {
                                        paint_history_graph(
                                            theme, &row, 10, None, false, selected, background,
                                            bounds, window, app,
                                        );
                                    }
                                },
                            )
                            .w(px(pixels))
                            .h(px(28.0))
                        },
                    );
                    counts.push(history_perf::count(Work::PaintPath));
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
}

#[cfg(test)]
mod lane_coalescing_regressions {
    use super::*;
    use history_graph::LanePaint;

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// Every lane winner the generic hash-based coalescing produces, in the same
    /// order, from the displayed-column scan: incoming and continuing passes of a
    /// commit row and both halves of a band row, over random frontiers, widths
    /// with sub-pixel edges, joins, connectors, node positions and selections.
    #[test]
    fn displayed_column_coalescing_matches_generic_coalescing() {
        let margin_x = px(HISTORY_GRAPH_MARGIN_X_PX);
        let col_gap = px(HISTORY_GRAPH_COL_GAP_PX);
        let margin_right = px(HISTORY_GRAPH_MARGIN_RIGHT_PX);
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for case in 0..4000usize {
            let len = (xorshift(&mut state) % 48) as usize + if case % 9 == 0 { 400 } else { 0 };
            let lanes: Vec<LanePaint> = (0..len)
                .map(|_| {
                    let r = xorshift(&mut state);
                    if r % 5 == 0 {
                        LanePaint::HOLE
                    } else {
                        LanePaint::lane((r % 6) as u8, r % 3 != 0, r % 4 == 0)
                    }
                })
                .collect();
            let width = px((xorshift(&mut state) % 8000) as f32 / 10.0 + 4.0);
            let edge_x = graph_edge_x(margin_x, margin_right, width);
            let x_for_col = |col: usize| graph_col_x(col, margin_x, col_gap, edge_x);
            let pin_col = pinned_from_col(margin_x, col_gap, edge_x);
            for col in 0..len.max(pin_col.saturating_add(2).min(len + 2)) {
                assert_eq!(
                    x_for_col(col) == edge_x,
                    col >= pin_col,
                    "case {case}: column {col} pin {pin_col} width {width:?}"
                );
            }
            let node_col = if len == 0 {
                0
            } else {
                (xorshift(&mut state) % len as u64) as usize
            };
            let node_x = x_for_col(node_col);
            let joins_out: Vec<usize> = (0..xorshift(&mut state) % 3)
                .map(|_| (xorshift(&mut state) % len.max(1) as u64) as usize)
                .collect();
            let joins_out_of = |col: usize| joins_out.contains(&col);
            let connect = (xorshift(&mut state) % 2 == 0)
                .then(|| (xorshift(&mut state) % len.max(1) as u64) as usize);
            let selection: Option<u8> = match xorshift(&mut state) % 3 {
                0 => None,
                1 => Some((xorshift(&mut state) % 6) as u8),
                // A colour absent from the frontier: the scan must run out cleanly.
                _ => Some(9),
            };
            let paints_last = |col: usize, color_ix: u8| {
                edge_paint_last(same_x(x_for_col(col), edge_x), || {
                    selection == Some(color_ix)
                })
            };
            let selection_possible = selection.is_some();

            let include = |col: usize, lane: LanePaint| {
                lane.is_active() && (lane.incoming() || connect == Some(col)) && !joins_out_of(col)
            };
            let expected = coalesced(
                lanes
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|&(col, lane)| include(col, lane)),
                |(col, _)| x_key(x_for_col(col)),
                |(col, lane)| paints_last(col, lane.color_ix),
            );
            let actual = coalesced_lanes(
                &lanes,
                pin_col,
                selection_possible,
                include,
                |col, lane| paints_last(col, lane.color_ix),
                (None, None),
            );
            assert_eq!(
                actual.as_slice(),
                expected.as_slice(),
                "incoming case {case}"
            );

            let row = history_graph::GraphRow {
                lanes_now: lanes.iter().copied().collect(),
                lanes_next: lanes.iter().copied().collect(),
                joins_in: Default::default(),
                edges_out: Default::default(),
                node_col: node_col as u16,
                node_color_ix: 0,
                is_merge: false,
                from_node_cols: history_graph::from_node_cols_of(&lanes),
            };
            let expected = coalesced(
                lanes
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(_, lane)| lane.is_active()),
                |(col, lane)| {
                    (
                        x_key(x_for_col(col)),
                        lane.starts_at_node() && !same_x(node_x, x_for_col(col)),
                    )
                },
                |(col, lane)| paints_last(col, lane.color_ix),
            );
            let actual = continuing_winners(
                &row,
                pin_col,
                same_x(node_x, edge_x),
                selection_possible,
                |col, lane| paints_last(col, lane.color_ix),
            );
            assert_eq!(
                actual.as_slice(),
                expected.as_slice(),
                "continuing case {case}"
            );

            let segments = band_lane_segments(&lanes, node_col, connect);
            for top in [true, false] {
                let expected: Vec<(usize, u8)> = coalesced(
                    segments.iter().copied().filter(|segment| {
                        if top {
                            segment.has_top
                        } else {
                            segment.has_bottom
                        }
                    }),
                    |segment| x_key(x_for_col(segment.col)),
                    |segment| paints_last(segment.col, segment.color_ix),
                )
                .into_iter()
                .map(|segment| (segment.col, segment.color_ix))
                .collect();
                let actual: Vec<(usize, u8)> = coalesced_lanes(
                    &lanes,
                    pin_col,
                    selection_possible,
                    |col, lane| {
                        lane.is_active()
                            && (lane.incoming()
                                || connect == Some(col)
                                || (!top && col == node_col))
                    },
                    |col, lane| paints_last(col, lane.color_ix),
                    (None, None),
                )
                .into_iter()
                .map(|(col, lane)| (col, lane.color_ix))
                .collect();
                assert_eq!(actual, expected, "band top={top} case {case}");
            }
        }
    }

    /// The scan stops at the displayed columns plus the edge winner: a wide
    /// frontier no longer costs every lane, in either pass, selected or not.
    #[test]
    fn displayed_column_coalescing_visits_only_displayed_columns_and_the_edge_winner() {
        let margin_x = px(HISTORY_GRAPH_MARGIN_X_PX);
        let col_gap = px(HISTORY_GRAPH_COL_GAP_PX);
        let edge_x = graph_edge_x(
            margin_x,
            px(HISTORY_GRAPH_MARGIN_RIGHT_PX),
            px(crate::view::HISTORY_COL_GRAPH_PX),
        );
        let pin_col = pinned_from_col(margin_x, col_gap, edge_x);
        assert!(
            pin_col < 8,
            "default width shows a handful of columns: {pin_col}"
        );
        let lanes: Vec<LanePaint> = (0..5261)
            .map(|col| LanePaint::lane((col % 64) as u8, true, false))
            .collect();
        let last_color = lanes.last().unwrap().color_ix;
        for selection in [None, Some(last_color)] {
            let visited = std::cell::Cell::new(0usize);
            let winners = coalesced_lanes(
                &lanes,
                pin_col,
                selection.is_some(),
                |_, lane| {
                    visited.set(visited.get() + 1);
                    lane.is_active() && lane.incoming()
                },
                |col, lane| col >= pin_col && selection == Some(lane.color_ix),
                (None, None),
            );
            assert_eq!(winners.len(), pin_col + 1, "{selection:?}");
            assert_eq!(winners.last().map(|(col, _)| *col), Some(5260));
            assert!(
                visited.get() <= pin_col + 1,
                "{selection:?}: visited {} lanes",
                visited.get()
            );
            let row = history_graph::GraphRow {
                lanes_now: lanes.iter().copied().collect(),
                lanes_next: lanes.iter().copied().collect(),
                joins_in: Default::default(),
                edges_out: Default::default(),
                node_col: 0,
                node_color_ix: 0,
                is_merge: false,
                from_node_cols: Default::default(),
            };
            let continuing =
                continuing_winners(&row, pin_col, false, selection.is_some(), |col, lane| {
                    col >= pin_col && selection == Some(lane.color_ix)
                });
            assert_eq!(continuing.len(), pin_col + 1, "{selection:?}");
        }
    }
}
