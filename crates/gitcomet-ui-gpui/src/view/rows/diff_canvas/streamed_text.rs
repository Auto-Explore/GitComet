use super::super::canvas_text::HighlightSpans;
use super::super::canvas_text::empty_highlights;
use super::super::diff_text::PreparedDocumentByteRangeHighlights;
use super::super::diff_text::build_cached_diff_query_overlay_styled_text;
use super::super::diff_text::build_cached_diff_styled_text;
use super::super::diff_text::build_cached_diff_styled_text_from_relative_highlights;
use super::super::diff_text::hash_rgba_bits as hash_rgba;
use super::super::diff_text::slice_cached_diff_styled_text;
use super::super::diff_text::syntax_highlights_for_streamed_line_slice_heuristic;
use super::super::diff_text::whitespace_visible_line_styled_text;
use super::super::diff_text::whitespace_visible_line_styled_text_for_raw;
use super::super::diff_text::whitespace_visible_line_text;
use super::super::*;
use super::DiffTextWrapSlice;
use super::STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE;
use super::StreamedDiffTextPaintSpec;
use super::StreamedDiffTextSyntaxSource;
use super::is_streamable_diff_text;
use super::whitespace_visible_diff_offset_map;
use crate::view::panes::diff::diff_search::DiffSearchOptions;
use gpui::Bounds;
use gpui::HighlightStyle;
use gpui::Pixels;
use gpui::TextStyle;
use gpui::Window;
use gpui::px;
use palette::IntoColor;
use rustc_hash::FxHasher;
use std::borrow::Cow;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Range;
use std::sync::Arc;

pub(super) const STREAMED_DIFF_TEXT_MIN_BYTES: usize = LARGE_DIFF_TEXT_MIN_BYTES;

pub(super) const STREAMED_DIFF_TEXT_OVERSCAN_COLUMNS: usize = 64;

const STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE: &str = "0000000000";

pub(super) struct DiffTextPaintPayload {
    pub(super) text: SharedString,
    pub(super) highlights: HighlightSpans,
    pub(super) highlights_hash: u64,
    pub(super) text_hash: u64,
    pub(super) offset_map: Option<DiffTextOffsetMap>,
}

pub(super) fn should_stream_diff_text(spec: Option<&StreamedDiffTextPaintSpec>) -> bool {
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

pub(super) fn streamed_diff_text_ascii_cell_width(
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

pub(super) fn streamed_diff_text_visible_slice_range(
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

fn streamed_diff_text_visible_text_hash(
    spec: &StreamedDiffTextPaintSpec,
    reveal_whitespace_chars: bool,
) -> u64 {
    let base = streamed_diff_text_text_hash(spec);
    if !reveal_whitespace_chars {
        return base;
    }

    let mut hasher = FxHasher::default();
    base.hash(&mut hasher);
    reveal_whitespace_chars.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn whitespace_marker_len(ch: char) -> usize {
    match ch {
        ' ' => '\u{00B7}'.len_utf8(),
        '\t' => '\u{2192}'.len_utf8(),
        '\r' => '\u{240D}'.len_utf8(),
        '\n' => '\u{21B5}'.len_utf8(),
        _ if ch.is_whitespace() => '\u{2420}'.len_utf8(),
        _ => ch.len_utf8(),
    }
}

fn display_range_for_source_range(
    map: &DiffTextOffsetMap,
    source_range: &Range<usize>,
) -> Range<usize> {
    let source_start = source_range.start.min(map.source_len());
    let source_end = source_range.end.min(map.source_len());
    let display_start = map.display_offset_for_source(source_start);
    let display_end = if source_end >= map.source_len() {
        map.display_len()
    } else {
        map.display_offset_for_source(source_end)
    };
    display_start.min(map.display_len())..display_end.min(map.display_len())
}

fn slice_diff_text_offset_map(
    map: &DiffTextOffsetMap,
    display_range: Range<usize>,
    source_range: Range<usize>,
) -> DiffTextOffsetMap {
    let display_start = display_range.start.min(map.display_len());
    let display_end = display_range.end.min(map.display_len()).max(display_start);
    let source_start = source_range.start.min(map.source_len());
    let source_end = source_range.end.min(map.source_len()).max(source_start);
    let display_len = display_end.saturating_sub(display_start);
    let source_len = source_end.saturating_sub(source_start);

    let display_to_source = (0..=display_len)
        .map(|offset| {
            map.source_offset_for_display(display_start.saturating_add(offset))
                .clamp(source_start, source_end)
                .saturating_sub(source_start)
        })
        .collect::<Vec<_>>();
    let source_to_display = (0..=source_len)
        .map(|offset| {
            map.display_offset_for_source(source_start.saturating_add(offset))
                .clamp(display_start, display_end)
                .saturating_sub(display_start)
        })
        .collect::<Vec<_>>();

    DiffTextOffsetMap {
        display_to_source: Arc::from(display_to_source),
        source_to_display: Arc::from(source_to_display),
    }
}

fn streamed_diff_text_highlights_hash(spec: &StreamedDiffTextPaintSpec) -> u64 {
    let mut hasher = FxHasher::default();
    spec.query.as_ref().hash(&mut hasher);
    spec.query_options.hash(&mut hasher);
    spec.query_emphasis.hash(&mut hasher);
    for range in spec.word_ranges.iter() {
        hash_range(&mut hasher, range);
    }
    spec.word_kind.hash(&mut hasher);
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
    foreground_color: Option<gpui::Hsla>,
) -> u64 {
    let mut hasher = FxHasher::default();
    base_highlights_hash.hash(&mut hasher);
    hash_rgba(&mut hasher, background_color.into_color());
    if let Some(foreground_color) = foreground_color {
        hash_rgba(&mut hasher, foreground_color.into_color());
    }
    for range in ranges {
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
    }
    hasher.finish()
}

/// Lays a semantic overlay -- the word-diff wash -- over already-styled text.
///
/// `foreground_color` is the text colour the wash pins under itself, and it is
/// applied only where nothing has coloured the run already: light themes carry
/// the wash opaque, which would drown a syntax colour, so the diff foreground
/// takes over there and syntax keeps its own everywhere else. That is the rule
/// the non-streamed builder follows, and both paths render the same hunk.
fn overlay_background_ranges_on_styled_text(
    base: &CachedDiffStyledText,
    ranges: &[Range<usize>],
    background_color: gpui::Hsla,
    foreground_color: Option<gpui::Hsla>,
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
                    color: foreground_color,
                    background_color: Some(background_color),
                    ..HighlightStyle::default()
                },
            );
        }
        return CachedDiffStyledText {
            text: base.text.clone(),
            highlights: Arc::from(merged),
            highlights_hash: hash_overlay_ranges(
                base.highlights_hash,
                ranges,
                background_color,
                foreground_color,
            ),
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
            if style.color.is_none() {
                style.color = foreground_color;
            }
        }

        if style != default_style {
            push_or_extend_highlight(&mut merged, cursor..next_boundary, style);
        }

        cursor = next_boundary;
    }

    CachedDiffStyledText {
        text: base.text.clone(),
        highlights: Arc::from(merged),
        highlights_hash: hash_overlay_ranges(
            base.highlights_hash,
            ranges,
            background_color,
            foreground_color,
        ),
        text_hash: base.text_hash,
    }
}

fn should_apply_query_overlay_to_streamed_slice(
    options: DiffSearchOptions,
    slice_range: &Range<usize>,
    total_len: usize,
) -> bool {
    if !options.whole_word && !options.regex {
        return true;
    }

    slice_range.start == 0 && slice_range.end >= total_len
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

pub(super) fn build_streamed_diff_slice_styled_text(
    tab_width: usize,
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
            tab_width,
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
                    tab_width,
                    slice_text_ref,
                    highlights.as_slice(),
                ),
                None => build_cached_diff_styled_text(
                    tab_width,
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
                    if relative.is_empty() {
                        match syntax_highlights_for_streamed_line_slice_heuristic(
                            theme,
                            &spec.raw_text,
                            *language,
                            requested_slice_range.clone(),
                            resolved_slice_range.clone(),
                        ) {
                            Some(highlights) => {
                                build_cached_diff_styled_text_from_relative_highlights(
                                    tab_width,
                                    slice_text_ref,
                                    highlights.as_slice(),
                                )
                            }
                            None => build_cached_diff_styled_text(
                                tab_width,
                                theme,
                                slice_text_ref,
                                &[],
                                "",
                                Some(*language),
                                rows::DiffSyntaxMode::HeuristicOnly,
                                None,
                            ),
                        }
                    } else {
                        build_cached_diff_styled_text_from_relative_highlights(
                            tab_width,
                            slice_text_ref,
                            relative.as_slice(),
                        )
                    }
                }
                None => build_cached_diff_styled_text(
                    tab_width,
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
        && let Some(word_kind) = spec.word_kind
    {
        let clipped = clip_ranges_to_slice(spec.word_ranges.as_ref(), &resolved_slice_range);
        if !clipped.is_empty() {
            // The same resolver the non-streamed builder uses, and both halves of
            // what it returns. Deriving the wash here instead gave a line past
            // `STREAMED_DIFF_TEXT_MIN_BYTES` a different word-diff colour from
            // its neighbours in the same diff; dropping the foreground did the
            // same to the text on light themes, which pin it under the wash.
            let (background, foreground) = diff_text::word_highlight_colors(theme, word_kind);
            base = overlay_background_ranges_on_styled_text(
                &base,
                clipped.as_slice(),
                background.into_color(),
                foreground.map(IntoColor::into_color),
            );
        }
    }

    if let Some(matcher) = spec.query_matcher.as_deref()
        && should_apply_query_overlay_to_streamed_slice(
            spec.query_options,
            &resolved_slice_range,
            spec.raw_text.len(),
        )
    {
        base =
            build_cached_diff_query_overlay_styled_text(theme, &base, matcher, spec.query_emphasis);
    }

    (base, pending, resolved_slice_range)
}

const DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES: usize = 8_192;

thread_local! {
    /// Whitespace-revealed text plus its offset map, keyed by the source
    /// styled text. Both were rebuilt for every visible row on every frame
    /// while "reveal whitespace" was on.
    static WHITESPACE_VISIBLE_TEXT_CACHE: std::cell::RefCell<
        super::FxLruCache<u64, (CachedDiffStyledText, DiffTextOffsetMap)>,
    > = std::cell::RefCell::new(super::new_fx_lru_cache(DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES));
    /// Word-wrap slices keyed by (source styled text, display range): a
    /// substring plus clipped highlights per visible wrapped row per frame.
    static WRAP_SLICE_CACHE: std::cell::RefCell<super::FxLruCache<u64, CachedDiffStyledText>> =
        std::cell::RefCell::new(super::new_fx_lru_cache(DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES));
}

fn whitespace_visible_cached(
    tab_width: usize,
    styled: &CachedDiffStyledText,
    raw_text: Option<&str>,
) -> (CachedDiffStyledText, DiffTextOffsetMap) {
    let key = {
        let mut hasher = FxHasher::default();
        0u8.hash(&mut hasher);
        tab_width.hash(&mut hasher);
        styled.text_hash.hash(&mut hasher);
        styled.highlights_hash.hash(&mut hasher);
        match raw_text {
            Some(raw_text) => {
                true.hash(&mut hasher);
                raw_text.hash(&mut hasher);
            }
            None => false.hash(&mut hasher),
        }
        hasher.finish()
    };
    if let Some(hit) =
        WHITESPACE_VISIBLE_TEXT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned())
    {
        return hit;
    }
    let value = match raw_text {
        Some(raw_text) => (
            whitespace_visible_line_styled_text_for_raw(tab_width, styled, raw_text),
            whitespace_visible_diff_offset_map(tab_width, raw_text, true),
        ),
        None => (
            whitespace_visible_line_styled_text(styled),
            whitespace_visible_diff_offset_map(tab_width, styled.text.as_ref(), true),
        ),
    };
    WHITESPACE_VISIBLE_TEXT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, value.clone());
    });
    value
}

fn whitespace_visible_raw_cached(
    tab_width: usize,
    raw_text: &str,
) -> (CachedDiffStyledText, DiffTextOffsetMap) {
    let key = {
        let mut hasher = FxHasher::default();
        1u8.hash(&mut hasher);
        tab_width.hash(&mut hasher);
        raw_text.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(hit) =
        WHITESPACE_VISIBLE_TEXT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned())
    {
        return hit;
    }
    let offset_map = whitespace_visible_diff_offset_map(tab_width, raw_text, true);
    let text = whitespace_visible_line_text(raw_text);
    let text_hash = {
        let mut hasher = FxHasher::default();
        text.as_ref().hash(&mut hasher);
        hasher.finish()
    };
    let value = (
        CachedDiffStyledText {
            text,
            highlights: empty_highlights(),
            highlights_hash: 0,
            text_hash,
        },
        offset_map,
    );
    WHITESPACE_VISIBLE_TEXT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, value.clone());
    });
    value
}

fn slice_cached_diff_styled_text_cached(
    styled: &CachedDiffStyledText,
    range: Range<usize>,
) -> CachedDiffStyledText {
    let key = {
        let mut hasher = FxHasher::default();
        styled.text_hash.hash(&mut hasher);
        styled.highlights_hash.hash(&mut hasher);
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(hit) = WRAP_SLICE_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned()) {
        return hit;
    }
    let sliced = slice_cached_diff_styled_text(styled, range);
    WRAP_SLICE_CACHE.with(|cache| {
        cache.borrow_mut().put(key, sliced.clone());
    });
    sliced
}

pub(super) fn diff_text_paint_payload(
    tab_width: usize,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<&StreamedDiffTextPaintSpec>,
    raw_text: Option<&str>,
    reveal_whitespace_chars: bool,
    region: DiffTextRegion,
    wrap: Option<DiffTextWrapSlice>,
) -> DiffTextPaintPayload {
    if reveal_whitespace_chars {
        if should_stream_diff_text(streamed_spec) {
            let spec = streamed_spec.expect("streamed spec checked above");
            return DiffTextPaintPayload {
                text: SharedString::default(),
                highlights: empty_highlights(),
                highlights_hash: streamed_diff_text_highlights_hash(spec),
                text_hash: streamed_diff_text_visible_text_hash(spec, true),
                offset_map: None,
            };
        }

        let mut offset_map: Option<DiffTextOffsetMap> = None;
        let styled = if let Some(styled) = styled {
            let (visible, map) = whitespace_visible_cached(tab_width, styled, raw_text);
            offset_map = Some(map);
            Some(visible)
        } else if let Some(spec) = streamed_spec {
            let (visible, map) = whitespace_visible_raw_cached(tab_width, spec.raw_text.as_ref());
            offset_map = Some(map);
            Some(visible)
        } else {
            None
        };

        let wrapped;
        let styled = if let (Some(styled), Some(wrap)) = (styled.as_ref(), wrap) {
            let source_range = wrap.range_for_region(region);
            let display_range = offset_map
                .as_ref()
                .map(|map| display_range_for_source_range(map, &source_range))
                .unwrap_or_else(|| source_range.clone());
            wrapped = slice_cached_diff_styled_text_cached(styled, display_range.clone());
            offset_map = offset_map
                .as_ref()
                .map(|map| slice_diff_text_offset_map(map, display_range, source_range));
            Some(&wrapped)
        } else {
            styled.as_ref()
        };
        let text = styled.map(|s| s.text.clone()).unwrap_or_default();
        let highlights = styled
            .map(|s| Arc::clone(&s.highlights))
            .unwrap_or_else(empty_highlights);
        let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
        let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);
        return DiffTextPaintPayload {
            text,
            highlights,
            highlights_hash,
            text_hash,
            offset_map,
        };
    }

    if should_stream_diff_text(streamed_spec) {
        let spec = streamed_spec.expect("streamed spec checked above");
        return DiffTextPaintPayload {
            text: SharedString::default(),
            highlights: empty_highlights(),
            highlights_hash: streamed_diff_text_highlights_hash(spec),
            text_hash: streamed_diff_text_visible_text_hash(spec, false),
            offset_map: None,
        };
    }

    let wrapped;
    let styled = if let (Some(styled), Some(wrap)) = (styled, wrap) {
        wrapped = slice_cached_diff_styled_text_cached(styled, wrap.range_for_region(region));
        Some(&wrapped)
    } else {
        styled
    };
    let text = styled.map(|s| s.text.clone()).unwrap_or_default();
    let highlights = styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
    let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);
    DiffTextPaintPayload {
        text,
        highlights,
        highlights_hash,
        text_hash,
        offset_map: None,
    }
}

#[cfg(test)]
mod tests;
