use super::path::truncate_path_like;
use super::projection::{
    ProjectionSegment, TruncationProjection, ellipsis_x_for_projection_and_line,
};
use super::{
    TRUNCATION_ELLIPSIS, TextTruncationProfile, char_boundaries, compute_highlight_runs,
    record_measure_candidate_call_for_test,
};
use gpui::{App, HighlightStyle, Pixels, SharedString, TextStyle, Window, px};
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) struct CandidateLayout {
    pub(super) display_text: SharedString,
    pub(super) display_highlights: Vec<(Range<usize>, HighlightStyle)>,
    pub(super) projection: Arc<TruncationProjection>,
    pub(super) truncated: bool,
}
pub(super) enum SegmentSpec {
    Source(Range<usize>),
    Ellipsis(Range<usize>),
}

#[derive(Clone)]
pub(super) struct MeasuredCandidate {
    pub(super) candidate: CandidateLayout,
    pub(super) width: Pixels,
    pub(super) ellipsis_x: Option<Pixels>,
}

pub(super) struct TruncatedCandidateParams<'a> {
    pub(super) text: &'a SharedString,
    pub(super) max_width: Option<Pixels>,
    pub(super) profile: TextTruncationProfile,
    pub(super) highlights: &'a [(Range<usize>, HighlightStyle)],
    pub(super) normalized_focus_range: Option<Range<usize>>,
    pub(super) path_ellipsis_anchor: Option<Pixels>,
    pub(super) font_size: Pixels,
}

pub(super) fn compare_pixels(lhs: Pixels, rhs: Pixels) -> Ordering {
    f32::from(lhs).total_cmp(&f32::from(rhs))
}

pub(super) fn candidate_visible_source_len(candidate: &CandidateLayout) -> usize {
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

pub(super) fn candidate_edge_visible_lengths(candidate: &CandidateLayout) -> (usize, usize) {
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

pub(super) fn compare_middle_measured_candidates(
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

pub(super) fn compare_focus_measured_candidates(
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

pub(super) fn maximize_frontier_candidate<F, C>(
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

fn widest_fitting_monotonic_candidate<F>(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    max_width: Pixels,
    option_len: usize,
    mut build_candidate: F,
) -> Option<CandidateLayout>
where
    F: FnMut(usize) -> CandidateLayout,
{
    let mut lo = 0usize;
    let mut hi = option_len;
    let mut best = None;

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let candidate = build_candidate(mid);
        if candidate_width(window, cx, base_style, font_size, &candidate) <= max_width {
            best = Some(candidate);
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    best
}

pub(super) fn build_truncated_candidate(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    params: TruncatedCandidateParams<'_>,
) -> CandidateLayout {
    let TruncatedCandidateParams {
        text,
        max_width,
        profile,
        highlights,
        normalized_focus_range,
        path_ellipsis_anchor,
        font_size,
    } = params;

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

pub(super) fn truncate_from_end(
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

    widest_fitting_monotonic_candidate(
        window,
        cx,
        base_style,
        font_size,
        max_width,
        boundaries.len().saturating_sub(1),
        |option_ix| {
            let prefix_end = boundaries[option_ix + 1];
            candidate_from_segments(
                text,
                &[
                    SegmentSpec::Source(0..prefix_end),
                    SegmentSpec::Ellipsis(prefix_end..text.len()),
                ],
                highlights,
            )
        },
    )
    .unwrap_or_else(|| ellipsis_only_candidate(text.len()))
}

pub(super) fn truncate_from_start(
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

    widest_fitting_monotonic_candidate(
        window,
        cx,
        base_style,
        font_size,
        max_width,
        boundaries.len().saturating_sub(1),
        |option_ix| {
            let suffix_boundary_ix = boundaries.len().saturating_sub(2 + option_ix);
            let suffix_start = boundaries[suffix_boundary_ix];
            candidate_from_segments(
                text,
                &[
                    SegmentSpec::Ellipsis(0..suffix_start),
                    SegmentSpec::Source(suffix_start..text.len()),
                ],
                highlights,
            )
        },
    )
    .unwrap_or_else(|| ellipsis_only_candidate(text.len()))
}

pub(super) fn truncate_from_middle(
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

pub(super) fn truncate_around_focus(
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

pub(super) fn candidate_with_focus_window(
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

pub(super) fn candidate_from_segments(
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

pub(super) fn measure_candidate(
    window: &mut Window,
    _cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    candidate: &CandidateLayout,
) -> (Pixels, Option<Pixels>) {
    record_measure_candidate_call_for_test();

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

pub(super) fn candidate_width(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    font_size: Pixels,
    candidate: &CandidateLayout,
) -> Pixels {
    measure_candidate(window, cx, base_style, font_size, candidate).0
}
