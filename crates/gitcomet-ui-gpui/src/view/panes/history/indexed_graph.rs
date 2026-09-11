use super::*;
use gitcomet_core::history_index::{HistoryIndexHandle, HistoryProjection, MISSING_PARENT};
use gitcomet_core::services::{CancellationToken, Result};

pub(super) const CHECKPOINT_STRIDE: usize = 1_024;

#[derive(Clone, Debug)]
struct Checkpoint {
    walk: history_graph::GraphWalk,
    labels: SmallVec<[Option<(u16, usize)>; 8]>,
}

pub(in crate::view) struct IndexedGraph {
    pub projection: HistoryProjection,
    checkpoints: Vec<Checkpoint>,
    branch_heads: FxHashSet<usize>,
    direct_labels: FxHashMap<usize, u16>,
    branch_names: Vec<SharedString>,
    containment: Vec<(u16, Vec<u64>)>,
    lane_spans: Vec<Vec<LaneSpan>>,
}

#[derive(Clone, Copy)]
struct LaneSpan {
    first: u32,
    last: u32,
    color: history_graph::LaneColorIx,
}

pub(in crate::view) struct GraphWindow {
    pub rows: Arc<[history_graph::GraphRow]>,
    pub labels: Vec<Option<SharedString>>,
}

impl IndexedGraph {
    pub fn build(
        index: HistoryIndexHandle,
        branches: &[Branch],
        remotes: &[RemoteBranch],
        stashes: &[StashEntry],
        head_branch: Option<&str>,
        head_id: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        let mut hidden = Vec::new();
        if stashes.is_empty() {
            for raw in 0..index.len() {
                if raw.is_multiple_of(CHECKPOINT_STRIDE) {
                    cancellation.check_cancelled()?;
                }
                if index.is_probable_stash(raw) {
                    hidden.extend(index.parents(raw).iter().skip(1).copied());
                }
            }
        } else {
            for stash in stashes {
                if let Some(raw) = index.position(stash.id.as_ref()) {
                    hidden.extend(index.parents(raw).iter().skip(1).copied());
                }
            }
        }
        let projection = HistoryProjection::new(index, hidden);
        let branch_heads = graph_branch_heads(projection.index.mode, branches, remotes)
            .filter_map(|id| projection.position(id))
            .collect();
        let (mut refs, head_refs) =
            build_history_branch_ref_items_by_target(branches, remotes, head_branch, head_id);
        if let (Some(head), Some(items)) = (head_id, head_refs) {
            refs.insert(head, items);
        }
        let tracked = branches
            .iter()
            .filter(|branch| branch.upstream.is_some())
            .map(|branch| branch.name.as_str())
            .collect();
        let mut names = Vec::new();
        let mut name_indices = FxHashMap::default();
        let mut direct_labels = FxHashMap::default();
        for (id, items) in refs {
            let Some(row) = projection.position(id) else {
                continue;
            };
            if let Some(name) = history_row_attribution_branch(&items, &tracked)
                && let Some(label) = intern_branch_name(&mut names, &mut name_indices, name)
            {
                direct_labels.insert(row, label);
            }
        }
        let mut containment = Vec::new();
        for (name, tip) in integration_branch_tips(branches, remotes) {
            let Some(raw) = projection.index.position(tip.as_ref()) else {
                continue;
            };
            let Some(label) = intern_branch_name(&mut names, &mut name_indices, &name) else {
                continue;
            };
            let mut bits = vec![0u64; projection.index.len().div_ceil(64)];
            let mut stack = vec![raw as u32];
            let mut visits = 0usize;
            while let Some(raw) = stack.pop() {
                if raw == MISSING_PARENT {
                    continue;
                }
                let raw = raw as usize;
                let mask = 1u64 << (raw % 64);
                if bits[raw / 64] & mask != 0 {
                    continue;
                }
                bits[raw / 64] |= mask;
                stack.extend_from_slice(projection.index.parents(raw));
                visits += 1;
                if visits.is_multiple_of(CHECKPOINT_STRIDE) {
                    cancellation.check_cancelled()?;
                }
            }
            containment.push((label, bits));
        }
        let main = head_id
            .and_then(|id| projection.position(id))
            .or_else(|| (!projection.is_empty()).then_some(0));
        let mut state = Checkpoint {
            walk: history_graph::GraphWalk::new(main),
            labels: SmallVec::new(),
        };
        let mut graph = Self {
            projection,
            checkpoints: Vec::new(),
            branch_heads,
            direct_labels,
            branch_names: names,
            containment,
            lane_spans: Vec::new(),
        };
        for row in 0..graph.projection.len() {
            if row.is_multiple_of(CHECKPOINT_STRIDE) {
                cancellation.check_cancelled()?;
                graph.checkpoints.push(state.clone());
            }
            let (paint, _) = graph.step(row, &mut state);
            graph
                .lane_spans
                .resize_with(graph.lane_spans.len().max(paint.lanes_now.len()), Vec::new);
            for (col, lane) in paint
                .lanes_now
                .iter()
                .enumerate()
                .filter(|(_, lane)| lane.is_active())
            {
                let spans = &mut graph.lane_spans[col];
                if let Some(last) = spans
                    .last_mut()
                    .filter(|last| last.color == lane.color_ix && last.last as usize + 1 == row)
                {
                    last.last = row as u32;
                } else {
                    spans.push(LaneSpan {
                        first: row as u32,
                        last: row as u32,
                        color: lane.color_ix,
                    });
                }
            }
        }
        cancellation.check_cancelled()?;
        Ok(graph)
    }

    fn step(&self, row: usize, state: &mut Checkpoint) -> (history_graph::GraphRow, Option<u16>) {
        let parents: SmallVec<[usize; 4]> = self
            .projection
            .parents(row)
            .filter(|&parent| parent > row)
            .collect();
        let raw = self.projection.raw_position(row).expect("graph row");
        let paint = state.walk.step(
            row,
            &parents,
            self.projection.index.parents(raw).len() > 1,
            self.branch_heads.contains(&row),
        );
        let node = usize::from(paint.node_col);
        let mut resolved = state.labels.get(node).copied().flatten();
        for edge in &paint.joins_in {
            if let Some(candidate) = state
                .labels
                .get(usize::from(edge.from_col))
                .copied()
                .flatten()
                && resolved.is_none_or(|(_, seeded)| candidate.1 > seeded)
            {
                resolved = Some(candidate);
            }
        }
        let contained = self
            .containment
            .iter()
            .find(|(_, bits)| bits[raw / 64] & (1u64 << (raw % 64)) != 0)
            .map(|(label, _)| *label);
        if let Some(label) = contained.or_else(|| self.direct_labels.get(&row).copied()) {
            resolved = Some((label, row));
        }
        if state.labels.len() <= node {
            state.labels.resize(node + 1, None);
        }
        state.labels[node] = resolved;
        if state.labels.len() < paint.lanes_next.len() {
            state.labels.resize(paint.lanes_next.len(), None);
        }
        for (col, lane) in paint.lanes_next.iter().enumerate() {
            if !lane.is_active() {
                state.labels[col] = None;
            } else if lane.starts_at_node() {
                state.labels[col] = resolved;
            }
        }
        // Discard ended trailing lanes as well as their attribution.
        state.labels.truncate(paint.lanes_next.len());
        (paint, resolved.map(|(label, _)| label))
    }

    pub fn selected_lane(
        &self,
        anchor: usize,
        on_branch: Option<bool>,
        window_start: usize,
        cancellation: &CancellationToken,
    ) -> Result<Option<crate::view::rows::history_graph_paint::SelectedLane>> {
        use crate::view::rows::history_graph_paint::{self as paint, SelectedLane};
        let window = self.window(anchor..anchor + 1, cancellation)?;
        let Some(row) = window.rows.first() else {
            return Ok(None);
        };
        let color = on_branch.map_or(row.node_color_ix, |on_branch| {
            paint::band_node_for(row, on_branch).color_ix
        });
        let Some(col) = paint::lane_col_for_color(row, color) else {
            return Ok(None);
        };
        let start = if row
            .lanes_now
            .get(col)
            .is_some_and(|lane| lane.is_active() && lane.color_ix == color)
        {
            anchor
        } else {
            anchor + 1
        };
        let span = self.lane_spans.get(col).and_then(|spans| {
            let ix = spans.partition_point(|span| (span.last as usize) < start);
            spans
                .get(ix)
                .filter(|span| span.first as usize <= start && span.color == color)
        });
        let (first, last) = span.map_or((anchor, anchor), |span| {
            (span.first as usize, span.last as usize)
        });
        // Keep an off-screen selection active so unrelated lanes still wash out.
        // A span wholly above this window cannot cover a local row.
        Ok(Some(if last < window_start {
            SelectedLane::span(color, usize::MAX - 1, usize::MAX - 1)
        } else {
            SelectedLane::span(
                color,
                first.saturating_sub(window_start),
                last - window_start,
            )
        }))
    }

    pub fn window(
        &self,
        range: std::ops::Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<GraphWindow> {
        let end = range.end.min(self.projection.len());
        let mut rows = Vec::with_capacity(end.saturating_sub(range.start));
        let mut labels = Vec::with_capacity(rows.capacity());
        if range.start < end {
            let checkpoint = range.start / CHECKPOINT_STRIDE;
            let mut state = self.checkpoints[checkpoint].clone();
            for row in checkpoint * CHECKPOINT_STRIDE..end {
                if row.is_multiple_of(128) {
                    cancellation.check_cancelled()?;
                }
                let (paint, label) = self.step(row, &mut state);
                if row >= range.start {
                    rows.push(paint);
                    labels.push(
                        label
                            .and_then(|label| self.branch_names.get(usize::from(label)))
                            .cloned(),
                    );
                }
            }
        }
        Ok(GraphWindow {
            rows: rows.into(),
            labels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;

    fn raw(row: usize) -> [u8; 20] {
        let mut raw = [0u8; 20];
        raw[..8].copy_from_slice(&(row as u64 + 1).to_be_bytes());
        raw
    }

    #[test]
    fn checkpoint_windows_match_continuous_graph_and_attribution() {
        for linear in [true, false] {
            let count = 4200;
            let mut builder = HistoryIndexBuilder::new(
                HistorySnapshot("graph".into()),
                LogScope::AllBranches,
                20,
            )
            .unwrap();
            let mut commits = Vec::new();
            for row in 0..count {
                let mut parents = Vec::new();
                if row + 1 < count {
                    parents.push(raw(row + 1));
                }
                if !linear && row.is_multiple_of(37) && row + 83 < count {
                    parents.push(raw(row + 83));
                }
                builder
                    .push(
                        &raw(row),
                        parents.iter().map(|id| id.as_slice()),
                        (count - row) as i64,
                        false,
                    )
                    .unwrap();
                commits.push(Commit {
                    id: CommitId(gitcomet_core::hex::encode(&raw(row)).into()),
                    parent_ids: parents
                        .iter()
                        .map(|id| CommitId(gitcomet_core::hex::encode(id).into()))
                        .collect(),
                    summary: Arc::from("commit"),
                    author: Arc::from("author"),
                    time: std::time::UNIX_EPOCH,
                });
            }
            let index = builder.finish(&CancellationToken::new()).unwrap();
            let branches = vec![
                Branch {
                    name: "main".into(),
                    target: commits[0].id.clone(),
                    upstream: None,
                    divergence: None,
                },
                Branch {
                    name: "feature".into(),
                    target: commits[35].id.clone(),
                    upstream: None,
                    divergence: None,
                },
            ];
            let graph = IndexedGraph::build(
                index,
                &branches,
                &[],
                &[],
                Some("main"),
                Some(commits[0].id.as_ref()),
                &CancellationToken::new(),
            )
            .unwrap();
            let request = HistoryBaseCacheRequest {
                repo_id: RepoId(1),
                history_scope: LogScope::AllBranches,
                log_source: 0,
                history_author_filter: None,
                head_branch_rev: 0,
                detached_head_commit: None,
                head_branch_target: Some(commits[0].id.clone()),
                branches_rev: 0,
                remote_branches_rev: 0,
                stashes_rev: 0,
            };
            let page = LogPage {
                commits,
                next_cursor: None,
            };
            let theme = AppTheme::gitcomet_dark();
            let base = build_history_base_cache(
                request.clone(),
                &page,
                theme,
                Some("main"),
                &branches,
                &[],
                &[],
            );
            let decorations = build_history_decoration_cache(
                HistoryDecorationCacheRequest {
                    base_request: request,
                    head_branch_rev: 0,
                    detached_head_commit: None,
                    branches_rev: 0,
                    remote_branches_rev: 0,
                    tags_rev: 0,
                },
                &page,
                &base,
                Some("main"),
                &branches,
                &[],
                &[],
            );
            for start in [0, 900, 1023, 1024, 2047, 3071, 4100] {
                let end = (start + 150).min(count);
                let window = graph.window(start..end, &CancellationToken::new()).unwrap();
                assert_eq!(
                    window.rows.as_ref(),
                    &base.graph_rows[start..end],
                    "linear={linear} start={start}"
                );
                for anchor in [0, 35, 1025, 3700] {
                    use crate::view::rows::history_graph_paint as paint;
                    for branch in [None, Some(false), Some(true)] {
                        let color =
                            branch.map_or(base.graph_rows[anchor].node_color_ix, |on_branch| {
                                paint::band_node_for(&base.graph_rows[anchor], on_branch).color_ix
                            });
                        let expected = paint::selected_lane_at(&base.graph_rows, anchor, color);
                        let actual = graph
                            .selected_lane(anchor, branch, start, &CancellationToken::new())
                            .unwrap();
                        for row in 0..window.rows.len() {
                            for color in 0..8 {
                                assert_eq!(
                                    actual.map(|lane| lane.covers(theme, row, color)),
                                    expected.map(|lane| lane.covers(theme, start + row, color)),
                                    "anchor={anchor} window={start} branch={branch:?}"
                                );
                            }
                        }
                    }
                }
                for (ix, label) in window.labels.iter().enumerate() {
                    let expected = decorations.row_vms[start + ix]
                        .lane_branch
                        .and_then(|ix| decorations.branch_names.get(ix as usize));
                    assert_eq!(
                        label.as_ref(),
                        expected,
                        "linear={linear} row={}",
                        start + ix
                    );
                }
            }
            assert_eq!(graph.checkpoints.len(), count.div_ceil(CHECKPOINT_STRIDE));
        }
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    #[test]
    #[ignore = "two million row index/graph benchmark"]
    fn indexed_history_two_million_graph_window_benchmark() {
        use gitcomet_core::history_index::HistoryIndexBuilder;
        use gitcomet_core::services::HistorySnapshot;
        use std::time::Instant;
        let raw = |row: usize| {
            let mut id = [0u8; 20];
            id[..8].copy_from_slice(&(row as u64).to_be_bytes());
            id
        };
        let count: usize = 2_000_000;
        let cancellation = CancellationToken::new();
        let mut builder =
            HistoryIndexBuilder::new(HistorySnapshot("bench".into()), LogScope::AllBranches, 20)
                .unwrap();
        for row in 0..count {
            let parents: SmallVec<[[u8; 20]; 2]> = [row + 1, row + 97]
                .into_iter()
                .filter(|&parent| parent < count && (parent == row + 1 || row.is_multiple_of(101)))
                .map(raw)
                .collect();
            builder
                .push(
                    &raw(row),
                    parents.iter().map(|parent| parent.as_slice()),
                    0,
                    false,
                )
                .unwrap();
        }
        let index = builder.finish(&cancellation).unwrap();
        let started = Instant::now();
        let graph = IndexedGraph::build(index, &[], &[], &[], None, None, &cancellation).unwrap();
        eprintln!(
            "graph rows={count} seconds={:.3} checkpoints={}",
            started.elapsed().as_secs_f64(),
            graph.checkpoints.len()
        );
        let mut samples = Vec::new();
        for ix in 0..100 {
            let start = (ix * 97171) % (count - 256);
            let started = Instant::now();
            let window = graph.window(start..start + 256, &cancellation).unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(window.rows.len(), 256);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "graph 256-row replay p50_ms={:.3} p95_ms={:.3}",
            samples[50], samples[95]
        );
        assert_eq!(graph.checkpoints.len(), count.div_ceil(CHECKPOINT_STRIDE));
    }
}
