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
    pub(super) by_key: HashMap<SingleLineSyntaxTokenCacheKey, CachedSingleLineSyntaxTokens>,
    pub(super) lru_order: VecDeque<SingleLineSyntaxTokenCacheKey>,
}

impl SingleLineSyntaxTokenCache {
    pub(super) fn new() -> Self {
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
    pub(super) line_token_chunks: HashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    pub(super) line_token_bytes: usize,
    pub(super) tree_state: Option<PreparedSyntaxTreeState>,
}

impl TreesitterCachedDocument {
    pub(super) fn from_chunked_line_tokens(
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
    by_cache_key: HashMap<PreparedSyntaxCacheKey, TreesitterCachedDocument>,
    by_source_identity: HashMap<PreparedSyntaxSourceIdentity, PreparedSyntaxCacheKey>,
    lru_order: VecDeque<PreparedSyntaxCacheKey>,
    pending_chunk_requests: HashSet<PreparedSyntaxChunkKey>,
    pending_chunk_request_counts: HashMap<PreparedSyntaxCacheKey, usize>,
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
            by_cache_key: HashMap::default(),
            by_source_identity: HashMap::default(),
            lru_order: VecDeque::new(),
            pending_chunk_requests: HashSet::default(),
            pending_chunk_request_counts: HashMap::default(),
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
                    .filter(|&(&chunk_ix, _)| chunk_ix < chunk_limit)
                    .map(|(&chunk_ix, chunk)| (chunk_ix, chunk.clone()))
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TreesitterQueryPass {
    pub(super) byte_range: Range<usize>,
    pub(super) containing_byte_range: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct TreesitterInjectionMatch {
    pub(super) language: DiffSyntaxLanguage,
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
    /// Hash of the injection content bytes. This ensures the cache is not
    /// confused when different parent documents happen to produce injection
    /// regions at the same byte offsets.
    pub(super) content_hash: u64,
}

pub(super) struct CachedInjectionTokens {
    /// Full tokenized lines in injection-local coordinates (all lines of the injection).
    pub(super) all_line_tokens: Vec<Vec<SyntaxToken>>,
    /// Line starts for the injection text, used for coordinate remapping.
    pub(super) injection_line_starts: Vec<usize>,
    /// First line in the parent document that this injection starts on.
    pub(super) injection_start_line_ix: usize,
    /// Monotonic access counter for LRU eviction.
    pub(super) last_access: u64,
}

#[derive(Clone, Copy)]
pub(super) struct TreesitterQueryAsset {
    pub(super) highlights: &'static str,
    pub(super) injections: Option<&'static str>,
}

impl TreesitterQueryAsset {
    pub(super) const fn highlights(source: &'static str) -> Self {
        Self {
            highlights: source,
            injections: None,
        }
    }

    pub(super) const fn with_injections(
        highlights: &'static str,
        injections: &'static str,
    ) -> Self {
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
        doc_hash: treesitter_text_hash(input),
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

pub(super) fn collect_treesitter_document_line_tokens_for_line_window(
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
        return Vec::new();
    }

    let mut seen = HashSet::default();
    let mut injections = Vec::new();
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

fn injection_language_from_name(name: &str) -> Option<DiffSyntaxLanguage> {
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

pub(super) fn normalize_non_overlapping_tokens(mut tokens: Vec<SyntaxToken>) -> Vec<SyntaxToken> {
    if tokens.len() <= 1 {
        return tokens;
    }

    let mut indexed_tokens = tokens
        .into_iter()
        .enumerate()
        .collect::<Vec<(usize, SyntaxToken)>>();
    indexed_tokens.sort_unstable_by(|(a_ix, a), (b_ix, b)| {
        a.range
            .start
            .cmp(&b.range.start)
            .then(a.range.end.cmp(&b.range.end))
            .then(a_ix.cmp(b_ix))
    });
    tokens = indexed_tokens.into_iter().map(|(_, token)| token).collect();

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
