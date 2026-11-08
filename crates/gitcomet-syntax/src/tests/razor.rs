use super::*;

fn assert_fragment_kind(
    document: PreparedSyntaxDocument,
    text: &str,
    fragment: &str,
    kind: Option<SyntaxTokenKind>,
) {
    let start = text
        .find(fragment)
        .unwrap_or_else(|| panic!("missing {fragment:?}"));
    assert_range_kind(document, text, start..start + fragment.len(), kind);
}

fn assert_range_kind(
    document: PreparedSyntaxDocument,
    text: &str,
    range: Range<usize>,
    kind: Option<SyntaxTokenKind>,
) {
    for byte in range.clone() {
        if text.as_bytes()[byte] == b'\n' || text.as_bytes()[byte] == b'\r' {
            continue;
        }
        let line_ix = text.as_bytes()[..byte]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        let line_start = text.as_bytes()[..byte]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        let tokens = syntax_tokens_for_prepared_document_line(document, line_ix).unwrap();
        let actual = tokens
            .iter()
            .find(|token| token.range.contains(&(byte - line_start)))
            .map(|token| token.kind);
        assert_eq!(
            actual,
            kind,
            "{:?} at byte {byte}, tokens {tokens:?}",
            &text[range.clone()]
        );
    }
}

#[test]
fn razor_paths_and_fences_preserve_companion_languages() {
    for path in [
        "Index.cshtml",
        "Views/Index.CSHTML",
        "Pages/Counter.razor",
        "Counter.RAZOR",
    ] {
        assert_eq!(
            diff_syntax_language_for_path(std::path::Path::new(path)),
            Some(DiffSyntaxLanguage::Razor),
            "{path}"
        );
    }
    for info in [
        "razor",
        "cshtml",
        "RAZOR title=demo",
        "{.cshtml}",
        "language-razor",
    ] {
        assert_eq!(
            diff_syntax_language_for_code_fence_info(info),
            Some(DiffSyntaxLanguage::Razor),
            "{info}"
        );
    }
    for (path, expected) in [
        ("Counter.razor.cs", Some(DiffSyntaxLanguage::CSharp)),
        ("Counter.razor.css", Some(DiffSyntaxLanguage::Css)),
        ("Index.vbhtml", None),
    ] {
        assert_eq!(
            diff_syntax_language_for_path(std::path::Path::new(path)),
            expected,
            "{path}"
        );
    }
}

#[test]
fn razor_combined_html_excludes_csharp_and_recaptures_nested_elements() {
    let text = "<div class=\"prefix @Model.Name suffix\">@if(ok){<span>@Model.Value</span>}</div>";
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Razor).unwrap();
    let tree =
        with_ts_parser_parse_result(&spec.ts_language, |parser| parser.parse(text, None)).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let matches = collect_treesitter_injection_matches_for_line_window_at(
        &tree,
        spec,
        text.as_bytes(),
        &[0],
        0,
        1,
        0,
        0,
    );
    assert!(matches.singles.is_empty());
    assert!(!matches.truncated);
    assert_eq!(matches.combined.len(), 1);
    let group = &matches.combined[0];
    assert_eq!(group.language, DiffSyntaxLanguage::Html);
    assert_eq!(
        group
            .ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<String>(),
        "<div class=\"prefix  suffix\"><span></span></div>"
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Razor, text);
    use SyntaxTokenKind as K;
    for (fragment, kind) in [
        ("div", K::Tag),
        ("span", K::Tag),
        ("class", K::Attribute),
        ("prefix ", K::String),
        (" suffix", K::String),
        ("Model", K::Variable),
        ("Name", K::Variable),
        ("Value", K::Variable),
        ("@if", K::Keyword),
    ] {
        assert_fragment_kind(document, text, fragment, Some(kind));
    }
}

#[test]
fn razor_implicit_chains_stop_before_markup_punctuation() {
    let text =
        "<p>@optional?.Items?[0].Name / @value - 1 / @value. / @value (literal) / @value [0]</p>";
    let document = prepare_test_document(DiffSyntaxLanguage::Razor, text);
    use SyntaxTokenKind as K;
    for (fragment, kind) in [
        ("optional", K::Variable),
        ("Items", K::Variable),
        ("Name", K::Variable),
        ("0", K::Number),
    ] {
        assert_fragment_kind(document, text, fragment, Some(kind));
    }
    for fragment in [" - 1", " (literal)", " [0]"] {
        assert_fragment_kind(document, text, fragment, None);
    }
    let dot = text.find("@value.").unwrap() + "@value".len();
    assert_range_kind(document, text, dot..dot + 1, None);
}

#[test]
fn razor_fallback_keeps_comment_like_prose_and_apostrophes_plain() {
    let text = "<p>It's prose // and /* still prose */ for each class</p>";
    let tokens = heuristic_tokens(text, DiffSyntaxLanguage::Razor);
    assert!(
        !tokens.iter().any(|token| matches!(
            token.kind,
            SyntaxTokenKind::Comment | SyntaxTokenKind::String | SyntaxTokenKind::Keyword
        )),
        "{tokens:?}"
    );
}

#[test]
fn razor_many_islands_share_one_html_parse_and_preserve_window_fallback() {
    reset_prepared_syntax_cache();
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
    TS_COMBINED_LAYER_PARSE_COUNT.with(|count| count.set(0));
    let text = (0..200)
        .map(|ix| format!("<p data-index=\"{ix}\">@Model.Name <b>@Model.Value</b></p>\n"))
        .collect::<String>();
    let document = prepare_test_document_from_shared_text(DiffSyntaxLanguage::Razor, &text);
    for line_ix in [0, 100, 199] {
        let tokens = syntax_tokens_for_prepared_document_line(document, line_ix).unwrap();
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Tag)
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Variable)
        );
    }
    assert_eq!(TS_COMBINED_LAYER_PARSE_COUNT.with(|count| count.get()), 1);
    assert!(TS_INJECTION_CACHE.with(|cache| cache.borrow().is_empty()));
    let state = prepared_document_tree_state(document).unwrap();
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Razor).unwrap();
    assert!(matches!(
        build_prepared_combined_layers(
            spec,
            &state.tree,
            text.as_bytes(),
            &state.line_starts,
            state.source_hash,
            None,
            8
        ),
        Some(None)
    ));
    let window = collect_treesitter_document_line_tokens_for_line_window_with_combined_layers(
        &state.tree,
        spec,
        text.as_bytes(),
        &state.line_starts,
        96,
        128,
        state.source_hash,
        None,
    );
    for (offset, tokens) in window.iter().enumerate() {
        assert_eq!(
            tokens.as_slice(),
            syntax_tokens_for_prepared_document_line(document, 96 + offset)
                .unwrap()
                .as_slice()
        );
    }
}

#[test]
fn razor_prepared_reparse_restores_markup_after_an_expression_edit() {
    reset_prepared_syntax_cache();
    let before = "<div>\n<p title=\"@Model.Name\">@(1 + 2)</p>\n</div>";
    let after = before.replace("@Model.Name", "plain");
    let base = prepare_test_document(DiffSyntaxLanguage::Razor, before);
    let PrepareTreesitterDocumentResult::Ready(edited) = prepare_test_document_with_budget_reuse(
        DiffSyntaxLanguage::Razor,
        &after,
        TEST_PARSE_BUDGET,
        Some(base),
    ) else {
        panic!("Razor edit should parse");
    };
    let state = prepared_document_tree_state(edited).unwrap();
    assert_eq!(state.parse_mode, TreesitterParseReuseMode::Incremental);
    let cold = inject_prepared_document_data(
        prepare_test_document_in_background(DiffSyntaxLanguage::Razor, &after).unwrap(),
    );
    for line_ix in 0..3 {
        assert_eq!(
            syntax_tokens_for_prepared_document_line(edited, line_ix),
            syntax_tokens_for_prepared_document_line(cold, line_ix)
        );
    }
    assert_fragment_kind(edited, &after, "plain", Some(SyntaxTokenKind::String));
}

#[test]
fn razor_zero_budget_defers_and_background_rebuild_preserves_language_ownership() {
    reset_prepared_syntax_cache();
    let text = "<p title=\"before @Model.Name after\">@Model?.Title</p>";
    assert_eq!(
        prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Razor,
            text,
            DiffSyntaxBudget {
                foreground_parse: Duration::ZERO
            },
            None
        ),
        PrepareTreesitterDocumentResult::TimedOut
    );
    let document = inject_prepared_document_data(
        prepare_test_document_in_background(DiffSyntaxLanguage::Razor, text).unwrap(),
    );
    assert_fragment_kind(document, text, "before ", Some(SyntaxTokenKind::String));
    assert_fragment_kind(document, text, "Name", Some(SyntaxTokenKind::Variable));
    assert_fragment_kind(document, text, "Title", Some(SyntaxTokenKind::Variable));
}

#[test]
fn razor_empty_blocks_and_raw_elements_end_cleanly_at_eof() {
    for (text, fragment, kind) in [
        ("@page", "@page", SyntaxTokenKind::Keyword),
        ("@{}", "{}", SyntaxTokenKind::PunctuationBracket),
        ("@code {}", "@code", SyntaxTokenKind::Keyword),
        (
            "<script></script><style></style>",
            "script",
            SyntaxTokenKind::Tag,
        ),
    ] {
        let document = prepare_test_document(DiffSyntaxLanguage::Razor, text);
        let state = prepared_document_tree_state(document).unwrap();
        assert!(
            !state.tree.root_node().has_error(),
            "{text:?}: {}",
            state.tree.root_node().to_sexp()
        );
        assert_fragment_kind(document, text, fragment, Some(kind));
    }
}
