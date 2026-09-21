//! Original frontier algorithm retained for differential tests.
use super::*;
/// A live lane. Its column is its index in the `lanes` vector, and that column
/// never changes for as long as the lane lives -- see [`Lanes`].
#[derive(Clone, Copy, Debug)]
struct LaneState {
    id: LaneId,
    color_ix: LaneColorIx,
    /// Index into the `commits` slice identifying which commit this lane is
    /// heading towards.  Using an index instead of a `&str` reference turns
    /// every target comparison from a 40-byte string compare into a `usize`
    /// compare.
    target_ix: usize,
    /// True once this lane has survived into a new row, i.e. its vertical
    /// continues from the row above. False on the lane's birth row, where it
    /// starts at the node instead.
    carried_in: bool,
    /// The column this lane was born into. Never changes; re-checked against the
    /// lane's actual index on every row under `debug_assert!` so the
    /// column-stability invariant cannot silently regress.
    home_col: u16,
}

/// Live lanes by column. `None` is a hole left behind by a lane that ended:
/// holes are kept rather than compacted away, because compacting would shift
/// every lane to the right of the removed one into a new column, and a lane that
/// changes column mid-life is drawn as a diagonal.
///
/// Only a brand-new lane may claim a hole (see `alloc_col`). Trailing holes are
/// truncated so `lanes.len()` still bounds the drawn width.
type Lanes = SmallVec<[Option<LaneState>; 4]>;

#[inline]
fn lane_at(lanes: &[Option<LaneState>], col: usize) -> Option<&LaneState> {
    lanes.get(col)?.as_ref()
}

/// Leftmost hole at or after `start`, or `lanes.len()` ("append a column") when
/// there is none.
fn free_col_from(lanes: &[Option<LaneState>], start: usize) -> usize {
    let start = start.min(lanes.len());
    lanes[start..]
        .iter()
        .position(Option::is_none)
        .map_or(lanes.len(), |offset| start + offset)
}

/// Column for a new lane: prefer a hole at or after `prefer_from`, but fall back
/// to *any* hole before widening the graph.
///
/// The fallback is what bounds the width. Lane lifetimes are contiguous row
/// intervals and lanes are born in row order, so leftmost-free assignment is
/// first-fit on an interval graph -- optimal. A column is only ever appended
/// when every existing column is live, so the drawn width never exceeds the peak
/// simultaneous-lane count, which is exactly what compaction used to achieve.
fn alloc_col(lanes: &[Option<LaneState>], prefer_from: usize) -> usize {
    let preferred = free_col_from(lanes, prefer_from);
    if preferred < lanes.len() {
        return preferred;
    }
    free_col_from(lanes, 0)
}

fn place_lane(lanes: &mut Lanes, col: usize, state: LaneState) {
    debug_assert!(col <= lanes.len());
    debug_assert_eq!(usize::from(state.home_col), col);
    if col == lanes.len() {
        lanes.push(Some(state));
    } else {
        debug_assert!(lanes[col].is_none(), "new lane would evict a live lane");
        lanes[col] = Some(state);
    }
}

/// State immediately before a row. Checkpoints retain this small lane frontier,
/// never the paint rows preceding it.
#[derive(Clone, Debug)]
pub(in crate::view) struct OracleGraphWalk {
    next_id: u32,
    next_color: usize,
    lanes: Lanes,
    main_lane_id: Option<LaneId>,
    main_target_ix: Option<usize>,
    seeded_main_lane_pending: bool,
}

impl OracleGraphWalk {
    pub(in crate::view) fn new(main_target_ix: Option<usize>) -> Self {
        let mut next_id = 1;
        let mut next_color = 0;
        let mut lanes = Lanes::new();
        let mut main_lane_id = None;
        let mut seeded_main_lane_pending = false;
        if let Some(main_target_ix) = main_target_ix {
            let id = LaneId(next_id);
            next_id += 1;
            lanes.push(Some(LaneState {
                id,
                color_ix: 0,
                target_ix: main_target_ix,
                carried_in: false,
                home_col: 0,
            }));
            main_lane_id = Some(id);
            next_color = 1;
            seeded_main_lane_pending = true;
        }

        Self {
            next_id,
            next_color,
            lanes,
            main_lane_id,
            main_target_ix,
            seeded_main_lane_pending,
        }
    }

    pub(in crate::view) fn step(
        &mut self,
        commit_ix: usize,
        parent_ixs: &[usize],
        is_merge: bool,
        is_branch_head: bool,
    ) -> GraphRow {
        let mut next_id = self.next_id;
        let mut next_color = self.next_color;
        let mut lanes = std::mem::take(&mut self.lanes);
        let main_lane_id = self.main_lane_id;
        let main_target_ix = self.main_target_ix;
        let seeded_main_lane_pending = self.seeded_main_lane_pending;
        let mut hits: SmallVec<[usize; 4]> = SmallVec::new();
        let mut ended_colors: SmallVec<[LaneColorIx; 4]> = SmallVec::new();
        let mut pick_lane_color_ix =
            |lanes: &[Option<LaneState>], avoid: &[LaneColorIx]| -> LaneColorIx {
                let start = next_color;
                for offset in 0..LANE_COLOR_PALETTE_SIZE {
                    let candidate = ((start + offset) % LANE_COLOR_PALETTE_SIZE) as LaneColorIx;
                    if lanes.iter().flatten().all(|l| l.color_ix != candidate)
                        && !avoid.contains(&candidate)
                    {
                        next_color = start + offset + 1;
                        return candidate;
                    }
                }
                let candidate = (start % LANE_COLOR_PALETTE_SIZE) as LaneColorIx;
                next_color = start + 1;
                candidate
            };

        // One pass: every surviving lane is now carried in from the row above,
        // its column is re-verified, and lanes aimed at this commit are gathered.
        hits.clear();
        ended_colors.clear();
        for (col, slot) in lanes.iter_mut().enumerate() {
            let Some(lane) = slot.as_mut() else { continue };
            debug_assert_eq!(
                usize::from(lane.home_col),
                col,
                "lane changed column mid-life"
            );
            lane.carried_in = true;
            if lane.target_ix == commit_ix {
                hits.push(col);
            }
        }
        let had_hit_lanes = !hits.is_empty();

        if hits.is_empty() {
            let id = LaneId(next_id);
            next_id += 1;
            let color_ix = pick_lane_color_ix(&lanes, &ended_colors);
            let col = alloc_col(&lanes, 0);
            place_lane(
                &mut lanes,
                col,
                LaneState {
                    id,
                    color_ix,
                    target_ix: commit_ix,
                    carried_in: false,
                    home_col: lane_col(col),
                },
            );
            hits.push(col);
        }

        // If a branch head points at a commit that's already reached by another lane (i.e. the
        // branch is behind some other branch), split a new lane at this row so the head has its
        // own lane/color instead of inheriting the descendant lane's color.
        //
        // We currently only do this for non-merge commits to avoid interfering with merge-parent
        // lane assignment.
        let only_hit_is_main_lane = hits.len() == 1
            && main_lane_id.is_some_and(|id| lane_at(&lanes, hits[0]).is_some_and(|l| l.id == id));
        let force_branch_head_lane = is_branch_head
            && had_hit_lanes
            && hits.len() == 1
            && parent_ixs.len() <= 1
            && !(main_target_ix == Some(commit_ix) && only_hit_is_main_lane);

        let mut node_col = if let Some(main_lane_id) = main_lane_id {
            hits.iter()
                .copied()
                .find(|&ix| lane_at(&lanes, ix).is_some_and(|l| l.id == main_lane_id))
                .or_else(|| hits.first().copied())
                .unwrap_or(0)
        } else {
            hits.first().copied().unwrap_or(0)
        };

        // The branch-head fork is drawn as a paint-only "whisker": a column that
        // exists on this row alone, joining into the node. It never becomes a
        // `LaneState`, so it cannot displace a live lane.
        //
        // When the head also takes over the continuation below the node
        // (`adopt_fork_color`), that is modelled as the old lane dying and a new
        // one being born *in the same column* -- which is what actually happens
        // -- rather than as a swap. `lanes_now` has already been snapshotted with
        // the old colour by then, so the segment above the node stays the
        // descendant's colour and everything below it is the head's.
        let fork_color_ix =
            force_branch_head_lane.then(|| pick_lane_color_ix(&lanes, &ended_colors));
        // The whisker only marks the head where it can sit immediately beside
        // the node. Reaching on to the next free column would draw a horizontal
        // straight across whatever live lanes lie between, which reads as a
        // stray line belonging to one of them rather than as a marker for this
        // head. The colour hand-over below does not depend on it.
        let fork = fork_color_ix.and_then(|color_ix| {
            let col = node_col + 1;
            lane_at(&lanes, col).is_none().then_some((col, color_ix))
        });
        let adopt_fork_color = force_branch_head_lane && !only_hit_is_main_lane;

        // Snapshot of lanes used for drawing this row. Dense over columns, so
        // holes are represented explicitly and the painter can keep using the
        // column index as the array index.
        let suppress_main_incoming = seeded_main_lane_pending && main_target_ix == Some(commit_ix);
        let now_len = lanes.len().max(fork.map_or(0, |(col, _)| col + 1));
        let mut lanes_now = LanePaints::with_capacity(now_len);
        for col in 0..now_len {
            lanes_now.push(match lane_at(&lanes, col) {
                Some(lane) => LanePaint::lane(
                    lane.color_ix,
                    lane.carried_in
                        && !(suppress_main_incoming
                            && main_lane_id.is_some_and(|mid| lane.id == mid)),
                    false,
                ),
                None => match fork {
                    Some((fork_col, color_ix)) if fork_col == col => {
                        LanePaint::lane(color_ix, false, false)
                    }
                    _ => LanePaint::HOLE,
                },
            });
        }

        if let Some(pos) = hits.iter().position(|&ix| ix == node_col) {
            hits.swap(0, pos);
        }

        // Ensure the node lane is the first hit lane for the parent assignment logic below.
        node_col = hits.first().copied().unwrap_or(node_col);

        // Incoming join edges: other lanes that were targeting this commit join into the node.
        let mut joins_in =
            GraphEdges::with_capacity(hits.len().saturating_sub(1) + usize::from(fork.is_some()));
        for &col in hits.iter().skip(1) {
            joins_in.push(GraphEdge {
                from_col: lane_col(col),
                to_col: lane_col(node_col),
                color_ix: lane_at(&lanes, col).map_or(0, |l| l.color_ix),
            });
        }
        if let Some((fork_col, color_ix)) = fork {
            joins_in.push(GraphEdge {
                from_col: lane_col(fork_col),
                to_col: lane_col(node_col),
                color_ix,
            });
        }

        // The node's colour: the fork colour when the branch head takes over the
        // lane, otherwise the colour of the lane the node sits on.
        let node_color_ix = match fork_color_ix {
            Some(color_ix) if adopt_fork_color => color_ix,
            _ => lane_at(&lanes, node_col).map_or(0, |l| l.color_ix),
        };

        // Ending a lane leaves a hole rather than compacting the vector.
        let end_lane = |lanes: &mut Lanes, ended: &mut SmallVec<[LaneColorIx; 4]>, col: usize| {
            if let Some(lane) = lanes.get_mut(col).and_then(Option::take) {
                ended.push(lane.color_ix);
            }
        };

        let mut covered_parents = 0usize;
        if parent_ixs.is_empty() {
            // No parents: end all lanes converging here.
            for &hit_ix in &hits {
                end_lane(&mut lanes, &mut ended_colors, hit_ix);
            }
        } else {
            if let Some(lane) = lanes.get_mut(node_col).and_then(Option::as_mut) {
                lane.target_ix = parent_ixs[0];
            }
            covered_parents = 1;

            for (&hit_ix, &parent_ix) in hits.iter().skip(1).zip(parent_ixs.iter().skip(1)) {
                if let Some(lane) = lanes.get_mut(hit_ix).and_then(Option::as_mut) {
                    lane.target_ix = parent_ix;
                }
                covered_parents += 1;
            }

            // End hit lanes that converged here but don't have a parent to follow.
            for &hit_ix in hits.iter().skip(parent_ixs.len().min(hits.len())) {
                end_lane(&mut lanes, &mut ended_colors, hit_ix);
            }
        }

        // Branch-head hand-over: the descendant lane dies at the node and the
        // head is born in the same column. `home_col` is deliberately untouched
        // -- the column is precisely what stays put.
        if adopt_fork_color
            && let Some(color_ix) = fork_color_ix
            && let Some(lane) = lanes.get_mut(node_col).and_then(Option::as_mut)
        {
            ended_colors.push(lane.color_ix);
            lane.id = LaneId(next_id);
            next_id += 1;
            lane.color_ix = color_ix;
            lane.carried_in = false;
        }

        // Create lanes for any remaining parents not covered by existing converged lanes.
        // Each claims a hole rather than being inserted, which would shift its
        // neighbours into new columns.
        if parent_ixs.len() > covered_parents {
            for &parent_ix in parent_ixs.iter().skip(covered_parents) {
                // If another lane already targets this parent, reuse it.
                if lanes.iter().flatten().any(|l| l.target_ix == parent_ix) {
                    continue;
                }
                let id = LaneId(next_id);
                next_id += 1;
                let color_ix = pick_lane_color_ix(&lanes, &ended_colors);
                let col = alloc_col(&lanes, node_col + 1);
                place_lane(
                    &mut lanes,
                    col,
                    LaneState {
                        id,
                        color_ix,
                        target_ix: parent_ix,
                        carried_in: false,
                        home_col: lane_col(col),
                    },
                );
            }
        }

        // Trailing holes are not columns.
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }

        // Build lanes_next directly from the lane state. Every surviving lane
        // continues straight down its own column; only lanes born on this row
        // start at the node.
        let mut lanes_next = LanePaints::with_capacity(lanes.len());
        for (col, slot) in lanes.iter().enumerate() {
            lanes_next.push(match slot {
                Some(lane) => {
                    debug_assert_eq!(
                        usize::from(lane.home_col),
                        col,
                        "lane changed column mid-life"
                    );
                    LanePaint::lane(lane.color_ix, false, !lane.carried_in)
                }
                None => LanePaint::HOLE,
            });
        }

        // Node->parent "merge" edges: connect the node into secondary-parent lanes.
        // - If the secondary parent lane existed already in this row, draw an explicit edge.
        // - If it was inserted this row, the continuation line already originates from the node.
        let mut edges_out = GraphEdges::with_capacity(parent_ixs.len().saturating_sub(1));
        for &parent_ix in parent_ixs.iter().skip(1) {
            if let Some((to_col, lane)) = lanes
                .iter()
                .enumerate()
                .filter_map(|(col, slot)| slot.as_ref().map(|lane| (col, lane)))
                .find(|(_, lane)| lane.target_ix == parent_ix && lane.carried_in)
            {
                edges_out.push(GraphEdge {
                    from_col: lane_col(node_col),
                    to_col: lane_col(to_col),
                    color_ix: lane.color_ix,
                });
            }
        }

        let from_node_cols = super::from_node_cols_of(&lanes_next);
        let row = GraphRow {
            lanes_now,
            lanes_next,
            joins_in,
            edges_out,
            node_col: lane_col(node_col),
            node_color_ix,
            is_merge,
            from_node_cols,
        };

        self.next_id = next_id;
        self.next_color = next_color;
        self.lanes = lanes;
        self.seeded_main_lane_pending = false;
        row
    }
}
