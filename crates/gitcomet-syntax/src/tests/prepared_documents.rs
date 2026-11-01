use super::*;

#[test]
fn prepared_document_cache_isolated_by_language_for_same_script_markup() {
    reset_ts_parser_test_state();
    reset_prepared_syntax_cache();

    let text = "<script>\nconst value = 1;\n</script>";
    let html = prepare_test_document(DiffSyntaxLanguage::Html, text);
    let xml = prepare_test_document(DiffSyntaxLanguage::Xml, text);

    let html_tokens = syntax_tokens_for_prepared_document_line(html, 1)
        .expect("HTML script line tokens should be available");
    assert!(
        html_tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Keyword),
        "HTML document should inject JavaScript keyword highlighting: {html_tokens:?}"
    );
    assert!(
        html_tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Number),
        "HTML document should inject JavaScript number highlighting: {html_tokens:?}"
    );

    let xml_tokens = syntax_tokens_for_prepared_document_line(xml, 1)
        .expect("XML script line tokens should be available");
    assert!(
        !xml_tokens.iter().any(|token| {
            matches!(
                token.kind,
                SyntaxTokenKind::Keyword | SyntaxTokenKind::Number
            )
        }),
        "XML document should not reuse HTML script injection tokens: {xml_tokens:?}"
    );
    assert_ne!(xml_tokens, html_tokens);
}

#[test]
fn single_line_syntax_cache_drops_text_hash_collisions_on_text_mismatch() {
    let mut cache = SingleLineSyntaxTokenCache::new();
    let key = SingleLineSyntaxTokenCacheKey {
        language: DiffSyntaxLanguage::Html,
        mode: DiffSyntaxMode::Auto,
        text_hash: 7,
    };
    let tokens: Arc<[SyntaxToken]> = vec![SyntaxToken {
        range: 0..5,
        kind: SyntaxTokenKind::Tag,
    }]
    .into();

    cache.insert(key, "<div>", Arc::clone(&tokens));

    assert!(cache.get(key, "<span>").is_none());
    assert!(cache.by_key.is_empty());
    assert!(cache.lru_order.is_empty());
}

#[test]
fn prepared_document_preserves_multiline_treesitter_context() {
    let lines = ["/* open comment", "still comment */ let x = 1;"];
    let doc = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    let first = syntax_tokens_for_prepared_document_line(doc, 0)
        .expect("prepared tokens should be available for line 0");
    let second = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("prepared tokens should be available for line 1");

    assert!(
        first.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "first line should include comment tokens"
    );
    assert!(
        second.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "second line should include comment tokens from multiline context"
    );
    assert!(
        second
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Comment && t.range.start == 0),
        "second line should start with comment highlighting from multiline context, got: {second:?}"
    );
}

#[test]
fn prepared_document_request_line_tokens_preserves_multiline_context() {
    let lines = ["/* open comment", "still comment */ let x = 1;"];
    let doc = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

    let expected = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("sync line-token lookup should materialize the continuation line chunk");

    match request_syntax_tokens_for_prepared_document_line(doc, 1) {
        Some(PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
            assert!(
                tokens
                    .iter()
                    .any(|t| t.kind == SyntaxTokenKind::Comment && t.range.start == 0),
                "requested second line should start with comment highlighting from multiline context, got: {tokens:?}"
            );
            assert_eq!(
                tokens.as_ref(),
                expected.as_slice(),
                "requested prepared continuation line should match the synchronously materialized tokens"
            );
        }
        other => panic!("expected ready prepared second line, got {other:?}"),
    }
}

#[test]
fn prepared_rust_document_highlights_macro_token_tree_via_injection() {
    let text = "test_macro!(value.field::<Vec<u32>>());";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 0)
        .expect("Rust macro line tokens should be available");
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::FunctionMethod, "field"),
        "Rust macro token trees should inject nested method calls: {tokens:?}"
    );
    assert!(
        has_token_kind_and_text(text, &tokens, SyntaxTokenKind::TypeBuiltin, "u32"),
        "Rust macro token trees should inject nested builtin types: {tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_highlights_fenced_rust_block_via_injection() {
    let lines = ["```rust", "fn main() { let value = 42; }", "```"];
    let doc = prepare_markdown_document(&lines);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("markdown fenced code line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "embedded Rust should highlight keywords inside fenced markdown, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
        "embedded Rust should highlight numbers inside fenced markdown, got: {tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_highlights_fenced_html_block_via_injection() {
    let doc = prepare_markdown_document(&["```html", "<div class=\"note\">ok</div>", "```"]);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("markdown fenced HTML line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "embedded HTML should highlight tags inside fenced markdown, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute),
        "embedded HTML should highlight attributes inside fenced markdown, got: {tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_highlights_fenced_ruby_block_via_path_alias() {
    let doc = prepare_markdown_document(&["```foo/bar/baz.rb", "if @user", "end", "```"]);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("markdown fenced Ruby line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "Ruby path aliases in fenced markdown should highlight keywords, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "Ruby path aliases in fenced markdown should highlight instance variables, got: {tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_highlights_fenced_tsx_block_via_path_alias() {
    let doc = prepare_markdown_document(&[
        "```src/components/button.tsx",
        "const node = <button disabled />;",
        "```",
    ]);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("markdown fenced TSX line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "TSX path aliases in fenced markdown should highlight JSX tags, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute),
        "TSX path aliases in fenced markdown should highlight JSX attributes, got: {tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_highlights_fenced_gomod_block_via_filename_alias() {
    let line = "module example.com/project";
    let doc = prepare_markdown_document(&["```go.mod", line, "```"]);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("markdown fenced go.mod line tokens should be available");
    assert!(
        has_token_kind_and_text(line, &tokens, SyntaxTokenKind::Keyword, "module"),
        "go.mod filename aliases in fenced markdown should highlight keywords, got: {tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_unknown_fence_does_not_reuse_previous_language_tokens() {
    let rust_doc = prepare_markdown_document(&["```rs", "fn main() { let value = 42; }", "```"]);
    let rust_tokens = syntax_tokens_for_prepared_document_line(rust_doc, 1)
        .expect("markdown fenced Rust line tokens should be available");
    assert!(
        rust_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Keyword),
        "supported fenced Rust should highlight keywords, got: {rust_tokens:?}"
    );
    assert!(
        rust_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Number),
        "supported fenced Rust should highlight numbers, got: {rust_tokens:?}"
    );

    let unknown_doc = prepare_markdown_document(&[
        "```foo/bar/baz.unknown",
        "fn main() { let value = 42; }",
        "```",
    ]);
    let unknown_tokens = syntax_tokens_for_prepared_document_line(unknown_doc, 1)
        .expect("markdown fenced unknown-language line tokens should be available");
    assert!(
        !unknown_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Keyword),
        "unsupported fenced languages should not reuse stale Rust keyword tokens, got: {unknown_tokens:?}"
    );
    assert!(
        !unknown_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Number),
        "unsupported fenced languages should not reuse stale Rust number tokens, got: {unknown_tokens:?}"
    );
}

#[test]
fn prepared_markdown_document_highlights_inline_code_and_html_block() {
    let doc = prepare_markdown_document(&["Use `git status` here", "<div class=\"note\">ok</div>"]);

    let inline_tokens = syntax_tokens_for_prepared_document_line(doc, 0)
        .expect("markdown inline line tokens should be available");
    assert!(
        inline_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::PunctuationDelimiter),
        "markdown inline code should at least preserve delimiter highlighting, got: {inline_tokens:?}"
    );

    let html_tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("markdown HTML block line tokens should be available");
    assert!(
        html_tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "markdown HTML blocks should inject HTML tag highlighting, got: {html_tokens:?}"
    );
    assert!(
        html_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Attribute),
        "markdown HTML blocks should inject HTML attribute highlighting, got: {html_tokens:?}"
    );
}

#[test]
fn prepared_html_document_highlights_style_element_contents_via_css_injection() {
    let lines = ["<style>", "body { color: red; }", "</style>"];
    let doc = prepare_html_document(&lines);

    let style_tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("style line tokens should be available");
    assert!(
        style_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Property),
        "embedded CSS should highlight properties inside <style>, got: {style_tokens:?}"
    );
}

#[test]
fn prepared_html_document_highlights_script_element_contents_via_javascript_injection() {
    let lines = ["<script>", "const value = 1;", "</script>"];
    let doc = prepare_html_document(&lines);

    let script_tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("script line tokens should be available");
    assert!(
        script_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Keyword),
        "embedded JavaScript should highlight keywords inside <script>, got: {script_tokens:?}"
    );
    assert!(
        script_tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Number),
        "embedded JavaScript should highlight numbers inside <script>, got: {script_tokens:?}"
    );
}

#[test]
fn prepared_html_document_highlights_onclick_attribute_via_javascript_injection() {
    let lines = [r#"<button onclick="const value = 1;">go</button>"#];
    let doc = prepare_html_document(&lines);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
        .expect("button line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute),
        "root HTML tokens should still include the onclick attribute, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "embedded JavaScript should highlight keywords inside onclick, got: {tokens:?}"
    );
}

#[test]
fn prepared_html_document_highlights_style_attribute_via_css_injection() {
    let lines = [r#"<div style="color: red; display: block">ok</div>"#];
    let doc = prepare_html_document(&lines);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
        .expect("div line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute),
        "root HTML tokens should still include the style attribute, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "embedded CSS should highlight properties inside style=, got: {tokens:?}"
    );
}

/// A single-file component covering every Vue injection path at once.
/// Line indices are asserted against by the tests below, so keep them stable.
pub(super) const VUE_SFC_FIXTURE: &[&str] = &[
    /* 0 */ "<template>",
    /* 1 */ r#"  <div :class="wrapperClass">"#,
    /* 2 */ r#"    <button v-if="count > 10">{{ count + 1 }}</button>"#,
    /* 3 */ "  </div>",
    /* 4 */ "</template>",
    /* 5 */ "",
    /* 6 */ r#"<script setup lang="ts">"#,
    /* 7 */ "const count = 42;",
    /* 8 */ "</script>",
    /* 9 */ "",
    /* 10 */ r#"<style lang="scss">"#,
    /* 11 */ ".wrapper { color: red; }",
    /* 12 */ "</style>",
];

#[test]
fn prepared_vue_document_highlights_template_natively() {
    // The Vue grammar inherits html, so <template> is parsed by the root
    // grammar rather than through an injection. That matters because the
    // injection engine is depth-1 only.
    let doc = prepare_vue_document(VUE_SFC_FIXTURE);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("template line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "template markup should highlight tag names, got: {tokens:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_script_setup_via_typescript_injection() {
    let doc = prepare_vue_document(VUE_SFC_FIXTURE);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 7)
        .expect("script line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "<script setup lang=\"ts\"> body should highlight keywords, got: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
        "<script setup lang=\"ts\"> body should highlight numbers, got: {tokens:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_scss_style_block_via_css_injection() {
    // "scss" resolves to DiffSyntaxLanguage::Css through the shared alias table.
    let doc = prepare_vue_document(VUE_SFC_FIXTURE);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 11)
        .expect("style line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "<style lang=\"scss\"> body should highlight properties, got: {tokens:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_interpolation_via_typescript_injection() {
    let doc = prepare_vue_document(VUE_SFC_FIXTURE);
    let kinds = token_kinds_for_line_fragment(doc, 2, VUE_SFC_FIXTURE[2], "count + 1");

    assert!(
        kinds.contains(&SyntaxTokenKind::Operator),
        "{{{{ }}}} interpolation should highlight operators, got: {kinds:?}"
    );
    assert!(
        kinds.contains(&SyntaxTokenKind::Number),
        "{{{{ }}}} interpolation should highlight numbers, got: {kinds:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_directive_value_as_expression_not_string() {
    // The html base rule `(attribute_value) @string` would otherwise colour the
    // whole directive expression as a string; vue_highlights.scm overrides it
    // with @variable so the TypeScript injection shows through.
    let doc = prepare_vue_document(VUE_SFC_FIXTURE);
    let kinds = token_kinds_for_line_fragment(doc, 2, VUE_SFC_FIXTURE[2], "count > 10");

    assert!(
        kinds.contains(&SyntaxTokenKind::Operator),
        "v-if expression should highlight operators, got: {kinds:?}"
    );
    assert!(
        kinds.contains(&SyntaxTokenKind::Number),
        "v-if expression should highlight numbers, got: {kinds:?}"
    );
    assert!(
        !kinds.contains(&SyntaxTokenKind::String),
        "v-if expression must not fall back to the html string rule, got: {kinds:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_directive_name_as_attribute() {
    // `@tag.attribute` has to map to Attribute; without an explicit arm the
    // dotted-suffix trimming would silently resolve it to Tag.
    let doc = prepare_vue_document(VUE_SFC_FIXTURE);
    let kinds = token_kinds_for_line_fragment(doc, 2, VUE_SFC_FIXTURE[2], "v-if");

    assert!(
        kinds.contains(&SyntaxTokenKind::Attribute),
        "directive names should highlight as attributes, got: {kinds:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_plain_script_via_javascript_injection() {
    // No `lang` attribute: this falls through to the inherited html_tags rule,
    // which is guarded by `#not-match? "\\slang\\s*="` so it cannot also fire
    // for the `lang="ts"` case above.
    let lines = ["<script>", "const value = 1;", "</script>"];
    let doc = prepare_vue_document(&lines);

    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("script line tokens should be available");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "plain <script> body should highlight keywords, got: {tokens:?}"
    );
}

#[test]
fn prepared_vue_document_highlights_slot_shorthand_sigil() {
    // `#` is the v-slot shorthand. Upstream captures `:`, `.` and `@` but not
    // `#`, which leaves it as the one unstyled sigil on the tag.
    let line = r#"  <MyComp #footer="{ row }">"#;
    let doc = prepare_vue_document(&["<template>", line, "</template>"]);
    let kinds = token_kinds_for_line_fragment(doc, 1, line, "#");

    assert!(
        kinds.contains(&SyntaxTokenKind::PunctuationSpecial),
        "the v-slot `#` shorthand should highlight like `:` and `@`, got: {kinds:?}"
    );
}

/// Regression guard for the injection-per-directive blowup. Without the
/// `#not-match?` guards in vue_injections.scm every directive and every
/// interpolation became its own injected layer: ~5 per line here, which
/// overruns TS_INJECTION_CACHE_MAX_ENTRIES (32) and evicts half the cache
/// mid-render, so scrolling re-parses everything.
#[test]
fn vue_plain_binding_directives_produce_no_injections() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let mut lines = vec!["<template>".to_string(), "  <ul>".to_string()];
    for ix in 0..30 {
        lines.push(format!(
            "    <li :key=\"row{ix}.id\" :class=\"row{ix}.cls\" \
                 v-model=\"form.field{ix}\" @click=\"select{ix}\">{{{{ row{ix}.label }}}}</li>"
        ));
    }
    lines.push("  </ul>".to_string());
    lines.push("</template>".to_string());
    let line_count = lines.len();

    let doc = prepare_test_document(DiffSyntaxLanguage::Vue, &lines.join("\n"));
    for line_ix in 0..line_count {
        let _ = syntax_tokens_for_prepared_document_line(doc, line_ix);
    }

    let cached = TS_INJECTION_CACHE.with(|cache| cache.borrow().len());
    assert_eq!(
        cached, 0,
        "{line_count} lines of bare identifier / dotted-path bindings need no TypeScript \
             parse -- vue_highlights.scm already colours them -- but {cached} injection cache \
             entries were created (cap is {TS_INJECTION_CACHE_MAX_ENTRIES})"
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// The other half of the guard above: skipping plain bindings must not cost
/// highlighting for the expressions the injection actually exists to serve.
#[test]
fn vue_expression_directives_still_inject_typescript() {
    let line = r#"  <button v-if="count > 10" @click="submit($event, 'now')">"#;
    let doc = prepare_vue_document(&["<template>", line, "</template>"]);

    let kinds = token_kinds_for_line_fragment(doc, 1, line, "count > 10");
    assert!(
        kinds.contains(&SyntaxTokenKind::Number),
        "an expression directive must still be parsed as TypeScript, got: {kinds:?}"
    );

    // `PreparedSyntaxDocument` is Copy, so the first handle is still usable.
    let kinds = token_kinds_for_line_fragment(doc, 1, line, "'now'");
    assert!(
        kinds.contains(&SyntaxTokenKind::String),
        "a call argument inside a directive should be parsed as TypeScript, got: {kinds:?}"
    );
}

/// Capturing the whole `(interpolation)` node paints the expression inside
/// it, not just the braces. Upstream relies on a companion `(raw_text) @none`
/// rule to punch the body back out, but `none` emits no token in this
/// engine, so the outer capture wins outright. That was invisible while
/// every interpolation was injected -- the injection carved the body out --
/// and became visible the moment plain interpolations stopped injecting.
#[test]
fn vue_plain_interpolation_does_not_paint_its_expression_as_a_sigil() {
    let line = r#"  <p>{{ msg }}</p>"#;
    let doc = prepare_vue_document(&["<template>", line, "</template>"]);

    let braces = token_kinds_for_line_fragment(doc, 1, line, "{{");
    assert!(
        braces.contains(&SyntaxTokenKind::PunctuationSpecial),
        "the interpolation delimiters should be sigil-coloured, got: {braces:?}"
    );

    let body = token_kinds_for_line_fragment(doc, 1, line, "msg");
    assert!(
        !body.contains(&SyntaxTokenKind::PunctuationSpecial),
        "the expression inside `{{{{ }}}}` must not inherit the delimiter colour, \
             got: {body:?}"
    );
}

/// The Vue grammar allows `v-if=ok` as well as `v-if="ok"`. Only the quoted
/// form has a `quoted_attribute_value`, so the unquoted one used to fall
/// through both the @variable override and the injection, landing on the
/// html `(attribute_value) @string` rule -- the exact miscolouring the
/// override exists to prevent.
#[test]
fn vue_unquoted_directive_value_is_not_coloured_as_a_string() {
    let line = r#"  <p v-if=ok>x</p>"#;
    let doc = prepare_vue_document(&["<template>", line, "</template>"]);
    let kinds = token_kinds_for_line_fragment(doc, 1, line, "ok");

    assert!(
        !kinds.contains(&SyntaxTokenKind::String),
        "an unquoted directive value is an expression, not a string: {kinds:?}"
    );
    assert!(
        !kinds.is_empty(),
        "an unquoted directive value should still be coloured, got nothing"
    );
}

/// `<script type="module" lang="ts">` matches a `type=` base rule and a
/// `lang=` vue rule over the same `raw_text`. prepared.rs tolerates the
/// duplicate by accident, but live.rs keeps both layers and interleaves
/// their captures at equal depth, so the editor colours the block
/// arbitrarily. The `lang` veto on the `type=` rules keeps it to one.
#[test]
fn vue_script_with_both_type_and_lang_injects_exactly_one_language() {
    let text = "<script type=\"module\" lang=\"ts\">\nconst x: number = 1;\n</script>\n";

    let lang: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, VUE_INJECTIONS_QUERY)
        .expect("vendored Vue injections.scm should compile");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang)
        .expect("vendored Vue grammar should load");
    let tree = parser.parse(text, None).expect("script should parse");

    let mut cursor = tree_sitter::QueryCursor::new();
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    let mut patterns = Vec::new();
    {
        let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
        tree_sitter::StreamingIterator::advance(&mut matches);
        while let Some(m) = matches.get() {
            patterns.push(m.pattern_index);
            tree_sitter::StreamingIterator::advance(&mut matches);
        }
    }

    assert_eq!(
        patterns.len(),
        1,
        "a script carrying both `type` and `lang` must match exactly one injection \
             pattern, matched {patterns:?}"
    );

    // …and it must be the TypeScript one, not the `type="module"` javascript one.
    let doc = prepare_test_document(DiffSyntaxLanguage::Vue, text);
    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("script body should have prepared tokens");
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Type || t.kind == SyntaxTokenKind::TypeBuiltin),
        "`lang=\"ts\"` should win over `type=\"module\"`, so the `: number` annotation \
             should be typed: {tokens:?}"
    );
}

/// The directive guard does not cover the inherited attribute rules, so
/// those had to stop injecting unconditionally too. Inline `style=` was both
/// the worst offender and actively wrong (the CSS grammar reads an attribute
/// body as a stylesheet, making `color` a type selector), so it was dropped.
#[test]
fn vue_static_inline_styles_do_not_flood_the_injection_cache() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let mut lines = vec!["<template>".to_string()];
    for ix in 0..40 {
        lines.push(format!("  <div style=\"color: red\" id=\"d{ix}\">x</div>"));
    }
    lines.push("</template>".to_string());
    let line_count = lines.len();

    let doc = prepare_test_document(DiffSyntaxLanguage::Vue, &lines.join("\n"));
    for line_ix in 0..line_count {
        let _ = syntax_tokens_for_prepared_document_line(doc, line_ix);
    }

    let cached = TS_INJECTION_CACHE.with(|cache| cache.borrow().len());
    assert_eq!(
        cached, 0,
        "static inline styles should not inject at all, but {cached} cache entries were \
             created from {line_count} lines (cap is {TS_INJECTION_CACHE_MAX_ENTRIES})"
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// A skipped injection still has to leave the value coloured -- that is the
/// premise the skip rests on.
#[test]
fn vue_plain_binding_directives_are_still_coloured_by_the_host_grammar() {
    let line = r#"  <div :class="wrapperClass">{{ label }}</div>"#;
    let doc = prepare_vue_document(&["<template>", line, "</template>"]);

    let kinds = token_kinds_for_line_fragment(doc, 1, line, "wrapperClass");
    assert!(
        !kinds.is_empty(),
        "a directive value skipped by the injection guard must still be coloured by \
             vue_highlights.scm, got nothing"
    );
    assert!(
        !kinds.contains(&SyntaxTokenKind::String),
        "…and must not fall back to the html `(attribute_value) @string` rule, got: {kinds:?}"
    );
}

#[test]
fn injection_cache_reuses_parsed_injection_across_chunks() {
    // Create an HTML document with a <script> block that spans multiple chunks
    // (> 64 lines). The injection cache should parse it once and reuse across chunks.
    let mut lines = Vec::new();
    lines.push("<html><body>".to_string());
    lines.push("<script>".to_string());
    for ix in 0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS + 20) {
        lines.push(format!("const value_{ix} = {ix};"));
    }
    lines.push("</script>".to_string());
    lines.push("</body></html>".to_string());

    let doc = prepare_test_document(DiffSyntaxLanguage::Html, &lines.join("\n"));

    // Request a line from the first chunk (inside the script block)
    let first_chunk_line = 5;
    let tokens_a = syntax_tokens_for_prepared_document_line(doc, first_chunk_line)
        .expect("tokens for first chunk line should be available");
    assert!(
        tokens_a.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "first chunk should have JavaScript keyword tokens via injection, got: {tokens_a:?}"
    );

    // Request a line from the second chunk (also inside the script block)
    let second_chunk_line = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS + 2;
    let tokens_b = syntax_tokens_for_prepared_document_line(doc, second_chunk_line)
        .expect("tokens for second chunk line should be available");
    assert!(
        tokens_b.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "second chunk should also have JavaScript keyword tokens (cached injection), got: {tokens_b:?}"
    );
}

#[test]
fn injection_cache_content_hash_distinguishes_different_documents() {
    // Two HTML documents that produce <script> injections at similar byte
    // positions but with different JavaScript content. The content_hash on
    // TreesitterInjectionMatch should prevent the second document from
    // reusing cached tokens from the first.
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let doc_a = prepare_test_document(
        DiffSyntaxLanguage::Html,
        "<html><body><script>\nconst alpha = 42;\n</script></body></html>",
    );

    // Fetch tokens from doc A's injection line to populate cache
    let tokens_a =
        syntax_tokens_for_prepared_document_line(doc_a, 1).expect("doc A should have tokens");
    assert!(
        tokens_a.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "doc A injection line should have keyword token, got: {tokens_a:?}"
    );

    // Doc B: different JS content at a similar structure but different text
    let doc_b = prepare_test_document(
        DiffSyntaxLanguage::Html,
        "<html><body><script>\nlet beta = \"hello\";\n</script></body></html>",
    );

    let tokens_b =
        syntax_tokens_for_prepared_document_line(doc_b, 1).expect("doc B should have tokens");
    assert!(
        tokens_b.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
        "doc B injection line should have keyword token, got: {tokens_b:?}"
    );
    // The token sets should differ since the JS content differs.
    // With the content hash, doc B gets its own injection parse.
    let a_kinds: Vec<_> = tokens_a.iter().map(|t| (t.range.clone(), t.kind)).collect();
    let b_kinds: Vec<_> = tokens_b.iter().map(|t| (t.range.clone(), t.kind)).collect();
    assert_ne!(
        a_kinds, b_kinds,
        "different JS content should produce different token sets"
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}
