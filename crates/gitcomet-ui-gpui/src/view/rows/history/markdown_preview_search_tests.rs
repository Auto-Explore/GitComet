use super::MarkdownPreviewRevealAlign::{Center, Top};
use super::{
    MarkdownPreviewQuery, markdown_preview_reveal_offset_y, markdown_preview_row_extent,
    markdown_preview_styled_row_with_query,
};
use crate::view::AppTheme;
use crate::view::markdown_preview::{
    MarkdownChangeHint, MarkdownInlineSpan, MarkdownInlineStyle, MarkdownPreviewRow,
    MarkdownPreviewRowKind,
};
use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};
use gpui::{Bounds, point, px, size};
use std::sync::Arc;

fn row(text: &str, spans: Vec<MarkdownInlineSpan>) -> MarkdownPreviewRow {
    MarkdownPreviewRow {
        kind: MarkdownPreviewRowKind::Paragraph,
        text: text.to_string().into(),
        inline_spans: Arc::new(spans),
        code_language: None,
        source_line_range: 0..1,
        change_hint: MarkdownChangeHint::None,
        indent_level: 0,
        blockquote_level: 0,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        image: None,
        inline_images: Arc::from(Vec::new()),
        styled_text_cache: Default::default(),
        table: None,
        task: None,
        continues_item: false,
    }
}

fn query(needle: &str, current_row: Option<usize>) -> MarkdownPreviewQuery {
    MarkdownPreviewQuery {
        matcher: Arc::new(DiffSearchMatcher::new(needle, DiffSearchOptions::default())),
        current_row,
    }
}

#[test]
fn reveal_centres_the_row_and_clamps_to_the_scrollable_range() {
    // Far down a long document: centre it in the viewport.
    assert_eq!(
        markdown_preview_reveal_offset_y(
            Center,
            px(1000.0),
            px(20.0),
            px(400.0),
            px(2000.0),
            px(0.0)
        ),
        Some(px(-810.0))
    );
    // A row near the top cannot be centred; the document stops at its top.
    assert_eq!(
        markdown_preview_reveal_offset_y(
            Center,
            px(10.0),
            px(20.0),
            px(400.0),
            px(2000.0),
            px(-50.0)
        ),
        Some(px(0.0))
    );
    // Past the end of the scrollable range, clamp to the bottom.
    assert_eq!(
        markdown_preview_reveal_offset_y(
            Center,
            px(5000.0),
            px(20.0),
            px(400.0),
            px(600.0),
            px(0.0)
        ),
        Some(px(-600.0))
    );
    // Already there: no scroll, so nothing repaints.
    assert_eq!(
        markdown_preview_reveal_offset_y(
            Center,
            px(1000.0),
            px(20.0),
            px(400.0),
            px(2000.0),
            px(-810.0)
        ),
        None
    );
    // An unmeasured container has no centre to compute.
    assert_eq!(
        markdown_preview_reveal_offset_y(
            Center,
            px(1000.0),
            px(20.0),
            px(0.0),
            px(2000.0),
            px(0.0)
        ),
        None
    );
}

#[test]
fn a_top_reveal_puts_the_row_at_the_top_of_the_viewport() {
    assert_eq!(
        markdown_preview_reveal_offset_y(Top, px(1000.0), px(20.0), px(400.0), px(2000.0), px(0.0)),
        Some(px(-1000.0))
    );
    // Near the end the document cannot scroll that far.
    assert_eq!(
        markdown_preview_reveal_offset_y(Top, px(1900.0), px(20.0), px(400.0), px(1600.0), px(0.0)),
        Some(px(-1600.0))
    );
}

#[test]
fn row_extent_spans_every_part_of_the_row() {
    let marker = Bounds {
        origin: point(px(0.0), px(120.0)),
        size: size(px(10.0), px(16.0)),
    };
    let text = Bounds {
        origin: point(px(12.0), px(118.0)),
        size: size(px(200.0), px(40.0)),
    };
    assert_eq!(
        markdown_preview_row_extent(&[marker, text]),
        Some((px(118.0), px(40.0)))
    );
    assert_eq!(markdown_preview_row_extent(&[]), None);
}

/// The wash is layered on the rendered text, so a query matches what the
/// reader sees — not the markdown that produced it.
#[test]
fn the_search_wash_covers_rendered_text_and_leaves_unmatched_rows_untouched() {
    let theme = AppTheme::gitcomet_dark();
    let bolded = row(
        "a bold word",
        vec![MarkdownInlineSpan {
            byte_range: 2..6,
            style: MarkdownInlineStyle::Bold,
            link_url: None,
        }],
    );

    let base = markdown_preview_styled_row_with_query(theme, &bolded, 0, None, None);
    // `word` sits outside the bold span, so the wash has to add a range of
    // its own rather than restyle one that was already there.
    let washed =
        markdown_preview_styled_row_with_query(theme, &bolded, 0, Some(&query("word", None)), None);
    assert!(
        washed.highlights.len() > base.highlights.len(),
        "expected the query wash to add a highlight range alongside the bold span"
    );

    // The `**` that made it bold is not in the rendered text.
    let unmatched =
        markdown_preview_styled_row_with_query(theme, &bolded, 0, Some(&query("**", None)), None);
    assert_eq!(
        unmatched.highlights.len(),
        base.highlights.len(),
        "markdown syntax the renderer consumed must not be searchable"
    );
}

/// The current match is washed differently from the rest, so stepping
/// through hits is visible.
#[test]
fn the_current_match_row_is_washed_differently_from_the_others() {
    let theme = AppTheme::gitcomet_dark();
    let plain = row("find me here", Vec::new());

    let current =
        markdown_preview_styled_row_with_query(theme, &plain, 3, Some(&query("me", Some(3))), None);
    let other =
        markdown_preview_styled_row_with_query(theme, &plain, 3, Some(&query("me", Some(9))), None);
    assert_ne!(
        current.highlights, other.highlights,
        "the row the search cursor sits on should not look like every other hit"
    );
}

/// Links are coloured, not underlined, until the pointer is on one; then the
/// whole link underlines, including runs of it that are styled differently.
#[test]
fn a_link_underlines_only_while_hovered() {
    let theme = AppTheme::gitcomet_dark();
    let link = |range: std::ops::Range<usize>, style, url: &str| MarkdownInlineSpan {
        byte_range: range,
        style,
        link_url: Some(url.into()),
    };
    // "see " [link **bold**](#a) " and " [other](#b)
    let linked = row(
        "see link bold and other",
        vec![
            link(4..9, MarkdownInlineStyle::Link, "#a"),
            link(9..13, MarkdownInlineStyle::Bold, "#a"),
            link(18..23, MarkdownInlineStyle::Link, "#b"),
        ],
    );
    let underlined = |hovered: Option<&std::ops::Range<usize>>| {
        markdown_preview_styled_row_with_query(theme, &linked, 0, None, hovered)
            .highlights
            .iter()
            .filter(|(_, style)| style.underline.is_some())
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        underlined(None),
        Vec::<std::ops::Range<usize>>::new(),
        "no underline at rest"
    );
    assert_eq!(
        underlined(Some(&(4..13))),
        vec![4..9, 9..13],
        "the hovered link underlines end to end, and only it"
    );
    // The hover survives the search wash laid over it.
    let washed = markdown_preview_styled_row_with_query(
        theme,
        &linked,
        0,
        Some(&query("link", None)),
        Some(&(4..13)),
    );
    assert!(
        washed
            .highlights
            .iter()
            .any(|(range, style)| range.start >= 4 && range.end <= 13 && style.underline.is_some()),
        "a search match on a hovered link keeps its underline: {:?}",
        washed.highlights
    );
}
