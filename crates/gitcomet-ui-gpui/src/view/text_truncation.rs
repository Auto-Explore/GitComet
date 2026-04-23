use gpui::{
    App, HighlightStyle, Hsla, Pixels, ShapedLine, SharedString, TextRun, TextStyle, Window, px,
};
use lru::LruCache;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::num::NonZeroUsize;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TextTruncationProfile {
    End,
    Middle,
    Path,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TruncationProjection {
    source_len: usize,
    display_len: usize,
    segments: SmallVec<[ProjectionSegment; 4]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionSegment {
    Source {
        source_range: Range<usize>,
        display_range: Range<usize>,
    },
    Ellipsis {
        hidden_range: Range<usize>,
        display_range: Range<usize>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct TruncatedLineLayout {
    pub(crate) display_text: SharedString,
    pub(crate) shaped_line: ShapedLine,
    pub(crate) projection: Arc<TruncationProjection>,
    pub(crate) truncated: bool,
    pub(crate) line_height: Pixels,
    pub(crate) has_background_runs: bool,
}

#[derive(Clone, Debug)]
struct CandidateLayout {
    display_text: SharedString,
    display_highlights: Vec<(Range<usize>, HighlightStyle)>,
    projection: Arc<TruncationProjection>,
    truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Affinity {
    Start,
    End,
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

enum SegmentSpec {
    Source(Range<usize>),
    Ellipsis(Range<usize>),
}

#[derive(Clone)]
struct MeasuredCandidate {
    candidate: CandidateLayout,
    width: Pixels,
    ellipsis_x: Option<Pixels>,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct PathAlignmentLayoutKey {
    pub(crate) width_key: Option<u32>,
    pub(crate) style_key: u64,
}

impl PathAlignmentLayoutKey {
    fn new(max_width: Option<Pixels>, style_key: u64) -> Self {
        Self {
            width_key: max_width.map(width_cache_key),
            style_key,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct TruncatedTextPathAlignmentGroup(Rc<RefCell<PathAlignmentState>>);

#[derive(Debug, Default)]
struct PathAlignmentState {
    visible_signature: Option<u64>,
    render_epoch: u64,
    layout_key: Option<PathAlignmentLayoutKey>,
    layout_epoch: u64,
    resolved_anchor: Option<Pixels>,
    pending_anchor: Option<Pixels>,
    notified_for_pending: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PathAlignmentSnapshot {
    pub(crate) visible_signature: Option<u64>,
    pub(crate) layout_key: Option<PathAlignmentLayoutKey>,
    pub(crate) resolved_anchor: Option<Pixels>,
    pub(crate) pending_anchor: Option<Pixels>,
    pub(crate) render_epoch: u64,
    pub(crate) layout_epoch: u64,
    pub(crate) notified_for_pending: bool,
}

impl PathAlignmentState {
    fn reset_layout_state(&mut self) {
        self.layout_key = None;
        self.layout_epoch = 0;
        self.resolved_anchor = None;
        self.pending_anchor = None;
        self.notified_for_pending = false;
    }

    fn prepare_layout(&mut self, layout_key: PathAlignmentLayoutKey) {
        if self.layout_key != Some(layout_key) {
            self.layout_key = Some(layout_key);
            self.layout_epoch = self.render_epoch;
            self.resolved_anchor = None;
            self.pending_anchor = None;
            self.notified_for_pending = false;
            return;
        }

        if self.layout_epoch != self.render_epoch {
            self.layout_epoch = self.render_epoch;
            if let Some(pending_anchor) = self.pending_anchor {
                self.resolved_anchor = Some(
                    self.resolved_anchor
                        .map_or(pending_anchor, |current| current.min(pending_anchor)),
                );
            }
            self.pending_anchor = None;
            self.notified_for_pending = false;
        }
    }
}

impl TruncatedTextPathAlignmentGroup {
    pub(crate) fn begin_visible_rows(&self, visible_signature: u64) {
        let mut state = self.0.borrow_mut();
        if state.visible_signature != Some(visible_signature) {
            state.visible_signature = Some(visible_signature);
            state.render_epoch = state.render_epoch.wrapping_add(1);
            state.reset_layout_state();
            return;
        }

        state.render_epoch = state.render_epoch.wrapping_add(1);
    }

    pub(crate) fn path_anchor_for_layout(
        &self,
        max_width: Option<Pixels>,
        style_key: u64,
    ) -> Option<Pixels> {
        let mut state = self.0.borrow_mut();
        state.prepare_layout(PathAlignmentLayoutKey::new(max_width, style_key));
        state.resolved_anchor
    }

    pub(crate) fn report_natural_ellipsis(
        &self,
        max_width: Option<Pixels>,
        style_key: u64,
        ellipsis_x: Pixels,
    ) -> bool {
        let mut state = self.0.borrow_mut();
        state.prepare_layout(PathAlignmentLayoutKey::new(max_width, style_key));
        let tightened = state
            .pending_anchor
            .is_none_or(|current| ellipsis_x < current);
        if !tightened {
            return false;
        }

        state.pending_anchor = Some(ellipsis_x);
        if state.notified_for_pending {
            return false;
        }

        state.notified_for_pending = true;
        true
    }

    #[cfg(test)]
    pub(crate) fn snapshot_for_test(&self) -> PathAlignmentSnapshot {
        let state = self.0.borrow();
        PathAlignmentSnapshot {
            visible_signature: state.visible_signature,
            layout_key: state.layout_key,
            resolved_anchor: state.resolved_anchor,
            pending_anchor: state.pending_anchor,
            render_epoch: state.render_epoch,
            layout_epoch: state.layout_epoch,
            notified_for_pending: state.notified_for_pending,
        }
    }
}

fn compare_pixels(lhs: Pixels, rhs: Pixels) -> Ordering {
    f32::from(lhs).total_cmp(&f32::from(rhs))
}

fn candidate_visible_source_len(candidate: &CandidateLayout) -> usize {
    candidate
        .projection
        .segments
        .iter()
        .filter_map(|segment| match segment {
            ProjectionSegment::Source { source_range, .. } => {
                Some(source_range.end.saturating_sub(source_range.start))
            }
            ProjectionSegment::Ellipsis { .. } => None,
        })
        .sum()
}

fn candidate_edge_visible_lengths(candidate: &CandidateLayout) -> (usize, usize) {
    let mut prefix_visible = 0usize;
    let mut suffix_visible = 0usize;
    let source_len = candidate.projection.source_len;

    for segment in &candidate.projection.segments {
        let ProjectionSegment::Source { source_range, .. } = segment else {
            continue;
        };
        if source_range.start == 0 {
            prefix_visible = source_range.end;
        }
        if source_range.end == source_len {
            suffix_visible = source_len.saturating_sub(source_range.start);
        }
    }

    (prefix_visible, suffix_visible)
}

fn compare_middle_measured_candidates(
    candidate: &MeasuredCandidate,
    current: &MeasuredCandidate,
) -> Ordering {
    let (candidate_prefix, candidate_suffix) = candidate_edge_visible_lengths(&candidate.candidate);
    let (current_prefix, current_suffix) = candidate_edge_visible_lengths(&current.candidate);
    let candidate_imbalance = candidate_prefix.abs_diff(candidate_suffix);
    let current_imbalance = current_prefix.abs_diff(current_suffix);

    compare_pixels(candidate.width, current.width)
        .then_with(|| {
            candidate_visible_source_len(&candidate.candidate)
                .cmp(&candidate_visible_source_len(&current.candidate))
        })
        .then_with(|| current_imbalance.cmp(&candidate_imbalance))
}

fn compare_focus_measured_candidates(
    candidate: &MeasuredCandidate,
    current: &MeasuredCandidate,
    focus: &Range<usize>,
) -> Ordering {
    let candidate_visible = candidate_visible_source_range(&candidate.candidate);
    let current_visible = candidate_visible_source_range(&current.candidate);
    let candidate_len = candidate_visible
        .as_ref()
        .map_or(0, |range| range.end.saturating_sub(range.start));
    let current_len = current_visible
        .as_ref()
        .map_or(0, |range| range.end.saturating_sub(range.start));
    let focus_center = focus.start + focus.end;
    let candidate_distance = candidate_visible.as_ref().map_or(usize::MAX, |range| {
        (range.start + range.end).abs_diff(focus_center)
    });
    let current_distance = current_visible.as_ref().map_or(usize::MAX, |range| {
        (range.start + range.end).abs_diff(focus_center)
    });
    let candidate_start = candidate_visible
        .as_ref()
        .map_or(usize::MAX, |range| range.start);
    let current_start = current_visible
        .as_ref()
        .map_or(usize::MAX, |range| range.start);

    compare_pixels(candidate.width, current.width)
        .then_with(|| candidate_len.cmp(&current_len))
        .then_with(|| current_distance.cmp(&candidate_distance))
        .then_with(|| current_start.cmp(&candidate_start))
}

fn compare_path_measured_candidates(
    candidate: &MeasuredCandidate,
    current: &MeasuredCandidate,
) -> Ordering {
    let (_, candidate_suffix) = candidate_edge_visible_lengths(&candidate.candidate);
    let (_, current_suffix) = candidate_edge_visible_lengths(&current.candidate);

    compare_pixels(candidate.width, current.width)
        .then_with(|| {
            candidate_visible_source_len(&candidate.candidate)
                .cmp(&candidate_visible_source_len(&current.candidate))
        })
        .then_with(|| candidate_suffix.cmp(&current_suffix))
}

fn compare_filename_tail_measured_candidates(
    candidate: &MeasuredCandidate,
    current: &MeasuredCandidate,
) -> Ordering {
    let (_, candidate_suffix) = candidate_edge_visible_lengths(&candidate.candidate);
    let (_, current_suffix) = candidate_edge_visible_lengths(&current.candidate);

    compare_pixels(candidate.width, current.width)
        .then_with(|| candidate_suffix.cmp(&current_suffix))
        .then_with(|| {
            candidate_visible_source_len(&candidate.candidate)
                .cmp(&candidate_visible_source_len(&current.candidate))
        })
}

fn compare_path_ellipsis_positions(
    candidate_ellipsis_x: Option<Pixels>,
    current_ellipsis_x: Option<Pixels>,
    ellipsis_anchor: Pixels,
) -> Ordering {
    match (candidate_ellipsis_x, current_ellipsis_x) {
        (Some(candidate_x), Some(current_x)) => {
            let candidate_at_or_left = candidate_x <= ellipsis_anchor;
            let current_at_or_left = current_x <= ellipsis_anchor;
            match (candidate_at_or_left, current_at_or_left) {
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (true, true) => compare_pixels(candidate_x, current_x),
                (false, false) => compare_pixels(current_x, candidate_x),
            }
        }
        _ => Ordering::Equal,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PathTruncationTier {
    KeepLastDirectory,
    KeepFilenameOnly,
    CollapsePrefix,
    FilenameTail,
}

fn maximize_frontier_candidate<F, C>(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    max_width: Pixels,
    left_len: usize,
    right_len: usize,
    mut build_candidate: F,
    mut compare: C,
) -> Option<MeasuredCandidate>
where
    F: FnMut(usize, usize) -> Option<CandidateLayout>,
    C: FnMut(&MeasuredCandidate, &MeasuredCandidate) -> Ordering,
{
    if left_len == 0 || right_len == 0 {
        return None;
    }

    let mut best: Option<MeasuredCandidate> = None;
    let mut right_ix = right_len.saturating_sub(1);

    'left: for left_ix in 0..left_len {
        loop {
            let Some(candidate) = build_candidate(left_ix, right_ix) else {
                if right_ix == 0 {
                    break 'left;
                }
                right_ix -= 1;
                continue;
            };

            let (width, ellipsis_x) =
                measure_candidate(window, cx, base_style, font_size, &candidate);
            if width <= max_width {
                let measured = MeasuredCandidate {
                    candidate,
                    width,
                    ellipsis_x,
                };
                if best
                    .as_ref()
                    .is_none_or(|current| compare(&measured, current) == Ordering::Greater)
                {
                    best = Some(measured);
                }
                break;
            }

            if right_ix == 0 {
                break 'left;
            }
            right_ix -= 1;
        }
    }

    best
}

fn extend_offset_bounds(bounds: &mut Option<(usize, usize)>, start: usize, end: usize) {
    match bounds {
        Some((min, max)) => {
            *min = (*min).min(start);
            *max = (*max).max(end);
        }
        None => *bounds = Some((start, end)),
    }
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

impl TruncationProjection {
    pub(crate) fn source_len(&self) -> usize {
        self.source_len
    }

    pub(crate) fn len(&self) -> usize {
        self.display_len
    }

    pub(crate) fn is_truncated(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| matches!(segment, ProjectionSegment::Ellipsis { .. }))
    }

    pub(crate) fn source_to_display_offset(&self, offset: usize) -> usize {
        self.source_to_display_offset_with_affinity(offset, Affinity::Start)
    }

    pub(crate) fn source_to_display_offset_at_end(&self, offset: usize) -> usize {
        self.source_to_display_offset_with_affinity(offset, Affinity::End)
    }

    pub(crate) fn source_to_display_offset_with_affinity(
        &self,
        offset: usize,
        affinity: Affinity,
    ) -> usize {
        let offset = offset.min(self.source_len);
        let mut bounds = None;
        for segment in &self.segments {
            match segment {
                ProjectionSegment::Source {
                    source_range,
                    display_range,
                } if offset >= source_range.start && offset <= source_range.end => {
                    let display_offset =
                        display_range.start + offset.saturating_sub(source_range.start);
                    extend_offset_bounds(&mut bounds, display_offset, display_offset);
                }
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } => {
                    if offset == hidden_range.start {
                        extend_offset_bounds(&mut bounds, display_range.start, display_range.start);
                    } else if offset == hidden_range.end {
                        extend_offset_bounds(&mut bounds, display_range.end, display_range.end);
                    } else if offset > hidden_range.start && offset < hidden_range.end {
                        extend_offset_bounds(&mut bounds, display_range.start, display_range.end);
                    }
                }
                _ => {}
            }
        }

        match (affinity, bounds) {
            (Affinity::Start, Some((min, _))) => min,
            (Affinity::End, Some((_, max))) => max,
            (_, None) => self.display_len,
        }
    }

    pub(crate) fn display_to_source_offset(
        &self,
        display_offset: usize,
        affinity: Affinity,
    ) -> usize {
        let display_offset = display_offset.min(self.display_len);
        let mut bounds = None;
        for segment in &self.segments {
            match segment {
                ProjectionSegment::Source {
                    source_range,
                    display_range,
                } if display_offset >= display_range.start
                    && display_offset <= display_range.end =>
                {
                    let source_offset =
                        source_range.start + display_offset.saturating_sub(display_range.start);
                    extend_offset_bounds(&mut bounds, source_offset, source_offset);
                }
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } if display_offset >= display_range.start
                    && display_offset <= display_range.end =>
                {
                    extend_offset_bounds(&mut bounds, hidden_range.start, hidden_range.end);
                }
                _ => {}
            }
        }

        match (affinity, bounds) {
            (Affinity::Start, Some((min, _))) => min,
            (Affinity::End, Some((_, max))) => max,
            (_, None) => self.source_len,
        }
    }

    pub(crate) fn display_to_source_start_offset(&self, display_offset: usize) -> usize {
        self.display_to_source_offset(display_offset, Affinity::Start)
    }

    pub(crate) fn display_to_source_end_offset(&self, display_offset: usize) -> usize {
        self.display_to_source_offset(display_offset, Affinity::End)
    }

    pub(crate) fn selection_display_ranges(
        &self,
        selection: Range<usize>,
    ) -> SmallVec<[Range<usize>; 4]> {
        let mut ranges = SmallVec::new();
        if selection.is_empty() {
            return ranges;
        }

        for segment in &self.segments {
            match segment {
                ProjectionSegment::Source {
                    source_range,
                    display_range,
                } => {
                    let start = selection.start.max(source_range.start);
                    let end = selection.end.min(source_range.end);
                    if start >= end {
                        continue;
                    }
                    ranges.push(
                        display_range.start + start.saturating_sub(source_range.start)
                            ..display_range.start + end.saturating_sub(source_range.start),
                    );
                }
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } => {
                    if selection.start < hidden_range.end && selection.end > hidden_range.start {
                        ranges.push(display_range.clone());
                    }
                }
            }
        }
        ranges
    }

    pub(crate) fn ellipsis_segment_at_display_offset(
        &self,
        display_offset: usize,
    ) -> Option<(Range<usize>, Range<usize>)> {
        self.segments.iter().find_map(|segment| match segment {
            ProjectionSegment::Ellipsis {
                hidden_range,
                display_range,
            } if display_offset >= display_range.start && display_offset <= display_range.end => {
                Some((hidden_range.clone(), display_range.clone()))
            }
            _ => None,
        })
    }

    pub(crate) fn ellipsis_segment_for_source_offset(
        &self,
        source_offset: usize,
    ) -> Option<(Range<usize>, Range<usize>)> {
        self.segments.iter().find_map(|segment| match segment {
            ProjectionSegment::Ellipsis {
                hidden_range,
                display_range,
            } if source_offset >= hidden_range.start && source_offset <= hidden_range.end => {
                Some((hidden_range.clone(), display_range.clone()))
            }
            _ => None,
        })
    }
}

fn first_ellipsis_display_start(projection: &TruncationProjection) -> Option<usize> {
    projection
        .segments
        .iter()
        .find_map(|segment| match segment {
            ProjectionSegment::Ellipsis { display_range, .. } => Some(display_range.start),
            _ => None,
        })
}

fn ellipsis_x_for_projection_and_line(
    projection: &TruncationProjection,
    shaped_line: &ShapedLine,
) -> Option<Pixels> {
    Some(shaped_line.x_for_index(first_ellipsis_display_start(projection)?))
}

pub(crate) fn truncated_line_ellipsis_x(line: &TruncatedLineLayout) -> Option<Pixels> {
    ellipsis_x_for_projection_and_line(line.projection.as_ref(), &line.shaped_line)
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
        text,
        max_width,
        profile,
        highlights,
        normalized_focus_range,
        path_ellipsis_anchor,
        font_size,
        line_height,
    ));

    TRUNCATED_LAYOUT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, Arc::clone(&layout));
    });

    layout
}

fn shape_truncated_line_uncached(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    max_width: Option<Pixels>,
    profile: TextTruncationProfile,
    highlights: &[(Range<usize>, HighlightStyle)],
    normalized_focus_range: Option<Range<usize>>,
    path_ellipsis_anchor: Option<Pixels>,
    font_size: Pixels,
    line_height: Pixels,
) -> TruncatedLineLayout {
    let candidate = build_truncated_candidate(
        window,
        cx,
        base_style,
        text,
        max_width,
        profile,
        highlights,
        normalized_focus_range,
        path_ellipsis_anchor,
        font_size,
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
        display_text: candidate.display_text,
        shaped_line,
        projection: candidate.projection,
        truncated: candidate.truncated,
        line_height,
        has_background_runs,
    }
}

fn build_truncated_candidate(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    max_width: Option<Pixels>,
    profile: TextTruncationProfile,
    highlights: &[(Range<usize>, HighlightStyle)],
    normalized_focus_range: Option<Range<usize>>,
    path_ellipsis_anchor: Option<Pixels>,
    font_size: Pixels,
) -> CandidateLayout {
    let Some(max_width) = max_width else {
        return full_candidate(text, highlights);
    };
    if max_width <= px(0.0) {
        return ellipsis_only_candidate(text.len());
    }

    let full = full_candidate(text, highlights);
    if candidate_width(window, cx, base_style, font_size, &full) <= max_width {
        return full;
    }

    if let Some(focus) = normalized_focus_range {
        let focused = truncate_around_focus(
            window, cx, base_style, text, highlights, font_size, max_width, focus,
        );
        if candidate_width(window, cx, base_style, font_size, &focused) <= max_width {
            return focused;
        }
    }

    match profile {
        TextTruncationProfile::End => truncate_from_end(
            window, cx, base_style, text, highlights, font_size, max_width,
        ),
        TextTruncationProfile::Middle => truncate_from_middle(
            window, cx, base_style, text, highlights, font_size, max_width,
        ),
        TextTruncationProfile::Path => truncate_path_like(
            window,
            cx,
            base_style,
            text,
            highlights,
            font_size,
            max_width,
            path_ellipsis_anchor,
        ),
    }
}

fn full_candidate(
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> CandidateLayout {
    let projection = Arc::new(TruncationProjection {
        source_len: text.len(),
        display_len: text.len(),
        segments: SmallVec::from_vec(vec![ProjectionSegment::Source {
            source_range: 0..text.len(),
            display_range: 0..text.len(),
        }]),
    });
    CandidateLayout {
        display_text: text.clone(),
        display_highlights: highlights.to_vec(),
        projection,
        truncated: false,
    }
}

fn ellipsis_only_candidate(source_len: usize) -> CandidateLayout {
    let display_text: SharedString = TRUNCATION_ELLIPSIS.into();
    let projection = Arc::new(TruncationProjection {
        source_len,
        display_len: display_text.len(),
        segments: SmallVec::from_vec(vec![ProjectionSegment::Ellipsis {
            hidden_range: 0..source_len,
            display_range: 0..display_text.len(),
        }]),
    });
    CandidateLayout {
        display_text,
        display_highlights: Vec::new(),
        projection,
        truncated: true,
    }
}

fn truncate_from_end(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
) -> CandidateLayout {
    let boundaries = char_boundaries(text.as_ref());
    if boundaries.len() <= 2 {
        return ellipsis_only_candidate(text.len());
    }

    let mut best = ellipsis_only_candidate(text.len());
    for &prefix_end in boundaries.iter().skip(1) {
        let candidate = candidate_from_segments(
            text,
            &[
                SegmentSpec::Source(0..prefix_end),
                SegmentSpec::Ellipsis(prefix_end..text.len()),
            ],
            highlights,
        );
        if candidate_width(window, cx, base_style, font_size, &candidate) <= max_width {
            best = candidate;
        } else {
            break;
        }
    }
    best
}

fn truncate_from_start(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
) -> CandidateLayout {
    let boundaries = char_boundaries(text.as_ref());
    if boundaries.len() <= 2 {
        return ellipsis_only_candidate(text.len());
    }

    let mut best = ellipsis_only_candidate(text.len());
    for &suffix_start in boundaries.iter().rev().skip(1) {
        let candidate = candidate_from_segments(
            text,
            &[
                SegmentSpec::Ellipsis(0..suffix_start),
                SegmentSpec::Source(suffix_start..text.len()),
            ],
            highlights,
        );
        if candidate_width(window, cx, base_style, font_size, &candidate) <= max_width {
            best = candidate;
        } else {
            break;
        }
    }
    best
}

fn truncate_from_middle(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
) -> CandidateLayout {
    let boundaries = char_boundaries(text.as_ref());
    if boundaries.len() <= 2 {
        return ellipsis_only_candidate(text.len());
    }

    let left_options: Vec<usize> = boundaries[1..boundaries.len().saturating_sub(1)].to_vec();
    let right_options: Vec<usize> = boundaries[1..boundaries.len().saturating_sub(1)]
        .iter()
        .rev()
        .copied()
        .collect();

    maximize_frontier_candidate(
        window,
        cx,
        base_style,
        font_size,
        max_width,
        left_options.len(),
        right_options.len(),
        |left_ix, right_ix| {
            let prefix_end = left_options[left_ix];
            let suffix_start = right_options[right_ix];
            (prefix_end < suffix_start).then(|| {
                candidate_from_segments(
                    text,
                    &[
                        SegmentSpec::Source(0..prefix_end),
                        SegmentSpec::Ellipsis(prefix_end..suffix_start),
                        SegmentSpec::Source(suffix_start..text.len()),
                    ],
                    highlights,
                )
            })
        },
        compare_middle_measured_candidates,
    )
    .map(|best| best.candidate)
    .unwrap_or_else(|| {
        truncate_from_end(
            window, cx, base_style, text, highlights, font_size, max_width,
        )
    })
}

fn truncate_around_focus(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
    focus: Range<usize>,
) -> CandidateLayout {
    let boundaries = char_boundaries(text.as_ref());
    let best = candidate_with_focus_window(text, highlights, focus.start, focus.end);

    if candidate_width(window, cx, base_style, font_size, &best) > max_width {
        return truncate_within_focus(
            window,
            cx,
            base_style,
            text,
            highlights,
            font_size,
            max_width,
            &boundaries,
            focus,
        );
    }

    let Ok(focus_start_ix) = boundaries.binary_search(&focus.start) else {
        return best;
    };
    let Ok(focus_end_ix) = boundaries.binary_search(&focus.end) else {
        return best;
    };
    let left_options: Vec<usize> = boundaries[..=focus_start_ix]
        .iter()
        .rev()
        .copied()
        .collect();
    let right_options: Vec<usize> = boundaries[focus_end_ix..].to_vec();

    maximize_frontier_candidate(
        window,
        cx,
        base_style,
        font_size,
        max_width,
        left_options.len(),
        right_options.len(),
        |left_ix, right_ix| {
            let candidate_start = left_options[left_ix];
            let candidate_end = right_options[right_ix];
            (candidate_start < candidate_end).then(|| {
                candidate_with_focus_window(text, highlights, candidate_start, candidate_end)
            })
        },
        |candidate, current| compare_focus_measured_candidates(candidate, current, &focus),
    )
    .map(|best| best.candidate)
    .unwrap_or(best)
}

fn truncate_within_focus(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
    boundaries: &[usize],
    focus: Range<usize>,
) -> CandidateLayout {
    let Ok(focus_start_ix) = boundaries.binary_search(&focus.start) else {
        return ellipsis_only_candidate(text.len());
    };
    let Ok(focus_end_ix) = boundaries.binary_search(&focus.end) else {
        return ellipsis_only_candidate(text.len());
    };

    let mut best: Option<MeasuredCandidate> = None;
    for (start, end) in centered_focus_seed_windows(boundaries, &focus) {
        let Ok(start_ix) = boundaries.binary_search(&start) else {
            continue;
        };
        let Ok(end_ix) = boundaries.binary_search(&end) else {
            continue;
        };
        let left_options: Vec<usize> = boundaries[focus_start_ix..=start_ix]
            .iter()
            .rev()
            .copied()
            .collect();
        let right_options: Vec<usize> = boundaries[end_ix..=focus_end_ix].to_vec();

        let Some(candidate) = maximize_frontier_candidate(
            window,
            cx,
            base_style,
            font_size,
            max_width,
            left_options.len(),
            right_options.len(),
            |left_ix, right_ix| {
                let candidate_start = left_options[left_ix];
                let candidate_end = right_options[right_ix];
                (candidate_start < candidate_end).then(|| {
                    candidate_with_focus_window(text, highlights, candidate_start, candidate_end)
                })
            },
            |candidate, current| compare_focus_measured_candidates(candidate, current, &focus),
        ) else {
            continue;
        };

        let replace = best.as_ref().is_none_or(|current| {
            compare_focus_measured_candidates(&candidate, current, &focus) == Ordering::Greater
        });
        if replace {
            best = Some(candidate);
        }
    }

    best.map(|candidate| candidate.candidate)
        .unwrap_or_else(|| ellipsis_only_candidate(text.len()))
}

fn centered_focus_seed_windows(
    boundaries: &[usize],
    focus: &Range<usize>,
) -> SmallVec<[(usize, usize); 2]> {
    let Ok(focus_start_ix) = boundaries.binary_search(&focus.start) else {
        return SmallVec::new();
    };
    let Ok(focus_end_ix) = boundaries.binary_search(&focus.end) else {
        return SmallVec::new();
    };
    if focus_start_ix >= focus_end_ix {
        return SmallVec::new();
    }

    let midpoint = focus.start + (focus.end.saturating_sub(focus.start) / 2);
    let mut seeds = SmallVec::<[(usize, usize); 2]>::new();

    if let Ok(mid_ix) = boundaries.binary_search(&midpoint)
        && mid_ix > focus_start_ix
        && mid_ix < focus_end_ix
    {
        seeds.push((boundaries[mid_ix - 1], boundaries[mid_ix]));
        seeds.push((boundaries[mid_ix], boundaries[mid_ix + 1]));
        return seeds;
    }

    let mut right_ix = boundaries.partition_point(|&boundary| boundary <= midpoint);
    right_ix = right_ix.max(focus_start_ix + 1).min(focus_end_ix);
    let left_ix = right_ix.saturating_sub(1).max(focus_start_ix);
    if left_ix < right_ix {
        seeds.push((boundaries[left_ix], boundaries[right_ix]));
    }
    seeds
}

fn candidate_visible_source_range(candidate: &CandidateLayout) -> Option<Range<usize>> {
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    for segment in &candidate.projection.segments {
        let ProjectionSegment::Source { source_range, .. } = segment else {
            continue;
        };
        start = Some(start.map_or(source_range.start, |current| {
            current.min(source_range.start)
        }));
        end = Some(end.map_or(source_range.end, |current| current.max(source_range.end)));
    }
    Some(start?..end?)
}

fn candidate_with_focus_window(
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    start: usize,
    end: usize,
) -> CandidateLayout {
    let mut segments = Vec::with_capacity(3);
    if start > 0 {
        segments.push(SegmentSpec::Ellipsis(0..start));
    }
    segments.push(SegmentSpec::Source(start..end));
    if end < text.len() {
        segments.push(SegmentSpec::Ellipsis(end..text.len()));
    }
    candidate_from_segments(text, &segments, highlights)
}

fn maximize_path_candidate<F>(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    max_width: Pixels,
    left_len: usize,
    right_len: usize,
    build_candidate: F,
    path_ellipsis_anchor: Option<Pixels>,
    compare_base: fn(&MeasuredCandidate, &MeasuredCandidate) -> Ordering,
) -> Option<MeasuredCandidate>
where
    F: FnMut(usize, usize) -> Option<CandidateLayout>,
{
    match path_ellipsis_anchor {
        Some(anchor) => maximize_frontier_candidate(
            window,
            cx,
            base_style,
            font_size,
            max_width,
            left_len,
            right_len,
            build_candidate,
            move |candidate, current| {
                compare_path_ellipsis_positions(candidate.ellipsis_x, current.ellipsis_x, anchor)
                    .then_with(|| compare_base(candidate, current))
            },
        ),
        None => maximize_frontier_candidate(
            window,
            cx,
            base_style,
            font_size,
            max_width,
            left_len,
            right_len,
            build_candidate,
            compare_base,
        ),
    }
}

fn path_prefix_options(path: &PathBoundaries, suffix_start: usize) -> Vec<usize> {
    path.prefix_cuts
        .iter()
        .copied()
        .filter(|&cut| cut >= path.min_prefix_end && cut < suffix_start)
        .collect()
}

fn path_suffix_start_for_tier(path: &PathBoundaries, tier: PathTruncationTier) -> Option<usize> {
    match tier {
        PathTruncationTier::KeepLastDirectory => path.separator_starts.iter().rev().nth(1).copied(),
        PathTruncationTier::KeepFilenameOnly | PathTruncationTier::CollapsePrefix => {
            path.separator_starts.last().copied()
        }
        PathTruncationTier::FilenameTail => None,
    }
}

fn maximize_path_suffix_candidate(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
    prefix_options: &[usize],
    suffix_start: usize,
    path_ellipsis_anchor: Option<Pixels>,
) -> Option<MeasuredCandidate> {
    maximize_path_candidate(
        window,
        cx,
        base_style,
        font_size,
        max_width,
        prefix_options.len(),
        1,
        |left_ix, _| {
            let prefix_end = prefix_options[left_ix];
            Some(candidate_from_segments(
                text,
                &[
                    SegmentSpec::Source(0..prefix_end),
                    SegmentSpec::Ellipsis(prefix_end..suffix_start),
                    SegmentSpec::Source(suffix_start..text.len()),
                ],
                highlights,
            ))
        },
        path_ellipsis_anchor,
        compare_path_measured_candidates,
    )
}

fn truncate_path_with_preserved_suffix_tier(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    path: &PathBoundaries,
    font_size: Pixels,
    max_width: Pixels,
    tier: PathTruncationTier,
    path_ellipsis_anchor: Option<Pixels>,
) -> Option<CandidateLayout> {
    let suffix_start = path_suffix_start_for_tier(path, tier)?;
    let prefix_options = path_prefix_options(path, suffix_start);
    maximize_path_suffix_candidate(
        window,
        cx,
        base_style,
        text,
        highlights,
        font_size,
        max_width,
        &prefix_options,
        suffix_start,
        path_ellipsis_anchor,
    )
    .map(|candidate| candidate.candidate)
}

fn candidate_with_hidden_prefix(
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    hidden_end: usize,
) -> CandidateLayout {
    candidate_from_segments(
        text,
        &[
            SegmentSpec::Ellipsis(0..hidden_end),
            SegmentSpec::Source(hidden_end..text.len()),
        ],
        highlights,
    )
}

fn maximize_single_ellipsis_filename_candidate(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
    tail_starts: &[usize],
) -> Option<MeasuredCandidate> {
    maximize_path_candidate(
        window,
        cx,
        base_style,
        font_size,
        max_width,
        1,
        tail_starts.len(),
        |_, right_ix| {
            let tail_start = tail_starts[right_ix];
            Some(candidate_with_hidden_prefix(text, highlights, tail_start))
        },
        None,
        compare_filename_tail_measured_candidates,
    )
}

fn truncate_path_with_collapsed_prefix_tier(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    path: &PathBoundaries,
    font_size: Pixels,
    max_width: Pixels,
) -> Option<CandidateLayout> {
    let hidden_end = path_suffix_start_for_tier(path, PathTruncationTier::CollapsePrefix)?;
    let candidate = candidate_with_hidden_prefix(text, highlights, hidden_end);
    (candidate_width(window, cx, base_style, font_size, &candidate) <= max_width)
        .then_some(candidate)
}

fn truncate_path_with_filename_tail_tier(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    path: &PathBoundaries,
    font_size: Pixels,
    max_width: Pixels,
    tier: PathTruncationTier,
) -> Option<CandidateLayout> {
    debug_assert_eq!(tier, PathTruncationTier::FilenameTail);
    let tail_starts: Vec<usize> = char_boundaries(text.as_ref())
        .into_iter()
        .rev()
        .filter(|&boundary| boundary >= path.min_suffix_start && boundary < text.len())
        .collect();
    maximize_single_ellipsis_filename_candidate(
        window,
        cx,
        base_style,
        text,
        highlights,
        font_size,
        max_width,
        &tail_starts,
    )
    .map(|candidate| candidate.candidate)
}

fn truncate_path_like(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
    path_ellipsis_anchor: Option<Pixels>,
) -> CandidateLayout {
    let Some(path) = path_boundaries(text.as_ref()) else {
        return truncate_from_middle(
            window, cx, base_style, text, highlights, font_size, max_width,
        );
    };

    if path.min_prefix_end > path.min_suffix_start {
        return truncate_from_start(
            window, cx, base_style, text, highlights, font_size, max_width,
        );
    }

    for tier in [
        PathTruncationTier::KeepLastDirectory,
        PathTruncationTier::KeepFilenameOnly,
    ] {
        if let Some(candidate) = truncate_path_with_preserved_suffix_tier(
            window,
            cx,
            base_style,
            text,
            highlights,
            &path,
            font_size,
            max_width,
            tier,
            path_ellipsis_anchor,
        ) {
            return candidate;
        }
    }

    if let Some(candidate) = truncate_path_with_collapsed_prefix_tier(
        window, cx, base_style, text, highlights, &path, font_size, max_width,
    ) {
        return candidate;
    }

    if let Some(candidate) = truncate_path_with_filename_tail_tier(
        window,
        cx,
        base_style,
        text,
        highlights,
        &path,
        font_size,
        max_width,
        PathTruncationTier::FilenameTail,
    ) {
        return candidate;
    }

    truncate_from_start(
        window, cx, base_style, text, highlights, font_size, max_width,
    )
}

fn candidate_from_segments(
    text: &SharedString,
    segments: &[SegmentSpec],
    highlights: &[(Range<usize>, HighlightStyle)],
) -> CandidateLayout {
    let mut display = String::new();
    let mut projection_segments = SmallVec::<[ProjectionSegment; 4]>::new();
    let mut truncated = false;

    for segment in segments {
        let display_start = display.len();
        match segment {
            SegmentSpec::Source(source_range) => {
                display.push_str(&text[source_range.clone()]);
                projection_segments.push(ProjectionSegment::Source {
                    source_range: source_range.clone(),
                    display_range: display_start..display.len(),
                });
            }
            SegmentSpec::Ellipsis(hidden_range) => {
                truncated = true;
                display.push_str(TRUNCATION_ELLIPSIS);
                projection_segments.push(ProjectionSegment::Ellipsis {
                    hidden_range: hidden_range.clone(),
                    display_range: display_start..display.len(),
                });
            }
        }
    }

    let projection = Arc::new(TruncationProjection {
        source_len: text.len(),
        display_len: display.len(),
        segments: projection_segments,
    });
    let display_text: SharedString = display.into();
    let display_highlights = remap_highlights(&projection, highlights);

    CandidateLayout {
        display_text,
        display_highlights,
        projection,
        truncated,
    }
}

fn remap_highlights(
    projection: &TruncationProjection,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<(Range<usize>, HighlightStyle)> {
    if highlights.is_empty() {
        return Vec::new();
    }

    let mut remapped = Vec::new();
    for segment in &projection.segments {
        let ProjectionSegment::Source {
            source_range,
            display_range,
        } = segment
        else {
            continue;
        };
        for (highlight_range, highlight_style) in highlights {
            let start = highlight_range.start.max(source_range.start);
            let end = highlight_range.end.min(source_range.end);
            if start >= end {
                continue;
            }
            remapped.push((
                display_range.start + start.saturating_sub(source_range.start)
                    ..display_range.start + end.saturating_sub(source_range.start),
                *highlight_style,
            ));
        }
    }
    remapped.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    remapped
}

fn measure_candidate(
    window: &mut Window,
    _cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    candidate: &CandidateLayout,
) -> (Pixels, Option<Pixels>) {
    let runs = compute_highlight_runs(
        &candidate.display_text,
        base_style,
        &candidate.display_highlights,
    );
    let shaped_line =
        window
            .text_system()
            .shape_line(candidate.display_text.clone(), font_size, &runs, None);
    (
        shaped_line.width,
        ellipsis_x_for_projection_and_line(candidate.projection.as_ref(), &shaped_line),
    )
}

fn candidate_width(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    candidate: &CandidateLayout,
) -> Pixels {
    measure_candidate(window, cx, base_style, font_size, candidate).0
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

struct PathBoundaries {
    prefix_cuts: Vec<usize>,
    separator_starts: Vec<usize>,
    min_prefix_end: usize,
    min_suffix_start: usize,
}

fn is_path_separator_byte(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

fn unc_root_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 || !is_path_separator_byte(bytes[0]) || !is_path_separator_byte(bytes[1]) {
        return None;
    }

    let server_sep = bytes[2..]
        .iter()
        .position(|&byte| is_path_separator_byte(byte))
        .map(|ix| ix + 2)?;
    let share_start = server_sep + 1;
    if share_start >= bytes.len() {
        return None;
    }

    let share_sep = bytes[share_start..]
        .iter()
        .position(|&byte| is_path_separator_byte(byte))
        .map(|ix| ix + share_start);
    Some(share_sep.map(|ix| ix + 1).unwrap_or(bytes.len()))
}

fn path_boundaries(text: &str) -> Option<PathBoundaries> {
    let mut separator_ends = Vec::new();
    let mut separator_starts = Vec::new();
    for (ix, ch) in text.char_indices() {
        if ch == '/' || ch == '\\' {
            separator_starts.push(ix);
            separator_ends.push(ix + ch.len_utf8());
        }
    }
    if separator_ends.is_empty() {
        return None;
    }

    let bytes = text.as_bytes();
    let root_end = match separator_ends.first().copied() {
        Some(_) if unc_root_end(text).is_some() => unc_root_end(text),
        Some(first) if text.starts_with('/') || text.starts_with('\\') => Some(first),
        Some(first)
            if first == 3
                && text.len() >= 2
                && bytes.get(1) == Some(&b':')
                && text[..first]
                    .chars()
                    .last()
                    .is_some_and(|ch| ch == '/' || ch == '\\') =>
        {
            Some(first)
        }
        _ => None,
    };

    let min_prefix_end = match root_end {
        Some(root) => separator_ends
            .iter()
            .copied()
            .find(|&cut| cut > root)
            .or(Some(root)),
        None => separator_ends.first().copied(),
    }?;
    let min_suffix_start = separator_ends.last().copied()?;

    let mut prefix_cuts = separator_ends.clone();
    prefix_cuts.retain(|&cut| cut < text.len());
    prefix_cuts.sort_unstable();
    prefix_cuts.dedup();

    Some(PathBoundaries {
        prefix_cuts,
        separator_starts,
        min_prefix_end,
        min_suffix_start,
    })
}

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

#[cfg(test)]
pub(crate) fn truncated_layout_cache_len_for_test() -> usize {
    TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{FontFallbacks, FontFeatures, StrikethroughStyle, UnderlineStyle, hsla};

    #[derive(Clone, Copy)]
    enum FocusSearchMode {
        Around,
        Within,
    }

    fn display_width(
        window: &mut Window,
        text: &str,
        style: &TextStyle,
        font_size: Pixels,
    ) -> Pixels {
        let runs = vec![style.clone().to_run(text.len())];
        window
            .text_system()
            .shape_line(text.to_string().into(), font_size, &runs, None)
            .width
    }

    fn width_between(
        window: &mut Window,
        narrower: &str,
        wider: &str,
        style: &TextStyle,
        font_size: Pixels,
    ) -> Pixels {
        let narrower_width = display_width(window, narrower, style, font_size);
        let wider_width = display_width(window, wider, style, font_size);
        narrower_width + (wider_width - narrower_width) / 2.0
    }

    fn visible_source_range(projection: &TruncationProjection) -> Option<Range<usize>> {
        let mut start: Option<usize> = None;
        let mut end: Option<usize> = None;
        for segment in &projection.segments {
            let ProjectionSegment::Source { source_range, .. } = segment else {
                continue;
            };
            start = Some(start.map_or(source_range.start, |current| {
                current.min(source_range.start)
            }));
            end = Some(end.map_or(source_range.end, |current| current.max(source_range.end)));
        }
        Some(start?..end?)
    }

    fn ellipsis_ranges(projection: &TruncationProjection) -> Vec<(Range<usize>, Range<usize>)> {
        projection
            .segments
            .iter()
            .filter_map(|segment| match segment {
                ProjectionSegment::Ellipsis {
                    hidden_range,
                    display_range,
                } => Some((hidden_range.clone(), display_range.clone())),
                _ => None,
            })
            .collect()
    }

    fn fitting_candidate(
        window: &mut Window,
        cx: &mut App,
        style: &TextStyle,
        font_size: Pixels,
        candidate: CandidateLayout,
        max_width: Pixels,
    ) -> Option<MeasuredCandidate> {
        let (width, ellipsis_x) = measure_candidate(window, cx, style, font_size, &candidate);
        (width <= max_width).then_some(MeasuredCandidate {
            candidate,
            width,
            ellipsis_x,
        })
    }

    fn best_middle_candidate_exhaustive(
        window: &mut Window,
        cx: &mut App,
        style: &TextStyle,
        text: &SharedString,
        font_size: Pixels,
        max_width: Pixels,
    ) -> Option<CandidateLayout> {
        let boundaries = char_boundaries(text.as_ref());
        let mut best: Option<MeasuredCandidate> = None;

        for &prefix_end in &boundaries[1..boundaries.len().saturating_sub(1)] {
            for &suffix_start in boundaries[1..boundaries.len().saturating_sub(1)]
                .iter()
                .rev()
            {
                if prefix_end >= suffix_start {
                    continue;
                }
                let Some(candidate) = fitting_candidate(
                    window,
                    cx,
                    style,
                    font_size,
                    candidate_from_segments(
                        text,
                        &[
                            SegmentSpec::Source(0..prefix_end),
                            SegmentSpec::Ellipsis(prefix_end..suffix_start),
                            SegmentSpec::Source(suffix_start..text.len()),
                        ],
                        &[],
                    ),
                    max_width,
                ) else {
                    continue;
                };

                if best.as_ref().is_none_or(|current| {
                    compare_middle_measured_candidates(&candidate, current) == Ordering::Greater
                }) {
                    best = Some(candidate);
                }
            }
        }

        best.map(|candidate| candidate.candidate)
    }

    fn best_focus_candidate_exhaustive(
        window: &mut Window,
        cx: &mut App,
        style: &TextStyle,
        text: &SharedString,
        font_size: Pixels,
        max_width: Pixels,
        focus: Range<usize>,
        mode: FocusSearchMode,
    ) -> Option<CandidateLayout> {
        let boundaries = char_boundaries(text.as_ref());
        let mut best: Option<MeasuredCandidate> = None;

        for &start in &boundaries {
            for &end in &boundaries {
                if start >= end {
                    continue;
                }

                let allowed = match mode {
                    FocusSearchMode::Around => start <= focus.start && end >= focus.end,
                    FocusSearchMode::Within => start >= focus.start && end <= focus.end,
                };
                if !allowed {
                    continue;
                }

                let Some(candidate) = fitting_candidate(
                    window,
                    cx,
                    style,
                    font_size,
                    candidate_with_focus_window(text, &[], start, end),
                    max_width,
                ) else {
                    continue;
                };

                if best.as_ref().is_none_or(|current| {
                    compare_focus_measured_candidates(&candidate, current, &focus)
                        == Ordering::Greater
                }) {
                    best = Some(candidate);
                }
            }
        }

        best.map(|candidate| candidate.candidate)
    }

    #[test]
    fn selection_display_ranges_include_ellipsis_when_hidden_range_selected() {
        let projection = TruncationProjection {
            source_len: 12,
            display_len: 9,
            segments: SmallVec::from_vec(vec![
                ProjectionSegment::Source {
                    source_range: 0..3,
                    display_range: 0..3,
                },
                ProjectionSegment::Ellipsis {
                    hidden_range: 3..9,
                    display_range: 3..6,
                },
                ProjectionSegment::Source {
                    source_range: 9..12,
                    display_range: 6..9,
                },
            ]),
        };

        let ranges = projection.selection_display_ranges(2..10);
        assert_eq!(ranges.as_slice(), &[2..3, 3..6, 6..7]);
    }

    #[test]
    fn display_to_source_offset_maps_ellipsis_boundaries_to_hidden_edges() {
        let projection = TruncationProjection {
            source_len: 10,
            display_len: 7,
            segments: SmallVec::from_vec(vec![
                ProjectionSegment::Source {
                    source_range: 0..2,
                    display_range: 0..2,
                },
                ProjectionSegment::Ellipsis {
                    hidden_range: 2..8,
                    display_range: 2..5,
                },
                ProjectionSegment::Source {
                    source_range: 8..10,
                    display_range: 5..7,
                },
            ]),
        };

        assert_eq!(projection.display_to_source_offset(2, Affinity::Start), 2);
        assert_eq!(projection.display_to_source_offset(2, Affinity::End), 8);
        assert_eq!(projection.display_to_source_offset(5, Affinity::Start), 2);
        assert_eq!(projection.display_to_source_offset(5, Affinity::End), 8);
    }

    #[test]
    fn source_to_display_offset_maps_hidden_boundaries_to_ellipsis_edges() {
        let projection = TruncationProjection {
            source_len: 10,
            display_len: 7,
            segments: SmallVec::from_vec(vec![
                ProjectionSegment::Source {
                    source_range: 0..2,
                    display_range: 0..2,
                },
                ProjectionSegment::Ellipsis {
                    hidden_range: 2..8,
                    display_range: 2..5,
                },
                ProjectionSegment::Source {
                    source_range: 8..10,
                    display_range: 5..7,
                },
            ]),
        };

        assert_eq!(
            projection.source_to_display_offset_with_affinity(2, Affinity::Start),
            2
        );
        assert_eq!(
            projection.source_to_display_offset_with_affinity(2, Affinity::End),
            2
        );
        assert_eq!(
            projection.source_to_display_offset_with_affinity(5, Affinity::Start),
            2
        );
        assert_eq!(
            projection.source_to_display_offset_with_affinity(5, Affinity::End),
            5
        );
        assert_eq!(
            projection.source_to_display_offset_with_affinity(8, Affinity::Start),
            5
        );
        assert_eq!(
            projection.source_to_display_offset_with_affinity(8, Affinity::End),
            5
        );
    }

    #[test]
    fn normalize_highlights_sorts_clamps_and_deoverlaps_ranges() {
        let highlights = vec![
            (6..12, HighlightStyle::default()),
            (0..2, HighlightStyle::default()),
            (1..4, HighlightStyle::default()),
            (4..4, HighlightStyle::default()),
        ];

        let normalized =
            normalize_highlights_if_needed(10, &highlights).expect("expected normalization");

        assert_eq!(
            normalized
                .iter()
                .map(|(range, _)| range.clone())
                .collect::<Vec<_>>(),
            vec![0..2, 2..4, 6..10]
        );
    }

    #[test]
    fn path_boundaries_preserve_unc_server_and_share_root() {
        let path = r"\\server\share\dir1\dir2\file.txt";
        let boundaries = path_boundaries(path).expect("expected path boundaries");

        assert_eq!(boundaries.min_prefix_end, path.find(r"\dir2").unwrap() + 1);
        assert_eq!(
            boundaries.min_suffix_start,
            path.find(r"\file.txt").unwrap() + 1
        );
    }

    #[gpui::test]
    fn truncated_layout_cache_distinguishes_fractional_widths(cx: &mut gpui::TestAppContext) {
        clear_truncated_layout_cache_for_test();

        let text: SharedString = "0123456789abcdef0123456789abcdef".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let _ = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(px(80.1)),
                TextTruncationProfile::Middle,
                &[],
                None,
            );
            let _ = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(px(80.4)),
                TextTruncationProfile::Middle,
                &[],
                None,
            );
        });

        assert_eq!(truncated_layout_cache_len_for_test(), 2);
        clear_truncated_layout_cache_for_test();
    }

    #[gpui::test]
    fn truncated_layout_cache_distinguishes_style_fields(cx: &mut gpui::TestAppContext) {
        clear_truncated_layout_cache_for_test();

        let text: SharedString = "0123456789abcdef0123456789abcdef".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let base = window.text_style();
            let mut line_height = base.clone();
            line_height.line_height = px(32.0).into();

            let mut font_features = base.clone();
            font_features.font_features = FontFeatures::disable_ligatures();

            let mut font_fallbacks = base.clone();
            font_fallbacks.font_fallbacks =
                Some(FontFallbacks::from_fonts(vec!["Noto Sans".into()]));

            let mut background = base.clone();
            background.background_color = Some(hsla(0.1, 0.4, 0.5, 0.2));

            let mut underline = base.clone();
            underline.underline = Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(hsla(0.6, 0.7, 0.5, 1.0)),
                wavy: true,
            });

            let mut strikethrough = base.clone();
            strikethrough.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(hsla(0.8, 0.7, 0.5, 1.0)),
            });

            for style in [
                &base,
                &line_height,
                &font_features,
                &font_fallbacks,
                &background,
                &underline,
                &strikethrough,
            ] {
                let _ = shape_truncated_line_cached(
                    window,
                    app,
                    style,
                    &text,
                    Some(px(80.0)),
                    TextTruncationProfile::Middle,
                    &[],
                    None,
                );
            }
        });

        assert_eq!(truncated_layout_cache_len_for_test(), 7);
        clear_truncated_layout_cache_for_test();
    }

    #[gpui::test]
    fn truncated_layout_cache_normalizes_focus_range_before_hashing(cx: &mut gpui::TestAppContext) {
        clear_truncated_layout_cache_for_test();

        let text: SharedString = "aé0123456789abcdef0123456789abcdef".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let _ = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(px(80.0)),
                TextTruncationProfile::Middle,
                &[],
                Some(1..2),
            );
            let _ = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(px(80.0)),
                TextTruncationProfile::Middle,
                &[],
                Some(2..3),
            );
        });

        assert_eq!(truncated_layout_cache_len_for_test(), 1);
        clear_truncated_layout_cache_for_test();
    }

    #[gpui::test]
    fn truncated_layout_background_flag_includes_base_style_background(
        cx: &mut gpui::TestAppContext,
    ) {
        let text: SharedString = "0123456789abcdef0123456789abcdef".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let mut style = window.text_style();
            style.background_color = Some(hsla(0.1, 0.4, 0.5, 0.2));

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(px(80.0)),
                TextTruncationProfile::Middle,
                &[],
                None,
            );

            assert!(line.truncated, "expected the line to truncate");
            assert!(line.has_background_runs);
        });
    }

    #[gpui::test]
    fn truncate_around_focus_preserves_centered_focus_slice_when_full_focus_overflows(
        cx: &mut gpui::TestAppContext,
    ) {
        let text: SharedString = "prefix-aaaaaaaaaa-suffix".into();
        let focus = 7..17;
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let width_four = display_width(window, "…aaaa…", &style, font_size);
            let width_five = display_width(window, "…aaaaa…", &style, font_size);
            let max_width = width_four + (width_five - width_four) / 2.0;

            let candidate = truncate_around_focus(
                window,
                app,
                &style,
                &text,
                &[],
                font_size,
                max_width,
                focus.clone(),
            );

            let visible =
                visible_source_range(&candidate.projection).expect("expected visible span");
            let ellipses = ellipsis_ranges(&candidate.projection);

            assert_eq!(candidate.display_text.as_ref(), "…aaaa…");
            assert_eq!(visible.end - visible.start, 4);
            assert!(visible.start >= focus.start && visible.end <= focus.end);
            assert_eq!(ellipses.len(), 2);
            assert_eq!(ellipses[0].0.end, visible.start);
            assert_eq!(ellipses[1].0.start, visible.end);
        });
    }

    #[gpui::test]
    fn truncate_around_focus_returns_ellipsis_only_when_center_seed_cannot_fit(
        cx: &mut gpui::TestAppContext,
    ) {
        let text: SharedString = "prefix-WWWWWW-suffix".into();
        let focus = 7..13;
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let max_width = display_width(window, TRUNCATION_ELLIPSIS, &style, font_size);

            let candidate =
                truncate_around_focus(window, app, &style, &text, &[], font_size, max_width, focus);

            assert_eq!(candidate.display_text.as_ref(), TRUNCATION_ELLIPSIS);
            assert!(candidate.truncated);
            assert!(visible_source_range(&candidate.projection).is_none());
        });
    }

    #[gpui::test]
    fn truncate_around_focus_handles_multibyte_focus_ranges_on_char_boundaries(
        cx: &mut gpui::TestAppContext,
    ) {
        let text: SharedString = "prefix-éééé-suffix".into();
        let normalized_focus =
            normalized_focus_range(text.as_ref(), Some(8..13)).expect("expected normalized focus");
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let width_one = display_width(window, "…é…", &style, font_size);
            let width_two = display_width(window, "…éé…", &style, font_size);
            let max_width = width_one + (width_two - width_one) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(max_width),
                TextTruncationProfile::Middle,
                &[],
                Some(8..13),
            );

            let visible = visible_source_range(&line.projection).expect("expected visible span");
            assert!(line.truncated);
            assert!(text.is_char_boundary(visible.start));
            assert!(text.is_char_boundary(visible.end));
            assert!(visible.start >= normalized_focus.start);
            assert!(visible.end <= normalized_focus.end);
        });
    }

    #[gpui::test]
    fn middle_truncation_matches_width_maximizing_reference(cx: &mut gpui::TestAppContext) {
        let text: SharedString = "WiWiWiWiWiWiWiWi".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let max_width = display_width(window, "WiWi…WiWi", &style, font_size);

            let expected =
                best_middle_candidate_exhaustive(window, app, &style, &text, font_size, max_width)
                    .expect("expected a fitting middle-truncation candidate");
            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(max_width),
                TextTruncationProfile::Middle,
                &[],
                None,
            );

            assert!(line.truncated);
            assert_eq!(line.display_text.as_ref(), expected.display_text.as_ref());
        });
    }

    #[gpui::test]
    fn focus_truncation_matches_width_maximizing_reference_when_full_focus_fits(
        cx: &mut gpui::TestAppContext,
    ) {
        let text: SharedString = "left-WiWiWiWi-mid-WWWW-suffix".into();
        let focus = text.find("WiWiWiWi-mid").unwrap();
        let focus = focus..focus + "WiWiWiWi-mid".len();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let full_focus = candidate_with_focus_window(&text, &[], focus.start, focus.end);
            let full_focus_width = candidate_width(window, app, &style, font_size, &full_focus);
            let full_text_width = display_width(window, text.as_ref(), &style, font_size);
            let max_width = full_focus_width + (full_text_width - full_focus_width) / 2.0;

            let expected = best_focus_candidate_exhaustive(
                window,
                app,
                &style,
                &text,
                font_size,
                max_width,
                focus.clone(),
                FocusSearchMode::Around,
            )
            .expect("expected a fitting focus-preserving candidate");
            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(max_width),
                TextTruncationProfile::Middle,
                &[],
                Some(focus.clone()),
            );

            assert!(line.truncated);
            assert_eq!(line.display_text.as_ref(), expected.display_text.as_ref());
        });
    }

    #[gpui::test]
    fn focus_truncation_matches_width_maximizing_reference_when_focus_overflows(
        cx: &mut gpui::TestAppContext,
    ) {
        let text: SharedString = "prefix-iiWWiiWWii-suffix".into();
        let focus = text.find("iiWWiiWWii").unwrap();
        let focus = focus..focus + "iiWWiiWWii".len();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let full_focus = candidate_with_focus_window(&text, &[], focus.start, focus.end);
            let full_focus_width = candidate_width(window, app, &style, font_size, &full_focus);
            let ellipsis_width = display_width(window, TRUNCATION_ELLIPSIS, &style, font_size);
            let max_width = ellipsis_width + (full_focus_width - ellipsis_width) / 2.0;

            let expected = best_focus_candidate_exhaustive(
                window,
                app,
                &style,
                &text,
                font_size,
                max_width,
                focus.clone(),
                FocusSearchMode::Within,
            )
            .expect("expected a fitting within-focus candidate");
            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(max_width),
                TextTruncationProfile::Middle,
                &[],
                Some(focus.clone()),
            );

            assert!(line.truncated);
            assert_eq!(line.display_text.as_ref(), expected.display_text.as_ref());
        });
    }

    #[gpui::test]
    fn path_profile_preserves_posix_and_drive_roots_in_display_output(
        cx: &mut gpui::TestAppContext,
    ) {
        let posix: SharedString = "/root/dir1/dir2/file.txt".into();
        let drive: SharedString = "C:\\root\\dir1\\dir2\\file.txt".into();
        let unc: SharedString = r"\\server\share\dir1\dir2\dir3\file.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());

            let posix_display = "/root/…/dir2/file.txt";
            let posix_width = display_width(window, posix_display, &style, font_size);
            let posix_line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &posix,
                Some(posix_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            assert_eq!(posix_line.display_text.as_ref(), posix_display);

            let drive_display = "C:\\root\\…\\dir2\\file.txt";
            let drive_width = display_width(window, drive_display, &style, font_size);
            let drive_line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &drive,
                Some(drive_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            assert_eq!(drive_line.display_text.as_ref(), drive_display);

            let unc_display = r"\\server\share\dir1\…\dir3\file.txt";
            let unc_width = display_width(window, unc_display, &style, font_size);
            let unc_line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &unc,
                Some(unc_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            assert_eq!(unc_line.display_text.as_ref(), unc_display);
        });
    }

    #[gpui::test]
    fn path_profile_prefers_last_dir_and_filename_over_partial_parent_names(
        cx: &mut gpui::TestAppContext,
    ) {
        let path: SharedString = "dir1/dir2/dir3/file_name_alpha.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "dir1/…/dir3/file_name_alpha.txt";
            let max_width = display_width(window, expected, &style, font_size);

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
        });
    }

    #[gpui::test]
    fn path_profile_falls_back_to_filename_only_when_last_dir_shape_is_too_wide(
        cx: &mut gpui::TestAppContext,
    ) {
        let path: SharedString = "dir1/dir2/file_name_alpha.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "dir1/…/file_name_alpha.txt";
            let longer = "dir1/…/dir2/file_name_alpha.txt";
            let expected_width = display_width(window, expected, &style, font_size);
            let longer_width = display_width(window, longer, &style, font_size);
            let max_width = expected_width + (longer_width - expected_width) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
        });
    }

    #[gpui::test]
    fn path_profile_collapses_entire_prefix_before_hiding_filename(cx: &mut gpui::TestAppContext) {
        let path: SharedString = "dir1/dir2/file_name_alpha.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…/file_name_alpha.txt";
            let longer = "dir1/…/file_name_alpha.txt";
            let expected_width = display_width(window, expected, &style, font_size);
            let longer_width = display_width(window, longer, &style, font_size);
            let max_width = expected_width + (longer_width - expected_width) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_drops_separator_before_hiding_filename_tail(cx: &mut gpui::TestAppContext) {
        let path: SharedString = "dir1/dir2/file_name_alpha.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…file_name_alpha.txt";
            let longer = "…/file_name_alpha.txt";
            let expected_width = display_width(window, expected, &style, font_size);
            let longer_width = display_width(window, longer, &style, font_size);
            let max_width = expected_width + (longer_width - expected_width) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_preserves_filename_tail_and_extension_with_single_ellipsis(
        cx: &mut gpui::TestAppContext,
    ) {
        let path: SharedString = "dir1/dir2/file_name_alpha.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…alpha.txt";
            let longer = "…_alpha.txt";
            let expected_width = display_width(window, expected, &style, font_size);
            let longer_width = display_width(window, longer, &style, font_size);
            let max_width = expected_width + (longer_width - expected_width) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_preserves_filename_tail_without_extension_with_single_ellipsis(
        cx: &mut gpui::TestAppContext,
    ) {
        let path: SharedString = "dir1/dir2/file_name_alpha".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…alpha";
            let longer = "…_alpha";
            let expected_width = display_width(window, expected, &style, font_size);
            let longer_width = display_width(window, longer, &style, font_size);
            let max_width = expected_width + (longer_width - expected_width) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_anchor_uses_nearest_left_candidate_within_the_same_tier(
        cx: &mut gpui::TestAppContext,
    ) {
        let path_a: SharedString = "dir1/dir2/dir3/dir4/file_name_alpha.txt".into();
        let path_b: SharedString = "dir1/very_long_directory_name/dir4/file_name_beta.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let natural_a_expected = "dir1/dir2/…/dir4/file_name_alpha.txt";
            let anchored_a_expected = "dir1/…/dir4/file_name_alpha.txt";
            let natural_b_expected = "dir1/…/dir4/file_name_beta.txt";
            let max_width = display_width(window, natural_a_expected, &style, font_size);

            let natural_a = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path_a,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            let natural_b = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path_b,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(natural_a.display_text.as_ref(), natural_a_expected);
            assert_eq!(natural_b.display_text.as_ref(), natural_b_expected);

            let anchor = truncated_line_ellipsis_x(&natural_a)
                .zip(truncated_line_ellipsis_x(&natural_b))
                .map(|(a, b)| a.min(b))
                .expect("expected natural ellipsis positions");

            let anchored_a = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &path_a,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(anchor),
            );
            let anchored_b = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &path_b,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(anchor),
            );

            assert_eq!(anchored_a.display_text.as_ref(), anchored_a_expected);
            assert_eq!(anchored_b.display_text.as_ref(), natural_b_expected);
            assert_eq!(truncated_line_ellipsis_x(&anchored_a), Some(anchor));
            assert_eq!(truncated_line_ellipsis_x(&anchored_b), Some(anchor));
        });
    }

    #[gpui::test]
    fn path_profile_anchor_does_not_force_filename_truncation_when_file_only_tier_fits(
        cx: &mut gpui::TestAppContext,
    ) {
        let path_a: SharedString = "dir1/dir2/file_name_alpha.txt".into();
        let path_b: SharedString = "very_long_directory_name/dir2/file_name_beta.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let file_only_a = "dir1/…/file_name_alpha.txt";
            let collapsed_b = "…/file_name_beta.txt";
            let max_width = display_width(window, file_only_a, &style, font_size);

            let natural_a = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path_a,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            let natural_b = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path_b,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(natural_a.display_text.as_ref(), file_only_a);
            assert_eq!(natural_b.display_text.as_ref(), collapsed_b);

            let anchor = truncated_line_ellipsis_x(&natural_a)
                .zip(truncated_line_ellipsis_x(&natural_b))
                .map(|(a, b)| a.min(b))
                .expect("expected natural ellipsis positions");

            let anchored_a = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &path_a,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(anchor),
            );

            assert_eq!(anchored_a.display_text.as_ref(), file_only_a);
        });
    }

    #[gpui::test]
    fn path_profile_anchor_preserves_posix_drive_and_unc_roots(cx: &mut gpui::TestAppContext) {
        let posix: SharedString = "/root/dir1/dir2/file.txt".into();
        let drive: SharedString = "C:\\root\\dir1\\dir2\\file.txt".into();
        let unc: SharedString = r"\\server\share\dir1\dir2\dir3\file.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let posix_display = "/root/…/dir2/file.txt";
            let drive_display = "C:\\root\\…\\dir2\\file.txt";
            let unc_display = r"\\server\share\dir1\…\dir3\file.txt";
            let posix_width = display_width(window, posix_display, &style, font_size);
            let drive_width = display_width(window, drive_display, &style, font_size);
            let unc_width = display_width(window, unc_display, &style, font_size);
            let posix_anchor = window
                .text_system()
                .shape_line(
                    posix_display.into(),
                    font_size,
                    &vec![style.clone().to_run(posix_display.len())],
                    None,
                )
                .x_for_index("/root/".len());
            let drive_anchor = window
                .text_system()
                .shape_line(
                    drive_display.into(),
                    font_size,
                    &vec![style.clone().to_run(drive_display.len())],
                    None,
                )
                .x_for_index("C:\\root\\".len());
            let unc_anchor = window
                .text_system()
                .shape_line(
                    unc_display.into(),
                    font_size,
                    &vec![style.clone().to_run(unc_display.len())],
                    None,
                )
                .x_for_index(r"\\server\share\dir1\".len());

            let posix_line = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &posix,
                Some(posix_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(posix_anchor),
            );
            let drive_line = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &drive,
                Some(drive_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(drive_anchor),
            );
            let unc_line = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &unc,
                Some(unc_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(unc_anchor),
            );

            assert_eq!(posix_line.display_text.as_ref(), posix_display);
            assert_eq!(drive_line.display_text.as_ref(), drive_display);
            assert_eq!(unc_line.display_text.as_ref(), unc_display);
        });
    }

    #[gpui::test]
    fn path_profile_collapses_root_prefix_before_hiding_filename(cx: &mut gpui::TestAppContext) {
        let posix: SharedString = "/root/dir1/dir2/file.txt".into();
        let drive: SharedString = "C:\\root\\dir1\\dir2\\file.txt".into();
        let unc: SharedString = r"\\server\share\dir1\dir2\file.txt".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());

            let posix_expected = "…/file.txt";
            let posix_longer = "/root/…/file.txt";
            let posix_expected_width = display_width(window, posix_expected, &style, font_size);
            let posix_longer_width = display_width(window, posix_longer, &style, font_size);
            let posix_width =
                posix_expected_width + (posix_longer_width - posix_expected_width) / 2.0;
            let posix_line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &posix,
                Some(posix_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            assert_eq!(posix_line.display_text.as_ref(), posix_expected);

            let drive_expected = "…\\file.txt";
            let drive_longer = "C:\\root\\…\\file.txt";
            let drive_expected_width = display_width(window, drive_expected, &style, font_size);
            let drive_longer_width = display_width(window, drive_longer, &style, font_size);
            let drive_width =
                drive_expected_width + (drive_longer_width - drive_expected_width) / 2.0;
            let drive_line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &drive,
                Some(drive_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            assert_eq!(drive_line.display_text.as_ref(), drive_expected);

            let unc_expected = "…\\file.txt";
            let unc_longer = r"\\server\share\dir1\…\file.txt";
            let unc_expected_width = display_width(window, unc_expected, &style, font_size);
            let unc_longer_width = display_width(window, unc_longer, &style, font_size);
            let unc_width = unc_expected_width + (unc_longer_width - unc_expected_width) / 2.0;
            let unc_line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &unc,
                Some(unc_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            assert_eq!(unc_line.display_text.as_ref(), unc_expected);
        });
    }

    #[gpui::test]
    fn path_profile_falls_back_to_middle_for_non_paths(cx: &mut gpui::TestAppContext) {
        let text: SharedString = "module_name_with_no_separators".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let middle_width = display_width(window, "m…s", &style, font_size);

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(middle_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), "m…s");
        });
    }

    #[gpui::test]
    fn path_profile_single_parent_multibyte_path_keeps_full_filename_before_hiding_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let path: SharedString = "dir/報告_日本語.文書".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…/報告_日本語.文書";
            let longer = "dir/報告_日本語.文書";
            let max_width = width_between(window, expected, longer, &style, font_size);

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_single_parent_multibyte_path_drops_separator_before_hiding_filename_tail(
        cx: &mut gpui::TestAppContext,
    ) {
        let path: SharedString = "dir/報告_日本語.文書".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…報告_日本語.文書";
            let longer = "…/報告_日本語.文書";
            let max_width = width_between(window, expected, longer, &style, font_size);

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_preserves_multibyte_filename_tail_and_extension(cx: &mut gpui::TestAppContext) {
        let path: SharedString = "dir1/dir2/報告_日本語.文書".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let expected = "…日本語.文書";
            let longer = "…_日本語.文書";
            let max_width = width_between(window, expected, longer, &style, font_size);

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(line.display_text.as_ref(), expected);
            assert_eq!(
                line.display_text
                    .as_ref()
                    .chars()
                    .filter(|&ch| ch == '…')
                    .count(),
                1
            );
        });
    }

    #[gpui::test]
    fn path_profile_anchor_does_not_apply_once_filename_tail_tier_is_needed(
        cx: &mut gpui::TestAppContext,
    ) {
        let path_a: SharedString = "very_long_directory_name/a.rs".into();
        let path_b: SharedString = "very_long_directory_name/報告_日本語.文書".into();
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let collapsed_a = "…/a.rs";
            let collapsed_b = "…/報告_日本語.文書";
            let max_width = width_between(window, "…日本語.文書", collapsed_b, &style, font_size);

            let natural_a = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path_a,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );
            let natural_b = shape_truncated_line_cached(
                window,
                app,
                &style,
                &path_b,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
            );

            assert_eq!(natural_a.display_text.as_ref(), collapsed_a);
            assert!(
                natural_b
                    .display_text
                    .as_ref()
                    .starts_with(TRUNCATION_ELLIPSIS)
            );
            assert!(!natural_b.display_text.as_ref().contains('/'));
            assert!(!natural_b.display_text.as_ref().contains('\\'));
            assert!(
                natural_b.display_text.as_ref().ends_with(".文書"),
                "expected filename-tail tier to preserve the extension: {}",
                natural_b.display_text.as_ref()
            );

            let anchor = truncated_line_ellipsis_x(&natural_a)
                .expect("expected natural path ellipsis anchor for collapsed-prefix tier");

            let anchored_b = shape_truncated_line_cached_with_path_anchor(
                window,
                app,
                &style,
                &path_b,
                Some(max_width),
                TextTruncationProfile::Path,
                &[],
                None,
                Some(anchor),
            );

            assert_eq!(
                anchored_b.display_text.as_ref(),
                natural_b.display_text.as_ref()
            );
        });
    }

    #[test]
    fn path_alignment_group_promotes_pending_anchor_on_second_render() {
        let group = TruncatedTextPathAlignmentGroup::default();

        group.begin_visible_rows(7);
        assert_eq!(group.path_anchor_for_layout(Some(px(180.0)), 11), None);
        assert!(group.report_natural_ellipsis(Some(px(180.0)), 11, px(52.0)));
        assert!(!group.report_natural_ellipsis(Some(px(180.0)), 11, px(48.0)));

        let after_first_render = group.snapshot_for_test();
        assert_eq!(after_first_render.resolved_anchor, None);
        assert_eq!(after_first_render.pending_anchor, Some(px(48.0)));

        group.begin_visible_rows(7);
        assert_eq!(
            group.path_anchor_for_layout(Some(px(180.0)), 11),
            Some(px(48.0))
        );

        let after_second_render = group.snapshot_for_test();
        assert_eq!(after_second_render.resolved_anchor, Some(px(48.0)));
        assert_eq!(after_second_render.pending_anchor, None);
    }

    #[test]
    fn path_alignment_group_resets_when_visible_signature_changes() {
        let group = TruncatedTextPathAlignmentGroup::default();

        group.begin_visible_rows(1);
        let _ = group.path_anchor_for_layout(Some(px(160.0)), 11);
        let _ = group.report_natural_ellipsis(Some(px(160.0)), 11, px(44.0));
        group.begin_visible_rows(1);
        assert_eq!(
            group.path_anchor_for_layout(Some(px(160.0)), 11),
            Some(px(44.0))
        );

        group.begin_visible_rows(2);
        assert_eq!(group.path_anchor_for_layout(Some(px(160.0)), 11), None);

        let snapshot = group.snapshot_for_test();
        assert_eq!(snapshot.visible_signature, Some(2));
        assert_eq!(snapshot.resolved_anchor, None);
        assert_eq!(snapshot.pending_anchor, None);
    }

    #[test]
    fn path_alignment_group_resets_when_width_changes() {
        let group = TruncatedTextPathAlignmentGroup::default();

        group.begin_visible_rows(9);
        let _ = group.path_anchor_for_layout(Some(px(120.0)), 11);
        let _ = group.report_natural_ellipsis(Some(px(120.0)), 11, px(36.0));
        group.begin_visible_rows(9);
        assert_eq!(
            group.path_anchor_for_layout(Some(px(120.0)), 11),
            Some(px(36.0))
        );

        group.begin_visible_rows(9);
        assert_eq!(group.path_anchor_for_layout(Some(px(140.0)), 11), None);

        let snapshot = group.snapshot_for_test();
        assert_eq!(
            snapshot.layout_key,
            Some(PathAlignmentLayoutKey {
                width_key: Some(width_cache_key(px(140.0))),
                style_key: 11,
            })
        );
        assert_eq!(snapshot.resolved_anchor, None);
        assert_eq!(snapshot.pending_anchor, None);
    }

    #[test]
    fn path_alignment_group_resets_when_style_metrics_change() {
        let group = TruncatedTextPathAlignmentGroup::default();

        group.begin_visible_rows(13);
        let _ = group.path_anchor_for_layout(Some(px(120.0)), 11);
        let _ = group.report_natural_ellipsis(Some(px(120.0)), 11, px(36.0));
        group.begin_visible_rows(13);
        assert_eq!(
            group.path_anchor_for_layout(Some(px(120.0)), 11),
            Some(px(36.0))
        );

        group.begin_visible_rows(13);
        assert_eq!(group.path_anchor_for_layout(Some(px(120.0)), 22), None);
        let snapshot = group.snapshot_for_test();
        assert_eq!(
            snapshot.layout_key,
            Some(PathAlignmentLayoutKey {
                width_key: Some(width_cache_key(px(120.0))),
                style_key: 22,
            })
        );
        assert_eq!(snapshot.resolved_anchor, None);
        assert_eq!(snapshot.pending_anchor, None);
    }

    #[gpui::test]
    fn two_ellipsis_projection_maps_hidden_boundaries_on_both_sides(cx: &mut gpui::TestAppContext) {
        let text: SharedString = "prefix-aaaaaaaaaa-suffix".into();
        let focus = 7..17;
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

        cx.update(|window, app| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let width_four = display_width(window, "…aaaa…", &style, font_size);
            let width_five = display_width(window, "…aaaaa…", &style, font_size);
            let max_width = width_four + (width_five - width_four) / 2.0;

            let line = shape_truncated_line_cached(
                window,
                app,
                &style,
                &text,
                Some(max_width),
                TextTruncationProfile::Middle,
                &[],
                Some(focus),
            );

            let ellipses = ellipsis_ranges(&line.projection);
            assert_eq!(ellipses.len(), 2);

            let (left_hidden, left_display) = &ellipses[0];
            let (right_hidden, right_display) = &ellipses[1];

            assert_eq!(
                line.projection
                    .display_to_source_offset(left_display.start, Affinity::Start),
                left_hidden.start
            );
            assert_eq!(
                line.projection
                    .display_to_source_offset(left_display.end, Affinity::End),
                left_hidden.end
            );
            assert_eq!(
                line.projection
                    .source_to_display_offset_with_affinity(left_hidden.end, Affinity::Start),
                left_display.end
            );
            assert_eq!(
                line.projection
                    .display_to_source_offset(right_display.start, Affinity::Start),
                right_hidden.start
            );
            assert_eq!(
                line.projection
                    .display_to_source_offset(right_display.end, Affinity::End),
                right_hidden.end
            );
            assert_eq!(
                line.projection
                    .source_to_display_offset_with_affinity(right_hidden.start, Affinity::End),
                right_display.start
            );
        });
    }
}
