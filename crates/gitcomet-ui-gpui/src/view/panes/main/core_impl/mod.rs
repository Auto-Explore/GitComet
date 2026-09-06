use super::helpers::*;
use super::*;
use crate::kit::text_model::TextModelSnapshot;
use gitcomet_core::domain::{Diff, FileDiffImage, FileDiffText, LogScope};
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceSideStats, MergetoolTraceStage,
};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::sync::Arc;
use std::time::Instant;

mod init;

pub(in crate::view) use init::MainPaneInit;

/// Resolve the file path and blame source for a diff target, or `None` for
/// targets that do not support blame annotation (e.g. whole-commit diffs with no
/// selected path).
///
/// Committed-file diffs blame the committed revision shown on the new side.
/// Working-tree diffs blame the displayed new-side content for their area (see
/// [`gitcomet_core::services::Repo::blame_worktree_file`]); lines not yet
/// committed are surfaced as "Not Committed Yet". In both cases blame is
/// computed against the exact content rendered on the new side, so the 1:1
/// `new_line` mapping in the annotation column stays correct.
fn blame_path_rev_for_target(
    target: &DiffTarget,
) -> Option<(std::path::PathBuf, gitcomet_core::domain::BlameSource)> {
    use gitcomet_core::domain::BlameSource;
    match target {
        DiffTarget::WorkingTree { path, area } => {
            Some((path.clone(), BlameSource::WorkingTree(*area)))
        }
        DiffTarget::Commit {
            commit_id,
            path: Some(path),
        } => Some((
            path.clone(),
            BlameSource::Revision(Some(commit_id.0.to_string())),
        )),
        DiffTarget::CommitRange {
            to_commit_id,
            path: Some(path),
            ..
        } => Some((
            path.clone(),
            match to_commit_id {
                Some(to_commit_id) => BlameSource::Revision(Some(to_commit_id.0.to_string())),
                // Working-tree tip: the new side is the worktree file.
                None => BlameSource::WorkingTree(gitcomet_core::domain::DiffArea::Unstaged),
            },
        )),
        _ => None,
    }
}

pub(super) fn uniform_list_base_handle(handle: &UniformListScrollHandle) -> ScrollHandle {
    handle.0.borrow().base_handle.clone()
}

impl MainPaneView {
    pub(in crate::view) fn sync_interactive_commit_editor_states(&mut self) {
        let repos_with_setup: Vec<RepoId> = self
            .state
            .repos
            .iter()
            .filter(|r| {
                r.interactive_rebase_setup.is_some() || r.interactive_cherry_pick_setup.is_some()
            })
            .map(|r| r.id)
            .collect();
        self.interactive_rebase_states
            .retain(|repo_id, _| repos_with_setup.contains(repo_id));
        for repo in self.state.repos.iter() {
            if let Some(setup) = repo.interactive_rebase_setup.as_ref() {
                let Loadable::Ready(entries) = &setup.entries else {
                    continue;
                };
                let replace = self
                    .interactive_rebase_states
                    .get(&repo.id)
                    .is_none_or(|st| {
                        st.mode != ICommitEditorMode::Rebase || st.original_entries != *entries
                    });
                if replace {
                    self.interactive_rebase_states.insert(
                        repo.id,
                        IRebaseViewState {
                            mode: ICommitEditorMode::Rebase,
                            entries: entries.clone(),
                            original_entries: entries.clone(),
                            ..Default::default()
                        },
                    );
                }
            } else if let Some(setup) = repo.interactive_cherry_pick_setup.as_ref() {
                if !matches!(setup.full_messages, Loadable::Ready(())) {
                    // Do not retain subject-only view-local entries from this
                    // or a replaced setup while full messages are pending.
                    self.interactive_rebase_states.remove(&repo.id);
                    continue;
                }
                let source_colors = setup
                    .source_colors
                    .iter()
                    .cloned()
                    .collect::<FxHashMap<_, _>>();
                // A repeated state application for the same setup must not
                // replace view-local reordering or action edits. A different
                // id set is a genuinely new setup.
                let same_plan =
                    self.interactive_rebase_states.get(&repo.id).is_some_and(
                        |st: &IRebaseViewState| {
                            st.mode == ICommitEditorMode::CherryPick
                                && st.original_entries.len() == setup.entries.len()
                                && st.original_entries.iter().zip(setup.entries.iter()).all(
                                    |(current, incoming)| current.commit_id == incoming.commit_id,
                                )
                        },
                    );
                if same_plan {
                    let st = self
                        .interactive_rebase_states
                        .get_mut(&repo.id)
                        .expect("same_plan implies the state exists");
                    for (current, incoming) in
                        st.original_entries.iter_mut().zip(setup.entries.iter())
                    {
                        current.message = incoming.message.clone();
                    }
                    for entry in st.entries.iter_mut() {
                        if let Some(incoming) = setup
                            .entries
                            .iter()
                            .find(|incoming| incoming.commit_id == entry.commit_id)
                        {
                            entry.message = incoming.message.clone();
                        }
                    }
                } else {
                    self.interactive_rebase_states.insert(
                        repo.id,
                        IRebaseViewState {
                            mode: ICommitEditorMode::CherryPick,
                            entries: setup.entries.clone(),
                            original_entries: setup.entries.clone(),
                            source_colors,
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }

    pub(super) fn notify_fingerprint_for(state: &AppState) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            match repo.diff_state.diff_target.as_ref() {
                Some(DiffTarget::WorkingTree { path, area }) => {
                    0u8.hash(&mut hasher);
                    path.hash(&mut hasher);
                    match area {
                        DiffArea::Staged => 0u8.hash(&mut hasher),
                        DiffArea::Unstaged => 1u8.hash(&mut hasher),
                    }
                }
                Some(DiffTarget::Commit { commit_id, path }) => {
                    1u8.hash(&mut hasher);
                    commit_id.hash(&mut hasher);
                    path.hash(&mut hasher);
                }
                Some(DiffTarget::CommitRange {
                    from_commit_id,
                    to_commit_id,
                    path,
                }) => {
                    2u8.hash(&mut hasher);
                    from_commit_id.hash(&mut hasher);
                    to_commit_id.hash(&mut hasher);
                    path.hash(&mut hasher);
                }
                None => {
                    3u8.hash(&mut hasher);
                }
            }
            repo.diff_state.diff_state_rev.hash(&mut hasher);
            // The historical-browse tint keys off content-preview mode, which can
            // share a diff_target with a plain diff of the same commit+path.
            repo.diff_state.content_preview.hash(&mut hasher);
            // Entering or leaving the editor swaps the whole content body and
            // the toolbar; without this the pane would not re-render for it.
            repo.diff_state.edit_mode.hash(&mut hasher);
            repo.conflict_state.conflict_rev.hash(&mut hasher);

            // Only include status changes when viewing a working tree diff.
            let status_rev = if matches!(
                repo.diff_state.diff_target,
                Some(DiffTarget::WorkingTree { .. })
            ) {
                repo.status_cache_rev()
            } else {
                0
            };
            status_rev.hash(&mut hasher);
            let commit_details_rev = if matches!(
                repo.diff_state.diff_target,
                Some(DiffTarget::Commit { path: Some(_), .. })
            ) {
                repo.history_state.commit_details_rev
            } else {
                0
            };
            commit_details_rev.hash(&mut hasher);
            // The historical-browse tint keys off the file browser source.
            repo.file_browser.file_browser_rev.hash(&mut hasher);

            match &repo.interactive_rebase_setup {
                Some(setup) => {
                    1u8.hash(&mut hasher);
                    setup.base.hash(&mut hasher);
                    match &setup.entries {
                        Loadable::NotLoaded => 0u8.hash(&mut hasher),
                        Loadable::Loading => 1u8.hash(&mut hasher),
                        Loadable::Ready(_) => 2u8.hash(&mut hasher),
                        Loadable::Error(err) => {
                            3u8.hash(&mut hasher);
                            err.hash(&mut hasher);
                        }
                    }
                }
                None => {
                    0u8.hash(&mut hasher);
                }
            }
            match &repo.interactive_cherry_pick_setup {
                Some(setup) => {
                    1u8.hash(&mut hasher);
                    setup.entries.len().hash(&mut hasher);
                    for entry in &setup.entries {
                        entry.commit_id.hash(&mut hasher);
                        entry.summary.hash(&mut hasher);
                    }
                    setup.source_colors.hash(&mut hasher);
                    match &setup.full_messages {
                        Loadable::NotLoaded => 0u8.hash(&mut hasher),
                        Loadable::Loading => 1u8.hash(&mut hasher),
                        Loadable::Ready(()) => 2u8.hash(&mut hasher),
                        Loadable::Error(error) => {
                            3u8.hash(&mut hasher);
                            error.hash(&mut hasher);
                        }
                    }
                }
                None => 0u8.hash(&mut hasher),
            }
            // Blame/annotate data — when blame loads for the first time or changes
            // target, the annotation sidebar needs to repaint.
            repo.history_state.blame_path.hash(&mut hasher);
            repo.history_state.blame_source.hash(&mut hasher);
            matches!(
                &repo.history_state.blame,
                gitcomet_state::model::Loadable::Ready(_)
            )
            .hash(&mut hasher);
        }

        hasher.finish()
    }

    pub(in crate::view) fn clear_diff_selection_or_exit(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        match clear_diff_selection_action(self.view_mode) {
            ClearDiffSelectionAction::ClearSelection => {
                self.store.dispatch(Msg::ClearDiffSelection { repo_id });
            }
            ClearDiffSelectionAction::ExitFocusedMergetool => {
                self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_CANCELED);
                cx.quit();
            }
        }
    }

    pub(in crate::view) fn reveal_history_commit(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(
            clear_diff_selection_action(self.view_mode),
            ClearDiffSelectionAction::ExitFocusedMergetool
        ) {
            self.clear_diff_selection_or_exit(repo_id, cx);
            return;
        }

        self.clear_diff_selection_or_exit(repo_id, cx);
        // Resolve and show the commit immediately; the history walk below only
        // has to find its row. Without this the details pane would sit on the
        // working tree — or flip in and out of it — for the whole walk.
        self.store.dispatch(Msg::RevealCommit {
            repo_id,
            reference: commit_id.clone(),
        });
        self.history_view.update(cx, |view, cx| {
            view.request_reveal_commit(repo_id, commit_id, fallback_scope, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn reveal_history_worktree(
        &mut self,
        repo_id: RepoId,
        worktree_path: std::path::PathBuf,
        is_current: bool,
        head: Option<CommitId>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.reveal_worktree(repo_id, worktree_path, is_current, head, cx);
        });
    }

    pub(in crate::view) fn reveal_history_branch_commit(
        &mut self,
        repo_id: RepoId,
        target: BranchMenuTarget,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_selected_branch(repo_id, target, cx);
        });
        self.reveal_history_commit(repo_id, commit_id, fallback_scope, cx);
    }

    pub(super) fn set_focused_mergetool_exit_code(&self, code: i32) {
        if let Some(exit_code) = &self.focused_mergetool_exit_code {
            exit_code.store(code, Ordering::SeqCst);
        }
    }

    pub(super) fn focused_mergetool_labels_or_default(&self) -> FocusedMergetoolLabels {
        self.focused_mergetool_labels
            .clone()
            .unwrap_or(FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            })
    }

    pub(in crate::view) fn focused_mergetool_save_and_exit(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        use gitcomet_core::conflict_output::ConflictMarkerLabels;

        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        };

        let labels = self.focused_mergetool_labels_or_default();
        let materialized_output = (!self.conflict_resolved_output_is_streamed()).then(|| {
            self.conflict_resolver_input
                .read_with(cx, |input, _| input.text().to_string())
        });
        let save_payload = build_focused_mergetool_save_payload(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            &self.conflict_resolved_output_block_map,
            materialized_output.as_deref(),
            ConflictMarkerLabels {
                local: labels.local.as_str(),
                remote: labels.remote.as_str(),
                base: labels.base.as_str(),
            },
        );
        if save_payload.total_conflicts != save_payload.resolved_conflicts
            || conflict_resolver::text_contains_conflict_markers(&save_payload.output)
        {
            cx.notify();
            return;
        }
        let output = save_payload.output;
        let exit_code = focused_mergetool_save_exit_code(
            save_payload.total_conflicts,
            save_payload.resolved_conflicts,
        );
        self.finish_focused_mergetool_output(
            &repo.spec.workdir,
            &path,
            FocusedMergetoolOutput::Write(output.as_bytes()),
            exit_code,
            cx,
        );
    }

    pub(in crate::view) fn focused_mergetool_write_side_and_exit(
        &self,
        repo_id: RepoId,
        path: &std::path::Path,
        bytes: &[u8],
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        };
        self.finish_focused_mergetool_output(
            &repo.spec.workdir,
            path,
            FocusedMergetoolOutput::Write(bytes),
            FOCUSED_MERGETOOL_EXIT_SUCCESS,
            cx,
        );
    }

    pub(in crate::view) fn focused_mergetool_delete_and_exit(
        &self,
        repo_id: RepoId,
        path: &std::path::Path,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            cx.quit();
            return;
        };
        self.finish_focused_mergetool_output(
            &repo.spec.workdir,
            path,
            FocusedMergetoolOutput::Delete,
            FOCUSED_MERGETOOL_EXIT_SUCCESS,
            cx,
        );
    }

    fn finish_focused_mergetool_output(
        &self,
        workdir: &std::path::Path,
        path: &std::path::Path,
        output: FocusedMergetoolOutput<'_>,
        success_exit_code: i32,
        cx: &mut gpui::Context<Self>,
    ) {
        match apply_focused_mergetool_output(workdir, path, output) {
            Ok(()) => self.set_focused_mergetool_exit_code(success_exit_code),
            Err(err) => {
                let operation = match output {
                    FocusedMergetoolOutput::Write(_) => "write merged output to",
                    FocusedMergetoolOutput::Delete => "delete merged output",
                };
                eprintln!("Failed to {operation} {}: {err}", path.display());
                self.set_focused_mergetool_exit_code(FOCUSED_MERGETOOL_EXIT_ERROR);
            }
        }
        cx.quit();
    }
}

impl MainPaneView {
    pub(in crate::view) fn sync_root_layout_snapshot(&mut self, cx: &mut gpui::Context<Self>) {
        let fallback_sidebar = self.layout_sidebar_render_width;
        let fallback_details = self.layout_details_render_width;
        let fallback_sidebar_collapsed = self.layout_sidebar_collapsed;
        let fallback_details_collapsed = self.layout_details_collapsed;

        let (sidebar_w, details_w, sidebar_collapsed, details_collapsed) = self
            .root_view
            .read_with(cx, |root, _cx| {
                (
                    root.sidebar_render_width,
                    root.details_render_width,
                    root.sidebar_collapsed,
                    root.details_collapsed,
                )
            })
            .unwrap_or((
                fallback_sidebar,
                fallback_details,
                fallback_sidebar_collapsed,
                fallback_details_collapsed,
            ));

        self.layout_sidebar_render_width = sidebar_w;
        self.layout_details_render_width = details_w;
        self.layout_sidebar_collapsed = sidebar_collapsed;
        self.layout_details_collapsed = details_collapsed;
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.conflict_resolved_output_provider_theme_epoch = self
            .conflict_resolved_output_provider_theme_epoch
            .wrapping_add(1)
            .max(1);
        self.file_editor_provider_theme_epoch =
            self.file_editor_provider_theme_epoch.wrapping_add(1).max(1);
        self.clear_diff_text_style_caches();
        self.clear_worktree_preview_segments_cache();
        self.clear_conflict_diff_style_caches();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.diff_raw_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.diff_search_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.conflict_resolver_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.file_editor_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.rebind_file_editor_highlight_provider(cx);
        if self.conflict_resolved_output_is_streamed() {
            self.conflict_resolved_preview_syntax_language = self
                .conflict_resolved_preview_path
                .as_ref()
                .and_then(rows::diff_syntax_language_for_path);
            self.conflict_resolved_output_measure_row = self
                .conflict_resolved_output_projection
                .as_ref()
                .map(conflict_resolver::ResolvedOutputProjection::widest_line_ix)
                .unwrap_or(0);
        } else {
            let output_snapshot = self
                .conflict_resolver_input
                .read_with(cx, |input, _| input.text_snapshot());
            self.conflict_resolved_preview_line_starts = output_snapshot.shared_line_starts();
            self.conflict_resolved_preview_line_count = output_snapshot.line_count().max(1);
            self.conflict_resolved_output_measure_row =
                resolved_output_measure_row(&output_snapshot);
            self.refresh_conflict_resolved_output_syntax(&output_snapshot, None, cx);
        }
        self.history_view
            .update(cx, |view, cx| view.set_theme(theme, cx));
        cx.notify();
    }

    pub(in crate::view) fn apply_ui_scale_percent(
        &mut self,
        previous_percent: u32,
        next_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_line_height(
                Some(ui_scale::design_px_from_percent(20.0, next_percent)),
                cx,
            );
        });
        // The editor's gutter sizes its rows from the same scale, so leaving
        // the buffer at the old line height would put the numbers out of step
        // with the code they label.
        self.file_editor_input.update(cx, |input, cx| {
            input.set_line_height(
                Some(ui_scale::design_px_from_percent(20.0, next_percent)),
                cx,
            );
        });
        self.history_view.update(cx, |view, cx| {
            view.apply_ui_scale_percent(previous_percent, next_percent, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn invalidate_font_metrics(&mut self, cx: &mut gpui::Context<Self>) {
        self.diff_text_hitboxes.clear();
        self.diff_text_motion_targets.clear();
        self.diff_stage_gutter_cells.clear();
        self.diff_text_layout_cache_epoch = self.diff_text_layout_cache_epoch.wrapping_add(1);
        self.diff_text_layout_cache.clear();
        cx.notify();
    }

    pub(in crate::view) fn reset_diff_horizontal_scroll_state(&mut self) {
        self.diff_horizontal_scroll.reset();
        // A reveal names a row in the view it was armed over. That view is gone,
        // so the request must go with it rather than fire against whatever row
        // now holds that index.
        self.diff_search_horizontal_reveal = None;
        self.markdown_preview_reveal.clear();
    }

    pub(in crate::view) fn diff_horizontal_content_width(&self) -> Pixels {
        self.diff_horizontal_content_width_for_column(DiffHorizontalScrollColumn::Primary)
    }

    pub(in crate::view) fn diff_horizontal_content_width_for_column(
        &self,
        column: DiffHorizontalScrollColumn,
    ) -> Pixels {
        self.diff_horizontal_scroll.content_widths[column.index()]
    }

    pub(in crate::view) fn diff_horizontal_layout_min_width(
        &self,
        column: DiffHorizontalScrollColumn,
    ) -> Pixels {
        self.diff_horizontal_content_width_for_column(column)
    }

    pub(in crate::view) fn record_diff_horizontal_content_width(
        &mut self,
        width: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        self.record_diff_horizontal_content_width_for_column(
            DiffHorizontalScrollColumn::Primary,
            width,
            cx,
        );
    }

    pub(in crate::view) fn record_diff_horizontal_content_width_for_column(
        &mut self,
        column: DiffHorizontalScrollColumn,
        width: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap {
            return;
        }

        if self
            .diff_horizontal_scroll
            .record_content_width(column, width)
        {
            cx.notify();
        }
    }

    pub(in crate::view) fn diff_vertical_scrollbar_gutter_for_column(
        &self,
        _column: DiffHorizontalScrollColumn,
        _handle: UniformListScrollHandle,
    ) -> Pixels {
        components::Scrollbar::gutter(components::ScrollbarAxis::Vertical)
    }

    #[cfg(test)]
    pub(in crate::view) fn diff_horizontal_scroll_max_offset_for_viewport(
        &self,
        column: DiffHorizontalScrollColumn,
        viewport_width: Pixels,
    ) -> Pixels {
        let viewport_width = viewport_width.max(px(0.0));
        let content_width = self.diff_horizontal_content_width_for_column(column);
        (content_width - viewport_width).max(px(0.0))
    }

    pub(in crate::view) fn conflict_resolved_output_is_streamed(&self) -> bool {
        self.conflict_resolved_output_projection.is_some()
    }

    pub(in crate::view) fn rebuild_conflict_resolved_output_block_map(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.output_is_protected {
            self.conflict_resolved_output_block_map =
                conflict_resolver::ResolvedOutputBlockMap::default();
            return;
        }
        let map = conflict_resolver::ResolvedOutputBlockMap::from_segments(
            &self.conflict_resolver.marker_segments,
        );
        if self.conflict_resolved_output_is_streamed()
            || self.conflict_resolver_input.read_with(cx, |input, _| {
                map.is_valid_for(&self.conflict_resolver.marker_segments, input.text())
            })
        {
            self.conflict_resolved_output_block_map = map;
        } else {
            self.conflict_resolved_output_block_map =
                conflict_resolver::ResolvedOutputBlockMap::default();
        }
    }

    pub(super) fn apply_conflict_resolved_output_edit_deltas(
        &mut self,
        edit_deltas: Vec<(Range<usize>, Range<usize>)>,
        output_text: &(impl conflict_resolver::ResolvedOutputSource + ?Sized),
    ) {
        if edit_deltas.is_empty() {
            return;
        }
        if !self
            .conflict_resolved_output_block_map
            .apply_edit_deltas(edit_deltas)
            || !self
                .conflict_resolved_output_block_map
                .is_valid_for(&self.conflict_resolver.marker_segments, output_text)
        {
            self.conflict_resolved_output_block_map =
                conflict_resolver::ResolvedOutputBlockMap::default();
        }
    }

    pub(in crate::view) fn conflict_resolved_output_is_modified(&self) -> bool {
        self.conflict_resolved_output_modified
    }

    pub(in crate::view) fn mark_conflict_resolved_output_saved(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolved_output_saved_snapshot =
            (!self.conflict_resolved_output_is_streamed()).then(|| {
                self.conflict_resolver_input
                    .read_with(cx, |input, _| input.text_snapshot())
            });
        self.conflict_resolved_output_modified = false;
    }
}

impl MainPaneView {
    fn sync_conflict_resolved_preview_projection(
        &mut self,
        projection: conflict_resolver::ResolvedOutputProjection,
        path: Option<&std::path::PathBuf>,
    ) {
        self.conflict_resolved_output_block_map =
            conflict_resolver::ResolvedOutputBlockMap::from_segments(
                &self.conflict_resolver.marker_segments,
            );
        self.conflict_resolved_output_projection = Some(projection.clone());
        self.conflict_resolved_preview_path = path.cloned();
        self.conflict_resolved_preview_source_revision = None;
        self.conflict_resolved_preview_text = TextModelSnapshot::default();
        self.conflict_resolved_preview_syntax_language =
            path.and_then(rows::diff_syntax_language_for_path);
        self.conflict_resolved_preview_line_count = projection.len();
        self.conflict_resolved_preview_line_starts = Arc::default();
        self.conflict_resolved_output_measure_row = projection.widest_line_ix();
        self.conflict_resolved_outline_stash = None;
        self.conflict_resolver.resolved_output_visible_dirty = true;
    }

    pub(in crate::view) fn refresh_streamed_resolved_output_preview_from_projection(
        &mut self,
        projection: conflict_resolver::ResolvedOutputProjection,
        path: Option<&std::path::PathBuf>,
    ) {
        let trace_started = Instant::now();
        let output_line_count = projection.len();
        let view_mode = self.conflict_resolver.view_mode;
        let computed = compute_resolved_outline_computation_from_projection(
            &projection,
            &self.conflict_resolver.marker_segments,
            view_mode,
            (!should_skip_resolved_outline_provenance(view_mode, output_line_count))
                .then(|| self.resolved_outline_source_view()),
        );
        self.sync_conflict_resolved_preview_projection(projection, path);
        self.apply_resolved_outline_computation(path, trace_started, computed);
    }

    pub(in crate::view) fn refresh_streamed_resolved_output_preview_from_markers(
        &mut self,
        path: Option<&std::path::PathBuf>,
    ) {
        let projection = conflict_resolver::ResolvedOutputProjection::from_segments(
            &self.conflict_resolver.marker_segments,
        );
        self.refresh_streamed_resolved_output_preview_from_projection(projection, path);
    }

    pub(in crate::view) fn ensure_conflict_resolved_output_materialized(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.conflict_resolved_output_is_streamed() {
            return;
        }

        // No size ceiling here. The output pane is editable by definition, and a
        // read-only fallback above some line count is a worse answer than a
        // slower one: the user opened a merge to resolve it. The buffer is
        // rope-backed and every hot path (syntax refresh, unresolved-row scan,
        // shaping) reads windows rather than the whole document, so cost scales
        // with the visible region plus the conflict count, not the file.
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        let path = self.conflict_resolver.path.clone();
        self.conflict_resolved_output_projection = None;
        self.conflict_resolved_preview_path = path.clone();
        self.fill_conflict_resolved_output_buffer(resolved, cx);
        self.conflict_resolved_preview_source_revision =
            Some(self.conflict_resolver_input.read_with(cx, |input, _| {
                ResolvedOutputSourceRevision::from_snapshot(&input.text_snapshot())
            }));
        self.rebuild_conflict_resolved_output_block_map(cx);
        self.recompute_conflict_resolved_outline_and_provenance(path.as_ref(), cx);
    }

    /// Load merged text into the resolved-output editor.
    ///
    /// Every path that fills this buffer goes through here, because filling it
    /// has one non-obvious obligation: `set_text` leaves the caret at
    /// end-of-document, and the pane opens scrolled to the top, so a caret
    /// parked at the far end sends the first arrow key (or undo) autoscrolling
    /// to the bottom of a file the user was reading from the top. Park it where
    /// the view actually is; the user has not placed a caret yet.
    pub(in crate::view) fn fill_conflict_resolved_output_buffer(
        &mut self,
        text: impl Into<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        let text = text.into();
        let line_ending = crate::kit::TextInput::detect_line_ending(text.as_ref());
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(text, cx);
            input.set_selected_range(0..0, false, cx);
        });
    }

    /// Configure the resolved-output `TextInput` for rendering as the editable
    /// output pane. This is called from the render path, so it must stay cheap
    /// and side-effect free: the merged text is materialized into the buffer at
    /// bootstrap (see [`ensure_conflict_resolved_output_materialized`]), not here.
    /// It only points the editor at its shared scroll handle so the line-number
    /// gutter and the column scroll-sync group stay coupled to it.
    pub(in crate::view) fn prepare_conflict_resolved_output_editor(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.sync_conflict_resolved_output_active_conflict_highlight(cx);
        let scroll = self.conflict_resolved_output_editor_scroll.clone();
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_read_only(false, cx);
            input.set_vertical_scroll_handle(Some(scroll));
            // Lay the editor out at content width so its `overflow_scroll`
            // container carries a horizontal `max_offset` on the shared handle,
            // letting the resolved output scroll-sync with the source columns
            // on the horizontal axis too.
            input.set_content_width_layout(true);
        });
    }
}

impl MainPaneView {
    pub(in crate::view) fn clear_diff_text_query_overlay_cache(&mut self) {
        self.diff_text_query_segments_cache.clear();
        self.diff_text_query_cache_query = SharedString::default();
        self.diff_text_query_cache_options = Default::default();
        self.diff_text_query_cache_matcher_shared = None;
        self.diff_text_query_cache_generation =
            self.diff_text_query_cache_generation.wrapping_add(1);
    }

    pub(in crate::view) fn invalidate_diff_text_query_overlay_cache(
        &mut self,
        query: &str,
        options: super::diff_search::DiffSearchOptions,
    ) {
        if self.diff_text_query_cache_query.as_ref() != query
            || self.diff_text_query_cache_options != options
        {
            self.diff_text_query_cache_query = query.to_string().into();
            self.diff_text_query_cache_options = options;
            self.diff_text_query_cache_matcher_shared = (!query.is_empty())
                .then(|| Arc::new(super::diff_search::DiffSearchMatcher::new(query, options)));
            self.diff_text_query_cache_generation =
                self.diff_text_query_cache_generation.wrapping_add(1);
        }
    }

    pub(in crate::view) fn sync_diff_text_query_overlay_cache(
        &mut self,
        query: &str,
        options: super::diff_search::DiffSearchOptions,
    ) {
        self.invalidate_diff_text_query_overlay_cache(query, options);
    }

    pub(in crate::view) fn clear_diff_text_style_caches(&mut self) {
        self.diff_text_segments_cache.clear();
        self.clear_diff_text_query_overlay_cache();
    }

    pub(in crate::view) fn clear_worktree_preview_segments_cache(&mut self) {
        self.worktree_preview_segments_cache.clear();
        self.worktree_preview_cache_write_blocked_until_rev = None;
    }

    pub(in crate::view) fn clear_conflict_diff_query_overlay_caches(&mut self) {
        self.conflict_diff_query_segments_cache_split.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.conflict_diff_query_cache_query = SharedString::default();
        self.conflict_diff_query_cache_options = Default::default();
    }

    pub(in crate::view) fn clear_conflict_diff_style_caches_preserving_query(&mut self) {
        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_query_segments_cache_split.clear();
        self.conflict_three_way_query_segments_cache.clear();
    }

    pub(in crate::view) fn sync_conflict_diff_query_overlay_caches(
        &mut self,
        query: &str,
        options: super::diff_search::DiffSearchOptions,
    ) {
        if self.conflict_diff_query_cache_query.as_ref() != query
            || self.conflict_diff_query_cache_options != options
        {
            self.conflict_diff_query_cache_query = query.to_string().into();
            self.conflict_diff_query_cache_options = options;
            self.conflict_diff_query_segments_cache_split.clear();
            self.conflict_three_way_query_segments_cache.clear();
        }
    }

    pub(in crate::view) fn clear_conflict_diff_style_caches(&mut self) {
        self.clear_conflict_diff_style_caches_preserving_query();
        self.conflict_diff_query_cache_query = SharedString::default();
        self.conflict_diff_query_cache_options = Default::default();
    }

    pub(super) fn conflict_resolver_invalidate_resolved_outline(&mut self) {
        self.conflict_resolver.resolver_pending_recompute_seq = self
            .conflict_resolver
            .resolver_pending_recompute_seq
            .wrapping_add(1);
        self.conflict_resolved_preview_path = None;
        self.conflict_resolved_preview_source_revision = None;
        self.conflict_resolved_output_projection = None;
        self.conflict_resolved_preview_text = TextModelSnapshot::default();
        self.conflict_resolved_preview_syntax_language = None;
        self.conflict_resolved_preview_line_count = 0;
        self.conflict_resolved_preview_line_starts = Arc::default();
        self.conflict_resolved_output_measure_row = 0;
        self.conflict_resolved_outline_stash = None;
        self.conflict_three_way_prepared_syntax_documents = ThreeWaySides::default();
        self.conflict_three_way_syntax_inflight = ThreeWaySides::default();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.conflict_resolver.resolved_outline = ResolvedOutlineData::default();
        self.conflict_resolver.resolved_output_visible_dirty = true;
    }
}

impl MainPaneView {
    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();
        self.history_view.update(cx, |view, cx| {
            view.set_active_context_menu_invoker(next, cx)
        });
        cx.notify();
    }

    pub(in crate::view) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.history_view
            .update(cx, |view, cx| view.set_date_time_format(next, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_history_highlight_commit_chain(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_highlight_commit_chain(enabled, cx)
        });
        cx.notify();
    }

    pub(in crate::view) fn history_highlight_commit_chain(&self, cx: &App) -> bool {
        self.history_view.read(cx).history_highlight_commit_chain
    }

    pub(in crate::view) fn set_history_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_history_relative_dates(enabled, cx));
        cx.notify();
    }

    pub(in crate::view) fn history_relative_dates(&self, cx: &App) -> bool {
        self.history_view.read(cx).history_relative_dates
    }

    pub(in crate::view) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        self.history_view
            .update(cx, |view, cx| view.set_timezone(next, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_show_timezone(enabled, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_diff_scroll_sync(
        &mut self,
        next: DiffScrollSync,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_scroll_sync == next {
            return;
        }

        self.diff_scroll_sync = next;
        self.sync_diff_split_scroll();
        self.sync_conflict_preview_scroll();
        cx.notify();
    }

    pub(in crate::view) fn set_diff_view_mode(
        &mut self,
        next: DiffViewMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_view == next {
            return;
        }

        self.diff_view = next;
        // Inline keys styled segments by `row_ix` while split keys them by
        // `row_ix * 2` / `row_ix * 2 + 1` (`file_diff_split_cache_key`) against
        // the same `split_left`/`split_right` epochs, so the two key spaces
        // alias. Clear on every mode change, not just the toolbar/hotkey ones.
        self.clear_diff_text_style_caches();
        self.clear_diff_text_projected_highlights();
        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_preserving_current();
        }
        cx.notify();
    }

    pub(in crate::view) fn set_annotate_enabled(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.annotate_enabled == next {
            return;
        }

        self.annotate_enabled = next;
        // The annotation column changes the available text width, so word-wrap
        // column counts and wrapped-row projection must be recomputed.
        self.invalidate_diff_wrap_visible_cache();
        if next {
            // An explicit toggle on: retry a previously failed blame for the same
            // target (force = true). The per-frame Render path never forces.
            self.request_blame_for_current_target(true, cx);
        }
        cx.notify();
    }

    /// Scaled pixel width of the annotation column at the current ui scale.
    pub(in crate::view) fn annotate_column_width_px(&self, ui_scale_percent: u32) -> Pixels {
        crate::ui_scale::design_px_from_percent(self.annotate_column_width, ui_scale_percent)
    }

    /// Whether the annotation column should be shown for the currently rendered
    /// diff target. Requires the user toggle to be on AND the target to support
    /// blame (committed-file and working-tree views — see
    /// [`blame_path_rev_for_target`]).
    pub(in crate::view) fn annotation_active(&self) -> bool {
        self.annotate_enabled
            && self
                .rendered_diff_target()
                .and_then(blame_path_rev_for_target)
                .is_some()
    }

    /// Whether the loaded (or retained) blame describes the diff target being
    /// rendered right now. `blame_path`/`blame_source` follow the store snapshot,
    /// which lags the dispatch by at least a frame, so just after a file switch
    /// they still name the previous file — its annotations must not be painted
    /// over the new one's rows.
    pub(in crate::view) fn blame_matches_rendered_target(&self) -> bool {
        let Some((path, source)) = self
            .rendered_diff_target()
            .and_then(blame_path_rev_for_target)
        else {
            return false;
        };
        self.active_repo().is_some_and(|repo| {
            repo.history_state.blame_path.as_deref() == Some(path.as_path())
                && repo.history_state.blame_source.as_ref() == Some(&source)
        })
    }
}

impl MainPaneView {
    /// When annotate is on, ensure blame for the currently displayed file/rev is
    /// loaded. Derives the path and revision from the rendered diff target and
    /// dispatches `LoadBlame`, skipping redundant loads.
    pub(in crate::view) fn request_blame_for_current_target(
        &mut self,
        force: bool,
        _cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some((path, source)) = self
            .rendered_diff_target()
            .and_then(blame_path_rev_for_target)
        else {
            return;
        };

        if let Some(repo) = self.active_repo() {
            let history = &repo.history_state;
            let same_target = history.blame_path.as_deref() == Some(path.as_path())
                && history.blame_source.as_ref() == Some(&source);
            if !should_request_blame(same_target, &history.blame, force) {
                return;
            }
        }

        self.store.dispatch(Msg::LoadBlame {
            repo_id,
            path,
            source,
        });
    }

    pub(in crate::view) fn set_diff_content_mode(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_content_mode == next {
            return;
        }

        self.diff_content_mode = next;
        self.diff_selection_anchor = None;
        self.diff_selection_range = None;
        self.clear_diff_text_style_caches();
        self.clear_diff_text_query_overlay_cache();
        self.clear_conflict_diff_style_caches();
        self.clear_conflict_diff_query_overlay_caches();
        self.clear_worktree_preview_segments_cache();
        self.reset_collapsed_diff_projection(false);
        self.ensure_rendered_patch_diff_cache(cx);
        if self.current_main_diff_supports_diff_content_toggle() {
            self.ensure_file_diff_cache(cx);
        }
        if self.current_main_diff_wants_file_diff() {
            self.ensure_file_image_diff_cache(cx);
        }
        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_preserving_current();
        }
        cx.notify();
    }

    pub(in crate::view) fn set_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_whitespace_mode == next {
            return;
        }

        self.diff_whitespace_mode = next;
        self.diff_selection_anchor = None;
        self.diff_selection_range = None;
        self.rebuild_patch_visual_line_kinds_from_current_diff();
        self.diff_word_highlights.clear();
        self.diff_word_highlights_inflight = None;
        self.reset_file_diff_word_highlight_caches();
        self.clear_diff_text_style_caches();
        self.clear_diff_text_query_overlay_cache();
        self.clear_conflict_diff_style_caches();
        self.clear_conflict_diff_query_overlay_caches();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.clear_worktree_preview_segments_cache();
        self.reset_collapsed_diff_projection(false);
        self.diff_visible_cache_len = 0;
        self.diff_visible_cache_projection_rev = u64::MAX;
        self.diff_scrollbar_markers_cache.clear();
        if self.current_main_diff_supports_diff_content_toggle() {
            self.reset_file_diff_cache_data();
            self.ensure_file_diff_cache(cx);
        }
        if self.diff_search_active && !self.diff_search_query.is_empty() {
            self.diff_search_recompute_matches_preserving_current();
        }
        cx.notify();
    }

    pub(in crate::view) fn set_diff_reveal_whitespace_chars(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.reveal_whitespace_chars == next {
            return;
        }

        self.reveal_whitespace_chars = next;
        self.clear_diff_text_style_caches();
        self.clear_conflict_diff_style_caches();
        self.conflict_three_way_segments_cache.clear();
        self.conflict_three_way_query_segments_cache.clear();
        self.diff_wrap_visible_cache_key = None;
        self.diff_wrap_visible_rows.clear();
        cx.notify();
    }

    pub(in crate::view) fn set_diff_word_wrap(&mut self, next: bool, cx: &mut gpui::Context<Self>) {
        if self.diff_word_wrap == next {
            return;
        }

        self.diff_word_wrap = next;
        self.diff_wrap_visible_cache_key = None;
        self.diff_wrap_visible_rows.clear();
        self.reset_diff_horizontal_scroll_state();
        cx.notify();
    }

    pub(in crate::view) fn set_diff_show_line_numbers(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_show_line_numbers == next {
            return;
        }

        self.diff_show_line_numbers = next;
        self.diff_wrap_visible_cache_key = None;
        self.reset_diff_horizontal_scroll_state();
        cx.notify();
    }

    pub(in crate::view) fn set_remote_markdown_image_policy(
        &mut self,
        next: RemoteMarkdownImagePolicy,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.remote_markdown_image_policy == next {
            return;
        }
        self.remote_markdown_image_policy = next;
        self.approved_remote_markdown_image_urls = Arc::default();
        self.remote_markdown_image_approval_revision =
            self.remote_markdown_image_approval_revision.wrapping_add(1);
        cx.notify();
    }

    pub(in crate::view) fn markdown_remote_image_access(
        &self,
        approval_view: Option<Entity<MainPaneView>>,
    ) -> rows::MarkdownRemoteImageAccess {
        rows::MarkdownRemoteImageAccess {
            policy: self.remote_markdown_image_policy,
            approved_urls: Arc::clone(&self.approved_remote_markdown_image_urls),
            approval_view,
        }
    }

    pub(in crate::view) fn approve_remote_markdown_image(
        &mut self,
        url: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.remote_markdown_image_policy != RemoteMarkdownImagePolicy::AskBeforeLoading {
            return;
        }
        if Arc::make_mut(&mut self.approved_remote_markdown_image_urls).insert(url) {
            self.remote_markdown_image_approval_revision =
                self.remote_markdown_image_approval_revision.wrapping_add(1);
            cx.notify();
        }
    }

    pub(in crate::view) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in crate::view) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in crate::view) fn active_inline_submodule_diff(
        &self,
    ) -> Option<&gitcomet_state::model::InlineSubmoduleDiffState> {
        self.active_repo()?
            .diff_state
            .inline_submodule_diff
            .as_ref()
    }

    pub(in crate::view) fn selected_inline_submodule_diff_entry(
        &self,
    ) -> Option<&gitcomet_state::model::InlineSubmoduleDiffEntry> {
        let inline = self.active_inline_submodule_diff()?;
        inline.entries.get(inline.selected_ix)
    }

    pub(in crate::view) fn is_inline_submodule_diff_active(&self) -> bool {
        self.active_inline_submodule_diff().is_some()
    }

    pub(in crate::view) fn rendered_diff_target(&self) -> Option<&DiffTarget> {
        self.active_inline_submodule_diff()
            .map(|inline| &inline.target)
            .or_else(|| self.active_repo()?.diff_state.diff_target.as_ref())
    }

    /// Whether the content pane is showing a file's full content *at the commit
    /// the file browser is pinned to*, i.e. whether it earns the historical
    /// browse tint. See [`historical_browse_content`].
    pub(in crate::view) fn historical_browse_content_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        historical_browse_content(repo, self.rendered_diff_target())
    }

    pub(in crate::view) fn rendered_patch_diff_loadable(
        &self,
    ) -> Option<&gitcomet_state::model::Loadable<gitcomet_state::model::Shared<Diff>>> {
        if let Some(inline) = self.active_inline_submodule_diff() {
            Some(&inline.diff)
        } else {
            self.active_repo().map(|repo| &repo.diff_state.diff)
        }
    }

    pub(in crate::view) fn rendered_patch_diff_rev(&self) -> u64 {
        self.active_inline_submodule_diff()
            .map(|inline| inline.diff_rev)
            .or_else(|| self.active_repo().map(|repo| repo.diff_state.diff_rev))
            .unwrap_or(0)
    }
}

impl MainPaneView {
    pub(in crate::view) fn history_visible_column_preferences(
        &self,
        cx: &gpui::App,
    ) -> (bool, bool, bool, bool) {
        self.history_view
            .read(cx)
            .history_visible_column_preferences()
    }

    /// Persisted merge tool preferences: (auto-advance, collapse-unchanged
    /// default, output scroll sync, show line numbers). Read by the root view's
    /// UI settings persist.
    pub(in crate::view) fn mergetool_preferences(&self) -> (bool, bool, bool, bool) {
        (
            self.mergetool_auto_advance,
            self.mergetool_collapse_unchanged,
            self.mergetool_output_scroll_sync,
            self.mergetool_show_line_numbers,
        )
    }

    pub(in crate::view) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.schedule_ui_settings_persist(cx);
        });
    }

    pub(in crate::view) fn set_mergetool_auto_advance_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.mergetool_auto_advance == next {
            return;
        }
        self.mergetool_auto_advance = next;
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_mergetool_output_scroll_sync_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.mergetool_output_scroll_sync == next {
            return;
        }
        self.mergetool_output_scroll_sync = next;
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_mergetool_view_three_way_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.mergetool_view_three_way == next {
            return;
        }
        self.mergetool_view_three_way = next;
        // Unlike the cog-menu setters this can run while the root view is
        // already being updated (view-mode toggles), so schedule the persist
        // after the current update flush.
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.schedule_ui_settings_persist(cx);
            });
        });
        cx.notify();
    }

    pub(in crate::view) fn set_mergetool_show_line_numbers_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.mergetool_show_line_numbers == next {
            return;
        }
        self.mergetool_show_line_numbers = next;
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    pub(in crate::view) fn history_tag_preferences(&self, cx: &gpui::App) -> (bool, bool) {
        self.history_view.read(cx).history_tag_preferences()
    }

    pub(in crate::view) fn set_history_column_preferences(
        &mut self,
        show_graph: bool,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_column_preferences(show_graph, show_author, show_date, show_sha, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn set_history_tag_preferences(
        &mut self,
        show_tags: bool,
        auto_fetch_tags_on_repo_activation: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_tag_preferences(show_tags, auto_fetch_tags_on_repo_activation, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn reset_history_column_widths(&mut self, cx: &mut gpui::Context<Self>) {
        self.history_view.update(cx, |view, cx| {
            view.reset_history_column_widths();
            cx.notify();
        });
        cx.notify();
    }
}

impl MainPaneView {
    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    pub(in crate::view) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
                });
            });
        });
    }

    pub(in crate::view) fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn open_conflict_resolver_input_row_context_menu(
        &mut self,
        invoker: SharedString,
        line_label: SharedString,
        line_target: ResolverPickTarget,
        chunk_label: SharedString,
        chunk_target: ResolverPickTarget,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.activate_context_menu_invoker(invoker, cx);
        self.open_popover_at(
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            },
            anchor,
            window,
            cx,
        );
    }
}

impl MainPaneView {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn open_conflict_resolver_chunk_context_menu(
        &mut self,
        invoker: SharedString,
        conflict_ix: usize,
        has_base: bool,
        is_three_way: bool,
        selected_choices: Vec<conflict_resolver::ConflictChoice>,
        output_line_ix: Option<usize>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.activate_context_menu_invoker(invoker, cx);
        // Opening the chunk menu selects that conflict and brings the
        // *other* pane to it — the pane the user right-clicked is already in
        // view and must not jump under the open menu. Reveals are non-strict:
        // nothing scrolls when the target rows are already fully visible.
        self.conflict_resolver_select_conflict(conflict_ix, cx);
        if output_line_ix.is_some() {
            // Invoked from the resolved output: reveal the source columns.
            if let Some(vi) = self.conflict_resolver_visible_ix_for_conflict(conflict_ix) {
                self.conflict_resolver_reveal_all_columns(vi);
            }
        } else {
            // Invoked from a source column: reveal the resolved output chunk.
            let output_text = (!self.conflict_resolved_output_is_streamed()).then(|| {
                self.conflict_resolver_input
                    .read_with(cx, |input, _| input.text().to_string())
            });
            let line_count = output_text
                .as_ref()
                .map(|text| text.split('\n').count().max(1))
                .unwrap_or_else(|| self.conflict_resolved_preview_line_count.max(1));
            if let Some(line) = self.conflict_resolver_output_line_for_conflict(
                conflict_ix,
                output_text.as_deref().unwrap_or(""),
            ) {
                self.conflict_resolver_reveal_resolved_output_line(line, line_count);
            }
        }
        let split_selection_rows = self.conflict_resolver_split_selection_row_count(conflict_ix);
        let (join_previous_region, join_next_region) =
            self.conflict_resolver_join_region_targets(conflict_ix);
        self.open_popover_at(
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
                split_selection_rows,
                join_previous_region,
                join_next_region,
                alignment_marked_columns: self.conflict_resolver_alignment_marked_columns(),
                has_manual_alignments: self.conflict_resolver_has_manual_alignments(),
                output_is_protected: self.conflict_resolver.output_is_protected,
            },
            anchor,
            window,
            cx,
        );
    }

    pub(in crate::view) fn conflict_resolver_selected_choices_for_conflict_ix(
        &self,
        conflict_ix: usize,
    ) -> Vec<conflict_resolver::ConflictChoice> {
        conflict_group_selected_choices_for_ix(
            &self.conflict_resolver.marker_segments,
            &self.conflict_resolver.conflict_region_indices,
            conflict_ix,
        )
    }

    pub(in crate::view) fn conflict_resolver_has_base_for_conflict_ix(
        &self,
        conflict_ix: usize,
    ) -> bool {
        self.conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
                _ => None,
            })
            .nth(conflict_ix)
            .unwrap_or(false)
    }

    pub(in crate::view) fn conflict_resolver_split_selection_row_count(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        let selection = self.conflict_resolver.row_selection?;
        if selection.selecting || selection.conflict_ix != conflict_ix {
            return None;
        }
        self.conflict_resolver.split_boundaries_for_selection()?;
        Some(selection.row_range().count())
    }

    fn conflict_resolver_join_region_targets(
        &self,
        conflict_ix: usize,
    ) -> (
        Option<ConflictResolverJoinTarget>,
        Option<ConflictResolverJoinTarget>,
    ) {
        let Some(region_index) = self
            .conflict_resolver
            .conflict_region_indices
            .get(conflict_ix)
            .copied()
        else {
            return (None, None);
        };
        if self
            .conflict_resolver
            .conflict_region_indices
            .iter()
            .filter(|&&index| index == region_index)
            .take(2)
            .count()
            != 1
        {
            return (None, None);
        }
        let Some(repo_id) = self
            .conflict_resolver
            .repo_id
            .or_else(|| self.active_repo_id())
        else {
            return (None, None);
        };
        let Some(path) = self.conflict_resolver.dispatch_path() else {
            return (None, None);
        };
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return (None, None);
        };
        if repo.conflict_state.conflict_rev != self.conflict_resolver.conflict_rev {
            return (None, None);
        }
        let Some(session) = repo.conflict_state.conflict_session.as_ref() else {
            return (None, None);
        };
        if session.path != path.as_path()
            || session.strategy
                != gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver
            || region_index >= session.regions.len()
        {
            return (None, None);
        }

        let target = |first_region_index| ConflictResolverJoinTarget {
            repo_id,
            path: path.clone(),
            conflict_rev: repo.conflict_state.conflict_rev,
            first_region_index,
        };
        let visible_ix_for_unique_region = |wanted: usize| {
            let mut matches = self
                .conflict_resolver
                .conflict_region_indices
                .iter()
                .enumerate()
                .filter_map(|(ix, &region)| (region == wanted).then_some(ix));
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        };
        let previous = region_index.checked_sub(1).and_then(|previous_region| {
            let previous_ix = visible_ix_for_unique_region(previous_region)?;
            (previous_ix.checked_add(1) == Some(conflict_ix)
                && self
                    .conflict_resolver
                    .conflict_blocks_have_joinable_context(previous_ix, conflict_ix))
            .then(|| target(previous_region))
        });
        let next = region_index.checked_add(1).and_then(|next_region| {
            if next_region >= session.regions.len() {
                return None;
            }
            let next_ix = visible_ix_for_unique_region(next_region)?;
            (conflict_ix.checked_add(1) == Some(next_ix)
                && self
                    .conflict_resolver
                    .conflict_blocks_have_joinable_context(conflict_ix, next_ix))
            .then(|| target(region_index))
        });
        (previous, next)
    }

    pub(in crate::view) fn open_conflict_resolver_output_context_menu(
        &mut self,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (selected_text, cursor_offset, clicked_offset, content) =
            self.conflict_resolver_input.read_with(cx, |i, _| {
                (
                    i.selected_text(),
                    i.cursor_offset(),
                    i.offset_for_position(anchor),
                    i.text().to_string(),
                )
            });
        let context_line =
            conflict_resolver_output_context_line(&content, cursor_offset, Some(clicked_offset));

        self.open_conflict_resolver_output_context_menu_at_line(
            context_line,
            selected_text,
            content,
            anchor,
            window,
            cx,
        );
    }

    pub(in crate::view) fn open_conflict_resolver_output_context_menu_for_line(
        &mut self,
        line_ix: usize,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolved_output_is_streamed() {
            let context_line =
                line_ix.min(self.conflict_resolved_preview_line_count.saturating_sub(1));
            self.open_conflict_resolver_output_context_menu_at_line(
                context_line,
                None,
                String::new(),
                anchor,
                window,
                cx,
            );
            return;
        }

        let content = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let context_line = line_ix.min(self.conflict_resolved_preview_line_count.saturating_sub(1));
        let cursor_offset = line_start_offset_for_index(
            self.conflict_resolved_preview_line_starts.as_ref(),
            content.len(),
            context_line,
        );
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_cursor_offset(cursor_offset, cx);
        });

        self.open_conflict_resolver_output_context_menu_at_line(
            context_line,
            None,
            content,
            anchor,
            window,
            cx,
        );
    }

    fn open_conflict_resolver_output_context_menu_at_line(
        &mut self,
        context_line: usize,
        selected_text: Option<String>,
        content: String,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let conflict_marker = if self.conflict_resolved_output_is_streamed() {
            self.conflict_resolver
                .resolved_outline
                .markers
                .get(context_line)
                .copied()
                .flatten()
        } else {
            resolved_output_marker_for_line(
                &self.conflict_resolver.marker_segments,
                &content,
                context_line,
                &self.conflict_resolved_output_block_map,
            )
        };
        if let Some(marker) = conflict_marker {
            let is_three_way = self.conflict_resolver.view_mode
                == conflict_resolver::ConflictResolverViewMode::ThreeWay;
            let selected_choices =
                self.conflict_resolver_selected_choices_for_conflict_ix(marker.conflict_ix);
            let has_base = self.conflict_resolver_has_base_for_conflict_ix(marker.conflict_ix);
            let invoker: SharedString = format!(
                "resolver_output_chunk_menu_{}_{}",
                marker.conflict_ix, context_line
            )
            .into();
            self.open_conflict_resolver_chunk_context_menu(
                invoker,
                marker.conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                Some(context_line),
                anchor,
                window,
                cx,
            );
            return;
        }

        let is_three_way = self.conflict_resolver.view_mode
            == conflict_resolver::ConflictResolverViewMode::ThreeWay;

        let (has_source_a, has_source_b, has_source_c) = if is_three_way {
            (
                self.conflict_resolver
                    .three_way_has_line(ThreeWayColumn::Base, context_line),
                self.conflict_resolver
                    .three_way_has_line(ThreeWayColumn::Ours, context_line),
                self.conflict_resolver
                    .three_way_has_line(ThreeWayColumn::Theirs, context_line),
            )
        } else {
            {
                let row = self
                    .conflict_resolver
                    .two_way_split_row_by_source(context_line);
                (
                    row.as_ref().and_then(|r| r.old.as_ref()).is_some(),
                    row.as_ref().and_then(|r| r.new.as_ref()).is_some(),
                    false,
                )
            }
        };

        self.open_popover_at(
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line: context_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            },
            anchor,
            window,
            cx,
        );
    }
}

impl MainPaneView {
    /// Paths a stage/unstage shortcut should act on when the file it targets is
    /// part of a multi-file status selection: the whole selection, resolved the
    /// same way the status row button and the context menu resolve it. `None`
    /// means there is no such selection and the caller keeps acting on the one
    /// file it already resolved.
    ///
    /// Reads only. The shortcut may still raise a confirmation the user cancels,
    /// so [`Self::clear_status_selection_for_shortcut`] is a separate step the
    /// caller owes once it commits to the action.
    pub(in crate::view) fn status_selection_for_shortcut(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        path: &std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) -> Option<Vec<std::path::PathBuf>> {
        self.root_view
            .update(cx, |root, cx| {
                let (paths, used_selection) = root
                    .details_pane
                    .read(cx)
                    .status_selected_paths_for_action(repo_id, area, path);
                used_selection.then_some(paths)
            })
            .ok()
            .flatten()
    }

    /// Drop the row selection a shortcut has just acted on.
    pub(in crate::view) fn clear_status_selection_for_shortcut(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane.update(cx, |pane, cx| {
                pane.clear_status_multi_selection(repo_id);
                cx.notify();
            });
        });
    }

    /// Raise the unresolved-conflict confirmation if staging `paths` would mark
    /// files resolved while they still contain conflict markers. Returns whether
    /// the dialog took over, in which case the caller must not stage: the dialog
    /// dispatches it if the user goes ahead. Unstaging never marks anything
    /// resolved, so it is left alone.
    pub(in crate::view) fn confirm_stage_conflict_markers(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        paths: Vec<std::path::PathBuf>,
        clear_selection: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if area != DiffArea::Unstaged {
            return false;
        }
        let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
            &self.state,
            repo_id,
            paths,
            clear_selection,
        ) else {
            return false;
        };
        let anchor = crate::view::conflict_markers::centered_dialog_anchor(window);
        self.open_popover_at(confirm, anchor, window, cx);
        cx.notify();
        true
    }

    /// Stage (or unstage) a whole status selection in one batch, clearing the
    /// diff selection first because every one of those files is about to move to
    /// the other section. Same order the context menu uses.
    pub(in crate::view) fn stage_or_unstage_status_paths(
        &mut self,
        repo_id: RepoId,
        area: DiffArea,
        paths: Vec<std::path::PathBuf>,
    ) {
        self.store.dispatch(Msg::ClearDiffSelection { repo_id });
        let paths = paths.into();
        self.store.dispatch(match area {
            DiffArea::Unstaged => Msg::StagePaths { repo_id, paths },
            DiffArea::Staged => Msg::UnstagePaths { repo_id, paths },
        });
    }

    pub(in crate::view) fn clear_status_multi_selection(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.remove(&repo_id);
                cx.notify();
            });
        });
    }

    pub(in crate::view) fn open_submodule_inner_diff(
        &mut self,
        submodule_repo_path: std::path::PathBuf,
        target: DiffTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.submodule_diff_bootstrap =
                Some(SubmoduleDiffBootstrap::new(submodule_repo_path, target));
            root.drive_submodule_diff_bootstrap();
            cx.notify();
        });
    }

    pub(in crate::view) fn active_change_tracking_view(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> ChangeTrackingView {
        self.root_view
            .update(cx, |root, _cx| root.change_tracking_view)
            .unwrap_or(ChangeTrackingView::Combined)
    }

    pub(in crate::view) fn scroll_status_section_to_ix(
        &mut self,
        section: StatusSection,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane
                .update(cx, |pane: &mut DetailsPaneView, cx| {
                    match section {
                        StatusSection::CombinedUnstaged | StatusSection::Unstaged => pane
                            .unstaged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                        StatusSection::Untracked => pane
                            .untracked_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                        StatusSection::Staged => pane
                            .staged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                    }
                    cx.notify();
                });
        });
    }
}

impl MainPaneView {
    pub(in crate::view) fn scroll_commit_details_file_to_ix(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane
                .update(cx, |pane: &mut DetailsPaneView, cx| {
                    pane.commit_files_scroll
                        .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center);
                    cx.notify();
                });
        });
    }

    pub(super) fn apply_state_snapshot(
        &mut self,
        next: Arc<AppState>,
        cx: &mut gpui::Context<Self>,
    ) {
        let prev_active_repo_id = self.state.active_repo;
        let prev_diff_target = Self::rendered_diff_target_for_state(self.state.as_ref());

        let next_repo_id = next.active_repo;
        let next_diff_target = Self::rendered_diff_target_for_state(next.as_ref());

        if prev_active_repo_id != next_repo_id || prev_diff_target != next_diff_target {
            self.approved_remote_markdown_image_urls = Arc::default();
            self.remote_markdown_image_approval_revision =
                self.remote_markdown_image_approval_revision.wrapping_add(1);
        }
        if prev_diff_target != next_diff_target {
            self.clear_diff_selection_state();
            self.diff_autoscroll_pending = next_diff_target.is_some();
            self.worktree_preview_path = None;
            self.worktree_preview = Loadable::NotLoaded;
            self.worktree_preview_content_rev = 0;
            self.worktree_markdown_preview_path = None;
            self.worktree_markdown_preview_source_rev = 0;
            self.worktree_markdown_preview = Loadable::NotLoaded;
            self.worktree_markdown_preview_inflight = None;
            self.worktree_preview_syntax_language = None;
            self.reset_worktree_preview_source_state();
            self.reset_diff_horizontal_scroll_state();
            self.reset_collapsed_diff_projection(true);
        }

        self.state = next;
        // A closed repo tab takes its `RepoId` with it; buffers stashed under it
        // can never be saved again and would block every future close.
        self.prune_orphaned_file_editor_stash();

        self.sync_conflict_resolver(cx);
        self.ensure_file_image_diff_cache(cx);
        if self.current_main_diff_supports_diff_content_toggle() {
            self.ensure_file_diff_cache(cx);
        }

        if prev_active_repo_id != next_repo_id {
            self.history_view.update(cx, |view, _| {
                view.history_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            });
        }

        self.ensure_rendered_patch_diff_cache(cx);

        // Sync per-repo interactive commit editing state. Each repo with a setup
        // gets its own `IRebaseViewState`, populated once its entries become Ready
        // and kept (with local edits) across repo-tab switches. State for repos
        // whose setup is gone (cancelled, started, repo closed) is dropped.
        self.sync_interactive_commit_editor_states();

        // History caches are now managed by HistoryView.
    }
}

impl MainPaneView {
    pub(in crate::view) fn cached_path_display(&self, path: &std::path::Path) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    pub(in crate::view) fn touch_diff_text_layout_cache(
        &mut self,
        key: u64,
        layout: Option<ShapedLine>,
    ) {
        let epoch = self.diff_text_layout_cache_epoch;
        match layout {
            Some(layout) => {
                self.diff_text_layout_cache.insert(
                    key,
                    DiffTextLayoutCacheEntry {
                        layout,
                        last_used_epoch: epoch,
                    },
                );
            }
            None => {
                if let Some(entry) = self.diff_text_layout_cache.get_mut(&key) {
                    entry.last_used_epoch = epoch;
                }
            }
        }
    }

    /// Prune the layout cache if it has grown past the high-water mark.
    /// Call once per render frame (after bumping the epoch), **not** from
    /// the per-row `touch_diff_text_layout_cache` hot path.
    pub(in crate::view) fn prune_diff_text_layout_cache(&mut self) {
        if self.diff_text_layout_cache.len()
            <= DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES + DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE
        {
            return;
        }

        let over_by = self
            .diff_text_layout_cache
            .len()
            .saturating_sub(DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES);
        if over_by == 0 {
            return;
        }

        let mut by_age: Vec<(u64, u64)> = self
            .diff_text_layout_cache
            .iter()
            .map(|(k, v)| (*k, v.last_used_epoch))
            .collect();
        by_age.sort_by_key(|(_, last_used)| *last_used);

        for (key, _) in by_age.into_iter().take(over_by) {
            self.diff_text_layout_cache.remove(&key);
        }
    }

    pub(in crate::view) fn diff_text_segments_cache_get(
        &self,
        key: usize,
        syntax_epoch: u64,
    ) -> Option<&CachedDiffStyledText> {
        versioned_cached_diff_styled_text_is_current(
            self.diff_text_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
        )
    }

    pub(in crate::view) fn file_diff_split_cache_key(
        &self,
        row_ix: usize,
        region: DiffTextRegion,
    ) -> Option<usize> {
        let base = row_ix.checked_mul(2)?;
        match region {
            DiffTextRegion::SplitLeft => Some(base),
            DiffTextRegion::SplitRight => base.checked_add(1),
            DiffTextRegion::Inline => None,
        }
    }

    pub(in crate::view) fn diff_text_segments_cache_set(
        &mut self,
        key: usize,
        syntax_epoch: u64,
        value: CachedDiffStyledText,
    ) -> &CachedDiffStyledText {
        if self.diff_text_segments_cache.len() <= key {
            self.diff_text_segments_cache.resize_with(key + 1, || None);
        }
        self.diff_text_segments_cache[key] = Some(VersionedCachedDiffStyledText {
            syntax_epoch,
            query_generation: 0,
            styled: value,
        });
        if self.diff_text_query_segments_cache.len() > key {
            self.diff_text_query_segments_cache[key] = None;
        }
        self.diff_text_segments_cache[key]
            .as_ref()
            .map(|entry| &entry.styled)
            .expect("just set")
    }
}

impl MainPaneView {
    /// Returns the current diff search query, or an empty `SharedString` if search is inactive.
    pub(in crate::view) fn diff_search_query_or_empty(&self) -> SharedString {
        if self.diff_search_active {
            self.diff_search_query.clone()
        } else {
            SharedString::default()
        }
    }

    /// Returns the syntax mode for patch diff views (non-full-document).
    /// Uses `Auto` for small diffs and `HeuristicOnly` for large ones.
    pub(in crate::view) fn patch_diff_syntax_mode(&self) -> rows::DiffSyntaxMode {
        if self.patch_diff_row_len() <= rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            rows::DiffSyntaxMode::Auto
        } else {
            rows::DiffSyntaxMode::HeuristicOnly
        }
    }

    pub(in crate::view) fn conflict_row_styling_enabled(&self) -> bool {
        !self.conflict_resolver.is_binary_conflict
    }

    pub(in crate::view) fn conflict_row_syntax_language(&self) -> Option<rows::DiffSyntaxLanguage> {
        self.conflict_resolver.conflict_syntax_language
    }

    pub(in crate::view) fn worktree_preview_segments_cache_get(
        &self,
        key: usize,
    ) -> Option<&CachedDiffStyledText> {
        versioned_cached_diff_styled_text_is_current(
            self.worktree_preview_segments_cache.get(&key),
            self.worktree_preview_style_cache_epoch,
        )
    }

    pub(in crate::view) fn worktree_preview_segments_cache_set(
        &mut self,
        key: usize,
        value: CachedDiffStyledText,
    ) {
        self.worktree_preview_segments_cache.insert(
            key,
            VersionedCachedDiffStyledText {
                syntax_epoch: self.worktree_preview_style_cache_epoch,
                query_generation: 0,
                styled: value,
            },
        );
    }

    pub(in crate::view) fn is_file_diff_view_active(&self) -> bool {
        self.effective_diff_content_mode() == DiffContentMode::Full
            && self.rendered_file_diff_cache_is_current()
    }

    /// Whether the rasterized image diff on screen belongs to the current
    /// target. Deliberately not gated on [`DiffContentMode`]: an image has no
    /// collapsed form, so its rendered view is the same in either diff mode.
    pub(in crate::view) fn is_file_image_diff_view_active(&self) -> bool {
        let Some((repo_id, diff_file_rev, diff_target, _workdir, abs_path)) =
            self.rendered_file_diff_identity()
        else {
            return false;
        };
        self.file_image_diff_cache_repo_id == Some(repo_id)
            && self.file_image_diff_cache_rev == diff_file_rev
            && self.file_image_diff_cache_target == Some(diff_target)
            && self.file_image_diff_cache_path.as_ref() == Some(&abs_path)
            && self.file_image_diff_cache_complete
    }

    pub(in crate::view) fn consume_suppress_click_after_drag(&mut self) -> bool {
        if self.diff_suppress_clicks_remaining > 0 {
            self.diff_suppress_clicks_remaining =
                self.diff_suppress_clicks_remaining.saturating_sub(1);
            return true;
        }
        false
    }
}

impl MainPaneView {
    /// Patch source lines behind a full-file diff row. The file-diff and
    /// collapsed views render whole file texts rather than patch rows, so a row
    /// is matched back to the patch by file path plus line number.
    pub(in crate::view) fn patch_src_ixs_for_file_diff_row(&self, row_ix: usize) -> Vec<usize> {
        let (old_line, new_line) = match self.diff_view {
            DiffViewMode::Inline => {
                let Some(line) = self.file_diff_inline_render_data(row_ix) else {
                    return Vec::new();
                };
                (line.old_line, line.new_line)
            }
            DiffViewMode::Split => {
                let Some(row) = self.file_diff_split_render_data(row_ix) else {
                    return Vec::new();
                };
                (row.old_line, row.new_line)
            }
        };
        self.patch_src_ixs_for_file_line(old_line, new_line)
    }

    fn patch_src_ixs_for_file_line(
        &self,
        old_line: Option<u32>,
        new_line: Option<u32>,
    ) -> Vec<usize> {
        // A row with no line number on either side cannot identify a patch line.
        if old_line.is_none() && new_line.is_none() {
            return Vec::new();
        }
        let Some(abs) = self.file_diff_cache_path.as_ref() else {
            return Vec::new();
        };
        let Some(workdir) = self.rendered_diff_workdir() else {
            return Vec::new();
        };
        let rel = abs.strip_prefix(workdir).unwrap_or(abs);
        // Git diffs use forward slashes even on Windows.
        let rel_str = rel.to_str().map(|text| text.replace('\\', "/"));

        let mut out = Vec::with_capacity(2);
        for src_ix in 0..self.patch_diff_row_len() {
            if self
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|p| p.as_deref())
                != rel_str.as_deref()
            {
                continue;
            }
            let Some(line) = self.patch_diff_row(src_ix) else {
                continue;
            };
            let matched = match line.kind {
                gitcomet_core::domain::DiffLineKind::Add => line.new_line == new_line,
                gitcomet_core::domain::DiffLineKind::Remove
                | gitcomet_core::domain::DiffLineKind::Context => line.old_line == old_line,
                gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => false,
            };
            if matched {
                out.push(src_ix);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub(in crate::view) fn diff_src_ixs_for_visible_ix(&self, visible_ix: usize) -> Vec<usize> {
        if self.is_collapsed_diff_projection_active() {
            let Some(source_visible_ix) = self.diff_source_visible_ix_for_visible_ix(visible_ix)
            else {
                return Vec::new();
            };
            let Some(row) = self.collapsed_visible_row(source_visible_ix) else {
                return Vec::new();
            };
            match row {
                CollapsedDiffVisibleRow::HunkHeader { .. } => {
                    return row.header_action_src_ix().into_iter().collect();
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } => {
                    return self.patch_src_ixs_for_file_diff_row(row_ix);
                }
            }
        }

        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            return Vec::new();
        };

        if self.is_file_diff_view_active() {
            return self.patch_src_ixs_for_file_diff_row(mapped_ix);
        }

        match self.diff_view {
            DiffViewMode::Inline => vec![mapped_ix],
            DiffViewMode::Split => {
                let Some(row) = self.patch_diff_split_row(mapped_ix) else {
                    return Vec::new();
                };
                match row {
                    PatchSplitRow::Raw { src_ix, .. } => vec![src_ix],
                    PatchSplitRow::Aligned {
                        old_src_ix,
                        new_src_ix,
                        ..
                    } => {
                        let mut out = Vec::with_capacity(2);
                        if let Some(ix) = old_src_ix {
                            out.push(ix);
                        }
                        if let Some(ix) = new_src_ix
                            && out.first().copied() != Some(ix)
                        {
                            out.push(ix);
                        }
                        out
                    }
                }
            }
        }
    }

    pub(super) fn diff_enclosing_hunk_src_ix(&self, src_ix: usize) -> Option<usize> {
        let src_ix = src_ix.min(self.patch_diff_row_len().saturating_sub(1));
        for ix in (0..=src_ix).rev() {
            let line = self.patch_diff_row(ix)?;
            if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                && line.text.starts_with("diff --git ")
            {
                break;
            }
            if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
                return Some(ix);
            }
        }
        None
    }
}

impl MainPaneView {
    pub(in crate::view) fn select_all_diff_text(&mut self) {
        // Markdown preview (both file preview and diff preview) uses
        // markdown preview row counts instead of source-text line counts.
        if self.is_markdown_preview_active() {
            let Some(count) = self.markdown_preview_row_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let region = if self.is_file_preview_active() {
                DiffTextRegion::Inline
            } else {
                match self.diff_view {
                    DiffViewMode::Inline => DiffTextRegion::Inline,
                    DiffViewMode::Split => self
                        .diff_text_head
                        .or(self.diff_text_anchor)
                        .map(|p| p.region)
                        .filter(|r| {
                            matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight)
                        })
                        .unwrap_or(DiffTextRegion::SplitLeft),
                }
            };
            let end_visible_ix = count - 1;
            let end_offset = self.diff_text_line_len_for_region(end_visible_ix, region);

            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix: 0,
                region,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                source_visible_ix: end_visible_ix,
                region,
                offset: end_offset,
            });
            self.sync_diff_focus_to_text_selection();
            return;
        }

        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let end_visible_ix = count - 1;
            let end_offset =
                self.diff_text_line_len_for_region(end_visible_ix, DiffTextRegion::Inline);

            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix: 0,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                source_visible_ix: end_visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_offset,
            });
            self.sync_diff_focus_to_text_selection();
            return;
        }

        if self.diff_source_visible_len() == 0 {
            return;
        }

        let start_region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => self
                .diff_text_head
                .or(self.diff_text_anchor)
                .map(|p| p.region)
                .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight))
                .unwrap_or(DiffTextRegion::SplitLeft),
        };

        let end_visible_ix = self.diff_source_visible_len() - 1;
        let end_region = start_region;
        let end_offset = self
            .diff_text_full_line_for_region(end_visible_ix, end_region)
            .len();

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            source_visible_ix: 0,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            source_visible_ix: end_visible_ix,
            region: end_region,
            offset: end_offset,
        });
        self.sync_diff_focus_to_text_selection();
    }

    pub(super) fn split_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(&PatchSplitRow) -> bool,
    ) -> Option<usize> {
        let visible_len = self.diff_visible_len();
        let from_visible_ix = from_visible_ix.min(visible_len.saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..visible_len {
            let row_ix = self.diff_mapped_ix_for_visible_ix(visible_ix)?;
            let row = self.patch_diff_split_row(row_ix)?;
            if is_boundary(&row) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    pub(super) fn diff_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let visible_len = self.diff_visible_len();
        let from_visible_ix = from_visible_ix.min(visible_len.saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..visible_len {
            let src_ix = self.diff_mapped_ix_for_visible_ix(visible_ix)?;
            if is_boundary(src_ix) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }
}

/// Decide whether a blame (re)load should be dispatched for the rendered target.
///
/// `same_target` is whether the currently loaded blame is for the same
/// file/source. `force` requests a retry of a previous failure (an explicit user
/// toggle); the per-frame Render path passes `false` so a persistent error does
/// not cause a dispatch-every-frame loop.
fn should_request_blame<T>(
    same_target: bool,
    blame: &gitcomet_state::model::Loadable<T>,
    force: bool,
) -> bool {
    use gitcomet_state::model::Loadable;
    if !same_target {
        // A new or changed target always (re)loads.
        return true;
    }
    match blame {
        // Already loaded or in flight for this target: nothing to do.
        Loadable::Ready(_) | Loadable::Loading => false,
        // A previous attempt failed: retry only on an explicit user toggle.
        Loadable::Error(_) => force,
        Loadable::NotLoaded => true,
    }
}

mod diff_wrap;
mod hover_tooltip;
mod live_syntax;
mod outline;
mod rendered_target;
mod scroll_sync;

use diff_wrap::*;
use outline::*;

#[cfg(test)]
use scroll_sync::*;

#[cfg(test)]
mod tests;
