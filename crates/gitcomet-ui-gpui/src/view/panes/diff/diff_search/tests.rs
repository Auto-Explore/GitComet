use super::file_editor_search_ranges;
use super::identity_three_way_aligned;
use super::{
    AsciiCaseInsensitiveNeedle, ConflictResolverSearchContext, ConflictResolverSearchTwoWayRows,
    ConflictResolverSearchVisibleRows, DiffSearchMatcher, DiffSearchOptions, DiffSearchQueryReuse,
    FILE_EDITOR_SEARCH_MAX_MATCHES, FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES,
    StreamMatchCollectionMode, collect_file_diff_line_text_stream_match_visible_rows,
    collect_split_stream_match_visible_rows, collect_stream_match_visible_rows,
    collect_stream_match_visible_rows_with_mode, conflict_resolver_visible_match_indices,
    conflict_resolver_visible_match_indices_with_matcher, contains_ascii_case_insensitive,
    diff_search_inline_patch_query_uses_trigram_index, diff_search_query_matcher,
    diff_search_query_reuse, diff_search_resume_match_ix, diff_search_split_row_texts_match_query,
    empty_conflict_resolver_search_two_way_rows, three_way_visible_item_matches_query,
};
use crate::view::conflict_resolver;
use crate::view::conflict_resolver::{
    ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
    ConflictSplitRowIndex, TwoWaySplitProjection, build_three_way_visible_projection,
};
use crate::view::{
    ConflictModeState, ConflictResolverUiState, StreamedConflictState, ThreeWaySides,
};
use std::borrow::Cow;
use std::io::Write;
use std::ops::Range;
use std::sync::Arc;

fn three_way_search_context<'a>(
    marker_segments: &'a [ConflictSegment],
    visible: &'a conflict_resolver::ThreeWayVisibleProjection,
    base: (&'a str, &'a [usize]),
    ours: (&'a str, &'a [usize]),
    theirs: (&'a str, &'a [usize]),
) -> ConflictResolverSearchContext<'a> {
    ConflictResolverSearchContext {
        view_mode: ConflictResolverViewMode::ThreeWay,
        marker_segments,
        three_way_visible: ConflictResolverSearchVisibleRows::Projection(visible),
        three_way_base_text: base.0,
        three_way_base_line_starts: base.1,
        three_way_ours_text: ours.0,
        three_way_ours_line_starts: ours.1,
        three_way_theirs_text: theirs.0,
        three_way_theirs_line_starts: theirs.1,
        three_way_aligned: identity_three_way_aligned(),
        two_way_rows: empty_conflict_resolver_search_two_way_rows(),
    }
}

fn editor_search(text: &str, query: &str, options: DiffSearchOptions) -> Vec<Range<usize>> {
    editor_search_capped(text, query, options, FILE_EDITOR_SEARCH_MAX_MATCHES)
}

fn editor_search_capped(
    text: &str,
    query: &str,
    options: DiffSearchOptions,
    cap: usize,
) -> Vec<Range<usize>> {
    let model = crate::kit::text_model::TextModel::from_large_text(text);
    let matcher = DiffSearchMatcher::new(query, options);
    file_editor_search_ranges(&model.snapshot(), &matcher, cap)
}

#[test]
fn diff_search_query_matcher_preserves_significant_whitespace() {
    let space_matcher =
        diff_search_query_matcher(" ", DiffSearchOptions::default()).expect("space query");
    assert_eq!(space_matcher.query(), " ");
    assert!(space_matcher.is_match("a b"));

    let padded_matcher =
        diff_search_query_matcher(" foo ", DiffSearchOptions::default()).expect("padded query");
    assert_eq!(padded_matcher.query(), " foo ");
    assert!(padded_matcher.is_match("x foo y"));
    assert!(!padded_matcher.is_match("foo"));

    assert!(diff_search_query_matcher("", DiffSearchOptions::default()).is_none());
}

/// The whole point of the editor's own scan: three hits on one line are
/// three stops, not one. Every other view counts matching *rows*.
#[test]
fn editor_search_reports_every_occurrence_including_repeats_on_one_line() {
    let text = "let needle = needle.needle();\nother\nlast needle\n";
    let ranges = editor_search(text, "needle", DiffSearchOptions::default());

    assert_eq!(ranges.len(), 4);
    for range in &ranges {
        assert_eq!(&text[range.clone()], "needle");
    }
    // Document order, and offsets are absolute — the per-line scan has to add
    // each line's start back or every hit past line one lands in the wrong place.
    assert!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));
    assert_eq!(ranges[3].start, text.rfind("needle").unwrap());
}

#[test]
fn editor_search_honors_case_whole_word_and_regex_options() {
    let text = "Needle needles needle\n";

    assert_eq!(
        editor_search(text, "needle", DiffSearchOptions::default()).len(),
        3,
        "case-insensitive by default"
    );
    assert_eq!(
        editor_search(
            text,
            "needle",
            DiffSearchOptions {
                match_case: true,
                ..Default::default()
            }
        )
        .len(),
        2
    );
    assert_eq!(
        editor_search(
            text,
            "needle",
            DiffSearchOptions {
                whole_word: true,
                ..Default::default()
            }
        )
        .len(),
        2,
        "`needles` is not a whole-word hit"
    );
    let regex = editor_search(
        text,
        r"n..dle",
        DiffSearchOptions {
            regex: true,
            ..Default::default()
        },
    );
    assert_eq!(regex.len(), 3);
}

/// Per-line scanning must not turn a line edge into a word character, or
/// whole-word would miss every match that starts a line.
#[test]
fn editor_search_whole_word_matches_at_a_line_edge() {
    let ranges = editor_search(
        "needle\nprefix needle\n",
        "needle",
        DiffSearchOptions {
            whole_word: true,
            ..Default::default()
        },
    );
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0], 0..6);
}

/// `^`/`$` anchor per line because the matcher builds with `multi_line(true)`,
/// which is exactly what the per-line split reproduces.
#[test]
fn editor_search_regex_anchors_are_per_line() {
    let ranges = editor_search(
        "alpha\nbeta\nalpha\n",
        r"^alpha$",
        DiffSearchOptions {
            regex: true,
            ..Default::default()
        },
    );
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[1].start, 11);
}

/// The one query shape a line at a time cannot answer, so it takes the
/// flattening path instead of silently reporting nothing.
#[test]
fn editor_search_finds_a_multi_line_query() {
    let text = "one\ntwo\nthree\n";
    let ranges = editor_search(text, "one\ntwo", DiffSearchOptions::default());
    assert_eq!(ranges.len(), 1);
    assert_eq!(&text[ranges[0].clone()], "one\ntwo");
}

/// The editor stores a range per occurrence rather than per row, so a query
/// that matches everywhere has to be bounded or the list grows with the file.
#[test]
fn editor_search_stops_at_the_match_cap() {
    let text = "aaaaaaaa\naaaaaaaa\n";
    assert_eq!(
        editor_search_capped(text, "a", DiffSearchOptions::default(), 5).len(),
        5
    );
}

#[test]
fn editor_search_reports_nothing_for_an_empty_or_invalid_query() {
    assert!(editor_search("needle\n", "", DiffSearchOptions::default()).is_empty());
    assert!(
        editor_search(
            "needle\n",
            "(unclosed",
            DiffSearchOptions {
                regex: true,
                ..Default::default()
            }
        )
        .is_empty()
    );
}

#[test]
fn matches_empty_needle() {
    assert!(contains_ascii_case_insensitive("abc", ""));
}

#[test]
fn matches_case_insensitively() {
    assert!(contains_ascii_case_insensitive("Hello", "he"));
    assert!(contains_ascii_case_insensitive("Hello", "HEL"));
    assert!(contains_ascii_case_insensitive("Hello", "lo"));
}

#[test]
fn does_not_match_absent_substring() {
    assert!(!contains_ascii_case_insensitive("Hello", "world"));
}

#[test]
fn diff_search_query_reuse_detects_same_semantics_and_refinements() {
    assert_eq!(
        diff_search_query_reuse("Render_Cache", "render_cache"),
        DiffSearchQueryReuse::SameSemantics
    );
    assert_eq!(
        diff_search_query_reuse("Render_Cache", " render_cache "),
        DiffSearchQueryReuse::None
    );
    assert_eq!(
        diff_search_query_reuse("render_cache", "render_cache_hot_path"),
        DiffSearchQueryReuse::Refinement
    );
    assert_eq!(
        diff_search_query_reuse("", "render_cache"),
        DiffSearchQueryReuse::None
    );
    assert_eq!(
        diff_search_query_reuse("render_cache", "cache_render"),
        DiffSearchQueryReuse::None
    );
}

#[test]
fn diff_search_inline_patch_short_queries_skip_trigram_index() {
    assert!(!diff_search_inline_patch_query_uses_trigram_index(
        AsciiCaseInsensitiveNeedle::new("a").expect("needle")
    ));
    assert!(!diff_search_inline_patch_query_uses_trigram_index(
        AsciiCaseInsensitiveNeedle::new("ab").expect("needle")
    ));
    assert!(diff_search_inline_patch_query_uses_trigram_index(
        AsciiCaseInsensitiveNeedle::new("abc").expect("needle")
    ));
}

#[test]
fn diff_search_matcher_regex_anchors_match_visible_rows() {
    let start_anchor = DiffSearchMatcher::new(
        r"^use",
        DiffSearchOptions {
            regex: true,
            ..DiffSearchOptions::default()
        },
    );
    let mut matches = Vec::new();
    collect_stream_match_visible_rows(
        [
            (10, Cow::Borrowed("mod app;")),
            (11, Cow::Borrowed("use crate::app;")),
            (12, Cow::Borrowed("  use crate::other;")),
        ],
        &start_anchor,
        &mut matches,
    );
    assert_eq!(matches, vec![11]);

    let end_anchor = DiffSearchMatcher::new(
        r";$",
        DiffSearchOptions {
            regex: true,
            ..DiffSearchOptions::default()
        },
    );
    matches.clear();
    collect_stream_match_visible_rows(
        [
            (20, Cow::Borrowed("let x = 1")),
            (21, Cow::Borrowed("let y = 2;")),
            (22, Cow::Borrowed("let z = 3")),
        ],
        &end_anchor,
        &mut matches,
    );
    assert_eq!(matches, vec![21]);
}

#[test]
fn diff_search_matcher_matches_across_adjacent_visible_rows() {
    let matcher = DiffSearchMatcher::new("alpha\nbeta", DiffSearchOptions::default());
    let mut matches = Vec::new();
    collect_stream_match_visible_rows(
        [
            (10, Cow::Borrowed("alpha")),
            (11, Cow::Borrowed("beta")),
            (12, Cow::Borrowed("gamma")),
        ],
        &matcher,
        &mut matches,
    );

    assert_eq!(matches, vec![10]);
}

#[test]
fn diff_search_stream_collector_normalizes_crlf_row_endings() {
    let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
    let mut matches = Vec::new();
    collect_stream_match_visible_rows(
        [
            (10, Cow::Borrowed("foo\r")),
            (11, Cow::Borrowed("bar\r")),
            (12, Cow::Borrowed("baz\r")),
        ],
        &matcher,
        &mut matches,
    );

    assert_eq!(matches, vec![10]);
}

#[test]
fn diff_search_file_slice_stream_collector_normalizes_crlf_row_endings() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(b"foo\r\nbar\r\nbaz\r\n")
        .expect("write temp file");
    let path = Arc::new(file.path().to_path_buf());
    let rows = [
        (
            10,
            gitcomet_core::file_diff::FileDiffLineText::file_slice(
                Arc::clone(&path),
                0..4,
                true,
                false,
            ),
        ),
        (
            11,
            gitcomet_core::file_diff::FileDiffLineText::file_slice(
                Arc::clone(&path),
                5..9,
                true,
                false,
            ),
        ),
        (
            12,
            gitcomet_core::file_diff::FileDiffLineText::file_slice(
                Arc::clone(&path),
                10..14,
                true,
                false,
            ),
        ),
    ];

    let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
    let mut matches = Vec::new();
    collect_file_diff_line_text_stream_match_visible_rows(rows, &matcher, &mut matches);

    assert_eq!(matches, vec![10]);
}

#[test]
fn diff_search_file_slice_general_collector_does_not_materialize_large_rows() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    let payload_bytes = FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES * 128;
    let mut source = Vec::with_capacity(payload_bytes);
    source.extend_from_slice(b"needle ");
    source.extend(std::iter::repeat_n(
        b'x',
        FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES,
    ));
    source.push(0xff);
    source.resize(payload_bytes, b'x');
    file.write_all(source.as_slice()).expect("write temp file");
    let path = Arc::new(file.path().to_path_buf());

    let materialized = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        0..source.len(),
        false,
        false,
    );
    assert!(
        materialized.as_ref().is_empty(),
        "invalid UTF-8 sentinel should make full-row materialization unusable"
    );

    let matcher = DiffSearchMatcher::new(
        "needle",
        DiffSearchOptions {
            whole_word: true,
            ..DiffSearchOptions::default()
        },
    );
    let mut matches = Vec::new();
    let streamed = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        0..source.len(),
        false,
        false,
    );
    collect_file_diff_line_text_stream_match_visible_rows([(77, streamed)], &matcher, &mut matches);

    assert_eq!(matches, vec![77]);

    let regex_matcher = DiffSearchMatcher::new(
        r"^needle",
        DiffSearchOptions {
            regex: true,
            ..DiffSearchOptions::default()
        },
    );
    let mut regex_matches = Vec::new();
    let regex_streamed = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        0..source.len(),
        false,
        false,
    );
    collect_file_diff_line_text_stream_match_visible_rows(
        [(78, regex_streamed)],
        &regex_matcher,
        &mut regex_matches,
    );

    assert_eq!(regex_matches, vec![78]);
}

#[test]
fn diff_search_file_slice_case_sensitive_collector_does_not_materialize_large_rows() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    let payload_bytes = FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES * 128;
    let mut source = Vec::with_capacity(payload_bytes);
    source.extend_from_slice(b"Needle ");
    source.extend(std::iter::repeat_n(
        b'x',
        FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES,
    ));
    source.push(0xff);
    source.resize(payload_bytes, b'x');
    file.write_all(source.as_slice()).expect("write temp file");
    let path = Arc::new(file.path().to_path_buf());

    let materialized = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        0..source.len(),
        false,
        false,
    );
    assert!(
        materialized.as_ref().is_empty(),
        "invalid UTF-8 sentinel should make full-row materialization unusable"
    );

    let matcher = DiffSearchMatcher::new(
        "Needle",
        DiffSearchOptions {
            match_case: true,
            ..DiffSearchOptions::default()
        },
    );
    let streamed = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        0..source.len(),
        false,
        false,
    );
    let mut matches = Vec::new();
    collect_file_diff_line_text_stream_match_visible_rows([(77, streamed)], &matcher, &mut matches);

    assert_eq!(matches, vec![77]);
}

#[test]
fn diff_search_file_slice_literal_stream_respects_unicode_whole_word_boundaries() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    let row1 = "αβγ";
    let row2 = "β delta";
    let text = format!("{row1}\n{row2}\n");
    file.write_all(text.as_bytes()).expect("write temp file");
    let path = Arc::new(file.path().to_path_buf());

    let matcher = DiffSearchMatcher::new(
        "β",
        DiffSearchOptions {
            whole_word: true,
            ..DiffSearchOptions::default()
        },
    );
    let mut matches = Vec::new();
    let row1_slice = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        0..row1.len(),
        false,
        false,
    );
    let row2_start = row1.len() + 1;
    let row2_slice = gitcomet_core::file_diff::FileDiffLineText::file_slice(
        Arc::clone(&path),
        row2_start..(row2_start + row2.len()),
        false,
        false,
    );

    collect_file_diff_line_text_stream_match_visible_rows(
        [(10, row1_slice), (11, row2_slice)],
        &matcher,
        &mut matches,
    );

    assert_eq!(matches, vec![11]);
}

#[test]
fn diff_search_matcher_does_not_match_multiline_fragments_on_non_adjacent_rows() {
    let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());
    let mut matches = Vec::new();
    collect_stream_match_visible_rows(
        [
            (10, Cow::Borrowed("foo")),
            (11, Cow::Borrowed("not the middle of the query")),
            (12, Cow::Borrowed("bar")),
        ],
        &matcher,
        &mut matches,
    );

    assert!(matches.is_empty());
}

#[test]
fn diff_search_stream_collector_reports_each_start_row_once() {
    let matcher = DiffSearchMatcher::new(
        "a",
        DiffSearchOptions {
            match_case: true,
            ..DiffSearchOptions::default()
        },
    );
    let mut matches = Vec::new();
    collect_stream_match_visible_rows(
        [
            (10, Cow::Borrowed("aaaa")),
            (11, Cow::Borrowed("bbbb")),
            (12, Cow::Borrowed("caca")),
        ],
        &matcher,
        &mut matches,
    );

    assert_eq!(matches, vec![10, 12]);
}

#[test]
fn diff_search_single_line_literal_stream_collector_avoids_concat_allocations() {
    let rows = (0..8192)
        .map(|ix| (ix, format!("row_{ix:04}_alpha_beta_gamma_delta")))
        .collect::<Vec<_>>();
    let matcher = DiffSearchMatcher::new(
        "NEEDLE",
        DiffSearchOptions {
            match_case: true,
            ..DiffSearchOptions::default()
        },
    );

    let mut matches = Vec::new();
    let mode = collect_stream_match_visible_rows_with_mode(
        rows.iter()
            .map(|(visible_ix, text)| (*visible_ix, Cow::Borrowed(text.as_str()))),
        &matcher,
        &mut matches,
    );

    assert!(matches.is_empty());
    assert_eq!(mode, StreamMatchCollectionMode::SingleRowLiteral);
}

#[test]
fn diff_search_single_line_literal_split_collector_honors_whole_word_boundaries() {
    let matcher = DiffSearchMatcher::new(
        "cat",
        DiffSearchOptions {
            whole_word: true,
            ..DiffSearchOptions::default()
        },
    );
    let mut matches = Vec::new();

    collect_split_stream_match_visible_rows(
        [
            (
                10,
                Some(Cow::Borrowed("concatenate")),
                Some(Cow::Borrowed("bobcat")),
            ),
            (11, Some(Cow::Borrowed("cat")), None),
            (12, None, Some(Cow::Borrowed("dog cat mouse"))),
            (
                13,
                Some(Cow::Borrowed("cat_thing")),
                Some(Cow::Borrowed("copycat")),
            ),
        ],
        &matcher,
        &mut matches,
    );

    assert_eq!(matches, vec![11, 12]);
}

#[test]
fn diff_search_stream_collector_keeps_empty_row_separators() {
    let matcher = DiffSearchMatcher::new("\nbar", DiffSearchOptions::default());
    let mut matches = Vec::new();
    collect_stream_match_visible_rows(
        [
            (10, Cow::Borrowed("")),
            (11, Cow::Borrowed("bar")),
            (12, Cow::Borrowed("baz")),
        ],
        &matcher,
        &mut matches,
    );

    assert_eq!(matches, vec![10]);
}

#[test]
fn diff_search_split_stream_collector_preserves_missing_side_rows() {
    let mut matches = Vec::new();
    collect_split_stream_match_visible_rows(
        [
            (10, Some(Cow::Borrowed("foo")), Some(Cow::Borrowed("alpha"))),
            (11, None, Some(Cow::Borrowed("beta"))),
            (12, Some(Cow::Borrowed("bar")), Some(Cow::Borrowed("gamma"))),
        ],
        &DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default()),
        &mut matches,
    );
    assert!(
        matches.is_empty(),
        "a missing split side cell must not collapse out of the search stream"
    );

    collect_split_stream_match_visible_rows(
        [
            (10, Some(Cow::Borrowed("foo")), Some(Cow::Borrowed("alpha"))),
            (11, None, Some(Cow::Borrowed("beta"))),
            (12, Some(Cow::Borrowed("bar")), Some(Cow::Borrowed("gamma"))),
        ],
        &DiffSearchMatcher::new("foo\n\nbar", DiffSearchOptions::default()),
        &mut matches,
    );
    assert_eq!(matches, vec![10]);
}

#[test]
fn diff_search_resume_keeps_exact_visible_match() {
    assert_eq!(diff_search_resume_match_ix(Some(20), &[4, 20, 30]), Some(1));
}

#[test]
fn diff_search_resume_positions_before_next_later_match_when_previous_disappears() {
    let matches = [4, 30, 50];
    let ix = diff_search_resume_match_ix(Some(20), &matches).expect("resume ix");
    assert_eq!(ix, 0);
    assert_eq!(matches[(ix + 1) % matches.len()], 30);
}

#[test]
fn diff_search_resume_wraps_when_no_later_match_remains() {
    let matches = [4, 12];
    let ix = diff_search_resume_match_ix(Some(20), &matches).expect("resume ix");
    assert_eq!(ix, 1);
    assert_eq!(matches[(ix + 1) % matches.len()], 4);
}

#[test]
fn diff_search_resume_starts_at_first_without_previous_match() {
    assert_eq!(diff_search_resume_match_ix(None, &[4, 20, 30]), Some(0));
    assert_eq!(diff_search_resume_match_ix(Some(20), &[]), None);
}

#[test]
fn split_row_text_search_matches_rendered_tab_expansion() {
    let tab_width = 4;

    let query = AsciiCaseInsensitiveNeedle::new("a   b").expect("query");
    let mut expanded_tabs = String::new();

    assert!(diff_search_split_row_texts_match_query(
        tab_width,
        query,
        Some("a\tb"),
        None,
        &mut expanded_tabs,
    ));
    assert!(diff_search_split_row_texts_match_query(
        tab_width,
        query,
        None,
        Some("a\tb"),
        &mut expanded_tabs,
    ));
}

#[test]
fn conflict_search_three_way_mode_uses_three_way_visible_rows() {
    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: Some("base".into()),
        ours: "needle\n".into(),
        theirs: "remote\n".into(),
        choice: ConflictChoice::Theirs,
        resolved: true,
        whitespace_only: false,
    })];
    let visible_range = 0..1;
    let three_way_visible_projection = build_three_way_visible_projection(
        1,
        std::slice::from_ref(&visible_range),
        &marker_segments,
        false,
    );
    let three_way_base_text = "base text\n";
    let three_way_ours_text = "needle\n";
    let three_way_theirs_text = "remote text\n";
    let three_way_base_line_starts = vec![0];
    let three_way_ours_line_starts = vec![0];
    let three_way_theirs_line_starts = vec![0];

    let three_way_ctx = ConflictResolverSearchContext {
        view_mode: ConflictResolverViewMode::ThreeWay,
        marker_segments: &marker_segments,
        three_way_visible: ConflictResolverSearchVisibleRows::Projection(
            &three_way_visible_projection,
        ),
        three_way_base_text,
        three_way_base_line_starts: &three_way_base_line_starts,
        three_way_ours_text,
        three_way_ours_line_starts: &three_way_ours_line_starts,
        three_way_theirs_text,
        three_way_theirs_line_starts: &three_way_theirs_line_starts,
        three_way_aligned: identity_three_way_aligned(),
        two_way_rows: empty_conflict_resolver_search_two_way_rows(),
    };

    assert_eq!(
        conflict_resolver_visible_match_indices("needle", &three_way_ctx),
        vec![0]
    );
    assert!(
        conflict_resolver_visible_match_indices("split-only", &three_way_ctx).is_empty(),
        "three-way search should ignore two-way rows",
    );

    let index = ConflictSplitRowIndex::new(&marker_segments, 1);
    let projection = TwoWaySplitProjection::new(&index, &marker_segments, false);
    let two_way_ctx = ConflictResolverSearchContext {
        three_way_aligned: identity_three_way_aligned(),
        view_mode: ConflictResolverViewMode::TwoWayDiff,
        marker_segments: &marker_segments,
        three_way_visible: ConflictResolverSearchVisibleRows::Projection(
            &three_way_visible_projection,
        ),
        three_way_base_text,
        three_way_base_line_starts: &three_way_base_line_starts,
        three_way_ours_text,
        three_way_ours_line_starts: &three_way_ours_line_starts,
        three_way_theirs_text,
        three_way_theirs_line_starts: &three_way_theirs_line_starts,
        two_way_rows: ConflictResolverSearchTwoWayRows::Streamed {
            split_row_index: &index,
            two_way_split_projection: &projection,
        },
    };
    assert_eq!(
        conflict_resolver_visible_match_indices("needle", &two_way_ctx),
        vec![0]
    );
}

#[test]
fn conflict_search_two_way_general_matches_across_adjacent_rows() {
    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "foo\nbar\n".into(),
        theirs: "remote\nside\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    let index = ConflictSplitRowIndex::new(&marker_segments, 1);
    let projection = TwoWaySplitProjection::new(&index, &marker_segments, false);
    let three_way_visible_projection =
        build_three_way_visible_projection(0, &[], &marker_segments, false);
    let ctx = ConflictResolverSearchContext {
        three_way_aligned: identity_three_way_aligned(),
        view_mode: ConflictResolverViewMode::TwoWayDiff,
        marker_segments: &marker_segments,
        three_way_visible: ConflictResolverSearchVisibleRows::Projection(
            &three_way_visible_projection,
        ),
        three_way_base_text: "",
        three_way_base_line_starts: &[],
        three_way_ours_text: "",
        three_way_ours_line_starts: &[],
        three_way_theirs_text: "",
        three_way_theirs_line_starts: &[],
        two_way_rows: ConflictResolverSearchTwoWayRows::Streamed {
            split_row_index: &index,
            two_way_split_projection: &projection,
        },
    };
    let matcher = DiffSearchMatcher::new("foo\nbar", DiffSearchOptions::default());

    assert_eq!(
        conflict_resolver_visible_match_indices_with_matcher(&matcher, &ctx),
        vec![0]
    );
}

#[test]
fn conflict_search_three_way_collapsed_rows_match_choice_summary() {
    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: Some("base".into()),
        ours: "ours".into(),
        theirs: "theirs".into(),
        choice: ConflictChoice::Theirs,
        resolved: true,
        whitespace_only: false,
    })];
    let visible_range = 0..1;
    let three_way_visible_projection = build_three_way_visible_projection(
        1,
        std::slice::from_ref(&visible_range),
        &marker_segments,
        true,
    );

    let ctx = three_way_search_context(
        &marker_segments,
        &three_way_visible_projection,
        ("", &[]),
        ("", &[]),
        ("", &[]),
    );

    assert_eq!(
        conflict_resolver_visible_match_indices("resolved", &ctx),
        vec![0]
    );
    assert_eq!(
        conflict_resolver_visible_match_indices("remote", &ctx),
        vec![0]
    );
}

#[test]
fn conflict_search_three_way_projection_uses_streamed_visible_rows() {
    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: Some("base".into()),
        ours: "needle\n".into(),
        theirs: "remote\n".into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    let conflict_ranges = 0..1;
    let three_way_visible_projection = build_three_way_visible_projection(
        1,
        std::slice::from_ref(&conflict_ranges),
        &marker_segments,
        false,
    );

    let ctx = three_way_search_context(
        &marker_segments,
        &three_way_visible_projection,
        ("base\n", &[0]),
        ("needle\n", &[0]),
        ("remote\n", &[0]),
    );

    assert_eq!(
        conflict_resolver_visible_match_indices("needle", &ctx),
        vec![0]
    );
}

#[test]
fn three_way_span_search_matches_per_item_search() {
    // Build a multi-line conflict with text + block segments and verify
    // that span-based search (projection path) yields the same results
    // as per-item search (map path).
    let marker_segments = vec![
        ConflictSegment::Text("header\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base_needle\nbase_plain\n".into()),
            ours: "ours_plain\nours_needle\n".into(),
            theirs: "theirs_plain\ntheirs_plain\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("footer\n".into()),
    ];

    // Three-way line count = max(text_lines) across segments = 1 + 2 + 1 = 4
    let three_way_len = 4;
    let conflict_ranges = 1..3; // lines 1..3 are the conflict block

    let base_text = "header\nbase_needle\nbase_plain\nfooter\n";
    let ours_text = "header\nours_plain\nours_needle\nfooter\n";
    let theirs_text = "header\ntheirs_plain\ntheirs_plain\nfooter\n";
    let base_line_starts = vec![0, 7, 19, 30];
    let ours_line_starts = vec![0, 7, 18, 30];
    let theirs_line_starts = vec![0, 7, 21, 35];

    let projection = build_three_way_visible_projection(
        three_way_len,
        std::slice::from_ref(&conflict_ranges),
        &marker_segments,
        false,
    );

    let projection_ctx = three_way_search_context(
        &marker_segments,
        &projection,
        (base_text, &base_line_starts),
        (ours_text, &ours_line_starts),
        (theirs_text, &theirs_line_starts),
    );
    let proj_matches = conflict_resolver_visible_match_indices("needle", &projection_ctx);
    let manual_matches: Vec<usize> = (0..projection_ctx.three_way_visible_len())
        .filter(|&visible_ix| {
            projection_ctx
                .three_way_visible_item(visible_ix)
                .is_some_and(|item| {
                    three_way_visible_item_matches_query(item, &projection_ctx, "needle")
                })
        })
        .collect();

    assert_eq!(
        manual_matches, proj_matches,
        "span-based search must produce same results as per-item search"
    );
    assert!(
        !proj_matches.is_empty(),
        "should find at least one needle match"
    );
}

#[test]
fn two_way_source_text_search_matches_row_based_search() {
    // Build segments, create a ConflictSplitRowIndex + TwoWaySplitProjection,
    // and verify the source-text search path finds the same visible indices
    // as the old row-generation path.
    let marker_segments = vec![
        ConflictSegment::Text("context_line\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "alpha\nneedle_ours\ngamma\n".into(),
            theirs: "delta\nepsilon\nneedle_theirs\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
    ];
    let index = ConflictSplitRowIndex::new(&marker_segments, 1);
    let proj = TwoWaySplitProjection::new(&index, &marker_segments, false);

    let query = "needle";

    // Source-text search path (new):
    let matching_rows =
        index.search_ascii_case_insensitive_matching_rows(&marker_segments, query.as_bytes());
    let mut source_text_matches: Vec<usize> = matching_rows
        .into_iter()
        .filter_map(|r| proj.source_to_visible(r))
        .collect();
    source_text_matches.sort_unstable();

    // Row-generation search path (old):
    let mut row_based_matches = Vec::new();
    for visible_ix in 0..proj.visible_len() {
        let Some((source_ix, _)) = proj.get(visible_ix) else {
            continue;
        };
        let Some(row) = index.row_at(&marker_segments, source_ix) else {
            continue;
        };
        if row
            .old
            .as_deref()
            .is_some_and(|s| contains_ascii_case_insensitive(s, query))
            || row
                .new
                .as_deref()
                .is_some_and(|s| contains_ascii_case_insensitive(s, query))
        {
            row_based_matches.push(visible_ix);
        }
    }

    assert_eq!(
        source_text_matches, row_based_matches,
        "source-text search must match row-based search"
    );
    assert!(
        !source_text_matches.is_empty(),
        "should find needle matches"
    );
}

#[test]
fn three_way_span_search_handles_collapsed_blocks() {
    // Verify that collapsed resolved blocks are searchable via span search.
    let marker_segments = vec![ConflictSegment::Block(ConflictBlock {
        base: Some("base\n".into()),
        ours: "ours\n".into(),
        theirs: "theirs\n".into(),
        choice: ConflictChoice::Theirs,
        resolved: true,
        whitespace_only: false,
    })];
    let conflict_ranges = 0..1;
    let projection = build_three_way_visible_projection(
        1,
        std::slice::from_ref(&conflict_ranges),
        &marker_segments,
        true,
    );

    let ctx = three_way_search_context(
        &marker_segments,
        &projection,
        ("base\n", &[0]),
        ("ours\n", &[0]),
        ("theirs\n", &[0]),
    );

    // Collapsed block summary should match "Resolved" and "Remote".
    assert_eq!(
        conflict_resolver_visible_match_indices("resolved", &ctx),
        vec![0]
    );
    assert_eq!(
        conflict_resolver_visible_match_indices("remote", &ctx),
        vec![0]
    );
    // Should not match line content since it's collapsed.
    assert!(
        conflict_resolver_visible_match_indices("ours", &ctx).is_empty(),
        "collapsed block should not expose line content in search"
    );
}

#[test]
fn search_context_from_conflict_resolver_uses_streamed_mode_state() {
    let mut conflict_resolver = ConflictResolverUiState {
        view_mode: ConflictResolverViewMode::TwoWayDiff,
        mode_state: ConflictModeState::Streamed(StreamedConflictState::default()),
        ..ConflictResolverUiState::default()
    };
    conflict_resolver.marker_segments = vec![ConflictSegment::Text("context\n".into())];
    conflict_resolver.three_way_line_starts = ThreeWaySides {
        base: Vec::new().into(),
        ours: vec![0].into(),
        theirs: vec![0].into(),
    };
    conflict_resolver.three_way_text = ThreeWaySides {
        base: "".into(),
        ours: "context".into(),
        theirs: "context".into(),
    };

    let ctx = ConflictResolverSearchContext::from_conflict_resolver(&conflict_resolver);

    assert!(matches!(
        ctx.three_way_visible,
        ConflictResolverSearchVisibleRows::Projection(_)
    ));
    assert!(matches!(
        ctx.two_way_rows,
        ConflictResolverSearchTwoWayRows::Streamed { .. }
    ));
}
