use super::*;
use crate::kit::text_model::TextModelSnapshot;
use crate::kit::{HighlightProvider, HighlightProviderResult};

#[derive(Default)]
pub(super) struct ResolvedOutputSyntaxState {
    /// Fallback highlights used when full-document syntax is unsupported.
    pub(super) highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub(super) prepared_document: Option<rows::PreparedDiffSyntaxDocument>,
    /// Lazy provider backed by a prepared document.
    pub(super) highlight_provider: Option<HighlightProvider>,
    /// When true, render plain text for now and continue parsing in the background.
    pub(super) needs_background_prepare: bool,
}

fn build_resolved_output_syntax_fallback_highlights(
    theme: AppTheme,
    output_text: &str,
    language: rows::DiffSyntaxLanguage,
    syntax_mode: rows::DiffSyntaxMode,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let line_starts = build_line_starts(output_text);
    let text_len = output_text.len();
    let mut highlights = Vec::new();
    for (line_ix, &line_start) in line_starts.iter().enumerate() {
        let line_end = line_starts
            .get(line_ix + 1)
            .map(|s| s.saturating_sub(1)) // exclude '\n'
            .unwrap_or(text_len);
        let line = &output_text[line_start..line_end];
        for (range, style) in rows::syntax_highlights_for_line(theme, line, language, syntax_mode) {
            highlights.push(((line_start + range.start)..(line_start + range.end), style));
        }
    }
    highlights
}

fn resolved_output_highlight_provider(
    theme: AppTheme,
    output_text: SharedString,
    line_starts: Arc<[usize]>,
    language: rows::DiffSyntaxLanguage,
    document: rows::PreparedDiffSyntaxDocument,
) -> HighlightProvider {
    let shared_text: Arc<str> = output_text.into();
    HighlightProvider::with_pending(
        move |byte_range: Range<usize>| {
            rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                &shared_text,
                line_starts.as_ref(),
                document,
                language,
                byte_range,
            )
            .map(|result| HighlightProviderResult {
                highlights: result.highlights,
                pending: result.pending,
            })
            .unwrap_or_default()
        },
        move || rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document),
        move || rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document),
    )
}

fn build_resolved_output_syntax_state_with_source(
    theme: AppTheme,
    output_text: SharedString,
    line_starts: Arc<[usize]>,
    language: Option<rows::DiffSyntaxLanguage>,
    old_document: Option<rows::PreparedDiffSyntaxDocument>,
    edit_hint: Option<rows::DiffSyntaxEdit>,
    budget: rows::DiffSyntaxBudget,
) -> ResolvedOutputSyntaxState {
    let Some(language) = language else {
        return ResolvedOutputSyntaxState::default();
    };
    if output_text.is_empty() {
        return ResolvedOutputSyntaxState::default();
    }

    match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        language,
        rows::DiffSyntaxMode::Auto,
        output_text.clone(),
        line_starts.clone(),
        budget,
        old_document,
        edit_hint,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => ResolvedOutputSyntaxState {
            highlights: Vec::new(),
            prepared_document: Some(document),
            highlight_provider: Some(resolved_output_highlight_provider(
                theme,
                output_text,
                line_starts,
                language,
                document,
            )),
            needs_background_prepare: false,
        },
        rows::PrepareDiffSyntaxDocumentResult::TimedOut => ResolvedOutputSyntaxState {
            highlights: Vec::new(),
            prepared_document: None,
            highlight_provider: None,
            needs_background_prepare: true,
        },
        rows::PrepareDiffSyntaxDocumentResult::Unsupported => ResolvedOutputSyntaxState {
            highlights: build_resolved_output_syntax_fallback_highlights(
                theme,
                output_text.as_ref(),
                language,
                rows::DiffSyntaxMode::HeuristicOnly,
            ),
            prepared_document: None,
            highlight_provider: None,
            needs_background_prepare: false,
        },
    }
}

#[cfg(test)]
pub(super) fn build_resolved_output_syntax_state_for_snapshot(
    theme: AppTheme,
    output_snapshot: &TextModelSnapshot,
    language: Option<rows::DiffSyntaxLanguage>,
    old_document: Option<rows::PreparedDiffSyntaxDocument>,
    edit_hint: Option<rows::DiffSyntaxEdit>,
) -> ResolvedOutputSyntaxState {
    build_resolved_output_syntax_state_with_source(
        theme,
        output_snapshot.as_shared_string(),
        output_snapshot.shared_line_starts(),
        language,
        old_document,
        edit_hint,
        rows::DiffSyntaxBudget::default(),
    )
}

pub(super) fn build_resolved_output_syntax_state_for_snapshot_with_budget(
    theme: AppTheme,
    output_snapshot: &TextModelSnapshot,
    language: Option<rows::DiffSyntaxLanguage>,
    old_document: Option<rows::PreparedDiffSyntaxDocument>,
    edit_hint: Option<rows::DiffSyntaxEdit>,
    budget: rows::DiffSyntaxBudget,
) -> ResolvedOutputSyntaxState {
    build_resolved_output_syntax_state_with_source(
        theme,
        output_snapshot.as_shared_string(),
        output_snapshot.shared_line_starts(),
        language,
        old_document,
        edit_hint,
        budget,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutputSyntaxBackgroundKey {
    pub(in crate::view) source_hash: u64,
    pub(in crate::view) language: rows::DiffSyntaxLanguage,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct VersionedCachedDiffStyledText {
    pub(in crate::view) syntax_epoch: u64,
    pub(in crate::view) styled: CachedDiffStyledText,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct FileDiffStyleCacheEpochs {
    pub(in crate::view) split_left: u64,
    pub(in crate::view) split_right: u64,
}

impl FileDiffStyleCacheEpochs {
    pub(in crate::view) fn bump_left(&mut self) {
        self.split_left = self.split_left.wrapping_add(1);
    }

    pub(in crate::view) fn bump_right(&mut self) {
        self.split_right = self.split_right.wrapping_add(1);
    }

    pub(in crate::view) fn bump_both(&mut self) {
        self.bump_left();
        self.bump_right();
    }

    pub(in crate::view) fn split_epoch(self, region: crate::view::DiffTextRegion) -> u64 {
        match region {
            crate::view::DiffTextRegion::SplitLeft => self.split_left,
            crate::view::DiffTextRegion::SplitRight => self.split_right,
            crate::view::DiffTextRegion::Inline => 0,
        }
    }

    pub(in crate::view) fn inline_epoch(self, kind: gitcomet_core::domain::DiffLineKind) -> u64 {
        match kind {
            gitcomet_core::domain::DiffLineKind::Remove => self.split_left,
            gitcomet_core::domain::DiffLineKind::Add
            | gitcomet_core::domain::DiffLineKind::Context => self.split_right,
            gitcomet_core::domain::DiffLineKind::Header
            | gitcomet_core::domain::DiffLineKind::Hunk => 0,
        }
    }
}

pub(in crate::view) fn versioned_cached_diff_styled_text_is_current(
    entry: Option<&VersionedCachedDiffStyledText>,
    syntax_epoch: u64,
) -> Option<&CachedDiffStyledText> {
    let entry = entry?;
    (entry.syntax_epoch == syntax_epoch).then_some(&entry.styled)
}

pub(super) fn split_text_lines_owned(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(|line| line.to_string()).collect()
    }
}

pub(super) fn count_newlines(text: &str) -> usize {
    text.as_bytes().iter().filter(|&&b| b == b'\n').count()
}

pub(super) fn build_line_starts(text: &str) -> Vec<usize> {
    let mut line_starts = Vec::with_capacity(count_newlines(text).saturating_add(1));
    line_starts.push(0usize);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(ix.saturating_add(1));
        }
    }
    line_starts
}

pub(super) fn preview_source_text_from_lines(lines: &[String], source_len: usize) -> SharedString {
    let mut source = lines.join("\n");
    if source.len() < source_len {
        source.push('\n');
    }
    debug_assert_eq!(
        source.len(),
        source_len,
        "preview lines/source length should only differ by an optional trailing newline",
    );
    source.into()
}

pub(in crate::view) fn preview_source_text_and_line_starts_from_lines(
    lines: &[String],
    source_len: usize,
) -> (SharedString, Arc<[usize]>) {
    let text = preview_source_text_from_lines(lines, source_len);
    let line_starts = Arc::from(build_line_starts(text.as_ref()));
    (text, line_starts)
}

pub(super) fn line_start_offset_for_index(
    line_starts: &[usize],
    text_len: usize,
    line_ix: usize,
) -> usize {
    line_starts.get(line_ix).copied().unwrap_or(text_len)
}

pub(super) fn source_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Number of logical rows represented by precomputed line starts.
///
/// Uses `split('\n')` row semantics for non-empty text, so a trailing newline
/// preserves a final empty row.
pub(super) fn indexed_line_count(text: &str, line_starts: &[usize]) -> usize {
    if text.is_empty() {
        0
    } else {
        line_starts.len().max(1)
    }
}

/// Number of logical rows produced by `split('\n')` (always at least 1).
pub(super) fn split_line_count(text: &str) -> usize {
    count_newlines(text).saturating_add(1)
}

/// Byte range of line content at `line_ix` (without trailing newline).
///
/// Uses `split('\n')` row semantics, so trailing newline creates a final empty row.
pub(super) fn line_content_byte_range_for_index(
    text: &str,
    line_ix: usize,
) -> Option<Range<usize>> {
    let line_count = split_line_count(text);
    if line_ix >= line_count {
        return None;
    }
    let line_starts = build_line_starts(text);
    let text_len = text.len();
    let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    Some(start..end)
}

/// Build insertion text for appending one logical line to output.
pub(super) fn append_line_insertion_text(existing: &str, line: &str) -> String {
    let needs_leading_newline = !existing.is_empty() && !existing.ends_with('\n');
    let mut out = String::with_capacity(
        line.len()
            .saturating_add(1)
            .saturating_add(usize::from(needs_leading_newline)),
    );
    if needs_leading_newline {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    out
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedOutlineDelta {
    pub(super) old_range: Range<usize>,
    pub(super) new_range: Range<usize>,
}

pub(super) fn resolved_outline_delta_between_texts(
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

pub(super) fn resolved_outline_delta_for_snapshot_transition(
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

    resolved_outline_delta_between_texts(old_snapshot.as_ref(), new_snapshot.as_ref())
}

fn line_index_for_byte_offset(line_starts: &[usize], byte_offset: usize) -> usize {
    if line_starts.is_empty() {
        return 0;
    }
    line_starts
        .partition_point(|&start| start <= byte_offset)
        .saturating_sub(1)
}

pub(super) fn dirty_byte_range_to_line_range(
    line_starts: &[usize],
    text_len: usize,
    dirty_range: Range<usize>,
) -> Range<usize> {
    let line_count = line_starts.len().max(1);
    let start_byte = dirty_range.start.min(text_len);
    let end_byte = dirty_range.end.min(text_len);
    let start_line = line_index_for_byte_offset(line_starts, start_byte).min(line_count - 1);
    let end_line_exclusive = if dirty_range.is_empty() {
        start_line.saturating_add(1)
    } else {
        line_index_for_byte_offset(line_starts, end_byte).saturating_add(1)
    }
    .clamp(start_line.saturating_add(1), line_count);
    start_line..end_line_exclusive
}

pub(super) fn shifted_line_index(ix: usize, delta: isize) -> usize {
    if delta >= 0 {
        ix.saturating_add(delta as usize)
    } else {
        ix.saturating_sub((-delta) as usize)
    }
}

pub(super) fn remap_line_keyed_cache_for_delta<T>(
    cache: &mut HashMap<usize, T>,
    old_range: Range<usize>,
    new_range: Range<usize>,
) {
    let shift = new_range.len() as isize - old_range.len() as isize;
    let previous = std::mem::take(cache);
    for (line_ix, value) in previous {
        if line_ix < old_range.start {
            cache.insert(line_ix, value);
            continue;
        }
        if line_ix >= old_range.end {
            cache.insert(shifted_line_index(line_ix, shift), value);
        }
    }
}

pub(super) fn resolved_output_conflict_block_ranges_in_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
) -> Option<Vec<Range<usize>>> {
    fn is_line_boundary(text: &str, byte_ix: usize) -> bool {
        if byte_ix == 0 || byte_ix == text.len() {
            return true;
        }
        text.as_bytes()
            .get(byte_ix.saturating_sub(1))
            .is_some_and(|b| *b == b'\n')
    }

    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    let mut line_offset = 0usize;
    for seg in marker_segments {
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
                let start_line = line_offset;
                let mut end_line = line_offset.saturating_add(count_newlines(&expected));
                if end == output_text.len() && !expected.is_empty() {
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

pub(super) fn conflict_marker_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
    line_range: Range<usize>,
) -> Vec<Range<usize>> {
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

pub(super) fn write_conflict_markers_for_ranges(
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

pub(super) fn output_line_range_for_conflict_block_in_text(
    segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    conflict_ix: usize,
) -> Option<Range<usize>> {
    resolved_output_conflict_block_ranges_in_text(segments, output_text)
        .and_then(|ranges| ranges.get(conflict_ix).cloned())
}

pub(super) fn conflict_fragment_text_for_choice(
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

pub(super) fn unresolved_subchunk_conflict_ranges_for_block(
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
pub(super) struct UnresolvedDecisionRegion {
    pub(super) row_range: Range<usize>,
    pub(super) selected_line_range: Range<usize>,
    pub(super) alternate_line_range: Range<usize>,
    pub(super) has_non_emitting_rows: bool,
}

pub(super) fn unresolved_decision_regions_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<UnresolvedDecisionRegion>> {
    let (left, right, choose_left) = match block.choice {
        conflict_resolver::ConflictChoice::Ours => (&block.ours, &block.theirs, true),
        conflict_resolver::ConflictChoice::Theirs => (&block.theirs, &block.ours, false),
        _ => return None,
    };
    let rows_with_anchors = gitcomet_core::file_diff::side_by_side_rows_with_anchors(left, right);
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

pub(super) fn unresolved_decision_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    unresolved_decision_regions_for_block(block).map(|regions| {
        regions
            .into_iter()
            .map(|region| region.selected_line_range)
            .collect()
    })
}

pub(super) fn build_resolved_output_conflict_markers(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    let Some(block_ranges) =
        resolved_output_conflict_block_ranges_in_text(marker_segments, output_text)
    else {
        return markers;
    };

    for (conflict_ix, (block, range)) in marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .zip(block_ranges.into_iter())
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

pub(super) fn push_conflict_text_segment(
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

pub(super) fn resolved_output_markers_for_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let output_line_count = conflict_resolver::split_output_lines_for_outline(output_text).len();
    build_resolved_output_conflict_markers(marker_segments, output_text, output_line_count)
}

pub(super) fn resolved_output_marker_for_line(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    output_line_ix: usize,
) -> Option<ResolvedOutputConflictMarker> {
    resolved_output_markers_for_text(marker_segments, output_text)
        .get(output_line_ix)
        .copied()
        .flatten()
}

pub(super) fn first_output_marker_line_for_conflict(
    markers: &[Option<ResolvedOutputConflictMarker>],
    conflict_ix: usize,
) -> Option<usize> {
    markers.iter().enumerate().find_map(|(line_ix, marker)| {
        marker
            .as_ref()
            .and_then(|m| (m.conflict_ix == conflict_ix && m.is_start).then_some(line_ix))
    })
}

pub(super) fn conflict_marker_nav_entries_from_markers(
    markers: &[Option<ResolvedOutputConflictMarker>],
) -> Vec<usize> {
    let mut seen_conflicts = std::collections::HashSet::new();
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

pub(super) fn line_index_for_offset(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count()
}

pub(super) fn conflict_resolver_output_context_line(
    content: &str,
    cursor_offset: usize,
    clicked_offset: Option<usize>,
) -> usize {
    clicked_offset
        .map(|offset| line_index_for_offset(content, offset))
        .unwrap_or_else(|| line_index_for_offset(content, cursor_offset))
}

pub(super) fn slice_text_by_line_range(text: &str, line_range: Range<usize>) -> String {
    if line_range.start >= line_range.end || text.is_empty() {
        return String::new();
    }

    let line_starts = build_line_starts(text);

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

pub(super) fn split_target_conflict_block_into_subchunks(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    target_conflict_ix: usize,
) -> bool {
    use gitcomet_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

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

pub(super) fn conflict_region_index_is_unique(
    conflict_region_indices: &[usize],
    region_ix: usize,
) -> bool {
    conflict_region_indices
        .iter()
        .filter(|&&ix| ix == region_ix)
        .take(2)
        .count()
        <= 1
}

pub(super) fn conflict_block_matches_group(
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

pub(super) fn conflict_group_member_indices_for_ix(
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

pub(super) fn conflict_group_selected_choices_for_ix(
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

pub(super) fn conflict_group_indices_for_choice(
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

pub(super) fn should_remove_conflict_block_on_reset(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> bool {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    group_indices.len() > 1
}

pub(super) fn remove_conflict_block_at(
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

pub(super) fn reset_conflict_block_selection(
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

pub(super) fn append_choice_after_conflict_block(
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
    let &group_end_ix = group_indices.last()?;
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

pub(super) fn scroll_conflict_resolved_output_to_line(
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

#[cfg(test)]
pub(super) fn apply_three_way_empty_base_provenance_hints(
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

                let base_empty = block.base.as_ref().is_none_or(|s| s.is_empty());
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

pub(super) fn apply_conflict_choice_provenance_hints(
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

pub(super) fn replacement_lines_for_conflict_block(
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

pub(super) fn replace_output_lines_in_range(
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
pub(super) enum ClearDiffSelectionAction {
    ClearSelection,
    ExitFocusedMergetool,
}

pub(super) fn clear_diff_selection_action(view_mode: GitCometViewMode) -> ClearDiffSelectionAction {
    match view_mode {
        GitCometViewMode::Normal => ClearDiffSelectionAction::ClearSelection,
        GitCometViewMode::FocusedMergetool => ClearDiffSelectionAction::ExitFocusedMergetool,
    }
}

pub(super) fn focused_mergetool_save_exit_code(
    total_conflicts: usize,
    resolved_conflicts: usize,
) -> i32 {
    if total_conflicts == 0 || total_conflicts == resolved_conflicts {
        FOCUSED_MERGETOOL_EXIT_SUCCESS
    } else {
        FOCUSED_MERGETOOL_EXIT_CANCELED
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum PreparedSyntaxViewMode {
    FileDiffSplitLeft,
    FileDiffSplitRight,
    WorktreePreview,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct PreparedSyntaxDocumentKey {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) target_rev: u64,
    pub(in crate::view) file_path: std::path::PathBuf,
    pub(in crate::view) view_mode: PreparedSyntaxViewMode,
}

pub(in crate::view) struct MainPaneView {
    pub(in crate::view) store: Arc<AppStore>,
    pub(super) state: Arc<AppState>,
    pub(in crate::view) view_mode: GitCometViewMode,
    pub(in crate::view) focused_mergetool_labels: Option<FocusedMergetoolLabels>,
    pub(in crate::view) focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) date_time_format: DateTimeFormat,
    pub(super) _ui_model_subscription: gpui::Subscription,
    pub(super) root_view: WeakEntity<GitCometView>,
    pub(super) tooltip_host: WeakEntity<TooltipHost>,
    pub(super) notify_fingerprint: u64,
    pub(in crate::view) active_context_menu_invoker: Option<SharedString>,

    pub(in crate::view) last_window_size: Size<Pixels>,
    pub(in crate::view) layout_sidebar_render_width: Pixels,
    pub(in crate::view) layout_details_render_width: Pixels,
    pub(in crate::view) layout_sidebar_collapsed: bool,
    pub(in crate::view) layout_details_collapsed: bool,

    pub(in crate::view) show_whitespace: bool,
    pub(in crate::view) diff_view: DiffViewMode,
    pub(in crate::view) rendered_preview_modes: RenderedPreviewModes,
    pub(in crate::view) diff_word_wrap: bool,
    pub(in crate::view) diff_split_ratio: f32,
    pub(in crate::view) diff_split_resize: Option<DiffSplitResizeState>,
    pub(in crate::view) diff_split_last_synced_y: [Pixels; 2],
    pub(in crate::view) diff_horizontal_min_width: Pixels,
    pub(in crate::view) diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) diff_cache_rev: u64,
    pub(in crate::view) diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) diff_cache: Vec<AnnotatedDiffLine>,
    pub(in crate::view) diff_row_provider: Option<Arc<super::diff_cache::PagedPatchDiffRows>>,
    pub(in crate::view) diff_split_row_provider:
        Option<Arc<super::diff_cache::PagedPatchSplitRows>>,
    pub(in crate::view) diff_file_for_src_ix: Vec<Option<Arc<str>>>,
    pub(in crate::view) diff_language_for_src_ix: Vec<Option<rows::DiffSyntaxLanguage>>,
    pub(in crate::view) diff_click_kinds: Vec<DiffClickKind>,
    pub(in crate::view) diff_line_kind_for_src_ix: Vec<gitcomet_core::domain::DiffLineKind>,
    pub(in crate::view) diff_hide_unified_header_for_src_ix: Vec<bool>,
    pub(in crate::view) diff_header_display_cache: HashMap<usize, SharedString>,
    pub(in crate::view) diff_split_cache: Vec<PatchSplitRow>,
    pub(in crate::view) diff_split_cache_len: usize,
    pub(in crate::view) diff_panel_focus_handle: FocusHandle,
    pub(in crate::view) diff_autoscroll_pending: bool,
    pub(in crate::view) diff_raw_input: Entity<components::TextInput>,
    pub(in crate::view) diff_visible_indices: Vec<usize>,
    pub(in crate::view) diff_visible_inline_map: Option<super::diff_cache::PatchInlineVisibleMap>,
    pub(in crate::view) diff_visible_cache_len: usize,
    pub(in crate::view) diff_visible_view: DiffViewMode,
    pub(in crate::view) diff_visible_is_file_view: bool,
    pub(in crate::view) diff_scrollbar_markers_cache: Vec<components::ScrollbarMarker>,
    pub(in crate::view) diff_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in crate::view) diff_word_highlights_inflight: Option<u64>,
    pub(in crate::view) diff_file_stats: Vec<Option<(usize, usize)>>,
    pub(in crate::view) diff_text_segments_cache: Vec<Option<VersionedCachedDiffStyledText>>,
    pub(in crate::view) diff_text_query_segments_cache: Vec<Option<VersionedCachedDiffStyledText>>,
    pub(in crate::view) diff_text_query_cache_query: SharedString,
    pub(in crate::view) diff_selection_anchor: Option<usize>,
    pub(in crate::view) diff_selection_range: Option<(usize, usize)>,
    pub(in crate::view) diff_text_selecting: bool,
    pub(in crate::view) diff_text_anchor: Option<DiffTextPos>,
    pub(in crate::view) diff_text_head: Option<DiffTextPos>,
    pub(super) diff_text_autoscroll_seq: u64,
    pub(super) diff_text_autoscroll_target: Option<DiffTextAutoscrollTarget>,
    pub(super) diff_text_last_mouse_pos: Point<Pixels>,
    pub(in crate::view) diff_suppress_clicks_remaining: u8,
    pub(in crate::view) diff_text_hitboxes: HashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    pub(in crate::view) diff_text_layout_cache_epoch: u64,
    pub(in crate::view) diff_text_layout_cache: HashMap<u64, DiffTextLayoutCacheEntry>,
    pub(in crate::view) diff_hunk_picker_search_input: Option<Entity<components::TextInput>>,
    pub(in crate::view) diff_search_active: bool,
    pub(in crate::view) diff_search_query: SharedString,
    pub(in crate::view) diff_search_matches: Vec<usize>,
    pub(in crate::view) diff_search_match_ix: Option<usize>,
    pub(in crate::view) diff_search_input: Entity<components::TextInput>,
    pub(super) _diff_search_subscription: gpui::Subscription,

    pub(in crate::view) file_diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_diff_cache_rev: u64,
    pub(in crate::view) file_diff_cache_content_signature: Option<u64>,
    pub(in crate::view) file_diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_diff_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_diff_cache_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) file_diff_cache_rows: Vec<FileDiffRow>,
    /// Real old-side file text used for split and inline syntax projection.
    pub(in crate::view) file_diff_old_text: SharedString,
    pub(in crate::view) file_diff_old_line_starts: Arc<[usize]>,
    /// Real new-side file text used for split and inline syntax projection.
    pub(in crate::view) file_diff_new_text: SharedString,
    pub(in crate::view) file_diff_new_line_starts: Arc<[usize]>,
    pub(in crate::view) file_diff_inline_cache: Vec<AnnotatedDiffLine>,
    pub(in crate::view) file_diff_inline_text: SharedString,
    pub(in crate::view) file_diff_inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in crate::view) file_diff_split_word_highlights_old: Vec<Option<Vec<Range<usize>>>>,
    pub(in crate::view) file_diff_split_word_highlights_new: Vec<Option<Vec<Range<usize>>>>,
    pub(in crate::view) file_diff_cache_seq: u64,
    pub(in crate::view) file_diff_cache_inflight: Option<u64>,
    pub(in crate::view) file_diff_syntax_generation: u64,
    pub(in crate::view) file_diff_style_cache_epochs: FileDiffStyleCacheEpochs,
    pub(in crate::view) syntax_chunk_poll_task: Option<gpui::Task<()>>,
    pub(in crate::view) prepared_syntax_documents:
        HashMap<PreparedSyntaxDocumentKey, rows::PreparedDiffSyntaxDocument>,
    #[cfg(test)]
    pub(in crate::view) diff_syntax_budget_override: Option<rows::DiffSyntaxBudget>,

    pub(in crate::view) file_markdown_preview_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_markdown_preview_cache_rev: u64,
    pub(in crate::view) file_markdown_preview_cache_content_signature: Option<u64>,
    pub(in crate::view) file_markdown_preview_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_markdown_preview: LoadableMarkdownDiff,
    pub(in crate::view) file_markdown_preview_seq: u64,
    pub(in crate::view) file_markdown_preview_inflight: Option<u64>,

    pub(in crate::view) file_image_diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_image_diff_cache_rev: u64,
    pub(in crate::view) file_image_diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_image_diff_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_diff_cache_old: Option<Arc<gpui::Image>>,
    pub(in crate::view) file_image_diff_cache_new: Option<Arc<gpui::Image>>,
    pub(in crate::view) file_image_diff_cache_old_svg_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_diff_cache_new_svg_path: Option<std::path::PathBuf>,

    pub(in crate::view) worktree_preview_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview: Loadable<usize>,
    pub(in crate::view) worktree_preview_text: SharedString,
    pub(in crate::view) worktree_preview_line_starts: Arc<[usize]>,
    pub(in crate::view) worktree_preview_content_rev: u64,
    pub(in crate::view) worktree_markdown_preview_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_markdown_preview_source_rev: u64,
    pub(in crate::view) worktree_markdown_preview: LoadableMarkdownDoc,
    pub(in crate::view) worktree_markdown_preview_seq: u64,
    pub(in crate::view) worktree_markdown_preview_inflight: Option<u64>,
    pub(in crate::view) worktree_preview_segments_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) worktree_preview_style_cache_epoch: u64,
    pub(in crate::view) worktree_preview_segments_cache:
        HashMap<usize, VersionedCachedDiffStyledText>,
    pub(in crate::view) diff_preview_is_new_file: bool,

    pub(in crate::view) conflict_resolver_input: Entity<components::TextInput>,
    pub(super) _conflict_resolver_input_subscription: gpui::Subscription,
    pub(in crate::view) conflict_resolver: ConflictResolverUiState,
    pub(in crate::view) conflict_resolver_vsplit_ratio: f32,
    pub(in crate::view) conflict_resolver_vsplit_resize: Option<ConflictVSplitResizeState>,
    pub(in crate::view) conflict_three_way_col_ratios: [f32; 2],
    pub(in crate::view) conflict_three_way_col_widths: [Pixels; 3],
    pub(in crate::view) conflict_hsplit_resize: Option<ConflictHSplitResizeState>,
    pub(in crate::view) conflict_diff_split_ratio: f32,
    pub(in crate::view) conflict_diff_split_resize: Option<ConflictDiffSplitResizeState>,
    pub(in crate::view) conflict_diff_split_col_widths: [Pixels; 2],
    pub(in crate::view) conflict_canvas_rows_enabled: bool,
    pub(in crate::view) conflict_diff_segments_cache_split:
        HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
    pub(in crate::view) conflict_diff_segments_cache_inline: HashMap<usize, CachedDiffStyledText>,
    pub(in crate::view) conflict_diff_query_segments_cache_split:
        HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
    pub(in crate::view) conflict_diff_query_segments_cache_inline:
        HashMap<usize, CachedDiffStyledText>,
    pub(in crate::view) conflict_diff_query_cache_query: SharedString,
    pub(in crate::view) conflict_three_way_segments_cache:
        HashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    pub(in crate::view) conflict_resolved_preview_path: Option<std::path::PathBuf>,
    pub(in crate::view) conflict_resolved_preview_source_hash: Option<u64>,
    pub(in crate::view) conflict_resolved_preview_text: TextModelSnapshot,
    pub(in crate::view) conflict_resolved_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) conflict_resolved_preview_highlight_provider_theme_epoch: u64,
    pub(in crate::view) conflict_resolved_preview_style_cache_epoch: u64,
    pub(in crate::view) conflict_resolved_preview_prepared_syntax_document:
        Option<rows::PreparedDiffSyntaxDocument>,
    pub(in crate::view) conflict_resolved_preview_syntax_inflight:
        Option<ResolvedOutputSyntaxBackgroundKey>,
    pub(in crate::view) conflict_resolved_preview_line_count: usize,
    pub(in crate::view) conflict_resolved_preview_line_starts: Arc<[usize]>,
    pub(in crate::view) conflict_resolved_preview_segments_cache:
        HashMap<usize, VersionedCachedDiffStyledText>,

    pub(in crate::view) history_view: Entity<super::HistoryView>,
    pub(in crate::view) diff_scroll: UniformListScrollHandle,
    pub(in crate::view) diff_split_right_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_resolver_diff_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_ours_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_theirs_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_last_synced_y: [Pixels; 3],
    pub(in crate::view) conflict_resolved_preview_scroll: UniformListScrollHandle,
    pub(in crate::view) worktree_preview_scroll: UniformListScrollHandle,

    pub(super) path_display_cache: std::cell::RefCell<HashMap<std::path::PathBuf, SharedString>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffTextAutoscrollTarget {
    DiffLeftOrInline,
    DiffSplitRight,
    WorktreePreview,
    ConflictResolvedPreview,
}

pub(super) fn parse_conflict_canvas_rows_env(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

pub(super) fn conflict_canvas_rows_enabled_from_env() -> bool {
    std::env::var("GITCOMET_CONFLICT_CANVAS_ROWS")
        .ok()
        .is_none_or(|value| parse_conflict_canvas_rows_env(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_line_count_returns_zero_for_empty_text() {
        assert_eq!(indexed_line_count("", &[]), 0);
    }

    #[test]
    fn indexed_line_count_matches_nonempty_line_starts() {
        let text = "alpha\nbeta";
        let line_starts = build_line_starts(text);

        assert_eq!(indexed_line_count(text, &line_starts), 2);
    }

    #[test]
    fn indexed_line_count_preserves_trailing_empty_row() {
        let text = "alpha\nbeta\n";
        let line_starts = build_line_starts(text);

        assert_eq!(line_starts, vec![0, 6, 11]);
        assert_eq!(indexed_line_count(text, &line_starts), 3);
    }
}
