use super::*;

#[test]
fn gitcommit_prepared_document_captures_path_symbol_and_trailer_tokens() {
    let text = [
        "Subject",
        "",
        "closes #123",
        "Signed-off-by: me@example.com",
        "# On branch feature/demo",
        "# Changes to be committed:",
        "# renamed: src/old.rs -> src/new.rs",
    ]
    .join("\n");
    let document = prepare_test_document(DiffSyntaxLanguage::GitCommit, &text);

    let trailer = syntax_tokens_for_prepared_document_line(document, 3)
        .expect("gitcommit trailer line should have prepared tokens");
    assert!(
        trailer.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "gitcommit trailer key should produce Property token, got: {trailer:?}"
    );

    let branch = syntax_tokens_for_prepared_document_line(document, 4)
        .expect("gitcommit branch line should have prepared tokens");
    assert!(
        branch
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::StringSpecial),
        "gitcommit branch line should produce StringSpecial token, got: {branch:?}"
    );

    let renamed = syntax_tokens_for_prepared_document_line(document, 6)
        .expect("gitcommit renamed file line should have prepared tokens");
    assert!(
        renamed.iter().any(|t| t.kind == SyntaxTokenKind::DiffDelta),
        "gitcommit renamed file line should produce DiffDelta token, got: {renamed:?}"
    );
    assert!(
        renamed
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::StringSpecial),
        "gitcommit renamed file path should produce StringSpecial token, got: {renamed:?}"
    );
}

#[test]
fn xml_treesitter_captures_markup_link_via_system_literal() {
    let text = "<!DOCTYPE root SYSTEM \"https://example.com/schema.dtd\">";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::MarkupLink),
        "XML system literal should produce MarkupLink token, got: {tokens:?}"
    );
}

#[test]
fn xml_heuristic_highlights_comment() {
    let text = "<!-- this is a comment -->";
    let tokens =
        syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::HeuristicOnly);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "XML heuristic should highlight <!-- --> comments"
    );
}

#[test]
fn yaml_auto_single_line_highlights_list_item_punctuation_and_strings() {
    let text = "      - \"scripts/windows/verify-signed-artifact.ps1\"";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Yaml, DiffSyntaxMode::Auto);

    assert!(
        tokens
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (6..7) }),
        "YAML single-line fallback should highlight the list dash: {tokens:?}"
    );
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::String),
        "YAML single-line fallback should highlight quoted scalars: {tokens:?}"
    );
}

#[test]
fn yaml_auto_single_line_highlights_mapping_keys() {
    let top_level = syntax_tokens_for_line(
        "permissions:",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        top_level
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Property && token.range == (0..11)),
        "YAML single-line fallback should highlight top-level mapping keys: {top_level:?}"
    );

    let nested = syntax_tokens_for_line(
        "      - name: Validate workflow YAML",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        nested
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (6..7) }),
        "YAML single-line fallback should still highlight list punctuation for list-item mappings: {nested:?}"
    );
    assert!(
        nested
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Property && token.range == (8..12)),
        "YAML single-line fallback should highlight mapping keys after a list dash: {nested:?}"
    );
}

#[test]
fn yaml_auto_single_line_highlights_mapping_punctuation_and_plain_scalars() {
    let required = syntax_tokens_for_line(
        "        required: false",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        required
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Property && token.range == (8..16) }),
        "YAML fallback should highlight mapping keys: {required:?}"
    );
    assert!(
        required
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (16..17) }),
        "YAML fallback should highlight mapping punctuation: {required:?}"
    );
    assert!(
        required
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Boolean && token.range == (18..23) }),
        "YAML fallback should highlight boolean scalars: {required:?}"
    );

    let string_value = syntax_tokens_for_line(
        "        type: string",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        string_value
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (12..13) }),
        "YAML fallback should highlight mapping punctuation for string values: {string_value:?}"
    );
    assert!(
        string_value
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::String && token.range == (14..20) }),
        "YAML fallback should highlight plain string scalars: {string_value:?}"
    );

    let expression_value = syntax_tokens_for_line(
        "      TAG: ${{ needs.prepare.outputs.tag }}",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        expression_value
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (9..10) }),
        "YAML fallback should highlight mapping punctuation for expressions: {expression_value:?}"
    );
    assert!(
        expression_value
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::String && token.range == (11..43) }),
        "YAML fallback should highlight GitHub expression scalars as strings: {expression_value:?}"
    );
}

#[test]
fn yaml_heuristic_handles_malformed_and_unicode_scalars_without_invalid_ranges() {
    for text in [
        r#"emoji: "😀"#,
        "emoji: 😀 # note",
        "ключ: значение",
        "- 😀",
        "name: ",
        "name:#not-a-comment",
        "  - name: café",
        "script: |+9 trailing",
        "script: >-2",
    ] {
        let tokens = syntax_tokens_for_line_heuristic(text, DiffSyntaxLanguage::Yaml);
        assert_token_ranges_are_utf8_safe(text, &tokens);
    }
}

#[test]
fn yaml_auto_single_line_highlights_block_scalar_indicators_and_sequence_mapping_values() {
    let sequence_mapping = syntax_tokens_for_line(
        "      - name: Build release binary",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        sequence_mapping
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (6..7) }),
        "YAML fallback should highlight list punctuation: {sequence_mapping:?}"
    );
    assert!(
        sequence_mapping
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Property && token.range == (8..12) }),
        "YAML fallback should highlight sequence mapping keys: {sequence_mapping:?}"
    );
    assert!(
        sequence_mapping
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (12..13) }),
        "YAML fallback should highlight sequence mapping punctuation: {sequence_mapping:?}"
    );
    assert!(
        sequence_mapping
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::String && token.range == (14..34) }),
        "YAML fallback should highlight sequence mapping scalar values: {sequence_mapping:?}"
    );

    let block_scalar = syntax_tokens_for_line(
        "        run: |",
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxMode::Auto,
    );
    assert!(
        block_scalar
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (11..12) }),
        "YAML fallback should highlight the mapping colon for block scalars: {block_scalar:?}"
    );
    assert!(
        block_scalar
            .iter()
            .any(|token| { token.kind == SyntaxTokenKind::Punctuation && token.range == (13..14) }),
        "YAML fallback should highlight block scalar indicators: {block_scalar:?}"
    );
}

#[test]
fn grammar_and_highlight_spec_agree_on_supported_languages() {
    for lang in all_supported_languages() {
        let has_grammar = tree_sitter_grammar(lang).is_some();
        let has_spec = tree_sitter_highlight_spec(lang).is_some();
        assert_eq!(
            has_grammar, has_spec,
            "tree_sitter_grammar and tree_sitter_highlight_spec disagree for {lang:?}: \
                 grammar={has_grammar}, spec={has_spec}"
        );
    }
}

/// Every path the config and assembly batch claims, and the collisions it
/// had to avoid.
///
/// Path resolution is the only thing between a wired grammar and a file that
/// still renders as plain text, and most of these resolve on the *file-name*
/// pass rather than the extension one -- `Dockerfile` and `CMakeLists.txt`
/// have no useful extension, and `Path::extension()` returns `None` for a
/// leading-dot name like `.editorconfig` -- so nothing else exercises them.
#[test]
fn config_language_paths_resolve() {
    let cases: &[(&str, DiffSyntaxLanguage)] = &[
        ("CMakeLists.txt", DiffSyntaxLanguage::Cmake),
        ("cmake/FindFoo.cmake", DiffSyntaxLanguage::Cmake),
        ("Dockerfile", DiffSyntaxLanguage::Dockerfile),
        ("Containerfile", DiffSyntaxLanguage::Dockerfile),
        ("docker/build.dockerfile", DiffSyntaxLanguage::Dockerfile),
        (".editorconfig", DiffSyntaxLanguage::Ini),
        ("gitconfig", DiffSyntaxLanguage::Ini),
        (".gitconfig", DiffSyntaxLanguage::Ini),
        ("etc/systemd/system/app.service", DiffSyntaxLanguage::Ini),
        ("app.timer", DiffSyntaxLanguage::Ini),
        ("app.socket", DiffSyntaxLanguage::Ini),
        ("multi-user.target", DiffSyntaxLanguage::Ini),
        ("share/applications/app.desktop", DiffSyntaxLanguage::Ini),
        ("setup.cfg", DiffSyntaxLanguage::Ini),
        ("etc/httpd/httpd.conf", DiffSyntaxLanguage::Conf),
        ("etc/nginx/nginx.conf", DiffSyntaxLanguage::Conf),
        ("my.cnf", DiffSyntaxLanguage::Conf),
        ("build/hello.ll", DiffSyntaxLanguage::Llvm),
        ("justfile", DiffSyntaxLanguage::Just),
        (".justfile", DiffSyntaxLanguage::Just),
        ("tasks.just", DiffSyntaxLanguage::Just),
        ("Caddyfile", DiffSyntaxLanguage::Caddyfile),
        (".gitignore", DiffSyntaxLanguage::Gitignore),
        (".dockerignore", DiffSyntaxLanguage::Gitignore),
        (".npmignore", DiffSyntaxLanguage::Gitignore),
        ("crontab", DiffSyntaxLanguage::Crontab),
        ("build/module.wat", DiffSyntaxLanguage::Wat),
        ("build/module.wast", DiffSyntaxLanguage::Wat),
        ("shaders/frag.spvasm", DiffSyntaxLanguage::Spirv),
        ("bin/hello.il", DiffSyntaxLanguage::Cil),
        ("src/app.gleam", DiffSyntaxLanguage::Gleam),
        ("src/main.v", DiffSyntaxLanguage::V),
        ("config.jsonnet", DiffSyntaxLanguage::Jsonnet),
        ("lib/util.libsonnet", DiffSyntaxLanguage::Jsonnet),
        ("application.properties", DiffSyntaxLanguage::JavaProperties),
        ("api/schema.proto", DiffSyntaxLanguage::Proto),
        ("src/unit.pas", DiffSyntaxLanguage::Pascal),
        ("Project.dpr", DiffSyntaxLanguage::Pascal),
        ("data/config.json5", DiffSyntaxLanguage::JavaScript),
        ("deploy/example.env", DiffSyntaxLanguage::Dotenv),
        (".env", DiffSyntaxLanguage::Dotenv),
        ("data/records.csv", DiffSyntaxLanguage::Csv),
        ("document.kdl", DiffSyntaxLanguage::Kdl),
        ("config.ron", DiffSyntaxLanguage::Ron),
        ("schema.cue", DiffSyntaxLanguage::Cue),
        ("grammars/expression.ebnf", DiffSyntaxLanguage::Ebnf),
        ("lib/Demo.pm", DiffSyntaxLanguage::Perl),
        ("bin/script.pl", DiffSyntaxLanguage::Perl),
        ("config.dhall", DiffSyntaxLanguage::Dhall),
        ("src/app.coffee", DiffSyntaxLanguage::CoffeeScript),
    ];
    for (path, expected) in cases {
        assert_eq!(
            diff_syntax_language_for_path(path),
            Some(*expected),
            "{path} should resolve to {expected:?}"
        );
    }

    // Collisions this batch had to step around. `.targets` and `.props` are
    // MSBuild XML and predate `.target`; `.md` is not a systemd `.mount`.
    let unchanged: &[(&str, DiffSyntaxLanguage)] = &[
        ("Directory.Build.targets", DiffSyntaxLanguage::Xml),
        ("Directory.Build.props", DiffSyntaxLanguage::Xml),
        ("Makefile", DiffSyntaxLanguage::Makefile),
        ("main.s", DiffSyntaxLanguage::Assembly),
    ];
    for (path, expected) in unchanged {
        assert_eq!(
            diff_syntax_language_for_path(path),
            Some(*expected),
            "{path} should still resolve to {expected:?}"
        );
    }
}

/// The three `#lua-match?` patterns in tree-sitter-zig's own query, which the
/// engine never evaluates, so each applied to everything it was meant to
/// filter.
///
/// Every assertion here failed before queries/zig_highlights.scm was vendored
/// in place of `tree_sitter_zig::HIGHLIGHTS_QUERY`. The identifier pair is why
/// a supplement was not enough: both rules captured *all* identifiers, and
/// `@constant` came second, so every name in every Zig file was a constant.
#[test]
fn zig_identifier_and_comment_predicates_are_actually_evaluated() {
    let text = concat!(
        "// an ordinary note\n",
        "/// a doc comment\n",
        "const lowercase_name = 1;\n",
        "const MAX_RETRIES = 3;\n",
        "const StructName = struct {};\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Zig, text);
    let kinds = |line_ix: usize, needle: &str| -> Vec<SyntaxTokenKind> {
        let line = text.lines().nth(line_ix).expect("line");
        let at = line.find(needle).expect("needle");
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| {
                tokens
                    .iter()
                    .filter(|token| token.range.start <= at && at < token.range.end)
                    .map(|token| token.kind)
                    .collect()
            })
            .unwrap_or_default()
    };

    assert_eq!(
        kinds(2, "lowercase_name"),
        vec![SyntaxTokenKind::Variable],
        "a lower-case binding is a variable, not a constant"
    );
    assert_eq!(
        kinds(3, "MAX_RETRIES"),
        vec![SyntaxTokenKind::Constant],
        "SCREAMING_CASE is what the constant rule is actually for"
    );
    assert_eq!(
        kinds(4, "StructName"),
        vec![SyntaxTokenKind::Type],
        "CamelCase is what the type rule is actually for"
    );
    assert_eq!(
        kinds(0, "an ordinary"),
        vec![SyntaxTokenKind::Comment],
        "a `//` comment is not documentation"
    );
    assert_eq!(
        kinds(1, "a doc"),
        vec![SyntaxTokenKind::CommentDoc],
        "a `///` comment is, which upstream's `^//!` missed even when evaluated"
    );
}

/// `.conf` has no grammar, so this is the whole of what it can say.
///
/// Each assertion here is a bug that was in the first cut: `;` read as a
/// comment and greyed the tail of every nginx statement, and `stream` in the
/// keyword list matched inside `application/octet-stream`.
#[test]
fn conf_heuristic_reads_directives_without_breaking_nginx() {
    let tokens = |line: &str| {
        syntax_tokens_for_line(line, DiffSyntaxLanguage::Conf, DiffSyntaxMode::Auto).to_vec()
    };

    let statement = tokens("    worker_connections  1024;");
    assert!(
        statement
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Property && t.range == (4..22)),
        "the first word of a line names the setting: {statement:?}"
    );
    assert!(
        statement.iter().all(|t| t.kind != SyntaxTokenKind::Comment),
        "a `;` ending an nginx statement is not a comment: {statement:?}"
    );

    let leading = tokens("; an INI-dialect comment");
    assert!(
        leading
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Comment && t.range.start == 0),
        "a `;` that opens the line is: {leading:?}"
    );

    let mime = tokens("    default_type  application/octet-stream;");
    assert!(
        mime.iter().all(|t| t.kind != SyntaxTokenKind::Keyword),
        "`stream` inside a MIME type is not a keyword: {mime:?}"
    );

    let section = tokens("</VirtualHost>");
    assert!(
        section
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Tag && t.range == (0..14)),
        "an Apache closing tag is the whole line, and nothing else colours it: {section:?}"
    );
}
