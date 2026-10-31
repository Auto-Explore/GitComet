use super::*;
use crate::view::panes::diff::diff_search::DiffSearchMatcher;

#[test]
fn crlf_paint_payload_clips_highlights_and_keeps_revealed_source() {
    let raw = "é  beta\r";
    let style = HighlightStyle {
        font_weight: Some(gpui::FontWeight::BOLD),
        ..Default::default()
    };
    let styled =
        build_cached_diff_styled_text_from_relative_highlights(4, raw, &[(0..raw.len(), style)]);
    for region in [
        DiffTextRegion::Inline,
        DiffTextRegion::SplitLeft,
        DiffTextRegion::SplitRight,
    ] {
        for reveal in [false, true, false] {
            let payload = diff_text_paint_payload(
                4,
                Some(&styled),
                None,
                Some(super::super::DiffRowText::Str(raw)),
                reveal,
                region,
                None,
            );
            if reveal {
                assert_eq!(payload.text.as_ref(), "é··beta␍↵");
                assert_eq!(payload.offset_map.as_ref().unwrap().source_len(), raw.len());
            } else {
                assert_eq!(payload.text.as_ref(), "é  beta");
                assert_eq!(payload.highlights.as_ref(), &[(0..raw.len() - 1, style)]);
                assert_ne!(payload.text_hash, styled.text_hash);
                assert_ne!(payload.highlights_hash, styled.highlights_hash);
                assert!(
                    payload.offset_map.is_none(),
                    "prefix offsets need no dense map"
                );
            }
            assert!(
                payload
                    .highlights
                    .iter()
                    .all(|(range, _)| range.end <= payload.text.len())
            );
            assert_eq!(styled.text.as_ref(), raw);
        }
    }
}

#[test]
fn crlf_streamed_display_width_reads_only_the_tail() {
    use gitcomet_core::file_diff::{
        FileDiffLineText, batch_file_slice_handles, take_file_slice_bytes_read_for_tests,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = Arc::new(dir.path().join("long.txt"));
    let text = format!("{}\r", "x".repeat(STREAMED_DIFF_TEXT_MIN_BYTES + 1));
    std::fs::write(path.as_ref(), &text).unwrap();
    let line = FileDiffLineText::file_slice(path.clone(), 0..text.len(), true, false);
    let _batch = batch_file_slice_handles();
    take_file_slice_bytes_read_for_tests();
    assert_eq!(streamed_line_display_len(&line), text.len() - 1);
    assert_eq!(take_file_slice_bytes_read_for_tests(), 1);
    for _ in 0..10 {
        assert_eq!(streamed_line_display_len(&line.clone()), text.len() - 1);
    }
    assert_eq!(
        take_file_slice_bytes_read_for_tests(),
        0,
        "repaints reuse the cached tail"
    );
    let loaded = FileDiffLineText::file_slice(path.clone(), 0..text.len(), true, false);
    assert_eq!(loaded.as_str(), text);
    take_file_slice_bytes_read_for_tests();
    assert_eq!(streamed_line_display_len(&loaded), text.len() - 1);
    assert_eq!(
        take_file_slice_bytes_read_for_tests(),
        0,
        "loaded text never rereads its tail"
    );
    for _ in 0..10 {
        let indexed = FileDiffLineText::file_slice(path.clone(), 0..text.len(), true, false)
            .with_known_trailing_cr(true);
        assert_eq!(streamed_line_display_len(&indexed), text.len() - 1);
    }
    assert_eq!(
        take_file_slice_bytes_read_for_tests(),
        0,
        "index metadata needs no I/O even for fresh row handles"
    );
    assert_eq!(line.len(), text.len());
    let mut spec = streamed_query_spec("", "", DiffSearchOptions::default());
    spec.raw_text = line;
    let (slice, _, range) = build_streamed_diff_slice_styled_text(
        4,
        AppTheme::gitcomet_dark(),
        &spec,
        &(text.len() - 16..text.len()),
    );
    let display = display_styled_slice(&slice, 0..slice.text.len(), true);
    assert_eq!(range.end, text.len());
    assert_eq!(display.text.as_ref(), "x".repeat(15));
    assert!(take_file_slice_bytes_read_for_tests() <= 32);
}

#[test]
fn crlf_streamed_terminal_slice_reconciles_stale_index_and_hit_testing() {
    use crate::view::text_geometry::TextGeometry;
    use gitcomet_core::file_diff::{
        FileDiffLineText, batch_file_slice_handles, take_file_slice_bytes_read_for_tests,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = Arc::new(dir.path().join("changed-tail.txt"));
    let prefix = "x".repeat(STREAMED_DIFF_TEXT_MIN_BYTES + 1);
    let source_len = prefix.len() + 1;
    for (indexed_cr, actual_cr) in [(false, true), (true, false)] {
        std::fs::write(
            path.as_ref(),
            format!("{prefix}{}", if indexed_cr { '\r' } else { 'x' }),
        )
        .unwrap();
        let line = FileDiffLineText::file_slice(path.clone(), 0..source_len, true, false)
            .with_known_trailing_cr(indexed_cr);
        // Keep the indexed byte range but change its terminator before paint.
        std::fs::write(
            path.as_ref(),
            format!("{prefix}{}", if actual_cr { '\r' } else { 'x' }),
        )
        .unwrap();
        let _batch = batch_file_slice_handles();
        take_file_slice_bytes_read_for_tests();
        let indexed_display_len = streamed_line_display_len(&line);
        assert_eq!(take_file_slice_bytes_read_for_tests(), 0);
        let mut spec = streamed_query_spec("", "", DiffSearchOptions::default());
        spec.raw_text = line;
        let start = source_len - 16;
        let requested = streamed_diff_text_visible_slice_range(
            Bounds::new(
                gpui::point(px(-(start as f32)), px(0.0)),
                gpui::size(px(source_len as f32), px(20.0)),
            ),
            Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                gpui::size(px(16.0), px(20.0)),
            ),
            source_len,
            px(1.0),
            0,
        );
        assert_eq!(requested, start..source_len);
        let (slice, _, range) =
            build_streamed_diff_slice_styled_text(4, AppTheme::gitcomet_dark(), &spec, &requested);
        let prepared = prepare_streamed_diff_slice_for_paint(
            &slice,
            &range,
            source_len,
            indexed_display_len,
            false,
        );
        assert_eq!(
            take_file_slice_bytes_read_for_tests(),
            16,
            "one visible-slice read, no tail reread"
        );
        let expected_end = if actual_cr { prefix.len() } else { source_len };
        let expected_slice_len = if actual_cr { 15 } else { 16 };
        assert_eq!(
            prepared.styled.text.as_ref(),
            "x".repeat(expected_slice_len)
        );
        assert_eq!(prepared.visible_source_len, expected_slice_len);
        assert_eq!(prepared.total_display_len, expected_end);
        for wrapped in [false, true] {
            let hitbox = DiffTextHitbox {
                atomic: false,
                bounds: Bounds::new(
                    gpui::point(px(0.0), px(0.0)),
                    gpui::size(px(source_len as f32 + 10.0), px(20.0)),
                ),
                layout_key: 0,
                source_visible_ix: 0,
                text_start_offset: if wrapped { start } else { 0 },
                text_len: if wrapped {
                    prepared.visible_source_len
                } else {
                    prepared.total_display_len
                },
                offset_map: None,
                painted_text: prepared.styled.text.clone(),
                streamed_ascii_monospace_cell_width: Some(px(1.0)),
                wrapped: None,
                cells: Vec::new(),
            };
            let hitboxes = rustc_hash::FxHashMap::default();
            let geometry = TextGeometry {
                hitboxes: &hitboxes,
                motion_targets: &[],
                layout_cache: None,
                anchor: None,
                selecting: false,
            };
            let hit = geometry
                .diff_text_hit_in_hitbox(
                    &hitbox,
                    DiffTextRegion::Inline,
                    gpui::point(px(source_len as f32 + 2.0), px(10.0)),
                )
                .unwrap();
            assert_eq!(
                hit.pos.offset, expected_end,
                "hit testing clamps to the actual painted end"
            );
        }
    }
}

#[test]
fn revealed_wrapped_payload_retains_interior_and_terminal_markers() {
    let raw = "ab\rcd\r";
    let styled = build_cached_diff_styled_text_from_relative_highlights(4, raw, &[]);
    for (range, expected) in [(0..3, "ab␍"), (3..6, "cd␍↵")] {
        let payload = diff_text_paint_payload(
            4,
            Some(&styled),
            None,
            Some(super::super::DiffRowText::Str(raw)),
            true,
            DiffTextRegion::Inline,
            Some(DiffTextWrapSlice {
                wrap_ix: 0,
                wrap_columns: 3,
                primary_range: super::super::DiffWrapByteRange::from_range(range.clone()),
                secondary_range: super::super::DiffWrapByteRange::from_range(range),
            }),
        );
        assert_eq!(payload.text.as_ref(), expected);
        assert_eq!(payload.offset_map.unwrap().source_len(), 3);
    }
}

fn streamed_query_spec(
    raw_text: &str,
    query: &str,
    query_options: DiffSearchOptions,
) -> StreamedDiffTextPaintSpec {
    StreamedDiffTextPaintSpec {
        raw_text: gitcomet_core::file_diff::FileDiffLineText::from(raw_text),
        query: query.to_owned().into(),
        query_options,
        query_matcher: (!query.is_empty())
            .then(|| Arc::new(DiffSearchMatcher::new(query, query_options))),
        query_emphasis: DiffSearchMatchEmphasis::Other,
        word_ranges: Arc::from(Vec::<Range<usize>>::new()),
        word_kind: None,
        syntax: StreamedDiffTextSyntaxSource::None,
    }
}

fn highlight_ranges(styled: &CachedDiffStyledText) -> Vec<Range<usize>> {
    styled
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect()
}

#[test]
fn whitespace_offset_cache_keeps_each_tab_width() {
    let raw = "a\tneedle";
    let styled = CachedDiffStyledText {
        text: raw.into(),
        highlights: empty_highlights(),
        highlights_hash: 0,
        text_hash: 12345,
    };
    for width in [2, 8, 2] {
        for (text, map) in [
            whitespace_visible_raw_cached(width, raw),
            whitespace_visible_cached(width, &styled, Some(raw)),
        ] {
            assert_eq!(text.text.as_ref(), "a→needle↵");
            assert_eq!(map.source_len(), width + "needle".len());
            assert_eq!(map.source_offset_for_display("a→".len()), width);
            assert_eq!(map.display_offset_for_source(width), "a→".len());
        }
    }
}

#[test]
fn diff_text_paint_payload_reveals_whitespace_markers() {
    let tab_width = 4;

    let style = HighlightStyle::default();
    let styled = CachedDiffStyledText {
        text: "a b\t".into(),
        highlights: Arc::from(vec![(1..4, style)]),
        highlights_hash: 7,
        text_hash: 11,
    };

    let payload = diff_text_paint_payload(
        tab_width,
        Some(&styled),
        None,
        Some(super::super::DiffRowText::Str("a b\t")),
        true,
        DiffTextRegion::Inline,
        None,
    );

    assert_eq!(payload.text.as_ref(), "a·b→↵");
    assert_eq!(payload.highlights[0].0, 1..7);
    assert_ne!(payload.text_hash, styled.text_hash);

    let offset_map = payload.offset_map.expect("reveal whitespace offset map");
    assert_eq!(offset_map.source_offset_for_display(0), 0);
    assert_eq!(offset_map.source_offset_for_display("a·".len()), 2);
    // The tab at column 3 expands to one space.
    assert_eq!(offset_map.source_offset_for_display("a·b→".len()), 4);
    assert_eq!(offset_map.display_offset_for_source(2), "a·".len());
    assert_eq!(offset_map.display_offset_for_source(4), "a·b→".len());
}

#[test]
fn diff_text_paint_payload_keeps_streamed_whitespace_rows_unmaterialized() {
    let tab_width = 4;

    let raw = "a ".repeat((STREAMED_DIFF_TEXT_MIN_BYTES / 2).saturating_add(1));
    let spec = streamed_query_spec(raw.as_str(), "", DiffSearchOptions::default());

    assert!(should_stream_diff_text(Some(&spec)));

    let payload = diff_text_paint_payload(
        tab_width,
        None,
        Some(&spec),
        None,
        true,
        DiffTextRegion::Inline,
        None,
    );

    assert!(payload.text.is_empty());
    assert!(payload.highlights.is_empty());
    assert_ne!(payload.text_hash, 0);
    assert!(payload.offset_map.is_none());
}

#[test]
fn streamed_query_overlay_skips_whole_word_on_partial_slice() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let spec = streamed_query_spec(
        "foo_suffix",
        "foo",
        DiffSearchOptions {
            whole_word: true,
            ..DiffSearchOptions::default()
        },
    );

    let (styled, _, resolved) =
        build_streamed_diff_slice_styled_text(tab_width, theme, &spec, &(0..3));

    assert_eq!(resolved, 0..3);
    assert_eq!(styled.text.as_ref(), "foo");
    assert!(styled.highlights.is_empty());
}

#[test]
fn streamed_query_overlay_skips_regex_anchor_on_partial_slice() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let spec = streamed_query_spec(
        "prefixfoo suffix",
        r"^foo",
        DiffSearchOptions {
            regex: true,
            ..DiffSearchOptions::default()
        },
    );

    let (styled, _, resolved) =
        build_streamed_diff_slice_styled_text(tab_width, theme, &spec, &(6..9));

    assert_eq!(resolved, 6..9);
    assert_eq!(styled.text.as_ref(), "foo");
    assert!(styled.highlights.is_empty());
}

#[test]
fn streamed_query_overlay_keeps_boundary_sensitive_matches_on_full_slice() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let spec = streamed_query_spec(
        "foo suffix",
        r"^foo",
        DiffSearchOptions {
            regex: true,
            ..DiffSearchOptions::default()
        },
    );

    let (styled, _, resolved) =
        build_streamed_diff_slice_styled_text(tab_width, theme, &spec, &(0.."foo suffix".len()));

    assert_eq!(resolved, 0.."foo suffix".len());
    assert_eq!(highlight_ranges(&styled), vec![0..3]);
}
