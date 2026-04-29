use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

#[test]
fn mask_text_preserves_length_and_newlines() {
    let input = "a\nb\r\nc";
    let masked = mask_text_for_display(input);
    assert_eq!(masked.len(), input.len());
    assert_eq!(masked, "*\n*\r\n*");
}

#[test]
fn mask_text_removes_original_characters() {
    let input = "secret-passphrase";
    let masked = mask_text_for_display(input);
    assert_ne!(masked, input);
    assert!(masked.chars().all(|ch| ch == '*'));
}

#[test]
fn truncate_line_for_shaping_respects_utf8_boundary_and_appends_suffix() {
    let input = "éééé";
    let (truncated, hash) = truncate_line_for_shaping(input, 5);
    assert_eq!(truncated.as_ref(), "é…");
    // hash_shaping_slice must be consistent with truncate_line_for_shaping
    let (hash2, _) = hash_shaping_slice(input, 5);
    assert_eq!(hash, hash2);
}

#[test]
fn visible_plain_line_range_applies_guard_rows() {
    let range = visible_plain_line_range(100, px(20.0), px(200.0), px(260.0), 2);
    assert_eq!(range, 8..16);
}

#[test]
fn provider_prefetch_byte_range_extends_visible_window_with_guard_rows() {
    let text = std::iter::repeat_n("x", 100).collect::<Vec<_>>().join("\n");
    let line_starts = compute_line_starts(text.as_str());
    let range = provider_prefetch_byte_range_for_visible_window(
        line_starts.as_slice(),
        text.len(),
        100,
        px(20.0),
        px(600.0),
        px(660.0),
    );

    assert_eq!(range, 12..116);
}

#[test]
fn provider_prefetch_byte_range_clamps_to_document_bounds() {
    let text = std::iter::repeat_n("x", 10).collect::<Vec<_>>().join("\n");
    let line_starts = compute_line_starts(text.as_str());
    let range = provider_prefetch_byte_range_for_visible_window(
        line_starts.as_slice(),
        text.len(),
        10,
        px(20.0),
        px(0.0),
        px(20.0),
    );

    assert_eq!(range, 0..text.len());
}

#[test]
fn wrapped_line_index_and_visible_range_use_row_counts() {
    let row_counts = vec![1, 3, 1, 2, 1];
    let y_offsets = vec![px(0.0), px(10.0), px(40.0), px(50.0), px(70.0)];
    let line_height = px(10.0);

    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(35.0)),
        1
    );
    let range =
        visible_wrapped_line_range(&y_offsets, &row_counts, line_height, px(42.0), px(58.0), 0);
    assert_eq!(range, 2..4);
}

#[test]
fn compute_line_starts_and_line_text_handle_trailing_newline() {
    let text = "alpha\nbeta\n";
    let starts = compute_line_starts(text);
    assert_eq!(starts, vec![0, 6, 11]);
    assert_eq!(line_text_for_index(text, starts.as_slice(), 0), "alpha");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 1), "beta");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 2), "");
    assert_eq!(line_text_for_index(text, starts.as_slice(), 3), "");
}

#[test]
fn wrapped_line_index_for_y_handles_row_boundaries() {
    let row_counts = vec![2, 1, 3];
    let y_offsets = vec![px(0.0), px(20.0), px(30.0)];
    let line_height = px(10.0);

    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(0.0)),
        0
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(19.0)),
        0
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(20.0)),
        1
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(30.0)),
        2
    );
    assert_eq!(
        wrapped_line_index_for_y(&y_offsets, &row_counts, line_height, px(250.0)),
        2
    );
}

#[test]
fn estimate_wrap_rows_for_line_handles_tabs_and_overflow() {
    assert_eq!(estimate_wrap_rows_for_line("abcd", 4), 1);
    assert_eq!(estimate_wrap_rows_for_line("abcde", 4), 2);
    assert_eq!(estimate_wrap_rows_for_line("a\tb", 4), 2);
}

#[test]
fn estimate_wrap_rows_for_line_matches_reference_for_ascii_tabs() {
    fn reference_wrap_rows_for_line(line_text: &str, wrap_columns: usize) -> usize {
        if line_text.is_empty() {
            return 1;
        }
        let wrap_columns = wrap_columns.max(1);
        let mut rows = 1usize;
        let mut column = 0usize;
        for ch in line_text.chars() {
            let width = if ch == '\t' {
                let rem = column % TEXT_INPUT_WRAP_TAB_STOP_COLUMNS;
                if rem == 0 {
                    TEXT_INPUT_WRAP_TAB_STOP_COLUMNS
                } else {
                    TEXT_INPUT_WRAP_TAB_STOP_COLUMNS - rem
                }
            } else {
                1
            };

            if width >= wrap_columns {
                if column > 0 {
                    rows += 1;
                }
                rows += width / wrap_columns;
                column = width % wrap_columns;
                if column == 0 {
                    column = wrap_columns;
                }
                continue;
            }

            if column + width > wrap_columns {
                rows += 1;
                column = width;
            } else {
                column += width;
            }
        }
        rows.max(1)
    }

    let samples = [
        "",
        "\t",
        "a\tb",
        "ab\tcd\tef",
        "\tsection_00000\tvalue = token\ttoken\ttoken\ttoken\t",
        "token\ttoken\ttoken\ttoken\ttoken\t",
        "abcd",
        "abcde",
        "\t\t\t",
        "trailing-tab\t",
    ];

    for wrap_columns in (TEXT_INPUT_WRAP_TAB_STOP_COLUMNS + 1)..=12 {
        for sample in samples {
            assert_eq!(
                estimate_wrap_rows_for_line(sample, wrap_columns),
                reference_wrap_rows_for_line(sample, wrap_columns),
                "sample={sample:?}, wrap_columns={wrap_columns}"
            );
        }
    }
}

#[test]
fn expanded_dirty_wrap_line_range_for_edit_keeps_tab_affected_line_dirty() {
    let text = "ax\tbb\nnext";
    let starts = compute_line_starts(text);
    let dirty = expanded_dirty_wrap_line_range_for_edit(text, starts.as_slice(), &(1..1), &(1..2));
    assert_eq!(dirty, 0..1);
}

#[test]
fn apply_interpolated_wrap_patch_delta_adjusts_rows_by_delta() {
    let mut rows = vec![6, 5, 4, 3];
    let patch = InterpolatedWrapPatch {
        width_key: 80,
        line_start: 1,
        old_rows: vec![3, 2],
        new_rows: vec![5, 1],
    };
    apply_interpolated_wrap_patch_delta(rows.as_mut_slice(), &patch);
    assert_eq!(rows, vec![6, 7, 3, 3]);
}

#[test]
fn reset_interpolated_wrap_patches_on_overflow_requests_full_recompute() {
    let patch = InterpolatedWrapPatch {
        width_key: 80,
        line_start: 12,
        old_rows: vec![1],
        new_rows: vec![2],
    };

    let mut below_limit =
        vec![patch.clone(); TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES.saturating_sub(1)];
    let mut recompute_requested = false;
    assert!(!reset_interpolated_wrap_patches_on_overflow(
        &mut below_limit,
        &mut recompute_requested
    ));
    assert_eq!(
        below_limit.len(),
        TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES.saturating_sub(1)
    );
    assert!(!recompute_requested);

    let mut saturated = vec![patch; TEXT_INPUT_MAX_INTERPOLATED_WRAP_PATCHES];
    assert!(reset_interpolated_wrap_patches_on_overflow(
        &mut saturated,
        &mut recompute_requested
    ));
    assert!(saturated.is_empty());
    assert!(recompute_requested);
}

#[test]
fn pending_wrap_job_accepts_interpolated_patch_respects_prepaint_launch_gate() {
    let job = PendingWrapJob {
        sequence: 5,
        width_key: 120,
        line_count: 64,
        wrap_columns: 80,
    };

    assert!(pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        120,
        64,
        true
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        120,
        64,
        false
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        121,
        64,
        true
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        Some(&job),
        120,
        63,
        true
    ));
    assert!(!pending_wrap_job_accepts_interpolated_patch(
        None, 120, 64, true
    ));
}

fn runs_fingerprint(runs: &[TextRun]) -> Vec<String> {
    runs.iter().map(|run| format!("{run:?}")).collect()
}

fn run_color_at_offset(runs: &[TextRun], offset: usize) -> gpui::Hsla {
    let mut cursor = 0usize;
    for run in runs {
        let end = cursor.saturating_add(run.len);
        if offset < end {
            return run.color;
        }
        cursor = end;
    }
    panic!("offset {offset} is outside the run coverage");
}

#[test]
fn highlight_runs_skip_hidden_overlap_end_boundaries() {
    let text = "abcdefghijklmnop";
    let line_starts = compute_line_starts(text);
    let style_low = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_mid = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_high = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights = vec![(0..10, style_low), (2..8, style_mid), (4..12, style_high)];
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts.as_slice(),
        0..1,
        highlights.as_slice(),
    );
    let legacy_runs = runs_for_line(&base_font, base_color, 0, text, Some(highlights.as_slice()));

    assert_eq!(streamed.line(0).unwrap_or(&[]).len(), 4);
    assert_eq!(legacy_runs.len(), 4);
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 1),
        style_low.color.expect("style_low color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 3),
        style_mid.color.expect("style_mid color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 6),
        style_high.color.expect("style_high color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 14),
        base_color
    );
}

#[test]
fn streamed_highlight_runs_match_legacy_visible_window() {
    let mut text = String::new();
    for ix in 0..160usize {
        text.push_str(format!("line_{ix:03}_abcdefghijklmnopqrstuvwxyz0123456789\n").as_str());
    }
    let line_starts = compute_line_starts(text.as_str());

    let style_a = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_b = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_c = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> = Vec::new();
    for line_ix in 0..line_starts.len() {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_len = line_text_for_index(text.as_str(), line_starts.as_slice(), line_ix).len();
        if line_len < 24 {
            continue;
        }
        if line_ix % 2 == 0 {
            highlights.push((line_start + 1..line_start + 14, style_a));
        }
        if line_ix % 3 == 0 {
            highlights.push((line_start + 6..line_start + line_len.min(24), style_b));
        }
    }
    let wide_start = line_starts.get(18).copied().unwrap_or(0).saturating_add(2);
    let wide_end = line_starts
        .get(140)
        .copied()
        .unwrap_or(text.len())
        .saturating_add(20)
        .min(text.len());
    highlights.push((wide_start..wide_end, style_c));
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let visible_range = 47..121;
    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text.as_str(),
        line_starts.as_slice(),
        visible_range.clone(),
        highlights.as_slice(),
    );
    assert_eq!(streamed.len(), visible_range.len());

    for local_ix in 0..streamed.len() {
        let line_ix = visible_range.start + local_ix;
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(text.as_str(), line_starts.as_slice(), line_ix);
        let (capped, _) = truncate_line_for_shaping(line_text, TEXT_INPUT_MAX_LINE_SHAPE_BYTES);
        let legacy_runs = runs_for_line(
            &base_font,
            base_color,
            line_start,
            capped.as_ref(),
            Some(highlights.as_slice()),
        );
        assert_eq!(
            runs_fingerprint(streamed.line(local_ix).unwrap_or(&[])),
            runs_fingerprint(legacy_runs.as_slice())
        );
    }
}

#[test]
fn streamed_highlight_runs_preserve_latest_overlap_precedence() {
    let text = "abcdefghijklmnop";
    let line_starts = compute_line_starts(text);
    let style_low = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let style_high = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights = vec![(2..12, style_low), (4..10, style_high)];
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts.as_slice(),
        0..1,
        highlights.as_slice(),
    );
    let legacy_runs = runs_for_line(&base_font, base_color, 0, text, Some(highlights.as_slice()));
    assert_eq!(
        runs_fingerprint(streamed.line(0).unwrap_or(&[])),
        runs_fingerprint(legacy_runs.as_slice())
    );

    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 3),
        style_low.color.expect("style_low color should exist")
    );
    assert_eq!(
        run_color_at_offset(streamed.line(0).unwrap_or(&[]), 6),
        style_high.color.expect("style_high color should exist")
    );
}

#[test]
fn highlight_runs_single_carry_in_highlight_matches_streamed() {
    let text = "prefix highlight continues here\nsuffix line";
    let line_starts = compute_line_starts(text);
    let style = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.12, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let mut highlights = vec![(3..30, style)];
    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let base_font = gpui::font(".SystemUIFont");
    let base_color = gpui::hsla(0.0, 0.0, 1.0, 1.0);
    let streamed = build_streamed_highlight_runs_for_visible_window(
        &base_font,
        base_color,
        text,
        line_starts.as_slice(),
        0..2,
        highlights.as_slice(),
    );

    for line_ix in 0..2 {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let line_text = line_text_for_index(text, line_starts.as_slice(), line_ix);
        let legacy_runs = runs_for_line(
            &base_font,
            base_color,
            line_start,
            line_text,
            Some(highlights.as_slice()),
        );
        assert_eq!(
            runs_fingerprint(streamed.line(line_ix).unwrap_or(&[])),
            runs_fingerprint(legacy_runs.as_slice())
        );
    }
}

#[test]
fn resolve_provider_highlights_caches_by_epoch_and_range() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    // Simulate the cache behavior without needing a full GPUI context.
    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch: u64 = 1;

    let h1 = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert!(!h1.pending);
    assert_eq!(h1.highlights.len(), 1);
    assert_eq!(h1.highlights[0].0, 0..100);

    // Same range and epoch → cached, no new call.
    let h2 = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&h1.highlights, &h2.highlights));

    // Contained range → cached, no new call.
    let h3 = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&h1.highlights, &h3.highlights));

    // Wider range → new call.
    let _h4 = test_resolve_with_cache(&mut cache, epoch, 0, 120, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    // Different epoch → new call even for same range.
    let _h5 = test_resolve_with_cache(&mut cache, epoch + 1, 0, 120, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

#[test]
fn resolve_provider_highlights_reuses_multiple_cached_ranges() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;

    let first = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    let second = test_resolve_with_cache(&mut cache, epoch, 200, 300, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    let first_subrange = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(Arc::ptr_eq(&first.highlights, &first_subrange.highlights));

    let second_subrange = test_resolve_with_cache(&mut cache, epoch, 220, 260, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(Arc::ptr_eq(&second.highlights, &second_subrange.highlights));

    let cache = cache.expect("resolved ranges should populate the provider cache");
    assert_eq!(cache.highlight_epoch, epoch);
    assert_eq!(cache.entries.len(), 2);
}

#[test]
fn resolve_provider_highlights_prefers_smallest_containing_cached_range() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;

    let narrow = test_resolve_with_cache(&mut cache, epoch, 50, 150, &provider);
    let wide = test_resolve_with_cache(&mut cache, epoch, 0, 200, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    let resolved = test_resolve_with_cache(&mut cache, epoch, 60, 140, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
    assert!(
        Arc::ptr_eq(&resolved.highlights, &narrow.highlights),
        "the smallest cached containing slice should win even if a wider slice is newer"
    );
    assert!(
        !Arc::ptr_eq(&resolved.highlights, &wide.highlights),
        "the wider containing slice should not be reused when a tighter one exists"
    );

    let cache = cache.expect("resolved ranges should populate the provider cache");
    assert_eq!(cached_provider_ranges(&cache), vec![0..200, 50..150]);
}

#[test]
fn resolve_provider_highlights_cache_is_bounded() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;
    for window in 0..TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT {
        let start = window * 100;
        let end = start + 100;
        let _ = test_resolve_with_cache(&mut cache, epoch, start, end, &provider);
    }
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
    );

    let _ = test_resolve_with_cache(
        &mut cache,
        epoch,
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT * 100,
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT * 100 + 100,
        &provider,
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT + 1
    );

    let cache_ref = cache.as_ref().expect("cache should retain recent ranges");
    assert_eq!(
        cache_ref.entries.len(),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
    );

    let _ = test_resolve_with_cache(&mut cache, epoch, 0, 50, &provider);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT + 2,
        "the oldest cached slice should be evicted once the cache reaches its bound"
    );
}

#[test]
fn resolve_provider_highlights_cache_hit_promotes_entry_before_eviction() {
    use std::sync::atomic::Ordering;

    let (call_count, provider) = make_counting_provider();

    let mut cache: Option<ProviderHighlightCache> = None;
    let epoch = 1;

    let first = test_resolve_with_cache(&mut cache, epoch, 0, 100, &provider);
    let _second = test_resolve_with_cache(&mut cache, epoch, 100, 200, &provider);
    let _third = test_resolve_with_cache(&mut cache, epoch, 200, 300, &provider);
    let _fourth = test_resolve_with_cache(&mut cache, epoch, 300, 400, &provider);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        TEXT_INPUT_PROVIDER_HIGHLIGHT_CACHE_LIMIT
    );

    let promoted = test_resolve_with_cache(&mut cache, epoch, 20, 80, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 4);
    assert!(Arc::ptr_eq(&promoted.highlights, &first.highlights));

    let _fifth = test_resolve_with_cache(&mut cache, epoch, 400, 500, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 5);

    let cache_ref = cache
        .as_ref()
        .expect("cache should retain recent ranges after a bounded insert");
    assert_eq!(
        cached_provider_ranges(cache_ref),
        vec![200..300, 300..400, 0..100, 400..500]
    );

    let reused = test_resolve_with_cache(&mut cache, epoch, 10, 50, &provider);
    assert_eq!(call_count.load(Ordering::SeqCst), 5);
    assert!(
        Arc::ptr_eq(&reused.highlights, &first.highlights),
        "a cache hit should keep the promoted slice resident across the next eviction"
    );

    let _evicted = test_resolve_with_cache(&mut cache, epoch, 120, 180, &provider);
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        6,
        "the cold slice should be evicted instead of the recently-used one"
    );
}

#[test]
fn highlight_provider_binding_key_reuses_existing_provider_when_unchanged() {
    assert!(!should_reset_highlight_provider_binding(
        true,
        Some(41),
        Some(41)
    ));
}

#[test]
fn highlight_provider_binding_key_rebinds_when_missing_changed_or_unkeyed() {
    assert!(should_reset_highlight_provider_binding(
        false,
        Some(41),
        Some(41)
    ));
    assert!(should_reset_highlight_provider_binding(
        true,
        Some(41),
        Some(42)
    ));
    assert!(should_reset_highlight_provider_binding(
        true,
        Some(41),
        None
    ));
}

fn test_resolve_with_cache(
    cache: &mut Option<ProviderHighlightCache>,
    epoch: u64,
    byte_start: usize,
    byte_end: usize,
    provider: &HighlightProvider,
) -> ResolvedProviderHighlights {
    let requested_range = byte_start..byte_end;
    if let Some(resolved) = cache
        .as_mut()
        .and_then(|c| c.resolve(epoch, &requested_range))
    {
        return resolved;
    }
    let mut result = provider.resolve(requested_range.clone());
    result
        .highlights
        .sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    let pending = result.pending;
    let highlights = Arc::new(result.highlights);
    cache
        .get_or_insert_with(|| ProviderHighlightCache::new(epoch))
        .insert(epoch, requested_range, pending, Arc::clone(&highlights));
    ResolvedProviderHighlights {
        pending,
        highlights,
    }
}

fn make_counting_provider() -> (Arc<std::sync::atomic::AtomicUsize>, HighlightProvider) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let call_count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&call_count);
    let provider = HighlightProvider::from_fn(move |range: Range<usize>| {
        counter.fetch_add(1, Ordering::SeqCst);
        vec![(
            range,
            gpui::HighlightStyle {
                color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
                ..gpui::HighlightStyle::default()
            },
        )]
    });

    (call_count, provider)
}

fn cached_provider_ranges(cache: &ProviderHighlightCache) -> Vec<Range<usize>> {
    cache
        .entries
        .iter()
        .map(|entry| entry.byte_start..entry.byte_end)
        .collect()
}

#[gpui::test]
fn truncated_read_only_select_all_returns_full_source_text(cx: &mut gpui::TestAppContext) {
    let text = "0123456789abcdef0123456789abcdef";
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                read_only: true,
                chromeless: true,
                soft_wrap: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text(text, cx);
            input.set_display_truncation(Some(TextTruncationProfile::Middle), cx);
            input.select_all_text(cx);

            assert_eq!(input.selected_text(), Some(text.to_string()));
        });
    });
}

#[gpui::test]
fn truncated_line_hit_testing_snaps_ellipsis_to_hidden_range_boundaries(
    cx: &mut gpui::TestAppContext,
) {
    let text: SharedString = "0123456789abcdef0123456789abcdef".into();
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let line = shape_truncated_line_cached(
            window,
            app,
            &window.text_style(),
            &text,
            Some(px(80.0)),
            TextTruncationProfile::Middle,
            &[],
            None,
        );

        assert!(line.truncated, "expected the line to truncate");
        let (hidden_range, display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(text.len() / 2)
            .expect("expected a middle ellipsis segment");

        let start_x = line.shaped_line.x_for_index(display_range.start);
        let end_x = line.shaped_line.x_for_index(display_range.end);
        let span = end_x - start_x;
        let left_x = start_x + span / 4.0;
        let right_x = start_x + (span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_x),
            hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_x),
            hidden_range.end
        );
    });
}

#[gpui::test]
fn focused_truncated_line_hit_testing_snaps_both_ellipsis_segments_to_hidden_boundaries(
    cx: &mut gpui::TestAppContext,
) {
    let text: SharedString = "prefix-aaaaaaaaaa-suffix".into();
    let focus = 7..17;
    let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);

    cx.update(|window, app| {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let runs_four = vec![style.clone().to_run("…aaaa…".len())];
        let runs_five = vec![style.clone().to_run("…aaaaa…".len())];
        let width_four = window
            .text_system()
            .shape_line("…aaaa…".into(), font_size, &runs_four, None)
            .width;
        let width_five = window
            .text_system()
            .shape_line("…aaaaa…".into(), font_size, &runs_five, None)
            .width;
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

        assert!(line.truncated, "expected the line to truncate");

        let (left_hidden_range, left_display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(0)
            .expect("expected a left ellipsis segment");
        let (right_hidden_range, right_display_range) = line
            .projection
            .ellipsis_segment_for_source_offset(text.len())
            .expect("expected a right ellipsis segment");

        assert_ne!(left_display_range, right_display_range);

        let left_x0 = line.shaped_line.x_for_index(left_display_range.start);
        let left_x1 = line.shaped_line.x_for_index(left_display_range.end);
        let left_span = left_x1 - left_x0;
        let left_inside = left_x0 + left_span / 4.0;
        let left_outside = left_x0 + (left_span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_inside),
            left_hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, left_outside),
            left_hidden_range.end
        );

        let right_x0 = line.shaped_line.x_for_index(right_display_range.start);
        let right_x1 = line.shaped_line.x_for_index(right_display_range.end);
        let right_span = right_x1 - right_x0;
        let right_inside = right_x0 + right_span / 4.0;
        let right_outside = right_x0 + (right_span * 3.0) / 4.0;

        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_inside),
            right_hidden_range.start
        );
        assert_eq!(
            truncated_line_source_offset_for_x(&line, right_outside),
            right_hidden_range.end
        );
    });
}

struct DualProviders {
    first_calls: Arc<std::sync::atomic::AtomicUsize>,
    second_calls: Arc<std::sync::atomic::AtomicUsize>,
    first_color: gpui::Hsla,
    second_color: gpui::Hsla,
    first: HighlightProvider,
    second: HighlightProvider,
}

fn make_dual_providers() -> DualProviders {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let first_color = gpui::hsla(0.0, 1.0, 0.5, 1.0);
    let second_color = gpui::hsla(0.66, 1.0, 0.5, 1.0);
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let fc = Arc::clone(&first_calls);
    let sc = Arc::clone(&second_calls);
    DualProviders {
        first_calls,
        second_calls,
        first_color,
        second_color,
        first: HighlightProvider::from_fn(move |range: Range<usize>| {
            fc.fetch_add(1, Ordering::SeqCst);
            vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(first_color),
                    ..gpui::HighlightStyle::default()
                },
            )]
        }),
        second: HighlightProvider::from_fn(move |range: Range<usize>| {
            sc.fetch_add(1, Ordering::SeqCst);
            vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(second_color),
                    ..gpui::HighlightStyle::default()
                },
            )]
        }),
    }
}

#[gpui::test]
fn multiline_shift_enter_inserts_a_line_break(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let expected = format!("alpha{}", input.line_ending);

            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), expected);
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should insert a newline instead of flagging enter-pressed"
            );
        });
    });
}

#[gpui::test]
fn single_line_shift_enter_is_a_noop(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);

            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), "alpha");
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should not submit or modify single-line inputs"
            );
        });
    });
}

#[gpui::test]
fn stable_highlight_provider_binding_key_preserves_existing_provider_and_cache(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

            let initial_resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(initial_resolved.highlights[0].1.color, Some(dp.first_color));

            let initial_cache = input
                .highlight
                .provider_cache
                .as_ref()
                .expect("initial resolve should populate the provider cache");
            assert_eq!(initial_cache.entries.len(), 1);
            let initial_entry = initial_cache
                .entries
                .last()
                .expect("initial cache should contain one provider slice");
            assert_eq!(initial_entry.byte_start, 0);
            assert_eq!(initial_entry.byte_end, 5);

            let initial_highlight_epoch = input.highlight.epoch;
            let initial_shape_epoch = input.layout.shape_style_epoch;
            let initial_cached_highlights = Arc::clone(&initial_entry.highlights);

            input.set_highlight_provider_with_key(41, dp.second.clone(), cx);

            assert_eq!(
                input.highlight.epoch, initial_highlight_epoch,
                "reinstalling the same binding key should not invalidate provider highlights"
            );
            assert_eq!(
                input.layout.shape_style_epoch, initial_shape_epoch,
                "reinstalling the same binding key should not invalidate shaped rows"
            );

            let cache = input
                .highlight
                .provider_cache
                .as_ref()
                .expect("stable binding key should preserve the cached provider range");
            let cache_entry = cache
                .entries
                .last()
                .expect("stable binding key should keep the cached provider slice");
            assert!(
                Arc::ptr_eq(&cache_entry.highlights, &initial_cached_highlights),
                "stable binding key should preserve the existing cached highlight vector"
            );

            let resolved = input.resolve_provider_highlights(1, 4);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "stable binding key should keep using the original provider/cache"
            );
            assert_eq!(
                dp.second_calls.load(Ordering::SeqCst),
                0,
                "stable binding key should not bind a replacement provider"
            );
            assert!(Arc::ptr_eq(
                &resolved.highlights,
                &initial_cached_highlights
            ));
            assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
        });
    });
}

#[gpui::test]
fn replace_utf8_range_clears_shaped_row_caches(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                soft_wrap: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);

            input.layout.plain_line_cache.insert(
                ShapedRowCacheKey {
                    line_ix: 0,
                    wrap_width_key: i32::MIN,
                },
                ShapedLine::default(),
            );
            input.layout.wrapped_line_cache.insert(
                ShapedRowCacheKey {
                    line_ix: 0,
                    wrap_width_key: wrap_width_cache_key(px(320.0)),
                },
                (),
            );

            assert_eq!(input.layout.plain_line_cache.len(), 1);
            assert_eq!(input.layout.wrapped_line_cache.len(), 1);

            input.replace_utf8_range(0..5, "gamma", cx);

            assert!(
                input.layout.plain_line_cache.is_empty(),
                "text edits must invalidate cached plain shaped rows"
            );
            assert!(
                input.layout.wrapped_line_cache.is_empty(),
                "text edits must invalidate cached wrapped shaped rows"
            );
        });
    });
}

#[gpui::test]
fn changed_highlight_provider_binding_key_rebinds_and_clears_cached_range(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), cx);
            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            let previous_shape_epoch = input.layout.shape_style_epoch;

            input.set_highlight_provider_with_key(42, dp.second.clone(), cx);

            assert!(
                input.highlight.provider_cache.is_none(),
                "changing the binding key should drop the cached provider range"
            );
            assert!(
                input.highlight.epoch > previous_highlight_epoch,
                "changing the binding key should invalidate provider highlight epochs"
            );
            assert!(
                input.layout.shape_style_epoch > previous_shape_epoch,
                "changing the binding key should invalidate shaped text caches"
            );

            let resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "rebinding should stop using the previous provider"
            );
            assert_eq!(
                dp.second_calls.load(Ordering::SeqCst),
                1,
                "rebinding should resolve highlights from the new provider"
            );
            assert_eq!(resolved.highlights[0].1.color, Some(dp.second_color));

            let cache = input
                .highlight
                .provider_cache
                .as_ref()
                .expect("resolving after a rebind should repopulate the provider cache");
            assert_eq!(cache.highlight_epoch, input.highlight.epoch);
            assert_eq!(cache.entries.len(), 1);
            let cache_entry = cache
                .entries
                .last()
                .expect("rebind resolve should cache the requested provider slice");
            assert_eq!(cache_entry.byte_start, 0);
            assert_eq!(cache_entry.byte_end, 5);
        });
    });
}

#[gpui::test]
fn replace_utf8_range_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            let inserted = input.replace_utf8_range(0..5, "gamma", cx);
            assert_eq!(inserted, 0..5);
            assert!(
                input.highlight.provider_cache.is_none(),
                "text edits should clear cached provider ranges"
            );
            assert!(
                input.highlight.epoch > previous_highlight_epoch,
                "text edits should invalidate provider highlight epochs"
            );

            let resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                2,
                "after an edit, the stable provider should be asked for a fresh range"
            );
            assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
        });
    });
}

#[gpui::test]
fn set_text_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            input.set_text("gamma\nbeta", cx);

            assert!(
                input.highlight.provider_cache.is_none(),
                "set_text should clear cached provider ranges"
            );
            assert!(
                input.highlight.epoch > previous_highlight_epoch,
                "set_text should invalidate provider highlight epochs"
            );

            let resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                2,
                "after set_text, the stable provider should be asked for a fresh range"
            );
            assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
        });
    });
}

#[gpui::test]
fn undo_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);

            let inserted = input.replace_utf8_range(0..5, "gamma", cx);
            assert_eq!(inserted, 0..5);
            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 2);
            let previous_highlight_epoch = input.highlight.epoch;

            input.undo(&Undo, window, cx);

            assert_eq!(input.text(), "alpha\nbeta");
            assert!(
                input.highlight.provider_cache.is_none(),
                "undo should clear cached provider ranges restored from the old snapshot"
            );
            assert!(
                input.highlight.epoch > previous_highlight_epoch,
                "undo should invalidate provider highlight epochs"
            );

            let resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                3,
                "after undo, the provider should be asked for a fresh range"
            );
            assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
        });
    });
}

#[gpui::test]
fn redo_restores_text_after_undo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);
            assert_eq!(input.text(), "beta");

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "beta");
            assert!(input.selection.redo_stack.is_empty());
        });
    });
}

#[gpui::test]
fn redo_is_cleared_by_a_new_edit_after_undo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);

            let inserted = input.replace_utf8_range(0..5, "gamma", cx);
            assert_eq!(inserted, 0..5);
            assert_eq!(input.text(), "gamma");
            assert!(input.selection.redo_stack.is_empty());

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "gamma");
            assert!(input.selection.redo_stack.is_empty());
        });
    });
}

#[gpui::test]
fn redo_is_noop_when_input_is_read_only(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);

            input.set_read_only(true, cx);
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);
        });
    });
}

#[gpui::test]
fn replace_text_in_range_invalidates_cached_provider_highlights(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            input.replace_text_in_range(Some(0..5), "gamma", window, cx);

            assert_eq!(input.text(), "gamma\nbeta");
            assert!(
                input.highlight.provider_cache.is_none(),
                "IME replace_text_in_range should clear cached provider ranges"
            );
            assert!(
                input.highlight.epoch > previous_highlight_epoch,
                "IME replace_text_in_range should invalidate provider highlight epochs"
            );

            let resolved = input.resolve_provider_highlights(0, 5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                2,
                "after replace_text_in_range, the stable provider should be asked for a fresh range"
            );
            assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
        });
    });
}

#[gpui::test]
fn replace_and_mark_text_in_range_invalidates_cached_provider_highlights(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    let dp = make_dual_providers();

    cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha\nbeta", cx);
                input.set_highlight_provider_with_key(41, dp.first.clone(), cx);

                let _ = input.resolve_provider_highlights(0, 5);
                assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
                let previous_highlight_epoch = input.highlight.epoch;
                assert!(
                    input.highlight.provider_cache.is_some(),
                    "initial resolve should populate the provider cache"
                );

                input.replace_and_mark_text_in_range(Some(0..5), "gamma", None, window, cx);

                assert_eq!(input.text(), "gamma\nbeta");
                assert_eq!(input.selection.marked_range, Some(0..5));
                assert!(
                    input.highlight.provider_cache.is_none(),
                    "IME replace_and_mark_text_in_range should clear cached provider ranges"
                );
                assert!(
                    input.highlight.epoch > previous_highlight_epoch,
                    "IME replace_and_mark_text_in_range should invalidate provider highlight epochs"
                );

                let resolved = input.resolve_provider_highlights(0, 5);
                assert_eq!(
                    dp.first_calls.load(Ordering::SeqCst),
                    2,
                    "after replace_and_mark_text_in_range, the stable provider should be asked for a fresh range"
                );
                assert_eq!(resolved.highlights[0].1.color, Some(dp.first_color));
            });
        });
}

#[test]
fn highlight_provider_with_pending_uses_custom_callbacks() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let pending = Arc::new(AtomicBool::new(true));
    let pending_for_resolve = Arc::clone(&pending);
    let pending_for_check = Arc::clone(&pending);
    let drain_calls = Arc::new(AtomicUsize::new(0));
    let drain_calls_for_provider = Arc::clone(&drain_calls);
    let provider = HighlightProvider::with_pending(
        move |range: Range<usize>| HighlightProviderResult {
            highlights: vec![(
                range,
                gpui::HighlightStyle {
                    color: Some(gpui::hsla(0.66, 1.0, 0.5, 1.0)),
                    ..gpui::HighlightStyle::default()
                },
            )],
            pending: pending_for_resolve.load(Ordering::SeqCst),
        },
        move || {
            drain_calls_for_provider.fetch_add(1, Ordering::SeqCst);
            pending.store(false, Ordering::SeqCst);
            1
        },
        move || pending_for_check.load(Ordering::SeqCst),
    );

    let first = provider.resolve(4..12);
    assert!(first.pending);
    assert_eq!(first.highlights[0].0, 4..12);
    assert!(provider.has_pending());
    assert_eq!(provider.drain_pending(), 1);
    assert_eq!(drain_calls.load(Ordering::SeqCst), 1);
    assert!(!provider.has_pending());

    let second = provider.resolve(4..12);
    assert!(!second.pending);
}
