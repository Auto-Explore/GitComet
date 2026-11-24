use super::*;
use gitcomet_core::domain::CommitId;
use std::time::SystemTime;

fn commit(id: &str, parent_ids: Vec<&str>) -> Commit {
    Commit {
        id: CommitId(id.into()),
        parent_ids: parent_ids.into_iter().map(|p| CommitId(p.into())).collect(),
        summary: "".into(),
        author: "".into(),
        time: SystemTime::UNIX_EPOCH,
    }
}

#[test]
fn new_lanes_avoid_reusing_active_lane_colors() {
    let theme = AppTheme::gitcomet_dark();
    let mut commits = Vec::new();

    // Advance the internal color counter beyond the palette size using disconnected commits.
    for i in 0..LANE_COLOR_PALETTE_SIZE {
        commits.push(commit(&format!("e{i}"), Vec::new()));
    }

    // Create a long-lived lane (it stays active until we later reach p0).
    commits.push(commit("head0", vec!["p0"]));

    // Consume more colors while keeping the original lane active, until the counter wraps.
    for i in 0..(LANE_COLOR_PALETTE_SIZE - 1) {
        commits.push(commit(&format!("f{i}"), Vec::new()));
    }

    // This new lane would reuse the first color if we weren't skipping colors currently in use.
    commits.push(commit("head1", vec!["p1"]));

    // Parents, placed after the heads so the lanes stay active long enough.
    commits.push(commit("p0", Vec::new()));
    commits.push(commit("p1", Vec::new()));

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    let head1_ix = LANE_COLOR_PALETTE_SIZE + 1 + (LANE_COLOR_PALETTE_SIZE - 1);
    let row = &graph[head1_ix];
    assert_eq!(row.lanes_now.len(), 2);

    let c0 = row.lanes_now[0].color_ix;
    let c1 = row.lanes_now[1].color_ix;
    assert_ne!(c0, c1);
}

/// The whisker marks a head that is behind, but only where it can sit right
/// beside the node. With a live lane in that column it would have to run
/// horizontally across it to reach the next free one, which reads as a stray
/// line belonging to that lane -- the shape a `main` sitting on the trunk
/// with `upstream/main` alongside produces.
#[test]
fn a_behind_head_gets_no_whisker_when_it_would_cross_a_live_lane() {
    let theme = AppTheme::gitcomet_dark();
    // `side` keeps a lane alive in the column right of the trunk while
    // `shared` -- a branch head -- sits on the trunk itself.
    let commits = vec![
        commit("tip", vec!["shared"]),
        commit("side", vec!["shared"]),
        commit("shared", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, ["side", "shared"], Some("tip"));
    let shared_row = &graph[2];

    assert!(
        shared_row.joins_in.iter().all(|edge| {
            // Every join here comes from a lane that genuinely reaches this
            // commit, not from a whisker conjured beside it.
            shared_row
                .lanes_now
                .get(usize::from(edge.from_col))
                .is_some_and(|lane| lane.is_active() && lane.incoming())
        }),
        "a whisker must not be drawn across the lane sitting beside the node"
    );
}

#[test]
fn branch_heads_split_off_new_lane_when_behind() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("new1", vec!["base"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let branch_heads = ["new1", "base"];
    let graph = compute_graph(&commits, theme, branch_heads, None);

    let base_row = &graph[1];
    assert_eq!(base_row.lanes_now.len(), 2);
    assert!(base_row.lanes_now[0].incoming());
    assert!(!base_row.lanes_now[1].incoming());
    assert_eq!(base_row.joins_in.len(), 1);
    assert_eq!(base_row.node_col, 0);
    assert_ne!(
        base_row.lanes_now[0].color_ix,
        base_row.lanes_now[1].color_ix
    );

    assert_eq!(base_row.lanes_next.len(), 1);
    assert!(!base_row.lanes_next[0].starts_at_node());
}

#[test]
fn branch_heads_do_not_split_when_multiple_lanes_converge() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("top1", vec!["base"]),
        commit("top2", vec!["base"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let branch_heads = ["top1", "base"];
    let graph = compute_graph(&commits, theme, branch_heads, None);

    let base_row = &graph[2];
    assert_eq!(base_row.lanes_now.len(), 2);
    assert_eq!(base_row.joins_in.len(), 1);
    assert_eq!(base_row.node_col, 0);
    assert_eq!(base_row.lanes_next.len(), 1);
    assert!(!base_row.lanes_next[0].starts_at_node());
}

#[test]
fn active_head_lane_stays_leftmost_even_when_head_commit_appears_later() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("feature2", vec!["base"]),
        commit("main2", vec!["base"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let branch_heads = ["feature2", "main2"];
    let graph = compute_graph(&commits, theme, branch_heads, Some("main2"));

    let seeded_lane = graph[0].lanes_now[0].color_ix;
    assert_eq!(graph[0].lanes_now.len(), 2);
    assert!(graph[0].lanes_now[0].incoming());
    assert!(!graph[0].lanes_now[1].incoming());
    assert_eq!(graph[0].node_col, 1);
    assert_eq!(graph[1].node_col, 0);
    assert_eq!(graph[2].node_col, 0);
    assert_eq!(graph[1].lanes_now[0].color_ix, seeded_lane);
    assert_eq!(graph[2].lanes_now[0].color_ix, seeded_lane);
}

#[test]
fn head_lane_is_reserved_until_head() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("feature2", vec!["base"]),
        commit("main2", vec!["base"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];
    let graph = compute_graph(&commits, theme, ["feature2", "main2"], Some("main2"));

    assert!(graph[0].lanes_now[0].reserved());
    assert!(graph[0].lanes_next[0].reserved());
    assert!(graph[1].lanes_now[0].reserved(), "the run into HEAD");
    assert!(!graph[1].lanes_next[0].reserved(), "HEAD's own lane");
    assert!(!graph[2].lanes_now[0].reserved());
    assert!(
        graph
            .iter()
            .flat_map(|row| row
                .lanes_now
                .iter()
                .skip(1)
                .chain(row.lanes_next.iter().skip(1)))
            .all(|lane| !lane.reserved()),
        "only the HEAD lane is reserved"
    );
}

#[test]
fn a_merge_into_the_head_lane_ends_its_reserved_run() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("merge", vec!["feature", "head"]),
        commit("feature", vec!["head"]),
        commit("head", vec!["root"]),
        commit("root", Vec::new()),
    ];
    let graph = compute_graph(&commits, theme, ["merge"], Some("head"));

    assert_eq!(graph[0].edges_out[0].to_col, 0);
    assert!(graph[0].lanes_now[0].reserved());
    // The merge edge's elbow draws this row's continuation.
    assert!(graph[0].lanes_next[0].reserved());
    assert!(!graph[1].lanes_now[0].reserved());
    assert!(!graph[1].lanes_next[0].reserved());
    assert!(!graph[2].lanes_now[0].reserved());
}

#[test]
fn head_on_the_first_row_reserves_nothing() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("head", vec!["base"]),
        commit("side", vec!["base"]),
        commit("base", Vec::new()),
    ];
    let graph = compute_graph(&commits, theme, ["head", "side"], Some("head"));

    assert!(
        graph
            .iter()
            .flat_map(|row| row.lanes_now.iter().chain(&row.lanes_next))
            .all(|lane| !lane.reserved())
    );
}

#[test]
fn inserted_secondary_parent_lane_has_no_previous_column() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("merge", vec!["base", "side"]),
        commit("side", vec!["root"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    let merge_row = &graph[0];
    assert_eq!(merge_row.lanes_next.len(), 2);
    assert!(!merge_row.lanes_next[0].starts_at_node());
    assert!(merge_row.lanes_next[1].starts_at_node());
    assert!(merge_row.edges_out.is_empty());
}

#[test]
fn parents_above_the_current_row_do_not_leave_dead_lanes() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![commit("base", Vec::new()), commit("tip", vec!["base"])];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    assert_eq!(graph.len(), 2);
    assert_eq!(graph[1].lanes_now.len(), 1);
    assert!(graph[1].lanes_next.is_empty());
    assert!(graph[1].edges_out.is_empty());
}

#[test]
fn linear_visible_history_keeps_single_lane_shape() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("c2", vec!["c1"]),
        commit("c1", vec!["c0"]),
        commit("c0", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    assert_eq!(graph.len(), 3);
    assert_eq!(graph[0].lanes_now.len(), 1);
    assert!(!graph[0].lanes_now[0].incoming());
    assert!(!graph[0].lanes_next[0].starts_at_node());
    assert!(graph[1].lanes_now[0].incoming());
    assert!(!graph[1].lanes_next[0].starts_at_node());
    assert!(graph[2].lanes_now[0].incoming());
    assert!(graph[2].lanes_next.is_empty());
    assert!(graph.iter().all(|row| row.node_col == 0));
    assert!(
        graph
            .iter()
            .all(|row| row.joins_in.is_empty() && row.edges_out.is_empty())
    );
}

#[test]
fn active_head_target_later_in_linear_history_still_uses_seeded_lane() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("feature", vec!["main"]),
        commit("main", vec!["base"]),
        commit("base", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), Some("main"));

    assert_eq!(graph[0].lanes_now.len(), 2);
    assert_eq!(graph[0].node_col, 1);
    assert_eq!(graph[1].node_col, 0);
}

#[test]
fn duplicate_branch_heads_do_not_create_extra_lanes() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("feature", vec!["base"]),
        commit("main", vec!["base"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let unique = compute_graph(&commits, theme, ["feature", "main"], None);
    let duplicate = compute_graph(&commits, theme, ["feature", "feature", "main"], None);

    assert_eq!(duplicate.len(), unique.len());
    for (duplicate_row, unique_row) in duplicate.iter().zip(unique.iter()) {
        let duplicate_now = duplicate_row
            .lanes_now
            .iter()
            .map(|lane| {
                (
                    lane.color_ix,
                    lane.incoming(),
                    lane.starts_at_node(),
                    lane.is_active(),
                )
            })
            .collect::<Vec<_>>();
        let unique_now = unique_row
            .lanes_now
            .iter()
            .map(|lane| {
                (
                    lane.color_ix,
                    lane.incoming(),
                    lane.starts_at_node(),
                    lane.is_active(),
                )
            })
            .collect::<Vec<_>>();
        let duplicate_next = duplicate_row
            .lanes_next
            .iter()
            .map(|lane| {
                (
                    lane.color_ix,
                    lane.incoming(),
                    lane.starts_at_node(),
                    lane.is_active(),
                )
            })
            .collect::<Vec<_>>();
        let unique_next = unique_row
            .lanes_next
            .iter()
            .map(|lane| {
                (
                    lane.color_ix,
                    lane.incoming(),
                    lane.starts_at_node(),
                    lane.is_active(),
                )
            })
            .collect::<Vec<_>>();
        let duplicate_joins = duplicate_row
            .joins_in
            .iter()
            .map(|edge| (edge.from_col, edge.to_col, edge.color_ix))
            .collect::<Vec<_>>();
        let unique_joins = unique_row
            .joins_in
            .iter()
            .map(|edge| (edge.from_col, edge.to_col, edge.color_ix))
            .collect::<Vec<_>>();
        let duplicate_edges = duplicate_row
            .edges_out
            .iter()
            .map(|edge| (edge.from_col, edge.to_col, edge.color_ix))
            .collect::<Vec<_>>();
        let unique_edges = unique_row
            .edges_out
            .iter()
            .map(|edge| (edge.from_col, edge.to_col, edge.color_ix))
            .collect::<Vec<_>>();

        assert_eq!(duplicate_now, unique_now);
        assert_eq!(duplicate_next, unique_next);
        assert_eq!(duplicate_joins, unique_joins);
        assert_eq!(duplicate_edges, unique_edges);
        assert_eq!(duplicate_row.node_col, unique_row.node_col);
        assert_eq!(duplicate_row.is_merge, unique_row.is_merge);
    }
}

/// The column-stability invariant, stated in terms of the public output: a
/// lane that continues into the next row continues in *the same column*.
///
/// This is what makes every continuation a straight vertical. Before lanes
/// became column-stable, a lane ending would compact the vector and its
/// right-hand neighbours would reappear one column left -- which is what the
/// painter drew as a diagonal.
fn assert_columns_are_stable(rows: &[GraphRow]) {
    for (ix, pair) in rows.windows(2).enumerate() {
        let (upper, lower) = (&pair[0], &pair[1]);

        for (col, lane) in upper.lanes_next.iter().enumerate() {
            if !lane.is_active() {
                continue;
            }
            let below = lower.lanes_now.get(col).copied().unwrap_or(LanePaint::HOLE);
            assert!(
                below.is_active(),
                "row {ix}: lane in column {col} vanished instead of continuing"
            );
            assert_eq!(
                below.color_ix, lane.color_ix,
                "row {ix}: lane in column {col} changed colour across the row boundary"
            );
            assert!(
                below.incoming(),
                "row {ix}: lane in column {col} lost its incoming segment"
            );
        }

        for (col, lane) in lower.lanes_now.iter().enumerate() {
            if !lane.is_active() || !lane.incoming() {
                continue;
            }
            let above = upper
                .lanes_next
                .get(col)
                .copied()
                .unwrap_or(LanePaint::HOLE);
            assert!(
                above.is_active(),
                "row {}: incoming segment in column {col} has no lane above it",
                ix + 1
            );
            assert_eq!(
                above.color_ix,
                lane.color_ix,
                "row {}: incoming segment in column {col} changed colour",
                ix + 1
            );
        }
    }
}

fn peak_active_lane_count(rows: &[GraphRow]) -> usize {
    let active = |lanes: &LanePaints| lanes.iter().filter(|lane| lane.is_active()).count();
    rows.iter()
        .map(|row| active(&row.lanes_now).max(active(&row.lanes_next)))
        .max()
        .unwrap_or(0)
}

fn drawn_width(rows: &[GraphRow]) -> usize {
    rows.iter()
        .map(|row| row.lanes_now.len().max(row.lanes_next.len()))
        .max()
        .unwrap_or(0)
}

#[test]
fn lane_column_survives_a_neighbour_lane_ending() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("a", vec!["a1"]),
        commit("b", vec!["b1"]),
        commit("c", vec!["c1"]),
        commit("b1", Vec::new()),
        commit("c1", Vec::new()),
        commit("a1", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    // `b1` ends the lane in column 1. Column 2 must stay put rather than
    // sliding left into the freed column.
    assert_eq!(graph[3].lanes_next[1], LanePaint::HOLE);
    assert!(graph[3].lanes_next[2].is_active());
    assert!(!graph[3].lanes_next[2].starts_at_node());
    assert!(graph[4].lanes_now[2].is_active());
    assert!(graph[4].lanes_now[2].incoming());
    assert_eq!(graph[4].node_col, 2);

    assert_columns_are_stable(&graph);
}

#[test]
fn new_lane_claims_the_leftmost_hole() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("a", vec!["a1"]),
        commit("b", vec!["b1"]),
        commit("c", vec!["c1"]),
        commit("b1", Vec::new()),
        commit("d", vec!["d1"]),
        commit("c1", Vec::new()),
        commit("d1", Vec::new()),
        commit("a1", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    // Column 1 was freed by `b1`; `d` reuses it instead of widening the graph.
    assert_eq!(graph[3].lanes_next[1], LanePaint::HOLE);
    assert_eq!(graph[4].node_col, 1);
    assert!(graph[4].lanes_next[1].is_active());
    assert!(graph[4].lanes_next[1].starts_at_node());

    assert_columns_are_stable(&graph);
}

#[test]
fn branch_head_fork_keeps_the_node_column() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("main_tip", vec!["base"]),
        commit("feat_tip", vec!["feat_mid"]),
        commit("feat_mid", vec!["base"]),
        commit("base", vec!["root"]),
        commit("root", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, ["feat_mid"], None);

    // `feat_mid` is a branch head sitting on a lane it does not own, so it
    // takes the lane over. The node stays in the lane's own column and the
    // fork is a paint-only whisker to its right -- previously this swapped
    // two live lanes and drew the node right of every continuing lane.
    let row = &graph[2];
    assert_eq!(row.node_col, 1);
    assert_eq!(row.lanes_now.len(), 3);
    assert!(row.lanes_now[2].is_active());
    assert!(!row.lanes_now[2].incoming());
    assert_eq!(row.joins_in.len(), 1);
    assert_eq!(row.joins_in[0].from_col, 2);
    assert_eq!(row.joins_in[0].to_col, 1);

    // The node and everything below it wear the head's new colour, while the
    // segment above the node keeps the descendant lane's colour.
    assert_eq!(row.node_color_ix, row.joins_in[0].color_ix);
    assert_eq!(row.lanes_next[1].color_ix, row.node_color_ix);
    assert_ne!(row.lanes_now[1].color_ix, row.node_color_ix);

    assert_columns_are_stable(&graph);
}

#[test]
fn graph_is_never_wider_than_its_peak_lane_count() {
    let theme = AppTheme::gitcomet_dark();
    let commits = vec![
        commit("a", vec!["a1"]),
        commit("b", vec!["b1"]),
        commit("c", vec!["c1"]),
        commit("b1", Vec::new()),
        commit("d", vec!["d1"]),
        commit("c1", Vec::new()),
        commit("d1", Vec::new()),
        commit("a1", Vec::new()),
    ];

    let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

    // Holes are reserved space, not extra width: a column is only ever added
    // when every existing column is live.
    assert_eq!(drawn_width(&graph), peak_active_lane_count(&graph));
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Deterministic pseudo-random history in log order: every parent index is
/// greater than its commit's own, with occasional roots and occasional
/// far-back second parents.
fn generated_history(seed: u64, len: usize) -> Vec<Commit> {
    let mut state = seed | 1;
    let mut commits = Vec::with_capacity(len);
    for ix in 0..len {
        let mut parents: Vec<String> = Vec::new();
        let remaining = len - ix - 1;
        if remaining > 0 {
            let roll = xorshift(&mut state) % 100;
            if roll >= 8 {
                let step = 1 + (xorshift(&mut state) as usize % remaining.min(4));
                parents.push(format!("c{}", ix + step));
                if roll >= 80 {
                    let back = 1 + (xorshift(&mut state) as usize % remaining.min(24));
                    if back != step {
                        parents.push(format!("c{}", ix + back));
                    }
                }
            }
        }
        commits.push(commit(
            &format!("c{ix}"),
            parents.iter().map(String::as_str).collect(),
        ));
    }
    commits
}

#[test]
fn generated_histories_keep_lane_columns_stable() {
    let theme = AppTheme::gitcomet_dark();

    for seed in 1..=8u64 {
        let commits = generated_history(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), 200);
        let heads: Vec<&str> = commits
            .iter()
            .enumerate()
            .filter(|(ix, _)| ix % 17 == 0)
            .map(|(_, c)| c.id.as_ref())
            .collect();

        // A branch-head fork adds a paint-only whisker column to the right of
        // the node, which can land one past the last live column even when a
        // hole exists to its left -- so those configurations get one column
        // of slack. Without branch heads the bound is exact.
        for (label, slack, graph) in [
            (
                "bare",
                0,
                compute_graph(&commits, theme, std::iter::empty::<&str>(), None),
            ),
            (
                "branch heads",
                1,
                compute_graph(&commits, theme, heads.iter().copied(), None),
            ),
            (
                "active head",
                1,
                compute_graph(&commits, theme, heads.iter().copied(), Some("c3")),
            ),
        ] {
            assert_eq!(graph.len(), commits.len(), "seed {seed} ({label})");
            assert_columns_are_stable(&graph);

            let width = drawn_width(&graph);
            let peak = peak_active_lane_count(&graph);
            assert!(
                width <= peak + slack,
                "seed {seed} ({label}): graph is {width} columns wide \
                     but never draws more than {peak} lanes at once"
            );
        }
    }
}
