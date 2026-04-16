use super::super::*;
use gpui::SharedString;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet, FxHasher};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use tree_sitter::StreamingIterator;

const TS_DOCUMENT_CACHE_MAX_ENTRIES: usize = 8;
const TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS: usize = 64;
const TS_DOCUMENT_LINE_TOKEN_PREFETCH_GUARD_CHUNKS: usize = 1;
const DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST: Duration = Duration::from_millis(1);
const DIFF_SYNTAX_FOREGROUND_SKIP_TEXT_BYTES: usize = 128 * 1024;
const DIFF_SYNTAX_FOREGROUND_SKIP_LINE_COUNT: usize = 2_048;
const TS_QUERY_MATCH_LIMIT: u32 = 256;
const TS_MAX_BYTES_TO_QUERY: usize = 16 * 1024;
const TS_QUERY_MAX_LINES_PER_PASS: usize = 256;
const TS_DEFERRED_DROP_MIN_BYTES: usize = 256 * 1024;
const TS_INCREMENTAL_REPARSE_ENABLE_ENV: &str = "GITCOMET_DIFF_SYNTAX_INCREMENTAL_REPARSE";
const TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES: usize = 64 * 1024;
const TS_INCREMENTAL_REPARSE_MAX_CHANGED_PERCENT: usize = 35;
const TS_INCREMENTAL_REPARSE_LATE_EDIT_MIN_PREFIX_BYTES: usize = 8 * 1024;
const TS_INCREMENTAL_REPARSE_LATE_EDIT_MAX_CHANGED_BYTES: usize = 384 * 1024;
const TS_INCREMENTAL_REPARSE_LATE_EDIT_MAX_CHANGED_PERCENT: usize = 80;
const TS_LINE_TOKEN_CACHE_MAX_ENTRIES: usize = 256;
// Extreme multi-megabyte documents are better served by the existing visible-line
// heuristic fallback than by building a full prepared tree-sitter document.
const TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES: usize = 64;
const TS_PENDING_PARSE_REQUEST_MAX_ENTRIES: usize = 8;
#[cfg(any(test, feature = "syntax-shell"))]
const BASH_HIGHLIGHTS_QUERY: &str = include_str!("queries/bash_highlights.scm");
#[cfg(any(test, feature = "syntax-extra"))]
const CSHARP_HIGHLIGHTS_QUERY: &str = include_str!("queries/csharp_highlights.scm");
#[cfg(any(test, feature = "syntax-repo"))]
const GITCOMMIT_HIGHLIGHTS_QUERY: &str = include_str!("queries/gitcommit_highlights.scm");
#[cfg(any(test, feature = "syntax-repo"))]
const GOMOD_HIGHLIGHTS_QUERY: &str = include_str!("queries/gomod_highlights.scm");
#[cfg(any(test, feature = "syntax-repo"))]
const GOWORK_HIGHLIGHTS_QUERY: &str = include_str!("queries/gowork_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const HTML_HIGHLIGHTS_QUERY: &str = include_str!("queries/html_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const HTML_INJECTIONS_QUERY: &str = include_str!("queries/html_injections.scm");
#[cfg(any(test, feature = "syntax-repo"))]
const MARKDOWN_HIGHLIGHTS_QUERY: &str = tree_sitter_md::HIGHLIGHT_QUERY_BLOCK;
#[cfg(any(test, feature = "syntax-repo"))]
const MARKDOWN_INJECTIONS_QUERY: &str = tree_sitter_md::INJECTION_QUERY_BLOCK;
#[cfg(any(test, feature = "syntax-repo"))]
const MARKDOWN_INLINE_HIGHLIGHTS_QUERY: &str = tree_sitter_md::HIGHLIGHT_QUERY_INLINE;
#[cfg(any(test, feature = "syntax-web"))]
const CSS_HIGHLIGHTS_QUERY: &str = include_str!("queries/css_highlights.scm");
#[cfg(any(test, feature = "syntax-go"))]
const GO_HIGHLIGHTS_QUERY: &str = include_str!("queries/go_highlights.scm");
#[cfg(any(test, feature = "syntax-go"))]
const GO_INJECTIONS_QUERY: &str = include_str!("queries/go_injections.scm");
#[cfg(any(test, feature = "syntax-web"))]
const JAVASCRIPT_HIGHLIGHTS_QUERY: &str = include_str!("queries/javascript_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const JAVASCRIPT_INJECTIONS_QUERY: &str = tree_sitter_javascript::INJECTIONS_QUERY;
#[cfg(any(test, feature = "syntax-data"))]
const JSON_HIGHLIGHTS_QUERY: &str = include_str!("queries/json_highlights.scm");
#[cfg(any(test, feature = "syntax-extra"))]
const POWERSHELL_HIGHLIGHTS_QUERY: &str = tree_sitter_powershell::HIGHLIGHTS_QUERY;
#[cfg(any(test, feature = "syntax-python"))]
const PYTHON_HIGHLIGHTS_QUERY: &str = include_str!("queries/python_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const TYPESCRIPT_HIGHLIGHTS_QUERY: &str = include_str!("queries/typescript_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const TYPESCRIPT_INJECTIONS_QUERY: &str = include_str!("queries/typescript_injections.scm");
#[cfg(any(test, feature = "syntax-web"))]
const TSX_HIGHLIGHTS_QUERY: &str = include_str!("queries/tsx_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const TSX_INJECTIONS_QUERY: &str = include_str!("queries/tsx_injections.scm");
#[cfg(any(test, feature = "syntax-rust"))]
const RUST_HIGHLIGHTS_QUERY: &str = include_str!("queries/rust_highlights.scm");
#[cfg(any(test, feature = "syntax-rust"))]
const RUST_INJECTIONS_QUERY: &str = include_str!("queries/rust_injections.scm");
#[cfg(any(test, feature = "syntax-data"))]
const YAML_HIGHLIGHTS_QUERY: &str = include_str!("queries/yaml_highlights.scm");
#[cfg(any(test, feature = "syntax-data"))]
const YAML_INJECTIONS_QUERY: &str = include_str!("queries/yaml_injections.scm");
#[cfg(any(test, feature = "syntax-xml"))]
const XML_HIGHLIGHTS_QUERY: &str = tree_sitter_xml::XML_HIGHLIGHT_QUERY;
#[cfg(any(test, feature = "syntax-extra"))]
const CPP_INJECTIONS_QUERY: &str = include_str!("queries/cpp_injections.scm");

/// Maximum injection nesting depth. Root document = 0, first injection = 1.
/// This prevents infinite recursion if an injected language's highlight spec
/// itself contains an injection query.
const TS_MAX_INJECTION_DEPTH: usize = 1;
const TS_INJECTION_CACHE_MAX_ENTRIES: usize = 32;

thread_local! {
    static TS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new(tree_sitter::Parser::new());
    static TS_PARSER_REQUIRES_LANGUAGE_RESET: Cell<bool> = const { Cell::new(false) };
    static TS_CURSOR: RefCell<tree_sitter::QueryCursor> = RefCell::new(tree_sitter::QueryCursor::new());
    static TS_INPUT: RefCell<String> = const { RefCell::new(String::new()) };
    static TS_DOCUMENT_CACHE: RefCell<TreesitterDocumentCache> = RefCell::new(TreesitterDocumentCache::new());
    static TS_LINE_TOKEN_CACHE: RefCell<SingleLineSyntaxTokenCache> = RefCell::new(SingleLineSyntaxTokenCache::new());
    static TS_INJECTION_CACHE: RefCell<HashMap<TreesitterInjectionMatch, CachedInjectionTokens>> = RefCell::new(HashMap::default());
    static TS_PENDING_PARSE_REQUESTS: RefCell<Vec<PendingParseRequest>> = const { RefCell::new(Vec::new()) };
    static TS_INJECTION_ACCESS_COUNTER: Cell<u64> = const { Cell::new(0) };
    static TS_INJECTION_DEPTH: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static TS_PARSER_SET_LANGUAGE_CALL_COUNT: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static TS_TREE_STATE_CLONE_COUNT: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static TS_INCREMENTAL_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static TS_INCREMENTAL_FALLBACK_COUNT: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    static TS_DOCUMENT_HASH_COUNT: Cell<usize> = const { Cell::new(0) };
}

fn invalidate_ts_parser_language_fast_path() {
    TS_PARSER_REQUIRES_LANGUAGE_RESET.with(|needs_reset| needs_reset.set(true));
}

fn catch_treesitter_query_panic<R>(f: impl FnOnce() -> R) -> Option<R> {
    // Upstream tree-sitter can panic during query predicate evaluation when a
    // recovered node reports a byte range that extends past the provided text.
    // Treat those as syntax-miss fallbacks instead of crashing the UI.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => Some(result),
        Err(_) => {
            TS_CURSOR.with(|cursor| {
                *cursor.borrow_mut() = tree_sitter::QueryCursor::new();
            });
            invalidate_ts_parser_language_fast_path();
            None
        }
    }
}

fn ascii_lowercase_for_match(s: &str) -> Cow<'_, str> {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(s.to_ascii_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

fn with_ts_parser<R>(
    ts_language: &tree_sitter::Language,
    f: impl FnOnce(&mut tree_sitter::Parser) -> R,
) -> Option<R> {
    TS_PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        let needs_language_reset =
            TS_PARSER_REQUIRES_LANGUAGE_RESET.with(|needs_reset| needs_reset.replace(false));
        let parser_language_matches = parser
            .language()
            .as_deref()
            .is_some_and(|current| current == ts_language);

        if needs_language_reset || !parser_language_matches {
            #[cfg(test)]
            TS_PARSER_SET_LANGUAGE_CALL_COUNT.with(|count| count.set(count.get() + 1));
            if parser.set_language(ts_language).is_err() {
                invalidate_ts_parser_language_fast_path();
                return None;
            }
        }
        Some(f(&mut parser))
    })
}

fn with_ts_parser_parse_result<R>(
    ts_language: &tree_sitter::Language,
    f: impl FnOnce(&mut tree_sitter::Parser) -> Option<R>,
) -> Option<R> {
    let result = with_ts_parser(ts_language, f).flatten();
    if result.is_none() {
        invalidate_ts_parser_language_fast_path();
    }
    result
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum DiffSyntaxLanguage {
    Markdown,
    MarkdownInline,
    Html,
    Css,
    Hcl,
    Bicep,
    Lua,
    Makefile,
    Kotlin,
    Zig,
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    GoMod,
    GoWork,
    C,
    Cpp,
    ObjectiveC,
    CSharp,
    FSharp,
    VisualBasic,
    Java,
    Php,
    Ruby,
    PowerShell,
    Swift,
    R,
    Dart,
    Scala,
    Perl,
    Json,
    Toml,
    Yaml,
    Sql,
    Diff,
    GitCommit,
    Bash,
    Xml,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum DiffSyntaxMode {
    Auto,
    HeuristicOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffSyntaxEdit {
    pub old_range: Range<usize>,
    pub new_range: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SyntaxToken {
    pub(super) range: Range<usize>,
    pub(super) kind: SyntaxTokenKind,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct PreparedSyntaxDocument {
    cache_key: PreparedSyntaxCacheKey,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PreparedSyntaxCacheKey {
    language: DiffSyntaxLanguage,
    doc_hash: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PreparedSyntaxSourceIdentity {
    language: DiffSyntaxLanguage,
    text_ptr: usize,
    text_len: usize,
    line_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SingleLineSyntaxTokenCacheKey {
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text_hash: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TreesitterParseReuseMode {
    Full,
    Incremental,
}

#[derive(Clone, Debug)]
struct PreparedSyntaxTreeState {
    language: DiffSyntaxLanguage,
    text: SharedString,
    line_starts: Arc<[usize]>,
    source_hash: u64,
    source_version: u64,
    tree: tree_sitter::Tree,
    #[cfg(test)]
    parse_mode: TreesitterParseReuseMode,
}

#[derive(Clone, Debug)]
pub(super) struct PreparedSyntaxDocumentData {
    cache_key: PreparedSyntaxCacheKey,
    line_count: usize,
    line_token_chunks: HashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    tree_state: Option<PreparedSyntaxTreeState>,
}

#[derive(Clone, Debug)]
pub(super) struct PreparedSyntaxReparseSeed {
    document: PreparedSyntaxDocument,
    tree_state: PreparedSyntaxTreeState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffSyntaxBudget {
    pub foreground_parse: Duration,
}

impl Default for DiffSyntaxBudget {
    fn default() -> Self {
        Self {
            foreground_parse: crate::ui_runtime::current().diff_syntax_foreground_parse_budget(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PrepareTreesitterDocumentResult {
    Ready(PreparedSyntaxDocument),
    TimedOut,
    Unsupported,
}

mod heuristic;
mod language;
mod prepared;

use heuristic::*;
use language::*;
use prepared::*;

pub(super) use heuristic::syntax_tokens_for_line_heuristic_into;
#[cfg(test)]
pub(super) use language::syntax_tokens_for_line;
pub(super) use language::syntax_tokens_for_line_shared;
pub(in crate::view) use language::{
    diff_syntax_language_for_code_fence_info, diff_syntax_language_for_path,
};
#[cfg(test)]
pub(super) use prepared::syntax_tokens_for_prepared_document_line;
pub(super) use prepared::{
    PreparedSyntaxLineTokensRequest, drain_completed_prepared_syntax_chunk_builds,
    drain_completed_prepared_syntax_chunk_builds_for_document,
    has_pending_prepared_syntax_chunk_builds,
    has_pending_prepared_syntax_chunk_builds_for_document, inject_prepared_document_data,
    prepare_treesitter_document_in_background_text_with_reparse_seed,
    prepare_treesitter_document_with_budget_reuse_text, prepared_document_reparse_seed,
    request_syntax_tokens_for_prepared_document_line,
    request_syntax_tokens_for_prepared_document_line_range_into,
};
#[cfg(feature = "benchmarks")]
pub(super) use prepared::{
    benchmark_cache_replacement_drop_step, benchmark_drop_payload_timed_step,
    benchmark_flush_deferred_drop_queue, benchmark_prepared_syntax_cache_contains_document,
    benchmark_prepared_syntax_cache_metrics, benchmark_prepared_syntax_loaded_chunk_count,
    benchmark_reset_prepared_syntax_cache_metrics,
};
#[cfg(test)]
pub(super) use prepared::{prepared_document_parse_mode, prepared_document_source_version};

#[cfg(test)]
pub(super) fn reset_prepared_syntax_cache() {
    prepared::reset_prepared_syntax_cache();
}

pub(super) fn syntax_tokens_for_streamed_line_slice_heuristic(
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    language: DiffSyntaxLanguage,
    requested_slice_range: Range<usize>,
    resolved_slice_range: Range<usize>,
) -> Option<Vec<SyntaxToken>> {
    heuristic::syntax_tokens_for_streamed_line_slice_heuristic(
        raw_text,
        language,
        requested_slice_range,
        resolved_slice_range,
    )
}

#[cfg(test)]
pub(super) fn reset_streamed_heuristic_line_cache() {
    heuristic::reset_streamed_heuristic_line_cache();
}

#[cfg(test)]
mod tests {
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
                token.kind == SyntaxTokenKind::String
                    && token.range.start == 0
                    && token.range.end > 64
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
                token.kind == SyntaxTokenKind::String
                    && token.range.start == 0
                    && token.range.end > 64
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

        let rust =
            syntax_tokens_for_line(rust_line, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        let json =
            syntax_tokens_for_line(json_line, DiffSyntaxLanguage::Json, DiffSyntaxMode::Auto);

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
    fn parser_fast_path_reuses_same_language_until_switch() {
        reset_ts_parser_test_state();

        let rust_tokens =
            syntax_tokens_for_line_treesitter("fn main() { let x = 1; }", DiffSyntaxLanguage::Rust)
                .expect("first rust parse should succeed");
        assert!(!rust_tokens.is_empty());
        assert_eq!(ts_parser_set_language_call_count(), 1);

        let rust_tokens_again = syntax_tokens_for_line_treesitter(
            "fn helper() { let y = 2; }",
            DiffSyntaxLanguage::Rust,
        )
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

        let reparsed = syntax_tokens_for_line_treesitter(
            "fn helper() { let y = 2; }",
            DiffSyntaxLanguage::Rust,
        )
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

        let reparsed = syntax_tokens_for_line_treesitter(
            "fn helper() { let y = 2; }",
            DiffSyntaxLanguage::Rust,
        )
        .expect("rust parse should recover after parser slot reset");
        assert!(
            reparsed
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Keyword),
            "rust parse after parser slot reset should still contain keyword highlights: {reparsed:?}"
        );
        assert_eq!(ts_parser_set_language_call_count(), 2);
    }

    #[cfg(any(test, feature = "syntax-xml"))]
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

        let auto_again =
            syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
        assert_eq!(auto_again, auto);
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

        let html_again =
            syntax_tokens_for_line(text, DiffSyntaxLanguage::Html, DiffSyntaxMode::Auto);
        assert_eq!(html_again, html);
    }

    #[cfg(any(test, feature = "syntax-xml"))]
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
    fn prepared_markdown_document_highlights_inline_code_and_html_block() {
        let doc =
            prepare_markdown_document(&["Use `git status` here", "<div class=\"note\">ok</div>"]);

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

    #[test]
    fn prepared_document_cache_keeps_multiple_documents_available() {
        let first_doc = prepare_test_document(DiffSyntaxLanguage::Rust, "/* one */ let a = 1;");
        let second_doc = prepare_test_document(DiffSyntaxLanguage::Rust, "/* two */ let b = 2;");

        let first_tokens = syntax_tokens_for_prepared_document_line(first_doc, 0)
            .expect("first prepared document should remain in cache");
        let second_tokens = syntax_tokens_for_prepared_document_line(second_doc, 0)
            .expect("second prepared document should be in cache");

        assert!(
            first_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Comment),
            "first document should keep its tokens available"
        );
        assert!(
            second_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Comment),
            "second document should keep its tokens available"
        );
    }

    #[test]
    fn prepared_document_tokens_are_chunked_and_materialized_lazily() {
        // The prepared-document cache is thread-local and persists across tests on the same worker
        // thread, so clear it before asserting exact miss/hit behavior.
        reset_prepared_syntax_cache();
        let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 3))
            .map(|ix| format!("let value_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            0,
            "prepared document should start with no chunk materialization"
        );

        let _ = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("first line tokens should resolve");
        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            1,
            "first lookup should materialize one chunk"
        );
        let after_first_lookup = prepared_syntax_cache_metrics();
        assert_eq!(after_first_lookup.miss, 1);
        assert_eq!(after_first_lookup.hit, 0);

        let _ = syntax_tokens_for_prepared_document_line(document, 1)
            .expect("same-chunk lookup should resolve");
        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            1,
            "same chunk lookup should reuse cached chunk"
        );
        let after_second_lookup = prepared_syntax_cache_metrics();
        assert_eq!(after_second_lookup.miss, 1);
        assert_eq!(after_second_lookup.hit, 1);

        let _ =
            syntax_tokens_for_prepared_document_line(document, TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
                .expect("next-chunk lookup should resolve");
        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            2,
            "lookup on next chunk boundary should build one additional chunk"
        );
        let after_third_lookup = prepared_syntax_cache_metrics();
        assert_eq!(after_third_lookup.miss, 2);
        assert_eq!(after_third_lookup.hit, 1);
        assert!(
            after_third_lookup.chunk_build_ms >= after_first_lookup.chunk_build_ms,
            "chunk build metric should accumulate monotonically"
        );
    }

    #[test]
    fn prepared_document_chunk_request_builds_in_background() {
        let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
            .map(|ix| format!("let value_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            0,
            "prepared document should start with no chunk materialization"
        );
        assert_eq!(
            request_syntax_tokens_for_prepared_document_line(document, 0),
            Some(PreparedSyntaxLineTokensRequest::Pending),
            "first request should enqueue a background chunk build"
        );
        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            0,
            "pending request should not materialize the chunk synchronously"
        );
        assert!(
            has_pending_prepared_syntax_chunk_builds(),
            "background chunk request should remain pending until drained"
        );

        assert!(
            wait_for_all_background_chunk_builds_for_document(document, Duration::from_secs(2)) > 0,
            "background chunk builds should complete within timeout"
        );
        assert_eq!(
            prepared_syntax_loaded_chunk_count(document),
            2,
            "first visible miss should also prefetch the adjacent chunk"
        );

        let ready = request_syntax_tokens_for_prepared_document_line(document, 0);
        match ready {
            Some(PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
                assert!(
                    tokens
                        .iter()
                        .any(|token| token.kind == SyntaxTokenKind::Keyword),
                    "ready chunk should expose syntax tokens"
                );
            }
            other => panic!("expected ready tokens after background chunk build, got {other:?}"),
        }
        let prefetched = request_syntax_tokens_for_prepared_document_line(
            document,
            TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS,
        );
        match prefetched {
            Some(PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
                assert!(
                    tokens
                        .iter()
                        .any(|token| token.kind == SyntaxTokenKind::Keyword),
                    "adjacent prefetched chunk should already be ready"
                );
            }
            other => panic!("expected prefetched adjacent chunk to be ready, got {other:?}"),
        }
        assert!(
            !has_pending_prepared_syntax_chunk_builds(),
            "drained chunk request should clear pending state"
        );
    }

    #[test]
    fn prepared_document_chunk_prefetch_shares_one_tree_state_clone() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        reset_prepared_syntax_cache();
        let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
            .map(|ix| format!("let value_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

        let clones_before_request = tree_state_clone_count();
        assert_eq!(
            request_syntax_tokens_for_prepared_document_line(document, 0),
            Some(PreparedSyntaxLineTokensRequest::Pending),
            "first request should enqueue the visible chunk and its prefetched neighbor"
        );
        assert_eq!(
            tree_state_clone_count(),
            clones_before_request.saturating_add(1),
            "the queued chunk burst should share one cloned tree state"
        );
    }

    #[test]
    fn document_scoped_chunk_drain_preserves_other_documents() {
        let lines_a = (0..TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
            .map(|ix| format!("let alpha_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let lines_b = (0..TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
            .map(|ix| format!("let beta_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document_a = prepare_test_document(DiffSyntaxLanguage::Rust, &lines_a.join("\n"));
        let document_b = prepare_test_document(DiffSyntaxLanguage::Rust, &lines_b.join("\n"));

        assert_eq!(
            request_syntax_tokens_for_prepared_document_line(document_a, 0),
            Some(PreparedSyntaxLineTokensRequest::Pending)
        );
        assert_eq!(
            request_syntax_tokens_for_prepared_document_line(document_b, 0),
            Some(PreparedSyntaxLineTokensRequest::Pending)
        );
        assert!(has_pending_prepared_syntax_chunk_builds_for_document(
            document_a
        ));
        assert!(has_pending_prepared_syntax_chunk_builds_for_document(
            document_b
        ));

        assert!(
            wait_for_background_chunk_build_for_document(document_a, Duration::from_secs(2)) > 0,
            "document-scoped drain should eventually apply the requested chunk"
        );
        assert_eq!(prepared_syntax_loaded_chunk_count(document_a), 1);
        assert_eq!(
            prepared_syntax_loaded_chunk_count(document_b),
            0,
            "draining document_a should not materialize document_b"
        );
        assert!(!has_pending_prepared_syntax_chunk_builds_for_document(
            document_a
        ));
        assert!(
            has_pending_prepared_syntax_chunk_builds_for_document(document_b),
            "other document work should remain pending"
        );

        assert!(
            wait_for_background_chunk_build_for_document(document_b, Duration::from_secs(2)) > 0,
            "remaining document chunk should still be drainable afterward"
        );
        assert_eq!(prepared_syntax_loaded_chunk_count(document_b), 1);
        assert!(!has_pending_prepared_syntax_chunk_builds_for_document(
            document_b
        ));
    }

    #[test]
    fn prepared_document_chunk_hit_does_not_clone_tree_state() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        reset_prepared_syntax_cache();
        let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
            .map(|ix| format!("let chunk_clone_probe_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

        let _ = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("first miss should resolve and build first chunk");
        let clones_after_miss = tree_state_clone_count();
        assert!(
            clones_after_miss >= 1,
            "chunk miss should clone tree state for chunk build"
        );

        let _ = syntax_tokens_for_prepared_document_line(document, 1)
            .expect("same-chunk hit should resolve");
        assert_eq!(
            tree_state_clone_count(),
            clones_after_miss,
            "chunk-hit lookup should not clone tree state"
        );
    }

    #[test]
    fn prepared_tree_state_clones_share_source_buffers() {
        let lines = (0..128usize)
            .map(|ix| format!("let value_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &lines.join("\n"));

        let (first, second) = TS_DOCUMENT_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let first = cache
                .tree_state(document.cache_key)
                .expect("first tree state clone should exist");
            let second = cache
                .tree_state(document.cache_key)
                .expect("second tree state clone should exist");
            (first, second)
        });

        assert!(
            first.text.as_ptr() == second.text.as_ptr() && first.text.len() == second.text.len(),
            "tree state clones should share source text storage"
        );
        assert!(
            Arc::ptr_eq(&first.line_starts, &second.line_starts),
            "tree state clones should share line start storage"
        );
    }

    #[test]
    fn shared_text_input_reuses_snapshot_line_start_storage() {
        let snapshot = crate::kit::text_model::TextModel::from("alpha\nbeta\ngamma").snapshot();
        let shared_line_starts = snapshot.shared_line_starts();
        let input = treesitter_document_input_from_shared_text(
            snapshot.as_shared_string(),
            shared_line_starts.clone(),
        );

        assert!(
            Arc::ptr_eq(&input.line_starts, &shared_line_starts),
            "full-text tree-sitter input should reuse snapshot line-start storage"
        );
        assert_eq!(input.line_starts.as_ref(), snapshot.line_starts());
    }

    #[test]
    fn collected_input_last_line_content_excludes_trailing_newline() {
        let input = treesitter_document_input_from_text("alpha\nbeta");

        assert_eq!(
            line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 0),
            5
        );
        assert_eq!(
            line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 1),
            input.text.len(),
            "text-built input should not include trailing content beyond the last line"
        );
    }

    #[test]
    fn shared_text_input_last_line_content_excludes_trailing_newline() {
        let snapshot = crate::kit::text_model::TextModel::from("alpha\nbeta\n").snapshot();
        let text_input = treesitter_document_input_from_text("alpha\nbeta\n");
        let input = treesitter_document_input_from_shared_text(
            snapshot.as_shared_string(),
            snapshot.shared_line_starts(),
        );

        assert_eq!(
            input.line_starts.as_ref(),
            text_input.line_starts.as_ref(),
            "shared full-text input should normalize trailing-newline line starts to the same shape as collected text input"
        );
        assert_eq!(
            line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 1),
            input.text.len() - 1,
            "shared full-text input should trim the real trailing newline from the last line"
        );
    }

    #[test]
    fn shared_text_input_preserves_real_empty_last_line_while_trimming_phantom_entry() {
        let source = "alpha\n\n";
        let snapshot = crate::kit::text_model::TextModel::from(source).snapshot();
        let input = treesitter_document_input_from_shared_text(
            snapshot.as_shared_string(),
            snapshot.shared_line_starts(),
        );

        assert_eq!(
            snapshot.line_starts(),
            &[0, 6, source.len()],
            "snapshot line starts should still include the text-model phantom trailing entry"
        );
        assert_eq!(
            input.line_starts.as_ref(),
            &[0, 6],
            "tree-sitter input should keep the real empty last line but drop the phantom trailing entry"
        );
        assert_eq!(
            line_content_end_byte(input.line_starts.as_ref(), input.text.as_bytes(), 1),
            source.len() - 1,
            "the empty last line should end before the terminal newline byte"
        );
    }

    #[test]
    fn treesitter_document_cache_lru_touch_keeps_recent_entry_alive() {
        for trial in 0..128usize {
            let mut cache = TreesitterDocumentCache::new();
            for key in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
                cache.insert_document(
                    TreesitterDocumentCache::make_test_cache_key(key as u64),
                    vec![Vec::new()],
                );
            }

            let touched_key = TreesitterDocumentCache::make_test_cache_key(0);
            assert!(cache.contains_document(touched_key, 1));
            cache.insert_document(
                TreesitterDocumentCache::make_test_cache_key(10_000 + trial as u64),
                vec![Vec::new()],
            );

            assert!(
                cache.contains_key(touched_key),
                "touched key should survive eviction on trial {trial}"
            );
        }
    }

    #[test]
    fn warm_shared_text_prepare_reuses_source_identity_without_rehashing() {
        let _lock = lock_global_counter_tests();
        reset_prepared_syntax_cache();
        reset_deferred_drop_counters();

        let source = vec!["fn warm_identity() { let value = Some(42); }"; 512].join("\n");
        let text: SharedString = source.clone().into();
        let line_starts = treesitter_document_input_from_text(&source).line_starts;
        let budget = DiffSyntaxBudget {
            foreground_parse: Duration::from_secs(1),
        };

        let first = match prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text.clone(),
            Arc::clone(&line_starts),
            budget,
            None,
            None,
        ) {
            PrepareTreesitterDocumentResult::Ready(document) => document,
            other => panic!("expected prepared document, got {other:?}"),
        };
        let first_hash_count = document_hash_count();
        assert!(
            first_hash_count > 0,
            "initial prepare should still hash the source at least once"
        );

        let second = match prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text,
            line_starts,
            budget,
            None,
            None,
        ) {
            PrepareTreesitterDocumentResult::Ready(document) => document,
            other => panic!("expected warm prepared document, got {other:?}"),
        };

        assert_eq!(second, first);
        assert_eq!(
            document_hash_count(),
            first_hash_count,
            "warm prepare should reuse the source-identity cache hit without rehashing the full text"
        );
    }

    #[test]
    fn cold_prepare_hashes_the_source_only_once_on_cache_miss() {
        let _lock = lock_global_counter_tests();
        reset_prepared_syntax_cache();
        reset_deferred_drop_counters();

        let source = vec!["fn cold_hash_miss() { let value = Some(42); }"; 512].join("\n");
        let text: SharedString = source.clone().into();
        let line_starts = treesitter_document_input_from_text(&source).line_starts;
        let budget = DiffSyntaxBudget {
            foreground_parse: Duration::from_secs(1),
        };

        let document = match prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text,
            line_starts,
            budget,
            None,
            None,
        ) {
            PrepareTreesitterDocumentResult::Ready(document) => document,
            other => panic!("expected prepared document, got {other:?}"),
        };

        assert_eq!(document_hash_count(), 1);
        assert_eq!(prepared_syntax_loaded_chunk_count(document), 0);
    }

    #[test]
    fn timed_out_prepare_reuses_pending_parse_request_in_background_without_rehashing() {
        let _lock = lock_global_counter_tests();
        reset_prepared_syntax_cache();
        reset_deferred_drop_counters();

        let source = vec!["fn background_reuse() { let value = Some(42); }"; 4_096].join("\n");
        let text: SharedString = source.clone().into();
        let line_starts = treesitter_document_input_from_text(&source).line_starts;

        let timed_out = prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text.clone(),
            Arc::clone(&line_starts),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(1),
            },
            None,
            None,
        );
        assert_eq!(timed_out, PrepareTreesitterDocumentResult::TimedOut);
        assert_eq!(
            document_hash_count(),
            1,
            "timed-out foreground prepare should hash once while storing the pending request"
        );

        let background = prepare_treesitter_document_in_background_text_with_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text,
            line_starts,
            None,
            None,
        )
        .expect("background parse should still succeed after foreground timeout");

        assert_eq!(
            document_hash_count(),
            1,
            "background parse should reuse the pending request instead of hashing again"
        );
        assert_eq!(background.line_count, 4_096);
    }

    #[test]
    fn oversized_shared_text_prepare_falls_back_without_prepared_tree_sitter() {
        let _lock = lock_global_counter_tests();
        reset_prepared_syntax_cache();
        reset_deferred_drop_counters();

        let line = "let oversized_value: usize = 1;";
        let repeat = (TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES / (line.len() + 1)).saturating_add(1);
        let source = std::iter::repeat_n(line, repeat)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            source.len() > TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES,
            "fixture should exceed the prepared full-document syntax byte gate"
        );
        let input = treesitter_document_input_from_text(&source);
        let text: SharedString = source.clone().into();

        let attempt = prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text.clone(),
            Arc::clone(&input.line_starts),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_secs(1),
            },
            None,
            None,
        );
        assert_eq!(
            attempt,
            PrepareTreesitterDocumentResult::Unsupported,
            "oversized full-document syntax should fall back before parsing"
        );
        assert_eq!(
            document_hash_count(),
            0,
            "oversized full-document syntax should skip whole-document hash work"
        );

        let background = prepare_treesitter_document_in_background_text_with_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            text,
            input.line_starts,
            None,
            None,
        );
        assert!(
            background.is_none(),
            "background prepared syntax should also skip oversized full-document inputs"
        );
    }

    #[test]
    fn incremental_edit_ranges_cover_the_changed_window() {
        let old = b"alpha\nbeta\ngamma\n";
        let new = b"alpha\nbeta changed\ngamma\n";
        let ranges = compute_incremental_edit_ranges(old, new);
        assert_eq!(
            ranges.len(),
            1,
            "single local edit should produce one edit range"
        );

        let edit = ranges[0];
        let mut rebuilt = Vec::new();
        rebuilt.extend_from_slice(&old[..edit.start_byte]);
        rebuilt.extend_from_slice(&new[edit.start_byte..edit.new_end_byte]);
        rebuilt.extend_from_slice(&old[edit.old_end_byte..]);
        assert_eq!(
            rebuilt.as_slice(),
            new,
            "edit range should reconstruct the new buffer when applied to old bytes"
        );
    }

    #[test]
    fn incremental_reparse_fallback_thresholds_cover_percent_and_absolute_limits() {
        let small_edit = [TreesitterByteEditRange {
            start_byte: 100,
            old_end_byte: 120,
            new_end_byte: 128,
        }];
        assert!(
            !incremental_reparse_should_fallback(&small_edit, 4_000, 4_008),
            "small deltas should stay on incremental path"
        );

        let percent_threshold_edit = [TreesitterByteEditRange {
            start_byte: 0,
            old_end_byte: 2_000,
            new_end_byte: 2_000,
        }];
        assert!(
            incremental_reparse_should_fallback(&percent_threshold_edit, 4_000, 4_000),
            "large percent deltas should force full parse fallback"
        );

        let absolute_threshold_edit = [TreesitterByteEditRange {
            start_byte: 0,
            old_end_byte: TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(8),
            new_end_byte: TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(8),
        }];
        assert!(
            incremental_reparse_should_fallback(
                &absolute_threshold_edit,
                TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(16),
                TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES.saturating_add(16),
            ),
            "absolute changed-byte cap should force full parse fallback"
        );
    }

    #[test]
    fn treesitter_point_for_byte_maps_newline_terminated_eof_to_next_row() {
        let input = b"alpha\nbeta\n";
        let line_starts: Vec<usize> = vec![0, 6];
        assert_eq!(
            treesitter_point_for_byte(&line_starts, input, input.len()),
            tree_sitter::Point::new(2, 0),
            "EOF for newline-terminated input should point to the next row start"
        );
    }

    #[test]
    fn small_reparse_reuses_old_tree_with_input_edit() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        let base_lines = vec!["let value = 1;".to_string(); 256];
        let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));
        let base_version =
            prepared_document_source_version(base_document).expect("base source version");
        assert_eq!(
            prepared_document_parse_mode(base_document),
            Some(TreesitterParseReuseMode::Full)
        );

        let mut edited = base_lines.clone();
        edited[42].push_str(" // tiny edit");
        let attempt = prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            &edited.join("\n"),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("small reparse should complete within default budget");
        };

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental)
        );
        let reparsed_version =
            prepared_document_source_version(reparsed_document).expect("reparsed source version");
        assert!(
            reparsed_version > base_version,
            "incremental reparse should advance source version"
        );

        let (incremental, fallback) = incremental_reparse_counters();
        assert!(
            incremental > 0,
            "small edit should use incremental reparse path"
        );
        assert_eq!(fallback, 0, "small edit should not trigger fallback");
    }

    #[test]
    fn unchanged_reparse_reuses_old_document_without_rehashing() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        reset_prepared_syntax_cache();

        let source = "let value = 1;\n".repeat(256);
        let base_input = treesitter_document_input_from_text(&source);
        let PrepareTreesitterDocumentResult::Ready(base_document) =
            prepare_treesitter_document_with_budget_reuse_text(
                DiffSyntaxLanguage::Rust,
                DiffSyntaxMode::Auto,
                source.clone().into(),
                base_input.line_starts.clone(),
                DiffSyntaxBudget {
                    foreground_parse: Duration::from_millis(50),
                },
                None,
                None,
            )
        else {
            panic!("base text document should parse");
        };

        reset_deferred_drop_counters();
        let repeated_input = treesitter_document_input_from_text(&source);
        let attempt = prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            source.into(),
            repeated_input.line_starts,
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
            None,
        );
        let PrepareTreesitterDocumentResult::Ready(reused_document) = attempt else {
            panic!("unchanged reparse should reuse the existing prepared document");
        };

        assert_eq!(reused_document, base_document);
        assert_eq!(
            document_hash_count(),
            0,
            "unchanged reparses with an old document should not rehash the full source"
        );
    }

    #[test]
    fn small_reparse_without_edit_hint_does_not_rehash_full_source() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        reset_prepared_syntax_cache();

        let base_text = "let value = 1;\n".repeat(256);
        let base_input = treesitter_document_input_from_text(&base_text);
        let PrepareTreesitterDocumentResult::Ready(base_document) =
            prepare_treesitter_document_with_budget_reuse_text(
                DiffSyntaxLanguage::Rust,
                DiffSyntaxMode::Auto,
                base_text.clone().into(),
                base_input.line_starts.clone(),
                DiffSyntaxBudget {
                    foreground_parse: Duration::from_millis(50),
                },
                None,
                None,
            )
        else {
            panic!("base text document should parse");
        };

        let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
        let mut edited_text = base_text;
        edited_text.insert_str(insert_offset, " // tiny edit");
        let edited_input = treesitter_document_input_from_text(&edited_text);

        reset_deferred_drop_counters();
        let attempt = prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited_text.into(),
            edited_input.line_starts,
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
            None,
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("small reparse should complete within budget");
        };

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental)
        );
        assert_eq!(
            document_hash_count(),
            0,
            "small no-hint reparses should reuse the old source fingerprint without hashing the full text"
        );
    }

    #[test]
    fn small_reparse_reuses_cached_prefix_chunks_before_the_edit() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        reset_prepared_syntax_cache();

        let line_count = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 3;
        let base_lines = (0..line_count)
            .map(|ix| format!("let value_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

        let _ = syntax_tokens_for_prepared_document_line(base_document, 0)
            .expect("base document should materialize its first chunk");
        assert_eq!(
            prepared_syntax_loaded_chunk_count(base_document),
            1,
            "base document should only have its first chunk materialized"
        );

        let mut edited = base_lines.clone();
        let edited_line = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2;
        edited[edited_line].push_str(" // tiny edit");
        let attempt = prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            &edited.join("\n"),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("small reparse should complete within budget");
        };

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental),
            "small later-line edit should stay on the incremental path"
        );
        assert_eq!(
            prepared_syntax_loaded_chunk_count(reparsed_document),
            1,
            "cached prefix chunks before the edit should carry forward to the reparsed document"
        );

        benchmark_reset_prepared_syntax_cache_metrics();
        let _ = syntax_tokens_for_prepared_document_line(reparsed_document, 0)
            .expect("reparsed document should reuse the carried prefix chunk");
        let after_prefix_hit = prepared_syntax_cache_metrics();
        assert_eq!(after_prefix_hit.hit, 1);
        assert_eq!(after_prefix_hit.miss, 0);

        let _ = syntax_tokens_for_prepared_document_line(reparsed_document, edited_line)
            .expect("changed chunk should still be buildable on demand");
        let after_changed_lookup = prepared_syntax_cache_metrics();
        assert_eq!(after_changed_lookup.hit, 1);
        assert_eq!(after_changed_lookup.miss, 1);
    }

    #[test]
    fn small_reparse_reuses_old_tree_with_explicit_edit_hint_text_input() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let base_text = "let value = 1;\n".repeat(256);
        let base_input = treesitter_document_input_from_text(&base_text);
        let base_document =
            prepare_test_document_from_shared_text(DiffSyntaxLanguage::Rust, &base_text);

        let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
        let mut edited_text = base_text.clone();
        edited_text.insert_str(insert_offset, " // tiny edit");
        let edited_input = treesitter_document_input_from_text(&edited_text);
        let attempt = prepare_treesitter_document_with_budget_reuse_text(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited_text.into(),
            edited_input.line_starts.clone(),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
            Some(DiffSyntaxEdit {
                old_range: insert_offset..insert_offset,
                new_range: insert_offset..insert_offset.saturating_add(" // tiny edit".len()),
            }),
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("explicit-edit text reparse should complete within budget");
        };

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental),
            "explicit edit hints should keep full-text reparses on the incremental path"
        );

        let (incremental, fallback) = incremental_reparse_counters();
        assert!(
            incremental > 0,
            "explicit edit hint path should use incremental reparse"
        );
        assert_eq!(
            fallback, 0,
            "explicit edit hint should not trigger fallback"
        );
    }

    #[test]
    fn large_reparse_falls_back_to_full_parse() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        let base_lines = vec!["let value = 1;".to_string(); 256];
        let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

        let mut edited = base_lines.clone();
        for line in edited.iter_mut().take(180) {
            *line = "pub fn massive_fallback_path() { let x = vec![1,2,3,4]; }".to_string();
        }
        let attempt = prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            &edited.join("\n"),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(200),
            },
            Some(base_document),
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("large reparse should complete within the test full-parse budget");
        };

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Full)
        );
        let (_incremental, fallback) = incremental_reparse_counters();
        assert!(
            fallback > 0,
            "large edit should trigger full-parse fallback path"
        );
    }

    #[test]
    fn large_late_edit_with_preserved_prefix_can_stay_incremental() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let base_lines = (0..256)
            .map(|ix| format!("let value_{ix} = {ix}; {}", "x".repeat(96)))
            .collect::<Vec<_>>();
        let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

        let mut edited = base_lines.clone();
        for (offset, line) in edited.iter_mut().skip(96).enumerate() {
            *line = format!(
                "pub fn large_late_edit_{offset}() {{ let values = [{offset}, {offset}, {offset}, {offset}]; }} {}",
                "y".repeat(64)
            );
        }
        let attempt = prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            &edited.join("\n"),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(200),
            },
            Some(base_document),
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("large later-line reparse should complete within the test budget");
        };

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental)
        );
        let (incremental, fallback) = incremental_reparse_counters();
        assert!(
            incremental > 0,
            "later large edit should use incremental reparse"
        );
        assert_eq!(
            fallback, 0,
            "later large edit should avoid full-parse fallback"
        );
    }

    #[test]
    fn incremental_reparse_append_line_matches_full_parse_tokens() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let base_lines = vec!["let value = 41;".to_string(); 256];
        let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

        let mut edited = base_lines.clone();
        edited.push("let appended = 42;".to_string());
        let attempt = prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            &edited.join("\n"),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
        );
        let PrepareTreesitterDocumentResult::Ready(incremental_document) = attempt else {
            panic!("incremental append reparse should complete within budget");
        };
        assert_eq!(
            prepared_document_parse_mode(incremental_document),
            Some(TreesitterParseReuseMode::Incremental),
            "small EOF append should stay on incremental reparse path"
        );

        let edited_text = edited.join("\n");
        let edited_input = treesitter_document_input_from_text(&edited_text);
        let request = treesitter_document_parse_request_from_input(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited_input,
        )
        .expect("edited rust lines should produce parse request");
        let full_tree = with_ts_parser(&request.ts_language, |parser| {
            parse_treesitter_tree(parser, request.input.text.as_bytes(), None, None)
        })
        .flatten()
        .expect("full parse should succeed");
        let highlight =
            tree_sitter_highlight_spec(request.language).expect("rust highlight spec should exist");

        let full_tokens = collect_treesitter_document_line_tokens_for_line_window(
            &full_tree,
            highlight,
            request.input.text.as_bytes(),
            &request.input.line_starts,
            0,
            request.input.line_starts.len(),
        );
        let incremental_tokens = (0..edited.len())
            .map(|line_ix| {
                syntax_tokens_for_prepared_document_line(incremental_document, line_ix)
                    .expect("incremental document should have line tokens")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            incremental_tokens, full_tokens,
            "incremental append reparse should match full-parse tokenization"
        );
    }

    #[test]
    fn large_cache_replacement_uses_deferred_drop_queue() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let mut cache = TreesitterDocumentCache::new();
        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(1),
            benchmark_line_tokens_payload(2_048, 8, 0),
        );
        let (queued_before, dropped_before, _) = deferred_drop_counters();

        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(1),
            benchmark_line_tokens_payload(2_048, 8, 0),
        );
        let (queued_after, _, _) = deferred_drop_counters();
        assert!(
            queued_after > queued_before,
            "large replacement should enqueue deferred drop work"
        );

        assert!(
            benchmark_flush_deferred_drop_queue(),
            "deferred drop queue should flush"
        );
        let (_, dropped_after, _) = deferred_drop_counters();
        assert!(
            dropped_after > dropped_before,
            "deferred drop worker should process queued payloads"
        );
    }

    #[test]
    fn small_cache_replacement_keeps_inline_drop_path() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let mut cache = TreesitterDocumentCache::new();
        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(1),
            benchmark_line_tokens_payload(8, 1, 0),
        );
        let (_, _, inline_before) = deferred_drop_counters();

        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(1),
            benchmark_line_tokens_payload(8, 1, 0),
        );
        let (_, _, inline_after) = deferred_drop_counters();
        assert!(
            inline_after > inline_before,
            "small replacement should drop old payload inline"
        );
    }

    #[test]
    fn recent_duplicate_line_tokens_reuse_existing_arcs() {
        let document = TreesitterCachedDocument::from_line_tokens(
            benchmark_line_tokens_payload(4, 8, 0),
            None,
        );
        let first_chunk = document
            .line_token_chunks
            .get(&0)
            .expect("single chunk should be present");
        assert_eq!(first_chunk.len(), 4);
        assert!(
            Arc::ptr_eq(&first_chunk[0], &first_chunk[2]),
            "alternating duplicate line tokens should reuse the two-back Arc"
        );
        assert!(
            Arc::ptr_eq(&first_chunk[1], &first_chunk[3]),
            "alternating duplicate line tokens should reuse the matching recent Arc"
        );
    }

    #[test]
    fn cached_document_drop_payload_bytes_match_flattened_chunks() {
        let mut document =
            TreesitterCachedDocument::from_chunked_line_tokens(128, HashMap::default(), None);
        let first_chunk = benchmark_line_tokens_payload(64, 4, 0)
            .into_iter()
            .map(Arc::from)
            .collect::<Vec<_>>();
        let second_chunk = benchmark_line_tokens_payload(64, 4, 1)
            .into_iter()
            .map(Arc::from)
            .collect::<Vec<_>>();

        insert_line_token_chunk(&mut document, 0, Some(first_chunk));
        let bytes_after_first_insert = document.line_token_bytes;
        insert_line_token_chunk(&mut document, 0, Some(second_chunk.clone()));
        assert_eq!(
            document.line_token_bytes, bytes_after_first_insert,
            "reinserting an existing chunk should not double-count drop bytes"
        );

        insert_line_token_chunk(&mut document, 1, Some(second_chunk));
        let payload = document.into_drop_payload();
        assert_eq!(
            payload.estimated_bytes,
            estimated_line_tokens_allocation_bytes(&payload.line_tokens),
            "cached drop bytes should match the flattened payload"
        );
        assert_eq!(payload.line_tokens.len(), 128);
    }

    #[test]
    fn large_cache_eviction_uses_deferred_drop_queue() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let mut cache = TreesitterDocumentCache::new();
        for key in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
            cache.insert_document(
                TreesitterDocumentCache::make_test_cache_key(key as u64),
                benchmark_line_tokens_payload(2_048, 8, 0),
            );
        }
        let (queued_before, dropped_before, _) = deferred_drop_counters();

        cache.insert_document(
            TreesitterDocumentCache::make_test_cache_key(TS_DOCUMENT_CACHE_MAX_ENTRIES as u64 + 1),
            benchmark_line_tokens_payload(2_048, 8, 0),
        );
        let (queued_after, _, _) = deferred_drop_counters();
        assert!(
            queued_after > queued_before,
            "large eviction should enqueue deferred drop work"
        );

        assert!(
            benchmark_flush_deferred_drop_queue(),
            "deferred drop queue should flush"
        );
        let (_, dropped_after, _) = deferred_drop_counters();
        assert!(
            dropped_after > dropped_before,
            "deferred drop worker should process evicted payloads"
        );
    }

    #[test]
    fn parse_budget_timeout_falls_back_to_background_prepare() {
        let text = vec!["/* budget */ let value = Some(42);"; 2_048].join("\n");
        let attempt = prepare_test_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            &text,
            DiffSyntaxBudget {
                foreground_parse: Duration::ZERO,
            },
            None,
        );
        assert_eq!(attempt, PrepareTreesitterDocumentResult::TimedOut);

        let prepared = prepare_test_document_in_background(DiffSyntaxLanguage::Rust, &text)
            .expect("background parse should produce a prepared document");
        let document = inject_prepared_document_data(prepared);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("background-prepared document should have tokens");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "background parse should still yield syntax tokens"
        );
    }

    #[test]
    fn large_full_documents_skip_default_foreground_probe_without_reuse() {
        let text = vec!["fn parse_budget_probe() { let value = Some(42); }"; 2_048].join("\n");
        let request = treesitter_document_parse_request_from_input(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            treesitter_document_input_from_text(&text),
        )
        .expect("rust request should build");

        assert!(should_skip_budgeted_foreground_parse(
            &request,
            DiffSyntaxBudget {
                foreground_parse: DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST,
            },
            false,
            false,
        ));
        assert!(!should_skip_budgeted_foreground_parse(
            &request,
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            false,
            false,
        ));
        assert!(!should_skip_budgeted_foreground_parse(
            &request,
            DiffSyntaxBudget {
                foreground_parse: DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST,
            },
            true,
            false,
        ));
    }

    #[test]
    fn small_full_documents_keep_default_foreground_probe() {
        let text = vec!["fn small_probe() { value += 1; }"; 256].join("\n");
        let request = treesitter_document_parse_request_from_input(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            treesitter_document_input_from_text(&text),
        )
        .expect("rust request should build");

        assert!(!should_skip_budgeted_foreground_parse(
            &request,
            DiffSyntaxBudget {
                foreground_parse: DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST,
            },
            false,
            false,
        ));
    }

    #[test]
    fn background_text_reparse_reuses_old_tree_without_explicit_edit_hint() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let base_text = "let value = 1;\n".repeat(256);
        let base_input = treesitter_document_input_from_text(&base_text);
        let base_document =
            prepare_test_document_from_shared_text(DiffSyntaxLanguage::Rust, &base_text);
        let base_version =
            prepared_document_source_version(base_document).expect("base source version");

        let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
        let mut edited_text = base_text.clone();
        edited_text.insert_str(insert_offset, " // background tiny edit");
        let edited_input = treesitter_document_input_from_text(&edited_text);

        let prepared = prepare_treesitter_document_in_background_text_with_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited_text.into(),
            edited_input.line_starts.clone(),
            Some(base_document),
            None,
        )
        .expect("background text reparse should produce prepared data");
        let reparsed_document = inject_prepared_document_data(prepared);

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental),
            "background text reparses should keep small edits on the incremental path even without explicit edit hints"
        );
        let reparsed_version =
            prepared_document_source_version(reparsed_document).expect("reparsed source version");
        assert!(
            reparsed_version > base_version,
            "background incremental reparse should advance source version"
        );

        let (incremental, fallback) = incremental_reparse_counters();
        assert!(
            incremental > 0,
            "background no-edit-hint path should use incremental reparse"
        );
        assert_eq!(
            fallback, 0,
            "background no-edit-hint path should not trigger fallback"
        );
    }

    #[test]
    fn background_text_reparse_reuses_old_tree_with_explicit_edit_hint() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();

        let base_text = "let value = 1;\n".repeat(256);
        let base_input = treesitter_document_input_from_text(&base_text);
        let base_document =
            prepare_test_document_from_shared_text(DiffSyntaxLanguage::Rust, &base_text);
        let base_version =
            prepared_document_source_version(base_document).expect("base source version");

        let insert_offset = base_input.line_starts[42].saturating_add("let value = 1;".len());
        let mut edited_text = base_text.clone();
        edited_text.insert_str(insert_offset, " // background tiny edit");
        let edited_input = treesitter_document_input_from_text(&edited_text);

        let prepared = prepare_treesitter_document_in_background_text_with_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited_text.into(),
            edited_input.line_starts.clone(),
            Some(base_document),
            Some(DiffSyntaxEdit {
                old_range: insert_offset..insert_offset,
                new_range: insert_offset
                    ..insert_offset.saturating_add(" // background tiny edit".len()),
            }),
        )
        .expect("background text reparse should produce prepared data");
        let reparsed_document = inject_prepared_document_data(prepared);

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Incremental),
            "background text reparses should keep small edits on the incremental path"
        );
        let reparsed_version =
            prepared_document_source_version(reparsed_document).expect("reparsed source version");
        assert!(
            reparsed_version > base_version,
            "background incremental reparse should advance source version"
        );

        let (incremental, fallback) = incremental_reparse_counters();
        assert!(
            incremental > 0,
            "background explicit edit hint path should use incremental reparse"
        );
        assert_eq!(
            fallback, 0,
            "background explicit edit hint should not trigger fallback"
        );
    }

    #[test]
    fn background_seed_reuses_cached_prefix_chunks_before_large_edit_fallback() {
        let _lock = lock_global_counter_tests();
        reset_deferred_drop_counters();
        reset_prepared_syntax_cache();

        let line_count = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 4;
        let base_lines = (0..line_count)
            .map(|ix| format!("let value_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let base_document = prepare_test_document(DiffSyntaxLanguage::Rust, &base_lines.join("\n"));

        let _ = syntax_tokens_for_prepared_document_line(base_document, 0)
            .expect("base document should materialize its first chunk");
        assert_eq!(
            prepared_syntax_loaded_chunk_count(base_document),
            1,
            "base document should only have its first chunk materialized"
        );

        let reparse_seed = prepared_document_reparse_seed(base_document)
            .expect("base document should expose a seed");
        let mut edited = base_lines.clone();
        let first_changed_line = TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2;
        for (offset, line) in edited.iter_mut().skip(first_changed_line).enumerate() {
            *line = format!(
                "pub fn fallback_edit_{offset}() {{ let values = [{offset}, {offset}, {offset}, {offset}]; }}"
            );
        }
        let edited_text = edited.join("\n");
        let edited_input = treesitter_document_input_from_text(&edited_text);

        let prepared = prepare_treesitter_document_in_background_text_with_reparse_seed(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited_text.into(),
            edited_input.line_starts,
            Some(reparse_seed),
            None,
        )
        .expect("background large-edit reparse should produce prepared data");
        let reparsed_document = inject_prepared_document_data(prepared);

        assert_eq!(
            prepared_document_parse_mode(reparsed_document),
            Some(TreesitterParseReuseMode::Full),
            "large edit should still take the full-parse fallback path"
        );
        assert_eq!(
            prepared_syntax_loaded_chunk_count(reparsed_document),
            1,
            "background reparse seed should preserve cached prefix chunks before the edit"
        );

        benchmark_reset_prepared_syntax_cache_metrics();
        let _ = syntax_tokens_for_prepared_document_line(reparsed_document, 0)
            .expect("reparsed document should reuse the preserved prefix chunk");
        let after_prefix_hit = prepared_syntax_cache_metrics();
        assert_eq!(after_prefix_hit.hit, 1);
        assert_eq!(after_prefix_hit.miss, 0);
    }

    #[test]
    fn background_prepared_document_not_in_tls_until_injected() {
        let text = "/* background comment */\nlet value = 42;".to_string();
        let prepared = std::thread::spawn({
            let text = text.clone();
            move || {
                let input = treesitter_document_input_from_text(&text);
                prepare_treesitter_document_in_background_text_with_reuse(
                    DiffSyntaxLanguage::Rust,
                    DiffSyntaxMode::Auto,
                    SharedString::from(text),
                    input.line_starts,
                    None,
                    None,
                )
                .expect("background parse should produce prepared data")
            }
        })
        .join()
        .expect("background parse thread should not panic");

        let unresolved_handle = PreparedSyntaxDocument {
            cache_key: prepared.cache_key,
        };
        assert!(
            syntax_tokens_for_prepared_document_line(unresolved_handle, 0).is_none(),
            "background parse must not populate main-thread TLS cache until injected"
        );

        let document = inject_prepared_document_data(prepared);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("injected background document should have tokens");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "injected document should include parsed comment tokens"
        );
    }

    #[test]
    fn capture_name_mapping_preserves_rich_semantics() {
        // Full dot-qualified names should map to specific variants
        assert_eq!(
            syntax_kind_from_capture_name("comment.doc"),
            Some(SyntaxTokenKind::CommentDoc)
        );
        assert_eq!(
            syntax_kind_from_capture_name("string.escape"),
            Some(SyntaxTokenKind::StringEscape)
        );
        assert_eq!(
            syntax_kind_from_capture_name("keyword.control"),
            Some(SyntaxTokenKind::KeywordControl)
        );
        assert_eq!(
            syntax_kind_from_capture_name("function.method"),
            Some(SyntaxTokenKind::FunctionMethod)
        );
        assert_eq!(
            syntax_kind_from_capture_name("function.special"),
            Some(SyntaxTokenKind::FunctionSpecial)
        );
        assert_eq!(
            syntax_kind_from_capture_name("type.builtin"),
            Some(SyntaxTokenKind::TypeBuiltin)
        );
        assert_eq!(
            syntax_kind_from_capture_name("type.interface"),
            Some(SyntaxTokenKind::TypeInterface)
        );
        assert_eq!(
            syntax_kind_from_capture_name("variable"),
            Some(SyntaxTokenKind::Variable)
        );
        assert_eq!(
            syntax_kind_from_capture_name("variable.parameter"),
            Some(SyntaxTokenKind::VariableParameter)
        );
        assert_eq!(
            syntax_kind_from_capture_name("variable.special"),
            Some(SyntaxTokenKind::VariableSpecial)
        );
        assert_eq!(
            syntax_kind_from_capture_name("operator"),
            Some(SyntaxTokenKind::Operator)
        );
        assert_eq!(
            syntax_kind_from_capture_name("punctuation.bracket"),
            Some(SyntaxTokenKind::PunctuationBracket)
        );
        assert_eq!(
            syntax_kind_from_capture_name("punctuation.delimiter"),
            Some(SyntaxTokenKind::PunctuationDelimiter)
        );
        assert_eq!(
            syntax_kind_from_capture_name("tag"),
            Some(SyntaxTokenKind::Tag)
        );
        assert_eq!(
            syntax_kind_from_capture_name("attribute"),
            Some(SyntaxTokenKind::Attribute)
        );
        assert_eq!(
            syntax_kind_from_capture_name("lifetime"),
            Some(SyntaxTokenKind::Lifetime)
        );
        assert_eq!(
            syntax_kind_from_capture_name("boolean"),
            Some(SyntaxTokenKind::Boolean)
        );
        assert_eq!(
            syntax_kind_from_capture_name("tag.jsx"),
            Some(SyntaxTokenKind::Tag)
        );
        assert_eq!(
            syntax_kind_from_capture_name("property.name"),
            Some(SyntaxTokenKind::Property)
        );
        assert_eq!(
            syntax_kind_from_capture_name("type.name"),
            Some(SyntaxTokenKind::Type)
        );
        assert_eq!(
            syntax_kind_from_capture_name("punctuation.bracket.html"),
            Some(SyntaxTokenKind::PunctuationBracket)
        );
        assert_eq!(
            syntax_kind_from_capture_name("punctuation.delimiter.jsx"),
            Some(SyntaxTokenKind::PunctuationDelimiter)
        );

        // Base names should still work
        assert_eq!(
            syntax_kind_from_capture_name("comment"),
            Some(SyntaxTokenKind::Comment)
        );
        assert_eq!(
            syntax_kind_from_capture_name("string"),
            Some(SyntaxTokenKind::String)
        );
        assert_eq!(
            syntax_kind_from_capture_name("keyword"),
            Some(SyntaxTokenKind::Keyword)
        );

        // Unknown dot-qualified names fall back through shorter dotted prefixes
        assert_eq!(
            syntax_kind_from_capture_name("keyword.operator.regex"),
            Some(SyntaxTokenKind::Keyword)
        );
        assert_eq!(
            syntax_kind_from_capture_name("comment.block"),
            Some(SyntaxTokenKind::Comment)
        );

        // Truly unknown names return None
        assert_eq!(syntax_kind_from_capture_name("none"), None);
        assert_eq!(syntax_kind_from_capture_name("embedded"), None);
        assert_eq!(syntax_kind_from_capture_name("text.jsx"), None);
    }

    #[test]
    fn normalize_non_overlapping_tokens_keeps_later_same_range_token() {
        let tokens = normalize_non_overlapping_tokens(vec![
            SyntaxToken {
                range: 0..5,
                kind: SyntaxTokenKind::Function,
            },
            SyntaxToken {
                range: 0..5,
                kind: SyntaxTokenKind::Type,
            },
        ]);
        assert_eq!(
            tokens,
            vec![SyntaxToken {
                range: 0..5,
                kind: SyntaxTokenKind::Type,
            }]
        );
    }

    #[cfg(any(test, feature = "syntax-rust"))]
    #[test]
    fn vendored_rust_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let source = RUST_HIGHLIGHTS_QUERY;
        tree_sitter::Query::new(&lang, source)
            .expect("vendored Rust highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_css_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_css::LANGUAGE.into();
        let source = CSS_HIGHLIGHTS_QUERY;
        tree_sitter::Query::new(&lang, source).expect("vendored CSS highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-shell"))]
    #[test]
    fn vendored_bash_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
        tree_sitter::Query::new(&lang, BASH_HIGHLIGHTS_QUERY)
            .expect("vendored Bash highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_html_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
        let source = HTML_HIGHLIGHTS_QUERY;
        tree_sitter::Query::new(&lang, source)
            .expect("vendored HTML highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_html_injections_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
        tree_sitter::Query::new(&lang, HTML_INJECTIONS_QUERY)
            .expect("vendored HTML injections.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_javascript_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        tree_sitter::Query::new(&lang, JAVASCRIPT_HIGHLIGHTS_QUERY)
            .expect("vendored JavaScript highlights.scm should compile against JS grammar");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_javascript_injections_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        tree_sitter::Query::new(&lang, JAVASCRIPT_INJECTIONS_QUERY)
            .expect("vendored JavaScript injections.scm should compile against JS grammar");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_typescript_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        tree_sitter::Query::new(&lang, TYPESCRIPT_HIGHLIGHTS_QUERY)
            .expect("vendored TypeScript highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_typescript_injections_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        tree_sitter::Query::new(&lang, TYPESCRIPT_INJECTIONS_QUERY)
            .expect("vendored TypeScript injections.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_tsx_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
        tree_sitter::Query::new(&lang, TSX_HIGHLIGHTS_QUERY)
            .expect("vendored TSX highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_tsx_injections_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
        tree_sitter::Query::new(&lang, TSX_INJECTIONS_QUERY)
            .expect("vendored TSX injections.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-go"))]
    #[test]
    fn vendored_go_queries_compile() {
        let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        tree_sitter::Query::new(&lang, GO_HIGHLIGHTS_QUERY)
            .expect("vendored Go highlights.scm should compile");
        tree_sitter::Query::new(&lang, GO_INJECTIONS_QUERY)
            .expect("vendored Go injections.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-data"))]
    #[test]
    fn vendored_json_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_json::LANGUAGE.into();
        tree_sitter::Query::new(&lang, JSON_HIGHLIGHTS_QUERY)
            .expect("vendored JSON highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-python"))]
    #[test]
    fn vendored_python_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        tree_sitter::Query::new(&lang, PYTHON_HIGHLIGHTS_QUERY)
            .expect("vendored Python highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-data"))]
    #[test]
    fn vendored_yaml_queries_compile() {
        let lang: tree_sitter::Language = tree_sitter_yaml::LANGUAGE.into();
        tree_sitter::Query::new(&lang, YAML_HIGHLIGHTS_QUERY)
            .expect("vendored YAML highlights.scm should compile");
        tree_sitter::Query::new(&lang, YAML_INJECTIONS_QUERY)
            .expect("vendored YAML injections.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-extra"))]
    #[test]
    fn vendored_csharp_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
        tree_sitter::Query::new(&lang, CSHARP_HIGHLIGHTS_QUERY)
            .expect("vendored C# highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-extra"))]
    #[test]
    fn vendored_cpp_injections_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
        tree_sitter::Query::new(&lang, CPP_INJECTIONS_QUERY)
            .expect("vendored C++ injections.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-repo"))]
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

    #[cfg(any(test, feature = "syntax-xml"))]
    #[test]
    fn vendored_xml_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_xml::LANGUAGE_XML.into();
        tree_sitter::Query::new(&lang, XML_HIGHLIGHTS_QUERY)
            .expect("XML highlights.scm should compile against XML grammar");
    }

    #[cfg(any(test, feature = "syntax-xml"))]
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

    #[cfg(any(test, feature = "syntax-xml"))]
    #[test]
    fn xml_treesitter_captures_comment() {
        let text = "<!-- a comment -->";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Xml, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "XML should capture comments: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn javascript_treesitter_captures_function_and_keyword() {
        let text = "function foo() { return 42; }";
        let tokens =
            syntax_tokens_for_line(text, DiffSyntaxLanguage::JavaScript, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Function),
            "JS should capture function names: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Keyword
                    || t.kind == SyntaxTokenKind::KeywordControl),
            "JS should capture keywords: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
            "JS should capture numbers: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn html_highlight_spec_compiles_injection_query() {
        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html)
            .expect("HTML highlight spec should exist");
        assert!(
            spec.injection_query.is_some(),
            "HTML should compile and retain its vendored injections.scm"
        );
    }

    #[cfg(any(test, feature = "syntax-web"))]
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
        name == "none"
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
        #[cfg(any(test, feature = "syntax-rust"))]
        assert_capture_names_are_supported(
            tree_sitter_rust::LANGUAGE.into(),
            RUST_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-web"))]
        assert_capture_names_are_supported(
            tree_sitter_html::LANGUAGE.into(),
            HTML_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-web"))]
        assert_capture_names_are_supported(tree_sitter_css::LANGUAGE.into(), CSS_HIGHLIGHTS_QUERY);
        #[cfg(any(test, feature = "syntax-shell"))]
        assert_capture_names_are_supported(
            tree_sitter_bash::LANGUAGE.into(),
            BASH_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-web"))]
        assert_capture_names_are_supported(
            tree_sitter_javascript::LANGUAGE.into(),
            JAVASCRIPT_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-python"))]
        assert_capture_names_are_supported(
            tree_sitter_python::LANGUAGE.into(),
            PYTHON_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-go"))]
        assert_capture_names_are_supported(tree_sitter_go::LANGUAGE.into(), GO_HIGHLIGHTS_QUERY);
        #[cfg(any(test, feature = "syntax-data"))]
        assert_capture_names_are_supported(
            tree_sitter_json::LANGUAGE.into(),
            JSON_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-data"))]
        assert_capture_names_are_supported(
            tree_sitter_yaml::LANGUAGE.into(),
            YAML_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-web"))]
        assert_capture_names_are_supported(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TYPESCRIPT_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-web"))]
        assert_capture_names_are_supported(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TSX_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-xml"))]
        assert_capture_names_are_supported(
            tree_sitter_xml::LANGUAGE_XML.into(),
            XML_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_cpp::LANGUAGE.into(),
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_c_sharp::LANGUAGE.into(),
            CSHARP_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_java::LANGUAGE.into(),
            tree_sitter_java::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_php::LANGUAGE_PHP.into(),
            tree_sitter_php::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_ruby::LANGUAGE.into(),
            tree_sitter_ruby::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_lua::LANGUAGE.into(),
            tree_sitter_lua::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_make::LANGUAGE.into(),
            tree_sitter_make::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_kotlin_sg::LANGUAGE.into(),
            tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_zig::LANGUAGE.into(),
            tree_sitter_zig::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_bicep::LANGUAGE.into(),
            tree_sitter_bicep::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_objc::LANGUAGE.into(),
            tree_sitter_objc::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
            tree_sitter_fsharp::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_powershell::LANGUAGE.into(),
            POWERSHELL_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_swift::LANGUAGE.into(),
            tree_sitter_swift::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_r::LANGUAGE.into(),
            tree_sitter_r::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_dart::LANGUAGE.into(),
            tree_sitter_dart::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_scala::LANGUAGE.into(),
            tree_sitter_scala::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-extra"))]
        assert_capture_names_are_supported(
            tree_sitter_sequel::LANGUAGE.into(),
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-repo"))]
        assert_capture_names_are_supported(
            tree_sitter_md::LANGUAGE.into(),
            MARKDOWN_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-repo"))]
        assert_capture_names_are_supported(
            tree_sitter_md::INLINE_LANGUAGE.into(),
            MARKDOWN_INLINE_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-repo"))]
        assert_capture_names_are_supported(
            tree_sitter_diff::LANGUAGE.into(),
            tree_sitter_diff::HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-repo"))]
        assert_capture_names_are_supported(
            tree_sitter_gitcommit::LANGUAGE.into(),
            GITCOMMIT_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-repo"))]
        assert_capture_names_are_supported(
            tree_sitter_gomod::LANGUAGE.into(),
            GOMOD_HIGHLIGHTS_QUERY,
        );
        #[cfg(any(test, feature = "syntax-repo"))]
        assert_capture_names_are_supported(
            tree_sitter_gowork::LANGUAGE.into(),
            GOWORK_HIGHLIGHTS_QUERY,
        );
    }

    #[cfg(any(test, feature = "syntax-rust"))]
    #[test]
    fn rust_treesitter_captures_variable_parameter() {
        let text = "fn foo(bar: u32) {}";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::VariableParameter),
            "Rust function parameter should produce VariableParameter token, got: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-rust"))]
    #[test]
    fn rust_treesitter_captures_type_builtin() {
        let text = "let x: u32 = 0;";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::TypeBuiltin),
            "Rust primitive type should produce TypeBuiltin token, got: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-rust"))]
    #[test]
    fn rust_treesitter_captures_macro_as_function_special() {
        let text = "println!(\"hello\");";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::FunctionSpecial),
            "Rust macro invocation should produce FunctionSpecial token, got: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn tsx_treesitter_highlights_jsx_tag_and_attribute() {
        let text = "const node = <button disabled />;";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Tsx, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag),
            "TSX should highlight JSX tags, got: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute),
            "TSX should highlight JSX attributes, got: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn css_treesitter_captures_property_and_keyword() {
        let text = "@media screen { .foo { color: red; } }";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Css, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
            "CSS should highlight @media as keyword: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
            "CSS should highlight 'color' as property: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn javascript_tagged_template_injects_css() {
        let document = prepare_test_document(
            DiffSyntaxLanguage::JavaScript,
            "const styles = css`color: red;`;",
        );
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Property),
            "tagged CSS template should inject CSS property highlighting: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-data"))]
    #[test]
    fn yaml_github_actions_script_injects_javascript() {
        let text = [
            "jobs:",
            "  test:",
            "    steps:",
            "      - uses: actions/github-script@v7",
            "        with:",
            "          script: |",
            "            const value = 42",
        ]
        .join("\n");
        let document = prepare_test_document(DiffSyntaxLanguage::Yaml, &text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 6)
            .expect("YAML github-script line should have prepared tokens");
        assert!(
            tokens.iter().any(|t| {
                t.kind == SyntaxTokenKind::Keyword || t.kind == SyntaxTokenKind::KeywordControl
            }),
            "github-script YAML block should inject JavaScript keywords: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
            "github-script YAML block should inject JavaScript numbers: {tokens:?}"
        );
    }

    #[cfg(any(test, feature = "syntax-extra"))]
    #[test]
    fn extra_languages_capture_basic_semantic_tokens() {
        let cases = [
            (
                DiffSyntaxLanguage::C,
                "int main(void) { return 0; }",
                SyntaxTokenKind::Function,
            ),
            (
                DiffSyntaxLanguage::Cpp,
                "auto value = std::vector<int>{1, 2};",
                SyntaxTokenKind::Type,
            ),
            (
                DiffSyntaxLanguage::CSharp,
                "public class Example { string Name { get; } }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Bicep,
                "param location string = 'westeurope'",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::ObjectiveC,
                "NSString *value = @\"hi\";",
                SyntaxTokenKind::Property,
            ),
            (
                DiffSyntaxLanguage::FSharp,
                "let value = 42",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Java,
                "class Example { int value() { return 1; } }",
                SyntaxTokenKind::FunctionMethod,
            ),
            (
                DiffSyntaxLanguage::Php,
                "<?php function foo(): int { return 1; }",
                SyntaxTokenKind::Function,
            ),
            (
                DiffSyntaxLanguage::Ruby,
                "class Example; def call(name) = 42 end",
                SyntaxTokenKind::Function,
            ),
            (
                DiffSyntaxLanguage::PowerShell,
                "function Invoke-Test { return 42 }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Swift,
                "struct Example { let value = 42 }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::R,
                "if (TRUE) print(1)",
                SyntaxTokenKind::Boolean,
            ),
            (
                DiffSyntaxLanguage::Dart,
                "class Example { int value() => 42; }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Scala,
                "object Example { def run(): Int = 42 }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Toml,
                "enabled = true",
                SyntaxTokenKind::Property,
            ),
            (
                DiffSyntaxLanguage::Lua,
                "local value = 42",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Kotlin,
                "class Example { fun run() = 42 }",
                SyntaxTokenKind::Function,
            ),
            (
                DiffSyntaxLanguage::Zig,
                "const value: u32 = 42;",
                SyntaxTokenKind::TypeBuiltin,
            ),
            (
                DiffSyntaxLanguage::Sql,
                "select name from users",
                SyntaxTokenKind::Keyword,
            ),
        ];

        for (language, text, expected_kind) in cases {
            let tokens = syntax_tokens_for_line(text, language, DiffSyntaxMode::Auto);
            assert!(
                tokens.iter().any(|token| token.kind == expected_kind),
                "{language:?} should capture {expected_kind:?}: {tokens:?}"
            );
        }
    }

    #[cfg(any(test, feature = "syntax-repo"))]
    #[test]
    fn repo_languages_capture_basic_semantic_tokens() {
        let cases = [
            (
                DiffSyntaxLanguage::GoMod,
                "module example.com/project",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::GoWork,
                "use ./module",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Diff,
                "diff --git a/src/lib.rs b/src/lib.rs",
                SyntaxTokenKind::VariableSpecial,
            ),
            (
                DiffSyntaxLanguage::GitCommit,
                "feat: widen syntax support",
                SyntaxTokenKind::Keyword,
            ),
        ];

        for (language, text, expected_kind) in cases {
            let tokens = syntax_tokens_for_line(text, language, DiffSyntaxMode::Auto);
            assert!(
                tokens.iter().any(|token| token.kind == expected_kind),
                "{language:?} should capture {expected_kind:?}: {tokens:?}"
            );
        }
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
            tokens.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (6..7)
            }),
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
            nested.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (6..7)
            }),
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
            required.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (16..17)
            }),
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
            string_value.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (12..13)
            }),
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
            expression_value.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (9..10)
            }),
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
            sequence_mapping.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (6..7)
            }),
            "YAML fallback should highlight list punctuation: {sequence_mapping:?}"
        );
        assert!(
            sequence_mapping
                .iter()
                .any(|token| { token.kind == SyntaxTokenKind::Property && token.range == (8..12) }),
            "YAML fallback should highlight sequence mapping keys: {sequence_mapping:?}"
        );
        assert!(
            sequence_mapping.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (12..13)
            }),
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
            block_scalar.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (11..12)
            }),
            "YAML fallback should highlight the mapping colon for block scalars: {block_scalar:?}"
        );
        assert!(
            block_scalar.iter().any(|token| {
                token.kind == SyntaxTokenKind::Punctuation && token.range == (13..14)
            }),
            "YAML fallback should highlight block scalar indicators: {block_scalar:?}"
        );
    }

    #[test]
    fn grammar_and_highlight_spec_agree_on_supported_languages() {
        let all_languages = [
            DiffSyntaxLanguage::Markdown,
            DiffSyntaxLanguage::MarkdownInline,
            DiffSyntaxLanguage::Html,
            DiffSyntaxLanguage::Css,
            DiffSyntaxLanguage::Hcl,
            DiffSyntaxLanguage::Bicep,
            DiffSyntaxLanguage::Lua,
            DiffSyntaxLanguage::Makefile,
            DiffSyntaxLanguage::Kotlin,
            DiffSyntaxLanguage::Zig,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxLanguage::Python,
            DiffSyntaxLanguage::JavaScript,
            DiffSyntaxLanguage::TypeScript,
            DiffSyntaxLanguage::Tsx,
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
        ];
        for lang in all_languages {
            let has_grammar = tree_sitter_grammar(lang).is_some();
            let has_spec = tree_sitter_highlight_spec(lang).is_some();
            assert_eq!(
                has_grammar, has_spec,
                "tree_sitter_grammar and tree_sitter_highlight_spec disagree for {lang:?}: \
                 grammar={has_grammar}, spec={has_spec}"
            );
        }
    }

    #[cfg(not(any(test, feature = "syntax-web")))]
    #[test]
    fn disabled_web_grammars_fall_back_to_none() {
        assert!(tree_sitter_grammar(DiffSyntaxLanguage::Html).is_none());
        assert!(tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).is_none());
        assert!(tree_sitter_grammar(DiffSyntaxLanguage::JavaScript).is_none());
        assert!(tree_sitter_highlight_spec(DiffSyntaxLanguage::JavaScript).is_none());
    }

    #[cfg(not(any(test, feature = "syntax-xml")))]
    #[test]
    fn disabled_xml_grammar_falls_back_to_none() {
        assert!(tree_sitter_grammar(DiffSyntaxLanguage::Xml).is_none());
        assert!(tree_sitter_highlight_spec(DiffSyntaxLanguage::Xml).is_none());
    }

    #[cfg(any(test, feature = "syntax-rust"))]
    #[test]
    fn highlight_spec_exposes_ts_language() {
        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Rust)
            .expect("Rust highlight spec should exist");
        // Verify the ts_language field is usable for parsing
        with_ts_parser(&spec.ts_language, |_| ()).expect("should accept the spec's ts_language");
    }

    #[cfg(any(test, feature = "syntax-rust"))]
    #[test]
    #[ignore]
    fn perf_treesitter_tokenization_smoke() {
        let text = "fn main() { let x = Some(123); println!(\"{x:?}\"); }";
        let start = Instant::now();
        for _ in 0..200_000 {
            let _ = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        }
        eprintln!("syntax_tokens_for_line (rust): {:?}", start.elapsed());
    }

    // ---- heuristic tokenizer tests ----

    #[test]
    fn heuristic_ruby_hash_comment() {
        let tokens = syntax_tokens_for_line_heuristic("x = 1 # comment", DiffSyntaxLanguage::Ruby);
        let comment = tokens.iter().find(|t| t.kind == SyntaxTokenKind::Comment);
        assert!(comment.is_some(), "Ruby '#' should be detected as comment");
        let c = comment.unwrap();
        assert!(c.range.start <= 6, "comment should start at or before '#'");
        assert_eq!(
            c.range.end,
            "x = 1 # comment".len(),
            "comment should extend to end of line"
        );
    }

    #[test]
    fn heuristic_python_hash_comment() {
        let tokens = syntax_tokens_for_line_heuristic("x = 1 # note", DiffSyntaxLanguage::Python);
        assert!(tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment));
    }

    #[test]
    fn heuristic_vb_rem_comment() {
        let tokens = syntax_tokens_for_line_heuristic(
            "REM this is a comment",
            DiffSyntaxLanguage::VisualBasic,
        );
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, SyntaxTokenKind::Comment);
        assert_eq!(tokens[0].range, 0..21);
    }

    #[test]
    fn heuristic_vb_apostrophe_comment() {
        let tokens = syntax_tokens_for_line_heuristic(
            "' this is a comment",
            DiffSyntaxLanguage::VisualBasic,
        );
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, SyntaxTokenKind::Comment);
    }

    #[test]
    fn heuristic_vb_keywords_are_case_insensitive() {
        let tokens = syntax_tokens_for_line_heuristic(
            "dim value As Integer",
            DiffSyntaxLanguage::VisualBasic,
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
            "Visual Basic keywords should be highlighted regardless of case"
        );
    }

    #[test]
    fn heuristic_rust_line_comment_and_string() {
        let tokens = syntax_tokens_for_line_heuristic(
            r#"let s = "hello"; // done"#,
            DiffSyntaxLanguage::Rust,
        );
        let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
        assert!(
            kinds.contains(&SyntaxTokenKind::Keyword),
            "should find 'let'"
        );
        assert!(
            kinds.contains(&SyntaxTokenKind::String),
            "should find string"
        );
        assert!(
            kinds.contains(&SyntaxTokenKind::Comment),
            "should find comment"
        );
    }

    #[test]
    fn heuristic_rust_block_comment_continues_scanning() {
        let tokens =
            syntax_tokens_for_line_heuristic("/* note */ let value = 1", DiffSyntaxLanguage::Rust);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "should find block comment"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
            "should keep scanning after block comment"
        );
    }

    #[test]
    fn heuristic_fsharp_block_comment_continues_scanning() {
        let tokens = syntax_tokens_for_line_heuristic(
            "(* note *) let value = 1",
            DiffSyntaxLanguage::FSharp,
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "should find F# block comment"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword),
            "should keep scanning after F# block comment"
        );
    }

    #[test]
    fn heuristic_hcl_hash_comment() {
        let tokens = syntax_tokens_for_line_heuristic("value = 1 # note", DiffSyntaxLanguage::Hcl);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "HCL '#' should be detected as comment"
        );
    }

    #[test]
    fn heuristic_powershell_hash_comment() {
        let tokens =
            syntax_tokens_for_line_heuristic("$value = 1 # note", DiffSyntaxLanguage::PowerShell);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment),
            "PowerShell '#' should be detected as comment"
        );
    }

    #[test]
    fn heuristic_html_comment() {
        let tokens =
            syntax_tokens_for_line_heuristic("<!-- comment --> <div>", DiffSyntaxLanguage::Html);
        assert!(tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment));
    }

    #[test]
    fn heuristic_lua_block_comment() {
        let tokens =
            syntax_tokens_for_line_heuristic("--[[ block ]] rest", DiffSyntaxLanguage::Lua);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, SyntaxTokenKind::Comment);
        // Should cover "--[[" through "]]"
        assert_eq!(tokens[0].range.end, 13);
    }

    #[test]
    fn heuristic_css_selector() {
        let tokens =
            syntax_tokens_for_line_heuristic(".my-class { color: red; }", DiffSyntaxLanguage::Css);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Type),
            "CSS class selector should be Type"
        );
    }

    #[test]
    fn heuristic_number_literal() {
        let tokens = syntax_tokens_for_line_heuristic("x = 42", DiffSyntaxLanguage::Python);
        assert!(tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number));
    }

    #[test]
    fn injection_cache_lru_eviction_preserves_recent_entries() {
        TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

        // Fill the cache to max capacity with distinct entries, using the
        // global counter so access values are monotonically ordered.
        for i in 0..TS_INJECTION_CACHE_MAX_ENTRIES {
            let key = TreesitterInjectionMatch {
                language: DiffSyntaxLanguage::JavaScript,
                byte_start: i * 100,
                byte_end: i * 100 + 50,
                content_hash: i as u64,
            };
            let access = next_injection_access();
            TS_INJECTION_CACHE.with(|cache| {
                cache.borrow_mut().insert(
                    key,
                    CachedInjectionTokens {
                        all_line_tokens: vec![],
                        injection_line_starts: vec![],
                        injection_start_line_ix: 0,
                        last_access: access,
                    },
                );
            });
        }

        // Access the first entry to make it "recent" (higher counter than all others).
        let first_key = TreesitterInjectionMatch {
            language: DiffSyntaxLanguage::JavaScript,
            byte_start: 0,
            byte_end: 50,
            content_hash: 0,
        };
        TS_INJECTION_CACHE.with(|cache| {
            if let Some(entry) = cache.borrow_mut().get_mut(&first_key) {
                entry.last_access = next_injection_access();
            }
        });

        // Now insert one more to trigger eviction.
        let overflow_key = TreesitterInjectionMatch {
            language: DiffSyntaxLanguage::JavaScript,
            byte_start: 99900,
            byte_end: 99950,
            content_hash: 99999,
        };
        let access = next_injection_access();
        TS_INJECTION_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= TS_INJECTION_CACHE_MAX_ENTRIES {
                let mut entries: Vec<_> = cache.iter().map(|(k, v)| (*k, v.last_access)).collect();
                entries.sort_unstable_by_key(|(_, a)| *a);
                let evict_count = entries.len() / 2;
                for (key, _) in entries.into_iter().take(evict_count) {
                    cache.remove(&key);
                }
            }
            cache.insert(
                overflow_key,
                CachedInjectionTokens {
                    all_line_tokens: vec![],
                    injection_line_starts: vec![],
                    injection_start_line_ix: 0,
                    last_access: access,
                },
            );
        });

        TS_INJECTION_CACHE.with(|cache| {
            let cache = cache.borrow();
            // The recently-accessed first entry should survive eviction.
            assert!(
                cache.contains_key(&first_key),
                "recently accessed entry should survive LRU eviction"
            );
            // The new entry should be present.
            assert!(
                cache.contains_key(&overflow_key),
                "newly inserted entry should be present"
            );
            // Cache should be below max.
            assert!(
                cache.len() <= TS_INJECTION_CACHE_MAX_ENTRIES,
                "cache should not exceed max entries"
            );
        });

        TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
    }
}
