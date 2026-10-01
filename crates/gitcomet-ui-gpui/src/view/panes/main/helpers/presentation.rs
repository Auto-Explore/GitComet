//! Pixel metrics, scroll-reveal arithmetic, and resolved-output highlighting.

use super::*;
use crate::kit::rope::Rope;
use crate::kit::{HighlightProvider, HighlightProviderResult};
use palette::IntoColor;
use rustc_hash::FxHasher;

const DIFF_FILE_HEADER_HEIGHT_PX: f32 = 28.0;
const DIFF_HUNK_HEADER_HEIGHT_PX: f32 = 24.0;

/// Frames a sideways search reveal waits for its row to paint before giving up.
pub(in crate::view) const DIFF_SEARCH_HORIZONTAL_REVEAL_ATTEMPTS: u8 = 4;

/// The scroll offset a `uniform_list` would land on to reveal `row_ix`, or
/// `None` when the row is already fully visible and the list would not move.
///
/// Mirrors `uniform_list`'s own non-strict `ScrollStrategy::Center` arithmetic —
/// centre the row's midpoint in the viewport, clamp into the scrollable range,
/// and leave an already-visible row alone. Kept here so the editable resolved
/// output, which is a `TextInput` rather than a list, can be placed on exactly
/// the offset its gutter list is about to compute.
pub(in crate::view) fn centered_reveal_scroll_y(
    row_ix: usize,
    row_height: Pixels,
    viewport_height: Pixels,
    max_offset_y: Pixels,
    current_y: Pixels,
) -> Option<Pixels> {
    if row_height <= px(0.0) || viewport_height <= px(0.0) {
        return None;
    }
    let row_top = row_height * row_ix as f32;
    let row_bottom = row_top + row_height;
    let scroll_top = -current_y;
    let above = row_top < scroll_top;
    let below = row_bottom > scroll_top + viewport_height;
    if !above && !below {
        return None;
    }
    let target_top = (row_top + row_height / 2.0) - viewport_height / 2.0;
    Some(-target_top.clamp(px(0.0), max_offset_y.max(px(0.0))))
}

/// Margin kept between a revealed search match and the edge it was scrolled
/// past, so the hit does not sit flush against the pane border.
pub(in crate::view) const SEARCH_REVEAL_MARGIN_PX: f32 = 24.0;

/// The horizontal scroll offset that brings `[match_left, match_right]` into
/// view, or `None` when it already is and the pane should not move.
///
/// Unlike the vertical reveal this scrolls the *least* it can rather than
/// centring: a long line jumping sideways on every match is disorienting, and
/// the surrounding text is what makes a hit readable. A match too wide for the
/// viewport is anchored by its start, which is where reading resumes.
///
/// `match_left`/`match_right` are in content space; offsets run negative as the
/// view scrolls right, matching `ScrollHandle`.
pub(in crate::view) fn reveal_scroll_x(
    match_left: Pixels,
    match_right: Pixels,
    viewport_width: Pixels,
    max_offset_x: Pixels,
    current_x: Pixels,
) -> Option<Pixels> {
    if viewport_width <= px(0.0) {
        return None;
    }
    let margin = px(SEARCH_REVEAL_MARGIN_PX).min(viewport_width / 4.0);
    let view_left = -current_x;
    let view_right = view_left + viewport_width;

    let target_left = if match_left < view_left + margin {
        match_left - margin
    } else if match_right > view_right - margin {
        // Anchor the start when the match cannot fit, so reading begins at the
        // hit rather than at its tail.
        (match_right + margin - viewport_width).min(match_left - margin)
    } else {
        return None;
    };

    let target = -target_left.clamp(px(0.0), max_offset_x.max(px(0.0)));
    (target != current_x).then_some(target)
}

#[inline]
pub(in crate::view) fn diff_file_header_height_for_ui_scale(
    theme: AppTheme,
    ui_scale_percent: u32,
) -> Pixels {
    crate::ui_scale::design_px_from_percent(
        theme.metrics.row_height(DIFF_FILE_HEADER_HEIGHT_PX, 32.0),
        ui_scale_percent,
    )
}

#[inline]
pub(in crate::view) fn diff_hunk_header_height_for_ui_scale(
    theme: AppTheme,
    ui_scale_percent: u32,
) -> Pixels {
    crate::ui_scale::design_px_from_percent(
        DIFF_HUNK_HEADER_HEIGHT_PX.max(theme.metrics.editor_line_height() + 4.0),
        ui_scale_percent,
    )
}

/// Heuristic highlights for the rows overlapping `byte_range`.
///
/// Windowing this is *exact*, not an approximation: the heuristic tokenizer is
/// line-local, so a row's tokens do not depend on anything above it. (A
/// tree-sitter query is the opposite — it needs the enclosing tree, which is why
/// that path queries a range of a whole-document parse instead.)
///
/// Reading rows through the rope keeps the cost proportional to the viewport.
/// The previous shape tokenized the entire document and handed the result to
/// `set_highlights` on every keystroke, which is the one thing this arm — the
/// arm reached by the *largest* buffers — could least afford.
pub(in crate::view) fn resolved_output_heuristic_highlights_for_range(
    theme: AppTheme,
    output_text: &Rope,
    language: rows::DiffSyntaxLanguage,
    byte_range: Range<usize>,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let len = output_text.len();
    let start = byte_range.start.min(len);
    let end = byte_range.end.min(len).max(start);
    if start == end {
        return Vec::new();
    }

    let first_row = output_text.offset_to_point(start).row;
    let last_row = output_text.offset_to_point(end).row;
    let mut highlights = Vec::new();
    for row in first_row..=last_row {
        let line_range = output_text.line_range(row);
        if line_range.start >= len && row > first_row {
            break;
        }
        let line = output_text.line_text(row);
        for (range, style) in rows::syntax_highlights_for_line(
            theme,
            &line,
            language,
            rows::DiffSyntaxMode::HeuristicOnly,
        ) {
            highlights.push((
                (line_range.start + range.start)..(line_range.start + range.end),
                style,
            ));
        }
    }
    highlights
}

/// The fallback counterpart to [`resolved_output_live_highlight_provider`], for
/// buffers with no live tree — no wired grammar, or past the parse ceiling.
///
/// Same contract: answers whatever window the input asks for, never reports
/// pending, and carries the unresolved-conflict overlay on top.
pub(in crate::view::panes::main) fn resolved_output_heuristic_highlight_provider(
    theme: AppTheme,
    output_text: Rope,
    language: Option<rows::DiffSyntaxLanguage>,
    unresolved_spans: ResolvedOutputUnresolvedSpans,
) -> HighlightProvider {
    let unresolved_style = resolved_output_unresolved_highlight_style(theme);
    let active_unresolved_style = resolved_output_active_unresolved_highlight_style(theme);
    HighlightProvider::with_pending(
        move |byte_range: Range<usize>| HighlightProviderResult {
            highlights: apply_resolved_output_unresolved_highlights(
                language
                    .map(|language| {
                        resolved_output_heuristic_highlights_for_range(
                            theme,
                            &output_text,
                            language,
                            byte_range.clone(),
                        )
                    })
                    .unwrap_or_default(),
                &unresolved_spans,
                byte_range,
                unresolved_style,
                active_unresolved_style,
            ),
            pending: false,
        },
        || 0,
        || false,
    )
}

/// Binding key for the heuristic provider.
///
/// The live provider keys on its tree's version; this one has no tree, so it
/// keys on the buffer revision the closure captured, plus the theme and the
/// overlay. Distinct from the live key space so the two can never collide on a
/// buffer that switches arms.
pub(in crate::view::panes::main) fn resolved_output_heuristic_provider_binding_key(
    revision: ResolvedOutputSourceRevision,
    theme_epoch: u64,
    unresolved_spans: &ResolvedOutputUnresolvedSpans,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    "heuristic".hash(&mut hasher);
    revision.model_id.hash(&mut hasher);
    revision.revision.hash(&mut hasher);
    theme_epoch.hash(&mut hasher);
    unresolved_spans.all.hash(&mut hasher);
    unresolved_spans.active.hash(&mut hasher);
    hasher.finish()
}

pub(in crate::view::panes::main) fn resolved_output_unresolved_highlight_style(
    theme: AppTheme,
) -> gpui::HighlightStyle {
    gpui::HighlightStyle {
        color: Some(theme.colors.status.danger.foreground.into_color()),
        ..gpui::HighlightStyle::default()
    }
}

/// The unresolved treatment for the conflict the resolver is parked on: the same
/// danger text over a yellow wash, so the output says which of several open
/// `<Merge Conflict>` rows the picks and the source columns are about.
pub(in crate::view::panes::main) fn resolved_output_active_unresolved_highlight_style(
    theme: AppTheme,
) -> gpui::HighlightStyle {
    gpui::HighlightStyle {
        background_color: Some(resolved_output_active_conflict_background(theme).into_color()),
        ..resolved_output_unresolved_highlight_style(theme)
    }
}

/// The yellow the active conflict's row is washed with, shared by the editable
/// output's text highlight, its gutter row and the streamed read-only rows so
/// one row reads as one band across all three.
pub(in crate::view) fn resolved_output_active_conflict_background(theme: AppTheme) -> gpui::Rgba {
    with_alpha(
        theme.colors.status.warning.foreground,
        if theme.is_dark { 0.30 } else { 0.34 },
    )
}

/// The still-unresolved output rows, split into every one of them and the subset
/// belonging to the conflict the resolver is parked on.
///
/// Both are derived in one pass because this runs on the keystroke path, and
/// `active` is always a subset of `all` — the two can never disagree about where
/// a row starts and ends.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutputUnresolvedSpans {
    pub(in crate::view) all: Arc<[Range<usize>]>,
    pub(in crate::view) active: Arc<[Range<usize>]>,
}

impl ResolvedOutputUnresolvedSpans {
    fn is_active(&self, range: &Range<usize>) -> bool {
        self.active.iter().any(|active| active == range)
    }
}

/// Replace syntax styles inside unresolved output ranges with one plain danger
/// style — the active conflict's rows with the washed variant of it. The
/// returned ranges are non-overlapping with the unresolved spans, so the text
/// input's later-highlight precedence cannot reveal syntax colours through the
/// conflict treatment.
pub(in crate::view::panes::main) fn apply_resolved_output_unresolved_highlights(
    mut syntax_highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    unresolved_spans: &ResolvedOutputUnresolvedSpans,
    requested_range: Range<usize>,
    unresolved_style: gpui::HighlightStyle,
    active_unresolved_style: gpui::HighlightStyle,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let unresolved_ranges = unresolved_spans.all.as_ref();
    if unresolved_ranges.is_empty() || requested_range.is_empty() {
        return syntax_highlights;
    }

    let mut highlights = Vec::with_capacity(
        syntax_highlights
            .len()
            .saturating_add(unresolved_ranges.len()),
    );
    for (syntax_range, style) in syntax_highlights.drain(..) {
        if syntax_range.is_empty() {
            continue;
        }

        let mut cursor = syntax_range.start;
        let first_unresolved =
            unresolved_ranges.partition_point(|range| range.end <= syntax_range.start);
        for unresolved in unresolved_ranges.iter().skip(first_unresolved) {
            let unresolved_start = unresolved.start.max(requested_range.start);
            let unresolved_end = unresolved.end.min(requested_range.end);
            if unresolved_start >= syntax_range.end {
                break;
            }
            if unresolved_end <= cursor || unresolved_start >= unresolved_end {
                continue;
            }
            if cursor < unresolved_start {
                highlights.push((cursor..unresolved_start.min(syntax_range.end), style));
            }
            cursor = cursor.max(unresolved_end);
            if cursor >= syntax_range.end {
                break;
            }
        }
        if cursor < syntax_range.end {
            highlights.push((cursor..syntax_range.end, style));
        }
    }

    for unresolved in unresolved_ranges {
        let start = unresolved.start.max(requested_range.start);
        let end = unresolved.end.min(requested_range.end);
        if start < end {
            let style = if unresolved_spans.is_active(unresolved) {
                active_unresolved_style
            } else {
                unresolved_style
            };
            highlights.push((start..end, style));
        }
    }
    highlights.sort_by(|(left, _), (right, _)| {
        left.start.cmp(&right.start).then(left.end.cmp(&right.end))
    });

    let mut merged: Vec<(Range<usize>, gpui::HighlightStyle)> =
        Vec::with_capacity(highlights.len());
    for (range, style) in highlights {
        if let Some((previous_range, previous_style)) = merged.last_mut()
            && previous_range.end == range.start
            && *previous_style == style
        {
            previous_range.end = range.end;
        } else {
            merged.push((range, style));
        }
    }
    merged
}

/// The placeholder spans as tree-sitter should see them: the protected rows
/// minus their line terminator.
///
/// Keeping the `\n` real is deliberate. It guarantees the lines either side of a
/// masked row cannot lex as one token, and it keeps every row index — and so
/// every `Point` the incremental edit path computes — aligned with the text.
///
/// Derived from the same spans the buffer protects from editing, so the mask and
/// the protection can never drift apart.
pub(in crate::view::panes::main) fn resolved_output_live_syntax_mask(
    protected_ranges: &[Range<usize>],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
) -> Arc<[Range<usize>]> {
    if protected_ranges.is_empty() {
        return Arc::default();
    }
    let mut mask = Vec::with_capacity(protected_ranges.len());
    for range in protected_ranges {
        let mut end = range.end.min(output_text.len());
        if end > range.start && output_text.byte_at(end - 1) == Some(b'\n') {
            end -= 1;
        }
        if end > range.start && output_text.byte_at(end - 1) == Some(b'\r') {
            end -= 1;
        }
        if end > range.start {
            mask.push(range.start..end);
        }
    }
    mask.into()
}

/// Identity of everything the resolved-output highlight provider closes over.
///
/// This has to be *stable* when nothing changed, not merely unique. Installing a
/// provider notifies the input, which re-enters the `cx.observe` that installed
/// it; an always-fresh key would rebind on that re-entry, notify again, and spin
/// forever. `set_highlight_provider_with_key` early-returns on an unchanged key
/// without notifying, which is what terminates the cycle.
///
/// The document version covers the text and the tree; the theme and the
/// unresolved-conflict spans are the other two things baked into the closure.
pub(in crate::view::panes::main) fn resolved_output_live_provider_binding_key(
    document_version: u64,
    theme_epoch: u64,
    unresolved_spans: &ResolvedOutputUnresolvedSpans,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    document_version.hash(&mut hasher);
    theme_epoch.hash(&mut hasher);
    unresolved_spans.all.hash(&mut hasher);
    // Navigating between conflicts moves only this half, and it is what decides
    // which row wears the active wash — leave it out and the provider stays
    // bound to the previous conflict's highlight.
    unresolved_spans.active.hash(&mut hasher);
    hasher.finish()
}

/// Highlights for the resolved output, straight off the live tree.
///
/// Unlike the prepared-document provider this replaced, it is always exact for
/// the text it was built over and so never reports `pending`: the document is
/// re-synced on the keystroke and the provider rebound with it. That is what
/// keeps `TextInput`'s interpolation and superseded-source machinery dormant
/// here — they exist to cover a recompute lag this path does not have.
pub(in crate::view::panes::main) fn resolved_output_live_highlight_provider(
    theme: AppTheme,
    snapshot: rows::LiveSyntaxSnapshot,
    unresolved_spans: ResolvedOutputUnresolvedSpans,
) -> HighlightProvider {
    let unresolved_style = resolved_output_unresolved_highlight_style(theme);
    let active_unresolved_style = resolved_output_active_unresolved_highlight_style(theme);
    HighlightProvider::with_pending(
        move |byte_range: Range<usize>| HighlightProviderResult {
            highlights: apply_resolved_output_unresolved_highlights(
                snapshot.highlights_for_byte_range(byte_range.clone()),
                &unresolved_spans,
                byte_range,
                unresolved_style,
                active_unresolved_style,
            ),
            pending: false,
        },
        || 0,
        || false,
    )
}

/// Fold a batch of edits into the single `(replaced, inserted)` span that covers
/// them all, in the coordinates `LiveSyntaxDocument::sync` expects.
///
/// Each delta is expressed against the buffer as it stood when that delta was
/// applied, and only the final line starts survive to this point, so translating
/// them individually would compute positions against the wrong text. One wider
/// edit is always sound — it just reparses a little more than strictly needed —
/// and GPUI coalesces notifications, so in practice the batch is one delta.
///
/// Mirrors the union arithmetic in `HighlightInterpolation::record_edit`.
pub(in crate::view::panes::main) fn coalesce_resolved_output_edit_deltas(
    deltas: &[(Range<usize>, Range<usize>)],
) -> Option<(Range<usize>, Range<usize>)> {
    let mut folded: Option<(usize, usize, usize)> = None; // (start, old_len, new_len)
    for (replaced, inserted) in deltas {
        folded = Some(match folded {
            None => (
                replaced.start,
                replaced.end.saturating_sub(replaced.start),
                inserted.end.saturating_sub(inserted.start),
            ),
            Some((start, old_len, new_len)) => {
                let union_start = start.min(replaced.start);
                let union_right = start.saturating_add(new_len).max(replaced.end);
                let source_right = union_right - new_len + old_len;
                let live_right =
                    union_right - (replaced.end - replaced.start) + (inserted.end - inserted.start);
                (
                    union_start,
                    source_right.saturating_sub(union_start),
                    live_right.saturating_sub(union_start),
                )
            }
        });
    }
    folded.map(|(start, old_len, new_len)| (start..start + old_len, start..start + new_len))
}

/// Whether the content pane is showing a file's full content *at the commit the
/// file browser is pinned to* — the state the historical browse tint marks.
///
/// Content-preview mode alone is not enough: a file's content can be opened from
/// some other commit while a browse point is active, and that content is not
/// what the browse point describes. The commit ids have to match.
pub(in crate::view::panes::main) fn historical_browse_content(
    repo: &RepoState,
    rendered_target: Option<&DiffTarget>,
) -> bool {
    if !repo.diff_state.content_preview {
        return false;
    }
    let Some(browsing) = repo.browsing_commit() else {
        return false;
    };
    matches!(
        rendered_target,
        Some(DiffTarget::Commit { commit_id, .. }) if commit_id == browsing
    )
}

pub(in crate::view::panes::main) fn parse_conflict_canvas_rows_env(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

pub(in crate::view::panes::main) fn conflict_canvas_rows_enabled_from_env() -> bool {
    std::env::var("GITCOMET_CONFLICT_CANVAS_ROWS")
        .ok()
        .is_none_or(|value| parse_conflict_canvas_rows_env(&value))
}

#[cfg(test)]
mod search_reveal_x_tests {
    use super::{SEARCH_REVEAL_MARGIN_PX, reveal_scroll_x};
    use gpui::px;

    fn viewport() -> gpui::Pixels {
        px(800.0)
    }

    fn max_offset() -> gpui::Pixels {
        px(4000.0)
    }

    #[test]
    fn a_match_already_on_screen_does_not_move_the_view() {
        assert_eq!(
            reveal_scroll_x(px(200.0), px(260.0), viewport(), max_offset(), px(0.0)),
            None
        );
    }

    #[test]
    fn a_match_off_the_right_edge_scrolls_just_far_enough_to_show_it() {
        // Right edge at 800; the match ends at 900, so the view slides by the
        // overshoot plus the margin and no further.
        let target = reveal_scroll_x(px(840.0), px(900.0), viewport(), max_offset(), px(0.0))
            .expect("expected the view to scroll right");
        assert_eq!(target, px(-(900.0 + SEARCH_REVEAL_MARGIN_PX - 800.0)));
    }

    #[test]
    fn a_match_off_the_left_edge_scrolls_back_to_it() {
        // Scrolled 1000 right, with the match at 300 behind the left edge.
        let target = reveal_scroll_x(px(300.0), px(360.0), viewport(), max_offset(), px(-1000.0))
            .expect("expected the view to scroll left");
        assert_eq!(target, px(-(300.0 - SEARCH_REVEAL_MARGIN_PX)));
    }

    #[test]
    fn a_match_wider_than_the_viewport_is_anchored_by_its_start() {
        let target = reveal_scroll_x(px(1000.0), px(3000.0), viewport(), max_offset(), px(0.0))
            .expect("expected the view to scroll right");
        assert_eq!(target, px(-(1000.0 - SEARCH_REVEAL_MARGIN_PX)));
    }

    #[test]
    fn the_target_is_clamped_into_the_scrollable_range() {
        // Never past the end of the content...
        assert_eq!(
            reveal_scroll_x(px(9000.0), px(9060.0), viewport(), px(500.0), px(0.0)),
            Some(px(-500.0))
        );
        // ...and never before its start.
        assert_eq!(
            reveal_scroll_x(px(0.0), px(10.0), viewport(), max_offset(), px(-40.0)),
            Some(px(0.0))
        );
    }

    #[test]
    fn an_unmeasured_viewport_has_no_reveal_to_compute() {
        assert_eq!(
            reveal_scroll_x(px(1000.0), px(1060.0), px(0.0), max_offset(), px(0.0)),
            None
        );
    }
}
