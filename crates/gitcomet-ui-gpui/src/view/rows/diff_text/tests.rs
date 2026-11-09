use super::*;
use palette::IntoColor;

fn query_overlay_for_test(
    theme: AppTheme,
    base: &CachedDiffStyledText,
    query: &str,
    options: crate::view::panes::diff::diff_search::DiffSearchOptions,
) -> CachedDiffStyledText {
    query_overlay_for_test_with_emphasis(
        theme,
        base,
        query,
        options,
        DiffSearchMatchEmphasis::Other,
    )
}

fn query_overlay_for_test_with_emphasis(
    theme: AppTheme,
    base: &CachedDiffStyledText,
    query: &str,
    options: crate::view::panes::diff::diff_search::DiffSearchOptions,
    emphasis: DiffSearchMatchEmphasis,
) -> CachedDiffStyledText {
    let matcher = crate::view::panes::diff::diff_search::DiffSearchMatcher::new(query, options);
    build_cached_diff_query_overlay_styled_text(theme, base, &matcher, emphasis)
}

fn test_line_starts(text: &str) -> Arc<[usize]> {
    let mut line_starts = Vec::with_capacity(
        text.as_bytes()
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count()
            + 1,
    );
    line_starts.push(0);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(ix + 1);
        }
    }
    Arc::from(line_starts)
}

fn prepare_test_document(language: DiffSyntaxLanguage, text: &str) -> PreparedDiffSyntaxDocument {
    let text: SharedString = text.to_owned().into();
    let line_starts = test_line_starts(text.as_ref());
    match prepare_diff_syntax_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_millis(50),
        },
        None,
        None,
    ) {
        PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        PrepareDiffSyntaxDocumentResult::TimedOut => {
            inject_background_prepared_diff_syntax_document(
                prepare_diff_syntax_document_in_background_text(
                    language,
                    DiffSyntaxMode::Auto,
                    text,
                    line_starts,
                )
                .expect("background parse should be available for supported test documents"),
            )
        }
        PrepareDiffSyntaxDocumentResult::Unsupported => {
            panic!("test document should support prepared syntax parsing")
        }
    }
}

#[test]
fn sorted_range_cursor_advances_without_rescanning() {
    let ranges = [2..4, 6..8];
    let mut cursor = 0usize;

    assert!(!segment_overlaps_sorted_ranges(0, 2, &ranges, &mut cursor));
    assert_eq!(cursor, 0);

    assert!(segment_overlaps_sorted_ranges(2, 3, &ranges, &mut cursor));
    assert_eq!(cursor, 0);

    assert!(!segment_overlaps_sorted_ranges(4, 6, &ranges, &mut cursor));
    assert_eq!(cursor, 1);

    assert!(segment_overlaps_sorted_ranges(6, 7, &ranges, &mut cursor));
    assert_eq!(cursor, 1);

    assert!(!segment_overlaps_sorted_ranges(8, 9, &ranges, &mut cursor));
    assert_eq!(cursor, 2);
}

#[test]
fn whitespace_visible_text_maps_spaces_tabs_cr_and_line_breaks() {
    assert_eq!(whitespace_visible_text(" \t\r\n").as_ref(), "·→␍↵");
    assert_eq!(
        whitespace_visible_multiline_text("a b\r\nc\t").as_ref(),
        "a·b␍↵\nc→"
    );
}

#[test]
fn whitespace_visible_line_text_appends_eol_marker_and_remaps_highlights() {
    let style = gpui::HighlightStyle::default();
    let (display, highlights) =
        whitespace_visible_line_text_and_highlights("a b\t", &[(1..4, style)]);

    assert_eq!(display.as_ref(), "a·b→↵");
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].0, 1..7);

    let styled = CachedDiffStyledText {
        text: "x y".into(),
        highlights: Arc::from(vec![(1..3, style)]),
        highlights_hash: 7,
        text_hash: 11,
    };
    let visible = whitespace_visible_line_styled_text(&styled);
    assert_eq!(visible.text.as_ref(), "x·y↵");
    assert_eq!(visible.highlights[0].0, 1..4);
    assert_ne!(visible.text_hash, styled.text_hash);
}

#[test]
fn whitespace_visible_styled_text_can_omit_synthetic_eol_marker() {
    let styled = CachedDiffStyledText {
        text: "x y".into(),
        highlights: Arc::from(Vec::<(Range<usize>, gpui::HighlightStyle)>::new()),
        highlights_hash: 0,
        text_hash: 11,
    };

    let visible = whitespace_visible_styled_text(&styled, false);

    assert_eq!(visible.text.as_ref(), "x·y");
}

#[test]
fn diff_wrap_ranges_prefer_word_boundaries() {
    let tab_width = 4;

    let text = "alpha beta gamma";
    let rows = diff_wrap_ranges_for_text(tab_width, text, 9)
        .into_iter()
        .map(|range| text[range].to_string())
        .collect::<Vec<_>>();

    assert_eq!(rows, ["alpha ", "beta ", "gamma"]);
}

#[test]
fn diff_wrap_ranges_keep_a_word_that_ends_on_the_column_boundary() {
    let tab_width = 4;

    // "alpha beta" is exactly 10 columns and the next character is a space,
    // so the row is not splitting a word: giving "beta" back to the next row
    // would wrap the line four columns short of the width it had.
    let text = "alpha beta gamma";
    let rows = diff_wrap_ranges_for_text(tab_width, text, 10)
        .into_iter()
        .map(|range| text[range].to_string())
        .collect::<Vec<_>>();

    assert_eq!(rows, ["alpha beta", " gamma"]);
}

#[test]
fn diff_wrap_ranges_tile_the_line_and_respect_the_column_budget() {
    let tab_width = 4;

    let text = "alpha beta gamma delta";
    for columns in 1..=text.len() + 4 {
        let ranges = diff_wrap_ranges_for_text(tab_width, text, columns);
        let joined = ranges
            .iter()
            .cloned()
            .map(|range| &text[range])
            .collect::<String>();
        assert_eq!(joined, text, "columns={columns}: rows must tile the line");

        for range in ranges {
            let width = text[range.clone()].chars().count();
            assert!(
                width <= columns,
                "columns={columns}: row {:?} exceeds the budget",
                &text[range]
            );
        }
    }
}

#[test]
fn diff_wrap_ranges_hard_break_long_words() {
    let tab_width = 4;

    let text = "abcdefghijkl";
    let rows = diff_wrap_ranges_for_text(tab_width, text, 5)
        .into_iter()
        .map(|range| text[range].to_string())
        .collect::<Vec<_>>();

    assert_eq!(rows, ["abcde", "fghij", "kl"]);
}

#[test]
fn diff_wrap_ranges_count_tabs_to_their_tab_stop() {
    let tab_width = 4;

    // At column 3 the tab takes one column, so "aaa\t" fills a row of 4.
    let text = "aaa\tbbb";
    let rows = diff_wrap_ranges_for_text(tab_width, text, 4)
        .into_iter()
        .map(|range| text[range].to_string())
        .collect::<Vec<_>>();
    assert_eq!(rows, ["aaa\t", "bbb"]);
    assert_eq!(diff_wrap_row_count_for_text(tab_width, text, 4), 2);

    // A tab at column 0 takes the whole row.
    let text = "\tbbb";
    let rows = diff_wrap_ranges_for_text(tab_width, text, 4)
        .into_iter()
        .map(|range| text[range].to_string())
        .collect::<Vec<_>>();
    assert_eq!(rows, ["\t", "bbb"]);
    assert_eq!(diff_wrap_range_for_text(tab_width, text, 4, 1), Some(1..4));
}

#[test]
fn diff_wrap_range_matches_full_range_builder() {
    let tab_width = 4;

    let text = "alpha beta γamma\nnext\trow";
    let ranges = diff_wrap_ranges_for_text(tab_width, text, 9);

    for (wrap_ix, range) in ranges.iter().cloned().enumerate() {
        assert!(text.is_char_boundary(range.start));
        assert!(text.is_char_boundary(range.end));
        assert_eq!(
            diff_wrap_range_for_text(tab_width, text, 9, wrap_ix),
            Some(range)
        );
    }
    assert_eq!(
        diff_wrap_row_count_for_text(tab_width, text, 9),
        ranges.len()
    );
    assert_eq!(
        diff_wrap_range_for_text(tab_width, text, 9, ranges.len()),
        None
    );
}

#[test]
fn whitespace_visible_line_styled_text_for_raw_preserves_tab_markers() {
    let tab_width = 4;

    let style = gpui::HighlightStyle::default();
    let styled = build_cached_diff_styled_text_from_relative_highlights(
        tab_width,
        "a b\t",
        &[(3..4, style)],
    );

    assert_eq!(styled.text.as_ref(), "a b ");
    let visible = whitespace_visible_line_styled_text_for_raw(tab_width, &styled, "a b\t");

    assert_eq!(visible.text.as_ref(), "a·b→↵");
    assert_eq!(visible.highlights[0].0, 4..7);
}

#[test]
fn build_segments_fast_path_skips_syntax_work() {
    let tab_width = 4;

    let segments =
        build_diff_text_segments(tab_width, "a\tb", &[], "", None, DiffSyntaxMode::Auto, None);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text.as_ref(), "a   b");
    assert!(!segments[0].in_word);
    assert!(!segments[0].in_query);
    assert_eq!(segments[0].syntax, SyntaxTokenKind::None);
}

#[test]
fn build_cached_styled_text_plain_has_no_highlights() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let styled = build_cached_diff_styled_text(
        tab_width,
        theme,
        "a\tb",
        &[],
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    assert_eq!(styled.text.as_ref(), "a   b");
    assert!(styled.highlights.is_empty());
    assert_eq!(styled.highlights_hash, 0);
}

#[test]
fn build_segments_does_not_panic_on_non_char_boundary_ranges() {
    let tab_width = 4;

    // This can happen if token ranges are computed in bytes that don't align to UTF-8
    // boundaries. We should never panic during diff rendering.
    let text = "aé"; // 'é' is 2 bytes in UTF-8
    let ranges = vec![Range { start: 1, end: 2 }];
    let segments = build_diff_text_segments(
        tab_width,
        text,
        &ranges,
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text.as_ref(), text);
}

#[test]
fn styled_text_highlights_cover_combined_ranges() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let segments = vec![
        CachedDiffTextSegment {
            text: "abc".into(),
            in_word: false,
            in_query: false,
            syntax: SyntaxTokenKind::None,
        },
        CachedDiffTextSegment {
            text: "def".into(),
            in_word: false,
            in_query: true,
            syntax: SyntaxTokenKind::Keyword,
        },
    ];

    let (text, highlights) = styled_text_for_diff_segments(theme, &segments, None);
    assert_eq!(text.as_ref(), "abcdef");
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].0, 3..6);
    assert_eq!(highlights[0].1.font_weight, None);
    assert!(highlights[0].1.background_color.is_some());

    // Hashing highlights is used for caching shaped layouts; it should be stable for identical
    // highlight sequences within a process.
    let styled = build_cached_diff_styled_text(
        tab_width,
        theme,
        "abcdef",
        &[],
        "def",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    assert_eq!(styled.highlights.len(), 1);
    assert_eq!(styled.highlights[0].0, 3..6);
}

#[test]
fn cached_styled_text_highlights_all_query_occurrences() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let styled = build_cached_diff_styled_text(
        tab_width,
        theme,
        "abxxab",
        &[],
        "ab",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    assert_eq!(styled.highlights.len(), 2);
    assert_eq!(styled.highlights[0].0, 0..2);
    assert_eq!(styled.highlights[1].0, 4..6);
}

#[test]
fn styled_text_word_highlight_sets_background() {
    let theme = AppTheme::gitcomet_dark();
    let segments = vec![CachedDiffTextSegment {
        text: "x".into(),
        in_word: true,
        in_query: false,
        syntax: SyntaxTokenKind::None,
    }];
    let (text, highlights) =
        styled_text_for_diff_segments(theme, &segments, Some(crate::theme::DiffColorKind::Removed));
    assert_eq!(text.as_ref(), "x");
    assert_eq!(highlights.len(), 1);
    assert!(highlights[0].1.background_color.is_some());
}

#[test]
fn light_word_and_search_highlights_use_paired_foregrounds() {
    let theme = AppTheme::gitcomet_light();
    let segments = vec![CachedDiffTextSegment {
        text: "removed".into(),
        in_word: true,
        in_query: false,
        syntax: SyntaxTokenKind::None,
    }];
    let (_text, highlights) =
        styled_text_for_diff_segments(theme, &segments, Some(crate::theme::DiffColorKind::Removed));
    assert_eq!(
        highlights[0].1.background_color,
        Some(theme.colors.diff.removed.word_background.into_color())
    );
    assert_eq!(
        highlights[0].1.color,
        Some(theme.colors.diff.removed.foreground.into_color())
    );

    let query_segments = vec![CachedDiffTextSegment {
        text: "match".into(),
        in_word: false,
        in_query: true,
        syntax: SyntaxTokenKind::String,
    }];
    let (_text, query_highlights) = styled_text_for_diff_segments(theme, &query_segments, None);
    assert_eq!(
        query_highlights[0].1.background_color,
        Some(theme.colors.editor.search_match_background.into_color())
    );
    assert_eq!(
        query_highlights[0].1.color,
        Some(theme.colors.editor.search_match_foreground.into_color())
    );
}

/// The word wash is the theme's own token in both appearances, and it is
/// resolved from the diff kind the caller already knows rather than by
/// recognising a colour it was handed. Deriving it per renderer gave a line
/// past `STREAMED_DIFF_TEXT_MIN_BYTES` a different colour from its
/// neighbours; recognising it by colour paints removed words with the added
/// wash in any theme that reuses one hue for two kinds.
#[test]
fn word_highlights_come_from_the_diff_palettes_own_token() {
    use crate::theme::DiffColorKind;

    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        for (kind, set) in [
            (DiffColorKind::Added, theme.colors.diff.added),
            (DiffColorKind::Removed, theme.colors.diff.removed),
            (DiffColorKind::Modified, theme.colors.diff.modified),
        ] {
            let (background, foreground) = word_highlight_colors(theme, kind);
            assert_eq!(background, set.word_background, "{kind:?}");
            // Dark carries the token as an alpha over the row and leaves the
            // syntax colour alone; light carries it opaque, which would
            // drown syntax colours, so the diff foreground is pinned.
            assert_eq!(
                foreground,
                (!theme.is_dark).then_some(set.foreground),
                "{kind:?}"
            );
        }
    }
}

#[test]
fn syntax_colors_are_softened_for_keywords() {
    let theme = AppTheme::gitcomet_light();
    let segments = vec![CachedDiffTextSegment {
        text: "fn".into(),
        in_word: false,
        in_query: false,
        syntax: SyntaxTokenKind::Keyword,
    }];

    let (_text, highlights) = styled_text_for_diff_segments(theme, &segments, None);
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].0, 0..2);
    assert_ne!(
        highlights[0].1.color,
        Some(theme.colors.accent.foreground.into_color())
    );
}

#[test]
fn prepared_yaml_fast_path_matches_legacy_segment_builder_for_real_diff_lines() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let text = concat!(
        "name: Deployment CI\n",
        "\n",
        "on:\n",
        "  pull_request:\n",
        "    branches: [\"main\"]\n",
        "    paths:\n",
        "      - \".github/workflows/build-release-artifacts.yml\"\n",
        "      - \".github/workflows/deploy-aur.yml\"\n",
        "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
        "      - \".github/workflows/deploy-apt-repo.yml\"\n",
        "      - \".github/workflows/release-manual-main.yml\"\n",
        "      - \"scripts/update-aur.sh\"\n",
        "      - \"scripts/package-macos.sh\"\n",
        "      - \"scripts/macos-cargo-config.sh\"\n",
        "      - \"scripts/generate-homebrew-formula.sh\"\n",
        "      - \"scripts/generate-homebrew-cask.sh\"\n",
        "      - \"scripts/build-apt-repo.sh\"\n",
        "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
        "  push:\n",
        "    branches: [\"main\"]\n",
        "    paths:\n",
        "      - \".github/workflows/build-release-artifacts.yml\"\n",
        "      - \".github/workflows/deploy-aur.yml\"\n",
        "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
        "      - \".github/workflows/deploy-apt-repo.yml\"\n",
        "      - \".github/workflows/release-manual-main.yml\"\n",
        "      - \"scripts/update-aur.sh\"\n",
        "      - \"scripts/package-macos.sh\"\n",
        "      - \"scripts/macos-cargo-config.sh\"\n",
        "      - \"scripts/generate-homebrew-formula.sh\"\n",
        "      - \"scripts/generate-homebrew-cask.sh\"\n",
        "      - \"scripts/build-apt-repo.sh\"\n",
        "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
        "  workflow_dispatch:\n",
        "\n",
        "permissions:\n",
        "  contents: read\n",
        "\n",
        "env:\n",
        "  CARGO_TERM_COLOR: always\n",
    );
    let lines: Vec<&str> = text.lines().collect();
    let document = prepare_test_document(DiffSyntaxLanguage::Yaml, text);
    let line_ixs = [
        16usize, 17, 18, 21, 23, 25, 26, 27, 28, 29, 30, 31, 32, 33, 35,
    ];

    for line_ix in line_ixs {
        let line_text = lines[line_ix];
        let tokens = syntax::syntax_tokens_for_prepared_document_line(document.inner, line_ix)
            .unwrap_or_else(|| panic!("prepared YAML line {line_ix} should be available"));

        let current = match build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
            tab_width,
            theme,
            line_text,
            &[],
            "",
            DiffSyntaxConfig {
                language: Some(DiffSyntaxLanguage::Yaml),
                mode: DiffSyntaxMode::Auto,
            },
            None,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix,
            },
        ) {
            PreparedDocumentLineStyledText::Cacheable(styled) => styled,
            PreparedDocumentLineStyledText::Pending(_) => {
                panic!("prepared YAML line {line_ix} should not still be pending")
            }
        };

        let legacy = segments_to_cached_styled_text(
            theme,
            &build_diff_text_segments(
                tab_width,
                line_text,
                &[],
                "",
                None,
                DiffSyntaxMode::HeuristicOnly,
                Some(tokens.as_ref()),
            ),
            None,
        );

        assert_eq!(
            current.text.as_ref(),
            legacy.text.as_ref(),
            "text mismatch for YAML line {line_ix}: {line_text:?}",
        );
        assert_eq!(
            current.highlights.as_ref(),
            legacy.highlights.as_ref(),
            "highlight mismatch for YAML line {line_ix}: {line_text:?}",
        );
    }
}

#[test]
fn heuristic_yaml_highlights_match_prepared_document_for_real_diff_lines() {
    let theme = AppTheme::gitcomet_dark();
    let text = concat!(
        "name: Deployment CI\n",
        "\n",
        "on:\n",
        "  pull_request:\n",
        "    branches: [\"main\"]\n",
        "    paths:\n",
        "      - \".github/workflows/build-release-artifacts.yml\"\n",
        "      - \".github/workflows/deploy-aur.yml\"\n",
        "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
        "      - \".github/workflows/deploy-apt-repo.yml\"\n",
        "      - \".github/workflows/release-manual-main.yml\"\n",
        "      - \"scripts/update-aur.sh\"\n",
        "      - \"scripts/package-macos.sh\"\n",
        "      - \"scripts/macos-cargo-config.sh\"\n",
        "      - \"scripts/generate-homebrew-formula.sh\"\n",
        "      - \"scripts/generate-homebrew-cask.sh\"\n",
        "      - \"scripts/build-apt-repo.sh\"\n",
        "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
        "  push:\n",
        "    branches: [\"main\"]\n",
        "    paths:\n",
        "      - \".github/workflows/build-release-artifacts.yml\"\n",
        "      - \".github/workflows/deploy-aur.yml\"\n",
        "      - \".github/workflows/deploy-homebrew-tap.yml\"\n",
        "      - \".github/workflows/deploy-apt-repo.yml\"\n",
        "      - \".github/workflows/release-manual-main.yml\"\n",
        "      - \"scripts/update-aur.sh\"\n",
        "      - \"scripts/package-macos.sh\"\n",
        "      - \"scripts/macos-cargo-config.sh\"\n",
        "      - \"scripts/generate-homebrew-formula.sh\"\n",
        "      - \"scripts/generate-homebrew-cask.sh\"\n",
        "      - \"scripts/build-apt-repo.sh\"\n",
        "      - \"scripts/windows/verify-signed-artifact.ps1\"\n",
        "  workflow_dispatch:\n",
        "\n",
        "permissions:\n",
        "  contents: read\n",
        "\n",
        "env:\n",
        "  CARGO_TERM_COLOR: always\n",
    );
    let lines: Vec<&str> = text.lines().collect();
    let document = prepare_test_document(DiffSyntaxLanguage::Yaml, text);
    let line_ixs = [
        16usize, 17, 18, 21, 23, 25, 26, 27, 28, 29, 30, 31, 32, 33, 35,
    ];

    for line_ix in line_ixs {
        let line_text = lines[line_ix];
        let tokens = syntax::syntax_tokens_for_prepared_document_line(document.inner, line_ix)
            .unwrap_or_else(|| panic!("prepared YAML line {line_ix} should be available"));
        let prepared =
            prepared_document_line_highlights_from_tokens(theme, line_text.len(), &tokens)
                .into_iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range, style.color))
                .collect::<Vec<_>>();
        let heuristic = syntax_highlights_for_line(
            theme,
            line_text,
            DiffSyntaxLanguage::Yaml,
            DiffSyntaxMode::HeuristicOnly,
        )
        .into_iter()
        .filter(|(_, style)| style.background_color.is_none())
        .map(|(range, style)| (range, style.color))
        .collect::<Vec<_>>();

        assert_eq!(
            heuristic, prepared,
            "heuristic YAML highlighting should match prepared document highlighting for line {line_ix}: {line_text:?}",
        );
    }
}

#[test]
fn cached_styled_text_from_relative_highlights_expands_tabs_and_remaps_ranges() {
    let tab_width = 4;

    let style = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let styled = build_cached_diff_styled_text_from_relative_highlights(
        tab_width,
        "\tlet value",
        &[(0..1, style), (1..4, style)],
    );

    assert_eq!(styled.text.as_ref(), "    let value");
    assert_eq!(styled.highlights.len(), 2);
    assert_eq!(styled.highlights[0].0, 0..4);
    assert_eq!(styled.highlights[1].0, 4..7);
}

#[test]
fn cached_styled_text_from_relative_highlights_handles_multibyte_utf8_with_tabs() {
    let tab_width = 4;

    let style = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.5, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    // "→" is 3 bytes (U+2192), tab is 1 byte.
    // Input: "\t→x" — 5 bytes: tab(0..1), arrow(1..4), x(4..5)
    let styled = build_cached_diff_styled_text_from_relative_highlights(
        tab_width,
        "\t\u{2192}x",
        &[(0..1, style), (1..4, style), (4..5, style)],
    );

    // Tab expands to 4 spaces, arrow stays 3 bytes, x stays 1 byte.
    assert_eq!(styled.text.as_ref(), "    \u{2192}x");
    assert_eq!(styled.highlights.len(), 3);
    // Tab (0..1) → expanded to 4-space span (0..4).
    assert_eq!(styled.highlights[0].0, 0..4);
    // Arrow (1..4) → starts at 4 (after tab expansion), length 3 bytes.
    assert_eq!(styled.highlights[1].0, 4..7);
    // x (4..5) → starts at 7.
    assert_eq!(styled.highlights[2].0, 7..8);
}

#[test]
fn cached_styled_text_from_relative_highlights_no_tabs_passes_through() {
    let tab_width = 4;

    let style = gpui::HighlightStyle {
        color: Some(gpui::hsla(0.5, 1.0, 0.5, 1.0)),
        ..gpui::HighlightStyle::default()
    };
    let styled = build_cached_diff_styled_text_from_relative_highlights(
        tab_width,
        "let x = 1;",
        &[(0..3, style), (8..9, style)],
    );

    assert_eq!(styled.text.as_ref(), "let x = 1;");
    assert_eq!(styled.highlights.len(), 2);
    assert_eq!(styled.highlights[0].0, 0..3);
    assert_eq!(styled.highlights[1].0, 8..9);
}

#[test]
fn cached_styled_text_syntax_only_expands_tabs_without_segment_build() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let styled = build_cached_diff_styled_text(
        tab_width,
        theme,
        "\tlet value = 42;",
        &[],
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );

    assert_eq!(styled.text.as_ref(), "    let value = 42;");
    assert!(styled.highlights.iter().any(|(range, _)| *range == (4..7)));
    assert!(
        styled
            .highlights
            .iter()
            .any(|(range, _)| *range == (16..18))
    );
}

#[allow(clippy::single_range_in_vec_init)]
#[test]
fn styled_line_caches_follow_tab_width_changes() {
    let theme = AppTheme::gitcomet_dark();
    let palette = syntax_highlight_palette(theme);
    let text = "\tlet value = 42;";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    assert!(syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some());
    for tab_width in [4, 8, 2, 4] {
        let plain = build_cached_diff_styled_text(
            tab_width,
            theme,
            text,
            &[],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
        let words = build_cached_diff_styled_text(
            tab_width,
            theme,
            text,
            &[5..10],
            "",
            Some(DiffSyntaxLanguage::Rust),
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
        let prepared =
            match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                tab_width,
                theme,
                &palette,
                PreparedDiffTextBuildRequest {
                    build: DiffTextBuildRequest {
                        text,
                        word_ranges: &[],
                        query: "",
                        syntax: DiffSyntaxConfig {
                            language: Some(DiffSyntaxLanguage::Rust),
                            mode: DiffSyntaxMode::Auto,
                        },
                        word_kind: None,
                    },
                    prepared_line: PreparedDiffSyntaxLine {
                        document: Some(document),
                        line_ix: 0,
                    },
                },
            ) {
                PreparedDocumentLineStyledText::Cacheable(styled) => styled,
                PreparedDocumentLineStyledText::Pending(_) => panic!("prepared line is ready"),
            };
        let width = tab_width;
        for (kind, styled) in [("syntax", plain), ("word", words), ("prepared", prepared)] {
            assert_eq!(
                styled.text.as_ref(),
                format!("{}let value = 42;", " ".repeat(width)),
                "{kind} cache at width {width}"
            );
            assert!(
                styled
                    .highlights
                    .iter()
                    .any(|(range, _)| *range == (width..width + 3)),
                "{kind} keyword must move with the tab width"
            );
        }
    }
}

#[test]
fn repeated_syntax_only_line_styled_text_reuses_cached_highlights() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let text = "let cached_value = 42;";

    let first = build_cached_diff_styled_text(
        tab_width,
        theme,
        text,
        &[],
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );
    let second = build_cached_diff_styled_text(
        tab_width,
        theme,
        text,
        &[],
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );

    assert!(
        !first.highlights.is_empty(),
        "heuristic syntax styling should produce highlights for Rust keywords"
    );
    assert!(
        Arc::ptr_eq(&first.highlights, &second.highlights),
        "repeated syntax-only lines should reuse the cached highlight arc"
    );
}

#[test]
fn repeated_word_highlighted_line_styled_text_reuses_cached_highlights() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let text = "let cached_value = replacement_value;";
    let word_ranges = [4..16, 19..36];

    let first = build_cached_diff_styled_text(
        tab_width,
        theme,
        text,
        &word_ranges,
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );
    let second = build_cached_diff_styled_text(
        tab_width,
        theme,
        text,
        &word_ranges,
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );

    assert!(
        !first.highlights.is_empty(),
        "word-highlighted syntax styling should produce at least one highlight segment"
    );
    assert!(
        Arc::ptr_eq(&first.highlights, &second.highlights),
        "repeated word-highlighted lines should reuse the cached highlight arc"
    );
}

#[test]
fn repeated_prepared_ready_line_styled_text_reuses_cached_text_and_highlights() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let highlight_palette = syntax_highlight_palette(theme);
    let text = "let prepared_cached_value = 42;";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    assert!(
        syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some(),
        "prepared document should expose a ready first line"
    );

    let request = PreparedDiffTextBuildRequest {
        build: DiffTextBuildRequest {
            text,
            word_ranges: &[],
            query: "",
            syntax: DiffSyntaxConfig {
                language: Some(DiffSyntaxLanguage::Rust),
                mode: DiffSyntaxMode::Auto,
            },
            word_kind: None,
        },
        prepared_line: PreparedDiffSyntaxLine {
            document: Some(document),
            line_ix: 0,
        },
    };

    let first =
        match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
            tab_width,
            theme,
            &highlight_palette,
            request,
        ) {
            PreparedDocumentLineStyledText::Cacheable(styled) => styled,
            PreparedDocumentLineStyledText::Pending(_) => {
                panic!("prepared first line should be cacheable once the chunk is loaded")
            }
        };
    let second =
        match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
            tab_width,
            theme,
            &highlight_palette,
            request,
        ) {
            PreparedDocumentLineStyledText::Cacheable(styled) => styled,
            PreparedDocumentLineStyledText::Pending(_) => {
                panic!("repeated prepared first line should remain cacheable")
            }
        };

    assert!(
        !first.highlights.is_empty(),
        "prepared syntax styling should produce highlights for Rust keywords"
    );
    assert!(
        Arc::ptr_eq(&first.highlights, &second.highlights),
        "repeated prepared ready-line lookups should reuse the cached highlight arc"
    );
    let first_text: Arc<str> = first.text.clone().into();
    let second_text: Arc<str> = second.text.clone().into();
    assert!(
        Arc::ptr_eq(&first_text, &second_text),
        "repeated prepared ready-line lookups should reuse the cached text arc"
    );
}

#[test]
fn prepared_ready_line_styled_text_cache_respects_full_document_context() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let highlight_palette = syntax_highlight_palette(theme);
    let line_text = "still comment */ let x = 1;";
    let multiline_document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &format!("/* open comment\n{line_text}"),
    );
    let standalone_document = prepare_test_document(DiffSyntaxLanguage::Rust, line_text);
    assert!(
        syntax::syntax_tokens_for_prepared_document_line(multiline_document.inner, 1).is_some(),
        "multiline prepared document should expose a ready continuation line"
    );
    assert!(
        syntax::syntax_tokens_for_prepared_document_line(standalone_document.inner, 0).is_some(),
        "standalone prepared document should expose a ready first line"
    );
    let multiline_tokens =
        syntax::syntax_tokens_for_prepared_document_line(multiline_document.inner, 1)
            .expect("multiline continuation tokens should remain available");
    assert!(
        multiline_tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment && token.range.start == 0),
        "multiline continuation tokens should start with a comment token, got: {multiline_tokens:?}"
    );

    let build = |document, line_ix| {
        match build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
            tab_width,
            theme,
            &highlight_palette,
            PreparedDiffTextBuildRequest {
                build: DiffTextBuildRequest {
                    text: line_text,
                    word_ranges: &[],
                    query: "",
                    syntax: DiffSyntaxConfig {
                        language: Some(DiffSyntaxLanguage::Rust),
                        mode: DiffSyntaxMode::Auto,
                    },
                    word_kind: None,
                },
                prepared_line: PreparedDiffSyntaxLine {
                    document: Some(document),
                    line_ix,
                },
            },
        ) {
            PreparedDocumentLineStyledText::Cacheable(styled) => styled,
            PreparedDocumentLineStyledText::Pending(_) => {
                panic!("ready prepared documents should not fall back to pending")
            }
        }
    };

    let multiline = build(multiline_document, 1);
    let standalone = build(standalone_document, 0);
    let start_style = |styled: &CachedDiffStyledText| {
        styled
            .highlights
            .iter()
            .find(|(range, _)| range.start == 0)
            .and_then(|(_, style)| style.color)
    };

    assert_eq!(
        start_style(&multiline),
        Some(theme.syntax.comment.into_color()),
        "multiline continuation should keep comment highlighting from document context"
    );
    assert_ne!(
        start_style(&standalone),
        Some(theme.syntax.comment.into_color()),
        "standalone line should not reuse cached comment styling from another prepared document"
    );
}

#[test]
fn syntax_only_line_styled_text_cache_is_scoped_by_theme() {
    let tab_width = 4;

    let text = "let themed_value = 42;";
    let dark_theme = AppTheme::gitcomet_dark();
    let light_theme = AppTheme::gitcomet_light();

    let dark = build_cached_diff_styled_text(
        tab_width,
        dark_theme,
        text,
        &[],
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );
    let light = build_cached_diff_styled_text(
        tab_width,
        light_theme,
        text,
        &[],
        "",
        Some(DiffSyntaxLanguage::Rust),
        DiffSyntaxMode::HeuristicOnly,
        None,
    );

    let dark_keyword = dark
        .highlights
        .iter()
        .find(|(range, _)| *range == (0..3))
        .and_then(|(_, style)| style.color);
    let light_keyword = light
        .highlights
        .iter()
        .find(|(range, _)| *range == (0..3))
        .and_then(|(_, style)| style.color);

    assert_eq!(dark_keyword, Some(dark_theme.syntax.keyword.into_color()));
    assert_eq!(light_keyword, Some(light_theme.syntax.keyword.into_color()));
    assert_ne!(
        dark_keyword, light_keyword,
        "theme-specific syntax colors should not bleed across cached entries"
    );
}

#[test]
fn prepared_document_byte_range_highlights_multiline_comment_continuation() {
    let theme = AppTheme::gitcomet_dark();
    let text = "/* open comment\nstill comment */ let x = 1;";
    let line_starts = vec![0, "/* open comment\n".len()];
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

    let second_line_start = line_starts[1];
    let highlights = syntax_highlights_for_prepared_document_byte_range(
        theme,
        text,
        &line_starts,
        document,
        second_line_start..text.len(),
    )
    .expect("prepared document should still be available");

    assert!(
        highlights
            .iter()
            .all(|(range, _)| range.start >= second_line_start),
        "returned highlights should be clipped to the requested byte range"
    );
    assert!(
        highlights.iter().any(|(range, style)| {
            range.start <= second_line_start
                && range.end > second_line_start
                && style.color == Some(theme.syntax.comment.into_color())
        }),
        "second line should retain comment highlighting from multiline document context"
    );
}

#[test]
fn nonblocking_prepared_document_byte_range_upgrades_after_chunk_build() {
    syntax::reset_prepared_syntax_cache();
    let theme = AppTheme::gitcomet_dark();
    let first_line = "/* unique nonblocking byte-range comment";
    let text_owned = format!("{first_line}\nstill unique comment */ let x = 1;");
    let line_starts = vec![0, first_line.len() + 1];
    let text = text_owned.as_str();
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

    let second_line_start = line_starts[1];
    let first = request_syntax_highlights_for_prepared_document_byte_range(
        theme,
        text,
        &line_starts,
        document,
        DiffSyntaxLanguage::Rust,
        second_line_start..text.len(),
    )
    .expect("prepared document should be requestable");
    assert!(first.pending);
    assert!(
        !first.highlights.iter().any(|(range, style)| {
            range.start <= second_line_start
                && range.end > second_line_start
                && style.color == Some(theme.syntax.comment.into_color())
        }),
        "heuristic fallback should not invent multiline comment state before the chunk is ready"
    );

    let started = std::time::Instant::now();
    while drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
        && started.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let second = request_syntax_highlights_for_prepared_document_byte_range(
        theme,
        text,
        &line_starts,
        document,
        DiffSyntaxLanguage::Rust,
        second_line_start..text.len(),
    )
    .expect("prepared document should still be available after chunk completion");
    assert!(!second.pending);
    assert!(
        second.highlights.iter().any(|(range, style)| {
            range.start <= second_line_start
                && range.end > second_line_start
                && style.color == Some(theme.syntax.comment.into_color())
        }),
        "resolved output should upgrade to full document-aware comment highlighting"
    );
}

#[test]
fn prepared_document_line_range_reports_ready_and_pending_rows_per_chunk() {
    let theme = AppTheme::gitcomet_dark();
    let lines: Vec<String> = (0..70)
        .map(|ix| format!("let chunk_boundary_value_{ix} = {ix};"))
        .collect();
    let text = lines.join("\n");
    let mut line_starts = Vec::with_capacity(lines.len());
    let mut offset = 0usize;
    for line in &lines {
        line_starts.push(offset);
        offset = offset.saturating_add(line.len()).saturating_add(1);
    }

    let document = prepare_test_document(DiffSyntaxLanguage::Rust, &text);
    assert!(
        syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some(),
        "first chunk should be loadable synchronously"
    );

    let first = request_syntax_highlights_for_prepared_document_line_range(
        theme,
        &text,
        &line_starts,
        document,
        DiffSyntaxLanguage::Rust,
        63..66,
    )
    .expect("row-range request should succeed");
    assert_eq!(first.len(), 3);
    assert_eq!(first[0].line_ix, 63);
    assert!(!first[0].pending, "loaded chunk row should be ready");
    assert!(
        !first[0].highlights.is_empty(),
        "ready row should include syntax highlights"
    );
    assert_eq!(first[1].line_ix, 64);
    assert_eq!(first[2].line_ix, 65);
    assert!(
        first[1].pending || first[2].pending,
        "at least one row in the next chunk should still be pending before the background drain"
    );

    let started = std::time::Instant::now();
    while drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
        && started.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let second = request_syntax_highlights_for_prepared_document_line_range(
        theme,
        &text,
        &line_starts,
        document,
        DiffSyntaxLanguage::Rust,
        63..66,
    )
    .expect("row-range request should still succeed after chunk drain");
    assert!(second.iter().all(|line| !line.pending));
}

#[test]
fn prepared_document_line_range_clamps_beyond_document_bounds() {
    let theme = AppTheme::gitcomet_dark();
    let text = "let a = 1;\nlet b = 2;";
    let line_starts = vec![0, "let a = 1;\n".len()];
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    // Load chunk 0 synchronously so the request API returns Ready.
    assert!(syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some());

    // Request range extends beyond the 2-line document.
    let result = request_syntax_highlights_for_prepared_document_line_range(
        theme,
        text,
        &line_starts,
        document,
        DiffSyntaxLanguage::Rust,
        1..5,
    )
    .expect("line-range request should succeed");
    // Should only return 1 line (line_ix 1), not 4.
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].line_ix, 1);
    assert!(!result[0].pending);

    // Fully out-of-bounds range returns empty.
    let result = request_syntax_highlights_for_prepared_document_line_range(
        theme,
        text,
        &line_starts,
        document,
        DiffSyntaxLanguage::Rust,
        10..15,
    )
    .expect("out-of-bounds range should still succeed");
    assert!(result.is_empty());
}

#[test]
fn nonblocking_prepared_line_helper_transitions_from_pending_to_cacheable() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let text = "let value = 1;";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

    let first = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
        tab_width,
        theme,
        text,
        &[],
        "",
        DiffSyntaxConfig {
            language: Some(DiffSyntaxLanguage::Rust),
            mode: DiffSyntaxMode::Auto,
        },
        None,
        PreparedDiffSyntaxLine {
            document: Some(document),
            line_ix: 0,
        },
    );
    match first {
        PreparedDocumentLineStyledText::Pending(styled) => {
            assert_eq!(styled.text.as_ref(), text);
        }
        PreparedDocumentLineStyledText::Cacheable(_) => {
            panic!("first nonblocking prepared-line request should be pending")
        }
    }

    let started = std::time::Instant::now();
    while drain_completed_prepared_diff_syntax_chunk_builds() == 0
        && started.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let second = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
        tab_width,
        theme,
        text,
        &[],
        "",
        DiffSyntaxConfig {
            language: Some(DiffSyntaxLanguage::Rust),
            mode: DiffSyntaxMode::Auto,
        },
        None,
        PreparedDiffSyntaxLine {
            document: Some(document),
            line_ix: 0,
        },
    );
    match second {
        PreparedDocumentLineStyledText::Cacheable(styled) => {
            assert!(
                !styled.highlights.is_empty(),
                "cacheable prepared-line styling should contain syntax highlights"
            );
        }
        PreparedDocumentLineStyledText::Pending(_) => {
            panic!("prepared-line helper should become cacheable after chunk drain")
        }
    }
}

#[test]
fn prepared_diff_syntax_line_for_one_based_line_converts_to_zero_based_index() {
    let document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &["let first = 1;", "let second = 2;"].join("\n"),
    );

    let prepared = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(2));
    assert_eq!(
        prepared,
        PreparedDiffSyntaxLine {
            document: Some(document),
            line_ix: 1,
        }
    );
}

#[test]
fn prepared_diff_syntax_line_for_one_based_line_rejects_missing_or_zero_lines() {
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, "let value = 1;");

    assert_eq!(
        prepared_diff_syntax_line_for_one_based_line(Some(document), None),
        PreparedDiffSyntaxLine {
            document: None,
            line_ix: 0,
        }
    );
    assert_eq!(
        prepared_diff_syntax_line_for_one_based_line(Some(document), Some(0)),
        PreparedDiffSyntaxLine {
            document: None,
            line_ix: 0,
        }
    );
}

#[test]
fn prepared_diff_syntax_line_for_inline_diff_row_projects_remove_add_and_context_lines() {
    use gitcomet_core::domain::DiffLineKind;

    let old_document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &["let old_one = 1;", "let old_two = 2;"].join("\n"),
    );
    let new_document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &["let new_one = 1;", "let new_two = 2;", "let new_three = 3;"].join("\n"),
    );

    let remove_line = AnnotatedDiffLine {
        kind: DiffLineKind::Remove,
        text: "-let old_two = 2;".into(),
        old_line: Some(2),
        new_line: None,
    };
    assert_eq!(
        prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &remove_line,
        ),
        PreparedDiffSyntaxLine {
            document: Some(old_document),
            line_ix: 1,
        }
    );

    let add_line = AnnotatedDiffLine {
        kind: DiffLineKind::Add,
        text: "+let new_three = 3;".into(),
        old_line: None,
        new_line: Some(3),
    };
    assert_eq!(
        prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &add_line,
        ),
        PreparedDiffSyntaxLine {
            document: Some(new_document),
            line_ix: 2,
        }
    );

    let context_line = AnnotatedDiffLine {
        kind: DiffLineKind::Context,
        text: " let new_one = 1;".into(),
        old_line: Some(1),
        new_line: Some(1),
    };
    assert_eq!(
        prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &context_line,
        ),
        PreparedDiffSyntaxLine {
            document: Some(new_document),
            line_ix: 0,
        }
    );
}

#[test]
fn prepared_diff_syntax_line_for_inline_diff_row_rejects_meta_rows_and_missing_lines() {
    use gitcomet_core::domain::DiffLineKind;

    let document = prepare_test_document(DiffSyntaxLanguage::Rust, "let value = 1;");

    let header_line = AnnotatedDiffLine {
        kind: DiffLineKind::Header,
        text: "diff --git a/file b/file".into(),
        old_line: None,
        new_line: None,
    };
    assert_eq!(
        prepared_diff_syntax_line_for_inline_diff_row(Some(document), Some(document), &header_line),
        PreparedDiffSyntaxLine {
            document: None,
            line_ix: 0,
        }
    );

    let missing_add_line = AnnotatedDiffLine {
        kind: DiffLineKind::Add,
        text: "+let value = 1;".into(),
        old_line: None,
        new_line: None,
    };
    assert_eq!(
        prepared_diff_syntax_line_for_inline_diff_row(
            Some(document),
            Some(document),
            &missing_add_line,
        ),
        PreparedDiffSyntaxLine {
            document: None,
            line_ix: 0,
        }
    );
}

#[test]
fn inline_projection_tokens_come_from_correct_document_side() {
    use gitcomet_core::domain::DiffLineKind;

    // Old document contains a struct definition; new document contains a function.
    // We verify that projected syntax tokens actually carry the expected token kinds
    // from the correct side, not just that the document/line_ix are set correctly.
    let old_document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &["struct Foo {", "    x: u32,", "}"].join("\n"),
    );
    let new_document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &["fn bar() {", "    let y = 42;", "}"].join("\n"),
    );

    // Remove line: old_line=1 should project from old document line 0 ("struct Foo {")
    let remove_line = AnnotatedDiffLine {
        kind: DiffLineKind::Remove,
        text: "-struct Foo {".into(),
        old_line: Some(1),
        new_line: None,
    };
    let projected = prepared_diff_syntax_line_for_inline_diff_row(
        Some(old_document),
        Some(new_document),
        &remove_line,
    );
    assert_eq!(projected.document, Some(old_document));
    let old_tokens = syntax::syntax_tokens_for_prepared_document_line(
        projected.document.unwrap().inner,
        projected.line_ix,
    );
    assert!(
        old_tokens
            .as_ref()
            .is_some_and(|tokens| tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword)),
        "remove line should get tokens from old doc containing 'struct' keyword: {old_tokens:?}"
    );

    // Add line: new_line=2 should project from new document line 1 ("    let y = 42;")
    let add_line = AnnotatedDiffLine {
        kind: DiffLineKind::Add,
        text: "+    let y = 42;".into(),
        old_line: None,
        new_line: Some(2),
    };
    let projected = prepared_diff_syntax_line_for_inline_diff_row(
        Some(old_document),
        Some(new_document),
        &add_line,
    );
    assert_eq!(projected.document, Some(new_document));
    let new_tokens = syntax::syntax_tokens_for_prepared_document_line(
        projected.document.unwrap().inner,
        projected.line_ix,
    );
    assert!(
        new_tokens
            .as_ref()
            .is_some_and(|tokens| tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number)),
        "add line should get tokens from new doc containing number literal: {new_tokens:?}"
    );
}

#[test]
fn split_view_projection_indexes_real_document_lines() {
    // Verify that prepared_diff_syntax_line_for_one_based_line gives correct
    // syntax tokens when the document is built from real file text rather than
    // the old aligned-row approach (which padded empty lines).
    let document = prepare_test_document(
        DiffSyntaxLanguage::Rust,
        &[
            "fn greet() {",          // line 1
            "    println!(\"hi\");", // line 2
            "}",                     // line 3
        ]
        .join("\n"),
    );

    // Line 1 should have a keyword ("fn")
    let line1 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(1));
    assert_eq!(line1.line_ix, 0);
    let tokens = syntax::syntax_tokens_for_prepared_document_line(
        line1.document.unwrap().inner,
        line1.line_ix,
    );
    assert!(
        tokens
            .as_ref()
            .is_some_and(|t| t.iter().any(|tok| tok.kind == SyntaxTokenKind::Keyword)),
        "line 1 should contain 'fn' keyword: {tokens:?}"
    );

    // Line 2 should have a string
    let line2 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(2));
    assert_eq!(line2.line_ix, 1);
    let tokens = syntax::syntax_tokens_for_prepared_document_line(
        line2.document.unwrap().inner,
        line2.line_ix,
    );
    assert!(
        tokens
            .as_ref()
            .is_some_and(|t| t.iter().any(|tok| tok.kind == SyntaxTokenKind::String)),
        "line 2 should contain a string literal: {tokens:?}"
    );

    // Line 3 should be just punctuation (closing brace)
    let line3 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(3));
    assert_eq!(line3.line_ix, 2);
    let tokens = syntax::syntax_tokens_for_prepared_document_line(
        line3.document.unwrap().inner,
        line3.line_ix,
    );
    assert!(
        tokens.as_ref().is_some_and(|t| t
            .iter()
            .any(|tok| tok.kind == SyntaxTokenKind::PunctuationBracket)),
        "line 3 should contain punctuation bracket: {tokens:?}"
    );
}

#[test]
fn query_overlay_reuses_base_when_query_is_empty_or_missing() {
    let theme = AppTheme::gitcomet_dark();
    let text: SharedString = "abcdef".into();
    let mut text_hasher = FxHasher::default();
    text.as_ref().hash(&mut text_hasher);
    let text_hash = text_hasher.finish();
    let style = gpui::HighlightStyle {
        color: Some(theme.colors.foreground.primary.into_color()),
        ..Default::default()
    };
    let base = CachedDiffStyledText {
        text,
        highlights: Arc::from(vec![(0..6, style)]),
        highlights_hash: 42,
        text_hash,
    };

    let empty_query = query_overlay_for_test(theme, &base, "", Default::default());
    assert!(Arc::ptr_eq(&empty_query.highlights, &base.highlights));
    assert_eq!(empty_query.highlights_hash, base.highlights_hash);

    let missing_query = query_overlay_for_test(theme, &base, "xyz", Default::default());
    assert!(Arc::ptr_eq(&missing_query.highlights, &base.highlights));
    assert_eq!(missing_query.highlights_hash, base.highlights_hash);
}

#[test]
fn query_overlay_adds_background_without_losing_existing_color() {
    let theme = AppTheme::gitcomet_dark();
    let text: SharedString = "abcdef".into();
    let mut text_hasher = FxHasher::default();
    text.as_ref().hash(&mut text_hasher);
    let text_hash = text_hasher.finish();
    let style = gpui::HighlightStyle {
        color: Some(theme.colors.status.warning.foreground.into_color()),
        ..Default::default()
    };
    let base = CachedDiffStyledText {
        text,
        highlights: Arc::from(vec![(0..6, style)]),
        highlights_hash: 7,
        text_hash,
    };

    let overlaid = query_overlay_for_test(theme, &base, "cd", Default::default());
    assert_eq!(overlaid.highlights.len(), 3);
    assert_eq!(overlaid.highlights[1].0, 2..4);
    assert_eq!(
        overlaid.highlights[1].1.color,
        Some(theme.colors.status.warning.foreground.into_color())
    );
    assert!(overlaid.highlights[1].1.background_color.is_some());
    assert_ne!(overlaid.highlights_hash, base.highlights_hash);
}

#[test]
fn query_overlay_honors_search_options() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let base = build_cached_diff_styled_text(
        tab_width,
        theme,
        "Render render cat concat cat",
        &[],
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );

    let case_sensitive = query_overlay_for_test(
        theme,
        &base,
        "render",
        crate::view::panes::diff::diff_search::DiffSearchOptions {
            match_case: true,
            ..Default::default()
        },
    );
    let case_sensitive_ranges: Vec<_> = case_sensitive
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(case_sensitive_ranges, vec![7..13]);

    let whole_word = query_overlay_for_test(
        theme,
        &base,
        "cat",
        crate::view::panes::diff::diff_search::DiffSearchOptions {
            whole_word: true,
            ..Default::default()
        },
    );
    let whole_word_ranges: Vec<_> = whole_word
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(whole_word_ranges, vec![14..17, 25..28]);

    let regex = query_overlay_for_test(
        theme,
        &base,
        r"r.n.e.",
        crate::view::panes::diff::diff_search::DiffSearchOptions {
            regex: true,
            ..Default::default()
        },
    );
    let regex_ranges: Vec<_> = regex
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(regex_ranges, vec![0..6, 7..13]);
}

#[test]
fn query_overlay_skips_literal_multiline_row_fragments_without_stream_match_context() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();

    let first_base = build_cached_diff_styled_text(
        tab_width,
        theme,
        "foo",
        &[],
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    let first = query_overlay_for_test(theme, &first_base, "foo\nbar", Default::default());
    assert!(Arc::ptr_eq(&first.highlights, &first_base.highlights));
    assert_eq!(first.highlights_hash, first_base.highlights_hash);

    let second_base = build_cached_diff_styled_text(
        tab_width,
        theme,
        "bar",
        &[],
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    let second = query_overlay_for_test(theme, &second_base, "foo\nbar", Default::default());
    assert!(Arc::ptr_eq(&second.highlights, &second_base.highlights));
    assert_eq!(second.highlights_hash, second_base.highlights_hash);
}

#[test]
fn query_overlay_splits_across_base_and_query_boundaries_without_sorting() {
    let theme = AppTheme::gitcomet_dark();
    let text: SharedString = "abcdefghij".into();
    let mut text_hasher = FxHasher::default();
    text.as_ref().hash(&mut text_hasher);
    let text_hash = text_hasher.finish();
    let left = gpui::HighlightStyle {
        color: Some(theme.colors.status.warning.foreground.into_color()),
        ..Default::default()
    };
    let right = gpui::HighlightStyle {
        color: Some(theme.colors.status.success.foreground.into_color()),
        ..Default::default()
    };
    let base = CachedDiffStyledText {
        text,
        highlights: Arc::from(vec![(1..3, left), (5..8, right)]),
        highlights_hash: 11,
        text_hash,
    };

    let overlaid = query_overlay_for_test(theme, &base, "cdefg", Default::default());
    assert_eq!(overlaid.highlights.len(), 5);

    assert_eq!(overlaid.highlights[0], (1..2, left));
    assert_eq!(overlaid.highlights[1].0, 2..3);
    assert_eq!(
        overlaid.highlights[1].1.color,
        Some(theme.colors.status.warning.foreground.into_color())
    );
    assert!(overlaid.highlights[1].1.background_color.is_some());
    assert_eq!(
        overlaid.highlights[2],
        (
            3..5,
            gpui::HighlightStyle {
                // The theme's own search-match token, the same one the
                // non-overlay builder uses. It used to be mixed from the
                // accent here, which left `editor.search_match_background`
                // unread on dark themes and painted the two paths differently.
                background_color: Some(theme.colors.editor.search_match_background.into_color()),
                ..Default::default()
            }
        )
    );
    assert_eq!(overlaid.highlights[3].0, 5..7);
    assert_eq!(
        overlaid.highlights[3].1.color,
        Some(theme.colors.status.success.foreground.into_color())
    );
    assert!(overlaid.highlights[3].1.background_color.is_some());
    assert_eq!(overlaid.highlights[4], (7..8, right));
}

/// The row the match cursor is on has to be told apart from the others, or
/// stepping through matches in a read-only view shows the reader nothing.
/// It wears the same token the editable buffer's selection does.
#[test]
fn query_overlay_marks_the_current_match_with_the_selection_token() {
    let tab_width = 4;

    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        let base = build_cached_diff_styled_text(
            tab_width,
            theme,
            "alpha needle beta",
            &[],
            "",
            None,
            DiffSyntaxMode::Auto,
            None,
        );

        let other = query_overlay_for_test(theme, &base, "needle", Default::default());
        let current = query_overlay_for_test_with_emphasis(
            theme,
            &base,
            "needle",
            Default::default(),
            DiffSearchMatchEmphasis::Current,
        );

        let match_range = 6..12;
        let background = |styled: &CachedDiffStyledText| {
            styled
                .highlights
                .iter()
                .find(|(range, _)| *range == match_range)
                .and_then(|(_, style)| style.background_color)
        };

        assert_eq!(
            background(&other),
            Some(theme.colors.editor.search_match_background.into_color()),
            "every other match keeps the plain search wash"
        );
        assert_eq!(
            background(&current),
            Some(theme.colors.editor.selection_background.into_color()),
            "the current one wears the selection token, so it reads the same \
             as the editable buffer's selected match"
        );
        assert_ne!(
            background(&other),
            background(&current),
            "the two washes must actually differ, or stepping is invisible"
        );
        assert_ne!(
            other.highlights_hash, current.highlights_hash,
            "the hash gates the repaint, so it has to move with the wash"
        );
    }
}

#[test]
fn query_overlay_reuses_prebuilt_regex_matcher_for_multiple_rows() {
    let tab_width = 4;

    let theme = AppTheme::gitcomet_dark();
    let matcher = crate::view::panes::diff::diff_search::DiffSearchMatcher::new(
        r"r.n.e.",
        crate::view::panes::diff::diff_search::DiffSearchOptions {
            regex: true,
            ..Default::default()
        },
    );
    let first_base = build_cached_diff_styled_text(
        tab_width,
        theme,
        "Render first",
        &[],
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );
    let second_base = build_cached_diff_styled_text(
        tab_width,
        theme,
        "render second",
        &[],
        "",
        None,
        DiffSyntaxMode::Auto,
        None,
    );

    let first = build_cached_diff_query_overlay_styled_text(
        theme,
        &first_base,
        &matcher,
        DiffSearchMatchEmphasis::Other,
    );
    let second = build_cached_diff_query_overlay_styled_text(
        theme,
        &second_base,
        &matcher,
        DiffSearchMatchEmphasis::Other,
    );

    let first_ranges: Vec<_> = first
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    let second_ranges: Vec<_> = second
        .highlights
        .iter()
        .map(|(range, _)| range.clone())
        .collect();
    assert_eq!(first_ranges, vec![0..6]);
    assert_eq!(second_ranges, vec![0..6]);
}
