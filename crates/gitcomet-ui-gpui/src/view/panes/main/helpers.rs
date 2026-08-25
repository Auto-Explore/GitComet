use super::*;
use crate::kit::rope::Rope;
use crate::kit::text_model::TextModelSnapshot;
use crate::kit::{HighlightProvider, HighlightProviderResult};
use crate::view::conflict_resolver::ConflictSegment;
use palette::IntoColor;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::collections::HashSet;

const DIFF_ROW_HEIGHT_PX: f32 = 20.0;
const DIFF_FILE_HEADER_HEIGHT_PX: f32 = 28.0;
const DIFF_HUNK_HEADER_HEIGHT_PX: f32 = 24.0;
/// Height of one resolved-output gutter row. The row space navigation scrolls
/// through is measured in these, so anything computing an output scroll offset
/// by hand has to agree with what the gutter actually lays out.
pub(in crate::view) const RESOLVED_OUTPUT_ROW_HEIGHT_PX: f32 = 20.0;

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
fn scaled_diff_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

#[inline]
pub(in crate::view) fn diff_row_height_for_ui_scale(ui_scale_percent: u32) -> Pixels {
    scaled_diff_px(DIFF_ROW_HEIGHT_PX, ui_scale_percent)
}

#[inline]
pub(in crate::view) fn diff_file_header_height_for_ui_scale(ui_scale_percent: u32) -> Pixels {
    scaled_diff_px(DIFF_FILE_HEADER_HEIGHT_PX, ui_scale_percent)
}

#[inline]
pub(in crate::view) fn diff_hunk_header_height_for_ui_scale(ui_scale_percent: u32) -> Pixels {
    scaled_diff_px(DIFF_HUNK_HEADER_HEIGHT_PX, ui_scale_percent)
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
pub(super) fn resolved_output_heuristic_highlights_for_range(
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
pub(super) fn resolved_output_heuristic_highlight_provider(
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
pub(super) fn resolved_output_heuristic_provider_binding_key(
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

pub(super) fn resolved_output_unresolved_highlight_style(theme: AppTheme) -> gpui::HighlightStyle {
    gpui::HighlightStyle {
        color: Some(theme.colors.status.danger.foreground.into_color()),
        ..gpui::HighlightStyle::default()
    }
}

/// The unresolved treatment for the conflict the resolver is parked on: the same
/// danger text over a yellow wash, so the output says which of several open
/// `<Merge Conflict>` rows the picks and the source columns are about.
pub(super) fn resolved_output_active_unresolved_highlight_style(
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
pub(super) fn apply_resolved_output_unresolved_highlights(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutputSourceRevision {
    pub(in crate::view) model_id: u64,
    pub(in crate::view) revision: u64,
}

impl ResolvedOutputSourceRevision {
    pub(in crate::view) fn from_snapshot(snapshot: &TextModelSnapshot) -> Self {
        Self {
            model_id: snapshot.model_id(),
            revision: snapshot.revision(),
        }
    }
}

pub(super) fn resolved_output_snapshot_is_modified(
    saved: Option<&TextModelSnapshot>,
    current: &TextModelSnapshot,
) -> bool {
    saved.is_some_and(|saved| current != saved)
}

/// Whether the worktree payload must be kept as opaque user output instead of
/// replacing it with the stage-derived marker projection.
///
/// The question this answers is "would projecting throw away work someone did by
/// hand?", and the only usable evidence is the document's own content: every
/// line of an untouched conflict document comes from one of the three stages,
/// because git assembled it out of them. A hand resolution types something new,
/// and that line belongs to no stage.
///
/// The comparison is deliberately *not* against the projection or the stage
/// blobs. Two correct three-way merges of the same stages may place their
/// conflict boundaries in entirely different places — ours anchors differently
/// than git's `xdiff` does, and the contributor-alignment pass moves boundaries
/// again — so the two documents interleave the same lines in different orders
/// and neither reconstructs to the other's sides nor to a whole stage blob.
/// Demanding either equality protected essentially every real merge, which left
/// the resolver inert: no marker geometry, every pick a silent no-op, and no way
/// out but *Reset conflict markers*.
///
/// The cost of the weaker test is that a hand resolution built purely by
/// *deleting* lines — picking a side in an editor, markers and all — reads as
/// untouched. Nothing is lost on disk either way: the resolver only rewrites its
/// own buffer, and the worktree file stands until an explicit Save.
pub(super) fn worktree_output_requires_protection(
    current: Option<&str>,
    marker_projection: Option<&str>,
    base: Option<&str>,
    ours: Option<&str>,
    theirs: Option<&str>,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    if marker_projection == Some(current) {
        return false;
    }
    // The projection renders every line with one detected ending, so a document
    // that mixes CRLF and LF cannot be reproduced from it even when every line
    // of it comes from a stage. Keep the worktree bytes rather than rewriting
    // terminators the user never touched.
    if gitcomet_core::text_utils::text_has_mixed_line_endings(current) {
        return true;
    }

    let marker_ranges = gitcomet_core::conflict_session::parse_conflict_marker_ranges(current);
    if !marker_ranges.iter().any(|segment| {
        matches!(
            segment,
            gitcomet_core::conflict_session::ParsedConflictSegmentRanges::Conflict(_)
        )
    }) {
        return true;
    }

    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        return true;
    };
    // `std`'s seeded `HashSet`, not `FxHashSet`: the keys are raw file lines
    // off disk, i.e. content an untrusted repository controls.
    let stage_lines: HashSet<&str> = base
        .into_iter()
        .chain([ours, theirs])
        .flat_map(str::lines)
        .collect();
    !conflict_document_content_lines(current, &marker_ranges).all(|line| stage_lines.contains(line))
}

/// The document's lines with the four marker lines left out, so a comparison
/// against stage content is not defeated by the labels git wrote.
fn conflict_document_content_lines<'a>(
    current: &'a str,
    marker_ranges: &'a [gitcomet_core::conflict_session::ParsedConflictSegmentRanges],
) -> impl Iterator<Item = &'a str> {
    use gitcomet_core::conflict_session::ParsedConflictSegmentRanges as Segment;

    marker_ranges
        .iter()
        .flat_map(move |segment| {
            let ranges = match segment {
                Segment::Text(range) => vec![range.clone()],
                Segment::Conflict(block) => [Some(block.ours.clone()), block.base.clone()]
                    .into_iter()
                    .flatten()
                    .chain([block.theirs.clone()])
                    .collect(),
            };
            ranges.into_iter()
        })
        .flat_map(move |range| current.get(range).unwrap_or_default().lines())
}

#[derive(Clone, Debug)]
pub(in crate::view) struct VersionedCachedDiffStyledText {
    pub(in crate::view) syntax_epoch: u64,
    pub(in crate::view) query_generation: u64,
    pub(in crate::view) styled: CachedDiffStyledText,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct StashedResolvedOutlineState {
    pub(in crate::view) text: TextModelSnapshot,
    pub(in crate::view) line_starts: Arc<[usize]>,
    pub(in crate::view) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    pub(in crate::view) view_mode: ConflictResolverViewMode,
    pub(in crate::view) outline: ResolvedOutlineData,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct FileDiffStyleCacheEpochs {
    pub(in crate::view) split_left: u64,
    pub(in crate::view) split_right: u64,
}

impl FileDiffStyleCacheEpochs {
    pub(in crate::view) fn bump_left(&mut self) {
        self.split_left = self.split_left.wrapping_add(1);
    }

    pub(in crate::view) fn bump_right(&mut self) {
        self.split_right = self.split_right.wrapping_add(1);
    }

    pub(in crate::view) fn bump_both(&mut self) {
        self.bump_left();
        self.bump_right();
    }

    pub(in crate::view) fn split_epoch(self, region: crate::view::DiffTextRegion) -> u64 {
        match region {
            crate::view::DiffTextRegion::SplitLeft => self.split_left,
            crate::view::DiffTextRegion::SplitRight => self.split_right,
            crate::view::DiffTextRegion::Inline => 0,
        }
    }

    pub(in crate::view) fn inline_epoch(self, kind: gitcomet_core::domain::DiffLineKind) -> u64 {
        match kind {
            gitcomet_core::domain::DiffLineKind::Remove => self.split_left,
            gitcomet_core::domain::DiffLineKind::Add
            | gitcomet_core::domain::DiffLineKind::Context => self.split_right,
            gitcomet_core::domain::DiffLineKind::Header
            | gitcomet_core::domain::DiffLineKind::Hunk => 0,
        }
    }
}

pub(in crate::view) const FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES: usize = 4_096;

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct FileDiffSplitWordHighlights {
    pub(in crate::view) old: Vec<Range<usize>>,
    pub(in crate::view) new: Vec<Range<usize>>,
}

pub(in crate::view) fn versioned_cached_diff_styled_text_is_current(
    entry: Option<&VersionedCachedDiffStyledText>,
    syntax_epoch: u64,
) -> Option<&CachedDiffStyledText> {
    let entry = entry?;
    (entry.syntax_epoch == syntax_epoch).then_some(&entry.styled)
}

pub(in crate::view) fn versioned_query_cached_diff_styled_text_is_current(
    entry: Option<&VersionedCachedDiffStyledText>,
    syntax_epoch: u64,
    query_generation: u64,
) -> Option<&CachedDiffStyledText> {
    let entry = entry?;
    (entry.syntax_epoch == syntax_epoch && entry.query_generation == query_generation)
        .then_some(&entry.styled)
}

pub(super) fn count_newlines(text: &str) -> usize {
    text.as_bytes().iter().filter(|&&b| b == b'\n').count()
}

pub(super) fn build_line_starts(text: &str) -> Vec<usize> {
    build_line_starts_with_count(text).0
}

pub(super) fn build_line_starts_with_count(text: &str) -> (Vec<usize>, usize) {
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
pub(super) fn preview_source_text_from_lines(lines: &[String], source_len: usize) -> SharedString {
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

pub(super) fn line_start_offset_for_index(
    line_starts: &[usize],
    text_len: usize,
    line_ix: usize,
) -> usize {
    line_starts.get(line_ix).copied().unwrap_or(text_len)
}

pub(super) fn source_line_count(text: &str) -> usize {
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
    if source_len == 0 {
        0
    } else {
        line_starts.len().max(1)
    }
}

pub(super) fn indexed_line_count(text: &str, line_starts: &[usize]) -> usize {
    indexed_line_count_from_len(text.len(), line_starts)
}

pub(in crate::view) fn indexed_line_byte_range(
    line_starts: &[usize],
    source_len: usize,
    line_ix: usize,
) -> Option<Range<usize>> {
    let line_count = indexed_line_count_from_len(source_len, line_starts);
    if line_ix >= line_count {
        return None;
    }

    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(source_len)
        .min(source_len);
    let end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .map(|next| next.saturating_sub(1))
        .unwrap_or(source_len)
        .min(source_len)
        .max(start);
    Some(start..end)
}

/// Number of logical rows produced by `split('\n')` (always at least 1).
pub(super) fn split_line_count(text: &str) -> usize {
    count_newlines(text).saturating_add(1)
}

/// Full resolved-output provenance is much more expensive in three-way mode,
/// because it builds source-line lookups across all three full documents.
pub(super) const LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES: usize = 50_000;
/// Two-way mode still needs a cap, because the source-index alone scales with
/// output-line count even when the diff-row lookup is small.
pub(super) const LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES: usize = 200_000;

pub(super) fn should_skip_resolved_outline_provenance(
    view_mode: ConflictResolverViewMode,
    output_line_count: usize,
) -> bool {
    match view_mode {
        ConflictResolverViewMode::ThreeWay => {
            output_line_count > LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES
        }
        ConflictResolverViewMode::TwoWayDiff => {
            output_line_count > LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES
        }
    }
}

/// Byte range of line content at `line_ix` (without trailing newline).
///
/// Uses `split('\n')` row semantics, so trailing newline creates a final empty row.
pub(super) fn line_content_byte_range_for_index(
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
pub(super) fn append_line_insertion_text(existing: &str, line: &str) -> String {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedOutlineDelta {
    pub(super) old_range: Range<usize>,
    pub(super) new_range: Range<usize>,
}

pub(super) fn resolved_outline_delta_between_texts(
    old_text: &str,
    new_text: &str,
) -> Option<ResolvedOutlineDelta> {
    if old_text == new_text {
        return None;
    }

    let old = old_text.as_bytes();
    let new = new_text.as_bytes();
    let old_len = old.len();
    let new_len = new.len();

    let mut prefix = 0usize;
    let prefix_max = old_len.min(new_len);
    while prefix < prefix_max && old[prefix] == new[prefix] {
        prefix = prefix.saturating_add(1);
    }
    while prefix > 0 && (!old_text.is_char_boundary(prefix) || !new_text.is_char_boundary(prefix)) {
        prefix = prefix.saturating_sub(1);
    }

    let mut suffix = 0usize;
    while suffix < old_len.saturating_sub(prefix)
        && suffix < new_len.saturating_sub(prefix)
        && old[old_len.saturating_sub(1 + suffix)] == new[new_len.saturating_sub(1 + suffix)]
    {
        suffix = suffix.saturating_add(1);
    }
    while suffix > 0
        && (!old_text.is_char_boundary(old_len.saturating_sub(suffix))
            || !new_text.is_char_boundary(new_len.saturating_sub(suffix)))
    {
        suffix = suffix.saturating_sub(1);
    }

    Some(ResolvedOutlineDelta {
        old_range: prefix..old_len.saturating_sub(suffix),
        new_range: prefix..new_len.saturating_sub(suffix),
    })
}

pub(super) fn resolved_outline_delta_for_snapshot_transition(
    old_snapshot: &TextModelSnapshot,
    new_snapshot: &TextModelSnapshot,
    recent_edit_delta: Option<(Range<usize>, Range<usize>)>,
) -> Option<ResolvedOutlineDelta> {
    if old_snapshot.model_id() == new_snapshot.model_id()
        && new_snapshot.revision() == old_snapshot.revision().saturating_add(1)
        && let Some((old_range, new_range)) = recent_edit_delta
    {
        return Some(ResolvedOutlineDelta {
            old_range,
            new_range,
        });
    }

    // Do not materialize and compare both documents on the immediate input
    // notification path. If observer delivery coalesced several revisions, the
    // surviving debounced task will perform the full outline recompute instead.
    None
}

fn line_index_for_byte_offset(line_starts: &[usize], byte_offset: usize) -> usize {
    if line_starts.is_empty() {
        return 0;
    }
    line_starts
        .partition_point(|&start| start <= byte_offset)
        .saturating_sub(1)
}

pub(super) fn dirty_byte_range_to_line_range(
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

pub(super) fn shifted_line_index(ix: usize, delta: isize) -> usize {
    if delta >= 0 {
        ix.saturating_add(delta as usize)
    } else {
        ix.saturating_sub((-delta) as usize)
    }
}

pub(super) fn remap_resolved_output_conflict_block_ranges_for_delta(
    old_block_ranges: &[Range<usize>],
    old_range: Range<usize>,
    new_range: Range<usize>,
    new_line_count: usize,
) -> Vec<Range<usize>> {
    let line_delta = new_range.len() as isize - old_range.len() as isize;
    old_block_ranges
        .iter()
        .map(|range| {
            let remapped = if range.end <= old_range.start {
                range.clone()
            } else if range.start >= old_range.end {
                shifted_line_index(range.start, line_delta)
                    ..shifted_line_index(range.end, line_delta)
            } else {
                let start = if old_range.start <= range.start {
                    new_range.start
                } else {
                    range.start
                };
                let end = if range.end <= old_range.end {
                    new_range.end
                } else {
                    shifted_line_index(range.end, line_delta)
                };
                start..end
            };
            remapped.start.min(new_line_count)..remapped.end.min(new_line_count)
        })
        .map(|range| range.start..range.end.max(range.start))
        .collect()
}

pub(super) fn resolved_output_conflict_block_ranges_in_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
) -> Option<Vec<Range<usize>>> {
    fn is_line_boundary(
        text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
        byte_ix: usize,
    ) -> bool {
        if byte_ix == 0 || byte_ix == text.len() {
            return true;
        }
        text.byte_at(byte_ix.saturating_sub(1))
            .is_some_and(|b| b == b'\n')
    }

    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    let mut line_offset = 0usize;
    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                if !output_text.starts_with_at(cursor, text.as_str()) {
                    return None;
                }
                cursor = cursor.saturating_add(text.len());
                line_offset = line_offset.saturating_add(count_newlines(text));
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let expected = conflict_resolver::generate_resolved_text(&[
                    conflict_resolver::ConflictSegment::Block(block.clone()),
                ]);
                if !output_text.starts_with_at(cursor, &expected) {
                    return None;
                }
                let end = cursor.saturating_add(expected.len());
                if end < cursor
                    || !is_line_boundary(output_text, cursor)
                    || !is_line_boundary(output_text, end)
                {
                    return None;
                }
                let start_line = line_offset;
                let mut end_line = line_offset.saturating_add(count_newlines(&expected));
                // A block that ends the file without a trailing newline still
                // occupies its last line, which no newline accounts for. Only
                // that case needs the extra row: when the block *is* newline
                // terminated, the outline still keeps an empty row after the
                // final newline (`resolved_output_outline_line_count`), and
                // claiming it would put this block's `?` gutter and conflict
                // bracket on a row that belongs to no conflict.
                if end == output_text.len() && !expected.is_empty() && !expected.ends_with('\n') {
                    end_line = end_line.saturating_add(1);
                }
                ranges.push(start_line..end_line);
                line_offset = line_offset.saturating_add(count_newlines(&expected));
                cursor = end;
            }
        }
    }

    Some(ranges)
}

/// Line ranges for the displayed conflict blocks, tolerating manual edits.
///
/// The walk above only reports ranges while the buffer still reads back exactly
/// as the segments render, so one keystroke anywhere in the output drops every
/// marker at once — placeholders lose their conflict color, their bracket and
/// their chunk menu. `ResolvedOutputBlockMap` carries block byte ownership
/// through edits, so fall back to it and convert its ranges into line space.
pub(super) fn resolved_output_conflict_block_line_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Option<Vec<Range<usize>>> {
    resolved_output_conflict_block_ranges_in_text(marker_segments, output_text).or_else(|| {
        conflict_block_line_ranges_from_block_map(marker_segments, output_text, block_map)
    })
}

fn conflict_block_line_ranges_from_block_map(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Option<Vec<Range<usize>>> {
    if !block_map.is_valid_for(marker_segments, output_text) {
        return None;
    }

    let byte_ranges = block_map.ranges();
    let mut line_ranges = Vec::with_capacity(byte_ranges.len());
    // The map keeps its ranges sorted and disjoint, so one forward pass counts
    // every newline exactly once instead of rescanning the prefix per block.
    let mut cursor = 0usize;
    let mut line = 0usize;
    for range in byte_ranges {
        let start_line = line.saturating_add(output_text.count_newlines_in(cursor..range.start));
        let body_newlines = output_text.count_newlines_in(range.clone());
        let mut end_line = start_line.saturating_add(body_newlines);
        // A block that ends the file without a trailing newline still occupies
        // its last row, which no newline accounts for — matching the strict
        // walk. The carried `line` below must not include this adjustment: it
        // counts newlines actually seen, and the next block's start is measured
        // from those.
        let body_is_empty = range.start == range.end;
        let body_ends_with_newline = range
            .end
            .checked_sub(1)
            .and_then(|last| output_text.byte_at(last))
            .is_some_and(|byte| byte == b'\n');
        if range.end == output_text.len() && !body_is_empty && !body_ends_with_newline {
            end_line = end_line.saturating_add(1);
        }
        line_ranges.push(start_line..end_line);
        line = start_line.saturating_add(body_newlines);
        cursor = range.end;
    }

    Some(line_ranges)
}

pub(super) fn conflict_marker_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
    line_range: Range<usize>,
) -> Vec<Range<usize>> {
    if !block.resolved && block.choice.is_empty() {
        return vec![line_range];
    }

    let mut marker_ranges = Vec::new();
    if !block.resolved
        && let Some(relative_subranges) = unresolved_decision_ranges_for_block(block)
            .or_else(|| unresolved_subchunk_conflict_ranges_for_block(block))
    {
        for relative in relative_subranges {
            let start = line_range
                .start
                .saturating_add(relative.start)
                .min(line_range.end);
            let end = line_range
                .start
                .saturating_add(relative.end)
                .min(line_range.end);
            marker_ranges.push(start..end);
        }
    }
    if marker_ranges.is_empty() {
        marker_ranges.push(line_range);
    }
    marker_ranges
}

pub(super) fn write_conflict_markers_for_ranges(
    markers: &mut [Option<ResolvedOutputConflictMarker>],
    conflict_ix: usize,
    unresolved: bool,
    marker_ranges: &[Range<usize>],
) {
    let output_line_count = markers.len();
    if output_line_count == 0 {
        return;
    }

    for marker_range in marker_ranges {
        if marker_range.start < marker_range.end {
            let end = marker_range.end.min(output_line_count);
            for (line_ix, marker_slot) in markers
                .iter_mut()
                .enumerate()
                .take(end)
                .skip(marker_range.start)
            {
                *marker_slot = Some(ResolvedOutputConflictMarker {
                    conflict_ix,
                    range_start: marker_range.start,
                    range_end: marker_range.end,
                    is_start: line_ix == marker_range.start,
                    is_end: line_ix + 1 == marker_range.end,
                    unresolved,
                });
            }
            continue;
        }

        let anchor = marker_range.start.min(output_line_count.saturating_sub(1));
        markers[anchor] = Some(ResolvedOutputConflictMarker {
            conflict_ix,
            range_start: marker_range.start,
            range_end: marker_range.end,
            is_start: true,
            is_end: true,
            unresolved,
        });
    }
}

pub(super) fn output_line_range_for_conflict_block_in_text(
    segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    conflict_ix: usize,
) -> Option<Range<usize>> {
    resolved_output_conflict_block_ranges_in_text(segments, output_text)
        .and_then(|ranges| ranges.get(conflict_ix).cloned())
}

pub(super) fn conflict_fragment_text_for_choice(
    base: &str,
    ours: &str,
    theirs: &str,
    choice: conflict_resolver::ConflictChoice,
) -> String {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    let mut out = String::new();
    for source in choice.iter() {
        match source {
            ConflictOutputSource::Base => out.push_str(base),
            ConflictOutputSource::Ours => out.push_str(ours),
            ConflictOutputSource::Theirs => out.push_str(theirs),
        }
    }
    out
}

pub(super) fn unresolved_subchunk_conflict_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    use gitcomet_core::conflict_session::Subchunk;

    let base = block.base.as_deref()?;
    let subchunks = gitcomet_core::conflict_session::split_conflict_into_subchunks(
        base,
        &block.ours,
        &block.theirs,
    )?;
    let mut ranges = Vec::new();
    let mut line_offset = 0usize;
    for subchunk in subchunks {
        let (fragment, is_conflict) = match subchunk {
            Subchunk::Resolved(text) => (text, false),
            Subchunk::Conflict { base, ours, theirs } => (
                conflict_fragment_text_for_choice(&base, &ours, &theirs, block.choice),
                true,
            ),
        };
        let start = line_offset;
        line_offset = line_offset.saturating_add(count_newlines(&fragment));
        if is_conflict {
            ranges.push(start..line_offset);
        }
    }
    if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

#[derive(Clone, Debug)]
pub(super) struct UnresolvedDecisionRegion {
    pub(super) row_range: Range<usize>,
    pub(super) selected_line_range: Range<usize>,
    pub(super) alternate_line_range: Range<usize>,
    pub(super) has_non_emitting_rows: bool,
}

pub(super) fn unresolved_decision_regions_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<UnresolvedDecisionRegion>> {
    let (left, right, choose_left) = match block.choice {
        conflict_resolver::ConflictChoice::Ours => (&block.ours, &block.theirs, true),
        conflict_resolver::ConflictChoice::Theirs => (&block.theirs, &block.ours, false),
        _ => return None,
    };
    let plan = gitcomet_core::file_diff::side_by_side_plan(left, right);
    if plan.row_count == 0 {
        return None;
    }
    let regions = gitcomet_core::file_diff::plan_row_region_anchors(&plan).region_anchors;
    if regions.is_empty() {
        return None;
    }
    let (old_prefix, new_prefix) = gitcomet_core::file_diff::plan_emitted_line_prefix_counts(&plan);
    let (selected_prefix, alternate_prefix) = if choose_left {
        (&old_prefix, &new_prefix)
    } else {
        (&new_prefix, &old_prefix)
    };

    let mut decision_regions: Vec<UnresolvedDecisionRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        let row_start = region.row_start.min(plan.row_count);
        let row_end = region.row_end_exclusive.min(plan.row_count).max(row_start);
        let selected_line_range = selected_prefix[row_start]..selected_prefix[row_end];
        let alternate_line_range = alternate_prefix[row_start]..alternate_prefix[row_end];
        let emitted_rows = selected_line_range
            .end
            .saturating_sub(selected_line_range.start);
        let has_non_emitting_rows = emitted_rows < row_end.saturating_sub(row_start);

        if let Some(last) = decision_regions.last_mut()
            && last.selected_line_range == selected_line_range
        {
            last.row_range.end = row_end;
            last.alternate_line_range.end =
                last.alternate_line_range.end.max(alternate_line_range.end);
            last.has_non_emitting_rows |= has_non_emitting_rows;
            continue;
        }

        decision_regions.push(UnresolvedDecisionRegion {
            row_range: row_start..row_end,
            selected_line_range,
            alternate_line_range,
            has_non_emitting_rows,
        });
    }
    if decision_regions.is_empty() {
        return None;
    }

    // Merge nearby non-zero ranges into one logical decision chunk while
    // preserving insertion anchors as independent picks.
    const MERGE_GAP_LINES: usize = 1;
    let mut merged: Vec<UnresolvedDecisionRegion> = Vec::with_capacity(decision_regions.len());
    for next in decision_regions {
        if let Some(prev) = merged.last_mut() {
            let prev_zero = prev.selected_line_range.start == prev.selected_line_range.end;
            let next_zero = next.selected_line_range.start == next.selected_line_range.end;
            let can_merge = if prev_zero || next_zero {
                prev_zero
                    && next_zero
                    && next.selected_line_range.start
                        <= prev.selected_line_range.end.saturating_add(MERGE_GAP_LINES)
            } else {
                // Keep ranges with insertion/deletion-only rows separate so
                // structural additions (e.g. trailing inserted methods) don't
                // collapse into preceding modification chunks.
                !prev.has_non_emitting_rows
                    && !next.has_non_emitting_rows
                    && next.selected_line_range.start
                        <= prev.selected_line_range.end.saturating_add(MERGE_GAP_LINES)
            };
            if can_merge {
                prev.row_range.end = next.row_range.end;
                prev.selected_line_range.end = prev
                    .selected_line_range
                    .end
                    .max(next.selected_line_range.end);
                prev.alternate_line_range.end = prev
                    .alternate_line_range
                    .end
                    .max(next.alternate_line_range.end);
                prev.has_non_emitting_rows |= next.has_non_emitting_rows;
                continue;
            }
        }
        merged.push(next);
    }

    Some(merged)
}

pub(super) fn unresolved_decision_ranges_for_block(
    block: &conflict_resolver::ConflictBlock,
) -> Option<Vec<Range<usize>>> {
    unresolved_decision_regions_for_block(block).map(|regions| {
        regions
            .into_iter()
            .map(|region| region.selected_line_range)
            .collect()
    })
}

pub(super) fn build_resolved_output_conflict_markers(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    output_line_count: usize,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let Some(block_ranges) =
        resolved_output_conflict_block_line_ranges(marker_segments, output_text, block_map)
    else {
        return vec![None; output_line_count];
    };

    build_resolved_output_conflict_markers_from_ranges(
        marker_segments,
        block_ranges.as_slice(),
        output_line_count,
    )
}

pub(super) fn build_resolved_output_conflict_markers_from_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_ranges: &[Range<usize>],
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    for (conflict_ix, (block, range)) in marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .zip(block_ranges.iter().cloned())
        .enumerate()
    {
        let marker_ranges = conflict_marker_ranges_for_block(block, range);
        write_conflict_markers_for_ranges(
            &mut markers,
            conflict_ix,
            !block.resolved,
            marker_ranges.as_slice(),
        );
    }

    markers
}

pub(super) fn build_resolved_output_conflict_markers_from_block_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_ranges: &[Range<usize>],
    output_line_count: usize,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let mut markers = vec![None; output_line_count];
    if output_line_count == 0 {
        return markers;
    }

    for (conflict_ix, (block, range)) in marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .zip(block_ranges.iter().cloned())
        .enumerate()
    {
        write_conflict_markers_for_ranges(
            &mut markers,
            conflict_ix,
            !block.resolved,
            std::slice::from_ref(&range),
        );
    }

    markers
}

pub(super) fn push_conflict_text_segment(
    segments: &mut Vec<conflict_resolver::ConflictSegment>,
    text: impl Into<conflict_resolver::ConflictText>,
) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    if let Some(conflict_resolver::ConflictSegment::Text(prev)) = segments.last_mut() {
        prev.push_str(text.as_str());
        return;
    }
    segments.push(conflict_resolver::ConflictSegment::Text(text));
}

pub(super) fn resolved_output_markers_for_text(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Vec<Option<ResolvedOutputConflictMarker>> {
    let output_line_count = output_text.row_count();
    build_resolved_output_conflict_markers(
        marker_segments,
        output_text,
        output_line_count,
        block_map,
    )
}

/// Byte ranges whose output rows are still unresolved, and the subset of them
/// owned by `active_conflict`. Derive these from the current segments instead of
/// the asynchronously refreshed outline so syntax styling never briefly wins
/// while outline metadata catches up.
/// Unresolved output rows and the conflict each belongs to.
pub(in crate::view) type UnresolvedRows = Arc<[(Range<usize>, usize)]>;

/// [`UnresolvedRows`] paired with the state they were scanned from.
pub(in crate::view) type CachedUnresolvedRows = (ResolvedOutputKey, UnresolvedRows);

/// What the unresolved rows actually depend on: the buffer *and* which blocks
/// are still open.
///
/// The revision alone is not enough. A pick can leave the output byte-identical
/// — choosing the side already displayed, or resolving a whitespace-only block —
/// so the buffer never bumps its revision while the answer changes. Keying on
/// the revision alone leaves the yellow wash painted on a block the user just
/// resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ResolvedOutputKey {
    pub(in crate::view) revision: ResolvedOutputSourceRevision,
    pub(in crate::view) resolution: u64,
    pub(in crate::view) block_map: u64,
}

impl ResolvedOutputKey {
    pub(in crate::view) fn new(
        snapshot: &TextModelSnapshot,
        marker_segments: &[conflict_resolver::ConflictSegment],
        block_map: &conflict_resolver::ResolvedOutputBlockMap,
    ) -> Self {
        Self {
            revision: ResolvedOutputSourceRevision::from_snapshot(snapshot),
            resolution: resolution_fingerprint(marker_segments),
            block_map: block_map_fingerprint(block_map),
        }
    }
}

/// O(conflicts) digest of the block map's byte ranges.
///
/// The rows fall back to the map for block geometry whenever the strict walk
/// fails — which is exactly once the user has edited the buffer. The map can be
/// rebuilt or reset without the text revision or any block's resolution moving,
/// and rows computed against the old geometry then land on the wrong lines.
fn block_map_fingerprint(block_map: &conflict_resolver::ResolvedOutputBlockMap) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    let ranges = block_map.ranges();
    ranges.len().hash(&mut hasher);
    for range in ranges {
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
    }
    hasher.finish()
}

/// O(conflicts) digest of which blocks are resolved. Not a hash of the text —
/// the revision already covers that.
fn resolution_fingerprint(marker_segments: &[conflict_resolver::ConflictSegment]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    for segment in marker_segments {
        match segment {
            conflict_resolver::ConflictSegment::Block(block) => {
                block.resolved.hash(&mut hasher);
                block.choice.hash(&mut hasher);
            }
            conflict_resolver::ConflictSegment::Text(_) => 0u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

/// Every still-unresolved output row, tagged with the conflict it belongs to.
///
/// Depends only on the *text*. Selecting a different conflict does not change
/// it, which is what lets the caller cache it across navigation.
///
/// Takes the rope rather than a materialized document plus a line-start array:
/// the rows wanted are the marker rows of unresolved blocks, and the rope
/// answers "byte range of row N" in O(log n), so this never has to build an
/// index proportional to the document.
pub(super) fn resolved_output_unresolved_rows(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &crate::kit::rope::Rope,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> UnresolvedRows {
    if !marker_segments.iter().any(|segment| {
        matches!(segment, conflict_resolver::ConflictSegment::Block(block) if !block.resolved)
    }) {
        return Arc::default();
    }
    let Some(block_ranges) =
        resolved_output_conflict_block_line_ranges(marker_segments, output_text, block_map)
    else {
        return Arc::default();
    };

    // Walk the unresolved blocks rather than building a marker entry for every
    // row and filtering it. The per-line array is proportional to the document;
    // this is proportional to the conflicts, which is what the caller actually
    // asked about.
    let mut rows = Vec::new();
    for (conflict_ix, (block, line_range)) in marker_segments
        .iter()
        .filter_map(|segment| match segment {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            conflict_resolver::ConflictSegment::Text(_) => None,
        })
        .zip(block_ranges.iter().cloned())
        .enumerate()
    {
        if block.resolved {
            continue;
        }
        for marker_range in conflict_marker_ranges_for_block(block, line_range) {
            for line_ix in marker_range.start..marker_range.end {
                let Ok(row) = u32::try_from(line_ix) else {
                    continue;
                };
                if row >= output_text.line_count() {
                    continue;
                }
                let mut range = output_text.line_range(row);
                while range.end > range.start
                    && conflict_resolver::ResolvedOutputSource::byte_at(output_text, range.end - 1)
                        == Some(b'\r')
                {
                    range.end -= 1;
                }
                if !range.is_empty() {
                    rows.push((range, conflict_ix));
                }
            }
        }
    }
    rows.sort_by_key(|(range, _)| (range.start, range.end));
    rows.into()
}

/// Split cached rows into "every unresolved row" and "the selected conflict's".
///
/// O(unresolved rows), so moving the wash between conflicts costs nothing that
/// scales with the document.
pub(super) fn resolved_output_unresolved_spans_for_active(
    rows: &[(Range<usize>, usize)],
    active_conflict: Option<usize>,
) -> ResolvedOutputUnresolvedSpans {
    let mut all = Vec::with_capacity(rows.len());
    let mut active = Vec::new();
    for (range, conflict_ix) in rows {
        if active_conflict == Some(*conflict_ix) {
            active.push(range.clone());
        }
        all.push(range.clone());
    }
    ResolvedOutputUnresolvedSpans {
        all: all.into(),
        active: active.into(),
    }
}

/// Scan and select in one call. Production splits the two so navigation can
/// reuse the scan; this stays for tests that only care about the result.
#[cfg(test)]
pub(super) fn resolved_output_unresolved_byte_ranges(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
    active_conflict: Option<usize>,
) -> ResolvedOutputUnresolvedSpans {
    let rope = crate::kit::rope::Rope::from_str(output_text);
    let rows = resolved_output_unresolved_rows(marker_segments, &rope, block_map);
    resolved_output_unresolved_spans_for_active(rows.as_ref(), active_conflict)
}

/// Byte spans of the unresolved-conflict placeholder rows, terminator included.
///
/// A `<Merge Conflict>` row is a drawing of an open decision, not text the file
/// will ever contain, so the buffer refuses to edit these spans however the
/// rest of the output has been rewritten by hand. Rows are identified by their
/// own content, which keeps the protection standing even once the marker
/// segments no longer line up with the buffer.
pub(super) fn resolved_output_placeholder_protected_ranges(
    output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
) -> Arc<[Range<usize>]> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    output_text.for_each_row_with_terminator(&mut |range, line| {
        if conflict_resolver::line_is_unresolved_conflict_placeholder(line) {
            ranges.push(range);
        }
    });
    ranges.into()
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
pub(super) fn resolved_output_live_syntax_mask(
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
pub(super) fn resolved_output_live_provider_binding_key(
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
pub(super) fn resolved_output_live_highlight_provider(
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
pub(super) fn coalesce_resolved_output_edit_deltas(
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

pub(super) fn resolved_output_marker_for_line(
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    output_line_ix: usize,
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
) -> Option<ResolvedOutputConflictMarker> {
    resolved_output_markers_for_text(marker_segments, output_text, block_map)
        .get(output_line_ix)
        .copied()
        .flatten()
}

pub(super) fn first_output_marker_line_for_conflict(
    markers: &[Option<ResolvedOutputConflictMarker>],
    conflict_ix: usize,
) -> Option<usize> {
    markers.iter().enumerate().find_map(|(line_ix, marker)| {
        marker
            .as_ref()
            .and_then(|m| (m.conflict_ix == conflict_ix && m.is_start).then_some(line_ix))
    })
}

#[cfg(test)]
pub(super) fn conflict_marker_nav_entries_from_markers(
    markers: &[Option<ResolvedOutputConflictMarker>],
) -> Vec<usize> {
    let mut seen_conflicts = FxHashSet::default();
    markers
        .iter()
        .enumerate()
        .filter_map(|(line_ix, marker)| {
            marker.as_ref().and_then(|m| {
                (m.is_start && seen_conflicts.insert(m.conflict_ix)).then_some(line_ix)
            })
        })
        .collect()
}

pub(super) fn line_index_for_offset(content: &str, offset: usize) -> usize {
    content[..offset.min(content.len())].matches('\n').count()
}

pub(super) fn conflict_resolver_output_context_line(
    content: &str,
    cursor_offset: usize,
    clicked_offset: Option<usize>,
) -> usize {
    clicked_offset
        .map(|offset| line_index_for_offset(content, offset))
        .unwrap_or_else(|| line_index_for_offset(content, cursor_offset))
}

pub(super) fn slice_text_by_line_range(text: &str, line_range: Range<usize>) -> String {
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

pub(super) fn split_target_conflict_block_into_subchunks(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    target_conflict_ix: usize,
) -> bool {
    use gitcomet_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

    let Some(target_block) = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .nth(target_conflict_ix)
        .cloned()
    else {
        return false;
    };
    if target_block.resolved {
        return false;
    }

    enum SplitMode {
        Subchunks(Vec<Subchunk>),
        DecisionRanges {
            regions: Vec<UnresolvedDecisionRegion>,
            choice_is_ours: bool,
        },
    }
    let split_mode = if let Some(base) = target_block.base.as_deref() {
        split_conflict_into_subchunks(base, &target_block.ours, &target_block.theirs).and_then(
            |subchunks| {
                let split_conflict_count = subchunks
                    .iter()
                    .filter(|subchunk| matches!(subchunk, Subchunk::Conflict { .. }))
                    .count();
                (split_conflict_count > 1).then_some(SplitMode::Subchunks(subchunks))
            },
        )
    } else {
        None
    }
    .or_else(|| {
        let (analysis_block, choice_is_ours) =
            if target_block.choice == conflict_resolver::ConflictChoice::Ours {
                (target_block.clone(), true)
            } else if target_block.choice == conflict_resolver::ConflictChoice::Theirs {
                (target_block.clone(), false)
            } else if target_block.choice.is_empty() {
                let mut analysis_block = target_block.clone();
                analysis_block.choice = conflict_resolver::ConflictChoice::Ours;
                (analysis_block, true)
            } else {
                return None;
            };
        unresolved_decision_regions_for_block(&analysis_block).and_then(|regions| {
            (regions.len() > 1).then_some(SplitMode::DecisionRanges {
                regions,
                choice_is_ours,
            })
        })
    });
    let Some(split_mode) = split_mode else {
        return false;
    };

    let mut next_segments = Vec::with_capacity(marker_segments.len().saturating_add(4));
    let mut next_region_indices =
        Vec::with_capacity(conflict_region_indices.len().saturating_add(4));
    let mut seen_conflict_ix = 0usize;
    for seg in marker_segments.drain(..) {
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                let region_ix = conflict_region_indices
                    .get(seen_conflict_ix)
                    .copied()
                    .unwrap_or(seen_conflict_ix);
                if seen_conflict_ix == target_conflict_ix {
                    match &split_mode {
                        SplitMode::Subchunks(subchunks) => {
                            for subchunk in subchunks {
                                match subchunk {
                                    Subchunk::Resolved(text) => {
                                        push_conflict_text_segment(
                                            &mut next_segments,
                                            text.clone(),
                                        );
                                    }
                                    Subchunk::Conflict { base, ours, theirs } => {
                                        next_segments.push(
                                            conflict_resolver::ConflictSegment::Block(
                                                conflict_resolver::ConflictBlock {
                                                    base: Some(base.clone().into()),
                                                    ours: ours.clone().into(),
                                                    theirs: theirs.clone().into(),
                                                    choice: target_block.choice,
                                                    resolved: false,
                                                    // Subchunks of a whitespace-only
                                                    // block are whitespace-only too.
                                                    whitespace_only: target_block.whitespace_only,
                                                },
                                            ),
                                        );
                                        next_region_indices.push(region_ix);
                                    }
                                }
                            }
                        }
                        SplitMode::DecisionRanges {
                            regions,
                            choice_is_ours,
                        } => {
                            let (selected_text, alternate_text) = if *choice_is_ours {
                                (&target_block.ours, &target_block.theirs)
                            } else {
                                (&target_block.theirs, &target_block.ours)
                            };
                            let selected_total_lines = source_line_count(selected_text);
                            let mut selected_cursor = 0usize;
                            for region in regions {
                                let prefix = slice_text_by_line_range(
                                    selected_text,
                                    selected_cursor..region.selected_line_range.start,
                                );
                                push_conflict_text_segment(&mut next_segments, prefix);

                                let selected_fragment = slice_text_by_line_range(
                                    selected_text,
                                    region.selected_line_range.clone(),
                                );
                                let alternate_fragment = slice_text_by_line_range(
                                    alternate_text,
                                    region.alternate_line_range.clone(),
                                );
                                let (ours, theirs) = if *choice_is_ours {
                                    (selected_fragment, alternate_fragment)
                                } else {
                                    (alternate_fragment, selected_fragment)
                                };
                                next_segments.push(conflict_resolver::ConflictSegment::Block(
                                    conflict_resolver::ConflictBlock {
                                        base: None,
                                        ours: ours.into(),
                                        theirs: theirs.into(),
                                        choice: target_block.choice,
                                        resolved: false,
                                        whitespace_only: target_block.whitespace_only,
                                    },
                                ));
                                next_region_indices.push(region_ix);
                                selected_cursor = region.selected_line_range.end;
                            }
                            let suffix = slice_text_by_line_range(
                                selected_text,
                                selected_cursor..selected_total_lines,
                            );
                            push_conflict_text_segment(&mut next_segments, suffix);
                        }
                    }
                } else {
                    next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                    next_region_indices.push(region_ix);
                }
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }

    *marker_segments = next_segments;
    *conflict_region_indices = next_region_indices;
    true
}

pub(super) fn conflict_region_index_is_unique(
    conflict_region_indices: &[usize],
    region_ix: usize,
) -> bool {
    conflict_region_indices
        .iter()
        .filter(|&&ix| ix == region_ix)
        .take(2)
        .count()
        <= 1
}

pub(super) fn conflict_block_matches_group(
    block: &conflict_resolver::ConflictBlock,
    region_ix: usize,
    target_block: &conflict_resolver::ConflictBlock,
    target_region_ix: usize,
) -> bool {
    region_ix == target_region_ix
        && block.base == target_block.base
        && block.ours == target_block.ours
        && block.theirs == target_block.theirs
}

pub(super) fn conflict_group_member_indices_for_ix(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> Vec<usize> {
    let mut blocks: Vec<&conflict_resolver::ConflictBlock> = Vec::new();
    // True when a block has non-empty text between it and the previous block.
    let mut separated_before: Vec<bool> = Vec::new();
    let mut saw_text_since_prev_block = false;
    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                if !text.is_empty() {
                    saw_text_since_prev_block = true;
                }
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                separated_before.push(saw_text_since_prev_block);
                blocks.push(block);
                saw_text_since_prev_block = false;
            }
        }
    }
    let Some(target_block) = blocks.get(conflict_ix).copied() else {
        return Vec::new();
    };
    let target_region_ix = conflict_region_indices
        .get(conflict_ix)
        .copied()
        .unwrap_or(conflict_ix);

    let mut start = conflict_ix;
    while start > 0 {
        if separated_before[start] {
            break;
        }
        let prev_ix = start - 1;
        let prev_block = blocks[prev_ix];
        let prev_region_ix = conflict_region_indices
            .get(prev_ix)
            .copied()
            .unwrap_or(prev_ix);
        if conflict_block_matches_group(prev_block, prev_region_ix, target_block, target_region_ix)
        {
            start = prev_ix;
        } else {
            break;
        }
    }

    let mut end_exclusive = conflict_ix + 1;
    while end_exclusive < blocks.len() {
        let next_ix = end_exclusive;
        if separated_before[next_ix] {
            break;
        }
        let next_block = blocks[next_ix];
        let next_region_ix = conflict_region_indices
            .get(next_ix)
            .copied()
            .unwrap_or(next_ix);
        if conflict_block_matches_group(next_block, next_region_ix, target_block, target_region_ix)
        {
            end_exclusive += 1;
        } else {
            break;
        }
    }

    (start..end_exclusive).collect()
}

pub(super) fn conflict_group_selected_choices_for_ix(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> Vec<conflict_resolver::ConflictChoice> {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    if group_indices.is_empty() {
        return Vec::new();
    }
    let blocks: Vec<&conflict_resolver::ConflictBlock> = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .collect();

    let mut has_base = false;
    let mut has_ours = false;
    let mut has_theirs = false;
    for ix in group_indices {
        let Some(block) = blocks.get(ix).copied() else {
            continue;
        };
        if !block.resolved {
            continue;
        }
        use gitcomet_core::conflict_output::ConflictOutputSource;
        has_base |= block.choice.contains(ConflictOutputSource::Base);
        has_ours |= block.choice.contains(ConflictOutputSource::Ours);
        has_theirs |= block.choice.contains(ConflictOutputSource::Theirs);
    }

    let mut selected = Vec::with_capacity(3);
    if has_base {
        selected.push(conflict_resolver::ConflictChoice::Base);
    }
    if has_ours {
        selected.push(conflict_resolver::ConflictChoice::Ours);
    }
    if has_theirs {
        selected.push(conflict_resolver::ConflictChoice::Theirs);
    }
    selected
}

pub(super) fn conflict_group_indices_for_choice(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> Vec<usize> {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    if group_indices.is_empty() {
        return Vec::new();
    }
    let blocks: Vec<&conflict_resolver::ConflictBlock> = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .collect();

    group_indices
        .into_iter()
        .filter(|&ix| {
            let Some(block) = blocks.get(ix).copied() else {
                return false;
            };
            if !block.resolved {
                return false;
            }
            match choice {
                conflict_resolver::ConflictChoice::Base => block
                    .choice
                    .contains(gitcomet_core::conflict_output::ConflictOutputSource::Base),
                conflict_resolver::ConflictChoice::Ours => block
                    .choice
                    .contains(gitcomet_core::conflict_output::ConflictOutputSource::Ours),
                conflict_resolver::ConflictChoice::Theirs => block
                    .choice
                    .contains(gitcomet_core::conflict_output::ConflictOutputSource::Theirs),
                conflict_resolver::ConflictChoice::Both => {
                    block.choice == conflict_resolver::ConflictChoice::Both
                }
                _ => block.choice == choice,
            }
        })
        .collect()
}

pub(super) fn should_remove_conflict_block_on_reset(
    marker_segments: &[conflict_resolver::ConflictSegment],
    conflict_region_indices: &[usize],
    conflict_ix: usize,
) -> bool {
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    group_indices.len() > 1
}

pub(super) fn remove_conflict_block_at(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
) -> bool {
    let mut next_segments = Vec::with_capacity(marker_segments.len());
    let mut seen_conflict_ix = 0usize;
    let mut removed = false;
    for seg in marker_segments.drain(..) {
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                if seen_conflict_ix == conflict_ix {
                    removed = true;
                } else {
                    next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                }
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }
    *marker_segments = next_segments;
    if removed && conflict_ix < conflict_region_indices.len() {
        conflict_region_indices.remove(conflict_ix);
    }
    removed
}

pub(super) fn reset_conflict_block_selection(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
) -> bool {
    if should_remove_conflict_block_on_reset(marker_segments, conflict_region_indices, conflict_ix)
    {
        return remove_conflict_block_at(marker_segments, conflict_region_indices, conflict_ix);
    }

    let mut seen_conflict_ix = 0usize;
    for seg in marker_segments.iter_mut() {
        let conflict_resolver::ConflictSegment::Block(block) = seg else {
            continue;
        };
        if seen_conflict_ix == conflict_ix {
            if !block.resolved {
                return false;
            }
            block.resolved = false;
            // A genuinely unpicked block has no implicit source. The output
            // projection renders its dedicated merge-conflict placeholder.
            block.choice = conflict_resolver::ConflictChoice::empty();
            return true;
        }
        seen_conflict_ix = seen_conflict_ix.saturating_add(1);
    }
    false
}

pub(super) fn append_choice_after_conflict_block(
    marker_segments: &mut Vec<conflict_resolver::ConflictSegment>,
    conflict_region_indices: &mut Vec<usize>,
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> Option<usize> {
    let target_block = marker_segments
        .iter()
        .filter_map(|seg| match seg {
            conflict_resolver::ConflictSegment::Block(block) => Some(block),
            _ => None,
        })
        .nth(conflict_ix)?
        .clone();
    let group_indices =
        conflict_group_member_indices_for_ix(marker_segments, conflict_region_indices, conflict_ix);
    let &group_end_ix = group_indices.last()?;
    let target_region_ix = conflict_region_indices
        .get(conflict_ix)
        .copied()
        .unwrap_or(conflict_ix);
    if !target_block.resolved {
        return None;
    }
    if matches!(choice, conflict_resolver::ConflictChoice::Base) && target_block.base.is_none() {
        return None;
    }
    if conflict_group_selected_choices_for_ix(marker_segments, conflict_region_indices, conflict_ix)
        .contains(&choice)
    {
        return None;
    }

    let mut next_segments = Vec::with_capacity(marker_segments.len().saturating_add(1));
    let mut next_region_indices =
        Vec::with_capacity(conflict_region_indices.len().saturating_add(1));
    let mut seen_conflict_ix = 0usize;
    let mut next_conflict_ix = 0usize;
    let mut inserted_conflict_ix = None;

    let push_appended = |next_segments: &mut Vec<conflict_resolver::ConflictSegment>,
                         next_region_indices: &mut Vec<usize>,
                         next_conflict_ix: &mut usize,
                         inserted_conflict_ix: &mut Option<usize>| {
        if inserted_conflict_ix.is_some() {
            return;
        }
        let mut appended = target_block.clone();
        appended.choice = choice;
        appended.resolved = true;
        next_segments.push(conflict_resolver::ConflictSegment::Block(appended));
        next_region_indices.push(target_region_ix);
        *inserted_conflict_ix = Some(*next_conflict_ix);
        *next_conflict_ix = next_conflict_ix.saturating_add(1);
    };

    for seg in marker_segments.drain(..) {
        if seen_conflict_ix == group_end_ix.saturating_add(1) {
            push_appended(
                &mut next_segments,
                &mut next_region_indices,
                &mut next_conflict_ix,
                &mut inserted_conflict_ix,
            );
        }
        match seg {
            conflict_resolver::ConflictSegment::Block(block) => {
                let region_ix = conflict_region_indices
                    .get(seen_conflict_ix)
                    .copied()
                    .unwrap_or(seen_conflict_ix);
                next_segments.push(conflict_resolver::ConflictSegment::Block(block));
                next_region_indices.push(region_ix);
                next_conflict_ix = next_conflict_ix.saturating_add(1);
                seen_conflict_ix = seen_conflict_ix.saturating_add(1);
            }
            conflict_resolver::ConflictSegment::Text(text) => {
                push_conflict_text_segment(&mut next_segments, text);
            }
        }
    }
    push_appended(
        &mut next_segments,
        &mut next_region_indices,
        &mut next_conflict_ix,
        &mut inserted_conflict_ix,
    );

    *marker_segments = next_segments;
    *conflict_region_indices = next_region_indices;
    inserted_conflict_ix
}

#[cfg(test)]
pub(super) fn apply_three_way_empty_base_provenance_hints(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
) {
    let generated = conflict_resolver::generate_resolved_text(marker_segments);
    if generated != output_text || meta.is_empty() {
        return;
    }

    let mut block_ix = 0usize;
    let mut a_line = 1u32;
    let mut b_line = 1u32;
    let mut c_line = 1u32;

    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let n = u32::try_from(source_line_count(text)).unwrap_or(0);
                a_line = a_line.saturating_add(n);
                b_line = b_line.saturating_add(n);
                c_line = c_line.saturating_add(n);
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let a_count =
                    u32::try_from(source_line_count(block.base.as_deref().unwrap_or_default()))
                        .unwrap_or(0);
                let b_count = u32::try_from(source_line_count(&block.ours)).unwrap_or(0);
                let c_count = u32::try_from(source_line_count(&block.theirs)).unwrap_or(0);

                let base_empty = block.base.as_ref().is_none_or(|s| s.is_empty());
                if base_empty
                    && let Some(range) = output_line_range_for_conflict_block_in_text(
                        marker_segments,
                        output_text,
                        block_ix,
                    )
                {
                    let mut output_offset = 0usize;
                    for source in block.choice.iter() {
                        let (source_count, resolved_source, input_line) = match source {
                            gitcomet_core::conflict_output::ConflictOutputSource::Base => {
                                (a_count, conflict_resolver::ResolvedLineSource::A, a_line)
                            }
                            gitcomet_core::conflict_output::ConflictOutputSource::Ours => {
                                (b_count, conflict_resolver::ResolvedLineSource::B, b_line)
                            }
                            gitcomet_core::conflict_output::ConflictOutputSource::Theirs => {
                                (c_count, conflict_resolver::ResolvedLineSource::C, c_line)
                            }
                        };
                        let remaining = range
                            .end
                            .saturating_sub(range.start.saturating_add(output_offset));
                        let take =
                            usize::min(remaining, usize::try_from(source_count).unwrap_or(0));
                        for off in 0..take {
                            if let Some(m) = meta.get_mut(range.start + output_offset + off)
                                && matches!(
                                    m.source,
                                    conflict_resolver::ResolvedLineSource::A
                                        | conflict_resolver::ResolvedLineSource::Manual
                                )
                            {
                                m.source = resolved_source;
                                m.input_line = Some(
                                    input_line.saturating_add(u32::try_from(off).unwrap_or(0)),
                                );
                            }
                        }
                        output_offset = output_offset.saturating_add(take);
                    }
                }

                a_line = a_line.saturating_add(a_count);
                b_line = b_line.saturating_add(b_count);
                c_line = c_line.saturating_add(c_count);
                block_ix = block_ix.saturating_add(1);
            }
        }
    }
}

pub(super) fn apply_conflict_choice_provenance_hints_for_ranges(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    block_ranges: &[Range<usize>],
    view_mode: ConflictResolverViewMode,
) {
    if meta.is_empty() {
        return;
    }

    let assign_range = |meta: &mut [conflict_resolver::ResolvedLineMeta],
                        range: Range<usize>,
                        source: conflict_resolver::ResolvedLineSource,
                        start_line: u32,
                        line_count: u32| {
        let len = range.end.saturating_sub(range.start);
        for off in 0..len {
            if let Some(m) = meta.get_mut(range.start + off) {
                m.source = source;
                let off_u32 = u32::try_from(off).unwrap_or(u32::MAX);
                m.input_line = (off_u32 < line_count).then_some(start_line.saturating_add(off_u32));
            }
        }
    };

    let assign_both_range = |meta: &mut [conflict_resolver::ResolvedLineMeta],
                             range: Range<usize>,
                             first_source: conflict_resolver::ResolvedLineSource,
                             first_start: u32,
                             first_count: u32,
                             second_source: conflict_resolver::ResolvedLineSource,
                             second_start: u32,
                             second_count: u32| {
        let len = range.end.saturating_sub(range.start);
        let first_count_usize = usize::try_from(first_count).unwrap_or(0);
        let first_take = len.min(first_count_usize);
        assign_range(
            meta,
            range.start..range.start.saturating_add(first_take),
            first_source,
            first_start,
            first_count,
        );
        assign_range(
            meta,
            range.start.saturating_add(first_take)..range.end,
            second_source,
            second_start,
            second_count,
        );
    };

    let mut block_ix = 0usize;
    let mut a_line = 1u32;
    let mut b_line = 1u32;
    let mut c_line = 1u32;

    for seg in marker_segments {
        match seg {
            conflict_resolver::ConflictSegment::Text(text) => {
                let n = u32::try_from(source_line_count(text)).unwrap_or(0);
                a_line = a_line.saturating_add(n);
                b_line = b_line.saturating_add(n);
                if view_mode == ConflictResolverViewMode::ThreeWay {
                    c_line = c_line.saturating_add(n);
                }
            }
            conflict_resolver::ConflictSegment::Block(block) => {
                let (a_count, b_count, c_count) = match view_mode {
                    ConflictResolverViewMode::ThreeWay => (
                        u32::try_from(source_line_count(block.base.as_deref().unwrap_or_default()))
                            .unwrap_or(0),
                        u32::try_from(source_line_count(&block.ours)).unwrap_or(0),
                        u32::try_from(source_line_count(&block.theirs)).unwrap_or(0),
                    ),
                    ConflictResolverViewMode::TwoWayDiff => (
                        u32::try_from(source_line_count(&block.ours)).unwrap_or(0),
                        u32::try_from(source_line_count(&block.theirs)).unwrap_or(0),
                        0,
                    ),
                };

                if let Some(range) = block_ranges.get(block_ix).cloned() {
                    if !block.resolved && block.choice.is_empty() {
                        assign_range(
                            meta,
                            range,
                            conflict_resolver::ResolvedLineSource::Manual,
                            0,
                            0,
                        );
                    } else {
                        match (view_mode, block.choice) {
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Base,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::A,
                                    a_line,
                                    a_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Ours,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Theirs,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::C,
                                    c_line,
                                    c_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::ThreeWay,
                                conflict_resolver::ConflictChoice::Both,
                            ) => {
                                assign_both_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                    conflict_resolver::ResolvedLineSource::C,
                                    c_line,
                                    c_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Theirs,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                );
                            }
                            (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Both,
                            ) => {
                                assign_both_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::A,
                                    a_line,
                                    a_count,
                                    conflict_resolver::ResolvedLineSource::B,
                                    b_line,
                                    b_count,
                                );
                            }
                            // In two-way mode, Base falls back to local-side semantics.
                            (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Base,
                            )
                            | (
                                ConflictResolverViewMode::TwoWayDiff,
                                conflict_resolver::ConflictChoice::Ours,
                            ) => {
                                assign_range(
                                    meta,
                                    range,
                                    conflict_resolver::ResolvedLineSource::A,
                                    a_line,
                                    a_count,
                                );
                            }
                            _ => {
                                // Arbitrary ordered combinations are rendered
                                // correctly; this compact hint table treats their
                                // mixed provenance as manual.
                            }
                        }
                    }
                }

                a_line = a_line.saturating_add(a_count);
                b_line = b_line.saturating_add(b_count);
                c_line = c_line.saturating_add(c_count);
                block_ix = block_ix.saturating_add(1);
            }
        }
    }
}

pub(super) fn apply_conflict_choice_provenance_hints(
    meta: &mut [conflict_resolver::ResolvedLineMeta],
    marker_segments: &[conflict_resolver::ConflictSegment],
    output_text: &str,
    view_mode: ConflictResolverViewMode,
) {
    let generated = conflict_resolver::generate_resolved_text(marker_segments);
    if generated != output_text {
        return;
    }

    let Some(block_ranges) =
        resolved_output_conflict_block_ranges_in_text(marker_segments, output_text)
    else {
        return;
    };

    apply_conflict_choice_provenance_hints_for_ranges(
        meta,
        marker_segments,
        block_ranges.as_slice(),
        view_mode,
    );
}

/// Whether the content pane is showing a file's full content *at the commit the
/// file browser is pinned to* — the state the historical browse tint marks.
///
/// Content-preview mode alone is not enough: a file's content can be opened from
/// some other commit while a browse point is active, and that content is not
/// what the browse point describes. The commit ids have to match.
pub(super) fn historical_browse_content(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ClearDiffSelectionAction {
    ClearSelection,
    ExitFocusedMergetool,
}

pub(super) fn clear_diff_selection_action(view_mode: GitCometViewMode) -> ClearDiffSelectionAction {
    match view_mode {
        GitCometViewMode::Normal => ClearDiffSelectionAction::ClearSelection,
        GitCometViewMode::FocusedMergetool => ClearDiffSelectionAction::ExitFocusedMergetool,
    }
}

pub(super) fn focused_mergetool_save_exit_code(
    total_conflicts: usize,
    resolved_conflicts: usize,
) -> i32 {
    if total_conflicts == 0 || total_conflicts == resolved_conflicts {
        FOCUSED_MERGETOOL_EXIT_SUCCESS
    } else {
        FOCUSED_MERGETOOL_EXIT_CANCELED
    }
}

pub(super) fn conflict_strategy_needs_full_side_payloads(
    strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
) -> bool {
    matches!(
        strategy,
        Some(
            gitcomet_core::conflict_session::ConflictResolverStrategy::BinarySidePick
                | gitcomet_core::conflict_session::ConflictResolverStrategy::TwoWayKeepDelete
                | gitcomet_core::conflict_session::ConflictResolverStrategy::DecisionOnly
        )
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FocusedMergetoolOutput<'a> {
    Write(&'a [u8]),
    Delete,
}

pub(super) fn apply_focused_mergetool_output(
    path: &std::path::Path,
    output: FocusedMergetoolOutput<'_>,
) -> std::io::Result<()> {
    match output {
        FocusedMergetoolOutput::Write(bytes) => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes)
        }
        FocusedMergetoolOutput::Delete => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FocusedMergetoolSavePayload {
    pub(super) output: String,
    pub(super) total_conflicts: usize,
    pub(super) resolved_conflicts: usize,
}

pub(super) fn build_focused_mergetool_save_payload(
    marker_segments: &[ConflictSegment],
    block_region_indices: &[usize],
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
    materialized_output_text: Option<&str>,
    labels: gitcomet_core::conflict_output::ConflictMarkerLabels<'_>,
) -> FocusedMergetoolSavePayload {
    use gitcomet_core::conflict_output::{GenerateResolvedTextOptions, UnresolvedConflictMode};

    let render_preserve_markers = |segments: &[ConflictSegment]| {
        conflict_resolver::generate_resolved_text_with_options(
            segments,
            GenerateResolvedTextOptions {
                unresolved_mode: UnresolvedConflictMode::PreserveMarkers,
                labels: Some(labels),
            },
        )
    };

    if let Some(output_text) = materialized_output_text {
        if let Some(updates) = conflict_resolver::derive_region_resolution_updates_from_output(
            marker_segments,
            block_region_indices,
            block_map,
            output_text,
        ) {
            let mut save_segments = marker_segments.to_vec();
            let ordered_resolutions: Vec<_> = updates
                .into_iter()
                .map(|(_, resolution)| resolution)
                .collect();
            conflict_resolver::apply_ordered_region_resolutions(
                &mut save_segments,
                &ordered_resolutions,
            );
            let mut output = output_text.to_string();
            let blocks: Vec<_> = marker_segments
                .iter()
                .filter_map(|segment| match segment {
                    ConflictSegment::Block(block) => Some(block),
                    ConflictSegment::Text(_) => None,
                })
                .collect();
            for ((block, range), resolution) in blocks
                .into_iter()
                .zip(block_map.ranges())
                .zip(&ordered_resolutions)
                .rev()
            {
                if matches!(
                    resolution,
                    gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
                ) {
                    let marker_text =
                        render_preserve_markers(&[ConflictSegment::Block(block.clone())]);
                    output.replace_range(range.clone(), &marker_text);
                }
            }
            return FocusedMergetoolSavePayload {
                output,
                total_conflicts: conflict_resolver::conflict_count(&save_segments),
                resolved_conflicts: conflict_resolver::resolved_conflict_count(&save_segments),
            };
        }

        let total_conflicts = conflict_resolver::conflict_count(marker_segments);
        return FocusedMergetoolSavePayload {
            output: output_text.to_string(),
            total_conflicts,
            resolved_conflicts: if conflict_resolver::text_contains_conflict_markers(output_text) {
                0
            } else {
                total_conflicts
            },
        };
    }

    FocusedMergetoolSavePayload {
        output: render_preserve_markers(marker_segments),
        total_conflicts: conflict_resolver::conflict_count(marker_segments),
        resolved_conflicts: conflict_resolver::resolved_conflict_count(marker_segments),
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum PreparedSyntaxViewMode {
    FileDiffSplitLeft,
    FileDiffSplitRight,
    WorktreePreview,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct PreparedSyntaxDocumentKey {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) target_rev: u64,
    pub(in crate::view) file_path: std::path::PathBuf,
    pub(in crate::view) view_mode: PreparedSyntaxViewMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) enum CollapsedDiffExpansionKind {
    #[default]
    None,
    Up,
    Down,
    Both,
    Short,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct CollapsedDiffHunk {
    pub(in crate::view) src_ix: usize,
    pub(in crate::view) base_row_start: usize,
    pub(in crate::view) base_row_end_exclusive: usize,
    pub(in crate::view) has_additions: bool,
    pub(in crate::view) has_removals: bool,
    pub(in crate::view) reveal_up_lines: usize,
    pub(in crate::view) reveal_down_lines: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct CollapsedDiffReveal {
    pub(in crate::view) up_lines: usize,
    pub(in crate::view) down_lines: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct CollapsedDiffProjectionIdentity {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) diff_target: DiffTarget,
    pub(in crate::view) file_path: std::path::PathBuf,
    pub(in crate::view) diff_whitespace_mode: DiffWhitespaceMode,
    pub(in crate::view) patch_content_signature: Option<u64>,
    pub(in crate::view) file_content_signature: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum CollapsedDiffVisibleRow {
    HunkHeader {
        src_ix: usize,
        expansion_kind: CollapsedDiffExpansionKind,
        display_src_ix: Option<usize>,
        hidden_rows: usize,
    },
    FileRow {
        row_ix: usize,
    },
}

impl CollapsedDiffVisibleRow {
    pub(in crate::view) const fn row_ix(self) -> Option<usize> {
        match self {
            Self::FileRow { row_ix } => Some(row_ix),
            Self::HunkHeader { .. } => None,
        }
    }

    pub(in crate::view) const fn header_display_src_ix(self) -> Option<usize> {
        match self {
            Self::HunkHeader { display_src_ix, .. } => display_src_ix,
            Self::FileRow { .. } => None,
        }
    }

    pub(in crate::view) const fn header_action_src_ix(self) -> Option<usize> {
        match self {
            Self::HunkHeader { src_ix, .. } => Some(src_ix),
            Self::FileRow { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiffHorizontalScrollColumn {
    Primary,
    SplitRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffWrapVisualRow {
    pub(in crate::view) source_visible_ix: usize,
    pub(in crate::view) wrap_ix: usize,
    pub(in crate::view) primary_range: rows::DiffWrapByteRange,
    pub(in crate::view) secondary_range: rows::DiffWrapByteRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffWrapVisibleCacheKey {
    pub(in crate::view) source_len: usize,
    pub(in crate::view) diff_view: DiffViewMode,
    pub(in crate::view) is_file_view: bool,
    pub(in crate::view) collapsed_projection_active: bool,
    pub(in crate::view) projection_rev: u64,
    pub(in crate::view) diff_cache_rev: u64,
    pub(in crate::view) file_diff_cache_seq: u64,
    pub(in crate::view) inline_columns: usize,
    pub(in crate::view) split_columns: usize,
    /// Columns a file preview row wraps at. The preview is a single column
    /// with its own gutter, so neither of the diff's two widths describes it.
    pub(in crate::view) preview_columns: usize,
    /// Bumped when the previewed file's content changes, so the rows are
    /// rebuilt for the new text rather than kept from the old.
    pub(in crate::view) preview_content_rev: u64,
    pub(in crate::view) reveal_whitespace_chars: bool,
}

impl DiffHorizontalScrollColumn {
    pub(in crate::view) const fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::SplitRight => 1,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::view) struct DiffHorizontalScrollState {
    pub(in crate::view) content_widths: [Pixels; 2],
}

/// Memoized blame author-time range, keyed by a clone of the blame `Arc`. See
/// [`MainPaneView::blame_time_range_cache`].
pub(in crate::view) type BlameTimeRangeCache = Option<(
    std::sync::Arc<Vec<gitcomet_core::services::BlameLine>>,
    Option<(i64, i64)>,
)>;

impl DiffHorizontalScrollState {
    pub(in crate::view) fn new() -> Self {
        Self {
            content_widths: [px(0.0); 2],
        }
    }

    pub(in crate::view) fn reset(&mut self) {
        self.content_widths = [px(0.0); 2];
    }

    pub(in crate::view) fn record_content_width(
        &mut self,
        column: DiffHorizontalScrollColumn,
        width: Pixels,
    ) -> bool {
        let ix = column.index();
        if width > self.content_widths[ix] {
            self.content_widths[ix] = width;
            true
        } else {
            false
        }
    }
}

pub(crate) struct MainPaneView {
    pub(in crate::view) store: Arc<AppStore>,
    pub(super) state: Arc<AppState>,
    pub(in crate::view) view_mode: GitCometViewMode,
    pub(in crate::view) focused_mergetool_labels: Option<FocusedMergetoolLabels>,
    pub(in crate::view) focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) date_time_format: DateTimeFormat,
    pub(super) _ui_model_subscription: gpui::Subscription,
    pub(in crate::view) root_view: WeakEntity<GitCometView>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
    pub(super) notify_fingerprint: u64,
    pub(in crate::view) active_context_menu_invoker: Option<SharedString>,

    pub(in crate::view) last_window_size: Size<Pixels>,
    pub(in crate::view) layout_sidebar_render_width: Pixels,
    pub(in crate::view) layout_details_render_width: Pixels,
    pub(in crate::view) layout_sidebar_collapsed: bool,
    pub(in crate::view) layout_details_collapsed: bool,

    pub(in crate::view) reveal_whitespace_chars: bool,
    /// section 30 merge tool: auto-advance to the next unresolved conflict after a
    /// source pick. Persisted UI setting (cog menu).
    pub(in crate::view) mergetool_auto_advance: bool,
    /// section 30 merge tool: default for the collapse-unchanged-context mode when a
    /// conflicted file opens. Persisted UI setting (cog menu).
    pub(in crate::view) mergetool_collapse_unchanged: bool,
    /// section 30 merge tool: sync the resolved output pane's scroll with the source
    /// columns (in modes where they share a row space). Persisted UI setting
    /// (cog menu). Merge-tool-specific rather than a general diff setting
    /// because the resolver ships as a standalone tool.
    pub(in crate::view) mergetool_output_scroll_sync: bool,
    /// section 30 merge tool: show per-column and resolved-output line number
    /// gutters. Persisted UI setting (cog menu).
    pub(in crate::view) mergetool_show_line_numbers: bool,
    /// section 30 merge tool: last-used view mode (true = 3-way). Fresh opens of
    /// base-present conflicts default to this; toolbar toggle persists it.
    pub(in crate::view) mergetool_view_three_way: bool,
    pub(in crate::view) diff_view: DiffViewMode,
    pub(in crate::view) annotate_enabled: bool,
    /// Width (design px) of the annotate column; user-resizable, session-local.
    pub(in crate::view) annotate_column_width: f32,
    /// Active annotate-column resize drag, if any.
    pub(in crate::view) annotate_resize: Option<AnnotateResizeState>,
    /// Blame annotation sub-area currently hovered (row index + area). Drives the
    /// accent highlight and tooltip for the annotation column on the next paint.
    pub(in crate::view) blame_annot_hover: Option<(usize, crate::view::rows::AnnotArea)>,
    /// Diff row whose stage/unstage gutter button is currently hovered, as the
    /// row index plus which column's gutter it sits in. Drives painting the
    /// button and its tooltip on the next paint; `None` means none is showing.
    pub(in crate::view) diff_stage_gutter_hover: Option<crate::view::rows::DiffStageHover>,
    /// Painted bounds of each row's stage-gutter cell, recorded during paint so
    /// tests can drive the button without duplicating its geometry.
    pub(in crate::view) diff_stage_gutter_cells:
        FxHashMap<(usize, crate::view::rows::DiffStageSlot), gpui::Bounds<Pixels>>,
    /// Memoized `(min, max)` author-time range for the currently loaded blame,
    /// keyed by a clone of the blame `Arc`. The range never changes after load,
    /// so this avoids rescanning all blame lines on every render frame. Holding
    /// the `Arc` (rather than a bare pointer) keeps the allocation alive while
    /// cached, so a reloaded blame can never alias the same address and return a
    /// stale range.
    pub(in crate::view) blame_time_range_cache: BlameTimeRangeCache,
    pub(in crate::view) rendered_preview_modes: RenderedPreviewModes,
    pub(in crate::view) diff_word_wrap: bool,
    pub(in crate::view) diff_show_line_numbers: bool,
    pub(in crate::view) diff_scroll_sync: DiffScrollSync,
    pub(in crate::view) diff_content_mode: DiffContentMode,
    pub(in crate::view) diff_whitespace_mode: DiffWhitespaceMode,
    pub(in crate::view) diff_split_ratio: f32,
    pub(in crate::view) diff_split_resize: Option<DiffSplitResizeState>,
    pub(in crate::view) diff_split_last_synced_x: [Pixels; 2],
    pub(in crate::view) diff_split_last_synced_y: [Pixels; 2],
    pub(in crate::view) diff_horizontal_scroll: DiffHorizontalScrollState,
    pub(in crate::view) diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) diff_cache_rev: u64,
    pub(in crate::view) diff_cache_content_signature: Option<u64>,
    pub(in crate::view) diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) diff_cache: Vec<AnnotatedDiffLine>,
    pub(in crate::view) diff_row_provider: Option<Arc<super::diff_cache::PagedPatchDiffRows>>,
    pub(in crate::view) diff_split_row_provider:
        Option<Arc<super::diff_cache::PagedPatchSplitRows>>,
    pub(in crate::view) diff_file_for_src_ix: Vec<Option<Arc<str>>>,
    pub(in crate::view) diff_language_for_src_ix: Vec<Option<rows::DiffSyntaxLanguage>>,
    pub(in crate::view) diff_yaml_block_scalar_for_src_ix: Vec<bool>,
    pub(in crate::view) diff_click_kinds: Vec<DiffClickKind>,
    pub(in crate::view) diff_line_kind_for_src_ix: Vec<gitcomet_core::domain::DiffLineKind>,
    pub(in crate::view) diff_visual_line_kind_for_src_ix: Vec<gitcomet_core::domain::DiffLineKind>,
    pub(in crate::view) diff_hide_unified_header_for_src_ix: Vec<bool>,
    pub(in crate::view) diff_header_display_cache: FxHashMap<usize, SharedString>,
    pub(in crate::view) diff_split_cache: Vec<PatchSplitRow>,
    pub(in crate::view) diff_split_cache_len: usize,
    pub(in crate::view) diff_panel_focus_handle: FocusHandle,
    pub(in crate::view) diff_autoscroll_pending: bool,
    pub(in crate::view) diff_raw_input: Entity<components::TextInput>,
    pub(in crate::view) submodule_hash_inputs: Vec<Entity<components::TextInput>>,
    pub(in crate::view) diff_visible_indices: Vec<usize>,
    pub(in crate::view) diff_visible_inline_map: Option<super::diff_cache::PatchInlineVisibleMap>,
    pub(in crate::view) diff_wrap_visible_rows: Vec<DiffWrapVisualRow>,
    pub(in crate::view) diff_wrap_visible_cache_key: Option<DiffWrapVisibleCacheKey>,
    pub(in crate::view) collapsed_diff_hunks: Vec<CollapsedDiffHunk>,
    pub(in crate::view) collapsed_diff_hunk_ix_by_src_ix: FxHashMap<usize, usize>,
    pub(in crate::view) collapsed_diff_reveals: FxHashMap<usize, CollapsedDiffReveal>,
    pub(in crate::view) collapsed_diff_visible_rows: Vec<CollapsedDiffVisibleRow>,
    pub(in crate::view) collapsed_diff_hunk_visible_indices: Vec<usize>,
    pub(in crate::view) collapsed_diff_header_display_cache: FxHashMap<usize, SharedString>,
    pub(in crate::view) collapsed_diff_projection_identity: Option<CollapsedDiffProjectionIdentity>,
    pub(in crate::view) diff_visible_cache_len: usize,
    pub(in crate::view) diff_visible_view: DiffViewMode,
    pub(in crate::view) diff_visible_is_file_view: bool,
    pub(in crate::view) diff_visible_projection_rev: u64,
    pub(in crate::view) diff_visible_cache_projection_rev: u64,
    pub(in crate::view) diff_scrollbar_markers_cache: Vec<components::ScrollbarMarker>,
    pub(in crate::view) diff_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in crate::view) diff_word_highlights_inflight: Option<u64>,
    pub(in crate::view) diff_file_stats: Vec<Option<(usize, usize)>>,
    pub(in crate::view) diff_text_segments_cache: Vec<Option<VersionedCachedDiffStyledText>>,
    pub(in crate::view) diff_text_query_segments_cache: Vec<Option<VersionedCachedDiffStyledText>>,
    pub(in crate::view) diff_text_query_cache_query: SharedString,
    pub(in crate::view) diff_text_query_cache_options: super::diff_search::DiffSearchOptions,
    pub(in crate::view) diff_text_query_cache_matcher:
        Option<super::diff_search::DiffSearchMatcher>,
    pub(in crate::view) diff_text_query_cache_generation: u64,
    pub(in crate::view) diff_selection_anchor: Option<usize>,
    pub(in crate::view) diff_selection_range: Option<(usize, usize)>,
    pub(in crate::view) diff_text_selecting: bool,
    pub(in crate::view) diff_text_anchor: Option<DiffTextPos>,
    pub(in crate::view) diff_text_head: Option<DiffTextPos>,
    pub(super) diff_text_autoscroll_seq: u64,
    pub(super) diff_text_autoscroll_target: Option<DiffTextAutoscrollTarget>,
    pub(super) diff_text_last_mouse_pos: Point<Pixels>,
    pub(in crate::view) diff_suppress_clicks_remaining: u8,
    /// The delimiter pair the last click selected, already projected onto rows.
    ///
    /// Not folded into any cache key: unlike the file editor's highlight runs,
    /// this is painted as a quad outside every cached artifact, and `KeyedCanvas`
    /// re-runs prepaint and paint every frame regardless of its revision key.
    pub(in crate::view) diff_text_pair_match: Option<DiffTextPairMatch>,
    /// Every place the clicked name appears, already projected onto rows and
    /// bucketed by the row that paints it.
    ///
    /// Separate from `diff_text_pair_match` because a click produces both: the
    /// name's other uses, and the construct enclosing it. Bucketed because the
    /// paint path asks per row per region per frame, and scanning a flat list of
    /// up to `MAX_OCCURRENCES` for each of them is work proportional to rows
    /// times matches, repeated at frame rate for as long as the highlight is up.
    pub(in crate::view) diff_text_occurrences:
        FxHashMap<(usize, DiffTextRegion), smallvec::SmallVec<[Range<usize>; 4]>>,
    /// A click waiting for a cold full-document syntax parse. The second region
    /// is the real old/new side to prepare (inline rows still belong to one of
    /// those documents). Projection resets clear this before its worker can
    /// replay stale coordinates.
    pub(in crate::view) diff_text_pending_syntax_click: Option<(DiffTextPos, DiffTextRegion)>,
    pub(in crate::view) diff_text_hitboxes: FxHashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    /// A search match whose row still has to be brought into view sideways, and
    /// how many more frames to keep trying for.
    ///
    /// The vertical jump is deferred to the list's own prepaint and the row is
    /// only measurable once it paints at its new position, which is not always
    /// the very next frame — the frame that applies the scroll can still be
    /// painting the rows it was showing before. The budget is what stops a row
    /// that never paints from leaving the request live for good.
    pub(in crate::view) diff_search_horizontal_reveal: Option<(usize, u8)>,
    /// Where the merge tool's column rows painted their text this frame, for the
    /// sideways half of a search reveal. Rebuilt every frame like
    /// [`Self::diff_text_hitboxes`].
    pub(in crate::view) conflict_text_hitboxes:
        FxHashMap<(usize, ThreeWayColumn), crate::view::mod_helpers::ConflictTextHitbox>,
    pub(in crate::view) diff_text_layout_cache_epoch: u64,
    pub(in crate::view) diff_text_layout_cache: FxHashMap<u64, DiffTextLayoutCacheEntry>,
    pub(in crate::view) diff_search_active: bool,
    pub(in crate::view) diff_search_query: SharedString,
    pub(in crate::view) diff_search_options: super::diff_search::DiffSearchOptions,
    pub(in crate::view) diff_search_regex_error: Option<SharedString>,
    pub(in crate::view) diff_search_matches: Vec<usize>,
    pub(in crate::view) diff_search_inline_patch_trigram_index:
        Option<super::diff_search::DiffSearchVisibleTrigramIndex>,
    pub(in crate::view) diff_search_match_ix: Option<usize>,
    pub(in crate::view) diff_search_debounce_seq: u64,
    pub(in crate::view) diff_search_pending_previous_query: Option<SharedString>,
    pub(in crate::view) diff_search_scroll: ScrollHandle,
    pub(in crate::view) diff_search_input: Entity<components::TextInput>,
    pub(super) _diff_search_subscription: gpui::Subscription,

    pub(in crate::view) file_diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_diff_cache_rev: u64,
    pub(in crate::view) file_diff_cache_content_signature: Option<u64>,
    pub(in crate::view) file_diff_cache_whitespace_mode: DiffWhitespaceMode,
    pub(in crate::view) file_diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_diff_cache_error: Option<String>,
    pub(in crate::view) file_diff_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_diff_cache_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) file_diff_cache_rows: Vec<FileDiffRow>,
    pub(in crate::view) file_diff_row_provider: Option<Arc<super::diff_cache::PagedFileDiffRows>>,
    /// Text read back from a source-backed side for a click, kept alive.
    ///
    /// Not just a cache: the prepared-document identity is keyed partly on the
    /// text's *address*, so handing it a `SharedString` that is dropped when the
    /// call returns leaves an identity pointing at freed memory, which a later
    /// allocation of the same length can alias. Retaining it also means a second
    /// click resolves by identity instead of re-reading and re-parsing the file.
    ///
    /// The read happens inline only for small files and otherwise on the click
    /// syntax worker. Cleared whenever the cache rebuilds, so what it holds is
    /// always the body the current generation's line index was built from.
    pub(in crate::view) file_diff_pair_syntax_text: FxHashMap<DiffTextRegion, SharedString>,
    /// Source sides currently being read and parsed for an interactive click.
    /// Prevents repeated clicks from launching duplicate full-document work.
    pub(in crate::view) file_diff_click_syntax_inflight: FxHashSet<DiffTextRegion>,
    /// Where each side's content lives when it is a file rather than text in
    /// memory. A source-backed side keeps its text off the heap so a huge diff
    /// can render from per-line slices; the click path reads it back from here
    /// when it needs a whole-document parse.
    pub(in crate::view) file_diff_old_source_path: Option<Arc<std::path::PathBuf>>,
    pub(in crate::view) file_diff_new_source_path: Option<Arc<std::path::PathBuf>>,
    /// Real old-side file text used for split and inline syntax projection.
    /// Empty when the side is source-backed; `file_diff_old_source_path` says so.
    pub(in crate::view) file_diff_old_text: SharedString,
    pub(in crate::view) file_diff_old_line_starts: Arc<[usize]>,
    pub(in crate::view) file_diff_old_line_to_row: Arc<[Option<usize>]>,
    pub(in crate::view) file_diff_old_line_to_inline_row: Arc<[Option<usize>]>,
    /// Real new-side file text used for split and inline syntax projection.
    /// Empty when the side is source-backed; `file_diff_new_source_path` says so.
    pub(in crate::view) file_diff_new_text: SharedString,
    pub(in crate::view) file_diff_new_line_starts: Arc<[usize]>,
    pub(in crate::view) file_diff_new_line_to_row: Arc<[Option<usize>]>,
    pub(in crate::view) file_diff_new_line_to_inline_row: Arc<[Option<usize>]>,
    pub(in crate::view) file_diff_inline_cache: Vec<AnnotatedDiffLine>,
    pub(in crate::view) file_diff_inline_row_provider:
        Option<Arc<super::diff_cache::PagedFileDiffInlineRows>>,
    pub(in crate::view) file_diff_inline_text: SharedString,
    pub(in crate::view) file_diff_inline_word_highlights: rows::LruCache<usize, Vec<Range<usize>>>,
    pub(in crate::view) file_diff_split_word_highlights:
        rows::LruCache<usize, FileDiffSplitWordHighlights>,
    pub(in crate::view) file_diff_cache_seq: u64,
    pub(in crate::view) file_diff_cache_inflight: Option<u64>,
    pub(in crate::view) file_diff_syntax_generation: u64,
    pub(in crate::view) file_diff_style_cache_epochs: FileDiffStyleCacheEpochs,
    pub(in crate::view) syntax_chunk_poll_task: Option<gpui::Task<()>>,
    pub(in crate::view) prepared_syntax_documents:
        FxHashMap<PreparedSyntaxDocumentKey, rows::PreparedDiffSyntaxDocument>,
    #[cfg(test)]
    pub(in crate::view) diff_syntax_budget_override: Option<rows::DiffSyntaxBudget>,

    pub(in crate::view) file_markdown_preview_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_markdown_preview_cache_rev: u64,
    pub(in crate::view) file_markdown_preview_cache_content_signature: Option<u64>,
    pub(in crate::view) file_markdown_preview_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_markdown_preview: LoadableMarkdownDiff,
    pub(in crate::view) file_markdown_preview_seq: u64,
    pub(in crate::view) file_markdown_preview_inflight: Option<u64>,
    pub(in crate::view) markdown_preview_wrap: MarkdownPreviewWrapCache,
    /// Row the quick-search cursor wants revealed in the flowing markdown
    /// preview, shared with the renderer that measures it. See
    /// [`rows::MarkdownPreviewRevealRequest`].
    pub(in crate::view) markdown_preview_reveal: rows::MarkdownPreviewRevealRequest,

    pub(in crate::view) file_image_diff_cache_repo_id: Option<RepoId>,
    pub(in crate::view) file_image_diff_cache_rev: u64,
    pub(in crate::view) file_image_diff_cache_content_signature: Option<u64>,
    pub(in crate::view) file_image_diff_cache_target: Option<DiffTarget>,
    pub(in crate::view) file_image_diff_cache_seq: u64,
    pub(in crate::view) file_image_diff_cache_inflight: Option<u64>,
    pub(in crate::view) file_image_diff_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_diff_cache_old: Option<Arc<gpui::RenderImage>>,
    pub(in crate::view) file_image_diff_cache_new: Option<Arc<gpui::RenderImage>>,
    pub(in crate::view) file_image_diff_cache_old_svg_path: Option<std::path::PathBuf>,
    pub(in crate::view) file_image_diff_cache_new_svg_path: Option<std::path::PathBuf>,

    pub(in crate::view) worktree_preview_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview_source_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview: Loadable<usize>,
    pub(in crate::view) worktree_preview_source_len: usize,
    pub(in crate::view) worktree_preview_text: SharedString,
    pub(in crate::view) worktree_preview_line_starts: Arc<[usize]>,
    pub(in crate::view) worktree_preview_line_flags: Arc<[u8]>,
    pub(in crate::view) worktree_preview_search_trigram_index:
        Option<super::diff_search::DiffSearchVisibleTrigramIndex>,
    pub(in crate::view) worktree_preview_content_rev: u64,
    pub(in crate::view) worktree_markdown_preview_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_markdown_preview_source_rev: u64,
    pub(in crate::view) worktree_markdown_preview: LoadableMarkdownDoc,
    /// Sizes read from the headers of the pictures the rendered preview draws,
    /// so a picture that has not decoded yet can still hold its box open.
    pub(in crate::view) worktree_markdown_preview_picture_sizes: rows::MarkdownPreviewPictureSizes,
    /// Where each sideways-scrolling block of the rendered preview is scrolled
    /// to, so its scrollbar has something to read.
    pub(in crate::view) worktree_markdown_preview_block_scrolls: rows::MarkdownDocumentBlockScrolls,
    /// Block grouping of the document the rendered preview last drew, so it is
    /// not re-derived on every frame.
    pub(in crate::view) worktree_markdown_preview_blocks: rows::MarkdownDocumentBlockCache,
    /// Pictures in the rendered preview that are still decoding and already
    /// have someone waiting to repaint the pane when they finish.
    pub(in crate::view) worktree_markdown_preview_image_waits: FxHashSet<gpui::Resource>,
    pub(in crate::view) worktree_markdown_preview_seq: u64,
    pub(in crate::view) worktree_markdown_preview_inflight: Option<u64>,
    pub(in crate::view) worktree_preview_segments_cache_path: Option<std::path::PathBuf>,
    pub(in crate::view) worktree_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) worktree_preview_style_cache_epoch: u64,
    pub(in crate::view) worktree_preview_cache_write_blocked_until_rev: Option<u64>,
    pub(in crate::view) worktree_preview_segments_cache:
        FxHashMap<usize, VersionedCachedDiffStyledText>,
    pub(in crate::view) diff_preview_is_new_file: bool,

    /// The editable working-tree buffer. See `super::file_editor`.
    pub(in crate::view) file_editor_input: Entity<components::TextInput>,
    pub(super) _file_editor_input_subscription: gpui::Subscription,
    /// Which repo/path the input currently holds, so a target change is one
    /// comparison rather than a reload every frame.
    pub(in crate::view) file_editor_key: Option<(RepoId, std::path::PathBuf)>,
    pub(in crate::view) file_editor_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) file_editor_loading: bool,
    /// Repo status revision the buffer was last read at. A clean buffer re-reads
    /// when this moves, so an external write to the open file is picked up
    /// rather than silently overwritten by the next save.
    pub(in crate::view) file_editor_loaded_status_rev: u64,
    pub(in crate::view) file_editor_error: Option<SharedString>,
    pub(in crate::view) file_editor_dirty: bool,
    /// The topmost 0-based line an unsaved edit has touched, or `None` while the
    /// buffer matches disk.
    ///
    /// Blame is indexed by committed line number, so an insertion or deletion
    /// shifts the attribution of everything under it — but only under it. This
    /// watermark is what lets the gutter keep showing blame for the untouched
    /// head of the file instead of blanking the whole column on the first
    /// keystroke. Deliberately pessimistic: an edit that changed no line count
    /// still moves it, because tracking that precisely costs more than the
    /// attribution below it is worth.
    pub(in crate::view) file_editor_first_dirty_line: Option<u32>,
    /// Fingerprint of the text last known to be on disk. `None` before the
    /// first read lands, which reads as "everything is unsaved".
    pub(in crate::view) file_editor_saved_fingerprint: Option<u64>,
    /// Unsaved buffers the user navigated away from, keyed by path. This is what
    /// makes leaving a file and coming back non-destructive with auto-save off.
    /// Keyed by repo *and* path: two repo tabs can hold the same relative path,
    /// and one must not restore over the other's buffer.
    pub(in crate::view) file_editor_stash:
        FxHashMap<(RepoId, std::path::PathBuf), super::file_editor::StashedFileEdit>,
    /// Bumped whenever the set of files with unsaved edits changes.
    ///
    /// That set lives here rather than in the store, so nothing outside this
    /// pane can notice it moving on its own — the sidebar keys its file-row
    /// cache off this counter and repaints on the notify that bumps it.
    pub(in crate::view) unsaved_file_edits_rev: u64,
    /// The pending quiet-period timer for auto-save. Dropping it cancels it, so
    /// every keystroke simply replaces it.
    pub(in crate::view) file_editor_autosave: Option<gpui::Task<()>>,
    /// The editor's tree-sitter document. Owned here for the same reason the
    /// resolved output's is: it must survive every keystroke, which is exactly
    /// what a content-hash-keyed cache cannot do.
    pub(in crate::view) file_editor_live_syntax: Option<rows::LiveSyntaxDocument>,
    /// `(model_id, revision)` the live tree was last built or synced for.
    pub(in crate::view) file_editor_live_syntax_source: Option<(u64, u64)>,
    pub(in crate::view) file_editor_live_syntax_building: Option<(u64, u64)>,
    /// In-flight *first* parse. Kept apart from the reparse slot, which is
    /// cleared whenever there is no document to reparse — the state a first
    /// parse runs in.
    pub(in crate::view) file_editor_live_syntax_build: Option<gpui::Task<()>>,
    pub(in crate::view) file_editor_live_syntax_reparse: Option<gpui::Task<()>>,
    /// The delimiters currently washed as the caret's bracket pair.
    pub(in crate::view) file_editor_syntax_pair: Option<rows::SyntaxPair>,
    /// Everywhere the editor's buffer names the token under the caret.
    pub(in crate::view) file_editor_occurrences: Vec<Range<usize>>,
    /// The document version `file_editor_occurrences` was computed against, so
    /// a caret moving *within* the same name does not rescan the document.
    pub(in crate::view) file_editor_occurrences_version: Option<u64>,
    /// Byte ranges of every search match in the editor buffer, one per
    /// occurrence and parallel to `diff_search_matches`, which carries the line
    /// each of them sits on. Keeping the two parallel is what lets the shared
    /// `n/N` label and match cursor work over the editor unchanged.
    pub(in crate::view) file_editor_search_matches: Vec<Range<usize>>,
    /// The buffer the scan reads. It runs without a `cx` and so cannot reach the
    /// input; a snapshot is an `Arc` bump and immutable under later edits, which
    /// makes caching one here the cheap way to hand it the live text.
    pub(in crate::view) file_editor_search_source: Option<TextModelSnapshot>,
    /// Bumped whenever the *painted* match set moves — a rescan, a cursor step,
    /// the search closing. `render_file_editor` rebinds the highlight provider
    /// when it differs from `file_editor_search_applied_rev`.
    pub(in crate::view) file_editor_search_rev: u64,
    pub(in crate::view) file_editor_search_applied_rev: u64,
    /// Bumped only when the match *cursor* moves. Separate from the rev above
    /// because it drives the selection, and a rescan alone must not re-select:
    /// the buffer is rescanned on every keystroke while the search box is open,
    /// which would drag the caret off what the user is typing.
    pub(in crate::view) file_editor_search_reveal_rev: u64,
    pub(in crate::view) file_editor_search_reveal_applied_rev: u64,
    /// Set once a search reveal has moved the caret, cleared once the editor
    /// has been scrolled sideways to it.
    ///
    /// The caret's x can only be read from the layout of a frame that already
    /// painted it, so the horizontal half of the reveal lands one frame after
    /// the selection does.
    pub(in crate::view) file_editor_search_reveal_x_pending: bool,
    /// Bumped on every theme change: the syntax palette is baked into the
    /// snapshot the provider closes over, so a new theme needs a new binding key.
    pub(in crate::view) file_editor_provider_theme_epoch: u64,
    /// Mirrors the settings window's toggle; the pane never writes it back.
    pub(in crate::view) auto_save_file_edits: bool,

    pub(in crate::view) conflict_resolver_input: Entity<components::TextInput>,
    pub(super) _conflict_resolver_input_subscription: gpui::Subscription,
    pub(in crate::view) conflict_resolver: ConflictResolverUiState,
    pub(in crate::view) conflict_open_summary_toasted_files:
        FxHashSet<(RepoId, std::path::PathBuf)>,
    pub(in crate::view) conflict_resolver_vsplit_ratio: f32,
    pub(in crate::view) conflict_resolver_vsplit_resize: Option<ConflictVSplitResizeState>,
    pub(in crate::view) conflict_three_way_col_ratios: [f32; 2],
    pub(in crate::view) conflict_three_way_col_widths: [Pixels; 3],
    pub(in crate::view) conflict_hsplit_resize: Option<ConflictHSplitResizeState>,
    pub(in crate::view) conflict_diff_split_ratio: f32,
    pub(in crate::view) conflict_diff_split_resize: Option<ConflictDiffSplitResizeState>,
    pub(in crate::view) conflict_diff_split_col_widths: [Pixels; 2],
    pub(in crate::view) conflict_canvas_rows_enabled: bool,
    pub(in crate::view) conflict_diff_segments_cache_split:
        crate::view::conflict_resolver::ConflictSplitStyledTextCache,
    pub(in crate::view) conflict_diff_query_segments_cache_split:
        crate::view::conflict_resolver::ConflictSplitStyledTextCache,
    pub(in crate::view) conflict_diff_query_cache_query: SharedString,
    pub(in crate::view) conflict_diff_query_cache_options: super::diff_search::DiffSearchOptions,
    pub(in crate::view) conflict_three_way_segments_cache:
        FxHashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    /// Quick-search overlay layered on top of `conflict_three_way_segments_cache`.
    ///
    /// Separate so a query change throws away only the wash and leaves the
    /// syntax/word-highlight work standing, the way the two-way columns split
    /// `conflict_diff_segments_cache_split` from its query twin. Holds only
    /// non-current matches — the current one moves with the search cursor and
    /// is built per frame.
    pub(in crate::view) conflict_three_way_query_segments_cache:
        FxHashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    /// Prepared full-document syntax trees for each merge-input side (base, ours, theirs).
    /// When present, three-way rendering uses document-based syntax instead of per-line heuristics.
    pub(in crate::view) conflict_three_way_prepared_syntax_documents:
        ThreeWaySides<Option<rows::PreparedDiffSyntaxDocument>>,
    /// Per-side flag tracking whether a background syntax parse is in-flight.
    pub(in crate::view) conflict_three_way_syntax_inflight: ThreeWaySides<bool>,
    pub(in crate::view) conflict_resolved_preview_path: Option<std::path::PathBuf>,
    /// Latest editable-output revision observed by the input subscription. This
    /// is intentionally independent of the content hash so a keypress can
    /// supersede debounce work without materializing and scanning the document.
    pub(in crate::view) conflict_resolved_preview_source_revision:
        Option<ResolvedOutputSourceRevision>,
    /// Editable-output snapshot at the last file load/save refresh. Snapshot
    /// equality is O(1), and undo restores the matching snapshot, so this can
    /// drive the user-facing Modified state without hashing the whole output.
    pub(in crate::view) conflict_resolved_output_saved_snapshot: Option<TextModelSnapshot>,
    pub(in crate::view) conflict_resolved_output_modified: bool,
    pub(in crate::view) conflict_resolved_output_projection:
        Option<conflict_resolver::ResolvedOutputProjection>,
    /// Byte ownership for displayed conflict blocks in the live output.
    pub(in crate::view) conflict_resolved_output_block_map:
        conflict_resolver::ResolvedOutputBlockMap,
    pub(in crate::view) conflict_resolved_preview_text: TextModelSnapshot,
    pub(in crate::view) conflict_resolved_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in crate::view) conflict_resolved_preview_line_count: usize,
    pub(in crate::view) conflict_resolved_preview_line_starts: Arc<[usize]>,
    /// The editable resolved output's tree-sitter document. Owned here rather
    /// than in the shared thread-local cache because there is exactly one of
    /// them at a time and it must survive every keystroke — which is precisely
    /// what a content-hash-keyed cache cannot do.
    pub(in crate::view) conflict_resolved_output_live_syntax: Option<rows::LiveSyntaxDocument>,
    /// In-flight reparse for an edit that outran the foreground budget.
    pub(in crate::view) conflict_resolved_output_live_syntax_reparse: Option<gpui::Task<()>>,
    /// What the live tree was last built for: the buffer revision and the
    /// placeholder mask. Both must be unchanged for a refresh to be a no-op.
    ///
    /// Deliberately not pointer identity on the text. `SharedString` can be
    /// `Borrowed`, and `Arc<str>::from(&str)` then allocates afresh on every
    /// call, so a pointer check would never match — turning every refresh into a
    /// reparse and, because installing a provider notifies the input that
    /// triggered the refresh, into an unbreakable loop.
    pub(in crate::view) conflict_resolved_output_live_syntax_source:
        Option<(ResolvedOutputSourceRevision, Arc<[Range<usize>]>)>,
    /// Bumped on every theme change. The syntax palette is baked into
    /// `LiveSyntaxSnapshot`, so a new theme needs a new provider -- and the
    /// binding key is the only thing that makes `TextInput` adopt one.
    /// Hashing theme *colours* into the key instead is not enough: two dark
    /// themes can agree on the few colours sampled and still differ on the
    /// syntax palette, leaving stale colours installed.
    pub(in crate::view) conflict_resolved_output_provider_theme_epoch: u64,
    /// Which conflict the installed output highlights wash yellow. Conflict
    /// navigation moves no text and touches no tree, so none of the refresh
    /// paths fire on it; this is what tells the render pass the active row moved
    /// and the provider has to be rebuilt.
    pub(in crate::view) conflict_resolved_output_highlighted_conflict: Option<usize>,
    /// Unresolved output rows and their conflict, cached for the buffer
    /// revision they were computed from.
    ///
    /// Conflict navigation moves the yellow wash but changes no text, so
    /// recomputing these would rescan the whole document on every jump — which
    /// is exactly what made F3 cost tens of milliseconds on a large file.
    pub(in crate::view) conflict_resolved_output_unresolved_rows: Option<CachedUnresolvedRows>,
    /// How many times the resolved output's syntax refresh has gone past its
    /// early-out and rescanned the document. Only an edit should do that;
    /// navigation must not.
    #[cfg(test)]
    pub(in crate::view) conflict_resolved_output_full_scans: usize,
    /// Revision an off-thread first parse is currently running for, so repeated
    /// refreshes over the same text do not pile up duplicate builds.
    pub(in crate::view) conflict_resolved_output_live_syntax_building:
        Option<ResolvedOutputSourceRevision>,
    /// In-flight *first* parse. Kept apart from the reparse slot: that one is
    /// cleared whenever there is no document to reparse, which is exactly the
    /// state a first parse runs in -- sharing the slot would cancel it.
    pub(in crate::view) conflict_resolved_output_live_syntax_build: Option<gpui::Task<()>>,
    pub(in crate::view) conflict_resolved_output_measure_row: usize,
    pub(in crate::view) conflict_resolved_outline_stash: Option<StashedResolvedOutlineState>,
    #[cfg(test)]
    pub(in crate::view) conflict_resolved_outline_background_delay_override:
        Option<std::time::Duration>,

    pub(in crate::view) history_view: Entity<super::HistoryView>,
    pub(in crate::view) diff_scroll: UniformListScrollHandle,
    pub(in crate::view) diff_split_right_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_resolver_diff_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_ours_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_theirs_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_preview_last_synced_x: [Pixels; 4],
    pub(in crate::view) conflict_preview_last_synced_y: [Pixels; 4],
    /// Source/output handle index that received the latest vertical wheel
    /// gesture: base/left=0, ours=1, theirs/right=2, output=3.
    pub(in crate::view) conflict_preview_vertical_wheel_master: Option<usize>,
    /// The next output/gutter sync belongs to that wheel gesture, so output
    /// must drive the pair instead of a stale gutter baseline.
    pub(in crate::view) conflict_output_gutter_wheel_sync_pending: bool,
    pub(in crate::view) conflict_resolved_preview_scroll: UniformListScrollHandle,
    /// Scroll handle for the editable resolved-output `TextInput`. The input lays
    /// out at full content height inside an `overflow_y_scroll` container that
    /// tracks this handle, and the input reads the same handle to window its line
    /// shaping. It is also the output member (index 3) of the conflict-preview
    /// scroll-sync group, so it stands in for `conflict_resolved_preview_scroll`
    /// (which now only backs the read-only projection paths).
    pub(in crate::view) conflict_resolved_output_editor_scroll: ScrollHandle,
    pub(in crate::view) conflict_resolved_preview_gutter_scroll: UniformListScrollHandle,
    pub(in crate::view) conflict_resolved_preview_gutter_last_synced_y: [Pixels; 2],
    pub(in crate::view) worktree_preview_scroll: UniformListScrollHandle,
    /// Scroll handle for the editor's `TextInput`: the input lays out at full
    /// content size inside an `overflow_scroll` container tracking this handle,
    /// and reads the same handle to window its line shaping.
    pub(in crate::view) file_editor_scroll: ScrollHandle,
    /// Gutter list, mirrored to `file_editor_scroll`'s vertical offset.
    pub(in crate::view) file_editor_gutter_scroll: UniformListScrollHandle,
    /// UI-scaled row height the gutter list paints at, computed by the render
    /// pass so the virtualized row processor can read it without a scale lookup.
    pub(in crate::view) file_editor_gutter_row_height: Pixels,
    /// The same, for the merge tool's resolved-output gutter. Navigation centres
    /// the editable output on a row from `&self`, where there is no `cx` to look
    /// the scale up through, so the render pass leaves it here.
    pub(in crate::view) conflict_resolved_gutter_row_height: Pixels,
    /// Blame for the edited file, resolved by the render pass so the virtualized
    /// gutter rows can read it without rebuilding the context per row.
    pub(in crate::view) file_editor_blame: Option<rows::BlameRenderCtx>,
    pub(in crate::view) file_editor_blame_width: Pixels,
    /// First gutter row owned by each logical line, so a wrapped line's number
    /// sits on the first of the rows it spans. Empty when the buffer is not
    /// wrapping. Retained to keep its allocation across frames.
    pub(in crate::view) file_editor_wrap_row_starts: Vec<usize>,

    pub(super) path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,

    /// Per-repo interactive rebase editing state, keyed by repo id so that
    /// setups open in several repo tabs at once stay independent. Entries are
    /// populated when a repo's setup becomes Ready and dropped when its setup
    /// goes away (see `apply_state`).
    pub(in crate::view) interactive_rebase_states: FxHashMap<RepoId, IRebaseViewState>,
}

/// View-local editing state for one repo's interactive rebase setup.
#[derive(Default)]
pub(in crate::view) struct IRebaseViewState {
    pub(in crate::view) mode: ICommitEditorMode,
    pub(in crate::view) entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
    pub(in crate::view) original_entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
    pub(in crate::view) source_colors: FxHashMap<String, u8>,
    /// Active auto-squash strategy, or None when auto-squash is off.
    pub(in crate::view) autosquash_mode: Option<AutosquashMode>,
    /// Commits folded away by auto-squash, keyed by the surviving commit id.
    /// Each survivor's `entries` row displays these ids; they are re-expanded
    /// into `fixup` todo entries when the rebase starts.
    pub(in crate::view) folded:
        FxHashMap<String, Vec<gitcomet_core::services::InteractiveRebaseEntry>>,
    pub(in crate::view) drag_state: Option<IRebaseDragState>,
    /// Variable-height virtualized list state, lazily created on first render
    /// (`ListState` has no `Default`). Kept in sync with `entries`/`folded` via
    /// `list_sig` (remeasure on same-count content change, reset on count change).
    pub(in crate::view) scroll: Option<gpui::ListState>,
    /// (content-hash, item-count) the `scroll` ListState was last synced to.
    pub(in crate::view) list_sig: (u64, usize),
    /// (ix_a, ix_b, version) — the two data-indices swapped by ▲/▼; drives fade-in animation.
    pub(in crate::view) reorder_anim: Option<(usize, usize, u32)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) enum ICommitEditorMode {
    #[default]
    Rebase,
    CherryPick,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct IRebaseDragState {
    pub(in crate::view) from_ix: usize,
    pub(in crate::view) to_ix: usize,
    /// Drop-target position in display order (0..=entry_count).
    pub(in crate::view) display_pos: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffTextAutoscrollTarget {
    DiffLeftOrInline,
    DiffSplitRight,
    WorktreePreview,
    ConflictResolvedPreview,
}

pub(super) fn parse_conflict_canvas_rows_env(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

pub(super) fn conflict_canvas_rows_enabled_from_env() -> bool {
    std::env::var("GITCOMET_CONFLICT_CANVAS_ROWS")
        .ok()
        .is_none_or(|value| parse_conflict_canvas_rows_env(&value))
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

    #[test]
    fn resolved_outline_provenance_skip_thresholds_match_view_mode() {
        assert!(!should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::ThreeWay,
            LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES,
        ));
        assert!(should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::ThreeWay,
            LARGE_RESOLVED_OUTLINE_THREE_WAY_PROVENANCE_MAX_LINES + 1,
        ));
        assert!(!should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::TwoWayDiff,
            LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES,
        ));
        assert!(should_skip_resolved_outline_provenance(
            ConflictResolverViewMode::TwoWayDiff,
            LARGE_RESOLVED_OUTLINE_TWO_WAY_PROVENANCE_MAX_LINES + 1,
        ));
    }

    #[test]
    fn unresolved_decision_regions_track_non_emitting_selected_rows() {
        let block = conflict_resolver::ConflictBlock {
            base: None,
            ours: "".into(),
            theirs: "added line\n".into(),
            choice: conflict_resolver::ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        };

        let regions =
            unresolved_decision_regions_for_block(&block).expect("expected one decision region");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].row_range, 0..1);
        assert_eq!(regions[0].selected_line_range, 0..0);
        assert_eq!(regions[0].alternate_line_range, 0..1);
        assert!(regions[0].has_non_emitting_rows);
    }
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
