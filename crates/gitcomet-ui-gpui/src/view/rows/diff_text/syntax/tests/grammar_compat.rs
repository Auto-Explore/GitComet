use super::prepared_documents::VUE_SFC_FIXTURE;
use super::*;

#[test]
fn vendored_rust_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let source = RUST_HIGHLIGHTS_QUERY;
    tree_sitter::Query::new(&lang, source).expect("vendored Rust highlights.scm should compile");
}

#[test]
fn vendored_css_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_css::LANGUAGE.into();
    let source = CSS_HIGHLIGHTS_QUERY;
    tree_sitter::Query::new(&lang, source).expect("vendored CSS highlights.scm should compile");
}

#[test]
fn vendored_bash_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
    tree_sitter::Query::new(&lang, BASH_HIGHLIGHTS_QUERY)
        .expect("vendored Bash highlights.scm should compile");
}

#[test]
fn vendored_html_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let source = HTML_HIGHLIGHTS_QUERY;
    tree_sitter::Query::new(&lang, source).expect("vendored HTML highlights.scm should compile");
}

#[test]
fn vendored_html_injections_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    tree_sitter::Query::new(&lang, HTML_INJECTIONS_QUERY)
        .expect("vendored HTML injections.scm should compile");
}

/// The Vue grammar is the one grammar we vendor rather than pull from
/// crates.io, so nothing external will tell us when it stops matching the
/// workspace `tree-sitter`. It binds through `tree-sitter-language`, which
/// means a tree-sitter upgrade only stays safe while this holds.
#[test]
fn vendored_vue_grammar_is_abi_compatible_with_workspace_tree_sitter() {
    let vue: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    let abi = vue.abi_version();
    assert!(
        (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&abi),
        "vendored Vue grammar ABI {abi} is outside the range this tree-sitter supports \
             ({}..={}); regenerate vendor/tree-sitter-vue with a newer tree-sitter-cli",
        tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
        tree_sitter::LANGUAGE_VERSION,
    );
}

// There is deliberately no test comparing the vendored ABI against the
// crates.io grammars'. Being *older* than tree-sitter-html is not a defect
// -- ABI versions stay supported across a wide range, which is exactly what
// the test above checks. Asserting `vue.abi >= html.abi` would instead turn
// any routine `cargo update` that bumps an unrelated grammar into a red CI
// while Vue still parses perfectly.

/// Every grammar in `vendor/` loads with the workspace's tree-sitter.
///
/// The vendored crates depend on `tree-sitter-language`, not on
/// `tree-sitter`, so nothing in cargo's resolution notices when a grammar's
/// generated ABI drifts out of range -- it fails at `set_language`, at run
/// time, on whichever file the user happened to open.
#[test]
fn vendored_grammars_are_abi_compatible_with_workspace_tree_sitter() {
    // (label, vendor directory, language) -- the directory is what the
    // failure message tells the reader to regenerate, and two crates hold
    // more than one grammar.
    let vendored: &[(&str, &str, tree_sitter::Language)] = &[
        ("asm", "tree-sitter-asm", tree_sitter_asm::LANGUAGE.into()),
        (
            "c-sharp",
            "tree-sitter-c-sharp",
            tree_sitter_c_sharp::LANGUAGE.into(),
        ),
        (
            "caddyfile",
            "tree-sitter-caddyfile",
            tree_sitter_caddyfile::LANGUAGE.into(),
        ),
        ("cil", "tree-sitter-cil", tree_sitter_cil::LANGUAGE.into()),
        (
            "coffee",
            "tree-sitter-coffee",
            tree_sitter_coffee::LANGUAGE.into(),
        ),
        ("cpp", "tree-sitter-cpp", tree_sitter_cpp::LANGUAGE.into()),
        (
            "crontab",
            "tree-sitter-crontab",
            tree_sitter_crontab::LANGUAGE.into(),
        ),
        ("css", "tree-sitter-css", tree_sitter_css::LANGUAGE.into()),
        ("csv", "tree-sitter-csv", tree_sitter_csv::LANGUAGE.into()),
        ("cue", "tree-sitter-cue", tree_sitter_cue::LANGUAGE.into()),
        (
            "dhall",
            "tree-sitter-dhall",
            tree_sitter_dhall::LANGUAGE.into(),
        ),
        (
            "ebnf",
            "tree-sitter-ebnf",
            tree_sitter_ebnf::LANGUAGE.into(),
        ),
        (
            "fsharp",
            "tree-sitter-fsharp",
            tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
        ),
        (
            "gitignore",
            "tree-sitter-gitignore",
            tree_sitter_gitignore::LANGUAGE.into(),
        ),
        (
            "haskell",
            "tree-sitter-haskell",
            tree_sitter_haskell::LANGUAGE.into(),
        ),
        (
            "html",
            "tree-sitter-html",
            tree_sitter_html::LANGUAGE.into(),
        ),
        (
            "julia",
            "tree-sitter-julia",
            tree_sitter_julia::LANGUAGE.into(),
        ),
        (
            "just",
            "tree-sitter-just",
            tree_sitter_just::LANGUAGE.into(),
        ),
        ("kdl", "tree-sitter-kdl", tree_sitter_kdl::LANGUAGE.into()),
        (
            "kotlin-sg",
            "tree-sitter-kotlin-sg",
            tree_sitter_kotlin_sg::LANGUAGE.into(),
        ),
        (
            "objc",
            "tree-sitter-objc",
            tree_sitter_objc::LANGUAGE.into(),
        ),
        (
            "ocaml",
            "tree-sitter-ocaml",
            tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        ),
        (
            "ocaml (interface)",
            "tree-sitter-ocaml",
            tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
        ),
        (
            "php",
            "tree-sitter-php",
            tree_sitter_php::LANGUAGE_PHP.into(),
        ),
        (
            "powershell",
            "tree-sitter-powershell",
            tree_sitter_powershell::LANGUAGE.into(),
        ),
        ("ron", "tree-sitter-ron", tree_sitter_ron::LANGUAGE.into()),
        (
            "ruby",
            "tree-sitter-ruby",
            tree_sitter_ruby::LANGUAGE.into(),
        ),
        (
            "rust",
            "tree-sitter-rust",
            tree_sitter_rust::LANGUAGE.into(),
        ),
        (
            "scala",
            "tree-sitter-scala",
            tree_sitter_scala::LANGUAGE.into(),
        ),
        (
            "sequel",
            "tree-sitter-sequel",
            tree_sitter_sequel::LANGUAGE.into(),
        ),
        (
            "spirv",
            "tree-sitter-spirv",
            tree_sitter_spirv::LANGUAGE.into(),
        ),
        (
            "swift",
            "tree-sitter-swift",
            tree_sitter_swift::LANGUAGE.into(),
        ),
        (
            "typescript",
            "tree-sitter-typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ),
        (
            "typescript (tsx)",
            "tree-sitter-typescript",
            tree_sitter_typescript::LANGUAGE_TSX.into(),
        ),
        ("vue", "tree-sitter-vue", tree_sitter_vue::LANGUAGE.into()),
        ("wat", "tree-sitter-wat", tree_sitter_wat::LANGUAGE.into()),
    ];
    for (name, dir, language) in vendored {
        let abi = language.abi_version();
        assert!(
            (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
                .contains(&abi),
            "vendored {name} grammar ABI {abi} is outside the range this tree-sitter \
                 supports ({}..={}); regenerate vendor/{dir} with a newer \
                 tree-sitter-cli",
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION,
        );
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(language)
            .unwrap_or_else(|err| panic!("vendored {name} grammar should load: {err}"));
    }
}

/// The grammars vendored for the small-state retune stay retuned.
///
/// `LARGE_STATE_COUNT` is how many parse states got a dense `ts_parse_table`
/// row -- `SYMBOL_COUNT` * 2 bytes each -- instead of a compact
/// `ts_small_parse_table` entry. It is the one number in a generated parser
/// that says whether the retune survived, and regenerating with a stock
/// tree-sitter-cli silently multiplies it: F# goes from 2,542 back to 9,268,
/// and the release binary grows by about 21 MB with no other symptom. Nothing
/// else in the build would notice, because the parse trees are identical
/// either way -- that is the whole point of the transformation.
///
/// vendor/README.md has the patched-CLI recipe. If a bound here is exceeded
/// because the grammar itself was updated, re-measure and move the number;
/// if it is exceeded because the CLI was not patched, regenerate.
#[test]
fn vendored_grammars_keep_the_small_state_retune() {
    // (grammar directory under vendor/, LARGE_STATE_COUNT as regenerated at
    // TS_SMALL_STATE_THRESHOLD=128)
    const RETUNED: &[(&str, usize)] = &[
        ("tree-sitter-c-sharp", 1981),
        ("tree-sitter-coffee", 42),
        ("tree-sitter-cpp", 845),
        ("tree-sitter-fsharp/fsharp", 2542),
        ("tree-sitter-fsharp/fsharp_signature", 2),
        ("tree-sitter-haskell", 117),
        ("tree-sitter-julia", 4076),
        ("tree-sitter-kotlin-sg", 1431),
        ("tree-sitter-objc", 2356),
        ("tree-sitter-ocaml/grammars/interface", 44),
        ("tree-sitter-ocaml/grammars/ocaml", 59),
        ("tree-sitter-ocaml/grammars/type", 46),
        ("tree-sitter-php/php", 204),
        ("tree-sitter-php/php_only", 185),
        ("tree-sitter-powershell", 83),
        ("tree-sitter-ruby", 741),
        ("tree-sitter-rust", 67),
        ("tree-sitter-scala", 504),
        ("tree-sitter-sequel", 2),
        ("tree-sitter-swift", 802),
        ("tree-sitter-typescript/tsx", 176),
        ("tree-sitter-typescript/typescript", 166),
    ];

    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor");
    for (grammar, expected_large_states) in RETUNED {
        let parser = vendor.join(grammar).join("src").join("parser.c");
        // The #defines are in the first few lines; these files run to tens of
        // megabytes, so read a prefix rather than the whole thing.
        let mut head = String::new();
        {
            use std::io::Read as _;
            let file = std::fs::File::open(&parser)
                .unwrap_or_else(|err| panic!("{} should exist: {err}", parser.display()));
            std::io::BufReader::new(file)
                .take(4096)
                .read_to_string(&mut head)
                .unwrap_or_else(|err| panic!("{} should be readable: {err}", parser.display()));
        }

        let large_states = head
            .lines()
            .find_map(|line| line.strip_prefix("#define LARGE_STATE_COUNT "))
            .and_then(|count| count.trim().parse::<usize>().ok())
            .unwrap_or_else(|| panic!("{} should define LARGE_STATE_COUNT", parser.display()));

        assert!(
            large_states <= *expected_large_states,
            "vendor/{grammar} has LARGE_STATE_COUNT {large_states}, above the {expected_large_states} \
                 it was vendored with -- it looks regenerated with a stock tree-sitter-cli. \
                 See vendor/README.md; the fix is to regenerate with \
                 TS_SMALL_STATE_THRESHOLD=128 against a patched tree-sitter-generate."
        );
    }
}

#[test]
fn vendored_asm_grammar_is_abi_compatible_with_workspace_tree_sitter() {
    let asm: tree_sitter::Language = tree_sitter_asm::LANGUAGE.into();
    let abi = asm.abi_version();
    assert!(
        (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&abi),
        "vendored asm grammar ABI {abi} is outside the range this tree-sitter supports \
             ({}..={}); regenerate vendor/tree-sitter-asm with a newer tree-sitter-cli",
        tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
        tree_sitter::LANGUAGE_VERSION,
    );
}

/// The two reasons vendor/tree-sitter-asm exists rather than the crates.io
/// crate, asserted from this side of the boundary.
///
/// `tree-sitter test` in that directory covers the same ground against the
/// grammar's own corpus; this covers it against the grammar GitComet
/// actually links, so a `cargo update` or a botched regeneration that
/// reverted either edit fails here rather than silently in a diff view.
#[test]
fn vendored_asm_grammar_parses_dotted_mnemonics_and_directives() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_asm::LANGUAGE.into())
        .expect("the vendored asm grammar should load");
    let source = concat!(
        ".section .text\n",
        ".LBB0_1:\n",
        "    .p2align 4\n",
        "    b.eq .LBB0_1\n",
        "main:\n",
        "    ret\n",
    );
    let tree = parser.parse(source, None).expect("asm should parse");
    assert!(
        !tree.root_node().has_error(),
        "upstream's `word` cannot span the dot in `b.eq`, and its `meta_ident` \
             is lowercase-only, so both of those lines error without the vendored \
             grammar's edits: {}",
        tree.root_node().to_sexp(),
    );
    assert!(
        tree.root_node().to_sexp().contains("mnemonic"),
        "the `mnemonic` token is what makes `b.eq` one node: {}",
        tree.root_node().to_sexp(),
    );
}

#[test]
fn vendored_vue_grammar_parses_with_workspace_tree_sitter() {
    let source = concat!(
        "<template>\n",
        "  <p v-if=\"ok\">{{ msg }}</p>\n",
        "</template>\n",
        "\n",
        "<script setup lang=\"ts\">\n",
        "const msg: string = 'hi';\n",
        "</script>\n",
    );
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_vue::LANGUAGE.into())
        .expect("vendored Vue grammar should load into the workspace tree-sitter");
    let tree = parser
        .parse(source, None)
        .expect("vendored Vue grammar should parse an SFC");
    assert!(
        !tree.root_node().has_error(),
        "vendored Vue grammar produced an ERROR node for a well-formed SFC: {}",
        tree.root_node().to_sexp(),
    );
}

/// Every language the Vue injections name has to be a language this
/// repository actually ships a grammar for, or the injection silently
/// no-ops. Reading the targets back off the compiled query keeps this
/// honest when the query changes.
#[test]
fn vue_injection_targets_resolve_to_working_grammars() {
    let lang: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, VUE_INJECTIONS_QUERY)
        .expect("vendored Vue injections.scm should compile");

    let mut checked = 0;
    for pattern_ix in 0..query.pattern_count() {
        for setting in query.property_settings(pattern_ix) {
            if setting.key.as_ref() != "injection.language" {
                continue;
            }
            let Some(value) = setting.value.as_deref() else {
                continue;
            };
            let language = diff_syntax_language_for_code_fence_info(value).unwrap_or_else(|| {
                panic!("vue_injections.scm names an unknown injection language {value:?}")
            });
            assert!(
                tree_sitter_highlight_spec(language).is_some(),
                "vue_injections.scm injects {value:?}, but {language:?} has no grammar wired up",
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 5,
        "expected several `#set! injection.language` targets in vue_injections.scm, found {checked}"
    );
}

/// The point of `request_highlight_spec_warmup` is to keep the expensive
/// specs off the render path, and the expensive one a `.vue` file reaches is
/// TypeScript (~86ms cold to compile, against Vue's own ~3ms). The warm-up
/// discovers its targets by walking `#set! injection.language` on the
/// compiled query, so a query edit that moved TypeScript behind an
/// `@injection.language` capture would silently stop warming it and put the
/// stall back. Assert it stays reachable the way the warm-up can see it.
#[test]
fn vue_spec_warmup_reaches_typescript_through_a_set_directive() {
    let lang: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, VUE_INJECTIONS_QUERY)
        .expect("vendored Vue injections.scm should compile");

    let mut warmable = Vec::new();
    for pattern_ix in 0..query.pattern_count() {
        for setting in query.property_settings(pattern_ix) {
            if setting.key.as_ref() != "injection.language" {
                continue;
            }
            if let Some(language) = setting
                .value
                .as_deref()
                .and_then(diff_syntax_language_for_code_fence_info)
            {
                warmable.push(language);
            }
        }
    }

    assert!(
        warmable.contains(&DiffSyntaxLanguage::TypeScript),
        "TypeScript must stay reachable from a `#set! injection.language` in \
             vue_injections.scm, or the warm-up cannot pre-build the one spec that \
             actually costs anything. Reachable targets: {warmable:?}"
    );
    for language in warmable {
        assert!(
            tree_sitter_highlight_spec(language).is_some(),
            "the warm-up would try to build {language:?}, which has no grammar wired up",
        );
    }
}

/// The warm-up runs on its own thread and races the render path by design.
/// This is a smoke test for the plumbing: repeated requests must be cheap and
/// must not deadlock against `OnceLock::get_or_init` on this thread.
#[test]
fn highlight_spec_warmup_requests_are_idempotent() {
    for _ in 0..3 {
        for language in [
            DiffSyntaxLanguage::Vue,
            DiffSyntaxLanguage::Html,
            DiffSyntaxLanguage::Markdown,
            DiffSyntaxLanguage::Rust,
        ] {
            request_highlight_spec_warmup(language);
        }
    }

    // Touching a warmed language from this thread must still return a spec,
    // whether this thread or the warm-up thread won the race.
    assert!(tree_sitter_highlight_spec(DiffSyntaxLanguage::Vue).is_some());
    assert!(tree_sitter_highlight_spec(DiffSyntaxLanguage::TypeScript).is_some());
}

/// `lang="…"` values are read out of the document at runtime, so unlike the
/// `#set!` targets above they cannot be enumerated from the query. Drive
/// them end to end instead: build a real SFC for each value and check the
/// block actually came out highlighted. Asserting on
/// `diff_syntax_language_for_code_fence_info` alone would not do -- that is
/// only half the path, and it would keep passing if the query rule that
/// forwards the attribute were deleted.
#[test]
fn vue_lang_attribute_values_highlight_their_block() {
    // (lang, block body, tag, kind the injected grammar must produce)
    let cases: &[(&str, &str, &str, SyntaxTokenKind)] = &[
        (
            "css",
            ".a { color: red; }",
            "style",
            SyntaxTokenKind::Property,
        ),
        (
            "scss",
            ".a { color: red; }",
            "style",
            SyntaxTokenKind::Property,
        ),
        (
            "less",
            ".a { color: red; }",
            "style",
            SyntaxTokenKind::Property,
        ),
        (
            "postcss",
            ".a { color: red; }",
            "style",
            SyntaxTokenKind::Property,
        ),
        (
            "sass",
            ".a { color: red; }",
            "style",
            SyntaxTokenKind::Property,
        ),
        (
            "ts",
            "const value = 42;",
            "script",
            SyntaxTokenKind::Keyword,
        ),
        (
            "js",
            "const value = 42;",
            "script",
            SyntaxTokenKind::Keyword,
        ),
        (
            "tsx",
            "const value = 42;",
            "script",
            SyntaxTokenKind::Keyword,
        ),
        (
            "jsx",
            "const value = 42;",
            "script",
            SyntaxTokenKind::Keyword,
        ),
        // Not in any `#any-of?` list anywhere: it resolves purely through the
        // shared alias table, which is the whole point of forwarding the
        // attribute verbatim rather than enumerating values in the query.
        (
            "typescript",
            "const value = 42;",
            "script",
            SyntaxTokenKind::Keyword,
        ),
        (
            "mts",
            "const value = 42;",
            "script",
            SyntaxTokenKind::Keyword,
        ),
    ];

    for (lang, body, tag, expected) in cases {
        let text = format!("<{tag} lang=\"{lang}\">\n{body}\n</{tag}>");
        let doc = prepare_test_document(DiffSyntaxLanguage::Vue, &text);
        let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
            .unwrap_or_else(|| panic!("`lang=\"{lang}\"` body should have prepared tokens"));
        assert!(
            tokens.iter().any(|t| t.kind == *expected),
            "`<{tag} lang=\"{lang}\">` should inject a grammar producing {expected:?}, \
                 got: {tokens:?}"
        );
    }
}

/// The failure mode this guards is specific: the html base rules are vetoed
/// by `#not-match? "\\slang\\s*="`, so a `lang` the vue rules do not handle
/// used to lose the fallback *and* match nothing, leaving the block with no
/// highlighting whatsoever.
#[test]
fn vue_unknown_lang_attribute_does_not_silently_disable_highlighting() {
    // A value no grammar in this repo can serve: no injection is the correct
    // outcome, and the surrounding markup must keep working. (Asserting only
    // `is_some()` here would be vacuous -- a blank block returns `Some(vec![])`.)
    let doc = prepare_vue_document(&["<script lang=\"coffee\">", "x = 1", "</script>"]);
    let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
        .expect("the opening tag should have prepared tokens");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "an unservable lang must not disturb the host grammar: {tokens:?}"
    );

    // …and the servable-but-unenumerated case really does highlight.
    let doc = prepare_vue_document(&["<style lang=\"pcss\">", ".a { color: red; }", "</style>"]);
    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("style body should have prepared tokens");
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "`lang=\"pcss\"` resolves to Css through the alias table and must highlight: \
             {tokens:?}"
    );
}

/// The tree-sitter parser is a thread-local reused across languages, with a
/// fast path that skips `set_language`. Adding a grammar that is loaded a
/// different way (vendored, not from crates.io) should not disturb that.
#[test]
fn vue_documents_interleave_with_other_language_documents() {
    let vue = prepare_vue_document(VUE_SFC_FIXTURE);
    let html = prepare_html_document(&["<style>", "body { color: red; }", "</style>"]);
    let vue_again = prepare_vue_document(VUE_SFC_FIXTURE);

    assert!(
        syntax_tokens_for_prepared_document_line(html, 1)
            .expect("html line tokens should be available")
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Property),
        "an HTML document prepared after a Vue one should still highlight"
    );
    for doc in [vue, vue_again] {
        assert!(
            syntax_tokens_for_prepared_document_line(doc, 7)
                .expect("vue script line tokens should be available")
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Keyword),
            "Vue documents should still highlight when interleaved with other languages"
        );
    }
}

#[test]
fn vendored_vue_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    tree_sitter::Query::new(&lang, VUE_HIGHLIGHTS_QUERY)
        .expect("vendored Vue highlights.scm should compile");
}

#[test]
fn vendored_vue_injections_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    tree_sitter::Query::new(&lang, VUE_INJECTIONS_QUERY)
        .expect("vendored Vue injections.scm should compile");
}

/// vue_highlights.scm inlines queries/html_highlights.scm, because the Vue
/// grammar inherits html and TreesitterQueryAsset takes a single source.
/// Nothing structural keeps the copy honest, so an edit to the html file
/// that is not mirrored here would silently skip .vue files.
///
/// The check is order-sensitive on purpose. Rule order is load-bearing:
/// `normalize_non_overlapping_tokens` resolves overlaps last-capture-wins,
/// so the html `(attribute_value) @string` rule has to come *before* the
/// `@variable` directive override for the override to take effect. A
/// per-line containment check would pass on a reordered copy.
#[test]
fn vue_highlights_query_embeds_the_html_base_verbatim() {
    let html_rules = query_rule_lines(HTML_HIGHLIGHTS_QUERY);
    let vue_rules = query_rule_lines(VUE_HIGHLIGHTS_QUERY);
    assert!(
        !html_rules.is_empty(),
        "html_highlights.scm should not be comment-only"
    );

    let embedded = vue_rules
        .windows(html_rules.len())
        .any(|window| window == html_rules.as_slice());
    assert!(
        embedded,
        "vue_highlights.scm must contain queries/html_highlights.scm as a contiguous, \
             in-order block -- the Vue grammar inherits html, and rule order decides which \
             capture wins. Mirror the change into the `--- html base ---` section.\n\
             expected block:\n{html_rules:#?}\nvue rules:\n{vue_rules:#?}"
    );
}

/// `configure_query_cursor` caps in-progress matches at TS_QUERY_MATCH_LIMIT
/// and nothing consults `did_exceed_match_limit`, so an overflow silently
/// drops injections. The Vue injection query has the most patterns of any in
/// the repo and anchors several on very common template nodes, which makes
/// it the one most likely to hit the cap.
#[test]
fn vue_injection_query_stays_under_the_match_limit_on_a_dense_template() {
    let mut lines = vec!["<template>".to_string(), "  <ul>".to_string()];
    for ix in 0..120 {
        lines.push(format!(
            "    <li v-if=\"n{ix} > {ix}\" :key=\"k{ix}\" :class=\"[a{ix}, b{ix}]\" \
                 @click.stop=\"pick{ix}($event)\" #row=\"{{ v{ix} }}\">{{{{ n{ix} + 1 }}}}</li>"
        ));
    }
    lines.push("  </ul>".to_string());
    lines.push("</template>".to_string());
    let text = lines.join("\n");

    let lang: tree_sitter::Language = tree_sitter_vue::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, VUE_INJECTIONS_QUERY)
        .expect("vendored Vue injections.scm should compile");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang)
        .expect("vendored Vue grammar should load");
    let tree = parser
        .parse(&text, None)
        .expect("dense template should parse");

    let mut cursor = tree_sitter::QueryCursor::new();
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    let mut matched = 0usize;
    {
        let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
        tree_sitter::StreamingIterator::advance(&mut matches);
        while matches.get().is_some() {
            matched += 1;
            tree_sitter::StreamingIterator::advance(&mut matches);
        }
    }

    assert!(
        !cursor.did_exceed_match_limit(),
        "the Vue injection query overflowed the {TS_QUERY_MATCH_LIMIT}-match in-progress \
             pool on a {}-line template; tree-sitter discards matches on overflow, so some \
             directives and interpolations would silently lose highlighting",
        lines.len(),
    );
    assert!(
        matched > 0,
        "the dense template should produce injection matches at all"
    );
}

#[test]
fn vendored_javascript_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
    tree_sitter::Query::new(&lang, JAVASCRIPT_HIGHLIGHTS_QUERY)
        .expect("vendored JavaScript highlights.scm should compile against JS grammar");
}

#[test]
fn vendored_javascript_injections_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
    tree_sitter::Query::new(&lang, JAVASCRIPT_INJECTIONS_QUERY)
        .expect("vendored JavaScript injections.scm should compile against JS grammar");
}

#[test]
fn vendored_typescript_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    tree_sitter::Query::new(&lang, TYPESCRIPT_HIGHLIGHTS_QUERY)
        .expect("vendored TypeScript highlights.scm should compile");
}

#[test]
fn vendored_typescript_injections_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    tree_sitter::Query::new(&lang, TYPESCRIPT_INJECTIONS_QUERY)
        .expect("vendored TypeScript injections.scm should compile");
}

#[test]
fn vendored_tsx_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    tree_sitter::Query::new(&lang, TSX_HIGHLIGHTS_QUERY)
        .expect("vendored TSX highlights.scm should compile");
}

#[test]
fn vendored_tsx_injections_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    tree_sitter::Query::new(&lang, TSX_INJECTIONS_QUERY)
        .expect("vendored TSX injections.scm should compile");
}

#[test]
fn vendored_go_queries_compile() {
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    tree_sitter::Query::new(&lang, GO_HIGHLIGHTS_QUERY)
        .expect("vendored Go highlights.scm should compile");
    tree_sitter::Query::new(&lang, GO_INJECTIONS_QUERY)
        .expect("vendored Go injections.scm should compile");
}

#[test]
fn vendored_json_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_json::LANGUAGE.into();
    tree_sitter::Query::new(&lang, JSON_HIGHLIGHTS_QUERY)
        .expect("vendored JSON highlights.scm should compile");
}

#[test]
fn vendored_python_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    tree_sitter::Query::new(&lang, PYTHON_HIGHLIGHTS_QUERY)
        .expect("vendored Python highlights.scm should compile");
}

#[test]
fn vendored_yaml_queries_compile() {
    let lang: tree_sitter::Language = tree_sitter_yaml::LANGUAGE.into();
    tree_sitter::Query::new(&lang, YAML_HIGHLIGHTS_QUERY)
        .expect("vendored YAML highlights.scm should compile");
    tree_sitter::Query::new(&lang, YAML_INJECTIONS_QUERY)
        .expect("vendored YAML injections.scm should compile");
}

#[test]
fn vendored_csharp_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    tree_sitter::Query::new(&lang, CSHARP_HIGHLIGHTS_QUERY)
        .expect("vendored C# highlights.scm should compile");
}

#[test]
fn vendored_c_queries_compile() {
    let lang: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
    tree_sitter::Query::new(&lang, C_HIGHLIGHTS_QUERY)
        .expect("vendored C highlights.scm should compile");
    tree_sitter::Query::new(&lang, C_INJECTIONS_QUERY)
        .expect("vendored C injections.scm should compile");
}

#[test]
fn vendored_cpp_queries_compile() {
    let lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
    tree_sitter::Query::new(&lang, CPP_HIGHLIGHTS_QUERY)
        .expect("vendored C++ highlights.scm should compile");
    tree_sitter::Query::new(&lang, CPP_INJECTIONS_QUERY)
        .expect("vendored C++ injections.scm should compile");
}

#[test]
fn vendored_injected_web_language_queries_compile() {
    let jsdoc_lang: tree_sitter::Language = tree_sitter_jsdoc::LANGUAGE.into();
    tree_sitter::Query::new(&jsdoc_lang, JSDOC_HIGHLIGHTS_QUERY)
        .expect("vendored JSDoc highlights.scm should compile");

    let regex_lang: tree_sitter::Language = tree_sitter_regex::LANGUAGE.into();
    tree_sitter::Query::new(&regex_lang, REGEX_HIGHLIGHTS_QUERY)
        .expect("vendored regex highlights.scm should compile");
}

#[test]
fn vendored_repo_queries_compile() {
    let markdown_lang: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
    tree_sitter::Query::new(&markdown_lang, MARKDOWN_HIGHLIGHTS_QUERY)
        .expect("Markdown block highlights.scm should compile");
    tree_sitter::Query::new(&markdown_lang, MARKDOWN_INJECTIONS_QUERY)
        .expect("Markdown block injections.scm should compile");

    let markdown_inline_lang: tree_sitter::Language = tree_sitter_md::INLINE_LANGUAGE.into();
    tree_sitter::Query::new(&markdown_inline_lang, MARKDOWN_INLINE_HIGHLIGHTS_QUERY)
        .expect("Markdown inline highlights.scm should compile");

    let diff_lang: tree_sitter::Language = tree_sitter_diff::LANGUAGE.into();
    tree_sitter::Query::new(&diff_lang, tree_sitter_diff::HIGHLIGHTS_QUERY)
        .expect("Diff highlights.scm should compile");

    let gitcommit_lang: tree_sitter::Language = tree_sitter_gitcommit::LANGUAGE.into();
    tree_sitter::Query::new(&gitcommit_lang, GITCOMMIT_HIGHLIGHTS_QUERY)
        .expect("Git commit highlights.scm should compile");

    let gomod_lang: tree_sitter::Language = tree_sitter_gomod::LANGUAGE.into();
    tree_sitter::Query::new(&gomod_lang, GOMOD_HIGHLIGHTS_QUERY)
        .expect("go.mod highlights.scm should compile");

    let gowork_lang: tree_sitter::Language = tree_sitter_gowork::LANGUAGE.into();
    tree_sitter::Query::new(&gowork_lang, GOWORK_HIGHLIGHTS_QUERY)
        .expect("go.work highlights.scm should compile");
}

#[test]
fn vendored_xml_query_compiles() {
    let lang: tree_sitter::Language = tree_sitter_xml::LANGUAGE_XML.into();
    tree_sitter::Query::new(&lang, XML_HIGHLIGHTS_QUERY)
        .expect("XML highlights.scm should compile against XML grammar");
}

#[test]
fn xml_treesitter_captures_tag_and_attribute() {
    let text = r#"<root attr="value">text</root>"#;
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
        "XML should capture tags: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
        "XML should capture attributes as properties: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::String),
        "XML should capture attribute values as strings: {tokens:?}"
    );
}

#[test]
fn xml_treesitter_captures_comment() {
    let text = "<!-- a comment -->";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
        "XML should capture comments: {tokens:?}"
    );
}

#[test]
fn javascript_treesitter_captures_function_and_keyword() {
    let text = "function foo() { return 42; }";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::JavaScript, DiffSyntaxMode::Auto);
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Function),
        "JS should capture function names: {tokens:?}"
    );
    assert!(
        tokens.iter().any(
            |t| t.kind == SyntaxTokenKind::Keyword || t.kind == SyntaxTokenKind::KeywordControl
        ),
        "JS should capture keywords: {tokens:?}"
    );
    assert!(
        tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
        "JS should capture numbers: {tokens:?}"
    );
}

#[test]
fn typescript_treesitter_preserves_arrow_operator_inside_arrow_function() {
    let text = "const fn = (x: number): number => x + 1;";
    let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::TypeScript, DiffSyntaxMode::Auto);
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Function && &text[t.range.clone()] == "fn"),
        "TypeScript should capture the arrow function name, got: {tokens:?}"
    );
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Operator && &text[t.range.clone()] == "=>"),
        "TypeScript should preserve the fat-arrow operator, got: {tokens:?}"
    );
}

#[test]
fn html_highlight_spec_compiles_injection_query() {
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html)
        .expect("HTML highlight spec should exist");
    assert!(
        spec.injection_query.is_some(),
        "HTML should compile and retain its vendored injections.scm"
    );
}

#[test]
fn javascript_highlight_spec_compiles_injection_query() {
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::JavaScript)
        .expect("JavaScript highlight spec should exist");
    assert!(
        spec.injection_query.is_some(),
        "JavaScript should compile and retain its injections.scm"
    );
}

fn capture_name_is_intentionally_ignored(name: &str) -> bool {
    // PowerShell tags `(array_expression)` `@array`, which spans the whole
    // `@(1, 2)` including the parens and the spaces between elements. There
    // is no kind that span should take -- colouring it at all swallows the
    // elements' own colours -- so it is left unmapped on purpose.
    name == "array"
        || name == "none"
        || name == "clean"
        || name == "assignvalue"
        || name == "embedded"
        || name == "error"
        || name == "nested"
        || name == "spell"
        || name == "injection.content"
        || name.starts_with("text.")
        || name.starts_with('_')
}

fn assert_capture_names_are_supported(language: tree_sitter::Language, source: &str) {
    let query = tree_sitter::Query::new(&language, source).expect("query should compile");
    for name in query.capture_names() {
        assert!(
            syntax_kind_from_capture_name(name).is_some()
                || capture_name_is_intentionally_ignored(name),
            "unsupported capture name in vendored asset: {name}"
        );
    }
}

#[test]
fn vendored_capture_names_are_supported_or_ignored() {
    assert_capture_names_are_supported(tree_sitter_rust::LANGUAGE.into(), RUST_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_html::LANGUAGE.into(), HTML_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_vue::LANGUAGE.into(), VUE_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_css::LANGUAGE.into(), CSS_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_bash::LANGUAGE.into(), BASH_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_javascript::LANGUAGE.into(),
        JAVASCRIPT_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_python::LANGUAGE.into(),
        PYTHON_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_go::LANGUAGE.into(), GO_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_json::LANGUAGE.into(), JSON_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_yaml::LANGUAGE.into(), YAML_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TYPESCRIPT_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        TSX_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_xml::LANGUAGE_XML.into(), XML_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_c::LANGUAGE.into(), C_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_cpp::LANGUAGE.into(), CPP_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_c_sharp::LANGUAGE.into(),
        CSHARP_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_java::LANGUAGE.into(),
        tree_sitter_java::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_php::LANGUAGE_PHP.into(),
        tree_sitter_php::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_ruby::LANGUAGE.into(), RUBY_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_toml_ng::LANGUAGE.into(),
        tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_lua::LANGUAGE.into(),
        tree_sitter_lua::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_make::LANGUAGE.into(),
        tree_sitter_make::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_kotlin_sg::LANGUAGE.into(),
        tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_zig::LANGUAGE.into(), ZIG_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        dekobon_tree_sitter_groovy::LANGUAGE.into(),
        dekobon_tree_sitter_groovy::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_clojure_orchard::LANGUAGE.into(),
        CLOJURE_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_elixir::LANGUAGE.into(),
        tree_sitter_elixir::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_erlang::LANGUAGE.into(),
        tree_sitter_erlang::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_haskell::LANGUAGE.into(),
        HASKELL_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_julia::LANGUAGE.into(), JULIA_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        OCAML_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_solidity::LANGUAGE.into(),
        SOLIDITY_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_asm::LANGUAGE.into(), ASM_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_svelte_ng::LANGUAGE.into(),
        SVELTE_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_bicep::LANGUAGE.into(),
        tree_sitter_bicep::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_objc::LANGUAGE.into(),
        tree_sitter_objc::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
        tree_sitter_fsharp::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_powershell::LANGUAGE.into(),
        POWERSHELL_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_swift::LANGUAGE.into(),
        tree_sitter_swift::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_jsdoc::LANGUAGE.into(), JSDOC_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(tree_sitter_regex::LANGUAGE.into(), REGEX_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_r::LANGUAGE.into(),
        tree_sitter_r::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_dart::LANGUAGE.into(),
        tree_sitter_dart::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_scala::LANGUAGE.into(),
        tree_sitter_scala::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_sequel::LANGUAGE.into(),
        tree_sitter_sequel::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_md::LANGUAGE.into(), MARKDOWN_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_md::INLINE_LANGUAGE.into(),
        MARKDOWN_INLINE_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_diff::LANGUAGE.into(),
        tree_sitter_diff::HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(
        tree_sitter_gitcommit::LANGUAGE.into(),
        GITCOMMIT_HIGHLIGHTS_QUERY,
    );
    assert_capture_names_are_supported(tree_sitter_gomod::LANGUAGE.into(), GOMOD_HIGHLIGHTS_QUERY);
    assert_capture_names_are_supported(
        tree_sitter_gowork::LANGUAGE.into(),
        GOWORK_HIGHLIGHTS_QUERY,
    );
}
