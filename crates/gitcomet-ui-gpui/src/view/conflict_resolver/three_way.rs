use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThreeWayConflictMaps {
    /// Per-side conflict ranges indexed by [base, ours, theirs].
    pub conflict_ranges: [Vec<std::ops::Range<usize>>; 3],
    /// Per-side per-line conflict maps (populated only in eager mode).
    pub line_conflict_maps: [Vec<Option<usize>>; 3],
    pub conflict_has_base: Vec<bool>,
    pub conflict_resolved: Vec<bool>,
}

/// Project marker-region ranges into the shared aligned source-row space.
///
/// This is the legacy/current-only fallback used when a merge plan is not
/// available. Callers may retain the result before resolved regions are
/// materialized into plain text so every original region remains navigable.
pub(in crate::view) fn project_conflict_ranges_to_aligned_rows(
    segments: &[ConflictSegment],
    aligned: &ThreeWayAlignedMap,
    side_line_counts: [usize; 3],
) -> Vec<Range<usize>> {
    let maps = build_three_way_conflict_maps_without_line_maps(
        segments,
        side_line_counts[0],
        side_line_counts[1],
        side_line_counts[2],
    );
    let block_count = maps.conflict_ranges[1].len();
    let mut aligned_ranges: Vec<Range<usize>> = Vec::with_capacity(block_count);
    for block_ix in 0..block_count {
        let mut start = usize::MAX;
        let mut end = 0usize;
        for side in 0..3 {
            let side_range = &maps.conflict_ranges[side][block_ix];
            if side_range.is_empty() {
                continue;
            }
            let mapped = aligned.aligned_range_for_side_range(side, side_range.clone());
            start = start.min(mapped.start);
            end = end.max(mapped.end);
        }
        if start == usize::MAX {
            start = end;
        }
        if let Some(previous) = aligned_ranges.last() {
            start = start.max(previous.end);
            end = end.max(start);
        }
        aligned_ranges.push(start..end);
    }
    aligned_ranges
}

/// Resolve visible marker blocks back to their exact aligned merge-plan rows.
///
/// Marker text is an output projection. Text between unresolved blocks can
/// come from only one source, so advancing every source offset by that text's
/// line count can merge adjacent conflict highlights or move later highlights
/// past their real rows. Full text sessions retain the authoritative mapping
/// from marker regions to merge-plan blocks; use it whenever it is available.
pub(in crate::view) fn merge_plan_aligned_conflict_ranges(
    session: &gitcomet_core::conflict_session::ConflictSession,
    visible_region_indices: &[usize],
    visible_plan_block_indices: &[usize],
) -> Option<Vec<Range<usize>>> {
    let plan = session.merge_plan.as_ref()?;
    if !visible_plan_block_indices.is_empty() {
        return visible_plan_block_indices
            .iter()
            .map(|block_index| {
                plan.blocks
                    .get(*block_index)
                    .map(|block| block.rows.clone())
            })
            .collect();
    }
    visible_region_indices
        .iter()
        .map(|region_index| {
            let block_index = *session.region_plan_blocks.get(*region_index)?;
            plan.blocks.get(block_index).map(|block| block.rows.clone())
        })
        .collect()
}

/// Binary search on sorted, non-overlapping ranges to find which conflict a line belongs to.
///
/// Returns `Some(conflict_index)` if the line falls within a range, `None` otherwise.
/// Ranges must be sorted by start and non-overlapping for correct results.
pub fn conflict_index_for_line(ranges: &[std::ops::Range<usize>], line: usize) -> Option<usize> {
    ranges
        .binary_search_by(|range| {
            if line < range.start {
                std::cmp::Ordering::Greater
            } else if line >= range.end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .ok()
}

/// Build per-column line-to-conflict maps for three-way conflict rendering.
///
/// The returned `conflict_ranges` follow the legacy behavior and are expressed
/// in the ours-column line space. The line maps provide O(1) conflict lookup
/// for each column at render/navigation time.
pub(in crate::view) fn build_three_way_conflict_maps_impl(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
    include_line_conflict_maps: bool,
) -> ThreeWayConflictMaps {
    if segments.is_empty() {
        return ThreeWayConflictMaps {
            conflict_ranges: Default::default(),
            line_conflict_maps: if include_line_conflict_maps {
                [
                    vec![None; base_line_count],
                    vec![None; ours_line_count],
                    vec![None; theirs_line_count],
                ]
            } else {
                Default::default()
            },
            conflict_has_base: Vec::new(),
            conflict_resolved: Vec::new(),
        };
    }

    let block_count = segments
        .iter()
        .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
        .count();
    let mut maps = ThreeWayConflictMaps {
        conflict_ranges: [
            Vec::with_capacity(block_count),
            Vec::with_capacity(block_count),
            Vec::with_capacity(block_count),
        ],
        line_conflict_maps: if include_line_conflict_maps {
            [
                vec![None; base_line_count],
                vec![None; ours_line_count],
                vec![None; theirs_line_count],
            ]
        } else {
            Default::default()
        },
        conflict_has_base: Vec::with_capacity(block_count),
        conflict_resolved: Vec::with_capacity(block_count),
    };

    fn mark_range(map: &mut [Option<usize>], start: usize, end: usize, conflict_ix: usize) {
        if map.is_empty() {
            return;
        }
        let from = start.min(map.len());
        let to = end.min(map.len());
        for slot in &mut map[from..to] {
            *slot = Some(conflict_ix);
        }
    }

    let mut base_offset = 0usize;
    let mut ours_offset = 0usize;
    let mut theirs_offset = 0usize;
    let mut conflict_ix = 0usize;
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => {
                let line_count = text_line_count_usize(text);
                base_offset = base_offset.saturating_add(line_count);
                ours_offset = ours_offset.saturating_add(line_count);
                theirs_offset = theirs_offset.saturating_add(line_count);
            }
            ConflictSegment::Block(block) => {
                let base_count = text_line_count_usize(block.base.as_deref().unwrap_or_default());
                let ours_count = text_line_count_usize(&block.ours);
                let theirs_count = text_line_count_usize(&block.theirs);

                let base_end = base_offset.saturating_add(base_count);
                let ours_end = ours_offset.saturating_add(ours_count);
                let theirs_end = theirs_offset.saturating_add(theirs_count);

                maps.conflict_ranges[0].push(base_offset..base_end);
                maps.conflict_ranges[1].push(ours_offset..ours_end);
                maps.conflict_ranges[2].push(theirs_offset..theirs_end);
                maps.conflict_has_base.push(block.base.is_some());
                maps.conflict_resolved.push(block.resolved);

                mark_range(
                    &mut maps.line_conflict_maps[0],
                    base_offset,
                    base_end,
                    conflict_ix,
                );
                mark_range(
                    &mut maps.line_conflict_maps[1],
                    ours_offset,
                    ours_end,
                    conflict_ix,
                );
                mark_range(
                    &mut maps.line_conflict_maps[2],
                    theirs_offset,
                    theirs_end,
                    conflict_ix,
                );

                base_offset = base_end;
                ours_offset = ours_end;
                theirs_offset = theirs_end;
                conflict_ix = conflict_ix.saturating_add(1);
            }
        }
    }

    maps
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_three_way_conflict_maps(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
) -> ThreeWayConflictMaps {
    build_three_way_conflict_maps_impl(
        segments,
        base_line_count,
        ours_line_count,
        theirs_line_count,
        true,
    )
}

/// Build compact three-way conflict metadata without eager per-line side maps.
pub fn build_three_way_conflict_maps_without_line_maps(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
) -> ThreeWayConflictMaps {
    build_three_way_conflict_maps_impl(
        segments,
        base_line_count,
        ours_line_count,
        theirs_line_count,
        false,
    )
}

/// Build conflict-index maps for two-way split and inline rows.
///
/// Each output entry is `Some(conflict_index)` when the row belongs to a marker
/// conflict block, or `None` for non-conflict context rows.
#[cfg(any(test, feature = "benchmarks"))]
pub fn map_two_way_rows_to_conflicts(
    segments: &[ConflictSegment],
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
    inline_rows: &[ConflictInlineRow],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let ranges = build_two_way_conflict_line_ranges(segments);
    let split = diff_rows
        .iter()
        .map(|row| row_conflict_index_for_lines(row.old_line, row.new_line, &ranges))
        .collect();
    let inline = inline_rows
        .iter()
        .map(|row| row_conflict_index_for_lines(row.old_line, row.new_line, &ranges))
        .collect();
    (split, inline)
}

/// Build visible row indices for two-way views.
///
/// When `hide_resolved` is true, rows belonging to resolved conflict blocks are
/// removed from the visible list. Non-conflict rows are always kept visible.
#[cfg(any(test, feature = "benchmarks"))]
pub fn build_two_way_visible_indices(
    row_conflict_map: &[Option<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<usize> {
    if !hide_resolved {
        return (0..row_conflict_map.len()).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    row_conflict_map
        .iter()
        .enumerate()
        .filter_map(|(ix, conflict_ix)| match conflict_ix {
            Some(ci) if resolved_blocks.get(*ci).copied().unwrap_or(false) => None,
            _ => Some(ix),
        })
        .collect()
}

/// Find the visible list index for the first row that belongs to `conflict_ix`.
///
/// `visible_row_indices` maps visible list rows to source row indices. This helper
/// resolves conflict index -> visible row index so callers can scroll/focus a
/// specific conflict in two-way resolver modes.
#[cfg(test)]
pub fn visible_index_for_two_way_conflict(
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
    conflict_ix: usize,
) -> Option<usize> {
    visible_row_indices.iter().position(|&row_ix| {
        row_conflict_map
            .get(row_ix)
            .copied()
            .flatten()
            .is_some_and(|ix| ix == conflict_ix)
    })
}

/// Build unresolved-only visible navigation entries for two-way views.
///
/// Returns visible list indices (not source row indices) in unresolved queue
/// order so callers can feed them directly into shared diff navigation helpers.
#[cfg(test)]
pub fn unresolved_visible_nav_entries_for_two_way(
    segments: &[ConflictSegment],
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
) -> Vec<usize> {
    unresolved_conflict_indices(segments)
        .into_iter()
        .filter_map(|conflict_ix| {
            visible_index_for_two_way_conflict(row_conflict_map, visible_row_indices, conflict_ix)
        })
        .collect()
}

/// Map a two-way visible index back to its conflict index.
#[cfg(test)]
pub fn two_way_conflict_index_for_visible_row(
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
    visible_ix: usize,
) -> Option<usize> {
    let row_ix = *visible_row_indices.get(visible_ix)?;
    row_conflict_map.get(row_ix).copied().flatten()
}

/// Represents a visible row in the three-way view when hide-resolved is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreeWayVisibleItem {
    /// A normal line at the given index in the three-way data.
    Line(usize),
    /// A collapsed summary row for a resolved conflict block (by conflict index).
    CollapsedBlock(usize),
    /// A folded run of unchanged context lines (section 30 collapsed context mode).
    CollapsedContext {
        source_line_start: usize,
        len: usize,
        /// Stable fold identity (the fold's start line before any reveals),
        /// used to key partial-reveal state.
        fold_id: usize,
    },
}

/// kdiff3-style aligned row space over base/ours/theirs (section 30).
///
/// Maps between visual rows (shared by all columns) and per-side line
/// indices. Sides shorter than a run are padded: their rows map to `None`.
/// The default value is an unbounded identity map (row == line on every
/// side), which is also the fallback when alignment is unavailable
/// (missing/binary sides, giant files).
#[derive(Clone, Debug, Default)]
pub struct ThreeWayAlignedMap {
    runs: Vec<AlignedMapRun>,
    aligned_len: usize,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct AlignedMapRun {
    aligned_start: usize,
    rows: usize,
    starts: [usize; 3],
    lens: [usize; 3],
    kind: gitcomet_core::merge::AlignedRunKind,
}

impl ThreeWayAlignedMap {
    /// Build from the merge engine's alignment runs.
    pub fn from_alignment(alignment: &[gitcomet_core::merge::AlignedRun]) -> Self {
        let mut runs = Vec::with_capacity(alignment.len());
        let mut aligned_start = 0usize;
        for run in alignment {
            let rows = run.visual_rows();
            runs.push(AlignedMapRun {
                aligned_start,
                rows,
                starts: [run.base.start, run.ours.start, run.theirs.start],
                lens: [run.base.len(), run.ours.len(), run.theirs.len()],
                kind: run.kind,
            });
            aligned_start += rows;
        }
        Self {
            runs,
            aligned_len: aligned_start,
        }
    }

    /// The identity map behaves as if every side had `row == line`.
    pub fn is_identity(&self) -> bool {
        self.runs.is_empty()
    }

    /// Total aligned rows. Zero for the identity map (callers keep their own
    /// length in that case).
    pub fn aligned_len(&self) -> usize {
        self.aligned_len
    }

    fn run_for_row(&self, row: usize) -> Option<&AlignedMapRun> {
        if row >= self.aligned_len {
            return None;
        }
        let ix = self
            .runs
            .partition_point(|run| run.aligned_start + run.rows <= row);
        self.runs.get(ix)
    }

    /// The side line rendered at `row`, or `None` for padding rows (and rows
    /// past the aligned end).
    pub fn side_line_for_row(&self, side: usize, row: usize) -> Option<usize> {
        if self.is_identity() {
            return Some(row);
        }
        let run = self.run_for_row(row)?;
        let offset = row - run.aligned_start;
        (offset < run.lens[side]).then(|| run.starts[side] + offset)
    }

    /// The row at which a side line renders. Lines past the side's end clamp
    /// to the end of the aligned space.
    pub fn row_for_side_line(&self, side: usize, line: usize) -> usize {
        if self.is_identity() {
            return line;
        }
        let ix = self
            .runs
            .partition_point(|run| run.starts[side] + run.lens[side] <= line);
        match self.runs.get(ix) {
            Some(run) => run.aligned_start + line.saturating_sub(run.starts[side]),
            None => self.aligned_len,
        }
    }

    /// section 30 split: the side line index a split boundary at aligned `row` maps
    /// to — i.e. the first side line at or after `row` (padding rows round up
    /// to the next real line; rows past the aligned end clamp to the side
    /// length). Use with `row` and `row_end + 1` to bracket a selection.
    pub fn side_line_lower_bound(&self, side: usize, row: usize) -> usize {
        if self.is_identity() {
            return row;
        }
        match self.run_for_row(row) {
            Some(run) => {
                let offset = row - run.aligned_start;
                run.starts[side] + offset.min(run.lens[side])
            }
            None => self
                .runs
                .last()
                .map(|run| run.starts[side] + run.lens[side])
                .unwrap_or(0),
        }
    }

    /// Map a per-side line range to the aligned row range covering it.
    pub fn aligned_range_for_side_range(
        &self,
        side: usize,
        range: std::ops::Range<usize>,
    ) -> std::ops::Range<usize> {
        if self.is_identity() {
            return range;
        }
        if range.is_empty() {
            let boundary = self.row_for_side_line(side, range.start);
            return boundary..boundary;
        }
        let start = self.row_for_side_line(side, range.start);
        let end = self.row_for_side_line(side, range.end.saturating_sub(1)) + 1;
        start..end.max(start)
    }
}

/// section 30 R11: cap on aligned rows that receive word-level highlights, bounding
/// the per-row word-diff work on files with huge change counts.
pub const ALIGNED_WORD_HIGHLIGHT_MAX_ROWS: usize = 4_000;

pub(in crate::view) fn merge_word_highlight_ranges(
    highlights: &mut WordHighlights,
    line_ix: usize,
    ranges: Vec<Range<usize>>,
) {
    if ranges.is_empty() {
        return;
    }
    let entry = highlights.entry(line_ix).or_default();
    entry.extend(ranges);
    entry.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(entry.len());
    for r in entry.drain(..) {
        if let Some(last) = merged.last_mut().filter(|l| r.start <= l.end) {
            last.end = last.end.max(r.end);
            continue;
        }
        merged.push(r);
    }
    *entry = merged;
}

/// section 30 R11 (kdiff3 change colours): word highlights over the aligned row
/// space. For each aligned row where a side's line differs from the base line
/// paired at the same row, word-diff the pair and record ranges keyed by each
/// side's own line index (the renderer's cache key space). Padding rows
/// (added/removed lines) get no word ranges — the per-side row tint already
/// marks them whole. Requires a real base; both-added (two-way) maps use the
/// two-way highlight path instead.
pub fn compute_aligned_three_way_word_highlights(
    aligned: &ThreeWayAlignedMap,
    base_text: &str,
    base_line_starts: &[usize],
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
) -> (WordHighlights, WordHighlights, WordHighlights) {
    let mut wh_base = WordHighlights::default();
    let mut wh_ours = WordHighlights::default();
    let mut wh_theirs = WordHighlights::default();
    if aligned.is_identity() || base_text.is_empty() {
        return (wh_base, wh_ours, wh_theirs);
    }

    let mut budget = ALIGNED_WORD_HIGHLIGHT_MAX_ROWS;
    for row in 0..aligned.aligned_len() {
        if budget == 0 {
            break;
        }
        let Some(base_ix) = aligned.side_line_for_row(0, row) else {
            continue;
        };
        let Some(base_line) = indexed_line_text(base_text, base_line_starts, base_ix) else {
            continue;
        };
        let mut row_diffed = false;
        for (side, side_text, side_starts, side_highlights) in [
            (1usize, ours_text, ours_line_starts, &mut wh_ours),
            (2usize, theirs_text, theirs_line_starts, &mut wh_theirs),
        ] {
            let Some(side_ix) = aligned.side_line_for_row(side, row) else {
                continue;
            };
            let Some(side_line) = indexed_line_text(side_text, side_starts, side_ix) else {
                continue;
            };
            if side_line == base_line {
                continue;
            }
            let (base_ranges, side_ranges) =
                crate::view::word_diff::capped_word_diff_ranges(base_line, side_line);
            merge_word_highlight_ranges(&mut wh_base, base_ix, base_ranges);
            merge_word_highlight_ranges(side_highlights, side_ix, side_ranges);
            row_diffed = true;
        }
        if row_diffed {
            budget -= 1;
        }
    }

    (wh_base, wh_ours, wh_theirs)
}

/// section 30 R11: aligned two-way (ours↔theirs) word highlights, precomputed
/// once per conflict-source rebuild and shared by both diff columns (Ours and
/// Theirs). Keyed by aligned row — the renderer's row space. Only rows where
/// both sides have a line and the two lines differ byte-wise get an entry; the
/// render-time whitespace mode still decides whether to *apply* them
/// (whitespace-equal rows render as context), so this stays independent of that
/// toggle. Replaces the previous per-render, per-column inline word diff.
pub fn compute_aligned_two_way_word_highlights(
    aligned: &ThreeWayAlignedMap,
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
) -> FxHashMap<usize, TwoWayWordHighlightPair> {
    let mut highlights = FxHashMap::default();
    if aligned.is_identity() {
        return highlights;
    }

    let mut budget = ALIGNED_WORD_HIGHLIGHT_MAX_ROWS;
    for row in 0..aligned.aligned_len() {
        if budget == 0 {
            break;
        }
        let (Some(ours_ix), Some(theirs_ix)) = (
            aligned.side_line_for_row(1, row),
            aligned.side_line_for_row(2, row),
        ) else {
            continue;
        };
        let (Some(ours_line), Some(theirs_line)) = (
            indexed_line_text(ours_text, ours_line_starts, ours_ix),
            indexed_line_text(theirs_text, theirs_line_starts, theirs_ix),
        ) else {
            continue;
        };
        if ours_line == theirs_line {
            continue;
        }
        if let Some(pair) = compute_word_highlights_for_texts(ours_line, theirs_line) {
            highlights.insert(row, pair);
            budget -= 1;
        }
    }

    highlights
}

/// Span-based replacement for `Vec<ThreeWayVisibleItem>` that uses O(spans) memory
/// instead of O(visible lines). Each span covers a contiguous run of source lines
/// or a single synthetic row (collapsed block / preview gap).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreeWayVisibleSpan {
    /// A contiguous run of source lines mapped 1:1 to visible indices.
    Lines {
        visible_start: usize,
        source_line_start: usize,
        len: usize,
    },
    /// A single collapsed-block row at the given visible index.
    CollapsedResolvedBlock {
        visible_index: usize,
        conflict_ix: usize,
    },
    /// A single fold row hiding `len` unchanged context lines.
    CollapsedContext {
        visible_index: usize,
        source_line_start: usize,
        len: usize,
        /// Stable fold identity (start line before any reveals).
        fold_id: usize,
    },
}

impl ThreeWayVisibleSpan {
    fn visible_start(&self) -> usize {
        match *self {
            Self::Lines { visible_start, .. } => visible_start,
            Self::CollapsedResolvedBlock { visible_index, .. } => visible_index,
            Self::CollapsedContext { visible_index, .. } => visible_index,
        }
    }

    fn visible_len(&self) -> usize {
        match *self {
            Self::Lines { len, .. } => len,
            Self::CollapsedResolvedBlock { .. } | Self::CollapsedContext { .. } => 1,
        }
    }
}

/// Compact visible-index projection for three-way views.
///
/// Replaces `Vec<ThreeWayVisibleItem>` for giant mode. Stores spans instead of
/// per-row entries, keeping memory proportional to the number of conflict blocks
/// rather than the number of file lines.
#[derive(Clone, Debug, Default)]
pub struct ThreeWayVisibleProjection {
    spans: Vec<ThreeWayVisibleSpan>,
    visible_len: usize,
}

pub(in crate::view) enum ThreeWayVisibleRun {
    Lines { start: usize, end: usize },
    Collapsed { conflict_ix: usize },
}

pub(in crate::view) fn for_each_three_way_visible_run(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
    mut visit: impl FnMut(ThreeWayVisibleRun),
) {
    if total_lines == 0 {
        return;
    }

    if !hide_resolved {
        visit(ThreeWayVisibleRun::Lines {
            start: 0,
            end: total_lines,
        });
        return;
    }

    let mut line_ix = 0usize;

    for (range_ix, range) in conflict_ranges.iter().enumerate() {
        if line_ix >= total_lines {
            break;
        }

        let range_start = range.start.min(total_lines);
        let range_end = range.end.min(total_lines);

        if line_ix < range_start {
            visit(ThreeWayVisibleRun::Lines {
                start: line_ix,
                end: range_start,
            });
            line_ix = range_start;
        }

        let resolved = conflict_resolved.get(range_ix).copied().unwrap_or(false);
        if resolved && range_start < range_end && line_ix < range_end {
            visit(ThreeWayVisibleRun::Collapsed {
                conflict_ix: range_ix,
            });
            line_ix = range_end;
            continue;
        }

        if line_ix < range_end {
            visit(ThreeWayVisibleRun::Lines {
                start: line_ix,
                end: range_end,
            });
            line_ix = range_end;
        }
    }

    if line_ix < total_lines {
        visit(ThreeWayVisibleRun::Lines {
            start: line_ix,
            end: total_lines,
        });
    }
}

impl ThreeWayVisibleProjection {
    /// Total number of visible rows.
    pub fn len(&self) -> usize {
        self.visible_len
    }

    /// Look up the visible item at the given visible index. O(log spans).
    pub fn get(&self, visible_ix: usize) -> Option<ThreeWayVisibleItem> {
        if visible_ix >= self.visible_len {
            return None;
        }
        let span_ix = self
            .spans
            .partition_point(|s| s.visible_start() + s.visible_len() <= visible_ix);
        let span = self.spans.get(span_ix)?;
        match *span {
            ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } => {
                let offset = visible_ix.checked_sub(visible_start)?;
                if offset >= len {
                    return None;
                }
                Some(ThreeWayVisibleItem::Line(source_line_start + offset))
            }
            ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index,
                conflict_ix,
            } => {
                if visible_ix != visible_index {
                    return None;
                }
                Some(ThreeWayVisibleItem::CollapsedBlock(conflict_ix))
            }
            ThreeWayVisibleSpan::CollapsedContext {
                visible_index,
                source_line_start,
                len,
                fold_id,
            } => {
                if visible_ix != visible_index {
                    return None;
                }
                Some(ThreeWayVisibleItem::CollapsedContext {
                    source_line_start,
                    len,
                    fold_id,
                })
            }
        }
    }

    /// Find the visible index for the first line of a conflict range, or its
    /// collapsed entry. Returns `None` if the range is not visible.
    /// O(log spans).
    pub fn visible_index_for_conflict(
        &self,
        conflict_ranges: &[std::ops::Range<usize>],
        range_ix: usize,
    ) -> Option<usize> {
        let range = conflict_ranges.get(range_ix)?;
        for span in &self.spans {
            match *span {
                ThreeWayVisibleSpan::Lines {
                    visible_start,
                    source_line_start,
                    len,
                } => {
                    let source_end = source_line_start + len;
                    if range.start >= source_line_start && range.start < source_end {
                        return Some(visible_start + (range.start - source_line_start));
                    }
                }
                ThreeWayVisibleSpan::CollapsedResolvedBlock {
                    visible_index,
                    conflict_ix,
                } if conflict_ix == range_ix => {
                    return Some(visible_index);
                }
                _ => {}
            }
        }
        None
    }

    /// Access the underlying spans for direct iteration (avoids per-item O(log n) lookup).
    pub fn spans(&self) -> &[ThreeWayVisibleSpan] {
        &self.spans
    }

    /// Find the visible index showing the given source line. Lines hidden
    /// inside a collapsed context fold map to the fold's row.
    pub fn visible_index_for_source_line(&self, line: usize) -> Option<usize> {
        for span in &self.spans {
            match *span {
                ThreeWayVisibleSpan::Lines {
                    visible_start,
                    source_line_start,
                    len,
                } => {
                    if line >= source_line_start && line < source_line_start + len {
                        return Some(visible_start + (line - source_line_start));
                    }
                }
                ThreeWayVisibleSpan::CollapsedContext {
                    visible_index,
                    source_line_start,
                    len,
                    ..
                } => {
                    if line >= source_line_start && line < source_line_start + len {
                        return Some(visible_index);
                    }
                }
                ThreeWayVisibleSpan::CollapsedResolvedBlock { .. } => {}
            }
        }
        None
    }
}

/// Blank rows appended below the last line of the source diff lists so the
/// tail of the file can be scrolled up into a comfortable reading position.
pub const CONFLICT_BOTTOM_OVERSCROLL_ROWS: usize = 10;

/// Number of bands the minimap column is quantized into.
///
/// kdiff3 paints one band per line; bounding the band count keeps paint cost
/// independent of file size while staying far finer than any column height.
pub const MINIMAP_BAND_COUNT: usize = 2048;

/// Build the minimap column's bands for the current three-way projection.
///
/// The result is in *visible* row space so the painted column lines up with
/// what the panes actually show: rows hidden by hide-resolved or collapsed
/// context are folded into their summary row's band, exactly as they are in
/// the lists. Returns an empty vector when the map carries no classification
/// (the identity fallback used for unaligned/giant files), which callers treat
/// as "no minimap available".
///
/// `conflict_ranges` and `conflict_resolved` are the aligned conflict ranges
/// and their resolution state, in step: a conflict the user has settled is
/// repainted in the resolved color so the bands that stay red are the work
/// that is left.
///
/// `trailing_rows` are the blank overscroll rows the lists append below the
/// last line. They carry no changes but do take up scroll range, so the bands
/// have to cover them for the viewport frame to line up with the panes.
pub fn build_minimap_bands(
    aligned: &ThreeWayAlignedMap,
    projection: &ThreeWayVisibleProjection,
    conflict_ranges: &[Range<usize>],
    conflict_resolved: &[bool],
    trailing_rows: usize,
) -> Vec<gitcomet_core::merge::MinimapRowKind> {
    use gitcomet_core::merge::MinimapRowKind;

    if aligned.is_identity() || projection.len() == 0 {
        return Vec::new();
    }
    let visible_len = projection.len() + trailing_rows;

    let band_count = MINIMAP_BAND_COUNT.min(visible_len);
    let mut bands = vec![MinimapRowKind::Unchanged; band_count];
    let band_span = |visible: std::ops::Range<usize>| {
        let first = visible.start.min(visible_len - 1) * band_count / visible_len;
        let last = (visible.end - 1).min(visible_len - 1) * band_count / visible_len;
        first..=last.min(band_count - 1)
    };
    let mut paint = |visible: std::ops::Range<usize>, kind: MinimapRowKind| {
        if kind == MinimapRowKind::Unchanged || visible.is_empty() {
            return;
        }
        for band in &mut bands[band_span(visible)] {
            *band = band.merge(kind);
        }
    };

    // Walk the visible spans and, for each, the aligned runs it covers. A
    // collapsed span shows several aligned rows on one visible row, so every
    // run it hides merges into that row's band.
    let spans = projection.spans();
    let runs = &aligned.runs;
    let aligned_len = aligned.aligned_len();
    let span_source_start = |span: &ThreeWayVisibleSpan| match *span {
        ThreeWayVisibleSpan::Lines {
            source_line_start, ..
        }
        | ThreeWayVisibleSpan::CollapsedContext {
            source_line_start, ..
        } => Some(source_line_start),
        // A collapsed conflict block carries no source range of its own.
        ThreeWayVisibleSpan::CollapsedResolvedBlock { .. } => None,
    };

    let mut covered = 0usize;
    for (span_ix, span) in spans.iter().enumerate() {
        let (source, visible_start, collapsed) = match *span {
            ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } => (
                source_line_start..source_line_start + len,
                visible_start,
                false,
            ),
            ThreeWayVisibleSpan::CollapsedContext {
                visible_index,
                source_line_start,
                len,
                ..
            } => (
                source_line_start..source_line_start + len,
                visible_index,
                true,
            ),
            ThreeWayVisibleSpan::CollapsedResolvedBlock { visible_index, .. } => {
                // The hidden rows are everything between the previous span and
                // the next one that names a source line.
                let next = spans[span_ix + 1..]
                    .iter()
                    .find_map(span_source_start)
                    .unwrap_or(aligned_len);
                (covered..next.max(covered), visible_index, true)
            }
        };
        covered = covered.max(source.end);
        if source.is_empty() {
            continue;
        }

        let mut run_ix = runs.partition_point(|run| run.aligned_start + run.rows <= source.start);
        while let Some(run) = runs
            .get(run_ix)
            .filter(|run| run.aligned_start < source.end)
        {
            run_ix += 1;
            let kind = gitcomet_core::merge::minimap_row_kind(run.kind);
            if kind == MinimapRowKind::Unchanged {
                continue;
            }
            if collapsed {
                paint(visible_start..visible_start + 1, kind);
                continue;
            }
            let start = run.aligned_start.max(source.start);
            let end = (run.aligned_start + run.rows).min(source.end);
            paint(
                visible_start + (start - source.start)..visible_start + (end - source.start),
                kind,
            );
        }
    }

    // Second pass: a conflict the user has settled recedes to the resolved
    // color. Only bands the first pass classified as an open conflict change,
    // so a one-sided change sharing a band keeps its own side's color.
    for (range_ix, range) in conflict_ranges.iter().enumerate() {
        if range.is_empty() || !conflict_resolved.get(range_ix).copied().unwrap_or(false) {
            continue;
        }
        // A block hidden behind hide-resolved has no visible rows of its own;
        // its summary row is the one to repaint.
        let visible = match (
            projection.visible_index_for_source_line(range.start),
            projection.visible_index_for_source_line(range.end - 1),
        ) {
            (Some(first), Some(last)) if last >= first => first..last + 1,
            _ => match projection.visible_index_for_conflict(conflict_ranges, range_ix) {
                Some(row) => row..row + 1,
                None => continue,
            },
        };
        for band in &mut bands[band_span(visible)] {
            if *band == MinimapRowKind::Conflict {
                *band = band.resolved();
            }
        }
    }

    bands
}

/// One `resolved` flag per marker block, in display order — the state the
/// minimap and the visible projection classify conflicts with.
pub(in crate::view) fn resolved_conflict_flags_from_segments(
    segments: &[ConflictSegment],
) -> Vec<bool> {
    segments
        .iter()
        .filter_map(|segment| match segment {
            ConflictSegment::Block(block) => Some(block.resolved),
            ConflictSegment::Text(_) => None,
        })
        .collect()
}

/// Build a span-based visible projection for three-way views.
///
/// All lines in every conflict block are included (no preview gaps).
/// Resolved blocks collapse to a single summary row when `hide_resolved` is true.
/// Context lines kept visible on each side of a conflict when collapsed
/// context mode is active (section 30).
pub(crate) const CONFLICT_COLLAPSED_CONTEXT_LINES: usize = 3;

/// Runs shorter than this stay expanded — a fold row would not be
/// meaningfully shorter than the lines it hides.
pub(in crate::view) const MIN_CONTEXT_FOLD_LINES: usize = 2;

/// Lines revealed per click of a fold's reveal arrows (matches the diff
/// view's collapsed-hunk reveal step).
pub(crate) const CONFLICT_FOLD_REVEAL_STEP: usize = 20;

/// Per-fold partial-reveal state, keyed by the fold's stable identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct ConflictFoldReveal {
    /// Lines revealed from the top edge of the fold.
    pub top: usize,
    /// Lines revealed from the bottom edge of the fold.
    pub bottom: usize,
    /// The user expanded the whole fold.
    pub expand_all: bool,
}

/// Visibility options for the three-way projection.
#[derive(Clone, Copy, Default)]
pub(in crate::view) struct ThreeWayVisibleOptions<'a> {
    pub hide_resolved: bool,
    /// section 30 collapsed context mode: fold unchanged runs beyond
    /// [`CONFLICT_COLLAPSED_CONTEXT_LINES`] around each conflict.
    pub collapse_context: bool,
    /// Per-fold reveal state, keyed by fold identity (pre-reveal start line).
    pub context_fold_reveals: Option<&'a FxHashMap<usize, ConflictFoldReveal>>,
}

/// Build the three-way visible projection with hide-resolved and collapsed
/// context folding applied.
pub(in crate::view) fn build_three_way_visible_projection_with_options(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    options: ThreeWayVisibleOptions<'_>,
) -> ThreeWayVisibleProjection {
    if !options.collapse_context || conflict_ranges.is_empty() {
        return build_three_way_visible_projection_with_resolved_flags(
            total_lines,
            conflict_ranges,
            conflict_resolved,
            options.hide_resolved,
        );
    }
    if total_lines == 0 {
        return ThreeWayVisibleProjection::default();
    }

    let fold_reveal = |fold_id: usize| {
        options
            .context_fold_reveals
            .and_then(|reveals| reveals.get(&fold_id).copied())
            .unwrap_or_default()
    };

    let mut spans: Vec<ThreeWayVisibleSpan> = Vec::new();
    let mut visible_ix = 0usize;
    let push_lines =
        |spans: &mut Vec<ThreeWayVisibleSpan>, visible_ix: &mut usize, start: usize, len: usize| {
            if len == 0 {
                return;
            }
            spans.push(ThreeWayVisibleSpan::Lines {
                visible_start: *visible_ix,
                source_line_start: start,
                len,
            });
            *visible_ix += len;
        };
    // Emit an unchanged gap, keeping `leading_keep` lines adjacent to the
    // previous conflict and `trailing_keep` lines before the next one;
    // anything beyond that folds unless the user expanded it.
    let push_gap = |spans: &mut Vec<ThreeWayVisibleSpan>,
                    visible_ix: &mut usize,
                    start: usize,
                    end: usize,
                    leading_keep: usize,
                    trailing_keep: usize| {
        let len = end.saturating_sub(start);
        if len == 0 {
            return;
        }
        let keep = leading_keep.saturating_add(trailing_keep);
        let fold_len = len.saturating_sub(keep);
        let fold_start = start + leading_keep;
        // The fold identity is its pre-reveal start line, so partial reveals
        // keep addressing the same fold.
        let fold_id = fold_start;
        let reveal = fold_reveal(fold_id);
        let revealed_top = reveal.top.min(fold_len);
        let revealed_bottom = reveal.bottom.min(fold_len.saturating_sub(revealed_top));
        let remaining = fold_len - revealed_top - revealed_bottom;
        if reveal.expand_all
            || fold_len < MIN_CONTEXT_FOLD_LINES
            || remaining < MIN_CONTEXT_FOLD_LINES
        {
            push_lines(spans, visible_ix, start, len);
            return;
        }
        push_lines(spans, visible_ix, start, leading_keep + revealed_top);
        spans.push(ThreeWayVisibleSpan::CollapsedContext {
            visible_index: *visible_ix,
            source_line_start: fold_start + revealed_top,
            len: remaining,
            fold_id,
        });
        *visible_ix += 1;
        push_lines(
            spans,
            visible_ix,
            fold_start + revealed_top + remaining,
            revealed_bottom + trailing_keep,
        );
    };

    let ctx = CONFLICT_COLLAPSED_CONTEXT_LINES;
    let mut line_ix = 0usize;
    for (range_ix, range) in conflict_ranges.iter().enumerate() {
        if line_ix >= total_lines {
            break;
        }
        let range_start = range.start.min(total_lines).max(line_ix);
        let range_end = range.end.min(total_lines).max(range_start);

        let leading_keep = if range_ix == 0 { 0 } else { ctx };
        push_gap(
            &mut spans,
            &mut visible_ix,
            line_ix,
            range_start,
            leading_keep,
            ctx,
        );

        let resolved = conflict_resolved.get(range_ix).copied().unwrap_or(false);
        if options.hide_resolved && resolved && range_start < range_end {
            spans.push(ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index: visible_ix,
                conflict_ix: range_ix,
            });
            visible_ix += 1;
        } else {
            push_lines(
                &mut spans,
                &mut visible_ix,
                range_start,
                range_end - range_start,
            );
        }
        line_ix = range_end;
    }
    if line_ix < total_lines {
        push_gap(&mut spans, &mut visible_ix, line_ix, total_lines, ctx, 0);
    }

    ThreeWayVisibleProjection {
        spans,
        visible_len: visible_ix,
    }
}

pub(in crate::view) fn build_three_way_visible_projection_with_resolved_flags(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
) -> ThreeWayVisibleProjection {
    if total_lines == 0 {
        return ThreeWayVisibleProjection::default();
    }

    if !hide_resolved {
        return ThreeWayVisibleProjection {
            spans: vec![ThreeWayVisibleSpan::Lines {
                visible_start: 0,
                source_line_start: 0,
                len: total_lines,
            }],
            visible_len: total_lines,
        };
    }

    let mut spans: Vec<ThreeWayVisibleSpan> =
        Vec::with_capacity(conflict_ranges.len().saturating_mul(2).saturating_add(1));
    let mut visible_ix = 0usize;
    for_each_three_way_visible_run(
        total_lines,
        conflict_ranges,
        conflict_resolved,
        true,
        |run| match run {
            ThreeWayVisibleRun::Lines { start, end } => {
                let len = end.saturating_sub(start);
                if len == 0 {
                    return;
                }
                spans.push(ThreeWayVisibleSpan::Lines {
                    visible_start: visible_ix,
                    source_line_start: start,
                    len,
                });
                visible_ix += len;
            }
            ThreeWayVisibleRun::Collapsed { conflict_ix } => {
                spans.push(ThreeWayVisibleSpan::CollapsedResolvedBlock {
                    visible_index: visible_ix,
                    conflict_ix,
                });
                visible_ix += 1;
            }
        },
    );

    ThreeWayVisibleProjection {
        spans,
        visible_len: visible_ix,
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_three_way_visible_projection(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> ThreeWayVisibleProjection {
    let conflict_resolved = resolved_conflict_flags_from_segments(segments);
    build_three_way_visible_projection_with_resolved_flags(
        total_lines,
        conflict_ranges,
        &conflict_resolved,
        hide_resolved,
    )
}

/// Build the mapping from visible row indices to actual three-way data items.
///
/// When `hide_resolved` is false, every line maps directly.
/// When true, resolved conflict ranges are collapsed to a single summary row.
#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn build_three_way_visible_map_with_resolved_flags(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if total_lines == 0 {
        return Vec::new();
    }

    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let mut visible_len = 0usize;
    for_each_three_way_visible_run(
        total_lines,
        conflict_ranges,
        conflict_resolved,
        true,
        |run| match run {
            ThreeWayVisibleRun::Lines { start, end } => {
                visible_len += end.saturating_sub(start);
            }
            ThreeWayVisibleRun::Collapsed { .. } => {
                visible_len += 1;
            }
        },
    );

    let mut visible = Vec::with_capacity(visible_len);
    for_each_three_way_visible_run(
        total_lines,
        conflict_ranges,
        conflict_resolved,
        true,
        |run| match run {
            ThreeWayVisibleRun::Lines { start, end } => {
                for line_ix in start..end {
                    visible.push(ThreeWayVisibleItem::Line(line_ix));
                }
            }
            ThreeWayVisibleRun::Collapsed { conflict_ix } => {
                visible.push(ThreeWayVisibleItem::CollapsedBlock(conflict_ix));
            }
        },
    );
    visible
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_three_way_visible_map(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    let conflict_resolved = resolved_conflict_flags_from_segments(segments);
    build_three_way_visible_map_with_resolved_flags(
        total_lines,
        conflict_ranges,
        &conflict_resolved,
        hide_resolved,
    )
}
