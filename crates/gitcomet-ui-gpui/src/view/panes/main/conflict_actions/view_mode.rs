//! View-mode switches, hide-resolved and collapsed-context toggles, and the
//! visible-map rebuild they share.

use super::*;

impl MainPaneView {
    pub(in crate::view) fn conflict_resolver_set_view_mode(
        &mut self,
        view_mode: ConflictResolverViewMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.view_mode == view_mode {
            if view_mode == ConflictResolverViewMode::ThreeWay {
                let _ = self.request_conflict_file_load_mode(
                    gitcomet_state::model::ConflictFileLoadMode::Full,
                );
            }
            return;
        }
        self.conflict_resolver.view_mode = view_mode;
        self.set_mergetool_view_three_way_and_persist(
            view_mode == ConflictResolverViewMode::ThreeWay,
            cx,
        );
        self.conflict_resolver.hovered_conflict = None;
        // View-mode switches rebuild visible projections and can temporarily
        // reuse the same cache keys with different row text or syntax state.
        // Drop both caches so the next draw restyles from the current prepared
        // documents instead of pinning stale fallback output across toggles.
        self.clear_conflict_diff_style_caches_preserving_query();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        if view_mode == ConflictResolverViewMode::ThreeWay
            && self
                .request_conflict_file_load_mode(gitcomet_state::model::ConflictFileLoadMode::Full)
        {
            // Build three-way visible state from the data we already have so
            // the view shows existing rows (with syntax) while the full file
            // reloads in the background.
            self.conflict_resolver.rebuild_three_way_visible_state();
            cx.notify();
            return;
        }
        if view_mode == ConflictResolverViewMode::ThreeWay {
            self.conflict_resolver.rebuild_three_way_visible_state();
        } else {
            // Rebuild two-way visible projections so the split view reflects
            // the current hide_resolved state and resolved conflict choices.
            self.conflict_resolver.rebuild_two_way_visible_projections();
        }
        let path = self.conflict_resolver.path.clone();
        let output_line_count = if self.conflict_resolved_output_is_streamed() {
            self.conflict_resolved_preview_line_count.max(1)
        } else {
            self.conflict_resolver_input.read_with(cx, |input, _| {
                input.text_snapshot().shared_line_starts().len().max(1)
            })
        };
        if should_skip_resolved_outline_provenance(view_mode, output_line_count) {
            // The existing marker overlay remains valid across view-mode switches,
            // but view-mode-specific provenance/dedupe metadata is too expensive to
            // rebuild synchronously for huge outputs.
            self.conflict_resolver.resolved_outline.meta.clear();
            self.conflict_resolver
                .resolved_outline
                .sources_index
                .clear();
            self.conflict_resolver.resolved_outline_gutter_rows.clear();
        } else {
            self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
        }
        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_preserving_current();
        }
        cx.notify();
    }

    pub(in crate::view) fn conflict_resolver_toggle_hide_resolved(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.hide_resolved = !self.conflict_resolver.hide_resolved;
        self.conflict_resolver_rebuild_visible_map();
        if let (Some(repo_id), Some(path)) = (
            self.conflict_resolver
                .repo_id
                .or_else(|| self.active_repo_id()),
            self.conflict_resolver.dispatch_path(),
        ) {
            self.store.dispatch(Msg::ConflictSetHideResolved {
                repo_id,
                path,
                hide_resolved: self.conflict_resolver.hide_resolved,
            });
        }
        cx.notify();
    }

    /// Toggle section 30 collapsed context mode: fold unchanged runs beyond the
    /// per-conflict context window in the source columns.
    pub(in crate::view) fn conflict_resolver_toggle_collapse_context(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.collapse_context = !self.conflict_resolver.collapse_context;
        // The live toggle doubles as the persisted default for the next
        // conflicted file (cog-menu setting).
        if self.mergetool_collapse_unchanged != self.conflict_resolver.collapse_context {
            self.mergetool_collapse_unchanged = self.conflict_resolver.collapse_context;
            self.schedule_ui_settings_persist(cx);
        }
        self.conflict_resolver.context_fold_reveals.clear();
        self.conflict_resolver.output_context_fold_reveals.clear();
        self.conflict_resolver.resolved_output_visible_dirty = true;
        self.conflict_resolver_rebuild_visible_map();
        // Keep the semantic target in view across the row-space change.
        if let Some(target_index) = self.conflict_resolver.selected_nav_target_index()
            && let Some(target) = self.conflict_resolver.nav_targets.get(target_index)
            && let Some(vi) = self.conflict_resolver_visible_ix_for_nav_target(target)
        {
            self.conflict_resolver_scroll_all_columns(vi, gpui::ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Fully expand one collapsed context fold.
    pub(in crate::view) fn conflict_resolver_expand_context_fold(
        &mut self,
        fold_id: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let reveal = self
            .conflict_resolver
            .context_fold_reveals
            .entry(fold_id)
            .or_default();
        if reveal.expand_all {
            return;
        }
        reveal.expand_all = true;
        self.conflict_resolver_rebuild_visible_map();
        cx.notify();
    }

    /// Reveal [`CONFLICT_FOLD_REVEAL_STEP`] more lines at one edge of a fold
    /// (top = extend the context above downward; bottom = extend the context
    /// below upward), mirroring the diff view's collapsed-hunk arrows.
    ///
    /// [`CONFLICT_FOLD_REVEAL_STEP`]: conflict_resolver::CONFLICT_FOLD_REVEAL_STEP
    pub(in crate::view) fn conflict_resolver_reveal_context_fold(
        &mut self,
        fold_id: usize,
        from_top: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let reveal = self
            .conflict_resolver
            .context_fold_reveals
            .entry(fold_id)
            .or_default();
        if from_top {
            reveal.top += conflict_resolver::CONFLICT_FOLD_REVEAL_STEP;
        } else {
            reveal.bottom += conflict_resolver::CONFLICT_FOLD_REVEAL_STEP;
        }
        self.conflict_resolver_rebuild_visible_map();
        cx.notify();
    }

    pub(in crate::view::panes::main) fn conflict_resolver_rebuild_visible_map(&mut self) {
        if self.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
            || self.conflict_resolver.has_three_way_visible_state_ready()
            || self.conflict_resolver.two_way_uses_aligned_rows()
        {
            self.conflict_resolver.rebuild_three_way_visible_state();
        } else {
            self.conflict_resolver
                .refresh_conflict_has_base_from_segments();
        }
        let block_count = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter(|seg| matches!(seg, conflict_resolver::ConflictSegment::Block(_)))
            .count();
        if self
            .conflict_resolver
            .hovered_conflict
            .is_some_and(|(ix, _)| ix >= block_count)
        {
            self.conflict_resolver.hovered_conflict = None;
        }
        self.conflict_resolver.rebuild_two_way_visible_state();
        self.conflict_resolver_refresh_nav_targets();
        self.conflict_resolver
            .debug_assert_rendering_mode_invariants();
    }

    /// Live collapse-unchanged-context state, read by the cog settings menu.
    pub(in crate::view) fn conflict_resolver_collapse_context(&self) -> bool {
        self.conflict_resolver.collapse_context
    }
}
