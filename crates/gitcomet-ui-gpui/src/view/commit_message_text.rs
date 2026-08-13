//! Shared styling for rendered commit messages.
//!
//! The details pane and the history hover card style the same text the same
//! way; the details pane additionally attaches clickable link targets, which is
//! why it keeps its own assembly of these pieces rather than calling
//! [`commit_message_highlights`].

use super::*;

pub(in crate::view) type TextHighlight = (std::ops::Range<usize>, gpui::HighlightStyle);
pub(in crate::view) type TextHighlights = Vec<TextHighlight>;

/// Accent + underline for a SHA or URL inside a commit message.
pub(in crate::view) fn commit_link_style(theme: AppTheme) -> gpui::HighlightStyle {
    gpui::HighlightStyle {
        color: Some(theme.colors.accent.into()),
        underline: Some(gpui::UnderlineStyle {
            thickness: px(1.0),
            color: Some(theme.colors.accent.into()),
            wavy: false,
        }),
        ..gpui::HighlightStyle::default()
    }
}

/// Emphasis for the commit message's summary line (everything before the first
/// newline), skipping stretches already claimed by link highlights so the
/// resulting highlight set stays sorted and non-overlapping.
pub(in crate::view) fn commit_message_summary_highlights(
    message: &str,
    theme: AppTheme,
    link_highlights: &[TextHighlight],
) -> TextHighlights {
    let summary_end = message.find('\n').unwrap_or(message.len());
    if summary_end == 0 {
        return Vec::new();
    }
    let style = gpui::HighlightStyle {
        color: Some(theme.colors.emphasis_text.into()),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..gpui::HighlightStyle::default()
    };

    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (range, _) in link_highlights
        .iter()
        .filter(|(range, _)| range.start < summary_end)
    {
        if range.start > cursor {
            out.push((cursor..range.start.min(summary_end), style));
        }
        cursor = cursor.max(range.end);
    }
    if cursor < summary_end {
        out.push((cursor..summary_end, style));
    }
    out
}

/// Highlights for a commit message rendered without interaction: the summary
/// line emphasised and any SHA or URL accented.
///
/// The result is sanitized, so it is safe to hand straight to `StyledText` --
/// a highlight range that fell off a UTF-8 boundary would otherwise abort the
/// process inside `str::split_at`.
pub(in crate::view) fn commit_message_highlights(message: &str, theme: AppTheme) -> TextHighlights {
    let link_style = commit_link_style(theme);
    let links: TextHighlights = crate::text_selection::commit_message_link_ranges(message)
        .into_iter()
        .map(|link| (link.range, link_style))
        .collect();

    // Summary emphasis is computed against the links so the two never overlap.
    let mut highlights = commit_message_summary_highlights(message, theme, &links);
    highlights.extend(links);
    highlights.sort_by_key(|(range, _)| range.start);
    crate::text_runs::sanitize_highlights(message, &mut highlights);
    highlights
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> AppTheme {
        AppTheme::gitcomet_dark()
    }

    #[test]
    fn summary_line_is_emphasised_and_body_is_not() {
        let message = "Fix the thing\n\nA longer explanation.";
        let highlights = commit_message_highlights(message, theme());

        assert!(
            highlights
                .iter()
                .any(|(range, _)| *range == (0..message.find('\n').unwrap()))
        );
        let body_start = message.len() - "A longer explanation.".len();
        assert!(
            !highlights
                .iter()
                .any(|(range, _)| range.contains(&body_start))
        );
    }

    #[test]
    fn summary_emphasis_does_not_overlap_a_link_in_the_summary() {
        let message = "Revert 1234567890abcdef1234567890abcdef12345678 for now\n\nbody";
        let highlights = commit_message_highlights(message, theme());

        for pair in highlights.windows(2) {
            assert!(
                pair[0].0.end <= pair[1].0.start,
                "highlights overlap: {:?} then {:?}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    #[test]
    fn highlights_survive_multibyte_text() {
        // A range landing mid-character would abort the process when shaped.
        let message = "Fix “smart quotes” and émoji 🎉\n\nbody é";
        let highlights = commit_message_highlights(message, theme());

        assert!(crate::text_runs::highlights_are_shapeable(
            message,
            &highlights
        ));
    }

    #[test]
    fn empty_summary_produces_no_highlights() {
        assert!(commit_message_highlights("\nbody only", theme()).is_empty());
    }
}
