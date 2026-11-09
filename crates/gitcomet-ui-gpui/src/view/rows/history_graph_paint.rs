use super::*;
use gpui::{App, Bounds, PaintQuad, Pixels, Window, point, px, size};
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
/// scan of the pinned columns. Lanes `leaves_node` claims are left out: the
/// caller draws them out of the node with the merge edges, and leaving them out
/// here rather than after coalescing keeps them from winning an edge line other
/// lanes still need drawn straight.
fn continuing_winners(
    row: &history_graph::GraphRow,
    pin_col: usize,
    node_pinned: bool,
    draw_reserved: bool,
    selection_possible: bool,
    selected: impl Fn(usize, history_graph::LanePaint) -> bool,
    leaves_node: impl Fn(usize, history_graph::LanePaint) -> bool,
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
        |col, lane| {
            lane.is_active()
                && (draw_reserved || !lane.reserved())
                && !(col >= pin_col && !node_pinned && lane.starts_at_node())
                && !leaves_node(col, lane)
        },
        selected,
        extra,
    )
}

/// Shared with ref leaders and author hover regions, including the pinned edge
/// position used when the graph column cannot display every lane separately.
pub(super) fn graph_node_bounds(
    row: &history_graph::GraphRow,
    is_stash: bool,
    initials_reveal: f32,
    bounds: Bounds<Pixels>,
    grid: GraphGrid,
    scale: ui_scale::UiScale,
) -> Bounds<Pixels> {
    let diameter = grid.even(if is_stash || row.is_merge {
        super::history_node::icon_diameter(scale)
    } else {
        super::history_node::author_diameter(scale, initials_reveal)
    });
    let x = grid.snap(bounds.left())
        + grid.col_x(usize::from(row.node_col), grid.edge_x(bounds.size.width));
    let (_, y, _) = grid.row_ys(bounds);
    Bounds::new(
        point(x - diameter * 0.5, y - diameter * 0.5),
        size(diameter, diameter),
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
    // Whether the HEAD lane's reserved run is drawn: only while the
    // uncommitted-changes row it leads up to is shown.
    draw_reserved: bool,
    is_stash_node: bool,
    // The lane the selected commit sits on. Every colour the graph draws goes
    // through `lane`, so that one lane stays saturated along its whole run and
    // every other lane recedes along its whole run -- a property of the lane, not
    // of which rows happen to connect to the selection.
    selected_lane: Option<SelectedLane>,
    // What the row is actually painted over, tints included. Node fills composite
    // their lane tint over it so they match hovered, selected and browsed rows.
    row_visual: history_canvas::HistoryRowVisual,
    author: &HistoryAuthorVm,
    initials_reveal: f32,
    selection: f32,
    bounds: Bounds<Pixels>,
    grid: GraphGrid,
    node_bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    if row.lanes_now.is_empty() {
        return;
    }

    let lane = |color_ix| lane_wash_color(theme, color_ix, row_ix, selected_lane);

    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
    let stroke_width = grid.stroke_width;
    let col_gap = grid.col_gap;
    let margin_x = grid.margin_x;

    let elbow_radius = grid.elbow_radius;

    let (y_top, y_center, y_bottom) = grid.row_ys(bounds);

    // Columns past the edge all land on the edge x, icons included.
    let edge_x = grid.edge_x(bounds.size.width);
    let x_for_col = |col: usize| grid.col_x(col, edge_x);
    let left = grid.snap(bounds.left());

    let node_x = x_for_col(usize::from(row.node_col));
    let node_color = row_visual.lane;
    let row_background = row_visual.background;
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

    let enters_from_top = |col: usize, lane: history_graph::LanePaint| {
        lane.is_active()
            && (lane.incoming() && (draw_reserved || !lane.reserved())
                || connect_from_top_col == Some(col))
    };
    // Whether column `col` draws a vertical down from the top edge of this row.
    let has_incoming_vertical = |col: usize| {
        row.lanes_now
            .get(col)
            .is_some_and(|&lane| enters_from_top(col, lane))
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
        |col, lane| enters_from_top(col, lane) && !joins_out_of(col),
        |col, lane| paints_last(col, lane.color_ix),
        (None, None),
    );
    for (col, lane_paint) in incoming {
        let x = x_for_col(col);
        paint_vertical_segment(
            left + x,
            y_top,
            y_center,
            stroke_width,
            segment_color(x, lane_paint.color_ix),
            window,
        );
    }

    // Draw joins only when a lane actually reaches the node from above,
    // including linked-worktree connectors. Ref leaders already mark branch
    // tips, so the old horizontal fork stubs no longer need to be painted.
    for edge in coalesced(
        row.joins_in
            .iter()
            .copied()
            .filter(|edge| has_incoming_vertical(usize::from(edge.from_col))),
        |edge| {
            (
                x_key(x_for_col(usize::from(edge.from_col))),
                x_key(x_for_col(usize::from(edge.to_col))),
            )
        },
        |_| false,
    ) {
        let from = usize::from(edge.from_col);
        if same_x(x_for_col(from), x_for_col(usize::from(edge.to_col))) {
            continue;
        }
        let color = lane(edge.color_ix);
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
    }

    // A merge hands each lane that reached it from another column on to one of
    // its parents, so such a lane joins the node and then carries on down its own
    // column. Above the centre line it has already turned into the node, which
    // leaves nothing on its column there: a straight run down from the centre
    // would start out of thin air, a flat-topped stub beside the join's elbow.
    // It leaves the node the way it arrived instead, through an elbow drawn with
    // the merge edges below. Only a merge hands lanes on (every other row ends the
    // lanes that join it), so ordinary rows skip the join scan for each lane.
    let leaves_node = |col: usize, lane: history_graph::LanePaint| {
        row.is_merge && !lane.starts_at_node() && joins_out_of(col)
    };

    // Continuations from current row to next row.
    let continuing = continuing_winners(
        row,
        pin_col,
        same_x(node_x, edge_x),
        draw_reserved,
        selection_possible,
        |col, lane| paints_last(col, lane.color_ix),
        leaves_node,
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
            paint_vertical_segment(
                left + x_out,
                y_center,
                y_bottom,
                stroke_width,
                color,
                window,
            );
        }
    }

    // The lanes `leaves_node` kept out of the continuations: exactly the ones the
    // continuation pass would otherwise have drawn straight down.
    let rejoined = row.joins_in.iter().filter_map(|edge| {
        let col = usize::from(edge.from_col);
        let lane = *row.lanes_next.get(col)?;
        (lane.is_active() && (draw_reserved || !lane.reserved()) && leaves_node(col, lane))
            .then_some(history_graph::GraphEdge {
                from_col: row.node_col,
                to_col: edge.from_col,
                color_ix: lane.color_ix,
            })
    });

    // Additional merge edges from the node into lanes that were re-targeted to secondary parents.
    // Collapsed onto the node's x, the target lane's own continuation covers it. A
    // merge edge into a lane that also joined the node shares its elbow, so the
    // two coalesce into one stroke.
    for edge in coalesced(
        row.edges_out.iter().copied().chain(rejoined),
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
    // its bounds include the author disc and its soft selection glow.
    let node_layer_half = node_bounds.size.width * 0.5 + scaled_px(8.0);
    let node_layer_bounds = Bounds::new(
        point(left + node_x - node_layer_half, y_center - node_layer_half),
        size(node_layer_half * 2.0, node_layer_half * 2.0),
    );
    window.paint_layer(node_layer_bounds, |window| {
        super::history_node::paint_selection_glow(
            theme,
            node_bounds,
            bounds,
            node_color,
            selection,
            window,
        );
        if is_stash_node {
            paint_icon_node(
                ui_scale::UiScale::from_window(window).with_appearance(theme.metrics),
                node_bounds,
                icons::GIT_STASH_NODE_ICON_PATH,
                row_background,
                node_color,
                window,
                cx,
            );
        } else if row.is_merge {
            paint_icon_node(
                ui_scale::UiScale::from_window(window).with_appearance(theme.metrics),
                node_bounds,
                icons::GIT_MERGE_ICON_PATH,
                row_background,
                node_color,
                window,
                cx,
            );
        } else {
            history_canvas::paint_history_author_node(
                theme,
                node_bounds,
                row_visual,
                author,
                initials_reveal,
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
    opacity: f32,
    window: &mut Window,
) {
    if graph_bounds.size.width <= px(0.0) || opacity <= 0.0 {
        return;
    }
    let fade_w = graph_bounds.size.width.min(fade_width);
    window.paint_quad(
        PaintQuad::new(Bounds::new(
            point(graph_bounds.right() - fade_w, graph_bounds.top()),
            size(fade_w, graph_bounds.size.height),
        ))
        .background(gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(with_alpha(color, 0.0), 0.0),
            gpui::linear_color_stop(with_alpha(color, opacity), 1.0),
        )),
    );
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
    draw_reserved: bool,
    selected_lane: Option<SelectedLane>,
    node: BandNodePaint,
    show_graph_color_marker: bool,
    graph_fade_opacity: f32,
    // What the row is painted over, so the node's middle -- which has to be
    // opaque, or the lane through its column shows inside it -- matches. The
    // caller resolves the row state and hover with the common interaction policy.
    row_background: gpui::Rgba,
    selection: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    if lanes.is_empty() {
        return;
    }

    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
    let grid = GraphGrid::new(
        window,
        ui_scale::UiScale::from_window(window).with_appearance(theme.metrics),
    );
    let stroke_width = grid.stroke_width;
    let col_gap = grid.col_gap;
    let margin_x = grid.margin_x;
    let elbow_radius = grid.elbow_radius;

    let left = grid.snap(bounds.left());
    let (y_top, y_center, y_bottom) = grid.row_ys(bounds);
    // Same edge pinning as the commit rows, or the edge line breaks at a band.
    let edge_x = grid.edge_x(bounds.size.width);
    let x_for_col = |col: usize| grid.col_x(col, edge_x);

    // The same wash the commit rows carry into their message border, painted
    // before the lanes so the strokes stay crisp on top of it.
    if show_graph_color_marker {
        paint_graph_fade(
            node.color,
            bounds,
            scaled_px(HISTORY_GRAPH_FADE_WIDTH_PX),
            graph_fade_opacity,
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
        lane.incoming() && (draw_reserved || !lane.reserved()) || connect_from_top_col == Some(col)
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
            paint_vertical_segment(left + x, from_y, to_y, stroke_width, color, window);
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

    paint_uncommitted_node(
        theme,
        grid,
        point(left + node_x_offset, y_center),
        node.color,
        row_background,
        bounds,
        selection,
        window,
        cx,
    );
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

/// Design width of a lane stroke.
const LANE_STROKE_PX: f32 = 1.6;

/// Lane geometry on the device-pixel grid. gpui snaps quads (straight runs) but
/// not paths (elbows), so off-grid geometry drew one lane at two widths.
#[derive(Clone, Copy, Debug)]
pub(super) struct GraphGrid {
    scale_factor: f32,
    pub(super) stroke_width: Pixels,
    pub(super) margin_x: Pixels,
    pub(super) col_gap: Pixels,
    margin_right: Pixels,
    pub(super) elbow_radius: Pixels,
}

impl GraphGrid {
    pub(super) fn new(window: &Window, scale: ui_scale::UiScale) -> Self {
        Self::for_scale(window.scale_factor(), scale)
    }

    fn for_scale(scale_factor: f32, scale: ui_scale::UiScale) -> Self {
        let growth = super::history_node::graph_font_growth(scale);
        let snap = |value: Pixels| snap_to_device(value, scale_factor);
        Self {
            scale_factor,
            stroke_width: even_device_px(scale.px(LANE_STROKE_PX), scale_factor),
            margin_x: snap(scale.px(HISTORY_GRAPH_MARGIN_X_PX) + growth * 0.5),
            col_gap: snap(scale.px(HISTORY_GRAPH_COL_GAP_PX) + growth),
            margin_right: snap(scale.px(HISTORY_GRAPH_MARGIN_RIGHT_PX) + growth * 0.5),
            elbow_radius: scale.px(HISTORY_GRAPH_ELBOW_RADIUS_PX),
        }
    }

    pub(super) fn snap(self, value: Pixels) -> Pixels {
        snap_to_device(value, self.scale_factor)
    }

    fn even(self, value: Pixels) -> Pixels {
        even_device_px(value, self.scale_factor)
    }

    pub(super) fn edge_x(self, width: Pixels) -> Pixels {
        self.snap(graph_edge_x(self.margin_x, self.margin_right, width))
    }

    pub(super) fn col_x(self, col: usize, edge_x: Pixels) -> Pixels {
        graph_col_x(col, self.margin_x, self.col_gap, edge_x)
    }

    /// The row's top, lane centre line and bottom.
    pub(super) fn row_ys(self, bounds: Bounds<Pixels>) -> (Pixels, Pixels, Pixels) {
        let top = self.snap(bounds.top());
        let bottom = self.snap(bounds.bottom());
        (top, self.snap(top + (bottom - top) / 2.0), bottom)
    }
}

/// An even device width keeps both edges on pixels around a whole-pixel centre,
/// as lane centre lines are.
fn even_device_px(value: Pixels, scale_factor: f32) -> Pixels {
    super::history_shapes::device_px(value * 0.5, scale_factor) * 2.0
}

/// Rounds to a device pixel the way gpui snaps quads (halves toward zero).
fn snap_to_device(value: Pixels, scale_factor: f32) -> Pixels {
    let device = f32::from(value) * scale_factor;
    px((device.abs() - 0.5).ceil().copysign(device) / scale_factor)
}

/// A straight vertical run of a lane. A quad rather than a tessellated path:
/// it is the same rectangle a butt-capped stroke yields, without lyon's per-path
/// buffers, and verticals are most of what a row draws. Quads paint under any
/// path in the layer, so only an elbow can now cover a straight run.
pub(super) fn paint_vertical_segment(
    x: Pixels,
    y_from: Pixels,
    y_to: Pixels,
    stroke_width: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
) {
    gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintSegmentQuad);
    window.paint_quad(
        PaintQuad::new(Bounds::new(
            point(x - stroke_width * 0.5, y_from),
            size(stroke_width, y_to - y_from),
        ))
        .background(color),
    );
}

/// Pixel-aligned straight connectors are rectangles. Avoid a path scratch
/// texture and rasterization/compositing passes for these ordinary lane exits.
/// Fractional edges, clipped strokes and nearly collinear connectors retain
/// the original path so their MSAA coverage and endpoints stay unchanged.
pub(super) fn paint_straight_connector(
    from: gpui::Point<Pixels>,
    to: gpui::Point<Pixels>,
    width: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
) {
    if from == to || width <= px(0.0) {
        return;
    }
    let bounds = if from.x == to.x {
        Some(Bounds::new(
            point(from.x - width * 0.5, from.y.min(to.y)),
            size(width, (to.y - from.y).abs()),
        ))
    } else if from.y == to.y {
        Some(Bounds::new(
            point(from.x.min(to.x), from.y - width * 0.5),
            size((to.x - from.x).abs(), width),
        ))
    } else {
        None
    };
    let scale = window.scale_factor();
    let aligned = |value: Pixels| {
        let device = f32::from(value) * scale;
        (device - device.round()).abs() < 0.001
    };
    let mask = window.content_mask();
    if let Some(bounds) = bounds.filter(|bounds| {
        // Paths preserve fractional clip coverage; quads snap their masks.
        // Keep that behavior when a connector reaches a clip or fade edge.
        bounds.intersect(&mask.bounds) == *bounds
            && mask.fade_out == gpui::Edges::default()
            && [bounds.left(), bounds.right(), bounds.top(), bounds.bottom()]
                .into_iter()
                .all(aligned)
    }) {
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintSegmentQuad);
        // Its own layer keeps the stroke's painter order, as a path's would.
        window.paint_layer(bounds, |window| {
            window.paint_quad(PaintQuad::new(bounds).background(color))
        });
    } else {
        let mut path = gpui::PathBuilder::stroke(width);
        path.move_to(from);
        path.line_to(to);
        if let Ok(path) = path.build() {
            gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::PaintPath);
            window.paint_path(path, color);
        }
    }
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

    if same_x(x_from, x_to) {
        paint_straight_connector(
            point(left + x_from, y_center),
            point(left + x_to, y_bottom),
            stroke_width,
            color,
            window,
        );
        return;
    }

    let mut path = PathBuilder::stroke(stroke_width);
    path.move_to(point(left + x_from, y_center));

    let dx = x_to - x_from;
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

/// Merge, stash and uncommitted nodes share the author nodes' lane-coloured
/// outline and faint fill, with the glyph also in the lane colour. Composite
/// over the actual row background so the opaque middle hides lane strokes.
pub(super) fn paint_icon_node(
    scale: ui_scale::UiScale,
    disc: Bounds<Pixels>,
    icon_path: &'static str,
    row_background: gpui::Rgba,
    lane_color: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) {
    let glyph = scale.ui_text(10.5);

    let background = crate::theme::composite_over(row_background, with_alpha(lane_color, 0.12));
    super::history_shapes::paint_rounded_rect(
        disc,
        disc.size.width * 0.5,
        background,
        super::history_node::ring_width(scale, window.scale_factor()),
        lane_color,
        window,
    );

    // gpui orders primitives within a layer by kind, and the sprite `paint_svg`
    // emits sorts after quads, so the glyph lands on top of the disc without
    // needing a layer of its own.
    super::diff_canvas::paint_centered_svg_icon(icon_path, disc, glyph, lane_color, window, cx);
}

/// The uncommitted-changes node uses the shared outlined icon and its selection
/// halo. `background` must be what the row is painted over, tints included.
/// A nested layer keeps both above the lane strokes; quads otherwise draw under
/// every path in the layer regardless of call order.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_uncommitted_node(
    theme: AppTheme,
    grid: GraphGrid,
    center: Point<Pixels>,
    ring_color: gpui::Rgba,
    background: gpui::Rgba,
    cell_bounds: Bounds<Pixels>,
    selection: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let scale = ui_scale::UiScale::from_window(window).with_appearance(theme.metrics);
    let scaled_px = ui_scale::scaler(scale);
    let diameter = grid.even(super::history_node::icon_diameter(scale));
    let disc = Bounds::new(
        point(center.x - diameter * 0.5, center.y - diameter * 0.5),
        size(diameter, diameter),
    );
    window.paint_layer(disc.dilate(scaled_px(8.0)), |window| {
        super::history_node::paint_selection_glow(
            theme,
            disc,
            cell_bounds,
            ring_color,
            selection,
            window,
        );
        paint_icon_node(
            scale,
            disc,
            icons::UNCOMMITTED_NODE_ICON_PATH,
            background,
            ring_color,
            window,
            cx,
        );
    });
}

#[cfg(test)]
mod author_node_geometry_tests;
#[cfg(test)]
mod band_tests;
#[cfg(test)]
mod coalescing_regression_tests;
#[cfg(test)]
mod lane_coalescing_regression_tests;
#[cfg(test)]
mod merge_rejoin_regression_tests;
#[cfg(test)]
mod tests;
