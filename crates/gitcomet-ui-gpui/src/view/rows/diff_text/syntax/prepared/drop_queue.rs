use super::*;

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
}

impl SyntaxCacheDropPayload {
    pub(crate) fn new(line_tokens: Vec<Arc<[SyntaxToken]>>, estimated_bytes: usize) -> Self {
        Self {
            line_tokens,
            estimated_bytes,
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
    pub(crate) lru_order: VecDeque<SingleLineSyntaxTokenCacheKey>,
}

impl SingleLineSyntaxTokenCache {
    pub(crate) fn new() -> Self {
        Self {
            by_key: FxHashMap::default(),
            lru_order: VecDeque::new(),
        }
    }

    pub(crate) fn touch_key(&mut self, key: SingleLineSyntaxTokenCacheKey) {
        if self.lru_order.back() == Some(&key) {
            return;
        }
        if let Some(pos) = self.lru_order.iter().position(|existing| *existing == key) {
            self.lru_order.remove(pos);
        }
        self.lru_order.push_back(key);
    }

    pub(crate) fn remove_key(&mut self, key: SingleLineSyntaxTokenCacheKey) {
        self.by_key.remove(&key);
        if let Some(pos) = self.lru_order.iter().position(|existing| *existing == key) {
            self.lru_order.remove(pos);
        }
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
            let Some(evicted) = self.lru_order.pop_front() else {
                break;
            };
            self.by_key.remove(&evicted);
        }
    }
}

#[cfg(test)]
pub(crate) static TS_DEFERRED_DROP_ENQUEUED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static TS_DEFERRED_DROP_COMPLETED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static TS_INLINE_DROP_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
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

pub(crate) fn shared_prepared_document_seed_store()
-> &'static Mutex<FxHashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>> {
    pub(crate) static STORE: OnceLock<
        Mutex<FxHashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>>,
    > = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

pub(crate) fn store_shared_prepared_document_seed(document: &PreparedSyntaxDocumentData) {
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
pub(crate) fn deferred_drop_counters() -> (usize, usize, usize) {
    (
        TS_DEFERRED_DROP_ENQUEUED.load(std::sync::atomic::Ordering::Relaxed),
        TS_DEFERRED_DROP_COMPLETED.load(std::sync::atomic::Ordering::Relaxed),
        TS_INLINE_DROP_COUNT.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
pub(crate) fn reset_deferred_drop_counters() {
    TS_DEFERRED_DROP_ENQUEUED.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_DEFERRED_DROP_COMPLETED.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_INLINE_DROP_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
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
