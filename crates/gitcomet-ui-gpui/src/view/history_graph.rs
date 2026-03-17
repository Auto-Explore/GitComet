use crate::theme::AppTheme;
use gitcomet_core::domain::Commit;
use gpui::Rgba;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

const LANE_COLOR_PALETTE_SIZE: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct LaneId(pub u64);

#[derive(Clone, Copy, Debug)]
pub struct LanePaint {
    pub id: LaneId,
    pub color: Rgba,
}

#[derive(Clone, Copy, Debug)]
pub struct GraphEdge {
    pub from_col: usize,
    pub to_col: usize,
    pub color: Rgba,
}

#[derive(Clone, Debug)]
pub struct GraphRow {
    pub incoming_mask: Vec<bool>,
    pub lanes_now: Vec<LanePaint>,
    pub lanes_next: Vec<LanePaint>,
    pub next_from_cols: Vec<Option<usize>>,
    pub joins_in: Vec<GraphEdge>,
    pub edges_out: Vec<GraphEdge>,
    pub node_col: usize,
    pub is_merge: bool,
}

#[derive(Clone, Debug)]
struct LaneState<'a> {
    id: LaneId,
    color: Rgba,
    target: &'a str,
}

pub fn compute_graph(
    commits: &[Commit],
    theme: AppTheme,
    branch_heads: &HashSet<&str>,
) -> Vec<GraphRow> {
    let mut palette: Vec<Rgba> = Vec::with_capacity(LANE_COLOR_PALETTE_SIZE);
    for i in 0..LANE_COLOR_PALETTE_SIZE {
        let hue = (i as f32 * 0.13) % 1.0;
        let sat = 0.75;
        let light = if theme.is_dark { 0.62 } else { 0.45 };
        palette.push(gpui::hsla(hue, sat, light, 1.0).into());
    }

    let known: HashSet<&str> = commits.iter().map(|c| c.id.as_ref()).collect();
    let by_id: HashMap<&str, &Commit> = commits.iter().map(|c| (c.id.as_ref(), c)).collect();

    // Approximate the "main line" as the first-parent chain from the first commit in the list,
    // which is typically the checked-out branch HEAD in our log view.
    let mut head_chain: HashSet<&str> = HashSet::default();
    if let Some(first) = commits.first() {
        let mut cur: &str = first.id.as_ref();
        loop {
            if !head_chain.insert(cur) {
                break;
            }
            let Some(commit) = by_id.get(cur) else {
                break;
            };
            let Some(parent) = commit
                .parent_ids
                .first()
                .map(|p| p.as_ref())
                .filter(|p| known.contains(p))
            else {
                break;
            };
            cur = parent;
        }
    }

    let mut next_id: u64 = 1;
    let mut next_color: usize = 0;
    let mut lanes: Vec<LaneState<'_>> = Vec::new();
    let mut rows: Vec<GraphRow> = Vec::with_capacity(commits.len());
    let mut main_lane_id: Option<LaneId> = None;

    let mut pick_lane_color = |lanes: &[LaneState]| -> Rgba {
        let start = next_color;
        for offset in 0..palette.len() {
            let candidate = palette[(start + offset) % palette.len()];
            if lanes.iter().all(|l| l.color != candidate) {
                next_color = start + offset + 1;
                return candidate;
            }
        }
        let candidate = palette[start % palette.len()];
        next_color = start + 1;
        candidate
    };

    for commit in commits.iter() {
        let incoming_ids = lanes.iter().map(|l| l.id).collect::<HashSet<_>>();

        let mut hits = lanes
            .iter()
            .enumerate()
            .filter_map(|(ix, l)| (l.target == commit.id.as_ref()).then_some(ix))
            .collect::<Vec<_>>();
        let had_hit_lanes = !hits.is_empty();

        let is_merge = commit.parent_ids.len() > 1;
        let parent_ids = commit
            .parent_ids
            .iter()
            .map(|p| p.as_ref())
            .filter(|p| known.contains(p))
            .collect::<Vec<_>>();

        if hits.is_empty() {
            let id = LaneId(next_id);
            next_id += 1;
            let color = pick_lane_color(&lanes);
            lanes.push(LaneState {
                id,
                color,
                target: commit.id.as_ref(),
            });
            hits.push(lanes.len() - 1);
        }

        // If a branch head points at a commit that's already reached by another lane (i.e. the
        // branch is behind some other branch), split a new lane at this row so the head has its
        // own lane/color instead of inheriting the descendant lane's color.
        //
        // We currently only do this for non-merge commits to avoid interfering with merge-parent
        // lane assignment.
        let force_branch_head_lane = had_hit_lanes
            && hits.len() == 1
            && branch_heads.contains(commit.id.as_ref())
            && parent_ids.len() <= 1;

        let mut node_col = if let Some(main_lane_id) = main_lane_id
            && head_chain.contains(commit.id.as_ref())
        {
            hits.iter()
                .copied()
                .find(|ix| lanes.get(*ix).is_some_and(|lane| lane.id == main_lane_id))
                .or_else(|| hits.first().copied())
                .unwrap_or(0)
        } else {
            hits.first().copied().unwrap_or(0)
        };

        let mut swap_node_into_col: Option<usize> = None;
        if force_branch_head_lane {
            let id = LaneId(next_id);
            next_id += 1;
            let color = pick_lane_color(&lanes);
            swap_node_into_col = Some(node_col);
            node_col = lanes.len();
            lanes.push(LaneState {
                id,
                color,
                target: commit.id.as_ref(),
            });
            hits.push(node_col);
        }

        // Snapshot of lanes used for drawing this row (including any lanes that have converged
        // onto this commit before we re-target them to parents).
        let lanes_now = lanes
            .iter()
            .map(|l| LanePaint {
                id: l.id,
                color: l.color,
            })
            .collect::<Vec<_>>();

        let incoming_mask = lanes_now
            .iter()
            .map(|lane| incoming_ids.contains(&lane.id))
            .collect::<Vec<_>>();

        if let Some(pos) = hits.iter().position(|&ix| ix == node_col) {
            hits.swap(0, pos);
        }

        // Ensure the node lane is the first hit lane for the parent assignment logic below.
        node_col = hits.first().copied().unwrap_or(node_col);

        let node_id = lanes[node_col].id;
        if main_lane_id.is_none()
            || (force_branch_head_lane && head_chain.contains(commit.id.as_ref()))
        {
            main_lane_id = Some(node_id);
        }

        // Incoming join edges: other lanes that were targeting this commit join into the node.
        let mut joins_in = Vec::with_capacity(hits.len().saturating_sub(1));
        for &col in hits.iter().skip(1) {
            joins_in.push(GraphEdge {
                from_col: col,
                to_col: node_col,
                color: lanes[col].color,
            });
        }

        let mut covered_parents = 0usize;
        if parent_ids.is_empty() {
            // No parents: end all lanes converging here.
            for &hit_ix in &hits {
                lanes[hit_ix].target = commit.id.as_ref();
            }
        } else {
            lanes[node_col].target = parent_ids[0];
            covered_parents = 1;

            for (&hit_ix, parent) in hits.iter().skip(1).zip(parent_ids.iter().skip(1)) {
                lanes[hit_ix].target = *parent;
                covered_parents += 1;
            }

            // End hit lanes that converged here but don't have a parent to follow.
            for &hit_ix in hits.iter().skip(parent_ids.len().min(hits.len())) {
                lanes[hit_ix].target = commit.id.as_ref();
            }
        }

        // Create lanes for any remaining parents not covered by existing converged lanes.
        if parent_ids.len() > covered_parents {
            let mut insert_at = node_col + 1;
            for parent in parent_ids.iter().skip(covered_parents) {
                // If another lane already targets this parent, reuse it.
                if lanes.iter().any(|l| l.target == *parent) {
                    continue;
                }
                let id = LaneId(next_id);
                next_id += 1;
                let color = pick_lane_color(&lanes);
                lanes.insert(
                    insert_at,
                    LaneState {
                        id,
                        color,
                        target: parent,
                    },
                );
                insert_at += 1;
            }
        }

        if let Some(swap_col) = swap_node_into_col {
            lanes.swap(node_col, swap_col);
        }

        // Remove ended lanes: lanes whose target is not part of the visible graph, or whose target
        // is this commit without a parent to follow.
        lanes.retain(|l| known.contains(l.target) && l.target != commit.id.as_ref());

        let lanes_next = lanes
            .iter()
            .map(|l| LanePaint {
                id: l.id,
                color: l.color,
            })
            .collect::<Vec<_>>();

        let mut now_index_by_lane: HashMap<LaneId, usize> =
            HashMap::with_capacity_and_hasher(lanes_now.len(), Default::default());
        for (ix, lane) in lanes_now.iter().enumerate() {
            now_index_by_lane.insert(lane.id, ix);
        }

        let next_from_cols = lanes_next
            .iter()
            .map(|lane| now_index_by_lane.get(&lane.id).copied())
            .collect::<Vec<_>>();

        // Node->parent "merge" edges: connect the node into secondary-parent lanes.
        // - If the secondary parent lane existed already in this row, draw an explicit edge.
        // - If it was inserted this row, the continuation line already originates from the node.
        let mut edges_out = Vec::with_capacity(parent_ids.len().saturating_sub(1));
        let mut next_index_by_lane: HashMap<LaneId, usize> =
            HashMap::with_capacity_and_hasher(lanes_next.len(), Default::default());
        for (ix, lane) in lanes_next.iter().enumerate() {
            next_index_by_lane.insert(lane.id, ix);
        }
        for parent in parent_ids.iter().skip(1) {
            if let Some(lane) = lanes
                .iter()
                .find(|l| l.target == *parent && now_index_by_lane.contains_key(&l.id))
                && let Some(to_col) = next_index_by_lane.get(&lane.id).copied()
            {
                edges_out.push(GraphEdge {
                    from_col: node_col,
                    to_col,
                    color: lanes_next[to_col].color,
                });
            }
        }

        rows.push(GraphRow {
            incoming_mask,
            lanes_now,
            lanes_next,
            next_from_cols,
            joins_in,
            edges_out,
            node_col,
            is_merge,
        });
    }

    rows
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

        let branch_heads = HashSet::default();
        let graph = compute_graph(&commits, theme, &branch_heads);

        let head1_ix = LANE_COLOR_PALETTE_SIZE + 1 + (LANE_COLOR_PALETTE_SIZE - 1);
        let row = &graph[head1_ix];
        assert_eq!(row.lanes_now.len(), 2);

        let c0 = row.lanes_now[0].color;
        let c1 = row.lanes_now[1].color;
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

        let mut branch_heads = HashSet::default();
        branch_heads.insert("new1");
        branch_heads.insert("base");

        let graph = compute_graph(&commits, theme, &branch_heads);

        let base_row = &graph[1];
        assert_eq!(base_row.lanes_now.len(), 2);
        assert_eq!(base_row.incoming_mask, vec![true, false]);
        assert_eq!(base_row.joins_in.len(), 1);
        assert_eq!(base_row.node_col, 1);
        assert_ne!(base_row.lanes_now[0].color, base_row.lanes_now[1].color);

        assert_eq!(base_row.lanes_next.len(), 1);
        assert_eq!(
            base_row.lanes_next[0].id,
            base_row.lanes_now[base_row.node_col].id
        );
        assert_eq!(base_row.next_from_cols, vec![Some(1)]);
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

        let mut branch_heads = HashSet::default();
        branch_heads.insert("top1");
        branch_heads.insert("base");

        let graph = compute_graph(&commits, theme, &branch_heads);

        let base_row = &graph[2];
        assert_eq!(base_row.lanes_now.len(), 2);
        assert_eq!(base_row.joins_in.len(), 1);
        assert_eq!(base_row.node_col, 0);
        assert_eq!(base_row.lanes_next.len(), 1);
        assert_eq!(base_row.next_from_cols, vec![Some(0)]);
    }
}
