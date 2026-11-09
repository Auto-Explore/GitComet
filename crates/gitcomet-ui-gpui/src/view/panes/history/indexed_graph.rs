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

    /// Retained bytes of the per-column lane spans that resolve selected-lane
    /// highlights, including each column's vector header.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn lane_span_bytes(&self) -> usize {
        self.lane_spans.capacity() * std::mem::size_of::<Vec<LaneSpan>>()
            + self
                .lane_spans
                .iter()
                .map(|spans| spans.capacity() * std::mem::size_of::<LaneSpan>())
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
    pub fn lane_span_bytes(&self) -> usize {
        self.graph.lane_span_bytes()
    }
    pub fn clear_window(&self) {
        *self.graph.window_cache.lock().unwrap() = None;
        *self.graph.geometry_cache.lock().unwrap() = None;
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

#[cfg(test)]
mod benchmark_tests;
#[cfg(test)]
mod performance_regression_tests;
#[cfg(test)]
mod reuse_regression_tests;
#[cfg(test)]
mod tests;
