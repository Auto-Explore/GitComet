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
fn a_reserved_lane_continues_only_when_drawn() {
    let mut row = band(&[born(0).with_reserved(true), born(1)], 1);
    row.lanes_now[0] = incoming(0).with_reserved(true);
    let cols = |draw_reserved| {
        continuing_winners(
            &row,
            usize::MAX,
            false,
            draw_reserved,
            false,
            |_, _| false,
            |_, _| false,
        )
        .iter()
        .map(|(col, _)| *col)
        .collect::<Vec<_>>()
    };
    assert_eq!(cols(true), [0, 1]);
    assert_eq!(cols(false), [1]);
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
