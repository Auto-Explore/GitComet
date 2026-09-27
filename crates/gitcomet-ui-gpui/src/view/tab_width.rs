//! How wide a tab is in the read-only text views, and the one expansion every
//! view uses: a tab advances to the next multiple of the width, counting
//! characters from the start of the line.
//!
//! Display offsets ("tab-expanded space") are byte offsets into the expanded
//! line, so every conversion walks from the line start.

use std::borrow::Cow;

pub(in crate::view) const DEFAULT_TAB_WIDTH: u8 = 4;
pub(in crate::view) const MAX_TAB_WIDTH: u8 = 16;

/// Spaces the tab at `column` takes.
#[inline]
fn tab_span(column: usize, width: usize) -> usize {
    width - column % width
}

/// Columns (and expanded bytes) `ch` takes at `column`.
#[inline]
pub(in crate::view) fn char_columns(width: usize, ch: char, column: usize) -> usize {
    if ch == '\t' {
        tab_span(column, width)
    } else {
        1
    }
}

/// Expanded bytes `ch` takes at `column`.
#[inline]
pub(in crate::view) fn char_expanded_len(width: usize, ch: char, column: usize) -> usize {
    if ch == '\t' {
        tab_span(column, width)
    } else {
        ch.len_utf8()
    }
}

/// Append `text` (a line, or the rest of one starting at `*column`) with its
/// tabs expanded, advancing `column`.
pub(in crate::view) fn push_expanded(
    width: usize,
    out: &mut String,
    text: &str,
    column: &mut usize,
) {
    let mut rest = text;
    while let Some(tab) = rest.find('\t') {
        let before = &rest[..tab];
        out.push_str(before);
        *column += column_count(before);
        let span = tab_span(*column, width);
        out.extend(std::iter::repeat_n(' ', span));
        *column += span;
        rest = &rest[tab + 1..];
    }
    out.push_str(rest);
    *column += column_count(rest);
}

#[inline]
fn column_count(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.chars().count()
    }
}

/// A line with its tabs expanded.
pub(in crate::view) fn expand_tabs(width: usize, line: &str) -> Cow<'_, str> {
    if !line.contains('\t') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len() + line.len() / 4 + width);
    push_expanded(width, &mut out, line, &mut 0);
    Cow::Owned(out)
}

/// Inline patches paint the sign separately; tab stops begin at the content.
/// Keep the sign in searchable/copyable text without counting it as a column.
pub(in crate::view) fn expand_patch_tabs(width: usize, line: &str) -> Cow<'_, str> {
    if !line.contains('\t') {
        return Cow::Borrowed(line);
    }
    if matches!(line.as_bytes().first(), Some(b'+' | b'-' | b' ')) {
        let mut out = String::with_capacity(line.len() + width);
        out.push_str(&line[..1]);
        push_expanded(width, &mut out, &line[1..], &mut 0);
        Cow::Owned(out)
    } else {
        expand_tabs(width, line)
    }
}

/// Length corresponding to `expand_patch_tabs`, without allocating text.
pub(in crate::view) fn expanded_patch_len(width: usize, line: &str) -> usize {
    if matches!(line.as_bytes().first(), Some(b'+' | b'-' | b' ')) {
        1 + expanded_len(width, &line[1..])
    } else {
        expanded_len(width, line)
    }
}

/// Byte length of `line` once expanded.
pub(in crate::view) fn expanded_len(width: usize, line: &str) -> usize {
    if !line.contains('\t') {
        return line.len();
    }
    let mut len = 0usize;
    let mut column = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let span = tab_span(column, width);
            len += span;
            column += span;
        } else {
            len += ch.len_utf8();
            column += 1;
        }
    }
    len
}

/// The expanded offset of raw byte `raw` in `line`. A tab maps to the start
/// of its spaces.
pub(in crate::view) fn display_offset_for_raw_offset(
    width: usize,
    line: &str,
    raw: usize,
) -> usize {
    if !line.contains('\t') {
        return raw.min(line.len());
    }
    let mut display = 0usize;
    let mut column = 0usize;
    for (ix, ch) in line.char_indices() {
        if ix >= raw {
            return display;
        }
        if ch == '\t' {
            let span = tab_span(column, width);
            display += span;
            column += span;
        } else {
            display += ch.len_utf8();
            column += 1;
        }
    }
    display
}

/// The raw byte in `line` whose display span contains `display`; an offset
/// inside a tab's spaces resolves to that tab.
pub(in crate::view) fn raw_offset_for_display_offset(
    width: usize,
    line: &str,
    display: usize,
) -> usize {
    if !line.contains('\t') {
        return display.min(line.len());
    }
    let mut shown = 0usize;
    let mut column = 0usize;
    for (ix, ch) in line.char_indices() {
        let span = if ch == '\t' {
            let span = tab_span(column, width);
            column += span;
            span
        } else {
            column += 1;
            ch.len_utf8()
        };
        if display < shown + span {
            return ix;
        }
        shown += span;
    }
    line.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_align_to_stops() {
        let width = 4;
        assert_eq!(expand_tabs(width, "\tx"), "    x");
        assert_eq!(expand_tabs(width, "ab\tx"), "ab  x");
        assert_eq!(expand_tabs(width, "abcd\tx"), "abcd    x");
        assert_eq!(expand_tabs(width, "é\tx"), "é   x");
        assert_eq!(expanded_len(width, "ab\tx"), 5);
        assert_eq!(expanded_len(width, "é\tx"), "é   x".len());
    }

    #[test]
    fn width_is_configurable() {
        let width = 8;
        assert_eq!(expand_tabs(width, "a\tb"), "a       b");
        let width = 2;
        assert_eq!(expand_tabs(width, "a\tb\tc"), "a b c");
    }

    #[test]
    fn continuing_a_line_keeps_the_column() {
        let width = 4;
        let mut out = String::new();
        let mut column = 0;
        push_expanded(width, &mut out, "ab", &mut column);
        push_expanded(width, &mut out, "\tx", &mut column);
        assert_eq!(out, expand_tabs(width, "ab\tx"));
        assert_eq!(column, 5);
    }

    #[test]
    fn offsets_round_trip_and_tab_interiors_resolve_to_the_tab() {
        let width = 4;
        let line = "a\tbé\tc";
        let expanded = expand_tabs(width, line);
        for (raw, _) in line.char_indices() {
            let display = display_offset_for_raw_offset(width, line, raw);
            assert_eq!(
                raw_offset_for_display_offset(width, line, display),
                raw,
                "raw {raw}"
            );
        }
        // "a" + 3 spaces: offsets 1..4 are inside the first tab.
        for display in 1..4 {
            assert_eq!(raw_offset_for_display_offset(width, line, display), 1);
        }
        assert_eq!(
            display_offset_for_raw_offset(width, line, line.len()),
            expanded.len()
        );
        assert_eq!(
            raw_offset_for_display_offset(width, line, expanded.len()),
            line.len()
        );
    }
}
