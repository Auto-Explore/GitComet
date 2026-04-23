use super::*;

impl GitCometView {
    pub(super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().persists_ui_settings() {
            let _ = cx;
            return;
        }

        self.ui_settings_persist_seq = self.ui_settings_persist_seq.wrapping_add(1);
        let seq = self.ui_settings_persist_seq;

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(250)).await;
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
                        let (change_tracking_height, untracked_height) =
                            this.details_pane.read(cx).saved_status_section_heights();
                        let repo_sidebar_collapsed_items =
                            this.sidebar_pane.read(cx).saved_sidebar_collapsed_items();
                        let font_preferences = crate::font_preferences::current(cx);

                        let settings = session::UiSettings {
                            window_width,
                            window_height,
                            sidebar_width: ui_scale::stored_design_units(
                                Some(this.ui_scale().design_units_from_pixels(this.sidebar_width)),
                            ),
                            details_width: ui_scale::stored_design_units(
                                Some(this.ui_scale().design_units_from_pixels(this.details_width)),
                            ),
                            repo_sidebar_collapsed_items: Some(repo_sidebar_collapsed_items),
                            theme_mode: Some(this.theme_mode.key().to_string()),
                            ui_scale_percent: Some(this.ui_scale_percent),
                            ui_font_family: Some(font_preferences.ui_font_family),
                            editor_font_family: Some(font_preferences.editor_font_family),
                            use_font_ligatures: Some(font_preferences.use_font_ligatures),
                            date_time_format: Some(this.date_time_format.key().to_string()),
                            timezone: Some(this.timezone.key()),
                            show_timezone: Some(this.show_timezone),
                            change_tracking_view: Some(this.change_tracking_view.key().to_string()),
                            diff_scroll_sync: Some(this.diff_scroll_sync.key().to_string()),
                            diff_content_mode: Some(this.diff_content_mode.key().to_string()),
                            change_tracking_height,
                            untracked_height,
                            history_show_graph: Some(history_show_graph),
                            history_show_author: Some(history_show_author),
                            history_show_date: Some(history_show_date),
                            history_show_sha: Some(history_show_sha),
                            history_show_tags: Some(history_show_tags),
                            history_tag_fetch_mode: Some(if history_auto_fetch_tags_on_repo_activation
                            {
                                gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
                            } else {
                                gitcomet_state::model::GitLogTagFetchMode::Disabled
                            }),
                            default_history_mode: None,
                            git_executable_path: None,
                        };

                        Some(settings)
                    })
                    .ok()
                    .flatten();

                let Some(settings) = settings else {
                    return;
                };

                let _ = smol::unblock(move || session::persist_ui_settings(settings)).await;
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
