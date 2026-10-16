use super::*;
use crate::view::panes::diff::diff_search::DiffSearchMatcher;

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
        Some("a b\t"),
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
