pub(super) use super::*;

#[test]
fn prepared_syntax_pair_is_none_outside_the_document() {
    let text = "fn main() {}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    assert_eq!(
        prepared_document_syntax_pair_at_display_offset(document, 99, 0),
        None,
        "a line past the end has no answer"
    );
    assert_eq!(
        prepared_document_syntax_pair_at_display_offset(document, 0, 0),
        None,
        "the caret before `fn` is inside nothing"
    );
}

#[test]
fn treesitter_line_length_guard() {
    assert!(super::should_use_treesitter_for_line("fn main() {}"));
    assert!(!super::should_use_treesitter_for_line(
        &"a".repeat(MAX_TREESITTER_LINE_BYTES + 1)
    ));
}

#[test]
fn treesitter_query_cursor_sets_match_limit_for_line_queries() {
    let _ = syntax_tokens_for_line(
        "fn main() { let value = Some(1); }",
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
    );
    TS_CURSOR.with(|cursor| {
        assert_eq!(cursor.borrow().match_limit(), TS_QUERY_MATCH_LIMIT);
    });
}

#[test]
fn large_document_query_passes_are_chunked_to_bounded_windows() {
    let lines = vec!["let value = 1;"; 8_192];
    let input = treesitter_document_input_from_text(&lines.join("\n"));
    let passes = treesitter_document_query_passes_for_line_window(
        input.line_starts.as_ref(),
        input.text.len(),
        0,
        input.line_starts.len(),
    );
    assert!(
        passes.len() > 1,
        "large document should be processed in multiple query passes"
    );
    assert!(passes.iter().all(|pass| {
        pass.byte_range.end.saturating_sub(pass.byte_range.start) <= TS_MAX_BYTES_TO_QUERY
    }));
}

#[test]
fn pathological_long_line_uses_containing_ranges_for_subpasses() {
    let long_line = format!("let value = {};", "x".repeat(TS_MAX_BYTES_TO_QUERY * 4));
    let input = treesitter_document_input_from_text(&long_line);
    let passes = treesitter_document_query_passes_for_line_window(
        input.line_starts.as_ref(),
        input.text.len(),
        0,
        input.line_starts.len(),
    );

    assert!(
        passes.len() >= 4,
        "long line should be split into multiple bounded query passes"
    );
    assert!(
        passes
            .iter()
            .all(|pass| pass.containing_byte_range.is_some()),
        "pathological line subpasses should use containing byte ranges"
    );
}

#[test]
fn streamed_ascii_json_slice_keeps_string_state_after_checkpoint() {
    const CHECKPOINT_SPACING: usize = 32 * 1024;
    reset_streamed_heuristic_line_cache();

    let payload = "x".repeat(CHECKPOINT_SPACING * 2);
    let text = format!(r#"{{"payload":"{payload}","tail":true}}"#);
    let payload_start = text.find(&payload).expect("payload should be present");
    let slice_start = payload_start + CHECKPOINT_SPACING + 137;
    let slice_end = slice_start + 256;
    let raw_text = gitcomet_core::file_diff::FileDiffLineText::shared(Arc::from(text));
    let (slice_text, resolved_range) = raw_text
        .slice_text_resolved(slice_start..slice_end)
        .expect("ASCII streamed slice should resolve");

    let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
        &raw_text,
        DiffSyntaxLanguage::Json,
        slice_start..slice_end,
        resolved_range,
    )
    .expect("ASCII streamed slice should be supported");
    assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

    assert!(
        tokens.iter().any(|token| {
            token.kind == SyntaxTokenKind::String && token.range.start == 0 && token.range.end > 64
        }),
        "slice that starts inside the payload string should keep string highlighting: {tokens:?}"
    );
}

#[test]
fn streamed_ascii_block_comment_slice_keeps_comment_state_and_tail_tokens() {
    const CHECKPOINT_SPACING: usize = 32 * 1024;
    reset_streamed_heuristic_line_cache();

    let comment = "x".repeat(CHECKPOINT_SPACING + 192);
    let text = format!("/*{comment}*/ let value = 1;");
    let comment_start = text.find(&comment).expect("comment body should be present");
    let comment_end = comment_start + comment.len();
    let slice_start = comment_start + CHECKPOINT_SPACING;
    let slice_end = text.len();
    let raw_text = gitcomet_core::file_diff::FileDiffLineText::shared(Arc::from(text));
    let (slice_text, resolved_range) = raw_text
        .slice_text_resolved(slice_start..slice_end)
        .expect("ASCII streamed slice should resolve");

    let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
        &raw_text,
        DiffSyntaxLanguage::Rust,
        slice_start..slice_end,
        resolved_range,
    )
    .expect("ASCII streamed slice should be supported");
    assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

    let comment_tail_len = comment_end.saturating_add(2).saturating_sub(slice_start);
    assert!(
        tokens.iter().any(|token| {
            token.kind == SyntaxTokenKind::Comment
                && token.range.start == 0
                && token.range.end >= comment_tail_len
        }),
        "slice should preserve the continued block comment: {tokens:?}"
    );
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "tail after the closing comment should still tokenize normally: {tokens:?}"
    );
}

#[test]
fn streamed_utf8_file_backed_json_slice_keeps_string_state_after_checkpoint() {
    const CHECKPOINT_SPACING: usize = 32 * 1024;
    reset_streamed_heuristic_line_cache();

    let payload = "x".repeat(CHECKPOINT_SPACING * 2);
    let text = format!(r#"{{"title":"Ä","payload":"{payload}","tail":true}}"#);
    let payload_start = text.find(&payload).expect("payload should be present");
    let slice_start = payload_start + CHECKPOINT_SPACING + 137;
    let slice_end = slice_start + 256;
    let fixture = TempFileBackedLineFixture::new("streamed_utf8_json_slice.json", &text);
    let (slice_text, resolved_range) = fixture
        .raw_text
        .slice_text_resolved(slice_start..slice_end)
        .expect("UTF-8 streamed slice should resolve");

    let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
        &fixture.raw_text,
        DiffSyntaxLanguage::Json,
        slice_start..slice_end,
        resolved_range,
    )
    .expect("UTF-8 streamed slice should be supported");

    assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);
    assert!(
        tokens.iter().any(|token| {
            token.kind == SyntaxTokenKind::String && token.range.start == 0 && token.range.end > 64
        }),
        "UTF-8 file-backed slice that starts inside the payload string should keep string highlighting: {tokens:?}"
    );
}

#[test]
fn streamed_utf8_file_backed_block_comment_slice_keeps_comment_state_and_tail_tokens() {
    const CHECKPOINT_SPACING: usize = 32 * 1024;
    reset_streamed_heuristic_line_cache();

    let comment = "x".repeat(CHECKPOINT_SPACING + 192);
    let text = format!(r#"let title = "Ä"; /*{comment}*/ let value = 1;"#);
    let comment_start = text.find(&comment).expect("comment body should be present");
    let comment_end = comment_start + comment.len();
    let slice_start = comment_start + CHECKPOINT_SPACING;
    let slice_end = text.len();
    let fixture = TempFileBackedLineFixture::new("streamed_utf8_comment_slice.rs", &text);
    let (slice_text, resolved_range) = fixture
        .raw_text
        .slice_text_resolved(slice_start..slice_end)
        .expect("UTF-8 streamed slice should resolve");

    let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
        &fixture.raw_text,
        DiffSyntaxLanguage::Rust,
        slice_start..slice_end,
        resolved_range.clone(),
    )
    .expect("UTF-8 streamed slice should be supported");

    assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

    let comment_tail_len = comment_end
        .saturating_add(2)
        .saturating_sub(resolved_range.start);
    assert!(
        tokens.iter().any(|token| {
            token.kind == SyntaxTokenKind::Comment
                && token.range.start == 0
                && token.range.end >= comment_tail_len
        }),
        "UTF-8 file-backed slice should preserve the continued block comment: {tokens:?}"
    );
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "tail after the closing comment should still tokenize normally: {tokens:?}"
    );
}

#[test]
fn xml_has_own_language_variant() {
    assert_eq!(
        diff_syntax_language_for_path("foo.xml"),
        Some(DiffSyntaxLanguage::Xml)
    );
    assert_eq!(
        diff_syntax_language_for_path("layout.svg"),
        Some(DiffSyntaxLanguage::Xml)
    );
    // HTML stays separate
    assert_eq!(
        diff_syntax_language_for_path("index.html"),
        Some(DiffSyntaxLanguage::Html)
    );
}

#[test]
fn js_and_jsx_use_distinct_language_variants() {
    assert_eq!(
        diff_syntax_language_for_path("main.js"),
        Some(DiffSyntaxLanguage::JavaScript)
    );
    assert_eq!(
        diff_syntax_language_for_path("main.jsx"),
        Some(DiffSyntaxLanguage::Tsx)
    );
    assert_eq!(
        diff_syntax_language_for_path("main.tsx"),
        Some(DiffSyntaxLanguage::Tsx)
    );
}

#[test]
fn vue_extension_is_supported() {
    assert_eq!(
        diff_syntax_language_for_path("src/components/App.vue"),
        Some(DiffSyntaxLanguage::Vue)
    );
    // The same alias table backs injections and fenced code info strings.
    assert_eq!(
        diff_syntax_language_for_code_fence_info("vue"),
        Some(DiffSyntaxLanguage::Vue)
    );
}

#[test]
fn sql_extension_is_supported() {
    assert_eq!(
        diff_syntax_language_for_path("query.sql"),
        Some(DiffSyntaxLanguage::Sql)
    );
}

#[test]
fn markdown_extension_is_supported() {
    assert_eq!(
        diff_syntax_language_for_path("README.md"),
        Some(DiffSyntaxLanguage::Markdown)
    );
    assert_eq!(
        diff_syntax_language_for_path("notes.markdown"),
        Some(DiffSyntaxLanguage::Markdown)
    );
}

#[test]
fn extended_path_aliases_are_supported() {
    assert_eq!(
        diff_syntax_language_for_path(".bashrc"),
        Some(DiffSyntaxLanguage::Bash)
    );
    assert_eq!(
        diff_syntax_language_for_path("PKGBUILD"),
        Some(DiffSyntaxLanguage::Bash)
    );
    assert_eq!(
        diff_syntax_language_for_path("module.cppm"),
        Some(DiffSyntaxLanguage::Cpp)
    );
    assert_eq!(
        diff_syntax_language_for_path("legacy.C"),
        Some(DiffSyntaxLanguage::Cpp)
    );
    assert_eq!(
        diff_syntax_language_for_path("legacy.H"),
        Some(DiffSyntaxLanguage::Cpp)
    );
    assert_eq!(
        diff_syntax_language_for_path("plain.c"),
        Some(DiffSyntaxLanguage::C)
    );
    assert_eq!(
        diff_syntax_language_for_path("sketch.ino"),
        Some(DiffSyntaxLanguage::Cpp)
    );
    assert_eq!(
        diff_syntax_language_for_path("styles.pcss"),
        Some(DiffSyntaxLanguage::Css)
    );
    assert_eq!(
        diff_syntax_language_for_path("types.pyi"),
        Some(DiffSyntaxLanguage::Python)
    );
    assert_eq!(
        diff_syntax_language_for_path("config.jsonc"),
        Some(DiffSyntaxLanguage::Json)
    );
    assert_eq!(
        diff_syntax_language_for_path(".prettierrc"),
        Some(DiffSyntaxLanguage::Json)
    );
    assert_eq!(
        diff_syntax_language_for_path(".clang-format"),
        Some(DiffSyntaxLanguage::Yaml)
    );
    assert_eq!(
        diff_syntax_language_for_path("README.mdx"),
        Some(DiffSyntaxLanguage::Markdown)
    );
    assert_eq!(
        diff_syntax_language_for_path("script.ps1"),
        Some(DiffSyntaxLanguage::PowerShell)
    );
    assert_eq!(
        diff_syntax_language_for_path("main.swift"),
        Some(DiffSyntaxLanguage::Swift)
    );
    assert_eq!(
        diff_syntax_language_for_path("analysis.R"),
        Some(DiffSyntaxLanguage::R)
    );
    assert_eq!(
        diff_syntax_language_for_path("app.dart"),
        Some(DiffSyntaxLanguage::Dart)
    );
    assert_eq!(
        diff_syntax_language_for_path("build.sbt"),
        Some(DiffSyntaxLanguage::Scala)
    );
    assert_eq!(
        diff_syntax_language_for_path("module.pm"),
        Some(DiffSyntaxLanguage::Perl)
    );
    assert_eq!(
        diff_syntax_language_for_path("main.m"),
        Some(DiffSyntaxLanguage::ObjectiveC)
    );
    assert_eq!(
        diff_syntax_language_for_path("changes.patch"),
        Some(DiffSyntaxLanguage::Diff)
    );
    assert_eq!(
        diff_syntax_language_for_path("COMMIT_EDITMSG"),
        Some(DiffSyntaxLanguage::GitCommit)
    );
    assert_eq!(
        diff_syntax_language_for_path("go.mod"),
        Some(DiffSyntaxLanguage::GoMod)
    );
    assert_eq!(
        diff_syntax_language_for_path("go.work"),
        Some(DiffSyntaxLanguage::GoWork)
    );
}

#[test]
fn fenced_code_info_aliases_are_supported() {
    assert_eq!(
        diff_syntax_language_for_code_fence_info("rust"),
        Some(DiffSyntaxLanguage::Rust)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("language-typescript title=\"main.ts\""),
        Some(DiffSyntaxLanguage::TypeScript)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("{.shell}"),
        Some(DiffSyntaxLanguage::Bash)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("jsonc"),
        Some(DiffSyntaxLanguage::Json)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("shellscript"),
        Some(DiffSyntaxLanguage::Bash)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("pwsh"),
        Some(DiffSyntaxLanguage::PowerShell)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("ps1"),
        Some(DiffSyntaxLanguage::PowerShell)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("objective-c"),
        Some(DiffSyntaxLanguage::ObjectiveC)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("go.mod"),
        Some(DiffSyntaxLanguage::GoMod)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("go.work"),
        Some(DiffSyntaxLanguage::GoWork)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("diff"),
        Some(DiffSyntaxLanguage::Diff)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("regex"),
        Some(DiffSyntaxLanguage::Regex)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("jsdoc"),
        Some(DiffSyntaxLanguage::Jsdoc)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("foo/bar/baz.rb"),
        Some(DiffSyntaxLanguage::Ruby)
    );
    assert_eq!(
        diff_syntax_language_for_code_fence_info("src/components/button.tsx"),
        Some(DiffSyntaxLanguage::Tsx)
    );
}

#[test]
fn markdown_heading_and_inline_code_are_highlighted() {
    let heading = syntax_tokens_for_line(
        "# Hello world",
        DiffSyntaxLanguage::Markdown,
        DiffSyntaxMode::Auto,
    );
    assert!(
        heading.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "expected markdown heading to be highlighted"
    );

    let inline = syntax_tokens_for_line(
        "Use `git status` here",
        DiffSyntaxLanguage::Markdown,
        DiffSyntaxMode::Auto,
    );
    assert!(
        inline.iter().any(|t| t.kind == SyntaxTokenKind::String),
        "expected markdown inline code to be highlighted"
    );
}

#[test]
fn markdown_inline_code_handles_unterminated_and_multibyte_spans_without_invalid_ranges() {
    for text in [
        "Use `cafe` here",
        "Use `café` here",
        "Use ``naïve `code` span`` here",
        "emoji `😀` end",
        "unterminated `😀",
        "`",
        "````",
        "prefix ``😀`` suffix",
    ] {
        let tokens = syntax_tokens_for_line_markdown(text);
        assert_token_ranges_are_utf8_safe(text, &tokens);
    }
}

#[test]
fn treesitter_variable_capture_maps_but_gets_no_color() {
    // `@variable` now maps to `Variable` (tracked but rendered without color)
    // so the capture info is preserved for potential theme use.
    assert_eq!(
        super::syntax_kind_from_capture_name("variable"),
        Some(SyntaxTokenKind::Variable)
    );
    // `@variable.parameter` maps to its own distinct kind
    assert_eq!(
        super::syntax_kind_from_capture_name("variable.parameter"),
        Some(SyntaxTokenKind::VariableParameter)
    );
}

#[test]
fn treesitter_tokenization_is_safe_across_languages() {
    let rust_line = "fn main() { let x = 1; }";
    let json_line = "{\"x\": 1}";

    let rust = syntax_tokens_for_line(rust_line, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    let json = syntax_tokens_for_line(json_line, DiffSyntaxLanguage::Json, DiffSyntaxMode::Auto);

    for t in rust {
        assert!(t.range.start <= t.range.end);
        assert!(t.range.end <= rust_line.len());
    }
    for t in json {
        assert!(t.range.start <= t.range.end);
        assert!(t.range.end <= json_line.len());
    }
}

#[test]
fn json_string_value_with_underscores_stays_one_string_token() {
    let line = r#"  "transition_policy": "adjacent_and_first","#;
    let key_start = line
        .find(r#""transition_policy""#)
        .expect("fixture should contain JSON key");
    let key_end = key_start + r#""transition_policy""#.len();
    let value_start = line
        .find(r#""adjacent_and_first""#)
        .expect("fixture should contain JSON string value");
    let value_end = value_start + r#""adjacent_and_first""#.len();

    let tokens = syntax_tokens_for_line(line, DiffSyntaxLanguage::Json, DiffSyntaxMode::Auto);

    assert!(
        tokens.iter().any(|token| {
            token.range == (key_start..key_end) && token.kind == SyntaxTokenKind::Property
        }),
        "JSON key should be highlighted as one property token: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|token| {
            token.range == (value_start..value_end) && token.kind == SyntaxTokenKind::String
        }),
        "JSON value should be highlighted as one string token: {tokens:?}"
    );
    assert!(
        !tokens.iter().any(|token| {
            token.range.start < key_end
                && key_start < token.range.end
                && token.kind != SyntaxTokenKind::Property
        }),
        "no non-property token should overlap the JSON key: {tokens:?}"
    );
    assert!(
        !tokens.iter().any(|token| {
            token.range.start < value_end
                && value_start < token.range.end
                && token.kind != SyntaxTokenKind::String
        }),
        "no non-string token should overlap the JSON value: {tokens:?}"
    );
}

#[test]
fn treesitter_line_fallback_survives_incomplete_fragments() {
    let cases = [
        (
            DiffSyntaxLanguage::Rust,
            vec![
                "pub struct Example<'a",
                "let value = Some(\"unterminated",
                "match value { Some(inner) => inner.",
            ],
        ),
        (
            DiffSyntaxLanguage::JavaScript,
            vec![
                "const element = document.querySelector(\".demo",
                "return values.map((entry) => entry.",
                "class Example extends React.Component {",
            ],
        ),
        (
            DiffSyntaxLanguage::TypeScript,
            vec![
                "const value: Promise<Result<string, Error>> =",
                "type Example<T extends Record<string, number>",
            ],
        ),
        (
            DiffSyntaxLanguage::Html,
            vec![
                "<button onclick=\"const value = 1;",
                "<div style=\"color: red;",
                "<input class=\"demo\"",
            ],
        ),
        (
            DiffSyntaxLanguage::Xml,
            vec![
                "<root attr=\"shared",
                "<?xml-stylesheet href=\"theme.css",
                "<item key=\"value\"",
            ],
        ),
    ];

    for (language, fragments) in cases {
        for fragment in fragments {
            let _ = syntax_tokens_for_line(fragment, language, DiffSyntaxMode::Auto);
            for trim in 0..=8usize {
                if trim > fragment.len()
                    || !fragment.is_char_boundary(fragment.len().saturating_sub(trim))
                {
                    continue;
                }
                let shortened = &fragment[..fragment.len().saturating_sub(trim)];
                let result = std::panic::catch_unwind(|| {
                    syntax_tokens_for_line(shortened, language, DiffSyntaxMode::Auto)
                });
                assert!(
                    result.is_ok(),
                    "single-line tree-sitter fallback should not panic for {language:?} fragment {shortened:?}"
                );
            }
        }
    }
}

#[test]
fn document_collector_reports_host_query_failure_after_collecting_injections() {
    let ts_language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = tree_sitter::Query::new(
        &ts_language,
        r#"
        ((identifier) @variable
          (#eq? @variable "host_failure"))
        "#,
    )
    .expect("test highlight query should compile");
    let capture_kinds = query
        .capture_names()
        .iter()
        .map(|name| syntax_kind_from_capture_name(name))
        .collect();
    let injection_query = tree_sitter::Query::new(
        &ts_language,
        r#"
        ((string_content) @injection.content
          (#set! injection.language "rust"))
        "#,
    )
    .expect("test injection query should compile");
    let spec = TreesitterHighlightSpec {
        ts_language: ts_language.clone(),
        query,
        capture_kinds,
        injection_query: Some(injection_query),
        injection_combined_patterns: vec![false],
        has_combined_injections: false,
    };

    let parsed_input = br#"const SCRIPT: &str = "fn injected() {}"; host_failure"#;
    let host_start = parsed_input
        .windows(b"host_failure".len())
        .position(|window| window == b"host_failure")
        .expect("host identifier should be present");
    // Model the upstream recovered-node bug: the tree's identifier extends
    // beyond the text provider, so evaluating its #eq? predicate panics. The
    // earlier injection remains wholly inside the available input.
    let input = &parsed_input[..host_start + "host".len()];
    let tree = with_ts_parser_parse_result(&ts_language, |parser| parser.parse(parsed_input, None))
        .expect("test input should parse");

    let result = with_silenced_panic_hook(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            collect_treesitter_document_line_tokens_for_line_window_with_host_query_status(
                &tree,
                &spec,
                input,
                &[0],
                0,
                1,
                treesitter_document_hash(DiffSyntaxLanguage::Rust, "collector regression"),
            )
        }))
    });
    let (per_line, host_query_succeeded) =
        result.expect("the collector should recover the query panic");

    assert!(
        !host_query_succeeded,
        "the caller needs the host failure signal to select its heuristic fallback"
    );
    assert!(
        per_line[0].iter().any(|token| {
            token.kind == SyntaxTokenKind::Keyword && &input[token.range.clone()] == b"fn"
        }),
        "injections should still be collected after the host query fails: {:?}",
        per_line[0]
    );
}

#[test]
fn parser_fast_path_reuses_same_language_until_switch() {
    reset_ts_parser_test_state();

    let rust_tokens =
        syntax_tokens_for_line_treesitter("fn main() { let x = 1; }", DiffSyntaxLanguage::Rust)
            .expect("first rust parse should succeed");
    assert!(!rust_tokens.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 1);

    let rust_tokens_again =
        syntax_tokens_for_line_treesitter("fn helper() { let y = 2; }", DiffSyntaxLanguage::Rust)
            .expect("second rust parse should succeed");
    assert!(!rust_tokens_again.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 1);

    let json_tokens = syntax_tokens_for_line_treesitter("{\"x\": 1}", DiffSyntaxLanguage::Json)
        .expect("json parse should succeed");
    assert!(!json_tokens.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 2);

    let json_tokens_again =
        syntax_tokens_for_line_treesitter("{\"y\": 2}", DiffSyntaxLanguage::Json)
            .expect("second json parse should succeed");
    assert!(!json_tokens_again.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 2);
}

#[test]
fn parser_fast_path_reconfigures_after_recovered_query_panic() {
    reset_ts_parser_test_state();

    let baseline =
        syntax_tokens_for_line_treesitter("fn main() { let x = 1; }", DiffSyntaxLanguage::Rust)
            .expect("baseline rust parse should succeed");
    assert!(!baseline.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 1);

    let recovered: Option<()> = with_silenced_panic_hook(|| {
        catch_treesitter_query_panic(|| panic!("simulate query panic"))
    });
    assert!(recovered.is_none());

    let reparsed =
        syntax_tokens_for_line_treesitter("fn main() { let y = 2; }", DiffSyntaxLanguage::Rust)
            .expect("rust parse after panic recovery should succeed");
    assert!(
        reparsed
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "rust parse after panic recovery should still contain keyword highlights: {reparsed:?}"
    );
    assert_eq!(ts_parser_set_language_call_count(), 2);
}

#[test]
fn parser_fast_path_reconfigures_after_interrupted_parse() {
    reset_ts_parser_test_state();

    let baseline =
        syntax_tokens_for_line_treesitter("fn main() { let x = 1; }", DiffSyntaxLanguage::Rust)
            .expect("baseline rust parse should succeed");
    assert!(!baseline.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 1);

    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Rust)
        .expect("Rust highlight spec should exist");
    let interrupted_input = "fn main() { let value = Some(42); }\n".repeat(4_096);
    let interrupted = with_ts_parser_parse_result(&spec.ts_language, |parser| {
        parse_treesitter_tree(
            parser,
            interrupted_input.as_bytes(),
            None,
            Some(Duration::ZERO),
        )
    });
    assert!(
        interrupted.is_none(),
        "zero-budget parse should interrupt before producing a tree"
    );

    let reparsed =
        syntax_tokens_for_line_treesitter("fn helper() { let y = 2; }", DiffSyntaxLanguage::Rust)
            .expect("rust parse after interrupted parse should succeed");
    assert!(
        reparsed
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "rust parse after interrupted parse should still contain keyword highlights: {reparsed:?}"
    );
    assert_eq!(ts_parser_set_language_call_count(), 2);
}

#[test]
fn parser_fast_path_reconfigures_when_parser_slot_loses_language() {
    reset_ts_parser_test_state();

    let first =
        syntax_tokens_for_line_treesitter("fn main() { let x = 1; }", DiffSyntaxLanguage::Rust)
            .expect("baseline rust parse should succeed");
    assert!(!first.is_empty());
    assert_eq!(ts_parser_set_language_call_count(), 1);

    TS_PARSER.with(|parser| {
        *parser.borrow_mut() = tree_sitter::Parser::new();
    });

    let reparsed =
        syntax_tokens_for_line_treesitter("fn helper() { let y = 2; }", DiffSyntaxLanguage::Rust)
            .expect("rust parse should recover after parser slot reset");
    assert!(
        reparsed
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "rust parse after parser slot reset should still contain keyword highlights: {reparsed:?}"
    );
    assert_eq!(ts_parser_set_language_call_count(), 2);
}

#[test]
fn single_line_syntax_cache_isolated_by_mode_for_xml_markup() {
    reset_ts_parser_test_state();

    let text = r#"<item enabled="true">value</item>"#;
    let auto = syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
    assert!(
        auto.iter().any(|token| {
            matches!(
                token.kind,
                SyntaxTokenKind::Tag | SyntaxTokenKind::Attribute
            )
        }),
        "tree-sitter XML mode should classify markup tokens: {auto:?}"
    );

    let heuristic =
        syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::HeuristicOnly);
    assert!(
        !heuristic.iter().any(|token| {
            matches!(
                token.kind,
                SyntaxTokenKind::Tag | SyntaxTokenKind::Attribute
            )
        }),
        "heuristic XML mode should not reuse tree-sitter markup tokens: {heuristic:?}"
    );

    let auto_again = syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
    assert_eq!(auto_again, auto);
}

// ---- Heuristic fallback: Nix and Jinja ------------------------------------

/// Treating `'` as a quote painted the rest of the line as a string from the
/// tick in `foldl'` onward. HeuristicOnly is a production path for large diffs,
/// not just a fallback.
#[test]
fn nix_apostrophe_identifiers_do_not_open_a_string() {
    for line in [
        "  x = lib.foldl' add 0 xs;",
        "  y = builtins.mapAttrs' (n: v: v) set;",
        "  inherit (lib) foldl' concatMapAttrs';",
    ] {
        assert!(
            heuristic_string_spans(line, DiffSyntaxLanguage::Nix).is_empty(),
            "an apostrophe identifier opened a string in {line:?}: {:?}",
            heuristic_tokens(line, DiffSyntaxLanguage::Nix)
        );
    }

    // The tick is part of the identifier, so a keyword check sees the whole
    // name rather than a truncated prefix.
    let tokens = heuristic_tokens("  inherit' = 1;", DiffSyntaxLanguage::Nix);
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "`inherit'` is not the keyword `inherit`: {tokens:?}"
    );

    // Double-quoted strings are untouched by any of this.
    assert_eq!(
        heuristic_string_spans("  z = \"literal\";", DiffSyntaxLanguage::Nix),
        vec!["\"literal\""]
    );
}

/// The reason the Nix arm exists instead of reusing the Hcl one: `//` is Nix's
/// update operator, so Hcl's `//` line comment would grey out the rest of the
/// line. Nothing else guards it -- every other Nix test takes the tree-sitter
/// path -- so folding Nix back into the `Hcl | Php` arm would pass the suite.
#[test]
fn nix_update_operator_is_not_a_line_comment() {
    let line = "  merged = { a = 1; } // { b = 2; };";
    let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Nix);
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "the `//` update operator was greyed out as a comment: {tokens:?}"
    );

    // `#` still is one, and `/* */` too.
    let hashed = heuristic_tokens("  a = 1; # note", DiffSyntaxLanguage::Nix);
    assert!(
        hashed
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "`#` is Nix's line comment: {hashed:?}"
    );
    let blocked = heuristic_tokens("  a = /* note */ 1;", DiffSyntaxLanguage::Nix);
    assert!(
        blocked
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "`/* */` is Nix's block comment: {blocked:?}"
    );
}

/// Both new keyword tables, which nothing else reaches: every other Nix and
/// Jinja test goes through `prepare_test_document`, i.e. tree-sitter.
#[test]
fn nix_and_jinja_heuristic_keyword_tables_are_covered() {
    let line = "  x = with pkgs; let y = 1; in rec { inherit y; }";
    let found = heuristic_keywords(line, DiffSyntaxLanguage::Nix);
    for expected in ["with", "let", "in", "rec", "inherit"] {
        assert!(
            found.contains(&expected),
            "Nix keyword `{expected}` missing from {found:?}"
        );
    }
    assert!(
        heuristic_keywords("  buildInputs = [ pkgs.hello ];", DiffSyntaxLanguage::Nix).is_empty(),
        "an ordinary Nix attribute name must not colour as a keyword"
    );

    // The Jinja table omits any identifier that could also be an HTML attribute
    // name or an English word: the heuristic sees the whole line.
    let found = heuristic_keywords("{% endif %}{% extends 'base' %}", DiffSyntaxLanguage::Jinja);
    for expected in ["endif", "extends"] {
        assert!(
            found.contains(&expected),
            "Jinja keyword `{expected}` missing from {found:?}"
        );
    }
    for prose in [
        "  <label for=\"name\">Name</label>",
        "  <p>Do it with care, and set it aside.</p>",
    ] {
        assert!(
            heuristic_keywords(prose, DiffSyntaxLanguage::Jinja).is_empty(),
            "an HTML attribute or English word coloured as a Jinja keyword in \
                 {prose:?}: {:?}",
            heuristic_keywords(prose, DiffSyntaxLanguage::Jinja)
        );
    }

    // The text-bodied reading shares the table.
    assert_eq!(
        heuristic_keywords("{% endif %}", DiffSyntaxLanguage::JinjaText),
        heuristic_keywords("{% endif %}", DiffSyntaxLanguage::Jinja),
        "both Jinja readings must share one keyword table"
    );
}

/// Templates are mostly prose, and an unconditional single-quote rule painted
/// the rest of the line from the first `It's`.
#[test]
fn markup_prose_apostrophes_do_not_open_a_string() {
    for language in [
        DiffSyntaxLanguage::Jinja,
        DiffSyntaxLanguage::Html,
        DiffSyntaxLanguage::Vue,
        DiffSyntaxLanguage::Xml,
    ] {
        for line in [
            "  <p>It's a test</p>",
            "  <p>don't panic</p>",
            "  <li>{{ user.name }}'s profile</li>",
        ] {
            assert!(
                heuristic_string_spans(line, language).is_empty(),
                "{language:?} treated a prose apostrophe as a quote in {line:?}: {:?}",
                heuristic_tokens(line, language)
            );
        }
    }
}

/// ... while a `'` in value position is still a quote, which is why the rule is
/// positional rather than a flat "markup has no single quotes".
#[test]
fn markup_single_quoted_values_are_still_strings() {
    assert_eq!(
        heuristic_string_spans("  <div class='card'>", DiffSyntaxLanguage::Html),
        vec!["'card'"]
    );
    assert_eq!(
        heuristic_string_spans("  {{ x|default('n/a') }}", DiffSyntaxLanguage::Jinja),
        vec!["'n/a'"]
    );
    assert_eq!(
        heuristic_string_spans("  {% if y == 'z' %}", DiffSyntaxLanguage::Jinja),
        vec!["'z'"]
    );
}

/// The positional rule must not leak into languages where `'` really does open
/// a string anywhere -- Rust byte and char literals are the sharp case.
#[test]
fn non_markup_languages_keep_unconditional_single_quote_strings() {
    assert_eq!(
        heuristic_string_spans("let b = b'x';", DiffSyntaxLanguage::Rust),
        vec!["'x'"]
    );
    assert_eq!(
        heuristic_string_spans("s = 'it''s'", DiffSyntaxLanguage::Sql),
        vec!["'it'", "'s'"]
    );
}

/// Pins a deliberate limitation rather than an achievement.
///
/// The heuristic tokenizer is per-line and has no notion of which SFC
/// section a line belongs to, so Vue has to pick one comment/keyword
/// dialect for the whole file. It is grouped with Html/Xml, which is right
/// for the template -- `<img src="//cdn/x">` must not grey out as a line
/// comment, and attributes named `class`/`for` must not render as keywords
/// -- but it means `<script>` bodies get no `//` comments and no JS
/// keywords when tree-sitter is unavailable.
///
/// This only bites the fallback paths: files over
/// TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES, over-long lines, and builds without
/// `syntax-web`. If that ever stops being acceptable, the fix is section
/// tracking in the streamed heuristic state, not flipping the dialect --
/// flipping it just moves the damage into the template.
#[test]
fn vue_heuristic_fallback_uses_the_markup_dialect_for_the_whole_file() {
    let template_line = r#"  <img class="logo" src="//cdn.example.com/logo.png">"#;
    let tokens = syntax_tokens_for_line(
        template_line,
        DiffSyntaxLanguage::Vue,
        DiffSyntaxMode::HeuristicOnly,
    );
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "a protocol-relative URL in a template must not be greyed out as a `//` comment: \
             {tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(template_line, &tokens, SyntaxTokenKind::Keyword, "class"),
        "template attribute names must not render as JS keywords: {tokens:?}"
    );

    // The accepted cost, asserted so a future change to
    // `heuristic_comment_config` cannot flip it unnoticed.
    let script_line = "const count = 42; // note";
    let tokens = syntax_tokens_for_line(
        script_line,
        DiffSyntaxLanguage::Vue,
        DiffSyntaxMode::HeuristicOnly,
    );
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "known limitation: the heuristic cannot see that this line is inside <script>, so \
             `//` is not a comment here. If this now fails, the dialect was changed -- re-check \
             the template assertions above: {tokens:?}"
    );
    assert!(
        !has_token_kind_and_text(script_line, &tokens, SyntaxTokenKind::Keyword, "const"),
        "known limitation: `const` is not a keyword in the markup dialect: {tokens:?}"
    );
}

#[test]
fn single_line_syntax_cache_isolated_by_language_for_same_markup_text() {
    reset_ts_parser_test_state();

    let text = r#"<div class="demo">ok</div>"#;
    let html = syntax_tokens_for_line(text, DiffSyntaxLanguage::Html, DiffSyntaxMode::Auto);
    assert!(
        html.iter().any(|token| {
            matches!(
                token.kind,
                SyntaxTokenKind::Tag | SyntaxTokenKind::Attribute
            )
        }),
        "HTML mode should classify markup tokens: {html:?}"
    );

    let json = syntax_tokens_for_line(text, DiffSyntaxLanguage::Json, DiffSyntaxMode::Auto);
    assert!(
        !json.iter().any(|token| {
            matches!(
                token.kind,
                SyntaxTokenKind::Tag | SyntaxTokenKind::Attribute
            )
        }),
        "JSON mode should not reuse HTML markup tokens: {json:?}"
    );
    assert_ne!(json, html);

    let html_again = syntax_tokens_for_line(text, DiffSyntaxLanguage::Html, DiffSyntaxMode::Auto);
    assert_eq!(html_again, html);
}
