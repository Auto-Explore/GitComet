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
        code_block_horizontal_scroll_hint: false,
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
        measured_width_px: Default::default(),
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
        markdown_preview_reveal_offset_y(px(1000.0), px(20.0), px(400.0), px(2000.0), px(0.0)),
        Some(px(-810.0))
    );
    // A row near the top cannot be centred; the document stops at its top.
    assert_eq!(
        markdown_preview_reveal_offset_y(px(10.0), px(20.0), px(400.0), px(2000.0), px(-50.0)),
        Some(px(0.0))
    );
    // Past the end of the scrollable range, clamp to the bottom.
    assert_eq!(
        markdown_preview_reveal_offset_y(px(5000.0), px(20.0), px(400.0), px(600.0), px(0.0)),
        Some(px(-600.0))
    );
    // Already there: no scroll, so nothing repaints.
    assert_eq!(
        markdown_preview_reveal_offset_y(px(1000.0), px(20.0), px(400.0), px(2000.0), px(-810.0)),
        None
    );
    // An unmeasured container has no centre to compute.
    assert_eq!(
        markdown_preview_reveal_offset_y(px(1000.0), px(20.0), px(0.0), px(2000.0), px(0.0)),
        None
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

    let base = markdown_preview_styled_row_with_query(theme, &bolded, 0, None);
    // `word` sits outside the bold span, so the wash has to add a range of
    // its own rather than restyle one that was already there.
    let washed =
        markdown_preview_styled_row_with_query(theme, &bolded, 0, Some(&query("word", None)));
    assert!(
        washed.highlights.len() > base.highlights.len(),
        "expected the query wash to add a highlight range alongside the bold span"
    );

    // The `**` that made it bold is not in the rendered text.
    let unmatched =
        markdown_preview_styled_row_with_query(theme, &bolded, 0, Some(&query("**", None)));
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
        markdown_preview_styled_row_with_query(theme, &plain, 3, Some(&query("me", Some(3))));
    let other =
        markdown_preview_styled_row_with_query(theme, &plain, 3, Some(&query("me", Some(9))));
    assert_ne!(
        current.highlights, other.highlights,
        "the row the search cursor sits on should not look like every other hit"
    );
}
