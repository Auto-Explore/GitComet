use gpui::SharedString;
use std::ops::Range;
use std::sync::Arc;

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MarkdownPreviewRow {
    pub(super) kind: MarkdownPreviewRowKind,
    pub(super) text: SharedString,
    pub(super) inline_spans: Arc<Vec<MarkdownInlineSpan>>,
    pub(super) source_line_range: Range<usize>,
    pub(super) change_hint: MarkdownChangeHint,
    pub(super) indent_level: u8,
    pub(super) blockquote_level: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MarkdownPreviewRowKind {
    Heading { level: u8 },
    Paragraph,
    ListItem { number: Option<u64> },
    BlockquoteLine,
    CodeLine { is_first: bool, is_last: bool },
    ThematicBreak,
    TableRow,
    PlainFallback,
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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum MarkdownChangeHint {
    #[default]
    None,
    Added,
    Removed,
    Modified,
}

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
    let diff_rows = gitcomet_core::file_diff::side_by_side_rows(old_source, new_source);
    let (old_mask, new_mask) = build_changed_line_masks(
        &diff_rows,
        old_source.lines().count(),
        new_source.lines().count(),
    );
    annotate_change_hints(&mut old, &mut new, &old_mask, &new_mask);
    Some(MarkdownPreviewDiff { old, new })
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
        row.change_hint = line_range_change_hint(&row.source_line_range, changed_old_lines, true);
    }
    for row in &mut new_doc.rows {
        row.change_hint = line_range_change_hint(&row.source_line_range, changed_new_lines, false);
    }
}

/// Build changed-line boolean vectors from `FileDiffRow` data.
fn build_changed_line_masks(
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
    old_line_count: usize,
    new_line_count: usize,
) -> (Vec<bool>, Vec<bool>) {
    use gitcomet_core::file_diff::FileDiffRowKind;

    let mut old_mask = vec![false; old_line_count];
    let mut new_mask = vec![false; new_line_count];

    let mark = |mask: &mut [bool], line: Option<u32>| {
        if let Some(l) = line {
            let ix = l.saturating_sub(1) as usize;
            if ix < mask.len() {
                mask[ix] = true;
            }
        }
    };

    for row in diff_rows {
        match row.kind {
            FileDiffRowKind::Context => {}
            FileDiffRowKind::Remove => mark(&mut old_mask, row.old_line),
            FileDiffRowKind::Add => mark(&mut new_mask, row.new_line),
            FileDiffRowKind::Modify => {
                mark(&mut old_mask, row.old_line);
                mark(&mut new_mask, row.new_line);
            }
        }
    }

    (old_mask, new_mask)
}

// ── Internal helpers ────────────────────────────────────────────────────

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
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

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

    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

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
    let mut in_code_block = false;
    let mut code_block_start_byte: usize = 0;
    let mut code_block_starts_after_fence = false;

    for (event, event_range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_heading = true;
            }
            Event::End(TagEnd::Heading(level)) => {
                push_row(
                    &mut rows,
                    MarkdownPreviewRowKind::Heading { level: level as u8 },
                    &text_buf,
                    &inline_spans,
                    source_line_range(source_start_byte, event_range.end, line_starts),
                    indent_level,
                    in_blockquote,
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
                let kind = if let Some(kind) = list_item_stack.last().copied() {
                    kind
                } else if in_blockquote > 0 {
                    MarkdownPreviewRowKind::BlockquoteLine
                } else {
                    MarkdownPreviewRowKind::Paragraph
                };

                push_row(
                    &mut rows,
                    kind,
                    &text_buf,
                    &inline_spans,
                    source_line_range(source_start_byte, event_range.end, line_starts),
                    indent_level,
                    in_blockquote,
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
                    push_row(
                        &mut rows,
                        kind,
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.start, line_starts),
                        indent_level,
                        in_blockquote,
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
                    push_row(
                        &mut rows,
                        kind,
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                }
                list_item_stack.pop();
            }

            Event::Start(Tag::BlockQuote(_)) => {
                in_blockquote = in_blockquote.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                in_blockquote = in_blockquote.saturating_sub(1);
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_block_start_byte = event_range.start;
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
                let last_ix = code_lines.len().saturating_sub(1);
                for (i, line) in code_lines.iter().enumerate() {
                    let line_ix = (content_start_line + i).min(block_end_line);
                    push_row(
                        &mut rows,
                        MarkdownPreviewRowKind::CodeLine {
                            is_first: i == 0,
                            is_last: i == last_ix,
                        },
                        line,
                        &[],
                        line_ix..line_ix + 1,
                        indent_level,
                        in_blockquote,
                    )?;
                }
                in_code_block = false;
                code_block_starts_after_fence = false;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
            }
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                push_row(
                    &mut rows,
                    MarkdownPreviewRowKind::TableRow,
                    &text_buf,
                    &inline_spans,
                    source_line_range(source_start_byte, event_range.end, line_starts),
                    indent_level,
                    in_blockquote,
                )?;
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

            Event::SoftBreak | Event::HardBreak => {
                if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                    if !text_buf.is_empty() {
                        push_row(
                            &mut rows,
                            MarkdownPreviewRowKind::BlockquoteLine,
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.start, line_starts),
                            indent_level,
                            in_blockquote,
                        )?;
                        text_buf.clear();
                        inline_spans.clear();
                    }
                    source_start_byte = event_range.end;
                } else {
                    text_buf.push(' ');
                }
            }

            Event::Rule => {
                push_row(
                    &mut rows,
                    MarkdownPreviewRowKind::ThematicBreak,
                    "───",
                    &[],
                    source_line_range(event_range.start, event_range.end, line_starts),
                    indent_level,
                    in_blockquote,
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
                let should_append = in_paragraph
                    || in_heading
                    || !list_stack.is_empty()
                    || in_blockquote > 0
                    || in_code_block;
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

    Some(rows)
}

fn push_row(
    rows: &mut Vec<MarkdownPreviewRow>,
    kind: MarkdownPreviewRowKind,
    text: &str,
    inline_spans: &[MarkdownInlineSpan],
    source_line_range: Range<usize>,
    indent_level: u8,
    blockquote_level: u8,
) -> Option<()> {
    let (row_text, row_spans) = match kind {
        // Paragraph-like rows collapse whitespace, so remap inline spans to
        // the normalized text instead of leaving them pointed at stale bytes.
        MarkdownPreviewRowKind::Paragraph | MarkdownPreviewRowKind::BlockquoteLine => {
            normalize_whitespace_with_spans(text, inline_spans)
        }
        _ => (text.to_owned(), inline_spans.to_vec()),
    };
    let spans = if row_spans.len() > MAX_INLINE_SPANS_PER_ROW {
        Arc::new(Vec::new())
    } else {
        Arc::new(row_spans)
    };

    rows.push(MarkdownPreviewRow {
        kind,
        text: SharedString::from(row_text),
        inline_spans: spans,
        source_line_range,
        change_hint: MarkdownChangeHint::None,
        indent_level,
        blockquote_level,
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
            MarkdownPreviewRowKind::PlainFallback,
            segment,
            &[],
            line_ix..line_ix.saturating_add(1),
            indent_level,
            blockquote_level,
        )?;
    }

    Some(())
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

    for &s in stack {
        match s {
            MarkdownInlineStyle::Bold => has_bold = true,
            MarkdownInlineStyle::Italic => has_italic = true,
            MarkdownInlineStyle::Strikethrough => has_strikethrough = true,
            MarkdownInlineStyle::Link => has_link = true,
            MarkdownInlineStyle::Code => has_code = true,
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

    // ── Code block tests ────────────────────────────────────────────────

    #[test]
    fn fenced_code_block_one_row_per_line() {
        let doc = parse("```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n");
        let code_rows = code_rows(&doc);
        assert_eq!(code_rows.len(), 3);
        assert_eq!(code_rows[0].text.as_ref(), "fn main() {");
        assert_eq!(code_rows[1].text.as_ref(), "    println!(\"hi\");");
        assert_eq!(code_rows[2].text.as_ref(), "}");
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

    // ── Table ───────────────────────────────────────────────────────────

    #[test]
    fn table_rows_are_flattened() {
        let doc = parse("| A | B |\n|---|---|\n| 1 | 2 |\n");
        let table_rows: Vec<_> = doc
            .rows
            .iter()
            .filter(|r| r.kind == MarkdownPreviewRowKind::TableRow)
            .collect();
        assert!(table_rows.len() >= 2);
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
    fn bold_italic_produces_bold_italic_span() {
        let doc = parse("***both***\n");
        assert_eq!(
            spans_with_style(&doc.rows[0], MarkdownInlineStyle::BoldItalic).len(),
            1
        );
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

    // ── build_changed_line_masks ─────────────────────────────────────────

    #[test]
    fn build_changed_line_masks_from_diff_rows() {
        use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

        let diff_rows = vec![
            FileDiffRow {
                kind: FileDiffRowKind::Context,
                old_line: Some(1),
                new_line: Some(1),
                old: Some("same".into()),
                new: Some("same".into()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: FileDiffRowKind::Remove,
                old_line: Some(2),
                new_line: None,
                old: Some("old".into()),
                new: None,
                eof_newline: None,
            },
            FileDiffRow {
                kind: FileDiffRowKind::Add,
                old_line: None,
                new_line: Some(2),
                old: None,
                new: Some("new".into()),
                eof_newline: None,
            },
        ];

        let (old_mask, new_mask) = build_changed_line_masks(&diff_rows, 3, 3);
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

        // Should have: Heading, Paragraph, ListItem, ListItem, CodeLine, ThematicBreak
        assert!(
            doc.rows.len() >= 6,
            "expected at least 6 rows, got {}",
            doc.rows.len()
        );
        assert!(matches!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::Heading { level: 1 }
        ));
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
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

    // ── Modify-kind mask coverage ────────────────────────────────────────

    #[test]
    fn build_changed_line_masks_handles_modify_kind() {
        use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

        let diff_rows = vec![FileDiffRow {
            kind: FileDiffRowKind::Modify,
            old_line: Some(1),
            new_line: Some(1),
            old: Some("before".into()),
            new: Some("after".into()),
            eof_newline: None,
        }];

        let (old_mask, new_mask) = build_changed_line_masks(&diff_rows, 2, 2);
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
