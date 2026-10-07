//! Window layout and UI-setting persistence: snapshot construction and the
//! debounced writes to the workspace store and the session file.

use super::*;

impl GitCometView {
    pub(crate) fn current_workspace_layout(&self, cx: &gpui::App) -> session::WorkspaceLayout {
        let (change_tracking_height, untracked_height) =
            self.details_pane.read(cx).saved_status_section_heights();
        let sidebar_width = ui_scale::stored_design_units(Some(
            self.ui_scale().design_units_from_pixels(self.sidebar_width),
        ));
        let details_width = ui_scale::stored_design_units(Some(
            self.ui_scale().design_units_from_pixels(self.details_width),
        ));
        session::WorkspaceLayout {
            sidebar_width,
            details_width,
            sidebar_collapsed: self.sidebar_collapsed,
            change_tracking_height,
            untracked_height,
        }
    }

    /// Record the live per-window layout synchronously. Debounced settings
    /// writes still handle ordinary resizing, but close/quit cannot wait for a
    /// weak-view timer that becomes invalid as soon as the window disappears.
    pub(crate) fn flush_workspace_environment(&self, cx: &mut gpui::Context<Self>) {
        let layout = self.current_workspace_layout(cx);
        let placement = self.window_placement.clone();
        crate::workspaces::update_window_environment(
            cx,
            self.window_handle.window_id(),
            layout,
            placement,
        );
    }

    /// Debounce the disk write for bounds-only changes. The live placement is
    /// already recorded in the group manager by the bounds observer; this
    /// flushes that manager without serializing this window's potentially
    /// stale copy of process-wide UI preferences.
    pub(super) fn schedule_workspace_persist(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().persists_ui_settings() {
            let _ = cx;
            return;
        }

        self.workspace_persist_seq = self.workspace_persist_seq.wrapping_add(1);
        let seq = self.workspace_persist_seq;
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let _ = view.update(cx, |this, cx| {
                    if this.workspace_persist_seq == seq {
                        crate::workspaces::persist_unsaved_placement(cx);
                    }
                });
            },
        )
        .detach();
    }

    /// Focused mergetool windows do not participate in durable workspaces, so
    /// they keep their own size keys, which workspace writes never touch.
    /// Persist only the dimensions: a resize must not write a stale snapshot
    /// of unrelated preferences owned by another window.
    pub(super) fn schedule_legacy_window_bounds_persist(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(test)]
        {
            self.ui_settings_persist_requests_for_test =
                self.ui_settings_persist_requests_for_test.wrapping_add(1);
        }
        if !crate::ui_runtime::current().persists_ui_settings() {
            let _ = cx;
            return;
        }

        self.ui_settings_persist_seq = self.ui_settings_persist_seq.wrapping_add(1);
        let seq = self.ui_settings_persist_seq;
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let size = view
                    .update(cx, |this, _cx| {
                        if this.ui_settings_persist_seq != seq {
                            return None;
                        }
                        let width: f32 = this.last_window_size.width.round().into();
                        let height: f32 = this.last_window_size.height.round().into();
                        let valid = |value: f32| value.is_finite() && value >= 1.0;
                        (valid(width) && valid(height)).then_some((width as u32, height as u32))
                    })
                    .ok()
                    .flatten();
                if let Some((width, height)) = size {
                    let _ = cx
                        .background_executor()
                        .spawn(async move { session::persist_mergetool_window_size(width, height) })
                        .await;
                }
            },
        )
        .detach();
    }

    pub(super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(test)]
        {
            self.ui_settings_persist_requests_for_test =
                self.ui_settings_persist_requests_for_test.wrapping_add(1);
        }
        if !crate::ui_runtime::current().persists_ui_settings() {
            let _ = cx;
            return;
        }

        self.ui_settings_persist_seq = self.ui_settings_persist_seq.wrapping_add(1);
        let seq = self.ui_settings_persist_seq;

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let settings = view
                    .update(cx, |this, cx| {
                        if this.ui_settings_persist_seq != seq {
                            return None;
                        }

                        let ww: f32 = this.last_window_size.width.round().into();
                        let wh: f32 = this.last_window_size.height.round().into();
                        let window_width = (ww.is_finite() && ww >= 1.0).then_some(ww as u32);
                        let window_height = (wh.is_finite() && wh >= 1.0).then_some(wh as u32);

                        let (
                            history_show_graph,
                            history_show_author,
                            history_show_date,
                            history_show_sha,
                        ) = this
                            .main_pane
                            .read(cx)
                            .history_visible_column_preferences(cx);
                        let (history_show_tags, history_auto_fetch_tags_on_repo_activation) = this
                            .main_pane
                            .read(cx)
                            .history_tag_preferences(cx);
                        let history_relative_dates =
                            this.main_pane.read(cx).history_relative_dates(cx);
                        let history_highlight_commit_chain =
                            this.main_pane.read(cx).history_highlight_commit_chain(cx);
                        let (
                            mergetool_auto_advance,
                            mergetool_collapse_unchanged,
                            mergetool_output_scroll_sync,
                            mergetool_show_line_numbers,
                        ) = this.main_pane.read(cx).mergetool_preferences();
                        let mergetool_view_three_way =
                            this.main_pane.read(cx).mergetool_view_three_way;
                        let group_layout = this.current_workspace_layout(cx);
                        let sidebar_width = group_layout.sidebar_width;
                        let details_width = group_layout.details_width;
                        let change_tracking_height = group_layout.change_tracking_height;
                        let untracked_height = group_layout.untracked_height;
                        crate::workspaces::update_window_environment(
                            cx,
                            this.window_handle.window_id(),
                            group_layout,
                            this.window_placement.clone(),
                        );
                        let font_preferences = crate::font_preferences::current(cx);

                        let settings = session::UiSettings {
                            window_width,
                            window_height,
                            sidebar_width,
                            details_width,
                            sidebar_collapsed: Some(this.sidebar_collapsed),
                            repo_sidebar_collapsed_items: None,
                            repo_sidebar_pinned_branches: None,
                            theme_mode: Some(this.theme_mode.key().to_string()),
                            // The default is the settings window's; a window's own zoom is not saved.
                            ui_scale_percent: None,
                            // Owned by the settings window; None preserves it.
                            window_controls_mode: None,
                            browser_open_target: None,
                            ui_density: Some(crate::appearance::current(cx).density.key().to_string()),
                            ui_font_size_px: Some(crate::appearance::current(cx).ui_font_size_px),
                            editor_font_size_px: Some(crate::appearance::current(cx).editor_font_size_px),
                            markdown_preview_font_size_px: Some(crate::appearance::current(cx).markdown_preview_font_size_px),
                            ui_font_family: Some(font_preferences.ui_font_family),
                            editor_font_family: Some(font_preferences.editor_font_family),
                            use_font_ligatures: Some(font_preferences.use_font_ligatures),
                            date_time_format: Some(this.date_time_format.key().to_string()),
                            timezone: Some(this.timezone.key()),
                            show_timezone: Some(this.show_timezone),
                            change_tracking_view: Some(this.change_tracking_view.key().to_string()),
                            file_list_layout: Some(this.file_list_layout.key().to_string()),
                            file_list_sort: Some(this.file_list_sort.key().to_string()),
                            // Owned by the repository picker, not this snapshot.
                            repo_picker_sort: None,
                            repo_picker_collapsed_sections: None,
                            diff_scroll_sync: Some(this.diff_scroll_sync.key().to_string()),
                            diff_content_mode: Some(this.diff_content_mode.key().to_string()),
                            diff_whitespace_mode: Some(
                                this.diff_whitespace_mode.key().to_string(),
                            ),
                            diff_view_mode: Some(this.diff_view_mode.key().to_string()),
                            annotate_enabled: Some(this.annotate_enabled),
                            diff_reveal_whitespace_chars: Some(
                                this.diff_reveal_whitespace_chars,
                            ),
                            diff_word_wrap: Some(this.diff_word_wrap),
                            diff_tab_size: Some(this.diff_tab_size),
                            diff_show_line_numbers: Some(this.diff_show_line_numbers),
                            remote_markdown_image_policy: Some(
                                this.remote_markdown_image_policy.key().to_string(),
                            ),
                            allowed_remote_protocols: Some(
                                this.remote_url_policy
                                    .allowed_protocols()
                                    .map(|protocol| protocol.key().to_string())
                                    .collect(),
                            ),
                            check_for_updates_on_startup: Some(
                                this.check_for_updates_on_startup,
                            ),
                            // Auto-save is only ever changed from the settings
                            // window; the main window mirrors it to drive the
                            // editor, so None keeps the stored value.
                            auto_save_file_edits: None,
                            mergetool_auto_advance: Some(mergetool_auto_advance),
                            mergetool_collapse_unchanged: Some(mergetool_collapse_unchanged),
                            mergetool_output_scroll_sync: Some(mergetool_output_scroll_sync),
                            mergetool_show_line_numbers: Some(mergetool_show_line_numbers),
                            mergetool_view_three_way: Some(mergetool_view_three_way),
                            change_tracking_height,
                            untracked_height,
                            // The settings window owns branch-name placement.
                            history_branch_names: None,
                            history_show_graph: Some(history_show_graph),
                            history_show_author: Some(history_show_author),
                            history_show_date: Some(history_show_date),
                            history_show_sha: Some(history_show_sha),
                            history_relative_dates: Some(history_relative_dates),
                            history_highlight_commit_chain: Some(history_highlight_commit_chain),
                            // Owned by the settings window, which persists it itself.
                            file_browser_follow_selected_commit: None,
                            annex_hide_bookkeeping_refs: None,
                            annex_pull_push_on_adjusted: None,
                            annex_sync_content: None,
                            terminal_external_mode: None,
                            terminal_external_program: None,
                            terminal_external_args: None,
                            terminal_action_bar_target: None,
                            history_show_tags: Some(history_show_tags),
                            history_verify_commit_signatures: None,
                            history_tag_fetch_mode: Some(if history_auto_fetch_tags_on_repo_activation
                            {
                                gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
                            } else {
                                gitcomet_state::model::GitLogTagFetchMode::Disabled
                            }),
                            default_history_mode: None,
                            commit_push_after_enabled: Some(this.commit_push_after_enabled),
                            default_tag_type: None,
                            fetch_prune_deleted_remote_branches: None,
                            recommend_repo_maintenance: None,
                            git_executable_path: None,
                            external_code_editor: None,
                        };

                        Some(settings)
                    })
                    .ok()
                    .flatten();

                let Some(settings) = settings else {
                    return;
                };

                let _ = cx.background_executor().spawn(async move {
                    session::persist_ui_settings(settings)
                }).await;
            },
        )
        .detach();
    }

    pub(super) fn clamp_pane_widths_to_window(&mut self) {
        let total_w = self.last_window_size.width;
        if total_w.is_zero() {
            return;
        }

        let sidebar_handle_w = if self.sidebar_collapsed {
            px(0.0)
        } else {
            self.pane_resize_handle_width()
        };
        let details_handle_w = if self.details_collapsed {
            px(0.0)
        } else {
            self.pane_resize_handle_width()
        };
        let handles_w = sidebar_handle_w + details_handle_w;
        let main_min = self.main_min_width();
        let sidebar_min = self.sidebar_min_width();
        let details_min = self.details_min_width();
        let collapsed_w = self.pane_collapsed_width();

        if !self.sidebar_collapsed {
            let details_w = if self.details_collapsed {
                collapsed_w
            } else {
                self.details_width.max(details_min)
            };
            let max_sidebar = (total_w - details_w - main_min - handles_w).max(sidebar_min);
            self.set_sidebar_width_from_pixels(
                self.sidebar_width.max(sidebar_min).min(max_sidebar),
            );
        } else {
            self.set_sidebar_width_from_pixels(self.sidebar_width.max(sidebar_min));
        }

        if !self.details_collapsed {
            let sidebar_w = if self.sidebar_collapsed {
                collapsed_w
            } else {
                self.sidebar_width.max(sidebar_min)
            };
            let max_details = (total_w - sidebar_w - main_min - handles_w).max(details_min);
            self.set_details_width_from_pixels(
                self.details_width.max(details_min).min(max_details),
            );
        } else {
            self.set_details_width_from_pixels(self.details_width.max(details_min));
        }

        let sidebar_target = if self.sidebar_collapsed {
            collapsed_w
        } else {
            self.sidebar_width
        };
        let details_target = if self.details_collapsed {
            collapsed_w
        } else {
            self.details_width
        };

        if !self.sidebar_width_animating {
            self.sidebar_render_width = sidebar_target;
        } else {
            self.sidebar_render_width = self.sidebar_render_width.max(px(0.0)).min(total_w);
        }
        if !self.details_width_animating {
            self.details_render_width = details_target;
        } else {
            self.details_render_width = self.details_render_width.max(px(0.0)).min(total_w);
        }
    }
}
