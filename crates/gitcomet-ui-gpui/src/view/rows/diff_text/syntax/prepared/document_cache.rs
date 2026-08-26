use super::*;

impl TreesitterCachedDocument {
    pub(crate) fn from_chunked_line_tokens(
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
    pub(crate) fn from_line_tokens(
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

    pub(crate) fn chunk_bounds(&self, chunk_ix: usize) -> Range<usize> {
        let start = chunk_ix.saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS);
        let end = start
            .saturating_add(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
            .min(self.line_count);
        start.min(end)..end
    }

    pub(crate) fn source_identity(&self) -> Option<PreparedSyntaxSourceIdentity> {
        self.tree_state
            .as_ref()
            .map(|tree_state| PreparedSyntaxSourceIdentity {
                language: tree_state.language,
                text_ptr: tree_state.text.as_ptr() as usize,
                text_len: tree_state.text.len(),
                line_count: self.line_count,
            })
    }

    pub(crate) fn into_drop_payload(self) -> SyntaxCacheDropPayload {
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
pub(crate) fn chunk_line_tokens_by_row(
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

pub(crate) fn insert_line_token_chunk(
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

pub(crate) fn clone_tree_state_for_chunk_build_ref(
    tree_state: &PreparedSyntaxTreeState,
) -> PreparedSyntaxTreeState {
    #[cfg(test)]
    TS_TREE_STATE_CLONE_COUNT.with(|count| count.set(count.get().saturating_add(1)));
    tree_state.clone()
}

pub(crate) fn shared_tree_state_for_chunk_build(
    tree_state: &Option<PreparedSyntaxTreeState>,
) -> Option<Arc<PreparedSyntaxTreeState>> {
    tree_state
        .as_ref()
        .map(clone_tree_state_for_chunk_build_ref)
        .map(Arc::new)
}

pub(crate) fn build_line_token_chunk_for_state(
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

pub(crate) fn chunk_count_for_line_count(line_count: usize) -> usize {
    if line_count == 0 {
        0
    } else {
        (line_count.saturating_sub(1) / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS).saturating_add(1)
    }
}

pub(crate) struct TreesitterDocumentCache {
    pub(crate) by_cache_key: FxHashMap<PreparedSyntaxCacheKey, TreesitterCachedDocument>,
    pub(crate) by_source_identity: FxHashMap<PreparedSyntaxSourceIdentity, PreparedSyntaxCacheKey>,
    pub(crate) lru_order: VecDeque<PreparedSyntaxCacheKey>,
    pub(crate) pending_chunk_requests: FxHashSet<PreparedSyntaxChunkKey>,
    pub(crate) pending_chunk_request_counts: FxHashMap<PreparedSyntaxCacheKey, usize>,
    pub(crate) metrics: PreparedSyntaxCacheMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedDocumentMergeResult {
    None,
    Inserted,
    Updated,
}

impl TreesitterDocumentCache {
    pub(crate) fn new() -> Self {
        Self {
            by_cache_key: FxHashMap::default(),
            by_source_identity: FxHashMap::default(),
            lru_order: VecDeque::new(),
            pending_chunk_requests: FxHashSet::default(),
            pending_chunk_request_counts: FxHashMap::default(),
            metrics: PreparedSyntaxCacheMetrics::default(),
        }
    }

    pub(crate) fn touch_key(&mut self, cache_key: PreparedSyntaxCacheKey) {
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

    pub(crate) fn record_hit(&mut self, cache_key: PreparedSyntaxCacheKey) {
        self.metrics.hit = self.metrics.hit.saturating_add(1);
        self.touch_key(cache_key);
    }

    pub(crate) fn insert_pending_chunk_request(&mut self, chunk_key: PreparedSyntaxChunkKey) {
        if !self.pending_chunk_requests.insert(chunk_key) {
            return;
        }
        *self
            .pending_chunk_request_counts
            .entry(chunk_key.cache_key)
            .or_default() += 1;
    }

    pub(crate) fn remove_pending_chunk_request(
        &mut self,
        chunk_key: PreparedSyntaxChunkKey,
    ) -> bool {
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

    pub(crate) fn remove_source_identity_mapping(
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

    pub(crate) fn index_source_identity(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        document: &TreesitterCachedDocument,
    ) {
        let Some(identity) = document.source_identity() else {
            return;
        };
        self.by_source_identity.insert(identity, cache_key);
    }

    pub(crate) fn evict_if_needed(&mut self, drop_mode: SyntaxCacheDropMode) {
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

    pub(crate) fn contains_document(
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

    pub(crate) fn document_for_source_identity(
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

    pub(crate) fn alias_source_identity(
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

    pub(crate) fn extract_line_from_chunk(
        &self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
        chunk_ix: usize,
    ) -> Arc<[SyntaxToken]> {
        pub(crate) static EMPTY: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
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
    pub(crate) fn lookup_chunk_state(
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

    pub(crate) fn merge_document_from_shared_seed(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> SharedDocumentMergeResult {
        if !self.by_cache_key.contains_key(&cache_key) {
            let shared_document = {
                let store = match shared_prepared_document_seed_store().lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                store.get(&cache_key).cloned()
            };
            let Some(shared_document) = shared_document else {
                return SharedDocumentMergeResult::None;
            };
            self.evict_if_needed(SyntaxCacheDropMode::DeferredWhenLarge);
            let document = TreesitterCachedDocument::from_chunked_line_tokens(
                shared_document.line_count,
                shared_document.line_token_chunks,
                shared_document.tree_state,
            );
            self.index_source_identity(cache_key, &document);
            self.by_cache_key.insert(cache_key, document);
            self.touch_key(cache_key);
            return SharedDocumentMergeResult::Inserted;
        }

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
            let Some(document) = self.by_cache_key.get_mut(&cache_key) else {
                return SharedDocumentMergeResult::None;
            };

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

        if updated {
            self.touch_key(cache_key);
        }

        if updated {
            SharedDocumentMergeResult::Updated
        } else {
            SharedDocumentMergeResult::None
        }
    }

    #[cfg(test)]
    pub(crate) fn line_tokens(
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

    pub(crate) fn build_chunk_sync_and_insert(
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

    pub(crate) fn queue_chunk_build_request_nonblocking(
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

    pub(crate) fn prefetch_adjacent_chunk_builds_nonblocking(
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

    pub(crate) fn request_line_tokens_with_context(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_ix: usize,
        allow_sync_build_on_insert: bool,
    ) -> Option<PreparedSyntaxLineTokensRequest> {
        let (line_count, chunk_ix, has_chunk) = self.lookup_chunk_state(cache_key, line_ix)?;

        pub(crate) static EMPTY_TOKENS: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
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

    pub(crate) fn request_line_tokens(
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

    pub(crate) fn request_line_tokens_range_into(
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

    pub(crate) fn collect_ready_line_token_requests_for_range(
        &self,
        cache_key: PreparedSyntaxCacheKey,
        line_range: Range<usize>,
        requests: &mut Vec<PreparedSyntaxLineTokensRequest>,
    ) -> Option<PreparedSyntaxLineTokensRangeSummary> {
        pub(crate) static EMPTY_TOKENS: OnceLock<Arc<[SyntaxToken]>> = OnceLock::new();
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

    pub(crate) fn prepared_document_data(
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

    pub(crate) fn tree_state(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> Option<PreparedSyntaxTreeState> {
        let tree_state = self.by_cache_key.get(&cache_key)?.tree_state.clone();
        self.touch_key(cache_key);
        tree_state
    }

    pub(crate) fn tree_state_is_available(&mut self, cache_key: PreparedSyntaxCacheKey) -> bool {
        let mut available = self
            .by_cache_key
            .get(&cache_key)
            .is_some_and(|document| document.tree_state.is_some());
        if !available {
            self.merge_document_from_shared_seed(cache_key);
            available = self
                .by_cache_key
                .get(&cache_key)
                .is_some_and(|document| document.tree_state.is_some());
        }
        if available {
            self.touch_key(cache_key);
        }
        available
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(crate) fn metrics(&self) -> PreparedSyntaxCacheMetrics {
        self.metrics
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(crate) fn reset_metrics(&mut self) {
        self.metrics = PreparedSyntaxCacheMetrics::default();
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(crate) fn loaded_chunk_count(&self, cache_key: PreparedSyntaxCacheKey) -> Option<usize> {
        Some(self.by_cache_key.get(&cache_key)?.line_token_chunks.len())
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(crate) fn contains_key(&self, cache_key: PreparedSyntaxCacheKey) -> bool {
        self.by_cache_key.contains_key(&cache_key)
    }

    pub(crate) fn drain_completed_chunk_builds(&mut self) -> usize {
        self.drain_completed_chunk_builds_matching(None)
    }

    pub(crate) fn drain_completed_chunk_builds_for_cache_key(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> usize {
        if !self.has_pending_chunk_requests_for_cache_key(cache_key) {
            return 0;
        }
        self.drain_completed_chunk_builds_matching(Some(cache_key))
    }

    pub(crate) fn drain_completed_chunk_builds_matching(
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

    pub(crate) fn has_pending_chunk_requests(&self) -> bool {
        !self.pending_chunk_requests.is_empty()
    }

    pub(crate) fn has_pending_chunk_requests_for_cache_key(
        &self,
        cache_key: PreparedSyntaxCacheKey,
    ) -> bool {
        self.pending_chunk_request_counts.contains_key(&cache_key)
    }

    #[cfg(test)]
    pub(crate) fn make_test_cache_key(doc_hash: u64) -> PreparedSyntaxCacheKey {
        PreparedSyntaxCacheKey {
            language: DiffSyntaxLanguage::Rust,
            doc_hash,
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_document(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        line_tokens: Vec<Vec<SyntaxToken>>,
    ) {
        self.insert_document_with_tree_state(cache_key, line_tokens, None);
    }

    #[cfg(test)]
    pub(crate) fn insert_document_with_tree_state(
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

    pub(crate) fn insert_document_with_mode(
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
