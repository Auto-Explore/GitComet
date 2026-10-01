//! Line-start indexes and the line/byte range arithmetic built on them.

use super::*;

pub(in crate::view::panes::main) fn count_newlines(text: &str) -> usize {
    text.as_bytes().iter().filter(|&&b| b == b'\n').count()
}

pub(in crate::view::panes::main) fn build_line_starts(text: &str) -> Vec<usize> {
    build_line_starts_with_count(text).0
}

pub(in crate::view::panes::main) fn build_line_starts_with_count(
    text: &str,
) -> (Vec<usize>, usize) {
    let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
    line_starts.push(0usize);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(ix.saturating_add(1));
        }
    }
    let line_count = if text.is_empty() {
        0
    } else {
        line_starts.len()
    };
    (line_starts, line_count)
}

#[cfg(test)]
pub(in crate::view::panes::main) fn preview_source_text_from_lines(
    lines: &[String],
    source_len: usize,
) -> SharedString {
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
    if lines.is_empty() {
        debug_assert_eq!(
            source_len, 0,
            "empty preview lines should only produce empty source text",
        );
        return (SharedString::default(), Arc::default());
    }

    let mut text = String::with_capacity(source_len);
    let mut line_starts = Vec::with_capacity(lines.len().saturating_add(1));
    line_starts.push(0);
    for (ix, line) in lines.iter().enumerate() {
        text.push_str(line);
        let has_more_lines = ix + 1 < lines.len();
        let needs_trailing_newline = !has_more_lines && text.len() < source_len;
        if has_more_lines || needs_trailing_newline {
            text.push('\n');
            line_starts.push(text.len());
        }
    }
    debug_assert_eq!(
        text.len(),
        source_len,
        "preview lines/source length should only differ by an optional trailing newline",
    );
    (text.into(), Arc::from(line_starts))
}

const PREVIEW_LINE_FLAG_ASCII_ONLY: u8 = 0b01;
const PREVIEW_LINE_FLAG_HAS_TABS: u8 = 0b10;

#[inline]
pub(in crate::view) fn preview_line_flags_for_text(text: &str) -> u8 {
    preview_line_flags_from_bools(text.is_ascii(), text.contains('\t'))
}

#[inline]
pub(in crate::view) fn preview_line_flags_from_bools(ascii_only: bool, has_tabs: bool) -> u8 {
    let mut flags = 0u8;
    if ascii_only {
        flags |= PREVIEW_LINE_FLAG_ASCII_ONLY;
    }
    if has_tabs {
        flags |= PREVIEW_LINE_FLAG_HAS_TABS;
    }
    flags
}

#[inline]
pub(in crate::view) fn preview_line_is_ascii_without_loading(flags: u8) -> bool {
    (flags & PREVIEW_LINE_FLAG_ASCII_ONLY) != 0
}

#[inline]
pub(in crate::view) fn preview_line_has_tabs_without_loading(flags: u8) -> bool {
    (flags & PREVIEW_LINE_FLAG_HAS_TABS) != 0
}

pub(in crate::view) fn preview_line_flags_from_source(
    text: &str,
    line_starts: &[usize],
) -> Arc<[u8]> {
    let line_count = indexed_line_count_from_len(text.len(), line_starts);
    let mut flags = Vec::with_capacity(line_count);
    for line_ix in 0..line_count {
        let range = indexed_line_byte_range(line_starts, text.len(), line_ix)
            .unwrap_or(text.len()..text.len());
        flags.push(preview_line_flags_for_text(
            text.get(range).unwrap_or_default(),
        ));
    }
    Arc::from(flags)
}

pub(in crate::view::panes::main) fn line_start_offset_for_index(
    line_starts: &[usize],
    text_len: usize,
    line_ix: usize,
) -> usize {
    line_starts.get(line_ix).copied().unwrap_or(text_len)
}

pub(in crate::view::panes::main) fn source_line_count(text: &str) -> usize {
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
pub(in crate::view) fn indexed_line_count_from_len(
    source_len: usize,
    line_starts: &[usize],
) -> usize {
    gitcomet_core::text_utils::line_count_from_starts(source_len, line_starts)
}

pub(in crate::view::panes::main) fn indexed_line_count(text: &str, line_starts: &[usize]) -> usize {
    indexed_line_count_from_len(text.len(), line_starts)
}

pub(in crate::view) fn indexed_line_byte_range(
    line_starts: &[usize],
    source_len: usize,
    line_ix: usize,
) -> Option<Range<usize>> {
    gitcomet_core::text_utils::line_byte_range(line_starts, source_len, line_ix)
}

/// Number of logical rows produced by `split('\n')` (always at least 1).
pub(in crate::view::panes::main) fn split_line_count(text: &str) -> usize {
    count_newlines(text).saturating_add(1)
}

/// Byte range of line content at `line_ix` (without trailing newline).
///
/// Uses `split('\n')` row semantics, so trailing newline creates a final empty row.
pub(in crate::view::panes::main) fn line_content_byte_range_for_index(
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
pub(in crate::view::panes::main) fn append_line_insertion_text(
    existing: &str,
    line: &str,
) -> String {
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

fn line_index_for_byte_offset(line_starts: &[usize], byte_offset: usize) -> usize {
    if line_starts.is_empty() {
        return 0;
    }
    line_starts
        .partition_point(|&start| start <= byte_offset)
        .saturating_sub(1)
}

pub(in crate::view::panes::main) fn dirty_byte_range_to_line_range(
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

pub(in crate::view::panes::main) fn shifted_line_index(ix: usize, delta: isize) -> usize {
    if delta >= 0 {
        ix.saturating_add(delta as usize)
    } else {
        ix.saturating_sub((-delta) as usize)
    }
}

pub(in crate::view::panes::main) fn line_index_for_offset(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count()
}

pub(in crate::view::panes::main) fn slice_text_by_line_range(
    text: &str,
    line_range: Range<usize>,
) -> String {
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
        let (line_starts, line_count) = build_line_starts_with_count(text);

        assert_eq!(line_count, 2);
        assert_eq!(indexed_line_count(text, &line_starts), 2);
    }

    #[test]
    fn indexed_line_count_preserves_trailing_empty_row() {
        let text = "alpha\nbeta\n";
        let (line_starts, line_count) = build_line_starts_with_count(text);

        assert_eq!(line_count, 3);
        assert_eq!(line_starts, vec![0, 6, 11]);
        assert_eq!(indexed_line_count(text, &line_starts), 3);
    }
}
