use super::candidate::{
    CandidateLayout, MeasuredCandidate, SegmentSpec, candidate_from_segments, candidate_width,
    candidate_with_focus_window, compare_focus_measured_candidates,
    compare_middle_measured_candidates, measure_candidate, truncate_around_focus,
    truncate_from_start,
};
use super::path::path_boundaries;
use super::path_alignment::{PathAlignmentLayoutKey, PathTruncationAlignmentGroup};
use super::projection::{Affinity, ProjectionSegment, TruncationProjection};
use super::*;
use gpui::{FontFallbacks, FontFeatures, StrikethroughStyle, UnderlineStyle, hsla, px};
use smallvec::SmallVec;
use std::cmp::Ordering;

#[derive(Clone, Copy)]
enum FocusSearchMode {
    Around,
    Within,
}

fn display_width(window: &mut Window, text: &str, style: &TextStyle, font_size: Pixels) -> Pixels {
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
                compare_focus_measured_candidates(&candidate, current, &focus) == Ordering::Greater
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
        font_fallbacks.font_fallbacks = Some(FontFallbacks::from_fonts(vec!["Noto Sans".into()]));

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
fn truncated_layout_background_flag_includes_base_style_background(cx: &mut gpui::TestAppContext) {
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
fn end_truncation_uses_bounded_candidate_measurements(cx: &mut gpui::TestAppContext) {
    clear_truncated_layout_cache_for_test();
    reset_measure_candidate_calls_for_test();
    let text: SharedString = "a".repeat(512).into();
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let style = window.text_style();
        let line = shape_truncated_line_cached(
            window,
            app,
            &style,
            &text,
            Some(px(80.0)),
            TextTruncationProfile::End,
            &[],
            None,
        );

        assert!(line.truncated, "expected the line to truncate");
        assert!(
            measure_candidate_calls_for_test() <= 16,
            "end truncation should not measure each character candidate"
        );
    });
    clear_truncated_layout_cache_for_test();
}

#[gpui::test]
fn start_truncation_uses_bounded_candidate_measurements(cx: &mut gpui::TestAppContext) {
    reset_measure_candidate_calls_for_test();
    let text: SharedString = "a".repeat(512).into();
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let candidate = truncate_from_start(window, app, &style, &text, &[], font_size, px(80.0));

        assert!(candidate.truncated, "expected the candidate to truncate");
        assert!(
            measure_candidate_calls_for_test() <= 16,
            "start truncation should not measure each character candidate"
        );
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

        let visible = visible_source_range(&candidate.projection).expect("expected visible span");
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
fn path_profile_preserves_posix_and_drive_roots_in_display_output(cx: &mut gpui::TestAppContext) {
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
                &[style.clone().to_run(posix_display.len())],
                None,
            )
            .x_for_index("/root/".len());
        let drive_anchor = window
            .text_system()
            .shape_line(
                drive_display.into(),
                font_size,
                &[style.clone().to_run(drive_display.len())],
                None,
            )
            .x_for_index("C:\\root\\".len());
        let unc_anchor = window
            .text_system()
            .shape_line(
                unc_display.into(),
                font_size,
                &[style.clone().to_run(unc_display.len())],
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
        let posix_width = posix_expected_width + (posix_longer_width - posix_expected_width) / 2.0;
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
        let drive_width = drive_expected_width + (drive_longer_width - drive_expected_width) / 2.0;
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
    let group = PathTruncationAlignmentGroup::default();

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
    let group = PathTruncationAlignmentGroup::default();

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
    let group = PathTruncationAlignmentGroup::default();

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
    let group = PathTruncationAlignmentGroup::default();

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
