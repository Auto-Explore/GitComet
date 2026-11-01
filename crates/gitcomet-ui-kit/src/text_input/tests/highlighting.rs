use super::*;

fn styled(range: Range<usize>) -> (Range<usize>, gpui::HighlightStyle) {
    (
        range,
        gpui::HighlightStyle {
            color: Some(gpui::hsla(0.0, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        },
    )
}

fn mapped_ranges(
    interpolation: &HighlightInterpolation,
    source: &[(Range<usize>, gpui::HighlightStyle)],
    clamp_len: usize,
) -> Vec<Range<usize>> {
    interpolation
        .map_highlights(source, clamp_len)
        .into_iter()
        .map(|(range, _)| range)
        .collect()
}

#[test]
fn highlight_interpolation_starts_exact() {
    let interpolation = HighlightInterpolation::default();
    assert!(interpolation.is_exact());
    assert_eq!(interpolation.to_source_offset(7), 7);
    assert_eq!(interpolation.debug_patch(), None);
}

#[test]
fn highlight_interpolation_shifts_highlights_after_an_insert() {
    let mut interpolation = HighlightInterpolation::default();
    // Two characters typed at offset 4.
    interpolation.record_edit(&(4..4), &(4..6));

    assert!(!interpolation.is_exact());
    // Before the caret: untouched. After it: shifted by the inserted length.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..3), styled(8..12)], 32),
        vec![0..3, 10..14]
    );
}

#[test]
fn highlight_interpolation_shifts_highlights_after_a_delete() {
    let mut interpolation = HighlightInterpolation::default();
    // Three characters deleted at offset 4.
    interpolation.record_edit(&(4..7), &(4..4));

    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..3), styled(10..14)], 32),
        vec![0..3, 7..11]
    );
    // A highlight entirely inside the deleted span has nothing left to describe.
    assert!(mapped_ranges(&interpolation, &[styled(5..6)], 32).is_empty());
}

#[test]
fn highlight_interpolation_shifts_highlights_after_a_replace() {
    let mut interpolation = HighlightInterpolation::default();
    // Two characters replaced by five at offset 4.
    interpolation.record_edit(&(4..6), &(4..9));

    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..4), styled(6..10)], 32),
        vec![0..4, 9..13]
    );
}

#[test]
fn highlight_interpolation_splits_a_highlight_straddling_the_edit() {
    let mut interpolation = HighlightInterpolation::default();
    // Source bytes 4..6 became live bytes 4..8.
    interpolation.record_edit(&(4..6), &(4..8));

    // The surviving halves keep their color; the freshly typed bytes 4..8 fall
    // back to the base color rather than taking the whole range down with them.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(0..10)], 32),
        vec![0..4, 8..12]
    );
}

#[test]
fn highlight_interpolation_sorts_split_pieces_across_overlapping_sources() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..6), &(4..8));

    // The first range's right piece (8..12) lands after the second range's left
    // piece (2..4), so source order is not output order — and `HighlightCursor`
    // binary-searches these.
    let mapped = mapped_ranges(&interpolation, &[styled(0..10), styled(2..3)], 32);
    assert_eq!(mapped, vec![0..4, 2..3, 8..12]);
    assert!(mapped.windows(2).all(|pair| pair[0].start <= pair[1].start));
}

#[test]
fn highlight_interpolation_clamps_to_the_live_text_length() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(0..6), &(0..2));

    // The source described a longer buffer; nothing may point past the end of
    // the one being rendered.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(6..20)], 8),
        vec![2..8]
    );
}

#[test]
fn highlight_interpolation_coalesces_a_run_of_single_character_inserts() {
    let mut interpolation = HighlightInterpolation::default();
    for offset in 0..10 {
        // A caret at 4 that advances one byte per keystroke.
        let caret = 4 + offset;
        interpolation.record_edit(&(caret..caret), &(caret..caret + 1));
    }

    assert_eq!(interpolation.generation(), 10);
    let patch = interpolation
        .debug_patch()
        .expect("typing should leave one patch");
    assert_eq!(patch.start, 4);
    assert_eq!(
        patch.old_len, 0,
        "an insert run replaces nothing, so the source span stays empty"
    );
    assert_eq!(patch.new_len, 10);
    // Highlights past the caret are shifted by the whole run at once.
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(4..8)], 64),
        vec![14..18]
    );
}

#[test]
fn highlight_interpolation_collapses_when_typing_is_backspaced_away() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..4), &(4..5));
    interpolation.record_edit(&(4..5), &(4..4));

    assert!(
        interpolation.is_exact(),
        "an insert and the backspace undoing it leave the source describing the buffer verbatim"
    );
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(2..8)], 32),
        vec![2..8]
    );
}

#[test]
fn highlight_interpolation_widens_to_the_union_across_disjoint_edits() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..4), &(4..5));
    interpolation.record_edit(&(20..20), &(20..21));

    let patch = interpolation
        .debug_patch()
        .expect("two edits should leave one widened patch");
    assert_eq!(patch.start, 4);
    // The documented degradation: text between two disjoint edits is inside the
    // union, so it renders in the base color until the recompute lands, rather
    // than keeping highlights that would now sit on the wrong bytes.
    assert_eq!(patch.old_len, 15);
    assert_eq!(patch.new_len, 17);
    assert_eq!(
        mapped_ranges(
            &interpolation,
            &[styled(0..2), styled(8..12), styled(24..28)],
            64
        ),
        vec![0..2, 26..30]
    );
}

#[test]
fn highlight_interpolation_reset_restores_the_identity_map() {
    let mut interpolation = HighlightInterpolation::default();
    interpolation.record_edit(&(4..4), &(4..6));
    let generation = interpolation.generation();

    interpolation.reset();

    assert!(interpolation.is_exact());
    assert_eq!(
        mapped_ranges(&interpolation, &[styled(8..12)], 32),
        vec![8..12]
    );
    assert!(
        interpolation.generation() > generation,
        "a reset changes the coordinates callers cached against"
    );
}

#[test]
fn highlight_interpolation_source_offsets_are_monotone_and_locally_exact() {
    // Every shape of edit — insert, delete, replace, both directions — against
    // every offset in a small buffer.
    for old_len in 0..5usize {
        for new_len in 0..5usize {
            let mut interpolation = HighlightInterpolation::default();
            interpolation.record_edit(&(4..4 + old_len), &(4..4 + new_len));

            let mut previous = 0usize;
            for offset in 0..24usize {
                let source = interpolation.to_source_offset(offset);
                assert!(
                    source >= previous,
                    "to_source_offset must be monotone (old_len={old_len}, new_len={new_len}, offset={offset})"
                );
                previous = source;

                if offset <= 4 {
                    assert_eq!(
                        source, offset,
                        "offsets before the patch are unchanged (old_len={old_len}, new_len={new_len})"
                    );
                }
                // Strict: with `new_len == 0` the offset at the patch start is
                // both "before" and "past" the edit, and the map answers with
                // the lower of the two. Only monotonicity binds there.
                if offset > 4 + new_len {
                    assert_eq!(
                        source,
                        offset + old_len - new_len,
                        "offsets past the patch shift by the length delta (old_len={old_len}, new_len={new_len})"
                    );
                }
            }
        }
    }
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
        &LineTextSource::Whole {
            text,
            starts: line_starts.as_slice(),
        },
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
        &LineTextSource::Whole {
            text: text.as_str(),
            starts: line_starts.as_slice(),
        },
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
        &LineTextSource::Whole {
            text,
            starts: line_starts.as_slice(),
        },
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
        &LineTextSource::Whole {
            text,
            starts: line_starts.as_slice(),
        },
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

const TOKEN_COLOR: gpui::Hsla = gpui::hsla(0.33, 1.0, 0.5, 1.0);

/// A provider shaped like a real syntax one: it answers in the coordinates of
/// the text it was built over, not of whatever the buffer holds now.
fn token_highlight_provider(source: &str, token: &'static str) -> HighlightProvider {
    let source = source.to_string();
    HighlightProvider::from_fn(move |range: Range<usize>| {
        source
            .match_indices(token)
            .map(|(start, matched)| start..start + matched.len())
            .filter(|found| found.start < range.end && found.end > range.start)
            .map(|found| {
                (
                    found,
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )
            })
            .collect()
    })
}

/// What the highlighted byte ranges actually cover in the buffer's live text —
/// the only assertion that catches a smear, since a wrong offset is still a
/// perfectly plausible-looking number.
fn highlighted_slices(input: &mut TextInput, window: Range<usize>) -> Vec<String> {
    let text = input.text().to_string();
    input
        .debug_effective_highlights_for_range(window)
        .into_iter()
        .map(|(range, _)| text[range].to_string())
        .collect()
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
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

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

            input.set_highlight_provider_with_key(41, dp.second.clone(), input.text().len(), cx);

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
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);
            let _ = input.resolve_provider_highlights(0, 5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            let previous_shape_epoch = input.layout.shape_style_epoch;

            input.set_highlight_provider_with_key(42, dp.second.clone(), input.text().len(), cx);

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
fn typing_keeps_provider_highlights_aligned(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                7,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );

            assert_eq!(
                highlighted_slices(input, 0..text.len()),
                vec!["incoming", "incoming"]
            );

            // Type inside the first line, ahead of both tokens, without letting
            // the owner's debounced provider rebuild run.
            input.replace_utf8_range(4..4, "X", cx);
            assert_eq!(
                input.text(),
                "let Xvalue = incoming;\n// keep incoming aligned\n"
            );

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"],
                "colors must stay pinned to their tokens between the edit and the recompute"
            );
        });
    });
}

#[gpui::test]
fn typing_a_newline_keeps_provider_highlights_aligned(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                7,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            let _ = highlighted_slices(input, 0..text.len());

            // The case that defeats reading the live text through the old
            // closure: the provider's line index no longer matches the buffer's,
            // so every line below would take its neighbour's tokens.
            input.replace_utf8_range(4..4, "\n", cx);
            assert_eq!(
                input.text(),
                "let \nvalue = incoming;\n// keep incoming aligned\n"
            );

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"],
                "an inserted newline must not smear the lines below it"
            );
        });
    });
}

#[gpui::test]
fn typing_multibyte_text_keeps_highlights_on_character_boundaries(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let café = incoming;\n// naïve incoming\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                7,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            let _ = highlighted_slices(input, 0..text.len());

            // Two multi-byte characters typed just before "café". Slicing the
            // result at all proves the mapped bounds are character boundaries;
            // a mid-character bound would panic here (and trips a debug assert
            // inside the mapping itself).
            input.replace_utf8_range(4..4, "üé", cx);
            assert_eq!(input.text(), "let üécafé = incoming;\n// naïve incoming\n");

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"]
            );
        });
    });
}

#[gpui::test]
fn typing_interpolates_statically_published_highlights(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("let value = incoming;", cx);
            let token = input.text().find("incoming").expect("token in fixture");
            input.set_highlights(
                vec![(
                    token..token + "incoming".len(),
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                cx,
            );

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming"]
            );

            // Highlights published through `set_highlights` were never
            // invalidated on edit at all, so they smeared silently.
            input.replace_utf8_range(4..4, "X", cx);

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming"],
                "a static highlight vector rides along with edits like a provider's"
            );
        });
    });
}

#[gpui::test]
fn rebinding_a_provider_resets_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);
            input.replace_utf8_range(0..1, "AA", cx);
            assert!(!input.highlight.interpolation.is_exact());

            // A different key means a different closure, built over the text as
            // it stands now.
            input.set_highlight_provider_with_key(42, dp.second.clone(), input.text().len(), cx);
            assert!(
                input.highlight.interpolation.is_exact(),
                "a fresh provider describes the buffer verbatim"
            );

            input.replace_utf8_range(0..1, "BB", cx);
            assert!(!input.highlight.interpolation.is_exact());

            // The same key means the same closure over the same text, so its
            // anchor has to survive.
            input.set_highlight_provider_with_key(42, dp.second.clone(), input.text().len(), cx);
            assert!(
                !input.highlight.interpolation.is_exact(),
                "reapplying an unchanged binding must not move the anchor"
            );
        });
    });
}

#[gpui::test]
fn set_highlights_resets_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlights(
                vec![(
                    0..5,
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                cx,
            );
            input.replace_utf8_range(0..1, "AA", cx);
            assert!(!input.highlight.interpolation.is_exact());

            input.set_highlights(
                vec![(
                    0..6,
                    gpui::HighlightStyle {
                        color: Some(TOKEN_COLOR),
                        ..gpui::HighlightStyle::default()
                    },
                )],
                cx,
            );

            assert!(input.highlight.interpolation.is_exact());
            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["AAlpha"]
            );
        });
    });
}

/// The commit-details SHA fields republish a byte-identical highlight vector
/// after rewriting the text — one 40-char id for another, always `[(0..40, link
/// style)]`. The identical-vector fast path must still drop the edit patch
/// `set_text` left behind, or the whole rewritten span loses its link styling.
#[gpui::test]
fn republishing_identical_highlights_resets_the_interpolation(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let link = vec![(
                0..8,
                gpui::HighlightStyle {
                    color: Some(TOKEN_COLOR),
                    ..gpui::HighlightStyle::default()
                },
            )];

            input.set_text("aaa11aaa", cx);
            input.set_highlights(link.clone(), cx);
            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["aaa11aaa"]
            );

            // Same length, differing only in the middle: `set_text` records an
            // edit patch over that span.
            input.set_text("aaa22aaa", cx);
            input.set_highlights(link, cx);

            assert!(input.highlight.interpolation.is_exact());
            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["aaa22aaa"]
            );
        });
    });
}

#[gpui::test]
fn rebinding_under_a_new_key_resets_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
    // This is the contract the live syntax engine depends on. It re-syncs its
    // tree on the keystroke and rebinds under the document's new version, so
    // its ranges are already correct for the edited text. If interpolation
    // survived the rebind, those correct ranges would be mapped through the
    // edit a second time and land in the wrong place.
    let (input, cx) = multiline_input(cx);
    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(1, dp.first.clone(), input.text().len(), cx);
            let _ = input.effective_highlights_for_window(0..5);

            input.replace_utf8_range(0..1, "AA", cx);
            assert!(
                !input.highlight.interpolation.is_exact(),
                "an edit against a still-installed provider must be interpolated"
            );

            // What the live engine does next: rebind under a fresh key.
            input.set_highlight_provider_with_key(2, dp.second.clone(), input.text().len(), cx);

            assert!(
                input.highlight.interpolation.is_exact(),
                "a new binding key declares the provider current for the edited text"
            );
            assert_eq!(
                highlighted_slices(input, 0..6),
                vec!["AAlpha"],
                "ranges from the rebound provider must pass through unmapped"
            );
        });
    });
}

#[gpui::test]
fn a_never_pending_provider_does_not_accumulate_superseded_sources(cx: &mut gpui::TestAppContext) {
    // The live engine rebinds on every keystroke. Each rebind sets the outgoing
    // source aside to cover for a replacement that cannot answer yet; a
    // provider that always answers must clear that reserve instead of piling up
    // an Arc per keystroke.
    let (input, cx) = multiline_input(cx);
    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            for key in 1..=8u64 {
                let provider = if key % 2 == 0 {
                    dp.second.clone()
                } else {
                    dp.first.clone()
                };
                input.set_highlight_provider_with_key(key, provider, input.text().len(), cx);
                let resolved = input.effective_highlights_for_window(0..5);
                assert!(
                    !resolved.pending,
                    "a live-engine provider is always exact and never reports pending"
                );
                assert!(
                    input.highlight.superseded.is_none(),
                    "a settled source must release the one it replaced (key {key})"
                );
            }
        });
    });
}

#[gpui::test]
fn landing_background_syntax_chunks_preserves_the_highlight_interpolation(
    cx: &mut gpui::TestAppContext,
) {
    let (input, cx) = multiline_input(cx);

    let dp = make_dual_providers();

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);
            let _ = input.effective_highlights_for_window(0..5);
            input.replace_utf8_range(0..1, "AA", cx);

            let generation = input.highlight.interpolation.generation();
            let previous_epoch = input.highlight.epoch;

            // Chunks landing improve the provider's tokens; the text it was
            // built over is unchanged, so its anchor must not move.
            input.note_provider_highlights_changed();

            assert!(!input.highlight.interpolation.is_exact());
            assert_eq!(input.highlight.interpolation.generation(), generation);
            assert!(
                input.highlight.epoch > previous_epoch,
                "better tokens are a reason to refetch"
            );
            assert!(input.highlight.interpolated_cache.is_none());

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, _)| range.clone())
                    .collect::<Vec<_>>(),
                vec![2..5],
                "the refetched tokens arrive shifted by the same +1 the edit caused"
            );
        });
    });
}

/// A provider whose tokens are built in the background: it answers `pending`
/// with nothing until `ready` is flipped, the way a freshly prepared syntax
/// document does.
fn deferred_token_provider(
    source: &str,
    token: &'static str,
    ready: Arc<std::sync::atomic::AtomicBool>,
) -> HighlightProvider {
    use std::sync::atomic::Ordering;

    let source = source.to_string();
    let resolve_ready = Arc::clone(&ready);
    let has_pending_ready = Arc::clone(&ready);
    HighlightProvider::with_pending(
        move |range: Range<usize>| {
            if !resolve_ready.load(Ordering::SeqCst) {
                return HighlightProviderResult {
                    highlights: Vec::new(),
                    pending: true,
                };
            }
            HighlightProviderResult {
                highlights: source
                    .match_indices(token)
                    .map(|(start, matched)| start..start + matched.len())
                    .filter(|found| found.start < range.end && found.end > range.start)
                    .map(|found| {
                        (
                            found,
                            gpui::HighlightStyle {
                                color: Some(TOKEN_COLOR),
                                ..gpui::HighlightStyle::default()
                            },
                        )
                    })
                    .collect(),
                pending: false,
            }
        },
        || 0,
        move || !has_pending_ready.load(Ordering::SeqCst),
    )
}

#[gpui::test]
fn rebinding_to_a_provider_that_cannot_answer_yet_keeps_the_previous_colors(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                1,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            assert_eq!(
                highlighted_slices(input, 0..text.len()),
                vec!["incoming", "incoming"]
            );

            // The user types; the owner's debounce then recomputes and rebinds
            // to a provider over a freshly prepared document, whose token
            // chunks are still being built in the background.
            input.replace_utf8_range(4..4, "X", cx);
            let edited = input.text().to_string();
            let ready = Arc::new(AtomicBool::new(false));
            input.set_highlight_provider_with_key(
                2,
                deferred_token_provider(&edited, "incoming", Arc::clone(&ready)),
                edited.len(),
                cx,
            );

            // Without the handoff this window renders in the base color — the
            // whole output flashing white for a frame or two.
            assert_eq!(
                highlighted_slices(input, 0..edited.len()),
                vec!["incoming", "incoming"],
                "the outgoing source must cover for its replacement until it can answer"
            );

            // Chunks land; the new provider takes over and the fallback is let go.
            ready.store(true, Ordering::SeqCst);
            input.note_provider_highlights_changed();

            assert_eq!(
                highlighted_slices(input, 0..edited.len()),
                vec!["incoming", "incoming"]
            );
            assert!(
                input.highlight.superseded.is_none(),
                "a settled source needs no stand-in"
            );
        });
    });
}

#[gpui::test]
fn a_provider_that_never_answered_does_not_become_the_fallback(cx: &mut gpui::TestAppContext) {
    use std::sync::atomic::AtomicBool;

    let (input, cx) = multiline_input(cx);

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let text = "let value = incoming;\n// keep incoming aligned\n";
            input.set_text(text, cx);
            input.set_highlight_provider_with_key(
                1,
                token_highlight_provider(input.text(), "incoming"),
                input.text().len(),
                cx,
            );
            let _ = highlighted_slices(input, 0..text.len());

            // Two rebinds in a row, neither replacement ready — the second must
            // not push the good source out in favour of the first, which never
            // managed to say anything.
            for key in [2u64, 3] {
                input.replace_utf8_range(4..4, "X", cx);
                let edited = input.text().to_string();
                input.set_highlight_provider_with_key(
                    key,
                    deferred_token_provider(&edited, "incoming", Arc::new(AtomicBool::new(false))),
                    edited.len(),
                    cx,
                );
            }

            assert_eq!(
                highlighted_slices(input, 0..input.text().len()),
                vec!["incoming", "incoming"],
                "the last source that actually answered stays the fallback"
            );
        });
    });
}

#[gpui::test]
fn replace_utf8_range_keeps_cached_provider_highlights_and_interpolates(
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
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            // One byte becomes two, so everything the provider described from
            // byte 1 onward now sits one byte further along.
            let inserted = input.replace_utf8_range(0..1, "AA", cx);
            assert_eq!(inserted, 0..2);
            assert_eq!(input.text(), "AAlpha\nbeta");
            assert!(
                input.highlight.provider_cache.is_some(),
                "an edit is interpolated over, so the cached provider range survives it"
            );
            assert_eq!(
                input.highlight.epoch, previous_highlight_epoch,
                "an edit does not change what the provider describes"
            );

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "the same window maps back into the range already fetched"
            );
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(2..6, Some(dp.first_color))],
                "the provider's 0..5 span keeps its tail, shifted past the insert"
            );
        });
    });
}

#[gpui::test]
fn set_text_records_an_edit_delta_for_highlights(cx: &mut gpui::TestAppContext) {
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
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            // A rewrite that appends one byte at offset 5, which `set_text`
            // diffs out of the two texts rather than invalidating blindly.
            input.set_text("alphaX\nbeta", cx);

            assert!(
                !input.highlight.interpolation.is_exact(),
                "set_text must record what it changed, not silently move the text out from under the highlights"
            );
            assert!(
                input.highlight.provider_cache.is_some(),
                "the cached provider range still describes the text it was fetched for"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(0..5, Some(dp.first_color))],
                "highlights entirely ahead of the edit are untouched"
            );
        });
    });
}

#[gpui::test]
fn undo_composes_into_the_highlight_interpolation(cx: &mut gpui::TestAppContext) {
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
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;

            let inserted = input.replace_utf8_range(0..1, "AA", cx);
            assert_eq!(inserted, 0..2);
            assert!(!input.highlight.interpolation.is_exact());

            input.undo(&Undo, window, cx);

            assert_eq!(input.text(), "alpha\nbeta");
            // The undo composes into the patch like any other edit. It restores
            // the provider's own text, but the interpolation tracks which bytes
            // were touched, not what they now say, so byte 0 stays marked.
            assert!(!input.highlight.interpolation.is_exact());
            assert!(
                input.highlight.provider_cache.is_some(),
                "the restored text is the text the cached range was fetched for"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(
                dp.first_calls.load(Ordering::SeqCst),
                1,
                "returning to the provider's own text should not re-query it"
            );
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(1..5, Some(dp.first_color))],
                "offsets are back where they started; only the touched byte waits \
                 for the debounced recompute"
            );
        });
    });
}

#[gpui::test]
fn replace_text_in_range_keeps_cached_provider_highlights_and_interpolates(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = multiline_input(cx);

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            input.replace_text_in_range(Some(0..1), "AA", window, cx);

            assert_eq!(input.text(), "AAlpha\nbeta");
            assert!(
                input.highlight.provider_cache.is_some(),
                "IME replace_text_in_range is interpolated over like any other edit"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(2..6, Some(dp.first_color))],
            );
        });
    });
}

#[gpui::test]
fn replace_and_mark_text_in_range_keeps_cached_provider_highlights_and_interpolates(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::atomic::Ordering;

    let (input, cx) = multiline_input(cx);

    let dp = make_dual_providers();

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            input.set_highlight_provider_with_key(41, dp.first.clone(), input.text().len(), cx);

            let _ = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            let previous_highlight_epoch = input.highlight.epoch;
            assert!(
                input.highlight.provider_cache.is_some(),
                "initial resolve should populate the provider cache"
            );

            input.replace_and_mark_text_in_range(Some(0..1), "AA", None, window, cx);

            assert_eq!(input.text(), "AAlpha\nbeta");
            assert_eq!(input.selection.marked_range, Some(0..2));
            assert!(
                input.highlight.provider_cache.is_some(),
                "an IME composition is interpolated over like any other edit"
            );
            assert_eq!(input.highlight.epoch, previous_highlight_epoch);

            let resolved = input.effective_highlights_for_window(0..5);
            assert_eq!(dp.first_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                resolved
                    .highlights
                    .iter()
                    .map(|(range, style)| (range.clone(), style.color))
                    .collect::<Vec<_>>(),
                vec![(2..6, Some(dp.first_color))],
            );
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
