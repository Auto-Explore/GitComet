//! Where conflict blocks land in the resolved output: block line ranges,
//! gutter markers, unresolved rows, and the edit deltas that move them.

use super::*;
use crate::kit::text_model::TextModelSnapshot;
use rustc_hash::FxHasher;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutputSourceRevision {
    pub(in crate::view) model_id: u64,
    pub(in crate::view) revision: u64,
}

impl ResolvedOutputSourceRevision {
    pub(in crate::view) fn from_snapshot(snapshot: &TextModelSnapshot) -> Self {
        Self {
            model_id: snapshot.model_id(),
            revision: snapshot.revision(),
        }
    }
}

pub(in crate::view::panes::main) fn resolved_output_snapshot_is_modified(
    saved: Option<&TextModelSnapshot>,
    current: &TextModelSnapshot,
) -> bool {
    saved.is_some_and(|saved| current != saved)
}

/// Whether the worktree payload must be kept as opaque user output instead of
/// replacing it with the stage-derived marker projection.
///
/// The question this answers is "would projecting throw away work someone did by
/// hand?", and the only usable evidence is the document's own content: every
/// line of an untouched conflict document comes from one of the three stages,
/// because git assembled it out of them. A hand resolution types something new,
/// and that line belongs to no stage.
///
/// The comparison is deliberately *not* against the projection or the stage
/// blobs. Two correct three-way merges of the same stages may place their
/// conflict boundaries in entirely different places — ours anchors differently
/// than git's `xdiff` does, and the contributor-alignment pass moves boundaries
/// again — so the two documents interleave the same lines in different orders
/// and neither reconstructs to the other's sides nor to a whole stage blob.
/// Demanding either equality protected essentially every real merge, which left
/// the resolver inert: no marker geometry, every pick a silent no-op, and no way
/// out but *Reset conflict markers*.
///
/// The cost of the weaker test is that a hand resolution built purely by
/// *deleting* lines — picking a side in an editor, markers and all — reads as
/// untouched. Nothing is lost on disk either way: the resolver only rewrites its
/// own buffer, and the worktree file stands until an explicit Save.
pub(in crate::view::panes::main) fn worktree_output_requires_protection(
    current: Option<&str>,
    marker_projection: Option<&str>,
    base: Option<&str>,
    ours: Option<&str>,
    theirs: Option<&str>,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    if marker_projection == Some(current) {
        return false;
    }
    // The projection renders every line with one detected ending, so a document
    // that mixes CRLF and LF cannot be reproduced from it even when every line
    // of it comes from a stage. Keep the worktree bytes rather than rewriting
    // terminators the user never touched.
    if gitcomet_core::text_utils::text_has_mixed_line_endings(current) {
        return true;
    }

    let marker_ranges = gitcomet_core::conflict_session::parse_conflict_marker_ranges(current);
    if !marker_ranges.iter().any(|segment| {
        matches!(
            segment,
            gitcomet_core::conflict_session::ParsedConflictSegmentRanges::Conflict(_)
        )
    }) {
        return true;
    }

    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        return true;
    };
    // `std`'s seeded `HashSet`, not `FxHashSet`: the keys are raw file lines
    // off disk, i.e. content an untrusted repository controls.
    let stage_lines: HashSet<&str> = base
        .into_iter()
        .chain([ours, theirs])
        .flat_map(str::lines)
        .collect();
    !conflict_document_content_lines(current, &marker_ranges).all(|line| stage_lines.contains(line))
}

/// The document's lines with the four marker lines left out, so a comparison
/// against stage content is not defeated by the labels git wrote.
fn conflict_document_content_lines<'a>(
    current: &'a str,
    marker_ranges: &'a [gitcomet_core::conflict_session::ParsedConflictSegmentRanges],
) -> impl Iterator<Item = &'a str> {
    use gitcomet_core::conflict_session::ParsedConflictSegmentRanges as Segment;

    marker_ranges
        .iter()
        .flat_map(move |segment| {
            let ranges = match segment {
                Segment::Text(range) => vec![range.clone()],
                Segment::Conflict(block) => [Some(block.ours.clone()), block.base.clone()]
                    .into_iter()
                    .flatten()
                    .chain([block.theirs.clone()])
                    .collect(),
            };
            ranges.into_iter()
        })
        .flat_map(move |range| current.get(range).unwrap_or_default().lines())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutlineDelta {
    pub(in crate::view::panes::main) old_range: Range<usize>,
    pub(in crate::view::panes::main) new_range: Range<usize>,
}

pub(in crate::view::panes::main) fn resolved_outline_delta_between_texts(
    old_text: &str,
    new_text: &str,
) -> Option<ResolvedOutlineDelta> {
    if old_text == new_text {
        return None;
    }

    let old = old_text.as_bytes();
    let new = new_text.as_bytes();
    let old_len = old.len();
    let new_len = new.len();

    let mut prefix = 0usize;
    let prefix_max = old_len.min(new_len);
    while prefix < prefix_max && old[prefix] == new[prefix] {
        prefix = prefix.saturating_add(1);
    }
    while prefix > 0 && (!old_text.is_char_boundary(prefix) || !new_text.is_char_boundary(prefix)) {
        prefix = prefix.saturating_sub(1);
    }

    let mut suffix = 0usize;
    while suffix < old_len.saturating_sub(prefix)
        && suffix < new_len.saturating_sub(prefix)
        && old[old_len.saturating_sub(1 + suffix)] == new[new_len.saturating_sub(1 + suffix)]
    {
        suffix = suffix.saturating_add(1);
    }
    while suffix > 0
        && (!old_text.is_char_boundary(old_len.saturating_sub(suffix))
            || !new_text.is_char_boundary(new_len.saturating_sub(suffix)))
    {
        suffix = suffix.saturating_sub(1);
    }

    Some(ResolvedOutlineDelta {
        old_range: prefix..old_len.saturating_sub(suffix),
        new_range: prefix..new_len.saturating_sub(suffix),
    })
}

pub(in crate::view::panes::main) fn resolved_outline_delta_for_snapshot_transition(
    old_snapshot: &TextModelSnapshot,
    new_snapshot: &TextModelSnapshot,
    recent_edit_delta: Option<(Range<usize>, Range<usize>)>,
) -> Option<ResolvedOutlineDelta> {
    if old_snapshot.model_id() == new_snapshot.model_id()
        && new_snapshot.revision() == old_snapshot.revision().saturating_add(1)
        && let Some((old_range, new_range)) = recent_edit_delta
    {
        return Some(ResolvedOutlineDelta {
            old_range,
            new_range,
        });
    }

    // Do not materialize and compare both documents on the immediate input
    // notification path. If observer delivery coalesced several revisions, the
    // surviving debounced task will perform the full outline recompute instead.
    None
}

pub(in crate::view::panes::main) fn remap_resolved_output_conflict_block_ranges_for_delta(
    old_block_ranges: &[Range<usize>],
    old_range: Range<usize>,
    new_range: Range<usize>,
    new_line_count: usize,
) -> Vec<Range<usize>> {
    let line_delta = new_range.len() as isize - old_range.len() as isize;
    old_block_ranges
        .iter()
        .map(|range| {
            let remapped = if range.end <= old_range.start {
                range.clone()
            } else if range.start >= old_range.end {
                shifted_line_index(range.start, line_delta)
                    ..shifted_line_index(range.end, line_delta)
            } else {
                let start = if old_range.start <= range.start {
                    new_range.start
                } else {
                    range.start
                };
                let end = if range.end <= old_range.end {
                    new_range.end
                } else {
                    shifted_line_index(range.end, line_delta)
                };
                start..end
            };
            remapped.start.min(new_line_count)..remapped.end.min(new_line_count)
        })
        .map(|range| range.start..range.end.max(range.start))
        .collect()
}

pub(in crate::view::panes::main) fn resolved_output_conflict_block_ranges_in_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
) -> Option<Vec<Range<usize>>> {
    fn is_line_boundary(
        text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
        byte_ix: usize,
    ) -> bool {
        if byte_ix == 0 || byte_ix == text.len() {
            return true;
        }
        text.byte_at(byte_ix.saturating_sub(1))
            .is_some_and(|b| b == b'\n')
    }

    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    let mut line_offset = 0usize;
    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                if !output_text.starts_with_at(cursor, text.as_str()) {
                    return None;
                }
                cursor = cursor.saturating_add(text.len());
                line_offset = line_offset.saturating_add(count_newlines(text));
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let expected = conflict_resolver::generate_resolved_text(&[
                    conflict_resolver::ConflictSegment::Block(block.clone()),
                ]);
                if !output_text.starts_with_at(cursor, &expected) {
                    return None;
                }
                let end = cursor.saturating_add(expected.len());
                if end < cursor
                    || !is_line_boundary(output_text, cursor)
                    || !is_line_boundary(output_text, end)
                {
                    return None;
                }
                let start_line = line_offset;
                let mut end_line = line_offset.saturating_add(count_newlines(&expected));
                // A block that ends the file without a trailing newline still
                // occupies its last line, which no newline accounts for. Only
                // that case needs the extra row: when the block *is* newline
                // terminated, the outline still keeps an empty row after the
                // final newline (`resolved_output_outline_line_count`), and
                // claiming it would put this block's `?` gutter and conflict
                // bracket on a row that belongs to no conflict.
                if end == output_text.len() && !expected.is_empty() && !expected.ends_with('\n') {
                    end_line = end_line.saturating_add(1);
                }
                ranges.push(start_line..end_line);
                line_offset = line_offset.saturating_add(count_newlines(&expected));
                cursor = end;
            }
        }
    }

    Some(ranges)
}

/// Line ranges for the displayed conflict blocks, tolerating manual edits.
///
/// The walk above only reports ranges while the buffer still reads back exactly
/// as the segments render, so one keystroke anywhere in the output drops every
/// marker at once — placeholders lose their conflict color, their bracket and
/// their chunk menu. `ResolvedOutputBlockMap` carries block byte ownership
/// through edits, so fall back to it and convert its ranges into line space.
pub(in crate::view::panes::main) fn resolved_output_conflict_block_line_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Option<Vec<Range<usize>>> {
    resolved_output_conflict_block_ranges_in_text(marker_segments, output_text).or_else(|| {
        conflict_block_line_ranges_from_block_map(marker_segments, output_text, block_map)
    })
}

fn conflict_block_line_ranges_from_block_map(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Option<Vec<Range<usize>>> {
    if !block_map.is_valid_for(marker_segments, output_text) {
        return None;
    }

    let byte_ranges = block_map.ranges();
    let mut line_ranges = Vec::with_capacity(byte_ranges.len());
    // The map keeps its ranges sorted and disjoint, so one forward pass counts
    // every newline exactly once instead of rescanning the prefix per block.
    let mut cursor = 0usize;
    let mut line = 0usize;
    for range in byte_ranges {
        let start_line = line.saturating_add(output_text.count_newlines_in(cursor..range.start));
        let body_newlines = output_text.count_newlines_in(range.clone());
        let mut end_line = start_line.saturating_add(body_newlines);
        // A block that ends the file without a trailing newline still occupies
        // its last row, which no newline accounts for — matching the strict
        // walk. The carried `line` below must not include this adjustment: it
        // counts newlines actually seen, and the next block's start is measured
        // from those.
        let body_is_empty = range.start == range.end;
        let body_ends_with_newline = range
            .end
            .checked_sub(1)
            .and_then(|last| output_text.byte_at(last))
            .is_some_and(|byte| byte == b'\n');
        if range.end == output_text.len() && !body_is_empty && !body_ends_with_newline {
            end_line = end_line.saturating_add(1);
        }
        line_ranges.push(start_line..end_line);
        line = start_line.saturating_add(body_newlines);
        cursor = range.end;
    }

    Some(line_ranges)
}

pub(in crate::view::panes::main) fn conflict_marker_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
    line_range: Range<usize>,
) -> Vec<Range<usize>> {
    if !block.resolved && block.choice.is_empty() {
        return vec![line_range];
    }

    let mut marker_ranges = Vec::new();
    if !block.resolved
        && let Some(relative_subranges) = unresolved_decision_ranges_for_block(block)
            .or_else(|| unresolved_subchunk_conflict_ranges_for_block(block))
    {
        for relative in relative_subranges {
            let start = line_range
                .start
                .saturating_add(relative.start)
                .min(line_range.end);
            let end = line_range
                .start
                .saturating_add(relative.end)
                .min(line_range.end);
            marker_ranges.push(start..end);
        }
    }
    if marker_ranges.is_empty() {
        marker_ranges.push(line_range);
    }
    marker_ranges
}

pub(in crate::view::panes::main) fn write_conflict_markers_for_ranges(
    markers: &mut [Option<ResolvedOutputConflictMarker>],
    conflict_ix: usize,
    unresolved: bool,
    marker_ranges: &[Range<usize>],
) {
    let output_line_count = markers.len();
    if output_line_count == 0 {
        return;
    }

    for marker_range in marker_ranges {
        if marker_range.start < marker_range.end {
            let end = marker_range.end.min(output_line_count);
            for (line_ix, marker_slot) in markers
                .iter_mut()
                .enumerate()
                .take(end)
                .skip(marker_range.start)
            {
                *marker_slot = Some(ResolvedOutputConflictMarker {
                    conflict_ix,
                    range_start: marker_range.start,
                    range_end: marker_range.end,
                    is_start: line_ix == marker_range.start,
                    is_end: line_ix + 1 == marker_range.end,
                    unresolved,
                });
            }
            continue;
        }

        let anchor = marker_range.start.min(output_line_count.saturating_sub(1));
        markers[anchor] = Some(ResolvedOutputConflictMarker {
            conflict_ix,
            range_start: marker_range.start,
            range_end: marker_range.end,
            is_start: true,
            is_end: true,
            unresolved,
        });
    }
}

pub(in crate::view::panes::main) fn output_line_range_for_conflict_block_in_text(
    segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    conflict_ix: usize,
) -> Option<Range<usize>> {
    resolved_output_conflict_block_ranges_in_text(segments, output_text)
        .and_then(|ranges| ranges.get(conflict_ix).cloned())
}

pub(in crate::view::panes::main) fn conflict_fragment_text_for_choice(
    base: &str,
    ours: &str,
    theirs: &str,
    choice: conflict_resolver::ConflictChoice,
) -> String {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    let mut out = String::new();
    for source in choice.iter() {
        match source {
            ConflictOutputSource::Base => out.push_str(base),
            ConflictOutputSource::Ours => out.push_str(ours),
            ConflictOutputSource::Theirs => out.push_str(theirs),
        }
    }
    out
}

pub(in crate::view::panes::main) fn unresolved_subchunk_conflict_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    use gitcomet_core::conflict_session::Subchunk;

    let base = block.base.as_deref()?;
    let subchunks = gitcomet_core::conflict_session::split_conflict_into_subchunks(
        base,
        &block.ours,
        &block.theirs,
    )?;
    let mut ranges = Vec::new();
    let mut line_offset = 0usize;
    for subchunk in subchunks {
        let (fragment, is_conflict) = match subchunk {
            Subchunk::Resolved(text) => (text, false),
            Subchunk::Conflict { base, ours, theirs } => (
                conflict_fragment_text_for_choice(&base, &ours, &theirs, block.choice),
                true,
            ),
        };
        let start = line_offset;
        line_offset = line_offset.saturating_add(count_newlines(&fragment));
        if is_conflict {
            ranges.push(start..line_offset);
        }
    }
    if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

#[derive(Clone, Debug)]
pub(in crate::view::panes::main) struct UnresolvedDecisionRegion {
    pub(in crate::view::panes::main) row_range: Range<usize>,
    pub(in crate::view::panes::main) selected_line_range: Range<usize>,
    pub(in crate::view::panes::main) alternate_line_range: Range<usize>,
    pub(in crate::view::panes::main) has_non_emitting_rows: bool,
}

pub(in crate::view::panes::main) fn unresolved_decision_regions_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<UnresolvedDecisionRegion>> {
    let (left, right, choose_left) = match block.choice {
        conflict_resolver::ConflictChoice::Ours => (&block.ours, &block.theirs, true),
        conflict_resolver::ConflictChoice::Theirs => (&block.theirs, &block.ours, false),
        _ => return None,
    };
    let plan = gitcomet_core::file_diff::side_by_side_plan(left, right);
    if plan.row_count == 0 {
        return None;
    }
    let regions = gitcomet_core::file_diff::plan_row_region_anchors(&plan).region_anchors;
    if regions.is_empty() {
        return None;
    }
    let (old_prefix, new_prefix) = gitcomet_core::file_diff::plan_emitted_line_prefix_counts(&plan);
    let (selected_prefix, alternate_prefix) = if choose_left {
        (&old_prefix, &new_prefix)
    } else {
        (&new_prefix, &old_prefix)
    };

    let mut decision_regions: Vec<UnresolvedDecisionRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        let row_start = region.row_start.min(plan.row_count);
        let row_end = region.row_end_exclusive.min(plan.row_count).max(row_start);
        let selected_line_range = selected_prefix[row_start]..selected_prefix[row_end];
        let alternate_line_range = alternate_prefix[row_start]..alternate_prefix[row_end];
        let emitted_rows = selected_line_range
            .end
            .saturating_sub(selected_line_range.start);
        let has_non_emitting_rows = emitted_rows < row_end.saturating_sub(row_start);

        if let Some(last) = decision_regions.last_mut()
            && last.selected_line_range == selected_line_range
        {
            last.row_range.end = row_end;
            last.alternate_line_range.end =
                last.alternate_line_range.end.max(alternate_line_range.end);
            last.has_non_emitting_rows |= has_non_emitting_rows;
            continue;
        }

        decision_regions.push(UnresolvedDecisionRegion {
            row_range: row_start..row_end,
            selected_line_range,
            alternate_line_range,
            has_non_emitting_rows,
        });
    }
    if decision_regions.is_empty() {
        return None;
    }

    // Merge nearby non-zero ranges into one logical decision chunk while
    // preserving insertion anchors as independent picks.
    const MERGE_GAP_LINES: usize = 1;
    let mut merged: Vec<UnresolvedDecisionRegion> = Vec::with_capacity(decision_regions.len());
    for next in decision_regions {
        if let Some(prev) = merged.last_mut() {
            let prev_zero = prev.selected_line_range.start == prev.selected_line_range.end;
            let next_zero = next.selected_line_range.start == next.selected_line_range.end;
            let can_merge = if prev_zero || next_zero {
                prev_zero
                    && next_zero
                    && next.selected_line_range.start
                        <= prev.selected_line_range.end.saturating_add(MERGE_GAP_LINES)
            } else {
                // Keep ranges with insertion/deletion-only rows separate so
                // structural additions (e.g. trailing inserted methods) don't
                // collapse into preceding modification chunks.
                !prev.has_non_emitting_rows
                    && !next.has_non_emitting_rows
                    && next.selected_line_range.start
                        <= prev.selected_line_range.end.saturating_add(MERGE_GAP_LINES)
            };
            if can_merge {
                prev.row_range.end = next.row_range.end;
                prev.selected_line_range.end = prev
                    .selected_line_range
                    .end
                    .max(next.selected_line_range.end);
                prev.alternate_line_range.end = prev
                    .alternate_line_range
                    .end
                    .max(next.alternate_line_range.end);
                prev.has_non_emitting_rows |= next.has_non_emitting_rows;
                continue;
            }
        }
        merged.push(next);
    }

    Some(merged)
}

pub(in crate::view::panes::main) fn unresolved_decision_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    unresolved_decision_regions_for_block(block).map(|regions| {
        regions
            .into_iter()
            .map(|region| region.selected_line_range)
            .collect()
    })
}

pub(in crate::view::panes::main) fn build_resolved_output_conflict_markers(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    output_line_count: usize,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let Some(block_ranges) =
        resolved_output_conflict_block_line_ranges(marker_segments, output_text, block_map)
    else {
        return vec![None; output_line_count];
    };

    build_resolved_output_conflict_markers_from_ranges(
        marker_segments,
        block_ranges.as_slice(),
        output_line_count,
    )
}

pub(in crate::view::panes::main) fn build_resolved_output_conflict_markers_from_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_ranges: &[Range<usize>],
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    for (conflict_ix, (block, range)) in marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .zip(block_ranges.iter().cloned())
        .enumerate()
    {
        let marker_ranges = conflict_marker_ranges_for_block(block, range);
        write_conflict_markers_for_ranges(
            &mut markers,
            conflict_ix,
            !block.resolved,
            marker_ranges.as_slice(),
        );
    }

    markers
}

pub(in crate::view::panes::main) fn build_resolved_output_conflict_markers_from_block_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_ranges: &[Range<usize>],
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    for (conflict_ix, (block, range)) in marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .zip(block_ranges.iter().cloned())
        .enumerate()
    {
        write_conflict_markers_for_ranges(
            &mut markers,
            conflict_ix,
            !block.resolved,
            std::slice::from_ref(&range),
        );
    }

    markers
}

pub(in crate::view::panes::main) fn resolved_output_markers_for_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let output_line_count = output_text.row_count();
    build_resolved_output_conflict_markers(
        marker_segments,
        output_text,
        output_line_count,
        block_map,
    )
}

/// Byte ranges whose output rows are still unresolved, and the subset of them
/// owned by `active_conflict`. Derive these from the current segments instead of
/// the asynchronously refreshed outline so syntax styling never briefly wins
/// while outline metadata catches up.
/// Unresolved output rows and the conflict each belongs to.
pub(in crate::view) type UnresolvedRows = Arc<[(Range<usize>, usize)]>;

/// [`UnresolvedRows`] paired with the state they were scanned from.
pub(in crate::view) type CachedUnresolvedRows = (ResolvedOutputKey, UnresolvedRows);

/// What the unresolved rows actually depend on: the buffer *and* which blocks
/// are still open.
///
/// The revision alone is not enough. A pick can leave the output byte-identical
/// — choosing the side already displayed, or resolving a whitespace-only block —
/// so the buffer never bumps its revision while the answer changes. Keying on
/// the revision alone leaves the yellow wash painted on a block the user just
/// resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutputKey {
    pub(in crate::view) revision: ResolvedOutputSourceRevision,
    pub(in crate::view) resolution: u64,
    pub(in crate::view) block_map: u64,
}

impl ResolvedOutputKey {
    pub(in crate::view) fn new(
        snapshot: &TextModelSnapshot,
        marker_segments: &[conflict_resolver::ConflictSegment],
        block_map: &conflict_resolver::ResolvedOutputBlockMap,
    ) -> Self {
        Self {
            revision: ResolvedOutputSourceRevision::from_snapshot(snapshot),
            resolution: resolution_fingerprint(marker_segments),
            block_map: block_map_fingerprint(block_map),
        }
    }
}

/// O(conflicts) digest of the block map's byte ranges.
///
/// The rows fall back to the map for block geometry whenever the strict walk
/// fails — which is exactly once the user has edited the buffer. The map can be
/// rebuilt or reset without the text revision or any block's resolution moving,
/// and rows computed against the old geometry then land on the wrong lines.
fn block_map_fingerprint(block_map: &conflict_resolver::ResolvedOutputBlockMap) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    let ranges = block_map.ranges();
    ranges.len().hash(&mut hasher);
    for range in ranges {
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
    }
    hasher.finish()
}

/// O(conflicts) digest of which blocks are resolved. Not a hash of the text —
/// the revision already covers that.
fn resolution_fingerprint(marker_segments: &[conflict_resolver::ConflictSegment]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    for segment in marker_segments {
        match segment {
            conflict_resolver::ConflictSegment::Block(block) => {
                block.resolved.hash(&mut hasher);
                block.choice.hash(&mut hasher);
            }
            conflict_resolver::ConflictSegment::Text(_) => 0u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

/// Every still-unresolved output row, tagged with the conflict it belongs to.
///
/// Depends only on the *text*. Selecting a different conflict does not change
/// it, which is what lets the caller cache it across navigation.
///
/// Takes the rope rather than a materialized document plus a line-start array:
/// the rows wanted are the marker rows of unresolved blocks, and the rope
/// answers "byte range of row N" in O(log n), so this never has to build an
/// index proportional to the document.
pub(in crate::view::panes::main) fn resolved_output_unresolved_rows(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &crate::kit::rope::Rope,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> UnresolvedRows {
    if !marker_segments.iter().any(|segment| {
        matches!(segment, conflict_resolver::ConflictSegment::Block(block) if !block.resolved)
    }) {
        return Arc::default();
    }
    let Some(block_ranges) =
        resolved_output_conflict_block_line_ranges(marker_segments, output_text, block_map)
    else {
        return Arc::default();
    };

    // Walk the unresolved blocks rather than building a marker entry for every
    // row and filtering it. The per-line array is proportional to the document;
    // this is proportional to the conflicts, which is what the caller actually
    // asked about.
    let mut rows = Vec::new();
    for (conflict_ix, (block, line_range)) in marker_segments
        .iter()
        .filter_map(|segment| match segment {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            conflict_resolver::ConflictSegment::Text(_) => None,
        })
        .zip(block_ranges.iter().cloned())
        .enumerate()
    {
        if block.resolved {
            continue;
        }
        for marker_range in conflict_marker_ranges_for_block(block, line_range) {
            for line_ix in marker_range.start..marker_range.end {
                let Ok(row) = u32::try_from(line_ix) else {
                    continue;
                };
                if row >= output_text.line_count() {
                    continue;
                }
                let mut range = output_text.line_range(row);
                while range.end > range.start
                    && conflict_resolver::ResolvedOutputSource::byte_at(output_text, range.end - 1)
                        == Some(b'\r')
                {
                    range.end -= 1;
                }
                if !range.is_empty() {
                    rows.push((range, conflict_ix));
                }
            }
        }
    }
    rows.sort_by_key(|(range, _)| (range.start, range.end));
    rows.into()
}

/// Split cached rows into "every unresolved row" and "the selected conflict's".
///
/// O(unresolved rows), so moving the wash between conflicts costs nothing that
/// scales with the document.
pub(in crate::view::panes::main) fn resolved_output_unresolved_spans_for_active(
    rows: &[(Range<usize>, usize)],
    active_conflict: Option<usize>,
) -> ResolvedOutputUnresolvedSpans {
    let mut all = Vec::with_capacity(rows.len());
    let mut active = Vec::new();
    for (range, conflict_ix) in rows {
        if active_conflict == Some(*conflict_ix) {
            active.push(range.clone());
        }
        all.push(range.clone());
    }
    ResolvedOutputUnresolvedSpans {
        all: all.into(),
        active: active.into(),
    }
}

/// Scan and select in one call. Production splits the two so navigation can
/// reuse the scan; this stays for tests that only care about the result.
#[cfg(test)]
pub(in crate::view::panes::main) fn resolved_output_unresolved_byte_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
    active_conflict: Option<usize>,
) -> ResolvedOutputUnresolvedSpans {
    let rope = crate::kit::rope::Rope::from_text(output_text);
    let rows = resolved_output_unresolved_rows(marker_segments, &rope, block_map);
    resolved_output_unresolved_spans_for_active(rows.as_ref(), active_conflict)
}

/// Byte spans of the unresolved-conflict placeholder rows, terminator included.
///
/// A `<Merge Conflict>` row is a drawing of an open decision, not text the file
/// will ever contain, so the buffer refuses to edit these spans however the
/// rest of the output has been rewritten by hand. Rows are identified by their
/// own content, which keeps the protection standing even once the marker
/// segments no longer line up with the buffer.
pub(in crate::view::panes::main) fn resolved_output_placeholder_protected_ranges(
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
) -> Arc<[Range<usize>]> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    output_text.for_each_row_with_terminator(&mut |range, line| {
        if conflict_resolver::line_is_unresolved_conflict_placeholder(line) {
            ranges.push(range);
        }
    });
    ranges.into()
}

pub(in crate::view::panes::main) fn resolved_output_marker_for_line(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    output_line_ix: usize,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Option<ResolvedOutputConflictMarker> {
    resolved_output_markers_for_text(marker_segments, output_text, block_map)
        .get(output_line_ix)
        .copied()
        .flatten()
}

pub(in crate::view::panes::main) fn first_output_marker_line_for_conflict(
    markers: &[Option<ResolvedOutputConflictMarker>],
    conflict_ix: usize,
) -> Option<usize> {
    markers.iter().enumerate().find_map(|(line_ix, marker)| {
        marker
            .as_ref()
            .and_then(|m| (m.conflict_ix == conflict_ix && m.is_start).then_some(line_ix))
    })
}

#[cfg(test)]
pub(in crate::view::panes::main) fn conflict_marker_nav_entries_from_markers(
    markers: &[Option<ResolvedOutputConflictMarker>],
) -> Vec<usize> {
    let mut seen_conflicts = FxHashSet::default();
    markers
        .iter()
        .enumerate()
        .filter_map(|(line_ix, marker)| {
            marker.as_ref().and_then(|m| {
                (m.is_start && seen_conflicts.insert(m.conflict_ix)).then_some(line_ix)
            })
        })
        .collect()
}

pub(in crate::view::panes::main) fn conflict_resolver_output_context_line(
    content: &str,
    cursor_offset: usize,
    clicked_offset: Option<usize>,
) -> usize {
    clicked_offset
        .map(|offset| line_index_for_offset(content, offset))
        .unwrap_or_else(|| line_index_for_offset(content, cursor_offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_decision_regions_track_non_emitting_selected_rows() {
        let block = conflict_resolver::ConflictBlock {
            base: None,
            ours: "".into(),
            theirs: "added line\n".into(),
            choice: conflict_resolver::ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        };

        let regions =
            unresolved_decision_regions_for_block(&block).expect("expected one decision region");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].row_range, 0..1);
        assert_eq!(regions[0].selected_line_range, 0..0);
        assert_eq!(regions[0].alternate_line_range, 0..1);
        assert!(regions[0].has_non_emitting_rows);
    }
}
