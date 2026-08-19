use crate::theme::{AppTheme, GRAPH_LANE_PALETTE_SIZE};
use gitcomet_core::domain::{Commit, CommitId};
use gpui::Rgba;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smallvec::SmallVec;

const LANE_COLOR_PALETTE_SIZE: usize = GRAPH_LANE_PALETTE_SIZE;
/// `LanePaint` is two bytes, so eight columns fit in the same 24-byte `SmallVec`
/// that three six-byte paints used to need 32 bytes for. Rows now carry holes
/// for ended lanes (see `LaneState::home_col`), which makes them longer than the
/// old compacted rows -- this keeps those rows off the heap.
const INLINE_LANE_CAPACITY: usize = 8;
const INLINE_EDGE_CAPACITY: usize = 2;

type LanePaints = SmallVec<[LanePaint; INLINE_LANE_CAPACITY]>;
type GraphEdges = SmallVec<[GraphEdge; INLINE_EDGE_CAPACITY]>;
pub(in crate::view) type LaneColorIx = u8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct LaneId(pub u32);

/// One column of one row. Two bytes: `lanes_now` / `lanes_next` are dense and
/// indexed by column, so there is one of these per column per row across up to
/// 50k rows.
///
/// Deliberately carries no "which column did this come from" field. A lane keeps
/// its column for its whole life (see [`LaneState::home_col`]), so the only
/// distinction left is whether the lane continues from the row above or starts
/// at this row's node -- and a field that *can* encode a foreign column is a
/// field that can reintroduce the diagonal it replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanePaint {
    /// Meaningless when the lane is inactive.
    pub color_ix: LaneColorIx,
    flags: u8,
}

impl LanePaint {
    const ACTIVE: u8 = 1 << 0;
    /// `lanes_now` only: this lane has an incoming segment from the row above.
    const INCOMING: u8 = 1 << 1;
    /// `lanes_next` only: this lane starts at this row's node rather than
    /// continuing down from the row above.
    const FROM_NODE: u8 = 1 << 2;

    /// No lane occupies this column on this row.
    pub const HOLE: Self = Self {
        color_ix: 0,
        flags: 0,
    };

    #[inline]
    pub(in crate::view) const fn lane(
        color_ix: LaneColorIx,
        incoming: bool,
        from_node: bool,
    ) -> Self {
        let mut flags = Self::ACTIVE;
        if incoming {
            flags |= Self::INCOMING;
        }
        if from_node {
            flags |= Self::FROM_NODE;
        }
        Self { color_ix, flags }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.flags & Self::ACTIVE != 0
    }

    #[inline]
    pub const fn incoming(self) -> bool {
        self.flags & Self::INCOMING != 0
    }

    #[inline]
    pub const fn starts_at_node(self) -> bool {
        self.flags & Self::FROM_NODE != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GraphEdge {
    pub from_col: u16,
    pub to_col: u16,
    pub color_ix: LaneColorIx,
}

#[derive(Clone, Debug)]
pub struct GraphRow {
    pub lanes_now: LanePaints,
    pub lanes_next: LanePaints,
    pub joins_in: GraphEdges,
    pub edges_out: GraphEdges,
    pub node_col: u16,
    /// Colour of the node dot. Not derivable from `lanes_now[node_col]`: when a
    /// branch head takes over its own lane, the segment *above* the node keeps
    /// the descendant lane's colour while the node and everything below it use
    /// the head's new colour.
    pub node_color_ix: LaneColorIx,
    pub is_merge: bool,
}

trait GraphCommitLike {
    fn id_str(&self) -> &str;
    fn parent_ids(&self) -> &[CommitId];
}

impl GraphCommitLike for Commit {
    fn id_str(&self) -> &str {
        self.id.as_ref()
    }

    fn parent_ids(&self) -> &[CommitId] {
        &self.parent_ids
    }
}

impl GraphCommitLike for &Commit {
    fn id_str(&self) -> &str {
        self.id.as_ref()
    }

    fn parent_ids(&self) -> &[CommitId] {
        &self.parent_ids
    }
}

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

#[inline]
fn lane_col(col: usize) -> u16 {
    u16::try_from(col).expect("history graph lane column overflow")
}

/// Colour of a graph lane, and with it the commit dot, the ref-column fade and
/// the message border that are tinted to match it.
///
/// Reads the theme's palette rather than a generated ramp, so a theme supplying
/// `graph_lane_palette` / `graph_lane_hues` actually takes effect. The built-in
/// themes generate the same ramp this used to hardcode, so their graphs are
/// unchanged.
#[inline]
pub fn lane_color(theme: AppTheme, color_ix: LaneColorIx) -> Rgba {
    theme.graph_lane_palette.color_at(color_ix)
}

#[inline]
fn single_lane_paints(lane: LanePaint) -> LanePaints {
    let mut lanes = LanePaints::new();
    lanes.push(lane);
    lanes
}

fn compute_linear_visible_history_fast_path<C: GraphCommitLike>(
    commits: &[C],
    has_branch_heads: bool,
    active_head_target: Option<&str>,
) -> Option<Vec<GraphRow>> {
    let Some(first_commit) = commits.first() else {
        return Some(Vec::new());
    };

    if has_branch_heads {
        return None;
    }
    if active_head_target.is_some_and(|target| target != first_commit.id_str()) {
        return None;
    }

    let first_row_lane = LanePaint::lane(0, false, false);
    let continuing_lane = LanePaint::lane(0, true, false);
    // Continues down its own column, so not `from_node`.
    let next_lane = LanePaint::lane(0, false, false);

    if commits.len() == 1 {
        return (first_commit.parent_ids().len() <= 1).then(|| {
            vec![GraphRow {
                lanes_now: single_lane_paints(first_row_lane),
                lanes_next: LanePaints::new(),
                joins_in: GraphEdges::new(),
                edges_out: GraphEdges::new(),
                node_col: 0,
                node_color_ix: 0,
                is_merge: false,
            }]
        });
    }

    let mut rows = Vec::with_capacity(commits.len());
    for ix in 0..(commits.len() - 1) {
        let commit = &commits[ix];
        let next_commit = &commits[ix + 1];
        let parent_ids = commit.parent_ids();
        if parent_ids.len() != 1 || parent_ids[0].as_ref() != next_commit.id_str() {
            return None;
        }

        rows.push(GraphRow {
            lanes_now: single_lane_paints(if ix == 0 {
                first_row_lane
            } else {
                continuing_lane
            }),
            lanes_next: single_lane_paints(next_lane),
            joins_in: GraphEdges::new(),
            edges_out: GraphEdges::new(),
            node_col: 0,
            node_color_ix: 0,
            is_merge: false,
        });
    }

    if commits[commits.len() - 1].parent_ids().len() > 1 {
        return None;
    }

    rows.push(GraphRow {
        lanes_now: single_lane_paints(continuing_lane),
        lanes_next: LanePaints::new(),
        joins_in: GraphEdges::new(),
        edges_out: GraphEdges::new(),
        node_col: 0,
        node_color_ix: 0,
        is_merge: false,
    });
    Some(rows)
}

fn compute_graph_impl<'a, C, I>(
    commits: &[C],
    _theme: AppTheme,
    branch_heads: I,
    active_head_target: Option<&str>,
) -> Vec<GraphRow>
where
    C: GraphCommitLike,
    I: IntoIterator<Item = &'a str>,
{
    let branch_heads: SmallVec<[&str; 8]> = branch_heads.into_iter().collect();
    let has_branch_heads = !branch_heads.is_empty();
    if let Some(graph) =
        compute_linear_visible_history_fast_path(commits, has_branch_heads, active_head_target)
    {
        return graph;
    }

    let mut required_lookup_ids: FxHashSet<&str> = FxHashSet::with_capacity_and_hasher(
        branch_heads.len() + usize::from(active_head_target.is_some()) + commits.len().min(256),
        Default::default(),
    );
    if let Some(target) = active_head_target {
        required_lookup_ids.insert(target);
    }
    if has_branch_heads {
        required_lookup_ids.extend(branch_heads.iter().copied());
    }
    for (commit_ix, commit) in commits.iter().enumerate() {
        let parent_ids = commit.parent_ids();
        if let Some(first_parent) = parent_ids.first() {
            let next_ix = commit_ix + 1;
            if next_ix >= commits.len() || commits[next_ix].id_str() != first_parent.as_ref() {
                required_lookup_ids.insert(first_parent.as_ref());
            }
        }
        for parent in parent_ids.iter().skip(1) {
            required_lookup_ids.insert(parent.as_ref());
        }
    }

    let id_to_index: FxHashMap<&str, usize> = if required_lookup_ids.is_empty() {
        FxHashMap::default()
    } else if required_lookup_ids.len().saturating_mul(2) < commits.len() {
        let mut id_to_index =
            FxHashMap::with_capacity_and_hasher(required_lookup_ids.len(), Default::default());
        for (ix, commit) in commits.iter().enumerate() {
            let id = commit.id_str();
            if required_lookup_ids.remove(id) {
                id_to_index.insert(id, ix);
                if required_lookup_ids.is_empty() {
                    break;
                }
            }
        }
        id_to_index
    } else {
        let mut id_to_index =
            FxHashMap::with_capacity_and_hasher(commits.len(), Default::default());
        for (ix, commit) in commits.iter().enumerate() {
            id_to_index.insert(commit.id_str(), ix);
        }
        id_to_index
    };
    let main_target_ix = active_head_target
        .and_then(|id| id_to_index.get(id).copied())
        .or_else(|| (!commits.is_empty()).then_some(0));
    let mut branch_head_mask = Vec::new();
    if has_branch_heads {
        branch_head_mask.resize(commits.len(), false);
        for branch_head in branch_heads.iter().copied() {
            if let Some(&ix) = id_to_index.get(branch_head) {
                branch_head_mask[ix] = true;
            }
        }
    }

    let mut next_id: u32 = 1;
    let mut next_color: usize = 0;
    let mut lanes: Lanes = SmallVec::new();
    let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
    let mut main_lane_id: Option<LaneId> = None;
    let mut hits: SmallVec<[usize; 4]> = SmallVec::new();
    let mut parent_ixs: SmallVec<[usize; 4]> = SmallVec::new();
    // Colours of lanes that ended on the current row. They are gone from `lanes`
    // by the time later lanes on the same row pick a colour, but their incoming
    // segment is still drawn above the node, so a new lane reusing the colour
    // would read as a continuation of the lane that just ended.
    let mut ended_colors: SmallVec<[LaneColorIx; 4]> = SmallVec::new();
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

    for (commit_ix, commit) in commits.iter().enumerate() {
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

        let is_merge = commit.parent_ids().len() > 1;
        parent_ixs.clear();
        for (parent_pos, parent) in commit.parent_ids().iter().enumerate() {
            let parent_ix = if parent_pos == 0 {
                resolve_first_parent_ix(commits, &id_to_index, commit_ix, parent.as_ref())
            } else {
                id_to_index.get(parent.as_ref()).copied()
            };
            if let Some(parent_ix) = parent_ix.filter(|&parent_ix| parent_ix > commit_ix) {
                parent_ixs.push(parent_ix);
            }
        }
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
        let force_branch_head_lane = has_branch_heads
            && had_hit_lanes
            && hits.len() == 1
            && branch_head_mask[commit_ix]
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

        rows.push(GraphRow {
            lanes_now,
            lanes_next,
            joins_in,
            edges_out,
            node_col: lane_col(node_col),
            node_color_ix,
            is_merge,
        });

        seeded_main_lane_pending = false;
    }

    rows
}

pub fn compute_graph<'a, I>(
    commits: &[Commit],
    theme: AppTheme,
    branch_heads: I,
    active_head_target: Option<&str>,
) -> Vec<GraphRow>
where
    I: IntoIterator<Item = &'a str>,
{
    compute_graph_impl(commits, theme, branch_heads, active_head_target)
}

pub fn compute_graph_refs<'a, 'commit, I>(
    commits: &[&'commit Commit],
    theme: AppTheme,
    branch_heads: I,
    active_head_target: Option<&str>,
) -> Vec<GraphRow>
where
    I: IntoIterator<Item = &'a str>,
{
    compute_graph_impl(commits, theme, branch_heads, active_head_target)
}

fn resolve_first_parent_ix<C: GraphCommitLike>(
    commits: &[C],
    id_to_index: &FxHashMap<&str, usize>,
    commit_ix: usize,
    parent_id: &str,
) -> Option<usize> {
    let next_ix = commit_ix + 1;
    // Most log rows continue along the first parent to the next visible row.
    if next_ix < commits.len() && commits[next_ix].id_str() == parent_id {
        Some(next_ix)
    } else {
        id_to_index.get(parent_id).copied()
    }
}

#[cfg(test)]
mod tests {
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
}
