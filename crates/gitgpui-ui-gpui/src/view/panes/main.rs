use super::super::path_display;
use super::super::*;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI32, Ordering};

mod diff_cache;
mod diff_search;
mod diff_text;
mod preview;

const CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS: u64 = 140;
const CONFLICT_RESOLVED_OUTLINE_AUTO_SYNTAX_MAX_LINES: usize = 4_000;
const CONFLICT_RESOLVED_OUTPUT_ROW_HEIGHT_PX: f32 = 20.0;
const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

/// Extract unique source lines from two-way diff rows for provenance matching.
///
/// Returns (old_lines, new_lines) as `Vec<SharedString>` suitable for `SourceLines`.
fn collect_two_way_source_lines(
    diff_rows: &[FileDiffRow],
) -> (Vec<SharedString>, Vec<SharedString>) {
    let mut old_lines = Vec::with_capacity(diff_rows.len());
    let mut new_lines = Vec::with_capacity(diff_rows.len());
    for row in diff_rows {
        if let Some(ref text) = row.old {
            old_lines.push(SharedString::from(text.clone()));
        }
        if let Some(ref text) = row.new {
            new_lines.push(SharedString::from(text.clone()));
        }
    }
    (old_lines, new_lines)
}

fn build_resolved_output_syntax_highlights(
    theme: AppTheme,
    lines: &[String],
    language: Option<rows::DiffSyntaxLanguage>,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let Some(language) = language else {
        return Vec::new();
    };
    let syntax_mode = if lines.len() <= CONFLICT_RESOLVED_OUTLINE_AUTO_SYNTAX_MAX_LINES {
        rows::DiffSyntaxMode::Auto
    } else {
        rows::DiffSyntaxMode::HeuristicOnly
    };

    let mut highlights = Vec::new();
    let mut line_offset = 0usize;
    for (line_ix, line) in lines.iter().enumerate() {
        for (range, style) in rows::syntax_highlights_for_line(theme, line, language, syntax_mode) {
            highlights.push((
                (line_offset + range.start)..(line_offset + range.end),
                style,
            ));
        }
        line_offset += line.len();
        if line_ix + 1 < lines.len() {
            line_offset += 1;
        }
    }
    highlights
}

fn split_text_lines_owned(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(|line| line.to_string()).collect()
    }
}

fn count_newlines(text: &str) -> usize {
    text.as_bytes().iter().filter(|&&b| b == b'\n').count()
}

fn source_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn output_line_range_for_conflict_block_in_text(
    segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    conflict_ix: usize,
) -> Option<Range<usize>> {
    fn is_line_boundary(text: &str, byte_ix: usize) -> bool {
        if byte_ix == 0 || byte_ix == text.len() {
            return true;
        }
        text.as_bytes()
            .get(byte_ix.saturating_sub(1))
            .is_some_and(|b| *b == b'\n')
    }

    let mut cursor = 0usize;
    let mut line_offset = 0usize;
    let mut block_ix = 0usize;

    for seg in segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let tail = output_text.get(cursor..)?;
                if !tail.starts_with(text) {
                    return None;
                }
                cursor = cursor.saturating_add(text.len());
                line_offset = line_offset.saturating_add(count_newlines(text));
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let expected = conflict_resolver::generate_resolved_text(&[
                    conflict_resolver::ConflictSegment::Block(block.clone()),
                ]);
                let tail = output_text.get(cursor..)?;
                if !tail.starts_with(&expected) {
                    return None;
                }
                let end = cursor.saturating_add(expected.len());
                if end < cursor
                    || !is_line_boundary(output_text, cursor)
                    || !is_line_boundary(output_text, end)
                {
                    return None;
                }

                let content = expected.as_str();
                let start_line = line_offset;
                let mut end_line = line_offset.saturating_add(count_newlines(content));
                if end == output_text.len() && !content.is_empty() {
                    end_line = end_line.saturating_add(1);
                }

                if block_ix == conflict_ix {
                    return Some(start_line..end_line);
                }

                line_offset = line_offset.saturating_add(count_newlines(content));
                cursor = end;
                block_ix = block_ix.saturating_add(1);
            }
        }
    }

    None
}

fn conflict_fragment_text_for_choice(
    base: &str,
    ours: &str,
    theirs: &str,
    choice: conflict_resolver::ConflictChoice,
) -> String {
    match choice {
        conflict_resolver::ConflictChoice::Base => base.to_string(),
        conflict_resolver::ConflictChoice::Ours => ours.to_string(),
        conflict_resolver::ConflictChoice::Theirs => theirs.to_string(),
        conflict_resolver::ConflictChoice::Both => {
            let mut out = String::with_capacity(ours.len().saturating_add(theirs.len()));
            out.push_str(ours);
            out.push_str(theirs);
            out
        }
    }
}

fn unresolved_subchunk_conflict_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    use gitgpui_core::conflict_session::Subchunk;

    let base = block.base.as_deref()?;
    let subchunks = gitgpui_core::conflict_session::split_conflict_into_subchunks(
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
struct UnresolvedDecisionRegion {
    row_range: Range<usize>,
    selected_line_range: Range<usize>,
    alternate_line_range: Range<usize>,
    has_non_emitting_rows: bool,
}

fn unresolved_decision_regions_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<UnresolvedDecisionRegion>> {
    let (left, right, choose_left) = match block.choice {
        conflict_resolver::ConflictChoice::Ours => (&block.ours, &block.theirs, true),
        conflict_resolver::ConflictChoice::Theirs => (&block.theirs, &block.ours, false),
        _ => return None,
    };
    let rows_with_anchors = gitgpui_core::file_diff::side_by_side_rows_with_anchors(left, right);
    let rows = rows_with_anchors.rows;
    let regions = rows_with_anchors.anchors.region_anchors;
    if rows.is_empty() || regions.is_empty() {
        return None;
    }

    let mut selected_prefix = Vec::with_capacity(rows.len().saturating_add(1));
    selected_prefix.push(0usize);
    let mut selected_count = 0usize;
    let mut alternate_prefix = Vec::with_capacity(rows.len().saturating_add(1));
    alternate_prefix.push(0usize);
    let mut alternate_count = 0usize;
    for row in &rows {
        let selected_emits = if choose_left {
            row.old.is_some()
        } else {
            row.new.is_some()
        };
        if selected_emits {
            selected_count = selected_count.saturating_add(1);
        }
        selected_prefix.push(selected_count);

        let alternate_emits = if choose_left {
            row.new.is_some()
        } else {
            row.old.is_some()
        };
        if alternate_emits {
            alternate_count = alternate_count.saturating_add(1);
        }
        alternate_prefix.push(alternate_count);
    }

    let mut decision_regions: Vec<UnresolvedDecisionRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        let row_start = region.row_start.min(rows.len());
        let row_end = region.row_end_exclusive.min(rows.len());
        let selected_line_range = selected_prefix[row_start]..selected_prefix[row_end];
        let alternate_line_range = alternate_prefix[row_start]..alternate_prefix[row_end];
        let has_non_emitting_rows = rows[row_start..row_end].iter().any(|row| {
            let emits = if choose_left {
                row.old.is_some()
            } else {
                row.new.is_some()
            };
            !emits
        });

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

fn unresolved_decision_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    unresolved_decision_regions_for_block(block).map(|regions| {
        regions
            .into_iter()
            .map(|region| region.selected_line_range)
            .collect()
    })
}

fn build_resolved_output_conflict_markers(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    let mut conflict_ix = 0usize;
    for seg in marker_segments {
        let conflict_resolver::ConflictSegment::Block(block) = seg else {
            continue;
        };
        let unresolved = !block.resolved;

        if let Some(range) =
            output_line_range_for_conflict_block_in_text(marker_segments, output_text, conflict_ix)
        {
            let mut marker_ranges: Vec<Range<usize>> = Vec::new();
            if unresolved
                && let Some(relative_subranges) = unresolved_decision_ranges_for_block(block)
                    .or_else(|| unresolved_subchunk_conflict_ranges_for_block(block))
            {
                for relative in relative_subranges {
                    let start = range.start.saturating_add(relative.start).min(range.end);
                    let end = range.start.saturating_add(relative.end).min(range.end);
                    marker_ranges.push(start..end);
                }
            }
            if marker_ranges.is_empty() {
                marker_ranges.push(range);
            }

            for marker_range in marker_ranges {
                if marker_range.start < marker_range.end {
                    let end = marker_range.end.min(output_line_count);
                    for line_ix in marker_range.start..end {
                        markers[line_ix] = Some(ResolvedOutputConflictMarker {
                            conflict_ix,
                            range_start: marker_range.start,
                            range_end: marker_range.end,
                            is_start: line_ix == marker_range.start,
                            is_end: line_ix + 1 == marker_range.end,
                            unresolved,
                        });
                    }
                } else {
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
        }
        conflict_ix = conflict_ix.saturating_add(1);
    }

    markers
}

fn push_conflict_text_segment(
    segments: &mut Vec<conflict_resolver::ConflictSegment>,
    text: String,
) {
    if text.is_empty() {
        return;
    }
    if let Some(conflict_resolver::ConflictSegment::Text(prev)) = segments.last_mut() {
        prev.push_str(&text);
        return;
    }
    segments.push(conflict_resolver::ConflictSegment::Text(text));
}

fn resolved_output_markers_for_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let output_line_count = conflict_resolver::split_output_lines_for_outline(output_text).len();
    build_resolved_output_conflict_markers(marker_segments, output_text, output_line_count)
}

fn resolved_output_marker_for_line(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    output_line_ix: usize,
) -> Option<ResolvedOutputConflictMarker> {
    resolved_output_markers_for_text(marker_segments, output_text)
        .get(output_line_ix)
        .copied()
        .flatten()
}

fn conflict_marker_nav_entries_from_markers(
    markers: &[Option<ResolvedOutputConflictMarker>],
) -> Vec<usize> {
    markers
        .iter()
        .enumerate()
        .filter_map(|(line_ix, marker)| marker.as_ref().and_then(|m| m.is_start.then_some(line_ix)))
        .collect()
}

fn line_index_for_offset(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count()
}

fn conflict_resolver_output_context_line(
    content: &str,
    cursor_offset: usize,
    clicked_offset: Option<usize>,
) -> usize {
    clicked_offset
        .map(|offset| line_index_for_offset(content, offset))
        .unwrap_or_else(|| line_index_for_offset(content, cursor_offset))
}

fn slice_text_by_line_range(text: &str, line_range: Range<usize>) -> String {
    if line_range.start >= line_range.end || text.is_empty() {
        return String::new();
    }

    let mut line_starts = Vec::with_capacity(count_newlines(text).saturating_add(2));
    line_starts.push(0usize);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(ix.saturating_add(1));
        }
    }

    let start_byte = line_starts
        .get(line_range.start)
        .copied()
        .unwrap_or(text.len());
    let end_byte = line_starts
        .get(line_range.end)
        .copied()
        .unwrap_or(text.len());
    if start_byte >= end_byte || start_byte >= text.len() {
        return String::new();
    }
    text[start_byte..end_byte.min(text.len())].to_string()
}

fn split_target_conflict_block_into_subchunks(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    target_conflict_ix: usize,
) -> bool {
    use gitgpui_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

    let Some(target_block) = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .nth(target_conflict_ix)
        .cloned()
    else {
        return false;
    };
    if target_block.resolved {
        return false;
    }

    enum SplitMode {
        Subchunks(Vec<Subchunk>),
        DecisionRanges(Vec<UnresolvedDecisionRegion>),
    }
    let split_mode = if let Some(base) = target_block.base.as_deref() {
        split_conflict_into_subchunks(base, &target_block.ours, &target_block.theirs).and_then(
            |subchunks| {
                let split_conflict_count = subchunks
                    .iter()
                    .filter(|subchunk| matches!(subchunk, Subchunk::Conflict { .. }))
                    .count();
                (split_conflict_count > 1).then_some(SplitMode::Subchunks(subchunks))
            },
        )
    } else {
        None
    }
    .or_else(|| {
        unresolved_decision_regions_for_block(&target_block)
            .and_then(|regions| (regions.len() > 1).then_some(SplitMode::DecisionRanges(regions)))
    });
    let Some(split_mode) = split_mode else {
        return false;
    };

    let mut next_segments = Vec::with_capacity(marker_segments.len().saturating_add(4));
    let mut next_region_indices =
        Vec::with_capacity(conflict_region_indices.len().saturating_add(4));
    let mut seen_conflict_ix = 0usize;
    for seg in marker_segments.drain(..) {
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                let region_ix = conflict_region_indices
                    .get(seen_conflict_ix)
                    .copied()
                    .unwrap_or(seen_conflict_ix);
                if seen_conflict_ix == target_conflict_ix {
                    match &split_mode {
                        SplitMode::Subchunks(subchunks) => {
                            for subchunk in subchunks {
                                match subchunk {
                                    Subchunk::Resolved(text) => {
                                        push_conflict_text_segment(
                                            &mut next_segments,
                                            text.clone(),
                                        );
                                    }
                                    Subchunk::Conflict { base, ours, theirs } => {
                                        next_segments.push(
                                            conflict_resolver::ConflictSegment::Block(
                                                conflict_resolver::ConflictBlock {
                                                    base: Some(base.clone()),
                                                    ours: ours.clone(),
                                                    theirs: theirs.clone(),
                                                    choice: target_block.choice,
                                                    resolved: false,
                                                },
                                            ),
                                        );
                                        next_region_indices.push(region_ix);
                                    }
                                }
                            }
                        }
                        SplitMode::DecisionRanges(regions) => {
                            let (selected_text, alternate_text, choice_is_ours) =
                                match target_block.choice {
                                    conflict_resolver::ConflictChoice::Ours => {
                                        (&target_block.ours, &target_block.theirs, true)
                                    }
                                    conflict_resolver::ConflictChoice::Theirs => {
                                        (&target_block.theirs, &target_block.ours, false)
                                    }
                                    _ => {
                                        return false;
                                    }
                                };
                            let selected_total_lines = source_line_count(selected_text);
                            let mut selected_cursor = 0usize;
                            for region in regions {
                                let prefix = slice_text_by_line_range(
                                    selected_text,
                                    selected_cursor..region.selected_line_range.start,
                                );
                                push_conflict_text_segment(&mut next_segments, prefix);

                                let selected_fragment = slice_text_by_line_range(
                                    selected_text,
                                    region.selected_line_range.clone(),
                                );
                                let alternate_fragment = slice_text_by_line_range(
                                    alternate_text,
                                    region.alternate_line_range.clone(),
                                );
                                let (ours, theirs) = if choice_is_ours {
                                    (selected_fragment, alternate_fragment)
                                } else {
                                    (alternate_fragment, selected_fragment)
                                };
                                next_segments.push(conflict_resolver::ConflictSegment::Block(
                                    conflict_resolver::ConflictBlock {
                                        base: None,
                                        ours,
                                        theirs,
                                        choice: target_block.choice,
                                        resolved: false,
                                    },
                                ));
                                next_region_indices.push(region_ix);
                                selected_cursor = region.selected_line_range.end;
                            }
                            let suffix = slice_text_by_line_range(
                                selected_text,
                                selected_cursor..selected_total_lines,
                            );
                            push_conflict_text_segment(&mut next_segments, suffix);
                        }
                    }
                } else {
                    next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                    next_region_indices.push(region_ix);
                }
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }

    *marker_segments = next_segments;
    *conflict_region_indices = next_region_indices;
    true
}

fn conflict_region_index_is_unique(conflict_region_indices: &[usize], region_ix: usize) -> bool {
    conflict_region_indices
        .iter()
        .filter(|&&ix| ix == region_ix)
        .take(2)
        .count()
        <= 1
}

fn conflict_block_matches_group(
    block: &conflict_resolver::ConflictBlock,
    region_ix: usize,
    target_block: &conflict_resolver::ConflictBlock,
    target_region_ix: usize,
) -> bool {
    region_ix == target_region_ix
        && block.base == target_block.base
        && block.ours == target_block.ours
        && block.theirs == target_block.theirs
}

fn conflict_group_member_indices_for_ix(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> Vec<usize> {
    let mut blocks: Vec<&conflict_resolver::ConflictBlock> = Vec::new();
    // True when a block has non-empty text between it and the previous block.
    let mut separated_before: Vec<bool> = Vec::new();
    let mut saw_text_since_prev_block = false;
    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                if !text.is_empty() {
                    saw_text_since_prev_block = true;
                }
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                separated_before.push(saw_text_since_prev_block);
                blocks.push(block);
                saw_text_since_prev_block = false;
            }
        }
    }
    let Some(target_block) = blocks.get(conflict_ix).copied() else {
        return Vec::new();
    };
    let target_region_ix = conflict_region_indices
        .get(conflict_ix)
        .copied()
        .unwrap_or(conflict_ix);

    let mut start = conflict_ix;
    while start > 0 {
        if separated_before[start] {
            break;
        }
        let prev_ix = start - 1;
        let prev_block = blocks[prev_ix];
        let prev_region_ix = conflict_region_indices
            .get(prev_ix)
            .copied()
            .unwrap_or(prev_ix);
        if conflict_block_matches_group(prev_block, prev_region_ix, target_block, target_region_ix)
        {
            start = prev_ix;
        } else {
            break;
        }
    }

    let mut end_exclusive = conflict_ix + 1;
    while end_exclusive < blocks.len() {
        let next_ix = end_exclusive;
        if separated_before[next_ix] {
            break;
        }
        let next_block = blocks[next_ix];
        let next_region_ix = conflict_region_indices
            .get(next_ix)
            .copied()
            .unwrap_or(next_ix);
        if conflict_block_matches_group(next_block, next_region_ix, target_block, target_region_ix)
        {
            end_exclusive += 1;
        } else {
            break;
        }
    }

    (start..end_exclusive).collect()
}

fn conflict_group_selected_choices_for_ix(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> Vec<conflict_resolver::ConflictChoice> {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    if group_indices.is_empty() {
        return Vec::new();
    }
    let blocks: Vec<&conflict_resolver::ConflictBlock> = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .collect();

    let mut has_base = false;
    let mut has_ours = false;
    let mut has_theirs = false;
    for ix in group_indices {
        let Some(block) = blocks.get(ix).copied() else {
            continue;
        };
        if !block.resolved {
            continue;
        }
        match block.choice {
            conflict_resolver::ConflictChoice::Base => has_base = true,
            conflict_resolver::ConflictChoice::Ours => has_ours = true,
            conflict_resolver::ConflictChoice::Theirs => has_theirs = true,
            conflict_resolver::ConflictChoice::Both => {
                has_ours = true;
                has_theirs = true;
            }
        }
    }

    let mut selected = Vec::with_capacity(3);
    if has_base {
        selected.push(conflict_resolver::ConflictChoice::Base);
    }
    if has_ours {
        selected.push(conflict_resolver::ConflictChoice::Ours);
    }
    if has_theirs {
        selected.push(conflict_resolver::ConflictChoice::Theirs);
    }
    selected
}

fn conflict_group_indices_for_choice(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> Vec<usize> {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    if group_indices.is_empty() {
        return Vec::new();
    }
    let blocks: Vec<&conflict_resolver::ConflictBlock> = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .collect();

    group_indices
        .into_iter()
        .filter(|&ix| {
            let Some(block) = blocks.get(ix).copied() else {
                return false;
            };
            if !block.resolved {
                return false;
            }
            match choice {
                conflict_resolver::ConflictChoice::Base => {
                    block.choice == conflict_resolver::ConflictChoice::Base
                }
                conflict_resolver::ConflictChoice::Ours => {
                    matches!(
                        block.choice,
                        conflict_resolver::ConflictChoice::Ours
                            | conflict_resolver::ConflictChoice::Both
                    )
                }
                conflict_resolver::ConflictChoice::Theirs => {
                    matches!(
                        block.choice,
                        conflict_resolver::ConflictChoice::Theirs
                            | conflict_resolver::ConflictChoice::Both
                    )
                }
                conflict_resolver::ConflictChoice::Both => {
                    block.choice == conflict_resolver::ConflictChoice::Both
                }
            }
        })
        .collect()
}

fn should_remove_conflict_block_on_reset(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> bool {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    group_indices.len() > 1
}

fn remove_conflict_block_at(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
) -> bool {
    let mut next_segments = Vec::with_capacity(marker_segments.len());
    let mut seen_conflict_ix = 0usize;
    let mut removed = false;
    for seg in marker_segments.drain(..) {
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                if seen_conflict_ix == conflict_ix {
                    removed = true;
                } else {
                    next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                }
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }
    *marker_segments = next_segments;
    if removed && conflict_ix < conflict_region_indices.len() {
        conflict_region_indices.remove(conflict_ix);
    }
    removed
}

fn reset_conflict_block_selection(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
) -> bool {
    if should_remove_conflict_block_on_reset(marker_segments, conflict_region_indices, conflict_ix)
    {
        return remove_conflict_block_at(marker_segments, conflict_region_indices, conflict_ix);
    }

    let mut seen_conflict_ix = 0usize;
    for seg in marker_segments.iter_mut() {
        let conflict_resolver::ConflictSegment::Block(block) = seg else {
            continue;
        };
        if seen_conflict_ix == conflict_ix {
            if !block.resolved {
                return false;
            }
            block.resolved = false;
            // Unpicked state should return to the default local-side choice.
            block.choice = conflict_resolver::ConflictChoice::Ours;
            return true;
        }
        seen_conflict_ix = seen_conflict_ix.saturating_add(1);
    }
    false
}

fn append_choice_after_conflict_block(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> Option<usize> {
    let target_block = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .nth(conflict_ix)?
        .clone();
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    let Some(&group_end_ix) = group_indices.last() else {
        return None;
    };
    let target_region_ix = conflict_region_indices
        .get(conflict_ix)
        .copied()
        .unwrap_or(conflict_ix);
    if !target_block.resolved {
        return None;
    }
    if matches!(choice, conflict_resolver::ConflictChoice::Base) && target_block.base.is_none() {
        return None;
    }
    if conflict_group_selected_choices_for_ix(marker_segments, conflict_region_indices, conflict_ix)
        .contains(&choice)
    {
        return None;
    }

    let mut next_segments = Vec::with_capacity(marker_segments.len().saturating_add(1));
    let mut next_region_indices =
        Vec::with_capacity(conflict_region_indices.len().saturating_add(1));
    let mut seen_conflict_ix = 0usize;
    let mut next_conflict_ix = 0usize;
    let mut inserted_conflict_ix = None;

    let push_appended = |next_segments: &mut Vec<conflict_resolver::ConflictSegment>,
                         next_region_indices: &mut Vec<usize>,
                         next_conflict_ix: &mut usize,
                         inserted_conflict_ix: &mut Option<usize>| {
        if inserted_conflict_ix.is_some() {
            return;
        }
        let mut appended = target_block.clone();
        appended.choice = choice;
        appended.resolved = true;
        next_segments.push(conflict_resolver::ConflictSegment::Block(appended));
        next_region_indices.push(target_region_ix);
        *inserted_conflict_ix = Some(*next_conflict_ix);
        *next_conflict_ix = next_conflict_ix.saturating_add(1);
    };

    for seg in marker_segments.drain(..) {
        if seen_conflict_ix == group_end_ix.saturating_add(1) {
            push_appended(
                &mut next_segments,
                &mut next_region_indices,
                &mut next_conflict_ix,
                &mut inserted_conflict_ix,
            );
        }
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                let region_ix = conflict_region_indices
                    .get(seen_conflict_ix)
                    .copied()
                    .unwrap_or(seen_conflict_ix);
                next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                next_region_indices.push(region_ix);
                next_conflict_ix = next_conflict_ix.saturating_add(1);
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }
    push_appended(
        &mut next_segments,
        &mut next_region_indices,
        &mut next_conflict_ix,
        &mut inserted_conflict_ix,
    );

    *marker_segments = next_segments;
    *conflict_region_indices = next_region_indices;
    inserted_conflict_ix
}

fn scroll_conflict_resolved_output_to_line(
    scroll_handle: &UniformListScrollHandle,
    target_line_ix: usize,
    line_count: usize,
) {
    if line_count == 0 {
        return;
    }

    let base_handle = scroll_handle.0.borrow().base_handle.clone();
    let viewport_h = base_handle.bounds().size.height.max(px(0.0));
    if viewport_h <= px(0.0) {
        return;
    }

    let line_h = px(CONFLICT_RESOLVED_OUTPUT_ROW_HEIGHT_PX);
    let total_h = line_h * line_count as f32;
    let max_scroll = (total_h - viewport_h).max(px(0.0));
    let target_line = target_line_ix.min(line_count.saturating_sub(1));
    let target_center = line_h * target_line as f32 + line_h * 0.5;
    let target_scroll_top = (target_center - viewport_h * 0.5)
        .max(px(0.0))
        .min(max_scroll);
    let current = base_handle.offset();
    base_handle.set_offset(point(current.x, -target_scroll_top));
}

#[allow(dead_code)]
fn apply_three_way_empty_base_provenance_hints(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
) {
    let generated = conflict_resolver::generate_resolved_text(marker_segments);
    if generated != output_text || meta.is_empty() {
        return;
    }

    let mut block_ix = 0usize;
    let mut a_line = 1u32;
    let mut b_line = 1u32;
    let mut c_line = 1u32;

    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let n = u32::try_from(source_line_count(text)).unwrap_or(0);
                a_line = a_line.saturating_add(n);
                b_line = b_line.saturating_add(n);
                c_line = c_line.saturating_add(n);
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let a_count =
                    u32::try_from(source_line_count(block.base.as_deref().unwrap_or_default()))
                        .unwrap_or(0);
                let b_count = u32::try_from(source_line_count(&block.ours)).unwrap_or(0);
                let c_count = u32::try_from(source_line_count(&block.theirs)).unwrap_or(0);

                let base_empty = block.base.as_ref().map_or(true, |s| s.is_empty());
                if base_empty
                    && let Some(range) = output_line_range_for_conflict_block_in_text(
                        marker_segments,
                        output_text,
                        block_ix,
                    )
                {
                    match block.choice {
                        conflict_resolver::ConflictChoice::Base => {}
                        conflict_resolver::ConflictChoice::Ours => {
                            let take = usize::min(
                                range.end.saturating_sub(range.start),
                                usize::try_from(b_count).unwrap_or(0),
                            );
                            for off in 0..take {
                                if let Some(m) = meta.get_mut(range.start + off)
                                    && matches!(
                                        m.source,
                                        conflict_resolver::ResolvedLineSource::A
                                            | conflict_resolver::ResolvedLineSource::Manual
                                    )
                                {
                                    m.source = conflict_resolver::ResolvedLineSource::B;
                                    m.input_line = Some(
                                        b_line.saturating_add(u32::try_from(off).unwrap_or(0)),
                                    );
                                }
                            }
                        }
                        conflict_resolver::ConflictChoice::Theirs => {
                            let take = usize::min(
                                range.end.saturating_sub(range.start),
                                usize::try_from(c_count).unwrap_or(0),
                            );
                            for off in 0..take {
                                if let Some(m) = meta.get_mut(range.start + off)
                                    && matches!(
                                        m.source,
                                        conflict_resolver::ResolvedLineSource::A
                                            | conflict_resolver::ResolvedLineSource::Manual
                                    )
                                {
                                    m.source = conflict_resolver::ResolvedLineSource::C;
                                    m.input_line = Some(
                                        c_line.saturating_add(u32::try_from(off).unwrap_or(0)),
                                    );
                                }
                            }
                        }
                        conflict_resolver::ConflictChoice::Both => {
                            let total = range.end.saturating_sub(range.start);
                            let ours_take =
                                usize::min(total, usize::try_from(b_count).unwrap_or(0));
                            for off in 0..ours_take {
                                if let Some(m) = meta.get_mut(range.start + off)
                                    && matches!(
                                        m.source,
                                        conflict_resolver::ResolvedLineSource::A
                                            | conflict_resolver::ResolvedLineSource::Manual
                                    )
                                {
                                    m.source = conflict_resolver::ResolvedLineSource::B;
                                    m.input_line = Some(
                                        b_line.saturating_add(u32::try_from(off).unwrap_or(0)),
                                    );
                                }
                            }

                            let theirs_take = total.saturating_sub(ours_take);
                            for off in 0..theirs_take {
                                if let Some(m) = meta.get_mut(range.start + ours_take + off)
                                    && matches!(
                                        m.source,
                                        conflict_resolver::ResolvedLineSource::A
                                            | conflict_resolver::ResolvedLineSource::Manual
                                    )
                                {
                                    m.source = conflict_resolver::ResolvedLineSource::C;
                                    m.input_line = Some(
                                        c_line.saturating_add(u32::try_from(off).unwrap_or(0)),
                                    );
                                }
                            }
                        }
                    }
                }

                a_line = a_line.saturating_add(a_count);
                b_line = b_line.saturating_add(b_count);
                c_line = c_line.saturating_add(c_count);
                block_ix = block_ix.saturating_add(1);
            }
        }
    }
}

fn apply_conflict_choice_provenance_hints(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    view_mode: ConflictResolverViewMode,
) {
    let generated = conflict_resolver::generate_resolved_text(marker_segments);
    if generated != output_text || meta.is_empty() {
        return;
    }

    let assign_range = |meta: &mut [conflict_resolver::ResolvedLineMeta],
                        range: Range<usize>,
                        source: conflict_resolver::ResolvedLineSource,
                        start_line: u32,
                        line_count: u32| {
        let len = range.end.saturating_sub(range.start);
        for off in 0..len {
            if let Some(m) = meta.get_mut(range.start + off) {
                m.source = source;
                let off_u32 = u32::try_from(off).unwrap_or(u32::MAX);
                m.input_line = (off_u32 < line_count).then_some(start_line.saturating_add(off_u32));
            }
        }
    };

    let assign_both_range = |meta: &mut [conflict_resolver::ResolvedLineMeta],
                             range: Range<usize>,
                             first_source: conflict_resolver::ResolvedLineSource,
                             first_start: u32,
                             first_count: u32,
                             second_source: conflict_resolver::ResolvedLineSource,
                             second_start: u32,
                             second_count: u32| {
        let len = range.end.saturating_sub(range.start);
        let first_count_usize = usize::try_from(first_count).unwrap_or(0);
        let first_take = len.min(first_count_usize);
        assign_range(
            meta,
            range.start..range.start.saturating_add(first_take),
            first_source,
            first_start,
            first_count,
        );
        assign_range(
            meta,
            range.start.saturating_add(first_take)..range.end,
            second_source,
            second_start,
            second_count,
        );
    };

    let mut block_ix = 0usize;
    let mut a_line = 1u32;
    let mut b_line = 1u32;
    let mut c_line = 1u32;

    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let n = u32::try_from(source_line_count(text)).unwrap_or(0);
                a_line = a_line.saturating_add(n);
                b_line = b_line.saturating_add(n);
                if view_mode == ConflictResolverViewMode::ThreeWay {
                    c_line = c_line.saturating_add(n);
                }
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let (a_count, b_count, c_count) = match view_mode {
                    ConflictResolverViewMode::ThreeWay => (
                        u32::try_from(source_line_count(block.base.as_deref().unwrap_or_default()))
                            .unwrap_or(0),
                        u32::try_from(source_line_count(&block.ours)).unwrap_or(0),
                        u32::try_from(source_line_count(&block.theirs)).unwrap_or(0),
                    ),
                    ConflictResolverViewMode::TwoWayDiff => (
                        u32::try_from(source_line_count(&block.ours)).unwrap_or(0),
                        u32::try_from(source_line_count(&block.theirs)).unwrap_or(0),
                        0,
                    ),
                };

                if let Some(range) = output_line_range_for_conflict_block_in_text(
                    marker_segments,
                    output_text,
                    block_ix,
                ) {
                    match (view_mode, block.choice) {
                        (
                            ConflictResolverViewMode::ThreeWay,
                            conflict_resolver::ConflictChoice::Base,
                        ) => {
                            assign_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::A,
                                a_line,
                                a_count,
                            );
                        }
                        (
                            ConflictResolverViewMode::ThreeWay,
                            conflict_resolver::ConflictChoice::Ours,
                        ) => {
                            assign_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::B,
                                b_line,
                                b_count,
                            );
                        }
                        (
                            ConflictResolverViewMode::ThreeWay,
                            conflict_resolver::ConflictChoice::Theirs,
                        ) => {
                            assign_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::C,
                                c_line,
                                c_count,
                            );
                        }
                        (
                            ConflictResolverViewMode::ThreeWay,
                            conflict_resolver::ConflictChoice::Both,
                        ) => {
                            assign_both_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::B,
                                b_line,
                                b_count,
                                conflict_resolver::ResolvedLineSource::C,
                                c_line,
                                c_count,
                            );
                        }
                        (
                            ConflictResolverViewMode::TwoWayDiff,
                            conflict_resolver::ConflictChoice::Theirs,
                        ) => {
                            assign_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::B,
                                b_line,
                                b_count,
                            );
                        }
                        (
                            ConflictResolverViewMode::TwoWayDiff,
                            conflict_resolver::ConflictChoice::Both,
                        ) => {
                            assign_both_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::A,
                                a_line,
                                a_count,
                                conflict_resolver::ResolvedLineSource::B,
                                b_line,
                                b_count,
                            );
                        }
                        // In two-way mode, Base falls back to local-side semantics.
                        (
                            ConflictResolverViewMode::TwoWayDiff,
                            conflict_resolver::ConflictChoice::Base,
                        )
                        | (
                            ConflictResolverViewMode::TwoWayDiff,
                            conflict_resolver::ConflictChoice::Ours,
                        ) => {
                            assign_range(
                                meta,
                                range,
                                conflict_resolver::ResolvedLineSource::A,
                                a_line,
                                a_count,
                            );
                        }
                    }
                }

                a_line = a_line.saturating_add(a_count);
                b_line = b_line.saturating_add(b_count);
                c_line = c_line.saturating_add(c_count);
                block_ix = block_ix.saturating_add(1);
            }
        }
    }
}

fn replacement_lines_for_conflict_block(
    block: &conflict_resolver::ConflictBlock,
    choice: conflict_resolver::ConflictChoice,
) -> Option<Vec<String>> {
    match choice {
        conflict_resolver::ConflictChoice::Base => {
            Some(split_text_lines_owned(block.base.as_deref()?))
        }
        conflict_resolver::ConflictChoice::Ours => Some(split_text_lines_owned(&block.ours)),
        conflict_resolver::ConflictChoice::Theirs => Some(split_text_lines_owned(&block.theirs)),
        conflict_resolver::ConflictChoice::Both => {
            let mut resolved_block = block.clone();
            resolved_block.choice = conflict_resolver::ConflictChoice::Both;
            resolved_block.resolved = true;
            let merged = conflict_resolver::generate_resolved_text(&[
                conflict_resolver::ConflictSegment::Block(resolved_block),
            ]);
            Some(split_text_lines_owned(&merged))
        }
    }
}

fn replace_output_lines_in_range(
    output: &str,
    range: Range<usize>,
    replacement_lines: &[String],
) -> String {
    let mut lines: Vec<String> = if output.is_empty() {
        Vec::new()
    } else {
        output.split('\n').map(|line| line.to_string()).collect()
    };
    let start = range.start.min(lines.len());
    let end = range.end.min(lines.len()).max(start);
    lines.splice(start..end, replacement_lines.iter().cloned());
    lines.join("\n")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClearDiffSelectionAction {
    ClearSelection,
    ExitFocusedMergetool,
}

fn clear_diff_selection_action(view_mode: GitGpuiViewMode) -> ClearDiffSelectionAction {
    match view_mode {
        GitGpuiViewMode::Normal => ClearDiffSelectionAction::ClearSelection,
        GitGpuiViewMode::FocusedMergetool => ClearDiffSelectionAction::ExitFocusedMergetool,
    }
}

fn focused_mergetool_save_exit_code(total_conflicts: usize, resolved_conflicts: usize) -> i32 {
    if total_conflicts == 0 || total_conflicts == resolved_conflicts {
        FOCUSED_MERGETOOL_EXIT_SUCCESS
    } else {
        FOCUSED_MERGETOOL_EXIT_CANCELED
    }
}

pub(in super::super) struct MainPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) view_mode: GitGpuiViewMode,
    pub(in super::super) focused_mergetool_labels: Option<FocusedMergetoolLabels>,
    pub(in super::super) focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) date_time_format: DateTimeFormat,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitGpuiView>,
    tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: u64,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,

    pub(in super::super) last_window_size: Size<Pixels>,

    pub(in super::super) show_whitespace: bool,
    pub(in super::super) conflict_enable_whitespace_autosolve: bool,
    pub(in super::super) conflict_enable_regex_autosolve: bool,
    pub(in super::super) conflict_enable_history_autosolve: bool,
    pub(in super::super) diff_view: DiffViewMode,
    pub(in super::super) svg_diff_view_mode: SvgDiffViewMode,
    pub(in super::super) diff_word_wrap: bool,
    pub(in super::super) diff_split_ratio: f32,
    pub(in super::super) diff_split_resize: Option<DiffSplitResizeState>,
    pub(in super::super) diff_split_last_synced_y: Pixels,
    pub(in super::super) diff_horizontal_min_width: Pixels,
    pub(in super::super) diff_cache_repo_id: Option<RepoId>,
    pub(in super::super) diff_cache_rev: u64,
    pub(in super::super) diff_cache_target: Option<DiffTarget>,
    pub(in super::super) diff_cache: Vec<AnnotatedDiffLine>,
    pub(in super::super) diff_file_for_src_ix: Vec<Option<Arc<str>>>,
    pub(in super::super) diff_language_for_src_ix: Vec<Option<rows::DiffSyntaxLanguage>>,
    pub(in super::super) diff_click_kinds: Vec<DiffClickKind>,
    pub(in super::super) diff_header_display_cache: HashMap<usize, SharedString>,
    pub(in super::super) diff_split_cache: Vec<PatchSplitRow>,
    pub(in super::super) diff_split_cache_len: usize,
    pub(in super::super) diff_panel_focus_handle: FocusHandle,
    pub(in super::super) diff_autoscroll_pending: bool,
    pub(in super::super) diff_raw_input: Entity<zed::TextInput>,
    pub(in super::super) diff_visible_indices: Vec<usize>,
    pub(in super::super) diff_visible_cache_len: usize,
    pub(in super::super) diff_visible_view: DiffViewMode,
    pub(in super::super) diff_visible_is_file_view: bool,
    pub(in super::super) diff_scrollbar_markers_cache: Vec<zed::ScrollbarMarker>,
    pub(in super::super) diff_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) diff_word_highlights_seq: u64,
    pub(in super::super) diff_word_highlights_inflight: Option<u64>,
    pub(in super::super) diff_file_stats: Vec<Option<(usize, usize)>>,
    pub(in super::super) diff_text_segments_cache: Vec<Option<CachedDiffStyledText>>,
    pub(in super::super) diff_selection_anchor: Option<usize>,
    pub(in super::super) diff_selection_range: Option<(usize, usize)>,
    pub(in super::super) diff_text_selecting: bool,
    pub(in super::super) diff_text_anchor: Option<DiffTextPos>,
    pub(in super::super) diff_text_head: Option<DiffTextPos>,
    diff_text_autoscroll_seq: u64,
    diff_text_autoscroll_target: Option<DiffTextAutoscrollTarget>,
    diff_text_last_mouse_pos: Point<Pixels>,
    pub(in super::super) diff_suppress_clicks_remaining: u8,
    pub(in super::super) diff_text_hitboxes: HashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    pub(in super::super) diff_text_layout_cache_epoch: u64,
    pub(in super::super) diff_text_layout_cache: HashMap<u64, DiffTextLayoutCacheEntry>,
    pub(in super::super) diff_hunk_picker_search_input: Option<Entity<zed::TextInput>>,
    pub(in super::super) diff_search_active: bool,
    pub(in super::super) diff_search_query: SharedString,
    pub(in super::super) diff_search_matches: Vec<usize>,
    pub(in super::super) diff_search_match_ix: Option<usize>,
    pub(in super::super) diff_search_input: Entity<zed::TextInput>,
    _diff_search_subscription: gpui::Subscription,

    pub(in super::super) file_diff_cache_repo_id: Option<RepoId>,
    pub(in super::super) file_diff_cache_rev: u64,
    pub(in super::super) file_diff_cache_target: Option<DiffTarget>,
    pub(in super::super) file_diff_cache_path: Option<std::path::PathBuf>,
    pub(in super::super) file_diff_cache_language: Option<rows::DiffSyntaxLanguage>,
    pub(in super::super) file_diff_cache_rows: Vec<FileDiffRow>,
    pub(in super::super) file_diff_inline_cache: Vec<AnnotatedDiffLine>,
    pub(in super::super) file_diff_inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) file_diff_split_word_highlights_old: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) file_diff_split_word_highlights_new: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) file_diff_cache_seq: u64,
    pub(in super::super) file_diff_cache_inflight: Option<u64>,

    pub(in super::super) file_image_diff_cache_repo_id: Option<RepoId>,
    pub(in super::super) file_image_diff_cache_rev: u64,
    pub(in super::super) file_image_diff_cache_target: Option<DiffTarget>,
    pub(in super::super) file_image_diff_cache_path: Option<std::path::PathBuf>,
    pub(in super::super) file_image_diff_cache_old: Option<Arc<gpui::Image>>,
    pub(in super::super) file_image_diff_cache_new: Option<Arc<gpui::Image>>,

    pub(in super::super) worktree_preview_path: Option<std::path::PathBuf>,
    pub(in super::super) worktree_preview: Loadable<Arc<Vec<String>>>,
    pub(in super::super) worktree_preview_segments_cache_path: Option<std::path::PathBuf>,
    pub(in super::super) worktree_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in super::super) worktree_preview_segments_cache: HashMap<usize, CachedDiffStyledText>,
    pub(in super::super) diff_preview_is_new_file: bool,
    pub(in super::super) diff_preview_new_file_lines: Arc<Vec<String>>,

    pub(in super::super) conflict_resolver_input: Entity<zed::TextInput>,
    _conflict_resolver_input_subscription: gpui::Subscription,
    pub(in super::super) conflict_resolver: ConflictResolverUiState,
    pub(in super::super) conflict_resolver_vsplit_ratio: f32,
    pub(in super::super) conflict_resolver_vsplit_resize: Option<ConflictVSplitResizeState>,
    pub(in super::super) conflict_three_way_col_ratios: [f32; 2],
    pub(in super::super) conflict_three_way_col_widths: [Pixels; 3],
    pub(in super::super) conflict_hsplit_resize: Option<ConflictHSplitResizeState>,
    pub(in super::super) conflict_diff_split_ratio: f32,
    pub(in super::super) conflict_diff_split_resize: Option<ConflictDiffSplitResizeState>,
    pub(in super::super) conflict_diff_split_col_widths: [Pixels; 2],
    pub(in super::super) conflict_diff_segments_cache_split:
        HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
    pub(in super::super) conflict_diff_segments_cache_inline: HashMap<usize, CachedDiffStyledText>,
    pub(in super::super) conflict_three_way_segments_cache:
        HashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    pub(in super::super) conflict_resolved_preview_path: Option<std::path::PathBuf>,
    pub(in super::super) conflict_resolved_preview_source_hash: Option<u64>,
    pub(in super::super) conflict_resolved_preview_syntax_language:
        Option<rows::DiffSyntaxLanguage>,
    pub(in super::super) conflict_resolved_preview_lines: Vec<String>,
    pub(in super::super) conflict_resolved_preview_segments_cache:
        HashMap<usize, CachedDiffStyledText>,

    pub(in super::super) history_view: Entity<super::HistoryView>,
    pub(in super::super) diff_scroll: UniformListScrollHandle,
    pub(in super::super) diff_split_right_scroll: UniformListScrollHandle,
    pub(in super::super) conflict_resolver_diff_scroll: UniformListScrollHandle,
    pub(in super::super) conflict_resolved_preview_scroll: UniformListScrollHandle,
    pub(in super::super) worktree_preview_scroll: UniformListScrollHandle,

    path_display_cache: std::cell::RefCell<HashMap<std::path::PathBuf, SharedString>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffTextAutoscrollTarget {
    DiffLeftOrInline,
    DiffSplitRight,
    WorktreePreview,
    ConflictResolvedPreview,
}

impl MainPaneView {
    fn notify_fingerprint_for(state: &AppState) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.diff_state_rev.hash(&mut hasher);
            repo.conflict_rev.hash(&mut hasher);

            // Only include status changes when viewing a working tree diff.
            let status_rev = if matches!(repo.diff_target, Some(DiffTarget::WorkingTree { .. })) {
                repo.status_rev
            } else {
                0
            };
            status_rev.hash(&mut hasher);
        }

        hasher.finish()
    }

    pub(in super::super) fn clear_diff_selection_or_exit(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        match clear_diff_selection_action(self.view_mode) {
            ClearDiffSelectionAction::ClearSelection => {
                self.store.dispatch(Msg::ClearDiffSelection { repo_id });
            }
            ClearDiffSelectionAction::ExitFocusedMergetool => {
                self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_CANCELED);
                cx.quit();
            }
        }
    }

    fn set_focused_mergetool_exit_code(&self, code: i32) {
        if let Some(exit_code) = &self.focused_mergetool_exit_code {
            exit_code.store(code, Ordering::SeqCst);
        }
    }

    fn focused_mergetool_labels_or_default(&self) -> FocusedMergetoolLabels {
        self.focused_mergetool_labels
            .clone()
            .unwrap_or(FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            })
    }

    pub(in super::super) fn focused_mergetool_save_and_exit(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        use gitgpui_core::conflict_output::{
            ConflictMarkerLabels, GenerateResolvedTextOptions, UnresolvedConflictMode,
        };

        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        };

        let labels = self.focused_mergetool_labels_or_default();
        let output = conflict_resolver::generate_resolved_text_with_options(
            &self.conflict_resolver.marker_segments,
            GenerateResolvedTextOptions {
                unresolved_mode: UnresolvedConflictMode::PreserveMarkers,
                labels: Some(ConflictMarkerLabels {
                    local: labels.local.as_str(),
                    remote: labels.remote.as_str(),
                    base: labels.base.as_str(),
                }),
            },
        );

        let full_path = repo.spec.workdir.join(&path);
        if let Some(parent) = full_path.parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!(
                "Failed to create parent directory for {}: {err}",
                full_path.display()
            );
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        }

        if let Err(err) = std::fs::write(&full_path, output.as_bytes()) {
            eprintln!(
                "Failed to write merged output to {}: {err}",
                full_path.display()
            );
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        }

        let total = conflict_resolver::conflict_count(&self.conflict_resolver.marker_segments);
        let resolved =
            conflict_resolver::resolved_conflict_count(&self.conflict_resolver.marker_segments);
        let exit_code = focused_mergetool_save_exit_code(total, resolved);
        self.set_focused_mergetool_exit_code(exit_code);
        cx.quit();
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        conflict_enable_whitespace_autosolve: bool,
        conflict_enable_regex_autosolve: bool,
        conflict_enable_history_autosolve: bool,
        view_mode: GitGpuiViewMode,
        focused_mergetool_labels: Option<FocusedMergetoolLabels>,
        focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
        root_view: WeakEntity<GitGpuiView>,
        tooltip_host: WeakEntity<TooltipHost>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.apply_state_snapshot(next, cx);
            cx.notify();
        });

        let diff_raw_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let conflict_resolver_input = cx.new(|cx| {
            let mut input = zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "Resolve file contents…".into(),
                    multiline: true,
                    read_only: false,
                    chromeless: true,
                    soft_wrap: false,
                },
                window,
                cx,
            );
            input.set_suppress_right_click(true);
            input.set_line_height(Some(px(20.0)), cx);
            input
        });

        let conflict_resolver_subscription =
            cx.observe(&conflict_resolver_input, |this, input, cx| {
                let output_text = input.read(cx).text().to_string();
                let mut output_hasher = std::collections::hash_map::DefaultHasher::new();
                output_text.hash(&mut output_hasher);
                let output_hash = output_hasher.finish();

                let path = this.conflict_resolver.path.clone();
                let needs_update = this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                    || this.conflict_resolved_preview_source_hash != Some(output_hash);
                if !needs_update {
                    return;
                }

                this.conflict_resolved_preview_path = path.clone();
                this.conflict_resolved_preview_source_hash = Some(output_hash);
                this.schedule_conflict_resolved_outline_recompute(path, output_hash, cx);
            });

        let diff_search_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "Search diff".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });
        let diff_search_subscription = cx.observe(&diff_search_input, |this, input, cx| {
            let next: SharedString = input.read(cx).text().to_string().into();
            if this.diff_search_query != next {
                this.diff_search_query = next;
                this.diff_text_segments_cache.clear();
                this.worktree_preview_segments_cache_path = None;
                this.worktree_preview_segments_cache.clear();
                this.conflict_diff_segments_cache_split.clear();
                this.conflict_diff_segments_cache_inline.clear();
                this.diff_search_recompute_matches();
                cx.notify();
            }
        });

        let diff_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);

        let last_window_size = window.window_bounds().get_bounds().size;
        let history_view = cx.new(|cx| {
            super::HistoryView::new(
                Arc::clone(&store),
                ui_model.clone(),
                theme,
                date_time_format,
                timezone,
                history_show_author,
                history_show_date,
                history_show_sha,
                root_view.clone(),
                tooltip_host.clone(),
                last_window_size,
                window,
                cx,
            )
        });

        let mut pane = Self {
            store,
            state,
            view_mode,
            focused_mergetool_labels,
            focused_mergetool_exit_code,
            theme,
            date_time_format,
            _ui_model_subscription: subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            last_window_size: size(px(0.0), px(0.0)),
            show_whitespace: false,
            conflict_enable_whitespace_autosolve,
            conflict_enable_regex_autosolve,
            conflict_enable_history_autosolve,
            diff_view: DiffViewMode::Split,
            svg_diff_view_mode: SvgDiffViewMode::Image,
            diff_word_wrap: false,
            diff_split_ratio: 0.5,
            diff_split_resize: None,
            diff_split_last_synced_y: px(0.0),
            diff_horizontal_min_width: px(0.0),
            diff_cache_repo_id: None,
            diff_cache_rev: 0,
            diff_cache_target: None,
            diff_cache: Vec::new(),
            diff_file_for_src_ix: Vec::new(),
            diff_language_for_src_ix: Vec::new(),
            diff_click_kinds: Vec::new(),
            diff_header_display_cache: HashMap::default(),
            diff_split_cache: Vec::new(),
            diff_split_cache_len: 0,
            diff_panel_focus_handle,
            diff_autoscroll_pending: false,
            diff_raw_input,
            diff_visible_indices: Vec::new(),
            diff_visible_cache_len: 0,
            diff_visible_view: DiffViewMode::Split,
            diff_visible_is_file_view: false,
            diff_scrollbar_markers_cache: Vec::new(),
            diff_word_highlights: Vec::new(),
            diff_word_highlights_seq: 0,
            diff_word_highlights_inflight: None,
            diff_file_stats: Vec::new(),
            diff_text_segments_cache: Vec::new(),
            diff_selection_anchor: None,
            diff_selection_range: None,
            diff_text_selecting: false,
            diff_text_anchor: None,
            diff_text_head: None,
            diff_text_autoscroll_seq: 0,
            diff_text_autoscroll_target: None,
            diff_text_last_mouse_pos: point(px(0.0), px(0.0)),
            diff_suppress_clicks_remaining: 0,
            diff_text_hitboxes: HashMap::default(),
            diff_text_layout_cache_epoch: 0,
            diff_text_layout_cache: HashMap::default(),
            diff_hunk_picker_search_input: None,
            diff_search_active: false,
            diff_search_query: "".into(),
            diff_search_matches: Vec::new(),
            diff_search_match_ix: None,
            diff_search_input,
            _diff_search_subscription: diff_search_subscription,
            file_diff_cache_repo_id: None,
            file_diff_cache_rev: 0,
            file_diff_cache_target: None,
            file_diff_cache_path: None,
            file_diff_cache_language: None,
            file_diff_cache_rows: Vec::new(),
            file_diff_inline_cache: Vec::new(),
            file_diff_inline_word_highlights: Vec::new(),
            file_diff_split_word_highlights_old: Vec::new(),
            file_diff_split_word_highlights_new: Vec::new(),
            file_diff_cache_seq: 0,
            file_diff_cache_inflight: None,
            file_image_diff_cache_repo_id: None,
            file_image_diff_cache_rev: 0,
            file_image_diff_cache_target: None,
            file_image_diff_cache_path: None,
            file_image_diff_cache_old: None,
            file_image_diff_cache_new: None,
            worktree_preview_path: None,
            worktree_preview: Loadable::NotLoaded,
            worktree_preview_segments_cache_path: None,
            worktree_preview_syntax_language: None,
            worktree_preview_segments_cache: HashMap::default(),
            diff_preview_is_new_file: false,
            diff_preview_new_file_lines: Arc::new(Vec::new()),
            conflict_resolver_input,
            _conflict_resolver_input_subscription: conflict_resolver_subscription,
            conflict_resolver: ConflictResolverUiState::default(),
            conflict_resolver_vsplit_ratio: 0.5,
            conflict_resolver_vsplit_resize: None,
            conflict_three_way_col_ratios: [1.0 / 3.0, 2.0 / 3.0],
            conflict_three_way_col_widths: [px(0.0); 3],
            conflict_hsplit_resize: None,
            conflict_diff_split_ratio: 0.5,
            conflict_diff_split_resize: None,
            conflict_diff_split_col_widths: [px(0.0); 2],
            conflict_diff_segments_cache_split: HashMap::default(),
            conflict_diff_segments_cache_inline: HashMap::default(),
            conflict_three_way_segments_cache: HashMap::default(),
            conflict_resolved_preview_path: None,
            conflict_resolved_preview_source_hash: None,
            conflict_resolved_preview_syntax_language: None,
            conflict_resolved_preview_lines: Vec::new(),
            conflict_resolved_preview_segments_cache: HashMap::default(),
            history_view,
            diff_scroll: UniformListScrollHandle::default(),
            diff_split_right_scroll: UniformListScrollHandle::default(),
            conflict_resolver_diff_scroll: UniformListScrollHandle::default(),
            conflict_resolved_preview_scroll: UniformListScrollHandle::default(),
            worktree_preview_scroll: UniformListScrollHandle::default(),
            path_display_cache: std::cell::RefCell::new(HashMap::default()),
        };

        pane.set_theme(theme, cx);
        pane.rebuild_diff_cache(cx);
        pane
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.diff_text_segments_cache.clear();
        self.worktree_preview_segments_cache_path = None;
        self.worktree_preview_segments_cache.clear();
        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_segments_cache_inline.clear();
        self.conflict_resolved_preview_segments_cache.clear();
        self.diff_raw_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.diff_search_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.conflict_resolver_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        let output_syntax_highlights = build_resolved_output_syntax_highlights(
            theme,
            &self.conflict_resolved_preview_lines,
            self.conflict_resolved_preview_syntax_language,
        );
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_highlights(output_syntax_highlights, cx);
        });
        if let Some(input) = &self.diff_hunk_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        self.history_view
            .update(cx, |view, cx| view.set_theme(theme, cx));
        cx.notify();
    }

    fn conflict_resolver_invalidate_resolved_outline(&mut self) {
        self.conflict_resolver.resolver_pending_recompute_seq = self
            .conflict_resolver
            .resolver_pending_recompute_seq
            .wrapping_add(1);
        self.conflict_resolved_preview_path = None;
        self.conflict_resolved_preview_source_hash = None;
        self.conflict_resolved_preview_syntax_language = None;
        self.conflict_resolved_preview_lines.clear();
        self.conflict_resolved_preview_segments_cache.clear();
        self.conflict_resolver.resolved_line_meta.clear();
        self.conflict_resolver
            .resolved_output_conflict_markers
            .clear();
        self.conflict_resolver
            .resolved_output_line_sources_index
            .clear();
    }

    fn recompute_conflict_resolved_outline_and_provenance(
        &mut self,
        path: Option<&std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolved_preview_syntax_language =
            path.and_then(|p| rows::diff_syntax_language_for_path(p.to_string_lossy().as_ref()));
        let output_text = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text().to_string());
        self.conflict_resolved_preview_lines =
            conflict_resolver::split_output_lines_for_outline(&output_text);
        self.conflict_resolved_preview_segments_cache.clear();
        let output_syntax_highlights = build_resolved_output_syntax_highlights(
            self.theme,
            &self.conflict_resolved_preview_lines,
            self.conflict_resolved_preview_syntax_language,
        );
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_highlights(output_syntax_highlights, cx);
        });

        // Provenance: classify each output line as A/B/C/Manual.
        let view_mode = self.conflict_resolver.view_mode;
        let (two_way_old, two_way_new) = if view_mode == ConflictResolverViewMode::TwoWayDiff {
            collect_two_way_source_lines(&self.conflict_resolver.diff_rows)
        } else {
            (Vec::new(), Vec::new())
        };
        let sources = match view_mode {
            ConflictResolverViewMode::ThreeWay => conflict_resolver::SourceLines {
                a: &self.conflict_resolver.three_way_base_lines,
                b: &self.conflict_resolver.three_way_ours_lines,
                c: &self.conflict_resolver.three_way_theirs_lines,
            },
            ConflictResolverViewMode::TwoWayDiff => conflict_resolver::SourceLines {
                a: &two_way_old,
                b: &two_way_new,
                c: &[],
            },
        };
        let mut meta = conflict_resolver::compute_resolved_line_provenance(
            &self.conflict_resolved_preview_lines,
            &sources,
        );
        apply_conflict_choice_provenance_hints(
            &mut meta,
            &self.conflict_resolver.marker_segments,
            &output_text,
            view_mode,
        );
        self.conflict_resolver.resolved_output_line_sources_index =
            conflict_resolver::build_resolved_output_line_sources_index(
                &meta,
                &self.conflict_resolved_preview_lines,
                view_mode,
            );
        self.conflict_resolver.resolved_output_conflict_markers =
            build_resolved_output_conflict_markers(
                &self.conflict_resolver.marker_segments,
                &output_text,
                self.conflict_resolved_preview_lines.len(),
            );
        self.conflict_resolver.resolved_line_meta = meta;
    }

    fn conflict_resolver_scroll_resolved_output_to_line(
        &self,
        target_line_ix: usize,
        line_count: usize,
    ) {
        scroll_conflict_resolved_output_to_line(
            &self.conflict_resolved_preview_scroll,
            target_line_ix,
            line_count,
        );
    }

    fn conflict_resolver_scroll_resolved_output_to_line_in_text(
        &self,
        target_line_ix: usize,
        output_text: &str,
    ) {
        let line_count = output_text.split('\n').count().max(1);
        self.conflict_resolver_scroll_resolved_output_to_line(target_line_ix, line_count);
    }

    fn schedule_conflict_resolved_outline_recompute(
        &mut self,
        path: Option<std::path::PathBuf>,
        output_hash: u64,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.resolver_pending_recompute_seq = self
            .conflict_resolver
            .resolver_pending_recompute_seq
            .wrapping_add(1);
        let seq = self.conflict_resolver.resolver_pending_recompute_seq;

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                Timer::after(Duration::from_millis(CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.conflict_resolver.resolver_pending_recompute_seq != seq {
                        return;
                    }
                    if this.conflict_resolved_preview_source_hash != Some(output_hash)
                        || this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                    {
                        return;
                    }
                    this.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);

                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();
        self.history_view.update(cx, |view, cx| {
            view.set_active_context_menu_invoker(next, cx)
        });
        cx.notify();
    }

    pub(in super::super) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.history_view
            .update(cx, |view, cx| view.set_date_time_format(next, cx));
        cx.notify();
    }

    pub(in super::super) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        self.history_view
            .update(cx, |view, cx| view.set_timezone(next, cx));
        cx.notify();
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn history_visible_column_preferences(
        &self,
        cx: &gpui::App,
    ) -> (bool, bool, bool) {
        self.history_view
            .read(cx)
            .history_visible_column_preferences()
    }

    pub(in super::super) fn conflict_advanced_autosolve_settings(&self) -> (bool, bool, bool) {
        (
            self.conflict_enable_whitespace_autosolve,
            self.conflict_enable_regex_autosolve,
            self.conflict_enable_history_autosolve,
        )
    }

    pub(in super::super) fn set_conflict_enable_whitespace_autosolve(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_enable_whitespace_autosolve == enabled {
            return;
        }
        self.conflict_enable_whitespace_autosolve = enabled;
        cx.notify();
    }

    pub(in super::super) fn set_conflict_enable_regex_autosolve(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_enable_regex_autosolve == enabled {
            return;
        }
        self.conflict_enable_regex_autosolve = enabled;
        cx.notify();
    }

    pub(in super::super) fn set_conflict_enable_history_autosolve(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_enable_history_autosolve == enabled {
            return;
        }
        self.conflict_enable_history_autosolve = enabled;
        cx.notify();
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    #[allow(clippy::too_many_arguments, dead_code)]
    pub(in super::super) fn open_conflict_resolver_input_row_context_menu(
        &mut self,
        invoker: SharedString,
        line_label: SharedString,
        line_target: ResolverPickTarget,
        chunk_label: SharedString,
        chunk_target: ResolverPickTarget,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.activate_context_menu_invoker(invoker, cx);
        self.open_popover_at(
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            },
            anchor,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn open_conflict_resolver_chunk_context_menu(
        &mut self,
        invoker: SharedString,
        conflict_ix: usize,
        has_base: bool,
        is_three_way: bool,
        selected_choices: Vec<conflict_resolver::ConflictChoice>,
        output_line_ix: Option<usize>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.activate_context_menu_invoker(invoker, cx);
        self.open_popover_at(
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
            },
            anchor,
            window,
            cx,
        );
    }

    pub(in super::super) fn conflict_resolver_selected_choices_for_conflict_ix(
        &self,
        conflict_ix: usize,
    ) -> Vec<conflict_resolver::ConflictChoice> {
        conflict_group_selected_choices_for_ix(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
        )
    }

    pub(in super::super) fn conflict_resolver_has_base_for_conflict_ix(
        &self,
        conflict_ix: usize,
    ) -> bool {
        self.conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
                _ => None,
            })
            .nth(conflict_ix)
            .unwrap_or(false)
    }

    pub(in super::super) fn open_conflict_resolver_output_context_menu(
        &mut self,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (selected_text, cursor_offset, clicked_offset, content) =
            self.conflict_resolver_input.read_with(cx, |i, _| {
                (
                    i.selected_text(),
                    i.cursor_offset(),
                    i.offset_for_position(anchor),
                    i.text().to_string(),
                )
            });
        let context_line =
            conflict_resolver_output_context_line(&content, cursor_offset, Some(clicked_offset));

        if let Some(marker) = resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            &content,
            context_line,
        ) {
            let is_three_way = self.conflict_resolver.view_mode
                == conflict_resolver::ConflictResolverViewMode::ThreeWay;
            let selected_choices =
                self.conflict_resolver_selected_choices_for_conflict_ix(marker.conflict_ix);
            let has_base = self.conflict_resolver_has_base_for_conflict_ix(marker.conflict_ix);
            let invoker: SharedString = format!(
                "resolver_output_chunk_menu_{}_{}",
                marker.conflict_ix, context_line
            )
            .into();
            self.open_conflict_resolver_chunk_context_menu(
                invoker,
                marker.conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                Some(context_line),
                anchor,
                window,
                cx,
            );
            return;
        }

        let is_three_way = self.conflict_resolver.view_mode
            == conflict_resolver::ConflictResolverViewMode::ThreeWay;

        let (has_source_a, has_source_b, has_source_c) = if is_three_way {
            (
                context_line < self.conflict_resolver.three_way_base_lines.len(),
                context_line < self.conflict_resolver.three_way_ours_lines.len(),
                context_line < self.conflict_resolver.three_way_theirs_lines.len(),
            )
        } else {
            (
                context_line < self.conflict_resolver.diff_rows.len()
                    && self
                        .conflict_resolver
                        .diff_rows
                        .get(context_line)
                        .and_then(|r| r.old.as_ref())
                        .is_some(),
                context_line < self.conflict_resolver.diff_rows.len()
                    && self
                        .conflict_resolver
                        .diff_rows
                        .get(context_line)
                        .and_then(|r| r.new.as_ref())
                        .is_some(),
                false,
            )
        };

        self.open_popover_at(
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line: context_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            },
            anchor,
            window,
            cx,
        );
    }

    pub(in super::super) fn open_popover_at_cursor(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, root.last_mouse_pos, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn clear_status_multi_selection(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.remove(&repo_id);
                cx.notify();
            });
        });
    }

    pub(in super::super) fn scroll_status_list_to_ix(
        &mut self,
        area: DiffArea,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane
                .update(cx, |pane: &mut DetailsPaneView, cx| {
                    match area {
                        DiffArea::Unstaged => pane
                            .unstaged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                        DiffArea::Staged => pane
                            .staged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                    }
                    cx.notify();
                });
        });
    }

    pub(in super::super) fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
        false
    }

    pub(in super::super) fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
        false
    }

    pub(super) fn apply_state_snapshot(
        &mut self,
        next: Arc<AppState>,
        cx: &mut gpui::Context<Self>,
    ) {
        let prev_active_repo_id = self.state.active_repo;
        let prev_diff_target = self
            .active_repo()
            .and_then(|r| r.diff_target.as_ref())
            .cloned();

        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
        let next_diff_target = next_repo.and_then(|r| r.diff_target.as_ref()).cloned();
        let next_diff_rev = next_repo.map(|r| r.diff_rev).unwrap_or(0);

        if prev_diff_target != next_diff_target {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            self.diff_autoscroll_pending = next_diff_target.is_some();
        }

        self.state = next;

        self.sync_conflict_resolver(cx);

        if prev_active_repo_id != next_repo_id {
            self.history_view.update(cx, |view, _| {
                view.history_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            });
        }

        let should_rebuild_diff_cache = self.diff_cache_repo_id != next_repo_id
            || self.diff_cache_rev != next_diff_rev
            || self.diff_cache_target != next_diff_target;
        if should_rebuild_diff_cache {
            self.rebuild_diff_cache(cx);
        }

        // History caches are now managed by HistoryView.
    }

    pub(in super::super) fn cached_path_display(&self, path: &std::path::PathBuf) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    pub(in super::super) fn touch_diff_text_layout_cache(
        &mut self,
        key: u64,
        layout: Option<ShapedLine>,
    ) {
        let epoch = self.diff_text_layout_cache_epoch;
        match layout {
            Some(layout) => {
                self.diff_text_layout_cache.insert(
                    key,
                    DiffTextLayoutCacheEntry {
                        layout,
                        last_used_epoch: epoch,
                    },
                );
            }
            None => {
                if let Some(entry) = self.diff_text_layout_cache.get_mut(&key) {
                    entry.last_used_epoch = epoch;
                }
            }
        }
    }

    /// Prune the layout cache if it has grown past the high-water mark.
    /// Call once per render frame (after bumping the epoch), **not** from
    /// the per-row `touch_diff_text_layout_cache` hot path.
    pub(in super::super) fn prune_diff_text_layout_cache(&mut self) {
        if self.diff_text_layout_cache.len()
            <= DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES + DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE
        {
            return;
        }

        let over_by = self
            .diff_text_layout_cache
            .len()
            .saturating_sub(DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES);
        if over_by == 0 {
            return;
        }

        let mut by_age: Vec<(u64, u64)> = self
            .diff_text_layout_cache
            .iter()
            .map(|(k, v)| (*k, v.last_used_epoch))
            .collect();
        by_age.sort_by_key(|(_, last_used)| *last_used);

        for (key, _) in by_age.into_iter().take(over_by) {
            self.diff_text_layout_cache.remove(&key);
        }
    }

    pub(in super::super) fn diff_text_segments_cache_get(
        &self,
        key: usize,
    ) -> Option<&CachedDiffStyledText> {
        self.diff_text_segments_cache
            .get(key)
            .and_then(Option::as_ref)
    }

    pub(in super::super) fn diff_text_segments_cache_set(
        &mut self,
        key: usize,
        value: CachedDiffStyledText,
    ) -> &CachedDiffStyledText {
        if self.diff_text_segments_cache.len() <= key {
            self.diff_text_segments_cache.resize_with(key + 1, || None);
        }
        self.diff_text_segments_cache[key] = Some(value);
        self.diff_text_segments_cache[key]
            .as_ref()
            .expect("just set")
    }

    pub(in super::super) fn is_file_diff_view_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        self.file_diff_cache_repo_id == Some(repo.id)
            && self.file_diff_cache_rev == repo.diff_file_rev
            && self.file_diff_cache_target == repo.diff_target
            && self.file_diff_cache_path.is_some()
    }

    pub(in super::super) fn is_file_image_diff_view_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        self.file_image_diff_cache_repo_id == Some(repo.id)
            && self.file_image_diff_cache_rev == repo.diff_file_rev
            && self.file_image_diff_cache_target == repo.diff_target
            && self.file_image_diff_cache_path.is_some()
            && (self.file_image_diff_cache_old.is_some()
                || self.file_image_diff_cache_new.is_some())
    }

    pub(in super::super) fn consume_suppress_click_after_drag(&mut self) -> bool {
        if self.diff_suppress_clicks_remaining > 0 {
            self.diff_suppress_clicks_remaining =
                self.diff_suppress_clicks_remaining.saturating_sub(1);
            return true;
        }
        false
    }

    fn diff_src_ixs_for_visible_ix(&self, visible_ix: usize) -> Vec<usize> {
        if self.is_file_diff_view_active() {
            return Vec::new();
        }
        let Some(&mapped_ix) = self.diff_visible_indices.get(visible_ix) else {
            return Vec::new();
        };

        match self.diff_view {
            DiffViewMode::Inline => vec![mapped_ix],
            DiffViewMode::Split => {
                let Some(row) = self.diff_split_cache.get(mapped_ix) else {
                    return Vec::new();
                };
                match row {
                    PatchSplitRow::Raw { src_ix, .. } => vec![*src_ix],
                    PatchSplitRow::Aligned {
                        old_src_ix,
                        new_src_ix,
                        ..
                    } => {
                        let mut out = Vec::with_capacity(2);
                        if let Some(ix) = old_src_ix {
                            out.push(*ix);
                        }
                        if let Some(ix) = new_src_ix
                            && out.first().copied() != Some(*ix)
                        {
                            out.push(*ix);
                        }
                        out
                    }
                }
            }
        }
    }

    fn diff_enclosing_hunk_src_ix(&self, src_ix: usize) -> Option<usize> {
        enclosing_hunk_src_ix(&self.diff_cache, src_ix)
    }

    pub(in super::super) fn select_all_diff_text(&mut self) {
        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let end_visible_ix = count - 1;
            let end_text = self.diff_text_line_for_region(end_visible_ix, DiffTextRegion::Inline);

            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix: 0,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix: end_visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_text.len(),
            });
            return;
        }

        if self.diff_visible_indices.is_empty() {
            return;
        }

        let start_region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => self
                .diff_text_head
                .or(self.diff_text_anchor)
                .map(|p| p.region)
                .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight))
                .unwrap_or(DiffTextRegion::SplitLeft),
        };

        let end_visible_ix = self.diff_visible_indices.len() - 1;
        let end_region = start_region;
        let end_text = self.diff_text_line_for_region(end_visible_ix, end_region);

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            visible_ix: 0,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            visible_ix: end_visible_ix,
            region: end_region,
            offset: end_text.len(),
        });
    }

    fn select_diff_text_rows_range(
        &mut self,
        start_visible_ix: usize,
        end_visible_ix: usize,
        region: DiffTextRegion,
    ) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            return;
        }

        let a = start_visible_ix.min(list_len - 1);
        let b = end_visible_ix.min(list_len - 1);
        let (a, b) = if a <= b { (a, b) } else { (b, a) };

        let region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => match region {
                DiffTextRegion::SplitRight => DiffTextRegion::SplitRight,
                _ => DiffTextRegion::SplitLeft,
            },
        };
        let start_region = region;
        let end_region = region;

        let end_text = self.diff_text_line_for_region(b, end_region);

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            visible_ix: a,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            visible_ix: b,
            region: end_region,
            offset: end_text.len(),
        });

        // Double-click produces two click events; suppress both.
        self.diff_suppress_clicks_remaining = 2;
    }

    pub(in super::super) fn double_click_select_diff_text(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        kind: DiffClickKind,
    ) {
        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let visible_ix = visible_ix.min(count - 1);
            let end_text = self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_text.len(),
            });

            // Double-click produces two click events; suppress both.
            self.diff_suppress_clicks_remaining = 2;
            return;
        }

        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            return;
        }
        let visible_ix = visible_ix.min(list_len - 1);

        // File-diff view doesn't have file/hunk header blocks; treat as row selection.
        if self.is_file_diff_view_active() {
            self.select_diff_text_rows_range(visible_ix, visible_ix, region);
            return;
        }

        let end = match self.diff_view {
            DiffViewMode::Inline => match kind {
                DiffClickKind::Line => visible_ix,
                DiffClickKind::HunkHeader => self
                    .diff_next_boundary_visible_ix(visible_ix, |src_ix| {
                        let line = &self.diff_cache[src_ix];
                        matches!(line.kind, gitgpui_core::domain::DiffLineKind::Hunk)
                            || (matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                                && line.text.starts_with("diff --git "))
                    })
                    .unwrap_or(list_len - 1),
                DiffClickKind::FileHeader => self
                    .diff_next_boundary_visible_ix(visible_ix, |src_ix| {
                        let line = &self.diff_cache[src_ix];
                        matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                            && line.text.starts_with("diff --git ")
                    })
                    .unwrap_or(list_len - 1),
            },
            DiffViewMode::Split => match kind {
                DiffClickKind::Line => visible_ix,
                DiffClickKind::HunkHeader => self
                    .split_next_boundary_visible_ix(visible_ix, |row| {
                        matches!(
                            row,
                            PatchSplitRow::Raw {
                                click_kind: DiffClickKind::HunkHeader | DiffClickKind::FileHeader,
                                ..
                            }
                        )
                    })
                    .unwrap_or(list_len - 1),
                DiffClickKind::FileHeader => self
                    .split_next_boundary_visible_ix(visible_ix, |row| {
                        matches!(
                            row,
                            PatchSplitRow::Raw {
                                click_kind: DiffClickKind::FileHeader,
                                ..
                            }
                        )
                    })
                    .unwrap_or(list_len - 1),
            },
        };

        self.select_diff_text_rows_range(visible_ix, end, region);
    }

    fn split_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(&PatchSplitRow) -> bool,
    ) -> Option<usize> {
        let from_visible_ix =
            from_visible_ix.min(self.diff_visible_indices.len().saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..self.diff_visible_indices.len() {
            let row_ix = *self.diff_visible_indices.get(visible_ix)?;
            let row = self.diff_split_cache.get(row_ix)?;
            if is_boundary(row) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    fn diff_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let from_visible_ix =
            from_visible_ix.min(self.diff_visible_indices.len().saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..self.diff_visible_indices.len() {
            let src_ix = *self.diff_visible_indices.get(visible_ix)?;
            if is_boundary(src_ix) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    pub(in super::super) fn sync_diff_split_vertical_scroll(&mut self) {
        let left_handle = self.diff_scroll.0.borrow().base_handle.clone();
        let right_handle = self.diff_split_right_scroll.0.borrow().base_handle.clone();
        let left_offset = left_handle.offset();
        let right_offset = right_handle.offset();

        if left_offset.y == right_offset.y {
            self.diff_split_last_synced_y = left_offset.y;
            return;
        }

        let last_synced_y = self.diff_split_last_synced_y;
        let left_changed = left_offset.y != last_synced_y;
        let right_changed = right_offset.y != last_synced_y;

        let master_y = match (left_changed, right_changed) {
            (true, false) => left_offset.y,
            (false, true) => right_offset.y,
            // If both changed (or neither changed), prefer the left scroll (the vertical scrollbar).
            _ => left_offset.y,
        };

        left_handle.set_offset(point(left_offset.x, master_y));
        right_handle.set_offset(point(right_offset.x, master_y));
        self.diff_split_last_synced_y = master_y;
    }

    pub(in super::super) fn main_pane_content_width(&self, cx: &mut gpui::Context<Self>) -> Pixels {
        let fallback_sidebar = px(280.0);
        let fallback_details = px(420.0);
        let (sidebar_w, details_w) = self
            .root_view
            .update(cx, |root, _cx| (root.sidebar_width, root.details_width))
            .unwrap_or((fallback_sidebar, fallback_details));

        let handles_w = px(PANE_RESIZE_HANDLE_PX) * 2.0;
        (self.last_window_size.width - sidebar_w - details_w - handles_w).max(px(0.0))
    }
}

impl MainPaneView {
    pub(in super::super) fn handle_patch_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        if self.is_file_diff_view_active() {
            self.handle_file_diff_row_click(clicked_visible_ix, shift);
            return;
        }
        match self.diff_view {
            DiffViewMode::Inline => self.handle_diff_row_click(clicked_visible_ix, kind, shift),
            DiffViewMode::Split => self.handle_split_row_click(clicked_visible_ix, kind, shift),
        }
    }

    fn handle_split_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);

        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        let end = match kind {
            DiffClickKind::Line => clicked_visible_ix,
            DiffClickKind::HunkHeader => self
                .split_next_boundary_visible_ix(clicked_visible_ix, |row| {
                    matches!(
                        row,
                        PatchSplitRow::Raw {
                            click_kind: DiffClickKind::HunkHeader | DiffClickKind::FileHeader,
                            ..
                        }
                    )
                })
                .unwrap_or(list_len - 1),
            DiffClickKind::FileHeader => self
                .split_next_boundary_visible_ix(clicked_visible_ix, |row| {
                    matches!(
                        row,
                        PatchSplitRow::Raw {
                            click_kind: DiffClickKind::FileHeader,
                            ..
                        }
                    )
                })
                .unwrap_or(list_len - 1),
        };

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, end));
    }

    fn handle_diff_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);

        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        let end = match kind {
            DiffClickKind::Line => clicked_visible_ix,
            DiffClickKind::HunkHeader => self
                .diff_next_boundary_visible_ix(clicked_visible_ix, |src_ix| {
                    let line = &self.diff_cache[src_ix];
                    matches!(line.kind, gitgpui_core::domain::DiffLineKind::Hunk)
                        || (matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                            && line.text.starts_with("diff --git "))
                })
                .unwrap_or(list_len - 1),
            DiffClickKind::FileHeader => self
                .diff_next_boundary_visible_ix(clicked_visible_ix, |src_ix| {
                    let line = &self.diff_cache[src_ix];
                    matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                        && line.text.starts_with("diff --git ")
                })
                .unwrap_or(list_len - 1),
        };

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, end));
    }

    fn handle_file_diff_row_click(&mut self, clicked_visible_ix: usize, shift: bool) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);
        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, clicked_visible_ix));
    }

    fn file_change_visible_indices(&self) -> Vec<usize> {
        if !self.is_file_diff_view_active() {
            return Vec::new();
        }
        match self.diff_view {
            DiffViewMode::Inline => diff_navigation::change_block_entries(
                self.diff_visible_indices.len(),
                |visible_ix| {
                    let Some(&inline_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return false;
                    };
                    self.file_diff_inline_cache.get(inline_ix).is_some_and(|l| {
                        matches!(
                            l.kind,
                            gitgpui_core::domain::DiffLineKind::Add
                                | gitgpui_core::domain::DiffLineKind::Remove
                        )
                    })
                },
            ),
            DiffViewMode::Split => diff_navigation::change_block_entries(
                self.diff_visible_indices.len(),
                |visible_ix| {
                    let Some(&row_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return false;
                    };
                    self.file_diff_cache_rows.get(row_ix).is_some_and(|row| {
                        !matches!(row.kind, gitgpui_core::file_diff::FileDiffRowKind::Context)
                    })
                },
            ),
        }
    }

    fn patch_hunk_entries(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (visible_ix, &ix) in self.diff_visible_indices.iter().enumerate() {
            match self.diff_view {
                DiffViewMode::Inline => {
                    let Some(line) = self.diff_cache.get(ix) else {
                        continue;
                    };
                    if matches!(line.kind, gitgpui_core::domain::DiffLineKind::Hunk) {
                        out.push((visible_ix, ix));
                    }
                }
                DiffViewMode::Split => {
                    let Some(row) = self.diff_split_cache.get(ix) else {
                        continue;
                    };
                    if let PatchSplitRow::Raw {
                        src_ix,
                        click_kind: DiffClickKind::HunkHeader,
                    } = row
                    {
                        out.push((visible_ix, *src_ix));
                    }
                }
            }
        }
        out
    }

    pub(in super::super) fn diff_nav_entries(&self) -> Vec<usize> {
        if self.is_file_diff_view_active() {
            return self.file_change_visible_indices();
        }
        self.patch_hunk_entries()
            .into_iter()
            .map(|(visible_ix, _)| visible_ix)
            .collect()
    }

    fn conflict_marker_nav_entries(&self) -> Vec<usize> {
        conflict_marker_nav_entries_from_markers(
            &self.conflict_resolver.resolved_output_conflict_markers,
        )
    }

    fn conflict_fallback_nav_entries(&self) -> Vec<usize> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                conflict_resolver::unresolved_visible_nav_entries_for_three_way(
                    &self.conflict_resolver.marker_segments,
                    &self.conflict_resolver.three_way_visible_map,
                    &self.conflict_resolver.three_way_conflict_ranges,
                )
            }
            ConflictResolverViewMode::TwoWayDiff => match self.conflict_resolver.diff_mode {
                ConflictDiffMode::Split => {
                    let (split_map, _) = conflict_resolver::map_two_way_rows_to_conflicts(
                        &self.conflict_resolver.marker_segments,
                        &self.conflict_resolver.diff_rows,
                        &self.conflict_resolver.inline_rows,
                    );
                    conflict_resolver::unresolved_visible_nav_entries_for_two_way(
                        &self.conflict_resolver.marker_segments,
                        &split_map,
                        &self.conflict_resolver.diff_visible_row_indices,
                    )
                }
                ConflictDiffMode::Inline => {
                    let (_, inline_map) = conflict_resolver::map_two_way_rows_to_conflicts(
                        &self.conflict_resolver.marker_segments,
                        &self.conflict_resolver.diff_rows,
                        &self.conflict_resolver.inline_rows,
                    );
                    conflict_resolver::unresolved_visible_nav_entries_for_two_way(
                        &self.conflict_resolver.marker_segments,
                        &inline_map,
                        &self.conflict_resolver.inline_visible_row_indices,
                    )
                }
            },
        }
    }

    pub(in super::super) fn conflict_nav_entries(&self) -> Vec<usize> {
        let marker_entries = self.conflict_marker_nav_entries();
        if !marker_entries.is_empty() {
            return marker_entries;
        }
        self.conflict_fallback_nav_entries()
    }

    pub(in super::super) fn conflict_jump_prev(&mut self) {
        let marker_entries = self.conflict_marker_nav_entries();
        let use_marker_nav = !marker_entries.is_empty();
        let entries = if use_marker_nav {
            marker_entries
        } else {
            self.conflict_fallback_nav_entries()
        };
        if entries.is_empty() {
            return;
        }

        let current = self.conflict_resolver.nav_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            return;
        };

        if use_marker_nav {
            self.conflict_resolver_scroll_resolved_output_to_line(
                target,
                self.conflict_resolved_preview_lines.len(),
            );
            if let Some(marker) = self
                .conflict_resolver
                .resolved_output_conflict_markers
                .get(target)
                .copied()
                .flatten()
            {
                self.conflict_resolver.active_conflict = marker.conflict_ix;
            }
        } else {
            match self.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    // target is now a visible index; scroll directly.
                    self.conflict_resolver_diff_scroll
                        .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
                    // Map visible index back to conflict range index.
                    if let Some(range_ix) = self.conflict_resolver_range_ix_for_visible(target) {
                        self.conflict_resolver.active_conflict = range_ix;
                    }
                }
                ConflictResolverViewMode::TwoWayDiff => {
                    self.conflict_resolver_diff_scroll
                        .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
                    if let Some(conflict_ix) =
                        self.conflict_resolver_two_way_conflict_ix_for_visible(target)
                    {
                        self.conflict_resolver.active_conflict = conflict_ix;
                    }
                }
            }
        }
        self.conflict_resolver.nav_anchor = Some(target);
    }

    pub(in super::super) fn conflict_jump_next(&mut self) {
        let marker_entries = self.conflict_marker_nav_entries();
        let use_marker_nav = !marker_entries.is_empty();
        let entries = if use_marker_nav {
            marker_entries
        } else {
            self.conflict_fallback_nav_entries()
        };
        if entries.is_empty() {
            return;
        }

        let current = self.conflict_resolver.nav_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            return;
        };

        if use_marker_nav {
            self.conflict_resolver_scroll_resolved_output_to_line(
                target,
                self.conflict_resolved_preview_lines.len(),
            );
            if let Some(marker) = self
                .conflict_resolver
                .resolved_output_conflict_markers
                .get(target)
                .copied()
                .flatten()
            {
                self.conflict_resolver.active_conflict = marker.conflict_ix;
            }
        } else {
            match self.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    // target is now a visible index; scroll directly.
                    self.conflict_resolver_diff_scroll
                        .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
                    // Map visible index back to conflict range index.
                    if let Some(range_ix) = self.conflict_resolver_range_ix_for_visible(target) {
                        self.conflict_resolver.active_conflict = range_ix;
                    }
                }
                ConflictResolverViewMode::TwoWayDiff => {
                    self.conflict_resolver_diff_scroll
                        .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
                    if let Some(conflict_ix) =
                        self.conflict_resolver_two_way_conflict_ix_for_visible(target)
                    {
                        self.conflict_resolver.active_conflict = conflict_ix;
                    }
                }
            }
        }
        self.conflict_resolver.nav_anchor = Some(target);
    }

    /// Map a visible index back to the conflict range index it belongs to.
    fn conflict_resolver_range_ix_for_visible(&self, vi: usize) -> Option<usize> {
        let item = self.conflict_resolver.three_way_visible_map.get(vi)?;
        match item {
            conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(ri) => Some(*ri),
            conflict_resolver::ThreeWayVisibleItem::Line(line_ix) => self
                .conflict_resolver
                .three_way_conflict_ranges
                .iter()
                .position(|r| r.contains(line_ix)),
        }
    }

    fn conflict_resolver_two_way_conflict_ix_for_visible(
        &self,
        visible_ix: usize,
    ) -> Option<usize> {
        let (split_map, inline_map) = conflict_resolver::map_two_way_rows_to_conflicts(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.diff_rows,
            &self.conflict_resolver.inline_rows,
        );

        match self.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => conflict_resolver::two_way_conflict_index_for_visible_row(
                &split_map,
                &self.conflict_resolver.diff_visible_row_indices,
                visible_ix,
            ),
            ConflictDiffMode::Inline => conflict_resolver::two_way_conflict_index_for_visible_row(
                &inline_map,
                &self.conflict_resolver.inline_visible_row_indices,
                visible_ix,
            ),
        }
    }

    fn conflict_resolver_two_way_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        let (split_map, inline_map) = conflict_resolver::map_two_way_rows_to_conflicts(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.diff_rows,
            &self.conflict_resolver.inline_rows,
        );

        match self.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => split_map
                .iter()
                .position(|mapped| *mapped == Some(conflict_ix))
                .and_then(|row_ix| {
                    self.conflict_resolver
                        .diff_visible_row_indices
                        .binary_search(&row_ix)
                        .ok()
                }),
            ConflictDiffMode::Inline => inline_map
                .iter()
                .position(|mapped| *mapped == Some(conflict_ix))
                .and_then(|row_ix| {
                    self.conflict_resolver
                        .inline_visible_row_indices
                        .binary_search(&row_ix)
                        .ok()
                }),
        }
    }

    pub(in super::super) fn scroll_diff_to_item(
        &mut self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.diff_scroll.scroll_to_item(target, strategy);
        if self.diff_view == DiffViewMode::Split {
            self.diff_split_right_scroll
                .scroll_to_item(target, strategy);
        }
    }

    pub(in super::super) fn scroll_diff_to_item_strict(
        &mut self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.diff_scroll.scroll_to_item_strict(target, strategy);
        if self.diff_view == DiffViewMode::Split {
            self.diff_split_right_scroll
                .scroll_to_item_strict(target, strategy);
        }
    }

    pub(in super::super) fn diff_jump_prev(&mut self) {
        let entries = self.diff_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.diff_selection_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in super::super) fn diff_jump_next(&mut self) {
        let entries = self.diff_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.diff_selection_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in super::super) fn maybe_autoscroll_diff_to_first_change(&mut self) {
        if !self.diff_autoscroll_pending {
            return;
        }
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_autoscroll_pending = false;
            return;
        }
        if self.diff_visible_indices.is_empty() {
            return;
        }

        let entries = self.diff_nav_entries();
        let target = entries.first().copied().unwrap_or(0);

        self.scroll_diff_to_item(target, gpui::ScrollStrategy::Top);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
        self.diff_autoscroll_pending = false;
    }

    fn sync_conflict_resolver(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };

        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };

        let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_target.as_ref() else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };
        if *area != DiffArea::Unstaged {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        }

        let conflict_entry = match &repo.status {
            Loadable::Ready(status) => status.unstaged.iter().find(|e| {
                e.path == *path && e.kind == gitgpui_core::domain::FileStatusKind::Conflicted
            }),
            _ => None,
        };
        let Some(conflict_entry) = conflict_entry else {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        };
        let conflict_kind = conflict_entry.conflict;

        let path = path.clone();

        let should_load = repo.conflict_file_path.as_ref() != Some(&path)
            && !matches!(repo.conflict_file, Loadable::Loading);
        if should_load {
            self.conflict_resolver = ConflictResolverUiState::default();
            self.conflict_resolver_invalidate_resolved_outline();
            let theme = self.theme;
            self.conflict_resolver_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text("", cx);
            });
            self.store.dispatch(Msg::LoadConflictFile { repo_id, path });
            return;
        }

        let Loadable::Ready(Some(file)) = &repo.conflict_file else {
            return;
        };
        if file.path != path {
            return;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        file.base.hash(&mut hasher);
        file.ours.hash(&mut hasher);
        file.theirs.hash(&mut hasher);
        file.current.hash(&mut hasher);
        let source_hash = hasher.finish();

        let needs_rebuild = self.conflict_resolver.repo_id != Some(repo_id)
            || self.conflict_resolver.path.as_ref() != Some(&path)
            || self.conflict_resolver.source_hash != Some(source_hash);

        // When the file content hasn't changed but state-side conflict data has
        // been updated (e.g. hide_resolved toggled externally, bulk picks, or
        // autosolve applied from state), do a lightweight re-sync that re-applies
        // session resolutions and rebuilds visible maps without recomputing the
        // expensive diff/highlight data.
        if !needs_rebuild {
            if self.conflict_resolver.conflict_rev != repo.conflict_rev {
                self.resync_conflict_resolver_from_state(cx);
            }
            return;
        }

        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_segments_cache_inline.clear();

        // Use the ConflictSession from state for strategy if available,
        // otherwise fall back to local computation.
        let (conflict_strategy, is_binary) = if let Some(session) = &repo.conflict_session {
            let binary =
                session.base.is_binary() || session.ours.is_binary() || session.theirs.is_binary();
            (Some(session.strategy), binary)
        } else {
            let has_non_text =
                |bytes: &Option<Vec<u8>>, text: &Option<String>| bytes.is_some() && text.is_none();
            let binary = has_non_text(&file.base_bytes, &file.base)
                || has_non_text(&file.ours_bytes, &file.ours)
                || has_non_text(&file.theirs_bytes, &file.theirs);
            (
                Self::conflict_resolver_strategy(conflict_kind, binary),
                binary,
            )
        };

        // For binary conflicts, populate minimal state and return early.
        if is_binary {
            let binary_side_sizes = [
                file.base_bytes.as_ref().map(|b| b.len()),
                file.ours_bytes.as_ref().map(|b| b.len()),
                file.theirs_bytes.as_ref().map(|b| b.len()),
            ];
            self.conflict_resolver = ConflictResolverUiState {
                repo_id: Some(repo_id),
                path: Some(path),
                source_hash: Some(source_hash),
                is_binary_conflict: true,
                binary_side_sizes,
                strategy: conflict_strategy,
                conflict_kind,
                last_autosolve_summary: None,
                conflict_rev: repo.conflict_rev,
                ..ConflictResolverUiState::default()
            };
            self.conflict_resolver_invalidate_resolved_outline();
            return;
        }

        let fallback_resolved = if let Some(cur) = file.current.as_deref() {
            cur.to_string()
        } else if let Some(ours) = file.ours.as_deref() {
            ours.to_string()
        } else if let Some(theirs) = file.theirs.as_deref() {
            theirs.to_string()
        } else {
            String::new()
        };
        let mut marker_segments = if let Some(cur) = file.current.as_deref() {
            let segments = conflict_resolver::parse_conflict_markers(cur);
            if conflict_resolver::conflict_count(&segments) > 0 {
                segments
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let ours_text = file.ours.as_deref().unwrap_or("");
        let theirs_text = file.theirs.as_deref().unwrap_or("");
        let base_text = file.base.as_deref().unwrap_or("");

        // When conflict markers are 2-way (no base section), populate block.base
        // from the git ancestor file so "A (base)" picks work.
        if !base_text.is_empty() {
            conflict_resolver::populate_block_bases_from_ancestor(&mut marker_segments, base_text);
        }
        let mut conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(&marker_segments);
        if let Some(session) = &repo.conflict_session {
            let applied = conflict_resolver::apply_session_region_resolutions_with_index_map(
                &mut marker_segments,
                &session.regions,
            );
            conflict_region_indices = applied.block_region_indices;
        }

        let resolved = if marker_segments.is_empty() {
            fallback_resolved
        } else {
            conflict_resolver::generate_resolved_text(&marker_segments)
        };

        let diff_rows = gitgpui_core::file_diff::side_by_side_rows(ours_text, theirs_text);
        let inline_rows = conflict_resolver::build_inline_rows(&diff_rows);
        let (diff_row_conflict_map, inline_row_conflict_map) =
            conflict_resolver::map_two_way_rows_to_conflicts(
                &marker_segments,
                &diff_rows,
                &inline_rows,
            );

        fn split_lines_shared(text: &str) -> Vec<SharedString> {
            if text.is_empty() {
                return Vec::new();
            }
            let mut out =
                Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
            out.extend(text.lines().map(|line| line.to_string().into()));
            out
        }

        let three_way_base_lines = split_lines_shared(base_text);
        let three_way_ours_lines = split_lines_shared(ours_text);
        let three_way_theirs_lines = split_lines_shared(theirs_text);
        let three_way_len = three_way_base_lines
            .len()
            .max(three_way_ours_lines.len())
            .max(three_way_theirs_lines.len());

        let three_way_conflict_ranges = {
            let mut ranges = Vec::new();
            let mut line_offset = 0usize;
            for seg in &marker_segments {
                match seg {
                    conflict_resolver::ConflictSegment::Text(text) => {
                        line_offset += text.lines().count();
                    }
                    conflict_resolver::ConflictSegment::Block(block) => {
                        let count = if block.ours.is_empty() {
                            0
                        } else {
                            block.ours.lines().count()
                        };
                        ranges.push(line_offset..line_offset + count);
                        line_offset += count;
                    }
                }
            }
            ranges
        };

        let view_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.view_mode
        } else if matches!(
            conflict_strategy,
            Some(gitgpui_core::conflict_session::ConflictResolverStrategy::FullTextResolver)
        ) && file.base.is_some()
        {
            ConflictResolverViewMode::ThreeWay
        } else {
            ConflictResolverViewMode::TwoWayDiff
        };

        let hide_resolved = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.hide_resolved
        } else {
            repo.conflict_hide_resolved
        };
        let diff_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.diff_mode
        } else {
            ConflictDiffMode::Split
        };
        let nav_anchor = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.nav_anchor
        } else {
            None
        };
        let active_conflict = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            let total = conflict_resolver::conflict_count(&marker_segments);
            if total == 0 {
                0
            } else {
                self.conflict_resolver.active_conflict.min(total - 1)
            }
        } else {
            0
        };
        let resolver_preview_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.resolver_preview_mode
        } else {
            ConflictResolverPreviewMode::default()
        };

        let (
            three_way_word_highlights_base,
            three_way_word_highlights_ours,
            three_way_word_highlights_theirs,
        ) = conflict_resolver::compute_three_way_word_highlights(
            &three_way_base_lines,
            &three_way_ours_lines,
            &three_way_theirs_lines,
            &marker_segments,
        );
        let diff_word_highlights_split =
            conflict_resolver::compute_two_way_word_highlights(&diff_rows);

        self.conflict_three_way_segments_cache.clear();

        let three_way_visible_map = conflict_resolver::build_three_way_visible_map(
            three_way_len,
            &three_way_conflict_ranges,
            &marker_segments,
            hide_resolved,
        );
        let diff_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &diff_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );
        let inline_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &inline_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );

        self.conflict_resolver = ConflictResolverUiState {
            repo_id: Some(repo_id),
            path: Some(path),
            source_hash: Some(source_hash),
            current: file.current.clone(),
            marker_segments,
            conflict_region_indices,
            active_conflict,
            hovered_conflict: None,
            view_mode,
            diff_rows,
            inline_rows,
            three_way_base_lines,
            three_way_ours_lines,
            three_way_theirs_lines,
            three_way_len,
            three_way_conflict_ranges,
            three_way_word_highlights_base,
            three_way_word_highlights_ours,
            three_way_word_highlights_theirs,
            diff_word_highlights_split,
            diff_mode,
            nav_anchor,
            hide_resolved,
            three_way_visible_map,
            diff_visible_row_indices,
            inline_visible_row_indices,
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: conflict_strategy,
            conflict_kind,
            last_autosolve_summary: None,
            conflict_rev: repo.conflict_rev,
            resolver_pending_recompute_seq: 0,
            resolved_line_meta: Vec::new(),
            resolved_output_conflict_markers: Vec::new(),
            resolved_output_line_sources_index: HashSet::default(),
            resolver_preview_mode,
        };

        let line_ending = crate::kit::TextInput::detect_line_ending(&resolved);
        let theme = self.theme;
        let mut output_hasher = std::collections::hash_map::DefaultHasher::new();
        resolved.hash(&mut output_hasher);
        let output_hash = output_hasher.finish();
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(resolved, cx);
        });
        let output_path = self.conflict_resolver.path.clone();
        self.conflict_resolved_preview_path = output_path.clone();
        self.conflict_resolved_preview_source_hash = Some(output_hash);
        self.schedule_conflict_resolved_outline_recompute(output_path, output_hash, cx);

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
    }

    /// Lightweight re-sync when `conflict_rev` changed but file content is the
    /// same. Re-parses markers, re-applies session resolutions, reads
    /// `hide_resolved` from state, and rebuilds visible maps — without
    /// recomputing the expensive diff rows and word highlights.
    fn resync_conflict_resolver_from_state(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let Loadable::Ready(Some(file)) = &repo.conflict_file else {
            return;
        };

        // Re-parse marker segments from original current text.
        let mut marker_segments = if let Some(cur) = file.current.as_deref() {
            let segments = conflict_resolver::parse_conflict_markers(cur);
            if conflict_resolver::conflict_count(&segments) > 0 {
                segments
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let base_text = file.base.as_deref().unwrap_or("");

        // Re-populate bases from ancestor (needed for 2-way markers).
        if !base_text.is_empty() {
            conflict_resolver::populate_block_bases_from_ancestor(&mut marker_segments, base_text);
        }
        let mut conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(&marker_segments);

        // Re-apply session region resolutions from state.
        if let Some(session) = &repo.conflict_session {
            let applied = conflict_resolver::apply_session_region_resolutions_with_index_map(
                &mut marker_segments,
                &session.regions,
            );
            conflict_region_indices = applied.block_region_indices;
        }

        // Regenerate resolved text.
        let resolved = if marker_segments.is_empty() {
            if let Some(cur) = file.current.as_deref() {
                cur.to_string()
            } else if let Some(ours) = file.ours.as_deref() {
                ours.to_string()
            } else if let Some(theirs) = file.theirs.as_deref() {
                theirs.to_string()
            } else {
                String::new()
            }
        } else {
            conflict_resolver::generate_resolved_text(&marker_segments)
        };

        // Read hide_resolved from state (authoritative source).
        let hide_resolved = repo.conflict_hide_resolved;

        // Recompute three-way conflict ranges from updated marker segments.
        let three_way_conflict_ranges = {
            let mut ranges = Vec::new();
            let mut line_offset = 0usize;
            for seg in &marker_segments {
                match seg {
                    conflict_resolver::ConflictSegment::Text(text) => {
                        line_offset += text.lines().count();
                    }
                    conflict_resolver::ConflictSegment::Block(block) => {
                        let count = if block.ours.is_empty() {
                            0
                        } else {
                            block.ours.lines().count()
                        };
                        ranges.push(line_offset..line_offset + count);
                        line_offset += count;
                    }
                }
            }
            ranges
        };

        // Recompute row→conflict maps using existing diff/inline rows.
        let (diff_row_conflict_map, inline_row_conflict_map) =
            conflict_resolver::map_two_way_rows_to_conflicts(
                &marker_segments,
                &self.conflict_resolver.diff_rows,
                &self.conflict_resolver.inline_rows,
            );

        // Rebuild visible maps.
        let three_way_visible_map = conflict_resolver::build_three_way_visible_map(
            self.conflict_resolver.three_way_len,
            &three_way_conflict_ranges,
            &marker_segments,
            hide_resolved,
        );
        let diff_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &diff_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );
        let inline_visible_row_indices = conflict_resolver::build_two_way_visible_indices(
            &inline_row_conflict_map,
            &marker_segments,
            hide_resolved,
        );

        // Clamp active_conflict to new conflict count.
        let total = conflict_resolver::conflict_count(&marker_segments);
        let active_conflict = if total == 0 {
            0
        } else {
            self.conflict_resolver.active_conflict.min(total - 1)
        };

        let new_rev = repo.conflict_rev;

        // Update only the fields that change during a state re-sync.
        self.conflict_resolver.marker_segments = marker_segments;
        self.conflict_resolver.conflict_region_indices = conflict_region_indices;
        self.conflict_resolver.hide_resolved = hide_resolved;
        self.conflict_resolver.three_way_conflict_ranges = three_way_conflict_ranges;
        self.conflict_resolver.three_way_visible_map = three_way_visible_map;
        self.conflict_resolver.diff_visible_row_indices = diff_visible_row_indices;
        self.conflict_resolver.inline_visible_row_indices = inline_visible_row_indices;
        self.conflict_resolver.active_conflict = active_conflict;
        if self
            .conflict_resolver
            .hovered_conflict
            .is_some_and(|(ix, _)| ix >= total)
        {
            self.conflict_resolver.hovered_conflict = None;
        }
        self.conflict_resolver.conflict_rev = new_rev;

        // Clear segment caches since marker_segments changed.
        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_segments_cache_inline.clear();
        self.conflict_three_way_segments_cache.clear();

        // Update the resolved text input.
        let line_ending = crate::kit::TextInput::detect_line_ending(&resolved);
        let theme = self.theme;
        let mut output_hasher = std::collections::hash_map::DefaultHasher::new();
        resolved.hash(&mut output_hasher);
        let output_hash = output_hasher.finish();
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(resolved, cx);
        });
        let output_path = self.conflict_resolver.path.clone();
        self.conflict_resolved_preview_path = output_path.clone();
        self.conflict_resolved_preview_source_hash = Some(output_hash);
        self.schedule_conflict_resolved_outline_recompute(output_path, output_hash, cx);

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
    }

    pub(in super::super) fn conflict_resolver_set_mode(
        &mut self,
        mode: ConflictDiffMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.diff_mode == mode {
            return;
        }
        self.conflict_resolver.diff_mode = mode;
        self.conflict_resolver.nav_anchor = None;
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_set_view_mode(
        &mut self,
        view_mode: ConflictResolverViewMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.view_mode == view_mode {
            return;
        }
        self.conflict_resolver.view_mode = view_mode;
        self.conflict_resolver.nav_anchor = None;
        self.conflict_resolver.hovered_conflict = None;
        let path = self.conflict_resolver.path.clone();
        self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_toggle_hide_resolved(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.hide_resolved = !self.conflict_resolver.hide_resolved;
        self.conflict_resolver_rebuild_visible_map();
        // If we just hid resolved conflicts, ensure active_conflict points to
        // an unresolved block so the user doesn't stare at a collapsed row.
        if self.conflict_resolver.hide_resolved
            && let Some(next) = conflict_resolver::next_unresolved_conflict_index(
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.active_conflict,
            )
        {
            self.conflict_resolver.active_conflict = next;
        }
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.path.clone(),
        ) {
            self.store.dispatch(Msg::ConflictSetHideResolved {
                repo_id,
                path,
                hide_resolved: self.conflict_resolver.hide_resolved,
            });
        }
        cx.notify();
    }

    fn conflict_resolver_rebuild_visible_map(&mut self) {
        self.conflict_resolver.three_way_visible_map =
            conflict_resolver::build_three_way_visible_map(
                self.conflict_resolver.three_way_len,
                &self.conflict_resolver.three_way_conflict_ranges,
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.hide_resolved,
            );
        let block_count = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter(|seg| matches!(seg, conflict_resolver::ConflictSegment::Block(_)))
            .count();
        if self
            .conflict_resolver
            .hovered_conflict
            .is_some_and(|(ix, _)| ix >= block_count)
        {
            self.conflict_resolver.hovered_conflict = None;
        }
        let (split_map, inline_map) = conflict_resolver::map_two_way_rows_to_conflicts(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.diff_rows,
            &self.conflict_resolver.inline_rows,
        );
        self.conflict_resolver.diff_visible_row_indices =
            conflict_resolver::build_two_way_visible_indices(
                &split_map,
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.hide_resolved,
            );
        self.conflict_resolver.inline_visible_row_indices =
            conflict_resolver::build_two_way_visible_indices(
                &inline_map,
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.hide_resolved,
            );
    }

    pub(in super::super) fn conflict_resolver_apply_pick_target(
        &mut self,
        target: ResolverPickTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        match target {
            ResolverPickTarget::ThreeWayLine { line_ix, choice } => {
                self.conflict_resolver_append_three_way_line_to_output(line_ix, choice, cx);
            }
            ResolverPickTarget::TwoWaySplitLine { row_ix, side } => {
                self.conflict_resolver_append_split_line_to_output(row_ix, side, cx);
            }
            ResolverPickTarget::TwoWayInlineLine { row_ix } => {
                self.conflict_resolver_append_inline_line_to_output(row_ix, cx);
            }
            ResolverPickTarget::Chunk {
                conflict_ix,
                choice,
                output_line_ix,
            } => {
                let target_conflict_ix = if let Some(output_line_ix) = output_line_ix {
                    let current_output = self
                        .conflict_resolver_input
                        .read_with(cx, |i, _| i.text().to_string());
                    self.conflict_resolver_split_chunk_target_for_output_line(
                        conflict_ix,
                        output_line_ix,
                        &current_output,
                    )
                } else {
                    conflict_ix
                };

                let selected_choices =
                    self.conflict_resolver_selected_choices_for_conflict_ix(target_conflict_ix);
                if selected_choices.contains(&choice) {
                    self.conflict_resolver_reset_choice_for_chunk(target_conflict_ix, choice, cx);
                    return;
                }
                if output_line_ix.is_some()
                    && !selected_choices.is_empty()
                    && self.conflict_resolver_append_choice_for_chunk(
                        target_conflict_ix,
                        choice,
                        cx,
                    )
                {
                    return;
                }

                if self.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay {
                    self.conflict_resolver_pick_three_way_chunk_at(target_conflict_ix, choice, cx);
                } else {
                    self.conflict_resolver_pick_at(target_conflict_ix, choice, cx);
                }
            }
        }
    }

    fn conflict_resolver_split_chunk_target_for_output_line(
        &mut self,
        fallback_conflict_ix: usize,
        output_line_ix: usize,
        output_text: &str,
    ) -> usize {
        let Some(marker) = resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            output_text,
            output_line_ix,
        ) else {
            return fallback_conflict_ix;
        };
        let target_conflict_ix = marker.conflict_ix;
        let marker_count_for_conflict =
            resolved_output_markers_for_text(&self.conflict_resolver.marker_segments, output_text)
                .iter()
                .flatten()
                .filter(|m| m.conflict_ix == target_conflict_ix && m.is_start)
                .count();
        if marker_count_for_conflict <= 1 {
            return target_conflict_ix;
        }

        if !split_target_conflict_block_into_subchunks(
            &mut self.conflict_resolver.marker_segments,
            &mut self.conflict_resolver.conflict_region_indices,
            target_conflict_ix,
        ) {
            return target_conflict_ix;
        }
        self.conflict_resolver_rebuild_visible_map();

        resolved_output_marker_for_line(
            &self.conflict_resolver.marker_segments,
            output_text,
            output_line_ix,
        )
        .map(|m| m.conflict_ix)
        .unwrap_or(target_conflict_ix)
    }

    fn conflict_resolver_append_choice_for_chunk(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(inserted_conflict_ix) = append_choice_after_conflict_block(
            &mut self.conflict_resolver.marker_segments,
            &mut self.conflict_resolver.conflict_region_indices,
            conflict_ix,
            choice,
        ) else {
            return false;
        };
        self.conflict_resolver.active_conflict = inserted_conflict_ix;

        let next =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let target_output_line = output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            &next,
            inserted_conflict_ix,
        )
        .map(|range| range.start);
        self.conflict_resolver_set_output(next.clone(), cx);
        if let Some(target_line_ix) = target_output_line {
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(target_line_ix, &next);
        }
        self.conflict_resolver_rebuild_visible_map();
        cx.notify();
        true
    }

    fn conflict_resolver_reset_choice_for_chunk(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut matching_indices = conflict_group_indices_for_choice(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
            choice,
        );
        if matching_indices.is_empty() {
            return;
        }
        matching_indices.sort_unstable();
        matching_indices.dedup();

        let mut changed = false;
        for ix in matching_indices.into_iter().rev() {
            changed |= reset_conflict_block_selection(
                &mut self.conflict_resolver.marker_segments,
                &mut self.conflict_resolver.conflict_region_indices,
                ix,
            );
        }
        if !changed {
            return;
        }

        let total_conflicts =
            conflict_resolver::conflict_count(&self.conflict_resolver.marker_segments);
        self.conflict_resolver.active_conflict = if total_conflicts == 0 {
            0
        } else {
            conflict_ix.min(total_conflicts.saturating_sub(1))
        };

        let next =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let target_output_line = if total_conflicts == 0 {
            None
        } else {
            output_line_range_for_conflict_block_in_text(
                &self.conflict_resolver.marker_segments,
                &next,
                self.conflict_resolver.active_conflict,
            )
            .map(|range| range.start)
        };
        self.conflict_resolver_set_output(next.clone(), cx);
        if let Some(target_line_ix) = target_output_line {
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(target_line_ix, &next);
        }
        self.conflict_resolver_rebuild_visible_map();
        let should_sync_region = self
            .conflict_resolver
            .conflict_region_indices
            .get(self.conflict_resolver.active_conflict)
            .copied()
            .is_some_and(|region_ix| {
                conflict_region_index_is_unique(
                    &self.conflict_resolver.conflict_region_indices,
                    region_ix,
                )
            });
        if should_sync_region {
            self.conflict_resolver_sync_session_resolutions_from_output(&next);
        }
        cx.notify();
    }

    /// Immediately append a single line from the two-way split view to resolved output.
    pub(in super::super) fn conflict_resolver_append_split_line_to_output(
        &mut self,
        row_ix: usize,
        side: ConflictPickSide,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.conflict_resolver.diff_rows.get(row_ix) else {
            return;
        };
        let text = match side {
            ConflictPickSide::Ours => row.old.as_deref(),
            ConflictPickSide::Theirs => row.new.as_deref(),
        };
        let Some(line) = text else {
            return;
        };
        let line_ix = match side {
            ConflictPickSide::Ours => row.old_line,
            ConflictPickSide::Theirs => row.new_line,
        }
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| n.checked_sub(1));
        let choice = match side {
            ConflictPickSide::Ours => conflict_resolver::ConflictChoice::Ours,
            ConflictPickSide::Theirs => conflict_resolver::ConflictChoice::Theirs,
        };
        if let Some(line_ix) = line_ix {
            self.conflict_resolver_output_replace_line(line_ix, choice, cx);
            return;
        }
        let current = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let append_line_ix = source_line_count(&current);
        let next = conflict_resolver::append_lines_to_output(&current, &[line.to_string()]);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(next.clone(), cx);
        });
        self.conflict_resolver_scroll_resolved_output_to_line_in_text(append_line_ix, &next);
    }

    /// Immediately append a single line from the two-way inline view to resolved output.
    pub(in super::super) fn conflict_resolver_append_inline_line_to_output(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.conflict_resolver.inline_rows.get(ix) else {
            return;
        };
        if row.content.is_empty() {
            return;
        }
        let line_ix = row
            .new_line
            .or(row.old_line)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| n.checked_sub(1));
        let choice = match row.side {
            ConflictPickSide::Ours => conflict_resolver::ConflictChoice::Ours,
            ConflictPickSide::Theirs => conflict_resolver::ConflictChoice::Theirs,
        };
        if let Some(line_ix) = line_ix {
            self.conflict_resolver_output_replace_line(line_ix, choice, cx);
            return;
        }
        let current = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let append_line_ix = source_line_count(&current);
        let next =
            conflict_resolver::append_lines_to_output(&current, std::slice::from_ref(&row.content));
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(next.clone(), cx);
        });
        self.conflict_resolver_scroll_resolved_output_to_line_in_text(append_line_ix, &next);
    }

    /// Immediately append a single line from the three-way view to resolved output.
    pub(in super::super) fn conflict_resolver_append_three_way_line_to_output(
        &mut self,
        line_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        let line = match choice {
            conflict_resolver::ConflictChoice::Base => {
                self.conflict_resolver.three_way_base_lines.get(line_ix)
            }
            conflict_resolver::ConflictChoice::Ours => {
                self.conflict_resolver.three_way_ours_lines.get(line_ix)
            }
            conflict_resolver::ConflictChoice::Theirs => {
                self.conflict_resolver.three_way_theirs_lines.get(line_ix)
            }
            conflict_resolver::ConflictChoice::Both => {
                // Both is chunk-level only, not line-level.
                return;
            }
        };
        let Some(_) = line else {
            return;
        };
        self.conflict_resolver_output_replace_line(line_ix, choice, cx);
    }

    pub(in super::super) fn conflict_resolver_set_output(
        &mut self,
        text: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let unchanged = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text() == text);
        let theme = self.theme;
        let next_text = text;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(next_text, cx);
        });
        if unchanged {
            // Choosing a chunk can flip resolved/unresolved state without changing output text.
            // Force marker/provenance refresh so conflict overlays disappear immediately.
            let path = self.conflict_resolver.path.clone();
            self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
            cx.notify();
        }
    }

    /// Delete the current text selection in the resolved output (used by Cut context action).
    pub(in super::super) fn conflict_resolver_output_delete_selection(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let (content, sel_range) = self
            .conflict_resolver_input
            .read_with(cx, |i, _| (i.text().to_string(), i.selected_range()));
        if sel_range.is_empty() {
            return;
        }
        let start = sel_range.start.min(content.len());
        let end = sel_range.end.min(content.len());
        let mut next = content[..start].to_string();
        next.push_str(&content[end..]);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(next, cx);
        });
    }

    /// Paste text into the resolved output at the current cursor position (used by Paste context action).
    pub(in super::super) fn conflict_resolver_output_paste_text(
        &mut self,
        paste_text: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let (content, cursor_offset) = self
            .conflict_resolver_input
            .read_with(cx, |i, _| (i.text().to_string(), i.cursor_offset()));
        let pos = cursor_offset.min(content.len());
        let mut next = content[..pos].to_string();
        next.push_str(paste_text);
        next.push_str(&content[pos..]);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(next, cx);
        });
    }

    /// Replace a line in the resolved output with the source line at the same index from A/B/C.
    pub(in super::super) fn conflict_resolver_output_replace_line(
        &mut self,
        line_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        let is_three_way = self.conflict_resolver.view_mode
            == conflict_resolver::ConflictResolverViewMode::ThreeWay;

        let replacement: Option<String> = if is_three_way {
            match choice {
                conflict_resolver::ConflictChoice::Base => self
                    .conflict_resolver
                    .three_way_base_lines
                    .get(line_ix)
                    .map(|s| s.to_string()),
                conflict_resolver::ConflictChoice::Ours => self
                    .conflict_resolver
                    .three_way_ours_lines
                    .get(line_ix)
                    .map(|s| s.to_string()),
                conflict_resolver::ConflictChoice::Theirs => self
                    .conflict_resolver
                    .three_way_theirs_lines
                    .get(line_ix)
                    .map(|s| s.to_string()),
                conflict_resolver::ConflictChoice::Both => return,
            }
        } else {
            let target_line_no = u32::try_from(line_ix + 1).ok();
            match choice {
                conflict_resolver::ConflictChoice::Ours => self
                    .conflict_resolver
                    .diff_rows
                    .iter()
                    .find(|r| target_line_no.is_some_and(|no| r.old_line == Some(no)))
                    .and_then(|r| r.old.clone())
                    .or_else(|| {
                        self.conflict_resolver
                            .diff_rows
                            .get(line_ix)
                            .and_then(|r| r.old.clone())
                    }),
                conflict_resolver::ConflictChoice::Theirs => self
                    .conflict_resolver
                    .diff_rows
                    .iter()
                    .find(|r| target_line_no.is_some_and(|no| r.new_line == Some(no)))
                    .and_then(|r| r.new.clone())
                    .or_else(|| {
                        self.conflict_resolver
                            .diff_rows
                            .get(line_ix)
                            .and_then(|r| r.new.clone())
                    }),
                _ => return,
            }
        };
        let Some(replacement) = replacement else {
            return;
        };

        let current = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let lines: Vec<&str> = current.split('\n').collect();

        if line_ix < lines.len() {
            let mut next = String::new();
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    next.push('\n');
                }
                if i == line_ix {
                    next.push_str(&replacement);
                } else {
                    next.push_str(line);
                }
            }
            let theme = self.theme;
            self.conflict_resolver_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text(next.clone(), cx);
            });
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(line_ix, &next);
        } else {
            let append_line_ix = source_line_count(&current);
            let next = conflict_resolver::append_lines_to_output(&current, &[replacement]);
            let theme = self.theme;
            self.conflict_resolver_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text(next.clone(), cx);
            });
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(append_line_ix, &next);
        }
    }

    pub(in super::super) fn conflict_resolver_sync_session_resolutions_from_output(
        &mut self,
        output_text: &str,
    ) {
        let Some(repo_id) = self
            .conflict_resolver
            .repo_id
            .or_else(|| self.active_repo_id())
        else {
            return;
        };
        let Some(path) = self.conflict_resolver.path.clone() else {
            return;
        };
        let Some(updates) = conflict_resolver::derive_region_resolution_updates_from_output(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            output_text,
        ) else {
            return;
        };
        if updates.is_empty() {
            return;
        }
        let updates = updates
            .into_iter()
            .map(
                |(region_index, resolution)| gitgpui_state::msg::ConflictRegionResolutionUpdate {
                    region_index,
                    resolution,
                },
            )
            .collect();
        self.store.dispatch(Msg::ConflictSyncRegionResolutions {
            repo_id,
            path,
            updates,
        });
    }

    pub(in super::super) fn conflict_resolver_reset_output_from_markers(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(current) = self.conflict_resolver.current.as_deref() else {
            return;
        };
        let segments = conflict_resolver::parse_conflict_markers(current);
        if conflict_resolver::conflict_count(&segments) == 0 {
            return;
        }
        self.conflict_resolver.marker_segments = segments;
        self.conflict_resolver.conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(
                &self.conflict_resolver.marker_segments,
            );
        self.conflict_resolver.active_conflict = 0;
        self.conflict_resolver.last_autosolve_summary = None;
        self.conflict_resolver_rebuild_visible_map();
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        self.conflict_resolver_set_output(resolved, cx);
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.path.clone(),
        ) {
            self.store
                .dispatch(Msg::ConflictResetResolutions { repo_id, path });
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_conflict_count(&self) -> usize {
        let (total, _) = conflict_resolver::effective_conflict_counts(
            &self.conflict_resolver.marker_segments,
            self.conflict_resolver_session_counts(),
        );
        total
    }

    fn conflict_resolver_session_counts(&self) -> Option<(usize, usize)> {
        let resolver_path = self.conflict_resolver.path.as_ref()?;
        let session = self.active_repo()?.conflict_session.as_ref()?;
        if session.path.as_path() != resolver_path.as_path() {
            return None;
        }
        Some((session.total_regions(), session.solved_count()))
    }

    fn conflict_resolver_active_block_mut(
        &mut self,
    ) -> Option<&mut conflict_resolver::ConflictBlock> {
        let target = self.conflict_resolver.active_conflict;
        let mut seen = 0usize;
        for seg in &mut self.conflict_resolver.marker_segments {
            let conflict_resolver::ConflictSegment::Block(block) = seg else {
                continue;
            };
            if seen == target {
                return Some(block);
            }
            seen += 1;
        }
        None
    }

    pub(in super::super) fn conflict_resolver_pick_at(
        &mut self,
        range_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.active_conflict = range_ix;
        self.conflict_resolver_pick_active_conflict(choice, cx);
    }

    pub(in super::super) fn conflict_resolver_pick_three_way_chunk_at(
        &mut self,
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        if self.conflict_resolver.view_mode != ConflictResolverViewMode::ThreeWay {
            self.conflict_resolver_pick_at(conflict_ix, choice, cx);
            return;
        }

        let Some(block) = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                _ => None,
            })
            .nth(conflict_ix)
        else {
            return;
        };

        let Some(replacement_lines) = replacement_lines_for_conflict_block(block, choice) else {
            return;
        };
        let current_output = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let output_range = output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            &current_output,
            conflict_ix,
        );
        let Some(output_range) = output_range else {
            return;
        };

        self.conflict_resolver.active_conflict = conflict_ix;
        self.conflict_resolver.hovered_conflict = None;
        let picked_region_index = self
            .conflict_resolver
            .conflict_region_indices
            .get(conflict_ix)
            .copied()
            .unwrap_or(conflict_ix);
        let dispatch_region_choice = conflict_region_index_is_unique(
            &self.conflict_resolver.conflict_region_indices,
            picked_region_index,
        );
        {
            let Some(active_block) = self.conflict_resolver_active_block_mut() else {
                return;
            };
            if matches!(choice, conflict_resolver::ConflictChoice::Base)
                && active_block.base.is_none()
            {
                return;
            }
            active_block.choice = choice;
            active_block.resolved = true;
        }
        if dispatch_region_choice
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            let region_choice = match choice {
                conflict_resolver::ConflictChoice::Base => {
                    gitgpui_state::msg::ConflictRegionChoice::Base
                }
                conflict_resolver::ConflictChoice::Ours => {
                    gitgpui_state::msg::ConflictRegionChoice::Ours
                }
                conflict_resolver::ConflictChoice::Theirs => {
                    gitgpui_state::msg::ConflictRegionChoice::Theirs
                }
                conflict_resolver::ConflictChoice::Both => {
                    gitgpui_state::msg::ConflictRegionChoice::Both
                }
            };
            self.store.dispatch(Msg::ConflictSetRegionChoice {
                repo_id,
                path,
                region_index: picked_region_index,
                choice: region_choice,
            });
        }

        let target_output_line = output_range.start;
        let next = replace_output_lines_in_range(&current_output, output_range, &replacement_lines);
        self.conflict_resolver_set_output(next.clone(), cx);
        self.conflict_resolver_scroll_resolved_output_to_line_in_text(target_output_line, &next);
        self.conflict_resolver_rebuild_visible_map();

        // Auto-advance to the next unresolved conflict (kdiff3-style).
        let current = self.conflict_resolver.active_conflict;
        if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
            &self.conflict_resolver.marker_segments,
            current,
        )
        .filter(|&next| next != current)
        {
            self.conflict_resolver.active_conflict = next_unresolved;
            let target_visible_ix = conflict_resolver::visible_index_for_conflict(
                &self.conflict_resolver.three_way_visible_map,
                &self.conflict_resolver.three_way_conflict_ranges,
                self.conflict_resolver.active_conflict,
            );
            if let Some(vi) = target_visible_ix {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(vi, gpui::ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_pick_active_conflict(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        let picked_conflict_index = self.conflict_resolver.active_conflict;
        let picked_region_index = self
            .conflict_resolver
            .conflict_region_indices
            .get(picked_conflict_index)
            .copied()
            .unwrap_or(picked_conflict_index);
        let dispatch_region_choice = conflict_region_index_is_unique(
            &self.conflict_resolver.conflict_region_indices,
            picked_region_index,
        );
        {
            let Some(block) = self.conflict_resolver_active_block_mut() else {
                return;
            };
            if matches!(choice, conflict_resolver::ConflictChoice::Base) && block.base.is_none() {
                return;
            }
            block.choice = choice;
            block.resolved = true;
        }
        if dispatch_region_choice
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            let region_choice = match choice {
                conflict_resolver::ConflictChoice::Base => {
                    gitgpui_state::msg::ConflictRegionChoice::Base
                }
                conflict_resolver::ConflictChoice::Ours => {
                    gitgpui_state::msg::ConflictRegionChoice::Ours
                }
                conflict_resolver::ConflictChoice::Theirs => {
                    gitgpui_state::msg::ConflictRegionChoice::Theirs
                }
                conflict_resolver::ConflictChoice::Both => {
                    gitgpui_state::msg::ConflictRegionChoice::Both
                }
            };
            self.store.dispatch(Msg::ConflictSetRegionChoice {
                repo_id,
                path,
                region_index: picked_region_index,
                choice: region_choice,
            });
        }
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let target_output_line = output_line_range_for_conflict_block_in_text(
            &self.conflict_resolver.marker_segments,
            &resolved,
            picked_conflict_index,
        )
        .map(|range| range.start);
        self.conflict_resolver_set_output(resolved.clone(), cx);
        if let Some(target_line_ix) = target_output_line {
            self.conflict_resolver_scroll_resolved_output_to_line_in_text(
                target_line_ix,
                &resolved,
            );
        }
        self.conflict_resolver_rebuild_visible_map();

        // Auto-advance to the next unresolved conflict (kdiff3-style).
        let current = self.conflict_resolver.active_conflict;
        if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
            &self.conflict_resolver.marker_segments,
            current,
        )
        .filter(|&next| next != current)
        {
            self.conflict_resolver.active_conflict = next_unresolved;
            let target_visible_ix = match self.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => {
                    conflict_resolver::visible_index_for_conflict(
                        &self.conflict_resolver.three_way_visible_map,
                        &self.conflict_resolver.three_way_conflict_ranges,
                        self.conflict_resolver.active_conflict,
                    )
                }
                ConflictResolverViewMode::TwoWayDiff => self
                    .conflict_resolver_two_way_visible_ix_for_conflict(
                        self.conflict_resolver.active_conflict,
                    ),
            };
            if let Some(vi) = target_visible_ix {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(vi, gpui::ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_resolved_count(&self) -> usize {
        let (_, resolved) = conflict_resolver::effective_conflict_counts(
            &self.conflict_resolver.marker_segments,
            self.conflict_resolver_session_counts(),
        );
        resolved
    }

    fn dispatch_conflict_autosolve_telemetry(
        &self,
        mode: gitgpui_state::msg::ConflictAutosolveMode,
        total_conflicts_before: usize,
        total_conflicts_after: usize,
        unresolved_before: usize,
        unresolved_after: usize,
        stats: gitgpui_state::msg::ConflictAutosolveStats,
    ) {
        let Some(repo_id) = self
            .conflict_resolver
            .repo_id
            .or_else(|| self.active_repo_id())
        else {
            return;
        };
        self.store.dispatch(Msg::RecordConflictAutosolveTelemetry {
            repo_id,
            path: self.conflict_resolver.path.clone(),
            mode,
            total_conflicts_before,
            total_conflicts_after,
            unresolved_before,
            unresolved_after,
            stats,
        });
    }

    /// Apply safe auto-resolve rules to all unresolved conflict blocks.
    /// Updates the resolved output text and notifies the UI.
    pub(in super::super) fn conflict_resolver_auto_resolve(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver_auto_resolve_inner(false, cx);
    }

    /// Apply safe + regex-assisted auto-resolve rules (explicit opt-in).
    pub(in super::super) fn conflict_resolver_auto_resolve_regex(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_enable_regex_autosolve {
            return;
        }
        self.conflict_resolver_auto_resolve_inner(true, cx);
    }

    fn conflict_resolver_auto_resolve_inner(
        &mut self,
        include_regex_pass: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let total_before = self.conflict_resolver_conflict_count();
        if total_before == 0 {
            return;
        }
        let unresolved_before =
            total_before.saturating_sub(self.conflict_resolver_resolved_count());
        let ws = self.conflict_enable_whitespace_autosolve;
        // Pass 1: safe whole-block auto-resolve.
        let pass1 = conflict_resolver::auto_resolve_segments_with_options(
            &mut self.conflict_resolver.marker_segments,
            ws,
        );
        // Pass 2: heuristic subchunk splitting — split remaining unresolved
        // blocks into finer line-level subchunks where possible.
        let pass2 = conflict_resolver::auto_resolve_segments_pass2_with_region_indices(
            &mut self.conflict_resolver.marker_segments,
            &mut self.conflict_resolver.conflict_region_indices,
        );
        let pass1_after_split = if pass2 > 0 {
            // Re-run Pass 1 on newly created sub-blocks (they may now
            // satisfy whole-block rules after splitting).
            conflict_resolver::auto_resolve_segments_with_options(
                &mut self.conflict_resolver.marker_segments,
                ws,
            )
        } else {
            0
        };
        let regex = if include_regex_pass {
            let options =
                gitgpui_core::conflict_session::RegexAutosolveOptions::whitespace_insensitive();
            conflict_resolver::auto_resolve_segments_regex(
                &mut self.conflict_resolver.marker_segments,
                &options,
            )
        } else {
            0
        };
        let count = pass1 + pass2 + pass1_after_split + regex;
        if count > 0 {
            let resolved =
                conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
            self.conflict_resolver_set_output(resolved, cx);
            self.conflict_resolver_rebuild_visible_map();
            // Keep focus aligned with unresolved navigation after auto-resolve.
            if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.active_conflict,
            ) {
                self.conflict_resolver.active_conflict = next_unresolved;
            }
        }
        let total_after = self.conflict_resolver_conflict_count();
        let unresolved_after = total_after.saturating_sub(self.conflict_resolver_resolved_count());
        let stats = gitgpui_state::msg::ConflictAutosolveStats {
            pass1,
            pass2_split: pass2,
            pass1_after_split,
            regex,
            history: 0,
        };
        let trace_mode = if include_regex_pass {
            conflict_resolver::AutosolveTraceMode::Regex
        } else {
            conflict_resolver::AutosolveTraceMode::Safe
        };
        self.conflict_resolver.last_autosolve_summary = Some(
            conflict_resolver::format_autosolve_trace_summary(
                trace_mode,
                unresolved_before,
                unresolved_after,
                &stats,
            )
            .into(),
        );
        self.dispatch_conflict_autosolve_telemetry(
            if include_regex_pass {
                gitgpui_state::msg::ConflictAutosolveMode::Regex
            } else {
                gitgpui_state::msg::ConflictAutosolveMode::Safe
            },
            total_before,
            total_after,
            unresolved_before,
            unresolved_after,
            stats,
        );
        if count > 0
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            self.store.dispatch(Msg::ConflictApplyAutosolve {
                repo_id,
                path,
                mode: if include_regex_pass {
                    gitgpui_state::msg::ConflictAutosolveMode::Regex
                } else {
                    gitgpui_state::msg::ConflictAutosolveMode::Safe
                },
                whitespace_normalize: ws,
            });
        }
        cx.notify();
    }

    /// Apply history-aware auto-resolve to unresolved conflict blocks.
    /// Detects changelog/history sections and merges entries by deduplication.
    pub(in super::super) fn conflict_resolver_auto_resolve_history(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_enable_history_autosolve {
            return;
        }
        let total_before = self.conflict_resolver_conflict_count();
        if total_before == 0 {
            return;
        }
        let unresolved_before =
            total_before.saturating_sub(self.conflict_resolver_resolved_count());
        // Use bullet_list preset as default; in a real settings integration
        // this would come from user configuration.
        let options = gitgpui_core::conflict_session::HistoryAutosolveOptions::bullet_list();
        let count = conflict_resolver::auto_resolve_segments_history_with_region_indices(
            &mut self.conflict_resolver.marker_segments,
            &options,
            &mut self.conflict_resolver.conflict_region_indices,
        );
        if count > 0 {
            let resolved =
                conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
            self.conflict_resolver_set_output(resolved, cx);
            self.conflict_resolver_rebuild_visible_map();
            if let Some(next_unresolved) = conflict_resolver::next_unresolved_conflict_index(
                &self.conflict_resolver.marker_segments,
                self.conflict_resolver.active_conflict,
            ) {
                self.conflict_resolver.active_conflict = next_unresolved;
            }
        }
        let total_after = self.conflict_resolver_conflict_count();
        let unresolved_after = total_after.saturating_sub(self.conflict_resolver_resolved_count());
        let stats = gitgpui_state::msg::ConflictAutosolveStats {
            pass1: 0,
            pass2_split: 0,
            pass1_after_split: 0,
            regex: 0,
            history: count,
        };
        self.conflict_resolver.last_autosolve_summary = Some(
            conflict_resolver::format_autosolve_trace_summary(
                conflict_resolver::AutosolveTraceMode::History,
                unresolved_before,
                unresolved_after,
                &stats,
            )
            .into(),
        );
        self.dispatch_conflict_autosolve_telemetry(
            gitgpui_state::msg::ConflictAutosolveMode::History,
            total_before,
            total_after,
            unresolved_before,
            unresolved_after,
            stats,
        );
        if count > 0
            && let (Some(repo_id), Some(path)) = (
                self.conflict_resolver
                    .repo_id
                    .or_else(|| self.active_repo_id()),
                self.conflict_resolver.path.clone(),
            )
        {
            self.store.dispatch(Msg::ConflictApplyAutosolve {
                repo_id,
                path,
                mode: gitgpui_state::msg::ConflictAutosolveMode::History,
                whitespace_normalize: false,
            });
        }
        cx.notify();
    }
}

impl Render for MainPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        debug_assert!(matches!(
            self.view_mode,
            GitGpuiViewMode::Normal | GitGpuiViewMode::FocusedMergetool
        ));
        self.last_window_size = window.window_bounds().get_bounds().size;
        self.history_view
            .update(cx, |v, _| v.set_last_window_size(self.last_window_size));

        let show_diff = self
            .active_repo()
            .and_then(|r| r.diff_target.as_ref())
            .is_some();
        if show_diff {
            div().size_full().child(self.diff_view(cx))
        } else {
            div().size_full().child(self.history_view.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClearDiffSelectionAction, ResolvedOutputConflictMarker,
        apply_conflict_choice_provenance_hints, apply_three_way_empty_base_provenance_hints,
        build_resolved_output_conflict_markers, clear_diff_selection_action,
        conflict_marker_nav_entries_from_markers, conflict_resolver_output_context_line,
        focused_mergetool_save_exit_code, output_line_range_for_conflict_block_in_text,
        replace_output_lines_in_range, resolved_output_marker_for_line,
        resolved_output_markers_for_text, split_target_conflict_block_into_subchunks,
    };
    use crate::view::GitGpuiViewMode;
    use crate::view::conflict_resolver::{
        self, ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
        ResolvedLineSource, SourceLines,
    };

    #[test]
    fn clear_diff_selection_action_is_clear_for_normal_mode() {
        assert_eq!(
            clear_diff_selection_action(GitGpuiViewMode::Normal),
            ClearDiffSelectionAction::ClearSelection
        );
    }

    #[test]
    fn clear_diff_selection_action_exits_focused_mergetool_mode() {
        assert_eq!(
            clear_diff_selection_action(GitGpuiViewMode::FocusedMergetool),
            ClearDiffSelectionAction::ExitFocusedMergetool
        );
    }

    #[test]
    fn focused_mergetool_save_exit_code_is_success_when_all_resolved() {
        assert_eq!(focused_mergetool_save_exit_code(0, 0), 0);
        assert_eq!(focused_mergetool_save_exit_code(3, 3), 0);
    }

    #[test]
    fn focused_mergetool_save_exit_code_is_canceled_when_unresolved_remain() {
        assert_eq!(focused_mergetool_save_exit_code(3, 2), 1);
    }

    #[test]
    fn replace_output_lines_in_range_replaces_only_target_chunk() {
        let output = "top\nkeep\nalso-keep\nbottom";
        let replacement = vec!["picked".to_string()];
        let next = replace_output_lines_in_range(output, 1..3, &replacement);
        assert_eq!(next, "top\npicked\nbottom");
    }

    #[test]
    fn replace_output_lines_in_range_preserves_trailing_newline() {
        let output = "a\nb\n";
        let replacement = vec!["x".to_string(), "y".to_string()];
        let next = replace_output_lines_in_range(output, 1..2, &replacement);
        assert_eq!(next, "a\nx\ny\n");
    }

    #[test]
    fn output_line_range_for_conflict_block_in_text_maps_middle_blocks_exactly() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\n".to_string(),
                theirs: "x\ny\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("mid\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\nc\n".to_string(),
                theirs: "z\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("tail\n".to_string()),
        ];

        let output = conflict_resolver::generate_resolved_text(&segments);
        assert_eq!(
            output_line_range_for_conflict_block_in_text(&segments, &output, 0),
            Some(1..2)
        );
        assert_eq!(
            output_line_range_for_conflict_block_in_text(&segments, &output, 1),
            Some(3..5)
        );
    }

    #[test]
    fn output_line_range_for_conflict_block_in_text_maps_eof_block_without_newline() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "tail".to_string(),
                theirs: "other".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
        ];

        let output = conflict_resolver::generate_resolved_text(&segments);
        assert_eq!(
            output_line_range_for_conflict_block_in_text(&segments, &output, 0),
            Some(1..2)
        );
    }

    #[test]
    fn output_line_range_for_conflict_block_in_text_returns_none_when_output_drifts() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\n".to_string(),
                theirs: "x\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("mid\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\n".to_string(),
                theirs: "y\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
        ];

        let drifted_output = "top\ndrift\nmid\nb\n";
        assert_eq!(
            output_line_range_for_conflict_block_in_text(&segments, drifted_output, 1),
            None
        );
    }

    #[test]
    fn build_resolved_output_conflict_markers_maps_chunk_boundaries() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\n".to_string(),
                theirs: "x\ny\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("mid\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\nc\n".to_string(),
                theirs: "z\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("tail\n".to_string()),
        ];

        let output = conflict_resolver::generate_resolved_text(&segments);
        let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
        let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

        assert_eq!(
            markers[1],
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 0,
                range_start: 1,
                range_end: 2,
                is_start: true,
                is_end: true,
                unresolved: false,
            })
        );
        assert_eq!(
            markers[3],
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 1,
                range_start: 3,
                range_end: 5,
                is_start: true,
                is_end: false,
                unresolved: false,
            })
        );
        assert_eq!(
            markers[4],
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 1,
                range_start: 3,
                range_end: 5,
                is_start: false,
                is_end: true,
                unresolved: false,
            })
        );
    }

    #[test]
    fn build_resolved_output_conflict_markers_anchors_zero_length_ranges() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some(String::new()),
                ours: String::new(),
                theirs: "x\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("tail\n".to_string()),
        ];

        let output = conflict_resolver::generate_resolved_text(&segments);
        let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
        let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

        assert_eq!(
            markers[1],
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 0,
                range_start: 1,
                range_end: 1,
                is_start: true,
                is_end: true,
                unresolved: false,
            })
        );
    }

    #[test]
    fn build_resolved_output_conflict_markers_marks_unresolved_blocks() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\n".to_string(),
                theirs: "x\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("tail\n".to_string()),
        ];

        let output = conflict_resolver::generate_resolved_text(&segments);
        let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
        let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

        assert_eq!(
            markers[1],
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 0,
                range_start: 1,
                range_end: 2,
                is_start: true,
                is_end: true,
                unresolved: true,
            })
        );
    }

    #[test]
    fn conflict_marker_nav_entries_include_only_marker_starts() {
        let markers = vec![
            None,
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 0,
                range_start: 1,
                range_end: 3,
                is_start: true,
                is_end: false,
                unresolved: true,
            }),
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 0,
                range_start: 1,
                range_end: 3,
                is_start: false,
                is_end: true,
                unresolved: true,
            }),
            Some(ResolvedOutputConflictMarker {
                conflict_ix: 1,
                range_start: 3,
                range_end: 4,
                is_start: true,
                is_end: true,
                unresolved: false,
            }),
        ];
        assert_eq!(
            conflict_marker_nav_entries_from_markers(&markers),
            vec![1, 3]
        );
    }

    #[test]
    fn conflict_resolver_output_context_line_prefers_clicked_offset() {
        let content = "top\nmiddle\nbottom\n";
        let cursor_offset = 0usize;
        let clicked_offset = "top\nmiddle\n".len();
        assert_eq!(
            conflict_resolver_output_context_line(content, cursor_offset, Some(clicked_offset)),
            2
        );
        assert_eq!(
            conflict_resolver_output_context_line(content, "top\n".len(), None),
            1
        );
    }

    #[test]
    fn clicked_unresolved_line_maps_to_chunk_marker() {
        let segments = vec![
            ConflictSegment::Text("top\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours-1\nours-2\n".to_string(),
                theirs: "theirs-1\ntheirs-2\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("tail\n".to_string()),
        ];
        let output = conflict_resolver::generate_resolved_text(&segments);
        let cursor_offset = 0usize;
        let clicked_offset = "top\nours-1\n".len();
        let clicked_line =
            conflict_resolver_output_context_line(&output, cursor_offset, Some(clicked_offset));
        let marker =
            resolved_output_marker_for_line(&segments, &output, clicked_line).expect("marker");
        assert!(marker.unresolved);
        assert_eq!(marker.conflict_ix, 0);
    }

    #[test]
    fn build_resolved_output_conflict_markers_splits_unresolved_subchunks() {
        let segments = vec![
            ConflictSegment::Text("pre\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("a\ncommon\nb\n".to_string()),
                ours: "ao\ncommon\nbo\n".to_string(),
                theirs: "at\ncommon\nbt\n".to_string(),
                choice: ConflictChoice::Base,
                resolved: false,
            }),
            ConflictSegment::Text("post\n".to_string()),
        ];

        let output = conflict_resolver::generate_resolved_text(&segments);
        let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
        let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

        let starts = markers
            .iter()
            .flatten()
            .filter(|m| m.conflict_ix == 0 && m.is_start)
            .count();
        assert_eq!(starts, 2, "expected two unresolved subchunk starts");
        assert!(
            markers.get(2).is_some_and(|m| m.is_none()),
            "resolved middle line should not be marked as conflict"
        );
    }

    #[test]
    fn build_resolved_output_conflict_markers_splits_method_edit_and_trailing_insertion() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some(
                "pub fn opposite(self) -> Color {\n    match self {\n        Color::White => Color::Black,\n        Color::Black => Color::White,\n    }\n}\n"
                    .to_string(),
            ),
            ours: "pub fn opposite(self) -> Color {\n    match self {\n        Color::White => Color::Black,\n        Color::Black => Color::White,\n    }\n}\n"
                .to_string(),
            theirs: "pub fn opposite(self) -> Self {\n    match self {\n        Self::White => Self::Black,\n        Self::Black => Self::White,\n    }\n}\n\npub fn name(self) -> &'static str {\n    match self {\n        Self::White => \"White\",\n        Self::Black => \"Black\",\n    }\n}\n"
                .to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];

        let output = conflict_resolver::generate_resolved_text(&segments);
        let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
        let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

        let starts = markers
            .iter()
            .flatten()
            .filter(|m| m.conflict_ix == 0 && m.is_start)
            .count();
        assert_eq!(starts, 2, "expected two decision marker starts");
    }

    #[test]
    fn build_resolved_output_conflict_markers_matches_combined_conflict_marker_case() {
        let conflict_text = "impl Color {\n<<<<<<< HEAD\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n=======\n    pub fn opposite(self) -> Self {\n        match self {\n            Self::White => Self::Black,\n            Self::Black => Self::White,\n        }\n    }\n\n    pub fn name(self) -> &'static str {\n        match self {\n            Self::White => \"White\",\n            Self::Black => \"Black\",\n>>>>>>> origin/version2\n        }\n    }\n}\n";
        let base_text = "impl Color {\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n        }\n    }\n}\n";
        let mut segments = conflict_resolver::parse_conflict_markers(conflict_text);
        conflict_resolver::populate_block_bases_from_ancestor(&mut segments, base_text);

        let output = conflict_resolver::generate_resolved_text(&segments);
        let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
        let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);
        let starts = markers
            .iter()
            .flatten()
            .filter(|m| m.conflict_ix == 0 && m.is_start)
            .count();
        assert_eq!(starts, 2, "expected two marker starts for impl Color case");
    }

    #[test]
    fn split_target_conflict_block_into_subchunks_isolates_close_markers() {
        let conflict_text = "impl Color {\n<<<<<<< HEAD\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n=======\n    pub fn opposite(self) -> Self {\n        match self {\n            Self::White => Self::Black,\n            Self::Black => Self::White,\n        }\n    }\n\n    pub fn name(self) -> &'static str {\n        match self {\n            Self::White => \"White\",\n            Self::Black => \"Black\",\n>>>>>>> origin/version2\n        }\n    }\n}\n";
        let base_text = "impl Color {\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n        }\n    }\n}\n";
        let mut segments = conflict_resolver::parse_conflict_markers(conflict_text);
        conflict_resolver::populate_block_bases_from_ancestor(&mut segments, base_text);
        let mut region_indices = conflict_resolver::sequential_conflict_region_indices(&segments);
        let output_before = conflict_resolver::generate_resolved_text(&segments);

        let before_markers = resolved_output_markers_for_text(&segments, &output_before);
        let before_starts = before_markers
            .iter()
            .flatten()
            .filter(|m| m.conflict_ix == 0 && m.is_start)
            .count();
        assert_eq!(
            before_starts, 2,
            "fixture should begin with two close markers"
        );

        assert!(
            split_target_conflict_block_into_subchunks(&mut segments, &mut region_indices, 0),
            "expected target block to split"
        );

        assert_eq!(conflict_resolver::conflict_count(&segments), 2);
        assert_eq!(region_indices, vec![0, 0]);
        let output_after = conflict_resolver::generate_resolved_text(&segments);
        assert_eq!(
            output_after, output_before,
            "split should preserve output text"
        );

        let after_markers = resolved_output_markers_for_text(&segments, &output_after);
        let mut starts_by_conflict: std::collections::BTreeMap<usize, usize> =
            std::collections::BTreeMap::new();
        for marker in after_markers.iter().flatten().filter(|m| m.is_start) {
            *starts_by_conflict.entry(marker.conflict_ix).or_default() += 1;
        }
        assert_eq!(starts_by_conflict.get(&0).copied(), Some(1));
        assert_eq!(starts_by_conflict.get(&1).copied(), Some(1));
    }

    #[test]
    fn conflict_region_index_is_unique_detects_split_subchunk_duplicates() {
        assert!(super::conflict_region_index_is_unique(&[0], 0));
        assert!(super::conflict_region_index_is_unique(&[0, 1], 0));
        assert!(!super::conflict_region_index_is_unique(&[0, 0], 0));
    }

    #[test]
    fn append_choice_after_conflict_block_appends_selected_order_for_single_marker() {
        let mut segments = vec![
            ConflictSegment::Text("pre\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("post\n".to_string()),
        ];
        let mut region_indices = vec![0];

        let inserted_ix = super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            0,
            ConflictChoice::Theirs,
        );

        assert_eq!(inserted_ix, Some(1));
        assert_eq!(conflict_resolver::conflict_count(&segments), 2);
        assert_eq!(region_indices, vec![0, 0]);
        let output = conflict_resolver::generate_resolved_text(&segments);
        assert_eq!(output, "pre\nours\ntheirs\npost\n");
    }

    #[test]
    fn append_choice_after_conflict_block_from_same_marker_keeps_single_choice_per_side() {
        let mut segments = vec![
            ConflictSegment::Text("pre\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Base,
                resolved: true,
            }),
            ConflictSegment::Text("post\n".to_string()),
        ];
        let mut region_indices = vec![0];

        assert_eq!(
            super::append_choice_after_conflict_block(
                &mut segments,
                &mut region_indices,
                0,
                ConflictChoice::Ours,
            ),
            Some(1)
        );
        assert_eq!(
            super::append_choice_after_conflict_block(
                &mut segments,
                &mut region_indices,
                0,
                ConflictChoice::Theirs,
            ),
            Some(2)
        );
        // Picking C again from the same marker should not append duplicate chunks.
        assert_eq!(
            super::append_choice_after_conflict_block(
                &mut segments,
                &mut region_indices,
                0,
                ConflictChoice::Theirs,
            ),
            None
        );

        assert_eq!(
            super::conflict_group_selected_choices_for_ix(&segments, &region_indices, 0),
            vec![
                ConflictChoice::Base,
                ConflictChoice::Ours,
                ConflictChoice::Theirs
            ]
        );
        assert_eq!(conflict_resolver::conflict_count(&segments), 3);
        assert_eq!(
            conflict_resolver::generate_resolved_text(&segments),
            "pre\nbase\nours\ntheirs\npost\n"
        );
    }

    #[test]
    fn non_contiguous_matching_blocks_do_not_share_choice_group() {
        let mut segments = vec![
            ConflictSegment::Text("pre\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Theirs,
                resolved: true,
            }),
            ConflictSegment::Text("middle\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("post\n".to_string()),
        ];
        // Simulate subchunk-derived duplicate region ids while preserving a text boundary.
        let mut region_indices = vec![0, 0];

        assert_eq!(
            super::conflict_group_selected_choices_for_ix(&segments, &region_indices, 1),
            Vec::<ConflictChoice>::new()
        );

        assert!(
            super::reset_conflict_block_selection(&mut segments, &mut region_indices, 0),
            "resetting first block should not remove it due later non-contiguous match"
        );
        assert_eq!(conflict_resolver::conflict_count(&segments), 2);
    }

    #[test]
    fn adjacent_markers_with_same_text_but_different_regions_do_not_interfere() {
        let mut segments = vec![
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Theirs,
                resolved: true,
            }),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
        ];
        let mut region_indices = vec![10, 11];

        assert_eq!(
            super::conflict_group_selected_choices_for_ix(&segments, &region_indices, 1),
            Vec::<ConflictChoice>::new()
        );
        assert_eq!(
            super::conflict_group_indices_for_choice(
                &segments,
                &region_indices,
                1,
                ConflictChoice::Theirs
            ),
            Vec::<usize>::new()
        );

        assert_eq!(
            super::append_choice_after_conflict_block(
                &mut segments,
                &mut region_indices,
                1,
                ConflictChoice::Theirs,
            ),
            None
        );
        assert_eq!(conflict_resolver::conflict_count(&segments), 2);
    }

    #[test]
    fn pick_sequence_is_reversible_to_original_unpicked_state() {
        let mut segments = vec![
            ConflictSegment::Text("pre\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("post\n".to_string()),
        ];
        let original = segments.clone();
        let mut region_indices = vec![0];

        // Pick A.
        let target = segments.iter_mut().find_map(|seg| match seg {
            ConflictSegment::Block(block) => Some(block),
            _ => None,
        });
        if let Some(block) = target {
            block.choice = ConflictChoice::Base;
            block.resolved = true;
        } else {
            panic!("expected conflict block");
        }
        // Pick B then C in order.
        assert_eq!(
            super::append_choice_after_conflict_block(
                &mut segments,
                &mut region_indices,
                0,
                ConflictChoice::Ours,
            ),
            Some(1)
        );
        assert_eq!(
            super::append_choice_after_conflict_block(
                &mut segments,
                &mut region_indices,
                1,
                ConflictChoice::Theirs,
            ),
            Some(2)
        );
        assert_eq!(
            conflict_resolver::generate_resolved_text(&segments),
            "pre\nbase\nours\ntheirs\npost\n"
        );

        // Deselect A, then B, then C.
        assert!(super::reset_conflict_block_selection(
            &mut segments,
            &mut region_indices,
            0
        ));
        assert!(super::reset_conflict_block_selection(
            &mut segments,
            &mut region_indices,
            0
        ));
        assert!(super::reset_conflict_block_selection(
            &mut segments,
            &mut region_indices,
            0
        ));

        assert_eq!(segments, original);
        assert_eq!(region_indices, vec![0]);
        assert_eq!(
            conflict_resolver::generate_resolved_text(&segments),
            conflict_resolver::generate_resolved_text(&original)
        );
    }

    #[test]
    fn pick_and_deselect_multiple_orders_always_restore_original_state() {
        fn initial_segments() -> Vec<ConflictSegment> {
            vec![
                ConflictSegment::Text("pre\n".to_string()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("base\n".to_string()),
                    ours: "ours\n".to_string(),
                    theirs: "theirs\n".to_string(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                }),
                ConflictSegment::Text("post\n".to_string()),
            ]
        }

        fn find_conflict_ix_by_choice(
            segments: &[ConflictSegment],
            choice: ConflictChoice,
        ) -> Option<usize> {
            segments
                .iter()
                .filter_map(|seg| match seg {
                    ConflictSegment::Block(block) => Some(block),
                    _ => None,
                })
                .enumerate()
                .find_map(|(ix, block)| (block.resolved && block.choice == choice).then_some(ix))
        }

        fn apply_pick_sequence(
            segments: &mut Vec<ConflictSegment>,
            region_indices: &mut Vec<usize>,
            picks: &[ConflictChoice],
        ) {
            let mut current_ix = 0usize;
            for (ix, choice) in picks.iter().copied().enumerate() {
                if ix == 0 {
                    let target = segments.iter_mut().find_map(|seg| match seg {
                        ConflictSegment::Block(block) => Some(block),
                        _ => None,
                    });
                    if let Some(block) = target {
                        block.choice = choice;
                        block.resolved = true;
                    } else {
                        panic!("expected conflict block");
                    }
                    continue;
                }
                let inserted_ix = super::append_choice_after_conflict_block(
                    segments,
                    region_indices,
                    current_ix,
                    choice,
                );
                assert_eq!(inserted_ix, Some(current_ix.saturating_add(1)));
                current_ix = inserted_ix.unwrap_or(current_ix);
            }
        }

        let original = initial_segments();
        let cases: Vec<(Vec<ConflictChoice>, Vec<ConflictChoice>)> = vec![
            // Full three-pick flows in different select/deselect orders.
            (
                vec![
                    ConflictChoice::Base,
                    ConflictChoice::Ours,
                    ConflictChoice::Theirs,
                ],
                vec![
                    ConflictChoice::Base,
                    ConflictChoice::Ours,
                    ConflictChoice::Theirs,
                ],
            ),
            (
                vec![
                    ConflictChoice::Base,
                    ConflictChoice::Ours,
                    ConflictChoice::Theirs,
                ],
                vec![
                    ConflictChoice::Theirs,
                    ConflictChoice::Ours,
                    ConflictChoice::Base,
                ],
            ),
            (
                vec![
                    ConflictChoice::Theirs,
                    ConflictChoice::Base,
                    ConflictChoice::Ours,
                ],
                vec![
                    ConflictChoice::Base,
                    ConflictChoice::Theirs,
                    ConflictChoice::Ours,
                ],
            ),
            (
                vec![
                    ConflictChoice::Ours,
                    ConflictChoice::Theirs,
                    ConflictChoice::Base,
                ],
                vec![
                    ConflictChoice::Base,
                    ConflictChoice::Ours,
                    ConflictChoice::Theirs,
                ],
            ),
            // Repeated two-pick cycle case.
            (
                vec![ConflictChoice::Ours, ConflictChoice::Theirs],
                vec![ConflictChoice::Theirs, ConflictChoice::Ours],
            ),
        ];

        for (picks, deselects) in cases {
            // Run each case twice to cover repeated select/deselect cycles.
            for _ in 0..2 {
                let mut segments = original.clone();
                let mut region_indices = vec![0];

                apply_pick_sequence(&mut segments, &mut region_indices, &picks);

                for deselect_choice in deselects.iter().copied() {
                    let Some(conflict_ix) = find_conflict_ix_by_choice(&segments, deselect_choice)
                    else {
                        panic!(
                            "expected to find selected conflict for {:?}",
                            deselect_choice
                        );
                    };
                    assert!(
                        super::reset_conflict_block_selection(
                            &mut segments,
                            &mut region_indices,
                            conflict_ix
                        ),
                        "expected deselect to succeed for {:?}",
                        deselect_choice
                    );
                }

                assert_eq!(segments, original);
                assert_eq!(region_indices, vec![0]);
                assert_eq!(
                    conflict_resolver::generate_resolved_text(&segments),
                    conflict_resolver::generate_resolved_text(&original)
                );
            }
        }
    }

    #[test]
    fn conflict_choice_hints_override_identical_text_to_selected_source() {
        fn shared(s: &str) -> gpui::SharedString {
            s.to_string().into()
        }

        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("same\n".to_string()),
            ours: "same\n".to_string(),
            theirs: "same\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: true,
        })];
        let output = conflict_resolver::generate_resolved_text(&segments);
        let output_lines = conflict_resolver::split_output_lines_for_outline(&output);
        let sources = SourceLines {
            a: &[shared("same")],
            b: &[shared("same")],
            c: &[shared("same")],
        };

        let mut meta = conflict_resolver::compute_resolved_line_provenance(&output_lines, &sources);
        // Raw text matching alone picks A because A has higher matching priority.
        assert_eq!(meta[0].source, ResolvedLineSource::A);

        apply_conflict_choice_provenance_hints(
            &mut meta,
            &segments,
            &output,
            ConflictResolverViewMode::ThreeWay,
        );

        assert_eq!(meta[0].source, ResolvedLineSource::B);
        assert_eq!(meta[0].input_line, Some(1));
    }

    #[test]
    fn empty_base_conflict_hint_overrides_false_a_badge() {
        fn shared(s: &str) -> gpui::SharedString {
            s.to_string().into()
        }

        let segments = vec![
            ConflictSegment::Text("dup\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some(String::new()),
                ours: "dup\n".to_string(),
                theirs: "other\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
        ];
        let output = conflict_resolver::generate_resolved_text(&segments);
        let output_lines = conflict_resolver::split_output_lines_for_outline(&output);

        let a = vec![shared("dup")];
        let b = vec![shared("dup"), shared("dup")];
        let c = vec![shared("dup"), shared("other")];
        let sources = SourceLines {
            a: &a,
            b: &b,
            c: &c,
        };

        let mut meta = conflict_resolver::compute_resolved_line_provenance(&output_lines, &sources);
        // Raw content matching can pick A because "dup" exists in A.
        assert_eq!(meta[1].source, ResolvedLineSource::A);

        apply_three_way_empty_base_provenance_hints(&mut meta, &segments, &output);

        assert_eq!(meta[1].source, ResolvedLineSource::B);
        assert_eq!(meta[1].input_line, Some(2));
        assert_eq!(
            conflict_resolver::build_resolved_output_line_sources_index(
                &meta,
                &output_lines,
                ConflictResolverViewMode::ThreeWay
            )
            .contains(&conflict_resolver::SourceLineKey::new(
                ConflictResolverViewMode::ThreeWay,
                ResolvedLineSource::B,
                2,
                "dup"
            )),
            true
        );
    }
}
