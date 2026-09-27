use super::*;

// ── Parser ──────────────────────────────────────────────────────────────

/// Build a `MarkdownPreviewDocument` from raw markdown source text.
///
/// Returns `None` if the source exceeds `MAX_PREVIEW_SOURCE_BYTES`
/// or the parsed document exceeds `MAX_PREVIEW_ROWS`.
pub(crate) fn parse_markdown(source: &str) -> Option<MarkdownPreviewDocument> {
    if source.len() > MAX_PREVIEW_SOURCE_BYTES {
        return None;
    }
    let document = build_markdown_document(source)?;
    document.index_anchors_if_linked();
    Some(document)
}

pub(crate) fn build_markdown_document(source: &str) -> Option<MarkdownPreviewDocument> {
    let line_starts = build_line_starts(source);
    let rows = flatten_to_rows(source, &line_starts)?;
    Some(MarkdownPreviewDocument::new(rows))
}

/// Build a pair of preview documents for a two-sided diff.
///
/// Returns `None` if combined source exceeds `MAX_DIFF_PREVIEW_SOURCE_BYTES`
/// or either document exceeds `MAX_PREVIEW_ROWS`.
///
/// Diff previews are limited by the combined payload size, so one side may
/// exceed `MAX_PREVIEW_SOURCE_BYTES` as long as the pair stays within the
/// diff-wide cap.
pub(crate) fn parse_markdown_diff(
    old_source: &str,
    new_source: &str,
) -> Option<(MarkdownPreviewDocument, MarkdownPreviewDocument)> {
    if old_source.len() + new_source.len() > MAX_DIFF_PREVIEW_SOURCE_BYTES {
        return None;
    }
    let old_doc = build_markdown_document(old_source)?;
    let new_doc = build_markdown_document(new_source)?;
    Some((old_doc, new_doc))
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn build_markdown_diff_preview(
    old_source: &str,
    new_source: &str,
) -> Option<MarkdownPreviewDiff> {
    build_markdown_diff_preview_of(Some(old_source), Some(new_source))
}

/// The diff preview of a change that may add or delete the file: `None` is a
/// side the file is not on.
pub(crate) fn build_markdown_diff_preview_of(
    old_source: Option<&str>,
    new_source: Option<&str>,
) -> Option<MarkdownPreviewDiff> {
    let sources = (
        MarkdownDiffSideSource::of(old_source),
        MarkdownDiffSideSource::of(new_source),
    );
    let (old_source, new_source) = (old_source.unwrap_or(""), new_source.unwrap_or(""));
    let (mut old, mut new) = parse_markdown_diff(old_source, new_source)?;
    let plan = gitcomet_core::file_diff::side_by_side_plan(old_source, new_source);
    let old_line_count = old_source.lines().count();
    let new_line_count = new_source.lines().count();
    let (old_mask, new_mask) =
        gitcomet_core::file_diff::plan_changed_line_masks(&plan, old_line_count, new_line_count);
    annotate_change_hints(&mut old, &mut new, &old_mask, &new_mask);
    let (old_line_to_diff_row, new_line_to_diff_row) =
        gitcomet_core::file_diff::plan_line_to_row_maps(&plan, old_line_count, new_line_count);
    let groups = markdown_diff_row_groups(
        old.rows,
        &old_line_to_diff_row,
        new.rows,
        &new_line_to_diff_row,
    );
    let (inline, inline_old) = inline_markdown_diff_document(&groups);
    let (old, new) = aligned_markdown_diff_documents(groups)?;
    let mut diff = MarkdownPreviewDiff::new(old, new, inline);
    diff.inline_old = inline_old;
    (diff.old_source, diff.new_source) = sources;
    for document in [&diff.old, &diff.new, &diff.inline] {
        document.index_anchors_if_linked();
    }
    Some(diff)
}

pub(crate) fn scrollbar_markers_for_diff_preview(
    preview: &MarkdownPreviewDiff,
) -> Vec<crate::view::components::ScrollbarMarker> {
    scrollbar_markers_for_documents(&[&preview.old, &preview.new])
}

pub(crate) fn scrollbar_markers_for_document(
    document: &MarkdownPreviewDocument,
) -> Vec<crate::view::components::ScrollbarMarker> {
    scrollbar_markers_for_documents(&[document])
}

/// Annotate change hints on a pair of preview documents using diff row data.
///
/// `changed_old_lines` and `changed_new_lines` are sets of 0-based line
/// indices that have changes (derived from `FileDiffRow` data).
pub(crate) fn annotate_change_hints(
    old_doc: &mut MarkdownPreviewDocument,
    new_doc: &mut MarkdownPreviewDocument,
    changed_old_lines: &[bool],
    changed_new_lines: &[bool],
) {
    for row in &mut old_doc.rows {
        if matches!(row.kind, MarkdownPreviewRowKind::Spacer) {
            continue;
        }
        row.change_hint = line_range_change_hint(&row.source_line_range, changed_old_lines, true);
    }
    for row in &mut new_doc.rows {
        if matches!(row.kind, MarkdownPreviewRowKind::Spacer) {
            continue;
        }
        row.change_hint = line_range_change_hint(&row.source_line_range, changed_new_lines, false);
    }
}

/// Rows of both sides that describe the same stretch of the diff: they are
/// drawn beside each other in the split view, and compared or replaced as a
/// unit in the inline one.
pub(crate) struct MarkdownDiffRowGroup {
    old: Vec<MarkdownPreviewRow>,
    new: Vec<MarkdownPreviewRow>,
}

/// The diff rows `row`'s source lines fall on, first and last.
fn markdown_row_diff_span(
    row: &MarkdownPreviewRow,
    line_to_diff_row: &[Option<usize>],
) -> Option<(usize, usize)> {
    let start = row.source_line_range.start.min(line_to_diff_row.len());
    let end = row.source_line_range.end.min(line_to_diff_row.len());
    let mut rows = line_to_diff_row[start..end].iter().flatten().copied();
    let first = rows.next()?;
    Some(rows.fold((first, first), |(lo, hi), row| (lo.min(row), hi.max(row))))
}

/// Group the rows of both sides wherever their diff spans overlap.
///
/// A row is not tied to one line: a paragraph whose first line was added
/// starts at that line on the new side, but its old version starts one diff
/// row later. Grouping by overlap keeps the two versions together, so they are
/// compared as one change instead of reading as an addition beside unchanged
/// text. A group that holds any change marks its unmarked rows modified: they
/// are the other version of that change.
pub(crate) fn markdown_diff_row_groups(
    old_rows: Vec<MarkdownPreviewRow>,
    old_line_to_diff_row: &[Option<usize>],
    new_rows: Vec<MarkdownPreviewRow>,
    new_line_to_diff_row: &[Option<usize>],
) -> Vec<MarkdownDiffRowGroup> {
    let with_spans = |rows: Vec<MarkdownPreviewRow>, map: &[Option<usize>]| {
        rows.into_iter()
            .map(|row| {
                let span = markdown_row_diff_span(&row, map);
                (row, span)
            })
            .collect::<Vec<_>>()
    };
    let mut old_rows = with_spans(old_rows, old_line_to_diff_row)
        .into_iter()
        .peekable();
    let mut new_rows = with_spans(new_rows, new_line_to_diff_row)
        .into_iter()
        .peekable();

    let mut groups: Vec<MarkdownDiffRowGroup> = Vec::new();
    // The last diff row the open group covers; `None` while it holds only rows
    // without a span.
    let mut group_end: Option<usize> = None;
    loop {
        // Take whichever side's next row starts first in the diff. A row with
        // no span follows the row before it on its own side.
        let take_old = match (old_rows.peek(), new_rows.peek()) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some((_, None)), _) => true,
            (_, Some((_, None))) => false,
            (Some((_, Some((old_start, _)))), Some((_, Some((new_start, _))))) => {
                old_start <= new_start
            }
        };
        let Some((row, span)) = (if take_old {
            old_rows.next()
        } else {
            new_rows.next()
        }) else {
            break;
        };
        let joins_open_group = !groups.is_empty()
            && match (span, group_end) {
                (None, _) | (Some(_), None) => true,
                (Some((start, _)), Some(end)) => start <= end,
            };
        if !joins_open_group {
            groups.push(MarkdownDiffRowGroup {
                old: Vec::new(),
                new: Vec::new(),
            });
            group_end = None;
        }
        if let Some((_, end)) = span {
            group_end = Some(group_end.map_or(end, |group_end| group_end.max(end)));
        }
        let group = groups.last_mut().expect("a group was opened for this row");
        if take_old {
            group.old.push(row);
        } else {
            group.new.push(row);
        }
    }

    for group in &mut groups {
        let changed = group
            .old
            .iter()
            .chain(&group.new)
            .any(|row| row.change_hint != MarkdownChangeHint::None);
        if !changed {
            continue;
        }
        for row in group.old.iter_mut().chain(group.new.iter_mut()) {
            if row.change_hint == MarkdownChangeHint::None
                && !matches!(row.kind, MarkdownPreviewRowKind::Spacer)
            {
                row.change_hint = MarkdownChangeHint::Modified;
            }
        }
    }
    groups
}

/// Both sides padded with spacer rows so each group occupies the same row
/// indices on the two.
fn aligned_markdown_diff_documents(
    groups: Vec<MarkdownDiffRowGroup>,
) -> Option<(MarkdownPreviewDocument, MarkdownPreviewDocument)> {
    // Each side ends up near max(old, new) plus padding rows -- not the sum --
    // and both vecs are moved into the documents below without shrinking, so an
    // over-estimate is retained for the cached document's lifetime.
    let capacity = groups
        .iter()
        .map(|group| group.old.len().max(group.new.len()))
        .sum();
    let mut old_aligned = Vec::with_capacity(capacity);
    let mut new_aligned = Vec::with_capacity(capacity);
    for group in groups {
        push_aligned_markdown_row_groups(&mut old_aligned, &mut new_aligned, group.old, group.new)?;
    }
    Some((
        MarkdownPreviewDocument::new(old_aligned),
        MarkdownPreviewDocument::new(new_aligned),
    ))
}

pub(crate) fn push_aligned_markdown_row_groups(
    old_out: &mut Vec<MarkdownPreviewRow>,
    new_out: &mut Vec<MarkdownPreviewRow>,
    old_rows: Vec<MarkdownPreviewRow>,
    new_rows: Vec<MarkdownPreviewRow>,
) -> Option<()> {
    let row_count = old_rows.len().max(new_rows.len());
    let mut old_iter = old_rows.into_iter();
    let mut new_iter = new_rows.into_iter();

    for _ in 0..row_count {
        old_out.push(old_iter.next().unwrap_or_else(markdown_preview_spacer_row));
        new_out.push(new_iter.next().unwrap_or_else(markdown_preview_spacer_row));

        if old_out.len() > MAX_PREVIEW_ROWS || new_out.len() > MAX_PREVIEW_ROWS {
            return None;
        }
    }

    Some(())
}

pub(crate) fn markdown_preview_spacer_row() -> MarkdownPreviewRow {
    markdown_preview_spacer_row_with_range(0..0)
}

pub(crate) fn markdown_preview_spacer_row_with_range(
    source_line_range: Range<usize>,
) -> MarkdownPreviewRow {
    MarkdownPreviewRow {
        source_line_range,
        ..MarkdownPreviewRow::default()
    }
}

/// The inline diff: each unchanged group once, in its current form, and each
/// changed group as its old rows followed by its new ones.
///
/// Emitting a changed group whole, rather than interleaving its rows one index
/// at a time, keeps a rewritten list or table reading as one old version and
/// one new version.
pub(crate) fn inline_markdown_diff_document(
    groups: &[MarkdownDiffRowGroup],
) -> (MarkdownPreviewDocument, Vec<bool>) {
    let drawn = |rows: &[MarkdownPreviewRow]| {
        rows.iter()
            .filter(|row| !matches!(row.kind, MarkdownPreviewRowKind::Spacer))
            .cloned()
            .collect::<Vec<_>>()
    };
    let capacity = groups
        .iter()
        .map(|group| group.old.len().max(group.new.len()))
        .sum();
    let mut rows = Vec::with_capacity(capacity);
    let mut from_old = Vec::with_capacity(capacity);
    for group in groups {
        let old = drawn(&group.old);
        let new = drawn(&group.new);
        let unchanged = old.len() == new.len()
            && old
                .iter()
                .zip(&new)
                .all(|(old, new)| markdown_inline_diff_rows_can_merge(old, new));
        if !unchanged {
            from_old.resize(from_old.len() + old.len(), true);
            rows.extend(old);
        }
        from_old.resize(from_old.len() + new.len(), false);
        rows.extend(new);
    }
    (MarkdownPreviewDocument::new(rows), from_old)
}

/// Whether an old row and a new one show the same thing and can be drawn once.
///
/// List numbers and link or picture destinations are not compared: an item
/// renumbered by an insertion above it, or a reference definition that moved
/// elsewhere in the file, has not changed on its own line. The merged row is
/// the new one, so what is drawn is the current document.
pub(crate) fn markdown_inline_diff_rows_can_merge(
    old_row: &MarkdownPreviewRow,
    new_row: &MarkdownPreviewRow,
) -> bool {
    let same_kind = match (old_row.kind, new_row.kind) {
        (
            MarkdownPreviewRowKind::ListItem { number: old },
            MarkdownPreviewRowKind::ListItem { number: new },
        ) => old.is_some() == new.is_some(),
        (old, new) => old == new,
    };
    let same_spans = old_row.inline_spans.len() == new_row.inline_spans.len()
        && old_row
            .inline_spans
            .iter()
            .zip(new_row.inline_spans.iter())
            .all(|(old, new)| old.byte_range == new.byte_range && old.style == new.style);
    old_row.change_hint == MarkdownChangeHint::None
        && new_row.change_hint == MarkdownChangeHint::None
        && !matches!(old_row.kind, MarkdownPreviewRowKind::Spacer)
        && !matches!(new_row.kind, MarkdownPreviewRowKind::Spacer)
        && same_kind
        && old_row.text == new_row.text
        && same_spans
        && old_row.code_language == new_row.code_language
        && old_row.indent_level == new_row.indent_level
        && old_row.blockquote_level == new_row.blockquote_level
        && old_row.footnote_label == new_row.footnote_label
        && old_row.alert_kind == new_row.alert_kind
        && old_row.starts_alert == new_row.starts_alert
        && old_row.continues_item == new_row.continues_item
        && old_row.task.map(|task| task.checked) == new_row.task.map(|task| task.checked)
}

// ── Internal helpers ────────────────────────────────────────────────────

pub(crate) fn scrollbar_markers_for_documents(
    documents: &[&MarkdownPreviewDocument],
) -> Vec<crate::view::components::ScrollbarMarker> {
    let max_len = documents
        .iter()
        .map(|document| document.rows.len())
        .max()
        .unwrap_or(0);
    if max_len == 0 {
        return Vec::new();
    }

    let bucket_count = 240usize.min(max_len).max(1);
    let mut buckets = vec![0u8; bucket_count];

    for document in documents {
        let len = document.rows.len();
        if len == 0 {
            continue;
        }

        for (row_ix, row) in document.rows.iter().enumerate() {
            let flag = scrollbar_flag_for_change_hint(row.change_hint);
            if flag == 0 {
                continue;
            }

            let bucket_ix = (row_ix * bucket_count) / len;
            if let Some(bucket) = buckets.get_mut(bucket_ix) {
                *bucket |= flag;
            }
        }
    }

    super::super::diff_utils::scrollbar_markers_from_flags(bucket_count, |bucket_ix| {
        buckets.get(bucket_ix).copied().unwrap_or(0)
    })
}

/// Markers for changes measured at `(top, bottom, flag)` in a scroll content
/// `content_height` tall.
pub(crate) fn scrollbar_markers_for_extents(
    extents: &[(f32, f32, u8)],
    content_height: f32,
) -> Vec<crate::view::components::ScrollbarMarker> {
    const BUCKETS: usize = 240;
    let mut buckets = [0u8; BUCKETS];
    for &(top, bottom, flag) in extents {
        let bucket = |y: f32| ((y / content_height).clamp(0.0, 1.0) * BUCKETS as f32) as usize;
        let first = bucket(top).min(BUCKETS - 1);
        let last = bucket(bottom).clamp(first, BUCKETS - 1);
        for cell in &mut buckets[first..=last] {
            *cell |= flag;
        }
    }
    super::super::diff_utils::scrollbar_markers_from_flags(BUCKETS, |ix| buckets[ix])
}

pub(crate) fn scrollbar_flag_for_change_hint(hint: MarkdownChangeHint) -> u8 {
    match hint {
        MarkdownChangeHint::None => 0,
        MarkdownChangeHint::Added => 1,
        MarkdownChangeHint::Removed => 2,
        MarkdownChangeHint::Modified => 3,
    }
}

/// Build a vec of byte offsets for the start of each line.
pub(crate) fn build_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Convert a byte offset to a 0-based line index.
pub(crate) fn byte_offset_to_line(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(ix) => ix,
        Err(ix) => ix.saturating_sub(1),
    }
}

/// Compute a source line range from byte offsets.
///
/// `start_byte` is the start of the element, `end_byte` is its exclusive end.
/// Returns a half-open `Range<usize>` of 0-based line indices.
pub(crate) fn source_line_range(
    start_byte: usize,
    end_byte: usize,
    line_starts: &[usize],
) -> Range<usize> {
    let start_line = byte_offset_to_line(start_byte, line_starts);
    let end_line = byte_offset_to_line(end_byte.saturating_sub(1).max(start_byte), line_starts);
    start_line..end_line + 1
}

/// Determine change hint for a source line range.
pub(crate) fn line_range_change_hint(
    range: &Range<usize>,
    changed_mask: &[bool],
    is_old_side: bool,
) -> MarkdownChangeHint {
    if range.is_empty() || changed_mask.is_empty() {
        return MarkdownChangeHint::None;
    }

    let start = range.start.min(changed_mask.len());
    let end = range.end.min(changed_mask.len());
    if start >= end {
        return MarkdownChangeHint::None;
    }

    let changed_count = changed_mask[start..end].iter().filter(|&&c| c).count();
    if changed_count == 0 {
        MarkdownChangeHint::None
    } else if changed_count < end.saturating_sub(start) {
        MarkdownChangeHint::Modified
    } else if is_old_side {
        MarkdownChangeHint::Removed
    } else {
        MarkdownChangeHint::Added
    }
}
