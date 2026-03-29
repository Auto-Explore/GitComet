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
const DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_TEST: Duration = Duration::from_millis(2);
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
const TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES: usize = 64;
const TS_PENDING_PARSE_REQUEST_MAX_ENTRIES: usize = 8;
#[cfg(any(test, feature = "syntax-web"))]
const HTML_HIGHLIGHTS_QUERY: &str = include_str!("queries/html_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const HTML_INJECTIONS_QUERY: &str = include_str!("queries/html_injections.scm");
#[cfg(any(test, feature = "syntax-web"))]
const CSS_HIGHLIGHTS_QUERY: &str = tree_sitter_css::HIGHLIGHTS_QUERY;
#[cfg(any(test, feature = "syntax-web"))]
const JAVASCRIPT_HIGHLIGHTS_QUERY: &str = include_str!("queries/javascript_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const TYPESCRIPT_HIGHLIGHTS_QUERY: &str = include_str!("queries/typescript_highlights.scm");
#[cfg(any(test, feature = "syntax-web"))]
const TSX_HIGHLIGHTS_QUERY: &str = include_str!("queries/tsx_highlights.scm");
#[cfg(any(test, feature = "syntax-rust"))]
const RUST_HIGHLIGHTS_QUERY: &str = include_str!("queries/rust_highlights.scm");
#[cfg(any(test, feature = "syntax-rust"))]
const RUST_INJECTIONS_QUERY: &str = include_str!("queries/rust_injections.scm");
#[cfg(any(test, feature = "syntax-xml"))]
const XML_HIGHLIGHTS_QUERY: &str = tree_sitter_xml::XML_HIGHLIGHT_QUERY;

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

        if (needs_language_reset || !parser_language_matches)
            && parser.set_language(ts_language).is_err() {
                invalidate_ts_parser_language_fast_path();
                return None;
            }
            #[cfg(test)]
            TS_PARSER_SET_LANGUAGE_CALL_COUNT.with(|count| count.set(count.get() + 1));
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
    C,
    Cpp,
    CSharp,
    FSharp,
    VisualBasic,
    Java,
    Php,
    Ruby,
    Json,
    Toml,
    Yaml,
    Sql,
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
    text: Arc<str>,
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
            foreground_parse: if cfg!(test) {
                DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_TEST
            } else {
                DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PrepareTreesitterDocumentResult {
    Ready(PreparedSyntaxDocument),
    TimedOut,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyntaxCacheDropMode {
    DeferredWhenLarge,
    #[cfg(feature = "benchmarks")]
    InlineWhenLarge,
}

enum SyntaxCacheDropMessage {
    Drop(SyntaxCacheDropPayload),
    #[cfg(any(test, feature = "benchmarks"))]
    Flush(mpsc::Sender<()>),
}

struct SyntaxCacheDropPayload {
    line_tokens: Vec<Arc<[SyntaxToken]>>,
    estimated_bytes: usize,
}

impl SyntaxCacheDropPayload {
    fn new(line_tokens: Vec<Arc<[SyntaxToken]>>, estimated_bytes: usize) -> Self {
        Self {
            line_tokens,
            estimated_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PreparedSyntaxChunkKey {
    cache_key: PreparedSyntaxCacheKey,
    chunk_ix: usize,
}

#[derive(Clone, Debug)]
struct PreparedSyntaxChunkBuildRequest {
    chunk_key: PreparedSyntaxChunkKey,
    line_count: usize,
    thread_id: std::thread::ThreadId,
    tree_state: Arc<PreparedSyntaxTreeState>,
}

#[derive(Clone, Debug)]
struct PreparedSyntaxChunkBuildResult {
    chunk_key: PreparedSyntaxChunkKey,
    chunk_tokens: Option<Vec<Arc<[SyntaxToken]>>>,
    chunk_build_ms: u64,
    thread_id: std::thread::ThreadId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PreparedSyntaxLineTokensRequest {
    Ready(Arc<[SyntaxToken]>),
    Pending,
}

#[derive(Clone)]
struct CachedSingleLineSyntaxTokens {
    text: Arc<str>,
    tokens: Arc<[SyntaxToken]>,
}

struct SingleLineSyntaxTokenCache {
    by_key: HashMap<SingleLineSyntaxTokenCacheKey, CachedSingleLineSyntaxTokens>,
    lru_order: VecDeque<SingleLineSyntaxTokenCacheKey>,
}

impl SingleLineSyntaxTokenCache {
    fn new() -> Self {
        Self {
            by_key: HashMap::default(),
            lru_order: VecDeque::new(),
        }
    }

    fn touch_key(&mut self, key: SingleLineSyntaxTokenCacheKey) {
        if self.lru_order.back() == Some(&key) {
            return;
        }
        if let Some(pos) = self.lru_order.iter().position(|existing| *existing == key) {
            self.lru_order.remove(pos);
        }
        self.lru_order.push_back(key);
    }

    fn remove_key(&mut self, key: SingleLineSyntaxTokenCacheKey) {
        self.by_key.remove(&key);
        if let Some(pos) = self.lru_order.iter().position(|existing| *existing == key) {
            self.lru_order.remove(pos);
        }
    }

    fn get(
        &mut self,
        key: SingleLineSyntaxTokenCacheKey,
        text: &str,
    ) -> Option<Arc<[SyntaxToken]>> {
        if self
            .by_key
            .get(&key)
            .is_some_and(|entry| entry.text.as_ref() != text)
        {
            self.remove_key(key);
            return None;
        }

        let tokens = self.by_key.get(&key)?.tokens.clone();
        self.touch_key(key);
        Some(tokens)
    }

    fn insert(
        &mut self,
        key: SingleLineSyntaxTokenCacheKey,
        text: &str,
        tokens: Arc<[SyntaxToken]>,
    ) {
        self.by_key.insert(
            key,
            CachedSingleLineSyntaxTokens {
                text: Arc::<str>::from(text),
                tokens,
            },
        );
        self.touch_key(key);
        while self.by_key.len() > TS_LINE_TOKEN_CACHE_MAX_ENTRIES {
            let Some(evicted) = self.lru_order.pop_front() else {
                break;
            };
            self.by_key.remove(&evicted);
        }
    }
}

#[cfg(test)]
static TS_DEFERRED_DROP_ENQUEUED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static TS_DEFERRED_DROP_COMPLETED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static TS_INLINE_DROP_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
fn syntax_cache_drop_sender() -> Option<&'static mpsc::Sender<SyntaxCacheDropMessage>> {
    static SENDER: OnceLock<Option<mpsc::Sender<SyntaxCacheDropMessage>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<SyntaxCacheDropMessage>();
            let builder = std::thread::Builder::new().name("gitcomet-syntax-drop".to_string());
            let _handle = builder
                .spawn(move || {
                    while let Ok(msg) = rx.recv() {
                        match msg {
                            SyntaxCacheDropMessage::Drop(drop_payload) => {
                                drop(drop_payload);
                                #[cfg(test)]
                                TS_DEFERRED_DROP_COMPLETED
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            #[cfg(any(test, feature = "benchmarks"))]
                            SyntaxCacheDropMessage::Flush(ack) => {
                                let _ = ack.send(());
                            }
                        }
                    }
                })
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

fn shared_prepared_document_seed_store()
-> &'static Mutex<HashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>> {
    static STORE: OnceLock<Mutex<HashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::default()))
}

fn store_shared_prepared_document_seed(document: &PreparedSyntaxDocumentData) {
    let mut store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if store.len() >= TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES
        && let Some(evict_key) = store.keys().next().copied()
        && evict_key != document.cache_key
    {
        store.remove(&evict_key);
    }
    store.insert(document.cache_key, document.clone());
}

fn shared_prepared_document_seed(
    cache_key: PreparedSyntaxCacheKey,
) -> Option<PreparedSyntaxDocumentData> {
    let store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    store.get(&cache_key).cloned()
}

fn merge_shared_prepared_document_chunk(
    cache_key: PreparedSyntaxCacheKey,
    chunk_ix: usize,
    chunk_tokens: Option<Vec<Arc<[SyntaxToken]>>>,
) -> bool {
    let mut store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(document) = store.get_mut(&cache_key) else {
        return false;
    };
    if document.line_token_chunks.contains_key(&chunk_ix) {
        return false;
    }

    let fallback_empty_chunk = || {
        let start = chunk_ix.saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
        let end = start
            .saturating_add(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
            .min(document.line_count);
        let empty: Arc<[SyntaxToken]> = Arc::from([]);
        vec![empty; end.saturating_sub(start)]
    };
    document
        .line_token_chunks
        .insert(chunk_ix, chunk_tokens.unwrap_or_else(fallback_empty_chunk));
    true
}

struct PreparedSyntaxChunkWorker {
    sender: mpsc::Sender<PreparedSyntaxChunkBuildRequest>,
    receiver: Mutex<mpsc::Receiver<PreparedSyntaxChunkBuildResult>>,
    deferred_results: Mutex<VecDeque<PreparedSyntaxChunkBuildResult>>,
}

fn syntax_chunk_worker() -> Option<&'static PreparedSyntaxChunkWorker> {
    static WORKER: OnceLock<Option<PreparedSyntaxChunkWorker>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (request_tx, request_rx) = mpsc::channel::<PreparedSyntaxChunkBuildRequest>();
            let (result_tx, result_rx) = mpsc::channel::<PreparedSyntaxChunkBuildResult>();
            let builder = std::thread::Builder::new().name("gitcomet-syntax-chunks".to_string());
            let _handle = builder
                .spawn(move || {
                    while let Ok(request) = request_rx.recv() {
                        let (chunk_tokens, chunk_build_ms) = build_line_token_chunk_for_state(
                            request.tree_state.as_ref(),
                            request.line_count,
                            request.chunk_key.chunk_ix,
                        );
                        let _ = result_tx.send(PreparedSyntaxChunkBuildResult {
                            chunk_key: request.chunk_key,
                            chunk_tokens,
                            chunk_build_ms,
                            thread_id: request.thread_id,
                        });
                    }
                })
                .ok()?;
            Some(PreparedSyntaxChunkWorker {
                sender: request_tx,
                receiver: Mutex::new(result_rx),
                deferred_results: Mutex::new(VecDeque::new()),
            })
        })
        .as_ref()
}

fn estimated_line_tokens_allocation_bytes(line_tokens: &[Arc<[SyntaxToken]>]) -> usize {
    let outer = line_tokens
        .len()
        .saturating_mul(std::mem::size_of::<Arc<[SyntaxToken]>>());
    let inner = line_tokens.iter().fold(0usize, |acc, line| {
        acc.saturating_add(
            line.len()
                .saturating_mul(std::mem::size_of::<SyntaxToken>()),
        )
    });
    outer.saturating_add(inner)
}

fn estimated_chunked_line_tokens_allocation_bytes(
    line_token_chunks: &HashMap<usize, Vec<Arc<[SyntaxToken]>>>,
) -> usize {
    line_token_chunks.values().fold(0usize, |acc, chunk| {
        acc.saturating_add(estimated_line_tokens_allocation_bytes(chunk))
    })
}

fn share_recent_line_token_arcs(line_tokens: Vec<Vec<SyntaxToken>>) -> Vec<Arc<[SyntaxToken]>> {
    let mut shared = Vec::with_capacity(line_tokens.len());
    let mut previous: Option<Arc<[SyntaxToken]>> = None;
    let mut previous_two_back: Option<Arc<[SyntaxToken]>> = None;

    for line in line_tokens {
        let line_slice = line.as_slice();
        let line_tokens = if line_slice.is_empty() {
            empty_line_syntax_tokens()
        } else if let Some(existing) = previous
            .as_ref()
            .filter(|candidate| candidate.as_ref() == line_slice)
            .or_else(|| {
                previous_two_back
                    .as_ref()
                    .filter(|candidate| candidate.as_ref() == line_slice)
            })
        {
            existing.clone()
        } else {
            Arc::from(line)
        };
        previous_two_back = previous.replace(line_tokens.clone());
        shared.push(line_tokens);
    }

    shared
}

fn drop_line_tokens_with_mode(
    drop_payload: SyntaxCacheDropPayload,
    drop_mode: SyntaxCacheDropMode,
) {
    let should_try_deferred = matches!(drop_mode, SyntaxCacheDropMode::DeferredWhenLarge)
        && drop_payload.estimated_bytes >= TS_DEFERRED_DROP_MIN_BYTES;

    if should_try_deferred && let Some(sender) = syntax_cache_drop_sender() {
        if sender
            .send(SyntaxCacheDropMessage::Drop(drop_payload))
            .is_ok()
        {
            #[cfg(test)]
            TS_DEFERRED_DROP_ENQUEUED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        #[cfg(test)]
        TS_INLINE_DROP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    #[cfg(test)]
    TS_INLINE_DROP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    drop(drop_payload);
}

#[cfg(test)]
fn deferred_drop_counters() -> (usize, usize, usize) {
    (
        TS_DEFERRED_DROP_ENQUEUED.load(std::sync::atomic::Ordering::Relaxed),
        TS_DEFERRED_DROP_COMPLETED.load(std::sync::atomic::Ordering::Relaxed),
        TS_INLINE_DROP_COUNT.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
fn reset_deferred_drop_counters() {
    TS_DEFERRED_DROP_ENQUEUED.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_DEFERRED_DROP_COMPLETED.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_INLINE_DROP_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_INCREMENTAL_PARSE_COUNT.with(|count| count.set(0));
    TS_INCREMENTAL_FALLBACK_COUNT.with(|count| count.set(0));
    TS_DOCUMENT_HASH_COUNT.with(|count| count.set(0));
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
fn incremental_reparse_counters() -> (usize, usize) {
    (
        TS_INCREMENTAL_PARSE_COUNT.with(Cell::get),
        TS_INCREMENTAL_FALLBACK_COUNT.with(Cell::get),
    )
}

#[cfg(test)]
fn tree_state_clone_count() -> usize {
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.get())
}

#[cfg(test)]
fn document_hash_count() -> usize {
    TS_DOCUMENT_HASH_COUNT.with(Cell::get)
}

#[cfg(any(test, feature = "benchmarks"))]
fn flush_deferred_syntax_cache_drop_queue_with_timeout(timeout: Duration) -> bool {
    let Some(sender) = syntax_cache_drop_sender() else {
        return false;
    };
    let (ack_tx, ack_rx) = mpsc::channel();
    if sender.send(SyntaxCacheDropMessage::Flush(ack_tx)).is_err() {
        return false;
    }
    ack_rx.recv_timeout(timeout).is_ok()
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn benchmark_flush_deferred_drop_queue() -> bool {
    flush_deferred_syntax_cache_drop_queue_with_timeout(Duration::from_secs(2))
}

fn incremental_reparse_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(TS_INCREMENTAL_REPARSE_ENABLE_ENV)
            .ok()
            .map(|raw| {
                let normalized = raw.trim().to_ascii_lowercase();
                !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
            })
            .unwrap_or(true)
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PreparedSyntaxCacheMetrics {
    hit: u64,
    miss: u64,
    evict: u64,
    chunk_build_ms: u64,
}

#[derive(Clone, Debug)]
struct TreesitterCachedDocument {
    line_count: usize,
    line_token_chunks: HashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    line_token_bytes: usize,
    tree_state: Option<PreparedSyntaxTreeState>,
}

impl TreesitterCachedDocument {
    fn from_chunked_line_tokens(
        line_count: usize,
        line_token_chunks: HashMap<usize, Vec<Arc<[SyntaxToken]>>>,
        tree_state: Option<PreparedSyntaxTreeState>,
    ) -> Self {
        let line_token_bytes = estimated_chunked_line_tokens_allocation_bytes(&line_token_chunks);
        Self {
            line_count,
            line_token_chunks,
            line_token_bytes,
            tree_state,
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn from_line_tokens(
        line_tokens: Vec<Vec<SyntaxToken>>,
        tree_state: Option<PreparedSyntaxTreeState>,
    ) -> Self {
        let line_count = line_tokens.len();
        let arc_tokens = share_recent_line_token_arcs(line_tokens);
        let line_token_bytes = estimated_line_tokens_allocation_bytes(&arc_tokens);
        Self {
            line_count,
            line_token_chunks: chunk_line_tokens_by_row(arc_tokens),
            line_token_bytes,
            tree_state,
        }
    }

    fn chunk_bounds(&self, chunk_ix: usize) -> Range<usize> {
        let start = chunk_ix.saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
        let end = start
            .saturating_add(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
            .min(self.line_count);
        start.min(end)..end
    }

    fn source_identity(&self) -> Option<PreparedSyntaxSourceIdentity> {
        self.tree_state
            .as_ref()
            .map(|tree_state| PreparedSyntaxSourceIdentity {
                language: tree_state.language,
                text_ptr: tree_state.text.as_ptr() as usize,
                text_len: tree_state.text.len(),
                line_count: self.line_count,
            })
    }

    fn into_drop_payload(self) -> SyntaxCacheDropPayload {
        if self.line_token_chunks.is_empty() {
            return SyntaxCacheDropPayload::new(Vec::new(), self.line_token_bytes);
        }

        let mut chunks = self.line_token_chunks.into_iter().collect::<Vec<_>>();
        chunks.sort_by_key(|(chunk_ix, _)| *chunk_ix);
        let line_capacity = chunks
            .iter()
            .map(|(_, chunk)| chunk.len())
            .fold(0usize, |acc, len| acc.saturating_add(len));
        let mut out = Vec::with_capacity(line_capacity);
        for (_, chunk) in chunks {
            out.extend(chunk);
        }
        let payload = SyntaxCacheDropPayload::new(out, self.line_token_bytes);
        debug_assert_eq!(
            payload.estimated_bytes,
            estimated_line_tokens_allocation_bytes(&payload.line_tokens),
            "cached line-token byte accounting should match flattened drop payloads"
        );
        payload
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn chunk_line_tokens_by_row(
    line_tokens: Vec<Arc<[SyntaxToken]>>,
) -> HashMap<usize, Vec<Arc<[SyntaxToken]>>> {
    if line_tokens.is_empty() {
        return HashMap::default();
    }

    let mut chunks = HashMap::default();
    let mut chunk_ix = 0usize;
    let mut chunk = Vec::with_capacity(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
    for line in line_tokens {
        chunk.push(line);
        if chunk.len() >= TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS {
            chunks.insert(chunk_ix, chunk);
            chunk_ix = chunk_ix.saturating_add(1);
            chunk = Vec::with_capacity(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
        }
    }
    if !chunk.is_empty() {
        chunks.insert(chunk_ix, chunk);
    }
    chunks
}

fn insert_line_token_chunk(
    document: &mut TreesitterCachedDocument,
    chunk_ix: usize,
    chunk_tokens: Option<Vec<Arc<[SyntaxToken]>>>,
) {
    if document.line_token_chunks.contains_key(&chunk_ix) {
        return;
    }

    let fallback_empty_chunk = || {
        let bounds = document.chunk_bounds(chunk_ix);
        let empty: Arc<[SyntaxToken]> = Arc::from([]);
        vec![empty; bounds.end.saturating_sub(bounds.start)]
    };
    let chunk = chunk_tokens.unwrap_or_else(fallback_empty_chunk);
    document.line_token_bytes = document
        .line_token_bytes
        .saturating_add(estimated_line_tokens_allocation_bytes(&chunk));
    document.line_token_chunks.insert(chunk_ix, chunk);
}

fn clone_tree_state_for_chunk_build_ref(
    tree_state: &PreparedSyntaxTreeState,
) -> PreparedSyntaxTreeState {
    #[cfg(test)]
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    tree_state.clone()
}

fn shared_tree_state_for_chunk_build(
    tree_state: &Option<PreparedSyntaxTreeState>,
) -> Option<Arc<PreparedSyntaxTreeState>> {
    tree_state
        .as_ref()
        .map(clone_tree_state_for_chunk_build_ref)
        .map(Arc::new)
}

fn build_line_token_chunk_for_state(
    tree_state: &PreparedSyntaxTreeState,
    line_count: usize,
    chunk_ix: usize,
) -> (Option<Vec<Arc<[SyntaxToken]>>>, u64) {
    let chunk_start = chunk_ix.saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
    if chunk_start >= line_count {
        return (Some(Vec::new()), 0);
    }
    let chunk_end = chunk_start
        .saturating_add(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
        .min(line_count);
    let Some(highlight) = tree_sitter_highlight_spec(tree_state.language) else {
        return (None, 0);
    };
    let started = Instant::now();
    let chunk = collect_treesitter_document_line_tokens_for_line_window(
        &tree_state.tree,
        highlight,
        tree_state.text.as_bytes(),
        &tree_state.line_starts,
        chunk_start,
        chunk_end,
    );
    let chunk_build_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let arc_chunk = share_recent_line_token_arcs(chunk);
    (Some(arc_chunk), chunk_build_ms)
}

fn chunk_count_for_line_count(line_count: usize) -> usize {
    if line_count == 0 {
        0
    } else {
        (line_count.saturating_sub(1) / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS).saturating_add(1)
    }
}

struct TreesitterDocumentCache {
    by_cache_key: HashMap<PreparedSyntaxCacheKey, TreesitterCachedDocument>,
    by_source_identity: HashMap<PreparedSyntaxSourceIdentity, PreparedSyntaxCacheKey>,
    lru_order: VecDeque<PreparedSyntaxCacheKey>,
    pending_chunk_requests: HashSet<PreparedSyntaxChunkKey>,
    metrics: PreparedSyntaxCacheMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SharedDocumentMergeResult {
    None,
    Inserted,
    Updated,
}

impl TreesitterDocumentCache {
    fn new() -> Self {
        Self {
            by_cache_key: HashMap::default(),
            by_source_identity: HashMap::default(),
            lru_order: VecDeque::new(),
            pending_chunk_requests: HashSet::default(),
            metrics: PreparedSyntaxCacheMetrics::default(),
        }
    }

    fn touch_key(&mut self, cache_key: PreparedSyntaxCacheKey) {
        if self.lru_order.back() == Some(&cache_key) {
            return;
        }
        if let Some(pos) = self
            .lru_order
            .iter()
            .position(|candidate| *candidate == cache_key)
        {
            self.lru_order.remove(pos);
        }
        self.lru_order.push_back(cache_key);
    }

    fn record_hit(&mut self, cache_key: PreparedSyntaxCacheKey) {
        self.metrics.hit = self.metrics.hit.saturating_add(1);
        self.touch_key(cache_key);
    }

    fn remove_source_identity_mapping(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        document: &TreesitterCachedDocument,
    ) {
        let Some(identity) = document.source_identity() else {
            return;
        };
        if self.by_source_identity.get(&identity) == Some(&cache_key) {
            self.by_source_identity.remove(&identity);
        }
    }

    fn index_source_identity(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        document: &TreesitterCachedDocument,
    ) {
        let Some(identity) = document.source_identity() else {
            return;
        };
        self.by_source_identity.insert(identity, cache_key);
    }

    fn evict_if_needed(&mut self, drop_mode: SyntaxCacheDropMode) {
        while self.by_cache_key.len() >= TS_DOCUMENT_CACHE_MAX_ENTRIES {
            let Some(evict_key) = self.lru_order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.by_cache_key.remove(&evict_key) {
                self.remove_source_identity_mapping(evict_key, &evicted);
                self.metrics.evict = self.metrics.evict.saturating_add(1);
                drop_line_tokens_with_mode(evicted.into_drop_payload(), drop_mode);
                break;
            }
        }
    }

    fn contains_document(&mut self, cache_key: PreparedSyntaxCacheKey, line_count: usize) -> bool {
        let exists = self
            .by_cache_key
            .get(&cache_key)
            .is_some_and(|document| document.line_count == line_count);
        if exists {
            self.touch_key(cache_key);
        }
        exists
    }

    fn document_for_source_identity(
        &mut self,
        identity: PreparedSyntaxSourceIdentity,
    ) -> Option<PreparedSyntaxDocument> {
        let cache_key = *self.by_source_identity.get(&identity)?;
        if !self.by_cache_key.contains_key(&cache_key) {
            self.by_source_identity.remove(&identity);
            return None;
        }
        self.touch_key(cache_key);
        Some(PreparedSyntaxDocument { cache_key })
    }

    fn alias_source_identity(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        identity: PreparedSyntaxSourceIdentity,
    ) {
        if !self.by_cache_key.contains_key(&cache_key) {
            return;
        }
        self.by_source_identity.insert(identity, cache_key);
        self.touch_key(cache_key);
    }

    fn extract_line_from_chunk(
        &self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
        chunk_ix: usize,
    ) -> Arc<[SyntaxToken]> {
        static EMPTY: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
        let empty = || Arc::clone(EMPTY.get_or_init(|| Arc::from([])));
        self.by_cache_key
            .get(&cache_key)
            .map(|document| {
                let chunk_bounds = document.chunk_bounds(chunk_ix);
                let line_offset = line_ix.saturating_sub(chunk_bounds.start);
                document
                    .line_token_chunks
                    .get(&chunk_ix)
                    .and_then(|chunk| chunk.get(line_offset))
                    .cloned()
                    .unwrap_or_else(empty)
            })
            .unwrap_or_else(empty)
    }

    /// Returns `(line_count, has_chunk)` for the given cache key and line index,
    /// or `None` if the document is not in the cache.
    fn lookup_chunk_state(
        &self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
    ) -> Option<(usize, usize, bool)> {
        let chunk_ix = line_ix / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
        let document = self.by_cache_key.get(&cache_key)?;
        Some((
            document.line_count,
            chunk_ix,
            document.line_token_chunks.contains_key(&chunk_ix),
        ))
    }

    fn merge_document_from_shared_seed(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> SharedDocumentMergeResult {
        let Some(shared_document) = shared_prepared_document_seed(cache_key) else {
            return SharedDocumentMergeResult::None;
        };

        let mut inserted = false;
        let mut updated = false;
        let mut remove_identity = None;
        let mut insert_identity = None;

        match self.by_cache_key.entry(cache_key) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                let document = TreesitterCachedDocument::from_chunked_line_tokens(
                    shared_document.line_count,
                    shared_document.line_token_chunks,
                    shared_document.tree_state,
                );
                insert_identity = document.source_identity();
                entry.insert(document);
                inserted = true;
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let document = entry.get_mut();
                if document.line_count != shared_document.line_count {
                    let old_identity = document.source_identity();
                    let replaced = std::mem::replace(
                        document,
                        TreesitterCachedDocument::from_chunked_line_tokens(
                            shared_document.line_count,
                            shared_document.line_token_chunks,
                            shared_document.tree_state,
                        ),
                    );
                    remove_identity = old_identity;
                    insert_identity = document.source_identity();
                    drop_line_tokens_with_mode(
                        replaced.into_drop_payload(),
                        SyntaxCacheDropMode::DeferredWhenLarge,
                    );
                    updated = true;
                } else {
                    let old_identity = document.source_identity();
                    if document.tree_state.is_none() && shared_document.tree_state.is_some() {
                        document.tree_state = shared_document.tree_state;
                        updated = true;
                    }

                    for (chunk_ix, chunk) in shared_document.line_token_chunks {
                        if document.line_token_chunks.contains_key(&chunk_ix) {
                            continue;
                        }
                        insert_line_token_chunk(document, chunk_ix, Some(chunk));
                        self.pending_chunk_requests.remove(&PreparedSyntaxChunkKey {
                            cache_key,
                            chunk_ix,
                        });
                        updated = true;
                    }
                    if document.source_identity() != old_identity {
                        remove_identity = old_identity;
                        insert_identity = document.source_identity();
                    }
                }
            }
        }

        if let Some(identity) = remove_identity
            && self.by_source_identity.get(&identity) == Some(&cache_key)
        {
            self.by_source_identity.remove(&identity);
        }
        if let Some(identity) = insert_identity {
            self.by_source_identity.insert(identity, cache_key);
        }

        if inserted || updated {
            self.touch_key(cache_key);
        }

        if inserted {
            SharedDocumentMergeResult::Inserted
        } else if updated {
            SharedDocumentMergeResult::Updated
        } else {
            SharedDocumentMergeResult::None
        }
    }

    #[cfg(test)]
    fn line_tokens(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
    ) -> Option<Vec<SyntaxToken>> {
        let (line_count, chunk_ix, has_chunk) = self.lookup_chunk_state(cache_key, line_ix)?;

        if line_ix >= line_count {
            self.record_hit(cache_key);
            return Some(Vec::new());
        }

        if !has_chunk {
            self.metrics.miss = self.metrics.miss.saturating_add(1);
            let tree_state = self
                .by_cache_key
                .get(&cache_key)
                .and_then(|document| shared_tree_state_for_chunk_build(&document.tree_state));
            if let Some(tree_state) = tree_state {
                self.build_chunk_sync_and_insert(
                    cache_key,
                    chunk_ix,
                    tree_state.as_ref(),
                    line_count,
                );
            }
        } else {
            self.metrics.hit = self.metrics.hit.saturating_add(1);
        }

        self.touch_key(cache_key);
        Some(
            self.extract_line_from_chunk(cache_key, line_ix, chunk_ix)
                .to_vec(),
        )
    }

    fn build_chunk_sync_and_insert(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        chunk_ix: usize,
        tree_state: &PreparedSyntaxTreeState,
        line_count: usize,
    ) {
        let (chunk_tokens, chunk_build_ms) =
            build_line_token_chunk_for_state(tree_state, line_count, chunk_ix);
        self.metrics.chunk_build_ms = self.metrics.chunk_build_ms.saturating_add(chunk_build_ms);
        let shared_chunk_tokens = chunk_tokens.clone();
        if let Some(document) = self.by_cache_key.get_mut(&cache_key) {
            insert_line_token_chunk(document, chunk_ix, chunk_tokens);
        }
        let _ = merge_shared_prepared_document_chunk(cache_key, chunk_ix, shared_chunk_tokens);
    }

    fn queue_chunk_build_request_nonblocking(
        &mut self,
        chunk_key: PreparedSyntaxChunkKey,
        line_count: usize,
        thread_id: std::thread::ThreadId,
        tree_state: &Arc<PreparedSyntaxTreeState>,
    ) -> bool {
        if self
            .by_cache_key
            .get(&chunk_key.cache_key)
            .is_some_and(|document| document.line_token_chunks.contains_key(&chunk_key.chunk_ix))
        {
            return true;
        }
        if self.pending_chunk_requests.contains(&chunk_key) {
            return true;
        }

        let Some(worker) = syntax_chunk_worker() else {
            return false;
        };
        let request = PreparedSyntaxChunkBuildRequest {
            chunk_key,
            line_count,
            thread_id,
            tree_state: Arc::clone(tree_state),
        };
        if worker.sender.send(request).is_err() {
            return false;
        }
        self.pending_chunk_requests.insert(chunk_key);
        true
    }

    fn prefetch_adjacent_chunk_builds_nonblocking(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_count: usize,
        center_chunk_ix: usize,
        thread_id: std::thread::ThreadId,
        tree_state: &Arc<PreparedSyntaxTreeState>,
    ) {
        let chunk_count = chunk_count_for_line_count(line_count);
        if chunk_count == 0 {
            return;
        }

        let start_chunk_ix =
            center_chunk_ix.saturating_sub(TS_DOCUMENT_LINE_TOKEN_PREFETCH_GUARD_CHUNKS);
        let end_chunk_ix = center_chunk_ix
            .saturating_add(TS_DOCUMENT_LINE_TOKEN_PREFETCH_GUARD_CHUNKS)
            .saturating_add(1)
            .min(chunk_count);
        for chunk_ix in start_chunk_ix..end_chunk_ix {
            if chunk_ix == center_chunk_ix {
                continue;
            }
            let _ = self.queue_chunk_build_request_nonblocking(
                PreparedSyntaxChunkKey {
                    cache_key,
                    chunk_ix,
                },
                line_count,
                thread_id,
                tree_state,
            );
        }
    }

    fn request_line_tokens_with_context(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
        allow_sync_build_on_insert: bool,
    ) -> Option<PreparedSyntaxLineTokensRequest> {
        let (line_count, chunk_ix, has_chunk) = self.lookup_chunk_state(cache_key, line_ix)?;

        static EMPTY_TOKENS: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
        let empty_tokens = || Arc::clone(EMPTY_TOKENS.get_or_init(|| Arc::from([])));

        if line_ix >= line_count {
            self.record_hit(cache_key);
            return Some(PreparedSyntaxLineTokensRequest::Ready(empty_tokens()));
        }

        if has_chunk {
            self.record_hit(cache_key);
            return Some(PreparedSyntaxLineTokensRequest::Ready(
                self.extract_line_from_chunk(cache_key, line_ix, chunk_ix),
            ));
        }

        self.metrics.miss = self.metrics.miss.saturating_add(1);
        let chunk_key = PreparedSyntaxChunkKey {
            cache_key,
            chunk_ix,
        };
        if self.pending_chunk_requests.contains(&chunk_key) {
            self.touch_key(cache_key);
            return Some(PreparedSyntaxLineTokensRequest::Pending);
        }

        let tree_state = self
            .by_cache_key
            .get(&cache_key)
            .and_then(|document| shared_tree_state_for_chunk_build(&document.tree_state));
        let Some(tree_state) = tree_state else {
            self.touch_key(cache_key);
            return Some(PreparedSyntaxLineTokensRequest::Ready(empty_tokens()));
        };

        if allow_sync_build_on_insert {
            self.build_chunk_sync_and_insert(cache_key, chunk_ix, tree_state.as_ref(), line_count);
            self.record_hit(cache_key);
            return Some(PreparedSyntaxLineTokensRequest::Ready(
                self.extract_line_from_chunk(cache_key, line_ix, chunk_ix),
            ));
        }

        let thread_id = std::thread::current().id();
        if self.queue_chunk_build_request_nonblocking(chunk_key, line_count, thread_id, &tree_state)
        {
            self.prefetch_adjacent_chunk_builds_nonblocking(
                cache_key,
                line_count,
                chunk_ix,
                thread_id,
                &tree_state,
            );
            self.touch_key(cache_key);
            return Some(PreparedSyntaxLineTokensRequest::Pending);
        }

        self.build_chunk_sync_and_insert(cache_key, chunk_ix, tree_state.as_ref(), line_count);

        self.record_hit(cache_key);
        Some(PreparedSyntaxLineTokensRequest::Ready(
            self.extract_line_from_chunk(cache_key, line_ix, chunk_ix),
        ))
    }

    fn request_line_tokens(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
    ) -> Option<PreparedSyntaxLineTokensRequest> {
        // Visible row draws can request a chunk on the render thread while the
        // main-pane poller drains worker completions on the app thread. Apply
        // any completions targeted at this current thread before we decide that
        // the line is still pending, otherwise the row can remain stuck on the
        // heuristic fallback until some other code path drains it here.
        let _ = self.drain_completed_chunk_builds_for_cache_key(cache_key);
        let allow_sync_build_on_insert = matches!(
            self.merge_document_from_shared_seed(cache_key),
            SharedDocumentMergeResult::Inserted
        );
        self.request_line_tokens_with_context(cache_key, line_ix, allow_sync_build_on_insert)
    }

    fn request_line_tokens_range(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_range: Range<usize>,
    ) -> Option<Vec<PreparedSyntaxLineTokensRequest>> {
        if line_range.is_empty() {
            return Some(Vec::new());
        }

        let _ = self.drain_completed_chunk_builds_for_cache_key(cache_key);
        let mut allow_sync_build_on_insert = matches!(
            self.merge_document_from_shared_seed(cache_key),
            SharedDocumentMergeResult::Inserted
        );
        let mut requests = Vec::with_capacity(line_range.len());
        for line_ix in line_range {
            requests.push(self.request_line_tokens_with_context(
                cache_key,
                line_ix,
                allow_sync_build_on_insert,
            )?);
            allow_sync_build_on_insert = false;
        }
        Some(requests)
    }

    fn prepared_document_data(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_count: usize,
    ) -> Option<PreparedSyntaxDocumentData> {
        let data = {
            let document = self.by_cache_key.get(&cache_key)?;
            if document.line_count != line_count {
                return None;
            }
            PreparedSyntaxDocumentData {
                cache_key,
                line_count: document.line_count,
                line_token_chunks: document.line_token_chunks.clone(),
                tree_state: document.tree_state.clone(),
            }
        };
        self.touch_key(cache_key);
        Some(data)
    }

    fn tree_state(&mut self, cache_key: PreparedSyntaxCacheKey) -> Option<PreparedSyntaxTreeState> {
        let tree_state = self.by_cache_key.get(&cache_key)?.tree_state.clone();
        self.touch_key(cache_key);
        tree_state
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn metrics(&self) -> PreparedSyntaxCacheMetrics {
        self.metrics
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn reset_metrics(&mut self) {
        self.metrics = PreparedSyntaxCacheMetrics::default();
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn loaded_chunk_count(&self, cache_key: PreparedSyntaxCacheKey) -> Option<usize> {
        Some(self.by_cache_key.get(&cache_key)?.line_token_chunks.len())
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn contains_key(&self, cache_key: PreparedSyntaxCacheKey) -> bool {
        self.by_cache_key.contains_key(&cache_key)
    }

    fn drain_completed_chunk_builds(&mut self) -> usize {
        self.drain_completed_chunk_builds_matching(None)
    }

    fn drain_completed_chunk_builds_for_cache_key(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> usize {
        self.drain_completed_chunk_builds_matching(Some(cache_key))
    }

    fn drain_completed_chunk_builds_matching(
        &mut self,
        target_cache_key: Option<PreparedSyntaxCacheKey>,
    ) -> usize {
        let Some(worker) = syntax_chunk_worker() else {
            return 0;
        };
        let current_thread = std::thread::current().id();

        let mut ready_results = Vec::new();
        {
            let mut deferred = match worker.deferred_results.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let mut remaining = VecDeque::with_capacity(deferred.len());
            while let Some(result) = deferred.pop_front() {
                let _ = merge_shared_prepared_document_chunk(
                    result.chunk_key.cache_key,
                    result.chunk_key.chunk_ix,
                    result.chunk_tokens.clone(),
                );
                if should_apply_chunk_build_result(&result, current_thread, target_cache_key) {
                    ready_results.push(result);
                } else {
                    remaining.push_back(result);
                }
            }
            *deferred = remaining;
        }

        let mut polled_results = Vec::new();
        {
            let receiver = match worker.receiver.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            while let Ok(result) = receiver.try_recv() {
                polled_results.push(result);
            }
        }
        if !polled_results.is_empty() {
            let mut deferred = match worker.deferred_results.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            for result in polled_results {
                let _ = merge_shared_prepared_document_chunk(
                    result.chunk_key.cache_key,
                    result.chunk_key.chunk_ix,
                    result.chunk_tokens.clone(),
                );
                if should_apply_chunk_build_result(&result, current_thread, target_cache_key) {
                    ready_results.push(result);
                } else {
                    deferred.push_back(result);
                }
            }
        }

        let mut applied = 0usize;
        for result in ready_results {
            self.pending_chunk_requests.remove(&result.chunk_key);
            self.metrics.chunk_build_ms = self
                .metrics
                .chunk_build_ms
                .saturating_add(result.chunk_build_ms);
            let shared_chunk_tokens = result.chunk_tokens.clone();
            let Some(document) = self.by_cache_key.get_mut(&result.chunk_key.cache_key) else {
                let _ = merge_shared_prepared_document_chunk(
                    result.chunk_key.cache_key,
                    result.chunk_key.chunk_ix,
                    shared_chunk_tokens,
                );
                applied = applied.saturating_add(1);
                continue;
            };
            if document
                .line_token_chunks
                .contains_key(&result.chunk_key.chunk_ix)
            {
                let _ = merge_shared_prepared_document_chunk(
                    result.chunk_key.cache_key,
                    result.chunk_key.chunk_ix,
                    shared_chunk_tokens,
                );
                continue;
            }
            insert_line_token_chunk(document, result.chunk_key.chunk_ix, result.chunk_tokens);
            let _ = merge_shared_prepared_document_chunk(
                result.chunk_key.cache_key,
                result.chunk_key.chunk_ix,
                shared_chunk_tokens,
            );
            applied = applied.saturating_add(1);
        }
        applied
    }

    fn has_pending_chunk_requests(&self) -> bool {
        !self.pending_chunk_requests.is_empty()
    }

    fn has_pending_chunk_requests_for_cache_key(&self, cache_key: PreparedSyntaxCacheKey) -> bool {
        self.pending_chunk_requests
            .iter()
            .any(|candidate| candidate.cache_key == cache_key)
    }

    #[cfg(test)]
    fn make_test_cache_key(doc_hash: u64) -> PreparedSyntaxCacheKey {
        PreparedSyntaxCacheKey {
            language: DiffSyntaxLanguage::Rust,
            doc_hash,
        }
    }

    #[cfg(test)]
    fn insert_document(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_tokens: Vec<Vec<SyntaxToken>>,
    ) {
        self.insert_document_with_tree_state(cache_key, line_tokens, None);
    }

    #[cfg(test)]
    fn insert_document_with_tree_state(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_tokens: Vec<Vec<SyntaxToken>>,
        tree_state: Option<PreparedSyntaxTreeState>,
    ) {
        self.insert_document_with_mode(
            cache_key,
            TreesitterCachedDocument::from_line_tokens(line_tokens, tree_state),
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    }

    fn insert_document_with_mode(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        document: TreesitterCachedDocument,
        drop_mode: SyntaxCacheDropMode,
    ) {
        if !self.by_cache_key.contains_key(&cache_key) {
            self.evict_if_needed(drop_mode);
        } else if let Some(pos) = self
            .lru_order
            .iter()
            .position(|candidate| *candidate == cache_key)
        {
            self.lru_order.remove(pos);
        }

        self.index_source_identity(cache_key, &document);
        if let Some(replaced) = self.by_cache_key.insert(cache_key, document) {
            self.remove_source_identity_mapping(cache_key, &replaced);
            drop_line_tokens_with_mode(replaced.into_drop_payload(), drop_mode);
        }
        self.touch_key(cache_key);
    }
}

fn should_apply_chunk_build_result(
    result: &PreparedSyntaxChunkBuildResult,
    current_thread: std::thread::ThreadId,
    target_cache_key: Option<PreparedSyntaxCacheKey>,
) -> bool {
    result.thread_id == current_thread
        && match target_cache_key {
            Some(cache_key) => result.chunk_key.cache_key == cache_key,
            None => true,
        }
}

#[derive(Clone)]
struct TreesitterDocumentInput {
    text: Arc<str>,
    line_starts: Arc<[usize]>,
}

#[derive(Clone)]
struct TreesitterDocumentParseRequest {
    language: DiffSyntaxLanguage,
    ts_language: tree_sitter::Language,
    input: TreesitterDocumentInput,
    cache_key: PreparedSyntaxCacheKey,
}

struct PendingParseRequest {
    identity: PreparedSyntaxSourceIdentity,
    request: TreesitterDocumentParseRequest,
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn benchmark_reset_prepared_syntax_cache_metrics() {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().reset_metrics());
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn benchmark_prepared_syntax_cache_metrics() -> (u64, u64, u64, u64) {
    TS_DOCUMENT_CACHE.with(|cache| {
        let metrics = cache.borrow().metrics();
        (
            metrics.hit,
            metrics.miss,
            metrics.evict,
            metrics.chunk_build_ms,
        )
    })
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn benchmark_prepared_syntax_loaded_chunk_count(
    document: PreparedSyntaxDocument,
) -> Option<usize> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().loaded_chunk_count(document.cache_key))
}

#[cfg(feature = "benchmarks")]
pub(super) fn benchmark_prepared_syntax_cache_contains_document(
    document: PreparedSyntaxDocument,
) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().contains_key(document.cache_key))
}

#[cfg(test)]
fn prepared_syntax_cache_metrics() -> PreparedSyntaxCacheMetrics {
    let (hit, miss, evict, chunk_build_ms) = benchmark_prepared_syntax_cache_metrics();
    PreparedSyntaxCacheMetrics {
        hit,
        miss,
        evict,
        chunk_build_ms,
    }
}

#[cfg(test)]
fn reset_prepared_syntax_cache() {
    TS_DOCUMENT_CACHE.with(|cache| {
        *cache.borrow_mut() = TreesitterDocumentCache::new();
    });
    TS_PENDING_PARSE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let mut store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    store.clear();
}

#[cfg(test)]
fn prepared_syntax_loaded_chunk_count(document: PreparedSyntaxDocument) -> usize {
    benchmark_prepared_syntax_loaded_chunk_count(document).unwrap_or_default()
}

fn diff_syntax_language_for_identifier(identifier: &str) -> Option<DiffSyntaxLanguage> {
    Some(match identifier {
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn" => DiffSyntaxLanguage::Markdown,
        "html" | "htm" => DiffSyntaxLanguage::Html,
        "xml" | "svg" | "xsl" | "xslt" | "xsd" | "xhtml" | "plist" | "csproj" | "fsproj"
        | "vbproj" | "sln" | "props" | "targets" | "resx" | "xaml" | "wsdl" | "rss" | "atom"
        | "opml" | "glade" | "ui" | "iml" => DiffSyntaxLanguage::Xml,
        "css" | "less" | "sass" | "scss" => DiffSyntaxLanguage::Css,
        "hcl" | "tf" | "tfvars" => DiffSyntaxLanguage::Hcl,
        "bicep" => DiffSyntaxLanguage::Bicep,
        "lua" => DiffSyntaxLanguage::Lua,
        "mk" | "make" | "makefile" | "gnumakefile" => DiffSyntaxLanguage::Makefile,
        "kt" | "kts" | "kotlin" => DiffSyntaxLanguage::Kotlin,
        "zig" => DiffSyntaxLanguage::Zig,
        "rs" | "rust" => DiffSyntaxLanguage::Rust,
        "py" | "python" => DiffSyntaxLanguage::Python,
        "js" | "mjs" | "cjs" | "javascript" => DiffSyntaxLanguage::JavaScript,
        "jsx" => DiffSyntaxLanguage::Tsx,
        "ts" | "cts" | "mts" | "typescript" => DiffSyntaxLanguage::TypeScript,
        "tsx" => DiffSyntaxLanguage::Tsx,
        "go" | "golang" => DiffSyntaxLanguage::Go,
        "c" | "h" => DiffSyntaxLanguage::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" => DiffSyntaxLanguage::Cpp,
        "cs" | "c#" | "csharp" => DiffSyntaxLanguage::CSharp,
        "fs" | "fsx" | "fsi" | "f#" | "fsharp" => DiffSyntaxLanguage::FSharp,
        "vb" | "vbs" | "vbnet" | "visualbasic" => DiffSyntaxLanguage::VisualBasic,
        "java" => DiffSyntaxLanguage::Java,
        "php" | "phtml" => DiffSyntaxLanguage::Php,
        "rb" | "ruby" => DiffSyntaxLanguage::Ruby,
        "json" => DiffSyntaxLanguage::Json,
        "toml" => DiffSyntaxLanguage::Toml,
        "yaml" | "yml" => DiffSyntaxLanguage::Yaml,
        "sql" => DiffSyntaxLanguage::Sql,
        "sh" | "bash" | "zsh" | "shell" | "console" => DiffSyntaxLanguage::Bash,
        _ => return None,
    })
}

pub(in crate::view) fn diff_syntax_language_for_path(
    path: impl AsRef<std::path::Path>,
) -> Option<DiffSyntaxLanguage> {
    let p = path.as_ref();
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    let ext = ascii_lowercase_for_match(ext);
    diff_syntax_language_for_identifier(ext.as_ref()).or_else(|| {
        let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let file_name = ascii_lowercase_for_match(file_name);
        diff_syntax_language_for_identifier(file_name.as_ref())
    })
}

pub(in crate::view) fn diff_syntax_language_for_code_fence_info(
    info: &str,
) -> Option<DiffSyntaxLanguage> {
    let token = info
        .trim()
        .split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
        .find(|segment| !segment.is_empty())?;
    let token = token.trim_matches(|ch| matches!(ch, '{' | '}'));
    let token = token.trim_start_matches('.');
    let token = token.strip_prefix("language-").unwrap_or(token);
    let token = ascii_lowercase_for_match(token);
    diff_syntax_language_for_identifier(token.as_ref())
}

fn empty_line_syntax_tokens() -> Arc<[SyntaxToken]> {
    static EMPTY: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from([])))
}

fn should_cache_single_line_syntax_tokens(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TREESITTER_LINE_BYTES
}

fn single_line_syntax_token_cache_key(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
) -> SingleLineSyntaxTokenCacheKey {
    SingleLineSyntaxTokenCacheKey {
        language,
        mode,
        text_hash: treesitter_text_hash(text),
    }
}

fn syntax_tokens_for_line_uncached(
    text: &str,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
) -> Vec<SyntaxToken> {
    if matches!(language, DiffSyntaxLanguage::Markdown) {
        return syntax_tokens_for_line_markdown(text);
    }

    match mode {
        DiffSyntaxMode::HeuristicOnly => syntax_tokens_for_line_heuristic(text, language),
        DiffSyntaxMode::Auto => {
            if matches!(language, DiffSyntaxLanguage::Yaml) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if !should_use_treesitter_for_line(text) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if is_heuristic_sufficient_for_line(text, language) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if let Some(tokens) = syntax_tokens_for_line_treesitter(text, language) {
                return tokens;
            }
            syntax_tokens_for_line_heuristic(text, language)
        }
    }
}

pub(super) fn syntax_tokens_for_line_shared(
    text: &str,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
) -> Arc<[SyntaxToken]> {
    if text.is_empty() {
        return empty_line_syntax_tokens();
    }

    if !should_cache_single_line_syntax_tokens(text) {
        return Arc::from(syntax_tokens_for_line_uncached(text, language, mode));
    }

    let key = single_line_syntax_token_cache_key(language, mode, text);
    if let Some(tokens) = TS_LINE_TOKEN_CACHE.with(|cache| cache.borrow_mut().get(key, text)) {
        return tokens;
    }

    let tokens: Arc<[SyntaxToken]> =
        Arc::from(syntax_tokens_for_line_uncached(text, language, mode));
    TS_LINE_TOKEN_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, text, Arc::clone(&tokens));
    });
    tokens
}

#[allow(dead_code)]
pub(super) fn syntax_tokens_for_line(
    text: &str,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
) -> Vec<SyntaxToken> {
    syntax_tokens_for_line_shared(text, language, mode)
        .as_ref()
        .to_vec()
}

pub(super) fn prepare_treesitter_document_with_budget_reuse_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    budget: DiffSyntaxBudget,
    old_document: Option<PreparedSyntaxDocument>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> PrepareTreesitterDocumentResult {
    let line_count = normalized_treesitter_line_starts(text.as_ref(), line_starts.as_ref()).len();
    if let Some(identity) =
        prepared_document_source_identity_for_shared_text(language, mode, text.as_ref(), line_count)
        && let Some(document) = TS_DOCUMENT_CACHE
            .with(|cache| cache.borrow_mut().document_for_source_identity(identity))
        {
            return PrepareTreesitterDocumentResult::Ready(document);
        }
    let source_identity = prepared_document_source_identity_for_shared_text(
        language,
        mode,
        text.as_ref(),
        line_count,
    );
    let old_tree_state = old_document.and_then(prepared_document_tree_state);
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    let reparse_plan = old_tree_state.as_ref().and_then(|state| {
        build_treesitter_reparse_plan(state, language, &input, edit_hint.as_ref())
    });
    if let (Some(document), Some(identity), Some(TreesitterReparsePlan::Unchanged)) =
        (old_document, source_identity, reparse_plan.as_ref())
    {
        TS_DOCUMENT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .alias_source_identity(document.cache_key, identity);
        });
        clear_pending_parse_request(identity);
        return PrepareTreesitterDocumentResult::Ready(document);
    }
    let Some(request) = treesitter_document_parse_request_from_input_with_reuse(
        language,
        mode,
        input,
        old_tree_state.as_ref(),
        reparse_plan.as_ref(),
    ) else {
        if let Some(identity) = source_identity {
            clear_pending_parse_request(identity);
        }
        return PrepareTreesitterDocumentResult::Unsupported;
    };
    let has_cache_hit = TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .contains_document(request.cache_key, line_count)
    });
    if has_cache_hit {
        if let Some(identity) = source_identity {
            clear_pending_parse_request(identity);
        }
        return PrepareTreesitterDocumentResult::Ready(PreparedSyntaxDocument {
            cache_key: request.cache_key,
        });
    }

    let result = prepare_treesitter_document_request_after_cache_lookup(
        request.clone(),
        Some(budget),
        old_document,
        edit_hint.is_some(),
        reparse_plan,
    );
    match (source_identity, result) {
        (Some(identity), PrepareTreesitterDocumentResult::TimedOut) => {
            store_pending_parse_request(identity, request);
        }
        (Some(identity), _) => {
            clear_pending_parse_request(identity);
        }
        (None, _) => {}
    }
    result
}

#[cfg(test)]
pub(super) fn prepare_treesitter_document_in_background_text_with_reuse(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    old_document: Option<PreparedSyntaxDocument>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> Option<PreparedSyntaxDocumentData> {
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    let old_tree_state = old_document.and_then(prepared_document_tree_state);
    let reparse_plan = old_tree_state.as_ref().and_then(|state| {
        build_treesitter_reparse_plan(state, language, &input, edit_hint.as_ref())
    });
    let request = take_pending_parse_request_for_shared_text(
        language,
        mode,
        input.text.as_ref(),
        input.line_starts.as_ref(),
    )
    .or_else(|| {
        treesitter_document_parse_request_from_input_with_reuse(
            language,
            mode,
            input,
            old_tree_state.as_ref(),
            reparse_plan.as_ref(),
        )
    })?;
    prepare_treesitter_document_data_request_impl(request, old_document, reparse_plan)
}

pub(super) fn prepare_treesitter_document_in_background_text_with_reparse_seed(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    reparse_seed: Option<PreparedSyntaxReparseSeed>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> Option<PreparedSyntaxDocumentData> {
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    let (old_document, old_tree_state) = match reparse_seed {
        Some(seed) => (Some(seed.document), Some(seed.tree_state)),
        None => (None, None),
    };
    let reparse_plan = old_tree_state.as_ref().and_then(|state| {
        build_treesitter_reparse_plan(state, language, &input, edit_hint.as_ref())
    });
    let request = take_pending_parse_request_for_shared_text(
        language,
        mode,
        input.text.as_ref(),
        input.line_starts.as_ref(),
    )
    .or_else(|| {
        treesitter_document_parse_request_from_input_with_reuse(
            language,
            mode,
            input,
            old_tree_state.as_ref(),
            reparse_plan.as_ref(),
        )
    })?;
    prepare_treesitter_document_data_request_impl(request, old_document, reparse_plan)
}

pub(super) fn inject_prepared_document_data(
    document: PreparedSyntaxDocumentData,
) -> PreparedSyntaxDocument {
    store_shared_prepared_document_seed(&document);
    TS_DOCUMENT_CACHE.with(|cache| {
        cache.borrow_mut().insert_document_with_mode(
            document.cache_key,
            TreesitterCachedDocument::from_chunked_line_tokens(
                document.line_count,
                document.line_token_chunks,
                document.tree_state,
            ),
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    });
    PreparedSyntaxDocument {
        cache_key: document.cache_key,
    }
}

#[cfg(test)]
pub(super) fn syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<Vec<SyntaxToken>> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().line_tokens(document.cache_key, line_ix))
}

pub(super) fn request_syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<PreparedSyntaxLineTokensRequest> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .request_line_tokens(document.cache_key, line_ix)
    })
}

pub(super) fn request_syntax_tokens_for_prepared_document_line_range(
    document: PreparedSyntaxDocument,
    line_range: Range<usize>,
) -> Option<Vec<PreparedSyntaxLineTokensRequest>> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .request_line_tokens_range(document.cache_key, line_range)
    })
}

pub(super) fn drain_completed_prepared_syntax_chunk_builds() -> usize {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().drain_completed_chunk_builds())
}

pub(super) fn has_pending_prepared_syntax_chunk_builds() -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().has_pending_chunk_requests())
}

pub(super) fn drain_completed_prepared_syntax_chunk_builds_for_document(
    document: PreparedSyntaxDocument,
) -> usize {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .drain_completed_chunk_builds_for_cache_key(document.cache_key)
    })
}

pub(super) fn has_pending_prepared_syntax_chunk_builds_for_document(
    document: PreparedSyntaxDocument,
) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow()
            .has_pending_chunk_requests_for_cache_key(document.cache_key)
    })
}

fn prepared_document_tree_state(
    document: PreparedSyntaxDocument,
) -> Option<PreparedSyntaxTreeState> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().tree_state(document.cache_key))
}

pub(super) fn prepared_document_reparse_seed(
    document: PreparedSyntaxDocument,
) -> Option<PreparedSyntaxReparseSeed> {
    prepared_document_tree_state(document).map(|tree_state| PreparedSyntaxReparseSeed {
        document,
        tree_state,
    })
}

#[cfg(test)]
pub(super) fn prepared_document_parse_mode(
    document: PreparedSyntaxDocument,
) -> Option<TreesitterParseReuseMode> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state(document.cache_key)
            .map(|state| state.parse_mode)
    })
}

#[cfg(test)]
pub(super) fn prepared_document_source_version(document: PreparedSyntaxDocument) -> Option<u64> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state(document.cache_key)
            .map(|state| state.source_version)
    })
}

#[cfg(feature = "benchmarks")]
pub(super) fn benchmark_cache_replacement_drop_step(
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
    defer_drop: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let payloads = benchmark_line_tokens_payload_batch(lines, tokens_per_line, replacements, 0);
    let drop_mode = if defer_drop {
        SyntaxCacheDropMode::DeferredWhenLarge
    } else {
        SyntaxCacheDropMode::InlineWhenLarge
    };
    let mut cache = TreesitterDocumentCache::new();
    let mut h = FxHasher::default();
    for (nonce, line_tokens) in payloads.into_iter().enumerate() {
        cache.insert_document_with_mode(
            PreparedSyntaxCacheKey {
                language: DiffSyntaxLanguage::Rust,
                doc_hash: 0,
            },
            TreesitterCachedDocument::from_line_tokens(line_tokens, None),
            drop_mode,
        );
        cache.by_cache_key.len().hash(&mut h);
        nonce.hash(&mut h);
    }
    h.finish()
}

#[cfg(feature = "benchmarks")]
pub(super) fn benchmark_drop_payload_timed_step(
    lines: usize,
    tokens_per_line: usize,
    seed: usize,
    defer_drop: bool,
) -> Duration {
    let payload = benchmark_line_tokens_payload(lines.max(1), tokens_per_line.max(1), seed);
    let arc_payload = share_recent_line_token_arcs(payload);
    let estimated_bytes = estimated_line_tokens_allocation_bytes(&arc_payload);
    let drop_mode = if defer_drop {
        SyntaxCacheDropMode::DeferredWhenLarge
    } else {
        SyntaxCacheDropMode::InlineWhenLarge
    };
    let start = std::time::Instant::now();
    drop_line_tokens_with_mode(
        SyntaxCacheDropPayload::new(arc_payload, estimated_bytes),
        drop_mode,
    );
    start.elapsed()
}

#[cfg(feature = "benchmarks")]
fn benchmark_line_tokens_payload_batch(
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
    seed: usize,
) -> Vec<Vec<Vec<SyntaxToken>>> {
    let lines = lines.max(1);
    let tokens_per_line = tokens_per_line.max(1);
    let replacements = replacements.max(1);
    let mut payloads = Vec::with_capacity(replacements);
    for nonce in 0..replacements {
        payloads.push(benchmark_line_tokens_payload(
            lines,
            tokens_per_line,
            seed.wrapping_add(nonce),
        ));
    }
    payloads
}

#[cfg(any(test, feature = "benchmarks"))]
fn benchmark_line_tokens_payload(
    lines: usize,
    tokens_per_line: usize,
    nonce: usize,
) -> Vec<Vec<SyntaxToken>> {
    let mut payload = Vec::with_capacity(lines);
    for line_ix in 0..lines {
        let mut line = Vec::with_capacity(tokens_per_line);
        for token_ix in 0..tokens_per_line {
            let start = token_ix.saturating_mul(2);
            let kind = if (line_ix.wrapping_add(nonce).wrapping_add(token_ix) & 1) == 0 {
                SyntaxTokenKind::Keyword
            } else {
                SyntaxTokenKind::String
            };
            line.push(SyntaxToken {
                range: start..start.saturating_add(1),
                kind,
            });
        }
        payload.push(line);
    }
    payload
}

/// Core parsing logic shared by both foreground (cache-inserting) and background (data-returning)
/// document preparation paths.
fn parse_treesitter_document_core(
    request: &TreesitterDocumentParseRequest,
    foreground_timeout: Option<Duration>,
    old_document: Option<PreparedSyntaxDocument>,
    reparse_plan: Option<&TreesitterReparsePlan>,
) -> Option<PreparedSyntaxDocumentData> {
    let incremental_seed = match reparse_plan {
        Some(TreesitterReparsePlan::Changed {
            incremental_seed, ..
        }) => incremental_seed.as_ref(),
        _ => None,
    };

    #[cfg(test)]
    {
        let used_old_document_without_incremental = incremental_reparse_enabled()
            && matches!(
                reparse_plan,
                Some(TreesitterReparsePlan::Changed {
                    incremental_seed: None,
                    ..
                })
            );
        if incremental_seed.is_some() {
            TS_INCREMENTAL_PARSE_COUNT.with(|count| count.set(count.get().saturating_add(1)));
        } else if used_old_document_without_incremental {
            TS_INCREMENTAL_FALLBACK_COUNT.with(|count| count.set(count.get().saturating_add(1)));
        }
    }

    let old_tree_for_parse = incremental_seed.as_ref().map(|seed| &seed.tree);
    let tree = with_ts_parser_parse_result(&request.ts_language, |parser| {
        parse_treesitter_tree(
            parser,
            request.input.text.as_bytes(),
            old_tree_for_parse,
            foreground_timeout,
        )
    })?;

    #[cfg(test)]
    let parse_mode = if incremental_seed.is_some() {
        TreesitterParseReuseMode::Incremental
    } else {
        TreesitterParseReuseMode::Full
    };
    let source_version = incremental_seed.map(|seed| seed.next_version).unwrap_or(1);
    let reused_prefix_chunks = match (old_document, reparse_plan) {
        (
            Some(document),
            Some(TreesitterReparsePlan::Changed {
                reusable_prefix_chunk_count,
                ..
            }),
        ) if *reusable_prefix_chunk_count > 0 => TS_DOCUMENT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .clone_prefix_line_token_chunks(document.cache_key, *reusable_prefix_chunk_count)
        }),
        _ => HashMap::default(),
    };

    Some(PreparedSyntaxDocumentData {
        cache_key: request.cache_key,
        line_count: request.input.line_starts.len(),
        line_token_chunks: reused_prefix_chunks,
        tree_state: Some(PreparedSyntaxTreeState {
            language: request.language,
            text: request.input.text.clone(),
            line_starts: request.input.line_starts.clone(),
            source_hash: request.cache_key.doc_hash,
            source_version,
            tree,
            #[cfg(test)]
            parse_mode,
        }),
    })
}

fn prepare_treesitter_document_request_after_cache_lookup(
    request: TreesitterDocumentParseRequest,
    foreground_budget: Option<DiffSyntaxBudget>,
    old_document: Option<PreparedSyntaxDocument>,
    has_edit_hint: bool,
    reparse_plan: Option<TreesitterReparsePlan>,
) -> PrepareTreesitterDocumentResult {
    if foreground_budget.is_some_and(|budget| budget.foreground_parse.is_zero()) {
        return PrepareTreesitterDocumentResult::TimedOut;
    }
    if foreground_budget.is_some_and(|budget| {
        should_skip_budgeted_foreground_parse(
            &request,
            budget,
            old_document.is_some(),
            has_edit_hint,
        )
    }) {
        return PrepareTreesitterDocumentResult::TimedOut;
    }

    let Some(data) = parse_treesitter_document_core(
        &request,
        foreground_budget.map(|b| b.foreground_parse),
        old_document,
        reparse_plan.as_ref(),
    ) else {
        return if foreground_budget.is_some() {
            PrepareTreesitterDocumentResult::TimedOut
        } else {
            PrepareTreesitterDocumentResult::Unsupported
        };
    };

    store_shared_prepared_document_seed(&data);
    TS_DOCUMENT_CACHE.with(|cache| {
        cache.borrow_mut().insert_document_with_mode(
            data.cache_key,
            TreesitterCachedDocument::from_chunked_line_tokens(
                data.line_count,
                data.line_token_chunks,
                data.tree_state,
            ),
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    });

    PrepareTreesitterDocumentResult::Ready(PreparedSyntaxDocument {
        cache_key: request.cache_key,
    })
}

fn prepare_treesitter_document_data_request_impl(
    request: TreesitterDocumentParseRequest,
    old_document: Option<PreparedSyntaxDocument>,
    reparse_plan: Option<TreesitterReparsePlan>,
) -> Option<PreparedSyntaxDocumentData> {
    if matches!(reparse_plan, Some(TreesitterReparsePlan::Unchanged))
        && let Some(document) = old_document
    {
        let line_count = request.input.line_starts.len();
        if let Some(cached) = TS_DOCUMENT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .prepared_document_data(document.cache_key, line_count)
        }) {
            return Some(cached);
        }
    }
    if let Some(cached) = TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .prepared_document_data(request.cache_key, request.input.line_starts.len())
    }) {
        return Some(cached);
    }

    parse_treesitter_document_core(&request, None, old_document, reparse_plan.as_ref())
}

fn should_skip_budgeted_foreground_parse(
    request: &TreesitterDocumentParseRequest,
    budget: DiffSyntaxBudget,
    has_old_document: bool,
    has_edit_hint: bool,
) -> bool {
    if budget.foreground_parse > DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST {
        return false;
    }
    if has_old_document || has_edit_hint {
        return false;
    }

    request.input.text.len() >= DIFF_SYNTAX_FOREGROUND_SKIP_TEXT_BYTES
        || request.input.line_starts.len() >= DIFF_SYNTAX_FOREGROUND_SKIP_LINE_COUNT
}

fn treesitter_document_parse_request_from_input(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    input: TreesitterDocumentInput,
) -> Option<TreesitterDocumentParseRequest> {
    treesitter_document_parse_request_from_input_with_reuse(language, mode, input, None, None)
}

fn treesitter_document_parse_request_from_input_with_reuse(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    input: TreesitterDocumentInput,
    old_tree_state: Option<&PreparedSyntaxTreeState>,
    reparse_plan: Option<&TreesitterReparsePlan>,
) -> Option<TreesitterDocumentParseRequest> {
    if mode != DiffSyntaxMode::Auto {
        return None;
    }
    if matches!(language, DiffSyntaxLanguage::Markdown) {
        return None;
    }

    let spec = tree_sitter_highlight_spec(language)?;
    let cache_key = match (old_tree_state, reparse_plan) {
        (Some(previous), Some(TreesitterReparsePlan::Changed { edit_ranges, .. })) => {
            treesitter_document_cache_key_for_reparse_plan(language, previous, &input, edit_ranges)
        }
        _ => treesitter_document_cache_key(language, input.text.as_ref()),
    };

    Some(TreesitterDocumentParseRequest {
        language,
        ts_language: spec.ts_language.clone(),
        input,
        cache_key,
    })
}

fn treesitter_document_parse_request_from_shared_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
) -> Option<TreesitterDocumentParseRequest> {
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    treesitter_document_parse_request_from_input_with_reuse(language, mode, input, None, None)
}

fn treesitter_document_input_from_shared_text(
    text: SharedString,
    line_starts: Arc<[usize]>,
) -> TreesitterDocumentInput {
    if text.is_empty() {
        return TreesitterDocumentInput {
            text: Arc::<str>::from(text),
            line_starts: Arc::default(),
        };
    }

    let normalized_line_starts =
        normalized_treesitter_line_starts(text.as_ref(), line_starts.as_ref());

    if normalized_line_starts.first().copied() != Some(0)
        || normalized_line_starts
            .windows(2)
            .any(|window| window[0] >= window[1])
        || normalized_line_starts.last().copied().unwrap_or(0) > text.len()
    {
        return treesitter_document_input_from_text(text.as_ref());
    }

    TreesitterDocumentInput {
        text: Arc::<str>::from(text),
        line_starts: if normalized_line_starts.len() == line_starts.len() {
            line_starts
        } else {
            Arc::<[usize]>::from(normalized_line_starts)
        },
    }
}

fn normalized_treesitter_line_starts<'a>(text: &str, line_starts: &'a [usize]) -> &'a [usize] {
    if text.as_bytes().ends_with(b"\n") && line_starts.last().copied() == Some(text.len()) {
        return &line_starts[..line_starts.len().saturating_sub(1)];
    }
    line_starts
}

fn treesitter_document_input_from_text(text: &str) -> TreesitterDocumentInput {
    if text.is_empty() {
        return TreesitterDocumentInput {
            text: Arc::<str>::from(""),
            line_starts: Arc::default(),
        };
    }

    let mut line_starts = vec![0usize];
    for (byte_ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(byte_ix.saturating_add(1));
        }
    }
    // If text ends with '\n', remove the phantom line start after the trailing newline.
    if text.as_bytes().ends_with(b"\n") {
        line_starts.pop();
    }

    TreesitterDocumentInput {
        text: Arc::<str>::from(text),
        line_starts: Arc::<[usize]>::from(line_starts),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TreesitterByteEditRange {
    start_byte: usize,
    old_end_byte: usize,
    new_end_byte: usize,
}

#[derive(Clone, Debug)]
struct TreesitterIncrementalSeed {
    tree: tree_sitter::Tree,
    next_version: u64,
}

#[derive(Clone, Debug)]
enum TreesitterReparsePlan {
    Unchanged,
    Changed {
        edit_ranges: Vec<TreesitterByteEditRange>,
        incremental_seed: Option<TreesitterIncrementalSeed>,
        reusable_prefix_chunk_count: usize,
    },
}

fn build_treesitter_reparse_plan(
    previous: &PreparedSyntaxTreeState,
    language: DiffSyntaxLanguage,
    input: &TreesitterDocumentInput,
    edit_hint: Option<&DiffSyntaxEdit>,
) -> Option<TreesitterReparsePlan> {
    if previous.language != language {
        return None;
    }

    let old_input = previous.text.as_bytes();
    let new_input = input.text.as_bytes();
    let edit_ranges = edit_hint
        .and_then(|hint| {
            treesitter_byte_edit_range_from_hint(hint, old_input.len(), new_input.len())
        })
        .map(|range| vec![range])
        .unwrap_or_else(|| compute_incremental_edit_ranges(old_input, new_input));
    if edit_ranges.is_empty() {
        return Some(TreesitterReparsePlan::Unchanged);
    }
    let reusable_prefix_chunk_count =
        reusable_prefix_chunk_count(&previous.line_starts, old_input, &edit_ranges);

    let incremental_enabled = incremental_reparse_enabled();
    let should_attempt_incremental = incremental_enabled
        && (!incremental_reparse_should_fallback(&edit_ranges, old_input.len(), new_input.len())
            || incremental_reparse_should_try_large_late_edit(
                &edit_ranges,
                old_input.len(),
                new_input.len(),
            ));
    let incremental_seed = if should_attempt_incremental {
        let new_line_starts = input.line_starts.as_ref();
        let mut tree = previous.tree.clone();
        for edit_range in &edit_ranges {
            let input_edit = tree_sitter::InputEdit {
                start_byte: edit_range.start_byte,
                old_end_byte: edit_range.old_end_byte,
                new_end_byte: edit_range.new_end_byte,
                start_position: treesitter_point_for_byte(
                    &previous.line_starts,
                    old_input,
                    edit_range.start_byte,
                ),
                old_end_position: treesitter_point_for_byte(
                    &previous.line_starts,
                    old_input,
                    edit_range.old_end_byte,
                ),
                new_end_position: treesitter_point_for_byte(
                    new_line_starts,
                    new_input,
                    edit_range.new_end_byte,
                ),
            };
            tree.edit(&input_edit);
        }

        Some(TreesitterIncrementalSeed {
            tree,
            next_version: previous.source_version.saturating_add(1),
        })
    } else {
        None
    };

    Some(TreesitterReparsePlan::Changed {
        edit_ranges,
        incremental_seed,
        reusable_prefix_chunk_count,
    })
}

fn treesitter_document_cache_key_for_reparse_plan(
    language: DiffSyntaxLanguage,
    previous: &PreparedSyntaxTreeState,
    input: &TreesitterDocumentInput,
    edit_ranges: &[TreesitterByteEditRange],
) -> PreparedSyntaxCacheKey {
    use std::hash::{Hash, Hasher};

    let old_input = previous.text.as_bytes();
    let new_input = input.text.as_bytes();
    let mut hasher = FxHasher::default();
    previous.source_hash.hash(&mut hasher);
    input.text.len().hash(&mut hasher);
    input.line_starts.len().hash(&mut hasher);
    edit_ranges.len().hash(&mut hasher);
    for edit in edit_ranges {
        edit.start_byte.hash(&mut hasher);
        edit.old_end_byte.hash(&mut hasher);
        edit.new_end_byte.hash(&mut hasher);
        old_input[edit.start_byte..edit.old_end_byte].hash(&mut hasher);
        new_input[edit.start_byte..edit.new_end_byte].hash(&mut hasher);
    }
    PreparedSyntaxCacheKey {
        language,
        doc_hash: hasher.finish(),
    }
}

fn reusable_prefix_chunk_count(
    old_line_starts: &[usize],
    old_input: &[u8],
    edit_ranges: &[TreesitterByteEditRange],
) -> usize {
    let Some(first_changed_byte) = edit_ranges.iter().map(|edit| edit.start_byte).min() else {
        return 0;
    };
    let first_changed_line =
        treesitter_point_for_byte(old_line_starts, old_input, first_changed_byte).row;
    first_changed_line / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS
}

impl TreesitterDocumentCache {
    fn clone_prefix_line_token_chunks(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        chunk_limit: usize,
    ) -> HashMap<usize, Vec<Arc<[SyntaxToken]>>> {
        if chunk_limit == 0 {
            return HashMap::default();
        }
        let reused: HashMap<usize, Vec<Arc<[SyntaxToken]>>> = self
            .by_cache_key
            .get(&cache_key)
            .map(|document| {
                document
                    .line_token_chunks
                    .iter()
                    .filter(|&(&chunk_ix, chunk)| (chunk_ix < chunk_limit)).map(|(&chunk_ix, chunk)| (chunk_ix, chunk.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if !reused.is_empty() {
            self.touch_key(cache_key);
        }
        reused
    }
}

fn treesitter_byte_edit_range_from_hint(
    edit_hint: &DiffSyntaxEdit,
    old_len: usize,
    new_len: usize,
) -> Option<TreesitterByteEditRange> {
    if edit_hint.old_range.start != edit_hint.new_range.start
        || edit_hint.old_range.start > edit_hint.old_range.end
        || edit_hint.new_range.start > edit_hint.new_range.end
        || edit_hint.old_range.end > old_len
        || edit_hint.new_range.end > new_len
    {
        return None;
    }

    Some(TreesitterByteEditRange {
        start_byte: edit_hint.old_range.start,
        old_end_byte: edit_hint.old_range.end,
        new_end_byte: edit_hint.new_range.end,
    })
}

fn compute_incremental_edit_ranges(old: &[u8], new: &[u8]) -> Vec<TreesitterByteEditRange> {
    if old == new {
        return Vec::new();
    }

    let mut prefix = 0usize;
    let max_prefix = old.len().min(new.len());
    while prefix < max_prefix && old[prefix] == new[prefix] {
        prefix += 1;
    }

    let mut old_suffix_start = old.len();
    let mut new_suffix_start = new.len();
    while old_suffix_start > prefix
        && new_suffix_start > prefix
        && old[old_suffix_start - 1] == new[new_suffix_start - 1]
    {
        old_suffix_start -= 1;
        new_suffix_start -= 1;
    }

    vec![TreesitterByteEditRange {
        start_byte: prefix,
        old_end_byte: old_suffix_start,
        new_end_byte: new_suffix_start,
    }]
}

fn incremental_reparse_should_fallback(
    edits: &[TreesitterByteEditRange],
    old_len: usize,
    new_len: usize,
) -> bool {
    let changed_bytes = incremental_reparse_changed_bytes(edits);
    if changed_bytes == 0 {
        return false;
    }
    if changed_bytes > TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES {
        return true;
    }

    let baseline = old_len.max(new_len).max(1);
    changed_bytes.saturating_mul(100)
        > baseline.saturating_mul(TS_INCREMENTAL_REPARSE_MAX_CHANGED_PERCENT)
}

fn incremental_reparse_changed_bytes(edits: &[TreesitterByteEditRange]) -> usize {
    edits.iter().fold(0usize, |acc, edit| {
        let old_delta = edit.old_end_byte.saturating_sub(edit.start_byte);
        let new_delta = edit.new_end_byte.saturating_sub(edit.start_byte);
        acc.saturating_add(old_delta.max(new_delta))
    })
}

fn incremental_reparse_should_try_large_late_edit(
    edits: &[TreesitterByteEditRange],
    old_len: usize,
    new_len: usize,
) -> bool {
    let [edit] = edits else {
        return false;
    };
    if edit.start_byte < TS_INCREMENTAL_REPARSE_LATE_EDIT_MIN_PREFIX_BYTES {
        return false;
    }

    let changed_bytes = incremental_reparse_changed_bytes(edits);
    if changed_bytes == 0 || changed_bytes > TS_INCREMENTAL_REPARSE_LATE_EDIT_MAX_CHANGED_BYTES {
        return false;
    }

    let baseline = old_len.max(new_len).max(1);
    changed_bytes.saturating_mul(100)
        <= baseline.saturating_mul(TS_INCREMENTAL_REPARSE_LATE_EDIT_MAX_CHANGED_PERCENT)
}

fn treesitter_point_for_byte(
    line_starts: &[usize],
    input: &[u8],
    byte_offset: usize,
) -> tree_sitter::Point {
    let input_len = input.len();
    let byte_offset = byte_offset.min(input_len);
    if line_starts.is_empty() {
        return tree_sitter::Point::new(0, byte_offset);
    }
    if byte_offset == input_len && input.last().copied() == Some(b'\n') {
        // For newline-terminated inputs, EOF is the start of a trailing empty row.
        return tree_sitter::Point::new(line_starts.len(), 0);
    }

    let line_ix = line_ix_for_byte(line_starts, byte_offset);
    let line_start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or_default()
        .min(byte_offset);
    tree_sitter::Point::new(line_ix, byte_offset.saturating_sub(line_start))
}

fn parse_treesitter_tree(
    parser: &mut tree_sitter::Parser,
    input: &[u8],
    old_tree: Option<&tree_sitter::Tree>,
    foreground_parse_budget: Option<Duration>,
) -> Option<tree_sitter::Tree> {
    let Some(foreground_parse_budget) = foreground_parse_budget else {
        return parser.parse(input, old_tree);
    };

    let start = std::time::Instant::now();
    let mut read_input = |byte_offset: usize, _position: tree_sitter::Point| -> &[u8] {
        if byte_offset < input.len() {
            &input[byte_offset..]
        } else {
            &[]
        }
    };
    let mut progress = |_state: &tree_sitter::ParseState| {
        if start.elapsed() >= foreground_parse_budget {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    };
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    parser.parse_with_options(&mut read_input, old_tree, Some(options))
}

const MAX_TREESITTER_LINE_BYTES: usize = 512;

fn should_use_treesitter_for_line(text: &str) -> bool {
    text.len() <= MAX_TREESITTER_LINE_BYTES
}

/// Returns `true` when the heuristic tokenizer is guaranteed to produce
/// results identical to tree-sitter for this line, making the expensive
/// per-line tree-sitter parse unnecessary. Currently covers:
///
/// - Whitespace-only lines (no tokens from either)
/// - Lines whose first non-whitespace content is a line comment prefix
///   (both tree-sitter and the heuristic produce a single Comment token
///   spanning the rest of the line)
fn is_heuristic_sufficient_for_line(text: &str, language: DiffSyntaxLanguage) -> bool {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    let config = heuristic_comment_config(language);
    if let Some(prefix) = config.line_comment
        && trimmed.starts_with(prefix)
    {
        return true;
    }
    if config.hash_comment && trimmed.starts_with('#') {
        return true;
    }
    if config.visual_basic_line_comment
        && (trimmed.starts_with('\'')
            || trimmed
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("rem ")))
    {
        return true;
    }
    false
}

struct TreesitterHighlightSpec {
    ts_language: tree_sitter::Language,
    query: tree_sitter::Query,
    capture_kinds: Vec<Option<SyntaxTokenKind>>,
    injection_query: Option<tree_sitter::Query>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreesitterQueryPass {
    byte_range: Range<usize>,
    containing_byte_range: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TreesitterInjectionMatch {
    language: DiffSyntaxLanguage,
    byte_start: usize,
    byte_end: usize,
    /// Hash of the injection content bytes. This ensures the cache is not
    /// confused when different parent documents happen to produce injection
    /// regions at the same byte offsets.
    content_hash: u64,
}

struct CachedInjectionTokens {
    /// Full tokenized lines in injection-local coordinates (all lines of the injection).
    all_line_tokens: Vec<Vec<SyntaxToken>>,
    /// Line starts for the injection text, used for coordinate remapping.
    injection_line_starts: Vec<usize>,
    /// First line in the parent document that this injection starts on.
    injection_start_line_ix: usize,
    /// Monotonic access counter for LRU eviction.
    last_access: u64,
}

#[derive(Clone, Copy)]
struct TreesitterQueryAsset {
    highlights: &'static str,
    injections: Option<&'static str>,
}

impl TreesitterQueryAsset {
    const fn highlights(source: &'static str) -> Self {
        Self {
            highlights: source,
            injections: None,
        }
    }

    const fn with_injections(highlights: &'static str, injections: &'static str) -> Self {
        Self {
            highlights,
            injections: Some(injections),
        }
    }
}

struct DocumentTokenCollectionContext<'a> {
    line_starts: &'a [usize],
    start_line_ix: usize,
    end_line_ix: usize,
    per_line: &'a mut [Vec<SyntaxToken>],
}

fn syntax_tokens_for_line_treesitter(
    text: &str,
    language: DiffSyntaxLanguage,
) -> Option<Vec<SyntaxToken>> {
    let highlight = tree_sitter_highlight_spec(language)?;
    let ts_language = &highlight.ts_language;

    let input_len = text.len();
    let tree = TS_INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.clear();
        input.push_str(text);
        input.push('\n');

        with_ts_parser_parse_result(ts_language, |parser| parser.parse(&*input, None))
    })?;

    let mut tokens: Vec<SyntaxToken> = Vec::new();
    let query_succeeded = catch_treesitter_query_panic(|| {
        TS_INPUT.with(|input| {
            let input = input.borrow();
            let query_pass = TreesitterQueryPass {
                byte_range: 0..input.len(),
                containing_byte_range: None,
            };
            TS_CURSOR.with(|cursor| {
                let mut cursor = cursor.borrow_mut();
                configure_query_cursor(&mut cursor, &query_pass, input.len());
                let mut captures =
                    cursor.captures(&highlight.query, tree.root_node(), input.as_bytes());
                tree_sitter::StreamingIterator::advance(&mut captures);
                while let Some((m, capture_ix)) = captures.get() {
                    let Some(capture) = m.captures.get(*capture_ix) else {
                        tree_sitter::StreamingIterator::advance(&mut captures);
                        continue;
                    };

                    let Some(kind) = highlight
                        .capture_kinds
                        .get(capture.index as usize)
                        .copied()
                        .flatten()
                    else {
                        tree_sitter::StreamingIterator::advance(&mut captures);
                        continue;
                    };

                    let mut range = capture.node.byte_range();
                    range.start = range.start.min(input_len);
                    range.end = range.end.min(input_len);
                    if range.start < range.end {
                        tokens.push(SyntaxToken { range, kind });
                    }

                    tree_sitter::StreamingIterator::advance(&mut captures);
                }
            });
        });
    })
    .is_some();
    if !query_succeeded {
        return None;
    }

    Some(normalize_non_overlapping_tokens(tokens))
}

fn treesitter_document_cache_key(
    language: DiffSyntaxLanguage,
    input: &str,
) -> PreparedSyntaxCacheKey {
    #[cfg(test)]
    TS_DOCUMENT_HASH_COUNT.with(|count| count.set(count.get().saturating_add(1)));

    PreparedSyntaxCacheKey {
        language,
        doc_hash: treesitter_text_hash(input),
    }
}

fn prepared_document_source_identity_for_shared_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
    line_count: usize,
) -> Option<PreparedSyntaxSourceIdentity> {
    if mode != DiffSyntaxMode::Auto {
        return None;
    }
    if matches!(language, DiffSyntaxLanguage::Markdown) {
        return None;
    }

    Some(PreparedSyntaxSourceIdentity {
        language,
        text_ptr: text.as_ptr() as usize,
        text_len: text.len(),
        line_count,
    })
}

fn store_pending_parse_request(
    identity: PreparedSyntaxSourceIdentity,
    request: TreesitterDocumentParseRequest,
) {
    TS_PENDING_PARSE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        if let Some(existing) = requests
            .iter_mut()
            .find(|existing| existing.identity == identity)
        {
            existing.request = request;
            return;
        }
        if requests.len() >= TS_PENDING_PARSE_REQUEST_MAX_ENTRIES {
            requests.remove(0);
        }
        requests.push(PendingParseRequest { identity, request });
    });
}

fn clear_pending_parse_request(identity: PreparedSyntaxSourceIdentity) {
    TS_PENDING_PARSE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        if let Some(pos) = requests
            .iter()
            .position(|existing| existing.identity == identity)
        {
            requests.remove(pos);
        }
    });
}

fn take_pending_parse_request_for_shared_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
    line_starts: &[usize],
) -> Option<TreesitterDocumentParseRequest> {
    let normalized_line_starts = normalized_treesitter_line_starts(text, line_starts);
    let identity = prepared_document_source_identity_for_shared_text(
        language,
        mode,
        text,
        normalized_line_starts.len(),
    )?;

    TS_PENDING_PARSE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        let pos = requests
            .iter()
            .position(|existing| existing.identity == identity)?;
        let request = requests.remove(pos).request;
        let text_matches =
            request.input.text.as_ptr() == text.as_ptr() && request.input.text.len() == text.len();
        if text_matches && request.input.line_starts.as_ref() == normalized_line_starts {
            return Some(request);
        }
        None
    })
}

fn treesitter_text_hash(input: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    input.hash(&mut hasher);
    hasher.finish()
}

fn collect_treesitter_document_line_tokens_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
) -> Vec<Vec<SyntaxToken>> {
    if line_starts.is_empty() {
        return Vec::new();
    }
    let end_line_ix = end_line_ix.min(line_starts.len());
    if start_line_ix >= end_line_ix {
        return Vec::new();
    }

    let mut per_line: Vec<Vec<SyntaxToken>> = vec![Vec::new(); end_line_ix - start_line_ix];
    let query_passes = treesitter_document_query_passes_for_line_window(
        line_starts,
        input.len(),
        start_line_ix,
        end_line_ix,
    );
    {
        let mut context = DocumentTokenCollectionContext {
            line_starts,
            start_line_ix,
            end_line_ix,
            per_line: &mut per_line,
        };
        for pass in &query_passes {
            collect_query_pass_tokens_for_document(tree, highlight, input, pass, &mut context);
        }
        apply_injection_query_tokens_for_document(tree, highlight, input, &mut context);
    }

    for line_tokens in &mut per_line {
        let normalized = normalize_non_overlapping_tokens(std::mem::take(line_tokens));
        *line_tokens = normalized;
    }
    per_line
}

fn line_ix_for_byte(line_starts: &[usize], byte: usize) -> usize {
    match line_starts.binary_search(&byte) {
        Ok(ix) => ix,
        Err(0) => 0,
        Err(ix) => ix - 1,
    }
}

fn clamp_query_range(range: Range<usize>, input_len: usize) -> Range<usize> {
    let start = range.start.min(input_len);
    let end = range.end.min(input_len).max(start);
    start..end
}

fn configure_query_cursor(
    cursor: &mut tree_sitter::QueryCursor,
    pass: &TreesitterQueryPass,
    input_len: usize,
) {
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    cursor.set_byte_range(clamp_query_range(pass.byte_range.clone(), input_len));
    match &pass.containing_byte_range {
        Some(range) => {
            cursor.set_containing_byte_range(clamp_query_range(range.clone(), input_len));
        }
        None => {
            cursor.set_containing_byte_range(0..usize::MAX);
        }
    }
}

/// Byte offset where the region for line `line_ix` ends (start of next line, or `input_len`).
/// Replaces the old `line_query_end_byte(line_starts[i], line_lengths[i], input_len)`.
fn line_region_end_byte(line_starts: &[usize], input_len: usize, line_ix: usize) -> usize {
    line_starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(input_len)
        .min(input_len)
}

/// Content-end byte offset for line `line_ix` (excludes a trailing `\n` when present).
fn line_content_end_byte(line_starts: &[usize], input: &[u8], line_ix: usize) -> usize {
    let region_end = line_region_end_byte(line_starts, input.len(), line_ix);
    if input.get(region_end.saturating_sub(1)) == Some(&b'\n') {
        region_end.saturating_sub(1)
    } else {
        region_end
    }
}

fn treesitter_document_query_passes_for_line_window(
    line_starts: &[usize],
    input_len: usize,
    start_line_ix: usize,
    end_line_ix: usize,
) -> Vec<TreesitterQueryPass> {
    if input_len == 0 || line_starts.is_empty() {
        return Vec::new();
    }
    let end_line_ix = end_line_ix.min(line_starts.len());
    if start_line_ix >= end_line_ix {
        return Vec::new();
    }

    let window_start_byte = line_starts[start_line_ix].min(input_len);
    let window_end_byte = line_region_end_byte(line_starts, input_len, end_line_ix - 1);
    if window_start_byte >= window_end_byte {
        return Vec::new();
    }

    let window_bytes = window_end_byte.saturating_sub(window_start_byte);

    if window_bytes <= TS_MAX_BYTES_TO_QUERY {
        return vec![TreesitterQueryPass {
            byte_range: window_start_byte..window_end_byte,
            containing_byte_range: None,
        }];
    }

    let mut passes = Vec::new();
    let mut line_ix = start_line_ix;
    while line_ix < end_line_ix {
        let line_start = line_starts[line_ix].min(input_len);
        let line_end = line_region_end_byte(line_starts, input_len, line_ix);
        let line_bytes = line_end.saturating_sub(line_start);

        if line_bytes > TS_MAX_BYTES_TO_QUERY {
            let mut chunk_start = line_start;
            while chunk_start < line_end {
                let chunk_end = chunk_start
                    .saturating_add(TS_MAX_BYTES_TO_QUERY)
                    .min(line_end);
                passes.push(TreesitterQueryPass {
                    byte_range: chunk_start..chunk_end,
                    containing_byte_range: Some(chunk_start..chunk_end),
                });
                chunk_start = chunk_end;
            }
            line_ix = line_ix.saturating_add(1);
            continue;
        }

        let window_start_line = line_ix;
        let window_start = line_start;
        let mut window_end_line = line_ix;
        let mut window_end = line_end;

        while window_end_line + 1 < end_line_ix
            && (window_end_line - window_start_line + 1) < TS_QUERY_MAX_LINES_PER_PASS
        {
            let next_line_ix = window_end_line + 1;
            let next_line_end = line_region_end_byte(line_starts, input_len, next_line_ix);
            let candidate_end = window_end.max(next_line_end);
            let candidate_bytes = candidate_end.saturating_sub(window_start);
            if candidate_bytes > TS_MAX_BYTES_TO_QUERY {
                break;
            }
            window_end = candidate_end;
            window_end_line = next_line_ix;
        }

        passes.push(TreesitterQueryPass {
            byte_range: window_start..window_end,
            containing_byte_range: None,
        });
        line_ix = window_end_line.saturating_add(1);
    }

    if passes.is_empty() {
        return vec![TreesitterQueryPass {
            byte_range: window_start_byte..window_end_byte,
            containing_byte_range: None,
        }];
    }

    passes
}

fn collect_query_pass_tokens_for_document(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    pass: &TreesitterQueryPass,
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    let _ = catch_treesitter_query_panic(|| {
        TS_CURSOR.with(|cursor| {
            let mut cursor = cursor.borrow_mut();
            configure_query_cursor(&mut cursor, pass, input.len());
            let pass_range = clamp_query_range(pass.byte_range.clone(), input.len());
            let mut captures = cursor.captures(&highlight.query, tree.root_node(), input);
            tree_sitter::StreamingIterator::advance(&mut captures);
            while let Some((m, capture_ix)) = captures.get() {
                let Some(capture) = m.captures.get(*capture_ix) else {
                    tree_sitter::StreamingIterator::advance(&mut captures);
                    continue;
                };
                let Some(kind) = highlight
                    .capture_kinds
                    .get(capture.index as usize)
                    .copied()
                    .flatten()
                else {
                    tree_sitter::StreamingIterator::advance(&mut captures);
                    continue;
                };

                let mut byte_range = capture.node.byte_range();
                byte_range.start = byte_range.start.min(input.len());
                byte_range.end = byte_range.end.min(input.len());
                byte_range.start = byte_range.start.max(pass_range.start);
                byte_range.end = byte_range.end.min(pass_range.end);
                if byte_range.start >= byte_range.end {
                    tree_sitter::StreamingIterator::advance(&mut captures);
                    continue;
                }

                let mut line_ix = line_ix_for_byte(context.line_starts, byte_range.start);
                if line_ix < context.start_line_ix {
                    line_ix = context.start_line_ix;
                }
                while line_ix < context.end_line_ix && line_ix < context.line_starts.len() {
                    let line_start = context.line_starts[line_ix];
                    let line_end = line_content_end_byte(context.line_starts, input, line_ix);
                    let token_start = byte_range.start.max(line_start);
                    let token_end = byte_range.end.min(line_end);
                    if token_start < token_end {
                        context.per_line[line_ix - context.start_line_ix].push(SyntaxToken {
                            range: (token_start - line_start)..(token_end - line_start),
                            kind,
                        });
                    }
                    if byte_range.end <= line_end {
                        break;
                    }
                    line_ix = line_ix.saturating_add(1);
                }

                tree_sitter::StreamingIterator::advance(&mut captures);
            }
        });
    });
}

struct InjectionDepthGuard(usize);

impl InjectionDepthGuard {
    fn enter() -> Option<Self> {
        let depth = TS_INJECTION_DEPTH.with(|d| d.get());
        if depth >= TS_MAX_INJECTION_DEPTH {
            return None;
        }
        TS_INJECTION_DEPTH.with(|d| d.set(depth + 1));
        Some(Self(depth))
    }
}

impl Drop for InjectionDepthGuard {
    fn drop(&mut self) {
        TS_INJECTION_DEPTH.with(|d| d.set(self.0));
    }
}

fn apply_injection_query_tokens_for_document(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    let Some(_guard) = InjectionDepthGuard::enter() else {
        return;
    };
    let injections = collect_treesitter_injection_matches_for_line_window(
        tree,
        highlight,
        input,
        context.line_starts,
        context.start_line_ix,
        context.end_line_ix,
    );
    for injection in injections {
        let Some(injected_tokens) = collect_injected_tokens_for_parent_line_window(
            input,
            context.line_starts,
            context.start_line_ix,
            context.end_line_ix,
            injection,
        ) else {
            continue;
        };

        subtract_absolute_range_from_document_tokens(
            context.line_starts,
            input,
            context.start_line_ix,
            context.per_line,
            injection.byte_start..injection.byte_end,
        );

        for (parent_line_ix, tokens) in injected_tokens {
            if tokens.is_empty() || parent_line_ix < context.start_line_ix {
                continue;
            }
            let Some(line_tokens) = context
                .per_line
                .get_mut(parent_line_ix.saturating_sub(context.start_line_ix))
            else {
                continue;
            };
            line_tokens.extend(tokens);
        }
    }
}

fn collect_treesitter_injection_matches_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
) -> Vec<TreesitterInjectionMatch> {
    let Some(injection_query) = highlight.injection_query.as_ref() else {
        return Vec::new();
    };
    let Some(injection_content_capture_ix) =
        injection_query.capture_index_for_name("injection.content")
    else {
        return Vec::new();
    };

    let query_passes = treesitter_document_query_passes_for_line_window(
        line_starts,
        input.len(),
        start_line_ix,
        end_line_ix,
    );
    if query_passes.is_empty() {
        return Vec::new();
    }

    let mut seen = HashSet::default();
    let mut injections = Vec::new();
    for pass in &query_passes {
        let _ = catch_treesitter_query_panic(|| {
            TS_CURSOR.with(|cursor| {
                let mut cursor = cursor.borrow_mut();
                configure_query_cursor(&mut cursor, pass, input.len());
                let mut matches = cursor.matches(injection_query, tree.root_node(), input);
                tree_sitter::StreamingIterator::advance(&mut matches);
                while let Some(m) = matches.get() {
                    let Some(language) =
                        injection_language_for_pattern(injection_query, m.pattern_index)
                    else {
                        tree_sitter::StreamingIterator::advance(&mut matches);
                        continue;
                    };
                    for capture in m
                        .captures
                        .iter()
                        .filter(|capture| capture.index == injection_content_capture_ix)
                    {
                        let mut byte_range = capture.node.byte_range();
                        byte_range.start = byte_range.start.min(input.len());
                        byte_range.end = byte_range.end.min(input.len());
                        if byte_range.start >= byte_range.end {
                            continue;
                        }
                        let injection = TreesitterInjectionMatch {
                            language,
                            byte_start: byte_range.start,
                            byte_end: byte_range.end,
                            content_hash: {
                                use std::hash::{Hash, Hasher};
                                let mut h = FxHasher::default();
                                input[byte_range.start..byte_range.end].hash(&mut h);
                                h.finish()
                            },
                        };
                        if seen.insert(injection) {
                            injections.push(injection);
                        }
                    }
                    tree_sitter::StreamingIterator::advance(&mut matches);
                }
            });
        });
    }

    injections.sort_by_key(|injection| (injection.byte_start, injection.byte_end));
    injections
}

fn injection_language_for_pattern(
    query: &tree_sitter::Query,
    pattern_index: usize,
) -> Option<DiffSyntaxLanguage> {
    let language_name = query
        .property_settings(pattern_index)
        .iter()
        .find_map(|setting| {
            matches!(setting.key.as_ref(), "injection.language" | "language")
                .then(|| setting.value.as_deref())
                .flatten()
        })?;
    injection_language_from_name(language_name)
}

fn injection_language_from_name(name: &str) -> Option<DiffSyntaxLanguage> {
    match name {
        "html" => Some(DiffSyntaxLanguage::Html),
        "css" => Some(DiffSyntaxLanguage::Css),
        "rust" => Some(DiffSyntaxLanguage::Rust),
        "python" => Some(DiffSyntaxLanguage::Python),
        "javascript" | "js" => Some(DiffSyntaxLanguage::JavaScript),
        "typescript" | "ts" => Some(DiffSyntaxLanguage::TypeScript),
        "tsx" => Some(DiffSyntaxLanguage::Tsx),
        "go" => Some(DiffSyntaxLanguage::Go),
        "json" => Some(DiffSyntaxLanguage::Json),
        "yaml" | "yml" => Some(DiffSyntaxLanguage::Yaml),
        "bash" | "sh" => Some(DiffSyntaxLanguage::Bash),
        _ => None,
    }
}

fn next_injection_access() -> u64 {
    TS_INJECTION_ACCESS_COUNTER.with(|c| {
        let val = c.get().wrapping_add(1);
        c.set(val);
        val
    })
}

fn ensure_injection_cached(
    input: &[u8],
    line_starts: &[usize],
    injection: TreesitterInjectionMatch,
) -> bool {
    TS_INJECTION_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(&injection) {
            entry.last_access = next_injection_access();
            return true;
        }

        let injection_byte_range =
            injection.byte_start.min(input.len())..injection.byte_end.min(input.len());
        if injection_byte_range.is_empty() {
            return false;
        }
        let Ok(injection_text) = std::str::from_utf8(&input[injection_byte_range.clone()]) else {
            return false;
        };
        if injection_text.is_empty() {
            return false;
        }
        let injection_input = treesitter_document_input_from_text(injection_text);
        if injection_input.line_starts.is_empty() {
            return false;
        }
        let Some(highlight) = tree_sitter_highlight_spec(injection.language) else {
            return false;
        };
        let Some(tree) = with_ts_parser_parse_result(&highlight.ts_language, |parser| {
            parse_treesitter_tree(parser, injection_input.text.as_bytes(), None, None)
        }) else {
            return false;
        };

        let injection_line_count = injection_input.line_starts.len();
        let all_line_tokens = collect_treesitter_document_line_tokens_for_line_window(
            &tree,
            highlight,
            injection_input.text.as_bytes(),
            injection_input.line_starts.as_ref(),
            0,
            injection_line_count,
        );

        let injection_start_line_ix = line_ix_for_byte(line_starts, injection.byte_start);
        let access = next_injection_access();

        let mut cache = cache.borrow_mut();
        if cache.len() >= TS_INJECTION_CACHE_MAX_ENTRIES {
            // Evict the least-recently-used half instead of clearing everything.
            let mut entries: Vec<_> = cache.iter().map(|(k, v)| (*k, v.last_access)).collect();
            entries.sort_unstable_by_key(|(_, a)| *a);
            let evict_count = entries.len() / 2;
            for (key, _) in entries.into_iter().take(evict_count) {
                cache.remove(&key);
            }
        }
        cache.insert(
            injection,
            CachedInjectionTokens {
                all_line_tokens,
                injection_line_starts: injection_input.line_starts.as_ref().to_vec(),
                injection_start_line_ix,
                last_access: access,
            },
        );
        true
    })
}

fn collect_injected_tokens_for_parent_line_window(
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    injection: TreesitterInjectionMatch,
) -> Option<Vec<(usize, Vec<SyntaxToken>)>> {
    if !ensure_injection_cached(input, line_starts, injection) {
        return Some(Vec::new());
    }

    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        let cached = cache.get(&injection)?;

        let injection_end_line_ix = cached
            .injection_start_line_ix
            .saturating_add(cached.all_line_tokens.len());
        let parent_start_line_ix = start_line_ix.max(cached.injection_start_line_ix);
        let parent_end_line_ix = end_line_ix.min(injection_end_line_ix);
        if parent_start_line_ix >= parent_end_line_ix {
            return Some(Vec::new());
        }

        let local_start_line_ix =
            parent_start_line_ix.saturating_sub(cached.injection_start_line_ix);
        let local_end_line_ix = parent_end_line_ix.saturating_sub(cached.injection_start_line_ix);

        let mut mapped_tokens = Vec::with_capacity(local_end_line_ix - local_start_line_ix);
        for local_line_ix in local_start_line_ix..local_end_line_ix {
            let parent_line_ix = cached.injection_start_line_ix.saturating_add(local_line_ix);
            let Some(parent_line_start) = line_starts.get(parent_line_ix).copied() else {
                continue;
            };
            let parent_content_end = line_content_end_byte(line_starts, input, parent_line_ix);
            let parent_line_len = parent_content_end.saturating_sub(parent_line_start);
            let Some(local_line_start) = cached.injection_line_starts.get(local_line_ix).copied()
            else {
                continue;
            };
            let absolute_line_start = injection.byte_start.saturating_add(local_line_start);
            let offset_within_parent = absolute_line_start.saturating_sub(parent_line_start);
            let tokens = cached
                .all_line_tokens
                .get(local_line_ix)
                .cloned()
                .unwrap_or_default();
            let mut remapped = Vec::with_capacity(tokens.len());
            for token in tokens {
                let start = offset_within_parent.saturating_add(token.range.start);
                let end = offset_within_parent
                    .saturating_add(token.range.end)
                    .min(parent_line_len);
                if start >= end || start >= parent_line_len {
                    continue;
                }
                remapped.push(SyntaxToken {
                    range: start..end,
                    kind: token.kind,
                });
            }
            mapped_tokens.push((parent_line_ix, remapped));
        }

        Some(mapped_tokens)
    })
}

fn subtract_absolute_range_from_document_tokens(
    line_starts: &[usize],
    input: &[u8],
    start_line_ix: usize,
    per_line: &mut [Vec<SyntaxToken>],
    absolute_range: Range<usize>,
) {
    if absolute_range.start >= absolute_range.end || per_line.is_empty() {
        return;
    }

    let first_line_ix = line_ix_for_byte(line_starts, absolute_range.start);
    let last_line_ix = line_ix_for_byte(line_starts, absolute_range.end.saturating_sub(1));
    let visible_end_line_ix = start_line_ix.saturating_add(per_line.len());
    let clipped_start_line_ix = first_line_ix.max(start_line_ix);
    let clipped_end_line_ix = last_line_ix.saturating_add(1).min(visible_end_line_ix);
    if clipped_start_line_ix >= clipped_end_line_ix {
        return;
    }

    for line_ix in clipped_start_line_ix..clipped_end_line_ix {
        let Some(line_start) = line_starts.get(line_ix).copied() else {
            continue;
        };
        let content_end = line_content_end_byte(line_starts, input, line_ix);
        let cut_start = absolute_range
            .start
            .max(line_start)
            .saturating_sub(line_start);
        let cut_end = absolute_range
            .end
            .min(content_end)
            .saturating_sub(line_start);
        if cut_start >= cut_end {
            continue;
        }
        let Some(line_tokens) = per_line.get_mut(line_ix.saturating_sub(start_line_ix)) else {
            continue;
        };
        subtract_relative_range_from_line_tokens(line_tokens, cut_start..cut_end);
    }
}

fn subtract_relative_range_from_line_tokens(
    line_tokens: &mut Vec<SyntaxToken>,
    cut_range: Range<usize>,
) {
    if cut_range.start >= cut_range.end || line_tokens.is_empty() {
        return;
    }

    let mut out = Vec::with_capacity(line_tokens.len().saturating_add(1));
    for token in line_tokens.drain(..) {
        if token.range.end <= cut_range.start || token.range.start >= cut_range.end {
            out.push(token);
            continue;
        }
        if token.range.start < cut_range.start {
            out.push(SyntaxToken {
                range: token.range.start..cut_range.start,
                kind: token.kind,
            });
        }
        if token.range.end > cut_range.end {
            out.push(SyntaxToken {
                range: cut_range.end..token.range.end,
                kind: token.kind,
            });
        }
    }
    *line_tokens = out;
}

fn normalize_non_overlapping_tokens(mut tokens: Vec<SyntaxToken>) -> Vec<SyntaxToken> {
    if tokens.len() <= 1 {
        return tokens;
    }

    tokens.sort_unstable_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(a.range.end.cmp(&b.range.end))
    });

    // Compact in-place: ensure non-overlapping tokens so the segment splitter
    // can pick a single style per range.
    // Tree-sitter queries follow "later pattern wins" semantics: when two patterns capture
    // the same node, the more specific pattern (later in the query file) should take priority.
    // Since captures() returns lower pattern indices first, later tokens at the same position
    // should override earlier ones.
    let mut write = 1usize;
    for read in 1..tokens.len() {
        let cur_start = tokens[read].range.start;
        let cur_end = tokens[read].range.end;
        let cur_kind = tokens[read].kind;

        let prev_start = tokens[write - 1].range.start;
        let prev_end = tokens[write - 1].range.end;

        if cur_start < prev_end {
            // Exact same range: later pattern wins (replace previous)
            if cur_start == prev_start && cur_end == prev_end {
                tokens[write - 1].kind = cur_kind;
                continue;
            }
            // Fully contained in previous: skip
            if cur_end <= prev_end {
                continue;
            }
            // Partial overlap: trim start to prev end
            let new_start = prev_end;
            if new_start >= cur_end {
                continue;
            }
            tokens[write] = SyntaxToken {
                range: new_start..cur_end,
                kind: cur_kind,
            };
            write += 1;
        } else {
            tokens[write] = SyntaxToken {
                range: cur_start..cur_end,
                kind: cur_kind,
            };
            write += 1;
        }
    }
    tokens.truncate(write);
    tokens
}

/// Single source of truth for tree-sitter grammar + query asset per language.
/// Returns `None` for languages without a wired tree-sitter grammar.
fn tree_sitter_grammar(
    language: DiffSyntaxLanguage,
) -> Option<(tree_sitter::Language, TreesitterQueryAsset)> {
    match language {
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Html => Some((
            tree_sitter_html::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(HTML_HIGHLIGHTS_QUERY, HTML_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Css => Some((
            tree_sitter_css::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(CSS_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-rust"))]
        DiffSyntaxLanguage::Rust => Some((
            tree_sitter_rust::LANGUAGE.into(),
            TreesitterQueryAsset::with_injections(RUST_HIGHLIGHTS_QUERY, RUST_INJECTIONS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-python"))]
        DiffSyntaxLanguage::Python => Some((
            tree_sitter_python::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_python::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-go"))]
        DiffSyntaxLanguage::Go => Some((
            tree_sitter_go::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_go::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Json => Some((
            tree_sitter_json::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_json::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Yaml => Some((
            tree_sitter_yaml::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_yaml::HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::TypeScript => Some((
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TreesitterQueryAsset::highlights(TYPESCRIPT_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Tsx => Some((
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TreesitterQueryAsset::highlights(TSX_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::JavaScript => Some((
            tree_sitter_javascript::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(JAVASCRIPT_HIGHLIGHTS_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-shell"))]
        DiffSyntaxLanguage::Bash => Some((
            tree_sitter_bash::LANGUAGE.into(),
            TreesitterQueryAsset::highlights(tree_sitter_bash::HIGHLIGHT_QUERY),
        )),
        #[cfg(any(test, feature = "syntax-xml"))]
        DiffSyntaxLanguage::Xml => Some((
            tree_sitter_xml::LANGUAGE_XML.into(),
            TreesitterQueryAsset::highlights(XML_HIGHLIGHTS_QUERY),
        )),
        // Languages without a wired tree-sitter grammar, or grammars gated off
        // by the current feature set, fall back to heuristic-only highlighting.
        _ => None,
    }
}

fn init_highlight_spec(language: DiffSyntaxLanguage) -> TreesitterHighlightSpec {
    let (ts_language, asset) =
        tree_sitter_grammar(language).expect("tree-sitter grammar should exist");
    let query = tree_sitter::Query::new(&ts_language, asset.highlights)
        .expect("highlights.scm should compile");
    let capture_kinds = query
        .capture_names()
        .iter()
        .map(|name| syntax_kind_from_capture_name(name))
        .collect::<Vec<_>>();
    let injection_query = asset.injections.map(|source| {
        tree_sitter::Query::new(&ts_language, source).expect("injections.scm should compile")
    });
    TreesitterHighlightSpec {
        ts_language,
        query,
        capture_kinds,
        injection_query,
    }
}

macro_rules! highlight_spec_entry {
    ($language_variant:ident) => {{
        static SPEC: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
        Some(SPEC.get_or_init(|| init_highlight_spec(DiffSyntaxLanguage::$language_variant)))
    }};
}

fn tree_sitter_highlight_spec(
    language: DiffSyntaxLanguage,
) -> Option<&'static TreesitterHighlightSpec> {
    match language {
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Html => highlight_spec_entry!(Html),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Css => highlight_spec_entry!(Css),
        #[cfg(any(test, feature = "syntax-rust"))]
        DiffSyntaxLanguage::Rust => highlight_spec_entry!(Rust),
        #[cfg(any(test, feature = "syntax-python"))]
        DiffSyntaxLanguage::Python => highlight_spec_entry!(Python),
        #[cfg(any(test, feature = "syntax-go"))]
        DiffSyntaxLanguage::Go => highlight_spec_entry!(Go),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Json => highlight_spec_entry!(Json),
        #[cfg(any(test, feature = "syntax-data"))]
        DiffSyntaxLanguage::Yaml => highlight_spec_entry!(Yaml),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::TypeScript => highlight_spec_entry!(TypeScript),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::Tsx => highlight_spec_entry!(Tsx),
        #[cfg(any(test, feature = "syntax-web"))]
        DiffSyntaxLanguage::JavaScript => highlight_spec_entry!(JavaScript),
        #[cfg(any(test, feature = "syntax-shell"))]
        DiffSyntaxLanguage::Bash => highlight_spec_entry!(Bash),
        #[cfg(any(test, feature = "syntax-xml"))]
        DiffSyntaxLanguage::Xml => highlight_spec_entry!(Xml),
        _ => None,
    }
}

fn syntax_kind_from_capture_name(mut name: &str) -> Option<SyntaxTokenKind> {
    // Try the full dotted capture name first and then progressively trim suffix
    // segments so vendored names like `punctuation.bracket.html` keep their
    // semantic class instead of collapsing all the way to `punctuation`.
    loop {
        if let Some(kind) = syntax_kind_for_capture_name(name) {
            return Some(kind);
        }

        let (prefix, _) = name.rsplit_once('.')?;
        name = prefix;
    }
}

fn syntax_kind_for_capture_name(name: &str) -> Option<SyntaxTokenKind> {
    Some(match name {
        // Comments
        "comment.doc" => SyntaxTokenKind::CommentDoc,
        "comment" => SyntaxTokenKind::Comment,
        // Strings
        "string.escape" => SyntaxTokenKind::StringEscape,
        "string" | "string.special" | "string.regex" | "character" => SyntaxTokenKind::String,
        // Keywords
        "keyword.control" => SyntaxTokenKind::KeywordControl,
        "keyword" | "keyword.declaration" | "keyword.import" | "include" | "preproc" => {
            SyntaxTokenKind::Keyword
        }
        // Numbers & booleans
        "number" => SyntaxTokenKind::Number,
        "boolean" => SyntaxTokenKind::Boolean,
        // Functions
        "function.method" => SyntaxTokenKind::FunctionMethod,
        "function.special" | "function.special.definition" => SyntaxTokenKind::FunctionSpecial,
        "function" | "function.definition" | "constructor" | "method" => SyntaxTokenKind::Function,
        // Types
        "type.builtin" => SyntaxTokenKind::TypeBuiltin,
        "type.interface" => SyntaxTokenKind::TypeInterface,
        "type" | "type.class" => SyntaxTokenKind::Type,
        // Variables - general `@variable` renders as plain text (no color) to avoid
        // "everything is highlighted" noise. Sub-captures get distinct treatment.
        "variable.parameter" => SyntaxTokenKind::VariableParameter,
        "variable.special" => SyntaxTokenKind::VariableSpecial,
        "variable" => SyntaxTokenKind::Variable,
        // Properties
        "property" | "field" => SyntaxTokenKind::Property,
        // Tags (HTML/JSX)
        "tag" | "tag.doctype" => SyntaxTokenKind::Tag,
        // Attributes
        "attribute" | "attribute.jsx" => SyntaxTokenKind::Attribute,
        // Constants
        "constant" | "constant.builtin" => SyntaxTokenKind::Constant,
        // Operators
        "operator" => SyntaxTokenKind::Operator,
        // Punctuation
        "punctuation.bracket" => SyntaxTokenKind::PunctuationBracket,
        "punctuation.delimiter" => SyntaxTokenKind::PunctuationDelimiter,
        "punctuation" | "punctuation.special" => SyntaxTokenKind::Punctuation,
        // Lifetime (Rust)
        "lifetime" => SyntaxTokenKind::Lifetime,
        // Labels (goto, DTD notation names)
        "label" => SyntaxTokenKind::Variable,
        // Markup (XML text content, CDATA, URIs)
        "markup.link" => SyntaxTokenKind::String,
        "markup.raw" => SyntaxTokenKind::String,
        "markup.heading" => SyntaxTokenKind::Keyword,
        "markup" => SyntaxTokenKind::Variable,
        // Selectors/namespaces map to Type for CSS/XML
        "namespace" | "selector" => SyntaxTokenKind::Type,
        // Skip `@none`, `@embedded`, `@text.*` and other non-semantic captures
        _ => return None,
    })
}

#[derive(Clone, Copy)]
struct HeuristicBlockCommentSpec {
    start: &'static str,
    end: &'static str,
}

#[derive(Clone, Copy)]
struct HeuristicCommentConfig {
    line_comment: Option<&'static str>,
    hash_comment: bool,
    block_comment: Option<HeuristicBlockCommentSpec>,
    visual_basic_line_comment: bool,
}

const HEURISTIC_HTML_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "<!--",
    end: "-->",
};
const HEURISTIC_FSHARP_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "(*",
    end: "*)",
};
const HEURISTIC_LUA_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "--[[",
    end: "]]",
};
const HEURISTIC_C_BLOCK_COMMENT: HeuristicBlockCommentSpec = HeuristicBlockCommentSpec {
    start: "/*",
    end: "*/",
};

fn heuristic_comment_config(language: DiffSyntaxLanguage) -> HeuristicCommentConfig {
    match language {
        DiffSyntaxLanguage::Html | DiffSyntaxLanguage::Xml => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: false,
            block_comment: Some(HEURISTIC_HTML_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::FSharp => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: false,
            block_comment: Some(HEURISTIC_FSHARP_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Lua => HeuristicCommentConfig {
            line_comment: Some("--"),
            hash_comment: false,
            block_comment: Some(HEURISTIC_LUA_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Python
        | DiffSyntaxLanguage::Toml
        | DiffSyntaxLanguage::Yaml
        | DiffSyntaxLanguage::Bash
        | DiffSyntaxLanguage::Makefile
        | DiffSyntaxLanguage::Ruby => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: true,
            block_comment: None,
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Sql => HeuristicCommentConfig {
            line_comment: Some("--"),
            hash_comment: false,
            block_comment: Some(HEURISTIC_C_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Rust
        | DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx
        | DiffSyntaxLanguage::Go
        | DiffSyntaxLanguage::C
        | DiffSyntaxLanguage::Cpp
        | DiffSyntaxLanguage::CSharp
        | DiffSyntaxLanguage::Java
        | DiffSyntaxLanguage::Kotlin
        | DiffSyntaxLanguage::Zig
        | DiffSyntaxLanguage::Bicep => HeuristicCommentConfig {
            line_comment: Some("//"),
            hash_comment: false,
            block_comment: Some(HEURISTIC_C_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::Hcl | DiffSyntaxLanguage::Php => HeuristicCommentConfig {
            line_comment: Some("//"),
            hash_comment: true,
            block_comment: Some(HEURISTIC_C_BLOCK_COMMENT),
            visual_basic_line_comment: false,
        },
        DiffSyntaxLanguage::VisualBasic => HeuristicCommentConfig {
            line_comment: None,
            hash_comment: false,
            block_comment: None,
            visual_basic_line_comment: true,
        },
        DiffSyntaxLanguage::Markdown | DiffSyntaxLanguage::Css | DiffSyntaxLanguage::Json => {
            HeuristicCommentConfig {
                line_comment: None,
                hash_comment: false,
                block_comment: None,
                visual_basic_line_comment: false,
            }
        }
    }
}

fn heuristic_comment_range(
    text: &str,
    start: usize,
    config: HeuristicCommentConfig,
) -> Option<std::ops::Range<usize>> {
    let rest = &text[start..];

    if let Some(block) = config.block_comment
        && rest.starts_with(block.start)
    {
        let end = rest
            .find(block.end)
            .map(|ix| start + ix + block.end.len())
            .unwrap_or(text.len());
        return Some(start..end);
    }

    if let Some(prefix) = config.line_comment
        && rest.starts_with(prefix)
    {
        return Some(start..text.len());
    }

    if config.visual_basic_line_comment
        && (rest.starts_with('\'')
            || rest
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rem ")))
    {
        return Some(start..text.len());
    }

    if config.hash_comment && rest.starts_with('#') {
        return Some(start..text.len());
    }

    None
}

fn heuristic_string_end(text: &str, start: usize, quote: char) -> usize {
    let len = text.len();
    let mut i = start + quote.len_utf8();
    let mut escaped = false;

    while i < len {
        let Some(next) = text[i..].chars().next() else {
            break;
        };
        let next_len = next.len_utf8();
        if escaped {
            escaped = false;
            i += next_len;
            continue;
        }
        if next == '\\' {
            escaped = true;
            i += next_len;
            continue;
        }
        if next == quote {
            i += next_len;
            break;
        }
        i += next_len;
    }

    i.min(len)
}

fn heuristic_allows_backtick_strings(language: DiffSyntaxLanguage) -> bool {
    matches!(
        language,
        DiffSyntaxLanguage::JavaScript
            | DiffSyntaxLanguage::TypeScript
            | DiffSyntaxLanguage::Tsx
            | DiffSyntaxLanguage::Go
            | DiffSyntaxLanguage::Bash
            | DiffSyntaxLanguage::Sql
    )
}

fn yaml_heuristic_key_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
    {
        end += 1;
    }
    (end > start && bytes.get(end) == Some(&b':')).then_some(end)
}

fn yaml_heuristic_key_context(bytes: &[u8], key_start: usize) -> bool {
    let mut seen_dash = false;
    for &byte in &bytes[..key_start] {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if !seen_dash && byte == b'-' {
            seen_dash = true;
            continue;
        }
        return false;
    }
    true
}

fn yaml_heuristic_value_start(bytes: &[u8], colon_ix: usize) -> usize {
    let mut start = colon_ix.saturating_add(1);
    while bytes
        .get(start)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        start += 1;
    }
    start
}

fn yaml_heuristic_value_end(bytes: &[u8], start: usize) -> usize {
    if start >= bytes.len() {
        return start;
    }

    let mut end = bytes.len();
    while end > start && bytes[end.saturating_sub(1)].is_ascii_whitespace() {
        end = end.saturating_sub(1);
    }

    let mut ix = start;
    while ix < end {
        if bytes[ix] == b'#' && (ix == start || bytes[ix.saturating_sub(1)].is_ascii_whitespace()) {
            let mut comment_start = ix;
            while comment_start > start
                && bytes[comment_start.saturating_sub(1)].is_ascii_whitespace()
            {
                comment_start = comment_start.saturating_sub(1);
            }
            return comment_start;
        }
        ix += 1;
    }

    end
}

fn yaml_heuristic_is_plain_boolean(text: &str) -> bool {
    matches!(
        text,
        "true" | "false" | "yes" | "no" | "on" | "off" | "True" | "False" | "TRUE" | "FALSE"
    )
}

fn yaml_heuristic_is_plain_null(text: &str) -> bool {
    matches!(text, "null" | "Null" | "NULL" | "~")
}

fn yaml_heuristic_is_plain_number(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }

    let mut ix = 0usize;
    if matches!(bytes[0], b'+' | b'-') {
        ix += 1;
    }
    let mut saw_digit = false;
    while ix < bytes.len() {
        match bytes[ix] {
            b'0'..=b'9' => {
                saw_digit = true;
                ix += 1;
            }
            b'_' | b'.' => {
                ix += 1;
            }
            b'e' | b'E' => {
                ix += 1;
                if matches!(bytes.get(ix), Some(b'+') | Some(b'-')) {
                    ix += 1;
                }
            }
            _ => return false,
        }
    }
    saw_digit
}

fn yaml_heuristic_emit_mapping_value_tokens(
    text: &str,
    bytes: &[u8],
    colon_ix: usize,
    allow_backtick_strings: bool,
    tokens: &mut Vec<SyntaxToken>,
) -> usize {
    let value_start = yaml_heuristic_value_start(bytes, colon_ix);
    if value_start >= bytes.len() {
        return colon_ix.saturating_add(1);
    }

    let value_end = yaml_heuristic_value_end(bytes, value_start);
    if value_start >= value_end {
        return value_end.max(colon_ix.saturating_add(1));
    }

    let value_bytes = &bytes[value_start..value_end];
    match value_bytes.first().copied() {
        Some(b'"' | b'\'') => {
            let quote = value_bytes[0] as char;
            let string_end = heuristic_string_end(text, value_start, quote);
            tokens.push(SyntaxToken {
                range: value_start..string_end,
                kind: SyntaxTokenKind::String,
            });
            string_end
        }
        Some(b'`') if allow_backtick_strings => {
            let string_end = heuristic_string_end(text, value_start, '`');
            tokens.push(SyntaxToken {
                range: value_start..string_end,
                kind: SyntaxTokenKind::String,
            });
            string_end
        }
        Some(b'|' | b'>') => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Punctuation,
            });
            value_end
        }
        Some(_) if yaml_heuristic_is_plain_boolean(&text[value_start..value_end]) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Boolean,
            });
            value_end
        }
        Some(_) if yaml_heuristic_is_plain_null(&text[value_start..value_end]) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Constant,
            });
            value_end
        }
        Some(_) if yaml_heuristic_is_plain_number(value_bytes) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::Number,
            });
            value_end
        }
        Some(_) => {
            tokens.push(SyntaxToken {
                range: value_start..value_end,
                kind: SyntaxTokenKind::String,
            });
            value_end
        }
        None => value_end,
    }
}

fn syntax_tokens_for_line_heuristic(text: &str, language: DiffSyntaxLanguage) -> Vec<SyntaxToken> {
    let mut tokens: Vec<SyntaxToken> = Vec::new();
    syntax_tokens_for_line_heuristic_into(text, language, &mut tokens);
    tokens
}

pub(super) fn syntax_tokens_for_line_heuristic_into(
    text: &str,
    language: DiffSyntaxLanguage,
    tokens: &mut Vec<SyntaxToken>,
) {
    tokens.clear();
    let bytes = text.as_bytes();
    let len = text.len();
    let mut i = 0usize;
    let comment_config = heuristic_comment_config(language);
    let allow_backtick_strings = heuristic_allows_backtick_strings(language);
    let highlight_css_selectors = matches!(language, DiffSyntaxLanguage::Css);

    let is_ident_start = |byte: u8| byte == b'_' || byte.is_ascii_alphabetic();
    let is_ident_continue = |byte: u8| byte == b'_' || byte.is_ascii_alphanumeric();
    let is_comment_lead = |byte: u8| {
        comment_config
            .line_comment
            .is_some_and(|prefix| prefix.as_bytes().first().copied() == Some(byte))
            || comment_config
                .block_comment
                .is_some_and(|block| block.start.as_bytes().first().copied() == Some(byte))
            || (comment_config.hash_comment && byte == b'#')
            || (comment_config.visual_basic_line_comment
                && (byte == b'\'' || byte.eq_ignore_ascii_case(&b'r')))
    };

    while i < len {
        let byte = bytes[i];

        if matches!(language, DiffSyntaxLanguage::Yaml) {
            if byte == b'-'
                && bytes[..i]
                    .iter()
                    .all(|candidate| candidate.is_ascii_whitespace())
                && bytes
                    .get(i.saturating_add(1))
                    .is_some_and(|next| next.is_ascii_whitespace())
            {
                tokens.push(SyntaxToken {
                    range: i..i.saturating_add(1),
                    kind: SyntaxTokenKind::Punctuation,
                });
                i = i.saturating_add(1);
                continue;
            }

            if yaml_heuristic_key_context(bytes, i)
                && let Some(key_end) = yaml_heuristic_key_end(bytes, i)
            {
                tokens.push(SyntaxToken {
                    range: i..key_end,
                    kind: SyntaxTokenKind::Property,
                });
                tokens.push(SyntaxToken {
                    range: key_end..key_end.saturating_add(1),
                    kind: SyntaxTokenKind::Punctuation,
                });
                i = yaml_heuristic_emit_mapping_value_tokens(
                    text,
                    bytes,
                    key_end,
                    allow_backtick_strings,
                    tokens,
                );
                continue;
            }
        }

        if is_comment_lead(byte)
            && let Some(comment_range) = heuristic_comment_range(text, i, comment_config)
        {
            tokens.push(SyntaxToken {
                range: comment_range.clone(),
                kind: SyntaxTokenKind::Comment,
            });
            i = comment_range.end;
            if i >= len {
                break;
            }
            continue;
        }

        if matches!(byte, b'"' | b'\'') || (allow_backtick_strings && byte == b'`') {
            let j = heuristic_string_end(text, i, byte as char);
            tokens.push(SyntaxToken {
                range: i..j,
                kind: SyntaxTokenKind::String,
            });
            i = j;
            continue;
        }

        if byte.is_ascii_digit() {
            let mut j = i;
            while j < len {
                let next = bytes[j];
                if next.is_ascii_digit() || matches!(next, b'_' | b'.' | b'x' | b'b') {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > i {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Number,
                });
                i = j;
                continue;
            }
        }

        if is_ident_start(byte) {
            let mut j = i + 1;
            while j < len && is_ident_continue(bytes[j]) {
                j += 1;
            }
            let ident = &text[i..j];
            if is_keyword(language, ident) {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Keyword,
                });
            }
            i = j;
            continue;
        }

        if highlight_css_selectors && matches!(byte, b'.' | b'#') {
            let mut j = i + 1;
            while j < len && (is_ident_continue(bytes[j]) || bytes[j] == b'-') {
                j += 1;
            }
            if j > i + 1 {
                tokens.push(SyntaxToken {
                    range: i..j,
                    kind: SyntaxTokenKind::Type,
                });
                i = j;
                continue;
            }
        }

        if byte.is_ascii() {
            i += 1;
        } else if let Some(ch) = text[i..].chars().next() {
            i += ch.len_utf8();
        } else {
            break;
        }
    }
}

fn is_keyword(language: DiffSyntaxLanguage, ident: &str) -> bool {
    // NOTE: This is a heuristic fallback when we don't want to use tree-sitter for a line.
    match language {
        DiffSyntaxLanguage::Markdown => false,
        DiffSyntaxLanguage::Html
        | DiffSyntaxLanguage::Xml
        | DiffSyntaxLanguage::Css
        | DiffSyntaxLanguage::Toml => matches!(ident, "true" | "false"),
        DiffSyntaxLanguage::Json | DiffSyntaxLanguage::Yaml => {
            matches!(ident, "true" | "false" | "null")
        }
        DiffSyntaxLanguage::Hcl => matches!(
            ident,
            "true" | "false" | "null" | "for" | "in" | "if" | "else" | "endif" | "endfor"
        ),
        DiffSyntaxLanguage::Bicep => matches!(
            ident,
            "param" | "var" | "resource" | "module" | "output" | "existing" | "true" | "false"
        ),
        DiffSyntaxLanguage::Lua => matches!(
            ident,
            "and"
                | "break"
                | "do"
                | "else"
                | "elseif"
                | "end"
                | "false"
                | "for"
                | "function"
                | "goto"
                | "if"
                | "in"
                | "local"
                | "nil"
                | "not"
                | "or"
                | "repeat"
                | "return"
                | "then"
                | "true"
                | "until"
                | "while"
        ),
        DiffSyntaxLanguage::Makefile => matches!(ident, "if" | "else" | "endif"),
        DiffSyntaxLanguage::Kotlin => matches!(
            ident,
            "as" | "break"
                | "class"
                | "continue"
                | "do"
                | "else"
                | "false"
                | "for"
                | "fun"
                | "if"
                | "in"
                | "interface"
                | "is"
                | "null"
                | "object"
                | "package"
                | "return"
                | "super"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typealias"
                | "val"
                | "var"
                | "when"
                | "while"
        ),
        DiffSyntaxLanguage::Zig => matches!(
            ident,
            "const"
                | "var"
                | "fn"
                | "pub"
                | "usingnamespace"
                | "test"
                | "if"
                | "else"
                | "while"
                | "for"
                | "switch"
                | "and"
                | "or"
                | "orelse"
                | "break"
                | "continue"
                | "return"
                | "try"
                | "catch"
                | "true"
                | "false"
                | "null"
        ),
        DiffSyntaxLanguage::Rust => matches!(
            ident,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "Self"
                | "self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        DiffSyntaxLanguage::Python => matches!(
            ident,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        DiffSyntaxLanguage::JavaScript
        | DiffSyntaxLanguage::TypeScript
        | DiffSyntaxLanguage::Tsx => {
            matches!(
                ident,
                "break"
                    | "case"
                    | "catch"
                    | "class"
                    | "const"
                    | "continue"
                    | "debugger"
                    | "default"
                    | "delete"
                    | "do"
                    | "else"
                    | "export"
                    | "extends"
                    | "false"
                    | "finally"
                    | "for"
                    | "function"
                    | "if"
                    | "import"
                    | "in"
                    | "instanceof"
                    | "new"
                    | "null"
                    | "return"
                    | "super"
                    | "switch"
                    | "this"
                    | "throw"
                    | "true"
                    | "try"
                    | "typeof"
                    | "var"
                    | "void"
                    | "while"
                    | "with"
                    | "yield"
            )
        }
        DiffSyntaxLanguage::Go => matches!(
            ident,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
        ),
        DiffSyntaxLanguage::C | DiffSyntaxLanguage::Cpp | DiffSyntaxLanguage::CSharp => matches!(
            ident,
            "auto"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "for"
                | "goto"
                | "if"
                | "inline"
                | "new"
                | "nullptr"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typedef"
                | "typename"
                | "union"
                | "using"
                | "virtual"
                | "void"
                | "volatile"
                | "while"
        ),
        DiffSyntaxLanguage::FSharp => matches!(
            ident,
            "let"
                | "in"
                | "match"
                | "with"
                | "type"
                | "member"
                | "interface"
                | "abstract"
                | "override"
                | "true"
                | "false"
                | "null"
        ),
        DiffSyntaxLanguage::VisualBasic => matches!(
            ident,
            "Dim"
                | "As"
                | "If"
                | "Then"
                | "Else"
                | "End"
                | "For"
                | "Each"
                | "In"
                | "Next"
                | "While"
                | "Do"
                | "Loop"
                | "True"
                | "False"
                | "Nothing"
        ),
        DiffSyntaxLanguage::Java => matches!(
            ident,
            "abstract"
                | "assert"
                | "boolean"
                | "break"
                | "byte"
                | "case"
                | "catch"
                | "char"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extends"
                | "final"
                | "finally"
                | "float"
                | "for"
                | "goto"
                | "if"
                | "implements"
                | "import"
                | "instanceof"
                | "int"
                | "interface"
                | "long"
                | "native"
                | "new"
                | "null"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "return"
                | "short"
                | "static"
                | "strictfp"
                | "super"
                | "switch"
                | "synchronized"
                | "this"
                | "throw"
                | "throws"
                | "transient"
                | "true"
                | "false"
                | "try"
                | "void"
                | "volatile"
                | "while"
        ),
        DiffSyntaxLanguage::Php => {
            let ident = ascii_lowercase_for_match(ident);
            matches!(
                ident.as_ref(),
                "function"
                    | "class"
                    | "public"
                    | "private"
                    | "protected"
                    | "static"
                    | "final"
                    | "abstract"
                    | "extends"
                    | "implements"
                    | "use"
                    | "namespace"
                    | "return"
                    | "if"
                    | "else"
                    | "elseif"
                    | "for"
                    | "foreach"
                    | "while"
                    | "do"
                    | "switch"
                    | "case"
                    | "default"
                    | "try"
                    | "catch"
                    | "finally"
                    | "throw"
                    | "new"
                    | "true"
                    | "false"
                    | "null"
            )
        }
        DiffSyntaxLanguage::Ruby => matches!(
            ident,
            "def"
                | "class"
                | "module"
                | "end"
                | "if"
                | "elsif"
                | "else"
                | "unless"
                | "case"
                | "when"
                | "while"
                | "until"
                | "for"
                | "in"
                | "do"
                | "break"
                | "next"
                | "redo"
                | "retry"
                | "return"
                | "yield"
                | "super"
                | "self"
                | "true"
                | "false"
                | "nil"
        ),
        DiffSyntaxLanguage::Sql => {
            let ident = ascii_lowercase_for_match(ident);
            matches!(
                ident.as_ref(),
                "add"
                    | "all"
                    | "alter"
                    | "and"
                    | "as"
                    | "asc"
                    | "begin"
                    | "between"
                    | "by"
                    | "case"
                    | "check"
                    | "column"
                    | "commit"
                    | "constraint"
                    | "create"
                    | "cross"
                    | "database"
                    | "default"
                    | "delete"
                    | "desc"
                    | "distinct"
                    | "drop"
                    | "else"
                    | "end"
                    | "exists"
                    | "false"
                    | "foreign"
                    | "from"
                    | "full"
                    | "group"
                    | "having"
                    | "if"
                    | "in"
                    | "index"
                    | "inner"
                    | "insert"
                    | "intersect"
                    | "into"
                    | "is"
                    | "join"
                    | "key"
                    | "left"
                    | "like"
                    | "limit"
                    | "materialized"
                    | "not"
                    | "null"
                    | "offset"
                    | "on"
                    | "or"
                    | "order"
                    | "outer"
                    | "primary"
                    | "references"
                    | "returning"
                    | "right"
                    | "rollback"
                    | "select"
                    | "set"
                    | "table"
                    | "then"
                    | "transaction"
                    | "true"
                    | "union"
                    | "unique"
                    | "update"
                    | "values"
                    | "view"
                    | "when"
                    | "where"
                    | "with"
            )
        }
        DiffSyntaxLanguage::Bash => matches!(
            ident,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "in"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "while"
                | "function"
                | "return"
                | "break"
                | "continue"
        ),
    }
}

fn syntax_tokens_for_line_markdown(text: &str) -> Vec<SyntaxToken> {
    let len = text.len();
    if len == 0 {
        return Vec::new();
    }

    let trimmed = text.trim_start_matches([' ', '\t']);
    let indent = len.saturating_sub(trimmed.len());

    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return vec![SyntaxToken {
            range: 0..len,
            kind: SyntaxTokenKind::Keyword,
        }];
    }

    if trimmed.starts_with('>') {
        return vec![SyntaxToken {
            range: indent..len,
            kind: SyntaxTokenKind::Comment,
        }];
    }

    // Headings: up to 6 leading `#` and a following space.
    let mut hashes = 0usize;
    for ch in trimmed.chars() {
        if ch == '#' && hashes < 6 {
            hashes += 1;
        } else {
            break;
        }
    }
    if hashes > 0 {
        let after_hashes = trimmed[hashes..].chars().next();
        if after_hashes.is_some_and(|c| c.is_whitespace()) {
            return vec![SyntaxToken {
                range: indent..len,
                kind: SyntaxTokenKind::Keyword,
            }];
        }
    }

    // Inline code: highlight backtick-delimited ranges.
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut tokens: Vec<SyntaxToken> = Vec::new();
    while i < len {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }

        let start = i;
        let mut tick_len = 0usize;
        while i < len && bytes[i] == b'`' {
            tick_len += 1;
            i += 1;
        }

        let mut j = i;
        while j < len {
            if bytes[j] != b'`' {
                j += 1;
                continue;
            }
            let mut run = 0usize;
            while j + run < len && bytes[j + run] == b'`' {
                run += 1;
            }
            if run == tick_len {
                let end = (j + run).min(len);
                if start < end {
                    tokens.push(SyntaxToken {
                        range: start..end,
                        kind: SyntaxTokenKind::String,
                    });
                }
                i = end;
                break;
            }
            j += run.max(1);
        }
        if j >= len {
            // Unterminated inline code; stop scanning to avoid odd highlighting.
            break;
        }
    }

    tokens
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
            Arc::ptr_eq(&first.text, &second.text),
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
    fn vendored_typescript_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        tree_sitter::Query::new(&lang, TYPESCRIPT_HIGHLIGHTS_QUERY)
            .expect("vendored TypeScript highlights.scm should compile");
    }

    #[cfg(any(test, feature = "syntax-web"))]
    #[test]
    fn vendored_tsx_query_compiles() {
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
        tree_sitter::Query::new(&lang, TSX_HIGHLIGHTS_QUERY)
            .expect("vendored TSX highlights.scm should compile");
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

    fn capture_name_is_intentionally_ignored(name: &str) -> bool {
        name == "none"
            || name == "embedded"
            || name == "error"
            || name == "nested"
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
        #[cfg(any(test, feature = "syntax-web"))]
        assert_capture_names_are_supported(
            tree_sitter_javascript::LANGUAGE.into(),
            JAVASCRIPT_HIGHLIGHTS_QUERY,
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
            DiffSyntaxLanguage::C,
            DiffSyntaxLanguage::Cpp,
            DiffSyntaxLanguage::CSharp,
            DiffSyntaxLanguage::FSharp,
            DiffSyntaxLanguage::VisualBasic,
            DiffSyntaxLanguage::Java,
            DiffSyntaxLanguage::Php,
            DiffSyntaxLanguage::Ruby,
            DiffSyntaxLanguage::Json,
            DiffSyntaxLanguage::Toml,
            DiffSyntaxLanguage::Yaml,
            DiffSyntaxLanguage::Sql,
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
