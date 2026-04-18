use super::canvas::keyed_canvas;
use super::diff_text::{
    PreparedDocumentByteRangeHighlights, build_cached_diff_query_overlay_styled_text,
    build_cached_diff_styled_text, build_cached_diff_styled_text_from_relative_highlights,
    syntax_highlights_for_streamed_line_slice_heuristic,
};
use super::*;
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, HighlightStyle, Hitbox, HitboxBehavior, Pixels,
    Styled, TextRun, TextStyle, Window, fill, point, px, size,
};
use rustc_hash::{FxHashMap, FxHasher};
use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;

const DIFF_FONT_SCALE: f32 = 0.80;

const GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 16_384;
const STREAMED_DIFF_TEXT_MIN_BYTES: usize = 64 * 1024;
const STREAMED_DIFF_TEXT_OVERSCAN_COLUMNS: usize = 64;
const STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE: &str = "0000000000";
const DIFF_ROW_HEIGHT_PX: f32 = 20.0;
const DIFF_GUTTER_BASE_WIDTH_PX: f32 = 44.0;
const DIFF_ROW_TEXT_TRAILING_PADDING_PX: f32 = 16.0;
const DIFF_CHANGE_BAR_WIDTH_PX: f32 = 3.0;

type HighlightSpans = Arc<[(Range<usize>, HighlightStyle)]>;

thread_local! {
    static GUTTER_TEXT_LAYOUT_CACHE: RefCell<FxLruCache<u64, gpui::ShapedLine>> =
        RefCell::new(new_fx_lru_cache(GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES));
    static STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE: RefCell<FxHashMap<u64, Pixels>> =
        RefCell::new(FxHashMap::default());
}

#[derive(Clone)]
pub(super) enum StreamedDiffTextSyntaxSource {
    None,
    Heuristic {
        language: rows::DiffSyntaxLanguage,
        mode: rows::DiffSyntaxMode,
    },
    Prepared {
        document_text: Arc<str>,
        line_starts: Arc<[usize]>,
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_ix: usize,
    },
}

#[derive(Clone)]
pub(super) struct StreamedDiffTextPaintSpec {
    pub(super) raw_text: gitcomet_core::file_diff::FileDiffLineText,
    pub(super) query: SharedString,
    pub(super) word_ranges: Arc<[Range<usize>]>,
    pub(super) word_color: Option<gpui::Rgba>,
    pub(super) syntax: StreamedDiffTextSyntaxSource,
}

fn hash_rgba(hasher: &mut FxHasher, color: gpui::Rgba) {
    color.r.to_bits().hash(hasher);
    color.g.to_bits().hash(hasher);
    color.b.to_bits().hash(hasher);
    color.a.to_bits().hash(hasher);
}

fn hash_shared_string(hasher: &mut FxHasher, text: &SharedString) {
    text.as_ref().hash(hasher);
}

fn inline_row_canvas_revision_key(
    old: &SharedString,
    new: &SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    text_hash: u64,
    highlights_hash: u64,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_shared_string(&mut hasher, old);
    hash_shared_string(&mut hasher, new);
    hash_rgba(&mut hasher, bg);
    hash_rgba(&mut hasher, fg);
    hash_rgba(&mut hasher, gutter_fg);
    text_hash.hash(&mut hasher);
    highlights_hash.hash(&mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
fn split_row_canvas_revision_key(
    old: &SharedString,
    new: &SharedString,
    left_bg: gpui::Rgba,
    left_fg: gpui::Rgba,
    left_gutter: gpui::Rgba,
    right_bg: gpui::Rgba,
    right_fg: gpui::Rgba,
    right_gutter: gpui::Rgba,
    left_text_hash: u64,
    left_highlights_hash: u64,
    right_text_hash: u64,
    right_highlights_hash: u64,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_shared_string(&mut hasher, old);
    hash_shared_string(&mut hasher, new);
    hash_rgba(&mut hasher, left_bg);
    hash_rgba(&mut hasher, left_fg);
    hash_rgba(&mut hasher, left_gutter);
    hash_rgba(&mut hasher, right_bg);
    hash_rgba(&mut hasher, right_fg);
    hash_rgba(&mut hasher, right_gutter);
    left_text_hash.hash(&mut hasher);
    left_highlights_hash.hash(&mut hasher);
    right_text_hash.hash(&mut hasher);
    right_highlights_hash.hash(&mut hasher);
    hasher.finish()
}

fn patch_split_row_canvas_revision_key(
    line_no: &SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    text_hash: u64,
    highlights_hash: u64,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_shared_string(&mut hasher, line_no);
    hash_rgba(&mut hasher, bg);
    hash_rgba(&mut hasher, fg);
    hash_rgba(&mut hasher, gutter_fg);
    text_hash.hash(&mut hasher);
    highlights_hash.hash(&mut hasher);
    hasher.finish()
}

fn semantic_diff_row_bg(theme: AppTheme, bg: gpui::Rgba) -> Option<gpui::Rgba> {
    (bg != theme.colors.window_bg).then_some(bg)
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(in crate::view) struct DiffPaintRecord {
    pub(in crate::view) visible_ix: usize,
    pub(in crate::view) region: DiffTextRegion,
    pub(in crate::view) text: SharedString,
    pub(in crate::view) highlights: Vec<(Range<usize>, Option<gpui::Hsla>, Option<gpui::Hsla>)>,
    pub(in crate::view) row_bg: Option<gpui::Rgba>,
}

#[cfg(test)]
thread_local! {
    static DIFF_PAINT_LOG: RefCell<Vec<DiffPaintRecord>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_diff_paint_for_tests(
    visible_ix: usize,
    region: DiffTextRegion,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    row_bg: Option<gpui::Rgba>,
) {
    DIFF_PAINT_LOG.with(|log| {
        log.borrow_mut().push(DiffPaintRecord {
            visible_ix,
            region,
            text: text.clone(),
            highlights: highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
            row_bg,
        });
    });
}

#[cfg(test)]
pub(in crate::view) fn clear_diff_paint_log_for_tests() {
    DIFF_PAINT_LOG.with(|log| log.borrow_mut().clear());
}

#[cfg(test)]
pub(in crate::view) fn diff_paint_log_for_tests() -> Vec<DiffPaintRecord> {
    DIFF_PAINT_LOG.with(|log| log.borrow().clone())
}

pub(in crate::view) fn is_streamable_diff_text(
    text: &gitcomet_core::file_diff::FileDiffLineText,
) -> bool {
    text.len() >= STREAMED_DIFF_TEXT_MIN_BYTES && !text.has_tabs_without_loading()
}

fn should_stream_diff_text(spec: Option<&StreamedDiffTextPaintSpec>) -> bool {
    let Some(spec) = spec else {
        return false;
    };
    is_streamable_diff_text(&spec.raw_text)
}

fn streamed_diff_text_cell_width_cache_key(base_style: &TextStyle, font_size: Pixels) -> u64 {
    let mut hasher = FxHasher::default();
    font_size.hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_weight.hash(&mut hasher);
    hasher.finish()
}

fn streamed_diff_text_ascii_cell_width(
    base_style: &TextStyle,
    font_size: Pixels,
    window: &mut Window,
) -> Pixels {
    let key = streamed_diff_text_cell_width_cache_key(base_style, font_size);
    if let Some(width) =
        STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE.with(|cache| cache.borrow().get(&key).copied())
    {
        return width;
    }

    let run = base_style.to_run(STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.len());
    let layout = window.text_system().shape_line(
        STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.into(),
        font_size,
        &[run],
        None,
    );
    let width = if STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.is_empty() {
        px(0.0)
    } else {
        layout.width / STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.len() as f32
    };
    STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, width);
    });
    width
}

fn streamed_diff_text_visible_slice_range(
    bounds: Bounds<Pixels>,
    clip_bounds: Bounds<Pixels>,
    total_len: usize,
    cell_width: Pixels,
    overscan_columns: usize,
) -> Range<usize> {
    if total_len == 0 || cell_width <= px(0.0) {
        return 0..0;
    }

    let visible = bounds.intersect(&clip_bounds);
    let left = if visible.size.width > px(0.0) {
        (visible.left() - bounds.left()).max(px(0.0))
    } else {
        px(0.0)
    };
    let right = if visible.size.width > px(0.0) {
        (visible.right() - bounds.left()).max(left)
    } else {
        left
    };

    let start = ((left / cell_width).floor() as usize).saturating_sub(overscan_columns);
    let mut end = ((right / cell_width).ceil() as usize)
        .saturating_add(overscan_columns)
        .min(total_len);
    if end <= start {
        end = (start + 1).min(total_len);
    }
    start.min(total_len)..end
}

fn clip_ranges_to_slice(ranges: &[Range<usize>], slice_range: &Range<usize>) -> Vec<Range<usize>> {
    if ranges.is_empty() || slice_range.is_empty() {
        return Vec::new();
    }

    let mut clipped = Vec::new();
    for range in ranges {
        let start = range.start.max(slice_range.start);
        let end = range.end.min(slice_range.end);
        if start < end {
            clipped.push(
                start.saturating_sub(slice_range.start)..end.saturating_sub(slice_range.start),
            );
        }
    }
    clipped
}

fn push_or_extend_highlight(
    merged: &mut Vec<(Range<usize>, HighlightStyle)>,
    range: Range<usize>,
    style: HighlightStyle,
) {
    if range.is_empty() {
        return;
    }

    if let Some(last) = merged.last_mut()
        && last.0.end == range.start
        && last.1 == style
    {
        last.0.end = range.end;
        return;
    }

    merged.push((range, style));
}

fn hash_range(hasher: &mut FxHasher, range: &Range<usize>) {
    range.start.hash(hasher);
    range.end.hash(hasher);
}

fn streamed_diff_text_text_hash(spec: &StreamedDiffTextPaintSpec) -> u64 {
    spec.raw_text.identity_hash_without_loading()
}

fn streamed_diff_text_highlights_hash(spec: &StreamedDiffTextPaintSpec) -> u64 {
    let mut hasher = FxHasher::default();
    spec.query.as_ref().hash(&mut hasher);
    for range in spec.word_ranges.iter() {
        hash_range(&mut hasher, range);
    }
    if let Some(color) = spec.word_color {
        hash_rgba(&mut hasher, color);
    }
    match &spec.syntax {
        StreamedDiffTextSyntaxSource::None => {
            0u8.hash(&mut hasher);
        }
        StreamedDiffTextSyntaxSource::Heuristic { language, mode } => {
            1u8.hash(&mut hasher);
            language.hash(&mut hasher);
            mode.hash(&mut hasher);
        }
        StreamedDiffTextSyntaxSource::Prepared {
            document_text,
            line_starts,
            language,
            line_ix,
            ..
        } => {
            2u8.hash(&mut hasher);
            language.hash(&mut hasher);
            line_ix.hash(&mut hasher);
            (document_text.as_ptr() as usize).hash(&mut hasher);
            document_text.len().hash(&mut hasher);
            (line_starts.as_ptr() as usize).hash(&mut hasher);
            line_starts.len().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn hash_overlay_ranges(
    base_highlights_hash: u64,
    ranges: &[Range<usize>],
    background_color: gpui::Hsla,
) -> u64 {
    let mut hasher = FxHasher::default();
    base_highlights_hash.hash(&mut hasher);
    hash_rgba(&mut hasher, background_color.into());
    for range in ranges {
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
    }
    hasher.finish()
}

fn overlay_background_ranges_on_styled_text(
    base: &CachedDiffStyledText,
    ranges: &[Range<usize>],
    background_color: gpui::Hsla,
) -> CachedDiffStyledText {
    if ranges.is_empty() || base.text.is_empty() {
        return base.clone();
    }

    let base_highlights = base.highlights.as_ref();
    if base_highlights.is_empty() {
        let mut merged = Vec::with_capacity(ranges.len());
        for range in ranges.iter().cloned() {
            push_or_extend_highlight(
                &mut merged,
                range,
                HighlightStyle {
                    background_color: Some(background_color),
                    ..HighlightStyle::default()
                },
            );
        }
        return CachedDiffStyledText {
            text: base.text.clone(),
            highlights: Arc::from(merged),
            highlights_hash: hash_overlay_ranges(base.highlights_hash, ranges, background_color),
            text_hash: base.text_hash,
        };
    }

    let mut merged = Vec::with_capacity(base_highlights.len() + ranges.len() * 2);
    let mut base_ix = 0usize;
    let mut range_ix = 0usize;
    let mut cursor = 0usize;
    let text_len = base.text.len();
    let default_style = HighlightStyle::default();

    while cursor < text_len {
        while base_ix < base_highlights.len() && base_highlights[base_ix].0.end <= cursor {
            base_ix += 1;
        }
        while range_ix < ranges.len() && ranges[range_ix].end <= cursor {
            range_ix += 1;
        }

        let active_base = base_highlights
            .get(base_ix)
            .filter(|(range, _)| range.start <= cursor && range.end > cursor);
        let active_range = ranges
            .get(range_ix)
            .filter(|range| range.start <= cursor && range.end > cursor);

        let mut next_boundary = text_len;
        if let Some((range, _)) = active_base {
            next_boundary = next_boundary.min(range.end.min(text_len));
        } else if let Some((range, _)) = base_highlights.get(base_ix) {
            next_boundary = next_boundary.min(range.start.min(text_len));
        }
        if let Some(range) = active_range {
            next_boundary = next_boundary.min(range.end.min(text_len));
        } else if let Some(range) = ranges.get(range_ix) {
            next_boundary = next_boundary.min(range.start.min(text_len));
        }

        if next_boundary <= cursor {
            break;
        }

        let mut style = active_base.map(|(_, style)| *style).unwrap_or_default();
        if active_range.is_some() {
            style.background_color = Some(background_color);
        }

        if style != default_style {
            push_or_extend_highlight(&mut merged, cursor..next_boundary, style);
        }

        cursor = next_boundary;
    }

    CachedDiffStyledText {
        text: base.text.clone(),
        highlights: Arc::from(merged),
        highlights_hash: hash_overlay_ranges(base.highlights_hash, ranges, background_color),
        text_hash: base.text_hash,
    }
}

fn streamed_diff_text_relative_prepared_highlights(
    theme: AppTheme,
    spec: &StreamedDiffTextPaintSpec,
    slice_range: &Range<usize>,
) -> Option<PreparedDocumentByteRangeHighlights> {
    let StreamedDiffTextSyntaxSource::Prepared {
        document_text,
        line_starts,
        document,
        language,
        line_ix,
    } = &spec.syntax
    else {
        return None;
    };

    let text_len = document_text.len();
    let line_start = line_starts
        .get(*line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let abs_start = line_start.saturating_add(slice_range.start).min(text_len);
    let abs_end = line_start.saturating_add(slice_range.end).min(text_len);
    if abs_start >= abs_end {
        return Some(PreparedDocumentByteRangeHighlights::default());
    }

    rows::request_syntax_highlights_for_prepared_document_byte_range(
        theme,
        document_text.as_ref(),
        line_starts.as_ref(),
        *document,
        *language,
        abs_start..abs_end,
    )
}

fn build_streamed_diff_slice_styled_text(
    theme: AppTheme,
    spec: &StreamedDiffTextPaintSpec,
    requested_slice_range: &Range<usize>,
) -> (CachedDiffStyledText, bool, Range<usize>) {
    let (slice_text, resolved_slice_range) = spec
        .raw_text
        .slice_text_resolved(requested_slice_range.clone())
        .unwrap_or((Cow::Borrowed(""), 0..0));
    let slice_text_ref = slice_text.as_ref();

    let mut pending = false;
    let mut base = match &spec.syntax {
        StreamedDiffTextSyntaxSource::None => build_cached_diff_styled_text(
            theme,
            slice_text_ref,
            &[],
            "",
            None,
            rows::DiffSyntaxMode::HeuristicOnly,
            None,
        ),
        StreamedDiffTextSyntaxSource::Heuristic { language, mode } => {
            match syntax_highlights_for_streamed_line_slice_heuristic(
                theme,
                &spec.raw_text,
                *language,
                requested_slice_range.clone(),
                resolved_slice_range.clone(),
            ) {
                Some(highlights) => build_cached_diff_styled_text_from_relative_highlights(
                    slice_text_ref,
                    highlights.as_slice(),
                ),
                None => build_cached_diff_styled_text(
                    theme,
                    slice_text_ref,
                    &[],
                    "",
                    Some(*language),
                    *mode,
                    None,
                ),
            }
        }
        StreamedDiffTextSyntaxSource::Prepared { language, .. } => {
            match streamed_diff_text_relative_prepared_highlights(
                theme,
                spec,
                &resolved_slice_range,
            ) {
                Some(result) => {
                    pending = result.pending;
                    let StreamedDiffTextSyntaxSource::Prepared {
                        line_starts,
                        line_ix,
                        ..
                    } = &spec.syntax
                    else {
                        unreachable!();
                    };
                    let line_start = line_starts
                        .get(*line_ix)
                        .copied()
                        .unwrap_or_default()
                        .saturating_add(resolved_slice_range.start);
                    let mut relative = Vec::with_capacity(result.highlights.len());
                    for (range, style) in result.highlights {
                        let start = range.start.max(line_start);
                        let end = range
                            .end
                            .min(line_start.saturating_add(resolved_slice_range.len()));
                        if start < end {
                            relative.push((
                                start.saturating_sub(line_start)..end.saturating_sub(line_start),
                                style,
                            ));
                        }
                    }
                    build_cached_diff_styled_text_from_relative_highlights(
                        slice_text_ref,
                        relative.as_slice(),
                    )
                }
                None => build_cached_diff_styled_text(
                    theme,
                    slice_text_ref,
                    &[],
                    "",
                    Some(*language),
                    rows::DiffSyntaxMode::HeuristicOnly,
                    None,
                ),
            }
        }
    };

    if !spec.word_ranges.is_empty()
        && let Some(mut color) = spec.word_color
    {
        let clipped = clip_ranges_to_slice(spec.word_ranges.as_ref(), &resolved_slice_range);
        if !clipped.is_empty() {
            color.a = if theme.is_dark { 0.22 } else { 0.16 };
            base =
                overlay_background_ranges_on_styled_text(&base, clipped.as_slice(), color.into());
        }
    }

    if !spec.query.as_ref().trim().is_empty() {
        base = build_cached_diff_query_overlay_styled_text(theme, &base, spec.query.as_ref());
    }

    (base, pending, resolved_slice_range)
}

fn diff_text_paint_payload(
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<&StreamedDiffTextPaintSpec>,
) -> (SharedString, HighlightSpans, u64, u64) {
    if should_stream_diff_text(streamed_spec) {
        let spec = streamed_spec.expect("streamed spec checked above");
        return (
            SharedString::default(),
            empty_highlights(),
            streamed_diff_text_highlights_hash(spec),
            streamed_diff_text_text_hash(spec),
        );
    }

    let text = styled.map(|s| s.text.clone()).unwrap_or_default();
    let highlights = styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
    let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);
    (text, highlights, highlights_hash, text_hash)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_diff_line_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    ui_scale_percent: u32,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    old: SharedString,
    new: SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
) -> AnyElement {
    let (text, highlights, highlights_hash, text_hash) =
        diff_text_paint_payload(styled, streamed_spec.as_ref());
    let revision =
        inline_row_canvas_revision_key(&old, &new, bg, fg, gutter_fg, text_hash, highlights_hash);
    let canvas_id: gpui::ElementId = ("diff_row_canvas_inline", visible_ix).into();
    let test_row_bg = semantic_diff_row_bg(theme, bg);

    keyed_canvas(
        (canvas_id, format!("{revision:016x}")),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
            let text_bounds = inline_text_bounds(bounds, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);

            InlineRowPrepaintState {
                bounds,
                pad,
                gutter_total,
                text_bounds,
                text_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            paint_gutter_text(
                &old,
                prepaint.bounds.left() + prepaint.pad,
                y,
                gutter_fg,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &new,
                prepaint.bounds.left() + prepaint.gutter_total + prepaint.pad,
                y,
                gutter_fg,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    DiffTextRegion::Inline,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    streamed_spec.as_ref(),
                    test_row_bg,
                    highlights_hash,
                    text_hash,
                    y,
                    fg,
                    line_metrics,
                    ui_scale_percent,
                    theme,
                    window,
                    cx,
                );
            });

            let row_bounds = prepaint.bounds;
            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_row_bounds = row_bounds.intersect(&clip_bounds);
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_bounds: visible_row_bounds,
                    regions: DiffRowTextRegions::single(
                        DiffTextRegion::Inline,
                        visible_text_bounds,
                    ),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                },
            );

            if selected {
                window.paint_quad(gpui::outline(
                    bounds,
                    with_alpha(theme.colors.accent, 0.55),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .bg(bg)
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn split_diff_line_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    ui_scale_percent: u32,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    old: SharedString,
    new: SharedString,
    left_bg: gpui::Rgba,
    left_fg: gpui::Rgba,
    left_gutter: gpui::Rgba,
    right_bg: gpui::Rgba,
    right_fg: gpui::Rgba,
    right_gutter: gpui::Rgba,
    left_styled: Option<&CachedDiffStyledText>,
    right_styled: Option<&CachedDiffStyledText>,
    left_streamed_spec: Option<StreamedDiffTextPaintSpec>,
    right_streamed_spec: Option<StreamedDiffTextPaintSpec>,
) -> AnyElement {
    let (left_text, left_highlights, left_highlights_hash, left_text_hash) =
        diff_text_paint_payload(left_styled, left_streamed_spec.as_ref());
    let (right_text, right_highlights, right_highlights_hash, right_text_hash) =
        diff_text_paint_payload(right_styled, right_streamed_spec.as_ref());
    let revision = split_row_canvas_revision_key(
        &old,
        &new,
        left_bg,
        left_fg,
        left_gutter,
        right_bg,
        right_fg,
        right_gutter,
        left_text_hash,
        left_highlights_hash,
        right_text_hash,
        right_highlights_hash,
    );
    let canvas_id: gpui::ElementId = ("diff_row_canvas_split", visible_ix).into();
    let left_test_row_bg = semantic_diff_row_bg(theme, left_bg);
    let right_test_row_bg = semantic_diff_row_bg(theme, right_bg);

    keyed_canvas(
        (canvas_id, format!("{revision:016x}")),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
            let (left_col, sep_bounds, right_col) = split_columns(bounds);
            let left_text_bounds = column_text_bounds(left_col, gutter_total, pad);
            let right_text_bounds = column_text_bounds(right_col, gutter_total, pad);

            let left_hitbox = window.insert_hitbox(left_text_bounds, HitboxBehavior::Normal);
            let right_hitbox = window.insert_hitbox(right_text_bounds, HitboxBehavior::Normal);

            SplitRowPrepaintState {
                bounds,
                pad,
                left_col,
                sep_bounds,
                right_col,
                left_text_bounds,
                right_text_bounds,
                left_hitbox,
                right_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.left_hitbox);
            window.set_cursor_style(CursorStyle::IBeam, &prepaint.right_hitbox);

            window.paint_quad(fill(prepaint.left_col, left_bg));
            window.paint_quad(fill(prepaint.sep_bounds, theme.colors.border));
            window.paint_quad(fill(prepaint.right_col, right_bg));

            paint_gutter_text(
                &old,
                prepaint.left_col.left() + prepaint.pad,
                y,
                left_gutter,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &new,
                prepaint.right_col.left() + prepaint.pad,
                y,
                right_gutter,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.left_text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    DiffTextRegion::SplitLeft,
                    prepaint.left_text_bounds,
                    &left_text,
                    &left_highlights,
                    left_streamed_spec.as_ref(),
                    left_test_row_bg,
                    left_highlights_hash,
                    left_text_hash,
                    y,
                    left_fg,
                    line_metrics,
                    ui_scale_percent,
                    theme,
                    window,
                    cx,
                );
            });

            window.paint_layer(prepaint.right_text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    DiffTextRegion::SplitRight,
                    prepaint.right_text_bounds,
                    &right_text,
                    &right_highlights,
                    right_streamed_spec.as_ref(),
                    right_test_row_bg,
                    right_highlights_hash,
                    right_text_hash,
                    y,
                    right_fg,
                    line_metrics,
                    ui_scale_percent,
                    theme,
                    window,
                    cx,
                );
            });

            let row_bounds = prepaint.bounds;
            let left_text_bounds = prepaint.left_text_bounds;
            let right_text_bounds = prepaint.right_text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_row_bounds = row_bounds.intersect(&clip_bounds);
            let visible_left_text_bounds = left_text_bounds.intersect(&clip_bounds);
            let visible_right_text_bounds = right_text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_bounds: visible_row_bounds,
                    regions: DiffRowTextRegions::split(
                        visible_left_text_bounds,
                        visible_right_text_bounds,
                    ),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                },
            );

            if selected {
                window.paint_quad(gpui::outline(
                    bounds,
                    with_alpha(theme.colors.accent, 0.55),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn patch_split_column_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    ui_scale_percent: u32,
    column: super::diff::PatchSplitColumn,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    line_no: SharedString,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
) -> AnyElement {
    let region = match column {
        super::diff::PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        super::diff::PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };
    let (text, highlights, highlights_hash, text_hash) =
        diff_text_paint_payload(styled, streamed_spec.as_ref());
    let revision = patch_split_row_canvas_revision_key(
        &line_no,
        bg,
        fg,
        gutter_fg,
        text_hash,
        highlights_hash,
    );
    let canvas_id: gpui::ElementId = (
        match column {
            super::diff::PatchSplitColumn::Left => "diff_row_canvas_file_split_left",
            super::diff::PatchSplitColumn::Right => "diff_row_canvas_file_split_right",
        },
        visible_ix,
    )
        .into();
    let test_row_bg = semantic_diff_row_bg(theme, bg);

    keyed_canvas(
        (canvas_id, format!("{revision:016x}")),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
            let text_bounds = single_column_text_bounds(bounds, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            SingleColumnRowPrepaintState {
                bounds,
                pad,
                text_bounds,
                text_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            window.paint_quad(fill(prepaint.bounds, bg));

            paint_gutter_text(
                &line_no,
                prepaint.bounds.left() + prepaint.pad,
                y,
                gutter_fg,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    region,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    streamed_spec.as_ref(),
                    test_row_bg,
                    highlights_hash,
                    text_hash,
                    y,
                    fg,
                    line_metrics,
                    ui_scale_percent,
                    theme,
                    window,
                    cx,
                );
            });

            let row_bounds = prepaint.bounds;
            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_row_bounds = row_bounds.intersect(&clip_bounds);
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_bounds: visible_row_bounds,
                    regions: DiffRowTextRegions::single(region, visible_text_bounds),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                },
            );

            if selected {
                window.paint_quad(gpui::outline(
                    bounds,
                    with_alpha(theme.colors.accent, 0.55),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

pub(super) fn worktree_preview_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    ui_scale_percent: u32,
    ix: usize,
    min_width: Pixels,
    bar_color: Option<gpui::Rgba>,
    line_no: SharedString,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
) -> AnyElement {
    let (text, highlights, highlights_hash, text_hash) =
        diff_text_paint_payload(styled, streamed_spec.as_ref());

    keyed_canvas(
        ("worktree_preview_row_canvas", ix),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
            let bar_w = if bar_color.is_some() {
                diff_scaled_px(DIFF_CHANGE_BAR_WIDTH_PX, ui_scale_percent)
            } else {
                px(0.0)
            };
            let inner = Bounds::new(
                point(bounds.left() + bar_w, bounds.top()),
                size((bounds.size.width - bar_w).max(px(0.0)), bounds.size.height),
            );
            let text_bounds = single_column_text_bounds(inner, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            WorktreePreviewRowPrepaintState {
                inner,
                pad,
                bar_w,
                text_bounds,
                text_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.paint_quad(fill(bounds, theme.colors.window_bg));
            if let Some(color) = bar_color
                && prepaint.bar_w > px(0.0)
            {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), bounds.top()),
                        size(prepaint.bar_w, bounds.size.height),
                    ),
                    color,
                ));
            }

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            paint_gutter_text(
                &line_no,
                prepaint.inner.left() + prepaint.pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    ix,
                    DiffTextRegion::Inline,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    streamed_spec.as_ref(),
                    None,
                    highlights_hash,
                    text_hash,
                    y,
                    theme.colors.text,
                    line_metrics,
                    ui_scale_percent,
                    theme,
                    window,
                    cx,
                );
            });

            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            window.on_mouse_event({
                let view = view.clone();
                move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || !visible_text_bounds.contains(&event.position)
                    {
                        return;
                    }

                    if event.button == gpui::MouseButton::Left {
                        let focus = view.read(cx).diff_panel_focus_handle.clone();
                        window.focus(&focus, cx);
                        let click_count = event.click_count;
                        let position = event.position;
                        view.update(cx, |this, cx| {
                            if click_count >= 2 {
                                this.double_click_select_diff_text(
                                    ix,
                                    DiffTextRegion::Inline,
                                    DiffClickKind::Line,
                                );
                            } else {
                                this.begin_diff_text_selection(
                                    ix,
                                    DiffTextRegion::Inline,
                                    position,
                                );
                                this.begin_diff_text_scroll_tracking(position, cx);
                            }
                            cx.notify();
                        });
                    } else if event.button == gpui::MouseButton::Right {
                        view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                ix,
                                DiffTextRegion::Inline,
                                event.position,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            });
        },
    )
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[derive(Clone, Debug)]
struct InlineRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    gutter_total: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
struct SplitRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    left_col: Bounds<Pixels>,
    sep_bounds: Bounds<Pixels>,
    right_col: Bounds<Pixels>,
    left_text_bounds: Bounds<Pixels>,
    right_text_bounds: Bounds<Pixels>,
    left_hitbox: Hitbox,
    right_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
struct SingleColumnRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
struct WorktreePreviewRowPrepaintState {
    inner: Bounds<Pixels>,
    pad: Pixels,
    bar_w: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
enum DiffRowTextRegions {
    Single {
        region: DiffTextRegion,
        bounds: Bounds<Pixels>,
    },
    Split {
        left_bounds: Bounds<Pixels>,
        right_bounds: Bounds<Pixels>,
    },
}

impl DiffRowTextRegions {
    fn single(region: DiffTextRegion, bounds: Bounds<Pixels>) -> Self {
        Self::Single { region, bounds }
    }

    fn split(left_bounds: Bounds<Pixels>, right_bounds: Bounds<Pixels>) -> Self {
        Self::Split {
            left_bounds,
            right_bounds,
        }
    }

    fn region_at(&self, position: gpui::Point<Pixels>) -> Option<DiffTextRegion> {
        match self {
            Self::Single { region, bounds } => bounds.contains(&position).then_some(*region),
            Self::Split {
                left_bounds,
                right_bounds,
            } => {
                if left_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitLeft)
                } else if right_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitRight)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffRowRightClickBehavior {
    OpenContextMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffRowMouseUpBehavior {
    None,
    HandlePatchRowClick,
}

#[derive(Clone, Debug)]
struct DiffRowMouseHandlers {
    row_bounds: Bounds<Pixels>,
    regions: DiffRowTextRegions,
    right_click: DiffRowRightClickBehavior,
    mouse_up: DiffRowMouseUpBehavior,
}

fn should_handle_row_mouse_event(
    phase: DispatchPhase,
    row_bounds: &Bounds<Pixels>,
    position: gpui::Point<Pixels>,
) -> bool {
    phase == DispatchPhase::Bubble && row_bounds.contains(&position)
}

fn install_diff_row_mouse_handlers(
    window: &mut Window,
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    handlers: DiffRowMouseHandlers,
) {
    let DiffRowMouseHandlers {
        row_bounds,
        regions,
        right_click,
        mouse_up,
    } = handlers;
    let row_bounds_for_down = row_bounds;
    let regions = regions.clone();
    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if !should_handle_row_mouse_event(phase, &row_bounds_for_down, event.position) {
                return;
            }

            let region = regions.region_at(event.position);

            if event.button == gpui::MouseButton::Left {
                let focus = view.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                if let Some(region) = region {
                    let click_count = event.click_count;
                    let position = event.position;
                    view.update(cx, |this, cx| {
                        if click_count >= 2 {
                            this.double_click_select_diff_text(
                                visible_ix,
                                region,
                                DiffClickKind::Line,
                            );
                        } else {
                            this.begin_diff_text_selection(visible_ix, region, position);
                            this.begin_diff_text_scroll_tracking(position, cx);
                        }
                        cx.notify();
                    });
                }
            } else if event.button == gpui::MouseButton::Right
                && let Some(region) = region
            {
                match right_click {
                    DiffRowRightClickBehavior::OpenContextMenu => {
                        view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                visible_ix,
                                region,
                                event.position,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            }
        }
    });

    if mouse_up == DiffRowMouseUpBehavior::None {
        return;
    }

    let row_bounds_for_up = row_bounds;
    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseUpEvent, phase, _window, cx| {
            if event.button != gpui::MouseButton::Left
                || !should_handle_row_mouse_event(phase, &row_bounds_for_up, event.position)
            {
                return;
            }

            let shift = event.modifiers.shift;
            view.update(cx, |this, cx| {
                if this.consume_suppress_click_after_drag() {
                    cx.notify();
                    return;
                }
                this.handle_patch_row_click(visible_ix, DiffClickKind::Line, shift);
                cx.notify();
            });
        }
    });
}

#[derive(Clone, Copy, Debug)]
struct LineMetrics {
    font_size: Pixels,
    line_height: Pixels,
}

fn diff_text_style(window: &Window) -> TextStyle {
    let mut style = window.text_style();
    style.font_weight = FontWeight::NORMAL;
    style
}

fn line_metrics(window: &Window) -> LineMetrics {
    let style = diff_text_style(window);
    let font_size = style.font_size.to_pixels(window.rem_size()) * DIFF_FONT_SCALE;
    let line_height = style
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    LineMetrics {
        font_size,
        line_height,
    }
}

fn center_text_y(bounds: Bounds<Pixels>, line_height: Pixels) -> Pixels {
    let extra = (bounds.size.height - line_height).max(px(0.0));
    bounds.top() + extra * 0.5
}

fn px_2(window: &Window) -> Pixels {
    window.rem_size() * 0.5
}

fn diff_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

fn diff_row_height(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_ROW_HEIGHT_PX, ui_scale_percent)
}

fn gutter_cell_total_width(pad: Pixels, ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_GUTTER_BASE_WIDTH_PX, ui_scale_percent) + pad * 2.0
}

fn inline_text_bounds(bounds: Bounds<Pixels>, gutter_total: Pixels, pad: Pixels) -> Bounds<Pixels> {
    let left = bounds.left() + gutter_total * 2.0 + pad;
    let width = (bounds.size.width - gutter_total * 2.0 - pad * 2.0).max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

fn single_column_text_bounds(
    bounds: Bounds<Pixels>,
    gutter_total: Pixels,
    pad: Pixels,
) -> Bounds<Pixels> {
    let left = bounds.left() + gutter_total + pad;
    let width = (bounds.size.width - gutter_total - pad * 2.0).max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

fn split_columns(bounds: Bounds<Pixels>) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    let sep = px(1.0);
    let total_w = bounds.size.width.max(px(0.0));
    let inner_w = (total_w - sep).max(px(0.0));
    let left_w = (inner_w * 0.5).floor();
    let right_w = (inner_w - left_w).max(px(0.0));
    let left = Bounds::new(bounds.origin, size(left_w, bounds.size.height));
    let sep_bounds = Bounds::new(
        point(bounds.left() + left_w, bounds.top()),
        size(sep, bounds.size.height),
    );
    let right = Bounds::new(
        point(bounds.left() + left_w + sep, bounds.top()),
        size(right_w, bounds.size.height),
    );
    (left, sep_bounds, right)
}

fn column_text_bounds(col: Bounds<Pixels>, gutter_total: Pixels, pad: Pixels) -> Bounds<Pixels> {
    single_column_text_bounds(col, gutter_total, pad)
}

fn paint_gutter_text(
    text: &SharedString,
    x: Pixels,
    y: Pixels,
    color: gpui::Rgba,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let mut style = diff_text_style(window);
    style.color = color.into();
    let key = {
        let mut hasher = FxHasher::default();
        text.as_ref().hash(&mut hasher);
        metrics.font_size.hash(&mut hasher);
        style.font_family.hash(&mut hasher);
        style.font_weight.hash(&mut hasher);
        color.r.to_bits().hash(&mut hasher);
        color.g.to_bits().hash(&mut hasher);
        color.b.to_bits().hash(&mut hasher);
        color.a.to_bits().hash(&mut hasher);
        hasher.finish()
    };

    let shaped = GUTTER_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned());
    let shaped = shaped.unwrap_or_else(|| {
        let run = style.to_run(text.len());
        let shaped = window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None);

        GUTTER_TEXT_LAYOUT_CACHE.with(|cache| {
            cache.borrow_mut().put(key, shaped.clone());
        });

        shaped
    });
    let _ = shaped.paint(
        point(x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_selectable_diff_text(
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    region: DiffTextRegion,
    bounds: Bounds<Pixels>,
    text: &SharedString,
    highlights: &Arc<[(Range<usize>, HighlightStyle)]>,
    streamed_spec: Option<&StreamedDiffTextPaintSpec>,
    row_bg: Option<gpui::Rgba>,
    highlights_hash: u64,
    text_hash: u64,
    y: Pixels,
    base_fg: gpui::Rgba,
    metrics: LineMetrics,
    ui_scale_percent: u32,
    theme: AppTheme,
    window: &mut Window,
    cx: &mut App,
) {
    let mut base_style = diff_text_style(window);
    base_style.color = base_fg.into();
    base_style.white_space = gpui::WhiteSpace::Nowrap;
    base_style.text_overflow = None;

    let pad = px_2(window);
    let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
    let row_extra = match region {
        DiffTextRegion::Inline => gutter_total * 2.0 + pad * 2.0,
        DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => gutter_total + pad * 2.0,
    };
    let total_text_len = streamed_spec
        .filter(|spec| should_stream_diff_text(Some(spec)))
        .map(|spec| spec.raw_text.len())
        .unwrap_or_else(|| text.len());
    let selection =
        view.read(cx)
            .diff_text_local_selection_range(visible_ix, region, total_text_len);

    let mut streamed_styled = None;
    let mut streamed_slice_range = None;
    let mut paint_x = bounds.left();
    let mut hitbox_cell_width = None;
    let mut pending_prepared_syntax = false;

    let (layout_key, layout, shaped_new, required_row_w) = if let Some(spec) =
        streamed_spec.filter(|spec| should_stream_diff_text(Some(spec)))
    {
        let cell_width =
            streamed_diff_text_ascii_cell_width(&base_style, metrics.font_size, window);
        let clip_bounds = window.content_mask().bounds;
        let overscan_columns = STREAMED_DIFF_TEXT_OVERSCAN_COLUMNS.max(spec.query.as_ref().len());
        let slice_range = streamed_diff_text_visible_slice_range(
            bounds,
            clip_bounds,
            spec.raw_text.len(),
            cell_width,
            overscan_columns,
        );
        let (slice_styled, pending, resolved_slice_range) =
            build_streamed_diff_slice_styled_text(theme, spec, &slice_range);
        let (layout_key, layout, shaped_new) = ensure_layout_cached(
            view,
            slice_styled.text_hash,
            &slice_styled.text,
            &base_style,
            base_fg,
            slice_styled.highlights.as_ref(),
            slice_styled.highlights_hash,
            metrics,
            window,
            cx,
        );
        paint_x = bounds.left() + cell_width * resolved_slice_range.start as f32;
        hitbox_cell_width = Some(cell_width);
        pending_prepared_syntax = pending;
        streamed_slice_range = Some(resolved_slice_range);
        let required_row_w = (row_extra
            + cell_width * spec.raw_text.len() as f32
            + diff_scaled_px(DIFF_ROW_TEXT_TRAILING_PADDING_PX, ui_scale_percent))
        .round();
        streamed_styled = Some(slice_styled);
        (layout_key, layout, shaped_new, required_row_w)
    } else {
        let (layout_key, layout, shaped_new) = ensure_layout_cached(
            view,
            text_hash,
            text,
            &base_style,
            base_fg,
            highlights.as_ref(),
            highlights_hash,
            metrics,
            window,
            cx,
        );
        let required_row_w = (row_extra
            + layout.width
            + diff_scaled_px(DIFF_ROW_TEXT_TRAILING_PADDING_PX, ui_scale_percent))
        .round();
        (layout_key, layout, shaped_new, required_row_w)
    };

    let paint_text = streamed_styled
        .as_ref()
        .map(|styled| &styled.text)
        .unwrap_or(text);
    let paint_highlights = streamed_styled
        .as_ref()
        .map(|styled| styled.highlights.as_ref())
        .unwrap_or_else(|| highlights.as_ref());

    #[cfg(test)]
    record_diff_paint_for_tests(visible_ix, region, paint_text, paint_highlights, row_bg);
    #[cfg(not(test))]
    let _ = row_bg;

    if let Some(r) = selection {
        let (x0, x1) = if let Some(cell_width) = hitbox_cell_width {
            let start = streamed_slice_range
                .as_ref()
                .map(|slice_range| r.start.max(slice_range.start))
                .unwrap_or(r.start)
                .min(total_text_len);
            let end = streamed_slice_range
                .as_ref()
                .map(|slice_range| r.end.min(slice_range.end))
                .unwrap_or(r.end)
                .min(total_text_len);
            (cell_width * start as f32, cell_width * end as f32)
        } else {
            (
                layout.x_for_index(r.start.min(total_text_len)),
                layout.x_for_index(r.end.min(total_text_len)),
            )
        };

        if x1 > x0 {
            let color = view.read(cx).diff_text_selection_color();
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left() + x0, bounds.top()),
                    point(bounds.left() + x1, bounds.bottom()),
                ),
                color,
            ));
        }
    }

    let hitbox = DiffTextHitbox {
        bounds,
        layout_key,
        text_len: total_text_len,
        streamed_ascii_monospace_cell_width: hitbox_cell_width,
    };

    view.update(cx, |this, cx| {
        this.set_diff_text_hitbox(visible_ix, region, hitbox);
        this.touch_diff_text_layout_cache(layout_key, shaped_new);
        if pending_prepared_syntax {
            this.ensure_prepared_syntax_chunk_poll(cx);
        }
        if required_row_w > this.diff_horizontal_min_width {
            this.diff_horizontal_min_width = required_row_w;
            cx.notify();
        }
    });

    if paint_text.is_empty() {
        return;
    }

    if paint_highlights.is_empty() {
        let _ = layout.paint(
            point(paint_x, y),
            metrics.line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
        return;
    }

    let _ = layout.paint_background(
        point(paint_x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
    let _ = layout.paint(
        point(paint_x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn diff_layout_base_key(
    text_hash: u64,
    base_style: &TextStyle,
    base_fg: gpui::Rgba,
    metrics: LineMetrics,
) -> u64 {
    let mut hasher = FxHasher::default();
    text_hash.hash(&mut hasher);
    metrics.font_size.hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_weight.hash(&mut hasher);
    base_fg.r.to_bits().hash(&mut hasher);
    base_fg.g.to_bits().hash(&mut hasher);
    base_fg.b.to_bits().hash(&mut hasher);
    base_fg.a.to_bits().hash(&mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
fn ensure_layout_cached(
    view: &Entity<MainPaneView>,
    text_hash: u64,
    text: &SharedString,
    base_style: &TextStyle,
    base_fg: gpui::Rgba,
    highlights: &[(Range<usize>, HighlightStyle)],
    highlights_hash: u64,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) -> (u64, gpui::ShapedLine, Option<gpui::ShapedLine>) {
    let base_key = diff_layout_base_key(text_hash, base_style, base_fg, metrics);

    let layout_key = if highlights.is_empty() {
        base_key
    } else {
        let mut hasher = FxHasher::default();
        base_key.hash(&mut hasher);
        highlights_hash.hash(&mut hasher);
        highlights.len().hash(&mut hasher);
        hasher.finish()
    };

    if let Some(entry) = view.read(cx).diff_text_layout_cache.get(&layout_key) {
        return (layout_key, entry.layout.clone(), None);
    }

    let shaped = if highlights.is_empty() {
        let run = base_style.to_run(text.len());
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None)
    } else {
        let runs = compute_runs(text.as_ref(), base_style, highlights);
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &runs, None)
    };
    (layout_key, shaped.clone(), Some(shaped))
}

fn compute_runs(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.len()),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

fn empty_highlights() -> HighlightSpans {
    static EMPTY: OnceLock<HighlightSpans> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(r: f32, g: f32, b: f32) -> gpui::Rgba {
        gpui::Rgba { r, g, b, a: 1.0 }
    }

    fn test_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn should_handle_row_mouse_event_requires_bubble_phase_and_in_bounds() {
        let row_bounds = test_bounds(10.0, 20.0, 50.0, 10.0);
        let inside = point(px(20.0), px(25.0));
        let outside = point(px(200.0), px(25.0));

        assert!(should_handle_row_mouse_event(
            DispatchPhase::Bubble,
            &row_bounds,
            inside,
        ));
        assert!(!should_handle_row_mouse_event(
            DispatchPhase::Capture,
            &row_bounds,
            inside,
        ));
        assert!(!should_handle_row_mouse_event(
            DispatchPhase::Bubble,
            &row_bounds,
            outside,
        ));
    }

    #[test]
    fn diff_row_text_regions_single_only_hits_inside_text() {
        let regions =
            DiffRowTextRegions::single(DiffTextRegion::Inline, test_bounds(5.0, 5.0, 20.0, 10.0));

        assert_eq!(
            regions.region_at(point(px(10.0), px(10.0))),
            Some(DiffTextRegion::Inline)
        );
        assert_eq!(regions.region_at(point(px(1.0), px(10.0))), None);
    }

    #[test]
    fn diff_row_text_regions_split_maps_left_and_right_regions() {
        let regions = DiffRowTextRegions::split(
            test_bounds(0.0, 0.0, 40.0, 20.0),
            test_bounds(41.0, 0.0, 40.0, 20.0),
        );

        assert_eq!(
            regions.region_at(point(px(10.0), px(10.0))),
            Some(DiffTextRegion::SplitLeft)
        );
        assert_eq!(
            regions.region_at(point(px(60.0), px(10.0))),
            Some(DiffTextRegion::SplitRight)
        );
        assert_eq!(regions.region_at(point(px(40.5), px(10.0))), None);
    }

    #[test]
    fn inline_row_canvas_revision_key_tracks_rendered_payload() {
        let base = inline_row_canvas_revision_key(
            &"1".into(),
            &"2".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            11,
            17,
        );

        assert_eq!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"2".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                11,
                17,
            )
        );
        assert_ne!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"3".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                11,
                17,
            )
        );
        assert_ne!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"2".into(),
                rgba(1.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                11,
                17,
            )
        );
        assert_ne!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"2".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                12,
                17,
            )
        );
    }

    #[test]
    fn split_row_canvas_revision_key_tracks_both_sides() {
        let base = split_row_canvas_revision_key(
            &"10".into(),
            &"20".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            3,
            5,
            7,
            11,
        );

        assert_ne!(
            base,
            split_row_canvas_revision_key(
                &"10".into(),
                &"20".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                4,
                5,
                7,
                11,
            )
        );
        assert_ne!(
            base,
            split_row_canvas_revision_key(
                &"10".into(),
                &"20".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                3,
                5,
                7,
                11,
            )
        );
        assert_ne!(
            base,
            split_row_canvas_revision_key(
                &"10".into(),
                &"21".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                3,
                5,
                7,
                11,
            )
        );
    }

    #[test]
    fn patch_split_row_canvas_revision_key_tracks_line_number_and_style() {
        let base = patch_split_row_canvas_revision_key(
            &"42".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            13,
            17,
        );

        assert_ne!(
            base,
            patch_split_row_canvas_revision_key(
                &"43".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                13,
                17,
            )
        );
        assert_ne!(
            base,
            patch_split_row_canvas_revision_key(
                &"42".into(),
                rgba(0.0, 1.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                13,
                17,
            )
        );
        assert_ne!(
            base,
            patch_split_row_canvas_revision_key(
                &"42".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                14,
                17,
            )
        );
    }
}
