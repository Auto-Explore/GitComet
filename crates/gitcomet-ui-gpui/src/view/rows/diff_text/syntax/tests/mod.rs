use super::*;
use std::time::{Duration, Instant};

/// Serializes tests that reset or assert on the shared syntax instrumentation
/// counters. Without this lock, concurrent tests can reset or bump those
/// counters while another test is asserting on them, causing flaky failures
/// under parallel test execution.
static GLOBAL_COUNTER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_global_counter_tests() -> std::sync::MutexGuard<'static, ()> {
    match GLOBAL_COUNTER_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn assert_token_ranges_are_utf8_safe(text: &str, tokens: &[SyntaxToken]) {
    for token in tokens {
        assert!(
            token.range.start <= token.range.end,
            "{token:?} in {text:?}"
        );
        assert!(token.range.end <= text.len(), "{token:?} in {text:?}");
        assert!(
            text.is_char_boundary(token.range.start),
            "{token:?} start is not a char boundary in {text:?}"
        );
        assert!(
            text.is_char_boundary(token.range.end),
            "{token:?} end is not a char boundary in {text:?}"
        );
    }
}

fn has_token_kind_and_text(
    text: &str,
    tokens: &[SyntaxToken],
    kind: SyntaxTokenKind,
    expected: &str,
) -> bool {
    tokens.iter().any(|token| {
        token.kind == kind
            && token.range.end <= text.len()
            && &text[token.range.clone()] == expected
    })
}

struct TempFileBackedLineFixture {
    path: std::path::PathBuf,
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
}

impl TempFileBackedLineFixture {
    fn new(name: &str, text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gitcomet_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be monotonic enough for test temp path")
                .as_nanos()
        ));
        std::fs::write(&path, text.as_bytes()).expect("write streamed slice fixture");
        let raw_text = gitcomet_core::file_diff::FileDiffLineText::file_slice(
            Arc::new(path.clone()),
            0..text.len(),
            false,
            false,
        );
        Self { path, raw_text }
    }
}

impl Drop for TempFileBackedLineFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn wait_for_background_chunk_build_for_document(
    document: PreparedSyntaxDocument,
    timeout: Duration,
) -> usize {
    let started = Instant::now();
    loop {
        let applied = drain_completed_prepared_syntax_chunk_builds_for_document(document);
        if applied > 0 {
            return applied;
        }
        if started.elapsed() >= timeout {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_all_background_chunk_builds_for_document(
    document: PreparedSyntaxDocument,
    timeout: Duration,
) -> usize {
    let started = Instant::now();
    let mut total_applied = 0usize;
    loop {
        let applied = drain_completed_prepared_syntax_chunk_builds_for_document(document);
        total_applied = total_applied.saturating_add(applied);
        if !has_pending_prepared_syntax_chunk_builds_for_document(document) {
            return total_applied;
        }
        if started.elapsed() >= timeout {
            return total_applied;
        }
        if applied == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn reset_ts_parser_test_state() {
    TS_PARSER.with(|parser| {
        *parser.borrow_mut() = tree_sitter::Parser::new();
    });
    TS_CURSOR.with(|cursor| {
        *cursor.borrow_mut() = tree_sitter::QueryCursor::new();
    });
    TS_INPUT.with(|input| input.borrow_mut().clear());
    TS_LINE_TOKEN_CACHE.with(|cache| {
        *cache.borrow_mut() = SingleLineSyntaxTokenCache::new();
    });
    TS_PARSER_REQUIRES_LANGUAGE_RESET.with(|needs_reset| needs_reset.set(false));
    TS_PARSER_SET_LANGUAGE_CALL_COUNT.with(|count| count.set(0));
}

fn ts_parser_set_language_call_count() -> usize {
    TS_PARSER_SET_LANGUAGE_CALL_COUNT.with(Cell::get)
}

fn with_silenced_panic_hook<R>(f: impl FnOnce() -> R) -> R {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = f();
    std::panic::set_hook(previous_hook);
    result
}

/// Incremental reparsing must leave the injection cache one document deep.
///
/// Prefix injections used to be *copied* under the new document hash while
/// the originals stayed resident, and nothing ever reclaimed a superseded
/// hash -- so every edit to a template-heavy document grew the cache until
/// `TS_INJECTION_CACHE_MAX_ENTRIES` forced the LRU to drop half of it,
/// including entries the next chunk build wanted.
#[test]
fn incremental_reparse_leaves_one_document_in_the_injection_cache() {
    let make = |suffix: &str| -> String {
        let mut text = String::new();
        for ix in 0..12 {
            text.push_str(&format!(
                "## Section {ix}\n\n```rust\nfn item{ix}() {{ let v = {ix}; }}\n```\n\n"
            ));
        }
        text.push_str(suffix);
        text
    };

    prepared::clear_injection_cache_for_tests();
    let mut document = prepare_test_document(DiffSyntaxLanguage::Markdown, &make("tail 0\n"));
    for line_ix in 0..80 {
        let _ = syntax_tokens_for_prepared_document_line(document, line_ix);
    }
    let (baseline, by_hash) = prepared::injection_cache_occupancy_by_document_hash_for_tests();
    assert!(
        baseline > 1,
        "the fixture must actually cache injections, got {baseline}"
    );
    assert_eq!(by_hash.len(), 1, "one document parsed, one hash");

    for step in 1..=6 {
        let text = make(&format!("tail {step}\n"));
        match prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Markdown,
            &text,
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(500),
            },
            Some(document),
        ) {
            PrepareTreesitterDocumentResult::Ready(next) => document = next,
            other => panic!("reparse {step} should succeed, got {other:?}"),
        }
        for line_ix in 0..80 {
            let _ = syntax_tokens_for_prepared_document_line(document, line_ix);
        }

        let (total, by_hash) = prepared::injection_cache_occupancy_by_document_hash_for_tests();
        assert_eq!(
            by_hash.len(),
            1,
            "reparse {step} left {} document hashes in the cache: {by_hash:?}",
            by_hash.len()
        );
        assert_eq!(
            total, baseline,
            "reparse {step} changed occupancy from {baseline} to {total}; the cache                  must not grow with the number of edits"
        );
    }
}

fn prepare_test_document(language: DiffSyntaxLanguage, text: &str) -> PreparedSyntaxDocument {
    let input = treesitter_document_input_from_text(text);
    match prepare_treesitter_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        SharedString::from(text.to_owned()),
        input.line_starts,
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(200),
        },
        None,
        None,
    ) {
        PrepareTreesitterDocumentResult::Ready(doc) => doc,
        other => panic!("test document should parse successfully, got {other:?}"),
    }
}

fn prepare_test_document_from_shared_text(
    language: DiffSyntaxLanguage,
    text: &str,
) -> PreparedSyntaxDocument {
    let input = treesitter_document_input_from_text(text);
    let prepared = prepare_treesitter_document_in_background_text_with_reuse(
        language,
        DiffSyntaxMode::Auto,
        SharedString::from(text.to_owned()),
        input.line_starts,
        None,
        None,
    )
    .expect("shared-text test document should parse successfully");
    inject_prepared_document_data(prepared)
}

fn prepare_test_document_with_budget_reuse(
    language: DiffSyntaxLanguage,
    text: &str,
    budget: DiffSyntaxBudget,
    old_document: Option<PreparedSyntaxDocument>,
) -> PrepareTreesitterDocumentResult {
    let input = treesitter_document_input_from_text(text);
    prepare_treesitter_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        SharedString::from(text.to_owned()),
        input.line_starts,
        budget,
        old_document,
        None,
    )
}

fn prepare_test_document_in_background(
    language: DiffSyntaxLanguage,
    text: &str,
) -> Option<PreparedSyntaxDocumentData> {
    let input = treesitter_document_input_from_text(text);
    prepare_treesitter_document_in_background_text_with_reuse(
        language,
        DiffSyntaxMode::Auto,
        SharedString::from(text.to_owned()),
        input.line_starts,
        None,
        None,
    )
}

fn prepare_html_document(lines: &[&str]) -> PreparedSyntaxDocument {
    prepare_test_document(DiffSyntaxLanguage::Html, &lines.join("\n"))
}

fn prepare_markdown_document(lines: &[&str]) -> PreparedSyntaxDocument {
    prepare_test_document(DiffSyntaxLanguage::Markdown, &lines.join("\n"))
}

fn prepare_vue_document(lines: &[&str]) -> PreparedSyntaxDocument {
    prepare_test_document(DiffSyntaxLanguage::Vue, &lines.join("\n"))
}

/// Kinds of every token overlapping `fragment` within `line_ix`. Token ranges
/// on a prepared document are line-relative, including tokens remapped back
/// from an injection, so this works across the injection boundary.
fn token_kinds_for_line_fragment(
    doc: PreparedSyntaxDocument,
    line_ix: usize,
    line_text: &str,
    fragment: &str,
) -> Vec<SyntaxTokenKind> {
    let start = line_text
        .find(fragment)
        .unwrap_or_else(|| panic!("fragment {fragment:?} should appear in {line_text:?}"));
    let end = start + fragment.len();
    syntax_tokens_for_prepared_document_line(doc, line_ix)
        .unwrap_or_else(|| panic!("line {line_ix} tokens should be available"))
        .iter()
        .filter(|token| token.range.start < end && token.range.end > start)
        .map(|token| token.kind)
        .collect()
}

fn heuristic_tokens(text: &str, language: DiffSyntaxLanguage) -> Vec<SyntaxToken> {
    syntax_tokens_for_line(text, language, DiffSyntaxMode::HeuristicOnly).to_vec()
}

fn heuristic_string_spans(text: &str, language: DiffSyntaxLanguage) -> Vec<&str> {
    heuristic_tokens(text, language)
        .into_iter()
        .filter(|token| token.kind == SyntaxTokenKind::String)
        .map(|token| &text[token.range])
        .collect()
}

/// The keyword and keyword-control spans a line yields on the heuristic path.
///
/// Shared rather than redefined per test: the three copies this replaced drifted
/// apart on whether `KeywordControl` counted.
fn heuristic_keywords(text: &str, language: DiffSyntaxLanguage) -> Vec<&str> {
    syntax_tokens_for_line(text, language, DiffSyntaxMode::HeuristicOnly)
        .iter()
        .filter(|token| {
            matches!(
                token.kind,
                SyntaxTokenKind::Keyword | SyntaxTokenKind::KeywordControl
            )
        })
        .map(|token| &text[token.range.clone()])
        .collect()
}

/// A query's rule lines, with blanks and `;` comments dropped.
///
/// Used by the three `..._embeds_the_..._base_verbatim` tripwires. They compare
/// vendored copies against their upstream, so all three have to strip comments
/// the same way or the comparison means different things in each.
fn query_rule_lines(query: &str) -> Vec<&str> {
    query
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.trim_start().starts_with(';'))
        .collect()
}

/// Every `DiffSyntaxLanguage` variant, listed by hand.
///
/// Deliberately not derived from the enum: the point is that adding a variant
/// breaks a test until someone states what the new language does, rather than
/// being silently swept into whatever the loop asserts.
fn all_supported_languages() -> Vec<DiffSyntaxLanguage> {
    Vec::from([
        DiffSyntaxLanguage::Markdown,
        DiffSyntaxLanguage::MarkdownInline,
        DiffSyntaxLanguage::Html,
        DiffSyntaxLanguage::Vue,
        DiffSyntaxLanguage::Svelte,
        DiffSyntaxLanguage::Jinja,
        DiffSyntaxLanguage::Css,
        DiffSyntaxLanguage::Hcl,
        DiffSyntaxLanguage::Bicep,
        DiffSyntaxLanguage::Lua,
        DiffSyntaxLanguage::Makefile,
        DiffSyntaxLanguage::Nix,
        DiffSyntaxLanguage::Kotlin,
        DiffSyntaxLanguage::Zig,
        DiffSyntaxLanguage::Groovy,
        DiffSyntaxLanguage::Clojure,
        DiffSyntaxLanguage::Elixir,
        DiffSyntaxLanguage::Erlang,
        DiffSyntaxLanguage::Haskell,
        DiffSyntaxLanguage::Julia,
        DiffSyntaxLanguage::OCaml,
        DiffSyntaxLanguage::OCamlInterface,
        DiffSyntaxLanguage::Solidity,
        DiffSyntaxLanguage::Assembly,
        DiffSyntaxLanguage::Rust,
        DiffSyntaxLanguage::Python,
        DiffSyntaxLanguage::JavaScript,
        DiffSyntaxLanguage::Jsdoc,
        DiffSyntaxLanguage::TypeScript,
        DiffSyntaxLanguage::Tsx,
        DiffSyntaxLanguage::Regex,
        DiffSyntaxLanguage::Go,
        DiffSyntaxLanguage::GoMod,
        DiffSyntaxLanguage::GoWork,
        DiffSyntaxLanguage::C,
        DiffSyntaxLanguage::Cpp,
        DiffSyntaxLanguage::ObjectiveC,
        DiffSyntaxLanguage::CSharp,
        DiffSyntaxLanguage::FSharp,
        DiffSyntaxLanguage::VisualBasic,
        DiffSyntaxLanguage::Java,
        DiffSyntaxLanguage::Php,
        DiffSyntaxLanguage::Ruby,
        DiffSyntaxLanguage::PowerShell,
        DiffSyntaxLanguage::Swift,
        DiffSyntaxLanguage::R,
        DiffSyntaxLanguage::Dart,
        DiffSyntaxLanguage::Scala,
        DiffSyntaxLanguage::Perl,
        DiffSyntaxLanguage::Json,
        DiffSyntaxLanguage::Toml,
        DiffSyntaxLanguage::Yaml,
        DiffSyntaxLanguage::Sql,
        DiffSyntaxLanguage::Diff,
        DiffSyntaxLanguage::GitCommit,
        DiffSyntaxLanguage::Bash,
        DiffSyntaxLanguage::Xml,
        DiffSyntaxLanguage::Cmake,
        DiffSyntaxLanguage::Dockerfile,
        DiffSyntaxLanguage::Ini,
        DiffSyntaxLanguage::Llvm,
        DiffSyntaxLanguage::Conf,
        DiffSyntaxLanguage::Just,
        DiffSyntaxLanguage::Caddyfile,
        DiffSyntaxLanguage::Gitignore,
        DiffSyntaxLanguage::Wat,
        DiffSyntaxLanguage::Spirv,
        DiffSyntaxLanguage::Crontab,
        DiffSyntaxLanguage::Cil,
        DiffSyntaxLanguage::JavaProperties,
        DiffSyntaxLanguage::Jsonnet,
        DiffSyntaxLanguage::Proto,
        DiffSyntaxLanguage::Gleam,
        DiffSyntaxLanguage::V,
        DiffSyntaxLanguage::Pascal,
        DiffSyntaxLanguage::Csv,
        DiffSyntaxLanguage::Kdl,
        DiffSyntaxLanguage::Ron,
        DiffSyntaxLanguage::Cue,
        DiffSyntaxLanguage::Ebnf,
        DiffSyntaxLanguage::Dhall,
        DiffSyntaxLanguage::CoffeeScript,
        DiffSyntaxLanguage::Dotenv,
    ])
}

fn prepare_nix_document(lines: &[&str]) -> PreparedSyntaxDocument {
    prepare_test_document(DiffSyntaxLanguage::Nix, &lines.join("\n"))
}

fn prepare_jinja_document(lines: &[&str]) -> PreparedSyntaxDocument {
    prepare_test_document(DiffSyntaxLanguage::Jinja, &lines.join("\n"))
}

/// A parsed-but-empty tree, for the LRU tests that build cache entries by
/// hand and care only about the eviction order.
fn empty_injection_tree() -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .expect("javascript should load");
    parser.parse("", None).expect("empty source should parse")
}

mod batch;
mod batch_regression;
mod chunk_cache;
mod combined_injections;
mod extra_languages;
mod grammar_compat;
mod heuristic;
mod injection_cache;
mod injections;
mod language;
mod nix_jinja;
mod normalization;
mod occurrences;
mod pairs;
mod prepared;
mod prepared_documents;
mod reparse;
