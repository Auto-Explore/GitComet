//! Branch-filter match highlighting for sidebar labels.

use super::*;

/// Render a sidebar label, bold-accent highlighting the first case-insensitive
/// occurrence of the branch filter `query` (already trimmed and lowercased).
/// With no query or no match the plain label is returned so the unfiltered
/// sidebar renders exactly as before.
///
/// `text_size`/`font_weight` must repeat what the surrounding row already sets:
/// TruncatedText resolves unset text styles inside a deferred measure closure
/// that doesn't see ancestor styling, so an unset size would fall back to the
/// 1rem window default and the label would grow as soon as it matched.
pub(super) fn search_label_highlights(
    search: &crate::view::sidebar_search::SidebarSearch,
    label: &str,
    color: gpui::Rgba,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let mut ranges = Vec::new();
    search.matcher.find_ranges_into(label, &mut ranges, 16);
    ranges
        .into_iter()
        .map(|range| {
            (
                range,
                gpui::HighlightStyle {
                    color: Some(color.into_color()),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            )
        })
        .collect()
}

pub(super) fn filtered_label_element<V: 'static>(
    label: SharedString,
    source: Option<&str>,
    search: &crate::view::sidebar_search::SidebarSearch,
    text_color: gpui::Rgba,
    highlight_color: gpui::Rgba,
    text_size: gpui::AbsoluteLength,
    font_weight: FontWeight,
    cx: &gpui::Context<V>,
) -> AnyElement {
    let mut ranges = Vec::new();
    let source = source.unwrap_or(&label);
    search.matcher.find_ranges_into(source, &mut ranges, 16);
    if source.ends_with(label.as_ref()) {
        let offset = source.len() - label.len();
        ranges = ranges
            .into_iter()
            .filter_map(|range| {
                (range.end > offset).then(|| range.start.saturating_sub(offset)..range.end - offset)
            })
            .collect();
    }
    if !ranges.is_empty() {
        let highlight = gpui::HighlightStyle {
            color: Some(highlight_color.into_color()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        components::TruncatedText::new(label, text_size)
            .text_color(text_color)
            .font_weight(font_weight)
            .highlights(ranges.into_iter().map(|range| (range, highlight)))
            .render(cx)
            .into_any_element()
    } else {
        label.into_any_element()
    }
}
