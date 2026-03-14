use super::super::*;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};
use tree_sitter::StreamingIterator;

const TS_DOCUMENT_CACHE_MAX_ENTRIES: usize = 8;
const TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS: usize = 64;
const DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST: Duration = Duration::from_millis(1);
const DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_TEST: Duration = Duration::from_millis(2);
const TS_QUERY_MATCH_LIMIT: u32 = 256;
const TS_MAX_BYTES_TO_QUERY: usize = 16 * 1024;
const TS_QUERY_MAX_LINES_PER_PASS: usize = 256;
const TS_DEFERRED_DROP_MIN_BYTES: usize = 256 * 1024;
const TS_INCREMENTAL_REPARSE_ENABLE_ENV: &str = "GITCOMET_DIFF_SYNTAX_INCREMENTAL_REPARSE";
const TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES: usize = 64 * 1024;
const TS_INCREMENTAL_REPARSE_MAX_CHANGED_PERCENT: usize = 35;

thread_local! {
    static TS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new(tree_sitter::Parser::new());
    static TS_CURSOR: RefCell<tree_sitter::QueryCursor> = RefCell::new(tree_sitter::QueryCursor::new());
    static TS_INPUT: RefCell<String> = const { RefCell::new(String::new()) };
    static TS_DOCUMENT_CACHE: RefCell<TreesitterDocumentCache> = RefCell::new(TreesitterDocumentCache::new());
}

fn ascii_lowercase_for_match(s: &str) -> Cow<'_, str> {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(s.to_ascii_lowercase())
    } else {
        Cow::Borrowed(s)
    }
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiffSyntaxMode {
    Auto,
    HeuristicOnly,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TreesitterParseReuseMode {
    Full,
    Incremental,
}

#[derive(Clone, Debug)]
struct PreparedSyntaxTreeState {
    language: DiffSyntaxLanguage,
    text: String,
    line_lengths: Vec<usize>,
    line_starts: Vec<usize>,
    source_hash: u64,
    source_version: u64,
    tree: tree_sitter::Tree,
    #[allow(dead_code)]
    parse_mode: TreesitterParseReuseMode,
}

#[derive(Clone, Debug)]
pub(super) struct PreparedSyntaxDocumentData {
    cache_key: PreparedSyntaxCacheKey,
    line_count: usize,
    line_token_chunks: HashMap<usize, Vec<Vec<SyntaxToken>>>,
    tree_state: Option<PreparedSyntaxTreeState>,
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
    InlineWhenLarge,
}

enum SyntaxCacheDropMessage {
    Drop(Vec<Vec<SyntaxToken>>),
    Flush(mpsc::Sender<()>),
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
#[cfg(test)]
static TS_INCREMENTAL_PARSE_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static TS_INCREMENTAL_FALLBACK_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static TS_TREE_STATE_CLONE_COUNT: std::sync::atomic::AtomicUsize =
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
                            SyntaxCacheDropMessage::Drop(line_tokens) => {
                                drop(line_tokens);
                                #[cfg(test)]
                                TS_DEFERRED_DROP_COMPLETED
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
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

fn estimated_line_tokens_allocation_bytes(line_tokens: &[Vec<SyntaxToken>]) -> usize {
    let outer = line_tokens
        .len()
        .saturating_mul(std::mem::size_of::<Vec<SyntaxToken>>());
    let inner = line_tokens.iter().fold(0usize, |acc, line| {
        acc.saturating_add(
            line.capacity()
                .saturating_mul(std::mem::size_of::<SyntaxToken>()),
        )
    });
    outer.saturating_add(inner)
}

fn should_defer_line_tokens_drop(line_tokens: &[Vec<SyntaxToken>]) -> bool {
    estimated_line_tokens_allocation_bytes(line_tokens) >= TS_DEFERRED_DROP_MIN_BYTES
}

fn drop_line_tokens_with_mode(line_tokens: Vec<Vec<SyntaxToken>>, drop_mode: SyntaxCacheDropMode) {
    let should_try_deferred = matches!(drop_mode, SyntaxCacheDropMode::DeferredWhenLarge)
        && should_defer_line_tokens_drop(&line_tokens);

    if should_try_deferred && let Some(sender) = syntax_cache_drop_sender() {
        if sender
            .send(SyntaxCacheDropMessage::Drop(line_tokens))
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
    drop(line_tokens);
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
    TS_INCREMENTAL_PARSE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_INCREMENTAL_FALLBACK_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_TREE_STATE_CLONE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
fn incremental_reparse_counters() -> (usize, usize) {
    (
        TS_INCREMENTAL_PARSE_COUNT.load(std::sync::atomic::Ordering::Relaxed),
        TS_INCREMENTAL_FALLBACK_COUNT.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
fn tree_state_clone_count() -> usize {
    TS_TREE_STATE_CLONE_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
fn reset_tree_state_clone_count() {
    TS_TREE_STATE_CLONE_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

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

#[cfg(test)]
fn flush_deferred_syntax_cache_drop_queue() -> bool {
    flush_deferred_syntax_cache_drop_queue_with_timeout(Duration::from_secs(2))
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
    line_token_chunks: HashMap<usize, Vec<Vec<SyntaxToken>>>,
    tree_state: Option<PreparedSyntaxTreeState>,
}

impl TreesitterCachedDocument {
    fn from_line_tokens(
        line_tokens: Vec<Vec<SyntaxToken>>,
        tree_state: Option<PreparedSyntaxTreeState>,
    ) -> Self {
        let line_count = line_tokens.len();
        Self {
            line_count,
            line_token_chunks: chunk_line_tokens_by_row(line_tokens),
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

    fn into_line_tokens_for_drop(self) -> Vec<Vec<SyntaxToken>> {
        if self.line_token_chunks.is_empty() {
            return Vec::new();
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
        out
    }
}

fn chunk_line_tokens_by_row(
    line_tokens: Vec<Vec<SyntaxToken>>,
) -> HashMap<usize, Vec<Vec<SyntaxToken>>> {
    if line_tokens.is_empty() {
        return HashMap::new();
    }

    let mut chunks = HashMap::new();
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

fn clone_tree_state_for_chunk_build(
    tree_state: &Option<PreparedSyntaxTreeState>,
) -> Option<PreparedSyntaxTreeState> {
    #[cfg(test)]
    if tree_state.is_some() {
        TS_TREE_STATE_CLONE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    tree_state.clone()
}

struct TreesitterDocumentCache {
    by_cache_key: HashMap<PreparedSyntaxCacheKey, TreesitterCachedDocument>,
    lru_order: VecDeque<PreparedSyntaxCacheKey>,
    metrics: PreparedSyntaxCacheMetrics,
}

impl TreesitterDocumentCache {
    fn new() -> Self {
        Self {
            by_cache_key: HashMap::new(),
            lru_order: VecDeque::new(),
            metrics: PreparedSyntaxCacheMetrics::default(),
        }
    }

    fn touch_key(&mut self, cache_key: PreparedSyntaxCacheKey) {
        if let Some(pos) = self
            .lru_order
            .iter()
            .position(|candidate| *candidate == cache_key)
        {
            self.lru_order.remove(pos);
        }
        self.lru_order.push_back(cache_key);
    }

    fn evict_if_needed(&mut self, drop_mode: SyntaxCacheDropMode) {
        while self.by_cache_key.len() >= TS_DOCUMENT_CACHE_MAX_ENTRIES {
            let Some(evict_key) = self.lru_order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.by_cache_key.remove(&evict_key) {
                self.metrics.evict = self.metrics.evict.saturating_add(1);
                drop_line_tokens_with_mode(evicted.into_line_tokens_for_drop(), drop_mode);
                break;
            }
        }
    }

    fn build_line_token_chunk(
        &mut self,
        tree_state: &PreparedSyntaxTreeState,
        line_count: usize,
        chunk_ix: usize,
    ) -> Option<Vec<Vec<SyntaxToken>>> {
        let chunk_start = chunk_ix.saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
        if chunk_start >= line_count {
            return Some(Vec::new());
        }
        let chunk_end = chunk_start
            .saturating_add(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
            .min(line_count);
        let highlight = tree_sitter_highlight_spec(tree_state.language)?;
        let started = Instant::now();
        let chunk = collect_treesitter_document_line_tokens_for_line_window(
            &tree_state.tree,
            highlight,
            tree_state.text.as_bytes(),
            &tree_state.line_starts,
            &tree_state.line_lengths,
            chunk_start,
            chunk_end,
        );
        let chunk_build_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.metrics.chunk_build_ms = self.metrics.chunk_build_ms.saturating_add(chunk_build_ms);
        Some(chunk)
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

    fn line_tokens(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
    ) -> Option<Vec<SyntaxToken>> {
        let chunk_ix = line_ix / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
        let (line_count, has_chunk) = {
            let document = self.by_cache_key.get(&cache_key)?;
            (
                document.line_count,
                document.line_token_chunks.contains_key(&chunk_ix),
            )
        };

        if line_ix >= line_count {
            self.metrics.hit = self.metrics.hit.saturating_add(1);
            self.touch_key(cache_key);
            return Some(Vec::new());
        }

        if has_chunk {
            self.metrics.hit = self.metrics.hit.saturating_add(1);
        } else {
            self.metrics.miss = self.metrics.miss.saturating_add(1);
            let tree_state = self
                .by_cache_key
                .get(&cache_key)
                .and_then(|document| clone_tree_state_for_chunk_build(&document.tree_state));
            let chunk_tokens = tree_state
                .as_ref()
                .and_then(|state| self.build_line_token_chunk(state, line_count, chunk_ix));
            if let Some(document) = self.by_cache_key.get_mut(&cache_key)
                && !document.line_token_chunks.contains_key(&chunk_ix)
            {
                let fallback_empty_chunk = {
                    let bounds = document.chunk_bounds(chunk_ix);
                    vec![Vec::new(); bounds.end.saturating_sub(bounds.start)]
                };
                document
                    .line_token_chunks
                    .insert(chunk_ix, chunk_tokens.unwrap_or(fallback_empty_chunk));
            }
        }

        self.touch_key(cache_key);
        let document = self.by_cache_key.get(&cache_key)?;
        let chunk_bounds = document.chunk_bounds(chunk_ix);
        let line_offset = line_ix.saturating_sub(chunk_bounds.start);
        Some(
            document
                .line_token_chunks
                .get(&chunk_ix)
                .and_then(|chunk| chunk.get(line_offset))
                .cloned()
                .unwrap_or_default(),
        )
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

    fn metrics(&self) -> PreparedSyntaxCacheMetrics {
        self.metrics
    }

    fn reset_metrics(&mut self) {
        self.metrics = PreparedSyntaxCacheMetrics::default();
    }

    fn loaded_chunk_count(&self, cache_key: PreparedSyntaxCacheKey) -> Option<usize> {
        Some(self.by_cache_key.get(&cache_key)?.line_token_chunks.len())
    }

    fn contains_key(&self, cache_key: PreparedSyntaxCacheKey) -> bool {
        self.by_cache_key.contains_key(&cache_key)
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

        if let Some(replaced) = self.by_cache_key.insert(cache_key, document) {
            drop_line_tokens_with_mode(replaced.into_line_tokens_for_drop(), drop_mode);
        }
        self.touch_key(cache_key);
    }
}

struct TreesitterDocumentInput {
    text: String,
    line_lengths: Vec<usize>,
}

struct TreesitterDocumentParseRequest {
    language: DiffSyntaxLanguage,
    ts_language: tree_sitter::Language,
    input: TreesitterDocumentInput,
    cache_key: PreparedSyntaxCacheKey,
}

pub(super) fn benchmark_reset_prepared_syntax_cache_metrics() {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().reset_metrics());
}

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

pub(super) fn benchmark_prepared_syntax_loaded_chunk_count(
    document: PreparedSyntaxDocument,
) -> Option<usize> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().loaded_chunk_count(document.cache_key))
}

pub(super) fn benchmark_prepared_syntax_cache_contains_document(
    document: PreparedSyntaxDocument,
) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().contains_key(document.cache_key))
}

#[cfg(test)]
fn prepared_syntax_cache_metrics() -> PreparedSyntaxCacheMetrics {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().metrics())
}

#[cfg(test)]
fn reset_prepared_syntax_cache() {
    TS_DOCUMENT_CACHE.with(|cache| {
        *cache.borrow_mut() = TreesitterDocumentCache::new();
    });
}

#[cfg(test)]
fn prepared_syntax_loaded_chunk_count(document: PreparedSyntaxDocument) -> usize {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow()
            .loaded_chunk_count(document.cache_key)
            .unwrap_or_default()
    })
}

#[cfg(test)]
fn make_test_cache_key(doc_hash: u64) -> PreparedSyntaxCacheKey {
    TreesitterDocumentCache::make_test_cache_key(doc_hash)
}

pub(in crate::view) fn diff_syntax_language_for_path(
    path: impl AsRef<std::path::Path>,
) -> Option<DiffSyntaxLanguage> {
    let p = path.as_ref();
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    let ext = ascii_lowercase_for_match(ext);

    let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");

    Some(match ext.as_ref() {
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn" => DiffSyntaxLanguage::Markdown,
        "html" | "htm" => DiffSyntaxLanguage::Html,
        // Use HTML highlighting for XML-ish formats as a pragmatic baseline.
        "xml" | "svg" | "xsl" | "xslt" | "xsd" => DiffSyntaxLanguage::Html,
        "css" | "less" | "sass" | "scss" => DiffSyntaxLanguage::Css,
        "hcl" | "tf" | "tfvars" => DiffSyntaxLanguage::Hcl,
        "bicep" => DiffSyntaxLanguage::Bicep,
        "lua" => DiffSyntaxLanguage::Lua,
        "mk" => DiffSyntaxLanguage::Makefile,
        "kt" | "kts" => DiffSyntaxLanguage::Kotlin,
        "zig" => DiffSyntaxLanguage::Zig,
        "rs" => DiffSyntaxLanguage::Rust,
        "py" => DiffSyntaxLanguage::Python,
        "js" | "jsx" | "mjs" | "cjs" => DiffSyntaxLanguage::JavaScript,
        "ts" | "cts" | "mts" => DiffSyntaxLanguage::TypeScript,
        "tsx" => DiffSyntaxLanguage::Tsx,
        "go" => DiffSyntaxLanguage::Go,
        "c" | "h" => DiffSyntaxLanguage::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => DiffSyntaxLanguage::Cpp,
        "cs" => DiffSyntaxLanguage::CSharp,
        "fs" | "fsx" | "fsi" => DiffSyntaxLanguage::FSharp,
        "vb" | "vbs" => DiffSyntaxLanguage::VisualBasic,
        "java" => DiffSyntaxLanguage::Java,
        "php" | "phtml" => DiffSyntaxLanguage::Php,
        "rb" => DiffSyntaxLanguage::Ruby,
        "json" => DiffSyntaxLanguage::Json,
        "toml" => DiffSyntaxLanguage::Toml,
        "yaml" | "yml" => DiffSyntaxLanguage::Yaml,
        "sql" => DiffSyntaxLanguage::Sql,
        "sh" | "bash" | "zsh" => DiffSyntaxLanguage::Bash,
        _ => {
            if file_name.eq_ignore_ascii_case("makefile")
                || file_name.eq_ignore_ascii_case("gnumakefile")
            {
                DiffSyntaxLanguage::Makefile
            } else {
                return None;
            }
        }
    })
}

pub(super) fn syntax_tokens_for_line(
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
            if !should_use_treesitter_for_line(text) {
                return syntax_tokens_for_line_heuristic(text, language);
            }
            if let Some(tokens) = syntax_tokens_for_line_treesitter(text, language) {
                return tokens;
            }
            syntax_tokens_for_line_heuristic(text, language)
        }
    }
}

pub(super) fn prepare_treesitter_document<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
) -> Option<PreparedSyntaxDocument>
where
    I: IntoIterator<Item = &'a str>,
{
    match prepare_treesitter_document_impl(language, mode, lines, None, None) {
        PrepareTreesitterDocumentResult::Ready(document) => Some(document),
        PrepareTreesitterDocumentResult::TimedOut
        | PrepareTreesitterDocumentResult::Unsupported => None,
    }
}

pub(super) fn prepare_treesitter_document_with_budget<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
    budget: DiffSyntaxBudget,
) -> PrepareTreesitterDocumentResult
where
    I: IntoIterator<Item = &'a str>,
{
    prepare_treesitter_document_with_budget_reuse(language, mode, lines, budget, None)
}

pub(super) fn prepare_treesitter_document_with_budget_reuse<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
    budget: DiffSyntaxBudget,
    old_document: Option<PreparedSyntaxDocument>,
) -> PrepareTreesitterDocumentResult
where
    I: IntoIterator<Item = &'a str>,
{
    prepare_treesitter_document_impl(language, mode, lines, Some(budget), old_document)
}

pub(super) fn prepare_treesitter_document_in_background<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
) -> Option<PreparedSyntaxDocumentData>
where
    I: IntoIterator<Item = &'a str>,
{
    prepare_treesitter_document_data_impl(language, mode, lines, None)
}

pub(super) fn inject_prepared_document_data(
    document: PreparedSyntaxDocumentData,
) -> PreparedSyntaxDocument {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache.borrow_mut().insert_document_with_mode(
            document.cache_key,
            TreesitterCachedDocument {
                line_count: document.line_count,
                line_token_chunks: document.line_token_chunks,
                tree_state: document.tree_state,
            },
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    });
    PreparedSyntaxDocument {
        cache_key: document.cache_key,
    }
}

pub(super) fn syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<Vec<SyntaxToken>> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().line_tokens(document.cache_key, line_ix))
}

#[cfg(test)]
fn prepared_document_parse_mode(
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
fn prepared_document_source_version(document: PreparedSyntaxDocument) -> Option<u64> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state(document.cache_key)
            .map(|state| state.source_version)
    })
}

pub(super) fn benchmark_cache_replacement_drop_step(
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
    defer_drop: bool,
) -> u64 {
    let payloads = benchmark_line_tokens_payload_batch(lines, tokens_per_line, replacements, 0);
    benchmark_cache_replacement_drop_step_with_payloads(payloads, defer_drop)
}

pub(super) fn benchmark_drop_payload_timed_step(
    lines: usize,
    tokens_per_line: usize,
    seed: usize,
    defer_drop: bool,
) -> Duration {
    let payload = benchmark_line_tokens_payload(lines.max(1), tokens_per_line.max(1), seed);
    let start = std::time::Instant::now();
    drop_line_tokens_with_mode(payload, benchmark_drop_mode(defer_drop));
    start.elapsed()
}

fn benchmark_cache_replacement_drop_step_with_payloads(
    payloads: Vec<Vec<Vec<SyntaxToken>>>,
    defer_drop: bool,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut cache = TreesitterDocumentCache::new();
    let drop_mode = benchmark_drop_mode(defer_drop);

    let mut h = DefaultHasher::new();
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

fn benchmark_drop_mode(defer_drop: bool) -> SyntaxCacheDropMode {
    if defer_drop {
        SyntaxCacheDropMode::DeferredWhenLarge
    } else {
        SyntaxCacheDropMode::InlineWhenLarge
    }
}

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

fn prepare_treesitter_document_impl<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
    foreground_budget: Option<DiffSyntaxBudget>,
    old_document: Option<PreparedSyntaxDocument>,
) -> PrepareTreesitterDocumentResult
where
    I: IntoIterator<Item = &'a str>,
{
    let Some(request) = treesitter_document_parse_request(language, mode, lines) else {
        return PrepareTreesitterDocumentResult::Unsupported;
    };

    let line_count = request.input.line_lengths.len();
    let has_cache_hit = TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .contains_document(request.cache_key, line_count)
    });
    if has_cache_hit {
        return PrepareTreesitterDocumentResult::Ready(PreparedSyntaxDocument {
            cache_key: request.cache_key,
        });
    }

    if foreground_budget.is_some_and(|budget| budget.foreground_parse.is_zero()) {
        return PrepareTreesitterDocumentResult::TimedOut;
    }

    let old_tree_state = old_document.and_then(|document| {
        TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().tree_state(document.cache_key))
    });
    let mut used_old_document_without_incremental = false;
    let incremental_seed = old_tree_state.as_ref().and_then(|state| {
        let seed = build_incremental_parse_seed(state, &request);
        if seed.is_none() && state.language == request.language && incremental_reparse_enabled() {
            used_old_document_without_incremental = true;
        }
        seed
    });

    #[cfg(test)]
    {
        if incremental_seed.is_some() {
            TS_INCREMENTAL_PARSE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else if used_old_document_without_incremental {
            TS_INCREMENTAL_FALLBACK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let old_tree_for_parse = incremental_seed.as_ref().map(|seed| &seed.tree);
    let tree = TS_PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        parser.set_language(&request.ts_language).ok()?;
        parse_treesitter_tree(
            &mut parser,
            request.input.text.as_bytes(),
            old_tree_for_parse,
            foreground_budget.map(|budget| budget.foreground_parse),
        )
    });

    let Some(tree) = tree else {
        return if foreground_budget.is_some() {
            PrepareTreesitterDocumentResult::TimedOut
        } else {
            PrepareTreesitterDocumentResult::Unsupported
        };
    };

    let parse_mode = if incremental_seed.is_some() {
        TreesitterParseReuseMode::Incremental
    } else {
        TreesitterParseReuseMode::Full
    };
    let source_version = incremental_seed
        .as_ref()
        .map(|seed| seed.next_version)
        .unwrap_or(1);
    let tree_state = Some(PreparedSyntaxTreeState {
        language: request.language,
        text: request.input.text.clone(),
        line_lengths: request.input.line_lengths.clone(),
        line_starts: treesitter_document_line_starts(&request.input.line_lengths),
        source_hash: request.cache_key.doc_hash,
        source_version,
        tree,
        parse_mode,
    });

    TS_DOCUMENT_CACHE.with(|cache| {
        cache.borrow_mut().insert_document_with_mode(
            request.cache_key,
            TreesitterCachedDocument {
                line_count,
                line_token_chunks: HashMap::new(),
                tree_state,
            },
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    });

    PrepareTreesitterDocumentResult::Ready(PreparedSyntaxDocument {
        cache_key: request.cache_key,
    })
}

fn prepare_treesitter_document_data_impl<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
    old_document: Option<PreparedSyntaxDocument>,
) -> Option<PreparedSyntaxDocumentData>
where
    I: IntoIterator<Item = &'a str>,
{
    let request = treesitter_document_parse_request(language, mode, lines)?;

    if let Some(cached) = TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .prepared_document_data(request.cache_key, request.input.line_lengths.len())
    }) {
        return Some(cached);
    }

    let old_tree_state = old_document.and_then(|document| {
        TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().tree_state(document.cache_key))
    });
    let mut used_old_document_without_incremental = false;
    let incremental_seed = old_tree_state.as_ref().and_then(|state| {
        let seed = build_incremental_parse_seed(state, &request);
        if seed.is_none() && state.language == request.language && incremental_reparse_enabled() {
            used_old_document_without_incremental = true;
        }
        seed
    });

    #[cfg(test)]
    {
        if incremental_seed.is_some() {
            TS_INCREMENTAL_PARSE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else if used_old_document_without_incremental {
            TS_INCREMENTAL_FALLBACK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let old_tree_for_parse = incremental_seed.as_ref().map(|seed| &seed.tree);
    let tree = TS_PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        parser.set_language(&request.ts_language).ok()?;
        parse_treesitter_tree(
            &mut parser,
            request.input.text.as_bytes(),
            old_tree_for_parse,
            None,
        )
    })?;

    let parse_mode = if incremental_seed.is_some() {
        TreesitterParseReuseMode::Incremental
    } else {
        TreesitterParseReuseMode::Full
    };
    let source_version = incremental_seed
        .as_ref()
        .map(|seed| seed.next_version)
        .unwrap_or(1);
    let line_lengths = request.input.line_lengths.clone();
    let line_starts = treesitter_document_line_starts(&line_lengths);
    let line_count = line_lengths.len();

    Some(PreparedSyntaxDocumentData {
        cache_key: request.cache_key,
        line_count,
        line_token_chunks: HashMap::new(),
        tree_state: Some(PreparedSyntaxTreeState {
            language: request.language,
            text: request.input.text,
            line_lengths,
            line_starts,
            source_hash: request.cache_key.doc_hash,
            source_version,
            tree,
            parse_mode,
        }),
    })
}

fn treesitter_document_parse_request<'a, I>(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    lines: I,
) -> Option<TreesitterDocumentParseRequest>
where
    I: IntoIterator<Item = &'a str>,
{
    if mode != DiffSyntaxMode::Auto {
        return None;
    }
    if matches!(language, DiffSyntaxLanguage::Markdown) {
        return None;
    }

    let ts_language = tree_sitter_language(language)?;
    tree_sitter_highlight_spec(language)?;
    let input = collect_treesitter_document_input(lines);
    let cache_key = treesitter_document_cache_key(language, &input.text);

    Some(TreesitterDocumentParseRequest {
        language,
        ts_language,
        input,
        cache_key,
    })
}

fn collect_treesitter_document_input<'a, I>(lines: I) -> TreesitterDocumentInput
where
    I: IntoIterator<Item = &'a str>,
{
    let mut text = String::new();
    let mut line_lengths = Vec::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
        line_lengths.push(line.len());
    }
    TreesitterDocumentInput { text, line_lengths }
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

fn build_incremental_parse_seed(
    previous: &PreparedSyntaxTreeState,
    request: &TreesitterDocumentParseRequest,
) -> Option<TreesitterIncrementalSeed> {
    if !incremental_reparse_enabled() {
        return None;
    }
    if previous.language != request.language {
        return None;
    }
    if previous.source_hash == request.cache_key.doc_hash {
        return None;
    }

    let old_input = previous.text.as_bytes();
    let new_input = request.input.text.as_bytes();
    let edit_ranges = compute_incremental_edit_ranges(old_input, new_input);
    if edit_ranges.is_empty() {
        return None;
    }
    if incremental_reparse_should_fallback(&edit_ranges, old_input.len(), new_input.len()) {
        return None;
    }

    let new_line_starts = treesitter_document_line_starts(&request.input.line_lengths);
    let mut tree = previous.tree.clone();
    for edit_range in edit_ranges {
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
                &new_line_starts,
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
    let changed_bytes = edits.iter().fold(0usize, |acc, edit| {
        let old_delta = edit.old_end_byte.saturating_sub(edit.start_byte);
        let new_delta = edit.new_end_byte.saturating_sub(edit.start_byte);
        acc.saturating_add(old_delta.max(new_delta))
    });
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

fn should_use_treesitter_for_line(text: &str) -> bool {
    text.len() <= MAX_TREESITTER_LINE_BYTES
}

struct TreesitterHighlightSpec {
    query: tree_sitter::Query,
    capture_kinds: Vec<Option<SyntaxTokenKind>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreesitterQueryPass {
    byte_range: Range<usize>,
    containing_byte_range: Option<Range<usize>>,
}

struct DocumentTokenCollectionContext<'a> {
    line_starts: &'a [usize],
    line_lengths: &'a [usize],
    start_line_ix: usize,
    end_line_ix: usize,
    per_line: &'a mut [Vec<SyntaxToken>],
}

fn syntax_tokens_for_line_treesitter(
    text: &str,
    language: DiffSyntaxLanguage,
) -> Option<Vec<SyntaxToken>> {
    let ts_language = tree_sitter_language(language)?;
    let highlight = tree_sitter_highlight_spec(language)?;

    let input_len = text.len();
    let tree = TS_INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.clear();
        input.push_str(text);
        input.push('\n');

        TS_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            parser.set_language(&ts_language).ok()?;
            parser.parse(&*input, None)
        })
    })?;

    let mut tokens: Vec<SyntaxToken> = Vec::new();
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

    Some(normalize_non_overlapping_tokens(tokens))
}

fn treesitter_document_cache_key(
    language: DiffSyntaxLanguage,
    input: &str,
) -> PreparedSyntaxCacheKey {
    PreparedSyntaxCacheKey {
        language,
        doc_hash: treesitter_document_doc_hash(input),
    }
}

fn treesitter_document_doc_hash(input: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
fn collect_treesitter_document_line_tokens(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_lengths: &[usize],
) -> Vec<Vec<SyntaxToken>> {
    if line_lengths.is_empty() {
        return Vec::new();
    }

    let line_starts = treesitter_document_line_starts(line_lengths);
    collect_treesitter_document_line_tokens_for_line_window(
        tree,
        highlight,
        input,
        &line_starts,
        line_lengths,
        0,
        line_lengths.len(),
    )
}

fn collect_treesitter_document_line_tokens_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    line_lengths: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
) -> Vec<Vec<SyntaxToken>> {
    if line_lengths.is_empty()
        || start_line_ix >= end_line_ix
        || start_line_ix >= line_lengths.len()
    {
        return Vec::new();
    }

    let end_line_ix = end_line_ix.min(line_lengths.len());
    let mut per_line: Vec<Vec<SyntaxToken>> = vec![Vec::new(); end_line_ix - start_line_ix];
    let query_passes = treesitter_document_query_passes_for_line_window(
        line_starts,
        line_lengths,
        input.len(),
        start_line_ix,
        end_line_ix,
    );
    {
        let mut context = DocumentTokenCollectionContext {
            line_starts,
            line_lengths,
            start_line_ix,
            end_line_ix,
            per_line: &mut per_line,
        };
        for pass in &query_passes {
            collect_query_pass_tokens_for_document(tree, highlight, input, pass, &mut context);
        }
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

fn treesitter_document_line_starts(line_lengths: &[usize]) -> Vec<usize> {
    let mut line_starts = Vec::with_capacity(line_lengths.len());
    let mut byte_offset = 0usize;
    for line_len in line_lengths {
        line_starts.push(byte_offset);
        byte_offset = byte_offset.saturating_add(line_len.saturating_add(1));
    }
    line_starts
}

fn line_query_end_byte(line_start: usize, line_len: usize, input_len: usize) -> usize {
    line_start
        .saturating_add(line_len.saturating_add(1))
        .min(input_len)
}

#[cfg(test)]
fn treesitter_document_query_passes(
    line_starts: &[usize],
    line_lengths: &[usize],
    input_len: usize,
) -> Vec<TreesitterQueryPass> {
    treesitter_document_query_passes_for_line_window(
        line_starts,
        line_lengths,
        input_len,
        0,
        line_lengths.len(),
    )
}

fn treesitter_document_query_passes_for_line_window(
    line_starts: &[usize],
    line_lengths: &[usize],
    input_len: usize,
    start_line_ix: usize,
    end_line_ix: usize,
) -> Vec<TreesitterQueryPass> {
    if input_len == 0
        || line_lengths.is_empty()
        || start_line_ix >= end_line_ix
        || start_line_ix >= line_lengths.len()
    {
        return Vec::new();
    }
    let end_line_ix = end_line_ix.min(line_lengths.len());
    if start_line_ix >= end_line_ix {
        return Vec::new();
    }

    let window_start_byte = line_starts[start_line_ix].min(input_len);
    let window_end_byte = line_query_end_byte(
        line_starts[end_line_ix - 1].min(input_len),
        line_lengths[end_line_ix - 1],
        input_len,
    );
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
        let line_end = line_query_end_byte(line_start, line_lengths[line_ix], input_len);
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
            let next_line_start = line_starts[next_line_ix].min(input_len);
            let next_line_end =
                line_query_end_byte(next_line_start, line_lengths[next_line_ix], input_len);
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
                let line_end = line_start.saturating_add(context.line_lengths[line_ix]);
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
}

fn normalize_non_overlapping_tokens(mut tokens: Vec<SyntaxToken>) -> Vec<SyntaxToken> {
    if tokens.is_empty() {
        return tokens;
    }

    tokens.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(a.range.end.cmp(&b.range.end))
    });

    // Ensure non-overlapping tokens so the segment splitter can pick a single style per range.
    let mut out: Vec<SyntaxToken> = Vec::with_capacity(tokens.len());
    for mut token in tokens {
        if let Some(prev) = out.last()
            && token.range.start < prev.range.end
        {
            if token.range.end <= prev.range.end {
                continue;
            }
            token.range.start = prev.range.end;
            if token.range.start >= token.range.end {
                continue;
            }
        }
        out.push(token);
    }
    out
}

fn tree_sitter_language(language: DiffSyntaxLanguage) -> Option<tree_sitter::Language> {
    Some(match language {
        DiffSyntaxLanguage::Markdown => return None,
        DiffSyntaxLanguage::Html => tree_sitter_html::LANGUAGE.into(),
        DiffSyntaxLanguage::Css => tree_sitter_css::LANGUAGE.into(),
        DiffSyntaxLanguage::Hcl => return None,
        DiffSyntaxLanguage::Bicep => return None,
        DiffSyntaxLanguage::Lua => return None,
        DiffSyntaxLanguage::Makefile => return None,
        DiffSyntaxLanguage::Kotlin => return None,
        DiffSyntaxLanguage::Zig => return None,
        DiffSyntaxLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        DiffSyntaxLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        DiffSyntaxLanguage::Go => tree_sitter_go::LANGUAGE.into(),
        DiffSyntaxLanguage::C => return None,
        DiffSyntaxLanguage::Cpp => return None,
        DiffSyntaxLanguage::CSharp => return None,
        DiffSyntaxLanguage::FSharp => return None,
        DiffSyntaxLanguage::VisualBasic => return None,
        DiffSyntaxLanguage::Java => return None,
        DiffSyntaxLanguage::Php => return None,
        DiffSyntaxLanguage::Ruby => return None,
        DiffSyntaxLanguage::Json => tree_sitter_json::LANGUAGE.into(),
        DiffSyntaxLanguage::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        DiffSyntaxLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        DiffSyntaxLanguage::Tsx | DiffSyntaxLanguage::JavaScript => {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        }
        DiffSyntaxLanguage::Sql => return None,
        DiffSyntaxLanguage::Bash => tree_sitter_bash::LANGUAGE.into(),
        DiffSyntaxLanguage::Toml => return None,
    })
}

fn tree_sitter_highlight_spec(
    language: DiffSyntaxLanguage,
) -> Option<&'static TreesitterHighlightSpec> {
    static HTML: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static CSS: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static RUST: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static PY: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static GO: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static JSON: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static YAML: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static TS: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static TSX: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static JS: OnceLock<TreesitterHighlightSpec> = OnceLock::new();
    static BASH: OnceLock<TreesitterHighlightSpec> = OnceLock::new();

    let init = |language: tree_sitter::Language, source: &'static str| -> TreesitterHighlightSpec {
        let query =
            tree_sitter::Query::new(&language, source).expect("highlights.scm should compile");
        let capture_kinds = query
            .capture_names()
            .iter()
            .map(|name| syntax_kind_from_capture_name(name))
            .collect::<Vec<_>>();
        TreesitterHighlightSpec {
            query,
            capture_kinds,
        }
    };

    Some(match language {
        DiffSyntaxLanguage::Markdown => return None,
        DiffSyntaxLanguage::Html => HTML.get_or_init(|| {
            init(
                tree_sitter_html::LANGUAGE.into(),
                tree_sitter_html::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Css => CSS.get_or_init(|| {
            init(
                tree_sitter_css::LANGUAGE.into(),
                tree_sitter_css::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Hcl => return None,
        DiffSyntaxLanguage::Bicep => return None,
        DiffSyntaxLanguage::Lua => return None,
        DiffSyntaxLanguage::Makefile => return None,
        DiffSyntaxLanguage::Kotlin => return None,
        DiffSyntaxLanguage::Zig => return None,
        DiffSyntaxLanguage::Rust => RUST.get_or_init(|| {
            init(
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Python => PY.get_or_init(|| {
            init(
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Go => GO.get_or_init(|| {
            init(
                tree_sitter_go::LANGUAGE.into(),
                tree_sitter_go::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::C => return None,
        DiffSyntaxLanguage::Cpp => return None,
        DiffSyntaxLanguage::CSharp => return None,
        DiffSyntaxLanguage::FSharp => return None,
        DiffSyntaxLanguage::VisualBasic => return None,
        DiffSyntaxLanguage::Java => return None,
        DiffSyntaxLanguage::Php => return None,
        DiffSyntaxLanguage::Ruby => return None,
        DiffSyntaxLanguage::Json => JSON.get_or_init(|| {
            init(
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Yaml => YAML.get_or_init(|| {
            init(
                tree_sitter_yaml::LANGUAGE.into(),
                tree_sitter_yaml::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::TypeScript => TS.get_or_init(|| {
            init(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Tsx => TSX.get_or_init(|| {
            init(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::JavaScript => JS.get_or_init(|| {
            init(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            )
        }),
        DiffSyntaxLanguage::Bash => BASH.get_or_init(|| {
            init(
                tree_sitter_bash::LANGUAGE.into(),
                tree_sitter_bash::HIGHLIGHT_QUERY,
            )
        }),
        DiffSyntaxLanguage::Sql => return None,
        DiffSyntaxLanguage::Toml => return None,
    })
}

fn syntax_kind_from_capture_name(name: &str) -> Option<SyntaxTokenKind> {
    let base = name.split('.').next().unwrap_or(name);
    Some(match base {
        "comment" => SyntaxTokenKind::Comment,
        "string" | "character" => SyntaxTokenKind::String,
        "keyword" => SyntaxTokenKind::Keyword,
        "include" | "preproc" => SyntaxTokenKind::Keyword,
        "number" => SyntaxTokenKind::Number,
        "boolean" => SyntaxTokenKind::Constant,
        "function" | "constructor" | "method" => SyntaxTokenKind::Function,
        "type" => SyntaxTokenKind::Type,
        // Tree-sitter highlight queries often capture most identifiers as `variable.*`.
        // Coloring these makes Rust diffs look like "everything is blue", so we skip them.
        "variable" => return None,
        "property" | "field" | "attribute" => SyntaxTokenKind::Property,
        "tag" | "namespace" | "selector" => SyntaxTokenKind::Type,
        "constant" => SyntaxTokenKind::Constant,
        "punctuation" | "operator" => SyntaxTokenKind::Punctuation,
        _ => return None,
    })
}

fn syntax_tokens_for_line_heuristic(text: &str, language: DiffSyntaxLanguage) -> Vec<SyntaxToken> {
    let mut tokens: Vec<SyntaxToken> = Vec::new();
    let len = text.len();
    let mut i = 0usize;

    let is_ident_start = |ch: char| ch == '_' || ch.is_ascii_alphabetic();
    let is_ident_continue = |ch: char| ch == '_' || ch.is_ascii_alphanumeric();
    let is_digit = |ch: char| ch.is_ascii_digit();

    while i < len {
        let rest = &text[i..];

        if matches!(language, DiffSyntaxLanguage::Html) && rest.starts_with("<!--") {
            let end = rest.find("-->").map(|ix| i + ix + 3).unwrap_or(len);
            tokens.push(SyntaxToken {
                range: i..end,
                kind: SyntaxTokenKind::Comment,
            });
            i = end;
            continue;
        }

        if matches!(language, DiffSyntaxLanguage::FSharp) && rest.starts_with("(*") {
            let end = rest.find("*)").map(|ix| i + ix + 2).unwrap_or(len);
            tokens.push(SyntaxToken {
                range: i..end,
                kind: SyntaxTokenKind::Comment,
            });
            i = end;
            continue;
        }

        if matches!(language, DiffSyntaxLanguage::Lua) && rest.starts_with("--") {
            if rest.starts_with("--[[") {
                let end = rest.find("]]").map(|ix| i + ix + 2).unwrap_or(len);
                tokens.push(SyntaxToken {
                    range: i..end,
                    kind: SyntaxTokenKind::Comment,
                });
                i = end;
                continue;
            }
            tokens.push(SyntaxToken {
                range: i..len,
                kind: SyntaxTokenKind::Comment,
            });
            break;
        }

        let (line_comment, hash_comment, block_comment) = match language {
            DiffSyntaxLanguage::Python | DiffSyntaxLanguage::Toml | DiffSyntaxLanguage::Yaml => {
                (None, Some('#'), false)
            }
            DiffSyntaxLanguage::Markdown => (None, None, false),
            DiffSyntaxLanguage::Bash => (None, Some('#'), false),
            DiffSyntaxLanguage::Makefile => (None, Some('#'), false),
            DiffSyntaxLanguage::Sql => (Some("--"), None, true),
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
            | DiffSyntaxLanguage::Bicep => (Some("//"), None, true),
            DiffSyntaxLanguage::Hcl => (Some("//"), Some('#'), true),
            DiffSyntaxLanguage::Php => (Some("//"), Some('#'), true),
            DiffSyntaxLanguage::Ruby
            | DiffSyntaxLanguage::FSharp
            | DiffSyntaxLanguage::VisualBasic
            | DiffSyntaxLanguage::Html
            | DiffSyntaxLanguage::Css => (None, None, false),
            DiffSyntaxLanguage::Json => (None, None, false),
            DiffSyntaxLanguage::Lua => (None, None, false),
        };

        if let Some(prefix) = line_comment
            && rest.starts_with(prefix)
        {
            tokens.push(SyntaxToken {
                range: i..len,
                kind: SyntaxTokenKind::Comment,
            });
            break;
        }

        if block_comment && rest.starts_with("/*") {
            let end = rest.find("*/").map(|ix| i + ix + 2).unwrap_or(len);
            tokens.push(SyntaxToken {
                range: i..end,
                kind: SyntaxTokenKind::Comment,
            });
            i = end;
            continue;
        }

        if matches!(language, DiffSyntaxLanguage::Ruby) && rest.starts_with('#') {
            tokens.push(SyntaxToken {
                range: i..len,
                kind: SyntaxTokenKind::Comment,
            });
            break;
        }

        if matches!(language, DiffSyntaxLanguage::VisualBasic)
            && (rest.starts_with('\'')
                || rest
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rem ")))
        {
            tokens.push(SyntaxToken {
                range: i..len,
                kind: SyntaxTokenKind::Comment,
            });
            break;
        }

        if let Some('#') = hash_comment
            && rest.starts_with('#')
        {
            tokens.push(SyntaxToken {
                range: i..len,
                kind: SyntaxTokenKind::Comment,
            });
            break;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };

        if ch == '"'
            || ch == '\''
            || (ch == '`'
                && matches!(
                    language,
                    DiffSyntaxLanguage::JavaScript
                        | DiffSyntaxLanguage::TypeScript
                        | DiffSyntaxLanguage::Tsx
                        | DiffSyntaxLanguage::Go
                        | DiffSyntaxLanguage::Bash
                        | DiffSyntaxLanguage::Sql
                ))
        {
            let quote = ch;
            let mut j = i + quote.len_utf8();
            let mut escaped = false;
            while j < len {
                let Some(next) = text[j..].chars().next() else {
                    break;
                };
                let next_len = next.len_utf8();
                if escaped {
                    escaped = false;
                    j += next_len;
                    continue;
                }
                if next == '\\' {
                    escaped = true;
                    j += next_len;
                    continue;
                }
                if next == quote {
                    j += next_len;
                    break;
                }
                j += next_len;
            }

            tokens.push(SyntaxToken {
                range: i..j.min(len),
                kind: SyntaxTokenKind::String,
            });
            i = j.min(len);
            continue;
        }

        if ch.is_ascii_digit() {
            let mut j = i;
            while j < len {
                let Some(next) = text[j..].chars().next() else {
                    break;
                };
                if is_digit(next) || next == '_' || next == '.' || next == 'x' || next == 'b' {
                    j += next.len_utf8();
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

        if is_ident_start(ch) {
            let mut j = i + ch.len_utf8();
            while j < len {
                let Some(next) = text[j..].chars().next() else {
                    break;
                };
                if is_ident_continue(next) {
                    j += next.len_utf8();
                } else {
                    break;
                }
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

        if matches!(language, DiffSyntaxLanguage::Css) && (ch == '.' || ch == '#') {
            let mut j = i + 1;
            while j < len {
                let Some(next) = text[j..].chars().next() else {
                    break;
                };
                if is_ident_continue(next) || next == '-' {
                    j += next.len_utf8();
                } else {
                    break;
                }
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

        i += ch.len_utf8();
    }

    tokens
}

fn is_keyword(language: DiffSyntaxLanguage, ident: &str) -> bool {
    // NOTE: This is a heuristic fallback when we don't want to use tree-sitter for a line.
    match language {
        DiffSyntaxLanguage::Markdown => false,
        DiffSyntaxLanguage::Html => matches!(ident, "true" | "false"),
        DiffSyntaxLanguage::Css => matches!(ident, "true" | "false"),
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
        DiffSyntaxLanguage::Json => matches!(ident, "true" | "false" | "null"),
        DiffSyntaxLanguage::Toml => matches!(ident, "true" | "false"),
        DiffSyntaxLanguage::Yaml => matches!(ident, "true" | "false" | "null"),
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

    static DEFERRED_DROP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_deferred_drop_test() -> std::sync::MutexGuard<'static, ()> {
        match DEFERRED_DROP_TEST_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn synthetic_line_tokens_for_drop_test(
        lines: usize,
        tokens_per_line: usize,
    ) -> Vec<Vec<SyntaxToken>> {
        benchmark_line_tokens_payload(lines, tokens_per_line, 0)
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
        let input = collect_treesitter_document_input(lines.iter().copied());
        let line_starts = treesitter_document_line_starts(&input.line_lengths);
        let passes =
            treesitter_document_query_passes(&line_starts, &input.line_lengths, input.text.len());
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
        let lines = [long_line.as_str()];
        let input = collect_treesitter_document_input(lines.iter().copied());
        let line_starts = treesitter_document_line_starts(&input.line_lengths);
        let passes =
            treesitter_document_query_passes(&line_starts, &input.line_lengths, input.text.len());

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
    fn xml_uses_html_highlighting() {
        assert_eq!(
            diff_syntax_language_for_path("foo.xml"),
            Some(DiffSyntaxLanguage::Html)
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
    fn treesitter_variable_capture_is_not_colored() {
        assert_eq!(super::syntax_kind_from_capture_name("variable"), None);
        assert_eq!(
            super::syntax_kind_from_capture_name("variable.parameter"),
            None
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
    fn prepared_document_preserves_multiline_treesitter_context() {
        let lines = ["/* open comment", "still comment */ let x = 1;"];
        let doc = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            lines.iter().copied(),
        )
        .expect("rust should support tree-sitter document preparation");

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
    fn prepared_document_cache_keeps_multiple_documents_available() {
        let first_doc = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            ["/* one */ let a = 1;"].iter().copied(),
        )
        .expect("first document should prepare");
        let second_doc = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            ["/* two */ let b = 2;"].iter().copied(),
        )
        .expect("second document should prepare");

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
        let document = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            lines.iter().map(String::as_str),
        )
        .expect("document should prepare");

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
    fn prepared_document_chunk_hit_does_not_clone_tree_state() {
        reset_prepared_syntax_cache();
        reset_tree_state_clone_count();
        let lines = (0..(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS * 2))
            .map(|ix| format!("let chunk_clone_probe_{ix} = {ix};"))
            .collect::<Vec<_>>();
        let document = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            lines.iter().map(String::as_str),
        )
        .expect("document should prepare");

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
    fn treesitter_document_cache_lru_touch_keeps_recent_entry_alive() {
        for trial in 0..128usize {
            let mut cache = TreesitterDocumentCache::new();
            for key in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
                cache.insert_document(make_test_cache_key(key as u64), vec![Vec::new()]);
            }

            let touched_key = make_test_cache_key(0);
            assert!(cache.contains_document(touched_key, 1));
            cache.insert_document(make_test_cache_key(10_000 + trial as u64), vec![Vec::new()]);

            assert!(
                cache.contains_key(touched_key),
                "touched key should survive eviction on trial {trial}"
            );
        }
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
        let line_starts = treesitter_document_line_starts(&[5, 4]);
        assert_eq!(
            treesitter_point_for_byte(&line_starts, input, input.len()),
            tree_sitter::Point::new(2, 0),
            "EOF for newline-terminated input should point to the next row start"
        );
    }

    #[test]
    fn small_reparse_reuses_old_tree_with_input_edit() {
        let _lock = lock_deferred_drop_test();
        reset_deferred_drop_counters();
        let base_lines = vec!["let value = 1;".to_string(); 256];
        let base_document = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            base_lines.iter().map(String::as_str),
        )
        .expect("base document should parse");
        let base_version =
            prepared_document_source_version(base_document).expect("base source version");
        assert_eq!(
            prepared_document_parse_mode(base_document),
            Some(TreesitterParseReuseMode::Full)
        );

        let mut edited = base_lines.clone();
        edited[42].push_str(" // tiny edit");
        let attempt = prepare_treesitter_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited.iter().map(String::as_str),
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
    fn large_reparse_falls_back_to_full_parse() {
        let _lock = lock_deferred_drop_test();
        reset_deferred_drop_counters();
        let base_lines = vec!["let value = 1;".to_string(); 256];
        let base_document = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            base_lines.iter().map(String::as_str),
        )
        .expect("base document should parse");

        let mut edited = base_lines.clone();
        for line in edited.iter_mut().take(180) {
            *line = "pub fn massive_fallback_path() { let x = vec![1,2,3,4]; }".to_string();
        }
        let attempt = prepare_treesitter_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited.iter().map(String::as_str),
            DiffSyntaxBudget {
                foreground_parse: Duration::from_millis(50),
            },
            Some(base_document),
        );
        let PrepareTreesitterDocumentResult::Ready(reparsed_document) = attempt else {
            panic!("large reparse should complete within default budget");
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
    fn incremental_reparse_append_line_matches_full_parse_tokens() {
        let _lock = lock_deferred_drop_test();
        reset_deferred_drop_counters();

        let base_lines = vec!["let value = 41;".to_string(); 256];
        let base_document = prepare_treesitter_document(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            base_lines.iter().map(String::as_str),
        )
        .expect("base document should parse");

        let mut edited = base_lines.clone();
        edited.push("let appended = 42;".to_string());
        let attempt = prepare_treesitter_document_with_budget_reuse(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited.iter().map(String::as_str),
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

        let request = treesitter_document_parse_request(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            edited.iter().map(String::as_str),
        )
        .expect("edited rust lines should produce parse request");
        let full_tree = TS_PARSER
            .with(|parser| {
                let mut parser = parser.borrow_mut();
                parser.set_language(&request.ts_language).ok()?;
                parse_treesitter_tree(&mut parser, request.input.text.as_bytes(), None, None)
            })
            .expect("full parse should succeed");
        let highlight =
            tree_sitter_highlight_spec(request.language).expect("rust highlight spec should exist");

        let full_tokens = collect_treesitter_document_line_tokens(
            &full_tree,
            highlight,
            request.input.text.as_bytes(),
            &request.input.line_lengths,
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
        let _lock = lock_deferred_drop_test();
        reset_deferred_drop_counters();

        let mut cache = TreesitterDocumentCache::new();
        cache.insert_document(
            make_test_cache_key(1),
            synthetic_line_tokens_for_drop_test(2_048, 8),
        );
        let (queued_before, dropped_before, _) = deferred_drop_counters();

        cache.insert_document(
            make_test_cache_key(1),
            synthetic_line_tokens_for_drop_test(2_048, 8),
        );
        let (queued_after, _, _) = deferred_drop_counters();
        assert!(
            queued_after > queued_before,
            "large replacement should enqueue deferred drop work"
        );

        assert!(
            flush_deferred_syntax_cache_drop_queue(),
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
        let _lock = lock_deferred_drop_test();
        reset_deferred_drop_counters();

        let mut cache = TreesitterDocumentCache::new();
        cache.insert_document(
            make_test_cache_key(1),
            synthetic_line_tokens_for_drop_test(8, 1),
        );
        let (_, _, inline_before) = deferred_drop_counters();

        cache.insert_document(
            make_test_cache_key(1),
            synthetic_line_tokens_for_drop_test(8, 1),
        );
        let (_, _, inline_after) = deferred_drop_counters();
        assert!(
            inline_after > inline_before,
            "small replacement should drop old payload inline"
        );
    }

    #[test]
    fn large_cache_eviction_uses_deferred_drop_queue() {
        let _lock = lock_deferred_drop_test();
        reset_deferred_drop_counters();

        let mut cache = TreesitterDocumentCache::new();
        for key in 0..TS_DOCUMENT_CACHE_MAX_ENTRIES {
            cache.insert_document(
                make_test_cache_key(key as u64),
                synthetic_line_tokens_for_drop_test(2_048, 8),
            );
        }
        let (queued_before, dropped_before, _) = deferred_drop_counters();

        cache.insert_document(
            make_test_cache_key(TS_DOCUMENT_CACHE_MAX_ENTRIES as u64 + 1),
            synthetic_line_tokens_for_drop_test(2_048, 8),
        );
        let (queued_after, _, _) = deferred_drop_counters();
        assert!(
            queued_after > queued_before,
            "large eviction should enqueue deferred drop work"
        );

        assert!(
            flush_deferred_syntax_cache_drop_queue(),
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
        let lines = vec!["/* budget */ let value = Some(42);"; 2_048];
        let attempt = prepare_treesitter_document_with_budget(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            lines.iter().copied(),
            DiffSyntaxBudget {
                foreground_parse: Duration::ZERO,
            },
        );
        assert_eq!(attempt, PrepareTreesitterDocumentResult::TimedOut);

        let prepared = prepare_treesitter_document_in_background(
            DiffSyntaxLanguage::Rust,
            DiffSyntaxMode::Auto,
            lines.iter().copied(),
        )
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
    fn background_prepared_document_not_in_tls_until_injected() {
        let lines = vec![
            "/* background comment */".to_string(),
            "let value = 42;".to_string(),
        ];
        let prepared = std::thread::spawn({
            let lines = lines.clone();
            move || {
                prepare_treesitter_document_in_background(
                    DiffSyntaxLanguage::Rust,
                    DiffSyntaxMode::Auto,
                    lines.iter().map(String::as_str),
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
    #[ignore]
    fn perf_treesitter_tokenization_smoke() {
        let text = "fn main() { let x = Some(123); println!(\"{x:?}\"); }";
        let start = Instant::now();
        for _ in 0..200_000 {
            let _ = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
        }
        eprintln!("syntax_tokens_for_line (rust): {:?}", start.elapsed());
    }
}
