use super::*;

#[derive(Clone, Copy)]
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

pub(super) struct SyntaxCacheDropPayload {
    pub(super) line_tokens: Vec<Arc<[SyntaxToken]>>,
    pub(super) estimated_bytes: usize,
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
pub(in super::super) enum PreparedSyntaxLineTokensRequest {
    Ready(Arc<[SyntaxToken]>),
    Pending,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in super::super) struct PreparedSyntaxLineTokensRangeSummary {
    pub ready_lines: usize,
    pub ready_tokens: usize,
}

#[derive(Clone)]
pub(super) struct CachedSingleLineSyntaxTokens {
    text: Arc<str>,
    tokens: Arc<[SyntaxToken]>,
}

pub(super) struct SingleLineSyntaxTokenCache {
    pub(super) by_key: FxHashMap<SingleLineSyntaxTokenCacheKey, CachedSingleLineSyntaxTokens>,
    pub(super) lru_order: VecDeque<SingleLineSyntaxTokenCacheKey>,
}

impl SingleLineSyntaxTokenCache {
    pub(super) fn new() -> Self {
        Self {
            by_key: FxHashMap::default(),
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

    pub(super) fn get(
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

    pub(super) fn insert(
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
-> &'static Mutex<FxHashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>> {
    static STORE: OnceLock<Mutex<FxHashMap<PreparedSyntaxCacheKey, PreparedSyntaxDocumentData>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(FxHashMap::default()))
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

fn merge_shared_prepared_document_chunk(
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

pub(super) fn estimated_line_tokens_allocation_bytes(line_tokens: &[Arc<[SyntaxToken]>]) -> usize {
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
    line_token_chunks: &FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
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
pub(super) fn deferred_drop_counters() -> (usize, usize, usize) {
    (
        TS_DEFERRED_DROP_ENQUEUED.load(std::sync::atomic::Ordering::Relaxed),
        TS_DEFERRED_DROP_COMPLETED.load(std::sync::atomic::Ordering::Relaxed),
        TS_INLINE_DROP_COUNT.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
pub(super) fn reset_deferred_drop_counters() {
    TS_DEFERRED_DROP_ENQUEUED.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_DEFERRED_DROP_COMPLETED.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_INLINE_DROP_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
    TS_INCREMENTAL_PARSE_COUNT.with(|count| count.set(0));
    TS_INCREMENTAL_FALLBACK_COUNT.with(|count| count.set(0));
    TS_DOCUMENT_HASH_COUNT.with(|count| count.set(0));
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn incremental_reparse_counters() -> (usize, usize) {
    (
        TS_INCREMENTAL_PARSE_COUNT.with(Cell::get),
        TS_INCREMENTAL_FALLBACK_COUNT.with(Cell::get),
    )
}

#[cfg(test)]
pub(super) fn tree_state_clone_count() -> usize {
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.get())
}

#[cfg(test)]
pub(super) fn document_hash_count() -> usize {
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
pub(in super::super) fn benchmark_flush_deferred_drop_queue() -> bool {
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
pub(super) struct PreparedSyntaxCacheMetrics {
    pub(super) hit: u64,
    pub(super) miss: u64,
    pub(super) evict: u64,
    pub(super) chunk_build_ms: u64,
}

#[derive(Clone, Debug)]
pub(super) struct TreesitterCachedDocument {
    pub(super) line_count: usize,
    pub(super) line_token_chunks: FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    pub(super) line_token_bytes: usize,
    pub(super) tree_state: Option<PreparedSyntaxTreeState>,
}

impl TreesitterCachedDocument {
    pub(super) fn from_chunked_line_tokens(
        line_count: usize,
        line_token_chunks: FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
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
    pub(super) fn from_line_tokens(
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

    pub(super) fn into_drop_payload(self) -> SyntaxCacheDropPayload {
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
) -> FxHashMap<usize, Vec<Arc<[SyntaxToken]>>> {
    if line_tokens.is_empty() {
        return FxHashMap::default();
    }

    let mut chunks = FxHashMap::default();
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

pub(super) fn insert_line_token_chunk(
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
        tree_state.source_hash,
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

pub(super) struct TreesitterDocumentCache {
    by_cache_key: FxHashMap<PreparedSyntaxCacheKey, TreesitterCachedDocument>,
    by_source_identity: FxHashMap<PreparedSyntaxSourceIdentity, PreparedSyntaxCacheKey>,
    lru_order: VecDeque<PreparedSyntaxCacheKey>,
    pending_chunk_requests: FxHashSet<PreparedSyntaxChunkKey>,
    pending_chunk_request_counts: FxHashMap<PreparedSyntaxCacheKey, usize>,
    metrics: PreparedSyntaxCacheMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SharedDocumentMergeResult {
    None,
    Inserted,
    Updated,
}

impl TreesitterDocumentCache {
    pub(super) fn new() -> Self {
        Self {
            by_cache_key: FxHashMap::default(),
            by_source_identity: FxHashMap::default(),
            lru_order: VecDeque::new(),
            pending_chunk_requests: FxHashSet::default(),
            pending_chunk_request_counts: FxHashMap::default(),
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

    fn insert_pending_chunk_request(&mut self, chunk_key: PreparedSyntaxChunkKey) {
        if !self.pending_chunk_requests.insert(chunk_key) {
            return;
        }
        *self
            .pending_chunk_request_counts
            .entry(chunk_key.cache_key)
            .or_default() += 1;
    }

    fn remove_pending_chunk_request(&mut self, chunk_key: PreparedSyntaxChunkKey) -> bool {
        if !self.pending_chunk_requests.remove(&chunk_key) {
            return false;
        }
        if let std::collections::hash_map::Entry::Occupied(mut entry) =
            self.pending_chunk_request_counts.entry(chunk_key.cache_key)
        {
            let count = entry.get_mut();
            *count = count.saturating_sub(1);
            if *count == 0 {
                entry.remove();
            }
        }
        true
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

    pub(super) fn contains_document(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_count: usize,
    ) -> bool {
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
        let mut inserted = false;
        let mut updated = false;
        let mut remove_identity = None;
        let mut insert_identity = None;
        let mut replaced_drop_payload = None;
        let mut cleared_pending_chunks = Vec::new();

        {
            let store = match shared_prepared_document_seed_store().lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let Some(shared_document) = store.get(&cache_key) else {
                return SharedDocumentMergeResult::None;
            };

            match self.by_cache_key.entry(cache_key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let document = TreesitterCachedDocument::from_chunked_line_tokens(
                        shared_document.line_count,
                        shared_document.line_token_chunks.clone(),
                        shared_document.tree_state.clone(),
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
                                shared_document.line_token_chunks.clone(),
                                shared_document.tree_state.clone(),
                            ),
                        );
                        remove_identity = old_identity;
                        insert_identity = document.source_identity();
                        replaced_drop_payload = Some(replaced.into_drop_payload());
                        updated = true;
                    } else {
                        let old_identity = document.source_identity();
                        if document.tree_state.is_none()
                            && let Some(tree_state) = shared_document.tree_state.clone()
                        {
                            document.tree_state = Some(tree_state);
                            updated = true;
                        }

                        for (&chunk_ix, chunk) in &shared_document.line_token_chunks {
                            if document.line_token_chunks.contains_key(&chunk_ix) {
                                continue;
                            }
                            insert_line_token_chunk(document, chunk_ix, Some(chunk.clone()));
                            cleared_pending_chunks.push(PreparedSyntaxChunkKey {
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
        }

        if let Some(drop_payload) = replaced_drop_payload {
            drop_line_tokens_with_mode(drop_payload, SyntaxCacheDropMode::DeferredWhenLarge);
        }
        for chunk_key in cleared_pending_chunks {
            self.remove_pending_chunk_request(chunk_key);
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
        merge_shared_prepared_document_chunk(cache_key, chunk_ix, shared_chunk_tokens);
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
        self.insert_pending_chunk_request(chunk_key);
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
            self.queue_chunk_build_request_nonblocking(
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
        self.drain_completed_chunk_builds_for_cache_key(cache_key);
        let allow_sync_build_on_insert = matches!(
            self.merge_document_from_shared_seed(cache_key),
            SharedDocumentMergeResult::Inserted
        );
        self.request_line_tokens_with_context(cache_key, line_ix, allow_sync_build_on_insert)
    }

    fn request_line_tokens_range_into(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_range: Range<usize>,
        requests: &mut Vec<PreparedSyntaxLineTokensRequest>,
    ) -> Option<PreparedSyntaxLineTokensRangeSummary> {
        if line_range.is_empty() {
            requests.clear();
            return Some(PreparedSyntaxLineTokensRangeSummary::default());
        }

        self.drain_completed_chunk_builds_for_cache_key(cache_key);
        let mut allow_sync_build_on_insert = matches!(
            self.merge_document_from_shared_seed(cache_key),
            SharedDocumentMergeResult::Inserted
        );

        if let Some(summary) = self.collect_ready_line_token_requests_for_range(
            cache_key,
            line_range.clone(),
            requests,
        ) {
            self.metrics.hit = self.metrics.hit.saturating_add(summary.ready_lines as u64);
            self.touch_key(cache_key);
            return Some(summary);
        }

        requests.clear();
        let mut summary = PreparedSyntaxLineTokensRangeSummary::default();
        for line_ix in line_range {
            let request = self.request_line_tokens_with_context(
                cache_key,
                line_ix,
                allow_sync_build_on_insert,
            )?;
            if let PreparedSyntaxLineTokensRequest::Ready(tokens) = &request {
                summary.ready_lines = summary.ready_lines.saturating_add(1);
                summary.ready_tokens = summary.ready_tokens.saturating_add(tokens.len());
            }
            requests.push(request);
            allow_sync_build_on_insert = false;
        }
        Some(summary)
    }

    fn collect_ready_line_token_requests_for_range(
        &self,
        cache_key: PreparedSyntaxCacheKey,
        line_range: Range<usize>,
        requests: &mut Vec<PreparedSyntaxLineTokensRequest>,
    ) -> Option<PreparedSyntaxLineTokensRangeSummary> {
        static EMPTY_TOKENS: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
        let empty_tokens = || Arc::clone(EMPTY_TOKENS.get_or_init(|| Arc::from([])));

        let document = self.by_cache_key.get(&cache_key)?;
        let original_len = requests.len();
        requests.clear();
        requests.reserve(line_range.len());

        let mut current_chunk_ix = usize::MAX;
        let mut current_chunk_start = 0usize;
        let mut current_chunk: Option<&[Arc<[SyntaxToken]>]> = None;
        let mut summary = PreparedSyntaxLineTokensRangeSummary::default();

        for line_ix in line_range {
            if line_ix >= document.line_count {
                requests.push(PreparedSyntaxLineTokensRequest::Ready(empty_tokens()));
                summary.ready_lines = summary.ready_lines.saturating_add(1);
                continue;
            }

            let chunk_ix = line_ix / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
            if chunk_ix != current_chunk_ix {
                current_chunk_ix = chunk_ix;
                current_chunk_start = chunk_ix.saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
                current_chunk = document.line_token_chunks.get(&chunk_ix).map(Vec::as_slice);
            }

            let Some(chunk) = current_chunk else {
                requests.truncate(original_len);
                return None;
            };
            let line_offset = line_ix.saturating_sub(current_chunk_start);
            let tokens = chunk.get(line_offset).cloned().unwrap_or_else(empty_tokens);
            summary.ready_lines = summary.ready_lines.saturating_add(1);
            summary.ready_tokens = summary.ready_tokens.saturating_add(tokens.len());
            requests.push(PreparedSyntaxLineTokensRequest::Ready(tokens));
        }

        Some(summary)
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

    pub(super) fn tree_state(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> Option<PreparedSyntaxTreeState> {
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
    pub(super) fn contains_key(&self, cache_key: PreparedSyntaxCacheKey) -> bool {
        self.by_cache_key.contains_key(&cache_key)
    }

    fn drain_completed_chunk_builds(&mut self) -> usize {
        self.drain_completed_chunk_builds_matching(None)
    }

    fn drain_completed_chunk_builds_for_cache_key(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> usize {
        if !self.has_pending_chunk_requests_for_cache_key(cache_key) {
            return 0;
        }
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
                merge_shared_prepared_document_chunk(
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
                merge_shared_prepared_document_chunk(
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
            self.remove_pending_chunk_request(result.chunk_key);
            self.metrics.chunk_build_ms = self
                .metrics
                .chunk_build_ms
                .saturating_add(result.chunk_build_ms);
            let shared_chunk_tokens = result.chunk_tokens.clone();
            let Some(document) = self.by_cache_key.get_mut(&result.chunk_key.cache_key) else {
                merge_shared_prepared_document_chunk(
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
                merge_shared_prepared_document_chunk(
                    result.chunk_key.cache_key,
                    result.chunk_key.chunk_ix,
                    shared_chunk_tokens,
                );
                continue;
            }
            insert_line_token_chunk(document, result.chunk_key.chunk_ix, result.chunk_tokens);
            merge_shared_prepared_document_chunk(
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
        self.pending_chunk_request_counts.contains_key(&cache_key)
    }

    #[cfg(test)]
    pub(super) fn make_test_cache_key(doc_hash: u64) -> PreparedSyntaxCacheKey {
        PreparedSyntaxCacheKey {
            language: DiffSyntaxLanguage::Rust,
            doc_hash,
        }
    }

    #[cfg(test)]
    pub(super) fn insert_document(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_tokens: Vec<Vec<SyntaxToken>>,
    ) {
        self.insert_document_with_tree_state(cache_key, line_tokens, None);
    }

    #[cfg(test)]
    pub(super) fn insert_document_with_tree_state(
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
pub(super) struct TreesitterDocumentInput {
    pub(super) text: SharedString,
    pub(super) line_starts: Arc<[usize]>,
}

#[derive(Clone)]
pub(super) struct TreesitterDocumentParseRequest {
    pub(super) language: DiffSyntaxLanguage,
    pub(super) ts_language: tree_sitter::Language,
    pub(super) input: TreesitterDocumentInput,
    pub(super) cache_key: PreparedSyntaxCacheKey,
}

pub(super) struct PendingParseRequest {
    identity: PreparedSyntaxSourceIdentity,
    request: TreesitterDocumentParseRequest,
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in super::super) fn benchmark_reset_prepared_syntax_cache_metrics() {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().reset_metrics());
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in super::super) fn benchmark_prepared_syntax_cache_metrics() -> (u64, u64, u64, u64) {
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
pub(in super::super) fn benchmark_prepared_syntax_loaded_chunk_count(
    document: PreparedSyntaxDocument,
) -> Option<usize> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().loaded_chunk_count(document.cache_key))
}

#[cfg(feature = "benchmarks")]
pub(in super::super) fn benchmark_prepared_syntax_cache_contains_document(
    document: PreparedSyntaxDocument,
) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().contains_key(document.cache_key))
}

#[cfg(test)]
pub(super) fn prepared_syntax_cache_metrics() -> PreparedSyntaxCacheMetrics {
    let (hit, miss, evict, chunk_build_ms) = benchmark_prepared_syntax_cache_metrics();
    PreparedSyntaxCacheMetrics {
        hit,
        miss,
        evict,
        chunk_build_ms,
    }
}

#[cfg(test)]
pub(super) fn reset_prepared_syntax_cache() {
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
pub(super) fn prepared_syntax_loaded_chunk_count(document: PreparedSyntaxDocument) -> usize {
    benchmark_prepared_syntax_loaded_chunk_count(document).unwrap_or_default()
}

pub(in super::super) fn prepare_treesitter_document_with_budget_reuse_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    budget: DiffSyntaxBudget,
    old_document: Option<PreparedSyntaxDocument>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> PrepareTreesitterDocumentResult {
    // Before the source-identity fast path: the specs this language injects into
    // are needed later, by the render path, whether or not this document is
    // already cached.
    request_highlight_spec_warmup(language);
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

pub(in super::super) fn prepare_treesitter_document_in_background_text_with_reparse_seed(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    reparse_seed: Option<PreparedSyntaxReparseSeed>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> Option<PreparedSyntaxDocumentData> {
    request_highlight_spec_warmup(language);
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

pub(in super::super) fn inject_prepared_document_data(
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
pub(in super::super) fn syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<Vec<SyntaxToken>> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().line_tokens(document.cache_key, line_ix))
}

pub(in super::super) fn request_syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<PreparedSyntaxLineTokensRequest> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .request_line_tokens(document.cache_key, line_ix)
    })
}

pub(in super::super) fn request_syntax_tokens_for_prepared_document_line_range_into(
    document: PreparedSyntaxDocument,
    line_range: Range<usize>,
    requests: &mut Vec<PreparedSyntaxLineTokensRequest>,
) -> Option<PreparedSyntaxLineTokensRangeSummary> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .request_line_tokens_range_into(document.cache_key, line_range, requests)
    })
}

pub(in super::super) fn drain_completed_prepared_syntax_chunk_builds() -> usize {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().drain_completed_chunk_builds())
}

pub(in super::super) fn has_pending_prepared_syntax_chunk_builds() -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().has_pending_chunk_requests())
}

pub(in super::super) fn drain_completed_prepared_syntax_chunk_builds_for_document(
    document: PreparedSyntaxDocument,
) -> usize {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .drain_completed_chunk_builds_for_cache_key(document.cache_key)
    })
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in super::super) fn has_pending_prepared_syntax_chunk_builds_for_document(
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

/// One end of a matched pair, in the coordinate space the row canvases speak:
/// a document line index plus a *display* byte range within that line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedSyntaxPairSpan {
    pub(in crate::view) line_ix: usize,
    pub(in crate::view) display_range: Range<usize>,
}

/// A matched pair on a prepared document, already projected into line space.
///
/// Each end is a *list* of spans, one per line it covers: a start tag written
/// across several lines (`<div\n  class="card">`, ordinary in HTML and JSX) is
/// one delimiter but several rows, and reporting only its first line would wash
/// `<div` and leave the rest bare.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedSyntaxPairHit {
    /// Which construct the pair delimits. Test-only for the same reason as
    /// [`crate::view::DiffTextPairMatch::kind`]: every kind is painted alike, so
    /// this exists to let an assertion say which pair was found, not to steer
    /// anything.
    #[cfg(test)]
    pub(in crate::view) kind: SyntaxPairKind,
    pub(in crate::view) open: Vec<PreparedSyntaxPairSpan>,
    pub(in crate::view) close: Vec<PreparedSyntaxPairSpan>,
}

/// One line's byte range, without its line terminator.
///
/// The newline belongs to the line's bytes but never to its display columns, so
/// it is trimmed before any offset is measured against the line.
///
/// Every bound goes through `text.get`, so a `line_starts` that does not
/// describe `text` answers `None` instead of panicking. That is not
/// hypothetical: the click path can index a side whose file was re-read after
/// the diff was built, and a file that shrank in between leaves starts past the
/// end of the text.
fn prepared_line_span(text: &str, line_starts: &[usize], ix: usize) -> Option<Range<usize>> {
    let start = *line_starts.get(ix)?;
    let end = line_starts
        .get(ix + 1)
        .copied()
        .unwrap_or(text.len())
        .min(text.len());
    let line = text.get(start..end)?;
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    Some(start..start + line.len())
}

/// The matching pair at a click, taking and returning the canvases' own
/// coordinates.
///
/// The whole display/raw tab conversion lives here rather than at the call site
/// because this is the only place that holds both halves of it: the tree indexes
/// raw bytes, the canvases count tab-expanded columns, and
/// [`PreparedSyntaxTreeState`] carries the `text` and `line_starts` that relate
/// them. The view layer therefore never handles a document byte offset.
///
/// Returns `None` when the document has no retained tree (it was evicted, or the
/// parse never finished), when `line_ix` is past the end, when the click landed
/// past the line's last character, or when nothing pairs.
///
/// Injections *are* consulted, and first -- see [`injected_syntax_pair_at`]. The
/// injected region's own tree is kept for exactly this, because to the host
/// grammar an injected body is one opaque leaf: a delimiter inside it matches
/// nothing, and the walk falls out to the enclosing element. That was the whole
/// bug -- clicking the `<` of a `<html>` tag in a PHP file did nothing.
/// Combined injections stay out of it, and the host tree remains the fallback.
pub(in crate::view) fn prepared_document_syntax_pair_at_display_offset(
    document: PreparedSyntaxDocument,
    line_ix: usize,
    display_offset: usize,
) -> Option<PreparedSyntaxPairHit> {
    let state = prepared_document_tree_state(document)?;
    let text = state.text.as_ref();
    let line_starts = state.line_starts.as_ref();

    let line_span = |ix: usize| prepared_line_span(text, line_starts, ix);

    let clicked = line_span(line_ix)?;
    let clicked_line = text.get(clicked.clone())?;
    let offset =
        clicked.start + clicked_raw_offset_for_display_offset(clicked_line, display_offset)?;

    let source_ranges_equal = |left: Range<usize>, right: Range<usize>| {
        let bytes = text.as_bytes();
        match (bytes.get(left), bytes.get(right)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    };
    ensure_injection_chain_cached_for_pair_lookup(&state, offset);
    let pair = injected_syntax_pair_at(text, state.source_hash, offset)
        .or_else(|| syntax_pair_in_tree(&state.tree, offset, &source_ranges_equal))?;

    let project = |range: &Range<usize>| -> Vec<PreparedSyntaxPairSpan> {
        // `partition_point` gives the count of starts at or before `range.start`,
        // so subtracting one lands on the line containing it.
        let Some(first) = line_starts
            .partition_point(|start| *start <= range.start)
            .checked_sub(1)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for ix in first..line_starts.len() {
            let Some(span) = line_span(ix) else { break };
            if span.start >= range.end && ix > first {
                break;
            }
            let Some(line) = text.get(span.clone()) else {
                break;
            };
            let start = range.start.clamp(span.start, span.end) - span.start;
            let end = range.end.clamp(span.start, span.end) - span.start;
            if start < end {
                out.push(PreparedSyntaxPairSpan {
                    line_ix: ix,
                    display_range: display_offset_for_raw_offset(line, start)
                        ..display_offset_for_raw_offset(line, end),
                });
            }
            if span.end >= range.end {
                break;
            }
        }
        out
    };

    let (open, close) = (project(&pair.open), project(&pair.close));
    (!open.is_empty() && !close.is_empty()).then_some(PreparedSyntaxPairHit {
        #[cfg(test)]
        kind: pair.kind,
        open,
        close,
    })
}

/// Every place the document names the token at a click, in the canvases' own
/// coordinates.
///
/// Shares the display/raw conversion and the line projection with
/// [`prepared_document_syntax_pair_at_display_offset`], for the same reason:
/// this is the only place holding both the tree's byte offsets and the text the
/// rows were painted from.
pub(in crate::view) fn prepared_document_occurrences_at_display_offset(
    document: PreparedSyntaxDocument,
    line_ix: usize,
    display_offset: usize,
) -> Vec<PreparedSyntaxPairSpan> {
    let Some(state) = prepared_document_tree_state(document) else {
        return Vec::new();
    };
    let text = state.text.as_ref();
    if text.len() > OCCURRENCE_MAX_TEXT_BYTES {
        return Vec::new();
    }
    let line_starts = state.line_starts.as_ref();

    let Some(clicked) = prepared_line_span(text, line_starts, line_ix) else {
        return Vec::new();
    };
    let Some(clicked_line) = text.get(clicked.clone()) else {
        return Vec::new();
    };
    // A click past the line's last character places a caret but names nothing;
    // see [`clicked_raw_offset_for_display_offset`].
    let Some(raw_offset) = clicked_raw_offset_for_display_offset(clicked_line, display_offset)
    else {
        return Vec::new();
    };
    let offset = clicked.start + raw_offset;

    let Some(found) = syntax_occurrences_in_tree(&state.tree, text, offset) else {
        return Vec::new();
    };
    found
        .ranges
        .iter()
        .filter_map(|range| {
            // A name never spans a line, so one span per occurrence is exact.
            let ix = line_starts
                .partition_point(|start| *start <= range.start)
                .checked_sub(1)?;
            let span = prepared_line_span(text, line_starts, ix)?;
            let line = text.get(span.clone())?;
            let start = range.start.clamp(span.start, span.end) - span.start;
            let end = range.end.clamp(span.start, span.end) - span.start;
            (start < end).then(|| PreparedSyntaxPairSpan {
                line_ix: ix,
                display_range: display_offset_for_raw_offset(line, start)
                    ..display_offset_for_raw_offset(line, end),
            })
        })
        .collect()
}

pub(in super::super) fn prepared_document_reparse_seed(
    document: PreparedSyntaxDocument,
) -> Option<PreparedSyntaxReparseSeed> {
    prepared_document_tree_state(document).map(|tree_state| PreparedSyntaxReparseSeed {
        document,
        tree_state,
    })
}

#[cfg(test)]
pub(in super::super) fn prepared_document_parse_mode(
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
pub(in super::super) fn prepared_document_source_version(
    document: PreparedSyntaxDocument,
) -> Option<u64> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state(document.cache_key)
            .map(|state| state.source_version)
    })
}

#[cfg(feature = "benchmarks")]
pub(in super::super) fn benchmark_cache_replacement_drop_step(
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
pub(in super::super) fn benchmark_drop_payload_timed_step(
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
pub(super) fn benchmark_line_tokens_payload(
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
    let reused_prefix = match (old_document, reparse_plan) {
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
        _ => ReusedPrefixLineTokenChunks::default(),
    };

    if let Some(source) = reused_prefix.injection_source {
        clone_prefix_injection_cache_entries(
            source.document_hash,
            request.cache_key.doc_hash,
            source.byte_end,
        );
    }

    Some(PreparedSyntaxDocumentData {
        cache_key: request.cache_key,
        line_count: request.input.line_starts.len(),
        line_token_chunks: reused_prefix.line_token_chunks,
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

pub(super) fn should_skip_budgeted_foreground_parse(
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

#[cfg(test)]
pub(super) fn treesitter_document_parse_request_from_input(
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
    if !should_prepare_treesitter_document(language, mode, input.text.len()) {
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

fn should_prepare_treesitter_document(
    _language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text_len: usize,
) -> bool {
    mode == DiffSyntaxMode::Auto && text_len <= TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES
}

pub(super) fn treesitter_document_input_from_shared_text(
    text: SharedString,
    line_starts: Arc<[usize]>,
) -> TreesitterDocumentInput {
    if text.is_empty() {
        return TreesitterDocumentInput {
            text,
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
        text,
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

pub(super) fn treesitter_document_input_from_text(text: &str) -> TreesitterDocumentInput {
    if text.is_empty() {
        return TreesitterDocumentInput {
            text: SharedString::new(""),
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
        text: SharedString::from(text.to_owned()),
        line_starts: Arc::<[usize]>::from(line_starts),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TreesitterByteEditRange {
    pub(super) start_byte: usize,
    pub(super) old_end_byte: usize,
    pub(super) new_end_byte: usize,
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

#[derive(Default)]
struct ReusedPrefixLineTokenChunks {
    line_token_chunks: FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    injection_source: Option<ReusedPrefixInjectionSource>,
}

#[derive(Clone, Copy)]
struct ReusedPrefixInjectionSource {
    document_hash: u64,
    byte_end: usize,
}

impl TreesitterDocumentCache {
    fn clone_prefix_line_token_chunks(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        chunk_limit: usize,
    ) -> ReusedPrefixLineTokenChunks {
        if chunk_limit == 0 {
            return ReusedPrefixLineTokenChunks::default();
        }
        let reused = self
            .by_cache_key
            .get(&cache_key)
            .map(|document| {
                let line_token_chunks = document
                    .line_token_chunks
                    .iter()
                    .filter(|&(&chunk_ix, _)| chunk_ix < chunk_limit)
                    .map(|(&chunk_ix, chunk)| (chunk_ix, chunk.clone()))
                    .collect();
                let injection_source = document.tree_state.as_ref().map(|state| {
                    let prefix_line_ix = chunk_limit
                        .saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
                        .min(state.line_starts.len());
                    ReusedPrefixInjectionSource {
                        document_hash: state.source_hash,
                        byte_end: state
                            .line_starts
                            .get(prefix_line_ix)
                            .copied()
                            .unwrap_or(state.text.len()),
                    }
                });
                ReusedPrefixLineTokenChunks {
                    line_token_chunks,
                    injection_source,
                }
            })
            .unwrap_or_default();
        if !reused.line_token_chunks.is_empty() {
            self.touch_key(cache_key);
        }
        reused
    }
}

/// Carry injection trees alongside prefix token chunks reused by an incremental
/// reparse. Only injections wholly contained in the reusable prefix are copied:
/// a tree spanning the edit boundary may depend on changed suffix bytes.
fn clone_prefix_injection_cache_entries(
    old_document_hash: u64,
    new_document_hash: u64,
    prefix_byte_end: usize,
) {
    if old_document_hash == new_document_hash || prefix_byte_end == 0 {
        return;
    }

    TS_INJECTION_CACHE.with(|cache| {
        let clones = cache
            .borrow()
            .iter()
            .filter(|(key, _)| {
                key.document_hash == old_document_hash && key.byte_end <= prefix_byte_end
            })
            .map(|(key, entry)| {
                let mut key = *key;
                key.document_hash = new_document_hash;
                (key, entry.clone())
            })
            .collect::<Vec<_>>();

        let mut cache = cache.borrow_mut();
        for (key, mut entry) in clones {
            if cache.contains_key(&key) {
                continue;
            }
            evict_injection_cache_if_full(&mut cache);
            entry.last_access = next_injection_access();
            cache.insert(key, entry);
        }
    });
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

pub(super) fn compute_incremental_edit_ranges(
    old: &[u8],
    new: &[u8],
) -> Vec<TreesitterByteEditRange> {
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

pub(super) fn incremental_reparse_should_fallback(
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

pub(super) fn treesitter_point_for_byte(
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

pub(super) fn parse_treesitter_tree(
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

pub(super) const MAX_TREESITTER_LINE_BYTES: usize = 512;

pub(super) fn should_use_treesitter_for_line(text: &str) -> bool {
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
pub(super) fn is_heuristic_sufficient_for_line(text: &str, language: DiffSyntaxLanguage) -> bool {
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

pub(super) struct TreesitterHighlightSpec {
    pub(super) ts_language: tree_sitter::Language,
    pub(super) query: tree_sitter::Query,
    pub(super) capture_kinds: Vec<Option<SyntaxTokenKind>>,
    pub(super) injection_query: Option<tree_sitter::Query>,
    /// One flag per injection-query pattern: `true` when the pattern carries
    /// `(#set! injection.combined)`, meaning every match of it belongs to one
    /// shared layer rather than getting a layer each.
    ///
    /// Computed once in `init_highlight_spec` so the hot match loop never has to
    /// walk `property_settings`.
    pub(super) injection_combined_patterns: Vec<bool>,
    /// `injection_combined_patterns.iter().any(|&c| c)`, hoisted so the whole
    /// combined path can be skipped with one branch. This is the gate that keeps
    /// the feature a no-op for every grammar that does not declare it -- today
    /// that is all of them except F#, whose `xml_doc` rule is combined upstream.
    pub(super) has_combined_injections: bool,
}

impl TreesitterHighlightSpec {
    /// Whether match `pattern_ix` belongs to a shared layer.
    ///
    /// Reads the hoisted `has_combined_injections` gate first, so a grammar with no
    /// combined pattern costs one branch. Shared by both collectors, which used to
    /// inline this and drifted apart.
    pub(super) fn is_combined_injection_pattern(&self, pattern_ix: usize) -> bool {
        self.has_combined_injections
            && self
                .injection_combined_patterns
                .get(pattern_ix)
                .copied()
                .unwrap_or(false)
    }
}

/// Merges the per-pattern range map both collectors build into a deterministically
/// ordered layer list.
///
/// The order is load-bearing: overlapping layers are applied in sequence and the
/// later one wins, so `FxHashMap` order would tie an overlap's colour to hash seeding.
pub(super) fn combined_injection_groups_in_apply_order(
    combined_ranges: FxHashMap<(DiffSyntaxLanguage, usize), Vec<Range<usize>>>,
) -> Vec<(DiffSyntaxLanguage, usize, Vec<Range<usize>>)> {
    let mut groups: Vec<(DiffSyntaxLanguage, usize, Vec<Range<usize>>)> = combined_ranges
        .into_iter()
        .filter_map(|((language, pattern_ix), ranges)| {
            let ranges = merge_sorted_injection_ranges(ranges);
            (!ranges.is_empty()).then_some((language, pattern_ix, ranges))
        })
        .collect();
    groups.sort_unstable_by_key(|(language, pattern_ix, _)| (*language, *pattern_ix));
    groups
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TreesitterQueryPass {
    pub(super) byte_range: Range<usize>,
    pub(super) containing_byte_range: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct TreesitterInjectionMatch {
    /// Identity of the root text this injection was found in. Nested injections
    /// retain the root document's identity and use root-relative byte ranges, so
    /// pair lookup can consider every layer without admitting a sibling document.
    ///
    /// Without it the other three fields can describe two *different* documents'
    /// injections at once, and [`injected_syntax_pair_at`] iterates a cache
    /// shared by every document. Changing a Markdown fence from ```` ```html ````
    /// to ```` ```bash ```` keeps the fenced bytes and their offsets identical
    /// and moves only `language`, so both keys go live -- and in a diff both
    /// sides are tokenized on this same thread. The lookup would then answer a
    /// click with whichever grammar the hash map happened to yield first.
    pub(super) document_hash: u64,
    pub(super) language: DiffSyntaxLanguage,
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
    /// Hash of the injection content bytes. This ensures the cache is not
    /// confused when different parent documents happen to produce injection
    /// regions at the same byte offsets *with different content*. It cannot
    /// separate same-content revisions on its own; that is `document_hash`'s job.
    pub(super) content_hash: u64,
}

#[derive(Clone)]
pub(super) struct CachedInjection {
    /// Full tokenized lines in injection-local coordinates (all lines of the injection).
    pub(super) all_line_tokens: Vec<Vec<SyntaxToken>>,
    /// Line starts for the injection text, used for coordinate remapping.
    pub(super) injection_line_starts: Vec<usize>,
    /// First line in the parent document that this injection starts on.
    pub(super) injection_start_line_ix: usize,
    /// The injected grammar's own tree, kept so a click can be answered by the
    /// grammar that actually owns those bytes.
    ///
    /// Tokens alone were enough while this cache only ever painted. Bracket
    /// matching reads the tree, and `prepared_document_syntax_pair_at_display_offset`
    /// had only the *host* tree -- so in an injected region there was no
    /// structure to pair against at all: clicking the `<` of `<html>` in a PHP
    /// file did nothing, because to PHP that whole span is one `text` node.
    /// Parsed during tokenization so the normal click path does not pay for a
    /// parse. Pair lookup recreates it only when this entry was evicted while
    /// the prepared document itself remained cached.
    ///
    /// Its offsets are injection-local: the injected text is parsed standalone,
    /// not with `included_ranges`, so document offsets need shifting by
    /// `byte_start` in both directions. The live engine differs here -- see
    /// `LiveSyntaxSnapshot::syntax_pair_at`, whose layers are already in document
    /// coordinates.
    pub(super) tree: tree_sitter::Tree,
    /// Monotonic access counter for LRU eviction.
    pub(super) last_access: u64,
}

#[derive(Clone, Copy)]
pub(super) struct TreesitterQueryAsset {
    pub(super) highlights: &'static str,
    pub(super) injections: Option<&'static str>,
    /// Extra patterns appended to `highlights` before it is compiled.
    ///
    /// Several grammars are used with the query their own crate ships, which
    /// cannot be edited here and in places captures nothing for constructs that
    /// matter -- brackets, or in Objective-C's case comments and strings. This
    /// is how those are filled in without vendoring a whole query and taking on
    /// the job of tracking upstream's. Appended, not prepended: overlapping
    /// captures resolve last-wins, so a supplement can also correct a capture
    /// upstream got wrong.
    pub(super) supplement: Option<&'static str>,
}

impl TreesitterQueryAsset {
    pub(super) const fn highlights(source: &'static str) -> Self {
        Self {
            highlights: source,
            injections: None,
            supplement: None,
        }
    }

    pub(super) const fn with_injections(
        highlights: &'static str,
        injections: &'static str,
    ) -> Self {
        Self {
            highlights,
            injections: Some(injections),
            supplement: None,
        }
    }

    /// Appends in-tree patterns to a query this repo does not own.
    pub(super) const fn with_supplement(
        highlights: &'static str,
        supplement: &'static str,
    ) -> Self {
        Self {
            highlights,
            injections: None,
            supplement: Some(supplement),
        }
    }

    pub(super) const fn with_injections_and_supplement(
        highlights: &'static str,
        injections: &'static str,
        supplement: &'static str,
    ) -> Self {
        Self {
            highlights,
            injections: Some(injections),
            supplement: Some(supplement),
        }
    }
}

struct DocumentTokenCollectionContext<'a> {
    line_starts: &'a [usize],
    start_line_ix: usize,
    end_line_ix: usize,
    /// Byte offset of this parsed input inside the root prepared document.
    /// Zero for the host tree; an outer injection's absolute start for a nested
    /// tree. Injection cache keys always use root-document coordinates.
    document_byte_start: usize,
    per_line: &'a mut [Vec<SyntaxToken>],
}

pub(super) fn syntax_tokens_for_line_treesitter(
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
        doc_hash: treesitter_document_hash(language, input),
    }
}

fn prepared_document_source_identity_for_shared_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
    line_count: usize,
) -> Option<PreparedSyntaxSourceIdentity> {
    if !should_prepare_treesitter_document(language, mode, text.len()) {
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

pub(super) fn treesitter_text_hash(input: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    input.hash(&mut hasher);
    hasher.finish()
}

/// Identity used by prepared trees and every injection derived from them.
///
/// `PreparedSyntaxCacheKey` already stores the host language separately for
/// cache lookup, but injection entries retain only this hash. Folding the host
/// language in here prevents identical bytes parsed as, for example, HTML and
/// Vue from sharing an injection identity.
fn treesitter_document_hash(language: DiffSyntaxLanguage, input: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    language.hash(&mut hasher);
    treesitter_text_hash(input).hash(&mut hasher);
    hasher.finish()
}

pub(super) fn collect_treesitter_document_line_tokens_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
) -> Vec<Vec<SyntaxToken>> {
    collect_treesitter_document_line_tokens_for_line_window_at(
        tree,
        highlight,
        input,
        line_starts,
        start_line_ix,
        end_line_ix,
        document_hash,
        0,
    )
}

fn collect_treesitter_document_line_tokens_for_line_window_at(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
    document_byte_start: usize,
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
            document_byte_start,
            per_line: &mut per_line,
        };
        for pass in &query_passes {
            collect_query_pass_tokens_for_document(tree, highlight, input, pass, &mut context);
        }
        apply_injection_query_tokens_for_document(
            tree,
            highlight,
            input,
            document_hash,
            &mut context,
        );
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
pub(super) fn line_region_end_byte(
    line_starts: &[usize],
    input_len: usize,
    line_ix: usize,
) -> usize {
    line_starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(input_len)
        .min(input_len)
}

/// Content-end byte offset for line `line_ix` (excludes a trailing `\n` when present).
pub(super) fn line_content_end_byte(line_starts: &[usize], input: &[u8], line_ix: usize) -> usize {
    let region_end = line_region_end_byte(line_starts, input.len(), line_ix);
    if input.get(region_end.saturating_sub(1)) == Some(&b'\n') {
        region_end.saturating_sub(1)
    } else {
        region_end
    }
}

pub(super) fn treesitter_document_query_passes_for_line_window(
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
    catch_treesitter_query_panic(|| {
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
    document_hash: u64,
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    let Some(_guard) = InjectionDepthGuard::enter() else {
        return;
    };
    let injections = collect_treesitter_injection_matches_for_line_window_at(
        tree,
        highlight,
        input,
        context.line_starts,
        context.start_line_ix,
        context.end_line_ix,
        document_hash,
        context.document_byte_start,
    );
    for injection in &injections.singles {
        let injection = *injection;
        let Some(injected_tokens) = collect_injected_tokens_for_parent_line_window(
            input,
            context.line_starts,
            context.start_line_ix,
            context.end_line_ix,
            injection,
            context.document_byte_start,
        ) else {
            continue;
        };

        // Subtract only what the injection actually paints, not its whole span.
        //
        // Blanking the span outright loses any host capture the injection has no
        // opinion about: a markdown heading is `@text.title` in the block
        // grammar, and the inline grammar that owns those bytes captures nothing
        // for plain prose, so the heading came out uncoloured. Cutting per
        // painted token keeps last-wins where the two overlap and leaves the
        // host's answer standing in the gaps.
        for (parent_line_ix, tokens) in &injected_tokens {
            // Same window guard as the append loop below: `saturating_sub` on a
            // line below the window would clamp to index 0 and subtract these
            // ranges out of the first visible row's host tokens instead.
            if *parent_line_ix < context.start_line_ix {
                continue;
            }
            // The tokens are already grouped by line and their ranges are
            // already line-relative, so going back through the absolute form
            // would re-derive by binary search what is known here.
            let Some(line_tokens) = context
                .per_line
                .get_mut(parent_line_ix.saturating_sub(context.start_line_ix))
            else {
                continue;
            };
            for token in tokens {
                subtract_relative_range_from_line_tokens(line_tokens, token.range.clone());
            }
        }

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

    // Combined last, and the order matters once a grammar declares both kinds over
    // the same bytes. Each layer clears what it is about to paint out of
    // `per_line` first -- per painted token for a single, over the whole span for
    // a combined group -- so the last one wins the overlap; running combined
    // first let a single delete combined tokens and then repaint only the bytes
    // its own captures cover, leaving the rest bare. No in-tree grammar mixes
    // them yet.
    if !injections.truncated {
        for group in &injections.combined {
            apply_combined_injection_tokens(group, input, document_hash, context);
        }
    }
}

/// Parses `ranges` as one document with `spec`'s grammar, leaving node offsets in
/// *document* coordinates.
///
/// `set_included_ranges` is what buys that: the parser reads the whole input but
/// only builds nodes inside the ranges, so no coordinate remapping is needed on
/// the way out -- unlike the per-injection path, which slices the text and maps
/// injection-local offsets back in `collect_injected_tokens_for_parent_line_window`.
///
/// The ranges are cleared again on every exit path. `TS_PARSER` is pooled, its
/// included ranges are sticky, and `with_ts_parser`'s fast path can skip
/// `set_language` entirely -- leaving them set would silently truncate the next
/// root parse on this thread, for any language. `IncludedRangesGuard` makes that
/// unconditional rather than relying on each `return` remembering.
pub(super) fn parse_combined_injection_tree(
    spec: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    ranges: &[Range<usize>],
) -> Option<tree_sitter::Tree> {
    // Never call `set_included_ranges` with an empty slice: that is tree-sitter's
    // reset to "the whole document", so a window whose group collapsed to nothing
    // would highlight the entire file with the injected grammar.
    if ranges.is_empty() {
        return None;
    }
    let ts_ranges = ranges
        .iter()
        .map(|range| tree_sitter::Range {
            start_byte: range.start,
            end_byte: range.end,
            start_point: treesitter_point_for_byte(line_starts, input, range.start),
            end_point: treesitter_point_for_byte(line_starts, input, range.end),
        })
        .collect::<Vec<_>>();

    with_ts_parser_parse_result(&spec.ts_language, |parser| {
        let mut guard = IncludedRangesGuard::set(parser, &ts_ranges)?;
        parse_treesitter_tree(guard.parser(), input, None, None)
    })
}

/// Clears a parser's included ranges on drop.
pub(super) struct IncludedRangesGuard<'parser>(&'parser mut tree_sitter::Parser);

impl<'parser> IncludedRangesGuard<'parser> {
    pub(super) fn set(
        parser: &'parser mut tree_sitter::Parser,
        ranges: &[tree_sitter::Range],
    ) -> Option<Self> {
        if parser.set_included_ranges(ranges).is_err() {
            let _ = parser.set_included_ranges(&[]);
            return None;
        }
        Some(Self(parser))
    }

    pub(super) fn parser(&mut self) -> &mut tree_sitter::Parser {
        self.0
    }
}

impl Drop for IncludedRangesGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.set_included_ranges(&[]);
    }
}

/// The byte span a combined layer is parsed over for a given window.
///
/// The rendered window, widened by [`TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES`]
/// on each side so a construct straddling the window edge is still parsed whole.
/// See that constant for why the margin is not optional.
pub(super) fn combined_injection_clip_region(
    line_starts: &[usize],
    input_len: usize,
    start_line_ix: usize,
    end_line_ix: usize,
) -> Range<usize> {
    let window_start = line_starts.get(start_line_ix).copied().unwrap_or(0);
    let window_end = line_region_end_byte(line_starts, input_len, end_line_ix.saturating_sub(1));
    let clip_start = window_start.saturating_sub(TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES);
    let clip_end = window_end
        .saturating_add(TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES)
        .min(input_len);
    clip_start..clip_end.max(clip_start)
}

/// `ranges` is already sorted and non-overlapping, and clipping preserves both, so
/// the result needs no re-merge.
pub(super) fn clip_injection_ranges_to_region(
    ranges: &[Range<usize>],
    region: &Range<usize>,
) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter_map(|range| {
            let start = range.start.max(region.start);
            let end = range.end.min(region.end);
            (start < end).then_some(start..end)
        })
        .collect()
}

/// Parses one combined group and splices its tokens into `context.per_line`.
fn apply_combined_injection_tokens(
    group: &CombinedInjectionGroup,
    input: &[u8],
    document_hash: u64,
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    let Some(spec) = tree_sitter_highlight_spec(group.language) else {
        return;
    };

    // Clip before the ceilings are applied, not after; see their doc comment.
    let clip_region = combined_injection_clip_region(
        context.line_starts,
        input.len(),
        context.start_line_ix,
        context.end_line_ix,
    );
    let ranges = clip_injection_ranges_to_region(&group.ranges, &clip_region);
    if ranges.is_empty() {
        return;
    }
    if ranges.len() > TS_COMBINED_INJECTION_MAX_RANGES {
        return;
    }
    let total_bytes: usize = ranges
        .iter()
        .map(|range| range.end.saturating_sub(range.start))
        .sum();
    if total_bytes > TS_COMBINED_INJECTION_MAX_BYTES {
        return;
    }
    let Some(tree) = parse_combined_injection_tree(spec, input, context.line_starts, &ranges)
    else {
        return;
    };

    let mut injected = collect_treesitter_document_line_tokens_for_line_window_at(
        &tree,
        spec,
        input,
        context.line_starts,
        context.start_line_ix,
        context.end_line_ix,
        document_hash,
        context.document_byte_start,
    );
    if injected.len() != context.per_line.len() {
        return;
    }

    // Clip the injected tokens back to the ranges the injected grammar actually
    // owns; see `combined_injection_gaps`.
    let window_start = context
        .line_starts
        .get(context.start_line_ix)
        .copied()
        .unwrap_or(0);
    let window_end = line_region_end_byte(
        context.line_starts,
        input.len(),
        context.end_line_ix.saturating_sub(1),
    );
    for gap in combined_injection_gaps(window_start..window_end, &ranges) {
        subtract_absolute_range_from_document_tokens(
            context.line_starts,
            input,
            context.start_line_ix,
            &mut injected,
            gap,
        );
    }

    // ... and drop the host grammar's tokens from the bytes the injection took over.
    for range in &ranges {
        subtract_absolute_range_from_document_tokens(
            context.line_starts,
            input,
            context.start_line_ix,
            context.per_line,
            range.clone(),
        );
    }

    for (line_tokens, tokens) in context.per_line.iter_mut().zip(injected) {
        line_tokens.extend(tokens);
    }
}

/// Every match of one `(#set! injection.combined)` pattern, gathered into the
/// single layer the directive asks for.
///
/// `ranges` is sorted, non-overlapping and non-empty by the time this leaves
/// [`collect_treesitter_injection_matches_for_line_window`] -- all three are hard
/// requirements of `Parser::set_included_ranges`, and an *empty* slice is
/// tree-sitter's "reset to the whole document" rather than "parse nothing".
///
/// The key is `(language, pattern_index)` with no host-node identity, matching
/// upstream tree-sitter's semantics. The cost is that unrelated matches of one
/// pattern share a document: `buildPhase` and `installPhase` in the same chunk parse
/// as one bash script, so an unterminated `if` in the first recolours the second.
///
/// Do not "fix" that by sub-grouping on the parent node. Nix `(string_fragment)`s do
/// sit under their own `indented_string_expression`, but Jinja `(text)` nodes are
/// children of whatever encloses them (`source_file`, `for_statement`,
/// `if_statement`), so a parent key would shatter one template's HTML into a
/// document per block. The Nix case needs a per-pattern key.
pub(super) struct CombinedInjectionGroup {
    pub(super) language: DiffSyntaxLanguage,
    pub(super) ranges: Vec<Range<usize>>,
}

pub(super) struct TreesitterInjectionMatches {
    /// One layer per match, the pre-existing behaviour. Unchanged for every
    /// grammar that does not declare `injection.combined`.
    pub(super) singles: Vec<TreesitterInjectionMatch>,
    pub(super) combined: Vec<CombinedInjectionGroup>,
    /// The query cursor overflowed `TS_QUERY_MATCH_LIMIT` somewhere in this
    /// window, so tree-sitter silently dropped matches.
    ///
    /// Only the combined groups act on this. Losing one *single* injection costs
    /// that node its highlighting and nothing else, which is the status quo;
    /// losing one range out of a combined set changes the document the injected
    /// grammar sees, so an unbalanced `<div>` can wreck the whole window. Better
    /// to leave the host grammar painting.
    pub(super) truncated: bool,
}

#[cfg(test)]
pub(super) fn collect_treesitter_injection_matches_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
) -> TreesitterInjectionMatches {
    collect_treesitter_injection_matches_for_line_window_at(
        tree,
        highlight,
        input,
        line_starts,
        start_line_ix,
        end_line_ix,
        document_hash,
        0,
    )
}

fn collect_treesitter_injection_matches_for_line_window_at(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
    document_byte_start: usize,
) -> TreesitterInjectionMatches {
    let empty = || TreesitterInjectionMatches {
        singles: Vec::new(),
        combined: Vec::new(),
        truncated: false,
    };
    let Some(injection_query) = highlight.injection_query.as_ref() else {
        return empty();
    };
    let Some(injection_content_capture_ix) =
        injection_query.capture_index_for_name("injection.content")
    else {
        return empty();
    };
    let injection_language_capture_ix =
        injection_query.capture_index_for_name("injection.language");
    let language_capture_ix = injection_query.capture_index_for_name("language");

    let query_passes = treesitter_document_query_passes_for_line_window(
        line_starts,
        input.len(),
        start_line_ix,
        end_line_ix,
    );
    if query_passes.is_empty() {
        return empty();
    }

    // The one gate that keeps this a no-op for every grammar that does not declare
    // `injection.combined`: no map allocated, no per-match pattern lookup, no
    // `did_exceed_match_limit` read, and `combined` stays empty below.
    let has_combined = highlight.has_combined_injections;

    let mut seen = FxHashSet::default();
    let mut injections = Vec::new();
    let mut combined_ranges: FxHashMap<(DiffSyntaxLanguage, usize), Vec<Range<usize>>> =
        FxHashMap::default();
    let mut truncated = false;
    for pass in &query_passes {
        catch_treesitter_query_panic(|| {
            TS_CURSOR.with(|cursor| {
                let mut cursor = cursor.borrow_mut();
                configure_query_cursor(&mut cursor, pass, input.len());
                let mut matches = cursor.matches(injection_query, tree.root_node(), input);
                tree_sitter::StreamingIterator::advance(&mut matches);
                while let Some(m) = matches.get() {
                    let Some(language) = injection_language_for_match(
                        injection_query,
                        m,
                        input,
                        injection_language_capture_ix,
                        language_capture_ix,
                    ) else {
                        tree_sitter::StreamingIterator::advance(&mut matches);
                        continue;
                    };
                    let pattern_ix = m.pattern_index;
                    let is_combined = highlight.is_combined_injection_pattern(pattern_ix);
                    for capture in m
                        .captures
                        .iter()
                        .filter(|capture| capture.index == injection_content_capture_ix)
                    {
                        let Some(byte_range) =
                            normalized_injection_content_byte_range(capture.node, input.len())
                        else {
                            continue;
                        };
                        if is_combined {
                            combined_ranges
                                .entry((language, pattern_ix))
                                .or_default()
                                .push(byte_range);
                            continue;
                        }
                        let injection = TreesitterInjectionMatch {
                            document_hash,
                            language,
                            byte_start: byte_range.start.saturating_add(document_byte_start),
                            byte_end: byte_range.end.saturating_add(document_byte_start),
                            content_hash: injection_content_hash(
                                &input[byte_range.start..byte_range.end],
                            ),
                        };
                        if seen.insert(injection) {
                            injections.push(injection);
                        }
                    }
                    tree_sitter::StreamingIterator::advance(&mut matches);
                }
                // Read inside the same borrow: the cursor is a thread-local and
                // the next pass reconfigures it.
                if has_combined {
                    truncated |= cursor.did_exceed_match_limit();
                }
            });
        });
    }

    injections.sort_by_key(|injection| (injection.byte_start, injection.byte_end));

    if !has_combined {
        return TreesitterInjectionMatches {
            singles: injections,
            combined: Vec::new(),
            truncated: false,
        };
    }

    let combined = combined_injection_groups_in_apply_order(combined_ranges)
        .into_iter()
        .map(|(language, _, ranges)| CombinedInjectionGroup { language, ranges })
        .collect::<Vec<_>>();

    TreesitterInjectionMatches {
        singles: injections,
        combined,
        truncated,
    }
}

/// Sorts and coalesces injection ranges into the form `set_included_ranges`
/// requires: ascending, non-overlapping, non-empty.
///
/// Touching ranges (`a.end == b.start`) are merged too. Leaving them adjacent
/// would be accepted by tree-sitter but produces one more range than necessary,
/// and the gap list in [`combined_injection_gaps`] would carry an empty entry.
pub(super) fn merge_sorted_injection_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.retain(|range| range.start < range.end);
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => {
                last.end = last.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// The complement of `ranges` inside `window`.
///
/// Combined injections need this because tree-sitter reports *document* offsets
/// for a tree parsed with disjoint included ranges: a node whose first token is
/// in one range and whose last token is in another spans every host byte in
/// between. `((element) @tag)` over
/// `{% if x %}<div>{% endif %}text</div>` would otherwise paint the `{% endif %}`
/// with the injected grammar's colour. Subtracting these gaps from the injected
/// tokens is what keeps each grammar inside its own bytes.
///
/// `ranges` must already be sorted and non-overlapping.
pub(super) fn combined_injection_gaps(
    window: Range<usize>,
    ranges: &[Range<usize>],
) -> Vec<Range<usize>> {
    let mut gaps = Vec::with_capacity(ranges.len() + 1);
    let mut cursor = window.start;
    for range in ranges {
        let start = range.start.max(window.start);
        if start > cursor {
            gaps.push(cursor..start.min(window.end));
        }
        cursor = cursor.max(range.end.min(window.end));
        if cursor >= window.end {
            break;
        }
    }
    if cursor < window.end {
        gaps.push(cursor..window.end);
    }
    gaps.retain(|gap| gap.start < gap.end);
    gaps
}

fn bounded_node_byte_range(node: tree_sitter::Node, input_len: usize) -> Option<Range<usize>> {
    let mut byte_range = node.byte_range();
    byte_range.start = byte_range.start.min(input_len);
    byte_range.end = byte_range.end.min(input_len);
    (byte_range.start < byte_range.end).then_some(byte_range)
}

pub(super) fn normalized_injection_content_byte_range(
    node: tree_sitter::Node,
    input_len: usize,
) -> Option<Range<usize>> {
    let byte_range = bounded_node_byte_range(node, input_len)?;
    if !matches!(node.kind(), "string" | "template_string") {
        return Some(byte_range);
    }

    let named_child_count = node.named_child_count();
    if named_child_count == 0 {
        return Some(byte_range);
    }

    let mut content_start = usize::MAX;
    let mut content_end = 0usize;
    for child_ix in 0..named_child_count {
        let Some(child) = node.named_child(child_ix as u32) else {
            continue;
        };
        match child.kind() {
            "string_fragment" | "string_content" | "escape_sequence" => {
                let child_range = bounded_node_byte_range(child, input_len)?;
                content_start = content_start.min(child_range.start);
                content_end = content_end.max(child_range.end);
            }
            _ => return Some(byte_range),
        }
    }

    if content_start < content_end {
        Some(content_start..content_end)
    } else {
        Some(byte_range)
    }
}

fn injection_language_for_match(
    query: &tree_sitter::Query,
    query_match: &tree_sitter::QueryMatch<'_, '_>,
    input: &[u8],
    injection_language_capture_ix: Option<u32>,
    language_capture_ix: Option<u32>,
) -> Option<DiffSyntaxLanguage> {
    let pattern_language = query
        .property_settings(query_match.pattern_index)
        .iter()
        .filter(|setting| matches!(setting.key.as_ref(), "injection.language" | "language"))
        .find_map(|setting| {
            setting
                .value
                .as_deref()
                .and_then(injection_language_from_name)
                .or_else(|| {
                    setting.capture_id.and_then(|capture_id| {
                        query_capture_text(query_match.captures, capture_id as u32, input)
                            .and_then(injection_language_from_name)
                    })
                })
        });
    pattern_language.or_else(|| {
        [injection_language_capture_ix, language_capture_ix]
            .into_iter()
            .flatten()
            .find_map(|capture_ix| {
                query_capture_text(query_match.captures, capture_ix, input)
                    .and_then(injection_language_from_name)
            })
    })
}

fn query_capture_text<'capture, 'input>(
    captures: &[tree_sitter::QueryCapture<'capture>],
    capture_ix: u32,
    input: &'input [u8],
) -> Option<&'input str> {
    let capture = captures
        .iter()
        .rev()
        .find(|capture| capture.index == capture_ix)?;
    let mut byte_range = capture.node.byte_range();
    byte_range.start = byte_range.start.min(input.len());
    byte_range.end = byte_range.end.min(input.len());
    if byte_range.start >= byte_range.end {
        return None;
    }
    std::str::from_utf8(&input[byte_range.start..byte_range.end]).ok()
}

pub(super) fn injection_language_from_name(name: &str) -> Option<DiffSyntaxLanguage> {
    let name =
        name.trim_matches(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | '`'));
    if name.is_empty() {
        return None;
    }
    diff_syntax_language_for_code_fence_info(name)
}

pub(super) fn next_injection_access() -> u64 {
    TS_INJECTION_ACCESS_COUNTER.with(|c| {
        let val = c.get().wrapping_add(1);
        c.set(val);
        val
    })
}

fn evict_injection_cache_if_full(cache: &mut FxHashMap<TreesitterInjectionMatch, CachedInjection>) {
    if cache.len() < TS_INJECTION_CACHE_MAX_ENTRIES {
        return;
    }

    // Evict the least-recently-used half instead of clearing everything.
    let mut entries: Vec<_> = cache
        .iter()
        .map(|(key, value)| (*key, value.last_access))
        .collect();
    entries.sort_unstable_by_key(|(_, access)| *access);
    let evict_count = entries.len() / 2;
    for (key, _) in entries.into_iter().take(evict_count) {
        cache.remove(&key);
    }
}

fn ensure_injection_cached(
    input: &[u8],
    line_starts: &[usize],
    injection: TreesitterInjectionMatch,
    parent_document_byte_start: usize,
) -> bool {
    TS_INJECTION_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(&injection) {
            entry.last_access = next_injection_access();
            return true;
        }

        let Some(local_byte_start) = injection.byte_start.checked_sub(parent_document_byte_start)
        else {
            return false;
        };
        let Some(local_byte_end) = injection.byte_end.checked_sub(parent_document_byte_start)
        else {
            return false;
        };
        let injection_byte_range =
            local_byte_start.min(input.len())..local_byte_end.min(input.len());
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
        let all_line_tokens = collect_treesitter_document_line_tokens_for_line_window_at(
            &tree,
            highlight,
            injection_input.text.as_bytes(),
            injection_input.line_starts.as_ref(),
            0,
            injection_line_count,
            injection.document_hash,
            injection.byte_start,
        );

        let injection_start_line_ix = line_ix_for_byte(line_starts, local_byte_start);
        let access = next_injection_access();

        let mut cache = cache.borrow_mut();
        evict_injection_cache_if_full(&mut cache);
        cache.insert(
            injection,
            CachedInjection {
                all_line_tokens,
                injection_line_starts: injection_input.line_starts.as_ref().to_vec(),
                injection_start_line_ix,
                tree,
                last_access: access,
            },
        );
        true
    })
}

/// The hash an injection's bytes are keyed by.
///
/// One function so the two callers cannot drift: the cache is keyed on this at
/// insertion, and [`injected_syntax_pair_at`] re-derives it to prove an entry
/// belongs to the document it is about to answer for. Hashing a `&[u8]` and
/// hashing the `&str` over the same bytes give different values, which is a
/// silent miss rather than a wrong answer -- the pair lookup just falls through
/// to the host grammar, exactly the bug it was added to fix.
fn injection_content_hash(content: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    content.hash(&mut hasher);
    hasher.finish()
}

/// The pair at `offset` as the *injected* grammar sees it.
///
/// Tried before the host tree by [`prepared_document_syntax_pair_at_display_offset`],
/// for the same reason `LiveSyntaxSnapshot::syntax_pair_at` tries its layers
/// first: a bracket inside an injected region belongs to the grammar that region
/// is written in. To PHP, the whole of a file's inline HTML is one `text` node,
/// so without this a click on a tag there found no delimiter and fell through to
/// whatever PHP block enclosed it -- which is why clicking a bracket appeared to
/// do nothing while clicking a column away appeared to work.
///
/// `document_hash` scopes the search to the caller's own document and
/// `content_hash` is re-checked against its text rather than trusted: the cache
/// is a thread-local shared by every document, and two of them can easily have
/// an injection at the same byte range -- with the same bytes in it, under
/// different grammars. Neither check subsumes the other, so both run.
///
/// Combined injections are deliberately not consulted. Their ranges are stitched
/// from several disjoint spans, so a single `byte_start` shift cannot map their
/// offsets back, and the host grammar remains the right answer in the gaps
/// between them.
fn injected_syntax_pair_at(text: &str, document_hash: u64, offset: usize) -> Option<SyntaxPair> {
    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        // The innermost injection wins, the same rule the tree walk uses: an
        // injection nested inside another is the more specific answer.
        let mut best: Option<(&TreesitterInjectionMatch, &CachedInjection)> = None;
        for (key, entry) in cache.iter() {
            if key.document_hash != document_hash {
                continue;
            }
            if offset < key.byte_start || offset >= key.byte_end {
                continue;
            }
            let Some(content) = text.as_bytes().get(key.byte_start..key.byte_end) else {
                continue;
            };
            if injection_content_hash(content) != key.content_hash {
                continue;
            }
            let width = key.byte_end.saturating_sub(key.byte_start);
            if best.is_none_or(|(best_key, _)| {
                width < best_key.byte_end.saturating_sub(best_key.byte_start)
            }) {
                best = Some((key, entry));
            }
        }
        let (key, entry) = best?;
        let local = offset.checked_sub(key.byte_start)?;
        let content = text.as_bytes().get(key.byte_start..key.byte_end)?;
        let source_ranges_equal =
            |left: Range<usize>, right: Range<usize>| match (content.get(left), content.get(right))
            {
                (Some(left), Some(right)) => left == right,
                _ => false,
            };
        let pair = syntax_pair_in_tree(&entry.tree, local, &source_ranges_equal)?;
        let shift = |range: Range<usize>| {
            range.start.saturating_add(key.byte_start)..range.end.saturating_add(key.byte_start)
        };
        Some(SyntaxPair {
            open: shift(pair.open),
            close: shift(pair.close),
            kind: pair.kind,
        })
    })
}

/// Recreates the injection chain containing `offset` after one of its LRU
/// entries was evicted while the prepared document and token chunks remained
/// cached.
///
/// Pair lookup normally reads the already-tokenized injection tree. A document
/// can outlive the 32-entry injection cache, though, and requesting an old token
/// chunk does not run its injection query again. Re-run just the clicked line's
/// query at each permitted layer, then feed the matching regions through the
/// same cache construction path used by token painting. Walking through cached
/// parents matters too: a parent can survive the LRU while its narrower child
/// was evicted.
fn ensure_injection_chain_cached_for_pair_lookup(state: &PreparedSyntaxTreeState, offset: usize) {
    let Some(highlight) = tree_sitter_highlight_spec(state.language) else {
        return;
    };
    let line_ix = line_ix_for_byte(state.line_starts.as_ref(), offset);
    let matches = collect_treesitter_injection_matches_for_line_window_at(
        &state.tree,
        highlight,
        state.text.as_bytes(),
        state.line_starts.as_ref(),
        line_ix,
        line_ix.saturating_add(1),
        state.source_hash,
        0,
    );
    let Some(injection) = matches
        .singles
        .into_iter()
        .filter(|injection| offset >= injection.byte_start && offset < injection.byte_end)
        .min_by_key(|injection| injection.byte_end.saturating_sub(injection.byte_start))
    else {
        return;
    };
    // `ensure_injection_cached` is normally called inside the host token
    // collector's depth guard. Pair lookup enters the equivalent guards itself
    // so rebuilding a nested entry cannot parse deeper than the configured
    // root-to-injection limit.
    let Some(root_depth_guard) = InjectionDepthGuard::enter() else {
        return;
    };
    let mut depth_guards = vec![root_depth_guard];
    if !ensure_injection_cached(
        state.text.as_bytes(),
        state.line_starts.as_ref(),
        injection,
        0,
    ) {
        return;
    }

    let mut parent = injection;
    for _ in 1..TS_MAX_INJECTION_DEPTH {
        let Some((tree, line_starts)) = TS_INJECTION_CACHE.with(|cache| {
            cache
                .borrow()
                .get(&parent)
                .map(|entry| (entry.tree.clone(), entry.injection_line_starts.clone()))
        }) else {
            break;
        };
        let Some(input) = state
            .text
            .as_bytes()
            .get(parent.byte_start..parent.byte_end)
        else {
            break;
        };
        let Some(highlight) = tree_sitter_highlight_spec(parent.language) else {
            break;
        };
        let local_offset = offset.saturating_sub(parent.byte_start);
        let line_ix = line_ix_for_byte(&line_starts, local_offset);
        let matches = collect_treesitter_injection_matches_for_line_window_at(
            &tree,
            highlight,
            input,
            &line_starts,
            line_ix,
            line_ix.saturating_add(1),
            state.source_hash,
            parent.byte_start,
        );
        let Some(child) = matches
            .singles
            .into_iter()
            .filter(|child| offset >= child.byte_start && offset < child.byte_end)
            .min_by_key(|child| child.byte_end.saturating_sub(child.byte_start))
        else {
            break;
        };
        let Some(child_depth_guard) = InjectionDepthGuard::enter() else {
            break;
        };
        if !ensure_injection_cached(input, &line_starts, child, parent.byte_start) {
            break;
        }
        depth_guards.push(child_depth_guard);
        parent = child;
    }
}

fn collect_injected_tokens_for_parent_line_window(
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    injection: TreesitterInjectionMatch,
    parent_document_byte_start: usize,
) -> Option<Vec<(usize, Vec<SyntaxToken>)>> {
    if !ensure_injection_cached(input, line_starts, injection, parent_document_byte_start) {
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
            let injection_start_in_parent = injection
                .byte_start
                .checked_sub(parent_document_byte_start)?;
            let absolute_line_start = injection_start_in_parent.saturating_add(local_line_start);
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

pub(super) fn subtract_absolute_range_from_document_tokens(
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

pub(super) fn subtract_relative_range_from_line_tokens(
    line_tokens: &mut Vec<SyntaxToken>,
    cut_range: Range<usize>,
) {
    if cut_range.start >= cut_range.end || line_tokens.is_empty() {
        return;
    }

    // Usually a no-op: the caller subtracts once per gap and once per range, so a
    // dense template reaches this ~2R times per chunk while any one line intersects
    // only a couple of those cuts. Rebuilding regardless cost a malloc, a memcpy and
    // a free per miss.
    if line_tokens
        .iter()
        .all(|token| token.range.end <= cut_range.start || token.range.start >= cut_range.end)
    {
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

pub(super) fn normalize_non_overlapping_tokens(tokens: Vec<SyntaxToken>) -> Vec<SyntaxToken> {
    let tokens = tokens
        .into_iter()
        .filter(|token| token.range.start < token.range.end)
        .collect::<Vec<_>>();
    if tokens.len() <= 1 {
        return tokens;
    }

    let mut boundaries = Vec::with_capacity(tokens.len().saturating_mul(2));
    for token in &tokens {
        boundaries.push(token.range.start);
        boundaries.push(token.range.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    // Convert overlapping captures into non-overlapping segments while preserving
    // tree-sitter's "later capture wins" semantics within each overlapped slice.
    let mut normalized: Vec<SyntaxToken> = Vec::with_capacity(tokens.len());
    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];
        if start >= end {
            continue;
        }

        let mut winner = None;
        for token in &tokens {
            if token.range.start <= start && end <= token.range.end {
                winner = Some(token.kind);
            }
        }

        let Some(kind) = winner else {
            continue;
        };

        if let Some(last) = normalized.last_mut()
            && last.kind == kind
            && last.range.end == start
        {
            last.range.end = end;
            continue;
        }

        normalized.push(SyntaxToken {
            range: start..end,
            kind,
        });
    }

    normalized
}
