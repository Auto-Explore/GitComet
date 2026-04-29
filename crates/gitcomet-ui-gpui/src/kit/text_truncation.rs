use gpui::{
    App, HighlightStyle, Hsla, Pixels, ShapedLine, SharedString, TextRun, TextStyle, Window,
};
use lru::LruCache;
use rustc_hash::FxHasher;
#[cfg(any(test, feature = "benchmarks"))]
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;

mod candidate;
mod path;
mod path_alignment;
mod projection;

use candidate::{TruncatedCandidateParams, build_truncated_candidate};
pub(crate) use path_alignment::PathTruncationAlignmentGroup;
pub(crate) use projection::{TruncationProjection, truncated_line_ellipsis_x};

pub(crate) const TRUNCATION_ELLIPSIS: &str = "…";
const TRUNCATED_LAYOUT_CACHE_MAX_ENTRIES: usize = 8_192;

type FxLruCache<K, V> = LruCache<K, V, BuildHasherDefault<FxHasher>>;

thread_local! {
    static TRUNCATED_LAYOUT_CACHE: RefCell<FxLruCache<TruncatedLayoutCacheKey, Arc<TruncatedLineLayout>>> =
        RefCell::new(FxLruCache::with_hasher(
            NonZeroUsize::new(TRUNCATED_LAYOUT_CACHE_MAX_ENTRIES)
                .expect("truncated layout cache capacity must be > 0"),
            BuildHasherDefault::default(),
        ));
}

#[cfg(any(test, feature = "benchmarks"))]
thread_local! {
    static MEASURE_CANDIDATE_CALLS_FOR_TEST: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TextTruncationProfile {
    End,
    Middle,
    Path,
}

#[derive(Clone, Debug)]
pub(crate) struct TruncatedLineLayout {
    #[cfg(test)]
    pub(crate) display_text: SharedString,
    pub(crate) shaped_line: ShapedLine,
    pub(crate) projection: Arc<TruncationProjection>,
    pub(crate) truncated: bool,
    pub(crate) line_height: Pixels,
    pub(crate) has_background_runs: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TruncatedLayoutCacheKey {
    text_hash: u64,
    max_width_key: Option<u32>,
    path_ellipsis_anchor_key: Option<u32>,
    font_size_bits: u32,
    line_height_bits: u32,
    font_family: SharedString,
    font_features_hash: u64,
    font_fallbacks_hash: u64,
    font_weight_bits: u32,
    font_style_hash: u64,
    color_hash: u64,
    background_hash: u64,
    underline_hash: u64,
    strikethrough_hash: u64,
    profile: TextTruncationProfile,
    highlights_hash: u64,
    focus_hash: u64,
}

fn normalize_highlights_if_needed(
    text_len: usize,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
    if highlights.is_empty() {
        return None;
    }

    let mut prev_start = 0usize;
    let mut prev_end = 0usize;
    let mut needs_normalization = false;

    for (ix, (range, _)) in highlights.iter().enumerate() {
        let start = range.start.min(text_len);
        let end = range.end.min(text_len);
        if start >= end || start != range.start || end != range.end {
            needs_normalization = true;
            break;
        }
        if ix > 0
            && (start < prev_start || (start == prev_start && end < prev_end) || start < prev_end)
        {
            needs_normalization = true;
            break;
        }
        prev_start = start;
        prev_end = end;
    }

    if !needs_normalization {
        return None;
    }

    let mut normalized: Vec<_> = highlights
        .iter()
        .filter_map(|(range, style)| {
            let start = range.start.min(text_len);
            let end = range.end.min(text_len);
            (start < end).then_some((start..end, *style))
        })
        .collect();
    normalized.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    // Canonicalize to strictly increasing spans so run generation stays valid.
    let mut cursor = 0usize;
    let mut write_ix = 0usize;
    for read_ix in 0..normalized.len() {
        let (range, style) = normalized[read_ix].clone();
        let start = range.start.max(cursor);
        if start >= range.end {
            continue;
        }
        normalized[write_ix] = (start..range.end, style);
        cursor = range.end;
        write_ix += 1;
    }
    normalized.truncate(write_ix);
    Some(normalized)
}

pub(crate) fn shape_truncated_line_cached(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    max_width: Option<Pixels>,
    profile: TextTruncationProfile,
    highlights: &[(Range<usize>, HighlightStyle)],
    focus_range: Option<Range<usize>>,
) -> Arc<TruncatedLineLayout> {
    shape_truncated_line_cached_with_path_anchor(
        window,
        cx,
        base_style,
        text,
        max_width,
        profile,
        highlights,
        focus_range,
        None,
    )
}

pub(crate) fn shape_truncated_line_cached_with_path_anchor(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    max_width: Option<Pixels>,
    profile: TextTruncationProfile,
    highlights: &[(Range<usize>, HighlightStyle)],
    focus_range: Option<Range<usize>>,
    path_ellipsis_anchor: Option<Pixels>,
) -> Arc<TruncatedLineLayout> {
    let normalized_highlights = normalize_highlights_if_needed(text.len(), highlights);
    let highlights = normalized_highlights.as_deref().unwrap_or(highlights);
    let normalized_focus_range = normalized_focus_range(text.as_ref(), focus_range);
    let path_ellipsis_anchor = if matches!(profile, TextTruncationProfile::Path) {
        path_ellipsis_anchor
    } else {
        None
    };
    let font_size = base_style.font_size.to_pixels(window.rem_size());
    let line_height = base_style.line_height_in_pixels(window.rem_size());
    let key = TruncatedLayoutCacheKey {
        text_hash: hash_value(text.as_ref()),
        max_width_key: max_width.map(width_cache_key),
        path_ellipsis_anchor_key: path_ellipsis_anchor.map(width_cache_key),
        font_size_bits: f32::from(font_size).to_bits(),
        line_height_bits: f32::from(line_height).to_bits(),
        font_family: base_style.font_family.clone(),
        font_features_hash: hash_value(&base_style.font_features),
        font_fallbacks_hash: hash_value(&base_style.font_fallbacks),
        font_weight_bits: base_style.font_weight.0.to_bits(),
        font_style_hash: hash_value(&base_style.font_style),
        color_hash: hash_color(base_style.color),
        background_hash: hash_value(&base_style.background_color),
        underline_hash: hash_value(&base_style.underline),
        strikethrough_hash: hash_value(&base_style.strikethrough),
        profile,
        highlights_hash: hash_highlights(highlights),
        focus_hash: hash_focus_range(normalized_focus_range.as_ref()),
    };

    if let Some(entry) = TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned())
    {
        return entry;
    }

    let layout = Arc::new(shape_truncated_line_uncached(
        window,
        cx,
        base_style,
        TruncatedLineParams {
            text,
            max_width,
            profile,
            highlights,
            normalized_focus_range,
            path_ellipsis_anchor,
            font_size,
            line_height,
        },
    ));

    TRUNCATED_LAYOUT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, Arc::clone(&layout));
    });

    layout
}

struct TruncatedLineParams<'a> {
    text: &'a SharedString,
    max_width: Option<Pixels>,
    profile: TextTruncationProfile,
    highlights: &'a [(Range<usize>, HighlightStyle)],
    normalized_focus_range: Option<Range<usize>>,
    path_ellipsis_anchor: Option<Pixels>,
    font_size: Pixels,
    line_height: Pixels,
}

fn shape_truncated_line_uncached(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    params: TruncatedLineParams<'_>,
) -> TruncatedLineLayout {
    let TruncatedLineParams {
        text,
        max_width,
        profile,
        highlights,
        normalized_focus_range,
        path_ellipsis_anchor,
        font_size,
        line_height,
    } = params;

    let candidate = build_truncated_candidate(
        window,
        cx,
        base_style,
        TruncatedCandidateParams {
            text,
            max_width,
            profile,
            highlights,
            normalized_focus_range,
            path_ellipsis_anchor,
            font_size,
        },
    );
    let has_background_runs = base_style.background_color.is_some()
        || candidate
            .display_highlights
            .iter()
            .any(|(_, highlight)| highlight.background_color.is_some());
    let runs = compute_highlight_runs(
        &candidate.display_text,
        base_style,
        &candidate.display_highlights,
    );
    let shaped_line =
        window
            .text_system()
            .shape_line(candidate.display_text.clone(), font_size, &runs, None);

    TruncatedLineLayout {
        #[cfg(test)]
        display_text: candidate.display_text,
        shaped_line,
        projection: candidate.projection,
        truncated: candidate.truncated,
        line_height,
        has_background_runs,
    }
}

fn compute_highlight_runs(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<TextRun> {
    if highlights.is_empty() {
        return vec![default_style.to_run(text.len())];
    }

    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        let start = range.start.min(text.len()).max(ix);
        let end = range.end.min(text.len());
        if ix < start {
            runs.push(default_style.clone().to_run(start - ix));
        }
        if start >= end {
            continue;
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(end - start),
        );
        ix = end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

fn normalized_focus_range(text: &str, focus_range: Option<Range<usize>>) -> Option<Range<usize>> {
    let focus_range = focus_range?;
    if focus_range.is_empty() || focus_range.start >= text.len() {
        return None;
    }

    let mut start = focus_range.start.min(text.len());
    let mut end = focus_range.end.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start = start.saturating_sub(1);
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    (start < end).then_some(start..end)
}

fn char_boundaries(text: &str) -> Vec<usize> {
    let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
    boundaries.push(0);
    boundaries.extend(text.char_indices().skip(1).map(|(ix, _)| ix));
    boundaries.push(text.len());
    boundaries
}

#[cfg(any(test, feature = "benchmarks"))]
fn record_measure_candidate_call_for_test() {
    MEASURE_CANDIDATE_CALLS_FOR_TEST.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(not(any(test, feature = "benchmarks")))]
fn record_measure_candidate_call_for_test() {}

fn width_cache_key(width: Pixels) -> u32 {
    let width = f32::from(width);
    if width == 0.0 {
        0
    } else if width.is_nan() {
        f32::NAN.to_bits()
    } else {
        width.to_bits()
    }
}

fn hash_value(value: &(impl Hash + ?Sized)) -> u64 {
    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

fn hash_color(color: Hsla) -> u64 {
    let mut hasher = DefaultHasher::new();
    color.hash(&mut hasher);
    hasher.finish()
}

fn hash_highlights(highlights: &[(Range<usize>, HighlightStyle)]) -> u64 {
    hash_value(&highlights)
}

fn hash_focus_range(focus_range: Option<&Range<usize>>) -> u64 {
    match focus_range {
        Some(range) => hash_value(range),
        None => 0,
    }
}

pub(crate) fn path_alignment_style_key(base_style: &TextStyle, rem_size: Pixels) -> u64 {
    let mut hasher = FxHasher::default();
    f32::from(base_style.font_size.to_pixels(rem_size))
        .to_bits()
        .hash(&mut hasher);
    f32::from(base_style.line_height_in_pixels(rem_size))
        .to_bits()
        .hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_features.hash(&mut hasher);
    base_style.font_fallbacks.hash(&mut hasher);
    base_style.font_weight.0.to_bits().hash(&mut hasher);
    base_style.font_style.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn path_alignment_visible_signature(value: &(impl Hash + ?Sized)) -> u64 {
    hash_value(value)
}

#[cfg(test)]
pub(crate) fn clear_truncated_layout_cache_for_test() {
    TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow_mut().clear());
}

#[cfg(feature = "benchmarks")]
pub(crate) fn clear_truncated_layout_cache_for_benchmark() {
    TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn truncated_layout_cache_len_for_test() -> usize {
    TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow().len())
}

#[cfg(test)]
pub(crate) fn reset_measure_candidate_calls_for_test() {
    MEASURE_CANDIDATE_CALLS_FOR_TEST.with(|calls| calls.set(0));
}

#[cfg(feature = "benchmarks")]
pub(crate) fn reset_measure_candidate_calls_for_benchmark() {
    MEASURE_CANDIDATE_CALLS_FOR_TEST.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn measure_candidate_calls_for_test() -> usize {
    MEASURE_CANDIDATE_CALLS_FOR_TEST.with(Cell::get)
}

#[cfg(feature = "benchmarks")]
pub(crate) fn measure_candidate_calls_for_benchmark() -> usize {
    MEASURE_CANDIDATE_CALLS_FOR_TEST.with(Cell::get)
}

#[cfg(test)]
mod tests;
