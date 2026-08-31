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
    build_markdown_document(source)
}

pub(crate) fn build_markdown_document(source: &str) -> Option<MarkdownPreviewDocument> {
    let line_starts = build_line_starts(source);
    let rows = flatten_to_rows(source, &line_starts)?;
    Some(MarkdownPreviewDocument { rows })
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

pub(crate) fn build_markdown_diff_preview(
    old_source: &str,
    new_source: &str,
) -> Option<MarkdownPreviewDiff> {
    let (mut old, mut new) = parse_markdown_diff(old_source, new_source)?;
    let plan = gitcomet_core::file_diff::side_by_side_plan(old_source, new_source);
    let old_line_count = old_source.lines().count();
    let new_line_count = new_source.lines().count();
    let (old_mask, new_mask) =
        gitcomet_core::file_diff::plan_changed_line_masks(&plan, old_line_count, new_line_count);
    annotate_change_hints(&mut old, &mut new, &old_mask, &new_mask);
    let (old_line_to_diff_row, new_line_to_diff_row) =
        gitcomet_core::file_diff::plan_line_to_row_maps(&plan, old_line_count, new_line_count);
    align_markdown_diff_rows(
        &mut old,
        &mut new,
        old_line_to_diff_row.as_slice(),
        new_line_to_diff_row.as_slice(),
        plan.row_count,
    )?;
    let inline = build_inline_markdown_diff_document(&old, &new);
    Some(MarkdownPreviewDiff { old, new, inline })
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

pub(crate) fn align_markdown_diff_rows(
    old_doc: &mut MarkdownPreviewDocument,
    new_doc: &mut MarkdownPreviewDocument,
    old_line_to_diff_row: &[Option<usize>],
    new_line_to_diff_row: &[Option<usize>],
    diff_row_count: usize,
) -> Option<()> {
    let old_rows = std::mem::take(&mut old_doc.rows);
    let new_rows = std::mem::take(&mut new_doc.rows);
    // Each side ends up near max(old, new) plus padding rows -- not the sum --
    // and both vecs are moved into the documents below without shrinking, so an
    // over-estimate is retained for the cached document's lifetime.
    let aligned_capacity = old_rows.len().max(new_rows.len());

    let (mut old_groups, old_trailing) =
        markdown_rows_grouped_by_diff_anchor(old_rows, old_line_to_diff_row, diff_row_count);
    let (mut new_groups, new_trailing) =
        markdown_rows_grouped_by_diff_anchor(new_rows, new_line_to_diff_row, diff_row_count);

    let mut old_aligned = Vec::with_capacity(aligned_capacity);
    let mut new_aligned = Vec::with_capacity(aligned_capacity);

    for diff_ix in 0..diff_row_count {
        let old_group = std::mem::take(&mut old_groups[diff_ix]);
        let new_group = std::mem::take(&mut new_groups[diff_ix]);
        push_aligned_markdown_row_groups(&mut old_aligned, &mut new_aligned, old_group, new_group)?;
    }

    push_aligned_markdown_row_groups(
        &mut old_aligned,
        &mut new_aligned,
        old_trailing,
        new_trailing,
    )?;

    old_doc.rows = old_aligned;
    new_doc.rows = new_aligned;
    Some(())
}

pub(crate) fn markdown_rows_grouped_by_diff_anchor(
    rows: Vec<MarkdownPreviewRow>,
    line_to_diff_row: &[Option<usize>],
    diff_row_count: usize,
) -> (Vec<Vec<MarkdownPreviewRow>>, Vec<MarkdownPreviewRow>) {
    let mut groups = vec![Vec::new(); diff_row_count];
    let mut trailing = Vec::new();

    for row in rows {
        if let Some(anchor_ix) = markdown_row_diff_anchor(&row, line_to_diff_row)
            && let Some(group) = groups.get_mut(anchor_ix)
        {
            group.push(row);
            continue;
        }
        trailing.push(row);
    }

    (groups, trailing)
}

pub(crate) fn markdown_row_diff_anchor(
    row: &MarkdownPreviewRow,
    line_to_diff_row: &[Option<usize>],
) -> Option<usize> {
    if row.source_line_range.is_empty() {
        return None;
    }

    let start = row.source_line_range.start.min(line_to_diff_row.len());
    let end = row.source_line_range.end.min(line_to_diff_row.len());
    if start >= end {
        return None;
    }

    line_to_diff_row[start..end].iter().flatten().copied().min()
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
        kind: MarkdownPreviewRowKind::Spacer,
        text: SharedString::from(""),
        inline_spans: Arc::new(Vec::new()),
        code_language: None,
        code_block_horizontal_scroll_hint: false,
        source_line_range,
        change_hint: MarkdownChangeHint::None,
        indent_level: 0,
        blockquote_level: 0,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: MarkdownPreviewRowStyledTextCache::default(),
        measured_width_px: MarkdownPreviewRowWidthCache::default(),
    }
}

pub(crate) fn build_inline_markdown_diff_document(
    old_doc: &MarkdownPreviewDocument,
    new_doc: &MarkdownPreviewDocument,
) -> MarkdownPreviewDocument {
    let row_count = old_doc.rows.len().max(new_doc.rows.len());
    let mut rows = Vec::with_capacity(row_count);

    for row_ix in 0..row_count {
        let old_row = old_doc.rows.get(row_ix);
        let new_row = new_doc.rows.get(row_ix);

        match (old_row, new_row) {
            (Some(old_row), Some(new_row))
                if markdown_inline_diff_rows_can_merge(old_row, new_row) =>
            {
                rows.push(old_row.clone());
            }
            (Some(old_row), Some(new_row)) => {
                if !matches!(old_row.kind, MarkdownPreviewRowKind::Spacer) {
                    rows.push(old_row.clone());
                }
                if !matches!(new_row.kind, MarkdownPreviewRowKind::Spacer) {
                    rows.push(new_row.clone());
                }
            }
            (Some(old_row), None) => {
                if !matches!(old_row.kind, MarkdownPreviewRowKind::Spacer) {
                    rows.push(old_row.clone());
                }
            }
            (None, Some(new_row)) => {
                if !matches!(new_row.kind, MarkdownPreviewRowKind::Spacer) {
                    rows.push(new_row.clone());
                }
            }
            (None, None) => {}
        }
    }

    MarkdownPreviewDocument { rows }
}

pub(crate) fn markdown_inline_diff_rows_can_merge(
    old_row: &MarkdownPreviewRow,
    new_row: &MarkdownPreviewRow,
) -> bool {
    old_row.change_hint == MarkdownChangeHint::None
        && new_row.change_hint == MarkdownChangeHint::None
        && !matches!(old_row.kind, MarkdownPreviewRowKind::Spacer)
        && !matches!(new_row.kind, MarkdownPreviewRowKind::Spacer)
        && old_row.kind == new_row.kind
        && old_row.text == new_row.text
        && old_row.inline_spans == new_row.inline_spans
        && old_row.code_language == new_row.code_language
        && old_row.code_block_horizontal_scroll_hint == new_row.code_block_horizontal_scroll_hint
        && old_row.indent_level == new_row.indent_level
        && old_row.blockquote_level == new_row.blockquote_level
        && old_row.footnote_label == new_row.footnote_label
        && old_row.alert_kind == new_row.alert_kind
        && old_row.starts_alert == new_row.starts_alert
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
