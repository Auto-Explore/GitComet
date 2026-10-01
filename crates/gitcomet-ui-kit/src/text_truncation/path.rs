use super::candidate::{
    CandidateLayout, MeasuredCandidate, SegmentSpec, candidate_edge_visible_lengths,
    candidate_from_segments, candidate_visible_source_len, candidate_width, compare_pixels,
    maximize_frontier_candidate, truncate_from_middle, truncate_from_start,
};
use super::char_boundaries;
use gpui::{App, HighlightStyle, Pixels, SharedString, TextStyle, Window};
use std::cmp::Ordering;
use std::ops::Range;

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

#[derive(Clone, Copy)]
struct PathCandidateSource<'a> {
    text: &'a SharedString,
    highlights: &'a [(Range<usize>, HighlightStyle)],
}

#[derive(Clone, Copy)]
struct PathCandidateSearch {
    font_size: Pixels,
    max_width: Pixels,
    left_len: usize,
    right_len: usize,
    path_ellipsis_anchor: Option<Pixels>,
}

fn maximize_path_candidate<F>(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    search: PathCandidateSearch,
    build_candidate: F,
    compare_base: fn(&MeasuredCandidate, &MeasuredCandidate) -> Ordering,
) -> Option<MeasuredCandidate>
where
    F: FnMut(usize, usize) -> Option<CandidateLayout>,
{
    let PathCandidateSearch {
        font_size,
        max_width,
        left_len,
        right_len,
        path_ellipsis_anchor,
    } = search;

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
    source: PathCandidateSource<'_>,
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
        PathCandidateSearch {
            font_size,
            max_width,
            left_len: prefix_options.len(),
            right_len: 1,
            path_ellipsis_anchor,
        },
        |left_ix, _| {
            let prefix_end = prefix_options[left_ix];
            Some(candidate_from_segments(
                source.text,
                &[
                    SegmentSpec::Source(0..prefix_end),
                    SegmentSpec::Ellipsis(prefix_end..suffix_start),
                    SegmentSpec::Source(suffix_start..source.text.len()),
                ],
                source.highlights,
            ))
        },
        compare_path_measured_candidates,
    )
}

fn truncate_path_with_preserved_suffix_tier(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    source: PathCandidateSource<'_>,
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
        source,
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
        PathCandidateSearch {
            font_size,
            max_width,
            left_len: 1,
            right_len: tail_starts.len(),
            path_ellipsis_anchor: None,
        },
        |_, right_ix| {
            let tail_start = tail_starts[right_ix];
            Some(candidate_with_hidden_prefix(text, highlights, tail_start))
        },
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

pub(super) fn truncate_path_like(
    window: &mut Window,
    cx: &mut App,
    base_style: &TextStyle,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    font_size: Pixels,
    max_width: Pixels,
    path_ellipsis_anchor: Option<Pixels>,
) -> CandidateLayout {
    let source = PathCandidateSource { text, highlights };
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
            source,
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

pub(super) struct PathBoundaries {
    prefix_cuts: Vec<usize>,
    separator_starts: Vec<usize>,
    pub(super) min_prefix_end: usize,
    pub(super) min_suffix_start: usize,
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

pub(super) fn path_boundaries(text: &str) -> Option<PathBoundaries> {
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
