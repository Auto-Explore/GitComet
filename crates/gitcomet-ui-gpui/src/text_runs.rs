//! Shared guards for turning highlight ranges into `gpui` text runs.
//!
//! `gpui` shapes a line by walking the run list and splitting the text at each
//! run length, so every run boundary has to land on a UTF-8 character boundary
//! and the runs have to tile the text in order. A single range that violates
//! that — a syntax token clipped mid-codepoint, an inline span left pointing at
//! a stale offset, a highlight that outlived the text it was computed for —
//! aborts the process inside `str::split_at` rather than merely rendering the
//! wrong colour. Highlight ranges reach the renderer from many producers
//! (tree-sitter and heuristic tokenizers, word diff, search matches, markdown
//! inline spans, truncation projections), so the boundary is enforced here,
//! once, on the way into shaping.

use gpui::{TextRun, TextStyle};
use std::ops::Range;

/// Clamp `range` to `text` and widen it onto the surrounding character
/// boundaries. Returns `None` when nothing is left to highlight.
pub(crate) fn snap_highlight_range(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    if start >= end {
        return None;
    }

    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }

    (start < end).then_some(start..end)
}

/// True when `highlights` already satisfies the run-builder contract: in
/// bounds, on character boundaries, sorted, and non-overlapping.
pub(crate) fn highlights_are_shapeable<S>(text: &str, highlights: &[(Range<usize>, S)]) -> bool {
    let mut prev_end = 0usize;
    for (range, _) in highlights {
        if range.start < prev_end
            || range.start >= range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return false;
        }
        prev_end = range.end;
    }
    true
}

/// Repair `highlights` in place so it satisfies the run-builder contract.
///
/// The common case is already valid, and then this is a single non-allocating
/// scan. Otherwise ranges are clamped, snapped outward to character
/// boundaries, sorted, and trimmed so they no longer overlap; ranges left
/// empty by that are dropped.
pub(crate) fn sanitize_highlights<S: Copy>(text: &str, highlights: &mut Vec<(Range<usize>, S)>) {
    if highlights.is_empty() || highlights_are_shapeable(text, highlights) {
        return;
    }

    let mut repaired: Vec<(Range<usize>, S)> = Vec::with_capacity(highlights.len());
    for (range, style) in highlights.iter() {
        if let Some(range) = snap_highlight_range(text, range) {
            repaired.push((range, *style));
        }
    }
    repaired.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    highlights.clear();
    let mut prev_end = 0usize;
    for (range, style) in repaired {
        let start = range.start.max(prev_end);
        if start >= range.end {
            continue;
        }
        prev_end = range.end;
        highlights.push((start..range.end, style));
    }
}

/// Build the run list for `text`, tiling it with `default_style` outside the
/// highlighted ranges.
///
/// Ranges that would break the tiling — out of bounds, overlapping a previous
/// run, or off a character boundary — are skipped or trimmed rather than
/// emitted, so the returned runs always sum to `text.len()`.
pub(crate) fn text_runs_for_highlights(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> Vec<TextRun> {
    if highlights.is_empty() {
        return vec![default_style.to_run(text.len())];
    }

    // The walk below advances a cursor, so an out-of-order range would be
    // skipped outright rather than repaired. Callers reach this directly, so
    // sort here for the same result `sanitize_highlights` produces.
    let sorted;
    let highlights = if highlights_are_shapeable(text, highlights) {
        highlights
    } else {
        let mut repaired = highlights.to_vec();
        sanitize_highlights(text, &mut repaired);
        sorted = repaired;
        &sorted
    };

    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        let Some(range) = snap_highlight_range(text, range) else {
            continue;
        };
        let start = range.start.max(ix);
        if start >= range.end {
            continue;
        }
        if ix < start {
            runs.push(default_style.clone().to_run(start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.end - start),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::HighlightStyle;

    fn style() -> HighlightStyle {
        HighlightStyle {
            font_weight: Some(gpui::FontWeight::BOLD),
            ..HighlightStyle::default()
        }
    }

    #[test]
    fn snap_widens_ranges_that_split_a_multibyte_char() {
        let text = "—dash";
        assert_eq!(snap_highlight_range(text, &(0..1)), Some(0..3));
        assert_eq!(snap_highlight_range(text, &(1..2)), Some(0..3));
        assert_eq!(snap_highlight_range(text, &(2..5)), Some(0..5));
        assert_eq!(snap_highlight_range(text, &(0..0)), None);
        assert_eq!(snap_highlight_range(text, &(99..120)), None);
    }

    #[test]
    fn sanitize_leaves_valid_highlights_untouched() {
        let text = "— bold —";
        let mut highlights = vec![(0..3, style()), (4..8, style())];
        let expected = highlights.clone();
        sanitize_highlights(text, &mut highlights);
        assert_eq!(highlights, expected);
    }

    #[test]
    fn sanitize_repairs_split_overlapping_and_unsorted_highlights() {
        let text = "—abc—";
        let mut highlights = vec![(4..99, style()), (1..2, style()), (2..5, style())];
        sanitize_highlights(text, &mut highlights);
        assert_eq!(
            highlights,
            vec![(0..3, style()), (3..5, style()), (5..9, style())]
        );
        for (range, _) in &highlights {
            assert!(text.is_char_boundary(range.start));
            assert!(text.is_char_boundary(range.end));
        }
    }

    #[test]
    fn runs_tile_the_text_even_for_broken_ranges() {
        let text = "—abc—";
        let default_style = TextStyle::default();
        for highlights in [
            vec![(0..1, style())],
            vec![(1..2, style())],
            vec![(4..99, style()), (1..2, style())],
            vec![(0..3, style()), (0..9, style())],
            vec![],
        ] {
            let runs = text_runs_for_highlights(text, &default_style, &highlights);
            let total: usize = runs.iter().map(|run| run.len).sum();
            assert_eq!(
                total,
                text.len(),
                "runs must tile {text:?} for {highlights:?}"
            );

            let mut rest = text;
            for run in &runs {
                assert!(
                    rest.is_char_boundary(run.len),
                    "run boundary splits a char in {text:?} for {highlights:?}"
                );
                rest = &rest[run.len..];
            }
        }
    }
}
