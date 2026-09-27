//! How wide a tab is in the read-only text views, and the one expansion every
//! view uses: a tab advances to the next multiple of the width, counting
//! characters from the start of the line.
//!
//! Display offsets ("tab-expanded space") are byte offsets into the expanded
//! line, so every conversion walks from the line start.

use std::borrow::Cow;

pub(in crate::view) const DEFAULT_TAB_WIDTH: u8 = 4;
pub(in crate::view) const MAX_TAB_WIDTH: u8 = 16;

// One width for the app, set by the main pane for the file it shows. Tests run
// views on their own threads (and inline, without background workers), so
// they keep it per thread and cannot leak a width into each other.
#[cfg(not(test))]
static TAB_WIDTH: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(DEFAULT_TAB_WIDTH);
#[cfg(test)]
thread_local! {
    static TAB_WIDTH: std::cell::Cell<u8> = const { std::cell::Cell::new(DEFAULT_TAB_WIDTH) };
}

/// Columns a tab advances to.
pub(in crate::view) fn tab_width() -> usize {
    #[cfg(not(test))]
    let width = TAB_WIDTH.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    let width = TAB_WIDTH.with(std::cell::Cell::get);
    usize::from(width.clamp(1, MAX_TAB_WIDTH))
}

/// Set the width; `true` when it changed, so callers drop what they
/// rendered with the old one.
pub(in crate::view) fn set_tab_width(width: u8) -> bool {
    let width = width.clamp(1, MAX_TAB_WIDTH);
    #[cfg(not(test))]
    let previous = TAB_WIDTH.swap(width, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    let previous = TAB_WIDTH.with(|cell| cell.replace(width));
    previous != width
}

/// Spaces the tab at `column` takes.
#[inline]
fn tab_span(column: usize, width: usize) -> usize {
    width - column % width
}

/// Columns (and expanded bytes) `ch` takes at `column`.
#[inline]
pub(in crate::view) fn char_columns(ch: char, column: usize) -> usize {
    if ch == '\t' {
        tab_span(column, tab_width())
    } else {
        1
    }
}

/// Expanded bytes `ch` takes at `column`.
#[inline]
pub(in crate::view) fn char_expanded_len(ch: char, column: usize) -> usize {
    if ch == '\t' {
        tab_span(column, tab_width())
    } else {
        ch.len_utf8()
    }
}

/// Append `text` (a line, or the rest of one starting at `*column`) with its
/// tabs expanded, advancing `column`.
pub(in crate::view) fn push_expanded(out: &mut String, text: &str, column: &mut usize) {
    let width = tab_width();
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
pub(in crate::view) fn expand_tabs(line: &str) -> Cow<'_, str> {
    if !line.contains('\t') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len() + line.len() / 4 + tab_width());
    push_expanded(&mut out, line, &mut 0);
    Cow::Owned(out)
}

/// Byte length of `line` once expanded.
pub(in crate::view) fn expanded_len(line: &str) -> usize {
    if !line.contains('\t') {
        return line.len();
    }
    let width = tab_width();
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
pub(in crate::view) fn display_offset_for_raw_offset(line: &str, raw: usize) -> usize {
    if !line.contains('\t') {
        return raw.min(line.len());
    }
    let width = tab_width();
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
pub(in crate::view) fn raw_offset_for_display_offset(line: &str, display: usize) -> usize {
    if !line.contains('\t') {
        return display.min(line.len());
    }
    let width = tab_width();
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

    struct Width(u8);
    impl Width {
        fn set(width: u8) -> Self {
            let previous = tab_width() as u8;
            set_tab_width(width);
            Self(previous)
        }
    }
    impl Drop for Width {
        fn drop(&mut self) {
            set_tab_width(self.0);
        }
    }

    #[test]
    fn tabs_align_to_stops() {
        let _width = Width::set(4);
        assert_eq!(expand_tabs("\tx"), "    x");
        assert_eq!(expand_tabs("ab\tx"), "ab  x");
        assert_eq!(expand_tabs("abcd\tx"), "abcd    x");
        assert_eq!(expand_tabs("é\tx"), "é   x");
        assert_eq!(expanded_len("ab\tx"), 5);
        assert_eq!(expanded_len("é\tx"), "é   x".len());
    }

    #[test]
    fn width_is_configurable() {
        let _width = Width::set(8);
        assert_eq!(expand_tabs("a\tb"), "a       b");
        let _narrow = Width::set(2);
        assert_eq!(expand_tabs("a\tb\tc"), "a b c");
    }

    #[test]
    fn continuing_a_line_keeps_the_column() {
        let _width = Width::set(4);
        let mut out = String::new();
        let mut column = 0;
        push_expanded(&mut out, "ab", &mut column);
        push_expanded(&mut out, "\tx", &mut column);
        assert_eq!(out, expand_tabs("ab\tx"));
        assert_eq!(column, 5);
    }

    #[test]
    fn offsets_round_trip_and_tab_interiors_resolve_to_the_tab() {
        let _width = Width::set(4);
        let line = "a\tbé\tc";
        let expanded = expand_tabs(line);
        for (raw, _) in line.char_indices() {
            let display = display_offset_for_raw_offset(line, raw);
            assert_eq!(
                raw_offset_for_display_offset(line, display),
                raw,
                "raw {raw}"
            );
        }
        // "a" + 3 spaces: offsets 1..4 are inside the first tab.
        for display in 1..4 {
            assert_eq!(raw_offset_for_display_offset(line, display), 1);
        }
        assert_eq!(
            display_offset_for_raw_offset(line, line.len()),
            expanded.len()
        );
        assert_eq!(
            raw_offset_for_display_offset(line, expanded.len()),
            line.len()
        );
    }
}
