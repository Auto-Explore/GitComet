use crate::theme::{AppTheme, GRAPH_LANE_PALETTE_SIZE};
use gitcomet_core::domain::{Commit, CommitId};
use gpui::Rgba;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smallvec::SmallVec;

const LANE_COLOR_PALETTE_SIZE: usize = GRAPH_LANE_PALETTE_SIZE;
/// `LanePaint` is two bytes, so eight columns fit in the same 24-byte `SmallVec`
/// that three six-byte paints used to need 32 bytes for. Rows now carry holes
/// for ended lanes (see the stable frontier columns), which makes them longer than the
/// old compacted rows -- this keeps those rows off the heap.
const INLINE_LANE_CAPACITY: usize = 8;
const INLINE_EDGE_CAPACITY: usize = 2;

type LanePaints = SmallVec<[LanePaint; INLINE_LANE_CAPACITY]>;
type GraphEdges = SmallVec<[GraphEdge; INLINE_EDGE_CAPACITY]>;
pub(in crate::view) type LaneColorIx = u8;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct LaneId(pub u32);

/// One column of one row. Two bytes: `lanes_now` / `lanes_next` are dense and
/// indexed by column, so there is one of these per column per row across up to
/// 50k rows.
///
/// Deliberately carries no "which column did this come from" field. A lane keeps
/// its column for its whole life (see the stable frontier columns), so the only
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
    /// The HEAD lane above anything that reaches it: kept free for the
    /// uncommitted-changes row, and only drawn while that row is shown.
    const RESERVED: u8 = 1 << 3;

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
    pub(in crate::view) const fn with_reserved(self, reserved: bool) -> Self {
        if reserved {
            Self {
                color_ix: self.color_ix,
                flags: self.flags | Self::RESERVED,
            }
        } else {
            self
        }
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

    #[inline]
    pub const fn reserved(self) -> bool {
        self.flags & Self::RESERVED != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub from_col: u16,
    pub to_col: u16,
    pub color_ix: LaneColorIx,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Columns of `lanes_next` whose lane starts at this row's node, ascending.
    /// A row births at most a few lanes, so the painter resolves them without
    /// scanning every column pinned past the graph's edge.
    pub from_node_cols: FromNodeCols,
}

pub(in crate::view) type FromNodeCols = SmallVec<[u16; 2]>;

/// [`GraphRow::from_node_cols`] read back from a dense `lanes_next`, for rows
/// the tests build outside the frontier walk.
#[cfg(test)]
pub(in crate::view) fn from_node_cols_of(lanes_next: &[LanePaint]) -> FromNodeCols {
    lanes_next
        .iter()
        .enumerate()
        .filter(|(_, lane)| lane.is_active() && lane.starts_at_node())
        .map(|(col, _)| lane_col(col))
        .collect()
}

pub(in crate::view) trait GraphCommitLike {
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
                from_node_cols: FromNodeCols::new(),
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
            from_node_cols: FromNodeCols::new(),
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
        from_node_cols: FromNodeCols::new(),
    });
    Some(rows)
}

/// The lanes and colour indices of `commits`. No theme changes them: a theme
/// only gives the indices their colours when they paint.
pub(in crate::view) fn compute_graph_lanes<'a, C, I>(
    commits: &[C],
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

    let mut walk = GraphWalk::new(main_target_ix);
    let mut rows = Vec::with_capacity(commits.len());
    let mut parent_ixs: SmallVec<[usize; 4]> = SmallVec::new();
    for (commit_ix, commit) in commits.iter().enumerate() {
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
        rows.push(walk.step(
            commit_ix,
            &parent_ixs,
            is_merge,
            branch_head_mask.get(commit_ix).copied().unwrap_or(false),
        ));
    }
    rows
}

#[cfg(test)]
#[path = "history_graph_oracle.rs"]
mod oracle;
#[path = "history_graph_walk.rs"]
mod walk;
#[cfg(test)]
pub(in crate::view) use walk::PLAIN_STEPS_OFF;
pub(in crate::view) use walk::{GraphCheckpoint, GraphTransition, GraphWalk};

/// The whole-page graph as tests and benchmarks ask for it.
#[cfg(any(test, feature = "benchmarks"))]
pub fn compute_graph<'a, I>(
    commits: &[Commit],
    _theme: AppTheme,
    branch_heads: I,
    active_head_target: Option<&str>,
) -> Vec<GraphRow>
where
    I: IntoIterator<Item = &'a str>,
{
    compute_graph_lanes(commits, branch_heads, active_head_target)
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
mod tests;
