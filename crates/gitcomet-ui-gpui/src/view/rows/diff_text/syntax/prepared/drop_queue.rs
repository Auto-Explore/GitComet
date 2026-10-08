use super::*;
use crate::view::rows::LruTouchQueue;

#[derive(Clone, Copy)]
pub(crate) enum SyntaxCacheDropMode {
    DeferredWhenLarge,
    #[cfg(feature = "benchmarks")]
    InlineWhenLarge,
}

pub(crate) enum SyntaxCacheDropMessage {
    Drop(SyntaxCacheDropPayload),
    #[cfg(any(test, feature = "benchmarks"))]
    Flush(mpsc::Sender<()>),
}

pub(crate) struct SyntaxCacheDropPayload {
    pub(crate) line_tokens: Vec<Arc<[SyntaxToken]>>,
    pub(crate) estimated_bytes: usize,
    #[cfg(test)]
    counters: Arc<SyntaxDropCounters>,
}

impl SyntaxCacheDropPayload {
    pub(crate) fn new(line_tokens: Vec<Arc<[SyntaxToken]>>, estimated_bytes: usize) -> Self {
        Self {
            line_tokens,
            estimated_bytes,
            #[cfg(test)]
            counters: syntax_drop_counters(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PreparedSyntaxChunkKey {
    pub(crate) cache_key: PreparedSyntaxCacheKey,
    pub(crate) chunk_ix: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSyntaxChunkBuildRequest {
    pub(crate) chunk_key: PreparedSyntaxChunkKey,
    pub(crate) line_count: usize,
    pub(crate) thread_id: std::thread::ThreadId,
    pub(crate) tree_state: Arc<PreparedSyntaxTreeState>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSyntaxChunkBuildResult {
    pub(crate) chunk_key: PreparedSyntaxChunkKey,
    pub(crate) chunk_tokens: Option<Vec<Arc<[SyntaxToken]>>>,
    pub(crate) chunk_build_ms: u64,
    pub(crate) thread_id: std::thread::ThreadId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreparedSyntaxLineTokensRequest {
    Ready(Arc<[SyntaxToken]>),
    Pending,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PreparedSyntaxLineTokensRangeSummary {
    pub ready_lines: usize,
    pub ready_tokens: usize,
}

#[derive(Clone)]
pub(crate) struct CachedSingleLineSyntaxTokens {
    pub(crate) text: Arc<str>,
    pub(crate) tokens: Arc<[SyntaxToken]>,
}

pub(crate) struct SingleLineSyntaxTokenCache {
    pub(crate) by_key: FxHashMap<SingleLineSyntaxTokenCacheKey, CachedSingleLineSyntaxTokens>,
    pub(crate) lru_order: LruTouchQueue<SingleLineSyntaxTokenCacheKey>,
}

impl SingleLineSyntaxTokenCache {
    pub(crate) fn new() -> Self {
        Self {
            by_key: FxHashMap::default(),
            lru_order: LruTouchQueue::default(),
        }
    }

    pub(crate) fn touch_key(&mut self, key: SingleLineSyntaxTokenCacheKey) {
        self.lru_order.touch(key);
    }

    pub(crate) fn remove_key(&mut self, key: SingleLineSyntaxTokenCacheKey) {
        self.by_key.remove(&key);
        self.lru_order.remove(&key);
    }

    pub(crate) fn get(
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

    pub(crate) fn insert(
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
            let Some(evicted) = self.lru_order.pop_oldest() else {
                break;
            };
            self.by_key.remove(&evicted);
        }
    }
}

#[cfg(test)]
#[derive(Default)]
struct SyntaxDropCounters {
    enqueued: std::sync::atomic::AtomicUsize,
    completed: std::sync::atomic::AtomicUsize,
    inline: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
thread_local! {
    static DROP_COUNTERS: std::cell::RefCell<Arc<SyntaxDropCounters>> =
        std::cell::RefCell::new(Arc::default());
    static TEST_SCOPE: std::cell::RefCell<SyntaxTestScope> =
        std::cell::RefCell::new(SyntaxTestScope::new());
}

#[cfg(test)]
fn syntax_drop_counters() -> Arc<SyntaxDropCounters> {
    // Token caches can be dropped during TLS destruction, after this cell.
    DROP_COUNTERS
        .try_with(|cell| Arc::clone(&cell.borrow()))
        .unwrap_or_default()
}

#[cfg(test)]
static ACTIVE_TEST_SCOPES: std::sync::LazyLock<Mutex<std::collections::HashSet<u64>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

#[cfg(test)]
struct SyntaxTestScope(u64);

#[cfg(test)]
impl SyntaxTestScope {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ACTIVE_TEST_SCOPES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id);
        Self(id)
    }
}

#[cfg(test)]
impl Drop for SyntaxTestScope {
    fn drop(&mut self) {
        let mut active = ACTIVE_TEST_SCOPES.lock().unwrap_or_else(|e| e.into_inner());
        active.remove(&self.0);
        if let Some(store) = SHARED_DOCUMENT_SEEDS.get() {
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|key, _| key.test_scope != self.0);
        }
    }
}

#[cfg(test)]
pub(crate) fn syntax_test_scope() -> u64 {
    TEST_SCOPE.with(|scope| scope.borrow().0)
}

#[cfg(test)]
pub(crate) fn reset_syntax_test_scope() {
    TEST_SCOPE.with(|scope| *scope.borrow_mut() = SyntaxTestScope::new());
}

#[cfg(test)]
pub(crate) fn syntax_test_scope_is_active(key: PreparedSyntaxCacheKey) -> bool {
    ACTIVE_TEST_SCOPES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&key.test_scope)
}
pub(crate) fn syntax_cache_drop_sender() -> Option<&'static mpsc::Sender<SyntaxCacheDropMessage>> {
    pub(crate) static SENDER: OnceLock<Option<mpsc::Sender<SyntaxCacheDropMessage>>> =
        OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<SyntaxCacheDropMessage>();
            let builder = std::thread::Builder::new().name("gitcomet-syntax-drop".to_string());
            let _handle = builder
                .spawn(move || {
                    while let Ok(msg) = rx.recv() {
                        match msg {
                            SyntaxCacheDropMessage::Drop(drop_payload) => {
                                #[cfg(test)]
                                let counters = Arc::clone(&drop_payload.counters);
                                drop(drop_payload);
                                #[cfg(test)]
                                counters
                                    .completed
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

static SHARED_DOCUMENT_SEEDS: OnceLock<
    Mutex<FxHashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>>,
> = OnceLock::new();

pub(crate) fn shared_prepared_document_seed_store()
-> &'static Mutex<FxHashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>> {
    SHARED_DOCUMENT_SEEDS.get_or_init(|| Mutex::new(FxHashMap::default()))
}

pub(crate) fn store_shared_prepared_document_seed(document: &PreparedSyntaxDocumentData) {
    // Hold the scope lock through insertion so a retiring generation cannot
    // repopulate its seeds after cleanup. Foreign-thread handles keep their key.
    #[cfg(test)]
    let active = ACTIVE_TEST_SCOPES.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(test)]
    if !active.contains(&document.cache_key.test_scope) {
        return;
    }
    let mut store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let belongs_to_scope = |key: &&PreparedSyntaxCacheKey| {
        #[cfg(test)]
        {
            key.test_scope == document.cache_key.test_scope
        }
        #[cfg(not(test))]
        {
            let _ = key;
            true
        }
    };
    #[cfg(test)]
    let at_capacity =
        store.keys().filter(belongs_to_scope).count() >= TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES;
    #[cfg(not(test))]
    let at_capacity = store.len() >= TS_SHARED_DOCUMENT_SEED_MAX_ENTRIES;
    if at_capacity
        && let Some(evict_key) = store.keys().find(belongs_to_scope).copied()
        && evict_key != document.cache_key
    {
        store.remove(&evict_key);
    }
    store.insert(document.cache_key, document.clone());
}

pub(crate) fn merge_shared_prepared_document_chunk(
    cache_key: PreparedSyntaxCacheKey,
    chunk_ix: usize,
    chunk_tokens: Option<Vec<Arc<[SyntaxToken]>>>,
) {
    let mut store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(document) = store.get_mut(&cache_key) else {
        return;
    };
    if document.line_token_chunks.contains_key(&chunk_ix) {
        return;
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
}

pub(crate) struct PreparedSyntaxChunkWorker {
    pub(crate) sender: mpsc::Sender<PreparedSyntaxChunkBuildRequest>,
    pub(crate) receiver: Mutex<mpsc::Receiver<PreparedSyntaxChunkBuildResult>>,
    pub(crate) deferred_results: Mutex<VecDeque<PreparedSyntaxChunkBuildResult>>,
}

pub(crate) fn syntax_chunk_worker() -> Option<&'static PreparedSyntaxChunkWorker> {
    pub(crate) static WORKER: OnceLock<Option<PreparedSyntaxChunkWorker>> = OnceLock::new();
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

pub(crate) fn estimated_line_tokens_allocation_bytes(line_tokens: &[Arc<[SyntaxToken]>]) -> usize {
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

pub(crate) fn estimated_chunked_line_tokens_allocation_bytes(
    line_token_chunks: &FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
) -> usize {
    line_token_chunks.values().fold(0usize, |acc, chunk| {
        acc.saturating_add(estimated_line_tokens_allocation_bytes(chunk))
    })
}

pub(crate) fn share_recent_line_token_arcs(
    line_tokens: Vec<Vec<SyntaxToken>>,
) -> Vec<Arc<[SyntaxToken]>> {
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

pub(crate) fn drop_line_tokens_with_mode(
    drop_payload: SyntaxCacheDropPayload,
    drop_mode: SyntaxCacheDropMode,
) {
    #[cfg(test)]
    let counters = Arc::clone(&drop_payload.counters);
    let should_try_deferred = matches!(drop_mode, SyntaxCacheDropMode::DeferredWhenLarge)
        && drop_payload.estimated_bytes >= TS_DEFERRED_DROP_MIN_BYTES;

    if should_try_deferred && let Some(sender) = syntax_cache_drop_sender() {
        #[cfg(test)]
        counters
            .enqueued
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if sender
            .send(SyntaxCacheDropMessage::Drop(drop_payload))
            .is_ok()
        {
            return;
        }
        #[cfg(test)]
        {
            counters
                .enqueued
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            counters
                .inline
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        return;
    }

    #[cfg(test)]
    counters
        .inline
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    drop(drop_payload);
}

#[cfg(test)]
pub(crate) fn deferred_drop_counters() -> (usize, usize, usize) {
    let counters = syntax_drop_counters();
    (
        counters.enqueued.load(std::sync::atomic::Ordering::Relaxed),
        counters
            .completed
            .load(std::sync::atomic::Ordering::Relaxed),
        counters.inline.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
pub(crate) fn reset_deferred_drop_counters() {
    DROP_COUNTERS.with(|cell| *cell.borrow_mut() = Arc::default());
    TS_INCREMENTAL_PARSE_COUNT.with(|count| count.set(0));
    TS_INCREMENTAL_FALLBACK_COUNT.with(|count| count.set(0));
    TS_DOCUMENT_HASH_COUNT.with(|count| count.set(0));
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn incremental_reparse_counters() -> (usize, usize) {
    (
        TS_INCREMENTAL_PARSE_COUNT.with(Cell::get),
        TS_INCREMENTAL_FALLBACK_COUNT.with(Cell::get),
    )
}

#[cfg(test)]
pub(crate) fn tree_state_clone_count() -> usize {
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.get())
}

#[cfg(test)]
pub(crate) fn document_hash_count() -> usize {
    TS_DOCUMENT_HASH_COUNT.with(Cell::get)
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn flush_deferred_syntax_cache_drop_queue_with_timeout(timeout: Duration) -> bool {
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
pub(crate) fn benchmark_flush_deferred_drop_queue() -> bool {
    flush_deferred_syntax_cache_drop_queue_with_timeout(Duration::from_secs(2))
}

pub(crate) fn incremental_reparse_enabled() -> bool {
    pub(crate) static ENABLED: OnceLock<bool> = OnceLock::new();
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
pub(crate) struct PreparedSyntaxCacheMetrics {
    pub(crate) hit: u64,
    pub(crate) miss: u64,
    pub(crate) evict: u64,
    pub(crate) chunk_build_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct TreesitterCachedDocument {
    pub(crate) line_count: usize,
    pub(crate) line_token_chunks: FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    pub(crate) line_token_bytes: usize,
    pub(crate) tree_state: Option<PreparedSyntaxTreeState>,
}
