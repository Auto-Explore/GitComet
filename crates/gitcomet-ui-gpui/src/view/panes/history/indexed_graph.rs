use super::*;
use gitcomet_core::history_index::{HistoryIndexHandle, HistoryProjection, MISSING_PARENT};
use gitcomet_core::services::{CancellationToken, Result};

pub(super) const CHECKPOINT_STRIDE: usize = 1_024;

#[derive(Clone, Debug)]
struct Checkpoint {
    walk: Arc<history_graph::GraphCheckpoint>,
    labels: Vec<SavedLabel>,
}

#[derive(Clone, Debug)]
struct SavedLabel {
    column: u16,
    label: u16,
    seeded: u32,
}

impl Checkpoint {
    fn restore(&self) -> WalkState {
        let mut labels = SmallVec::new();
        for saved in &self.labels {
            labels.resize(usize::from(saved.column) + 1, None);
            labels[usize::from(saved.column)] = Some((saved.label, saved.seeded));
        }
        WalkState {
            walk: self.walk.restore(),
            labels,
        }
    }
}

struct WalkState {
    walk: history_graph::GraphWalk,
    labels: SmallVec<[Option<(u16, u32)>; 8]>,
}

type GeometryWindow = (std::ops::Range<usize>, Arc<[history_graph::GraphRow]>);
type SelectedSpan = (
    (usize, Option<bool>),
    Option<crate::view::rows::history_graph_paint::SelectedLane>,
);

/// Rows reachable from one integration branch tip. Reachability depends only
/// on the topology and the tip row, so rebuilds of the same index share it.
#[derive(Clone)]
struct Containment {
    label: u16,
    tip: u32,
    bits: Arc<[u64]>,
}

pub(in crate::view) struct IndexedGraph {
    pub projection: HistoryProjection,
    checkpoints: Vec<Checkpoint>,
    branch_heads: FxHashSet<usize>,
    main: Option<usize>,
    geometry_cache: Arc<std::sync::Mutex<Option<GeometryWindow>>>,
    direct_labels: FxHashMap<usize, u16>,
    branch_names: Vec<SharedString>,
    containment: Vec<Containment>,
    lane_spans: Arc<Vec<Vec<LaneSpan>>>,
    window_cache: std::sync::Mutex<Option<(std::ops::Range<usize>, GraphWindow)>>,
    selection_cache: Arc<std::sync::Mutex<Option<SelectedSpan>>>,
}

#[derive(Clone, Copy)]
struct LaneSpan {
    first: u32,
    last: u32,
    color: history_graph::LaneColorIx,
}

#[derive(Clone)]
pub(in crate::view) struct GraphWindow {
    pub rows: Arc<[history_graph::GraphRow]>,
    pub labels: Arc<[Option<SharedString>]>,
}

impl IndexedGraph {
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints.capacity() * std::mem::size_of::<Checkpoint>()
            + self
                .checkpoints
                .iter()
                .map(|checkpoint| {
                    checkpoint.walk.estimated_bytes()
                        + checkpoint.labels.capacity() * std::mem::size_of::<SavedLabel>()
                })
                .sum::<usize>()
    }

    /// Unused elements retained by the checkpoints, as element counts.
    #[cfg(test)]
    pub fn checkpoint_capacity_slack(&self) -> usize {
        self.checkpoints.capacity() - self.checkpoints.len()
            + self
                .checkpoints
                .iter()
                .map(|checkpoint| {
                    checkpoint.walk.lane_capacity_slack() + checkpoint.labels.capacity()
                        - checkpoint.labels.len()
                })
                .sum::<usize>()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn build(
        index: HistoryIndexHandle,
        branches: &[Branch],
        remotes: &[RemoteBranch],
        stashes: &[StashEntry],
        head_branch: Option<&str>,
        head_id: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        Self::build_reusing(
            index,
            branches,
            remotes,
            stashes,
            head_branch,
            head_id,
            cancellation,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_reusing(
        index: HistoryIndexHandle,
        branches: &[Branch],
        remotes: &[RemoteBranch],
        stashes: &[StashEntry],
        head_branch: Option<&str>,
        head_id: Option<&str>,
        cancellation: &CancellationToken,
        previous: Option<&Self>,
    ) -> Result<Self> {
        let mut hidden = Vec::new();
        if stashes.is_empty() {
            for &raw in index.probable_stash_rows() {
                cancellation.check_cancelled()?;
                hidden.extend(index.parents(raw as usize).iter().skip(1).copied());
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
            let reused = previous
                .filter(|old| Arc::ptr_eq(&old.projection.index, &projection.index))
                .and_then(|old| old.containment.iter().find(|old| old.tip == raw as u32))
                .map(|old| old.bits.clone());
            let bits = match reused {
                Some(bits) => bits,
                None => {
                    gitcomet_core::history_perf::record(
                        gitcomet_core::history_perf::Work::ContainmentWalk,
                    );
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
                    bits.into()
                }
            };
            containment.push(Containment {
                label,
                tip: raw as u32,
                bits,
            });
        }
        let main = head_id
            .and_then(|id| projection.position(id))
            .or_else(|| (!projection.is_empty()).then_some(0));
        // Ref names/decorations can change without changing lane geometry. A new
        // head position (even at an existing commit) must invalidate geometry.
        let reusable = previous.filter(|old| {
            Arc::ptr_eq(&old.projection.index, &projection.index)
                && old.projection == projection
                && old.branch_heads == branch_heads
                && old.main == main
        });
        // Attribution is a function of the geometry inputs plus these three, so
        // when they all match the previous checkpoints already carry the labels
        // and the row replay can be skipped outright.
        let labels_reusable = reusable.filter(|old| {
            old.direct_labels == direct_labels
                && old.branch_names == names
                && old.containment.len() == containment.len()
                && old
                    .containment
                    .iter()
                    .zip(&containment)
                    .all(|(old, new)| old.label == new.label && old.tip == new.tip)
        });
        let mut state = WalkState {
            walk: history_graph::GraphWalk::new(main),
            labels: SmallVec::new(),
        };
        let mut graph = Self {
            projection,
            checkpoints: Vec::new(),
            branch_heads,
            main,
            geometry_cache: reusable
                .map(|old| old.geometry_cache.clone())
                .unwrap_or_default(),
            direct_labels,
            branch_names: names,
            containment,
            lane_spans: reusable
                .map(|old| old.lane_spans.clone())
                .unwrap_or_default(),
            window_cache: Default::default(),
            selection_cache: reusable
                .map(|old| old.selection_cache.clone())
                .unwrap_or_default(),
        };
        if reusable.is_none() && main.is_some() && !graph.projection.is_empty() {
            Arc::get_mut(&mut graph.lane_spans)
                .unwrap()
                .push(vec![LaneSpan {
                    first: 0,
                    last: graph.projection.len() as u32 - 1,
                    color: 0,
                }]);
        }
        if let Some(old) = labels_reusable {
            graph.checkpoints = old.checkpoints.clone();
            cancellation.check_cancelled()?;
            return Ok(graph);
        }
        graph
            .checkpoints
            .reserve_exact(graph.projection.len().div_ceil(CHECKPOINT_STRIDE));
        for row in 0..graph.projection.len() {
            if row.is_multiple_of(CHECKPOINT_STRIDE) {
                cancellation.check_cancelled()?;
                let mut labels = Vec::with_capacity(state.labels.iter().flatten().count());
                labels.extend(state.labels.iter().enumerate().filter_map(|(col, label)| {
                    label.map(|(label, seeded)| SavedLabel {
                        column: col as u16,
                        label,
                        seeded,
                    })
                }));
                graph.checkpoints.push(Checkpoint {
                    walk: reusable
                        .map(|old| old.checkpoints[row / CHECKPOINT_STRIDE].walk.clone())
                        .unwrap_or_else(|| Arc::new(state.walk.checkpoint())),
                    labels,
                });
            }
            let (transition, _) = graph.step(row, &mut state, false);
            if reusable.is_some() {
                continue;
            }
            for (col, lane) in transition.now {
                graph.change_span(col, row, lane);
            }
            for (col, lane) in transition.next {
                graph.change_span(col, row + 1, lane);
            }
        }
        cancellation.check_cancelled()?;
        Ok(graph)
    }

    fn change_span(&mut self, col: usize, row: usize, lane: history_graph::LanePaint) {
        let columns = Arc::get_mut(&mut self.lane_spans).expect("new geometry");
        columns.resize_with(columns.len().max(col + 1), Vec::new);
        let spans = &mut columns[col];
        if let Some(last) = spans.last_mut().filter(|span| span.last as usize >= row) {
            if lane.is_active() && last.color == lane.color_ix {
                return;
            }
            if last.first as usize == row {
                spans.pop();
            } else {
                last.last = row as u32 - 1;
            }
        }
        if lane.is_active() && row < self.projection.len() {
            spans.push(LaneSpan {
                first: row as u32,
                last: self.projection.len() as u32 - 1,
                color: lane.color_ix,
            });
        }
    }

    fn step(
        &self,
        row: usize,
        state: &mut WalkState,
        materialize: bool,
    ) -> (history_graph::GraphTransition, Option<u16>) {
        // One raw lookup per row; the parents map through the projection directly.
        let raw = self.projection.raw_position(row).expect("graph row");
        let raw_parents = self.projection.index.parents(raw);
        let parents: SmallVec<[usize; 4]> = raw_parents
            .iter()
            .filter_map(|&parent| self.projection.visible_position(parent as usize))
            .filter(|&parent| parent > row)
            .collect();
        let transition = state.walk.transition(
            row,
            &parents,
            raw_parents.len() > 1,
            self.branch_heads.contains(&row),
            materialize,
        );
        let paint = &transition.paint;
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
            .find(|set| set.bits[raw / 64] & (1u64 << (raw % 64)) != 0)
            .map(|set| set.label);
        if let Some(label) = contained.or_else(|| self.direct_labels.get(&row).copied()) {
            resolved = Some((label, row as u32));
        }
        if state.labels.len() <= node {
            state.labels.resize(node + 1, None);
        }
        state.labels[node] = resolved;
        if state.labels.len() < transition.next_len {
            state.labels.resize(transition.next_len, None);
        }
        for &(col, lane) in &transition.next {
            if col >= state.labels.len() {
                continue;
            }
            if !lane.is_active() {
                state.labels[col] = None;
            } else if lane.starts_at_node() {
                state.labels[col] = resolved;
            }
        }
        state.labels.truncate(transition.next_len);
        (transition, resolved.map(|(label, _)| label))
    }

    pub fn selected_lane(
        &self,
        anchor: usize,
        on_branch: Option<bool>,
        window_start: usize,
        cancellation: &CancellationToken,
    ) -> Result<Option<crate::view::rows::history_graph_paint::SelectedLane>> {
        let key = (anchor, on_branch);
        let cached = *self.selection_cache.lock().unwrap();
        let absolute =
            if let Some((cached_key, lane)) = cached.filter(|(cached_key, _)| *cached_key == key) {
                let _ = cached_key;
                lane
            } else {
                let lane = self.selected_lane_absolute(anchor, on_branch, cancellation)?;
                *self.selection_cache.lock().unwrap() = Some((key, lane));
                lane
            };
        Ok(absolute.map(|lane| lane.relative_to(window_start)))
    }

    fn selected_lane_absolute(
        &self,
        anchor: usize,
        on_branch: Option<bool>,
        cancellation: &CancellationToken,
    ) -> Result<Option<crate::view::rows::history_graph_paint::SelectedLane>> {
        use crate::view::rows::history_graph_paint::{self as paint, SelectedLane};
        let saved_geometry = self.geometry_cache.lock().unwrap().take();
        let saved = self.window_cache.lock().unwrap().take();
        let window = self.window(anchor..anchor + 1, cancellation)?;
        *self.window_cache.lock().unwrap() = saved;
        *self.geometry_cache.lock().unwrap() = saved_geometry;
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
        Ok(Some(SelectedLane::span(color, first, last)))
    }

    pub fn window(
        &self,
        range: std::ops::Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<GraphWindow> {
        cancellation.check_cancelled()?;
        if let Some((_, window)) = self
            .window_cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(key, _)| *key == range)
        {
            return Ok(window.clone());
        }
        let geometry = self
            .geometry_cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(key, _)| *key == range)
            .map(|(_, rows)| rows.clone());
        let end = range.end.min(self.projection.len());
        let mut rows = Vec::with_capacity(end.saturating_sub(range.start));
        let mut labels = Vec::with_capacity(rows.capacity());
        if range.start < end {
            let checkpoint = range.start / CHECKPOINT_STRIDE;
            let saved = &self.checkpoints[checkpoint];
            let mut state = saved.restore();
            for row in checkpoint * CHECKPOINT_STRIDE..end {
                if row.is_multiple_of(128) {
                    cancellation.check_cancelled()?;
                }
                let (transition, label) =
                    self.step(row, &mut state, geometry.is_none() && row >= range.start);
                if row >= range.start {
                    if geometry.is_none() {
                        rows.push(transition.paint);
                    }
                    labels.push(
                        label
                            .and_then(|label| self.branch_names.get(usize::from(label)))
                            .cloned(),
                    );
                }
            }
        }
        let rows = geometry.unwrap_or_else(|| rows.into());
        *self.geometry_cache.lock().unwrap() = Some((range.clone(), rows.clone()));
        let window = GraphWindow {
            rows,
            labels: labels.into(),
        };
        *self.window_cache.lock().unwrap() = Some((range, window.clone()));
        Ok(window)
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
        for (linear, width) in [(true, 1), (false, 83), (false, 512)] {
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
                if row + width < count {
                    parents.push(raw(row + width));
                }
                if !linear && row.is_multiple_of(37) && row + 1 < count {
                    parents.push(raw(row + 1));
                }
                builder
                    .push(&raw(row), parents.iter().map(|id| id.as_slice()), false)
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

/// Benchmark entry points use the same index, checkpoints and windows as the UI.
#[cfg(any(test, feature = "benchmarks"))]
pub struct IndexedHistoryFixture {
    count: usize,
    width: usize,
    hash_len: usize,
    index_peak_bytes: usize,
    graph: IndexedGraph,
}

#[cfg(any(test, feature = "benchmarks"))]
impl IndexedHistoryFixture {
    pub fn new(count: usize, width: usize, hash_len: usize) -> Self {
        let (index, index_peak_bytes) = Self::index(count, width, hash_len);
        let graph =
            IndexedGraph::build(index, &[], &[], &[], None, None, &CancellationToken::new())
                .unwrap();
        Self {
            count,
            width,
            hash_len,
            index_peak_bytes,
            graph,
        }
    }
    fn id(row: usize) -> [u8; 32] {
        let mut id = [0; 32];
        let mut seed = row as u64;
        for chunk in id.as_chunks_mut::<8>().0 {
            seed = seed.wrapping_add(0x9e3779b97f4a7c15);
            let mut value = seed;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
            chunk.copy_from_slice(&(value ^ (value >> 31)).to_be_bytes());
        }
        id
    }
    fn index(count: usize, width: usize, hash_len: usize) -> (HistoryIndexHandle, usize) {
        use gitcomet_core::{history_index::HistoryIndexBuilder, services::HistorySnapshot};
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("indexed-benchmark".into()),
            LogScope::AllBranches,
            hash_len,
        )
        .unwrap();
        for row in 0..count {
            let parents: SmallVec<[[u8; 32]; 2]> = [row + width.max(1), row + 1]
                .into_iter()
                .enumerate()
                .filter(|&(ix, parent)| parent < count && (ix == 0 || (width > 1 && row % 37 == 0)))
                .map(|(_, row)| Self::id(row))
                .collect();
            builder
                .push(
                    &Self::id(row)[..hash_len],
                    parents.iter().map(|id| &id[..hash_len]),
                    false,
                )
                .unwrap();
        }
        let peak = builder.estimated_peak_bytes();
        (builder.finish(&CancellationToken::new()).unwrap(), peak)
    }
    pub fn build_index(&self) -> usize {
        Self::index(self.count, self.width, self.hash_len)
            .0
            .estimated_bytes()
    }
    pub fn build_graph(&self) -> usize {
        let graph = IndexedGraph::build(
            self.graph.projection.index.clone(),
            &[],
            &[],
            &[],
            None,
            None,
            &CancellationToken::new(),
        )
        .unwrap();
        graph.checkpoint_bytes()
    }
    pub fn window(&self, start: usize, rows: usize, selection: Option<usize>) -> usize {
        let start = start.min(self.count.saturating_sub(rows));
        let window = self
            .graph
            .window(
                start..(start + rows).min(self.count),
                &CancellationToken::new(),
            )
            .unwrap();
        if let Some(anchor) = selection {
            std::hint::black_box(
                self.graph
                    .selected_lane(anchor, None, start, &CancellationToken::new())
                    .unwrap(),
            );
        }
        std::hint::black_box(&window);
        window.rows.len()
    }
    pub fn retained_bytes(&self) -> usize {
        self.graph.projection.index.estimated_bytes()
    }
    pub fn estimated_index_peak_bytes(&self) -> usize {
        self.index_peak_bytes
    }
    pub fn checkpoint_bytes(&self) -> usize {
        self.graph.checkpoint_bytes()
    }
    pub fn clear_window(&self) {
        *self.graph.window_cache.lock().unwrap() = None;
        *self.graph.geometry_cache.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod performance_regressions {
    use super::*;
    use gitcomet_core::history_perf::{self, Work};

    #[test]
    fn indexed_history_skipped_rows_and_warm_windows_materialize_no_paint() {
        for width in [1, 64, 512, 5261] {
            let fixture = IndexedHistoryFixture::new(20_000, width, 20);
            assert_eq!(fixture.build_index(), fixture.retained_bytes());
            assert!(fixture.estimated_index_peak_bytes() >= fixture.retained_bytes());
            {
                let _capture = history_perf::capture();
                fixture.build_graph();
                assert_eq!(history_perf::count(Work::PaintRow), 0);
                assert_eq!(history_perf::count(Work::GraphTransition), 20_000);
            }
            let _capture = history_perf::capture();
            fixture.window(10_239, 40, None);
            assert_eq!(history_perf::count(Work::PaintRow), 40);
            let transitions = history_perf::count(Work::GraphTransition);
            assert_eq!(transitions, 1023 + 40);
            fixture.window(10_239, 40, None);
            assert_eq!(history_perf::count(Work::GraphTransition), transitions);
            fixture.window(10_239, 40, Some(100));
            let transitions = history_perf::count(Work::GraphTransition);
            fixture.window(10_239, 40, Some(100));
            assert_eq!(history_perf::count(Work::GraphTransition), transitions);
            assert!(fixture.checkpoint_bytes() > 0);
        }
    }

    #[test]
    #[ignore = "production index/graph phase and window latency benchmark"]
    fn indexed_history_wide_graph_phase_benchmark() {
        use std::time::Instant;
        for count in [100_000, 2_000_000] {
            for width in [1, 64, 512, 5261] {
                let started = Instant::now();
                let fixture = IndexedHistoryFixture::new(count, width, 20);
                let construction = started.elapsed();
                let mut cold = Vec::new();
                let mut warm = Vec::new();
                for sample in 0..100 {
                    let start = (sample * 97171) % (count - 40);
                    fixture.clear_window();
                    let started = Instant::now();
                    fixture.window(start, 40, Some(count / 3));
                    cold.push(started.elapsed().as_secs_f64() * 1000.0);
                    let started = Instant::now();
                    fixture.window(start, 40, Some(count / 3));
                    warm.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                cold.sort_by(f64::total_cmp);
                warm.sort_by(f64::total_cmp);
                eprintln!(
                    "rows={count} width={width} construct_s={:.3} retained_bytes={} checkpoint_bytes={} first_touch_ms_p50_p95_p99={:?} warm_ms_p50_p95_p99={:?}",
                    construction.as_secs_f64(),
                    fixture.retained_bytes(),
                    fixture.checkpoint_bytes(),
                    [cold[50], cold[95], cold[99]],
                    [warm[50], warm[95], warm[99]]
                );
            }
        }
    }
}

#[cfg(test)]
mod reuse_regressions {
    use super::*;
    use gitcomet_core::domain::UpstreamDivergence;
    use gitcomet_core::history_perf::{self, Work};

    fn branch(name: &str, target: CommitId) -> Branch {
        Branch {
            name: name.into(),
            target,
            upstream: None,
            divergence: None,
        }
    }

    #[test]
    fn checkpoints_carry_no_capacity_slack_at_any_width() {
        for width in [1usize, 64, 512, 5261] {
            let fixture = IndexedHistoryFixture::new(20_000, width, 20);
            assert_eq!(fixture.graph.checkpoints.len(), 20);
            assert_eq!(
                fixture.graph.checkpoint_capacity_slack(),
                0,
                "width={width}"
            );
        }
    }

    #[test]
    fn integration_containment_and_labels_are_reused_when_their_inputs_hold_still() {
        let fixture = IndexedHistoryFixture::new(2000, 64, 20);
        let index = fixture.graph.projection.index.clone();
        let cancel = CancellationToken::new();
        let head = index.commit_id(0).unwrap();
        let mut branches = vec![
            branch("main", head.clone()),
            branch("feature", index.commit_id(65).unwrap()),
        ];
        let _capture = history_perf::capture();
        let first = IndexedGraph::build(
            index.clone(),
            &branches,
            &[],
            &[],
            Some("main"),
            Some(head.as_ref()),
            &cancel,
        )
        .unwrap();
        assert_eq!(history_perf::count(Work::ContainmentWalk), 1);
        assert_eq!(history_perf::count(Work::GraphTransition), 2000);
        let window = first.window(0..40, &cancel).unwrap();
        assert_eq!(window.labels[0].as_deref(), Some("main"));

        // Divergence moves after every fetch without touching attribution.
        branches[1].divergence = Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        });
        let before = history_perf::count(Work::GraphTransition);
        let same = IndexedGraph::build_reusing(
            index.clone(),
            &branches,
            &[],
            &[],
            Some("main"),
            Some(head.as_ref()),
            &cancel,
            Some(&first),
        )
        .unwrap();
        assert_eq!(
            history_perf::count(Work::ContainmentWalk),
            1,
            "reachability reused"
        );
        assert_eq!(
            history_perf::count(Work::GraphTransition),
            before,
            "labels reused without replaying the rows"
        );
        assert!(Arc::ptr_eq(
            &first.containment[0].bits,
            &same.containment[0].bits
        ));
        let reused = same.window(0..40, &cancel).unwrap();
        assert_eq!(reused.labels, window.labels);
        assert!(Arc::ptr_eq(&window.rows, &reused.rows));

        // A rename changes attribution but not reachability: replay, keep the walk.
        branches[1].name = "renamed".into();
        let before = history_perf::count(Work::GraphTransition);
        let renamed = IndexedGraph::build_reusing(
            index.clone(),
            &branches,
            &[],
            &[],
            Some("main"),
            Some(head.as_ref()),
            &cancel,
            Some(&same),
        )
        .unwrap();
        assert_eq!(history_perf::count(Work::ContainmentWalk), 1);
        assert_eq!(history_perf::count(Work::GraphTransition), before + 2000);
        assert!(Arc::ptr_eq(
            &same.containment[0].bits,
            &renamed.containment[0].bits
        ));
        assert_eq!(
            renamed.window(0..40, &cancel).unwrap().labels,
            window.labels
        );

        // Moving the integration tip invalidates its reachability.
        branches[0].target = index.commit_id(1).unwrap();
        let moved = IndexedGraph::build_reusing(
            index,
            &branches,
            &[],
            &[],
            Some("main"),
            Some(branches[0].target.as_ref()),
            &cancel,
            Some(&renamed),
        )
        .unwrap();
        assert_eq!(history_perf::count(Work::ContainmentWalk), 2);
        assert!(!Arc::ptr_eq(
            &renamed.containment[0].bits,
            &moved.containment[0].bits
        ));
        assert_eq!(moved.containment[0].tip, 1);
    }

    #[test]
    #[ignore = "reachability walk cost per integration tip at two million rows"]
    fn integration_containment_walk_timing() {
        use std::time::Instant;
        let fixture = IndexedHistoryFixture::new(2_000_000, 64, 20);
        let index = fixture.graph.projection.index.clone();
        let head = index.commit_id(0).unwrap();
        let mut branches = vec![branch("main", head.clone())];
        let cancel = CancellationToken::new();
        let started = Instant::now();
        let first = IndexedGraph::build(
            index.clone(),
            &branches,
            &[],
            &[],
            Some("main"),
            Some(head.as_ref()),
            &cancel,
        )
        .unwrap();
        let build = started.elapsed().as_secs_f64();
        branches[0].divergence = Some(UpstreamDivergence {
            ahead: 3,
            behind: 0,
        });
        let started = Instant::now();
        let _capture = history_perf::capture();
        let again = IndexedGraph::build_reusing(
            index,
            &branches,
            &[],
            &[],
            Some("main"),
            Some(head.as_ref()),
            &cancel,
            Some(&first),
        )
        .unwrap();
        eprintln!(
            "containment rows=2000000 first_build_s={build:.3} rebuild_s={:.3} walks={} transitions={}",
            started.elapsed().as_secs_f64(),
            history_perf::count(Work::ContainmentWalk),
            history_perf::count(Work::GraphTransition)
        );
        assert_eq!(again.checkpoints.len(), first.checkpoints.len());
    }
}

#[cfg(test)]
mod geometry_reuse_regressions {
    use super::*;

    #[test]
    fn indexed_history_ref_renames_reuse_geometry_but_new_head_positions_do_not() {
        let fixture = IndexedHistoryFixture::new(2000, 64, 20);
        let index = fixture.graph.projection.index.clone();
        let cancel = CancellationToken::new();
        let head = index.commit_id(0).unwrap();
        let mut branches = vec![Branch {
            name: "feature".into(),
            target: head.clone(),
            upstream: None,
            divergence: None,
        }];
        let first = IndexedGraph::build(
            index.clone(),
            &branches,
            &[],
            &[],
            Some("feature"),
            Some(head.as_ref()),
            &cancel,
        )
        .unwrap();
        let window = first.window(0..40, &cancel).unwrap();
        branches[0].name = "renamed".into();
        let next = IndexedGraph::build_reusing(
            index.clone(),
            &branches,
            &[],
            &[],
            Some("renamed"),
            Some(head.as_ref()),
            &cancel,
            Some(&first),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&first.lane_spans, &next.lane_spans));
        assert!(Arc::ptr_eq(
            &first.checkpoints[0].walk,
            &next.checkpoints[0].walk
        ));
        let renamed = next.window(0..40, &cancel).unwrap();
        assert!(Arc::ptr_eq(&window.rows, &renamed.rows));
        assert_eq!(
            renamed.labels[0].as_ref().map(|name| name.as_ref()),
            Some("renamed")
        );
        branches.push(Branch {
            name: "new".into(),
            target: index.commit_id(65).unwrap(),
            upstream: None,
            divergence: None,
        });
        let changed = IndexedGraph::build_reusing(
            index,
            &branches,
            &[],
            &[],
            Some("renamed"),
            Some(head.as_ref()),
            &cancel,
            Some(&next),
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&next.lane_spans, &changed.lane_spans));
        assert!(!Arc::ptr_eq(
            &next.checkpoints[0].walk,
            &changed.checkpoints[0].walk
        ));
    }
}
