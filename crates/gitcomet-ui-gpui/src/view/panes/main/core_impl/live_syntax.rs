use super::*;

impl MainPaneView {
    /// Unresolved rows for `snapshot`, from the cache when it is still current.
    ///
    /// A miss only happens when navigation runs before any refresh has scanned
    /// this revision; the scan is then done once and cached like any other.
    pub(super) fn conflict_resolved_output_unresolved_rows_for(
        &mut self,
        snapshot: &TextModelSnapshot,
    ) -> UnresolvedRows {
        let key = ResolvedOutputKey::new(
            snapshot,
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolved_output_block_map,
        );
        if let Some((cached_for, rows)) = self.conflict_resolved_output_unresolved_rows.as_ref()
            && *cached_for == key
        {
            return Arc::clone(rows);
        }

        #[cfg(test)]
        {
            self.conflict_resolved_output_full_scans += 1;
        }
        let rows = resolved_output_unresolved_rows(
            &self.conflict_resolver.marker_segments,
            &snapshot.rope(),
            &self.conflict_resolved_output_block_map,
        );
        self.conflict_resolved_output_unresolved_rows = Some((key, Arc::clone(&rows)));
        rows
    }

    /// Rebuild the output highlights when conflict navigation lands on another
    /// conflict, so the yellow wash follows the selection.
    ///
    /// Every other refresh path hangs off the text or the tree, and navigation
    /// moves neither — it only reassigns `active_conflict`, from a dozen call
    /// sites on a state struct that cannot reach the input. Comparing against the
    /// conflict the installed provider was built for catches all of them in one
    /// place, and makes the common render (nothing moved) a single comparison.
    pub(super) fn sync_conflict_resolved_output_active_conflict_highlight(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_highlighted_conflict
            == self.conflict_resolver.active_conflict
        {
            return;
        }
        self.conflict_resolved_output_highlighted_conflict = self.conflict_resolver.active_conflict;
        // Streamed output is drawn row by row from the projection, which reads
        // the active conflict as it renders; only the editable buffer carries
        // highlights that have to be reinstalled.
        if self.conflict_resolved_output_is_streamed() {
            return;
        }
        // With a tree in hand, rebinding the provider is the whole job:
        // navigation moves no text, so the tree, the placeholder mask and the
        // protected spans all still stand. Going through the full syntax refresh
        // would redo them on every jump — and on the tree-less arm it would
        // re-tokenize the entire document, which is exactly the kind of
        // per-keypress cost the live engine exists to avoid.
        if self.conflict_resolved_output_live_syntax.is_some() {
            self.rebind_conflict_resolved_output_highlight_provider(cx);
            return;
        }
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        self.refresh_conflict_resolved_output_syntax(&output_snapshot, None, cx);
    }

    pub(in crate::view) fn current_conflict_resolved_output_text(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> String {
        if self.conflict_resolved_output_is_streamed() {
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments)
        } else {
            self.conflict_resolver_input
                .read_with(cx, |input, _| input.text().to_string())
        }
    }

    pub(in crate::view) fn conflict_resolver_save_contents_from_text(
        &mut self,
        text: String,
    ) -> String {
        self.conflict_resolver_sync_session_resolutions_from_output(&text);
        text
    }

    pub(in crate::view) fn ensure_prepared_syntax_chunk_poll(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.syntax_chunk_poll_task.is_some() {
            return;
        }

        if !crate::ui_runtime::current().uses_background_compute() {
            while self.apply_prepared_syntax_chunk_updates(cx) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            self.syntax_chunk_poll_task = None;
            return;
        }

        let task = cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| loop {
                let should_continue = view
                    .update(cx, |this, cx| this.apply_prepared_syntax_chunk_updates(cx))
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }

                smol::Timer::after(std::time::Duration::from_millis(16)).await;
            },
        );
        self.syntax_chunk_poll_task = Some(task);
    }

    pub(super) fn apply_prepared_syntax_chunk_updates(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let mut applied = false;

        let split_left_applied = self
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if split_left_applied > 0 {
            self.file_diff_style_cache_epochs.bump_left();
            applied = true;
        }

        let split_right_applied = self
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if split_right_applied > 0 {
            self.file_diff_style_cache_epochs.bump_right();
            applied = true;
        }

        let worktree_preview_applied = self
            .worktree_preview_prepared_syntax_document()
            .map(rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document)
            .unwrap_or(0);
        if worktree_preview_applied > 0 {
            self.worktree_preview_style_cache_epoch =
                self.worktree_preview_style_cache_epoch.wrapping_add(1);
            applied = true;
        }

        if rows::drain_completed_prepared_diff_syntax_chunk_builds() > 0 {
            applied = true;
        }

        if applied {
            cx.notify();
        }

        let pending = rows::has_pending_prepared_diff_syntax_chunk_builds();
        if !pending {
            self.syntax_chunk_poll_task = None;
        }
        pending
    }

    /// Build the first tree off-thread after the foreground budget ran out.
    ///
    /// Guarded on the revision it is building for, so a burst of refreshes over
    /// the same text schedules one parse rather than one per call. A result for
    /// text the buffer has since moved past is never installed — it is re-issued
    /// against the current text instead, so the pane cannot be left on the
    /// heuristic fallback by an edit that raced the parse.
    pub(super) fn ensure_conflict_resolved_output_live_syntax_build(
        &mut self,
        language: rows::DiffSyntaxLanguage,
        rope: crate::kit::rope::Rope,
        mask: Arc<[Range<usize>]>,
        revision: ResolvedOutputSourceRevision,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_live_syntax_building == Some(revision) {
            return;
        }
        self.conflict_resolved_output_live_syntax_building = Some(revision);

        let build_mask = Arc::clone(&mask);
        self.conflict_resolved_output_live_syntax_build =
            Some(cx.spawn(async move |view: WeakEntity<MainPaneView>, cx| {
                let build = move || rows::LiveSyntaxDocument::new(language, rope, build_mask, None);
                let built = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(build).await
                } else {
                    build()
                };
                let _ = view.update(cx, |this, cx| {
                    if this.conflict_resolved_output_live_syntax_building != Some(revision) {
                        // A newer generation owns the guard. Leave it alone --
                        // clearing it here would let its own scheduling check
                        // pass again and start a duplicate build.
                        return;
                    }
                    this.conflict_resolved_output_live_syntax_building = None;
                    let Some(document) = built else {
                        // Unbudgeted, so this is not a timeout: the text is past
                        // the size ceiling or the language has no wired grammar.
                        // Both are permanent, so re-issuing would spin. The
                        // heuristic arm of the refresh is the right answer here,
                        // and it is the same one the diff panes take.
                        return;
                    };
                    // Zed's `parse_again` (`Buffer::reparse`): a result for text
                    // the buffer has moved past is useless, but so is waiting --
                    // nothing else is guaranteed to come along and ask again, so
                    // re-issue from where the buffer is now.
                    let still_current = this.conflict_resolver_input.read_with(cx, |input, _| {
                        ResolvedOutputSourceRevision::from_snapshot(&input.text_snapshot())
                    }) == revision;
                    if !still_current {
                        this.reissue_conflict_resolved_output_live_syntax_build(cx);
                        return;
                    }
                    this.conflict_resolved_output_live_syntax = Some(document);
                    // Record what it was built for, or the next refresh sees a
                    // stale source, retries in the foreground, fails the budget
                    // again and schedules another build -- forever.
                    this.conflict_resolved_output_live_syntax_source = Some((revision, mask));
                    this.rebind_conflict_resolved_output_highlight_provider(cx);
                });
            }));
    }

    /// Re-run the off-thread first parse against the buffer as it stands now.
    ///
    /// Called when a build lands for text the buffer has already moved past.
    /// Recomputes the source the way [`Self::refresh_conflict_resolved_output_syntax`]
    /// does, so the two cannot disagree about what the tree is being built over.
    /// A no-op once a document exists -- from there on, edits go through
    /// `sync`, which always has a tree to fall back on.
    pub(super) fn reissue_conflict_resolved_output_live_syntax_build(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_live_syntax.is_some() {
            return;
        }
        let Some(language) = self.conflict_resolved_preview_syntax_language else {
            return;
        };
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        let rope = output_snapshot.rope();
        let protected_ranges = resolved_output_placeholder_protected_ranges(&rope);
        let mask = resolved_output_live_syntax_mask(protected_ranges.as_ref(), &rope);
        let revision = ResolvedOutputSourceRevision::from_snapshot(&output_snapshot);
        self.ensure_conflict_resolved_output_live_syntax_build(
            language,
            output_snapshot.rope(),
            mask,
            revision,
            cx,
        );
    }

    /// Finish a reparse the foreground budget could not.
    ///
    /// Only reachable when an edit landed on a document too large to reparse in
    /// the budget. The viewport is not blocked meanwhile: the `tree.edit()`ed
    /// tree is already positionally correct, so it keeps painting — this just
    /// restores exactness near the edit.
    pub(super) fn ensure_conflict_resolved_output_live_syntax_reparse(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(request) = self
            .conflict_resolved_output_live_syntax
            .as_ref()
            .and_then(rows::LiveSyntaxDocument::background_reparse_request)
        else {
            self.conflict_resolved_output_live_syntax_reparse = None;
            return;
        };

        self.conflict_resolved_output_live_syntax_reparse =
            Some(cx.spawn(async move |view: WeakEntity<MainPaneView>, cx| {
                let reparse = move || rows::live_syntax_reparse(request);
                let parsed = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(reparse).await
                } else {
                    reparse()
                };
                let Some((version, tree, injections)) = parsed else {
                    return;
                };
                let _ = view.update(cx, |this, cx| {
                    let adopted = this
                        .conflict_resolved_output_live_syntax
                        .as_mut()
                        .is_some_and(|document| {
                            document.adopt_background_tree(version, tree, injections)
                        });
                    if !adopted {
                        // The buffer moved while this was in flight, so the tree
                        // describes text that no longer exists. Re-issue from
                        // wherever the document is now.
                        this.conflict_resolved_output_live_syntax_reparse = None;
                        this.ensure_conflict_resolved_output_live_syntax_reparse(cx);
                        return;
                    }
                    this.conflict_resolved_output_live_syntax_reparse = None;
                    this.rebind_conflict_resolved_output_highlight_provider(cx);
                });
            }));
    }

    /// Hand the input a provider over the document's current tree.
    pub(super) fn rebind_conflict_resolved_output_highlight_provider(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((version, snapshot)) = self
            .conflict_resolved_output_live_syntax
            .as_ref()
            .map(|document| (document.version(), document.snapshot(self.theme)))
        else {
            return;
        };
        let output_snapshot = self
            .conflict_resolver_input
            .read_with(cx, |input, _| input.text_snapshot());
        self.conflict_resolved_output_highlighted_conflict = self.conflict_resolver.active_conflict;
        // Reuse the scan from the last refresh when the text has not moved.
        // Rebinding happens on every conflict jump, and rescanning here is what
        // made navigation scale with the file rather than with the conflict.
        let rows = self.conflict_resolved_output_unresolved_rows_for(&output_snapshot);
        let unresolved_spans = resolved_output_unresolved_spans_for_active(
            rows.as_ref(),
            self.conflict_resolver.active_conflict,
        );
        let binding_key = resolved_output_live_provider_binding_key(
            version,
            self.conflict_resolved_output_provider_theme_epoch,
            &unresolved_spans,
        );
        let provider =
            resolved_output_live_highlight_provider(self.theme, snapshot, unresolved_spans);
        let source_len = output_snapshot.len();
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_highlight_provider_with_key(binding_key, provider, source_len, cx);
        });
    }

    /// Bring the resolved output's live tree up to date with `output_snapshot`
    /// and rebind the highlight provider to it.
    ///
    /// `edit` is the coalesced `(replaced, inserted)` span, or `None` when the
    /// text was replaced wholesale (bootstrap, a conflict resolution, an undo of
    /// one) — which reparses from scratch.
    ///
    /// Cheap enough to run on the keystroke: the tree is edited in place and the
    /// reparse reuses it, rather than rebuilding the prepared document this
    /// replaced.
    ///
    /// The root parse is incremental; the *injected* layers are not. A reparse
    /// re-runs the injection query over the whole document and reparses every
    /// injected region from scratch, each with its own copy of the foreground
    /// budget. On a document with many injections (fenced blocks, `<script>`
    /// bodies) that is the dominant per-keystroke cost and does scale with the
    /// document — the outstanding gap against Zed's `SyntaxMap`, which keys
    /// layers by (language, range) and reparses them incrementally.
    pub(super) fn refresh_conflict_resolved_output_syntax(
        &mut self,
        output_snapshot: &TextModelSnapshot,
        edit: Option<(Range<usize>, Range<usize>)>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Everything below is derived from the *text*. When the text has not
        // moved, all of it still stands and the only thing that can need
        // updating is the provider binding — a theme change, or a different
        // conflict wearing the wash.
        //
        // Only when the caller reports no edit. An `edit` is the caller stating
        // that the text moved, and the tree must be folded forward even if the
        // revision happens to look settled — skipping the sync there leaves the
        // tree describing the pre-edit text, which shows up as the row you just
        // typed into keeping its old colours.
        let revision = ResolvedOutputSourceRevision::from_snapshot(output_snapshot);
        let text_is_unchanged = self
            .conflict_resolved_output_live_syntax_source
            .as_ref()
            .is_some_and(|(built_for, _)| *built_for == revision);
        // The language has to match too, not just the text. The reuse check
        // that would drop a document built by the wrong grammar lives *below*
        // this return, so leaving it out lets a language change with unchanged
        // text keep the previous grammar's tree — the state
        // `conflict_resolver_invalidate_resolved_outline` leaves behind, which
        // clears the language but not the live document.
        let language_is_unchanged = self
            .conflict_resolved_output_live_syntax
            .as_ref()
            .is_some_and(|document| {
                Some(document.language()) == self.conflict_resolved_preview_syntax_language
            });
        if edit.is_none() && text_is_unchanged && language_is_unchanged {
            self.conflict_resolved_output_highlighted_conflict =
                self.conflict_resolver.active_conflict;
            self.rebind_conflict_resolved_output_highlight_provider(cx);
            return;
        }

        // Every read below goes through the rope, and nothing on this path
        // builds either whole-document cache — not the flattened string, and
        // not the line-start array, which is the quieter of the two and was the
        // one that lingered here. `no_materialization_tests` asserts both.
        let rope = output_snapshot.rope();
        self.conflict_resolved_output_highlighted_conflict = self.conflict_resolver.active_conflict;
        #[cfg(test)]
        {
            self.conflict_resolved_output_full_scans += 1;
        }
        let unresolved_rows = resolved_output_unresolved_rows(
            &self.conflict_resolver.marker_segments,
            &rope,
            &self.conflict_resolved_output_block_map,
        );
        self.conflict_resolved_output_unresolved_rows = Some((
            ResolvedOutputKey::new(
                output_snapshot,
                &self.conflict_resolver.marker_segments,
                &self.conflict_resolved_output_block_map,
            ),
            Arc::clone(&unresolved_rows),
        ));
        let unresolved_spans = resolved_output_unresolved_spans_for_active(
            unresolved_rows.as_ref(),
            self.conflict_resolver.active_conflict,
        );
        // The placeholder rows are a rendering of open decisions, so hand them
        // to the buffer as uneditable spans — and hide the same spans from the
        // parser, which would otherwise read `<Merge Conflict>` as code.
        let protected_ranges = resolved_output_placeholder_protected_ranges(&rope);
        let mask = resolved_output_live_syntax_mask(protected_ranges.as_ref(), &rope);
        let budget = Some(self.full_document_syntax_budget().foreground_parse);

        let language = self.conflict_resolved_preview_syntax_language;
        let reusable = self
            .conflict_resolved_output_live_syntax
            .as_ref()
            .is_some_and(|document| Some(document.language()) == language);
        if !reusable {
            self.conflict_resolved_output_live_syntax = None;
            self.conflict_resolved_output_live_syntax_source = None;
        }

        let revision = ResolvedOutputSourceRevision::from_snapshot(output_snapshot);
        let current = self
            .conflict_resolved_output_live_syntax_source
            .as_ref()
            .is_some_and(|(built_for, built_mask)| {
                *built_for == revision && built_mask.as_ref() == mask.as_ref()
            });

        match self.conflict_resolved_output_live_syntax.as_mut() {
            // Nothing about the buffer moved. The tree stands, and so does its
            // version — the binding key below folds in the theme and the
            // unresolved spans, so an overlay change still rebinds while a
            // no-op re-entry does not, which is what stops this method from
            // re-triggering the observe that called it.
            Some(_) if current => {}
            Some(document) => {
                let outcome = document.sync(rope.clone(), Arc::clone(&mask), edit, budget);
                if outcome == rows::LiveSyntaxSyncOutcome::Abandoned {
                    // The edit took the buffer past the size ceiling, so the
                    // document now describes text that no longer exists. Drop it
                    // and take the heuristic arm below, which is the same answer
                    // a buffer that started out this large would have got.
                    self.conflict_resolved_output_live_syntax = None;
                    self.conflict_resolved_output_live_syntax_source = None;
                } else {
                    self.conflict_resolved_output_live_syntax_source =
                        Some((revision, Arc::clone(&mask)));
                }
            }
            None => {
                // Zed's fast path (`Buffer::reparse` under `sync_parse_timeout`):
                // worth a budgeted attempt because a small buffer finishes inside
                // it and never shows a frame of unhighlighted text. Skipped when
                // a build for exactly this text is already off-thread -- that
                // attempt has demonstrably failed once, so re-running it on the
                // keystroke path is pure latency.
                let already_building =
                    self.conflict_resolved_output_live_syntax_building == Some(revision);
                self.conflict_resolved_output_live_syntax =
                    language.filter(|_| !already_building).and_then(|language| {
                        rows::LiveSyntaxDocument::new(
                            language,
                            rope.clone(),
                            Arc::clone(&mask),
                            budget,
                        )
                    });
                self.conflict_resolved_output_live_syntax_source = self
                    .conflict_resolved_output_live_syntax
                    .is_some()
                    .then(|| (revision, Arc::clone(&mask)));

                // A first parse has no tree to fall back on, so exhausting the
                // foreground budget leaves nothing at all -- and an incremental
                // reparse can never rescue it, because there is no document to
                // reparse. Finish it off-thread instead. Not a rare path: the
                // live budget is 1ms and a cold parse of a ~10KB file is
                // already over it, so without this the resolved output would
                // sit on heuristic tokens for the whole session.
                if let Some(language) =
                    language.filter(|_| self.conflict_resolved_output_live_syntax.is_none())
                {
                    self.ensure_conflict_resolved_output_live_syntax_build(
                        language,
                        rope.clone(),
                        Arc::clone(&mask),
                        revision,
                        cx,
                    );
                }
            }
        }

        let live = self
            .conflict_resolved_output_live_syntax
            .as_ref()
            .map(|document| (document.version(), document.snapshot(self.theme)));
        self.ensure_conflict_resolved_output_live_syntax_reparse(cx);

        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_protected_ranges(protected_ranges);
            match live {
                Some((version, snapshot)) => {
                    let provider = resolved_output_live_highlight_provider(
                        self.theme,
                        snapshot,
                        unresolved_spans.clone(),
                    );
                    // Rebinding under a fresh key whenever the text moved is
                    // load-bearing: it resets the interpolation that would
                    // otherwise map these already-current highlights through a
                    // stale patch.
                    let binding_key = resolved_output_live_provider_binding_key(
                        version,
                        self.conflict_resolved_output_provider_theme_epoch,
                        &unresolved_spans,
                    );
                    input.set_highlight_provider_with_key(binding_key, provider, rope.len(), cx);
                }
                None => {
                    // Heuristic tokens, with the open conflicts still called out
                    // in red. Reachable in exactly two states, both permanent and
                    // both shared with the diff panes above -- the language has
                    // no wired grammar, or the text is past
                    // `PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES`. It is *not*
                    // a general fallback: the tokenizer knows only keywords,
                    // strings, numbers and comments, so landing here while a
                    // grammar exists is precisely the bug where the output stops
                    // matching the panes above it. A budget-exhausted first parse
                    // must go to `ensure_conflict_resolved_output_live_syntax_build`
                    // instead.
                    //
                    // A provider rather than a whole-document `set_highlights`:
                    // this arm is reached by the *largest* buffers, and the
                    // tokenizer is line-local, so answering per window is both
                    // exact and proportional to the viewport.
                    let provider = resolved_output_heuristic_highlight_provider(
                        self.theme,
                        rope.clone(),
                        language,
                        unresolved_spans.clone(),
                    );
                    let binding_key = resolved_output_heuristic_provider_binding_key(
                        revision,
                        self.conflict_resolved_output_provider_theme_epoch,
                        &unresolved_spans,
                    );
                    input.set_highlight_provider_with_key(binding_key, provider, rope.len(), cx);
                }
            }
        });
    }

    /// Schedule a background tree-sitter parse for one merge-input side.
    ///
    /// When the parse completes, the prepared document is injected into the
    /// global cache and the three-way styled-text cache is cleared so the next
    /// render picks up document-based syntax highlighting.
    pub(in crate::view) fn ensure_conflict_three_way_background_syntax_prepare(
        &mut self,
        side: ThreeWayColumn,
        text: SharedString,
        line_starts: Arc<[usize]>,
        language: rows::DiffSyntaxLanguage,
        source_hash: Option<u64>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_three_way_syntax_inflight[side] {
            return;
        }
        self.conflict_three_way_syntax_inflight[side] = true;
        let expected_source_hash = source_hash;
        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let prepare_document = move || {
                    rows::prepare_diff_syntax_document_in_background_text_with_reuse(
                        language,
                        rows::DiffSyntaxMode::Auto,
                        text,
                        line_starts,
                        None,
                        None,
                    )
                };
                let parsed = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(prepare_document).await
                } else {
                    prepare_document()
                };

                let _ = view.update(cx, |this, cx| {
                    this.conflict_three_way_syntax_inflight[side] = false;

                    // Stale: source hash changed while we were parsing.
                    if this.conflict_resolver.source_hash != expected_source_hash {
                        return;
                    }

                    if let Some(parsed) = parsed {
                        let document =
                            rows::inject_background_prepared_diff_syntax_document(parsed);
                        this.conflict_three_way_prepared_syntax_documents[side] = Some(document);
                        // Invalidate cached styled text so the next render uses
                        // the prepared document across three-way and two-way
                        // conflict views instead of per-line fallback styling.
                        this.clear_conflict_diff_style_caches_preserving_query();
                        this.conflict_three_way_segments_cache.clear();
                        this.conflict_three_way_query_segments_cache.clear();
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }
}
