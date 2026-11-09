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
                if r.is_multiple_of(5) {
                    LanePaint::HOLE
                } else {
                    LanePaint::lane((r % 6) as u8, !r.is_multiple_of(3), r.is_multiple_of(4))
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
        let connect = xorshift(&mut state)
            .is_multiple_of(2)
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
        // Lanes that joined the node and carry on leave it through an elbow
        // drawn with the merge edges, wherever they sit.
        let leaves_node = |col: usize, lane: LanePaint| !lane.starts_at_node() && joins_out_of(col);
        let expected = coalesced(
            lanes
                .iter()
                .copied()
                .enumerate()
                .filter(|&(col, lane)| lane.is_active() && !leaves_node(col, lane)),
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
            true,
            selection_possible,
            |col, lane| paints_last(col, lane.color_ix),
            leaves_node,
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
                        && (lane.incoming() || connect == Some(col) || (!top && col == node_col))
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
        let continuing = continuing_winners(
            &row,
            pin_col,
            false,
            true,
            selection.is_some(),
            |col, lane| col >= pin_col && selection == Some(lane.color_ix),
            |_, _| false,
        );
        assert_eq!(continuing.len(), pin_col + 1, "{selection:?}");
    }
}
