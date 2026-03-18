use gpui::SharedString;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

/// Maximum source size (bytes) for a single markdown preview document.
pub(super) const MAX_PREVIEW_SOURCE_BYTES: usize = 1_024 * 1_024; // 1 MiB

/// Maximum combined source size (bytes) for a two-sided diff preview.
pub(super) const MAX_DIFF_PREVIEW_SOURCE_BYTES: usize = 2 * 1_024 * 1_024; // 2 MiB

/// Maximum number of preview rows per document.
pub(super) const MAX_PREVIEW_ROWS: usize = 20_000;

/// Maximum number of inline spans per row before degrading to plain text.
const MAX_INLINE_SPANS_PER_ROW: usize = 512;

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewDocument {
    pub(super) rows: Vec<MarkdownPreviewRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewDiff {
    pub(super) old: MarkdownPreviewDocument,
    pub(super) new: MarkdownPreviewDocument,
    pub(super) inline: MarkdownPreviewDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewRow {
    pub(super) kind: MarkdownPreviewRowKind,
    pub(super) text: SharedString,
    pub(super) inline_spans: Arc<Vec<MarkdownInlineSpan>>,
    pub(super) code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
    pub(super) code_block_horizontal_scroll_hint: bool,
    pub(super) source_line_range: Range<usize>,
    pub(super) change_hint: MarkdownChangeHint,
    pub(super) indent_level: u8,
    pub(super) blockquote_level: u8,
    pub(super) footnote_label: Option<SharedString>,
    pub(super) alert_kind: Option<MarkdownAlertKind>,
    pub(super) starts_alert: bool,
    pub(super) measured_width_px: MarkdownPreviewRowWidthCache,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarkdownPreviewRowKind {
    Heading { level: u8 },
    Paragraph,
    DetailsSummary,
    ListItem { number: Option<u64> },
    BlockquoteLine,
    CodeLine { is_first: bool, is_last: bool },
    ThematicBreak,
    TableRow { is_header: bool },
    PlainFallback,
    Spacer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownInlineSpan {
    pub(super) byte_range: Range<usize>,
    pub(super) style: MarkdownInlineStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarkdownInlineStyle {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
    Strikethrough,
    Link,
    Underline,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum MarkdownChangeHint {
    #[default]
    None,
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum MarkdownAlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkdownBlockQuoteContext {
    alert_kind: Option<MarkdownAlertKind>,
    emitted_row: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkdownFootnoteContext {
    label: SharedString,
    emitted_label: bool,
}

struct MarkdownPreviewRowInput<'a> {
    kind: MarkdownPreviewRowKind,
    text: &'a str,
    inline_spans: &'a [MarkdownInlineSpan],
    code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
    code_block_horizontal_scroll_hint: bool,
    source_line_range: Range<usize>,
    indent_level: u8,
    blockquote_level: u8,
}

impl<'a> MarkdownPreviewRowInput<'a> {
    fn plain(
        kind: MarkdownPreviewRowKind,
        text: &'a str,
        inline_spans: &'a [MarkdownInlineSpan],
        source_line_range: Range<usize>,
        indent_level: u8,
        blockquote_level: u8,
    ) -> Self {
        Self {
            kind,
            text,
            inline_spans,
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range,
            indent_level,
            blockquote_level,
        }
    }

    fn code(
        kind: MarkdownPreviewRowKind,
        text: &'a str,
        source_line_range: Range<usize>,
        code_language: Option<crate::view::rows::DiffSyntaxLanguage>,
        code_block_horizontal_scroll_hint: bool,
        indent_level: u8,
        blockquote_level: u8,
    ) -> Self {
        Self {
            kind,
            text,
            inline_spans: &[],
            code_language,
            code_block_horizontal_scroll_hint,
            source_line_range,
            indent_level,
            blockquote_level,
        }
    }
}

#[derive(Default)]
struct MarkdownPreviewRowDecoration {
    footnote_label: Option<SharedString>,
    alert_kind: Option<MarkdownAlertKind>,
    starts_alert: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct MarkdownPreviewRowWidthCache(OnceLock<u32>);

impl MarkdownPreviewRowWidthCache {
    pub(super) fn get_or_init(&self, compute: impl FnOnce() -> u32) -> u32 {
        *self.0.get_or_init(compute)
    }
}

impl PartialEq for MarkdownPreviewRowWidthCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for MarkdownPreviewRowWidthCache {}

// ── Error messages ──────────────────────────────────────────────────────

/// Return a user-facing reason why a single-document markdown preview is
/// unavailable for a source of `source_len` bytes.
pub(super) fn single_preview_unavailable_reason(source_len: usize) -> &'static str {
    if source_len > MAX_PREVIEW_SOURCE_BYTES {
        "Markdown preview unavailable: file exceeds the 1 MiB preview limit."
    } else {
        "Markdown preview unavailable: rendered row limit exceeded."
    }
}

/// Return a user-facing reason why a two-sided diff markdown preview is
/// unavailable for sources of `combined_len` bytes.
pub(super) fn diff_preview_unavailable_reason(combined_len: usize) -> &'static str {
    if combined_len > MAX_DIFF_PREVIEW_SOURCE_BYTES {
        "Markdown preview unavailable: diff exceeds the 2 MiB preview limit."
    } else {
        "Markdown preview unavailable: rendered row limit exceeded."
    }
}

// ── Parser ──────────────────────────────────────────────────────────────

/// Build a `MarkdownPreviewDocument` from raw markdown source text.
///
/// Returns `None` if the source exceeds `MAX_PREVIEW_SOURCE_BYTES`
/// or the parsed document exceeds `MAX_PREVIEW_ROWS`.
pub(super) fn parse_markdown(source: &str) -> Option<MarkdownPreviewDocument> {
    if source.len() > MAX_PREVIEW_SOURCE_BYTES {
        return None;
    }
    build_markdown_document(source)
}

fn build_markdown_document(source: &str) -> Option<MarkdownPreviewDocument> {
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
fn parse_markdown_diff(
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

pub(super) fn build_markdown_diff_preview(
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

pub(super) fn scrollbar_markers_for_diff_preview(
    preview: &MarkdownPreviewDiff,
) -> Vec<crate::view::components::ScrollbarMarker> {
    scrollbar_markers_for_documents(&[&preview.old, &preview.new])
}

pub(super) fn scrollbar_markers_for_document(
    document: &MarkdownPreviewDocument,
) -> Vec<crate::view::components::ScrollbarMarker> {
    scrollbar_markers_for_documents(&[document])
}

/// Annotate change hints on a pair of preview documents using diff row data.
///
/// `changed_old_lines` and `changed_new_lines` are sets of 0-based line
/// indices that have changes (derived from `FileDiffRow` data).
fn annotate_change_hints(
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

fn align_markdown_diff_rows(
    old_doc: &mut MarkdownPreviewDocument,
    new_doc: &mut MarkdownPreviewDocument,
    old_line_to_diff_row: &[Option<usize>],
    new_line_to_diff_row: &[Option<usize>],
    diff_row_count: usize,
) -> Option<()> {
    let old_rows = std::mem::take(&mut old_doc.rows);
    let new_rows = std::mem::take(&mut new_doc.rows);

    let (mut old_groups, old_trailing) =
        markdown_rows_grouped_by_diff_anchor(old_rows, old_line_to_diff_row, diff_row_count);
    let (mut new_groups, new_trailing) =
        markdown_rows_grouped_by_diff_anchor(new_rows, new_line_to_diff_row, diff_row_count);

    let mut old_aligned = Vec::new();
    let mut new_aligned = Vec::new();

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

fn markdown_rows_grouped_by_diff_anchor(
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

fn markdown_row_diff_anchor(
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

fn push_aligned_markdown_row_groups(
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

fn markdown_preview_spacer_row() -> MarkdownPreviewRow {
    markdown_preview_spacer_row_with_range(0..0)
}

fn markdown_preview_spacer_row_with_range(source_line_range: Range<usize>) -> MarkdownPreviewRow {
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
        measured_width_px: MarkdownPreviewRowWidthCache::default(),
    }
}

fn build_inline_markdown_diff_document(
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

fn markdown_inline_diff_rows_can_merge(
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

fn scrollbar_markers_for_documents(
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

    super::diff_utils::scrollbar_markers_from_flags(bucket_count, |bucket_ix| {
        buckets.get(bucket_ix).copied().unwrap_or(0)
    })
}

fn scrollbar_flag_for_change_hint(hint: MarkdownChangeHint) -> u8 {
    match hint {
        MarkdownChangeHint::None => 0,
        MarkdownChangeHint::Added => 1,
        MarkdownChangeHint::Removed => 2,
        MarkdownChangeHint::Modified => 3,
    }
}

/// Build a vec of byte offsets for the start of each line.
fn build_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Convert a byte offset to a 0-based line index.
fn byte_offset_to_line(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(ix) => ix,
        Err(ix) => ix.saturating_sub(1),
    }
}

/// Compute a source line range from byte offsets.
///
/// `start_byte` is the start of the element, `end_byte` is its exclusive end.
/// Returns a half-open `Range<usize>` of 0-based line indices.
fn source_line_range(start_byte: usize, end_byte: usize, line_starts: &[usize]) -> Range<usize> {
    let start_line = byte_offset_to_line(start_byte, line_starts);
    let end_line = byte_offset_to_line(end_byte.saturating_sub(1).max(start_byte), line_starts);
    start_line..end_line + 1
}

/// Determine change hint for a source line range.
fn line_range_change_hint(
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

/// Flatten markdown events into preview rows.
fn flatten_to_rows(source: &str, line_starts: &[usize]) -> Option<Vec<MarkdownPreviewRow>> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ListContext {
        Unordered,
        Ordered { next_number: u64 },
    }

    impl ListContext {
        fn next_item_kind(&mut self) -> MarkdownPreviewRowKind {
            match self {
                Self::Unordered => MarkdownPreviewRowKind::ListItem { number: None },
                Self::Ordered { next_number } => {
                    let number = *next_number;
                    *next_number = next_number.saturating_add(1);
                    MarkdownPreviewRowKind::ListItem {
                        number: Some(number),
                    }
                }
            }
        }
    }

    let options = markdown_parser_options();

    let mut rows = Vec::new();
    let mut text_buf = String::new();
    let mut span_stack: Vec<MarkdownInlineStyle> = Vec::new();
    let mut inline_spans: Vec<MarkdownInlineSpan> = Vec::new();
    let mut source_start_byte: usize = 0;
    let mut indent_level: u8 = 0;
    let mut list_stack: Vec<ListContext> = Vec::new();
    let mut list_item_stack: Vec<MarkdownPreviewRowKind> = Vec::new();
    let mut in_heading = false;
    let mut in_paragraph = false;
    let mut in_blockquote: u8 = 0;
    let mut blockquote_stack: Vec<MarkdownBlockQuoteContext> = Vec::new();
    let mut in_code_block = false;
    let mut in_table_row = false;
    let mut table_row_is_header = false;
    let mut code_block_start_byte: usize = 0;
    let mut code_block_starts_after_fence = false;
    let mut code_block_language: Option<crate::view::rows::DiffSyntaxLanguage> = None;
    let mut footnote_context: Option<MarkdownFootnoteContext> = None;

    for (event, event_range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_heading = true;
            }
            Event::End(TagEnd::Heading(level)) => {
                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::Heading { level: level as u8 },
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut blockquote_stack,
                )?;
                in_heading = false;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::Paragraph) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_paragraph = true;
            }
            Event::End(TagEnd::Paragraph) => {
                let kind = current_row_kind(&list_item_stack, in_blockquote);

                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        kind,
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut blockquote_stack,
                )?;
                in_paragraph = false;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::List(first_number)) => {
                // Flush any accumulated item text before entering the sub-list,
                // so the parent item gets its own row at the current indent level.
                if !text_buf.is_empty() && !list_item_stack.is_empty() {
                    let kind = list_item_stack
                        .last()
                        .copied()
                        .unwrap_or(MarkdownPreviewRowKind::ListItem { number: None });
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::plain(
                            kind,
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.start, line_starts),
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut blockquote_stack,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                }
                list_stack.push(match first_number {
                    Some(next_number) => ListContext::Ordered { next_number },
                    None => ListContext::Unordered,
                });
                indent_level = indent_level.saturating_add(1);
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                indent_level = indent_level.saturating_sub(1);
            }

            Event::Start(Tag::Item) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                if let Some(context) = list_stack.last_mut() {
                    list_item_stack.push(context.next_item_kind());
                }
            }
            Event::End(TagEnd::Item) => {
                // Only emit a row if there is text that hasn't already been
                // emitted by a nested paragraph or sub-list.
                if !text_buf.is_empty() {
                    let kind = list_item_stack
                        .last()
                        .copied()
                        .unwrap_or(MarkdownPreviewRowKind::ListItem { number: None });
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::plain(
                            kind,
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.end, line_starts),
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut blockquote_stack,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                }
                list_item_stack.pop();
            }

            Event::Start(Tag::BlockQuote(kind)) => {
                blockquote_stack.push(MarkdownBlockQuoteContext {
                    alert_kind: kind.and_then(markdown_alert_kind_from_blockquote_kind),
                    emitted_row: false,
                });
                in_blockquote = in_blockquote.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                blockquote_stack.pop();
                in_blockquote = in_blockquote.saturating_sub(1);
            }

            Event::Start(Tag::FootnoteDefinition(label)) => {
                footnote_context = Some(MarkdownFootnoteContext {
                    label: label.to_string().into(),
                    emitted_label: false,
                });
                indent_level = indent_level.saturating_add(1);
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                footnote_context = None;
                indent_level = indent_level.saturating_sub(1);
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_block_start_byte = event_range.start;
                code_block_language = match &kind {
                    CodeBlockKind::Fenced(info) => {
                        crate::view::rows::diff_syntax_language_for_code_fence_info(info.as_ref())
                    }
                    CodeBlockKind::Indented => None,
                };
                code_block_starts_after_fence = matches!(kind, CodeBlockKind::Fenced(_));
                text_buf.clear();
                inline_spans.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                // Emit one row per code line.
                let block_range =
                    source_line_range(code_block_start_byte, event_range.end, line_starts);
                let block_start_line = block_range.start;
                let block_end_line = block_range.end.saturating_sub(1);
                let content_start_line =
                    block_start_line + usize::from(code_block_starts_after_fence);
                let code_text = text_buf.strip_suffix('\n').unwrap_or(&text_buf);
                let code_lines: Vec<&str> = if code_text.is_empty() {
                    vec![""]
                } else {
                    code_text.split('\n').collect()
                };
                let code_block_horizontal_scroll_hint = code_lines
                    .iter()
                    .any(|line| line.contains('\t') || line.chars().count() > 80);
                let last_ix = code_lines.len().saturating_sub(1);
                for (i, line) in code_lines.iter().enumerate() {
                    let line_ix = (content_start_line + i).min(block_end_line);
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::code(
                            MarkdownPreviewRowKind::CodeLine {
                                is_first: i == 0,
                                is_last: i == last_ix,
                            },
                            line,
                            line_ix..line_ix + 1,
                            code_block_language,
                            code_block_horizontal_scroll_hint,
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut blockquote_stack,
                    )?;
                }
                in_code_block = false;
                code_block_starts_after_fence = false;
                code_block_language = None;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::TableHead) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_table_row = true;
                table_row_is_header = true;
            }
            Event::Start(Tag::TableRow) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_table_row = true;
                table_row_is_header = false;
            }
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::TableRow {
                            is_header: table_row_is_header,
                        },
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut blockquote_stack,
                )?;
                in_table_row = false;
                table_row_is_header = false;
                text_buf.clear();
                inline_spans.clear();
            }
            Event::End(TagEnd::TableCell) => {
                // Separate cells with a tab character for display.
                text_buf.push('\t');
            }

            // Inline styling tags
            Event::Start(Tag::Strong) => {
                span_stack.push(MarkdownInlineStyle::Bold);
            }
            Event::Start(Tag::Emphasis) => {
                span_stack.push(MarkdownInlineStyle::Italic);
            }
            Event::Start(Tag::Strikethrough) => {
                span_stack.push(MarkdownInlineStyle::Strikethrough);
            }
            Event::Start(Tag::Link { .. }) => {
                span_stack.push(MarkdownInlineStyle::Link);
            }
            Event::End(
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                span_stack.pop();
            }

            Event::Text(cow) => {
                let style = resolve_style_stack(&span_stack);
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                if style != MarkdownInlineStyle::Normal && !in_code_block {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style,
                    });
                }
            }

            Event::Code(cow) => {
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                if !in_code_block {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style: MarkdownInlineStyle::Code,
                    });
                }
            }

            Event::FootnoteReference(label) => {
                let start = text_buf.len();
                text_buf.push('[');
                text_buf.push_str(&label);
                text_buf.push(']');
                let end = text_buf.len();
                if !in_code_block {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style: MarkdownInlineStyle::Link,
                    });
                }
            }

            Event::SoftBreak => {
                if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                    if !text_buf.is_empty() {
                        push_row_with_context(
                            &mut rows,
                            MarkdownPreviewRowInput::plain(
                                MarkdownPreviewRowKind::BlockquoteLine,
                                &text_buf,
                                &inline_spans,
                                source_line_range(
                                    source_start_byte,
                                    event_range.start,
                                    line_starts,
                                ),
                                indent_level,
                                in_blockquote,
                            ),
                            footnote_context.as_mut(),
                            &mut blockquote_stack,
                        )?;
                        text_buf.clear();
                        inline_spans.clear();
                    }
                    source_start_byte = event_range.end;
                } else if !text_buf.is_empty() {
                    text_buf.push(' ');
                }
            }
            Event::HardBreak => {
                if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                    if !text_buf.is_empty() {
                        push_row_with_context(
                            &mut rows,
                            MarkdownPreviewRowInput::plain(
                                MarkdownPreviewRowKind::BlockquoteLine,
                                &text_buf,
                                &inline_spans,
                                source_line_range(
                                    source_start_byte,
                                    event_range.start,
                                    line_starts,
                                ),
                                indent_level,
                                in_blockquote,
                            ),
                            footnote_context.as_mut(),
                            &mut blockquote_stack,
                        )?;
                        text_buf.clear();
                        inline_spans.clear();
                    }
                    source_start_byte = event_range.end;
                } else if !in_code_block && !in_heading && !text_buf.is_empty() {
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::plain(
                            current_row_kind(&list_item_stack, in_blockquote),
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.start, line_starts),
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut blockquote_stack,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                    source_start_byte = event_range.end;
                } else if !text_buf.is_empty() {
                    text_buf.push(' ');
                }
            }

            Event::Rule => {
                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::ThematicBreak,
                        "───",
                        &[],
                        source_line_range(event_range.start, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut blockquote_stack,
                )?;
            }

            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                text_buf.insert_str(0, marker);
                // Shift existing span byte ranges.
                let shift = marker.len();
                for span in &mut inline_spans {
                    span.byte_range.start += shift;
                    span.byte_range.end += shift;
                }
            }

            Event::Html(cow) | Event::InlineHtml(cow) => {
                match classify_supported_html(cow.as_ref()) {
                    HtmlHandling::Ignore => continue,
                    HtmlHandling::HardBreak => {
                        if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                            if !text_buf.is_empty() {
                                push_row_with_context(
                                    &mut rows,
                                    MarkdownPreviewRowInput::plain(
                                        MarkdownPreviewRowKind::BlockquoteLine,
                                        &text_buf,
                                        &inline_spans,
                                        source_line_range(
                                            source_start_byte,
                                            event_range.start,
                                            line_starts,
                                        ),
                                        indent_level,
                                        in_blockquote,
                                    ),
                                    footnote_context.as_mut(),
                                    &mut blockquote_stack,
                                )?;
                                text_buf.clear();
                                inline_spans.clear();
                            }
                            source_start_byte = event_range.end;
                            continue;
                        }
                        if !in_code_block && !in_heading && !text_buf.is_empty() {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text_buf,
                                    &inline_spans,
                                    source_line_range(
                                        source_start_byte,
                                        event_range.start,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut blockquote_stack,
                            )?;
                            text_buf.clear();
                            inline_spans.clear();
                            source_start_byte = event_range.end;
                        }
                        continue;
                    }
                    HtmlHandling::DetailsSummary(summary_source) => {
                        if !text_buf.is_empty() {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text_buf,
                                    &inline_spans,
                                    source_line_range(
                                        source_start_byte,
                                        event_range.start,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut blockquote_stack,
                            )?;
                            text_buf.clear();
                            inline_spans.clear();
                        }

                        let (summary_text, summary_spans) =
                            parse_inline_markdown_fragment(&summary_source);
                        if !summary_text.is_empty() {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    MarkdownPreviewRowKind::DetailsSummary,
                                    &summary_text,
                                    &summary_spans,
                                    source_line_range(
                                        event_range.start,
                                        event_range.end,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut blockquote_stack,
                            )?;
                        }
                        source_start_byte = event_range.end;
                        continue;
                    }
                    HtmlHandling::StartInlineStyle(style) => {
                        span_stack.push(style);
                        continue;
                    }
                    HtmlHandling::EndInlineStyle(style) => {
                        pop_matching_inline_style(&mut span_stack, style);
                        continue;
                    }
                    HtmlHandling::AppendText(text) => {
                        let should_append = html_event_should_append(
                            in_paragraph,
                            in_heading,
                            !list_stack.is_empty(),
                            in_blockquote,
                            in_code_block,
                            in_table_row,
                        );
                        if should_append {
                            text_buf.push_str(&text);
                        } else {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text,
                                    &[],
                                    source_line_range(
                                        event_range.start,
                                        event_range.end,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut blockquote_stack,
                            )?;
                        }
                        continue;
                    }
                    HtmlHandling::AppendLiteral => {}
                }

                let should_append = html_event_should_append(
                    in_paragraph,
                    in_heading,
                    !list_stack.is_empty(),
                    in_blockquote,
                    in_code_block,
                    in_table_row,
                );
                if should_append {
                    text_buf.push_str(&cow);
                } else {
                    push_plain_fallback_rows(
                        &mut rows,
                        cow.as_ref(),
                        event_range.start,
                        event_range.end,
                        line_starts,
                        indent_level,
                        in_blockquote,
                    )?;
                }
            }

            // Ignore footnotes, metadata, and math in v1.
            _ => {}
        }
    }

    align_table_columns(&mut rows);
    insert_top_level_heading_spacer_rows(&mut rows);
    Some(rows)
}

fn insert_top_level_heading_spacer_rows(rows: &mut Vec<MarkdownPreviewRow>) {
    if rows.len() < 2 {
        return;
    }

    let mut spaced_rows = Vec::with_capacity(rows.len() + rows.len() / 4);
    let mut pending_gap_after_heading: Option<Range<usize>> = None;

    for row in rows.drain(..) {
        let is_top_level_heading = markdown_row_is_top_level_heading(&row);
        if let Some(source_line_range) = pending_gap_after_heading.take()
            && !is_top_level_heading
        {
            spaced_rows.push(markdown_preview_spacer_row_with_range(source_line_range));
        }

        if is_top_level_heading {
            let has_content_before_heading = matches!(
                spaced_rows.last(),
                Some(previous_row)
                    if !matches!(
                        previous_row.kind,
                        MarkdownPreviewRowKind::Spacer | MarkdownPreviewRowKind::Heading { .. }
                    )
            );

            if has_content_before_heading {
                spaced_rows.push(markdown_preview_spacer_row_with_range(
                    row.source_line_range.clone(),
                ));
            } else {
                pending_gap_after_heading = Some(row.source_line_range.clone());
            }
        }

        spaced_rows.push(row);
    }

    *rows = spaced_rows;
}

fn markdown_row_is_top_level_heading(row: &MarkdownPreviewRow) -> bool {
    matches!(row.kind, MarkdownPreviewRowKind::Heading { .. })
        && row.indent_level == 0
        && row.blockquote_level == 0
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum HtmlHandling {
    Ignore,
    HardBreak,
    DetailsSummary(String),
    StartInlineStyle(MarkdownInlineStyle),
    EndInlineStyle(MarkdownInlineStyle),
    AppendText(String),
    AppendLiteral,
}

fn current_row_kind(
    list_item_stack: &[MarkdownPreviewRowKind],
    blockquote_level: u8,
) -> MarkdownPreviewRowKind {
    if let Some(kind) = list_item_stack.last().copied() {
        kind
    } else if blockquote_level > 0 {
        MarkdownPreviewRowKind::BlockquoteLine
    } else {
        MarkdownPreviewRowKind::Paragraph
    }
}

fn markdown_alert_kind_from_blockquote_kind(
    kind: pulldown_cmark::BlockQuoteKind,
) -> Option<MarkdownAlertKind> {
    Some(match kind {
        pulldown_cmark::BlockQuoteKind::Note => MarkdownAlertKind::Note,
        pulldown_cmark::BlockQuoteKind::Tip => MarkdownAlertKind::Tip,
        pulldown_cmark::BlockQuoteKind::Important => MarkdownAlertKind::Important,
        pulldown_cmark::BlockQuoteKind::Warning => MarkdownAlertKind::Warning,
        pulldown_cmark::BlockQuoteKind::Caution => MarkdownAlertKind::Caution,
    })
}

fn html_event_should_append(
    in_paragraph: bool,
    in_heading: bool,
    in_list: bool,
    blockquote_level: u8,
    in_code_block: bool,
    in_table_row: bool,
) -> bool {
    in_paragraph || in_heading || in_list || blockquote_level > 0 || in_code_block || in_table_row
}

fn markdown_parser_options() -> pulldown_cmark::Options {
    pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS
        | pulldown_cmark::Options::ENABLE_FOOTNOTES
        | pulldown_cmark::Options::ENABLE_GFM
}

fn classify_supported_html(html: &str) -> HtmlHandling {
    let trimmed = html.trim();
    if trimmed.is_empty() {
        return HtmlHandling::Ignore;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<!--") {
        return HtmlHandling::Ignore;
    }
    if let Some(summary_source) = extract_html_summary_content(trimmed) {
        return HtmlHandling::DetailsSummary(summary_source);
    }
    if let Some(alt_text) = extract_html_image_alt(trimmed) {
        return HtmlHandling::AppendText(alt_text);
    }
    if matches!(lower.as_str(), "<br>" | "<br/>" | "<br />") {
        return HtmlHandling::HardBreak;
    }
    if matches!(lower.as_str(), "<ins>") {
        return HtmlHandling::StartInlineStyle(MarkdownInlineStyle::Underline);
    }
    if matches!(lower.as_str(), "</ins>") {
        return HtmlHandling::EndInlineStyle(MarkdownInlineStyle::Underline);
    }
    if matches!(lower.as_str(), "<sub>" | "</sub>" | "<sup>" | "</sup>") {
        return HtmlHandling::Ignore;
    }
    if lower.starts_with("<a ") && (lower.contains(" name=") || lower.contains(" id=")) {
        return HtmlHandling::Ignore;
    }
    if lower.starts_with("<a ") && lower.contains(" href=") {
        return HtmlHandling::StartInlineStyle(MarkdownInlineStyle::Link);
    }
    if lower == "</a>" {
        return HtmlHandling::EndInlineStyle(MarkdownInlineStyle::Link);
    }
    if lower.starts_with("<picture")
        || lower == "</picture>"
        || lower.starts_with("<source")
        || lower == "</source>"
    {
        return HtmlHandling::Ignore;
    }
    if is_html_open_tag(lower.as_str(), "details") || is_html_close_tag(lower.as_str(), "details") {
        return HtmlHandling::Ignore;
    }

    HtmlHandling::AppendLiteral
}

fn is_html_open_tag(lower_html: &str, tag_name: &str) -> bool {
    if !lower_html.starts_with('<') || lower_html.starts_with("</") {
        return false;
    }

    let Some(rest) = lower_html.strip_prefix('<') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(tag_name) else {
        return false;
    };

    rest.is_empty()
        || rest.starts_with('>')
        || rest.starts_with('/')
        || rest.starts_with(char::is_whitespace)
}

fn is_html_close_tag(lower_html: &str, tag_name: &str) -> bool {
    let Some(rest) = lower_html.strip_prefix("</") else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(tag_name) else {
        return false;
    };

    rest.is_empty() || rest.starts_with('>') || rest.starts_with(char::is_whitespace)
}

fn extract_html_summary_content(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open_ix = lower.find("<summary")?;
    let start_tag_end_rel = html[open_ix..].find('>')?;
    let content_start = open_ix + start_tag_end_rel + 1;
    let close_rel = lower[content_start..].find("</summary>")?;
    Some(html[content_start..content_start + close_rel].to_owned())
}

fn extract_html_image_alt(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let img_ix = lower.find("<img")?;
    extract_html_attribute(&html[img_ix..], "alt")
}

fn extract_html_attribute(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("{name}=");
    let mut search_start = 0;

    while let Some(rel_ix) = lower[search_start..].find(&needle) {
        let attr_ix = search_start + rel_ix;
        if attr_ix > 0 {
            let prev = lower.as_bytes()[attr_ix - 1];
            if !prev.is_ascii_whitespace() && prev != b'<' {
                search_start = attr_ix + needle.len();
                continue;
            }
        }

        let value_start = attr_ix + needle.len();
        if value_start >= html.len() {
            return None;
        }

        let value = &html[value_start..];
        let mut chars = value.chars();
        let first = chars.next()?;
        if first == '"' || first == '\'' {
            let end_rel = value[1..].find(first)?;
            return Some(value[1..1 + end_rel].to_owned());
        }

        let end = value
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '>' | '/'))
            .unwrap_or(value.len());
        return Some(value[..end].to_owned());
    }

    None
}

fn pop_matching_inline_style(stack: &mut Vec<MarkdownInlineStyle>, style: MarkdownInlineStyle) {
    if let Some(ix) = stack.iter().rposition(|s| *s == style) {
        stack.remove(ix);
    }
}

fn strip_generic_html_tags(fragment: &str) -> String {
    let mut stripped = String::with_capacity(fragment.len());
    let mut chars = fragment.chars().peekable();
    let mut in_tag = false;

    while let Some(ch) = chars.next() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
            }
            continue;
        }

        if ch == '<'
            && chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?'))
        {
            in_tag = true;
            continue;
        }

        stripped.push(ch);
    }

    stripped
}

fn parse_inline_markdown_fragment(source: &str) -> (String, Vec<MarkdownInlineSpan>) {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut text_buf = String::new();
    let mut span_stack = Vec::new();
    let mut inline_spans = Vec::new();

    for event in Parser::new_ext(source, markdown_parser_options()) {
        match event {
            Event::Start(Tag::Strong) => span_stack.push(MarkdownInlineStyle::Bold),
            Event::Start(Tag::Emphasis) => span_stack.push(MarkdownInlineStyle::Italic),
            Event::Start(Tag::Strikethrough) => {
                span_stack.push(MarkdownInlineStyle::Strikethrough);
            }
            Event::Start(Tag::Link { .. }) => span_stack.push(MarkdownInlineStyle::Link),
            Event::End(
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                span_stack.pop();
            }
            Event::Text(cow) => {
                let style = resolve_style_stack(&span_stack);
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                if style != MarkdownInlineStyle::Normal {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style,
                    });
                }
            }
            Event::Code(cow) => {
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                inline_spans.push(MarkdownInlineSpan {
                    byte_range: start..end,
                    style: MarkdownInlineStyle::Code,
                });
            }
            Event::FootnoteReference(label) => {
                let start = text_buf.len();
                text_buf.push('[');
                text_buf.push_str(&label);
                text_buf.push(']');
                let end = text_buf.len();
                inline_spans.push(MarkdownInlineSpan {
                    byte_range: start..end,
                    style: MarkdownInlineStyle::Link,
                });
            }
            Event::SoftBreak | Event::HardBreak => {
                if !text_buf.is_empty() {
                    text_buf.push(' ');
                }
            }
            Event::Html(cow) | Event::InlineHtml(cow) => {
                match classify_supported_html(cow.as_ref()) {
                    HtmlHandling::Ignore => {}
                    HtmlHandling::HardBreak => {
                        if !text_buf.is_empty() {
                            text_buf.push(' ');
                        }
                    }
                    HtmlHandling::DetailsSummary(summary_source) => {
                        let summary_text = strip_generic_html_tags(&summary_source);
                        if !summary_text.is_empty() {
                            if !text_buf.is_empty() {
                                text_buf.push(' ');
                            }
                            text_buf.push_str(&summary_text);
                        }
                    }
                    HtmlHandling::StartInlineStyle(style) => span_stack.push(style),
                    HtmlHandling::EndInlineStyle(style) => {
                        pop_matching_inline_style(&mut span_stack, style);
                    }
                    HtmlHandling::AppendText(text) => {
                        text_buf.push_str(&text);
                    }
                    HtmlHandling::AppendLiteral => {
                        text_buf.push_str(&strip_generic_html_tags(cow.as_ref()));
                    }
                }
            }
            _ => {}
        }
    }

    normalize_whitespace_with_spans(&text_buf, &inline_spans)
}

fn push_row_with_context(
    rows: &mut Vec<MarkdownPreviewRow>,
    row: MarkdownPreviewRowInput<'_>,
    footnote_context: Option<&mut MarkdownFootnoteContext>,
    blockquote_stack: &mut [MarkdownBlockQuoteContext],
) -> Option<()> {
    let footnote_label = footnote_context.and_then(|ctx| {
        if ctx.emitted_label {
            None
        } else {
            ctx.emitted_label = true;
            Some(ctx.label.clone())
        }
    });

    let mut decoration = MarkdownPreviewRowDecoration {
        footnote_label,
        ..MarkdownPreviewRowDecoration::default()
    };
    if let Some(alert_ix) = blockquote_stack
        .iter()
        .rposition(|ctx| ctx.alert_kind.is_some())
    {
        let ctx = &mut blockquote_stack[alert_ix];
        decoration.alert_kind = ctx.alert_kind;
        if !ctx.emitted_row {
            ctx.emitted_row = true;
            decoration.starts_alert = true;
        }
    }

    push_row(rows, row, decoration)
}

fn push_row(
    rows: &mut Vec<MarkdownPreviewRow>,
    row: MarkdownPreviewRowInput<'_>,
    decoration: MarkdownPreviewRowDecoration,
) -> Option<()> {
    let (row_text, row_spans) = match row.kind {
        // Paragraph-like rows collapse whitespace, so remap inline spans to
        // the normalized text instead of leaving them pointed at stale bytes.
        MarkdownPreviewRowKind::Paragraph
        | MarkdownPreviewRowKind::DetailsSummary
        | MarkdownPreviewRowKind::BlockquoteLine => {
            normalize_whitespace_with_spans(row.text, row.inline_spans)
        }
        _ => (row.text.to_owned(), row.inline_spans.to_vec()),
    };
    let spans = if row_spans.len() > MAX_INLINE_SPANS_PER_ROW {
        Arc::new(Vec::new())
    } else {
        Arc::new(row_spans)
    };

    rows.push(MarkdownPreviewRow {
        kind: row.kind,
        text: SharedString::from(row_text),
        inline_spans: spans,
        code_language: row.code_language,
        code_block_horizontal_scroll_hint: row.code_block_horizontal_scroll_hint,
        source_line_range: row.source_line_range,
        change_hint: MarkdownChangeHint::None,
        indent_level: row.indent_level,
        blockquote_level: row.blockquote_level,
        footnote_label: decoration.footnote_label,
        alert_kind: decoration.alert_kind,
        starts_alert: decoration.starts_alert,
        measured_width_px: MarkdownPreviewRowWidthCache::default(),
    });

    (rows.len() <= MAX_PREVIEW_ROWS).then_some(())
}

fn push_plain_fallback_rows(
    rows: &mut Vec<MarkdownPreviewRow>,
    text: &str,
    start_byte: usize,
    end_byte: usize,
    line_starts: &[usize],
    indent_level: u8,
    blockquote_level: u8,
) -> Option<()> {
    let range = source_line_range(start_byte, end_byte, line_starts);
    let segments = if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect::<Vec<_>>()
    };
    let end_line = range.end.saturating_sub(1);

    for (ix, segment) in segments.into_iter().enumerate() {
        let line_ix = (range.start + ix).min(end_line);
        push_row(
            rows,
            MarkdownPreviewRowInput::plain(
                MarkdownPreviewRowKind::PlainFallback,
                segment,
                &[],
                line_ix..line_ix.saturating_add(1),
                indent_level,
                blockquote_level,
            ),
            MarkdownPreviewRowDecoration::default(),
        )?;
    }

    Some(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkdownTableCell {
    text: String,
    spans: Vec<MarkdownInlineSpan>,
}

fn align_table_columns(rows: &mut [MarkdownPreviewRow]) {
    let mut start = 0usize;
    while start < rows.len() {
        if !matches!(rows[start].kind, MarkdownPreviewRowKind::TableRow { .. }) {
            start += 1;
            continue;
        }

        let mut end = start + 1;
        while end < rows.len() && matches!(rows[end].kind, MarkdownPreviewRowKind::TableRow { .. })
        {
            end += 1;
        }

        align_table_block_rows(&mut rows[start..end]);
        start = end;
    }
}

fn align_table_block_rows(rows: &mut [MarkdownPreviewRow]) {
    let split_rows = rows
        .iter()
        .map(|row| split_markdown_table_cells(row.text.as_ref(), row.inline_spans.as_ref()))
        .collect::<Vec<_>>();
    let column_count = split_rows.iter().map(Vec::len).max().unwrap_or(0);
    if column_count == 0 {
        return;
    }

    let mut column_widths = vec![0usize; column_count];
    for cells in &split_rows {
        for (ix, cell) in cells.iter().enumerate() {
            column_widths[ix] = column_widths[ix].max(cell.text.chars().count());
        }
    }

    for (row, cells) in rows.iter_mut().zip(split_rows) {
        let (text, spans) = build_aligned_table_row_text(cells, &column_widths);
        row.text = text.into();
        row.inline_spans = Arc::new(spans);
    }
}

fn split_markdown_table_cells(
    text: &str,
    inline_spans: &[MarkdownInlineSpan],
) -> Vec<MarkdownTableCell> {
    let mut cell_ranges = Vec::new();
    let mut cell_start = 0usize;
    for (byte_ix, ch) in text.char_indices() {
        if ch == '\t' {
            cell_ranges.push(cell_start..byte_ix);
            cell_start = byte_ix + ch.len_utf8();
        }
    }
    if cell_ranges.is_empty() || cell_start < text.len() {
        cell_ranges.push(cell_start..text.len());
    }

    cell_ranges
        .into_iter()
        .map(|range| {
            let cell_text = text
                .get(range.clone())
                .map(str::to_owned)
                .unwrap_or_default();
            let spans = inline_spans
                .iter()
                .filter_map(|span| {
                    let start = span.byte_range.start.max(range.start);
                    let end = span.byte_range.end.min(range.end);
                    if start < end {
                        Some(MarkdownInlineSpan {
                            byte_range: (start - range.start)..(end - range.start),
                            style: span.style,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            MarkdownTableCell {
                text: cell_text,
                spans,
            }
        })
        .collect()
}

fn build_aligned_table_row_text(
    cells: Vec<MarkdownTableCell>,
    column_widths: &[usize],
) -> (String, Vec<MarkdownInlineSpan>) {
    const TABLE_COLUMN_SEPARATOR: &str = " | ";

    let mut text = String::new();
    let mut spans = Vec::new();
    let mut cells = cells.into_iter().map(Some).collect::<Vec<_>>();

    for (ix, width) in column_widths.iter().copied().enumerate() {
        let cell = cells.get_mut(ix).and_then(Option::take);
        let cell_width = cell
            .as_ref()
            .map(|cell| cell.text.chars().count())
            .unwrap_or(0);
        let cell_start = text.len();
        if let Some(cell) = cell {
            text.push_str(&cell.text);
            spans.extend(cell.spans.into_iter().map(|span| MarkdownInlineSpan {
                byte_range: (cell_start + span.byte_range.start)
                    ..(cell_start + span.byte_range.end),
                style: span.style,
            }));
        }

        if ix + 1 < column_widths.len() {
            let pad = width.saturating_sub(cell_width);
            for _ in 0..pad {
                text.push(' ');
            }
            text.push_str(TABLE_COLUMN_SEPARATOR);
        }
    }

    (text, spans)
}

fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result
}

fn normalize_whitespace_with_spans(
    text: &str,
    inline_spans: &[MarkdownInlineSpan],
) -> (String, Vec<MarkdownInlineSpan>) {
    if inline_spans.is_empty() {
        return (normalize_whitespace(text), Vec::new());
    }

    let mut normalized = String::with_capacity(text.len());
    let mut byte_map = vec![0usize; text.len() + 1];
    let mut prev_ws = false;
    let mut normalized_len = 0usize;

    for (byte_ix, ch) in text.char_indices() {
        byte_map[byte_ix] = normalized_len;
        if ch.is_whitespace() {
            if !prev_ws {
                normalized.push(' ');
                normalized_len += 1;
            }
            prev_ws = true;
        } else {
            normalized.push(ch);
            normalized_len += ch.len_utf8();
            prev_ws = false;
        }
        byte_map[byte_ix + ch.len_utf8()] = normalized_len;
    }

    let remapped_spans = inline_spans
        .iter()
        .filter_map(|span| {
            debug_assert!(text.is_char_boundary(span.byte_range.start));
            debug_assert!(text.is_char_boundary(span.byte_range.end));
            let start = *byte_map.get(span.byte_range.start)?;
            let end = *byte_map.get(span.byte_range.end)?;
            (start < end).then_some(MarkdownInlineSpan {
                byte_range: start..end,
                style: span.style,
            })
        })
        .collect();

    (normalized, remapped_spans)
}

/// Combine the inline style stack into a single effective style.
fn resolve_style_stack(stack: &[MarkdownInlineStyle]) -> MarkdownInlineStyle {
    let mut has_bold = false;
    let mut has_italic = false;
    let mut has_strikethrough = false;
    let mut has_link = false;
    let mut has_code = false;
    let mut has_underline = false;

    for &s in stack {
        match s {
            MarkdownInlineStyle::Bold => has_bold = true,
            MarkdownInlineStyle::Italic => has_italic = true,
            MarkdownInlineStyle::Strikethrough => has_strikethrough = true,
            MarkdownInlineStyle::Link => has_link = true,
            MarkdownInlineStyle::Code => has_code = true,
            MarkdownInlineStyle::Underline => has_underline = true,
            _ => {}
        }
    }

    if has_code {
        MarkdownInlineStyle::Code
    } else if has_bold && has_italic {
        MarkdownInlineStyle::BoldItalic
    } else if has_bold {
        MarkdownInlineStyle::Bold
    } else if has_italic {
        MarkdownInlineStyle::Italic
    } else if has_strikethrough {
        MarkdownInlineStyle::Strikethrough
    } else if has_link {
        MarkdownInlineStyle::Link
    } else if has_underline {
        MarkdownInlineStyle::Underline
    } else {
        MarkdownInlineStyle::Normal
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> MarkdownPreviewDocument {
        parse_markdown(src).expect("parse should succeed")
    }

    fn thematic_break_rows(count: usize) -> String {
        "---\n".repeat(count)
    }

    fn row_kinds(doc: &MarkdownPreviewDocument) -> Vec<&MarkdownPreviewRowKind> {
        doc.rows.iter().map(|r| &r.kind).collect()
    }

    fn row_texts(doc: &MarkdownPreviewDocument) -> Vec<&str> {
        doc.rows.iter().map(|r| r.text.as_ref()).collect()
    }

    fn code_rows(doc: &MarkdownPreviewDocument) -> Vec<&MarkdownPreviewRow> {
        doc.rows
            .iter()
            .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::CodeLine { .. }))
            .collect()
    }

    fn spans_with_style(
        row: &MarkdownPreviewRow,
        style: MarkdownInlineStyle,
    ) -> Vec<&MarkdownInlineSpan> {
        row.inline_spans
            .iter()
            .filter(|s| s.style == style)
            .collect()
    }

    // ── Heading tests ───────────────────────────────────────────────────

    #[test]
    fn heading_levels_are_preserved() {
        let doc = parse("# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n");
        assert_eq!(
            row_kinds(&doc),
            vec![
                &MarkdownPreviewRowKind::Heading { level: 1 },
                &MarkdownPreviewRowKind::Heading { level: 2 },
                &MarkdownPreviewRowKind::Heading { level: 3 },
                &MarkdownPreviewRowKind::Heading { level: 4 },
                &MarkdownPreviewRowKind::Heading { level: 5 },
                &MarkdownPreviewRowKind::Heading { level: 6 },
            ]
        );
        assert_eq!(row_texts(&doc), vec!["H1", "H2", "H3", "H4", "H5", "H6"]);
    }

    #[test]
    fn top_level_heading_inserts_section_spacer_before_following_content() {
        let doc = parse("# Title\n\nParagraph\n");

        assert_eq!(doc.rows.len(), 3);
        assert_eq!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        );
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(doc.rows[1].change_hint, MarkdownChangeHint::None);
        assert_eq!(doc.rows[2].kind, MarkdownPreviewRowKind::Paragraph);
    }

    #[test]
    fn content_before_top_level_heading_gets_section_spacer_before_heading() {
        let doc = parse("Paragraph\n\n# Title\n");

        assert_eq!(doc.rows.len(), 3);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(
            doc.rows[2].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        );
    }

    #[test]
    fn consecutive_headings_do_not_insert_spacers_between_heading_rows() {
        let doc = parse("# Title\n## Subtitle\n");

        assert_eq!(
            row_kinds(&doc),
            vec![
                &MarkdownPreviewRowKind::Heading { level: 1 },
                &MarkdownPreviewRowKind::Heading { level: 2 },
            ]
        );
    }

    #[test]
    fn middle_heading_uses_a_single_section_spacer_before_the_heading() {
        let doc = parse("Intro\n\n# Title\n\nBody\n");

        assert_eq!(
            row_kinds(&doc),
            vec![
                &MarkdownPreviewRowKind::Paragraph,
                &MarkdownPreviewRowKind::Spacer,
                &MarkdownPreviewRowKind::Heading { level: 1 },
                &MarkdownPreviewRowKind::Paragraph,
            ]
        );
    }

    // ── Paragraph tests ─────────────────────────────────────────────────

    #[test]
    fn paragraph_produces_one_row() {
        let doc = parse("Hello world.\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[0].text.as_ref(), "Hello world.");
    }

    #[test]
    fn multiline_paragraph_normalizes_whitespace() {
        let doc = parse("Line one\nLine two\nLine three\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].text.as_ref(), "Line one Line two Line three");
    }

    #[test]
    fn hard_breaks_split_paragraph_rows() {
        let doc = parse("This example  \nWill span two lines\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[0].text.as_ref(), "This example");
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[1].text.as_ref(), "Will span two lines");
    }

    #[test]
    fn backslash_hard_breaks_split_paragraph_rows() {
        let doc = parse("This example\\\nWill span two lines\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].text.as_ref(), "This example");
        assert_eq!(doc.rows[1].text.as_ref(), "Will span two lines");
    }

    #[test]
    fn html_br_splits_paragraph_rows() {
        let doc = parse("This example<br/>\nWill span two lines\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].text.as_ref(), "This example");
        assert_eq!(doc.rows[1].text.as_ref(), "Will span two lines");
    }

    #[test]
    fn whitespace_normalization_preserves_inline_span_offsets() {
        let doc = parse("Prefix  **bold**\nnext line\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].text.as_ref(), "Prefix bold next line");

        let bold_span = doc.rows[0]
            .inline_spans
            .iter()
            .find(|span| span.style == MarkdownInlineStyle::Bold)
            .expect("expected bold span");
        assert_eq!(
            &doc.rows[0].text.as_ref()[bold_span.byte_range.clone()],
            "bold"
        );
    }

    // ── List tests ──────────────────────────────────────────────────────

    #[test]
    fn unordered_list_items_become_rows() {
        let doc = parse("- alpha\n- beta\n- gamma\n");
        assert_eq!(doc.rows.len(), 3);
        for row in &doc.rows {
            assert_eq!(row.kind, MarkdownPreviewRowKind::ListItem { number: None });
        }
        assert_eq!(row_texts(&doc), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn ordered_list_items_preserve_numbers() {
        let doc = parse("3. first\n4. second\n5. third\n");
        assert_eq!(doc.rows.len(), 3);
        assert_eq!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::ListItem { number: Some(3) }
        );
        assert_eq!(
            doc.rows[1].kind,
            MarkdownPreviewRowKind::ListItem { number: Some(4) }
        );
        assert_eq!(
            doc.rows[2].kind,
            MarkdownPreviewRowKind::ListItem { number: Some(5) }
        );
    }

    #[test]
    fn loose_list_items_still_render_as_list_rows() {
        let doc = parse("- first\n\n- second\n");
        assert_eq!(doc.rows.len(), 2);
        for row in &doc.rows {
            assert_eq!(row.kind, MarkdownPreviewRowKind::ListItem { number: None });
        }
    }

    #[test]
    fn nested_list_increases_indent() {
        let doc = parse("- outer\n  - inner\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].indent_level, 1);
        assert_eq!(doc.rows[1].indent_level, 2);
    }

    // ── Blockquote tests ────────────────────────────────────────────────

    #[test]
    fn blockquote_produces_blockquote_row() {
        let doc = parse("> quoted text\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::BlockquoteLine);
        assert_eq!(doc.rows[0].text.as_ref(), "quoted text");
    }

    #[test]
    fn multiline_blockquote_produces_one_row_per_logical_quote_line() {
        let doc = parse("> first line\n> second line\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::BlockquoteLine);
        assert_eq!(doc.rows[0].text.as_ref(), "first line");
        assert_eq!(doc.rows[0].source_line_range, 0..1);
        assert_eq!(doc.rows[0].blockquote_level, 1);
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::BlockquoteLine);
        assert_eq!(doc.rows[1].text.as_ref(), "second line");
        assert_eq!(doc.rows[1].source_line_range, 1..2);
        assert_eq!(doc.rows[1].blockquote_level, 1);
    }

    #[test]
    fn nested_blockquotes_preserve_quote_depth_per_row() {
        let doc = parse("> outer\n>> inner\n>>> deepest\n");
        assert_eq!(doc.rows.len(), 3);
        assert_eq!(doc.rows[0].blockquote_level, 1);
        assert_eq!(doc.rows[1].blockquote_level, 2);
        assert_eq!(doc.rows[2].blockquote_level, 3);
    }

    #[test]
    fn list_items_inside_blockquotes_keep_quote_depth() {
        let doc = parse("> - first\n>> 3. second\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::ListItem { number: None }
        );
        assert_eq!(doc.rows[0].blockquote_level, 1);
        assert_eq!(
            doc.rows[1].kind,
            MarkdownPreviewRowKind::ListItem { number: Some(3) }
        );
        assert_eq!(doc.rows[1].blockquote_level, 2);
    }

    #[test]
    fn code_block_inside_blockquote_keeps_quote_depth() {
        let doc = parse("> ```\n> code\n> ```\n");
        let cr = code_rows(&doc);
        assert_eq!(cr.len(), 1);
        assert_eq!(cr[0].text.as_ref(), "code");
        assert_eq!(cr[0].blockquote_level, 1);
    }

    #[test]
    fn gfm_alert_blockquotes_capture_alert_kind_and_hide_marker_line() {
        let doc = parse("> [!NOTE]\n> Line 1.\n> Line 2.\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::BlockquoteLine);
        assert_eq!(doc.rows[0].text.as_ref(), "Line 1.");
        assert_eq!(doc.rows[0].alert_kind, Some(MarkdownAlertKind::Note));
        assert!(doc.rows[0].starts_alert);
        assert_eq!(doc.rows[1].text.as_ref(), "Line 2.");
        assert_eq!(doc.rows[1].alert_kind, Some(MarkdownAlertKind::Note));
        assert!(!doc.rows[1].starts_alert);
    }

    #[test]
    fn nested_alert_blockquotes_stay_scoped_to_inner_quote_rows() {
        let doc = parse("> outer\n>\n> > [!WARNING]\n> > inner\n>\n> outer again\n");
        assert_eq!(doc.rows.len(), 3);

        assert_eq!(doc.rows[0].text.as_ref(), "outer");
        assert_eq!(doc.rows[0].blockquote_level, 1);
        assert_eq!(doc.rows[0].alert_kind, None);
        assert!(!doc.rows[0].starts_alert);

        assert_eq!(doc.rows[1].text.as_ref(), "inner");
        assert_eq!(doc.rows[1].blockquote_level, 2);
        assert_eq!(doc.rows[1].alert_kind, Some(MarkdownAlertKind::Warning));
        assert!(doc.rows[1].starts_alert);

        assert_eq!(doc.rows[2].text.as_ref(), "outer again");
        assert_eq!(doc.rows[2].blockquote_level, 1);
        assert_eq!(doc.rows[2].alert_kind, None);
        assert!(!doc.rows[2].starts_alert);
    }

    // ── Code block tests ────────────────────────────────────────────────

    #[test]
    fn fenced_code_block_one_row_per_line() {
        let doc = parse("```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 3);
        assert_eq!(code_rows[0].text.as_ref(), "fn main() {");
        assert_eq!(code_rows[1].text.as_ref(), "    println!(\"hi\");");
        assert_eq!(code_rows[2].text.as_ref(), "}");
        assert_eq!(
            code_rows[0].code_language,
            Some(crate::view::rows::DiffSyntaxLanguage::Rust)
        );
    }

    #[test]
    fn code_block_first_last_flags() {
        let doc = parse("```\na\nb\nc\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 3);
        assert!(matches!(
            code_rows[0].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: false
            }
        ));
        assert!(matches!(
            code_rows[1].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: false
            }
        ));
        assert!(matches!(
            code_rows[2].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: true
            }
        ));
    }

    #[test]
    fn single_line_code_block_is_both_first_and_last() {
        let doc = parse("```\nonly\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 1);
        assert_eq!(code_rows[0].text.as_ref(), "only");
        assert!(matches!(
            code_rows[0].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: true
            }
        ));
    }

    #[test]
    fn indented_code_block_rows_keep_actual_source_line_ranges() {
        let doc = parse("    old\n    keep\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 2);
        assert_eq!(code_rows[0].text.as_ref(), "old");
        assert_eq!(code_rows[0].source_line_range, 0..1);
        assert_eq!(code_rows[1].text.as_ref(), "keep");
        assert_eq!(code_rows[1].source_line_range, 1..2);
    }

    #[test]
    fn fenced_code_block_preserves_trailing_blank_line() {
        let doc = parse("```\na\n\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 2);
        assert_eq!(code_rows[0].text.as_ref(), "a");
        assert_eq!(code_rows[0].source_line_range, 1..2);
        assert_eq!(code_rows[1].text.as_ref(), "");
        assert_eq!(code_rows[1].source_line_range, 2..3);
        assert!(matches!(
            code_rows[1].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: true
            }
        ));
    }

    #[test]
    fn empty_fenced_code_block_produces_single_empty_row() {
        let doc = parse("```\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 1);
        assert_eq!(code_rows[0].text.as_ref(), "");
        assert!(matches!(
            code_rows[0].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: true
            }
        ));
        assert_eq!(code_rows[0].code_language, None);
    }

    #[test]
    fn fenced_code_block_language_aliases_are_resolved() {
        let doc = parse("```language-typescript\nconst x = 1;\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 1);
        assert_eq!(
            code_rows[0].code_language,
            Some(crate::view::rows::DiffSyntaxLanguage::TypeScript)
        );
    }

    #[test]
    fn wide_fenced_code_blocks_set_horizontal_scroll_hints() {
        let long_line = "scroll_hint_token_".repeat(6);
        let doc = parse(&format!("```text\n{long_line}\nshort\n```\n"));
        let code_rows = code_rows(&doc);

        assert_eq!(code_rows.len(), 2);
        assert!(
            code_rows
                .iter()
                .all(|row| row.code_block_horizontal_scroll_hint)
        );
    }

    // ── Thematic break ──────────────────────────────────────────────────

    #[test]
    fn thematic_break_produces_row() {
        let doc = parse("---\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::ThematicBreak);
    }

    // ── Task list ───────────────────────────────────────────────────────

    #[test]
    fn task_list_markers_are_prepended() {
        let doc = parse("- [x] done\n- [ ] todo\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].text.as_ref(), "[x] done");
        assert_eq!(doc.rows[1].text.as_ref(), "[ ] todo");
    }

    #[test]
    fn footnote_references_and_definitions_are_preserved() {
        let doc = parse("Here is a simple footnote[^1].\n\n[^1]: My reference.\n");
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].text.as_ref(), "Here is a simple footnote[1].");
        let links = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Link);
        assert_eq!(links.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[links[0].byte_range.clone()],
            "[1]"
        );

        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[1].text.as_ref(), "My reference.");
        assert_eq!(
            doc.rows[1]
                .footnote_label
                .as_ref()
                .map(SharedString::as_ref),
            Some("1")
        );
        assert_eq!(doc.rows[1].indent_level, 1);
    }

    #[test]
    fn footnote_definition_emits_label_only_for_first_rendered_row() {
        let doc = parse("Reference[^1].\n\n[^1]: First paragraph.\n\n    Second paragraph.\n");
        assert_eq!(doc.rows.len(), 3);

        assert_eq!(doc.rows[1].text.as_ref(), "First paragraph.");
        assert_eq!(
            doc.rows[1]
                .footnote_label
                .as_ref()
                .map(SharedString::as_ref),
            Some("1")
        );
        assert_eq!(doc.rows[1].indent_level, 1);

        assert_eq!(doc.rows[2].text.as_ref(), "Second paragraph.");
        assert_eq!(doc.rows[2].footnote_label, None);
        assert_eq!(doc.rows[2].indent_level, 1);
    }

    // ── Table ───────────────────────────────────────────────────────────

    #[test]
    fn table_rows_are_flattened() {
        let doc = parse("| A | B |\n|---|---|\n| 1 | 2 |\n");
        let table_rows: Vec<_> = doc
            .rows
            .iter()
            .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
            .collect();
        assert!(table_rows.len() >= 2);
        assert!(matches!(
            table_rows[0].kind,
            MarkdownPreviewRowKind::TableRow { is_header: true }
        ));
        assert_eq!(table_rows[0].text.as_ref(), "A | B");
        assert_eq!(table_rows[1].text.as_ref(), "1 | 2");
    }

    #[test]
    fn table_rows_align_columns_across_block() {
        let doc = parse("| Name | Age |\n|---|---|\n| Alexander | 3 |\n| Bo | 27 |\n");
        let table_rows: Vec<_> = doc
            .rows
            .iter()
            .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 3);

        let header_sep = table_rows[0]
            .text
            .find('|')
            .expect("header row should contain a column separator");
        let first_row_sep = table_rows[1]
            .text
            .find('|')
            .expect("body row should contain a column separator");
        let second_row_sep = table_rows[2]
            .text
            .find('|')
            .expect("body row should contain a column separator");

        assert_eq!(header_sep, first_row_sep);
        assert_eq!(first_row_sep, second_row_sep);
    }

    #[test]
    fn table_alignment_preserves_inline_spans_after_padding_cells() {
        let doc = parse(
            "| A | **Header Bold** |\n| --- | --- |\n| A much longer first column | [link](https://example.com) |\n",
        );
        let table_rows: Vec<_> = doc
            .rows
            .iter()
            .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 2);

        let header_sep = table_rows[0]
            .text
            .find('|')
            .expect("header row should contain a column separator");
        let body_sep = table_rows[1]
            .text
            .find('|')
            .expect("body row should contain a column separator");
        assert_eq!(header_sep, body_sep);

        let header_bold = spans_with_style(table_rows[0], MarkdownInlineStyle::Bold);
        assert_eq!(header_bold.len(), 1);
        assert_eq!(
            &table_rows[0].text.as_ref()[header_bold[0].byte_range.clone()],
            "Header Bold"
        );

        let body_links = spans_with_style(table_rows[1], MarkdownInlineStyle::Link);
        assert_eq!(body_links.len(), 1);
        assert_eq!(
            &table_rows[1].text.as_ref()[body_links[0].byte_range.clone()],
            "link"
        );
    }

    #[test]
    fn table_alignment_handles_inline_spans_in_earlier_cells() {
        let doc = parse(
            "| **Header Bold** | B |\n| --- | --- |\n| [link](https://example.com) | plain |\n",
        );
        let table_rows: Vec<_> = doc
            .rows
            .iter()
            .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 2);

        let header_bold = spans_with_style(table_rows[0], MarkdownInlineStyle::Bold);
        assert_eq!(header_bold.len(), 1);
        assert_eq!(
            &table_rows[0].text.as_ref()[header_bold[0].byte_range.clone()],
            "Header Bold"
        );

        let body_links = spans_with_style(table_rows[1], MarkdownInlineStyle::Link);
        assert_eq!(body_links.len(), 1);
        assert_eq!(
            &table_rows[1].text.as_ref()[body_links[0].byte_range.clone()],
            "link"
        );
    }

    #[test]
    fn row_width_cache_does_not_affect_preview_row_equality() {
        let cached = parse("Paragraph\n").rows.remove(0);
        let fresh = parse("Paragraph\n").rows.remove(0);

        cached.measured_width_px.get_or_init(|| 123);

        assert_eq!(cached, fresh);
    }

    // ── Inline spans ────────────────────────────────────────────────────

    #[test]
    fn bold_text_produces_bold_span() {
        let doc = parse("Some **bold** text\n");
        assert_eq!(doc.rows.len(), 1);
        let bold = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Bold);
        assert_eq!(bold.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[bold[0].byte_range.clone()],
            "bold"
        );
    }

    #[test]
    fn italic_text_produces_italic_span() {
        let doc = parse("Some *italic* text\n");
        assert_eq!(
            spans_with_style(&doc.rows[0], MarkdownInlineStyle::Italic).len(),
            1
        );
    }

    #[test]
    fn inline_code_produces_code_span() {
        let doc = parse("Use `code` here\n");
        let code = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Code);
        assert_eq!(code.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[code[0].byte_range.clone()],
            "code"
        );
    }

    #[test]
    fn strikethrough_produces_span() {
        let doc = parse("Some ~~struck~~ text\n");
        assert_eq!(
            spans_with_style(&doc.rows[0], MarkdownInlineStyle::Strikethrough).len(),
            1
        );
    }

    #[test]
    fn link_produces_link_span() {
        let doc = parse("[click](http://example.com)\n");
        let links = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Link);
        assert_eq!(links.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[links[0].byte_range.clone()],
            "click"
        );
    }

    #[test]
    fn inline_html_links_preserve_text_and_link_span() {
        let doc = parse("Built with <a href=\"https://pages.github.com/\">GitHub Pages</a>.\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].text.as_ref(), "Built with GitHub Pages.");
        let links = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Link);
        assert_eq!(links.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[links[0].byte_range.clone()],
            "GitHub Pages"
        );
    }

    #[test]
    fn bold_italic_produces_bold_italic_span() {
        let doc = parse("***both***\n");
        assert_eq!(
            spans_with_style(&doc.rows[0], MarkdownInlineStyle::BoldItalic).len(),
            1
        );
    }

    #[test]
    fn underline_html_produces_underline_span() {
        let doc = parse("This is an <ins>underlined</ins> text\n");
        assert_eq!(doc.rows[0].text.as_ref(), "This is an underlined text");
        let underline = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Underline);
        assert_eq!(underline.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[underline[0].byte_range.clone()],
            "underlined"
        );
    }

    #[test]
    fn subscript_and_superscript_tags_are_stripped_from_preview_text() {
        let doc = parse("This is a <sub>subscript</sub> and <sup>superscript</sup> text\n");
        assert_eq!(
            doc.rows[0].text.as_ref(),
            "This is a subscript and superscript text"
        );
    }

    #[test]
    fn details_summary_renders_as_structured_preview_rows() {
        let doc = parse(
            "<details open>\n<summary>**Quick start**</summary>\n\nInstall the package.\n</details>\n",
        );

        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::DetailsSummary);
        assert_eq!(doc.rows[0].text.as_ref(), "Quick start");
        let summary_bold = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Bold);
        assert_eq!(summary_bold.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[summary_bold[0].byte_range.clone()],
            "Quick start"
        );

        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[1].text.as_ref(), "Install the package.");
    }

    #[test]
    fn details_summary_on_same_html_line_ignores_wrapper_tags() {
        let doc = parse(
            "<details><summary><strong>Examples</strong> and `usage`</summary>\n\nBody text.\n</details>\n",
        );

        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::DetailsSummary);
        assert_eq!(doc.rows[0].text.as_ref(), "Examples and usage");
        let summary_code = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Code);
        assert_eq!(summary_code.len(), 1);
        assert_eq!(
            &doc.rows[0].text.as_ref()[summary_code[0].byte_range.clone()],
            "usage"
        );
        assert_eq!(doc.rows[1].text.as_ref(), "Body text.");
    }

    #[test]
    fn escaped_markdown_characters_remain_literal() {
        let doc = parse("Let's rename \\*our-new-project\\* to \\*our-old-project\\*.\n");
        assert_eq!(
            doc.rows[0].text.as_ref(),
            "Let's rename *our-new-project* to *our-old-project*."
        );
        assert!(doc.rows[0].inline_spans.is_empty());
    }

    #[test]
    fn excessive_inline_spans_degrade_to_plain_text() {
        // Build a paragraph with more than MAX_INLINE_SPANS_PER_ROW inline
        // code spans so the cap fires and all styling is dropped.
        let mut src = String::new();
        for i in 0..MAX_INLINE_SPANS_PER_ROW + 10 {
            if i > 0 {
                src.push(' ');
            }
            src.push_str(&format!("`s{i}`"));
        }
        src.push('\n');

        let doc = parse(&src);
        assert_eq!(doc.rows.len(), 1);
        assert!(
            doc.rows[0].inline_spans.is_empty(),
            "expected all spans to be dropped when exceeding MAX_INLINE_SPANS_PER_ROW, got {}",
            doc.rows[0].inline_spans.len()
        );
    }

    #[test]
    fn normalize_whitespace_with_spans_handles_multibyte_utf8() {
        // Emoji and accented characters with inline bold around a non-ASCII word.
        let doc = parse("café  **résumé**\nnext\n");
        assert_eq!(doc.rows.len(), 1);
        // Whitespace should be collapsed and span should point at the bold text.
        assert_eq!(doc.rows[0].text.as_ref(), "café résumé next");
        let bold_span = doc.rows[0]
            .inline_spans
            .iter()
            .find(|s| s.style == MarkdownInlineStyle::Bold)
            .expect("expected bold span");
        assert_eq!(
            &doc.rows[0].text.as_ref()[bold_span.byte_range.clone()],
            "résumé"
        );
    }

    // ── Source line range tests ──────────────────────────────────────────

    #[test]
    fn source_line_ranges_are_plausible() {
        let doc = parse("# Heading\n\nParagraph\n");
        assert!(!doc.rows[0].source_line_range.is_empty());
        assert!(doc.rows[0].source_line_range.start < 5);
    }

    // ── Change hint annotation tests ────────────────────────────────────

    #[test]
    fn change_hints_mark_changed_rows() {
        let old_src = "# Title\n\nOld paragraph\n";
        let new_src = "# Title\n\nNew paragraph\n";
        let (mut old_doc, mut new_doc) = parse_markdown_diff(old_src, new_src).unwrap();

        // Line 2 (0-based) is changed in both.
        let old_mask = vec![false, false, true];
        let new_mask = vec![false, false, true];
        annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

        // Title row should be unchanged.
        assert_eq!(old_doc.rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_doc.rows[0].change_hint, MarkdownChangeHint::None);

        // Paragraph row should be marked.
        let old_para = old_doc
            .rows
            .iter()
            .find(|r| r.text.as_ref() == "Old paragraph")
            .unwrap();
        assert_eq!(old_para.change_hint, MarkdownChangeHint::Removed);
        let new_para = new_doc
            .rows
            .iter()
            .find(|r| r.text.as_ref() == "New paragraph")
            .unwrap();
        assert_eq!(new_para.change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn heading_spacing_rows_do_not_receive_change_hints() {
        let preview =
            build_markdown_diff_preview("# Title\n\nOld paragraph\n", "# Title\n\nNew paragraph\n")
                .unwrap();

        let old_spacer = preview
            .old
            .rows
            .iter()
            .find(|row| matches!(row.kind, MarkdownPreviewRowKind::Spacer))
            .expect("expected heading spacer row on old side");
        let new_spacer = preview
            .new
            .rows
            .iter()
            .find(|row| matches!(row.kind, MarkdownPreviewRowKind::Spacer))
            .expect("expected heading spacer row on new side");

        assert_eq!(old_spacer.change_hint, MarkdownChangeHint::None);
        assert_eq!(new_spacer.change_hint, MarkdownChangeHint::None);
    }

    #[test]
    fn partial_change_ranges_use_modified_hint() {
        let (mut old_doc, mut new_doc) =
            parse_markdown_diff("line one\nline two\n", "line one\nline two\n").unwrap();

        let old_mask = vec![false, true];
        let new_mask = vec![false, true];
        annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

        assert_eq!(old_doc.rows[0].change_hint, MarkdownChangeHint::Modified);
        assert_eq!(new_doc.rows[0].change_hint, MarkdownChangeHint::Modified);
    }

    #[test]
    fn list_item_change_hints_follow_changed_lines() {
        let old_src = "- keep\n- remove me\n";
        let new_src = "- keep\n- add me\n";
        let (mut old_doc, mut new_doc) = parse_markdown_diff(old_src, new_src).unwrap();

        let old_mask = vec![false, true];
        let new_mask = vec![false, true];
        annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

        assert_eq!(old_doc.rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(old_doc.rows[1].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(new_doc.rows[1].change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn changed_code_lines_are_marked_individually() {
        let old_src = "```\nold\nkeep\n```\n";
        let new_src = "```\nnew\nkeep\n```\n";
        let (mut old_doc, mut new_doc) = parse_markdown_diff(old_src, new_src).unwrap();

        let old_mask = vec![false, true, false, false];
        let new_mask = vec![false, true, false, false];
        annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

        let old_code_rows = code_rows(&old_doc);
        let new_code_rows = code_rows(&new_doc);
        assert_eq!(old_code_rows[0].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_code_rows[0].change_hint, MarkdownChangeHint::Added);
        assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::None);
    }

    #[test]
    fn changed_indented_code_lines_are_marked_individually() {
        let preview =
            build_markdown_diff_preview("    old\n    keep\n", "    new\n    keep\n").unwrap();

        let old_code_rows = code_rows(&preview.old);
        let new_code_rows = code_rows(&preview.new);

        assert_eq!(old_code_rows[0].source_line_range, 0..1);
        assert_eq!(old_code_rows[1].source_line_range, 1..2);
        assert_eq!(new_code_rows[0].source_line_range, 0..1);
        assert_eq!(new_code_rows[1].source_line_range, 1..2);
        assert_eq!(old_code_rows[0].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_code_rows[0].change_hint, MarkdownChangeHint::Added);
        assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::None);
    }

    #[test]
    fn changed_trailing_blank_code_line_is_marked_individually() {
        let preview = build_markdown_diff_preview("```\na\n\n```\n", "```\na\nb\n```\n").unwrap();

        let old_code_rows = code_rows(&preview.old);
        let new_code_rows = code_rows(&preview.new);

        assert_eq!(old_code_rows.len(), 2);
        assert_eq!(new_code_rows.len(), 2);
        assert_eq!(old_code_rows[1].text.as_ref(), "");
        assert_eq!(new_code_rows[1].text.as_ref(), "b");
        assert_eq!(old_code_rows[1].source_line_range, 2..3);
        assert_eq!(new_code_rows[1].source_line_range, 2..3);
        assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn build_markdown_diff_preview_applies_change_hints() {
        let preview = build_markdown_diff_preview("- old item\n", "- new item\n").unwrap();

        assert_eq!(preview.old.rows.len(), 1);
        assert_eq!(preview.new.rows.len(), 1);
        assert_eq!(preview.old.rows[0].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(preview.new.rows[0].change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn diff_preview_inserts_spacer_rows_for_added_markdown_blocks() {
        let preview = build_markdown_diff_preview("- keep\n", "- keep\n- add me\n").unwrap();

        assert_eq!(preview.old.rows.len(), 2);
        assert_eq!(preview.new.rows.len(), 2);
        assert_eq!(preview.old.rows[0].text.as_ref(), "keep");
        assert_eq!(preview.new.rows[0].text.as_ref(), "keep");
        assert_eq!(preview.old.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(preview.old.rows[1].change_hint, MarkdownChangeHint::None);
        assert_eq!(
            preview.new.rows[1].kind,
            MarkdownPreviewRowKind::ListItem { number: None }
        );
        assert_eq!(preview.new.rows[1].text.as_ref(), "add me");
        assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn diff_preview_inserts_spacer_rows_for_removed_markdown_blocks() {
        let preview = build_markdown_diff_preview("keep\n\nremove me\n", "keep\n").unwrap();

        assert_eq!(preview.old.rows.len(), 2);
        assert_eq!(preview.new.rows.len(), 2);
        assert_eq!(preview.old.rows[0].text.as_ref(), "keep");
        assert_eq!(preview.new.rows[0].text.as_ref(), "keep");
        assert_eq!(preview.old.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(preview.old.rows[1].text.as_ref(), "remove me");
        assert_eq!(preview.old.rows[1].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(preview.new.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::None);
    }

    #[test]
    fn diff_preview_builds_inline_document_for_changed_rows() {
        let preview =
            build_markdown_diff_preview("keep\n\nremove me\n", "keep\n\nadd me\n").unwrap();

        assert_eq!(preview.inline.rows.len(), 3);
        assert_eq!(preview.inline.rows[0].text.as_ref(), "keep");
        assert_eq!(preview.inline.rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(preview.inline.rows[1].text.as_ref(), "remove me");
        assert_eq!(
            preview.inline.rows[1].change_hint,
            MarkdownChangeHint::Removed
        );
        assert_eq!(preview.inline.rows[2].text.as_ref(), "add me");
        assert_eq!(
            preview.inline.rows[2].change_hint,
            MarkdownChangeHint::Added
        );
    }

    #[test]
    fn diff_preview_inline_document_merges_unchanged_rows_after_insertions() {
        let preview = build_markdown_diff_preview("- keep\n", "- add\n- keep\n").unwrap();

        assert_eq!(preview.inline.rows.len(), 2);
        assert_eq!(preview.inline.rows[0].text.as_ref(), "add");
        assert_eq!(
            preview.inline.rows[0].change_hint,
            MarkdownChangeHint::Added
        );
        assert_eq!(preview.inline.rows[1].text.as_ref(), "keep");
        assert_eq!(preview.inline.rows[1].change_hint, MarkdownChangeHint::None);
    }

    #[test]
    fn diff_preview_aligns_added_code_lines_with_spacer_rows() {
        let preview =
            build_markdown_diff_preview("```\nkeep\n```\n", "```\nkeep\nadd\n```\n").unwrap();

        assert_eq!(preview.old.rows.len(), 2);
        assert_eq!(preview.new.rows.len(), 2);
        assert!(matches!(
            preview.old.rows[0].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: true
            }
        ));
        assert!(matches!(
            preview.new.rows[0].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: false
            }
        ));
        assert_eq!(preview.old.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert!(matches!(
            preview.new.rows[1].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: true
            }
        ));
        assert_eq!(preview.new.rows[1].text.as_ref(), "add");
        assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn diff_preview_marks_last_line_change_with_trailing_newline() {
        // The diff engine and mask sizing both use str::lines(), which strips
        // trailing newlines. Verify that a change on the very last line is still
        // detected and annotated correctly regardless of trailing newline.
        let preview =
            build_markdown_diff_preview("# Same\n\nold last\n", "# Same\n\nnew last\n").unwrap();

        let old_last = preview.old.rows.last().unwrap();
        let new_last = preview.new.rows.last().unwrap();
        assert_eq!(old_last.text.as_ref(), "old last");
        assert_eq!(new_last.text.as_ref(), "new last");
        assert_eq!(old_last.change_hint, MarkdownChangeHint::Removed);
        assert_eq!(new_last.change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn diff_preview_marks_last_line_change_without_trailing_newline() {
        let preview =
            build_markdown_diff_preview("# Same\n\nold last", "# Same\n\nnew last").unwrap();

        let old_last = preview.old.rows.last().unwrap();
        let new_last = preview.new.rows.last().unwrap();
        assert_eq!(old_last.text.as_ref(), "old last");
        assert_eq!(new_last.text.as_ref(), "new last");
        assert_eq!(old_last.change_hint, MarkdownChangeHint::Removed);
        assert_eq!(new_last.change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn multiline_blockquote_change_hints_follow_changed_quote_lines() {
        let preview =
            build_markdown_diff_preview("> keep\n> remove me\n", "> keep\n> add me\n").unwrap();

        assert_eq!(preview.old.rows.len(), 2);
        assert_eq!(preview.new.rows.len(), 2);
        assert_eq!(preview.old.rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(preview.new.rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(preview.old.rows[1].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::Added);
    }

    #[test]
    fn mixed_markdown_blocks_keep_change_hints_scoped_to_changed_rows() {
        let old_src = concat!(
            "# Title\n",
            "\n",
            "- keep\n",
            "- old item\n",
            "\n",
            "```rust\n",
            "let old_value = 1;\n",
            "let stable = 2;\n",
            "```\n",
            "\n",
            "| Name | Count |\n",
            "| --- | --- |\n",
            "| keep | 1 |\n",
            "| old | 2 |\n",
        );
        let new_src = concat!(
            "# Title\n",
            "\n",
            "- keep\n",
            "- new item\n",
            "\n",
            "```rust\n",
            "let new_value = 1;\n",
            "let stable = 2;\n",
            "```\n",
            "\n",
            "| Name | Count |\n",
            "| --- | --- |\n",
            "| keep | 1 |\n",
            "| new | 3 |\n",
        );

        let preview = build_markdown_diff_preview(old_src, new_src).unwrap();

        assert_eq!(
            preview.old.rows[0].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        );
        assert_eq!(preview.old.rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(preview.new.rows[0].change_hint, MarkdownChangeHint::None);

        let old_list_rows: Vec<_> = preview
            .old
            .rows
            .iter()
            .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }))
            .collect();
        let new_list_rows: Vec<_> = preview
            .new
            .rows
            .iter()
            .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }))
            .collect();
        assert_eq!(old_list_rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_list_rows[0].change_hint, MarkdownChangeHint::None);
        assert_ne!(old_list_rows[1].change_hint, MarkdownChangeHint::None);
        assert_ne!(new_list_rows[1].change_hint, MarkdownChangeHint::None);

        let old_code_rows = code_rows(&preview.old);
        let new_code_rows = code_rows(&preview.new);
        assert_eq!(old_code_rows[0].change_hint, MarkdownChangeHint::Removed);
        assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_code_rows[0].change_hint, MarkdownChangeHint::Added);
        assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::None);

        let old_table_rows: Vec<_> = preview
            .old
            .rows
            .iter()
            .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::TableRow { .. }))
            .collect();
        let new_table_rows: Vec<_> = preview
            .new
            .rows
            .iter()
            .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::TableRow { .. }))
            .collect();
        assert_eq!(old_table_rows.len(), 3);
        assert_eq!(new_table_rows.len(), 3);
        assert_eq!(old_table_rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_table_rows[0].change_hint, MarkdownChangeHint::None);
        assert_eq!(old_table_rows[1].change_hint, MarkdownChangeHint::None);
        assert_eq!(new_table_rows[1].change_hint, MarkdownChangeHint::None);
        assert_ne!(old_table_rows[2].change_hint, MarkdownChangeHint::None);
        assert_ne!(new_table_rows[2].change_hint, MarkdownChangeHint::None);
    }

    // ── plan_changed_line_masks ──────────────────────────────────────────

    #[test]
    fn plan_changed_line_masks_from_plan_rows() {
        use gitcomet_core::file_diff::{FileDiffPlan, FileDiffPlanRun};

        let plan = FileDiffPlan {
            runs: vec![
                FileDiffPlanRun::Context {
                    old_start: 0,
                    new_start: 0,
                    len: 1,
                },
                FileDiffPlanRun::Remove {
                    old_start: 1,
                    len: 1,
                },
                FileDiffPlanRun::Add {
                    new_start: 1,
                    len: 1,
                },
            ],
            row_count: 3,
            inline_row_count: 3,
            eof_newline: None,
        };

        let (old_mask, new_mask) = gitcomet_core::file_diff::plan_changed_line_masks(&plan, 3, 3);
        assert!(!old_mask[0]); // context line
        assert!(old_mask[1]); // removed line
        assert!(!new_mask[0]); // context line
        assert!(new_mask[1]); // added line
    }

    // ── Limit tests ─────────────────────────────────────────────────────

    #[test]
    fn parse_returns_none_for_oversized_source() {
        let huge = "x".repeat(MAX_PREVIEW_SOURCE_BYTES + 1);
        assert!(parse_markdown(&huge).is_none());
    }

    #[test]
    fn parse_returns_none_when_rendered_rows_exceed_limit() {
        let too_many_rows = thematic_break_rows(MAX_PREVIEW_ROWS + 1);
        assert!(too_many_rows.len() < MAX_PREVIEW_SOURCE_BYTES);
        assert!(parse_markdown(&too_many_rows).is_none());
    }

    #[test]
    fn parse_diff_returns_none_for_oversized_combined() {
        let big = "x".repeat(MAX_DIFF_PREVIEW_SOURCE_BYTES / 2 + 1);
        assert!(parse_markdown_diff(&big, &big).is_none());
    }

    #[test]
    fn parse_diff_returns_none_when_one_side_exceeds_rendered_row_limit() {
        let too_many_rows = thematic_break_rows(MAX_PREVIEW_ROWS + 1);
        assert!(too_many_rows.len() < MAX_DIFF_PREVIEW_SOURCE_BYTES);
        assert!(parse_markdown_diff(&too_many_rows, "# ok\n").is_none());
    }

    #[test]
    fn parse_diff_allows_single_side_over_single_preview_limit_within_combined_cap() {
        let old = "x".repeat(MAX_PREVIEW_SOURCE_BYTES + 1);
        let new = "y".repeat(MAX_DIFF_PREVIEW_SOURCE_BYTES - old.len());

        assert!(parse_markdown(&old).is_none());

        let (old_doc, new_doc) =
            parse_markdown_diff(&old, &new).expect("combined diff under 2 MiB should parse");
        assert_eq!(old_doc.rows.len(), 1);
        assert_eq!(new_doc.rows.len(), 1);
    }

    // ── Empty input ─────────────────────────────────────────────────────

    #[test]
    fn empty_source_produces_empty_document() {
        let doc = parse("");
        assert!(doc.rows.is_empty());
    }

    // ── Mixed document ──────────────────────────────────────────────────

    #[test]
    fn mixed_document_produces_correct_row_sequence() {
        let src = "\
# Title

A paragraph with **bold** text.

- item one
- item two

```
code line
```

---
";
        let doc = parse(src);

        // Should have: Heading, Spacer, Paragraph, ListItem, ListItem, CodeLine, ThematicBreak
        assert!(
            doc.rows.len() >= 7,
            "expected at least 7 rows, got {}",
            doc.rows.len()
        );
        assert!(matches!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        ));
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(doc.rows[2].kind, MarkdownPreviewRowKind::Paragraph);
    }

    // ── Internal helpers ────────────────────────────────────────────────

    #[test]
    fn build_line_starts_correct() {
        let src = "abc\ndef\nghi";
        let starts = build_line_starts(src);
        assert_eq!(starts, vec![0, 4, 8]);
    }

    #[test]
    fn byte_offset_to_line_maps_correctly() {
        let starts = vec![0, 4, 8];
        assert_eq!(byte_offset_to_line(0, &starts), 0);
        assert_eq!(byte_offset_to_line(3, &starts), 0);
        assert_eq!(byte_offset_to_line(4, &starts), 1);
        assert_eq!(byte_offset_to_line(7, &starts), 1);
        assert_eq!(byte_offset_to_line(8, &starts), 2);
    }

    #[test]
    fn normalize_whitespace_collapses_runs() {
        assert_eq!(normalize_whitespace("a  b\tc\n d"), "a b c d");
        assert_eq!(normalize_whitespace("  leading"), " leading");
        assert_eq!(normalize_whitespace(""), "");
    }

    #[test]
    fn unsupported_html_degrades_cleanly() {
        let doc = parse("<div>block html</div>\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::PlainFallback);
        assert_eq!(doc.rows[0].text.as_ref(), "<div>block html</div>");
    }

    #[test]
    fn inline_html_is_preserved_inside_paragraphs() {
        let doc = parse("Text with <b>html</b> inline\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[0].text.as_ref(), "Text with <b>html</b> inline");
    }

    #[test]
    fn html_comments_are_hidden_from_preview() {
        let doc = parse("Visible <!-- hidden --> text\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].text.as_ref(), "Visible text");
    }

    #[test]
    fn block_html_comments_do_not_create_rows() {
        let doc = parse("<!-- hidden -->\nVisible\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].text.as_ref(), "Visible");
    }

    #[test]
    fn custom_anchor_tags_are_hidden_from_preview() {
        let doc = parse("# Section Heading\n\n<a name=\"my-custom-anchor-point\"></a>\nVisible\n");
        assert_eq!(doc.rows.len(), 3);
        assert_eq!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        );
        assert_eq!(doc.rows[0].text.as_ref(), "Section Heading");
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(doc.rows[2].text.as_ref(), "Visible");
    }

    #[test]
    fn custom_anchor_id_tags_are_hidden_from_preview() {
        let doc = parse("# Section Heading\n\n<a id=\"jump-target\"></a>\nVisible\n");
        assert_eq!(doc.rows.len(), 3);
        assert_eq!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        );
        assert_eq!(doc.rows[0].text.as_ref(), "Section Heading");
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
        assert_eq!(doc.rows[2].text.as_ref(), "Visible");
    }

    #[test]
    fn markdown_images_preserve_alt_text() {
        let doc = parse("![Octocat smiling](https://example.com/octocat.svg)\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[0].text.as_ref(), "Octocat smiling");
    }

    #[test]
    fn html_img_tags_preserve_alt_text() {
        let doc =
            parse("<img alt=\"Octocat smiling\" src=\"https://example.com/octocat.svg\" />\n");
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[0].text.as_ref(), "Octocat smiling");
    }

    #[test]
    fn picture_elements_preserve_nested_img_alt_text() {
        let doc = parse(
            "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"dark.svg\" />\n  <img alt=\"Octocat smiling\" src=\"light.svg\" />\n</picture>\n",
        );
        assert_eq!(doc.rows.len(), 1);
        assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
        assert_eq!(doc.rows[0].text.as_ref(), "Octocat smiling");
    }

    // ── Modify-kind mask coverage ────────────────────────────────────────

    #[test]
    fn plan_changed_line_masks_handles_modify_kind() {
        use gitcomet_core::file_diff::{FileDiffPlan, FileDiffPlanRun};

        let plan = FileDiffPlan {
            runs: vec![FileDiffPlanRun::Modify {
                old_start: 0,
                new_start: 0,
                len: 1,
            }],
            row_count: 1,
            inline_row_count: 2,
            eof_newline: None,
        };

        let (old_mask, new_mask) = gitcomet_core::file_diff::plan_changed_line_masks(&plan, 2, 2);
        assert!(old_mask[0]); // modify marks old side
        assert!(!old_mask[1]);
        assert!(new_mask[0]); // modify marks new side
        assert!(!new_mask[1]);
    }

    // ── Identical content diff produces no change hints ──────────────────

    #[test]
    fn identical_content_diff_produces_no_change_hints() {
        let src = "# Title\n\nSame paragraph\n\n- item one\n";
        let preview = build_markdown_diff_preview(src, src).unwrap();

        for row in &preview.old.rows {
            assert_eq!(
                row.change_hint,
                MarkdownChangeHint::None,
                "old row {:?} should be unchanged",
                row.text
            );
        }
        for row in &preview.new.rows {
            assert_eq!(
                row.change_hint,
                MarkdownChangeHint::None,
                "new row {:?} should be unchanged",
                row.text
            );
        }
    }

    #[test]
    fn markdown_diff_scrollbar_markers_show_added_rows_for_one_sided_preview() {
        let preview = build_markdown_diff_preview("", "# New\n").unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Add,
            }]
        );
    }

    #[test]
    fn markdown_diff_scrollbar_markers_show_removed_rows_for_one_sided_preview() {
        let preview = build_markdown_diff_preview("# Gone\n", "").unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Remove,
            }]
        );
    }

    #[test]
    fn markdown_diff_scrollbar_markers_merge_replacements_into_modify_markers() {
        let preview = build_markdown_diff_preview("old\n", "new\n").unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Modify,
            }]
        );
    }

    #[test]
    fn markdown_diff_scrollbar_markers_split_disjoint_change_regions() {
        let preview = build_markdown_diff_preview(
            "- old one\n- keep two\n- keep three\n- keep four\n- old five\n",
            "- new one\n- keep two\n- keep three\n- keep four\n- new five\n",
        )
        .unwrap();

        assert_eq!(
            scrollbar_markers_for_diff_preview(&preview),
            vec![
                crate::view::components::ScrollbarMarker {
                    start: 0.0,
                    end: 0.2,
                    kind: crate::view::components::ScrollbarMarkerKind::Modify,
                },
                crate::view::components::ScrollbarMarker {
                    start: 0.8,
                    end: 1.0,
                    kind: crate::view::components::ScrollbarMarkerKind::Modify,
                },
            ]
        );
    }

    // ── Code span inside code block is not styled ────────────────────────

    #[test]
    fn code_block_lines_have_no_inline_spans() {
        let doc = parse("```\n**not bold** `not code`\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 1);
        assert!(
            code_rows[0].inline_spans.is_empty(),
            "inline spans inside code blocks should be empty"
        );
    }

    // ── Deeply nested list preserves indent levels ───────────────────────

    #[test]
    fn deeply_nested_lists_increment_indent() {
        let doc = parse("- a\n  - b\n    - c\n");
        assert!(doc.rows.len() >= 3);
        assert!(
            doc.rows[0].indent_level < doc.rows[1].indent_level,
            "second level should be more indented"
        );
        assert!(
            doc.rows[1].indent_level < doc.rows[2].indent_level,
            "third level should be more indented"
        );
    }

    // ── Edge case: line_range_change_hint with empty mask ────────────────

    #[test]
    fn line_range_change_hint_with_empty_mask_is_none() {
        assert_eq!(
            line_range_change_hint(&(0..3), &[], true),
            MarkdownChangeHint::None
        );
    }

    #[test]
    fn line_range_change_hint_with_empty_range_is_none() {
        assert_eq!(
            line_range_change_hint(&(2..2), &[true, true, true], true),
            MarkdownChangeHint::None
        );
    }

    // ── source_line_range helper ────────────────────────────────────────

    #[test]
    fn source_line_range_computes_correct_range() {
        let starts = build_line_starts("abc\ndef\nghi\n");
        // "abc\n" starts at 0 (line 0), "def\n" starts at 4 (line 1),
        // "ghi\n" starts at 8 (line 2)
        assert_eq!(source_line_range(0, 4, &starts), 0..1);
        assert_eq!(source_line_range(0, 8, &starts), 0..2);
        assert_eq!(source_line_range(4, 12, &starts), 1..3);
    }

    #[test]
    fn source_line_range_handles_empty_range() {
        let starts = build_line_starts("abc\n");
        assert_eq!(source_line_range(0, 0, &starts), 0..1);
    }

    // ── Error message helpers ───────────────────────────────────────────

    #[test]
    fn single_preview_unavailable_reason_reports_size_for_oversized() {
        let reason = single_preview_unavailable_reason(MAX_PREVIEW_SOURCE_BYTES + 1);
        assert!(
            reason.contains("1 MiB"),
            "should mention size limit: {reason}"
        );
    }

    #[test]
    fn single_preview_unavailable_reason_reports_rows_for_normal_size() {
        let reason = single_preview_unavailable_reason(100);
        assert!(
            reason.contains("row limit"),
            "should mention row limit: {reason}"
        );
    }

    #[test]
    fn diff_preview_unavailable_reason_reports_size_for_oversized() {
        let reason = diff_preview_unavailable_reason(MAX_DIFF_PREVIEW_SOURCE_BYTES + 1);
        assert!(
            reason.contains("2 MiB"),
            "should mention size limit: {reason}"
        );
    }

    #[test]
    fn diff_preview_unavailable_reason_reports_rows_for_normal_size() {
        let reason = diff_preview_unavailable_reason(100);
        assert!(
            reason.contains("row limit"),
            "should mention row limit: {reason}"
        );
    }
}
