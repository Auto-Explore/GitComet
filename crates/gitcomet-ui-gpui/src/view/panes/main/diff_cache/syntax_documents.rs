use super::*;

impl MainPaneView {
    pub(super) fn prepared_syntax_document(
        &self,
        key: &PreparedSyntaxDocumentKey,
    ) -> Option<rows::PreparedDiffSyntaxDocument> {
        let document = self.prepared_syntax_documents.get(key).copied()?;
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
            .map(|(_, document)| *document)
    }

    pub(super) fn insert_prepared_syntax_document(
        &mut self,
        key: PreparedSyntaxDocumentKey,
        document: rows::PreparedDiffSyntaxDocument,
    ) -> bool {
        if self.prepared_syntax_documents.contains_key(&key) {
            return false;
        }
        if self.prepared_syntax_documents.len() >= PREPARED_SYNTAX_DOCUMENT_CACHE_MAX_ENTRIES
            && let Some(evict_key) = self.prepared_syntax_documents.keys().next().cloned()
        {
            self.prepared_syntax_documents.remove(&evict_key);
        }
        self.prepared_syntax_documents.insert(key, document);
        true
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
        repo_id: RepoId,
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

    pub(in crate::view) fn full_document_syntax_budget(&self) -> rows::DiffSyntaxBudget {
        #[cfg(test)]
        if let Some(budget) = self.diff_syntax_budget_override {
            return budget;
        }

        rows::DiffSyntaxBudget::default()
    }

    #[cfg(test)]
    pub(in crate::view) fn set_full_document_syntax_budget_override_for_tests(
        &mut self,
        budget: rows::DiffSyntaxBudget,
    ) {
        self.diff_syntax_budget_override = Some(budget);
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
        if let Some(document) = self.file_diff_split_prepared_syntax_document(region) {
            return Some(document);
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
        match rows::prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            FULL_DOCUMENT_SYNTAX_MODE,
            text.clone(),
            Arc::clone(&line_starts),
            budget,
            None,
            None,
        ) {
            rows::PrepareDiffSyntaxDocumentResult::Ready(document) => Some(document),
            // Timing out means the parse is too slow to sit in front of a mouse
            // press, so it goes to the worker -- `None` here makes the caller
            // record the click as pending and
            // [`Self::request_file_diff_click_syntax_document`] replays it when
            // the document lands. Finishing it inline instead was the same parse
            // with the budget removed: a 900 KiB C++ file spent its 50 ms,
            // discarded that, and then blocked the UI thread for a further
            // 210 ms before the mouse-down returned.
            //
            // The retained body above is what keeps this cheap -- the worker
            // reuses that allocation instead of reading the file again.
            rows::PrepareDiffSyntaxDocumentResult::TimedOut
            | rows::PrepareDiffSyntaxDocumentResult::Unsupported => None,
        }
    }

    /// Completes a cold click's full-document syntax work away from the UI
    /// thread. This is what lets interaction share the 8 MiB syntax ceiling:
    /// the click remains responsive, and its exact row/offset is replayed once
    /// the prepared tree is installed.
    pub(in crate::view) fn request_file_diff_click_syntax_document(
        &mut self,
        region: DiffTextRegion,
        cx: &mut gpui::Context<Self>,
    ) {
        let side = Self::split_side_for_region(region);
        // As in the synchronous path, a cache hit is not proof that a
        // source-backed file still matches the indexed generation: a file that
        // has moved on since this generation indexed it describes different
        // columns than the rows do, so there is nothing worth parsing until the
        // rebuild lands.
        if !self.file_diff_source_is_current(side) {
            self.file_diff_pair_syntax_text.remove(&side);
            self.clear_pending_diff_text_syntax_click_for(side);
            return;
        }
        if self
            .file_diff_split_prepared_syntax_document(side)
            .is_some()
        {
            self.retry_pending_diff_text_syntax_click();
            cx.notify();
            return;
        }
        let syntax_generation = self.file_diff_syntax_generation;
        if self.file_diff_click_syntax_inflight.contains_key(&side) {
            return;
        }
        self.file_diff_click_syntax_inflight
            .insert(side, syntax_generation);

        let Some(language) = self.file_diff_cache_language else {
            self.file_diff_click_syntax_inflight.remove(&side);
            self.clear_pending_diff_text_syntax_click_for(side);
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
        if text
            .as_ref()
            .is_some_and(|text| text.len() > rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES)
        {
            self.file_diff_click_syntax_inflight.remove(&side);
            self.clear_pending_diff_text_syntax_click_for(side);
            return;
        }
        if text.is_none()
            && !source_path.as_ref().is_some_and(|path| {
                std::fs::metadata(path.as_ref())
                    .ok()
                    .is_some_and(|metadata| {
                        metadata.len() <= rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES as u64
                    })
            })
        {
            self.file_diff_click_syntax_inflight.remove(&side);
            self.clear_pending_diff_text_syntax_click_for(side);
            return;
        }

        let Some(key) = self.file_diff_prepared_syntax_key(view_mode) else {
            self.file_diff_click_syntax_inflight.remove(&side);
            self.clear_pending_diff_text_syntax_click_for(side);
            return;
        };
        let repo_id = self.file_diff_cache_repo_id;
        let diff_file_rev = self.file_diff_cache_rev;
        let diff_target = self.file_diff_cache_target.clone();
        let source_backed = text.is_none();
        let indexed_identity = match side {
            DiffTextRegion::SplitLeft => self.file_diff_old_source_identity.clone(),
            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                self.file_diff_new_source_identity.clone()
            }
        };
        #[cfg(test)]
        let after_prepare_hook = self.file_diff_click_syntax_after_prepare_hook.clone();
        #[cfg(test)]
        let before_complete_hook = self.file_diff_click_syntax_before_complete_hook.clone();

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let prepare_document = move || {
                    let text = match text {
                        Some(text) => text,
                        None => {
                            let path = source_path?;
                            let read =
                                SharedString::from(std::fs::read_to_string(path.as_ref()).ok()?);
                            // Re-checked after the read, not only before the
                            // spawn: the file can change in between, and this
                            // side of the check is off the UI thread anyway.
                            if let Some(indexed) = indexed_identity.as_ref()
                                && file_diff_source_identity(Some(&path)).as_deref()
                                    != Some(indexed.as_ref())
                            {
                                return None;
                            }
                            read
                        }
                    };
                    if text.is_empty()
                        || text.len() > rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES
                        || !line_starts_describe(text.as_ref(), line_starts.as_ref())
                    {
                        return None;
                    }
                    let document =
                        rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                            language,
                            FULL_DOCUMENT_SYNTAX_MODE,
                            text.clone(),
                            line_starts,
                            None,
                            None,
                        )?;
                    Some((text, document))
                };
                let prepared = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(prepare_document).await
                } else {
                    prepare_document()
                };
                #[cfg(test)]
                if prepared.is_some()
                    && let Some(hook) = after_prepare_hook
                {
                    hook();
                }

                let _ = view.update(cx, |this, cx| {
                    #[cfg(test)]
                    if let Some(hook) = before_complete_hook {
                        hook(this);
                    }
                    // Before the guard below, so a superseded worker releases
                    // its own marker. A rebuild can clear and reacquire the side
                    // for a newer generation while this task is still parsing;
                    // in that case the newer worker remains the owner.
                    if this.file_diff_click_syntax_inflight.get(&side) == Some(&syntax_generation) {
                        this.file_diff_click_syntax_inflight.remove(&side);
                    }
                    if this.file_diff_syntax_generation != syntax_generation
                        || this.file_diff_cache_repo_id != repo_id
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != diff_target
                    {
                        return;
                    }
                    // The worker's pre-spawn and post-read checks do not cover
                    // an edit during parsing. Re-stat at completion before the
                    // parsed tree or its retained source can enter the cache.
                    if !this.file_diff_source_is_current(side) {
                        this.file_diff_pair_syntax_text.remove(&side);
                        this.clear_pending_diff_text_syntax_click_for(side);
                        return;
                    }
                    let Some((text, document)) = prepared else {
                        this.clear_pending_diff_text_syntax_click_for(side);
                        return;
                    };
                    if source_backed {
                        // Prepared-document identity includes the source
                        // allocation's address, so keep it alive for cache hits
                        // and for subsequent occurrence scans.
                        this.file_diff_pair_syntax_text.insert(side, text);
                    }
                    let inserted = this.insert_prepared_syntax_document(
                        key,
                        rows::inject_background_prepared_diff_syntax_document(document),
                    );
                    if inserted {
                        match side {
                            DiffTextRegion::SplitLeft => {
                                this.file_diff_style_cache_epochs.bump_left()
                            }
                            DiffTextRegion::SplitRight | DiffTextRegion::Inline => {
                                this.file_diff_style_cache_epochs.bump_right()
                            }
                        }
                    }
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
        let repo_id = self.active_repo_id()?;
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

        let cache_matches = self.worktree_markdown_preview_path.as_ref() == Some(&path)
            && self.worktree_markdown_preview_source_rev == source_rev;
        if cache_matches {
            match &self.worktree_markdown_preview {
                Loadable::Ready(_) | Loadable::Error(_) => return,
                Loadable::Loading if self.worktree_markdown_preview_inflight.is_some() => return,
                _ => {}
            }
        }

        self.worktree_markdown_preview_path = Some(path.clone());
        self.worktree_markdown_preview_source_rev = source_rev;

        let source_len = if self.worktree_preview_text.is_empty() {
            self.worktree_preview_source_len
        } else {
            self.worktree_preview_text.len()
        };
        if source_len > markdown_preview::MAX_PREVIEW_SOURCE_BYTES {
            self.worktree_markdown_preview = Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source_len).to_string(),
            );
            self.worktree_markdown_preview_inflight = None;
            return;
        }

        self.worktree_markdown_preview = Loadable::Loading;
        self.worktree_markdown_preview_seq = self.worktree_markdown_preview_seq.wrapping_add(1);
        let seq = self.worktree_markdown_preview_seq;
        self.worktree_markdown_preview_inflight = Some(seq);
        let source_text =
            (!self.worktree_preview_text.is_empty()).then_some(self.worktree_preview_text.clone());
        let source_path = self.worktree_preview_source_path.clone();
        let image_base_dir = self.markdown_preview_image_base_dir();

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
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
                            image_base_dir.as_deref(),
                        );
                        Ok((document, picture_sizes))
                    };
                let result = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(build_preview).await
                } else {
                    build_preview()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.worktree_markdown_preview_inflight != Some(seq) {
                        return;
                    }
                    if this.worktree_preview_path.as_ref() != Some(&path)
                        || this.worktree_preview_content_rev != source_rev
                    {
                        return;
                    }

                    this.worktree_markdown_preview_inflight = None;
                    match result {
                        Ok((document, picture_sizes)) => {
                            this.worktree_markdown_preview_picture_sizes = picture_sizes;
                            // The blocks these positions belonged to are gone
                            // with the document that described them.
                            this.worktree_markdown_preview_block_scrolls.clear();
                            this.worktree_markdown_preview = Loadable::Ready(document);
                            // An open search scanned nothing while this was
                            // parsing, so without a rescan it would keep
                            // reporting "no matches" over a document that
                            // plainly holds the term.
                            this.diff_search_recompute_matches();
                        }
                        Err(refusal) => {
                            // The document these described is gone too, so they
                            // are cleared here for the same reason as above.
                            this.worktree_markdown_preview_picture_sizes = Default::default();
                            this.worktree_markdown_preview_block_scrolls.clear();
                            let prefers_source = refusal.prefers_source();
                            this.worktree_markdown_preview =
                                Loadable::Error(refusal.into_message());
                            // A document that parsed but is too big to lay out
                            // still reads fine as source, so the reader is
                            // taken there rather than left on an empty pane
                            // with a message and a toggle to find.
                            if prefers_source {
                                this.rendered_preview_modes.set(
                                    RenderedPreviewKind::Markdown,
                                    RenderedPreviewMode::Source,
                                );
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
            self.worktree_markdown_preview_path = None;
            self.worktree_markdown_preview_source_rev = 0;
            self.worktree_markdown_preview = Loadable::NotLoaded;
            self.worktree_markdown_preview_inflight = None;
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
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
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
                if self.insert_prepared_syntax_document(key, document) {
                    // A click made before this landed is waiting on exactly this
                    // document -- the same replay the file-diff paths run.
                    self.retry_pending_diff_text_syntax_click();
                }
            }
            rows::PrepareDiffSyntaxDocumentResult::TimedOut => {
                cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
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

                            let inserted = this.insert_prepared_syntax_document(
                                key.clone(),
                                rows::inject_background_prepared_diff_syntax_document(
                                    parsed_document,
                                ),
                            );
                            if inserted
                                && this.worktree_preview_prepared_syntax_key().as_ref()
                                    == Some(&key)
                            {
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
        attempt: Option<rows::PrepareDiffSyntaxDocumentResult>,
        key: &Option<PreparedSyntaxDocumentKey>,
    ) -> SyncFileDiffPreparedSyntaxApplyResult {
        match attempt {
            Some(rows::PrepareDiffSyntaxDocumentResult::Ready(document)) => {
                SyncFileDiffPreparedSyntaxApplyResult {
                    inserted: key.as_ref().is_some_and(|key| {
                        self.insert_prepared_syntax_document(key.clone(), document)
                    }),
                    needs_background_prepare: false,
                }
            }
            Some(rows::PrepareDiffSyntaxDocumentResult::TimedOut) => {
                SyncFileDiffPreparedSyntaxApplyResult {
                    inserted: false,
                    needs_background_prepare: true,
                }
            }
            _ => SyncFileDiffPreparedSyntaxApplyResult::default(),
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
            applied.split_left = self.insert_prepared_syntax_document(
                key.clone(),
                rows::inject_background_prepared_diff_syntax_document(document),
            );
        }
        if let (Some(key), Some(document)) = (right_key.as_ref(), right_doc) {
            applied.split_right = self.insert_prepared_syntax_document(
                key.clone(),
                rows::inject_background_prepared_diff_syntax_document(document),
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
        if self.file_diff_old_text.is_empty() && self.file_diff_new_text.is_empty() {
            return;
        }

        let Some(language) = self.file_diff_cache_language else {
            return;
        };

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

        let needs_split_left_prepare = split_left_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        let needs_split_right_prepare = split_right_key
            .as_ref()
            .is_some_and(|key| self.prepared_syntax_document(key).is_none());
        if !needs_split_left_prepare && !needs_split_right_prepare {
            return;
        }

        let budget = self.full_document_syntax_budget();

        let split_left_attempt = needs_split_left_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_old_text.clone(),
                Arc::clone(&self.file_diff_old_line_starts),
                budget,
                split_left_reparse_seed,
                split_left_edit_hint.clone(),
            )
        });
        let split_right_attempt = needs_split_right_prepare.then(|| {
            rows::prepare_diff_syntax_document_with_budget_reuse_text(
                language,
                FULL_DOCUMENT_SYNTAX_MODE,
                self.file_diff_new_text.clone(),
                Arc::clone(&self.file_diff_new_line_starts),
                budget,
                split_right_reparse_seed,
                split_right_edit_hint.clone(),
            )
        });

        let split_left_sync = self.apply_sync_syntax_result(split_left_attempt, &split_left_key);
        let split_right_sync = self.apply_sync_syntax_result(split_right_attempt, &split_right_key);
        let needs_split_left_async = split_left_sync.needs_background_prepare;
        let needs_split_right_async = split_right_sync.needs_background_prepare;

        if split_left_sync.inserted {
            self.file_diff_style_cache_epochs.bump_left();
        }
        if split_right_sync.inserted {
            self.file_diff_style_cache_epochs.bump_right();
        }
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

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let prepare_documents = move || FileDiffBackgroundPreparedSyntaxDocuments {
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
                };
                let parsed_documents = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(prepare_documents).await
                } else {
                    prepare_documents()
                };

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_syntax_generation != syntax_generation {
                        return;
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
                        if applied.split_left {
                            this.file_diff_style_cache_epochs.bump_left();
                        }
                        if applied.split_right {
                            this.file_diff_style_cache_epochs.bump_right();
                        }
                        this.retry_pending_diff_text_syntax_click();
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    /// Resets file-diff data fields (syntax, rows, text, highlights) without
    /// touching the identity fields (repo_id, target, rev).
    pub(in crate::view) fn reset_file_diff_cache_data(&mut self) {
        self.reset_collapsed_diff_projection(false);
        self.file_diff_cache_content_signature = None;
        self.file_diff_cache_inflight = None;
        self.file_diff_cache_error = None;
        self.advance_file_diff_syntax_generation();
        self.file_diff_style_cache_epochs.bump_both();
        self.file_diff_cache_path = None;
        self.file_diff_cache_language = None;
        self.file_diff_cache_rows.clear();
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
        self.file_diff_inline_cache.clear();
        self.file_diff_inline_row_provider = None;
        self.file_diff_inline_text = SharedString::default();
        self.reset_file_diff_word_highlight_caches();
    }
}
