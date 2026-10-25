//! Opt-in serpentine "band fold" for flowchart aspect-ratio goals.
//!
//! When `LayoutConfig::preferred_aspect_ratio` is set and a horizontal
//! flowchart lays out much wider than the goal, this pass folds the
//! monotonic main axis into 2..=4 serpentine bands at rank boundaries, the way
//! text wraps into lines. Odd bands are mirrored so consecutive ranks that
//! straddle a band boundary stay on the same side and the wrap edge becomes a
//! short hop instead of a full-width return.
//!
//! Subgraphs are atomic: the rank span covered by each subgraph's members is
//! merged into a single unbreakable run, and band boundaries are only placed
//! between those runs. A diagram whose subgraph spans every rank therefore
//! never folds. Subgraph boxes themselves are materialized downstream from
//! the folded node positions, so they shrink to fit their band.
//!
//! The fold is best-effort and scored: every candidate band count is applied
//! to a cloned node map and accepted only if the log-aspect error improves by
//! a real margin while straight-line edge crossings stay within budget. When
//! no candidate is accepted the node map is left bit-identical and the
//! existing `apply_preferred_aspect_ratio_layout` stretch pass remains the
//! only aspect mechanism. After an accepted fold the stretch pass still runs
//! and closes the residual gap.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use crate::config::LayoutConfig;
use crate::ir::{DiagramKind, Graph};

use super::super::geometry::segments_intersect;
use super::super::{NodeLayout, is_horizontal};
use super::manual_layout::ManualLayoutRanks;

/// Only fold when the natural ratio is at least this multiple of the goal.
/// Layouts already near the goal are handled by the stretch pass alone.
const MIN_FOLD_TRIGGER: f32 = 2.0;
/// Never fold into more than this many bands.
const MAX_FOLD_BANDS: usize = 4;
/// Minimum number of ranks required before folding is considered.
const MIN_FOLD_RANKS: usize = 4;
/// Greedy band partition closes a band once it exceeds `target * slack`.
const BAND_WIDTH_SLACK: f32 = 1.15;
/// A fold must reduce the log-aspect error to at most this fraction of the
/// unfolded error to be accepted.
const ASPECT_IMPROVE_FACTOR: f32 = 0.7;
/// Crossing budget: folded straight-line crossings must stay within
/// `baseline * CROSSING_MULT + CROSSING_SLACK`.
const CROSSING_MULT: f32 = 2.0;
const CROSSING_SLACK: usize = 2;
/// Cross-axis corridor between bands, in units of the larger spacing knob.
const WRAP_GUTTER_FACTOR: f32 = 1.0;

#[derive(Debug)]
pub(in crate::layout) struct BandFoldOutcome {
    #[allow(dead_code)] // read by unit tests; reserved for stage metrics
    pub(in crate::layout) band_count: usize,
    #[allow(dead_code)] // reserved for wrap-edge routing roles (phase 2)
    pub(in crate::layout) node_bands: HashMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FoldScore {
    aspect_error: f32,
    crossings: usize,
}

#[derive(Debug, Clone, Copy)]
struct RankExtent {
    main_start: f32,
    main_end: f32,
}

/// Fold an over-wide horizontal flowchart into serpentine bands so its aspect
/// ratio moves toward `config.preferred_aspect_ratio`.
///
/// Returns `None` (leaving `nodes` untouched) whenever the layout is not
/// eligible or no scored candidate improves on the unfolded geometry.
pub(in crate::layout) fn apply_aspect_ratio_band_fold(
    graph: &Graph,
    rank_info: &ManualLayoutRanks,
    layout_edges: &[crate::ir::Edge],
    nodes: &mut BTreeMap<String, NodeLayout>,
    config: &LayoutConfig,
) -> Option<BandFoldOutcome> {
    let goal = config
        .preferred_aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)?;
    if graph.kind != DiagramKind::Flowchart {
        return None;
    }
    if !is_horizontal(graph.direction) {
        return None;
    }

    let ranks = collect_placed_ranks(rank_info, nodes);
    if ranks.len() < MIN_FOLD_RANKS {
        return None;
    }
    // Every visible node must belong to a rank bucket; otherwise folding
    // could drop it onto another band.
    let ranked_total: usize = ranks.iter().map(Vec::len).sum();
    let visible_total = nodes.values().filter(|node| !node.hidden).count();
    if ranked_total != visible_total {
        return None;
    }

    let (min_x, min_y, max_x, max_y) = layout_bounds(nodes)?;
    let natural_ratio = (max_x - min_x).max(1.0) / (max_y - min_y).max(1.0);
    if !natural_ratio.is_finite() || natural_ratio / goal < MIN_FOLD_TRIGGER {
        return None;
    }

    let extents = rank_extents(&ranks, nodes);
    let allowed_boundaries = subgraph_safe_boundaries(graph, &ranks);
    // If no interior rank boundary is safe (e.g. one subgraph spans every
    // rank), folding is impossible.
    if !allowed_boundaries.iter().skip(1).any(|allowed| *allowed) {
        return None;
    }
    let gutter = config.rank_spacing.max(config.node_spacing) * WRAP_GUTTER_FACTOR;
    let baseline = FoldScore {
        aspect_error: layout_aspect_error(nodes, goal)?,
        crossings: straight_line_crossings(layout_edges, nodes),
    };

    let mut best: Option<(
        FoldScore,
        usize,
        BTreeMap<String, NodeLayout>,
        HashMap<String, usize>,
    )> = None;
    for band_count in candidate_band_counts(natural_ratio, goal, ranks.len()) {
        let ranges = plan_band_ranges(&extents, band_count, &allowed_boundaries);
        if ranges.len() != band_count {
            continue;
        }
        let mut candidate_nodes = nodes.clone();
        let node_bands =
            apply_fold_to_nodes(&ranges, &extents, &ranks, &mut candidate_nodes, gutter);
        let Some(aspect_error) = layout_aspect_error(&candidate_nodes, goal) else {
            continue;
        };
        let score = FoldScore {
            aspect_error,
            crossings: straight_line_crossings(layout_edges, &candidate_nodes),
        };
        if !fold_candidate_accepted(&baseline, &score) {
            continue;
        }
        let improves = match &best {
            None => true,
            Some((best_score, best_bands, _, _)) => {
                (score.aspect_error, score.crossings, band_count)
                    < (best_score.aspect_error, best_score.crossings, *best_bands)
            }
        };
        if improves {
            best = Some((score, band_count, candidate_nodes, node_bands));
        }
    }

    let (_, band_count, winner_nodes, node_bands) = best?;
    *nodes = winner_nodes;
    Some(BandFoldOutcome {
        band_count,
        node_bands,
    })
}

/// Accept a folded candidate only when it improves the aspect error by a real
/// margin and keeps the straight-line crossing proxy within budget.
fn fold_candidate_accepted(baseline: &FoldScore, candidate: &FoldScore) -> bool {
    candidate.aspect_error <= baseline.aspect_error * ASPECT_IMPROVE_FACTOR
        && (candidate.crossings as f32)
            <= (baseline.crossings as f32) * CROSSING_MULT + CROSSING_SLACK as f32
}

/// Filter the manual-layout rank buckets down to visible placed nodes,
/// dropping ordering-dummy ids and empty buckets.
fn collect_placed_ranks(
    rank_info: &ManualLayoutRanks,
    nodes: &BTreeMap<String, NodeLayout>,
) -> Vec<Vec<String>> {
    rank_info
        .rank_nodes
        .iter()
        .filter_map(|bucket| {
            let filtered: Vec<String> = bucket
                .iter()
                .filter(|id| nodes.get(*id).is_some_and(|node| !node.hidden))
                .cloned()
                .collect();
            (!filtered.is_empty()).then_some(filtered)
        })
        .collect()
}

fn layout_bounds(nodes: &BTreeMap<String, NodeLayout>) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for node in nodes.values() {
        if node.hidden {
            continue;
        }
        min_x = min_x.min(node.x);
        min_y = min_y.min(node.y);
        max_x = max_x.max(node.x + node.width);
        max_y = max_y.max(node.y + node.height);
    }
    (min_x != f32::MAX).then_some((min_x, min_y, max_x, max_y))
}

/// Absolute log-ratio distance between the node-bounds aspect and the goal.
fn layout_aspect_error(nodes: &BTreeMap<String, NodeLayout>, goal: f32) -> Option<f32> {
    let (min_x, min_y, max_x, max_y) = layout_bounds(nodes)?;
    let ratio = (max_x - min_x).max(1.0) / (max_y - min_y).max(1.0);
    Some((ratio / goal).ln().abs())
}

/// Count pairwise intersections between center-to-center edge segments.
/// Edge pairs sharing an endpoint node are skipped, as are self-loops.
fn straight_line_crossings(
    layout_edges: &[crate::ir::Edge],
    nodes: &BTreeMap<String, NodeLayout>,
) -> usize {
    let segments: Vec<(&str, &str, (f32, f32), (f32, f32))> = layout_edges
        .iter()
        .filter(|edge| edge.from != edge.to)
        .filter_map(|edge| {
            let from = nodes.get(&edge.from)?;
            let to = nodes.get(&edge.to)?;
            Some((
                edge.from.as_str(),
                edge.to.as_str(),
                (from.x + from.width / 2.0, from.y + from.height / 2.0),
                (to.x + to.width / 2.0, to.y + to.height / 2.0),
            ))
        })
        .collect();
    let mut crossings = 0usize;
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            let (a_from, a_to, a1, a2) = segments[i];
            let (b_from, b_to, b1, b2) = segments[j];
            if a_from == b_from || a_from == b_to || a_to == b_from || a_to == b_to {
                continue;
            }
            if segments_intersect(a1, a2, b1, b2) {
                crossings += 1;
            }
        }
    }
    crossings
}

/// Candidate band counts around the analytic ideal `sqrt(natural / goal)`.
///
/// With `k` bands the folded layout is roughly `width/k` by `k*height`, so
/// the ratio scales by `1/k^2`.
fn candidate_band_counts(natural_ratio: f32, goal: f32, rank_count: usize) -> Vec<usize> {
    let max_bands = rank_count.min(MAX_FOLD_BANDS);
    if max_bands < 2 || goal <= 0.0 || natural_ratio <= 0.0 {
        return Vec::new();
    }
    let ideal = (natural_ratio / goal).sqrt();
    if !ideal.is_finite() {
        return Vec::new();
    }
    let floor = ideal.floor().max(0.0) as usize;
    let ceil = ideal.ceil().max(0.0) as usize;
    let mut candidates: Vec<usize> = [floor, ceil, ceil + 1]
        .into_iter()
        .map(|k| k.clamp(2, max_bands))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

/// Main-axis extents per rank (ranks are already in main-axis order).
fn rank_extents(ranks: &[Vec<String>], nodes: &BTreeMap<String, NodeLayout>) -> Vec<RankExtent> {
    ranks
        .iter()
        .map(|bucket| {
            let mut main_start = f32::MAX;
            let mut main_end = f32::MIN;
            for id in bucket {
                if let Some(node) = nodes.get(id) {
                    main_start = main_start.min(node.x);
                    main_end = main_end.max(node.x + node.width);
                }
            }
            RankExtent {
                main_start,
                main_end,
            }
        })
        .collect()
}

/// Boundary mask over rank indices: `mask[idx]` is true when a band may
/// start at rank `idx`, i.e. the boundary between `idx - 1` and `idx` does
/// not split any subgraph's rank span. Subgraphs are atomic: a band boundary
/// inside one would tear its box across bands. The subgraph anchor node
/// (a node named after the subgraph id used for edges to the subgraph) is
/// treated as a member for span purposes.
fn subgraph_safe_boundaries(graph: &Graph, ranks: &[Vec<String>]) -> Vec<bool> {
    let mut allowed = vec![true; ranks.len()];
    let mut node_rank: HashMap<&str, usize> = HashMap::new();
    for (idx, bucket) in ranks.iter().enumerate() {
        for id in bucket {
            node_rank.insert(id.as_str(), idx);
        }
    }
    for sub in &graph.subgraphs {
        let member_ranks = sub
            .nodes
            .iter()
            .map(String::as_str)
            .chain(sub.id.as_deref())
            .filter_map(|id| node_rank.get(id).copied());
        let mut min_rank = usize::MAX;
        let mut max_rank = 0usize;
        for rank in member_ranks {
            min_rank = min_rank.min(rank);
            max_rank = max_rank.max(rank);
        }
        if min_rank == usize::MAX {
            continue;
        }
        for flag in allowed.iter_mut().take(max_rank + 1).skip(min_rank + 1) {
            *flag = false;
        }
    }
    allowed
}

/// Partition ranks into `band_count` contiguous runs with roughly balanced
/// main-axis spans. Ranks are atomic: fold boundaries are rank boundaries.
/// Band boundaries are further restricted to `allowed_boundaries` (subgraph
/// spans must not be split). Every band is guaranteed at least one rank; if
/// the boundary mask cannot support `band_count` bands the result has fewer
/// ranges and the caller rejects it.
fn plan_band_ranges(
    extents: &[RankExtent],
    band_count: usize,
    allowed_boundaries: &[bool],
) -> Vec<Range<usize>> {
    let rank_count = extents.len();
    if band_count < 2 || band_count > rank_count {
        return Vec::new();
    }
    let total_span = (extents[rank_count - 1].main_end - extents[0].main_start).max(1.0);
    let target = total_span / band_count as f32;
    let mut ranges: Vec<Range<usize>> = Vec::with_capacity(band_count);
    let mut start = 0usize;
    for idx in 1..rank_count {
        if !allowed_boundaries.get(idx).copied().unwrap_or(true) {
            continue;
        }
        if idx == start {
            continue;
        }
        let bands_left = band_count - ranges.len();
        if bands_left <= 1 {
            break;
        }
        // Allowed boundaries strictly after `idx`. Closing at `idx` leaves
        // `bands_left - 2` more closures to place among them, so `idx` is the
        // last chance once they run down to that count.
        let boundaries_after = (idx + 1..rank_count)
            .filter(|next| allowed_boundaries.get(*next).copied().unwrap_or(true))
            .count();
        let must_close = boundaries_after < bands_left - 1;
        let span_with_idx = extents[idx].main_end - extents[start].main_start;
        let over_budget = span_with_idx > target * BAND_WIDTH_SLACK;
        if must_close || over_budget {
            ranges.push(start..idx);
            start = idx;
        }
    }
    ranges.push(start..rank_count);
    ranges
}

/// Apply the serpentine fold: each band is rebased to main 0 (odd bands are
/// mirrored) and stacked along the cross axis with a wrap-edge gutter. Each
/// band is then shifted along the main axis so its entry rank sits directly
/// under the previous band's exit rank, keeping wrap edges short even when
/// band spans are unbalanced. Returns the band index per node id.
fn apply_fold_to_nodes(
    ranges: &[Range<usize>],
    extents: &[RankExtent],
    ranks: &[Vec<String>],
    nodes: &mut BTreeMap<String, NodeLayout>,
    gutter: f32,
) -> HashMap<String, usize> {
    let mut node_bands: HashMap<String, usize> = HashMap::new();
    let mut cross_offset = 0.0f32;
    let mut prev_exit_center: Option<f32> = None;
    for (band_idx, range) in ranges.iter().enumerate() {
        let reversed = band_idx % 2 == 1;
        let band_main_origin = extents[range.start].main_start;
        let band_main_span = extents[range.clone()]
            .iter()
            .map(|extent| extent.main_end)
            .fold(f32::MIN, f32::max)
            - band_main_origin;

        let mut band_cross_min = f32::MAX;
        let mut band_cross_max = f32::MIN;
        for rank_idx in range.clone() {
            for id in &ranks[rank_idx] {
                if let Some(node) = nodes.get(id) {
                    band_cross_min = band_cross_min.min(node.y);
                    band_cross_max = band_cross_max.max(node.y + node.height);
                }
            }
        }
        if band_cross_min == f32::MAX {
            continue;
        }

        // Band-local main position: rebase to 0, mirroring odd bands.
        let local_main = |node: &NodeLayout| {
            let rebased = node.x - band_main_origin;
            if reversed {
                band_main_span - (rebased + node.width)
            } else {
                rebased
            }
        };
        // Align this band's entry rank (its first rank in rank order, which
        // mirroring places on the previous band's exit side) under the
        // previous band's exit rank so the wrap edge is a straight hop.
        let rank_local_center = |rank_idx: usize, nodes: &BTreeMap<String, NodeLayout>| {
            let mut min_main = f32::MAX;
            let mut max_main = f32::MIN;
            for id in &ranks[rank_idx] {
                if let Some(node) = nodes.get(id) {
                    let main = local_main(node);
                    min_main = min_main.min(main);
                    max_main = max_main.max(main + node.width);
                }
            }
            (min_main + max_main) * 0.5
        };
        let shift = match prev_exit_center {
            Some(prev) => prev - rank_local_center(range.start, nodes),
            None => 0.0,
        };

        for rank_idx in range.clone() {
            for id in &ranks[rank_idx] {
                if let Some(node) = nodes.get_mut(id) {
                    let main = {
                        let rebased = node.x - band_main_origin;
                        if reversed {
                            band_main_span - (rebased + node.width)
                        } else {
                            rebased
                        }
                    };
                    node.x = main + shift;
                    node.y = (node.y - band_cross_min) + cross_offset;
                    node_bands.insert(id.clone(), band_idx);
                }
            }
        }
        prev_exit_center = Some({
            let mut min_main = f32::MAX;
            let mut max_main = f32::MIN;
            for id in &ranks[range.end - 1] {
                if let Some(node) = nodes.get(id) {
                    min_main = min_main.min(node.x);
                    max_main = max_main.max(node.x + node.width);
                }
            }
            (min_main + max_main) * 0.5
        });
        cross_offset += (band_cross_max - band_cross_min) + gutter;
    }
    node_bands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Direction;
    use crate::layout::TextBlock;

    const GOAL_4_3: f32 = 4.0 / 3.0;

    fn make_node(id: &str, x: f32, y: f32, width: f32, height: f32) -> NodeLayout {
        NodeLayout {
            id: id.to_string(),
            x,
            y,
            width,
            height,
            label: TextBlock {
                lines: vec![id.to_string()],
                width: 20.0,
                height: 14.0,
            },
            shape: crate::ir::NodeShape::Rectangle,
            style: crate::ir::NodeStyle::default(),
            link: None,
            anchor_subgraph: None,
            hidden: false,
            icon: None,
        }
    }

    fn make_edge(from: &str, to: &str) -> crate::ir::Edge {
        crate::ir::Edge {
            from: from.to_string(),
            to: to.to_string(),
            label: None,
            start_label: None,
            end_label: None,
            directed: true,
            arrow_start: false,
            arrow_end: true,
            arrow_start_kind: None,
            arrow_end_kind: None,
            start_decoration: None,
            end_decoration: None,
            style: crate::ir::EdgeStyle::Solid,
        }
    }

    /// 6-node LR chain: one node per rank, 100px pitch, 60x36 nodes.
    fn chain_fixture() -> (
        Graph,
        ManualLayoutRanks,
        Vec<crate::ir::Edge>,
        BTreeMap<String, NodeLayout>,
    ) {
        let ids = ["A", "B", "C", "D", "E", "F"];
        let mut graph = Graph::new();
        graph.kind = DiagramKind::Flowchart;
        graph.direction = Direction::LeftRight;
        for (idx, id) in ids.iter().enumerate() {
            graph.nodes.insert(
                (*id).to_string(),
                crate::ir::Node {
                    id: (*id).to_string(),
                    label: (*id).to_string(),
                    shape: crate::ir::NodeShape::Rectangle,
                    value: None,
                    icon: None,
                },
            );
            graph.node_order.insert((*id).to_string(), idx);
        }
        let edges: Vec<crate::ir::Edge> = ids
            .windows(2)
            .map(|pair| make_edge(pair[0], pair[1]))
            .collect();
        graph.edges = edges.clone();
        let rank_info = ManualLayoutRanks {
            rank_nodes: ids.iter().map(|id| vec![(*id).to_string()]).collect(),
        };
        let mut nodes = BTreeMap::new();
        for (idx, id) in ids.iter().enumerate() {
            nodes.insert(
                (*id).to_string(),
                make_node(id, idx as f32 * 100.0, 0.0, 60.0, 36.0),
            );
        }
        (graph, rank_info, edges, nodes)
    }

    fn nodes_bit_identical(
        a: &BTreeMap<String, NodeLayout>,
        b: &BTreeMap<String, NodeLayout>,
    ) -> bool {
        a.len() == b.len()
            && a.iter().zip(b.iter()).all(|((ida, na), (idb, nb))| {
                ida == idb
                    && na.x.to_bits() == nb.x.to_bits()
                    && na.y.to_bits() == nb.y.to_bits()
                    && na.width.to_bits() == nb.width.to_bits()
                    && na.height.to_bits() == nb.height.to_bits()
            })
    }

    fn config_with_goal(goal: Option<f32>) -> LayoutConfig {
        let mut config = LayoutConfig::default();
        config.preferred_aspect_ratio = goal;
        config
    }

    #[test]
    fn candidate_band_counts_for_wide_chain() {
        // Motivating case: natural 7:1, goal 4:3 -> ideal k ~= 2.29.
        let candidates = candidate_band_counts(7.0, GOAL_4_3, 6);
        assert!(candidates.contains(&2), "candidates: {candidates:?}");
        assert!(candidates.contains(&3), "candidates: {candidates:?}");
        assert!(
            candidates.iter().all(|k| (2..=4).contains(k)),
            "candidates outside [2,4]: {candidates:?}"
        );
    }

    #[test]
    fn candidate_band_counts_capped_by_rank_count() {
        let candidates = candidate_band_counts(50.0, 1.0, 3);
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|k| *k <= 3));
        assert!(candidate_band_counts(50.0, 1.0, 1).is_empty());
    }

    #[test]
    fn plan_bands_balances_rank_widths() {
        // 6 equal-width ranks folded into 3 bands -> 2 ranks each.
        let extents: Vec<RankExtent> = (0..6)
            .map(|idx| RankExtent {
                main_start: idx as f32 * 100.0,
                main_end: idx as f32 * 100.0 + 60.0,
            })
            .collect();
        let ranges = plan_band_ranges(&extents, 3, &vec![true; extents.len()]);
        assert_eq!(ranges, vec![0..2, 2..4, 4..6]);
    }

    #[test]
    fn plan_bands_respects_blocked_boundaries() {
        // 6 equal-width ranks, but boundaries at 2 and 3 are blocked by a
        // subgraph spanning ranks 1..=3. The 3-band split must move its
        // boundaries to allowed positions (1 and 4).
        let extents: Vec<RankExtent> = (0..6)
            .map(|idx| RankExtent {
                main_start: idx as f32 * 100.0,
                main_end: idx as f32 * 100.0 + 60.0,
            })
            .collect();
        let allowed = vec![true, true, false, false, true, true];
        let ranges = plan_band_ranges(&extents, 3, &allowed);
        assert_eq!(ranges.len(), 3);
        for range in &ranges[1..] {
            assert!(
                allowed[range.start],
                "band boundary at blocked rank {}",
                range.start
            );
        }
    }

    #[test]
    fn plan_bands_returns_short_when_boundaries_insufficient() {
        let extents: Vec<RankExtent> = (0..6)
            .map(|idx| RankExtent {
                main_start: idx as f32 * 100.0,
                main_end: idx as f32 * 100.0 + 60.0,
            })
            .collect();
        // Only one interior boundary allowed: a 3-band plan is impossible
        // and the caller must see fewer ranges than requested.
        let allowed = vec![true, false, false, true, false, false];
        let ranges = plan_band_ranges(&extents, 3, &allowed);
        assert!(ranges.len() < 3, "got {ranges:?}");
    }

    #[test]
    fn plan_bands_gives_every_band_at_least_one_rank() {
        // One huge rank up front must not starve the remaining bands.
        let mut extents = vec![RankExtent {
            main_start: 0.0,
            main_end: 900.0,
        }];
        for idx in 0..3 {
            extents.push(RankExtent {
                main_start: 950.0 + idx as f32 * 50.0,
                main_end: 980.0 + idx as f32 * 50.0,
            });
        }
        let ranges = plan_band_ranges(&extents, 4, &vec![true; extents.len()]);
        assert_eq!(ranges.len(), 4);
        assert!(ranges.iter().all(|range| !range.is_empty()));
        assert_eq!(ranges.last().unwrap().end, extents.len());
    }

    #[test]
    fn serpentine_reversal_keeps_wrap_edge_short() {
        let (graph, rank_info, edges, mut nodes) = chain_fixture();
        let unfolded = nodes.clone();
        let config = config_with_goal(Some(GOAL_4_3));

        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config)
            .expect("wide chain with goal should fold");
        assert!((2..=4).contains(&outcome.band_count));

        // Wrap edge C->D (if 2 bands) or the boundary pair must be much
        // shorter than the unfolded distance between those ranks.
        let dist = |map: &BTreeMap<String, NodeLayout>, a: &str, b: &str| {
            let na = &map[a];
            let nb = &map[b];
            let ax = na.x + na.width / 2.0;
            let ay = na.y + na.height / 2.0;
            let bx = nb.x + nb.width / 2.0;
            let by = nb.y + nb.height / 2.0;
            ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
        };
        // The wrap hop across the first band boundary must be a short
        // vertical hop (one band of nodes plus the gutter), not a
        // full-width carriage return across the band span.
        let ids = ["A", "B", "C", "D", "E", "F"];
        let boundary = ids
            .windows(2)
            .find(|pair| outcome.node_bands[pair[0]] != outcome.node_bands[pair[1]])
            .expect("fold must create at least one band boundary");
        let gutter = config.rank_spacing.max(config.node_spacing);
        let node_height = 36.0;
        assert!(
            dist(&nodes, boundary[0], boundary[1]) <= node_height + gutter + 1.0,
            "wrap edge should be a short vertical hop after serpentine fold, got {}",
            dist(&nodes, boundary[0], boundary[1])
        );

        // Odd bands are mirrored: the first node of band 1 sits on the same
        // side as the last node of band 0 (serpentine, not carriage-return).
        let last_of_band0 = ids
            .iter()
            .rev()
            .find(|id| outcome.node_bands[**id] == 0)
            .unwrap();
        let first_of_band1 = ids.iter().find(|id| outcome.node_bands[**id] == 1).unwrap();
        let cx = |map: &BTreeMap<String, NodeLayout>, id: &str| {
            let n = &map[id];
            n.x + n.width / 2.0
        };
        assert!(
            (cx(&nodes, last_of_band0) - cx(&nodes, first_of_band1)).abs() < 1.0,
            "band boundary nodes should stack on the same side"
        );

        // Aspect must move toward the goal by the acceptance margin.
        let before = layout_aspect_error(&unfolded, GOAL_4_3).unwrap();
        let after = layout_aspect_error(&nodes, GOAL_4_3).unwrap();
        assert!(after <= before * ASPECT_IMPROVE_FACTOR);

        // Bands must not overlap: no node rectangle intersections.
        let list: Vec<&NodeLayout> = nodes.values().collect();
        for i in 0..list.len() {
            for j in (i + 1)..list.len() {
                let (a, b) = (list[i], list[j]);
                let overlap = a.x < b.x + b.width
                    && b.x < a.x + a.width
                    && a.y < b.y + b.height
                    && b.y < a.y + a.height;
                assert!(!overlap, "nodes {} and {} overlap after fold", a.id, b.id);
            }
        }
    }

    #[test]
    fn fold_rejected_when_aspect_near_goal() {
        let (graph, rank_info, edges, mut nodes) = chain_fixture();
        // Stretch the chain vertically so the natural ratio (~560/300 = 1.87,
        // i.e. ~1.4x the goal) sits below MIN_FOLD_TRIGGER.
        for node in nodes.values_mut() {
            node.height = 300.0;
        }
        let before = nodes.clone();
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_none());
        assert!(nodes_bit_identical(&before, &nodes));
    }

    #[test]
    fn fold_skipped_without_goal() {
        let (graph, rank_info, edges, mut nodes) = chain_fixture();
        let before = nodes.clone();
        let config = config_with_goal(None);
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_none());
        assert!(nodes_bit_identical(&before, &nodes));
    }

    #[test]
    fn fold_skipped_for_vertical_direction() {
        let (mut graph, rank_info, edges, mut nodes) = chain_fixture();
        graph.direction = Direction::TopDown;
        let before = nodes.clone();
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_none());
        assert!(nodes_bit_identical(&before, &nodes));
    }

    #[test]
    fn fold_keeps_subgraph_members_in_one_band() {
        // Subgraph over A..D spans ranks 0..=3, so the only allowed interior
        // boundary is at rank 4 (E). The fold must still fire and must keep
        // all subgraph members in one band.
        let (mut graph, rank_info, edges, mut nodes) = chain_fixture();
        graph.subgraphs.push(crate::ir::Subgraph {
            id: Some("sg".to_string()),
            label: "sg".to_string(),
            nodes: vec![
                "A".to_string(),
                "B".to_string(),
                "C".to_string(),
                "D".to_string(),
            ],
            direction: None,
            icon: None,
        });
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config)
            .expect("wide chain with a partial subgraph should still fold");
        let member_bands: Vec<usize> = ["A", "B", "C", "D"]
            .iter()
            .map(|id| outcome.node_bands[*id])
            .collect();
        assert!(
            member_bands.iter().all(|band| *band == member_bands[0]),
            "subgraph members split across bands: {member_bands:?}"
        );
    }

    #[test]
    fn fold_skipped_when_subgraph_spans_all_ranks() {
        let (mut graph, rank_info, edges, mut nodes) = chain_fixture();
        graph.subgraphs.push(crate::ir::Subgraph {
            id: Some("sg".to_string()),
            label: "sg".to_string(),
            nodes: ["A", "B", "C", "D", "E", "F"]
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
            direction: None,
            icon: None,
        });
        let before = nodes.clone();
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_none());
        assert!(nodes_bit_identical(&before, &nodes));
    }

    #[test]
    fn fold_skipped_for_non_flowchart_kind() {
        let (mut graph, rank_info, edges, mut nodes) = chain_fixture();
        graph.kind = DiagramKind::Class;
        let before = nodes.clone();
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_none());
        assert!(nodes_bit_identical(&before, &nodes));
    }

    #[test]
    fn fold_skipped_with_too_few_ranks() {
        let (graph, mut rank_info, edges, mut nodes) = chain_fixture();
        // Squash everything into 3 ranks.
        rank_info.rank_nodes = vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["C".to_string(), "D".to_string()],
            vec!["E".to_string(), "F".to_string()],
        ];
        let before = nodes.clone();
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_none());
        assert!(nodes_bit_identical(&before, &nodes));
    }

    #[test]
    fn straight_line_crossings_counts_intersections() {
        let mut nodes = BTreeMap::new();
        nodes.insert("A".to_string(), make_node("A", 0.0, 0.0, 10.0, 10.0));
        nodes.insert("B".to_string(), make_node("B", 100.0, 100.0, 10.0, 10.0));
        nodes.insert("C".to_string(), make_node("C", 100.0, 0.0, 10.0, 10.0));
        nodes.insert("D".to_string(), make_node("D", 0.0, 100.0, 10.0, 10.0));
        // X pattern: A->B crosses C->D.
        let crossing = vec![make_edge("A", "B"), make_edge("C", "D")];
        assert_eq!(straight_line_crossings(&crossing, &nodes), 1);
        // Parallel: A->C and D->B do not cross.
        let parallel = vec![make_edge("A", "C"), make_edge("D", "B")];
        assert_eq!(straight_line_crossings(&parallel, &nodes), 0);
        // Shared endpoints are skipped.
        let shared = vec![make_edge("A", "B"), make_edge("A", "C")];
        assert_eq!(straight_line_crossings(&shared, &nodes), 0);
    }

    #[test]
    fn fold_candidate_rejected_when_crossings_explode() {
        let baseline = FoldScore {
            aspect_error: 2.0,
            crossings: 0,
        };
        // Aspect improves hugely but crossings blow past 0 * 2 + 2.
        let too_many_crossings = FoldScore {
            aspect_error: 0.1,
            crossings: 3,
        };
        assert!(!fold_candidate_accepted(&baseline, &too_many_crossings));
        let within_budget = FoldScore {
            aspect_error: 0.1,
            crossings: 2,
        };
        assert!(fold_candidate_accepted(&baseline, &within_budget));
        // Aspect improvement below the required margin is rejected even with
        // zero crossings.
        let weak_improvement = FoldScore {
            aspect_error: 1.5,
            crossings: 0,
        };
        assert!(!fold_candidate_accepted(&baseline, &weak_improvement));
    }

    #[test]
    fn back_edge_role_preserved_after_fold() {
        let (mut graph, rank_info, _, mut nodes) = chain_fixture();
        graph.edges.push(make_edge("F", "A"));
        let edges = graph.edges.clone();
        let config = config_with_goal(Some(GOAL_4_3));
        let outcome = apply_aspect_ratio_band_fold(&graph, &rank_info, &edges, &mut nodes, &config);
        assert!(outcome.is_some(), "chain with back edge should still fold");
        // Roles are rank-based and the fold never touches ranks, so the back
        // edge keeps its classification on the folded layout.
        let roles = super::super::roles::classify_edge_roles(&graph);
        assert!(roles[edges.len() - 1].is_back_edge);
        assert!(!roles[0].is_back_edge);
    }
}
