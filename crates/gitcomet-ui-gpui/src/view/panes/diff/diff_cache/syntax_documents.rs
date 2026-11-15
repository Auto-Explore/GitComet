use super::*;

// One initial read plus three retries. A missing/locked source must not create
// one whole-document request per frame while the visible rows remain unchanged.
const FILE_DIFF_SYNTAX_RETRY_DELAYS: [std::time::Duration; 3] = [
    std::time::Duration::from_millis(250),
    std::time::Duration::from_secs(1),
    std::time::Duration::from_secs(4),
];

#[derive(Debug)]
enum FileDiffSideSyntaxFailure {
    Read(std::io::ErrorKind),
    SourceMoved,
    LinesMoved,
    Empty,
    TooLarge,
    Unparsed,
}

impl FileDiffSideSyntaxFailure {
    fn may_retry(&self) -> bool {
        matches!(self, Self::Read(kind) if *kind != std::io::ErrorKind::InvalidData)
    }
}

impl DiffRenderer {
    pub(super) fn prepared_syntax_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let document = self.prepared_syntax_documents.get(key)?.document;
        rows::prepared_diff_syntax_document_is_available(document).then_some(document)
    }

    pub(super) fn prepared_syntax_reparse_seed_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        self.prepared_syntax_documents
            .iter()
            .filter(|(candidate_key, _)| {
                candidate_key.repo_id == key.repo_id
                    && candidate_key.file_path == key.file_path
                    && candidate_key.view_mode == key.view_mode
                    && candidate_key.target_rev != key.target_rev
            })
            .max_by_key(|(candidate_key, _)| candidate_key.target_rev)
            .map(|(_, cached)| cached.document)
    }

    pub(super) fn insert_prepared_syntax_document(
        &mut self,
        key: PreparedSyntaxDocumentKey,
        document: rows::PreparedDiffSyntaxDocument,
    ) -> bool {
        if key.view_mode != PreparedSyntaxViewMode::WorktreePreview {
            // Only this file's current sides can be exact hits. Retain an older
            // side until its replacement is installed, so an in-progress
            // reparse may still borrow it; previews keep their per-path rule.
            self.prepared_syntax_documents.retain(|cached, _| {
                cached.view_mode == PreparedSyntaxViewMode::WorktreePreview
                    || (cached.repo_id == key.repo_id
                        && cached.file_path == key.file_path
                        && (cached.view_mode != key.view_mode
                            || cached.target_rev >= key.target_rev))
            });
        }
        if let Some(cached) = self.prepared_syntax_documents.get_mut(&key) {
            let changed = cached.document != document;
            cached.document = document;
            return changed;
        }
        if self.prepared_syntax_documents.len() >= PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES
            && let Some(evict_key) = self
                .prepared_syntax_documents
                .iter()
                .filter(|(cached, _)| {
                    cached.repo_id != key.repo_id || cached.file_path != key.file_path
                })
                .min_by_key(|(_, cached)| cached.insertion_sequence)
                .map(|(key, _)| key.clone())
        {
            self.prepared_syntax_documents.remove(&evict_key);
        }
        self.prepared_syntax_document_insert_sequence = self
            .prepared_syntax_document_insert_sequence
            .wrapping_add(1);
        self.prepared_syntax_documents.insert(
            key,
            CachedPreparedSyntaxDocument {
                document,
                insertion_sequence: self.prepared_syntax_document_insert_sequence,
            },
        );
        true
    }

    /// Injection can revive a handle that is still in the view map. Observe
    /// availability first so rows cached while it was absent get recoloured.
    pub(super) fn install_file_diff_side_document(
        &mut self,
        side: DiffTextRegion,
        key: PreparedSyntaxDocumentKey,
        make_document: impl FnOnce() -> Option<rows::PreparedDiffSyntaxDocument>,
    ) -> bool {
        let was_resolvable = self.prepared_syntax_document(&key).is_some();
        let sides = [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight];
        let before = sides.map(|side| {
            self.file_diff_split_prepared_syntax_document(side)
                .is_some()
        });
        let Some(document) = make_document() else {
            return false;
        };
        let inserted = self.insert_prepared_syntax_document(key, document);
        let installed_side_changed = inserted || !was_resolvable;
        let mut changed = false;
        for (ix, candidate) in sides.into_iter().enumerate() {
            let available = self
                .file_diff_split_prepared_syntax_document(candidate)
                .is_some();
            // Equal old/new texts may share a handle: one injection can revive
            // both sides. A different pane may also have revived our handle
            // since this pane last built fallback rows.
            let became_available =
                available && (!before[ix] || self.file_diff_syntax_availability[ix] == Some(false));
            if became_available
                || (candidate == Self::split_side_for_region(side) && installed_side_changed)
            {
                self.bump_file_diff_side_style_epoch(candidate);
                changed = true;
            }
            self.file_diff_syntax_availability[ix] = Some(available);
        }
        changed
    }

    fn bump_file_diff_side_style_epoch(&mut self, side: DiffTextRegion) {
        match side {
            DiffTextRegion::SplitLeft => self.file_diff_style_cache_epochs.bump_left(),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                self.file_diff_style_cache_epochs.bump_right()
            }
        }
    }

    /// Called once per visible side's render pass, before styled-row lookup.
    /// Availability can change without an install in this pane because the
    /// backing documents are shared between panes and may be evicted/revived.
    pub(in crate::view) fn observe_file_diff_side_syntax_availability(
        &mut self,
        side: DiffTextRegion,
    ) -> bool {
        let available = self
            .file_diff_split_prepared_syntax_document(side)
            .is_some();
        let ix = usize::from(side != DiffTextRegion::SplitLeft);
        if self.file_diff_syntax_availability[ix] == Some(false) && available {
            self.bump_file_diff_side_style_epoch(side);
        }
        self.file_diff_syntax_availability[ix] = Some(available);
        available
    }

    pub(super) fn rekey_prepared_syntax_document(
        &mut self,
        old_key: PreparedSyntaxDocumentKey,
        new_key: PreparedSyntaxDocumentKey,
    ) {
        if old_key == new_key {
            return;
        }
        let Some(document) = self.prepared_syntax_documents.remove(&old_key) else {
            return;
        };
        self.prepared_syntax_documents
            .entry(new_key)
            .or_insert(document);
    }

    pub(super) fn rekey_file_diff_prepared_syntax_documents_for_rev(&mut self, new_rev: u64) {
        let Some(repo_id) = self.file_diff_cache_repo_id else {
            return;
        };
        let Some(path) = self.file_diff_cache_path.clone() else {
            return;
        };
        let old_rev = self.file_diff_cache_rev;
        if old_rev == new_rev {
            return;
        }

        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            let old_key = prepared_syntax_document_key(repo_id, old_rev, &path, view_mode);
            let new_key = prepared_syntax_document_key(repo_id, new_rev, &path, view_mode);
            self.rekey_prepared_syntax_document(old_key, new_key);
        }
    }

    pub(super) fn remove_file_diff_prepared_syntax_documents_for_rev(
        &mut self,
        repo_id: DiffSourceId,
        rev: u64,
        path: &std::path::Path,
    ) {
        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            let key = prepared_syntax_document_key(repo_id, rev, path, view_mode);
            self.prepared_syntax_documents.remove(&key);
        }
    }

    /// Every preview load has a new rev, so the parse under `key` supersedes the
    /// file's older ones: drop them rather than let them fill the bounded map.
    fn insert_worktree_preview_syntax_document(
        &mut self,
        key: PreparedSyntaxDocumentKey,
        document: rows::PreparedDiffSyntaxDocument,
    ) -> bool {
        self.prepared_syntax_documents.retain(|cached, _| {
            cached.view_mode != PreparedSyntaxViewMode::WorktreePreview
                || cached.repo_id != key.repo_id
                || cached.file_path != key.file_path
                || cached.target_rev == key.target_rev
        });
        self.insert_prepared_syntax_document(key, document)
    }

    pub(in crate::view) fn full_document_syntax_budget(&self) -> rows::DiffSyntaxBudget {
        #[cfg(test)]
        if let Some(budget) = self.diff_syntax_budget_override {
            return budget;
        }

        rows::default_diff_syntax_budget()
    }

    #[cfg(test)]
    pub(in crate::view) fn set_full_document_syntax_budget_override_for_tests(
        &mut self,
        budget: rows::DiffSyntaxBudget,
    ) {
        self.live_syntax_unbounded_for_tests = false;
        self.diff_syntax_budget_override = Some(budget);
    }

    pub(in crate::view) fn live_syntax_foreground_budget(&self) -> Option<std::time::Duration> {
        #[cfg(test)]
        if self.live_syntax_unbounded_for_tests {
            return None;
        }
        Some(self.full_document_syntax_budget().foreground_parse)
    }

    /// Turns the eager source-backed prepare off so a test can drive the
    /// click-triggered path it would otherwise pre-empt.
    #[cfg(test)]
    pub(in crate::view) fn set_eager_source_backed_syntax_prepare_for_tests(
        &mut self,
        enabled: bool,
    ) {
        self.eager_source_backed_syntax_prepare = enabled;
    }

    /// The bytes a side's prepared document was built from.
    ///
    /// A source-backed side keeps no resident text, so the body retained
    /// alongside its prepared document is the only copy. Streamed long-line rows
    /// measure their slice against this; handed the empty resident text they
    /// compute a zero-length document and paint no syntax at all, silently.
    pub(in crate::view) fn file_diff_side_document_text(
        &self,
        region: DiffTextRegion,
    ) -> SharedString {
        let side = Self::split_side_for_region(region);
        let resident = match side {
            DiffTextRegion::SplitLeft => &self.file_diff_old_text,
            _ => &self.file_diff_new_text,
        };
        if !resident.is_empty() {
            return resident.clone();
        }
        self.file_diff_pair_syntax_text
            .get(&side)
            .cloned()
            .unwrap_or_default()
    }

    pub(in crate::view) fn file_diff_prepared_syntax_key(
        &self,
        view_mode: PreparedSyntaxViewMode,
    ) -> Option<PreparedSyntaxDocumentKey> {
        let repo_id = self.file_diff_cache_repo_id?;
        let path = self.file_diff_cache_path.as_ref()?;
        Some(prepared_syntax_document_key(
            repo_id,
            self.file_diff_cache_rev,
            path,
            view_mode,
        ))
    }

    pub(super) fn file_diff_prepared_syntax_document(
        &self,
        view_mode: PreparedSyntaxViewMode,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let key = self.file_diff_prepared_syntax_key(view_mode)?;
        self.prepared_syntax_document(&key)
    }

    /// One side's document for a click-time lookup, preparing small cold
    /// documents inline if the render path has not left one behind.
    ///
    /// The rendered document is keyed by the diff revision and is genuinely
    /// often absent: an unstaged file's revision moves under it as the worktree
    /// is polled, the first paint can blow its 1 ms foreground budget and defer
    /// to a background build, and this view-level map holds only a handful of
    /// entries. Rendering copes because it falls back to per-line tokens; a
    /// delimiter pair cannot, because it needs the whole tree.
    ///
    /// Small files are prepared here instead of depending on having won that
    /// race, from the in-memory text when there is one and from the side's file
    /// when there is not. Larger syntax-sized files return `None` here and are
    /// handed to [`Self::request_file_diff_click_syntax_document`], which
    /// replays the click after its worker completes. When rendering got there
    /// first this costs nothing: the cached document returns immediately.
    /// Whether a source-backed side's file is still the one this generation
    /// indexed.
    ///
    /// A worktree file can change under an open diff -- the poll is 250 ms --
    /// and the click path reads that file back, while the rows are per-line
    /// slices of whatever it holds now. Any answer computed across that boundary
    /// is projected onto columns it does not describe, so a changed file means
    /// decline until the rebuild catches up. Sides with no file (an in-memory
    /// blob) cannot go stale and always pass.
    ///
    /// One `stat`, on a click.
    pub(super) fn file_diff_source_is_current(&self, side: DiffTextRegion) -> bool {
        let (source_path, indexed) = match side {
            DiffTextRegion::SplitLeft => (
                self.file_diff_old_source_path.as_ref(),
                self.file_diff_old_source_identity.as_ref(),
            ),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => (
                self.file_diff_new_source_path.as_ref(),
                self.file_diff_new_source_identity.as_ref(),
            ),
        };
        let Some(source_path) = source_path else {
            return true;
        };
        let Some(indexed) = indexed else {
            return true;
        };
        file_diff_source_identity(Some(source_path)).as_deref() == Some(indexed.as_ref())
    }

    /// The split side a region reads its source from. Inline shares the new
    /// side's file, so only the left side is its own source.
    pub(super) fn split_side_for_region(region: DiffTextRegion) -> DiffTextRegion {
        match region {
            DiffTextRegion::SplitLeft => DiffTextRegion::SplitLeft,
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => DiffTextRegion::SplitRight,
        }
    }

    pub(in crate::view) fn file_diff_pair_syntax_document(
        &mut self,
        region: DiffTextRegion,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let side = Self::split_side_for_region(region);
        // A prepared cache hit is still source-dependent. The file can move in
        // the 250 ms before the diff revision advances, while rows already read
        // its current bytes, so freshness must precede every return path.
        //
        // A source-backed side is re-read further down, and a retained body is a
        // read from an earlier click. Both describe the file as it was, and the
        // rows beside them are slices of the file as it is -- so neither may be
        // used once the file has moved on. The retained body is why this has to
        // come first: it never re-reads, and would otherwise answer from bytes
        // that are two edits old.
        if !self.file_diff_source_is_current(side) {
            self.file_diff_pair_syntax_text.remove(&side);
            return None;
        }
        if self.observe_file_diff_side_syntax_availability(side) {
            return self.file_diff_split_prepared_syntax_document(region);
        }
        let language = self.file_diff_cache_language?;
        let (text, line_starts, source_path) = match region {
            DiffTextRegion::SplitLeft => (
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
                self.file_diff_old_source_path.clone(),
            ),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => (
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
                self.file_diff_new_source_path.clone(),
            ),
        };
        let source_path = source_path.as_ref();
        // A source-backed side keeps its text out of memory on purpose, so a
        // huge diff can render from per-line slices. That is the ordinary case
        // for a worktree file, whose new side *is* the file, and it leaves
        // nothing to parse a document from. Small sources are read back here;
        // larger syntax-sized sources are deliberately left for the worker.
        let text = if text.is_empty() {
            match self.file_diff_pair_syntax_text.get(&side) {
                // Kept from an earlier click: the same allocation, so the parse
                // below resolves by source identity instead of re-parsing. The
                // guard above is what keeps it honest -- it is dropped when the
                // cache rebuilds *and* when the file behind it changes.
                Some(retained) => retained.clone(),
                None => {
                    let path = source_path?;
                    let len = std::fs::metadata(path.as_ref()).ok()?.len();
                    // Availability follows the prepared document's 8 MiB
                    // ceiling, but only the smaller foreground allowance may
                    // read synchronously. The request worker handles the rest.
                    if len > rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES as u64 {
                        return None;
                    }
                    if len > DIFF_CLICK_FOREGROUND_COMPLETION_MAX_TEXT_BYTES as u64 {
                        return None;
                    }
                    let read = SharedString::from(std::fs::read_to_string(path.as_ref()).ok()?);
                    // The rows' line numbers, and the `line_starts` the parse is
                    // handed, were indexed from this file when the diff was
                    // built. A worktree file can have changed since -- the poll
                    // is 250 ms -- and then the index describes a document this
                    // text no longer is: at best the pair lands on the wrong
                    // line, at worst a start sits past the end of the shortened
                    // text. Decline until the rebuild catches up.
                    if !line_starts_describe(read.as_ref(), line_starts.as_ref()) {
                        return None;
                    }
                    self.file_diff_pair_syntax_text.insert(side, read.clone());
                    read
                }
            }
        } else {
            text.clone()
        };
        if text.is_empty() || line_starts.is_empty() {
            return None;
        }
        // Text the view already holds is bounded by the ceiling the *render*
        // path uses, not by the occurrence one: nothing is read here, so the
        // only cost is the parse, and using a lower ceiling would mean the same
        // file paired or did not depending on whether the render path had won
        // the race to prepare its document. The occurrence scan, which really is
        // O(document), keeps its own ceiling inside
        // `prepared_document_occurrences_at_display_offset`.
        if text.len() > rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES {
            return None;
        }
        // A click is not a frame: it can afford a real parse where the render
        // path deliberately cannot. It is still the UI thread, though, so the
        // budget is the whole of what a click may spend here.
        let budget = rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_millis(50),
        };
        let mode = match side {
            DiffTextRegion::SplitLeft => PreparedSyntaxViewMode::FileDiffSplitLeft,
            _ => PreparedSyntaxViewMode::FileDiffSplitRight,
        };
        let key = self.file_diff_prepared_syntax_key(mode)?;
        let mut prepared = None;
        self.install_file_diff_side_document(side, key, || {
            prepared = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                text,
                line_starts,
                budget,
                None,
                None,
            ) {
                rows::PrepareDiffSyntaxDocumentResult::Ready(document) => Some(document),
                // A timed-out click records a pending selection and lets the
                // worker reuse the retained body, then replay the click.
                rows::PrepareDiffSyntaxDocumentResult::TimedOut
                | rows::PrepareDiffSyntaxDocumentResult::Unsupported => None,
            };
            prepared
        });
        prepared
    }

    /// Prepares a whole-document tree for every side whose content is a file.
    ///
    /// [`Self::refresh_file_diff_syntax_documents`] can only work from resident
    /// text, and a worktree diff has none on either side. Rows then fall back to
    /// the single-line tokenizer, which cannot see anything spanning lines: a
    /// `<script>` body renders bare, `</div>` loses its tag name, and a block
    /// comment loses every line after its first.
    ///
    /// Deliberately not solved by materializing text during the rebuild. That is
    /// what keeps a multi-megabyte diff streaming per-line instead of resident,
    /// and the worker reads the file once, off-thread, under the same 8 MiB
    /// ceiling the read-only preview already uses.
    fn refresh_source_backed_file_diff_syntax_documents(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(test)]
        if !self.eager_source_backed_syntax_prepare {
            return;
        }
        if self.file_diff_cache_language.is_none() {
            return;
        }
        for side in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight] {
            let (resident_text, source_path) = match side {
                DiffTextRegion::SplitLeft => (
                    &self.file_diff_old_text,
                    self.file_diff_old_source_path.as_ref(),
                ),
                _ => (
                    &self.file_diff_new_text,
                    self.file_diff_new_source_path.as_ref(),
                ),
            };
            // `source_path` is `Some` only for a side backed by a file, so a
            // genuinely empty side -- the old half of an added file -- never
            // enters the in-flight map.
            if !resident_text.is_empty() || source_path.is_none() {
                continue;
            }
            if self
                .file_diff_split_prepared_syntax_document(side)
                .is_some()
            {
                continue;
            }
            self.request_file_diff_side_syntax_document(side, cx);
        }
    }

    /// Rendering only reserves one deferred request per side. Filesystem work
    /// is deferred out of render; reads and parses use the existing side worker.
    pub(in crate::view) fn ensure_file_diff_side_syntax(
        &mut self,
        region: DiffTextRegion,
        cx: &mut gpui::Context<Self>,
    ) {
        let side = Self::split_side_for_region(region);
        let available = self.observe_file_diff_side_syntax_availability(side);
        #[cfg(test)]
        if !self.eager_source_backed_syntax_prepare {
            return;
        }
        let (source_backed, has_text) = match side {
            DiffTextRegion::SplitLeft => (
                self.file_diff_old_source_path.is_some(),
                !self.file_diff_old_text.is_empty(),
            ),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => (
                self.file_diff_new_source_path.is_some(),
                !self.file_diff_new_text.is_empty(),
            ),
        };
        if (!source_backed && !has_text)
            || (!source_backed && self.file_diff_resident_syntax_inflight != 0)
            || self.file_diff_cache_language.is_none()
            || self.file_diff_click_syntax_inflight.contains_key(&side)
            || self.file_diff_side_syntax_deferred.contains_key(&side)
            || self
                .file_diff_side_syntax_attempts
                .get(&side)
                .is_some_and(|attempt| attempt.stopped || attempt.retry_pending)
            || available
        {
            return;
        }
        let generation = self.file_diff_syntax_generation;
        self.file_diff_side_syntax_deferred.insert(side, generation);
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = view.update(cx, |this, cx| {
                if this.file_diff_side_syntax_deferred.get(&side) != Some(&generation) {
                    return;
                }
                this.file_diff_side_syntax_deferred.remove(&side);
                if this.file_diff_syntax_generation == generation {
                    this.request_file_diff_side_syntax_document(side, cx);
                }
            });
        });
    }

    fn file_diff_side_syntax_identity(&self, side: DiffTextRegion) -> Option<Arc<str>> {
        match side {
            DiffTextRegion::SplitLeft => self.file_diff_old_source_identity.clone(),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                self.file_diff_new_source_identity.clone()
            }
        }
    }

    fn fail_file_diff_side_syntax(
        &mut self,
        side: DiffTextRegion,
        failure: FileDiffSideSyntaxFailure,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(attempt) = self.file_diff_side_syntax_attempts.get_mut(&side) else {
            return;
        };
        let delay = if failure.may_retry() {
            FILE_DIFF_SYNTAX_RETRY_DELAYS.get(attempt.retries).copied()
        } else {
            None
        };
        let Some(delay) = delay else {
            attempt.stopped = true;
            attempt.retry_pending = false;
            self.clear_pending_diff_text_syntax_click_for(side);
            return;
        };
        attempt.retries += 1;
        attempt.retry_pending = true;
        let retries = attempt.retries;
        let generation = attempt.generation;
        let identity = attempt.identity.clone();
        cx.spawn(
            async move |view: WeakEntity<DiffRenderer>, cx: &mut gpui::AsyncApp| {
                cx.background_executor().timer(delay).await;
                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_syntax_generation != generation
                        || this.file_diff_side_syntax_identity(side) != identity
                        || !this
                            .file_diff_side_syntax_attempts
                            .get(&side)
                            .is_some_and(|attempt| {
                                attempt.generation == generation
                                    && attempt.identity == identity
                                    && attempt.retries == retries
                                    && attempt.retry_pending
                            })
                    {
                        return;
                    }
                    if this.file_diff_source_is_current(side)
                        && this.observe_file_diff_side_syntax_availability(side)
                    {
                        this.file_diff_side_syntax_attempts.remove(&side);
                        this.retry_pending_diff_text_syntax_click();
                        cx.notify();
                        return;
                    }
                    this.file_diff_side_syntax_attempts
                        .get_mut(&side)
                        .unwrap()
                        .retry_pending = false;
                    this.request_file_diff_side_syntax_document(side, cx);
                });
            },
        )
        .detach();
    }

    /// An explicit click may retry an exhausted side, but ordinary frames may
    /// not reset its budget. A scheduled retry already owns the pending click.
    pub(in crate::view) fn request_file_diff_click_syntax_document(
        &mut self,
        region: DiffTextRegion,
        cx: &mut gpui::Context<Self>,
    ) {
        let side = Self::split_side_for_region(region);
        if self
            .file_diff_side_syntax_attempts
            .get(&side)
            .is_some_and(|attempt| attempt.stopped)
        {
            self.file_diff_side_syntax_attempts.remove(&side);
        }
        self.request_file_diff_side_syntax_document(side, cx);
    }

    /// Prepares one side off the UI thread. Render requests, retries and cold
    /// clicks share the same ownership, size and source-freshness checks.
    pub(in crate::view) fn request_file_diff_side_syntax_document(
        &mut self,
        side: DiffTextRegion,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_diff_click_syntax_inflight.contains_key(&side)
            || self
                .file_diff_side_syntax_attempts
                .get(&side)
                .is_some_and(|attempt| attempt.stopped || attempt.retry_pending)
        {
            return;
        }
        let source_is_current = self.file_diff_source_is_current(side);
        if !source_is_current {
            self.file_diff_pair_syntax_text.remove(&side);
        }
        if source_is_current && self.observe_file_diff_side_syntax_availability(side) {
            self.retry_pending_diff_text_syntax_click();
            cx.notify();
            return;
        }
        let syntax_generation = self.file_diff_syntax_generation;
        let indexed_identity = self.file_diff_side_syntax_identity(side);
        self.file_diff_side_syntax_attempts
            .entry(side)
            .or_insert_with(|| FileDiffSideSyntaxAttempt {
                generation: syntax_generation,
                identity: indexed_identity.clone(),
                retries: 0,
                retry_pending: false,
                stopped: false,
            });
        let Some(language) = self.file_diff_cache_language else {
            self.fail_file_diff_side_syntax(side, FileDiffSideSyntaxFailure::Unparsed, cx);
            return;
        };
        let (resident_text, line_starts, source_path, view_mode) = match side {
            DiffTextRegion::SplitLeft => (
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
                self.file_diff_old_source_path.clone(),
                PreparedSyntaxViewMode::FileDiffSplitLeft,
            ),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => (
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
                self.file_diff_new_source_path.clone(),
                PreparedSyntaxViewMode::FileDiffSplitRight,
            ),
        };
        let retained_text = self.file_diff_pair_syntax_text.get(&side).cloned();
        let text = (!resident_text.is_empty())
            .then_some(resident_text)
            .or(retained_text);
        let Some(key) = self.file_diff_prepared_syntax_key(view_mode) else {
            self.fail_file_diff_side_syntax(side, FileDiffSideSyntaxFailure::Unparsed, cx);
            return;
        };
        self.file_diff_click_syntax_inflight
            .insert(side, syntax_generation);
        let repo_id = self.file_diff_cache_repo_id;
        let diff_target = self.file_diff_cache_target.clone();
        let source_backed = text.is_none();
        #[cfg(test)]
        let after_prepare_hook = self.file_diff_click_syntax_after_prepare_hook.clone();
        #[cfg(test)]
        let before_complete_hook = self.file_diff_click_syntax_before_complete_hook.clone();

        cx.spawn(
            async move |view: WeakEntity<DiffRenderer>, cx: &mut gpui::AsyncApp| {
                // A superseded queued request must not read or parse a file merely
                // to reject its result later.
                let still_current = view
                    .read_with(cx, |this, _| {
                        this.file_diff_syntax_generation == syntax_generation
                            && this.file_diff_side_syntax_identity(side) == indexed_identity
                    })
                    .unwrap_or(false);
                if !still_current {
                    let _ = view.update(cx, |this, _| {
                        if this.file_diff_click_syntax_inflight.get(&side)
                            == Some(&syntax_generation)
                        {
                            this.file_diff_click_syntax_inflight.remove(&side);
                        }
                    });
                    return;
                }
                let prepared_identity = indexed_identity.clone();
                let prepare_document = move || {
                    use FileDiffSideSyntaxFailure as Failure;
                    let text = match text {
                        Some(text) => text,
                        None => {
                            let path = source_path.ok_or(Failure::Empty)?;
                            let metadata = std::fs::metadata(path.as_ref())
                                .map_err(|error| Failure::Read(error.kind()))?;
                            if metadata.len()
                                > rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES as u64
                            {
                                return Err(Failure::TooLarge);
                            }
                            // A temporarily replaced directory is a read error,
                            // not proof that a new text generation has arrived.
                            if metadata.is_file()
                                && prepared_identity.as_ref().is_some_and(|indexed| {
                                    file_diff_source_identity(Some(&path)).as_deref()
                                        != Some(indexed.as_ref())
                                })
                            {
                                return Err(Failure::SourceMoved);
                            }
                            let read = SharedString::from(
                                std::fs::read_to_string(path.as_ref())
                                    .map_err(|error| Failure::Read(error.kind()))?,
                            );
                            if prepared_identity.as_ref().is_some_and(|indexed| {
                                file_diff_source_identity(Some(&path)).as_deref()
                                    != Some(indexed.as_ref())
                            }) {
                                return Err(Failure::SourceMoved);
                            }
                            read
                        }
                    };
                    if text.is_empty() {
                        return Err(Failure::Empty);
                    }
                    if text.len() > rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES {
                        return Err(Failure::TooLarge);
                    }
                    if !line_starts_describe(text.as_ref(), line_starts.as_ref()) {
                        return Err(Failure::LinesMoved);
                    }
                    let document =
                        rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                            language,
                            FULL_DOCUMENT_SYNTAX_MODE,
                            text.clone(),
                            line_starts,
                            None,
                            None,
                        )
                        .ok_or(Failure::Unparsed)?;
                    Ok((text, document))
                };
                let prepared = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(prepare_document).await
                } else {
                    prepare_document()
                };
                #[cfg(test)]
                if prepared.is_ok()
                    && let Some(hook) = after_prepare_hook
                {
                    hook();
                }
                let _ = view.update(cx, |this, cx| {
                    #[cfg(test)]
                    if let Some(hook) = before_complete_hook {
                        hook(this);
                    }
                    if this.file_diff_click_syntax_inflight.get(&side) == Some(&syntax_generation) {
                        this.file_diff_click_syntax_inflight.remove(&side);
                    }
                    if this.file_diff_syntax_generation != syntax_generation
                        || this.file_diff_cache_repo_id != repo_id
                        || this.file_diff_cache_target != diff_target
                        || this.file_diff_side_syntax_identity(side) != indexed_identity
                    {
                        return;
                    }
                    // Preserve a read failure before re-statting: a sharing failure
                    // or temporary replacement can end before the retry, with the
                    // original indexed identity restored and no new store revision.
                    let (text, document) = match prepared {
                        Ok(prepared) => prepared,
                        Err(failure) => {
                            this.fail_file_diff_side_syntax(side, failure, cx);
                            return;
                        }
                    };
                    if !this.file_diff_source_is_current(side) {
                        this.file_diff_pair_syntax_text.remove(&side);
                        this.fail_file_diff_side_syntax(
                            side,
                            FileDiffSideSyntaxFailure::SourceMoved,
                            cx,
                        );
                        return;
                    }
                    this.file_diff_side_syntax_attempts.remove(&side);
                    if source_backed {
                        // Keep the identity's source allocation alive for future
                        // document lookups and occurrence scans.
                        this.file_diff_pair_syntax_text.insert(side, text);
                    }
                    // Revision-only refreshes retain the indexed generation
                    // and coalesce with this worker. Install under its current
                    // revision, just as already-prepared documents are rekeyed.
                    this.install_file_diff_side_document(
                        side,
                        PreparedSyntaxDocumentKey {
                            target_rev: this.file_diff_cache_rev,
                            ..key
                        },
                        || {
                            Some(rows::inject_background_prepared_diff_syntax_document(
                                document,
                            ))
                        },
                    );
                    this.retry_pending_diff_text_syntax_click();
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn file_diff_split_style_cache_epoch(&self, region: DiffTextRegion) -> u64 {
        self.file_diff_style_cache_epochs.split_epoch(region)
    }

    pub(in crate::view) fn file_diff_inline_style_cache_epoch(
        &self,
        line: &AnnotatedDiffLine,
    ) -> u64 {
        self.file_diff_style_cache_epochs.inline_epoch(line.kind)
    }

    /// Project inline-diff syntax from the real old/new (split) documents.
    ///
    /// Instead of parsing the synthetic mixed inline stream, project each row into
    /// the correct real old/new document using its 1-based diff line numbers.
    pub(in crate::view) fn file_diff_inline_projected_syntax(
        &self,
        line: &AnnotatedDiffLine,
    ) -> rows::PreparedDiffSyntaxLine {
        rows::prepared_diff_syntax_line_for_inline_diff_row(
            self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
            self.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            line,
        )
    }

    pub(in crate::view) fn file_diff_split_prepared_syntax_document(
        &self,
        region: DiffTextRegion,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let view_mode = match region {
            DiffTextRegion::SplitLeft => PreparedSyntaxViewMode::FileDiffSplitLeft,
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                PreparedSyntaxViewMode::FileDiffSplitRight
            }
        };
        self.file_diff_prepared_syntax_document(view_mode)
    }

    #[cfg(test)]
    pub(in crate::view) fn cache_file_diff_pair_syntax_document_for_tests(
        &mut self,
        region: DiffTextRegion,
        document: rows::PreparedDiffSyntaxDocument,
    ) {
        let view_mode = match region {
            DiffTextRegion::SplitLeft => PreparedSyntaxViewMode::FileDiffSplitLeft,
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                PreparedSyntaxViewMode::FileDiffSplitRight
            }
        };
        let key = self
            .file_diff_prepared_syntax_key(view_mode)
            .expect("test file diff should have a prepared syntax key");
        self.insert_prepared_syntax_document(key, document);
    }

    pub(in crate::view) fn worktree_preview_prepared_syntax_key(
        &self,
    ) -> Option<PreparedSyntaxDocumentKey> {
        let repo_id = self.diff_source_id()?;
        let path = self.worktree_preview_path.as_ref()?;
        Some(prepared_syntax_document_key(
            repo_id,
            self.worktree_preview_content_rev,
            path,
            PreparedSyntaxViewMode::WorktreePreview,
        ))
    }

    pub(in crate::view) fn worktree_preview_prepared_syntax_document(
        &self,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let key = self.worktree_preview_prepared_syntax_key()?;
        self.prepared_syntax_document(&key)
    }

    pub(in crate::view) fn ensure_single_markdown_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(path) = self.worktree_preview_path.clone() else {
            return;
        };
        let source_rev = self.worktree_preview_content_rev;
        if !matches!(self.worktree_preview, Loadable::Ready(_)) {
            return;
        }

        let cache_matches = self.worktree_markdown.path.as_ref() == Some(&path)
            && self.worktree_markdown.source_rev == source_rev;
        if cache_matches {
            match &self.worktree_markdown.document {
                Loadable::Ready(_) | Loadable::Error(_) => return,
                Loadable::Loading if self.worktree_markdown.inflight.is_some() => return,
                _ => {}
            }
        }

        self.worktree_markdown.path = Some(path.clone());
        self.worktree_markdown.source_rev = source_rev;

        let source_len = if self.worktree_preview_text.is_empty() {
            self.worktree_preview_source_len
        } else {
            self.worktree_preview_text.len()
        };
        if source_len > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
            self.worktree_markdown.document = Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source_len).to_string(),
            );
            self.worktree_markdown.inflight = None;
            return;
        }

        self.worktree_markdown.document = Loadable::Loading;
        self.worktree_markdown.seq = self.worktree_markdown.seq.wrapping_add(1);
        let seq = self.worktree_markdown.seq;
        self.worktree_markdown.inflight = Some(seq);
        let source_text =
            (!self.worktree_preview_text.is_empty()).then_some(self.worktree_preview_text.clone());
        let source_path = self.worktree_preview_source_path.clone();
        let image_root = self.markdown_preview_image_root();

        cx.spawn(
            async move |view: WeakEntity<DiffRenderer>, cx: &mut gpui::AsyncApp| {
                type BuiltPreview = (
                    Arc<markdown_preview::MarkdownPreviewDocument>,
                    rows::MarkdownPreviewPictureSizes,
                );
                let build_preview =
                    move || -> Result<BuiltPreview, markdown_preview::MarkdownPreviewRefusal> {
                        let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
                        let source_text = match source_text {
                            Some(source_text) => source_text,
                            None => {
                                let source_path = source_path.ok_or_else(|| {
                                    "Preview source path is unavailable.".to_string()
                                })?;
                                std::fs::read_to_string(&source_path)
                                .map(SharedString::from)
                                .map_err(|e| {
                                    if e.kind() == std::io::ErrorKind::InvalidData {
                                        "File is not valid UTF-8; binary preview is not supported."
                                            .to_string()
                                    } else {
                                        e.to_string()
                                    }
                                })?
                            }
                        };
                        let document =
                            build_single_markdown_preview_document(source_text.as_ref())?;
                        // Measured here rather than on the first frame: it reads
                        // files, and this is already the thread that does that.
                        let picture_sizes = measure_markdown_preview_pictures(
                            document.as_ref(),
                            image_root.as_ref(),
                        );
                        Ok((document, picture_sizes))
                    };
                let result = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(build_preview).await
                } else {
                    build_preview()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.worktree_markdown.inflight != Some(seq) {
                        return;
                    }
                    if this.worktree_preview_path.as_ref() != Some(&path)
                        || this.worktree_preview_content_rev != source_rev
                    {
                        return;
                    }

                    this.worktree_markdown.inflight = None;
                    match result {
                        Ok((document, picture_sizes)) => {
                            this.worktree_markdown.picture_sizes = picture_sizes;
                            // The blocks these positions belonged to are gone
                            // with the document that described them.
                            this.worktree_markdown.block_scrolls.clear();
                            this.worktree_markdown.document = Loadable::Ready(document);
                            // An open search scanned nothing while this was
                            // parsing, so without a rescan it would keep
                            // reporting "no matches" over a document that
                            // plainly holds the term.
                            this.diff_search_recompute_matches();
                        }
                        Err(refusal) => {
                            // The document these described is gone too, so they
                            // are cleared here for the same reason as above.
                            this.worktree_markdown.picture_sizes = Default::default();
                            this.worktree_markdown.block_scrolls.clear();
                            let prefers_source = refusal.prefers_source();
                            this.worktree_markdown.document =
                                Loadable::Error(refusal.into_message());
                            // A document that parsed but is too big to lay out
                            // still reads fine as source, so the reader is
                            // taken there rather than left on an empty pane
                            // with a message and a toggle to find.
                            if prefers_source {
                                this.rendered_preview_modes.fall_back_to_markdown_source();
                            }
                        }
                    }
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(super) fn apply_worktree_preview_ready_state(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        source_len: usize,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        line_flags: Arc<[u8]>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_count = indexed_line_count_from_len(source_len, line_starts.as_ref());
        let source_changed = self.worktree_preview_path.as_ref() != Some(&display_path)
            || self.worktree_preview_source_path.as_ref() != Some(&source_path)
            || self.worktree_preview_line_count() != Some(line_count)
            || self.worktree_preview_source_len != source_len
            || self.worktree_preview_text.as_ref() != source_text.as_ref()
            || self.worktree_preview_line_starts.as_ref() != line_starts.as_ref()
            || self.worktree_preview_line_flags.as_ref() != line_flags.as_ref();
        let cache_binding_changed =
            self.worktree_preview_segments_cache_path.as_ref() != Some(&display_path);
        let same_path_source_refresh = source_changed && !cache_binding_changed;

        self.worktree_preview_path = Some(display_path.clone());
        self.worktree_preview_source_path = Some(source_path);
        self.worktree_preview = Loadable::Ready(line_count);
        self.worktree_preview_source_len = source_len;
        self.worktree_preview_text = source_text;
        self.worktree_preview_line_starts = line_starts;
        self.worktree_preview_line_flags = line_flags;
        self.worktree_preview_search_trigram_index = None;
        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&display_path);
        self.worktree_preview_segments_cache_path = Some(display_path);
        self.worktree_preview_cache_write_blocked_until_rev = None;
        if source_changed || cache_binding_changed {
            self.worktree_preview_segments_cache.clear();
        }

        if source_changed {
            self.worktree_preview_content_rev = self.worktree_preview_content_rev.wrapping_add(1);
            self.worktree_preview_style_cache_epoch =
                self.worktree_preview_style_cache_epoch.wrapping_add(1);
            self.clear_diff_text_projected_highlights();
            self.worktree_markdown.invalidate();
        }

        if same_path_source_refresh {
            let blocked_rev = self.worktree_preview_content_rev;
            self.worktree_preview_cache_write_blocked_until_rev = Some(blocked_rev);
            if !crate::ui_runtime::current().uses_background_compute() {
                if self.worktree_preview_cache_write_blocked_until_rev == Some(blocked_rev) {
                    self.worktree_preview_cache_write_blocked_until_rev = None;
                }
            } else {
                cx.spawn(
                    async move |view: WeakEntity<DiffRenderer>, cx: &mut gpui::AsyncApp| {
                        smol::Timer::after(std::time::Duration::from_millis(1)).await;
                        let _ = view.update(cx, |this, _cx| {
                            if this.worktree_preview_cache_write_blocked_until_rev
                                == Some(blocked_rev)
                            {
                                this.worktree_preview_cache_write_blocked_until_rev = None;
                            }
                        });
                    },
                )
                .detach();
            }
        }

        self.refresh_worktree_preview_syntax_document(cx);
    }

    pub(in crate::view) fn set_worktree_preview_ready_source(
        &mut self,
        path: std::path::PathBuf,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_flags = preview_line_flags_from_source(source_text.as_ref(), line_starts.as_ref());
        self.apply_worktree_preview_ready_state(
            path.clone(),
            path,
            source_text.len(),
            source_text,
            line_starts,
            line_flags,
            cx,
        );
    }

    pub(in crate::view) fn set_worktree_preview_ready_materialized_source(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        source_text: SharedString,
        line_starts: Arc<[usize]>,
        line_flags: Arc<[u8]>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.apply_worktree_preview_ready_state(
            display_path,
            source_path,
            source_text.len(),
            source_text,
            line_starts,
            line_flags,
            cx,
        );
    }

    pub(in crate::view) fn set_worktree_preview_ready_indexed_source(
        &mut self,
        display_path: std::path::PathBuf,
        source_path: std::path::PathBuf,
        source_len: usize,
        line_starts: Arc<[usize]>,
        line_flags: Arc<[u8]>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.apply_worktree_preview_ready_state(
            display_path,
            source_path,
            source_len,
            SharedString::default(),
            line_starts,
            line_flags,
            cx,
        );
    }

    pub(in crate::view) fn set_worktree_preview_ready_rows(
        &mut self,
        path: std::path::PathBuf,
        lines: &[String],
        source_len: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let (source_text, line_starts) =
            preview_source_text_and_line_starts_from_lines(lines, source_len);
        self.set_worktree_preview_ready_source(path, source_text, line_starts, cx);
    }

    pub(in crate::view) fn refresh_worktree_preview_syntax_document(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(language) = self.worktree_preview_syntax_language else {
            return;
        };
        let Some(key) = self.worktree_preview_prepared_syntax_key() else {
            return;
        };
        if !matches!(self.worktree_preview, Loadable::Ready(_)) {
            return;
        }
        if self.worktree_preview_text.is_empty() {
            return;
        }
        let source_text = self.worktree_preview_text.clone();
        let line_starts = Arc::clone(&self.worktree_preview_line_starts);

        if self.prepared_syntax_document(&key).is_some() {
            return;
        }
        let reparse_seed = self.prepared_syntax_reparse_seed_document(&key);
        let background_reparse_seed: Option<rows::PreparedDiffSyntaxReparseSeed> =
            reparse_seed.and_then(rows::prepared_diff_syntax_reparse_seed);

        let budget = self.full_document_syntax_budget();
        match rows::prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            FULL_DOCUMENT_SYNTAX_MODE,
            source_text.clone(),
            Arc::clone(&line_starts),
            budget,
            reparse_seed,
            None,
        ) {
            rows::PrepareDiffSyntaxDocumentResult::Ready(document) => {
                if self.insert_worktree_preview_syntax_document(key, document) {
                    // A click made before this landed is waiting on exactly this
                    // document -- the same replay the file-diff paths run.
                    self.retry_pending_diff_text_syntax_click();
                }
            }
            rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                cx.spawn(
                    async move |view: WeakEntity<DiffRenderer>, cx: &mut gpui::AsyncApp| {
                        let prepare_document = move || {
                            rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                                language,
                                FULL_DOCUMENT_SYNTAX_MODE,
                                source_text,
                                line_starts,
                                background_reparse_seed,
                                None,
                            )
                        };
                        let parsed_document =
                            if crate::ui_runtime::current().uses_background_compute() {
                                smol::unblock(prepare_document).await
                            } else {
                                prepare_document()
                            };

                        let _ = view.update(cx, |this, cx| {
                            let Some(parsed_document) = parsed_document else {
                                return;
                            };
                            // Cached by content whatever happens next, so coming
                            // back to the same text reuses it instead of reparsing.
                            let document = rows::inject_background_prepared_diff_syntax_document(
                                parsed_document,
                            );
                            // Only the preview still showing the parsed text may
                            // take it; after another file or mid-reload it is stale.
                            if !matches!(this.worktree_preview, Loadable::Ready(_))
                                || this.worktree_preview_prepared_syntax_key().as_ref()
                                    != Some(&key)
                            {
                                return;
                            }

                            if this.insert_worktree_preview_syntax_document(key, document) {
                                this.worktree_preview_style_cache_epoch =
                                    this.worktree_preview_style_cache_epoch.wrapping_add(1);
                                this.retry_pending_diff_text_syntax_click();
                                cx.notify();
                            }
                        });
                    },
                )
                .detach();
            }
            rows::PrepareDiffSyntaxDocumentResult::Unsupported => {}
        }
    }

    /// Applies a foreground sync prepare result for one side. Returns `true` if
    /// the side needs a background async parse instead.
    pub(super) fn apply_sync_syntax_result(
        &mut self,
        side: DiffTextRegion,
        key: &Option<PreparedSyntaxDocumentKey>,
        prepare: impl FnOnce() -> Option<rows::PrepareDiffSyntaxDocumentResult>,
    ) -> SyncFileDiffPreparedSyntaxApplyResult {
        let Some(key) = key else {
            return SyncFileDiffPreparedSyntaxApplyResult::default();
        };
        let mut needs_background_prepare = false;
        let inserted =
            self.install_file_diff_side_document(side, key.clone(), || match prepare() {
                Some(rows::PrepareDiffSyntaxDocumentResult::Ready(document)) => Some(document),
                Some(rows::PrepareDiffSyntaxDocumentResult::TimedOut) => {
                    needs_background_prepare = true;
                    None
                }
                _ => None,
            });
        SyncFileDiffPreparedSyntaxApplyResult {
            inserted,
            needs_background_prepare,
        }
    }

    /// Applies background-parsed documents for both sides and reports which
    /// side became newly cacheable.
    pub(super) fn apply_background_syntax_documents(
        &mut self,
        left_key: &Option<PreparedSyntaxDocumentKey>,
        left_doc: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
        right_key: &Option<PreparedSyntaxDocumentKey>,
        right_doc: Option<rows::BackgroundPreparedDiffSyntaxDocument>,
    ) -> FileDiffPreparedSyntaxApplyResult {
        let mut applied = FileDiffPreparedSyntaxApplyResult::default();
        if let (Some(key), Some(document)) = (left_key.as_ref(), left_doc) {
            applied.split_left = self.install_file_diff_side_document(
                DiffTextRegion::SplitLeft,
                key.clone(),
                || {
                    Some(rows::inject_background_prepared_diff_syntax_document(
                        document,
                    ))
                },
            );
        }
        if let (Some(key), Some(document)) = (right_key.as_ref(), right_doc) {
            applied.split_right = self.install_file_diff_side_document(
                DiffTextRegion::SplitRight,
                key.clone(),
                || {
                    Some(rows::inject_background_prepared_diff_syntax_document(
                        document,
                    ))
                },
            );
        }
        applied
    }

    pub(super) fn refresh_file_diff_syntax_documents(
        &mut self,
        cx: &mut gpui::Context<Self>,
        split_left_reparse_seed_override: Option<rows::PreparedDiffSyntaxDocument>,
        split_right_reparse_seed_override: Option<rows::PreparedDiffSyntaxDocument>,
        split_left_edit_hint: Option<rows::DiffSyntaxEdit>,
        split_right_edit_hint: Option<rows::DiffSyntaxEdit>,
    ) {
        let Some(language) = self.file_diff_cache_language else {
            return;
        };

        // A side whose content is a file on disk has no resident text by
        // construction (`file_diff_source_text`), which is what lets a huge diff
        // render from per-line slices. Nothing below can parse it, so it goes to
        // the worker instead -- and for an ordinary worktree diff that is *both*
        // sides, which is why skipping it left every cross-line construct (a
        // `<script>` body, front matter, a block comment) uncoloured.
        self.refresh_source_backed_file_diff_syntax_documents(cx);
        if self.file_diff_old_text.is_empty() && self.file_diff_new_text.is_empty() {
            return;
        }

        // Split and inline syntax both project from the real old/new documents.
        // Only those real side documents are parsed here; inline rows later map
        // through old_line/new_line instead of parsing any synthetic diff stream.
        let split_left_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitLeft);
        let split_right_key =
            self.file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight);
        let split_left_reparse_seed = split_left_reparse_seed_override.or_else(|| {
            split_left_key
                .as_ref()
                .and_then(|key| self.prepared_syntax_reparse_seed_document(key))
        });
        let split_right_reparse_seed = split_right_reparse_seed_override.or_else(|| {
            split_right_key
                .as_ref()
                .and_then(|key| self.prepared_syntax_reparse_seed_document(key))
        });

        // A hosted producer may mix resident and source-backed sides. The
        // empty resident placeholder of a source side is not its document.
        let needs_split_left_prepare = (self.file_diff_old_source_path.is_none()
            || !self.file_diff_old_text.is_empty())
            && split_left_key
                .as_ref()
                .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        let needs_split_right_prepare = (self.file_diff_new_source_path.is_none()
            || !self.file_diff_new_text.is_empty())
            && split_right_key
                .as_ref()
                .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        if !needs_split_left_prepare && !needs_split_right_prepare {
            return;
        }

        let budget = self.full_document_syntax_budget();

        let old_text = self.file_diff_old_text.clone();
        let old_line_starts = Arc::clone(&self.file_diff_old_line_starts);
        let split_left_sync =
            self.apply_sync_syntax_result(DiffTextRegion::SplitLeft, &split_left_key, || {
                needs_split_left_prepare.then(|| {
                    rows::prepare_diff_syntax_document_with_budget_reuse_text(
                        language,
                        FULL_DOCUMENT_SYNTAX_MODE,
                        old_text,
                        old_line_starts,
                        budget,
                        split_left_reparse_seed,
                        split_left_edit_hint.clone(),
                    )
                })
            });
        let new_text = self.file_diff_new_text.clone();
        let new_line_starts = Arc::clone(&self.file_diff_new_line_starts);
        let split_right_sync =
            self.apply_sync_syntax_result(DiffTextRegion::SplitRight, &split_right_key, || {
                needs_split_right_prepare.then(|| {
                    rows::prepare_diff_syntax_document_with_budget_reuse_text(
                        language,
                        FULL_DOCUMENT_SYNTAX_MODE,
                        new_text,
                        new_line_starts,
                        budget,
                        split_right_reparse_seed,
                        split_right_edit_hint.clone(),
                    )
                })
            });
        let needs_split_left_async = split_left_sync.needs_background_prepare;
        let needs_split_right_async = split_right_sync.needs_background_prepare;

        if split_left_sync.inserted || split_right_sync.inserted {
            cx.notify();
        }

        if !needs_split_left_async && !needs_split_right_async {
            return;
        }

        let syntax_generation = self.file_diff_syntax_generation;
        let repo_id = self.file_diff_cache_repo_id;
        let diff_file_rev = self.file_diff_cache_rev;
        let diff_target = self.file_diff_cache_target.clone();

        let split_left_source = needs_split_left_async.then(|| {
            (
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
            )
        });
        let split_left_background_reparse_seed = split_left_reparse_seed
            .filter(|_| needs_split_left_async)
            .and_then(rows::prepared_diff_syntax_reparse_seed);
        let split_left_edit_hint = split_left_edit_hint.filter(|_| needs_split_left_async);
        let split_right_source = needs_split_right_async.then(|| {
            (
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
            )
        });
        let split_right_background_reparse_seed = split_right_reparse_seed
            .filter(|_| needs_split_right_async)
            .and_then(rows::prepared_diff_syntax_reparse_seed);
        let split_right_edit_hint = split_right_edit_hint.filter(|_| needs_split_right_async);

        self.file_diff_resident_syntax_inflight += 1;
        crate::ui_runtime::run_background_compute(
            cx,
            move || FileDiffBackgroundPreparedSyntaxDocuments {
                split_left: split_left_source.and_then(|(text, line_starts)| {
                    rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                        language,
                        FULL_DOCUMENT_SYNTAX_MODE,
                        text,
                        line_starts,
                        split_left_background_reparse_seed,
                        split_left_edit_hint,
                    )
                }),
                split_right: split_right_source.and_then(|(text, line_starts)| {
                    rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                        language,
                        FULL_DOCUMENT_SYNTAX_MODE,
                        text,
                        line_starts,
                        split_right_background_reparse_seed,
                        split_right_edit_hint,
                    )
                }),
            },
            move |this, cx, parsed_documents| {
                if this.file_diff_syntax_generation != syntax_generation {
                    return;
                }
                this.file_diff_resident_syntax_inflight -= 1;
                if this.file_diff_resident_syntax_inflight == 0 {
                    // A rejected revision or unsupported parse must also let
                    // visible-side recovery reconsider a missing document.
                    cx.notify();
                }
                if this.file_diff_cache_repo_id != repo_id
                    || this.file_diff_cache_rev != diff_file_rev
                    || this.file_diff_cache_target != diff_target
                {
                    return;
                }

                let applied = this.apply_background_syntax_documents(
                    &split_left_key,
                    parsed_documents.split_left,
                    &split_right_key,
                    parsed_documents.split_right,
                );

                if applied.any() {
                    this.retry_pending_diff_text_syntax_click();
                    cx.notify();
                }
            },
        )
        .detach();
    }

    /// Resets file-diff data fields (syntax, rows, text, highlights) without
    /// touching the identity fields (repo_id, target, rev).
    pub(in crate::view) fn reset_file_diff_cache_data(&mut self) {
        // Identity fields may already name the next selection. No file-diff
        // document remains reachable once its rows are dropped; preview history
        // has an independent lifetime and must survive this reset.
        self.prepared_syntax_documents
            .retain(|key, _| key.view_mode == PreparedSyntaxViewMode::WorktreePreview);
        self.file_diff_cancellation.cancel();
        self.diff_engine_status = DiffEngineStatus::LineByLine;
        self.reset_collapsed_diff_projection(false);
        self.file_diff_cache_content_signature = None;
        self.file_diff_cache_inflight = None;
        self.file_diff_cache_error = None;
        self.advance_file_diff_syntax_generation();
        self.file_diff_style_cache_epochs.bump_both();
        self.file_diff_cache_path = None;
        self.file_diff_cache_language = None;
        self.file_diff_cache_rows = Arc::from([]);
        self.file_diff_row_provider = None;
        self.file_diff_old_source_path = None;
        self.file_diff_new_source_path = None;
        self.file_diff_old_source_identity = None;
        self.file_diff_new_source_identity = None;
        self.file_diff_old_text = SharedString::default();
        self.file_diff_old_line_starts = Arc::default();
        self.file_diff_old_line_to_row = Arc::default();
        self.file_diff_old_line_to_inline_row = Arc::default();
        self.file_diff_new_text = SharedString::default();
        self.file_diff_new_line_starts = Arc::default();
        self.file_diff_new_line_to_row = Arc::default();
        self.file_diff_new_line_to_inline_row = Arc::default();
        self.file_diff_inline_cache = Arc::from([]);
        self.file_diff_inline_row_provider = None;
        self.file_diff_inline_text = SharedString::default();
        self.reset_file_diff_word_highlight_caches();
    }
}
