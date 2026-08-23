use super::super::*;
use gpui::SharedString;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
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
pub(in crate::view) const PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES: usize = PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES;
const TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES: usize = 64;
const TS_PENDING_PARSE_REQUEST_MAX_ENTRIES: usize = 8;
const BASH_HIGHLIGHTS_QUERY: &str = include_str!("queries/bash_highlights.scm");
const C_HIGHLIGHTS_QUERY: &str = include_str!("queries/c_highlights.scm");
const C_INJECTIONS_QUERY: &str = include_str!("queries/c_injections.scm");
const CSHARP_HIGHLIGHTS_QUERY: &str = include_str!("queries/csharp_highlights.scm");
const CPP_HIGHLIGHTS_QUERY: &str = include_str!("queries/cpp_highlights.scm");
const GITCOMMIT_HIGHLIGHTS_QUERY: &str = include_str!("queries/gitcommit_highlights.scm");
const GOMOD_HIGHLIGHTS_QUERY: &str = include_str!("queries/gomod_highlights.scm");
const GOWORK_HIGHLIGHTS_QUERY: &str = include_str!("queries/gowork_highlights.scm");
const GROOVY_SUPPLEMENT_QUERY: &str = include_str!("queries/groovy_supplement.scm");
const HCL_HIGHLIGHTS_QUERY: &str = include_str!("queries/hcl_highlights.scm");
const JAVA_SUPPLEMENT_QUERY: &str = include_str!("queries/java_supplement.scm");
const OBJC_SUPPLEMENT_QUERY: &str = include_str!("queries/objc_supplement.scm");
const PHP_SUPPLEMENT_QUERY: &str = include_str!("queries/php_supplement.scm");
const POWERSHELL_SUPPLEMENT_QUERY: &str = include_str!("queries/powershell_supplement.scm");
const SQL_SUPPLEMENT_QUERY: &str = include_str!("queries/sql_supplement.scm");
const ZIG_SUPPLEMENT_QUERY: &str = include_str!("queries/zig_supplement.scm");
const XML_SUPPLEMENT_QUERY: &str = include_str!("queries/xml_supplement.scm");
const HTML_HIGHLIGHTS_QUERY: &str = include_str!("queries/html_highlights.scm");
const HTML_INJECTIONS_QUERY: &str = include_str!("queries/html_injections.scm");
const JINJA_HIGHLIGHTS_QUERY: &str = include_str!("queries/jinja_highlights.scm");
const JINJA_INJECTIONS_QUERY: &str = include_str!("queries/jinja_injections.scm");
const VUE_HIGHLIGHTS_QUERY: &str = include_str!("queries/vue_highlights.scm");
const VUE_INJECTIONS_QUERY: &str = include_str!("queries/vue_injections.scm");
const MARKDOWN_HIGHLIGHTS_QUERY: &str = tree_sitter_md::HIGHLIGHT_QUERY_BLOCK;
const MARKDOWN_INJECTIONS_QUERY: &str = tree_sitter_md::INJECTION_QUERY_BLOCK;
const MARKDOWN_INLINE_HIGHLIGHTS_QUERY: &str = tree_sitter_md::HIGHLIGHT_QUERY_INLINE;
const CSS_HIGHLIGHTS_QUERY: &str = include_str!("queries/css_highlights.scm");
const GO_HIGHLIGHTS_QUERY: &str = include_str!("queries/go_highlights.scm");
const GO_INJECTIONS_QUERY: &str = include_str!("queries/go_injections.scm");
const JAVASCRIPT_HIGHLIGHTS_QUERY: &str = include_str!("queries/javascript_highlights.scm");
const JAVASCRIPT_INJECTIONS_QUERY: &str = include_str!("queries/javascript_injections.scm");
const JSDOC_HIGHLIGHTS_QUERY: &str = include_str!("queries/jsdoc_highlights.scm");
const JSON_HIGHLIGHTS_QUERY: &str = include_str!("queries/json_highlights.scm");
const POWERSHELL_HIGHLIGHTS_QUERY: &str = tree_sitter_powershell::HIGHLIGHTS_QUERY;
const PYTHON_HIGHLIGHTS_QUERY: &str = include_str!("queries/python_highlights.scm");
const REGEX_HIGHLIGHTS_QUERY: &str = include_str!("queries/regex_highlights.scm");
const TYPESCRIPT_HIGHLIGHTS_QUERY: &str = include_str!("queries/typescript_highlights.scm");
const TYPESCRIPT_INJECTIONS_QUERY: &str = include_str!("queries/typescript_injections.scm");
const TSX_HIGHLIGHTS_QUERY: &str = include_str!("queries/tsx_highlights.scm");
const TSX_INJECTIONS_QUERY: &str = include_str!("queries/tsx_injections.scm");
const NIX_HIGHLIGHTS_QUERY: &str = include_str!("queries/nix_highlights.scm");
const NIX_INJECTIONS_QUERY: &str = include_str!("queries/nix_injections.scm");
const RUST_HIGHLIGHTS_QUERY: &str = include_str!("queries/rust_highlights.scm");
const RUST_INJECTIONS_QUERY: &str = include_str!("queries/rust_injections.scm");
const YAML_HIGHLIGHTS_QUERY: &str = include_str!("queries/yaml_highlights.scm");
const YAML_INJECTIONS_QUERY: &str = include_str!("queries/yaml_injections.scm");
const XML_HIGHLIGHTS_QUERY: &str = tree_sitter_xml::XML_HIGHLIGHT_QUERY;
const CPP_INJECTIONS_QUERY: &str = include_str!("queries/cpp_injections.scm");
const CLOJURE_HIGHLIGHTS_QUERY: &str = include_str!("queries/clojure_highlights.scm");
const JULIA_HIGHLIGHTS_QUERY: &str = include_str!("queries/julia_highlights.scm");
const OCAML_HIGHLIGHTS_QUERY: &str = include_str!("queries/ocaml_highlights.scm");
const SOLIDITY_HIGHLIGHTS_QUERY: &str = include_str!("queries/solidity_highlights.scm");
const SVELTE_HIGHLIGHTS_QUERY: &str = include_str!("queries/svelte_highlights.scm");
const SVELTE_INJECTIONS_QUERY: &str = include_str!("queries/svelte_injections.scm");

/// Maximum injection nesting depth. Root document = 0, first injection = 1.
/// This prevents infinite recursion if an injected language's highlight spec
/// itself contains an injection query.
const TS_MAX_INJECTION_DEPTH: usize = 1;
const TS_INJECTION_CACHE_MAX_ENTRIES: usize = 32;

/// Ceilings for one `(#set! injection.combined)` layer in the *prepared* path.
///
/// Hard limits rather than a time budget: the prepared path cannot repair a layer
/// dropped by a `ControlFlow::Break` -- that mechanism exists only in `live.rs` --
/// so a budgeted layer would appear and vanish with scroll timing.
///
/// Both are measured *after* the ranges are clipped by
/// [`combined_injection_clip_region`], and that ordering is load-bearing. A
/// template grammar hands out one `(text)` node per gap between tags, so an
/// unclipped node is document-sized: a 1900-line `.njk` is a single 138KB range,
/// which tripped the byte ceiling for every window and dropped HTML highlighting
/// from the whole file.
///
/// The byte ceiling is the one that bounds cost, since a combined parse lexes only
/// included bytes. The range ceiling is only an allocation guard on the
/// `Vec<tree_sitter::Range>`, sized so template density cannot reach it -- at 512 an
/// ordinary 8-column table row (~950 ranges in a clipped window) tripped it.
///
/// `live.rs` deliberately has no equivalent; see `parse_injection_layers`.
const TS_COMBINED_INJECTION_MAX_RANGES: usize = 16_384;
const TS_COMBINED_INJECTION_MAX_BYTES: usize = 128 * 1024;

/// Context on each side of the rendered window that a combined injection is still
/// parsed with.
///
/// Clipping to *exactly* the window is not output-preserving: a `<section` whose
/// attributes run onto the next lines is cut in half, and the surviving half loses
/// its captures to error recovery. Measured on that fixture: margin 0 differs on two
/// lines, margin 1024 is byte-identical to an unclipped parse. 4KB is well past any
/// realistic multi-line tag and still bounds the parse at window + 8KB.
const TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES: usize = 4 * 1024;

thread_local! {
    static TS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new(tree_sitter::Parser::new());
    static TS_PARSER_REQUIRES_LANGUAGE_RESET: Cell<bool> = const { Cell::new(false) };
    static TS_CURSOR: RefCell<tree_sitter::QueryCursor> = RefCell::new(tree_sitter::QueryCursor::new());
    static TS_INPUT: RefCell<String> = const { RefCell::new(String::new()) };
    static TS_DOCUMENT_CACHE: RefCell<TreesitterDocumentCache> = RefCell::new(TreesitterDocumentCache::new());
    static TS_LINE_TOKEN_CACHE: RefCell<SingleLineSyntaxTokenCache> = RefCell::new(SingleLineSyntaxTokenCache::new());
    static TS_INJECTION_CACHE: RefCell<FxHashMap<TreesitterInjectionMatch, CachedInjectionTokens>> = RefCell::new(FxHashMap::default());
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

// `Ord` so injection layers can be sorted by (range, language) and deduped;
// the order itself carries no meaning beyond being total and stable.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::view) enum DiffSyntaxLanguage {
    Markdown,
    MarkdownInline,
    Html,
    Vue,
    Svelte,
    /// Nunjucks, Jinja2, Twig and Django templates. One grammar parses the union
    /// of all four dialects; the HTML around the tags arrives as a combined
    /// injection (see queries/jinja_injections.scm).
    Jinja,
    /// The same grammar with the HTML injection removed, for templates whose body is
    /// not markup: `values.yaml.j2`, `deploy.sh.j2`, `nginx.conf.j2`. Template tags
    /// still highlight; only the injection is dropped.
    JinjaText,
    Css,
    Hcl,
    Bicep,
    Lua,
    Makefile,
    Nix,
    Kotlin,
    Zig,
    Groovy,
    Clojure,
    Elixir,
    Erlang,
    Haskell,
    Julia,
    /// `.ml`. Parsed by `LANGUAGE_OCAML`; see [`DiffSyntaxLanguage::OCamlInterface`]
    /// for the other half of the pair.
    OCaml,
    /// `.mli`. A separate grammar rather than a mode of the one above: an
    /// interface file is a different language shape (`val f : int -> int` has no
    /// implementation counterpart), and upstream ships it as its own
    /// `LANGUAGE_OCAML_INTERFACE`. Both share queries/ocaml_highlights.scm.
    OCamlInterface,
    Solidity,
    /// Generic assembly -- GAS and Intel-flavoured alike. The grammar is
    /// dialect-agnostic, so it labels mnemonics and operands without knowing the
    /// target architecture.
    Assembly,
    Rust,
    Python,
    JavaScript,
    Jsdoc,
    TypeScript,
    Tsx,
    Regex,
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
    line_token_chunks: FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
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
mod live;
mod occurrences;
mod pairs;
mod prepared;

use heuristic::*;
use language::*;
pub(in crate::view) use occurrences::OCCURRENCE_MAX_TEXT_BYTES;
use occurrences::*;
use pairs::*;
use prepared::*;

pub(super) use heuristic::syntax_tokens_for_line_heuristic_into;
#[cfg(test)]
pub(super) use language::syntax_tokens_for_line;
pub(super) use language::syntax_tokens_for_line_shared;
pub(in crate::view) use language::{
    diff_syntax_language_for_code_fence_info, diff_syntax_language_for_path,
};
pub(in crate::view) use live::{
    LiveSyntaxDocument, LiveSyntaxSnapshot, LiveSyntaxSyncOutcome, live_syntax_document_supported,
    live_syntax_reparse,
};
pub(in crate::view) use pairs::SyntaxPair;
// Only assertions name a pair kind -- every kind is painted alike; see
// `DiffTextPairMatch::kind`.
#[cfg(test)]
pub(in crate::view) use pairs::SyntaxPairKind;
#[cfg(any(test, feature = "benchmarks"))]
pub(super) use prepared::has_pending_prepared_syntax_chunk_builds_for_document;
#[cfg(test)]
pub(super) use prepared::syntax_tokens_for_prepared_document_line;
pub(super) use prepared::{
    PreparedSyntaxLineTokensRequest, drain_completed_prepared_syntax_chunk_builds,
    drain_completed_prepared_syntax_chunk_builds_for_document,
    has_pending_prepared_syntax_chunk_builds, inject_prepared_document_data,
    prepare_treesitter_document_in_background_text_with_reparse_seed,
    prepare_treesitter_document_with_budget_reuse_text, prepared_document_reparse_seed,
    request_syntax_tokens_for_prepared_document_line,
    request_syntax_tokens_for_prepared_document_line_range_into,
};
pub(in crate::view) use prepared::{
    PreparedSyntaxPairHit, PreparedSyntaxPairSpan, prepared_document_occurrences_at_display_offset,
    prepared_document_syntax_pair_at_display_offset,
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

    /// The display<->raw conversion in isolation. This is the one piece of the
    /// feature that silently produces a plausible-but-wrong answer when it is
    /// wrong, so it is pinned on its own rather than only through a document.
    #[test]
    fn display_and_raw_offsets_round_trip_across_tabs() {
        // Two tabs then `x`: display columns 0..8 are the tabs, `x` is at 8.
        let line = "\t\tx = 1";
        assert_eq!(display_offset_for_raw_offset(line, 0), 0);
        assert_eq!(
            display_offset_for_raw_offset(line, 1),
            4,
            "one tab is 4 wide"
        );
        assert_eq!(display_offset_for_raw_offset(line, 2), 8);
        assert_eq!(
            display_offset_for_raw_offset(line, line.find('x').expect("x")),
            8
        );

        assert_eq!(raw_offset_for_display_offset(line, 0), 0);
        assert_eq!(raw_offset_for_display_offset(line, 4), 1);
        assert_eq!(raw_offset_for_display_offset(line, 8), 2);
        // A click landing inside an expanded tab resolves to that tab.
        assert_eq!(raw_offset_for_display_offset(line, 2), 0);
        assert_eq!(raw_offset_for_display_offset(line, 6), 1);

        // Round trip on every real byte boundary.
        for (raw, _) in line.char_indices() {
            assert_eq!(
                raw_offset_for_display_offset(line, display_offset_for_raw_offset(line, raw)),
                raw,
                "raw offset {raw} did not survive the round trip"
            );
        }

        // A tab-free line is the identity, including past the end.
        let plain = "let x = 1;";
        for raw in 0..=plain.len() {
            assert_eq!(display_offset_for_raw_offset(plain, raw), raw);
            assert_eq!(raw_offset_for_display_offset(plain, raw), raw);
        }
        assert_eq!(raw_offset_for_display_offset(plain, 999), plain.len());
    }

    #[test]
    fn prepared_syntax_pair_reports_display_columns_on_tab_indented_lines() {
        // Every body line is tab-indented, so a raw-offset answer would be three
        // columns per indent level to the left of where the row was painted.
        let text = "fn main() {\n\tif a {\n\t\tb();\n\t}\n}\n";
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        // Line 1 is "\tif a {": display columns 0..4 are the tab, so the `{` is
        // painted at column 9 and its raw offset within the line is 6.
        let hit = prepared_document_syntax_pair_at_display_offset(document, 1, 9)
            .expect("the `if` block braces pair");
        assert_eq!(hit.kind, SyntaxPairKind::Bracket);
        assert_eq!(hit.open.len(), 1);
        assert_eq!(hit.open[0].line_ix, 1);
        assert_eq!(
            hit.open[0].display_range,
            9..10,
            "the open brace is reported where the canvas painted it, not at raw offset 6"
        );
        assert_eq!(hit.close.len(), 1);
        assert_eq!(hit.close[0].line_ix, 3);
        assert_eq!(
            hit.close[0].display_range,
            4..5,
            "the closing brace sits after one expanded tab"
        );
    }

    #[test]
    fn prepared_syntax_pair_projects_each_end_onto_its_own_line() {
        let text = "<div class=\"card\">\n  <span>hi</span>\n</div>\n";
        let document = prepare_test_document(DiffSyntaxLanguage::Html, text);

        // Caret on the inner element's text: an inner pair, both ends on line 1.
        let inner = prepared_document_syntax_pair_at_display_offset(document, 1, 9)
            .expect("the span element pairs");
        assert_eq!(inner.kind, SyntaxPairKind::Tag);
        assert_eq!(inner.open[0].line_ix, 1);
        assert_eq!(inner.close[0].line_ix, 1);
        // `  <span>hi</span>`: two spaces, then the tags at columns 2 and 10.
        assert_eq!(inner.open[0].display_range, 2..8);
        assert_eq!(inner.close[0].display_range, 10..17);

        // Caret on the outer element name: ends on lines 0 and 2, whole tags.
        let outer = prepared_document_syntax_pair_at_display_offset(document, 0, 2)
            .expect("the div element pairs");
        assert_eq!(outer.kind, SyntaxPairKind::Tag);
        assert_eq!(outer.open[0].line_ix, 0);
        assert_eq!(
            outer.open[0].display_range,
            0..18,
            "the whole start tag, attributes included"
        );
        assert_eq!(outer.close[0].line_ix, 2);
        assert_eq!(outer.close[0].display_range, 0..6);
    }

    /// A start tag split across lines is one delimiter but several rows, so it
    /// reports one span per row rather than washing only its first line.
    #[test]
    fn prepared_syntax_pair_spans_every_line_a_tag_covers() {
        let text = "<div\n  class=\"card\"\n  id=\"x\">\ntext\n</div>\n";
        let document = prepare_test_document(DiffSyntaxLanguage::Html, text);

        let hit = prepared_document_syntax_pair_at_display_offset(document, 3, 1)
            .expect("the caret in the element content pairs its tags");
        assert_eq!(hit.kind, SyntaxPairKind::Tag);
        assert_eq!(
            hit.open
                .iter()
                .map(|span| (span.line_ix, span.display_range.clone()))
                .collect::<Vec<_>>(),
            vec![(0, 0..4), (1, 0..14), (2, 0..9)],
            "every line of the start tag is washed, not just the first"
        );
        assert_eq!(hit.close.len(), 1);
        assert_eq!(hit.close[0].line_ix, 4);
    }

    fn occurrences_in(
        language: DiffSyntaxLanguage,
        text: &str,
        offset: usize,
    ) -> Option<SyntaxOccurrences> {
        let input = treesitter_document_input_from_text(text);
        let spec = tree_sitter_highlight_spec(language)?;
        let tree = with_ts_parser_parse_result(&spec.ts_language, |parser| {
            parse_treesitter_tree(parser, input.text.as_bytes(), None, None)
        })?;
        syntax_occurrences_in_tree(&tree, text, offset)
    }

    /// The clicked name lights everywhere the grammar also tokenised it.
    #[test]
    fn occurrences_find_every_use_of_the_clicked_name() {
        let text = "fn main() {\n    let values = vec![1];\n    for v in values {}\n}\n";
        let click = text.find("values").expect("declaration");
        let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");

        assert_eq!(found.token, click..click + "values".len());
        assert_eq!(
            found
                .ranges
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["values", "values"],
        );
        assert_eq!(found.ranges[1].start, text.rfind("values").expect("use"));
    }

    /// A name inside a comment or a string is content, not a use of the symbol.
    #[test]
    fn occurrences_skip_comments_and_string_bodies() {
        let text = concat!(
            "fn main() {\n",
            "    let total = 1;\n",
            "    // total is not a use\n",
            "    let label = \"total\";\n",
            "    let sum = total + 1;\n",
            "}\n",
        );
        let click = text.find("total").expect("declaration");
        let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");

        assert_eq!(found.ranges.len(), 2, "declaration and the later use only");
        for range in &found.ranges {
            assert_eq!(&text[range.clone()], "total");
            assert!(
                !text[..range.start].ends_with("// ") && !text[..range.start].ends_with('"'),
                "matched inside a comment or string: {range:?}"
            );
        }
        assert_eq!(found.ranges[1].start, text.rfind("total").expect("use"));
    }

    /// A longer word that merely contains the name is not a use of it.
    #[test]
    fn occurrences_respect_word_boundaries() {
        let text = "fn main() {\n    let sum = 1;\n    let summary = 2;\n    let x = sum;\n}\n";
        let click = text.find("sum").expect("declaration");
        let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");

        assert_eq!(found.ranges.len(), 2, "`summary` must not match `sum`");
        assert!(
            found
                .ranges
                .iter()
                .all(|range| &text[range.clone()] == "sum"),
        );
    }

    /// A name whose first character is multi-byte must still light up.
    ///
    /// The scan used to step one byte past each hit, which lands inside the
    /// leading character of such a name; the slice then failed and the `?` threw
    /// away every match found so far, including the clicked one, so the whole
    /// highlight silently vanished.
    #[test]
    fn occurrences_handle_non_ascii_names() {
        let text = "x = 1\ncafé = 2\ny = café\n";
        let click = text.find("café").expect("name");
        let found = occurrences_in(DiffSyntaxLanguage::Python, text, click).expect("a name");
        assert_eq!(
            found
                .ranges
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["café", "café"],
        );
        assert_eq!(found.ranges[1].start, text.rfind("café").expect("use"));

        // And a name that is entirely multi-byte.
        let cjk = "日本語 = 1\nz = 日本語\n";
        let at = cjk.find("日本語").expect("name");
        let found = occurrences_in(DiffSyntaxLanguage::Python, cjk, at).expect("a name");
        assert_eq!(found.ranges.len(), 2, "got {:?}", found.ranges);
    }

    /// Clicking punctuation, whitespace or a literal is not clicking a name.
    #[test]
    fn occurrences_are_none_off_a_name() {
        let text = "fn main() {\n    let n = 1234;\n}\n";
        for probe in ["{", " 1234", ";"] {
            let at = text.find(probe).expect("probe");
            assert_eq!(
                occurrences_in(DiffSyntaxLanguage::Rust, text, at + 1).map(|found| found.token),
                None,
                "clicking {probe:?} should not name anything",
            );
        }
    }

    /// Fields and calls are names too, which is the point of asking the tree
    /// rather than scanning for words.
    #[test]
    fn occurrences_cover_calls_and_fields() {
        let text = concat!(
            "fn main() {\n",
            "    let item = Item { width: 1 };\n",
            "    let w = item.width;\n",
            "    resize(item.width);\n",
            "}\n",
        );
        let click = text.find("width").expect("field declaration");
        let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");
        assert_eq!(
            found.ranges.len(),
            3,
            "the shorthand field and both reads, got {:?}",
            found
                .ranges
                .iter()
                .map(|r| &text[r.clone()])
                .collect::<Vec<_>>()
        );
    }

    /// Every wired language, a snippet of it, and the kinds it must colour.
    ///
    /// The bar is what a reader looks for rather than what a grammar happens to
    /// offer: comments must not read as code, literals must not read as names,
    /// and brackets must not be flat. Data formats are held to what they have --
    /// no functions in JSON -- but a programming language that cannot colour a
    /// call or a bracket has a gap worth fixing, not a style to accept.
    ///
    /// Adding a grammar without adding a row here leaves it unguarded, which is
    /// how Objective-C shipped with no comment colour at all.
    const LANGUAGE_BASELINES: &[(DiffSyntaxLanguage, &str, &[SyntaxTokenKind])] = {
        use SyntaxTokenKind as K;
        &[
            (
                DiffSyntaxLanguage::Rust,
                "// c\nfn main() {\n    let x = \"s\";\n    let n = 1;\n    f(x);\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Python,
                "# c\ndef main():\n    x = \"s\"\n    n = 1\n    f(x)\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::JavaScript,
                "// c\nfunction main() {\n  const x = \"s\";\n  const n = 1;\n  f(x);\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::TypeScript,
                "// c\nfunction main(): void {\n  const x: string = \"s\";\n  const n = 1;\n  f(x);\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                    K::Type,
                ],
            ),
            (
                DiffSyntaxLanguage::Tsx,
                "// c\nconst a = <div id=\"x\">{f(1)}</div>;\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Tag,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Go,
                "// c\nfunc main() {\n\tx := \"s\"\n\tn := 1\n\tf(x)\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::C,
                "// c\nint main(void) {\n  const char *x = \"s\";\n  int n = 1;\n  f(x);\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                    K::Type,
                ],
            ),
            (
                DiffSyntaxLanguage::Cpp,
                "// c\nclass A {};\nint main() {\n  std::string x = \"s\";\n  return 1;\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                    K::Type,
                ],
            ),
            (
                DiffSyntaxLanguage::ObjectiveC,
                "// c\n@implementation Foo\n- (void)bar {\n  NSString *s = @\"hi\";\n  int n = 42;\n}\n@end\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                    K::Type,
                ],
            ),
            (
                DiffSyntaxLanguage::CSharp,
                "// c\nclass Foo {\n  void Bar() {\n    var x = \"s\";\n    int n = 1;\n  }\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                    K::Type,
                ],
            ),
            (
                DiffSyntaxLanguage::Java,
                "// c\nclass Foo {\n  int count = 1;\n  void bar() {\n    f(\"s\");\n  }\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                    K::Type,
                ],
            ),
            (
                DiffSyntaxLanguage::Kotlin,
                "// c\nfun main() {\n  val x = \"s\"\n  val n = 1\n  f(x)\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Scala,
                "// c\nobject Foo {\n  def bar(): Unit = {\n    val x = \"s\"\n    val n = 1\n  }\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Groovy,
                "// c\nclass Foo {\n  int count = 1\n  def bar() { f(\"s\") }\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Php,
                "<?php\n// c\nclass Foo {\n  public $count = 1;\n  function bar() { return f(\"s\"); }\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Ruby,
                "# c\nclass Foo\n  def bar\n    x = \"s\"\n    n = 1\n    f(x)\n  end\nend\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Swift,
                "// c\nfunc main() {\n  let x = \"s\"\n  let n = 1\n  f(x)\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Dart,
                "// c\nvoid main() {\n  var x = \"s\";\n  var n = 1;\n  f(x);\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Lua,
                "-- c\nlocal function main()\n  local x = \"s\"\n  local n = 1\n  f(x)\nend\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Bash,
                "# c\nmain() {\n  x=\"s\"\n  n=1\n  f \"$x\"\n}\n",
                &[K::Comment, K::String, K::Number, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::PowerShell,
                "# c\nfunction Get-Thing {\n  $x = \"s\"\n  $n = 1\n  Write-Host $x\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Zig,
                "// c\npub fn main() void {\n    const x = \"s\";\n    const n = 1;\n    f(x);\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Nix,
                "# c\n{\n  x = \"s\";\n  n = 1;\n}\n",
                &[K::Comment, K::String, K::Number, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Hcl,
                "# c\nresource \"a\" \"b\" {\n  n = 1\n  s = \"x\"\n  v = f(\"y\")\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Bicep,
                "// c\nparam name string = 's'\nvar n = 1\n",
                &[K::Comment, K::String, K::Number, K::Keyword],
            ),
            (
                DiffSyntaxLanguage::Sql,
                "-- c\nSELECT id, name FROM t WHERE n = 1 AND s = 'x';\n",
                &[K::Comment, K::String, K::Number, K::Keyword],
            ),
            (
                DiffSyntaxLanguage::R,
                "# c\nmain <- function() {\n  x <- \"s\"\n  n <- 1\n  f(x)\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Function,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Julia,
                "# c\nfunction main()\n    x = \"s\"\n    n = 1\n    f(x)\nend\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Haskell,
                "-- c\nmain :: IO ()\nmain = do\n  let x = \"s\"\n  let n = 1\n  f x\n",
                &[K::Comment, K::String, K::Number, K::Keyword],
            ),
            (
                DiffSyntaxLanguage::Elixir,
                "# c\ndefmodule Foo do\n  def bar do\n    x = \"s\"\n    n = 1\n    f(x)\n  end\nend\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Erlang,
                "% c\n-module(foo).\nbar() ->\n    X = \"s\",\n    N = 1,\n    f(X).\n",
                &[K::Comment, K::String, K::Number, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::OCaml,
                "(* c *)\nlet main () =\n  let x = \"s\" in\n  let n = 1 in\n  f x\n",
                &[K::Comment, K::String, K::Number, K::Keyword],
            ),
            (
                DiffSyntaxLanguage::FSharp,
                "// c\nlet main () =\n  let x = \"s\"\n  let n = 1\n  f x\n",
                &[K::Comment, K::String, K::Number, K::Keyword],
            ),
            (
                DiffSyntaxLanguage::Clojure,
                ";; c\n(defn main []\n  (let [x \"s\" n 1]\n    (f x)))\n",
                &[K::Comment, K::String, K::Number, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Solidity,
                "// c\ncontract Foo {\n  uint n = 1;\n  function bar() public { f(\"s\"); }\n}\n",
                &[
                    K::Comment,
                    K::String,
                    K::Number,
                    K::Keyword,
                    K::PunctuationBracket,
                ],
            ),
            (
                DiffSyntaxLanguage::Assembly,
                "; c\n.section .text\nmain:\n    mov $1, %eax\n    ret\n",
                // Instructions are `@function` and labels `@label` in this
                // grammar; there is no separate keyword class to ask for.
                &[K::Comment, K::Number, K::Function, K::Label],
            ),
            (
                DiffSyntaxLanguage::Makefile,
                "# c\nall: build\n\techo \"s\"\n",
                // A recipe line is opaque shell text to this grammar, so nothing
                // inside it is coloured; only the makefile's own syntax is.
                &[K::Comment, K::Constant, K::PunctuationDelimiter],
            ),
            (
                DiffSyntaxLanguage::Css,
                "/* c */\n.a { color: red; width: 1px; }\n",
                &[K::Comment, K::Number, K::Property, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Html,
                "<!-- c -->\n<div id=\"x\">t</div>\n",
                &[K::Comment, K::String, K::Tag, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Xml,
                "<!-- c -->\n<root a=\"x\">t</root>\n",
                &[K::Comment, K::String, K::Tag, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Vue,
                "<template>\n  <div id=\"x\">t</div>\n</template>\n",
                &[K::String, K::Tag, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Svelte,
                "<script>\n  let n = 1;\n</script>\n<div id=\"x\">t</div>\n",
                &[K::Tag, K::String, K::PunctuationBracket],
            ),
            (
                DiffSyntaxLanguage::Json,
                "{\n  \"a\": \"s\",\n  \"n\": 1,\n  \"b\": true\n}\n",
                &[
                    K::String,
                    K::Number,
                    K::Property,
                    K::PunctuationBracket,
                    K::Boolean,
                ],
            ),
            (
                DiffSyntaxLanguage::Yaml,
                "# c\na: \"s\"\nn: 1\nb: true\n",
                &[K::Comment, K::String, K::Number, K::Property],
            ),
            (
                DiffSyntaxLanguage::Toml,
                "# c\n[t]\na = \"s\"\nn = 1\n",
                &[K::Comment, K::String, K::Number, K::Property],
            ),
            (
                DiffSyntaxLanguage::GoMod,
                "// c\nmodule example.com/m\n\ngo 1.22\n",
                &[K::Comment, K::Keyword],
            ),
            (
                DiffSyntaxLanguage::Markdown,
                "# Title\n\nSome *text* and [a link](http://x).\n",
                &[K::MarkupHeading],
            ),
        ]
    };

    /// Every token kind a sample comes out coloured with.
    ///
    /// The one definition of "what this language emits", so the per-language
    /// tests below and the baseline sweep cannot disagree about what counts.
    fn token_kinds_in_sample(language: DiffSyntaxLanguage, text: &str) -> Vec<SyntaxTokenKind> {
        let document = prepare_test_document(language, text);
        let mut seen: Vec<SyntaxTokenKind> = Vec::new();
        for ix in 0..text.lines().count() {
            if let Some(chunk) = syntax_tokens_for_prepared_document_line(document, ix) {
                seen.extend(chunk.iter().map(|token| token.kind));
            }
        }
        seen
    }

    /// The kinds a language must be able to colour before it counts as wired.
    ///
    /// Not a style preference: each of these is something a reader looks for. A
    /// grammar whose query cannot emit `Comment` leaves comments the same colour
    /// as code; one with no `PunctuationBracket` leaves every brace flat.
    fn assert_language_colours(
        language: DiffSyntaxLanguage,
        text: &str,
        required: &[SyntaxTokenKind],
    ) {
        let seen = token_kinds_in_sample(language, text);
        for kind in required {
            assert!(
                seen.contains(kind),
                "{language:?} never produced {kind:?}; it emitted {seen:?}"
            );
        }
    }

    /// Every language in [`LANGUAGE_BASELINES`] colours what it must.
    ///
    /// Reports every gap in one run rather than stopping at the first, so a
    /// grammar batch can be assessed in one go.
    #[test]
    fn every_wired_language_meets_its_highlight_baseline() {
        let mut gaps: Vec<String> = Vec::new();
        for (language, sample, required) in LANGUAGE_BASELINES {
            let mut seen = token_kinds_in_sample(*language, sample);
            let missing: Vec<_> = required
                .iter()
                .filter(|kind| !seen.contains(kind))
                .collect();
            if !missing.is_empty() {
                seen.sort_by_key(|kind| format!("{kind:?}"));
                seen.dedup();
                gaps.push(format!("{language:?} missing {missing:?} (has {seen:?})"));
            }
        }
        assert!(gaps.is_empty(), "highlight gaps:\n  {}", gaps.join("\n  "));
    }

    /// Objective-C's own query captures neither comments, strings nor numbers,
    /// so a `.m` file used to render those as plain code.
    #[test]
    fn objective_c_colours_comments_strings_and_numbers() {
        assert_language_colours(
            DiffSyntaxLanguage::ObjectiveC,
            "// a comment\n@implementation Foo\n- (void)bar {\n  NSString *s = @\"hi\";\n  int n = 42;\n}\n@end\n",
            &[
                SyntaxTokenKind::Comment,
                SyntaxTokenKind::String,
                SyntaxTokenKind::Number,
                SyntaxTokenKind::Type,
                SyntaxTokenKind::PunctuationBracket,
            ],
        );
    }

    /// Java, PHP and Groovy all use queries that capture no punctuation at all.
    #[test]
    fn jvm_and_php_family_colour_their_brackets() {
        assert_language_colours(
            DiffSyntaxLanguage::Java,
            "// c\nclass Foo {\n  int count = 1;\n  void bar() {\n    this.count = other.value;\n  }\n}\n",
            &[
                SyntaxTokenKind::Comment,
                SyntaxTokenKind::PunctuationBracket,
                SyntaxTokenKind::PunctuationDelimiter,
                SyntaxTokenKind::Property,
            ],
        );
        assert_language_colours(
            DiffSyntaxLanguage::Php,
            "<?php\n// c\nclass Foo {\n  public $count = 1;\n  function bar() { return $this->count; }\n}\n",
            &[
                SyntaxTokenKind::Comment,
                SyntaxTokenKind::PunctuationBracket,
                SyntaxTokenKind::PunctuationDelimiter,
                SyntaxTokenKind::Property,
            ],
        );
        assert_language_colours(
            DiffSyntaxLanguage::Groovy,
            "// c\nclass Foo {\n  int count = 1\n  def bar() { return this.count }\n}\n",
            &[
                SyntaxTokenKind::Comment,
                SyntaxTokenKind::PunctuationBracket,
                SyntaxTokenKind::Keyword,
            ],
        );
    }

    /// PowerShell's query tags `(array_expression)` `@array`, which spans the
    /// whole `@(1, 2)` -- parens, commas and the spaces between. Mapping that to
    /// a type made an array literal read as one long type name.
    #[test]
    fn powershell_array_literals_are_not_one_long_type() {
        let text = "# c\nfunction Get-Thing {\n  $x = @(1, 2)\n  return $x.Count\n}\n";
        let document = prepare_test_document(DiffSyntaxLanguage::PowerShell, text);
        let line = text.lines().nth(2).expect("array line");
        let tokens = syntax_tokens_for_prepared_document_line(document, 2)
            .map(|chunk| chunk.to_vec())
            .unwrap_or_default();
        let rendered: Vec<(&str, SyntaxTokenKind)> = tokens
            .iter()
            .map(|token| (&line[token.range.clone()], token.kind))
            .collect();
        assert!(
            !rendered
                .iter()
                .any(|(_, kind)| *kind == SyntaxTokenKind::Type),
            "nothing on an array literal line is a type, got {rendered:?}"
        );
        assert!(
            rendered.contains(&("@(", SyntaxTokenKind::PunctuationBracket))
                && rendered.contains(&(")", SyntaxTokenKind::PunctuationBracket)),
            "the array's own brackets should be brackets, got {rendered:?}"
        );
        assert!(
            rendered.contains(&("1", SyntaxTokenKind::Number)),
            "its elements keep their own colours, got {rendered:?}"
        );
    }

    /// Terraform gets real syntax, and with it the tree that delimiter matching
    /// and name highlighting need.
    ///
    /// `tree-sitter-hcl` ships no highlights query, so `.tf` was heuristic-only
    /// and had no document tree at all: no bracket pairs, no occurrences.
    /// `queries/hcl_highlights.scm` is authored in-tree for exactly that.
    #[test]
    fn terraform_files_get_tree_sitter_tokens() {
        let text = concat!(
            "# managed by terraform\n",
            "resource \"aws_instance\" \"web\" {\n",
            "  ami           = var.ami_id\n",
            "  count         = 2\n",
            "  enabled       = true\n",
            "  name          = \"web-${var.env}\"\n",
            "  user_data     = file(\"init.sh\")\n",
            "}\n",
        );
        assert_eq!(
            diff_syntax_language_for_path(std::path::Path::new("main.tf")),
            Some(DiffSyntaxLanguage::Hcl)
        );
        let document = prepare_test_document(DiffSyntaxLanguage::Hcl, text);
        let kinds = |ix: usize| -> Vec<(String, SyntaxTokenKind)> {
            let line = text.lines().nth(ix).expect("line");
            syntax_tokens_for_prepared_document_line(document, ix)
                .map(|chunk| {
                    chunk
                        .iter()
                        .map(|t| (line[t.range.clone()].to_string(), t.kind))
                        .collect()
                })
                .unwrap_or_default()
        };

        assert_eq!(
            kinds(0),
            vec![("# managed by terraform".into(), SyntaxTokenKind::Comment)]
        );
        let header = kinds(1);
        assert!(
            header.contains(&("resource".into(), SyntaxTokenKind::Keyword))
                && header.contains(&("\"aws_instance\"".into(), SyntaxTokenKind::String)),
            "block header should name its type and labels, got {header:?}"
        );
        assert!(
            kinds(3).contains(&("2".into(), SyntaxTokenKind::Number)),
            "numbers should be numbers, got {:?}",
            kinds(3)
        );
        assert!(
            kinds(4).contains(&("true".into(), SyntaxTokenKind::Boolean)),
            "booleans should be booleans, got {:?}",
            kinds(4)
        );
        // An interpolated string keeps its literal halves coloured around `${}`.
        let interpolated = kinds(5);
        assert!(
            interpolated.contains(&("\"web-".into(), SyntaxTokenKind::String))
                && interpolated.contains(&("${".into(), SyntaxTokenKind::PunctuationSpecial))
                && interpolated.contains(&("var".into(), SyntaxTokenKind::Variable)),
            "interpolation should split string from code, got {interpolated:?}"
        );
        assert!(
            kinds(6).contains(&("file".into(), SyntaxTokenKind::Function)),
            "calls should be functions, got {:?}",
            kinds(6)
        );
    }

    /// And with a tree in hand, both click features work in Terraform.
    #[test]
    fn terraform_supports_pairs_and_occurrences() {
        let text = concat!(
            "resource \"aws_instance\" \"web\" {\n",
            "  ami   = var.ami_id\n",
            "  other = var.ami_id\n",
            "}\n",
        );
        let document = prepare_test_document(DiffSyntaxLanguage::Hcl, text);

        // The block braces pair across the whole resource.
        let hit = prepared_document_syntax_pair_at_display_offset(document, 1, 4)
            .expect("the resource block braces should pair");
        assert_eq!(hit.kind, SyntaxPairKind::Bracket);
        assert_eq!(hit.open[0].line_ix, 0);
        assert_eq!(hit.close[0].line_ix, 3);

        // And `ami_id` is a name with two uses.
        let column = text
            .lines()
            .nth(1)
            .expect("line")
            .find("ami_id")
            .expect("name");
        let spans = prepared_document_occurrences_at_display_offset(document, 1, column);
        assert_eq!(
            spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// A prose-only `/** ... */` must still read as a comment.
    ///
    /// The jsdoc injection subtracts the host's `(comment) @comment` from the
    /// document before painting its own captures, so a doc comment with no
    /// `@tag` in it used to come out with no colour at all -- plain foreground
    /// text in the middle of a file. The base capture in `jsdoc_highlights.scm`
    /// is what puts the comment colour back.
    #[test]
    fn jsdoc_comments_keep_their_comment_colour() {
        let text = concat!(
            "/**\n",
            " * Where to go after signing in.\n",
            " *\n",
            " * Same-origin paths only, so no open redirect.\n",
            " */\n",
            "export function f() {}\n",
        );
        let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
        for (ix, line) in text.lines().enumerate().take(5) {
            let tokens = syntax_tokens_for_prepared_document_line(document, ix)
                .map(|chunk| chunk.to_vec())
                .unwrap_or_default();
            assert!(
                !tokens.is_empty(),
                "line {ix} ({line:?}) of a doc comment has no tokens at all"
            );
            assert!(
                tokens
                    .iter()
                    .all(|token| token.kind == SyntaxTokenKind::Comment),
                "line {ix} ({line:?}) should be all comment, got {tokens:?}"
            );
        }
    }

    /// ...and a tagged one still gets its tag, type and name picked out of it.
    #[test]
    fn jsdoc_tags_still_win_over_the_comment_base() {
        let text = "/**\n * @param {string} search - the query.\n */\nlet a = 1;\n";
        let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
        let line = text.lines().nth(1).expect("tag line");
        let tokens = syntax_tokens_for_prepared_document_line(document, 1)
            .map(|chunk| chunk.to_vec())
            .unwrap_or_default();
        let kinds: Vec<(&str, SyntaxTokenKind)> = tokens
            .iter()
            .map(|token| (&line[token.range.clone()], token.kind))
            .collect();
        assert!(
            kinds.contains(&("@param", SyntaxTokenKind::Keyword)),
            "expected the tag to stay a keyword, got {kinds:?}"
        );
        assert!(
            kinds.contains(&("string", SyntaxTokenKind::Type)),
            "expected the type to stay a type, got {kinds:?}"
        );
        assert!(
            kinds
                .iter()
                .any(|(text, kind)| *kind == SyntaxTokenKind::Comment && text.contains("query")),
            "the prose around the tag should still be comment, got {kinds:?}"
        );
    }

    /// A JSX expression comment spans several rows, and every one of them is
    /// comment -- backticks and quotes inside it are prose, not code.
    #[test]
    fn jsx_expression_comments_stay_comments_on_every_line() {
        let text = concat!(
            "const a = (\n",
            "  <div>\n",
            "    {/* `exact` matters: `/app/device` is a prefix of\n",
            "    `/app/devices`, so a user clicking \"Devices\" would land\n",
            "    on the wrong page. */}\n",
            "  </div>\n",
            ");\n",
        );
        let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
        for ix in 2..=4 {
            let line = text.lines().nth(ix).expect("comment line");
            let tokens = syntax_tokens_for_prepared_document_line(document, ix)
                .map(|chunk| chunk.to_vec())
                .unwrap_or_default();
            let non_comment: Vec<_> = tokens
                .iter()
                .filter(|token| token.kind != SyntaxTokenKind::Comment)
                .map(|token| (&line[token.range.clone()], token.kind))
                .collect();
            // Only the braces holding the expression may be anything else.
            assert!(
                non_comment
                    .iter()
                    .all(|(text, _)| *text == "{" || *text == "}"),
                "line {ix} ({line:?}) coloured non-comment spans: {non_comment:?}"
            );
        }
    }

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

    // ---- Heuristic fallback: Nix and Jinja ------------------------------------

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
            heuristic_keywords("  buildInputs = [ pkgs.hello ];", DiffSyntaxLanguage::Nix)
                .is_empty(),
            "an ordinary Nix attribute name must not colour as a keyword"
        );

        // The Jinja table omits any identifier that could also be an HTML attribute
        // name or an English word: the heuristic sees the whole line.
        let found =
            heuristic_keywords("{% endif %}{% extends 'base' %}", DiffSyntaxLanguage::Jinja);
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

        let html_again =
            syntax_tokens_for_line(text, DiffSyntaxLanguage::Html, DiffSyntaxMode::Auto);
        assert_eq!(html_again, html);
    }

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
        let rust_doc =
            prepare_markdown_document(&["```rs", "fn main() { let value = 42; }", "```"]);
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

    /// A single-file component covering every Vue injection path at once.
    /// Line indices are asserted against by the tests below, so keep them stable.
    const VUE_SFC_FIXTURE: &[&str] = &[
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
            TreesitterCachedDocument::from_chunked_line_tokens(128, FxHashMap::default(), None);
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
            syntax_kind_from_capture_name("constructor"),
            Some(SyntaxTokenKind::Constructor)
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
            syntax_kind_from_capture_name("namespace"),
            Some(SyntaxTokenKind::Namespace)
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
            syntax_kind_from_capture_name("variable.builtin"),
            Some(SyntaxTokenKind::VariableBuiltin)
        );
        assert_eq!(
            syntax_kind_from_capture_name("label"),
            Some(SyntaxTokenKind::Label)
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
            syntax_kind_from_capture_name("punctuation.special"),
            Some(SyntaxTokenKind::PunctuationSpecial)
        );
        assert_eq!(
            syntax_kind_from_capture_name("punctuation.list_marker.markup"),
            Some(SyntaxTokenKind::PunctuationListMarker)
        );
        assert_eq!(
            syntax_kind_from_capture_name("punctuation.list_marker"),
            Some(SyntaxTokenKind::PunctuationListMarker)
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
            syntax_kind_from_capture_name("preproc"),
            Some(SyntaxTokenKind::Preproc)
        );
        assert_eq!(
            syntax_kind_from_capture_name("string.regex"),
            Some(SyntaxTokenKind::StringRegex)
        );
        assert_eq!(
            syntax_kind_from_capture_name("string.regexp"),
            Some(SyntaxTokenKind::StringRegex)
        );
        assert_eq!(
            syntax_kind_from_capture_name("string.special.regex"),
            Some(SyntaxTokenKind::StringRegex)
        );
        assert_eq!(
            syntax_kind_from_capture_name("string.special.symbol"),
            Some(SyntaxTokenKind::StringSpecial)
        );
        assert_eq!(
            syntax_kind_from_capture_name("constant.builtin"),
            Some(SyntaxTokenKind::ConstantBuiltin)
        );
        assert_eq!(
            syntax_kind_from_capture_name("markup.heading"),
            Some(SyntaxTokenKind::MarkupHeading)
        );
        assert_eq!(
            syntax_kind_from_capture_name("title.markup"),
            Some(SyntaxTokenKind::MarkupHeading)
        );
        assert_eq!(
            syntax_kind_from_capture_name("markup.link.url"),
            Some(SyntaxTokenKind::MarkupLink)
        );
        assert_eq!(
            syntax_kind_from_capture_name("link_uri.markup"),
            Some(SyntaxTokenKind::MarkupLink)
        );
        assert_eq!(
            syntax_kind_from_capture_name("text.uri"),
            Some(SyntaxTokenKind::MarkupLink)
        );
        assert_eq!(
            syntax_kind_from_capture_name("text.literal.markup"),
            Some(SyntaxTokenKind::TextLiteral)
        );
        assert_eq!(
            syntax_kind_from_capture_name("text.literal"),
            Some(SyntaxTokenKind::TextLiteral)
        );
        assert_eq!(
            syntax_kind_from_capture_name("text.title"),
            Some(SyntaxTokenKind::MarkupHeading)
        );
        assert_eq!(
            syntax_kind_from_capture_name("diff.plus"),
            Some(SyntaxTokenKind::DiffPlus)
        );
        assert_eq!(
            syntax_kind_from_capture_name("diff.minus"),
            Some(SyntaxTokenKind::DiffMinus)
        );
        assert_eq!(
            syntax_kind_from_capture_name("diff.delta"),
            Some(SyntaxTokenKind::DiffDelta)
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

    #[test]
    fn normalize_non_overlapping_tokens_splits_outer_token_for_inner_semantics() {
        let tokens = normalize_non_overlapping_tokens(vec![
            SyntaxToken {
                range: 0..22,
                kind: SyntaxTokenKind::Comment,
            },
            SyntaxToken {
                range: 2..10,
                kind: SyntaxTokenKind::DiffPlus,
            },
            SyntaxToken {
                range: 12..22,
                kind: SyntaxTokenKind::StringSpecial,
            },
        ]);
        assert_eq!(
            tokens,
            vec![
                SyntaxToken {
                    range: 0..2,
                    kind: SyntaxTokenKind::Comment,
                },
                SyntaxToken {
                    range: 2..10,
                    kind: SyntaxTokenKind::DiffPlus,
                },
                SyntaxToken {
                    range: 10..12,
                    kind: SyntaxTokenKind::Comment,
                },
                SyntaxToken {
                    range: 12..22,
                    kind: SyntaxTokenKind::StringSpecial,
                },
            ]
        );
    }

    #[test]
    fn normalize_non_overlapping_tokens_splits_contained_later_token() {
        let tokens = normalize_non_overlapping_tokens(vec![
            SyntaxToken {
                range: 0..10,
                kind: SyntaxTokenKind::Function,
            },
            SyntaxToken {
                range: 4..6,
                kind: SyntaxTokenKind::Operator,
            },
        ]);
        assert_eq!(
            tokens,
            vec![
                SyntaxToken {
                    range: 0..4,
                    kind: SyntaxTokenKind::Function,
                },
                SyntaxToken {
                    range: 4..6,
                    kind: SyntaxTokenKind::Operator,
                },
                SyntaxToken {
                    range: 6..10,
                    kind: SyntaxTokenKind::Function,
                },
            ]
        );
    }

    #[test]
    fn normalize_non_overlapping_tokens_assigns_partial_overlap_to_later_token() {
        let tokens = normalize_non_overlapping_tokens(vec![
            SyntaxToken {
                range: 0..8,
                kind: SyntaxTokenKind::Comment,
            },
            SyntaxToken {
                range: 5..12,
                kind: SyntaxTokenKind::DiffMinus,
            },
        ]);
        assert_eq!(
            tokens,
            vec![
                SyntaxToken {
                    range: 0..5,
                    kind: SyntaxTokenKind::Comment,
                },
                SyntaxToken {
                    range: 5..12,
                    kind: SyntaxTokenKind::DiffMinus,
                },
            ]
        );
    }

    #[test]
    fn vendored_rust_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let source = RUST_HIGHLIGHTS_QUERY;
        tree_sitter::Query::new(&lang, source)
            .expect("vendored Rust highlights.scm should compile");
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
        tree_sitter::Query::new(&lang, source)
            .expect("vendored HTML highlights.scm should compile");
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
                let language =
                    diff_syntax_language_for_code_fence_info(value).unwrap_or_else(|| {
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
        let doc =
            prepare_vue_document(&["<style lang=\"pcss\">", ".a { color: red; }", "</style>"]);
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

    #[test]
    fn typescript_treesitter_preserves_arrow_operator_inside_arrow_function() {
        let text = "const fn = (x: number): number => x + 1;";
        let tokens =
            syntax_tokens_for_line(text, DiffSyntaxLanguage::TypeScript, DiffSyntaxMode::Auto);
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
        assert_capture_names_are_supported(
            tree_sitter_rust::LANGUAGE.into(),
            RUST_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_html::LANGUAGE.into(),
            HTML_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(tree_sitter_vue::LANGUAGE.into(), VUE_HIGHLIGHTS_QUERY);
        assert_capture_names_are_supported(tree_sitter_css::LANGUAGE.into(), CSS_HIGHLIGHTS_QUERY);
        assert_capture_names_are_supported(
            tree_sitter_bash::LANGUAGE.into(),
            BASH_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_javascript::LANGUAGE.into(),
            JAVASCRIPT_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_python::LANGUAGE.into(),
            PYTHON_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(tree_sitter_go::LANGUAGE.into(), GO_HIGHLIGHTS_QUERY);
        assert_capture_names_are_supported(
            tree_sitter_json::LANGUAGE.into(),
            JSON_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_yaml::LANGUAGE.into(),
            YAML_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TYPESCRIPT_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TSX_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_xml::LANGUAGE_XML.into(),
            XML_HIGHLIGHTS_QUERY,
        );
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
        assert_capture_names_are_supported(
            tree_sitter_ruby::LANGUAGE.into(),
            tree_sitter_ruby::HIGHLIGHTS_QUERY,
        );
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
        assert_capture_names_are_supported(
            tree_sitter_zig::LANGUAGE.into(),
            tree_sitter_zig::HIGHLIGHTS_QUERY,
        );
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
            tree_sitter_haskell::HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_julia::LANGUAGE.into(),
            JULIA_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_ocaml::LANGUAGE_OCAML.into(),
            OCAML_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_solidity::LANGUAGE.into(),
            SOLIDITY_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_asm::LANGUAGE.into(),
            tree_sitter_asm::HIGHLIGHTS_QUERY,
        );
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
        assert_capture_names_are_supported(
            tree_sitter_jsdoc::LANGUAGE.into(),
            JSDOC_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_regex::LANGUAGE.into(),
            REGEX_HIGHLIGHTS_QUERY,
        );
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
        assert_capture_names_are_supported(
            tree_sitter_md::LANGUAGE.into(),
            MARKDOWN_HIGHLIGHTS_QUERY,
        );
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
        assert_capture_names_are_supported(
            tree_sitter_gomod::LANGUAGE.into(),
            GOMOD_HIGHLIGHTS_QUERY,
        );
        assert_capture_names_are_supported(
            tree_sitter_gowork::LANGUAGE.into(),
            GOWORK_HIGHLIGHTS_QUERY,
        );
    }

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

    #[test]
    fn rust_treesitter_captures_self_as_variable_special() {
        let text = "impl Widget { fn render(&self, item: Item) { self.draw(item); } }";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::VariableSpecial, "self"),
            "Rust `self` should produce VariableSpecial token, got: {tokens:?}"
        );
    }

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

    #[test]
    fn rust_treesitter_captures_keyword_function_type_and_string_families() {
        let text = r#"fn foo(bar: u32) { let x = "hi"; }"#;
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "fn"),
            "Rust should highlight `fn` as a keyword, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "foo"),
            "Rust function declarations should capture the function name, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "let"),
            "Rust should highlight `let` as a keyword, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::TypeBuiltin, "u32"),
            "Rust primitive types should keep their dedicated type token, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::String, "\"hi\""),
            "Rust string literals should produce String tokens, got: {tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_captures_impl_family_as_preproc() {
        let impl_text = "impl Widget where T: Trait {}";
        let impl_tokens =
            syntax_tokens_for_line(impl_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(impl_text, &impl_tokens, SyntaxTokenKind::Preproc, "impl"),
            "Rust `impl` should route through Preproc for the violet family, got: {impl_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(impl_text, &impl_tokens, SyntaxTokenKind::Preproc, "where"),
            "Rust `where` should route through Preproc, got: {impl_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(impl_text, &impl_tokens, SyntaxTokenKind::Type, "Widget"),
            "Rust impl targets should keep their type token, got: {impl_tokens:?}"
        );

        let trait_text = "trait Painter where Self: Sized {}";
        let trait_tokens =
            syntax_tokens_for_line(trait_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(trait_text, &trait_tokens, SyntaxTokenKind::Preproc, "trait"),
            "Rust `trait` should route through Preproc, got: {trait_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(trait_text, &trait_tokens, SyntaxTokenKind::Preproc, "where"),
            "Rust `where` should stay violet in trait declarations, got: {trait_tokens:?}"
        );

        let dyn_text = "let painter: dyn Painter = todo!();";
        let dyn_tokens =
            syntax_tokens_for_line(dyn_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(dyn_text, &dyn_tokens, SyntaxTokenKind::Preproc, "dyn"),
            "Rust `dyn` should route through Preproc, got: {dyn_tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_captures_use_roots_and_tails() {
        let type_text = "use foo::Bar;";
        let type_tokens =
            syntax_tokens_for_line(type_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Preproc, "foo"),
            "Non-`crate` import roots should route through Preproc, got: {type_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Type, "Bar"),
            "Imported uppercase tails should keep their type token, got: {type_tokens:?}"
        );

        let function_text = "use foo::bar;";
        let function_tokens = syntax_tokens_for_line(
            function_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                function_text,
                &function_tokens,
                SyntaxTokenKind::Preproc,
                "foo",
            ),
            "Non-`crate` import roots should stay violet, got: {function_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                function_text,
                &function_tokens,
                SyntaxTokenKind::Function,
                "bar",
            ),
            "Imported lowercase tails should route through Function, got: {function_tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_keeps_use_middle_modules_neutral() {
        let type_text = "use foo::bar::Baz;";
        let type_tokens =
            syntax_tokens_for_line(type_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Preproc, "foo"),
            "The top import root should stay violet, got: {type_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Type, "Baz"),
            "The imported type should stay green, got: {type_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Preproc, "bar"),
            "Middle modules should not inherit the root violet accent, got: {type_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(type_text, &type_tokens, SyntaxTokenKind::Function, "bar"),
            "Middle modules should not be recolored as imported tails, got: {type_tokens:?}"
        );

        let crate_type_text = "use crate::foo::Bar;";
        let crate_type_tokens = syntax_tokens_for_line(
            crate_type_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                crate_type_text,
                &crate_type_tokens,
                SyntaxTokenKind::Keyword,
                "crate",
            ),
            "Rust should keep `crate` on the keyword/orange family, got: {crate_type_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                crate_type_text,
                &crate_type_tokens,
                SyntaxTokenKind::Type,
                "Bar",
            ),
            "Imported types under `crate` should stay green, got: {crate_type_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(
                crate_type_text,
                &crate_type_tokens,
                SyntaxTokenKind::Preproc,
                "foo",
            ),
            "The segment after `crate::` should stay neutral, got: {crate_type_tokens:?}"
        );

        let crate_function_text = "use crate::foo::bar;";
        let crate_function_tokens = syntax_tokens_for_line(
            crate_function_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                crate_function_text,
                &crate_function_tokens,
                SyntaxTokenKind::Function,
                "bar",
            ),
            "The final lowercase import tail should stay blue under `crate`, got: {crate_function_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(
                crate_function_text,
                &crate_function_tokens,
                SyntaxTokenKind::Preproc,
                "foo",
            ),
            "The segment after `crate::` should remain neutral, got: {crate_function_tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_captures_root_modules_before_functions_and_types() {
        let call_text = "let handler = foo::bar::baz();";
        let call_tokens =
            syntax_tokens_for_line(call_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Preproc, "foo"),
            "Rust code paths should color the bare root module as Preproc, got: {call_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Preproc, "bar"),
            "Inner code-path modules should stay neutral instead of inheriting the root violet, got: {call_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Function, "bar"),
            "Inner code-path modules should not be recolored as callable tails, got: {call_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(call_text, &call_tokens, SyntaxTokenKind::Function, "baz"),
            "Rust function paths should keep the callable name as Function, got: {call_tokens:?}"
        );

        let associated_text = "let factory = foo::bar::Baz::new();";
        let associated_tokens = syntax_tokens_for_line(
            associated_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                associated_text,
                &associated_tokens,
                SyntaxTokenKind::Preproc,
                "foo",
            ),
            "Associated paths should keep the bare root module violet, got: {associated_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(
                associated_text,
                &associated_tokens,
                SyntaxTokenKind::Preproc,
                "bar",
            ),
            "Inner modules before associated functions should stay neutral, got: {associated_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                associated_text,
                &associated_tokens,
                SyntaxTokenKind::Type,
                "Baz",
            ),
            "Associated function paths should keep the type token, got: {associated_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                associated_text,
                &associated_tokens,
                SyntaxTokenKind::Function,
                "new",
            ),
            "Associated function paths should keep the callable name as Function, got: {associated_tokens:?}"
        );

        let crate_text = "let value: crate::foo::Bar = todo!();";
        let crate_tokens =
            syntax_tokens_for_line(crate_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Keyword, "crate"),
            "Rust should keep `crate` on the keyword/orange family, got: {crate_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Preproc, "foo"),
            "The first named segment after `crate::` should stay neutral in code paths, got: {crate_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Type, "Bar"),
            "Rust type tails under `crate` should stay green, got: {crate_tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_captures_constants_in_scoped_paths() {
        let constant_text = "let mode = NotForContentType::SSE;";
        let constant_tokens = syntax_tokens_for_line(
            constant_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                constant_text,
                &constant_tokens,
                SyntaxTokenKind::Type,
                "NotForContentType",
            ),
            "Rust should keep the type side of associated constants green, got: {constant_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                constant_text,
                &constant_tokens,
                SyntaxTokenKind::Constant,
                "SSE",
            ),
            "Rust ALL_CAPS associated constants should route through Constant, got: {constant_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(
                constant_text,
                &constant_tokens,
                SyntaxTokenKind::Type,
                "SSE",
            ),
            "Rust ALL_CAPS associated constants should no longer be typed green, got: {constant_tokens:?}"
        );

        let scoped_text = "let root = foo::BAR;";
        let scoped_tokens =
            syntax_tokens_for_line(scoped_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(scoped_text, &scoped_tokens, SyntaxTokenKind::Preproc, "foo"),
            "Bare module roots should stay violet before constant tails, got: {scoped_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                scoped_text,
                &scoped_tokens,
                SyntaxTokenKind::Constant,
                "BAR",
            ),
            "Scoped ALL_CAPS references should route through Constant, got: {scoped_tokens:?}"
        );

        let crate_scoped_text = "let root = crate::foo::BAR;";
        let crate_scoped_tokens = syntax_tokens_for_line(
            crate_scoped_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                crate_scoped_text,
                &crate_scoped_tokens,
                SyntaxTokenKind::Keyword,
                "crate",
            ),
            "Rust should keep `crate` orange before constant tails, got: {crate_scoped_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(
                crate_scoped_text,
                &crate_scoped_tokens,
                SyntaxTokenKind::Preproc,
                "foo",
            ),
            "The first named segment after `crate::` should stay neutral before constants, got: {crate_scoped_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                crate_scoped_text,
                &crate_scoped_tokens,
                SyntaxTokenKind::Constant,
                "BAR",
            ),
            "ALL_CAPS constant tails under `crate` should stay pink/Constant, got: {crate_scoped_tokens:?}"
        );

        let standalone_text = "let standalone = SSE;";
        let standalone_tokens = syntax_tokens_for_line(
            standalone_text,
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                standalone_text,
                &standalone_tokens,
                SyntaxTokenKind::Constant,
                "SSE",
            ),
            "Standalone ALL_CAPS Rust names should route through Constant, got: {standalone_tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_captures_grouped_use_import_semantics() {
        let text = "use foo::{bar, baz::Qux};";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "foo"),
            "Grouped imports should accent the non-`crate` root, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "bar"),
            "Grouped imports should keep lowercase imported tails blue, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Type, "Qux"),
            "Grouped imports should keep uppercase imported tails green, got: {tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "baz"),
            "Grouped middle modules should not inherit the root violet accent, got: {tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "baz"),
            "Grouped middle modules should stay neutral when importing a type, got: {tokens:?}"
        );

        let crate_text = "use crate::{foo::bar, baz::Qux};";
        let crate_tokens =
            syntax_tokens_for_line(crate_text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Keyword, "crate",),
            "Grouped imports should keep `crate` on the keyword/orange family, got: {crate_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Function, "bar",),
            "Grouped imports under `crate` should keep lowercase tails blue, got: {crate_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Type, "Qux"),
            "Grouped imports under `crate` should keep uppercase tails green, got: {crate_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Preproc, "foo",),
            "Paths under `crate::{{...}}` should not add a violet root accent, got: {crate_tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(crate_text, &crate_tokens, SyntaxTokenKind::Function, "baz",),
            "Middle grouped modules should stay neutral before imported types, got: {crate_tokens:?}"
        );
    }

    #[test]
    fn rust_treesitter_keeps_use_aliases_neutral() {
        let text = "use foo::bar as baz;";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "foo"),
            "Aliased imports should keep the non-`crate` root violet, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "bar"),
            "Aliased imports should keep the source tail blue, got: {tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Preproc, "baz"),
            "Import aliases should stay neutral instead of inheriting the root accent, got: {tokens:?}"
        );
        assert!(
            !has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Function, "baz"),
            "Import aliases should stay neutral instead of inheriting the source tail color, got: {tokens:?}"
        );
    }

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

    #[test]
    fn javascript_tagged_template_injects_html() {
        let text = "const markup = html`<div class=\"note\">ok</div>`;";
        let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
            "tagged HTML template should inject HTML tags in JavaScript: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
            "tagged HTML template should inject HTML attributes in JavaScript: {tokens:?}"
        );
    }

    #[test]
    fn javascript_styled_member_template_injects_css() {
        let text = "const Button = styled.div`color: red;`;";
        let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
            "styled member templates should inject CSS properties in JavaScript: {tokens:?}"
        );
    }

    #[test]
    fn javascript_styled_call_template_injects_css() {
        let text = "const Button = styled(Link)`color: red;`;";
        let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
            "styled call templates should inject CSS properties in JavaScript: {tokens:?}"
        );
    }

    #[test]
    fn javascript_comment_prefixed_string_injects_html() {
        let text = r#"const markup = /* html */ "<div class='note'>ok</div>";"#;
        let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
            "comment-prefixed HTML string should inject HTML tags: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
            "comment-prefixed HTML string should inject HTML attributes: {tokens:?}"
        );
    }

    #[test]
    fn javascript_comment_prefixed_string_injects_css() {
        let text = r#"const styles = /* css */ "color: red;";"#;
        let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
            "comment-prefixed CSS strings should inject CSS properties in JavaScript: {tokens:?}"
        );
    }

    #[test]
    fn javascript_comment_prefixed_template_literal_injects_css() {
        let text = "const styles = /* css */ `color: red;`;";
        let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("JavaScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
            "comment-prefixed CSS template literals should inject CSS properties: {tokens:?}"
        );
    }

    #[test]
    fn typescript_tagged_template_injects_yaml() {
        let text = "const workflow = yaml`enabled: true`;";
        let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("TypeScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "enabled"),
            "tagged YAML template should inject YAML properties: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Boolean, "true"),
            "tagged YAML template should inject YAML booleans: {tokens:?}"
        );
    }

    #[test]
    fn typescript_tagged_template_injects_html() {
        let text = "const markup = html`<div class=\"note\">ok</div>`;";
        let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("TypeScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
            "tagged HTML template should inject HTML tags in TypeScript: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
            "tagged HTML template should inject HTML attributes in TypeScript: {tokens:?}"
        );
    }

    #[test]
    fn typescript_tagged_template_injects_sql() {
        let text = "const query = sql`select name from users`;";
        let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("TypeScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "select"),
            "tagged SQL template should inject SQL keywords in TypeScript: {tokens:?}"
        );
    }

    #[test]
    fn typescript_comment_prefixed_string_injects_css() {
        let text = r#"const styles = /* css */ "color: red;";"#;
        let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("TypeScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
            "comment-prefixed CSS strings should inject CSS properties in TypeScript: {tokens:?}"
        );
    }

    #[test]
    fn typescript_component_styles_array_template_injects_css() {
        let text = "Component({ styles: [`div { color: red; }`] });";
        let document = prepare_test_document(DiffSyntaxLanguage::TypeScript, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("TypeScript document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "color"),
            "TypeScript Component styles templates should inject CSS properties: {tokens:?}"
        );
    }

    #[test]
    fn tsx_tagged_template_injects_html() {
        let text = "const markup = html`<div class=\"note\">ok</div>`;";
        let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("TSX document should have prepared tokens");
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Tag, "div"),
            "tagged HTML template should inject HTML tags in TSX: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Attribute, "class"),
            "tagged HTML template should inject HTML attributes in TSX: {tokens:?}"
        );
    }

    #[test]
    fn go_treesitter_captures_function_method_property_and_number() {
        let declaration = "func Hello(a B) C { return C{} }";
        let declaration_tokens =
            syntax_tokens_for_line(declaration, DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                declaration,
                &declaration_tokens,
                SyntaxTokenKind::Function,
                "Hello",
            ),
            "Go should capture function declarations: {declaration_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(declaration, &declaration_tokens, SyntaxTokenKind::Type, "B"),
            "Go should capture parameter types: {declaration_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(declaration, &declaration_tokens, SyntaxTokenKind::Type, "C"),
            "Go should capture return or composite literal types: {declaration_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                declaration,
                &declaration_tokens,
                SyntaxTokenKind::Keyword,
                "return",
            ),
            "Go should capture keywords: {declaration_tokens:?}"
        );

        let method_call = "value.Do(42)";
        let method_call_tokens =
            syntax_tokens_for_line(method_call, DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                method_call,
                &method_call_tokens,
                SyntaxTokenKind::FunctionMethod,
                "Do",
            ),
            "Go should capture method calls: {method_call_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                method_call,
                &method_call_tokens,
                SyntaxTokenKind::Number,
                "42",
            ),
            "Go should capture numeric literals: {method_call_tokens:?}"
        );

        let field_access = "value.Field";
        let field_access_tokens =
            syntax_tokens_for_line(field_access, DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                field_access,
                &field_access_tokens,
                SyntaxTokenKind::Property,
                "Field",
            ),
            "Go should capture field accesses: {field_access_tokens:?}"
        );
    }

    #[test]
    fn go_comment_prefixed_strings_inject_supported_languages() {
        let cases = [
            (
                r#"var payload = /* json */ `{"count": 42}`;"#,
                SyntaxTokenKind::Number,
                "42",
            ),
            (
                r#"var config = /* yaml */ `enabled: true`;"#,
                SyntaxTokenKind::Boolean,
                "true",
            ),
            (
                r#"var markup = /* html */ `<div class="note">ok</div>`;"#,
                SyntaxTokenKind::Tag,
                "div",
            ),
            (
                r#"var markup = /* xml */ `<root attr="value"/>`;"#,
                SyntaxTokenKind::Tag,
                "root",
            ),
            (
                r#"var script = /* js */ `const value = 42;`;"#,
                SyntaxTokenKind::Number,
                "42",
            ),
            (
                r#"var query = /* sql */ `select name from users`;"#,
                SyntaxTokenKind::Keyword,
                "select",
            ),
        ];

        for (text, expected_kind, expected_text) in cases {
            let document = prepare_test_document(DiffSyntaxLanguage::Go, text);
            let tokens = syntax_tokens_for_prepared_document_line(document, 0)
                .expect("Go document should have prepared tokens");
            assert!(
                has_token_kind_and_text(text, &tokens, expected_kind, expected_text),
                "Go comment-prefixed injection should produce {expected_kind:?} token {expected_text:?}: {tokens:?}"
            );
        }
    }

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

    #[test]
    fn yaml_github_actions_inline_script_injects_javascript() {
        let text = [
            "jobs:",
            "  test:",
            "    steps:",
            "      - uses: actions/github-script@v7",
            "        with:",
            "          script: const value = 42",
        ]
        .join("\n");
        let inline_line = text.lines().nth(5).unwrap_or_default();
        let document = prepare_test_document(DiffSyntaxLanguage::Yaml, &text);
        let tokens = syntax_tokens_for_prepared_document_line(document, 5)
            .expect("YAML github-script inline line should have prepared tokens");
        assert!(
            has_token_kind_and_text(inline_line, &tokens, SyntaxTokenKind::Keyword, "const")
                || tokens
                    .iter()
                    .any(|t| t.kind == SyntaxTokenKind::KeywordControl),
            "github-script YAML inline scalars should inject JavaScript keywords: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number),
            "github-script YAML inline scalars should inject JavaScript numbers: {tokens:?}"
        );
    }

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
                SyntaxTokenKind::FunctionMethod,
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
            (
                DiffSyntaxLanguage::Groovy,
                "class Example { def run() { return 42 } }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Clojure,
                "(defn run [] 42)",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Elixir,
                "defmodule Example do end",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Erlang,
                "run(X) -> X + 1.",
                SyntaxTokenKind::Function,
            ),
            (
                DiffSyntaxLanguage::Haskell,
                "run :: Int -> Int",
                SyntaxTokenKind::Type,
            ),
            (
                DiffSyntaxLanguage::Julia,
                "function run(x) x + 1 end",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::OCaml,
                "let run x = x + 1",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::OCamlInterface,
                "val run : int -> int",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Solidity,
                "contract Example { uint256 value; }",
                SyntaxTokenKind::Keyword,
            ),
            (
                DiffSyntaxLanguage::Assembly,
                "  mov eax, 1",
                SyntaxTokenKind::Number,
            ),
            (
                DiffSyntaxLanguage::Svelte,
                "<button class=\"go\">go</button>",
                SyntaxTokenKind::Tag,
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
                SyntaxTokenKind::VariableBuiltin,
            ),
            (
                DiffSyntaxLanguage::GitCommit,
                "feat: widen syntax support",
                SyntaxTokenKind::MarkupHeading,
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
    fn javascript_treesitter_captures_regex_literal() {
        let text = "const re = /foo+/gi;";
        let tokens =
            syntax_tokens_for_line(text, DiffSyntaxLanguage::JavaScript, DiffSyntaxMode::Auto);
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::StringRegex),
            "JavaScript regex literal should produce StringRegex token, got: {tokens:?}"
        );
    }

    #[test]
    fn javascript_treesitter_captures_constructor_and_constant_builtin() {
        let constructor_tokens = syntax_tokens_for_line(
            "class Example { constructor() {} }",
            DiffSyntaxLanguage::JavaScript,
            DiffSyntaxMode::Auto,
        );
        assert!(
            constructor_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Constructor),
            "JavaScript constructor should produce Constructor token, got: {constructor_tokens:?}"
        );

        let builtin_tokens = syntax_tokens_for_line(
            "const value = undefined;",
            DiffSyntaxLanguage::JavaScript,
            DiffSyntaxMode::Auto,
        );
        assert!(
            builtin_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::ConstantBuiltin),
            "JavaScript builtins should produce ConstantBuiltin token, got: {builtin_tokens:?}"
        );
    }

    #[test]
    fn go_treesitter_captures_namespace_package_identifier() {
        let tokens =
            syntax_tokens_for_line("package main", DiffSyntaxLanguage::Go, DiffSyntaxMode::Auto);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Namespace),
            "Go package identifier should produce Namespace token, got: {tokens:?}"
        );
    }

    #[test]
    fn lua_and_c_treesitter_capture_preprocessor_and_label() {
        let preproc = syntax_tokens_for_line(
            "#!/usr/bin/env lua",
            DiffSyntaxLanguage::Lua,
            DiffSyntaxMode::Auto,
        );
        assert!(
            preproc.iter().any(|t| t.kind == SyntaxTokenKind::Preproc),
            "Lua hash bang should produce Preproc token, got: {preproc:?}"
        );

        let label = syntax_tokens_for_line(
            "start: return 0;",
            DiffSyntaxLanguage::C,
            DiffSyntaxMode::Auto,
        );
        assert!(
            label.iter().any(|t| t.kind == SyntaxTokenKind::Label),
            "C label should produce Label token, got: {label:?}"
        );
    }

    #[test]
    fn c_treesitter_uses_vendored_zed_query() {
        let preproc = syntax_tokens_for_line(
            "#define VALUE 42",
            DiffSyntaxLanguage::C,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                "#define VALUE 42",
                &preproc,
                SyntaxTokenKind::Preproc,
                "#define"
            ),
            "C preprocessor directives should produce Preproc tokens, got: {preproc:?}"
        );

        let text = "struct Example { int field; };";
        let tokens = syntax_tokens_for_line(text, DiffSyntaxLanguage::C, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Keyword, "struct"),
            "C storage/type keywords should be captured, got: {tokens:?}"
        );
        assert!(
            has_token_kind_and_text(text, &tokens, SyntaxTokenKind::Property, "field"),
            "C field identifiers should produce Property tokens, got: {tokens:?}"
        );
    }

    #[test]
    fn cpp_treesitter_uses_vendored_zed_query() {
        let concept_text = "template <typename T> concept Addable = requires(T a, T b) { a + b; };";
        let concept_tokens =
            syntax_tokens_for_line(concept_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                concept_text,
                &concept_tokens,
                SyntaxTokenKind::TypeInterface,
                "Addable"
            ),
            "C++ concepts should produce TypeInterface tokens, got: {concept_tokens:?}"
        );
        assert!(
            has_token_kind_and_text(
                concept_text,
                &concept_tokens,
                SyntaxTokenKind::Keyword,
                "requires"
            ),
            "C++ requires should produce Keyword tokens, got: {concept_tokens:?}"
        );

        let module_text = "export module math.core; import std;";
        let module_tokens =
            syntax_tokens_for_line(module_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                module_text,
                &module_tokens,
                SyntaxTokenKind::Keyword,
                "module"
            ),
            "C++ module declarations should produce Keyword tokens, got: {module_tokens:?}"
        );
        assert!(
            module_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Namespace),
            "C++ module names should produce Namespace tokens, got: {module_tokens:?}"
        );

        let static_assert_text = "static_assert(sizeof(int) > 0);";
        let static_assert_tokens = syntax_tokens_for_line(
            static_assert_text,
            DiffSyntaxLanguage::Cpp,
            DiffSyntaxMode::Auto,
        );
        assert!(
            has_token_kind_and_text(
                static_assert_text,
                &static_assert_tokens,
                SyntaxTokenKind::Function,
                "static_assert"
            ),
            "C++ static_assert should produce Function tokens, got: {static_assert_tokens:?}"
        );

        let operator_text = "auto cmp = lhs <=> rhs;";
        let operator_tokens =
            syntax_tokens_for_line(operator_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                operator_text,
                &operator_tokens,
                SyntaxTokenKind::Operator,
                "<=>"
            ),
            "C++ spaceship operators should produce Operator tokens, got: {operator_tokens:?}"
        );

        let preproc_text = "#include <vector>";
        let preproc_tokens =
            syntax_tokens_for_line(preproc_text, DiffSyntaxLanguage::Cpp, DiffSyntaxMode::Auto);
        assert!(
            has_token_kind_and_text(
                preproc_text,
                &preproc_tokens,
                SyntaxTokenKind::Preproc,
                "#include"
            ),
            "C++ preprocessor directives should produce Preproc tokens, got: {preproc_tokens:?}"
        );
    }

    #[test]
    fn injected_web_helper_languages_capture_basic_tokens() {
        let regex_text = "(foo|bar)+";
        let regex_tokens =
            syntax_tokens_for_line(regex_text, DiffSyntaxLanguage::Regex, DiffSyntaxMode::Auto);
        assert!(
            regex_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Operator),
            "Regex syntax should capture operators, got: {regex_tokens:?}"
        );

        let jsdoc_text = "@param {string} name";
        let jsdoc_tokens =
            syntax_tokens_for_line(jsdoc_text, DiffSyntaxLanguage::Jsdoc, DiffSyntaxMode::Auto);
        assert!(
            jsdoc_tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Keyword),
            "JSDoc syntax should capture tags as keywords, got: {jsdoc_tokens:?}"
        );
    }

    #[test]
    fn gitcommit_treesitter_captures_diff_change_kinds() {
        let text = [
            "Subject",
            "",
            "# Changes to be committed:",
            "# new file: src/new.rs",
            "# deleted: src/old.rs",
            "# modified: src/lib.rs",
        ]
        .join("\n");
        let document = prepare_test_document(DiffSyntaxLanguage::GitCommit, &text);

        let plus = syntax_tokens_for_prepared_document_line(document, 3)
            .expect("gitcommit added line should have prepared tokens");
        assert!(
            plus.iter().any(|t| t.kind == SyntaxTokenKind::DiffPlus),
            "gitcommit additions should produce DiffPlus tokens, got: {plus:?}"
        );

        let minus = syntax_tokens_for_prepared_document_line(document, 4)
            .expect("gitcommit removed line should have prepared tokens");
        assert!(
            minus.iter().any(|t| t.kind == SyntaxTokenKind::DiffMinus),
            "gitcommit removals should produce DiffMinus tokens, got: {minus:?}"
        );

        let delta = syntax_tokens_for_prepared_document_line(document, 5)
            .expect("gitcommit modified file line should have prepared tokens");
        assert!(
            delta.iter().any(|t| t.kind == SyntaxTokenKind::DiffDelta),
            "gitcommit modified files should produce DiffDelta tokens, got: {delta:?}"
        );
    }

    #[test]
    fn prepared_documents_capture_markup_specific_tokens() {
        let gitcommit =
            prepare_test_document(DiffSyntaxLanguage::GitCommit, "Subject\n\ncloses #123");

        let heading = syntax_tokens_for_prepared_document_line(gitcommit, 0)
            .expect("gitcommit subject line should have prepared tokens");
        assert!(
            heading
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::MarkupHeading),
            "gitcommit subject should produce MarkupHeading token, got: {heading:?}"
        );

        let link = syntax_tokens_for_prepared_document_line(gitcommit, 2)
            .expect("gitcommit body line should have prepared tokens");
        assert!(
            link.iter().any(|t| t.kind == SyntaxTokenKind::MarkupLink),
            "gitcommit issue reference should produce MarkupLink token, got: {link:?}"
        );

        let xml = prepare_test_document(DiffSyntaxLanguage::Xml, "<root><![CDATA[code]]></root>");
        let literal = syntax_tokens_for_prepared_document_line(xml, 0)
            .expect("XML CDATA line should have prepared tokens");
        assert!(
            literal
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::TextLiteral),
            "XML CDATA should produce TextLiteral token, got: {literal:?}"
        );
    }

    #[test]
    fn markdown_inline_treesitter_captures_text_literal_and_markup_link() {
        let text = "[link](https://example.com) `code`";
        let tokens = syntax_tokens_for_line(
            text,
            DiffSyntaxLanguage::MarkdownInline,
            DiffSyntaxMode::Auto,
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::MarkupLink),
            "Markdown inline link destination should produce MarkupLink token, got: {tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::TextLiteral),
            "Markdown inline code span should produce TextLiteral token, got: {tokens:?}"
        );
    }

    #[test]
    fn markdown_prepared_document_captures_heading_marker_as_punctuation_special() {
        let document = prepare_test_document(DiffSyntaxLanguage::Markdown, "# Heading");
        let tokens = syntax_tokens_for_prepared_document_line(document, 0)
            .expect("markdown heading line should have prepared tokens");
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::PunctuationSpecial),
            "Markdown heading marker should remain PunctuationSpecial, got: {tokens:?}"
        );
    }

    #[test]
    fn ruby_and_swift_treesitter_capture_regex_aliases() {
        let cases = [
            (DiffSyntaxLanguage::Ruby, "value = /foo+/"),
            (DiffSyntaxLanguage::Swift, "let pattern = /foo+/"),
        ];

        for (language, text) in cases {
            let tokens = syntax_tokens_for_line(text, language, DiffSyntaxMode::Auto);
            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::StringRegex),
                "{language:?} regex literal should produce StringRegex token, got: {tokens:?}"
            );
        }
    }

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
        ])
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

    // ---- Batch: Groovy, Clojure, Elixir, Erlang, Haskell, Julia, OCaml, ------
    // ---- Solidity, Assembly, Svelte ------------------------------------------

    /// Every extension and fence alias the batch claims, plus the collisions the
    /// mapping had to avoid. Path resolution is the only thing standing between a
    /// wired-up grammar and a file that still renders as plain text, and nothing
    /// else in the suite exercises these arms.
    #[test]
    fn batch_language_paths_and_fences_resolve() {
        let cases: &[(&str, DiffSyntaxLanguage)] = &[
            ("src/Demo.groovy", DiffSyntaxLanguage::Groovy),
            ("build.gradle", DiffSyntaxLanguage::Groovy),
            ("Jenkinsfile", DiffSyntaxLanguage::Groovy),
            ("src/demo/core.clj", DiffSyntaxLanguage::Clojure),
            ("src/demo/core.cljs", DiffSyntaxLanguage::Clojure),
            ("deps.edn", DiffSyntaxLanguage::Clojure),
            ("lib/demo/worker.ex", DiffSyntaxLanguage::Elixir),
            ("test/demo_test.exs", DiffSyntaxLanguage::Elixir),
            ("src/demo.erl", DiffSyntaxLanguage::Erlang),
            ("include/demo.hrl", DiffSyntaxLanguage::Erlang),
            ("rebar.config", DiffSyntaxLanguage::Erlang),
            ("src/Demo/Worker.hs", DiffSyntaxLanguage::Haskell),
            ("src/demo.jl", DiffSyntaxLanguage::Julia),
            ("lib/demo.ml", DiffSyntaxLanguage::OCaml),
            ("lib/demo.mli", DiffSyntaxLanguage::OCamlInterface),
            ("contracts/Demo.sol", DiffSyntaxLanguage::Solidity),
            ("src/boot.asm", DiffSyntaxLanguage::Assembly),
            ("src/memcpy.s", DiffSyntaxLanguage::Assembly),
            // `.S` is preprocessed assembly; the lowercasing in
            // `diff_syntax_language_for_path` is what makes it land here.
            ("src/entry.S", DiffSyntaxLanguage::Assembly),
            ("src/App.svelte", DiffSyntaxLanguage::Svelte),
        ];
        for (path, expected) in cases {
            assert_eq!(
                diff_syntax_language_for_path(path),
                Some(*expected),
                "{path} should resolve to {expected:?}"
            );
        }

        for (fence, expected) in [
            ("groovy", DiffSyntaxLanguage::Groovy),
            ("clojure", DiffSyntaxLanguage::Clojure),
            ("elixir", DiffSyntaxLanguage::Elixir),
            ("erlang", DiffSyntaxLanguage::Erlang),
            ("haskell", DiffSyntaxLanguage::Haskell),
            ("julia", DiffSyntaxLanguage::Julia),
            ("ocaml", DiffSyntaxLanguage::OCaml),
            ("solidity", DiffSyntaxLanguage::Solidity),
            ("asm", DiffSyntaxLanguage::Assembly),
            ("svelte", DiffSyntaxLanguage::Svelte),
        ] {
            assert_eq!(
                diff_syntax_language_for_code_fence_info(fence),
                Some(expected),
                "```{fence} should resolve to {expected:?}"
            );
        }
    }

    /// The three collisions the batch had to route around. Each one is a silent
    /// regression if the identifier table is ever reordered: the file still
    /// highlights, just as the wrong language.
    #[test]
    fn batch_language_extensions_do_not_steal_existing_ones() {
        // `.gradle.kts` is Kotlin. The extension pass sees `kts` and never reaches
        // the `gradle` arm.
        assert_eq!(
            diff_syntax_language_for_path("build.gradle.kts"),
            Some(DiffSyntaxLanguage::Kotlin),
        );

        // `.m` stays Objective-C: the new `.ml` arm is one character away from it,
        // and an extension table is edited by hand.
        assert_eq!(
            diff_syntax_language_for_path("src/Demo.m"),
            Some(DiffSyntaxLanguage::ObjectiveC),
        );

        // `.ml` is OCaml, and must not be confused with the `.m` above.
        assert_eq!(
            diff_syntax_language_for_path("lib/demo.ml"),
            Some(DiffSyntaxLanguage::OCaml),
        );
    }

    // ---- Elixir ---------------------------------------------------------------

    const ELIXIR_FIXTURE: &[&str] = &[
        /*  0 */ "defmodule Demo.Worker do",
        /*  1 */ "  @moduledoc \"Runs jobs.\"",
        /*  2 */ "",
        /*  3 */ "  def run(%{id: id} = job) when is_integer(id) do",
        /*  4 */ "    :ok",
        /*  5 */ "  end",
        /*  6 */ "end",
    ];

    #[test]
    fn prepared_elixir_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Elixir, &ELIXIR_FIXTURE.join("\n"));

        for (line_ix, fragment, expected) in [
            (0usize, "defmodule", SyntaxTokenKind::Keyword),
            (0, "Demo.Worker", SyntaxTokenKind::Namespace),
            (3, "when", SyntaxTokenKind::Keyword),
            (5, "end", SyntaxTokenKind::Keyword),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, ELIXIR_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }

        // Atoms are the shape that separates Elixir from a curly-brace language.
        let atom = token_kinds_for_line_fragment(doc, 4, ELIXIR_FIXTURE[4], ":ok");
        assert!(
            atom.contains(&SyntaxTokenKind::StringSpecial),
            "`:ok` is an atom, not an identifier: {atom:?}"
        );
    }

    // ---- Erlang ---------------------------------------------------------------

    const ERLANG_FIXTURE: &[&str] = &[
        /*  0 */ "-module(demo).",
        /*  1 */ "-export([run/1]).",
        /*  2 */ "",
        /*  3 */ "%% Adds one.",
        /*  4 */ "run(X) when is_integer(X) ->",
        /*  5 */ "    Y = X + 1,",
        /*  6 */ "    {ok, Y}.",
    ];

    #[test]
    fn prepared_erlang_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Erlang, &ERLANG_FIXTURE.join("\n"));

        let directive = token_kinds_for_line_fragment(doc, 0, ERLANG_FIXTURE[0], "module");
        assert!(
            directive.contains(&SyntaxTokenKind::Keyword),
            "`-module` is a directive: {directive:?}"
        );

        // `%` is Erlang's line comment and nothing else in the tree uses it.
        let comment = token_kinds_for_line_fragment(doc, 3, ERLANG_FIXTURE[3], "Adds one");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`%%` starts an Erlang comment: {comment:?}"
        );

        let guard = token_kinds_for_line_fragment(doc, 4, ERLANG_FIXTURE[4], "when");
        assert!(
            guard.contains(&SyntaxTokenKind::Keyword),
            "`when` introduces a guard: {guard:?}"
        );

        let number = token_kinds_for_line_fragment(doc, 5, ERLANG_FIXTURE[5], "1");
        assert!(
            number.contains(&SyntaxTokenKind::Number),
            "`1` is a number: {number:?}"
        );
    }

    // ---- Haskell --------------------------------------------------------------

    const HASKELL_FIXTURE: &[&str] = &[
        /*  0 */ "module Demo.Worker (run) where",
        /*  1 */ "",
        /*  2 */ "import Data.List (foldl')",
        /*  3 */ "",
        /*  4 */ "-- | Adds one.",
        /*  5 */ "run :: Int -> Int",
        /*  6 */ "run x = foldl' (+) 0 [x, 1]",
    ];

    #[test]
    fn prepared_haskell_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Haskell, &HASKELL_FIXTURE.join("\n"));

        for (line_ix, fragment, expected) in [
            (0usize, "module", SyntaxTokenKind::Keyword),
            (0, "where", SyntaxTokenKind::Keyword),
            (2, "import", SyntaxTokenKind::Keyword),
            (5, "Int", SyntaxTokenKind::Type),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, HASKELL_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }

        // Haddock's `-- |` is a documentation comment, not a plain one.
        let haddock = token_kinds_for_line_fragment(doc, 4, HASKELL_FIXTURE[4], "Adds one");
        assert!(
            haddock.contains(&SyntaxTokenKind::CommentDoc),
            "`-- |` opens a Haddock comment: {haddock:?}"
        );
    }

    // ---- Julia ----------------------------------------------------------------

    const JULIA_FIXTURE: &[&str] = &[
        /*  0 */ "module Demo",
        /*  1 */ "",
        /*  2 */ "# Adds one.",
        /*  3 */ "function run(x::Int)::Int",
        /*  4 */ "    y = x + 1",
        /*  5 */ "    return y",
        /*  6 */ "end",
        /*  7 */ "",
        /*  8 */ "end",
    ];

    #[test]
    fn prepared_julia_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Julia, &JULIA_FIXTURE.join("\n"));

        let comment = token_kinds_for_line_fragment(doc, 2, JULIA_FIXTURE[2], "Adds one");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`#` is Julia's line comment: {comment:?}"
        );

        for (line_ix, fragment, expected) in [
            (3usize, "function", SyntaxTokenKind::Keyword),
            (3, "Int", SyntaxTokenKind::TypeBuiltin),
            (5, "return", SyntaxTokenKind::Keyword),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, JULIA_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }
    }

    // ---- OCaml ----------------------------------------------------------------

    const OCAML_FIXTURE: &[&str] = &[
        /*  0 */ "(* Adds one. *)",
        /*  1 */ "let run (x : int) : int =",
        /*  2 */ "  let y = x + 1 in",
        /*  3 */ "  y",
    ];

    const OCAML_INTERFACE_FIXTURE: &[&str] = &[
        /*  0 */ "(* Adds one. *)",
        /*  1 */ "val run : int -> int",
        /*  2 */ "",
        /*  3 */ "type t = { id : int }",
    ];

    /// Both halves of the `.ml`/`.mli` pair, because they are separate grammars
    /// sharing one query file. A change that compiles against the implementation
    /// grammar can still fail against the interface one -- that is exactly why
    /// `(shebang)` had to come out of the vendored copy.
    #[test]
    fn prepared_ocaml_documents_highlight_both_halves_of_the_pair() {
        let ml = prepare_test_document(DiffSyntaxLanguage::OCaml, &OCAML_FIXTURE.join("\n"));

        let comment = token_kinds_for_line_fragment(ml, 0, OCAML_FIXTURE[0], "Adds one");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`(* *)` is OCaml's only comment form: {comment:?}"
        );
        for (line_ix, fragment, expected) in [
            (1usize, "let", SyntaxTokenKind::Keyword),
            (1, "int", SyntaxTokenKind::TypeBuiltin),
            (2, "in", SyntaxTokenKind::Keyword),
        ] {
            let kinds =
                token_kinds_for_line_fragment(ml, line_ix, OCAML_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?} in a .ml: {kinds:?}"
            );
        }

        let mli = prepare_test_document(
            DiffSyntaxLanguage::OCamlInterface,
            &OCAML_INTERFACE_FIXTURE.join("\n"),
        );
        for (line_ix, fragment, expected) in [
            (1usize, "val", SyntaxTokenKind::Keyword),
            (3, "type", SyntaxTokenKind::Keyword),
            (3, "id", SyntaxTokenKind::Property),
            (3, "int", SyntaxTokenKind::TypeBuiltin),
        ] {
            let kinds = token_kinds_for_line_fragment(
                mli,
                line_ix,
                OCAML_INTERFACE_FIXTURE[line_ix],
                fragment,
            );
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?} in a .mli: {kinds:?}"
            );
        }
    }

    /// The reason queries/ocaml_highlights.scm exists rather than a reference to
    /// `tree_sitter_ocaml::HIGHLIGHTS_QUERY`: upstream names `(shebang)`, which the
    /// interface grammar has no rule for, and one unknown node type fails the whole
    /// query rather than the pattern that names it.
    #[test]
    fn ocaml_query_serves_both_grammars_and_upstream_does_not() {
        for language in [
            tree_sitter_ocaml::LANGUAGE_OCAML,
            tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE,
        ] {
            tree_sitter::Query::new(&language.into(), OCAML_HIGHLIGHTS_QUERY)
                .expect("the vendored query should compile against both OCaml grammars");
        }

        assert!(
            tree_sitter::Query::new(
                &tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
                tree_sitter_ocaml::HIGHLIGHTS_QUERY,
            )
            .is_err(),
            "upstream's query now compiles against the interface grammar -- drop the \
             vendored copy and use `tree_sitter_ocaml::HIGHLIGHTS_QUERY` for both."
        );
    }

    // ---- Groovy ---------------------------------------------------------------

    const GROOVY_FIXTURE: &[&str] = &[
        /*  0 */ "// Build config.",
        /*  1 */ "plugins {",
        /*  2 */ "    id 'java'",
        /*  3 */ "}",
        /*  4 */ "",
        /*  5 */ "class Demo {",
        /*  6 */ "    static int run(int x) {",
        /*  7 */ "        return x + 1",
        /*  8 */ "    }",
        /*  9 */ "}",
    ];

    #[test]
    fn prepared_groovy_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Groovy, &GROOVY_FIXTURE.join("\n"));

        let comment = token_kinds_for_line_fragment(doc, 0, GROOVY_FIXTURE[0], "Build config");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`//` is a Groovy line comment: {comment:?}"
        );

        // Single quotes are a plain string in Groovy, unlike the Haskell/OCaml/
        // Clojure arms added alongside it.
        let string = token_kinds_for_line_fragment(doc, 2, GROOVY_FIXTURE[2], "'java'");
        assert!(
            string.contains(&SyntaxTokenKind::String),
            "`'java'` is a string: {string:?}"
        );

        for (line_ix, fragment, expected) in [
            (5usize, "class", SyntaxTokenKind::Keyword),
            (6, "static", SyntaxTokenKind::Keyword),
            (6, "int", SyntaxTokenKind::TypeBuiltin),
            (7, "return", SyntaxTokenKind::KeywordControl),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, GROOVY_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }
    }

    // ---- Clojure --------------------------------------------------------------

    const CLOJURE_FIXTURE: &[&str] = &[
        /*  0 */ "(ns demo.worker)",
        /*  1 */ "",
        /*  2 */ ";; Adds one.",
        /*  3 */ "(defn run [x]",
        /*  4 */ "  (let [y (+ x 1)]",
        /*  5 */ "    {:id y :name \"demo\"}))",
    ];

    #[test]
    fn prepared_clojure_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Clojure, &CLOJURE_FIXTURE.join("\n"));

        let comment = token_kinds_for_line_fragment(doc, 2, CLOJURE_FIXTURE[2], "Adds one");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`;;` is Clojure's line comment: {comment:?}"
        );

        // The head-position rules in queries/clojure_highlights.scm. Upstream's
        // six-pattern query has none of this: without them a Clojure file is
        // literals and nothing else.
        for (line_ix, fragment, expected) in [
            (0usize, "ns", SyntaxTokenKind::Keyword),
            (3, "defn", SyntaxTokenKind::Keyword),
            (3, "run", SyntaxTokenKind::Function),
            (4, "let", SyntaxTokenKind::Keyword),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, CLOJURE_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }

        let keyword_literal = token_kinds_for_line_fragment(doc, 5, CLOJURE_FIXTURE[5], ":id");
        assert!(
            keyword_literal.contains(&SyntaxTokenKind::Constant),
            "`:id` is a keyword literal: {keyword_literal:?}"
        );
        let string = token_kinds_for_line_fragment(doc, 5, CLOJURE_FIXTURE[5], "\"demo\"");
        assert!(
            string.contains(&SyntaxTokenKind::String),
            "`\"demo\"` is a string: {string:?}"
        );
    }

    /// The quoting literals span the whole quoted form, so capturing the *node*
    /// paints `'(alpha beta)` end to end. Upstream captures the one-character
    /// marker instead. Capturing the node is cheap to reintroduce and invisible
    /// without a test.
    #[test]
    fn clojure_quoted_form_paints_only_its_marker() {
        let line = "(def syms '(alpha beta))";
        let doc = prepare_test_document(DiffSyntaxLanguage::Clojure, line);

        let marker = token_kinds_for_line_fragment(doc, 0, line, "'");
        assert!(
            marker.contains(&SyntaxTokenKind::Operator),
            "the quote marker should be an operator: {marker:?}"
        );

        let quoted = token_kinds_for_line_fragment(doc, 0, line, "alpha");
        assert!(
            !quoted.contains(&SyntaxTokenKind::Operator)
                && !quoted.contains(&SyntaxTokenKind::PunctuationSpecial),
            "the quoted form itself should not take the marker's colour: {quoted:?}"
        );
    }

    /// queries/clojure_highlights.scm opens with a verbatim copy of the upstream
    /// query and adds to it. A grammar bump that changes upstream leaves the copy
    /// stale and silently diverging, which is the one failure mode a compile check
    /// cannot see.
    #[test]
    fn clojure_highlights_query_embeds_the_upstream_base_verbatim() {
        let upstream = query_rule_lines(tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY);
        let vendored = query_rule_lines(CLOJURE_HIGHLIGHTS_QUERY);
        assert!(
            !upstream.is_empty(),
            "the upstream Clojure query should not be comment-only"
        );
        assert!(
            vendored
                .windows(upstream.len())
                .any(|window| window == upstream.as_slice()),
            "clojure_highlights.scm must contain tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY \
             as a contiguous, in-order block. Mirror the upstream change into the \
             `--- upstream ---` section.\nexpected block:\n{upstream:#?}\nvendored:\n{vendored:#?}"
        );
    }

    // ---- Solidity -------------------------------------------------------------

    const SOLIDITY_FIXTURE: &[&str] = &[
        /*  0 */ "// SPDX-License-Identifier: MIT",
        /*  1 */ "pragma solidity ^0.8.0;",
        /*  2 */ "",
        /*  3 */ "contract Demo {",
        /*  4 */ "    uint256 public total;",
        /*  5 */ "",
        /*  6 */ "    function add(uint256 x) public returns (uint256) {",
        /*  7 */ "        total += x;",
        /*  8 */ "        return total;",
        /*  9 */ "    }",
        /* 10 */ "}",
    ];

    #[test]
    fn prepared_solidity_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Solidity, &SOLIDITY_FIXTURE.join("\n"));

        let comment = token_kinds_for_line_fragment(doc, 0, SOLIDITY_FIXTURE[0], "SPDX");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "the SPDX header is a comment: {comment:?}"
        );

        for (line_ix, fragment, expected) in [
            (1usize, "pragma", SyntaxTokenKind::Keyword),
            (3, "contract", SyntaxTokenKind::Keyword),
            (4, "uint256", SyntaxTokenKind::Type),
            (6, "function", SyntaxTokenKind::Keyword),
            (6, "returns", SyntaxTokenKind::Keyword),
            (8, "return", SyntaxTokenKind::Keyword),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, SOLIDITY_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }
    }

    /// If a grammar bump ships a query that compiles as-is, the vendored copy and
    /// this test can both go.
    #[test]
    fn solidity_upstream_query_still_needs_the_vendored_fix() {
        assert!(
            tree_sitter::Query::new(
                &tree_sitter_solidity::LANGUAGE.into(),
                tree_sitter_solidity::HIGHLIGHT_QUERY,
            )
            .is_err(),
            "tree_sitter_solidity::HIGHLIGHT_QUERY now compiles -- drop \
             queries/solidity_highlights.scm and use the crate constant."
        );
    }

    // ---- Assembly -------------------------------------------------------------

    const ASSEMBLY_FIXTURE: &[&str] = &[
        /*  0 */ "section .text",
        /*  1 */ "global run",
        /*  2 */ "run:",
        /*  3 */ "    mov eax, 1 ; seed",
        /*  4 */ "    add eax, edi",
        /*  5 */ "    ret",
    ];

    #[test]
    fn prepared_assembly_document_highlights_core_syntax() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Assembly, &ASSEMBLY_FIXTURE.join("\n"));

        for (line_ix, fragment, expected) in [
            (2usize, "run", SyntaxTokenKind::Label),
            (3, "mov", SyntaxTokenKind::Function),
            (3, "eax", SyntaxTokenKind::VariableBuiltin),
            (3, "1", SyntaxTokenKind::Number),
            (5, "ret", SyntaxTokenKind::Function),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, ASSEMBLY_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }

        // Trailing comments are the only comment position this grammar accepts;
        // see `assembly_standalone_comment_lines_fall_back_to_the_heuristic`.
        let comment = token_kinds_for_line_fragment(doc, 3, ASSEMBLY_FIXTURE[3], "; seed");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "a trailing `;` comment should be greyed out: {comment:?}"
        );
    }

    /// A documented limitation, not a bug in the wiring: tree-sitter-asm only
    /// admits a comment after an instruction, so a comment on its own line -- which
    /// is most comments in real assembly -- puts the tree into error recovery.
    ///
    /// Recovery is survivable (the instructions around it still highlight) and the
    /// heuristic path, which this repo also runs for short lines and oversized
    /// diffs, gets it right. The test pins both halves so a grammar bump that fixes
    /// the parse shows up here rather than going unnoticed.
    #[test]
    fn assembly_standalone_comment_lines_fall_back_to_the_heuristic() {
        let source = "; set the return value\n    mov eax, 1\n    ret\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_asm::LANGUAGE.into())
            .expect("asm grammar should load");
        let tree = parser.parse(source, None).expect("asm should parse");
        assert!(
            tree.root_node().has_error(),
            "tree-sitter-asm now parses a standalone comment line -- update this test and \
             the note on the Assembly arm of heuristic_comment_config"
        );

        // The instructions after the bad line still highlight.
        let doc = prepare_test_document(DiffSyntaxLanguage::Assembly, source);
        let kinds = token_kinds_for_line_fragment(doc, 1, "    mov eax, 1", "mov");
        assert!(
            kinds.contains(&SyntaxTokenKind::Function),
            "error recovery should leave the following instructions intact: {kinds:?}"
        );

        // And the heuristic, which does not care what the grammar thinks, greys the
        // whole comment line.
        let heuristic = heuristic_tokens("; set the return value", DiffSyntaxLanguage::Assembly);
        assert!(
            heuristic
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "`;` is the heuristic's assembly line comment: {heuristic:?}"
        );
    }

    /// The identifier scanner starts on `_` or a letter, never `.`, so a GAS
    /// directive reaches `is_keyword` as its bare tail, so a table spelling its
    /// entries `".text"` and `".globl"` looks complete and matches nothing.
    /// Nothing else in the suite would notice.
    #[test]
    fn assembly_gas_dot_directives_reach_the_keyword_table() {
        for (line, expected) in [
            ("    .text", "text"),
            ("    .data", "data"),
            ("    .bss", "bss"),
            ("    .globl main", "globl"),
            ("    .global main", "global"),
            ("    .align 4", "align"),
            ("    .long 1", "long"),
            ("    .quad 2", "quad"),
            ("    .short 3", "short"),
            ("    .byte 1", "byte"),
            ("    .word 1", "word"),
            ("    .ascii \"hi\"", "ascii"),
            ("    .section .rodata", "section"),
            // The NASM/MASM spellings, which carry no dot to begin with.
            ("section .text", "section"),
            ("global main", "global"),
            ("extern printf", "extern"),
        ] {
            let found = heuristic_keywords(line, DiffSyntaxLanguage::Assembly);
            assert!(
                found.contains(&expected),
                "{line:?} should yield the `{expected}` directive keyword: {found:?}"
            );
        }
    }

    /// `#` is an ARM immediate (`mov r0, #1`), not a comment. Giving the Assembly
    /// arm `hash_comment: true` would grey out the operand of every such
    /// instruction, which is why it shares nothing with the Python/Ruby arm.
    #[test]
    fn assembly_hash_immediate_is_not_a_comment() {
        let line = "    mov r0, #1";
        let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Assembly);
        assert!(
            !tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "an ARM immediate was greyed out as a comment: {tokens:?}"
        );
    }

    // ---- Svelte ---------------------------------------------------------------

    const SVELTE_FIXTURE: &[&str] = &[
        /*  0 */ "<script lang=\"ts\">",
        /*  1 */ "  let count: number = 0;",
        /*  2 */ "</script>",
        /*  3 */ "",
        /*  4 */ "{#if count > 0}",
        /*  5 */ "  <button class=\"btn\">{count}</button>",
        /*  6 */ "{:else}",
        /*  7 */ "  <p>none</p>",
        /*  8 */ "{/if}",
        /*  9 */ "",
        /* 10 */ "<style>",
        /* 11 */ "  .btn { color: red; }",
        /* 12 */ "</style>",
    ];

    #[test]
    fn prepared_svelte_document_highlights_markup_and_block_tags() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &SVELTE_FIXTURE.join("\n"));

        // The html base embedded in queries/svelte_highlights.scm. Without it the
        // upstream query colours the block markers and leaves the markup plain.
        for (line_ix, fragment, expected) in [
            (0usize, "script", SyntaxTokenKind::Tag),
            (0, "lang", SyntaxTokenKind::Attribute),
            (5, "button", SyntaxTokenKind::Tag),
            (5, "class", SyntaxTokenKind::Attribute),
            (5, "btn", SyntaxTokenKind::String),
        ] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, SVELTE_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&expected),
                "`{fragment}` should be {expected:?}: {kinds:?}"
            );
        }

        // The svelte half: `{#if}` / `{:else}` / `{/if}`.
        for (line_ix, fragment) in [(4usize, "if"), (6, "else"), (8, "if")] {
            let kinds =
                token_kinds_for_line_fragment(doc, line_ix, SVELTE_FIXTURE[line_ix], fragment);
            assert!(
                kinds.contains(&SyntaxTokenKind::Keyword),
                "the `{{{fragment}}}` block marker should be a keyword: {kinds:?}"
            );
        }
    }

    /// The script and style bodies are the bulk of a `.svelte` file and neither is
    /// reachable from the highlights query -- they arrive as injections or not at
    /// all. The `lang="ts"` veto is what keeps the default javascript rule from
    /// firing over the same `raw_text`; see the note in svelte_injections.scm.
    #[test]
    fn svelte_script_and_style_blocks_inject_their_languages() {
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &SVELTE_FIXTURE.join("\n"));

        let script = token_kinds_for_line_fragment(doc, 1, SVELTE_FIXTURE[1], "let");
        assert!(
            script.contains(&SyntaxTokenKind::Keyword),
            "`<script lang=\"ts\">` should inject TypeScript: {script:?}"
        );

        let style = token_kinds_for_line_fragment(doc, 11, SVELTE_FIXTURE[11], "color");
        assert!(
            style.contains(&SyntaxTokenKind::Property),
            "`<style>` should inject CSS: {style:?}"
        );
    }

    /// The `lang` veto in svelte_injections.scm, which is the whole reason the two
    /// default rules carry a `#not-match?`. Without it a `<script lang="ts">` body
    /// matches the default javascript rule *and* the typescript one over the same
    /// `raw_text`; live.rs keeps both layers and interleaves their captures at the
    /// same depth, so the block comes out coloured by whichever wrote last.
    #[test]
    fn svelte_script_with_lang_injects_exactly_one_language() {
        let text = "<script lang=\"ts\">\nconst x: number = 1;\n</script>\n";

        let lang: tree_sitter::Language = tree_sitter_svelte_ng::LANGUAGE.into();
        let query = tree_sitter::Query::new(&lang, SVELTE_INJECTIONS_QUERY)
            .expect("vendored Svelte injections.scm should compile");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&lang)
            .expect("Svelte grammar should load");
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
            "a `<script lang=\"ts\">` must match exactly one injection pattern, \
             matched {patterns:?}"
        );

        // ...and it must be the TypeScript one, so the annotation is typed.
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, text);
        let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
            .expect("script body should have prepared tokens");
        assert!(
            tokens
                .iter()
                .any(|t| t.kind == SyntaxTokenKind::Type || t.kind == SyntaxTokenKind::TypeBuiltin),
            "`lang=\"ts\"` should inject TypeScript, so `: number` should be typed: {tokens:?}"
        );
    }

    /// The trap vue_injections.scm documents: the default rule is vetoed by
    /// `#not-match? "\\slang\\s*="` whenever *any* `lang` is present, so
    /// enumerating the servable values with `#any-of?` means every unlisted one --
    /// `lang="js"`, `lang="css"`, and the unquoted `lang=ts` -- falls into a gap
    /// and the whole block renders with no highlighting at all.
    ///
    /// Forwarding the value as `@injection.language` is what closes it. Asserting
    /// `is_some()` would be vacuous here: the broken version returned
    /// `Some(vec![])`, not `None`.
    #[test]
    fn svelte_lang_values_outside_the_default_still_inject() {
        for (open_tag, body, expected) in [
            // `js` and `css` name grammars we have but are not the default value
            // for their element -- the exact pair the enumerated version dropped.
            (
                "<script lang=\"js\">",
                "const x = 1;",
                SyntaxTokenKind::Keyword,
            ),
            (
                "<style lang=\"css\">",
                "  .b { color: red; }",
                SyntaxTokenKind::Property,
            ),
            // Resolved through the alias table rather than by name.
            (
                "<script lang=\"mts\">",
                "const x = 1;",
                SyntaxTokenKind::Keyword,
            ),
            (
                "<style lang=\"pcss\">",
                "  .b { color: red; }",
                SyntaxTokenKind::Property,
            ),
            // The unquoted form the grammar permits.
            ("<script lang=ts>", "const x = 1;", SyntaxTokenKind::Keyword),
        ] {
            let close = if open_tag.starts_with("<script") {
                "</script>"
            } else {
                "</style>"
            };
            let text = format!("{open_tag}\n{body}\n{close}\n");
            let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &text);
            let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
                .expect("the block body should have prepared tokens");
            assert!(
                tokens.iter().any(|token| token.kind == expected),
                "`{open_tag}` should still inject: expected {expected:?}, got {tokens:?}"
            );
        }
    }

    /// The other half of the same trade-off: a `lang` no grammar here can serve
    /// injects nothing, and that must not disturb the host grammar. Same contract
    /// as `vue_unknown_lang_attribute_does_not_silently_disable_highlighting`.
    #[test]
    fn svelte_unservable_lang_leaves_the_markup_alone() {
        let text = "<script lang=\"coffee\">\nx = 1\n</script>\n";
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, text);
        let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
            .expect("the opening tag should have prepared tokens");
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Tag),
            "an unservable lang must not disturb the host grammar: {tokens:?}"
        );
    }

    /// The bug in `tree_sitter_svelte_ng::INJECTIONS_QUERY` that svelte_injections.scm
    /// exists to avoid: its bare `(raw_text)` catch-all matches the body of `<style>`
    /// too, so a stylesheet gets parsed as JavaScript.
    #[test]
    fn svelte_style_block_injects_css_not_javascript() {
        let text = "<style>\n  .btn { color: red; }\n</style>\n";
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, text);
        let kinds = token_kinds_for_line_fragment(doc, 1, "  .btn { color: red; }", "color");
        assert!(
            kinds.contains(&SyntaxTokenKind::Property),
            "`color` is a CSS property, which the JavaScript grammar would never \
             produce: {kinds:?}"
        );
    }

    /// The counterpart to `vue_static_inline_styles_do_not_flood_the_injection_cache`.
    /// A `.svelte` template injects per *expression*, not per file, so an ordinary
    /// list render emits one layer per row. Without the bare-identifier guard in
    /// svelte_injections.scm a 30-row list produced 30 cache entries against a cap
    /// of 32, evicting everything else on its own.
    #[test]
    fn svelte_bare_identifier_expressions_do_not_flood_the_injection_cache() {
        TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

        let mut lines = vec!["<ul>".to_string()];
        for ix in 0..30 {
            lines.push(format!("  <li>{{row{ix}}}</li>"));
        }
        lines.push("</ul>".to_string());
        let line_count = lines.len();

        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &lines.join("\n"));
        for line_ix in 0..line_count {
            let _ = syntax_tokens_for_prepared_document_line(doc, line_ix);
        }

        let cached = TS_INJECTION_CACHE.with(|cache| cache.borrow().len());
        assert_eq!(
            cached, 0,
            "a bare identifier gains nothing from a JavaScript parse, but {cached} cache \
             entries were created from {line_count} lines (cap is \
             {TS_INJECTION_CACHE_MAX_ENTRIES})"
        );

        TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
    }

    /// ...and the guard must not be so broad that real expressions stop injecting.
    #[test]
    fn svelte_non_trivial_expressions_still_inject() {
        let line = "  <p>{count > 0 ? \"many\" : \"none\"}</p>";
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, line);

        let kinds = token_kinds_for_line_fragment(doc, 0, line, "\"many\"");
        assert!(
            kinds.contains(&SyntaxTokenKind::String),
            "a real expression should still be parsed as JavaScript, so the string \
             literal inside it is a string: {kinds:?}"
        );
    }

    /// The same tripwire vue_highlights.scm carries, for the same reason: the
    /// Svelte grammar is html-shaped, the base rules have to be present in the
    /// file, and rule order decides which capture wins.
    #[test]
    fn svelte_highlights_query_embeds_the_html_base_verbatim() {
        let html_rules = query_rule_lines(HTML_HIGHLIGHTS_QUERY);
        let svelte_rules = query_rule_lines(SVELTE_HIGHLIGHTS_QUERY);
        assert!(
            !html_rules.is_empty(),
            "html_highlights.scm should not be comment-only"
        );
        assert!(
            svelte_rules
                .windows(html_rules.len())
                .any(|window| window == html_rules.as_slice()),
            "svelte_highlights.scm must contain queries/html_highlights.scm as a contiguous, \
             in-order block. Mirror the change into the `--- html base ---` section.\n\
             expected block:\n{html_rules:#?}\nsvelte rules:\n{svelte_rules:#?}"
        );
    }

    // ---- Heuristic fallback for the batch --------------------------------------

    /// Three of the new languages spell something other than a string with `'`:
    /// Haskell primes identifiers, OCaml opens type variables, Clojure quotes
    /// forms. Left as `HeuristicSingleQuote::String` each one runs a string from
    /// the tick to the end of the line -- the Nix bug, three more times.
    #[test]
    fn batch_apostrophes_do_not_open_a_string() {
        for (line, language) in [
            ("run xs = foldl' (+) 0 xs", DiffSyntaxLanguage::Haskell),
            ("let ids : 'a list = []", DiffSyntaxLanguage::OCaml),
            (
                "val map : ('a -> 'b) -> 'a list -> 'b list",
                DiffSyntaxLanguage::OCamlInterface,
            ),
            ("(def syms '(alpha beta))", DiffSyntaxLanguage::Clojure),
        ] {
            assert!(
                heuristic_string_spans(line, language).is_empty(),
                "an apostrophe opened a string in {language:?} line {line:?}: {:?}",
                heuristic_tokens(line, language)
            );
        }

        // Double quotes still work everywhere.
        assert_eq!(
            heuristic_string_spans("  name = \"demo\"", DiffSyntaxLanguage::Haskell),
            vec!["\"demo\""]
        );
    }

    /// Julia is the one language in the batch where `'` is both: `A'` is the
    /// adjoint operator and `'c'` is a character literal. ValuePositionOnly tells
    /// them apart by what precedes the tick.
    #[test]
    fn julia_adjoint_is_not_a_string_but_a_char_literal_is() {
        assert!(
            heuristic_string_spans("    b = A' * x", DiffSyntaxLanguage::Julia).is_empty(),
            "the adjoint operator opened a string: {:?}",
            heuristic_tokens("    b = A' * x", DiffSyntaxLanguage::Julia)
        );
        assert_eq!(
            heuristic_string_spans("    c = 'x'", DiffSyntaxLanguage::Julia),
            vec!["'x'"],
            "a character literal in value position is still a string"
        );
    }

    /// Every comment form the batch introduced. The heuristic runs in production
    /// for lines past MAX_TREESITTER_LINE_BYTES and in HeuristicOnly mode, and
    /// these arms are reached by nothing else.
    #[test]
    fn batch_heuristic_comment_forms_are_covered() {
        for (line, language) in [
            ("-- | Adds one.", DiffSyntaxLanguage::Haskell),
            ("{- block -}", DiffSyntaxLanguage::Haskell),
            ("%% Adds one.", DiffSyntaxLanguage::Erlang),
            (";; Adds one.", DiffSyntaxLanguage::Clojure),
            ("(* Adds one. *)", DiffSyntaxLanguage::OCaml),
            ("(* Adds one. *)", DiffSyntaxLanguage::OCamlInterface),
            ("# Adds one.", DiffSyntaxLanguage::Elixir),
            ("# Adds one.", DiffSyntaxLanguage::Julia),
            ("// Adds one.", DiffSyntaxLanguage::Groovy),
            ("/* Adds one. */", DiffSyntaxLanguage::Solidity),
            ("    ret ; done", DiffSyntaxLanguage::Assembly),
            ("<!-- Adds one. -->", DiffSyntaxLanguage::Svelte),
        ] {
            let tokens = heuristic_tokens(line, language);
            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "{language:?} should treat {line:?} as a comment: {tokens:?}"
            );
        }

        // Haskell's `--` must not swallow an operator section: `x -- y` is a
        // comment, but the heuristic has no way to know that `--` in
        // `f -->> g` is not one either. Pin the ordinary case only.
        let subtraction = heuristic_tokens("    y = x - 1", DiffSyntaxLanguage::Haskell);
        assert!(
            !subtraction
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "a single `-` is not a Haskell comment: {subtraction:?}"
        );
    }

    /// Per the Haskell report a run of dashes is a comment only when it is *not*
    /// followed by a symbol character; otherwise the whole run is an operator.
    /// `line_comment: Some("--")` cannot express that, so it greyed `a --> b` from
    /// the dashes to the end of the line -- the worst failure mode this path has,
    /// because it hides code rather than mis-colouring it.
    #[test]
    fn haskell_operator_sections_starting_with_dashes_are_not_comments() {
        for line in [
            "  step = a --> b",
            "  merged = xs --| ys",
            "  shifted = a --< b",
            "  chained = f --. g",
        ] {
            let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Haskell);
            assert!(
                !tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "an operator section was greyed out as a comment in {line:?}: {tokens:?}"
            );
        }

        // ...while every genuine comment still is one, including the `---` run that
        // the naive "any symbol after `--`" rule would have broken.
        for line in [
            "-- plain",
            "--- ruled off",
            "  x = 1 -- trailing",
            "-- | haddock",
        ] {
            let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Haskell);
            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "{line:?} is a Haskell comment: {tokens:?}"
            );
        }
    }

    /// Neighbouring entries in four of the keyword tables, each gap visible as two
    /// adjacent lines highlighting differently.
    #[test]
    fn batch_keyword_tables_cover_their_neighbours() {
        for (line, language, expected) in [
            // `let ... in` is one form; highlighting half of it looked like a bug.
            ("  let x = 1 in x + 1", DiffSyntaxLanguage::Haskell, "in"),
            // Erlang's word-spelled operators are reserved words.
            ("  Y = X div 2,", DiffSyntaxLanguage::Erlang, "div"),
            ("  Z = X band 255,", DiffSyntaxLanguage::Erlang, "band"),
            ("  ok = not Flag,", DiffSyntaxLanguage::Erlang, "not"),
            // Groovy had `boolean` but none of the other primitives.
            ("    int x = 1", DiffSyntaxLanguage::Groovy, "int"),
            ("    double d = 1.0", DiffSyntaxLanguage::Groovy, "double"),
            ("    char c = 'x'", DiffSyntaxLanguage::Groovy, "char"),
            // Solidity's two block forms.
            (
                "        assembly { let p := 1 }",
                DiffSyntaxLanguage::Solidity,
                "assembly",
            ),
            (
                "        unchecked { x += 1; }",
                DiffSyntaxLanguage::Solidity,
                "unchecked",
            ),
        ] {
            let found = heuristic_keywords(line, language);
            assert!(
                found.contains(&expected),
                "{language:?} should treat `{expected}` in {line:?} as a keyword: {found:?}"
            );
        }
    }

    /// The other half of the Solidity fix: sized types are *uniformly* absent now.
    /// Listing `uint256` alone meant `uint256 total;` highlighted and `uint8 flags;`
    /// two lines below it did not.
    #[test]
    fn solidity_sized_types_are_uniformly_absent_from_the_keyword_table() {
        for line in [
            "    uint8 a;",
            "    uint256 b;",
            "    int128 c;",
            "    bytes32 d;",
        ] {
            let found = heuristic_keywords(line, DiffSyntaxLanguage::Solidity);
            assert!(
                found.is_empty(),
                "sized types should all behave alike on the heuristic path, but {line:?} \
                 yielded {found:?}"
            );
        }

        // The base names still resolve, so the arm is not simply dead.
        assert!(heuristic_keywords("    uint x;", DiffSyntaxLanguage::Solidity).contains(&"uint"));
        assert!(
            heuristic_keywords("    address owner;", DiffSyntaxLanguage::Solidity)
                .contains(&"address")
        );
    }

    /// The eleven keyword tables the batch added to `is_keyword`, none of which any
    /// other test reaches: every other test in this section goes through
    /// `prepare_test_document`, i.e. tree-sitter.
    #[test]
    fn batch_heuristic_keyword_tables_are_covered() {
        for (line, language, expected) in [
            (
                "class Demo extends Base {",
                DiffSyntaxLanguage::Groovy,
                "class",
            ),
            ("(defn run [x] x)", DiffSyntaxLanguage::Clojure, "defn"),
            ("defmodule Demo do", DiffSyntaxLanguage::Elixir, "defmodule"),
            (
                "run(X) when is_integer(X) ->",
                DiffSyntaxLanguage::Erlang,
                "when",
            ),
            (
                "newtype Wrapper = Wrapper Int",
                DiffSyntaxLanguage::Haskell,
                "newtype",
            ),
            ("mutable struct Point", DiffSyntaxLanguage::Julia, "struct"),
            ("let rec loop n =", DiffSyntaxLanguage::OCaml, "rec"),
            (
                "val run : int -> int",
                DiffSyntaxLanguage::OCamlInterface,
                "val",
            ),
            (
                "contract Demo is Base {",
                DiffSyntaxLanguage::Solidity,
                "contract",
            ),
            ("section .text", DiffSyntaxLanguage::Assembly, "section"),
            ("{#each items as item}", DiffSyntaxLanguage::Svelte, "each"),
        ] {
            let found = heuristic_keywords(line, language);
            assert!(
                found.contains(&expected),
                "{language:?} should treat `{expected}` in {line:?} as a keyword: {found:?}"
            );
        }
    }

    // ---- Regression net for the batch ------------------------------------------

    /// Every language the batch added, with a snippet that exercises its comment
    /// form, its string form and one keyword.
    ///
    /// Shared by the invariant sweeps below so a new language is added in one place
    /// and picked up by all of them.
    fn batch_language_samples() -> Vec<(DiffSyntaxLanguage, &'static str)> {
        Vec::from([
            (
                DiffSyntaxLanguage::Groovy,
                "class D { def s = 'x' } // note",
            ),
            (DiffSyntaxLanguage::Clojure, "(defn f [x] \"s\") ;; note"),
            (DiffSyntaxLanguage::Elixir, "def f(x), do: \"s\" # note"),
            (DiffSyntaxLanguage::Erlang, "f(X) -> \"s\". % note"),
            (DiffSyntaxLanguage::Haskell, "f x = \"s\" -- note"),
            (DiffSyntaxLanguage::Julia, "f(x) = \"s\" # note"),
            (DiffSyntaxLanguage::OCaml, "let f x = \"s\" (* note *)"),
            (
                DiffSyntaxLanguage::OCamlInterface,
                "val f : int -> int (* note *)",
            ),
            (
                DiffSyntaxLanguage::Solidity,
                "function f() { s = \"x\"; } // note",
            ),
            (DiffSyntaxLanguage::Assembly, "    mov eax, 1 ; note"),
            (
                DiffSyntaxLanguage::Svelte,
                "<p class=\"c\">x</p> <!-- note -->",
            ),
        ])
    }

    /// The `potential_open_state_lead` fast-skip decides which bytes are even worth
    /// examining, and a language whose comment lead is missing from it has its
    /// comments run past entirely on the streamed path. Haskell's `-` had to be
    /// added there when `line_comment` became None for it.
    ///
    /// The long body is not padding. Below the checkpoint threshold the streamed
    /// entry point hands the visible region straight to the per-line tokenizer, so a
    /// short-line version of this test exercises the scanner not at all and passes
    /// with the fast-skip entry deleted.
    ///
    /// Each case puts the comment opener *before* the slice, so the token can only
    /// be right if the scanner resumed in the comment state.
    #[test]
    fn streamed_slices_resume_inside_batch_line_comments() {
        const CHECKPOINT_SPACING: usize = 32 * 1024;

        for (language, opener) in [
            (DiffSyntaxLanguage::Haskell, "-- "),
            (DiffSyntaxLanguage::Erlang, "% "),
            (DiffSyntaxLanguage::Clojure, "; "),
            (DiffSyntaxLanguage::Assembly, "    ret ; "),
            (DiffSyntaxLanguage::Groovy, "// "),
            (DiffSyntaxLanguage::Elixir, "# "),
        ] {
            reset_streamed_heuristic_line_cache();

            let body = "note ".repeat(CHECKPOINT_SPACING / 5 + 64);
            let text = format!("{opener}{body}");
            let slice_start = opener.len() + CHECKPOINT_SPACING;
            let slice_end = slice_start + 128;
            let raw_text =
                gitcomet_core::file_diff::FileDiffLineText::shared(Arc::from(text.clone()));
            let (slice_text, resolved) = raw_text
                .slice_text_resolved(slice_start..slice_end)
                .expect("ASCII slice should resolve");

            let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
                &raw_text,
                language,
                slice_start..slice_end,
                resolved,
            )
            .expect("streamed slice should be supported");
            assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "{language:?}: a slice {CHECKPOINT_SPACING} bytes into a `{opener}` comment \
                 must still be a comment, got {tokens:?}"
            );
        }
    }

    /// The two block-comment kinds the batch touched: Haskell's `{- -}` is a new
    /// `HeuristicBlockCommentKind`, and OCaml reuses the F# `(* *)` spec. Both are
    /// resumed from a checkpoint here, which is the only place the start/end byte
    /// tables are consulted rather than the per-line `starts_with`.
    #[test]
    fn streamed_slices_resume_inside_haskell_and_ocaml_block_comments() {
        const CHECKPOINT_SPACING: usize = 32 * 1024;

        for (language, open, close) in [
            (DiffSyntaxLanguage::Haskell, "{-", "-}"),
            (DiffSyntaxLanguage::OCaml, "(*", "*)"),
            (DiffSyntaxLanguage::OCamlInterface, "(*", "*)"),
        ] {
            reset_streamed_heuristic_line_cache();

            let body = "b".repeat(CHECKPOINT_SPACING + 192);
            let text = format!("{open}{body}{close} let x = 1");
            let slice_start = open.len() + CHECKPOINT_SPACING;
            let slice_end = slice_start + 96;
            let raw_text =
                gitcomet_core::file_diff::FileDiffLineText::shared(Arc::from(text.clone()));
            let (slice_text, resolved) = raw_text
                .slice_text_resolved(slice_start..slice_end)
                .expect("ASCII slice should resolve");

            let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
                &raw_text,
                language,
                slice_start..slice_end,
                resolved,
            )
            .expect("streamed slice should be supported");
            assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "{language:?}: a slice inside a `{open} {close}` block must still be a \
                 comment, got {tokens:?}"
            );

            // …and the block has to *end*. Asserting only the line above passes even
            // with `heuristic_block_comment_end_bytes` corrupted: an unterminated
            // comment swallows the rest of the line, so the slice above stays inside
            // it either way.
            let tail_start = text.find(close).expect("close should be present") + close.len();
            let (tail_text, tail_resolved) = raw_text
                .slice_text_resolved(tail_start..text.len())
                .expect("tail slice should resolve");
            let tail = syntax_tokens_for_streamed_line_slice_heuristic(
                &raw_text,
                language,
                tail_start..text.len(),
                tail_resolved,
            )
            .expect("streamed tail slice should be supported");
            assert_token_ranges_are_utf8_safe(tail_text.as_ref(), &tail);
            assert!(
                !tail
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "{language:?}: `{close}` must close the block, but the code after it is \
                 still a comment: {tail:?}"
            );
        }
    }

    /// Token ranges are used to slice the line for rendering, so an out-of-bounds or
    /// mid-codepoint range panics rather than mis-colouring. The batch added eleven
    /// languages to a hand-written scanner; these are the inputs that break scanners.
    #[test]
    fn batch_languages_emit_well_formed_tokens_on_hostile_input() {
        let hostile = [
            "",
            " ",
            "\t",
            // Unterminated everything.
            "\"unterminated",
            "'unterminated",
            "`unterminated",
            "/* unterminated",
            "{- unterminated",
            "(* unterminated",
            "<!-- unterminated",
            // Bare openers at end of line, where a lookahead can run past the end.
            "-",
            "--",
            "/",
            "//",
            "{",
            "(",
            "#",
            ";",
            "%",
            "\\",
            "\"",
            "'",
            // Multi-byte, including a comment opener immediately before one.
            "-- ✨ é 日本語",
            "x = \"日本語\" -- ✨",
            "'é'",
            "«»‹›",
            // Adjacent delimiters.
            "\"\"''``",
            "/*/*/*",
            "{-{-{-",
            "(*(*(*",
            "-->--|--<",
            "REM",
            "rem\tx",
        ];

        for (language, _) in batch_language_samples() {
            for line in hostile {
                let tokens = syntax_tokens_for_line(line, language, DiffSyntaxMode::HeuristicOnly);
                assert_token_ranges_are_utf8_safe(line, &tokens);

                // Ranges must also be ordered and non-overlapping: the renderer walks
                // them with a single forward cursor.
                let mut previous_end = 0usize;
                for token in tokens.iter() {
                    assert!(
                        token.range.start >= previous_end,
                        "{language:?} emitted overlapping or unsorted tokens for {line:?}: \
                         {tokens:?}"
                    );
                    previous_end = token.range.end;
                }
            }
        }
    }

    /// `heuristic_comment_range` now delegates to `line_comment_start_len`, which
    /// tests `is_ascii_whitespace()` where the old copy compared against a literal
    /// `"rem "`. Visual Basic is the only caller that notices, and it is not a
    /// language the batch touched -- exactly the kind of bystander a refactor
    /// breaks quietly.
    #[test]
    fn visual_basic_rem_comment_survives_the_shared_comment_decision() {
        for line in ["REM note", "rem note", "Rem note", "REM\tnote"] {
            let tokens = heuristic_tokens(line, DiffSyntaxLanguage::VisualBasic);
            assert!(
                tokens
                    .iter()
                    .any(|token| token.kind == SyntaxTokenKind::Comment),
                "{line:?} is a Visual Basic REM comment: {tokens:?}"
            );
        }

        // `REM` still needs a delimiter: `REMARK` is an identifier.
        let tokens = heuristic_tokens("REMARK = 1", DiffSyntaxLanguage::VisualBasic);
        assert!(
            !tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "`REMARK` is not a REM comment: {tokens:?}"
        );
    }

    /// A completeness sweep rather than a behaviour check: every language that
    /// claims a grammar must actually produce tokens for a line of itself. A
    /// mis-wired grammar, a query that compiles but matches nothing, or an enum
    /// variant wired to the wrong `LANGUAGE` constant all show up here as silence.
    #[test]
    fn every_batch_language_produces_treesitter_tokens() {
        for (language, sample) in batch_language_samples() {
            assert!(
                tree_sitter_grammar(language).is_some(),
                "{language:?} should have a grammar"
            );

            let doc = prepare_test_document(language, sample);
            let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
                .unwrap_or_else(|| panic!("{language:?} should produce prepared tokens"));
            assert!(
                !tokens.is_empty(),
                "{language:?} produced no tokens for {sample:?} -- the grammar is wired but \
                 its query matches nothing"
            );
            assert_token_ranges_are_utf8_safe(sample, &tokens);
        }
    }

    // ---- Nix ------------------------------------------------------------------

    const NIX_FIXTURE: &[&str] = &[
        /*  0 */ "# Build a demo package.",
        /*  1 */ "{ pkgs, lib ? pkgs.lib, ... }:",
        /*  2 */ "let",
        /*  3 */ "  inherit (pkgs) stdenv;",
        /*  4 */ "  version = \"1.0\";",
        /*  5 */ "  readme = builtins.readFile ./README.md;",
        /*  6 */ "in",
        /*  7 */ "stdenv.mkDerivation rec {",
        /*  8 */ "  pname = \"demo\";",
        /*  9 */ "  meta.description = \"demo v${version}\";",
        /* 10 */ "  buildPhase = ''",
        /* 11 */ "    export OUT=$out",
        /* 12 */ "    if [ -d bin ]; then",
        /* 13 */ "      cp -r bin \"$out/bin\"",
        /* 14 */ "    fi",
        /* 15 */ "  '';",
        /* 16 */ "}",
    ];

    fn prepare_nix_document(lines: &[&str]) -> PreparedSyntaxDocument {
        prepare_test_document(DiffSyntaxLanguage::Nix, &lines.join("\n"))
    }

    #[test]
    fn nix_extension_is_supported() {
        for path in ["flake.nix", "pkgs/demo/default.nix", "nix/modules/web.nix"] {
            assert_eq!(
                diff_syntax_language_for_path(path),
                Some(DiffSyntaxLanguage::Nix),
                "{path} should resolve to the Nix grammar"
            );
        }
        assert_eq!(
            diff_syntax_language_for_code_fence_info("nix"),
            Some(DiffSyntaxLanguage::Nix),
        );
        // `flake.lock` is JSON and must keep resolving that way.
        assert_eq!(
            diff_syntax_language_for_path("flake.lock"),
            Some(DiffSyntaxLanguage::Json),
        );
    }

    #[test]
    fn prepared_nix_document_highlights_core_syntax() {
        let doc = prepare_nix_document(NIX_FIXTURE);

        let comment = token_kinds_for_line_fragment(doc, 0, NIX_FIXTURE[0], "demo package");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`# …` is a Nix line comment: {comment:?}"
        );

        for (line_ix, keyword) in [(2usize, "let"), (3, "inherit"), (6, "in"), (7, "rec")] {
            let kinds = token_kinds_for_line_fragment(doc, line_ix, NIX_FIXTURE[line_ix], keyword);
            assert!(
                kinds.contains(&SyntaxTokenKind::Keyword),
                "`{keyword}` should be a keyword: {kinds:?}"
            );
        }

        let formal = token_kinds_for_line_fragment(doc, 1, NIX_FIXTURE[1], "pkgs");
        assert!(
            formal.contains(&SyntaxTokenKind::VariableParameter),
            "`pkgs` is a formal in the function's argument set: {formal:?}"
        );

        let attr = token_kinds_for_line_fragment(doc, 8, NIX_FIXTURE[8], "pname");
        assert!(
            attr.contains(&SyntaxTokenKind::Property),
            "a binding attrpath should read as a property: {attr:?}"
        );

        let string = token_kinds_for_line_fragment(doc, 8, NIX_FIXTURE[8], "\"demo\"");
        assert!(
            string.contains(&SyntaxTokenKind::String),
            "`\"demo\"` should be a string: {string:?}"
        );

        let path = token_kinds_for_line_fragment(doc, 5, NIX_FIXTURE[5], "./README.md");
        assert!(
            path.contains(&SyntaxTokenKind::StringSpecial),
            "a bare Nix path is `@string.special.path`: {path:?}"
        );

        let interpolation = token_kinds_for_line_fragment(doc, 9, NIX_FIXTURE[9], "${");
        assert!(
            interpolation.contains(&SyntaxTokenKind::PunctuationSpecial),
            "`${{` opens an interpolation: {interpolation:?}"
        );
    }

    /// The one real guard on the reordering in nix_highlights.scm.
    ///
    /// Two patterns capturing the *same* node tie on start byte, so the tiebreak is
    /// pattern index — the later rule in the file wins. Upstream ends with a blanket
    /// `(identifier) @variable`, which ported verbatim buries every specific
    /// identifier rule. Confirmed to fail against upstream's ordering: `builtins`
    /// comes back as `[Variable]`. If this fails after a re-sync, the query was not
    /// re-sorted.
    #[test]
    fn nix_specific_captures_survive_the_generic_identifier_rule() {
        let doc = prepare_nix_document(NIX_FIXTURE);

        let builtins = token_kinds_for_line_fragment(doc, 5, NIX_FIXTURE[5], "builtins");
        assert!(
            builtins.contains(&SyntaxTokenKind::VariableBuiltin)
                && !builtins.contains(&SyntaxTokenKind::Variable),
            "`builtins` must keep its builtin colour rather than falling back to the \
             blanket `(identifier) @variable` rule: {builtins:?}"
        );

        let applied = token_kinds_for_line_fragment(doc, 7, NIX_FIXTURE[7], "mkDerivation");
        assert!(
            applied.contains(&SyntaxTokenKind::Function)
                && !applied.contains(&SyntaxTokenKind::Variable),
            "an identifier in function-application position must read as a function: \
             {applied:?}"
        );

        let inherited = token_kinds_for_line_fragment(doc, 3, NIX_FIXTURE[3], "stdenv");
        assert!(
            inherited.contains(&SyntaxTokenKind::Property),
            "`inherit (pkgs) stdenv` names a property: {inherited:?}"
        );
    }

    /// An escape inside a string keeps its own colour.
    ///
    /// Not an ordering guard, despite appearances: `normalize_non_overlapping_tokens`
    /// hands each slice to the last *containing* capture in emission order, and the
    /// cursor emits by node start byte, so a nested `(escape_sequence)` always beats
    /// the `(string_expression)` around it whichever order their rules appear in.
    /// Verified — this passes against upstream's ordering too. It is here to pin the
    /// behaviour, not the query layout.
    #[test]
    fn nix_escape_sequences_outrank_the_string_rule() {
        let lines = ["{ s = \"a\\nb\"; }"];
        let doc = prepare_nix_document(&lines);
        let escape = token_kinds_for_line_fragment(doc, 0, lines[0], "\\n");
        assert!(
            escape.contains(&SyntaxTokenKind::StringEscape),
            "`\\n` inside a string must outrank the enclosing `@string` capture: {escape:?}"
        );
    }

    /// The interior of `"demo v${version}"` is Nix code, not string text.
    ///
    /// Like the escape test above, this holds by node position rather than by rule
    /// order — the interpolated expression starts after the string does, so it wins
    /// its own bytes regardless.
    #[test]
    fn nix_interpolation_interior_is_not_flat_string() {
        let doc = prepare_nix_document(NIX_FIXTURE);
        let inner = token_kinds_for_line_fragment(doc, 9, NIX_FIXTURE[9], "version");
        assert!(
            !inner.is_empty() && !inner.contains(&SyntaxTokenKind::String),
            "the expression inside `${{…}}` should be highlighted as code, not as part \
             of the surrounding string: {inner:?}"
        );
    }

    /// `buildPhase = '' … ''` is shell script, and the combined Bash injection is
    /// what makes it read as one. Only the injected layer has a concept of `if`.
    #[test]
    fn nix_build_phase_is_highlighted_as_bash() {
        let doc = prepare_nix_document(NIX_FIXTURE);

        let conditional = token_kinds_for_line_fragment(doc, 12, NIX_FIXTURE[12], "if");
        assert!(
            conditional.contains(&SyntaxTokenKind::KeywordControl)
                || conditional.contains(&SyntaxTokenKind::Keyword),
            "`if` inside buildPhase should come from the injected Bash layer, not read \
             as string text: {conditional:?}"
        );

        // And the injection stays inside the indented string: the Nix binding on
        // the line above is still Nix.
        let binding = token_kinds_for_line_fragment(doc, 10, NIX_FIXTURE[10], "buildPhase");
        assert!(
            binding.contains(&SyntaxTokenKind::Property),
            "`buildPhase` is a Nix attrpath, not part of the shell script: {binding:?}"
        );
    }

    #[test]
    fn nix_injection_targets_resolve_to_working_grammars() {
        let lang: tree_sitter::Language = tree_sitter_nix::LANGUAGE.into();
        let query = tree_sitter::Query::new(&lang, NIX_INJECTIONS_QUERY)
            .expect("nix_injections.scm should compile");
        let mut checked = 0usize;
        for pattern_ix in 0..query.pattern_count() {
            for setting in query.property_settings(pattern_ix) {
                if setting.key.as_ref() != "injection.language" {
                    continue;
                }
                let Some(value) = setting.value.as_deref() else {
                    continue;
                };
                let target = injection_language_from_name(value).unwrap_or_else(|| {
                    panic!("nix_injections.scm names an unknown injection language {value:?}")
                });
                assert!(
                    tree_sitter_highlight_spec(target).is_some(),
                    "nix_injections.scm injects {value:?} but no grammar is wired for \
                     {target:?}, so the injection would silently no-op"
                );
                checked += 1;
            }
        }
        assert_eq!(
            checked, 4,
            "expected the four curated bash rules; upstream's comment-marked \
             arbitrary-language rule is deliberately not ported"
        );
    }

    #[test]
    fn nix_spec_warmup_reaches_bash_through_a_set_directive() {
        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Nix).expect("nix spec");
        let injection_query = spec.injection_query.as_ref().expect("nix injection query");
        let reaches_bash = (0..injection_query.pattern_count()).any(|pattern_ix| {
            injection_query
                .property_settings(pattern_ix)
                .iter()
                .filter(|setting| setting.key.as_ref() == "injection.language")
                .any(|setting| {
                    setting
                        .value
                        .as_deref()
                        .and_then(injection_language_from_name)
                        == Some(DiffSyntaxLanguage::Bash)
                })
        });
        assert!(
            reaches_bash,
            "warm_reachable_highlight_specs must be able to see the bash target, or the \
             Bash query compile lands on the draw path"
        );
    }

    #[test]
    fn nix_grammar_is_abi_compatible_with_workspace_tree_sitter() {
        let nix: tree_sitter::Language = tree_sitter_nix::LANGUAGE.into();
        let abi = nix.abi_version();
        assert!(
            (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
                .contains(&abi),
            "tree-sitter-nix ABI {abi} is outside the range this tree-sitter supports \
             ({}..={})",
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION,
        );
    }

    #[test]
    fn nix_grammar_parses_a_flake() {
        let source = concat!(
            "{\n",
            "  description = \"demo\";\n",
            "  inputs.nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";\n",
            "  outputs = { self, nixpkgs }: {\n",
            "    packages.x86_64-linux.default =\n",
            "      nixpkgs.legacyPackages.x86_64-linux.hello;\n",
            "  };\n",
            "}\n",
        );
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_nix::LANGUAGE.into())
            .expect("nix grammar should load into the workspace tree-sitter");
        let tree = parser.parse(source, None).expect("flake.nix should parse");
        assert!(
            !tree.root_node().has_error(),
            "the nix grammar produced an ERROR node for a well-formed flake: {}",
            tree.root_node().to_sexp(),
        );
    }

    // ---- Nunjucks / Jinja2 ----------------------------------------------------

    /// The `.njk` / `.j2` / `.jinja` fixture, shared by the tests below.
    const JINJA_TEMPLATE_FIXTURE: &[&str] = &[
        /* 0 */ "{# page heading #}",
        /* 1 */ "<ul class=\"list\">",
        /* 2 */ "  {% for item in items %}",
        /* 3 */ "    <li>{{ item.name | upper }}</li>",
        /* 4 */ "  {% endfor %}",
        /* 5 */ "</ul>",
    ];

    fn prepare_jinja_document(lines: &[&str]) -> PreparedSyntaxDocument {
        prepare_test_document(DiffSyntaxLanguage::Jinja, &lines.join("\n"))
    }

    #[test]
    fn jinja_extension_is_supported() {
        for path in [
            "templates/index.njk",
            "templates/base.html.j2",
            "templates/macros.jinja",
            "templates/macros.jinja2",
            "templates/page.twig",
            "templates/page.html.dj",
        ] {
            assert_eq!(
                diff_syntax_language_for_path(path),
                Some(DiffSyntaxLanguage::Jinja),
                "{path} should resolve to the Jinja grammar"
            );
        }
        // The same table backs markdown fence info.
        for fence in ["njk", "jinja", "jinja2", "twig", "nunjucks"] {
            assert_eq!(
                diff_syntax_language_for_code_fence_info(fence),
                Some(DiffSyntaxLanguage::Jinja),
                "```{fence} should resolve to the Jinja grammar"
            );
        }
    }

    /// A `.j2` says the file is templated, not that it is markup. Resolving a shell
    /// or config template to the HTML-injecting reading hands the HTML grammar
    /// `cat <<EOF` and `2>&1`, which open bogus elements.
    #[test]
    fn text_bodied_jinja_templates_do_not_get_html_injected() {
        for path in [
            "roles/web/templates/nginx.conf.j2",
            "charts/app/values.yaml.j2",
            "deploy/deploy.sh.j2",
            "docker-compose.yml.j2",
            "config/settings.ini.jinja",
            "db/schema.sql.j2",
        ] {
            assert_eq!(
                diff_syntax_language_for_path(path),
                Some(DiffSyntaxLanguage::JinjaText),
                "{path} has a non-markup body, so it must not inject HTML"
            );
        }

        let markup = tree_sitter_highlight_spec(DiffSyntaxLanguage::Jinja).expect("jinja spec");
        let text = tree_sitter_highlight_spec(DiffSyntaxLanguage::JinjaText).expect("text spec");
        assert!(
            markup.injection_query.is_some(),
            "the markup reading is the one that injects HTML"
        );
        assert!(
            text.injection_query.is_none(),
            "the text reading must have no injection query at all"
        );
        assert!(
            !text.has_combined_injections,
            "with no injection query there is no combined group to build"
        );
    }

    /// The shell-template shape that motivated the split, end to end.
    #[test]
    fn shell_bodied_jinja_template_does_not_colour_redirects_as_tags() {
        let lines = [
            /* 0 */ "#!/bin/sh",
            /* 1 */ "{% if debug %}",
            /* 2 */ "cat <<EOF > {{ target }}",
            /* 3 */ "  value=1",
            /* 4 */ "EOF",
            /* 5 */ "{% endif %}",
            /* 6 */ "run --flag 2>&1 < input",
        ];
        let doc = prepare_test_document(DiffSyntaxLanguage::JinjaText, &lines.join("\n"));

        for (line_ix, fragment) in [(2usize, "EOF"), (6, "input")] {
            let kinds = token_kinds_for_line_fragment(doc, line_ix, lines[line_ix], fragment);
            assert!(
                !kinds.contains(&SyntaxTokenKind::Tag),
                "`{fragment}` on line {line_ix} was coloured as an HTML tag: {kinds:?}"
            );
        }

        // The template tags themselves still highlight -- only the injection is gone.
        let endif = token_kinds_for_line_fragment(doc, 5, lines[5], "endif");
        assert!(
            endif.contains(&SyntaxTokenKind::KeywordControl),
            "template keywords must survive the split: {endif:?}"
        );
    }

    #[test]
    fn prepared_jinja_document_highlights_template_tags() {
        let doc = prepare_jinja_document(JINJA_TEMPLATE_FIXTURE);

        let comment = token_kinds_for_line_fragment(doc, 0, JINJA_TEMPLATE_FIXTURE[0], "heading");
        assert!(
            comment.contains(&SyntaxTokenKind::Comment),
            "`{{# … #}}` is a Jinja comment: {comment:?}"
        );

        let open = token_kinds_for_line_fragment(doc, 2, JINJA_TEMPLATE_FIXTURE[2], "{%");
        assert!(
            open.contains(&SyntaxTokenKind::PunctuationSpecial),
            "the `{{%` delimiter should be punctuation, not plain text: {open:?}"
        );

        for (line_ix, keyword) in [(2usize, "for"), (4, "endfor")] {
            let kinds = token_kinds_for_line_fragment(
                doc,
                line_ix,
                JINJA_TEMPLATE_FIXTURE[line_ix],
                keyword,
            );
            assert!(
                kinds.contains(&SyntaxTokenKind::KeywordControl),
                "`{keyword}` is control flow and should render semibold: {kinds:?}"
            );
        }

        let filter = token_kinds_for_line_fragment(doc, 3, JINJA_TEMPLATE_FIXTURE[3], "upper");
        assert!(
            filter.contains(&SyntaxTokenKind::Function),
            "a filter name after `|` should read as a function: {filter:?}"
        );

        let property = token_kinds_for_line_fragment(doc, 3, JINJA_TEMPLATE_FIXTURE[3], "name");
        assert!(
            property.contains(&SyntaxTokenKind::Property),
            "`item.name` should colour `name` as a property: {property:?}"
        );
    }

    /// The HTML half of a template comes from the combined injection, not the
    /// Jinja grammar -- which sees only opaque `text` nodes.
    #[test]
    fn prepared_jinja_document_highlights_html_via_the_combined_injection() {
        let doc = prepare_jinja_document(JINJA_TEMPLATE_FIXTURE);

        let tag = token_kinds_for_line_fragment(doc, 1, JINJA_TEMPLATE_FIXTURE[1], "ul");
        assert!(
            tag.contains(&SyntaxTokenKind::Tag),
            "`<ul>` should be tagged by the injected HTML layer: {tag:?}"
        );
        let attribute = token_kinds_for_line_fragment(doc, 1, JINJA_TEMPLATE_FIXTURE[1], "class");
        assert!(
            attribute.contains(&SyntaxTokenKind::Attribute),
            "`class=` should be an HTML attribute: {attribute:?}"
        );

        // The whole point of the combined injection: `<li>` sits inside the loop
        // body, in a different `text` node from `<ul>`, and still highlights.
        let inner = token_kinds_for_line_fragment(doc, 3, JINJA_TEMPLATE_FIXTURE[3], "li");
        assert!(
            inner.contains(&SyntaxTokenKind::Tag),
            "`<li>` is in a separate text run from `<ul>`; only a combined layer \
             sees them as one document: {inner:?}"
        );
    }

    /// The injected HTML must stay off the template tags, which the Jinja
    /// grammar owns. See `combined_injection_gaps`.
    #[test]
    fn jinja_html_injection_does_not_bleed_onto_template_tags() {
        let doc = prepare_jinja_document(JINJA_TEMPLATE_FIXTURE);
        let kinds = token_kinds_for_line_fragment(doc, 4, JINJA_TEMPLATE_FIXTURE[4], "endfor");
        assert!(
            !kinds.contains(&SyntaxTokenKind::Tag),
            "`{{% endfor %}}` sits in a gap between two HTML ranges; an HTML element \
             node spanning it must not colour it as a tag: {kinds:?}"
        );
    }

    #[test]
    fn jinja_injection_targets_resolve_to_working_grammars() {
        let lang: tree_sitter::Language = tree_sitter_jinja_dialects::LANGUAGE.into();
        let query = tree_sitter::Query::new(&lang, JINJA_INJECTIONS_QUERY)
            .expect("jinja_injections.scm should compile");
        let mut checked = 0usize;
        for pattern_ix in 0..query.pattern_count() {
            for setting in query.property_settings(pattern_ix) {
                if setting.key.as_ref() != "injection.language" {
                    continue;
                }
                let Some(value) = setting.value.as_deref() else {
                    continue;
                };
                let target = injection_language_from_name(value).unwrap_or_else(|| {
                    panic!("jinja_injections.scm names an unknown injection language {value:?}")
                });
                assert!(
                    tree_sitter_highlight_spec(target).is_some(),
                    "jinja_injections.scm injects {value:?} but no grammar is wired for \
                     {target:?}, so the injection would silently no-op"
                );
                checked += 1;
            }
        }
        assert!(
            checked > 0,
            "expected at least one `#set! injection.language`"
        );
    }

    /// Warm-up reads targets off the compiled query, and only sees `#set!`
    /// literals. If the HTML target ever moved into an `@injection.language`
    /// capture, the ~0.5ms HTML spec compile would move back onto the draw path.
    #[test]
    fn jinja_spec_warmup_reaches_html_through_a_set_directive() {
        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Jinja).expect("jinja spec");
        let injection_query = spec
            .injection_query
            .as_ref()
            .expect("jinja injection query");
        let reaches_html = (0..injection_query.pattern_count()).any(|pattern_ix| {
            injection_query
                .property_settings(pattern_ix)
                .iter()
                .filter(|setting| setting.key.as_ref() == "injection.language")
                .any(|setting| {
                    setting
                        .value
                        .as_deref()
                        .and_then(injection_language_from_name)
                        == Some(DiffSyntaxLanguage::Html)
                })
        });
        assert!(
            reaches_html,
            "warm_reachable_highlight_specs must be able to see the html target"
        );
    }

    #[test]
    fn jinja_injection_query_stays_under_the_match_limit_on_a_dense_template() {
        let mut lines = vec!["<ul>".to_string()];
        for ix in 0..120 {
            lines.push(format!(
                "  {{% if show{ix} %}}<li class=\"r{ix}\">{{{{ row{ix}.label | title }}}}</li>{{% endif %}}"
            ));
        }
        lines.push("</ul>".to_string());
        let text = lines.join("\n");

        let lang: tree_sitter::Language = tree_sitter_jinja_dialects::LANGUAGE.into();
        let query = tree_sitter::Query::new(&lang, JINJA_INJECTIONS_QUERY)
            .expect("jinja_injections.scm should compile");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&lang)
            .expect("jinja grammar should load");
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
            "the Jinja injection query overflowed the {TS_QUERY_MATCH_LIMIT}-match \
             in-progress pool on a {}-line template. A combined group that loses ranges \
             assembles a different HTML document, so the engine drops the whole group and \
             the template renders with no HTML highlighting at all",
            lines.len(),
        );
        assert!(matched > 0, "the dense template should produce matches");
    }

    /// The grammar is a young crates.io release binding through
    /// `tree-sitter-language`, so a tree-sitter bump could outrun it.
    #[test]
    fn jinja_grammar_is_abi_compatible_with_workspace_tree_sitter() {
        let jinja: tree_sitter::Language = tree_sitter_jinja_dialects::LANGUAGE.into();
        let abi = jinja.abi_version();
        assert!(
            (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
                .contains(&abi),
            "tree-sitter-jinja-dialects ABI {abi} is outside the range this tree-sitter \
             supports ({}..={})",
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION,
        );
    }

    #[test]
    fn jinja_grammar_parses_every_dialect_it_claims() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_jinja_dialects::LANGUAGE.into())
            .expect("jinja grammar should load into the workspace tree-sitter");
        // One sample per dialect the crate advertises, since a single grammar
        // serving all of njk/j2/twig/dj is the reason it was chosen.
        for (dialect, source) in [
            ("jinja2", "{% for x in xs %}{{ x|e }}{% endfor %}\n"),
            ("nunjucks", "{% set n = 1 %}{{ n + 1 }}\n"),
            ("twig", "{% if a is not empty %}{{ a.b }}{% endif %}\n"),
            (
                "django",
                "{% extends \"base.html\" %}{% block body %}{% endblock %}\n",
            ),
        ] {
            let tree = parser
                .parse(source, None)
                .unwrap_or_else(|| panic!("{dialect} sample should parse"));
            assert!(
                !tree.root_node().has_error(),
                "{dialect} sample produced an ERROR node: {}",
                tree.root_node().to_sexp(),
            );
        }
    }

    // ---- `#set! injection.combined` ------------------------------------------

    /// The inventory tripwire.
    ///
    /// Combined injections change how a grammar's whole document is assembled, so
    /// a grammar bump that quietly introduces the directive must not slip through
    /// review. F#'s `xml_doc` rule is the only one in the tree today; it arrived
    /// with the upstream `tree_sitter_fsharp::INJECTIONS_QUERY` rather than being
    /// written here.
    #[test]
    fn combined_injection_declarations_are_exactly_the_known_set() {
        let mut declared = Vec::new();
        for lang in all_supported_languages() {
            let Some(spec) = tree_sitter_highlight_spec(lang) else {
                continue;
            };
            for (pattern_ix, combined) in spec.injection_combined_patterns.iter().enumerate() {
                if *combined {
                    declared.push((lang, pattern_ix));
                }
            }
            assert_eq!(
                spec.has_combined_injections,
                spec.injection_combined_patterns.iter().any(|c| *c),
                "{lang:?} has a stale has_combined_injections flag"
            );
        }
        assert_eq!(
            declared,
            vec![
                // queries/jinja_injections.scm -- the HTML around the template tags.
                (DiffSyntaxLanguage::Jinja, 0),
                // queries/nix_injections.scm -- bash in script/hook attributes.
                (DiffSyntaxLanguage::Nix, 0),
                (DiffSyntaxLanguage::Nix, 1),
                (DiffSyntaxLanguage::Nix, 2),
                (DiffSyntaxLanguage::Nix, 3),
                // Upstream tree_sitter_fsharp::INJECTIONS_QUERY -- `xml_doc` lines.
                (DiffSyntaxLanguage::FSharp, 3),
            ],
            "the set of grammars declaring `#set! injection.combined` changed. Every entry \
             here parses all its matches as one document via set_included_ranges, so a new \
             one needs the gap-clipping and cache behaviour reviewed -- it is not a \
             drop-in.\nfound: {declared:?}"
        );
    }

    // The one-range cases are the point: a single included range is the shape
    // every non-combined injection has, and both helpers have to leave it alone.
    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn merge_sorted_injection_ranges_normalises_for_set_included_ranges() {
        // Empty stays empty: an empty slice is tree-sitter's "whole document"
        // reset, which callers must detect rather than pass on.
        assert!(merge_sorted_injection_ranges(Vec::new()).is_empty());
        // Degenerate ranges are dropped, not kept as zero-width.
        assert!(merge_sorted_injection_ranges(vec![5..5]).is_empty());
        assert_eq!(merge_sorted_injection_ranges(vec![2..5]), vec![2..5]);
        // Unsorted input is sorted: set_included_ranges rejects descending ranges.
        assert_eq!(
            merge_sorted_injection_ranges(vec![10..12, 2..5]),
            vec![2..5, 10..12]
        );
        // Touching ranges coalesce, so the gap list carries no empty entries.
        assert_eq!(merge_sorted_injection_ranges(vec![2..5, 5..9]), vec![2..9]);
        // Overlapping ranges coalesce: set_included_ranges rejects overlap.
        assert_eq!(merge_sorted_injection_ranges(vec![2..7, 5..9]), vec![2..9]);
        // Fully contained range is absorbed rather than shortening the outer one.
        assert_eq!(
            merge_sorted_injection_ranges(vec![2..20, 5..9]),
            vec![2..20]
        );
    }

    // The one-range cases are the point: a single included range is the shape
    // every non-combined injection has, and both helpers have to leave it alone.
    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn combined_injection_gaps_are_the_complement_within_the_window() {
        assert_eq!(combined_injection_gaps(0..100, &[]), vec![0..100]);
        assert!(combined_injection_gaps(0..100, &[0..100]).is_empty());
        assert_eq!(
            combined_injection_gaps(0..100, &[10..20, 30..40]),
            vec![0..10, 20..30, 40..100]
        );
        // Range flush against each edge produces no leading/trailing gap.
        assert_eq!(combined_injection_gaps(0..100, &[0..20]), vec![20..100]);
        assert_eq!(combined_injection_gaps(0..100, &[80..100]), vec![0..80]);
        // Ranges reaching outside the window are clipped to it, not extrapolated.
        assert_eq!(combined_injection_gaps(20..80, &[0..30]), vec![30..80]);
        assert_eq!(combined_injection_gaps(20..80, &[70..200]), vec![20..70]);
        assert!(combined_injection_gaps(20..80, &[0..200]).is_empty());
        // A range entirely outside contributes nothing and does not swallow the window.
        assert_eq!(combined_injection_gaps(20..80, &[200..300]), vec![20..80]);
    }

    /// Two halves of an HTML element split across a host-grammar tag must parse as
    /// one element, and the injected grammar must not colour the host bytes
    /// between them.
    ///
    /// HTML stands in for the eventual template grammar here so the test needs no
    /// new dependency. The ranges are the same shape a `(text) @injection.content`
    /// rule produces on a real template.
    #[test]
    fn combined_injection_parses_disjoint_ranges_as_one_document() {
        let text = "<ul>\n{% for x in xs %}<li>hi</li>{% endfor %}\n</ul>\n";
        let input = treesitter_document_input_from_text(text);
        let bytes = text.as_bytes();
        let ranges = combined_test_ranges(text);

        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
        let tree = parse_combined_injection_tree(spec, bytes, input.line_starts.as_ref(), &ranges)
            .expect("combined parse should succeed");

        assert_eq!(
            tree.root_node().start_byte(),
            ranges[0].start,
            "a tree parsed with included_ranges reports document offsets"
        );
        assert!(
            !tree.root_node().has_error(),
            "the <ul> opened before `{{% for %}}` should close after `{{% endfor %}}` when the \
             three text runs are parsed as one document: {}",
            tree.root_node().to_sexp(),
        );
    }

    /// The other half: nodes straddling two included ranges report a byte range
    /// covering the host bytes in between, so their captures have to be clipped.
    #[test]
    fn combined_injection_tokens_do_not_bleed_into_the_gaps() {
        let text = "<ul>\n{% for x in xs %}<li>hi</li>{% endfor %}\n</ul>\n";
        let input = treesitter_document_input_from_text(text);
        let bytes = text.as_bytes();
        let ranges = combined_test_ranges(text);
        let line_starts = input.line_starts.as_ref();

        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
        let tree = parse_combined_injection_tree(spec, bytes, line_starts, &ranges)
            .expect("combined parse should succeed");

        let line_count = line_starts.len();
        let mut tokens = collect_treesitter_document_line_tokens_for_line_window(
            &tree,
            spec,
            bytes,
            line_starts,
            0,
            line_count,
        );
        let window_end = line_region_end_byte(line_starts, bytes.len(), line_count - 1);
        for gap in combined_injection_gaps(0..window_end, &ranges) {
            subtract_absolute_range_from_document_tokens(line_starts, bytes, 0, &mut tokens, gap);
        }

        // Line 1 is `{% for x in xs %}<li>hi</li>{% endfor %}`. Only the `<li>hi</li>`
        // slice belongs to HTML; both template tags are host-grammar bytes.
        let line_start = line_starts[1];
        let html_start = text.find("<li>").expect("li") - line_start;
        let html_end = text.find("{% endfor %}").expect("endfor") - line_start;
        for token in &tokens[1] {
            assert!(
                token.range.start >= html_start && token.range.end <= html_end,
                "injected HTML token {:?} escaped its included range \
                 ({html_start}..{html_end}) into a `{{% … %}}` gap",
                token.range,
            );
        }
        assert!(
            !tokens[1].is_empty(),
            "clipping should not have removed the genuine <li> tokens as well"
        );
    }

    /// Byte ranges of the HTML runs in the combined-injection fixture, i.e. what a
    /// template grammar's `(text)` nodes would capture.
    fn combined_test_ranges(text: &str) -> Vec<Range<usize>> {
        let for_tag = text.find("{% for x in xs %}").expect("for tag");
        let li = text.find("<li>").expect("li");
        let endfor = text.find("{% endfor %}").expect("endfor");
        let after_endfor = endfor + "{% endfor %}".len();
        merge_sorted_injection_ranges(vec![0..for_tag, li..endfor, after_endfor..text.len()])
    }

    // ---- Combined-injection scoping -------------------------------------------

    /// A template dense enough to exercise the per-window ceilings, `rows` lines of
    /// `cells` cells each wrapped in a block so the body is one big text run.
    fn dense_jinja_table(rows: usize, cells: usize) -> String {
        let mut lines = vec!["{% block body %}".to_string()];
        for row in 0..rows {
            let mut line = String::from("<tr>");
            for cell in 0..cells {
                line.push_str(&format!("<td>{{{{ r{row}.c{cell} }}}}</td>"));
            }
            line.push_str("</tr>");
            lines.push(line);
        }
        lines.push("{% endblock %}".to_string());
        lines.join("\n")
    }

    /// An 8-column table row used to produce 513 ranges in one 64-line chunk, one
    /// over the ceiling, and the whole chunk lost its HTML.
    #[test]
    fn dense_table_template_keeps_its_html_highlighting() {
        for cells in [4usize, 8, 16] {
            let text = dense_jinja_table(200, cells);
            let lines: Vec<&str> = text.lines().collect();
            let doc = prepare_test_document(DiffSyntaxLanguage::Jinja, &text);

            let kinds = token_kinds_for_line_fragment(doc, 100, lines[100], "<td>");
            assert!(
                kinds.contains(&SyntaxTokenKind::Tag),
                "a {cells}-cell table row lost its HTML highlighting: {kinds:?}"
            );
        }
    }

    /// The byte ceiling had the same defect at an ordinary file size: all the HTML
    /// between two template tags is ONE `(text)` node, so a ~1800-line template
    /// tripped the 128KB ceiling in every window.
    #[test]
    fn large_template_with_one_huge_text_run_keeps_its_html_highlighting() {
        let mut lines = vec!["{% block body %}".to_string()];
        for ix in 0..2_400 {
            lines.push(format!(
                "  <span class=\"cell\" data-row=\"{ix}\">value {ix} padded out</span>"
            ));
        }
        lines.push("{% endblock %}".to_string());
        let text = lines.join("\n");
        assert!(
            text.len() > TS_COMBINED_INJECTION_MAX_BYTES,
            "fixture must exceed the byte ceiling to be a regression test ({} bytes)",
            text.len()
        );

        let line_refs: Vec<&str> = text.lines().collect();
        let doc = prepare_test_document(DiffSyntaxLanguage::Jinja, &text);
        for line_ix in [1usize, 700, 1_500, 2_300] {
            let kinds = token_kinds_for_line_fragment(doc, line_ix, line_refs[line_ix], "span");
            assert!(
                kinds.contains(&SyntaxTokenKind::Tag),
                "line {line_ix} of a {}-byte template lost its HTML: {kinds:?}",
                text.len()
            );
        }
    }

    /// The property the whole optimisation rests on, and the reason for the margin:
    /// a `<section` whose attributes run onto the next lines straddles the window
    /// edge, and an exact clip cuts it in half. Asserted against an unclipped parse
    /// so it stays honest if the margin is ever tuned.
    #[test]
    fn clipping_a_combined_layer_to_the_window_preserves_its_tokens() {
        let mut lines = vec!["{% block body %}".to_string()];
        for ix in 0..300 {
            if ix == 62 || ix == 126 {
                lines.push("  <section".to_string());
                lines.push("     id=\"straddle\"".to_string());
                lines.push("     class=\"wide\">body</section>".to_string());
            } else {
                lines.push(format!(
                    "  <span class=\"c{ix}\" data-x='y'>row {ix}</span>"
                ));
            }
        }
        lines.push("{% endblock %}".to_string());
        let text = lines.join("\n") + "\n";

        let input = treesitter_document_input_from_text(&text);
        let bytes = text.as_bytes();
        let line_starts = input.line_starts.as_ref();
        let jinja = tree_sitter_highlight_spec(DiffSyntaxLanguage::Jinja).expect("jinja spec");
        let root = with_ts_parser_parse_result(&jinja.ts_language, |parser| {
            parse_treesitter_tree(parser, bytes, None, None)
        })
        .expect("root parse");

        let start_line_ix = 64usize;
        let end_line_ix = start_line_ix + TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
        let matches = collect_treesitter_injection_matches_for_line_window(
            &root,
            jinja,
            bytes,
            line_starts,
            start_line_ix,
            end_line_ix,
        );
        let group = matches.combined.first().expect("one combined html group");
        assert_eq!(
            group.ranges.len(),
            1,
            "the fixture's body must be one text run, or it is not testing the hard case"
        );

        let window_start = line_starts[start_line_ix];
        let window_end = line_region_end_byte(line_starts, bytes.len(), end_line_ix - 1);
        let html = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
        let render = |ranges: &[Range<usize>]| -> Vec<Vec<SyntaxToken>> {
            let tree = parse_combined_injection_tree(html, bytes, line_starts, ranges)
                .expect("combined parse");
            let mut injected = collect_treesitter_document_line_tokens_for_line_window(
                &tree,
                html,
                bytes,
                line_starts,
                start_line_ix,
                end_line_ix,
            );
            for gap in combined_injection_gaps(window_start..window_end, ranges) {
                subtract_absolute_range_from_document_tokens(
                    line_starts,
                    bytes,
                    start_line_ix,
                    &mut injected,
                    gap,
                );
            }
            injected
        };

        let clip_region =
            combined_injection_clip_region(line_starts, bytes.len(), start_line_ix, end_line_ix);
        let clipped_ranges = clip_injection_ranges_to_region(&group.ranges, &clip_region);
        let clipped_bytes: usize = clipped_ranges.iter().map(|r| r.end - r.start).sum();
        let full_bytes: usize = group.ranges.iter().map(|r| r.end - r.start).sum();
        assert!(
            clipped_bytes < full_bytes,
            "the clip must actually shrink the parse ({clipped_bytes} vs {full_bytes})"
        );

        assert_eq!(
            render(&group.ranges),
            render(&clipped_ranges),
            "clipping to the window changed the tokens the window renders"
        );
    }

    /// The clip region is the window plus a margin on both sides, and the margin is
    /// load-bearing rather than decorative -- see the constant.
    #[test]
    fn combined_injection_clip_region_pads_the_window_on_both_sides() {
        let text = dense_jinja_table(400, 2);
        let input = treesitter_document_input_from_text(&text);
        let line_starts = input.line_starts.as_ref();
        let len = text.len();

        let start_line_ix = 200usize;
        let end_line_ix = start_line_ix + TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
        let region = combined_injection_clip_region(line_starts, len, start_line_ix, end_line_ix);
        let window_start = line_starts[start_line_ix];
        let window_end = line_region_end_byte(line_starts, len, end_line_ix - 1);

        assert!(
            region.start < window_start && region.end > window_end,
            "clip region {region:?} must strictly contain the window \
             {window_start}..{window_end}"
        );
        assert_eq!(
            window_start - region.start,
            TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES,
            "leading margin"
        );

        // ... and still bounded, which is what makes the ceilings window-scoped.
        assert!(
            region.end - region.start
                < window_end - window_start + 2 * TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES + 1,
            "clip region must not grow past window + 2 * margin"
        );

        // At the top of the document the margin runs out rather than underflowing.
        let head = combined_injection_clip_region(line_starts, len, 0, 8);
        assert_eq!(head.start, 0, "no underflow at the start of the document");
    }

    /// A cut that touches nothing must leave the line's tokens exactly as they were,
    /// and must not reallocate to do it.
    #[test]
    fn subtracting_a_non_overlapping_range_leaves_line_tokens_untouched() {
        let original = vec![
            SyntaxToken {
                range: 0..4,
                kind: SyntaxTokenKind::Tag,
            },
            SyntaxToken {
                range: 10..14,
                kind: SyntaxTokenKind::String,
            },
        ];

        // Entirely before, entirely after, and in the gap between the two tokens.
        for cut in [20..30usize, 4..10, 100..200] {
            let mut tokens = original.clone();
            subtract_relative_range_from_line_tokens(&mut tokens, cut.clone());
            assert_eq!(tokens, original, "cut {cut:?} must be a no-op");
        }

        // ... and a cut that does overlap still splits, so the fast path is not
        // swallowing real work.
        let mut tokens = original.clone();
        subtract_relative_range_from_line_tokens(&mut tokens, 2..12);
        assert_eq!(
            tokens,
            vec![
                SyntaxToken {
                    range: 0..2,
                    kind: SyntaxTokenKind::Tag,
                },
                SyntaxToken {
                    range: 12..14,
                    kind: SyntaxTokenKind::String,
                },
            ]
        );
    }

    /// Pins the ordering rather than a symptom: no in-tree grammar declares both
    /// kinds over one span yet, but with combined applied first an overlapping
    /// single would delete its tokens and repaint only part of the span.
    #[test]
    fn combined_injection_groups_are_applied_after_the_single_ones() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/view/rows/diff_text/syntax/prepared.rs"
        ))
        .expect("prepared.rs should be readable");
        let body_start = source
            .find("fn apply_injection_query_tokens_for_document")
            .expect("the function that applies both kinds of layer");
        let body = &source[body_start..];
        let body_end = body.find("\n}\n").expect("the end of the function");
        let body = &body[..body_end];

        let singles_at = body
            .find("for injection in &injections.singles")
            .expect("the singles loop");
        let combined_at = body
            .find("for group in &injections.combined")
            .expect("the combined loop");
        assert!(
            singles_at < combined_at,
            "the singles loop must run before the combined one, or a single's \
             subtraction erases combined tokens nothing repaints"
        );
    }

    /// F# XML doc comments are the one in-tree consumer of `injection.combined`.
    ///
    /// `xml_doc` is a per-line token, so before combined support each `///` line
    /// was its own XML document: `<summary>` on one line and `</summary>` on
    /// another never met, and each cost an entry in the 32-slot injection cache.
    #[test]
    fn fsharp_xml_doc_comment_is_highlighted_as_one_xml_document() {
        let lines = [
            /* 0 */ "/// <summary>",
            /* 1 */ "/// Adds two numbers.",
            /* 2 */ "/// </summary>",
            /* 3 */ "let add x y = x + y",
        ];
        let doc = prepare_test_document(DiffSyntaxLanguage::FSharp, &lines.join("\n"));

        let closing = token_kinds_for_line_fragment(doc, 2, lines[2], "summary");
        assert!(
            closing.contains(&SyntaxTokenKind::Tag),
            "`</summary>` closes a tag opened two lines earlier, which only parses \
             when the three xml_doc lines are one document: {closing:?}"
        );

        // And the layer stays inside its own ranges: the following line is F#.
        let keyword = token_kinds_for_line_fragment(doc, 3, lines[3], "let");
        assert!(
            keyword.contains(&SyntaxTokenKind::Keyword),
            "the combined XML layer leaked past the doc comment onto `let`: {keyword:?}"
        );
    }

    /// Combined layers must not touch the per-node injection cache at all.
    ///
    /// The 32-slot LRU is keyed by a single node's content hash, which a combined
    /// layer does not have -- its identity is a *set* of ranges. Feeding it one
    /// entry per constituent node is what F# used to do: 200 `///` lines meant 200
    /// entries into a 32-slot cache, evicting everything
    /// `vue_static_inline_styles_do_not_flood_the_injection_cache` depends on.
    ///
    /// This is not a claim that combined parses are memoised elsewhere. They are
    /// not: each of the N/64 chunks pays its own on first build, and clipping is
    /// what keeps that cost proportional to the window.
    #[test]
    fn combined_injections_do_not_consume_the_per_node_injection_cache() {
        TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

        let mut lines = vec!["/// <summary>".to_string()];
        for ix in 0..200 {
            lines.push(format!("/// line {ix}"));
        }
        lines.push("/// </summary>".to_string());
        lines.push("let add x y = x + y".to_string());
        let line_count = lines.len();

        let doc = prepare_test_document(DiffSyntaxLanguage::FSharp, &lines.join("\n"));
        for line_ix in 0..line_count {
            let _ = syntax_tokens_for_prepared_document_line(doc, line_ix);
        }

        let cached = TS_INJECTION_CACHE.with(|cache| cache.borrow().len());
        assert_eq!(
            cached, 0,
            "a combined layer's identity is a set of ranges, not one node's content \
             hash, so it must not enter TS_INJECTION_CACHE (cap \
             {TS_INJECTION_CACHE_MAX_ENTRIES}); {line_count} lines of xml doc comment \
             created {cached} entries"
        );

        TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
    }

    /// The failure this would cause is invisible and global.
    ///
    /// `TS_PARSER` is pooled and its included ranges are sticky; `with_ts_parser`
    /// can skip `set_language` entirely on the fast path, so a combined parse that
    /// forgot to clear them would truncate the *next* root parse on this thread —
    /// for any language, with no error anywhere. Asserted behaviourally so it
    /// survives tree-sitter API changes.
    #[test]
    fn combined_injection_parse_clears_the_pooled_parsers_included_ranges() {
        let fsharp = ["/// <summary>", "/// x", "/// </summary>", "let x = 1"];
        let _ = prepare_test_document(DiffSyntaxLanguage::FSharp, &fsharp.join("\n"));

        let mut rust_lines = Vec::new();
        for ix in 0..300 {
            rust_lines.push(format!("fn f{ix}() -> u32 {{ {ix} }}"));
        }
        let last_ix = rust_lines.len() - 1;
        let last_line = rust_lines[last_ix].clone();
        let doc = prepare_test_document(DiffSyntaxLanguage::Rust, &rust_lines.join("\n"));

        let kinds = token_kinds_for_line_fragment(doc, last_ix, &last_line, "fn");
        assert!(
            kinds.contains(&SyntaxTokenKind::Keyword),
            "the last line of a 300-line Rust document lost its tokens after a combined \
             injection ran on this thread -- the pooled parser's included ranges were not \
             cleared, so the root parse was truncated: {kinds:?}"
        );
    }

    /// A `(text)`-style combined rule fires once per node, so this is the query
    /// most likely to overflow the in-progress match pool. Overflow is worse for a
    /// combined layer than a single one: tree-sitter discards matches silently, and
    /// a missing range changes the document the injected grammar assembles.
    #[test]
    fn fsharp_xml_doc_injection_stays_under_the_match_limit_on_a_long_doc_comment() {
        let mut lines = vec!["/// <summary>".to_string()];
        for ix in 0..200 {
            lines.push(format!("/// line {ix}"));
        }
        lines.push("/// </summary>".to_string());
        let text = lines.join("\n");

        let lang: tree_sitter::Language = tree_sitter_fsharp::LANGUAGE_FSHARP.into();
        let query = tree_sitter::Query::new(&lang, tree_sitter_fsharp::INJECTIONS_QUERY)
            .expect("fsharp injections.scm should compile");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("fsharp grammar");
        let tree = parser.parse(&text, None).expect("doc comment should parse");

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
            "the F# injection query overflowed the {TS_QUERY_MATCH_LIMIT}-match in-progress \
             pool on a {}-line doc comment; a combined group that loses ranges assembles a \
             different document, so the whole group is dropped when this happens",
            lines.len(),
        );
        assert!(matched > 0, "the doc comment should produce matches at all");
    }

    #[test]
    fn highlight_spec_exposes_ts_language() {
        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Rust)
            .expect("Rust highlight spec should exist");
        // Verify the ts_language field is usable for parsing
        with_ts_parser(&spec.ts_language, |_| ()).expect("should accept the spec's ts_language");
    }

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
