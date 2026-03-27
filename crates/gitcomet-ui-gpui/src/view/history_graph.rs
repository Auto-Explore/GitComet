use crate::theme::AppTheme;
use gitcomet_core::domain::{Commit, CommitId};
use gpui::Rgba;
use rustc_hash::FxHashMap as HashMap;
use smallvec::SmallVec;
use std::sync::OnceLock;

const LANE_COLOR_PALETTE_SIZE: usize = 64;
const INLINE_LANE_CAPACITY: usize = 3;
const INLINE_EDGE_CAPACITY: usize = 2;

type LanePaints = SmallVec<[LanePaint; INLINE_LANE_CAPACITY]>;
type GraphEdges = SmallVec<[GraphEdge; INLINE_EDGE_CAPACITY]>;
type LaneColorIx = u8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct LaneId(pub u32);

#[derive(Clone, Copy, Debug)]
pub struct LanePaint {
    pub color_ix: LaneColorIx,
    /// Whether this lane has an incoming segment from the previous row.
    /// Only meaningful in `lanes_now`; always `false` in `lanes_next`.
    pub incoming: bool,
    /// Which column in `lanes_now` this lane continues from.
    /// Only meaningful in `lanes_next`; always `None` in `lanes_now`.
    pub from_col: Option<u16>,
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

#[derive(Clone, Copy, Debug)]
struct LaneState {
    id: LaneId,
    color_ix: LaneColorIx,
    /// Index into the `commits` slice identifying which commit this lane is
    /// heading towards.  Using an index instead of a `&str` reference turns
    /// every target comparison from a 40-byte string compare into a `usize`
    /// compare.
    target_ix: usize,
    /// Column this lane occupied in the current row before lane mutation.
    prev_col: Option<u16>,
}

fn lane_color_palette(is_dark: bool) -> &'static [Rgba; LANE_COLOR_PALETTE_SIZE] {
    static DARK: OnceLock<[Rgba; LANE_COLOR_PALETTE_SIZE]> = OnceLock::new();
    static LIGHT: OnceLock<[Rgba; LANE_COLOR_PALETTE_SIZE]> = OnceLock::new();

    let build = |light: f32| {
        std::array::from_fn(|i| {
            let hue = (i as f32 * 0.13) % 1.0;
            gpui::hsla(hue, 0.75, light, 1.0).into()
        })
    };

    if is_dark {
        DARK.get_or_init(|| build(0.62))
    } else {
        LIGHT.get_or_init(|| build(0.45))
    }
}

#[inline]
fn lane_col(col: usize) -> u16 {
    u16::try_from(col).expect("history graph lane column overflow")
}

#[inline]
pub fn lane_color(theme: AppTheme, color_ix: LaneColorIx) -> Rgba {
    lane_color_palette(theme.is_dark)[usize::from(color_ix)]
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
    if commits.windows(2).any(|pair| {
        pair[0].parent_ids().len() != 1 || pair[0].parent_ids()[0].as_ref() != pair[1].id_str()
    }) {
        return None;
    }
    if commits
        .last()
        .is_some_and(|commit| commit.parent_ids().len() > 1)
    {
        return None;
    }

    let first_row_lane = LanePaint {
        color_ix: 0,
        incoming: false,
        from_col: None,
    };
    let continuing_lane = LanePaint {
        color_ix: 0,
        incoming: true,
        from_col: None,
    };
    let next_lane = LanePaint {
        color_ix: 0,
        incoming: false,
        from_col: Some(0),
    };
    let mut rows = Vec::with_capacity(commits.len());

    for ix in 0..commits.len() {
        let mut lanes_now = LanePaints::with_capacity(1);
        lanes_now.push(if ix == 0 {
            first_row_lane
        } else {
            continuing_lane
        });

        let mut lanes_next = LanePaints::new();
        if ix + 1 < commits.len() {
            lanes_next.push(next_lane);
        }

        rows.push(GraphRow {
            lanes_now,
            lanes_next,
            joins_in: GraphEdges::new(),
            edges_out: GraphEdges::new(),
            node_col: 0,
            is_merge: false,
        });
    }

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
    let mut branch_heads = branch_heads.into_iter();
    let first_branch_head = branch_heads.next();
    let has_branch_heads = first_branch_head.is_some();
    if let Some(graph) =
        compute_linear_visible_history_fast_path(commits, has_branch_heads, active_head_target)
    {
        return graph;
    }

    let mut id_to_index: HashMap<&str, usize> =
        HashMap::with_capacity_and_hasher(commits.len(), Default::default());
    for (ix, commit) in commits.iter().enumerate() {
        id_to_index.insert(commit.id_str(), ix);
    }
    let main_target_ix = active_head_target
        .and_then(|id| id_to_index.get(id).copied())
        .or_else(|| (!commits.is_empty()).then_some(0));
    let mut branch_head_mask = Vec::new();
    if has_branch_heads {
        branch_head_mask.resize(commits.len(), false);
        for branch_head in first_branch_head.into_iter().chain(branch_heads) {
            if let Some(&ix) = id_to_index.get(branch_head) {
                branch_head_mask[ix] = true;
            }
        }
    }

    let mut next_id: u32 = 1;
    let mut next_color: usize = 0;
    let mut lanes: SmallVec<[LaneState; 4]> = SmallVec::new();
    let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
    let mut main_lane_id: Option<LaneId> = None;
    let mut hits: SmallVec<[usize; 4]> = SmallVec::new();
    let mut parent_ixs: SmallVec<[usize; 4]> = SmallVec::new();
    let mut seeded_main_lane_pending = false;

    if let Some(main_target_ix) = main_target_ix {
        let id = LaneId(next_id);
        next_id += 1;
        lanes.push(LaneState {
            id,
            color_ix: 0,
            target_ix: main_target_ix,
            prev_col: None,
        });
        main_lane_id = Some(id);
        next_color = 1;
        seeded_main_lane_pending = true;
    }

    let mut pick_lane_color_ix = |lanes: &[LaneState]| -> LaneColorIx {
        let start = next_color;
        for offset in 0..LANE_COLOR_PALETTE_SIZE {
            let candidate = ((start + offset) % LANE_COLOR_PALETTE_SIZE) as LaneColorIx;
            if lanes.iter().all(|l| l.color_ix != candidate) {
                next_color = start + offset + 1;
                return candidate;
            }
        }
        let candidate = (start % LANE_COLOR_PALETTE_SIZE) as LaneColorIx;
        next_color = start + 1;
        candidate
    };

    for (commit_ix, commit) in commits.iter().enumerate() {
        let incoming_lane_count = lanes.len();

        hits.clear();
        for (ix, lane) in lanes.iter().enumerate() {
            if lane.target_ix == commit_ix {
                hits.push(ix);
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
            if let Some(parent_ix) = parent_ix {
                parent_ixs.push(parent_ix);
            }
        }
        if hits.is_empty() {
            let id = LaneId(next_id);
            next_id += 1;
            let color_ix = pick_lane_color_ix(&lanes);
            lanes.push(LaneState {
                id,
                color_ix,
                target_ix: commit_ix,
                prev_col: None,
            });
            hits.push(lanes.len() - 1);
        }

        // If a branch head points at a commit that's already reached by another lane (i.e. the
        // branch is behind some other branch), split a new lane at this row so the head has its
        // own lane/color instead of inheriting the descendant lane's color.
        //
        // We currently only do this for non-merge commits to avoid interfering with merge-parent
        // lane assignment.
        let only_hit_is_main_lane = hits.len() == 1
            && main_lane_id.is_some_and(|id| lanes.get(hits[0]).is_some_and(|lane| lane.id == id));
        let force_branch_head_lane = has_branch_heads
            && had_hit_lanes
            && hits.len() == 1
            && branch_head_mask[commit_ix]
            && parent_ixs.len() <= 1
            && !(main_target_ix == Some(commit_ix) && only_hit_is_main_lane);

        let mut node_col = if let Some(main_lane_id) = main_lane_id {
            hits.iter()
                .copied()
                .find(|&ix| lanes[ix].id == main_lane_id)
                .or_else(|| hits.first().copied())
                .unwrap_or(0)
        } else {
            hits.first().copied().unwrap_or(0)
        };

        let keep_main_lane_as_node = force_branch_head_lane && only_hit_is_main_lane;
        let mut swap_node_into_col: Option<usize> = None;
        if force_branch_head_lane {
            let id = LaneId(next_id);
            next_id += 1;
            let color_ix = pick_lane_color_ix(&lanes);
            if !keep_main_lane_as_node {
                swap_node_into_col = Some(node_col);
                node_col = lanes.len();
            }
            lanes.push(LaneState {
                id,
                color_ix,
                target_ix: commit_ix,
                prev_col: None,
            });
            hits.push(lanes.len() - 1);
        }

        // Snapshot of lanes used for drawing this row.  The `incoming` flag on
        // each lane replaces the former separate `incoming_mask` Vec, saving one
        // heap allocation per row.
        let suppress_main_incoming = seeded_main_lane_pending && main_target_ix == Some(commit_ix);
        let mut lanes_now = LanePaints::with_capacity(lanes.len());
        for (col, lane) in lanes.iter_mut().enumerate() {
            lane.prev_col = (col < incoming_lane_count).then(|| lane_col(col));
            let incoming = lane.prev_col.is_some()
                && !(suppress_main_incoming && main_lane_id.is_some_and(|mid| lane.id == mid));
            lanes_now.push(LanePaint {
                color_ix: lane.color_ix,
                incoming,
                from_col: None,
            });
        }

        if let Some(pos) = hits.iter().position(|&ix| ix == node_col) {
            hits.swap(0, pos);
        }

        // Ensure the node lane is the first hit lane for the parent assignment logic below.
        node_col = hits.first().copied().unwrap_or(node_col);

        // Incoming join edges: other lanes that were targeting this commit join into the node.
        let mut joins_in = GraphEdges::with_capacity(hits.len().saturating_sub(1));
        for &col in hits.iter().skip(1) {
            joins_in.push(GraphEdge {
                from_col: lane_col(col),
                to_col: lane_col(node_col),
                color_ix: lanes[col].color_ix,
            });
        }

        let mut covered_parents = 0usize;
        if parent_ixs.is_empty() {
            // No parents: end all lanes converging here.
            for &hit_ix in &hits {
                lanes[hit_ix].target_ix = commit_ix;
            }
        } else {
            lanes[node_col].target_ix = parent_ixs[0];
            covered_parents = 1;

            for (&hit_ix, &parent_ix) in hits.iter().skip(1).zip(parent_ixs.iter().skip(1)) {
                lanes[hit_ix].target_ix = parent_ix;
                covered_parents += 1;
            }

            // End hit lanes that converged here but don't have a parent to follow.
            for &hit_ix in hits.iter().skip(parent_ixs.len().min(hits.len())) {
                lanes[hit_ix].target_ix = commit_ix;
            }
        }

        // Create lanes for any remaining parents not covered by existing converged lanes.
        if parent_ixs.len() > covered_parents {
            let mut insert_at = node_col + 1;
            for &parent_ix in parent_ixs.iter().skip(covered_parents) {
                // If another lane already targets this parent, reuse it.
                if lanes.iter().any(|l| l.target_ix == parent_ix) {
                    continue;
                }
                let id = LaneId(next_id);
                next_id += 1;
                let color_ix = pick_lane_color_ix(&lanes);
                lanes.insert(
                    insert_at,
                    LaneState {
                        id,
                        color_ix,
                        target_ix: parent_ix,
                        prev_col: None,
                    },
                );
                insert_at += 1;
            }
        }

        if let Some(swap_col) = swap_node_into_col {
            lanes.swap(node_col, swap_col);
        }

        // Remove ended lanes: lanes targeting this commit (no parent to follow).
        // All valid target indices are in the commits array by construction.
        lanes.retain(|lane| lane.target_ix != commit_ix);

        // Build lanes_next directly from the lane state. Existing lanes carry their prior column
        // through `prev_col`, while lanes created mid-row keep `None`.
        let mut lanes_next = LanePaints::with_capacity(lanes.len());
        for lane in lanes.iter() {
            lanes_next.push(LanePaint {
                color_ix: lane.color_ix,
                incoming: false,
                from_col: lane.prev_col,
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
                .find(|(_, lane)| lane.target_ix == parent_ix && lane.prev_col.is_some())
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
    id_to_index: &HashMap<&str, usize>,
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
        let theme = AppTheme::zed_ayu_dark();
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

    #[test]
    fn branch_heads_split_off_new_lane_when_behind() {
        let theme = AppTheme::zed_ayu_dark();
        let commits = vec![
            commit("new1", vec!["base"]),
            commit("base", vec!["root"]),
            commit("root", Vec::new()),
        ];

        let branch_heads = ["new1", "base"];
        let graph = compute_graph(&commits, theme, branch_heads, None);

        let base_row = &graph[1];
        assert_eq!(base_row.lanes_now.len(), 2);
        assert!(base_row.lanes_now[0].incoming);
        assert!(!base_row.lanes_now[1].incoming);
        assert_eq!(base_row.joins_in.len(), 1);
        assert_eq!(base_row.node_col, 0);
        assert_ne!(
            base_row.lanes_now[0].color_ix,
            base_row.lanes_now[1].color_ix
        );

        assert_eq!(base_row.lanes_next.len(), 1);
        assert_eq!(base_row.lanes_next[0].from_col, Some(0));
    }

    #[test]
    fn branch_heads_do_not_split_when_multiple_lanes_converge() {
        let theme = AppTheme::zed_ayu_dark();
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
        assert_eq!(base_row.lanes_next[0].from_col, Some(0));
    }

    #[test]
    fn active_head_lane_stays_leftmost_even_when_head_commit_appears_later() {
        let theme = AppTheme::zed_ayu_dark();
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
        assert!(graph[0].lanes_now[0].incoming);
        assert!(!graph[0].lanes_now[1].incoming);
        assert_eq!(graph[0].node_col, 1);
        assert_eq!(graph[1].node_col, 0);
        assert_eq!(graph[2].node_col, 0);
        assert_eq!(graph[1].lanes_now[0].color_ix, seeded_lane);
        assert_eq!(graph[2].lanes_now[0].color_ix, seeded_lane);
    }

    #[test]
    fn inserted_secondary_parent_lane_has_no_previous_column() {
        let theme = AppTheme::zed_ayu_dark();
        let commits = vec![
            commit("merge", vec!["base", "side"]),
            commit("side", vec!["root"]),
            commit("base", vec!["root"]),
            commit("root", Vec::new()),
        ];

        let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

        let merge_row = &graph[0];
        assert_eq!(merge_row.lanes_next.len(), 2);
        assert_eq!(merge_row.lanes_next[0].from_col, Some(0));
        assert_eq!(merge_row.lanes_next[1].from_col, None);
        assert!(merge_row.edges_out.is_empty());
    }

    #[test]
    fn linear_visible_history_keeps_single_lane_shape() {
        let theme = AppTheme::zed_ayu_dark();
        let commits = vec![
            commit("c2", vec!["c1"]),
            commit("c1", vec!["c0"]),
            commit("c0", Vec::new()),
        ];

        let graph = compute_graph(&commits, theme, std::iter::empty::<&str>(), None);

        assert_eq!(graph.len(), 3);
        assert_eq!(graph[0].lanes_now.len(), 1);
        assert!(!graph[0].lanes_now[0].incoming);
        assert_eq!(graph[0].lanes_next[0].from_col, Some(0));
        assert!(graph[1].lanes_now[0].incoming);
        assert_eq!(graph[1].lanes_next[0].from_col, Some(0));
        assert!(graph[2].lanes_now[0].incoming);
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
        let theme = AppTheme::zed_ayu_dark();
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
}
