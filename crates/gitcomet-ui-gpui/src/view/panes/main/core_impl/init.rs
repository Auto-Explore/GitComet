use super::*;

pub(in crate::view) struct MainPaneInit {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) view_mode: GitCometViewMode,
    pub(in crate::view) focused_mergetool_labels: Option<FocusedMergetoolLabels>,
    pub(in crate::view) focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub(in crate::view) root_view: WeakEntity<GitCometView>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
}

impl MainPaneView {
    pub(in crate::view) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        init: MainPaneInit,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let MainPaneInit {
            theme,
            view_mode,
            focused_mergetool_labels,
            focused_mergetool_exit_code,
            root_view,
            tooltip_host,
        } = init;
        let preferences = ui_model.read(cx).preferences.clone();
        let date_time_format = preferences.appearance.date_time_format;
        let timezone = preferences.appearance.timezone;
        let show_timezone = preferences.appearance.show_timezone;
        let history_relative_dates = preferences.history.relative_dates;
        let history_highlight_commit_chain = preferences.history.highlight_commit_chain;
        let diff_scroll_sync = preferences.diff.scroll_sync;
        let diff_content_mode = preferences.diff.content_mode;
        let diff_whitespace_mode = preferences.diff.whitespace_mode;
        let diff_view_mode = preferences.diff.view_mode;
        let annotate_enabled = preferences.diff.annotate_enabled;
        let diff_reveal_whitespace_chars = preferences.diff.reveal_whitespace_chars;
        let diff_word_wrap = preferences.diff.word_wrap;
        let diff_show_line_numbers = preferences.diff.show_line_numbers;
        let auto_save_file_edits = preferences.file_editing.auto_save;
        let history_show_graph = preferences.history.show_graph;
        let history_show_author = preferences.history.show_author;
        let history_show_date = preferences.history.show_date;
        let history_show_sha = preferences.history.show_sha;
        let history_show_tags = preferences.history.show_tags;
        let history_auto_fetch_tags_on_repo_activation = matches!(
            preferences.history.tag_fetch_mode,
            gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
        );
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.apply_state_snapshot(next, cx);
            cx.notify();
        });

        let diff_raw_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    multiline: true,
                    read_only: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        let submodule_hash_inputs = (0..4)
            .map(|_| {
                cx.new(|cx| {
                    let mut input = components::TextInput::new(
                        components::TextInputOptions {
                            read_only: true,
                            ..Default::default()
                        },
                        window,
                        cx,
                    );
                    input.set_read_only(true, cx);
                    input
                })
            })
            .collect::<Vec<_>>();

        let conflict_resolver_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Resolve file contents…".into(),
                    multiline: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_suppress_right_click(true);
            input.set_line_height(
                Some(ui_scale::design_px_from_percent(
                    20.0,
                    ui_scale::current(cx).percent,
                )),
                cx,
            );
            input
        });

        let conflict_resolver_subscription =
            cx.observe(&conflict_resolver_input, |this, input, cx| {
                let _perf_scope = crate::view::perf::span(
                    crate::view::perf::ViewPerfSpan::ResolvedOutputEditObserve,
                );
                let (output_snapshot, edit_deltas) = input.update(cx, |input, _| {
                    (input.text_snapshot(), input.drain_recent_utf8_edit_deltas())
                });
                let outline_edit_delta = (edit_deltas.len() == 1)
                    .then(|| edit_deltas.first().cloned())
                    .flatten();
                // Fold the tree forward before anything else looks at the
                // buffer, so the very next frame paints from a tree that
                // already describes what was just typed.
                let syntax_edit = coalesce_resolved_output_edit_deltas(&edit_deltas);
                this.apply_conflict_resolved_output_edit_deltas(
                    edit_deltas,
                    &output_snapshot.rope(),
                );
                if !this.conflict_resolved_output_is_streamed() {
                    this.refresh_conflict_resolved_output_syntax(&output_snapshot, syntax_edit, cx);
                }
                let source_revision = ResolvedOutputSourceRevision::from_snapshot(&output_snapshot);
                let output_modified = resolved_output_snapshot_is_modified(
                    this.conflict_resolved_output_saved_snapshot.as_ref(),
                    &output_snapshot,
                );
                if this.conflict_resolved_output_modified != output_modified {
                    this.conflict_resolved_output_modified = output_modified;
                    cx.notify();
                }
                let outline_delta = resolved_outline_delta_for_snapshot_transition(
                    &this.conflict_resolved_preview_text,
                    &output_snapshot,
                    outline_edit_delta,
                );

                let path = this.conflict_resolver.path.clone();
                let needs_update = this.conflict_resolved_preview_path.as_ref() != path.as_ref()
                    || this.conflict_resolved_preview_source_revision != Some(source_revision);
                if !needs_update {
                    return;
                }

                this.conflict_resolved_preview_path = path.clone();
                this.conflict_resolved_preview_source_revision = Some(source_revision);
                this.schedule_conflict_resolved_outline_recompute(
                    path,
                    source_revision,
                    outline_delta,
                    cx,
                );
                // The Save gates derive effective resolutions from the live
                // editor text, so the containing toolbar must re-render for
                // every edit even while session state remains deferred.
                cx.notify();
            });

        let file_editor_scroll = ScrollHandle::new();
        let file_editor_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    multiline: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            // The input lays out at its content width inside an
            // `overflow_scroll` container, which is what gives that container a
            // horizontal extent to scroll — the same arrangement the resolved
            // output uses.
            input.set_content_width_layout(true);
            input.set_vertical_scroll_handle(Some(file_editor_scroll.clone()));
            input.set_line_height(
                Some(ui_scale::design_px_from_percent(
                    20.0,
                    ui_scale::current(cx).percent,
                )),
                cx,
            );
            input
        });
        let file_editor_subscription = cx.observe(&file_editor_input, |this, _input, cx| {
            this.on_file_editor_edited(cx);
        });

        let diff_search_scroll = ScrollHandle::new();
        let diff_search_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Search diff".into(),
                    multiline: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_submit_on_enter(true);
            input.set_vertical_scroll_handle(Some(diff_search_scroll.clone()));
            input.set_vertical_padding(Some(px(4.0)), cx);
            input.set_line_height(
                Some(ui_scale::design_px_from_percent(
                    18.0,
                    ui_scale::current(cx).percent,
                )),
                cx,
            );
            input
        });
        let diff_search_subscription = cx.observe(&diff_search_input, |this, input, cx| {
            if input.update(cx, |input, _| input.take_enter_pressed()) {
                if this.diff_search_active {
                    this.diff_search_next_match();
                    cx.notify();
                }
                return;
            }
            let next: SharedString = input.read(cx).text().to_string().into();
            if this.diff_search_query != next {
                let previous_query = this.diff_search_query.clone();
                this.diff_search_query = next.clone();
                if next.is_empty() {
                    this.diff_search_scroll.set_offset(point(px(0.0), px(0.0)));
                }
                this.invalidate_diff_text_query_overlay_cache(
                    next.as_ref(),
                    this.diff_search_options,
                );
                this.clear_worktree_preview_segments_cache();
                this.clear_conflict_diff_query_overlay_caches();
                if next.is_empty() {
                    this.diff_search_cancel_pending_query_recompute();
                    this.diff_search_recompute_matches_for_query_change(previous_query.as_ref());
                } else {
                    this.diff_search_schedule_query_recompute(previous_query, cx);
                }
                cx.notify();
            }
        });

        let diff_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);

        let last_window_size = window.viewport_size();
        let ui_scale_percent = ui_scale::current(cx).percent;
        let history_view = cx.new(|cx| {
            HistoryView::new(
                Arc::clone(&store),
                ui_model.clone(),
                theme,
                ui_scale_percent,
                date_time_format,
                timezone,
                show_timezone,
                history_relative_dates,
                history_highlight_commit_chain,
                history_show_graph,
                history_show_author,
                history_show_date,
                history_show_sha,
                history_show_tags,
                history_auto_fetch_tags_on_repo_activation,
                root_view.clone(),
                last_window_size,
                window,
                cx,
            )
        });

        let mut pane = Self {
            store,
            state,
            view_mode,
            focused_mergetool_labels,
            focused_mergetool_exit_code,
            theme,
            date_time_format,
            _ui_model_subscription: subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            last_window_size: size(px(0.0), px(0.0)),
            layout_sidebar_render_width: px(280.0),
            layout_details_render_width: px(420.0),
            layout_sidebar_collapsed: false,
            layout_details_collapsed: false,
            reveal_whitespace_chars: diff_reveal_whitespace_chars,
            mergetool_auto_advance: preferences.merge_tool.auto_advance,
            mergetool_collapse_unchanged: preferences.merge_tool.collapse_unchanged,
            mergetool_output_scroll_sync: preferences.merge_tool.output_scroll_sync,
            mergetool_show_line_numbers: preferences.merge_tool.show_line_numbers,
            mergetool_view_three_way: preferences.merge_tool.view_three_way,
            diff_view: diff_view_mode,
            annotate_enabled,
            annotate_column_width: rows::DIFF_ANNOTATION_COLUMN_WIDTH_PX,
            annotate_resize: None,
            blame_annot_hover: None,
            diff_stage_gutter_hover: None,
            diff_stage_gutter_cells: FxHashMap::default(),
            blame_time_range_cache: None,
            rendered_preview_modes: RenderedPreviewModes::default(),
            diff_word_wrap,
            diff_show_line_numbers,
            diff_scroll_sync,
            diff_content_mode,
            diff_whitespace_mode,
            diff_split_ratio: 0.5,
            diff_split_resize: None,
            diff_split_last_synced_x: [px(0.0); 2],
            diff_split_last_synced_y: [px(0.0); 2],
            diff_horizontal_scroll: DiffHorizontalScrollState::new(),
            diff_cache_repo_id: None,
            diff_cache_rev: 0,
            diff_cache_content_signature: None,
            diff_cache_target: None,
            diff_cache: Vec::new(),
            diff_row_provider: None,
            diff_split_row_provider: None,
            diff_file_for_src_ix: Vec::new(),
            diff_language_for_src_ix: Vec::new(),
            diff_yaml_block_scalar_for_src_ix: Vec::new(),
            diff_click_kinds: Vec::new(),
            diff_line_kind_for_src_ix: Vec::new(),
            diff_visual_line_kind_for_src_ix: Vec::new(),
            diff_hide_unified_header_for_src_ix: Vec::new(),
            diff_header_display_cache: FxHashMap::default(),
            diff_split_cache: Vec::new(),
            diff_split_cache_len: 0,
            diff_panel_focus_handle,
            diff_autoscroll_pending: false,
            diff_raw_input,
            submodule_hash_inputs,
            diff_visible_indices: Vec::new(),
            diff_visible_inline_map: None,
            diff_wrap_visible_rows: Vec::new(),
            diff_wrap_visible_cache_key: None,
            collapsed_diff_hunks: Vec::new(),
            collapsed_diff_hunk_ix_by_src_ix: FxHashMap::default(),
            collapsed_diff_reveals: FxHashMap::default(),
            collapsed_diff_visible_rows: Vec::new(),
            collapsed_diff_hunk_visible_indices: Vec::new(),
            collapsed_diff_header_display_cache: FxHashMap::default(),
            collapsed_diff_projection_identity: None,
            diff_visible_cache_len: 0,
            diff_visible_view: DiffViewMode::Split,
            diff_visible_is_file_view: false,
            diff_visible_projection_rev: 0,
            diff_visible_cache_projection_rev: u64::MAX,
            diff_scrollbar_markers_cache: Vec::new(),
            diff_word_highlights: Vec::new(),
            diff_word_highlights_inflight: None,
            diff_file_stats: Vec::new(),
            diff_text_segments_cache: Vec::new(),
            diff_text_query_segments_cache: Vec::new(),
            diff_text_query_cache_query: SharedString::default(),
            diff_text_query_cache_options: Default::default(),
            diff_text_query_cache_matcher: None,
            diff_text_query_cache_generation: 0,
            diff_selection_anchor: None,
            diff_selection_range: None,
            diff_text_selecting: false,
            diff_text_anchor: None,
            diff_text_head: None,
            diff_text_autoscroll_seq: 0,
            diff_text_autoscroll_target: None,
            diff_text_last_mouse_pos: point(px(0.0), px(0.0)),
            diff_suppress_clicks_remaining: 0,
            diff_text_hitboxes: FxHashMap::default(),
            diff_search_horizontal_reveal: None,
            diff_text_pair_match: None,
            diff_text_occurrences: FxHashMap::default(),
            diff_text_pending_syntax_click: None,
            conflict_text_hitboxes: FxHashMap::default(),
            diff_text_layout_cache_epoch: 0,
            diff_text_layout_cache: FxHashMap::default(),
            diff_search_active: false,
            diff_search_query: "".into(),
            diff_search_options: Default::default(),
            diff_search_regex_error: None,
            diff_search_matches: Vec::new(),
            diff_search_inline_patch_trigram_index: None,
            diff_search_match_ix: None,
            diff_search_debounce_seq: 0,
            diff_search_pending_previous_query: None,
            diff_search_scroll,
            diff_search_input,
            _diff_search_subscription: diff_search_subscription,
            file_diff_cache_repo_id: None,
            file_diff_cache_rev: 0,
            file_diff_cache_content_signature: None,
            file_diff_cache_whitespace_mode: diff_whitespace_mode,
            file_diff_cache_target: None,
            file_diff_cache_error: None,
            file_diff_cache_path: None,
            file_diff_cache_language: None,
            file_diff_cache_rows: Vec::new(),
            file_diff_row_provider: None,
            file_diff_old_text: SharedString::default(),
            file_diff_old_line_starts: Arc::default(),
            file_diff_pair_syntax_text: FxHashMap::default(),
            file_diff_click_syntax_inflight: FxHashMap::default(),
            #[cfg(test)]
            file_diff_click_syntax_after_prepare_hook: None,
            #[cfg(test)]
            file_diff_click_syntax_before_complete_hook: None,
            file_diff_old_source_path: None,
            file_diff_new_source_path: None,
            file_diff_old_source_identity: None,
            file_diff_new_source_identity: None,
            file_diff_old_line_to_row: Arc::default(),
            file_diff_old_line_to_inline_row: Arc::default(),
            file_diff_new_text: SharedString::default(),
            file_diff_new_line_starts: Arc::default(),
            file_diff_new_line_to_row: Arc::default(),
            file_diff_new_line_to_inline_row: Arc::default(),
            file_diff_inline_cache: Vec::new(),
            file_diff_inline_row_provider: None,
            file_diff_inline_text: SharedString::default(),
            file_diff_inline_word_highlights: rows::new_lru_cache(
                FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES,
            ),
            file_diff_split_word_highlights: rows::new_lru_cache(
                FILE_DIFF_WORD_HIGHLIGHT_CACHE_MAX_ENTRIES,
            ),
            file_diff_cache_seq: 0,
            file_diff_cache_inflight: None,
            file_diff_syntax_generation: 0,
            file_diff_style_cache_epochs: FileDiffStyleCacheEpochs::default(),
            syntax_chunk_poll_task: None,
            prepared_syntax_documents: FxHashMap::default(),
            #[cfg(test)]
            diff_syntax_budget_override: None,
            file_markdown_preview_cache_repo_id: None,
            file_markdown_preview_cache_rev: 0,
            file_markdown_preview_cache_content_signature: None,
            file_markdown_preview_cache_target: None,
            file_markdown_preview: Loadable::NotLoaded,
            markdown_preview_wrap: MarkdownPreviewWrapCache::default(),
            markdown_preview_reveal: Default::default(),
            file_markdown_preview_seq: 0,
            file_markdown_preview_inflight: None,
            file_image_diff_cache_repo_id: None,
            file_image_diff_cache_rev: 0,
            file_image_diff_cache_content_signature: None,
            file_image_diff_cache_target: None,
            file_image_diff_cache_seq: 0,
            file_image_diff_cache_inflight: None,
            file_image_diff_cache_path: None,
            file_image_diff_cache_old: None,
            file_image_diff_cache_new: None,
            file_image_diff_cache_old_svg_path: None,
            file_image_diff_cache_new_svg_path: None,
            worktree_preview_path: None,
            worktree_preview_source_path: None,
            worktree_preview: Loadable::NotLoaded,
            worktree_preview_source_len: 0,
            worktree_preview_text: SharedString::default(),
            worktree_preview_line_starts: Arc::default(),
            worktree_preview_line_flags: Arc::default(),
            worktree_preview_search_trigram_index: None,
            worktree_preview_content_rev: 0,
            worktree_markdown_preview_path: None,
            worktree_markdown_preview_source_rev: 0,
            worktree_markdown_preview: Loadable::NotLoaded,
            worktree_markdown_preview_picture_sizes: Default::default(),
            worktree_markdown_preview_block_scrolls: Default::default(),
            worktree_markdown_preview_blocks: Default::default(),
            worktree_markdown_preview_image_waits: FxHashSet::default(),
            worktree_markdown_preview_seq: 0,
            worktree_markdown_preview_inflight: None,
            worktree_preview_segments_cache_path: None,
            worktree_preview_syntax_language: None,
            worktree_preview_style_cache_epoch: 0,
            worktree_preview_cache_write_blocked_until_rev: None,
            worktree_preview_segments_cache: FxHashMap::default(),
            diff_preview_is_new_file: false,
            file_editor_input,
            _file_editor_input_subscription: file_editor_subscription,
            file_editor_key: None,
            file_editor_language: None,
            file_editor_loading: false,
            file_editor_loaded_status_rev: 0,
            file_editor_error: None,
            file_editor_dirty: false,
            file_editor_first_dirty_line: None,
            unsaved_file_edits_rev: 0,
            file_editor_saved_fingerprint: None,
            file_editor_stash: FxHashMap::default(),
            file_editor_autosave: None,
            file_editor_live_syntax: None,
            file_editor_live_syntax_source: None,
            file_editor_live_syntax_building: None,
            file_editor_live_syntax_build: None,
            file_editor_live_syntax_reparse: None,
            file_editor_syntax_pair: None,
            file_editor_occurrences: Vec::new(),
            file_editor_occurrences_version: None,
            file_editor_search_matches: Vec::new(),
            file_editor_search_source: None,
            file_editor_search_rev: 0,
            file_editor_search_applied_rev: 0,
            file_editor_search_reveal_rev: 0,
            file_editor_search_reveal_applied_rev: 0,
            file_editor_search_reveal_x_pending: false,
            file_editor_provider_theme_epoch: 1,
            file_editor_scroll,
            file_editor_gutter_scroll: UniformListScrollHandle::new(),
            file_editor_gutter_row_height: ui_scale::design_px_from_percent(
                RESOLVED_OUTPUT_ROW_HEIGHT_PX,
                ui_scale::current(cx).percent,
            ),
            conflict_resolved_gutter_row_height: ui_scale::design_px_from_percent(
                RESOLVED_OUTPUT_ROW_HEIGHT_PX,
                ui_scale::current(cx).percent,
            ),
            file_editor_blame: None,
            file_editor_blame_width: px(0.0),
            file_editor_wrap_row_starts: Vec::new(),
            auto_save_file_edits,
            conflict_resolver_input,
            _conflict_resolver_input_subscription: conflict_resolver_subscription,
            conflict_resolver: ConflictResolverUiState::default(),
            conflict_open_summary_toasted_files: FxHashSet::default(),
            conflict_resolver_vsplit_ratio: 0.6,
            conflict_resolver_vsplit_resize: None,
            conflict_three_way_col_ratios: [1.0 / 3.0, 2.0 / 3.0],
            conflict_three_way_col_widths: [px(0.0); 3],
            conflict_hsplit_resize: None,
            conflict_diff_split_ratio: 0.5,
            conflict_diff_split_resize: None,
            conflict_diff_split_col_widths: [px(0.0); 2],
            conflict_canvas_rows_enabled: conflict_canvas_rows_enabled_from_env(),
            conflict_diff_segments_cache_split:
                conflict_resolver::ConflictSplitStyledTextCache::default(),
            conflict_diff_query_segments_cache_split:
                conflict_resolver::ConflictSplitStyledTextCache::default(),
            conflict_diff_query_cache_query: SharedString::default(),
            conflict_diff_query_cache_options: Default::default(),
            conflict_three_way_segments_cache: FxHashMap::default(),
            conflict_three_way_query_segments_cache: FxHashMap::default(),
            conflict_three_way_prepared_syntax_documents: ThreeWaySides::default(),
            conflict_three_way_syntax_inflight: ThreeWaySides::default(),
            conflict_resolved_preview_path: None,
            conflict_resolved_preview_source_revision: None,
            conflict_resolved_output_saved_snapshot: None,
            conflict_resolved_output_modified: false,
            conflict_resolved_output_projection: None,
            conflict_resolved_output_block_map: conflict_resolver::ResolvedOutputBlockMap::default(
            ),
            conflict_resolved_preview_text: TextModelSnapshot::default(),
            conflict_resolved_preview_syntax_language: None,
            conflict_resolved_preview_line_count: 0,
            conflict_resolved_preview_line_starts: Arc::default(),
            conflict_resolved_output_live_syntax: None,
            conflict_resolved_output_live_syntax_reparse: None,
            conflict_resolved_output_live_syntax_source: None,
            conflict_resolved_output_provider_theme_epoch: 1,
            conflict_resolved_output_highlighted_conflict: None,
            conflict_resolved_output_unresolved_rows: None,
            #[cfg(test)]
            conflict_resolved_output_full_scans: 0,
            conflict_resolved_output_live_syntax_building: None,
            conflict_resolved_output_live_syntax_build: None,
            conflict_resolved_output_measure_row: 0,
            conflict_resolved_outline_stash: None,
            #[cfg(test)]
            conflict_resolved_outline_background_delay_override: None,
            history_view,
            diff_scroll: UniformListScrollHandle::default(),
            diff_split_right_scroll: UniformListScrollHandle::default(),
            conflict_resolver_diff_scroll: UniformListScrollHandle::default(),
            conflict_preview_ours_scroll: UniformListScrollHandle::default(),
            conflict_preview_theirs_scroll: UniformListScrollHandle::default(),
            conflict_preview_last_synced_x: [px(0.0); 4],
            conflict_preview_last_synced_y: [px(0.0); 4],
            conflict_preview_vertical_wheel_master: None,
            conflict_output_gutter_wheel_sync_pending: false,
            conflict_resolved_preview_scroll: UniformListScrollHandle::default(),
            conflict_resolved_output_editor_scroll: ScrollHandle::new(),
            conflict_resolved_preview_gutter_scroll: UniformListScrollHandle::default(),
            conflict_resolved_preview_gutter_last_synced_y: [px(0.0); 2],
            worktree_preview_scroll: UniformListScrollHandle::default(),
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            interactive_rebase_states: FxHashMap::default(),
        };

        pane.set_theme(theme, cx);
        pane.ensure_rendered_patch_diff_cache(cx);
        pane
    }
}
