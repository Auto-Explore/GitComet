//! State-to-resolver synchronization: bootstrap, lightweight resync, and
//! conflict file load requests.

use super::bootstrap::{
    ConflictBootstrapSources, ConflictMarkerLayout, ConflictSessionMapping,
    MergetoolBootstrapTraceDecisions, MergetoolTraceContext, apply_bootstrap_session_resolutions,
    bootstrap_marker_layout, bootstrap_split_rows, bootstrap_three_way_sides,
    bootstrap_word_highlights, conflict_file_source_fingerprint, conflict_session_plan_projection,
    generate_bootstrap_resolved_output, prepare_three_way_syntax_documents,
};
use super::*;

/// The active repo's conflicted, unstaged working-tree target and its conflict
/// kind; `None` when the diff shows anything else.
fn conflicted_worktree_target(
    state: &AppState,
    repo_id: RepoId,
) -> Option<(
    &RepoState,
    &std::path::PathBuf,
    Option<gitcomet_core::domain::FileConflictKind>,
)> {
    let repo = state.repos.iter().find(|r| r.id == repo_id)?;

    let Some(DiffTarget::WorkingTree { path, area, .. }) = repo.diff_state.diff_target.as_ref()
    else {
        return None;
    };
    if *area != DiffArea::Unstaged {
        return None;
    }

    let conflict_entry = repo
        .status_entry_for_path(DiffArea::Unstaged, path.as_path())
        .filter(|entry| entry.kind == gitcomet_core::domain::FileStatusKind::Conflicted)?;
    Some((repo, path, conflict_entry.conflict))
}

/// View options a same-conflict rebuild carries over from the previous state.
struct CarriedConflictViewState {
    view_mode: ConflictResolverViewMode,
    hide_resolved: bool,
    collapse_context: bool,
    nav_anchor: Option<conflict_resolver::ConflictNavAnchor>,
    nav_targets: Vec<conflict_resolver::ConflictNavTarget>,
    active_conflict: Option<usize>,
    resolver_preview_mode: ConflictResolverPreviewMode,
}

impl MainPaneView {
    fn clear_conflict_resolver_state(&mut self, cx: &mut gpui::Context<Self>) {
        self.reset_conflict_image_preview_cache(cx);
        self.conflict_resolver = ConflictResolverUiState::default();
        self.conflict_resolved_output_saved_snapshot = None;
        self.conflict_resolved_output_modified = false;
        self.conflict_resolved_output_block_map =
            conflict_resolver::ResolvedOutputBlockMap::default();
        self.conflict_resolver_invalidate_resolved_outline();
    }

    pub(in crate::view::panes::main) fn sync_conflict_resolver(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.active_repo_id() else {
            self.clear_conflict_resolver_state(cx);
            return;
        };

        let Some((repo, path, conflict_kind)) = conflicted_worktree_target(&self.state, repo_id)
        else {
            self.clear_conflict_resolver_state(cx);
            return;
        };

        let path = path.clone();
        let trace_path = path.clone();

        let should_load = repo.conflict_state.conflict_file_path.as_ref() != Some(&path)
            && !matches!(repo.conflict_state.conflict_file, Loadable::Loading);
        if should_load {
            self.start_conflict_file_load(repo_id, path, cx);
            return;
        }

        let Loadable::Ready(Some(file)) = &repo.conflict_state.conflict_file else {
            return;
        };
        if file.path != path {
            return;
        }

        let source_hash = conflict_file_source_fingerprint(file);

        let needs_rebuild = self.conflict_resolver.repo_id != Some(repo_id)
            || self.conflict_resolver.path.as_ref() != Some(&path)
            || self.conflict_resolver.source_hash != Some(source_hash);

        // When the file content hasn't changed but state-side conflict data has
        // been updated (e.g. hide_resolved toggled externally, bulk picks, or
        // autosolve applied from state), do a lightweight re-sync that re-applies
        // session resolutions and rebuilds visible maps without recomputing the
        // expensive diff/highlight data.
        if !needs_rebuild {
            if self.conflict_resolver.conflict_rev != repo.conflict_state.conflict_rev {
                self.resync_conflict_resolver_from_state(cx);
            }
            return;
        }

        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_query_segments_cache_split.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.conflict_diff_query_cache_query = SharedString::default();

        // A CurrentOnly load intentionally omits all three immutable conflict
        // sides. Specialized resolvers need those exact bytes before their
        // completion actions can be enabled.
        let needs_full_side_payloads =
            file.base_bytes.is_none() && file.ours_bytes.is_none() && file.theirs_bytes.is_none();

        let (conflict_strategy, is_binary) = Self::conflict_file_strategy(
            repo.conflict_state.conflict_session.as_ref(),
            file,
            conflict_kind,
        );
        let conflict_syntax_language = rows::diff_syntax_language_for_path(&path);
        let shared_path = gitcomet_state::msg::RepoPath::from(path.clone());

        // For binary conflicts, populate minimal state and return early.
        if is_binary {
            let binary_side_sizes = [
                file.base_bytes.as_ref().map(|b| b.len()),
                file.ours_bytes.as_ref().map(|b| b.len()),
                file.theirs_bytes.as_ref().map(|b| b.len()),
            ];
            if let Some(cancel) = self.conflict_image_preview_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Release);
            }
            self.conflict_image_preview_inflight = None;
            super::super::preview::release_conflict_preview_render_images(
                &self.conflict_resolver.image_preview,
                cx,
            );
            self.conflict_resolver = ConflictResolverUiState {
                repo_id: Some(repo_id),
                path: Some(path),
                shared_path: Some(shared_path),
                loaded_file: Some(file.clone()),
                conflict_syntax_language,
                source_hash: Some(source_hash),
                is_binary_conflict: true,
                binary_side_sizes,
                strategy: conflict_strategy,
                conflict_kind,
                last_autosolve_summary: None,
                open_summary_counts: None,
                conflict_rev: repo.conflict_state.conflict_rev,
                ..ConflictResolverUiState::default()
            };
            self.conflict_resolver_invalidate_resolved_outline();
            if needs_full_side_payloads {
                let _ = self.request_conflict_file_load_mode(
                    gitcomet_state::model::ConflictFileLoadMode::Full,
                );
            }
            return;
        }

        let bootstrap_started = Instant::now();
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .filter(|session| session.path == path);
        let ConflictBootstrapSources {
            current_text,
            plan_projection,
            marker_snapshot,
            output_is_protected,
            base_text,
            ours_text,
            theirs_text,
            trace_ctx,
            needs_full_side_texts,
            full_text_plan_upgrade_expected,
            three_way_base_len,
            three_way_ours_len,
            three_way_theirs_len,
            three_way_side_max_len,
        } = ConflictBootstrapSources::gather(file, session, trace_path, conflict_strategy);
        let is_same_conflict = self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path);

        let ConflictMarkerLayout {
            mut marker_segments,
            conflict_region_marker_has_base,
            rendering_mode,
            three_way_aligned,
            three_way_len,
            mut trace_decisions,
        } = bootstrap_marker_layout(
            &marker_snapshot,
            session,
            base_text,
            ours_text,
            theirs_text,
            three_way_side_max_len,
            conflict_syntax_language,
            &trace_ctx,
        );

        let ConflictSessionMapping {
            original_region_aligned_ranges,
            conflict_region_indices,
            display_plan_block_indices,
            merge_plan_aligned_conflict_ranges,
        } = apply_bootstrap_session_resolutions(
            &mut marker_segments,
            file,
            session,
            &plan_projection,
            &three_way_aligned,
            three_way_base_len,
            three_way_ours_len,
            three_way_theirs_len,
        );
        let conflict_block_count = conflict_resolver::conflict_count(&marker_segments);

        let (resolved_output_text, streamed_output_projection, resolved_line_count) =
            generate_bootstrap_resolved_output(
                output_is_protected,
                &current_text,
                rendering_mode,
                &marker_segments,
                &marker_snapshot,
                file,
                &trace_ctx,
                &mut trace_decisions,
                conflict_block_count,
            );

        let (three_way_text, three_way_line_starts) = bootstrap_three_way_sides(
            file,
            three_way_base_len,
            three_way_ours_len,
            three_way_theirs_len,
        );

        let (mode_state, diff_row_count) = bootstrap_split_rows(
            &marker_segments,
            &trace_ctx,
            &mut trace_decisions,
            conflict_block_count,
        );
        let inline_row_count = 0;

        let (three_way_word_highlights, two_way_aligned_word_highlights) =
            bootstrap_word_highlights(
                &three_way_aligned,
                base_text,
                ours_text,
                theirs_text,
                &three_way_line_starts,
                &trace_ctx,
                trace_decisions,
                conflict_block_count,
                diff_row_count,
            );

        // Three-way conflict maps and visible state are deferred to
        // `rebuild_three_way_visible_state()` after state construction.

        let CarriedConflictViewState {
            view_mode,
            hide_resolved,
            collapse_context,
            nav_anchor,
            nav_targets,
            active_conflict,
            resolver_preview_mode,
        } = self.carried_conflict_view_state(
            repo,
            file,
            is_same_conflict,
            conflict_strategy,
            conflict_kind,
            needs_full_side_texts,
            &marker_segments,
        );
        let (last_autosolve_summary, open_summary_counts, open_summary_announced) = self
            .conflict_open_summary_state(
                repo,
                repo_id,
                &path,
                is_same_conflict,
                full_text_plan_upgrade_expected,
            );

        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();

        // Try foreground tree-sitter parse for each merge-input side.
        // If a parse times out, we schedule a background task below.
        let budget = self.full_document_syntax_budget();
        let (three_way_prepared_docs, three_way_needs_background) =
            prepare_three_way_syntax_documents(
                conflict_syntax_language,
                &three_way_text,
                &three_way_line_starts,
                budget,
            );
        self.conflict_three_way_prepared_syntax_documents = three_way_prepared_docs;
        self.conflict_three_way_syntax_inflight = ThreeWaySides::default();
        let shared_path = gitcomet_state::msg::RepoPath::from(path.clone());

        // Build state with core/shared fields; mode-dependent visible state
        // is populated by the rebuild methods below.
        if let Some(cancel) = self.conflict_image_preview_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        self.conflict_image_preview_inflight = None;
        super::super::preview::release_conflict_preview_render_images(
            &self.conflict_resolver.image_preview,
            cx,
        );
        self.conflict_resolver = ConflictResolverUiState {
            repo_id: Some(repo_id),
            path: Some(path),
            shared_path: Some(shared_path),
            loaded_file: Some(file.clone()),
            conflict_syntax_language,
            source_hash: Some(source_hash),
            output_is_protected,
            // A re-bootstrap means a different conflict or different file
            // content, so an earlier waiver no longer speaks for it.
            output_protection_waived: false,
            current: marker_snapshot,
            marker_segments,
            collapse_context,
            context_fold_reveals: if is_same_conflict {
                std::mem::take(&mut self.conflict_resolver.context_fold_reveals)
            } else {
                FxHashMap::default()
            },
            resolved_output_visible: None,
            resolved_output_visible_dirty: true,
            output_context_fold_reveals: if is_same_conflict {
                std::mem::take(&mut self.conflict_resolver.output_context_fold_reveals)
            } else {
                FxHashMap::default()
            },
            conflict_region_indices,
            display_plan_block_indices,
            conflict_region_marker_has_base,
            active_conflict,
            nav_targets,
            original_region_aligned_ranges,
            hovered_conflict: None,
            // section 30 split: any pending row selection is invalidated by a source
            // rebuild (which happens after a split changes the segmentation).
            row_selection: None,
            // Pending alignment marks are line numbers into the old source, so
            // a rebuild invalidates them the same way.
            alignment_selection: ThreeWaySides::default(),
            mode_state,
            view_mode,
            three_way_text,
            three_way_line_starts,
            three_way_len,
            three_way_aligned,
            minimap_bands: Arc::from([]),
            merge_plan_aligned_conflict_ranges,
            three_way_visible_state_ready: false,
            three_way_conflict_ranges: ThreeWaySides::default(),
            three_way_horizontal_measure_rows: [0; 3],
            conflict_has_base: Vec::new(),
            conflict_choices: Vec::new(),
            two_way_split_visual_kind_cache: FxHashMap::default(),
            two_way_horizontal_measure_rows: [0; 2],
            three_way_word_highlights,
            two_way_aligned_word_highlights,
            two_way_split_word_highlight_cache: Default::default(),
            nav_anchor,
            hide_resolved,
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: conflict_strategy,
            conflict_kind,
            last_autosolve_summary,
            open_summary_counts,
            open_summary_announced,
            conflict_rev: repo.conflict_state.conflict_rev,
            visible_projection_rev: self.conflict_resolver.visible_projection_rev,
            resolver_pending_recompute_seq: 0,
            resolved_outline: ResolvedOutlineData::default(),
            resolved_outline_gutter_rows: Vec::new(),
            markdown_preview: ConflictResolverMarkdownPreviewState::default(),
            image_preview: ConflictResolverImagePreviewState::default(),
            resolver_preview_mode,
            output_save_format: None,
            output_saved_format: None,
        };
        self.rebuild_bootstrapped_conflict_visible_state(
            &trace_ctx,
            trace_decisions,
            conflict_block_count,
        );

        self.install_bootstrapped_resolved_output(
            streamed_output_projection,
            resolved_output_text,
            &trace_ctx,
            trace_decisions,
            conflict_block_count,
            diff_row_count,
            inline_row_count,
            resolved_line_count,
            cx,
        );
        // The resolved output is an editable, kdiff3-style free-text pane, so the
        // merged text must live in the buffer (not a read-only streamed
        // projection). Materialize here at bootstrap — driven, deterministic, and
        // one-time — rather than in the render path. Once materialized the output
        // stays out of streamed mode for this open, so every downstream refresh
        // keeps the buffer authoritative (all streamed paths are gated on
        // `conflict_resolved_output_is_streamed`).
        self.ensure_conflict_resolved_output_materialized(cx);
        self.rebuild_conflict_resolved_output_block_map(cx);
        self.mark_conflict_resolved_output_saved(cx);
        // On a fresh open, center the first unresolved semantic target (then
        // the first original conflict, then the first delta). Deferred item
        // scrolls apply once the lists lay out.
        if !is_same_conflict
            && let Some(target_index) = self.conflict_resolver.selected_nav_target_index()
        {
            self.conflict_jump_to_nav_target(target_index, cx);
        }
        self.announce_conflict_open_summary(cx);
        // section 30 aligned row space: whole-file column rows (three-way and
        // two-way full mode) need the side texts, which the fast CurrentOnly
        // first paint does not include. Upgrade fresh opens of reasonably
        // sized text conflicts to a Full load in the background; this
        // bootstrap re-runs with the sides once it lands. Giant files stay
        // on the block-local rows (the alignment gates reject them anyway).
        let specialized_strategy_needs_full_sides =
            conflict_strategy_needs_full_side_payloads(conflict_strategy);
        if !is_same_conflict
            && needs_full_side_texts
            && (specialized_strategy_needs_full_sides || full_text_plan_upgrade_expected)
        {
            let _ = self
                .request_conflict_file_load_mode(gitcomet_state::model::ConflictFileLoadMode::Full);
        }
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::ConflictResolverBootstrapTotal,
                    bootstrap_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
                .with_diff_row_count(Some(diff_row_count))
                .with_inline_row_count(Some(inline_row_count))
                .with_resolved_output_line_count(resolved_line_count)
        });

        self.schedule_conflict_three_way_background_syntax(
            conflict_syntax_language,
            three_way_needs_background,
            cx,
        );

        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_preserving_current();
        }
    }

    /// Clear the resolver and request the fast CurrentOnly conflict file load.
    fn start_conflict_file_load(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        self.clear_conflict_resolver_state(cx);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text("", cx);
        });
        self.store.dispatch(Msg::LoadConflictFile {
            repo_id,
            path,
            mode: gitcomet_state::model::ConflictFileLoadMode::CurrentOnly,
        });
    }

    /// Resolver strategy and binary flag for a loaded conflict file.
    fn conflict_file_strategy(
        session: Option<&gitcomet_core::conflict_session::ConflictSession>,
        file: &gitcomet_state::model::ConflictFile,
        conflict_kind: Option<gitcomet_core::domain::FileConflictKind>,
    ) -> (
        Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
        bool,
    ) {
        // Use the ConflictSession from state for strategy if available,
        // otherwise fall back to local computation.
        if let Some(session) = session {
            let binary =
                session.base.is_binary() || session.ours.is_binary() || session.theirs.is_binary();
            (Some(session.strategy), binary)
        } else {
            let binary = conflict_file_is_binary(file);
            (
                Self::conflict_resolver_strategy(conflict_kind, binary),
                binary,
            )
        }
    }

    /// View options for a rebuilt resolver: kept across a same-conflict
    /// rebuild, otherwise taken from settings and the store.
    fn carried_conflict_view_state(
        &self,
        repo: &RepoState,
        file: &gitcomet_state::model::ConflictFile,
        is_same_conflict: bool,
        conflict_strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
        conflict_kind: Option<gitcomet_core::domain::FileConflictKind>,
        needs_full_side_texts: bool,
        marker_segments: &[conflict_resolver::ConflictSegment],
    ) -> CarriedConflictViewState {
        let three_way_source_available = file.base.is_some()
            || (needs_full_side_texts
                && matches!(
                    conflict_kind,
                    Some(gitcomet_core::domain::FileConflictKind::BothModified)
                ));
        let view_mode = if is_same_conflict {
            self.conflict_resolver.view_mode
        } else if matches!(
            conflict_strategy,
            Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver)
        ) && three_way_source_available
            && self.mergetool_view_three_way
        {
            ConflictResolverViewMode::ThreeWay
        } else {
            // Base-absent conflicts and non-full-text strategies always open
            // two-way; base-present opens honor the persisted last-used mode.
            ConflictResolverViewMode::TwoWayDiff
        };

        let hide_resolved = if is_same_conflict {
            self.conflict_resolver.hide_resolved
        } else {
            repo.conflict_state.conflict_hide_resolved
        };
        let collapse_context = if is_same_conflict {
            self.conflict_resolver.collapse_context
        } else {
            // Fresh opens honor the persisted collapse-unchanged default.
            self.mergetool_collapse_unchanged
        };
        let nav_anchor = if is_same_conflict {
            self.conflict_resolver.nav_anchor
        } else {
            None
        };
        let nav_targets = if is_same_conflict {
            self.conflict_resolver.nav_targets.clone()
        } else {
            Vec::new()
        };
        let active_conflict = if is_same_conflict {
            self.conflict_resolver
                .active_conflict
                .filter(|index| *index < conflict_resolver::conflict_count(marker_segments))
        } else {
            None
        };
        let resolver_preview_mode = if is_same_conflict {
            self.conflict_resolver.resolver_preview_mode
        } else {
            ConflictResolverPreviewMode::default()
        };
        CarriedConflictViewState {
            view_mode,
            hide_resolved,
            collapse_context,
            nav_anchor,
            nav_targets,
            active_conflict,
            resolver_preview_mode,
        }
    }

    /// Autosolve summary and the kdiff3-style open report for a rebuilt
    /// resolver.
    fn conflict_open_summary_state(
        &self,
        repo: &RepoState,
        repo_id: RepoId,
        path: &std::path::Path,
        is_same_conflict: bool,
        full_text_plan_upgrade_expected: bool,
    ) -> (
        Option<SharedString>,
        Option<conflict_resolver::ConflictSummaryCounts>,
        bool,
    ) {
        let last_autosolve_summary = if is_same_conflict {
            self.conflict_resolver.last_autosolve_summary.clone()
        } else {
            repo.conflict_state
                .conflict_session
                .as_ref()
                .and_then(conflict_resolver::on_open_autosolve_summary)
                .map(Into::into)
        };
        let session_open_summary = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .filter(|session| session.path.as_path() == path)
            // CurrentOnly is a provisional marker-only session. Wait for its
            // Full upgrade so the open snapshot uses the plan-backed KDiff3
            // denominator and exact whitespace classification.
            .filter(|_| !full_text_plan_upgrade_expected)
            .map(conflict_resolver::conflict_session_summary_counts);
        // Same-conflict syncs keep the open-time snapshot, but backfill it when
        // still unset: the fast CurrentOnly first paint can run before the
        // session (and its autosolve pass) exists.
        let open_summary_counts =
            if is_same_conflict && self.conflict_resolver.open_summary_counts.is_some() {
                self.conflict_resolver.open_summary_counts
            } else {
                session_open_summary
            };
        let open_summary_announced = (is_same_conflict
            && self.conflict_resolver.open_summary_announced)
            || self
                .conflict_open_summary_toasted_files
                .contains(&(repo_id, path.to_path_buf()));
        (
            last_autosolve_summary,
            open_summary_counts,
            open_summary_announced,
        )
    }

    /// Build the visible projections and nav targets of a freshly
    /// bootstrapped resolver.
    fn rebuild_bootstrapped_conflict_visible_state(
        &mut self,
        trace_ctx: &MergetoolTraceContext,
        trace_decisions: MergetoolBootstrapTraceDecisions,
        conflict_block_count: usize,
    ) {
        // Populate mode-dependent visible state using the same code path as
        // later rebuilds (hide-resolved toggle, conflict picks, etc.). The
        // aligned two-way view shares the three-way projection, so it needs
        // the same build.
        let three_way_rebuild_started = Instant::now();
        if self.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay
            || self.conflict_resolver.two_way_uses_aligned_rows()
        {
            self.conflict_resolver.rebuild_three_way_visible_state();
        } else {
            self.conflict_resolver
                .refresh_conflict_has_base_from_segments();
        }
        mergetool_trace::record_with(|| {
            trace_ctx
                .bootstrap_event(
                    MergetoolTraceStage::BuildThreeWayConflictMaps,
                    three_way_rebuild_started,
                    trace_decisions,
                )
                .with_conflict_block_count(Some(conflict_block_count))
        });
        self.conflict_resolver.rebuild_two_way_visible_projections();
        self.conflict_resolver_refresh_nav_targets();
    }

    /// Hand the bootstrap's resolved output to the streamed preview or the
    /// editable buffer.
    #[allow(clippy::too_many_arguments)]
    fn install_bootstrapped_resolved_output(
        &mut self,
        streamed_output_projection: Option<conflict_resolver::ResolvedOutputProjection>,
        resolved_output_text: Option<conflict_resolver::ResolvedOutputText>,
        trace_ctx: &MergetoolTraceContext,
        trace_decisions: MergetoolBootstrapTraceDecisions,
        conflict_block_count: usize,
        diff_row_count: usize,
        inline_row_count: usize,
        resolved_line_count: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) {
        let output_path = self.conflict_resolver.path.clone();
        if let Some(projection) = streamed_output_projection {
            self.refresh_streamed_resolved_output_preview_from_projection(
                projection,
                output_path.as_ref(),
            );
        } else if let Some(resolved) = resolved_output_text {
            self.conflict_resolved_output_projection = None;
            let input_set_text_started = Instant::now();
            self.fill_conflict_resolved_output_buffer(resolved.into_shared_string(), cx);
            mergetool_trace::record_with(|| {
                trace_ctx
                    .bootstrap_event(
                        MergetoolTraceStage::ConflictResolverInputSetText,
                        input_set_text_started,
                        trace_decisions,
                    )
                    .with_conflict_block_count(Some(conflict_block_count))
                    .with_diff_row_count(Some(diff_row_count))
                    .with_inline_row_count(Some(inline_row_count))
                    .with_resolved_output_line_count(resolved_line_count)
            });
            self.conflict_resolved_preview_path = output_path.clone();
            let source_revision = self.conflict_resolver_input.read_with(cx, |input, _| {
                ResolvedOutputSourceRevision::from_snapshot(&input.text_snapshot())
            });
            self.conflict_resolved_preview_source_revision = Some(source_revision);
            self.schedule_conflict_resolved_outline_recompute(
                output_path.clone(),
                source_revision,
                None,
                cx,
            );
        }
    }

    fn announce_conflict_open_summary(&mut self, cx: &mut gpui::Context<Self>) {
        // kdiff3-style one-shot open summary: announce total / auto-solved /
        // unsolved once per resolver open, as soon as the stage-backed report
        // is available (the fast first paint may be CurrentOnly).
        if !self.conflict_resolver.open_summary_announced
            && let Some(counts) = self.conflict_resolver.open_summary_counts
            && let Some(message) = conflict_resolver::format_open_summary_toast(counts)
        {
            self.conflict_resolver.open_summary_announced = true;
            if let (Some(repo_id), Some(path)) = (
                self.conflict_resolver.repo_id,
                self.conflict_resolver.path.as_ref(),
            ) {
                self.conflict_open_summary_toasted_files
                    .insert((repo_id, path.clone()));
            }
            // The sync runs inside a GitCometView update; push the
            // toast after the current update flush to avoid reentrant
            // root-view updates.
            let root_view = self.root_view.clone();
            cx.defer(move |cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.push_toast(crate::view::components::ToastKind::Success, message, cx);
                });
            });
        }
    }

    fn schedule_conflict_three_way_background_syntax(
        &mut self,
        conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
        three_way_needs_background: ThreeWaySides<bool>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Schedule background syntax parses for merge-input sides that timed out.
        // Collect data up front to avoid borrowing conflict_resolver across the
        // mutable ensure_* call.
        if let Some(language) = conflict_syntax_language {
            let bg_source_hash = self.conflict_resolver.source_hash;
            let bg_sides: Vec<_> = ThreeWayColumn::ALL
                .into_iter()
                .filter(|&side| three_way_needs_background[side])
                .map(|side| {
                    (
                        side,
                        self.conflict_resolver.three_way_text[side].clone(),
                        self.conflict_resolver.three_way_shared_line_starts(side),
                    )
                })
                .collect();
            for (side, text, line_starts) in bg_sides {
                self.ensure_conflict_three_way_background_syntax_prepare(
                    side,
                    text,
                    line_starts,
                    language,
                    bg_source_hash,
                    cx,
                );
            }
        }
    }

    /// Lightweight re-sync when `conflict_rev` changed but file content is the
    /// same. Re-parses markers, re-applies session resolutions, reads
    /// `hide_resolved` from state, and rebuilds visible maps — without
    /// recomputing the expensive diff rows and word highlights.
    pub(in crate::view::panes::main) fn resync_conflict_resolver_from_state(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let Loadable::Ready(Some(file)) = &repo.conflict_state.conflict_file else {
            return;
        };
        let previous_blocks: Vec<_> = self
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.clone()),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
        let previous_region_indices = self.conflict_resolver.conflict_region_indices.clone();
        let previous_marker_projection = self.conflict_resolver.current.clone();
        let previous_output_is_protected = self.conflict_resolver.output_is_protected;
        let live_materialized_output = (!self.conflict_resolved_output_is_streamed()).then(|| {
            self.conflict_resolver_input
                .read_with(cx, |input, _| input.text().to_string())
        });
        let previous_map_valid = live_materialized_output.as_ref().is_some_and(|output| {
            self.conflict_resolved_output_block_map
                .is_valid_for(&self.conflict_resolver.marker_segments, output.as_str())
        });
        let previous_generated_output_matches_live =
            live_materialized_output.as_deref().is_some_and(|output| {
                conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments)
                    == output
            });

        let worktree_current = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .and_then(|session| session.current.as_ref()?.as_shared_text().cloned())
            .or_else(|| file.current.clone());
        let structural_marker_snapshot = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .and_then(|session| session.marker_projection.clone())
            .or_else(|| worktree_current.clone());
        let plan_projection = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .and_then(conflict_session_plan_projection);
        let marker_snapshot = plan_projection
            .as_ref()
            .map(|(text, _)| Arc::clone(text))
            .or_else(|| structural_marker_snapshot.clone());
        let next_output_is_protected = !self.conflict_resolver.output_protection_waived
            && worktree_output_requires_protection(
                worktree_current.as_deref(),
                structural_marker_snapshot.as_deref(),
                file.base.as_deref(),
                file.ours.as_deref(),
                file.theirs.as_deref(),
            );
        // The stage-derived marker snapshot drives conflict geometry. The
        // worktree payload remains independent so a partial or complete manual
        // resolution can be retained without making stale worktree markers the
        // structural source of truth.
        let mut marker_segments = marker_snapshot
            .clone()
            .map(conflict_resolver::parse_conflict_markers_shared_nonempty)
            .unwrap_or_default();
        let conflict_region_marker_has_base = marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
        // Re-populate bases from ancestor (needed for 2-way markers).
        if let Some(base_text) = file.base.clone() {
            conflict_resolver::populate_block_bases_from_shared_ancestor(
                &mut marker_segments,
                base_text,
            );
        }
        let original_display_aligned_ranges =
            conflict_resolver::project_conflict_ranges_to_aligned_rows(
                &marker_segments,
                &self.conflict_resolver.three_way_aligned,
                [
                    self.conflict_resolver
                        .three_way_line_count(ThreeWayColumn::Base),
                    self.conflict_resolver
                        .three_way_line_count(ThreeWayColumn::Ours),
                    self.conflict_resolver
                        .three_way_line_count(ThreeWayColumn::Theirs),
                ],
            );
        let mut conflict_region_indices =
            conflict_resolver::sequential_conflict_region_indices(&marker_segments);

        // Re-apply session region resolutions from state.
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .filter(|session| session.path == file.path);
        let original_region_aligned_ranges = session
            .map(|session| {
                conflict_resolver::conflict_nav_region_aligned_ranges(
                    session,
                    &original_display_aligned_ranges,
                )
            })
            .unwrap_or_else(|| {
                original_display_aligned_ranges
                    .iter()
                    .cloned()
                    .map(Some)
                    .collect()
            });
        let mut display_plan_block_indices = Vec::new();
        if let Some(session) = session {
            if let Some((_, projected_plan_blocks)) = plan_projection.as_ref()
                && let Some(applied) =
                    conflict_resolver::apply_plan_session_region_resolutions_with_index_map(
                        &mut marker_segments,
                        session,
                        projected_plan_blocks,
                    )
            {
                conflict_region_indices = applied.block_region_indices;
                display_plan_block_indices = applied.block_plan_indices;
            } else {
                let applied = conflict_resolver::apply_session_region_resolutions_with_index_map(
                    &mut marker_segments,
                    &session.regions,
                );
                conflict_region_indices = applied.block_region_indices;
            }
        }
        let merge_plan_aligned_conflict_ranges = session.and_then(|session| {
            conflict_resolver::merge_plan_aligned_conflict_ranges(
                session,
                &conflict_region_indices,
                &display_plan_block_indices,
            )
        });

        let use_streamed_projection = self.conflict_resolved_output_is_streamed()
            && !marker_segments.is_empty()
            && !next_output_is_protected;
        let next_blocks: Vec<_> = marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
        let mapped_replacements = (previous_marker_projection.as_deref()
            == marker_snapshot.as_deref()
            && previous_map_valid
            && previous_region_indices == conflict_region_indices
            && previous_blocks.len() == next_blocks.len()
            && previous_blocks
                .iter()
                .zip(&next_blocks)
                .all(|(previous, next)| {
                    previous.base == next.base
                        && previous.ours == next.ours
                        && previous.theirs == next.theirs
                }))
        .then(|| {
            previous_blocks
                .iter()
                .zip(&next_blocks)
                .enumerate()
                .filter_map(|(index, (previous, next))| {
                    (previous.choice != next.choice || previous.resolved != next.resolved)
                        .then_some(index)
                })
                .collect::<Vec<_>>()
        });
        let resolved = (!use_streamed_projection).then(|| {
            if next_output_is_protected {
                worktree_current
                    .clone()
                    .map(conflict_resolver::ResolvedOutputText::Shared)
                    .unwrap_or_else(|| {
                        conflict_resolver::bootstrap_resolved_output_text(
                            &marker_segments,
                            marker_snapshot.as_ref(),
                            file.ours.as_ref(),
                            file.theirs.as_ref(),
                        )
                    })
            } else {
                conflict_resolver::bootstrap_resolved_output_text(
                    &marker_segments,
                    marker_snapshot.as_ref(),
                    file.ours.as_ref(),
                    file.theirs.as_ref(),
                )
            }
        });

        // Read hide_resolved from state (authoritative source).
        let hide_resolved = repo.conflict_state.conflict_hide_resolved;

        let new_rev = repo.conflict_state.conflict_rev;

        // Update only the fields that change during a state re-sync.
        self.conflict_resolver.current = marker_snapshot;
        self.conflict_resolver.output_is_protected = next_output_is_protected;
        self.conflict_resolver.marker_segments = marker_segments;
        self.conflict_resolver.conflict_region_indices = conflict_region_indices;
        self.conflict_resolver.display_plan_block_indices = display_plan_block_indices;
        self.conflict_resolver.merge_plan_aligned_conflict_ranges =
            merge_plan_aligned_conflict_ranges;
        self.conflict_resolver.original_region_aligned_ranges = original_region_aligned_ranges;
        self.conflict_resolver.conflict_region_marker_has_base = conflict_region_marker_has_base;
        self.conflict_resolver.hide_resolved = hide_resolved;
        self.conflict_resolver.row_selection = None;
        self.conflict_resolver.conflict_syntax_language = self
            .conflict_resolver
            .path
            .as_ref()
            .and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolver.loaded_file = Some(file.clone());
        self.conflict_resolver.conflict_rev = new_rev;

        // Clear segment caches since marker_segments changed.
        self.clear_conflict_diff_style_caches();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.conflict_resolver_rebuild_visible_map();

        let output_path = self.conflict_resolver.path.clone();
        // Protection has to be able to clear itself. Carrying
        // `previous_output_is_protected` alone re-armed the flag on every
        // resync, so a session that was protected once stayed protected for
        // good — with the markers undecorated and every pick a silent no-op —
        // no matter what the predicate said afterwards. Unsaved manual edits to
        // the buffer are still held by the second disjunct, which is the case
        // that branch exists for.
        let preserve_unmapped_live_output = live_materialized_output.is_some()
            && ((previous_output_is_protected && next_output_is_protected)
                || (self.conflict_resolved_output_modified
                    && mapped_replacements.is_none()
                    && !previous_generated_output_matches_live));
        let mut preserved_materialized_output = preserve_unmapped_live_output;
        if preserve_unmapped_live_output {
            self.conflict_resolver.output_is_protected = true;
            self.conflict_resolved_output_block_map =
                conflict_resolver::ResolvedOutputBlockMap::default();
        }
        if use_streamed_projection {
            self.refresh_streamed_resolved_output_preview_from_markers(output_path.as_ref());
        } else if !preserved_materialized_output && let Some(block_indices) = mapped_replacements {
            let choices_unchanged = block_indices.is_empty();
            preserved_materialized_output = choices_unchanged
                || self.conflict_resolver_replace_mapped_blocks(&block_indices, cx);
            if preserved_materialized_output && choices_unchanged {
                let source_revision = self.conflict_resolver_input.read_with(cx, |input, _| {
                    ResolvedOutputSourceRevision::from_snapshot(&input.text_snapshot())
                });
                self.conflict_resolved_preview_path = output_path.clone();
                self.conflict_resolved_preview_source_revision = Some(source_revision);
                self.schedule_conflict_resolved_outline_recompute(
                    output_path.clone(),
                    source_revision,
                    None,
                    cx,
                );
            }
        }
        if !use_streamed_projection
            && !preserved_materialized_output
            && let Some(resolved) = resolved
        {
            self.conflict_resolved_output_projection = None;
            self.fill_conflict_resolved_output_buffer(resolved.into_shared_string(), cx);
            self.conflict_resolved_preview_path = output_path.clone();
            let source_revision = self.conflict_resolver_input.read_with(cx, |input, _| {
                ResolvedOutputSourceRevision::from_snapshot(&input.text_snapshot())
            });
            self.conflict_resolved_preview_source_revision = Some(source_revision);
            self.schedule_conflict_resolved_outline_recompute(
                output_path,
                source_revision,
                None,
                cx,
            );
        }
        if !preserved_materialized_output {
            self.rebuild_conflict_resolved_output_block_map(cx);
        }

        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_preserving_current();
        }
    }

    pub(in crate::view) fn request_conflict_file_load_mode(
        &mut self,
        mode: gitcomet_state::model::ConflictFileLoadMode,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let Some(path) = self.conflict_resolver.path.clone() else {
            return false;
        };
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        if repo.conflict_state.conflict_file_path.as_ref() != Some(&path) {
            return false;
        }
        if repo.conflict_state.conflict_file_load_mode == mode
            || matches!(repo.conflict_state.conflict_file, Loadable::Loading)
        {
            return false;
        }

        self.store.dispatch(Msg::LoadConflictFile {
            repo_id,
            path,
            mode,
        });
        true
    }
}
