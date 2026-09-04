use super::*;

impl GitCometView {
    pub(in crate::view) fn set_remote_markdown_image_policy(
        &mut self,
        next: RemoteMarkdownImagePolicy,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.remote_markdown_image_policy == next {
            return;
        }
        self.remote_markdown_image_policy = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.security.remote_markdown_images = next;
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.set_remote_markdown_image_policy(next, cx);
        });
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_check_for_updates_on_startup(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.check_for_updates_on_startup == next {
            return;
        }
        self.check_for_updates_on_startup = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.security.check_for_updates_on_startup = next;
        });
        self.schedule_ui_settings_persist(cx);
    }

    pub(super) fn ui_scale(&self) -> ui_scale::UiScale {
        ui_scale::UiScale::from_percent(self.ui_scale_percent)
    }

    pub(in crate::view) fn update_ui_preferences(
        &self,
        cx: &mut gpui::Context<Self>,
        update: impl FnOnce(&mut UiPreferences) + 'static,
    ) {
        self.ui_model
            .update(cx, |model, _cx| model.update_preferences(update));
    }

    pub(super) fn sync_cached_pane_widths_from_design(&mut self) {
        let scale = self.ui_scale();
        self.sidebar_width = scale.px(self.sidebar_width_design);
        self.details_width = scale.px(self.details_width_design);
    }

    pub(super) fn set_sidebar_width_from_pixels(&mut self, width: Pixels) {
        self.sidebar_width = width;
        self.sidebar_width_design = self.ui_scale().design_units_from_pixels(width);
    }

    pub(super) fn set_details_width_from_pixels(&mut self, width: Pixels) {
        self.details_width = width;
        self.details_width_design = self.ui_scale().design_units_from_pixels(width);
    }

    pub(super) fn scaled_px(&self, value: f32) -> Pixels {
        self.ui_scale().px(value)
    }

    pub(super) fn pane_collapsed_width(&self) -> Pixels {
        self.scaled_px(PANE_COLLAPSED_PX)
    }

    pub(super) fn main_min_width(&self) -> Pixels {
        self.scaled_px(MAIN_MIN_PX)
    }

    pub(super) fn sidebar_min_width(&self) -> Pixels {
        self.scaled_px(SIDEBAR_MIN_PX)
    }

    pub(super) fn details_min_width(&self) -> Pixels {
        self.scaled_px(DETAILS_MIN_PX)
    }

    pub(super) fn pane_resize_handle_width(&self) -> Pixels {
        self.scaled_px(PANE_RESIZE_HANDLE_PX)
    }

    pub(crate) fn apply_ui_scale_percent(
        &mut self,
        percent: u32,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let percent = ui_scale::sanitize_percent(Some(percent));
        if self.ui_scale_percent == percent {
            return;
        }

        let previous_percent = self.ui_scale_percent;
        let scale = self.ui_scale();
        self.sidebar_width_design = scale.design_units_from_pixels(self.sidebar_width);
        self.details_width_design = scale.design_units_from_pixels(self.details_width);
        self.ui_scale_percent = percent;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.appearance.ui_scale_percent = percent;
        });
        self.pane_resize = None;
        self.sidebar_width_anim_seq = self.sidebar_width_anim_seq.wrapping_add(1);
        self.details_width_anim_seq = self.details_width_anim_seq.wrapping_add(1);
        self.sidebar_width_animating = false;
        self.details_width_animating = false;

        ui_scale::apply_to_window(window, percent);
        crate::app::ensure_window_respects_min_size(
            window,
            crate::app::main_window_min_size_for_percent(percent),
        );

        self.last_window_size = window.viewport_size();
        self.ui_window_size_last_seen = self.last_window_size;
        self.sync_cached_pane_widths_from_design();

        let change_tracking_view = self.change_tracking_view;
        self.details_pane.update(cx, |pane, cx| {
            pane.apply_ui_scale_percent(previous_percent, percent, change_tracking_view, cx);
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.apply_ui_scale_percent(previous_percent, percent, cx);
        });
        self.reflog_pane.update(cx, |pane, cx| {
            pane.set_ui_scale_percent(percent, cx);
        });
        self.popover_host.update(cx, |_host, cx| {
            cx.notify();
        });

        self.clamp_pane_widths_to_window();
        self.notify_font_preferences_changed(cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(super) fn set_theme_mode(
        &mut self,
        mode: ThemeMode,
        appearance: gpui::WindowAppearance,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == mode {
            return;
        }

        self.theme_mode = mode.clone();
        let shared_mode = mode.clone();
        self.update_ui_preferences(cx, move |preferences| {
            preferences.appearance.theme_mode = shared_mode;
        });
        self.set_theme(mode.resolve_theme(appearance), cx);
        self.schedule_ui_settings_persist(cx);
    }

    fn sync_date_preferences_to_children(&mut self, cx: &mut gpui::Context<Self>) {
        let format = self.date_time_format;
        let timezone = self.timezone;
        let show_timezone = self.show_timezone;
        self.main_pane.update(cx, |pane, cx| {
            pane.set_date_time_format(format, cx);
            pane.set_timezone(timezone, cx);
            pane.set_show_timezone(show_timezone, cx);
        });
        self.details_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx)
        });
        self.reflog_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx)
        });
        self.popover_host.update(cx, |host, cx| {
            host.set_date_time_format(format, cx);
            host.set_timezone(timezone, cx);
            host.set_show_timezone(show_timezone, cx);
        });
    }

    pub(in crate::view) fn set_date_time_format_preference(
        &mut self,
        format: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == format {
            return;
        }
        self.date_time_format = format;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.appearance.date_time_format = format;
        });
        self.sync_date_preferences_to_children(cx);
    }

    pub(in crate::view) fn set_timezone_preference(
        &mut self,
        timezone: Timezone,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.timezone == timezone {
            return;
        }
        self.timezone = timezone;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.appearance.timezone = timezone;
        });
        self.sync_date_preferences_to_children(cx);
    }

    pub(in crate::view) fn set_show_timezone_preference(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.show_timezone == enabled {
            return;
        }
        self.show_timezone = enabled;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.appearance.show_timezone = enabled;
        });
        self.sync_date_preferences_to_children(cx);
    }

    pub(in crate::view) fn set_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.change_tracking.view = next;
        });
        self.details_pane
            .update(cx, |pane, cx| pane.set_change_tracking_view(next, cx));
        self.popover_host
            .update(cx, |host, cx| host.sync_change_tracking_view(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_commit_push_after_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_push_after_enabled == enabled {
            return;
        }

        self.commit_push_after_enabled = enabled;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.repository.commit_push_after_enabled = enabled;
        });
        self.details_pane.update(cx, |pane, cx| {
            pane.set_commit_push_after_enabled(enabled, cx)
        });
        self.popover_host.update(cx, |host, cx| {
            host.sync_commit_push_after_enabled(enabled, cx)
        });
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_commit_amend_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.details_pane
            .update(cx, |pane, cx| pane.set_commit_amend_enabled(enabled, cx));
        self.popover_host
            .update(cx, |host, cx| host.sync_commit_amend_enabled(enabled, cx));
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
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.scroll_sync = next;
        });
        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_scroll_sync(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_diff_view_mode(
        &mut self,
        next: DiffViewMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_view_mode == next {
            return;
        }

        self.diff_view_mode = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.view_mode = next;
        });
        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_view_mode(next, cx));
        self.schedule_ui_settings_persist(cx);
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
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.annotate_enabled = next;
        });
        // Blame is an annotation column, not a view mode: it renders in the left
        // column in Split (see `rows::diff`, `annotation_active() && is_left`)
        // just as it does in Inline, and the wrap widths already account for it
        // in both. Toggling it must leave the selected mode alone.
        self.main_pane
            .update(cx, |pane, cx| pane.set_annotate_enabled(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(super) fn apply_diff_content_mode_preference(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_content_mode == next {
            return false;
        }

        self.diff_content_mode = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.content_mode = next;
        });
        self.popover_host
            .update(cx, |host, cx| host.sync_diff_content_mode(next, cx));
        self.schedule_ui_settings_persist(cx);
        true
    }

    // MainPaneView sometimes owns the active GPUI update when the diff-header
    // toggle is clicked, so syncing the root preference must not call back into
    // `main_pane.update(...)`.
    pub(in crate::view) fn sync_diff_content_mode_from_pane(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.apply_diff_content_mode_preference(next, cx);
    }

    pub(in crate::view) fn set_diff_content_mode(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.apply_diff_content_mode_preference(next, cx) {
            return;
        }

        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_content_mode(next, cx));
    }

    pub(super) fn apply_diff_whitespace_mode_preference(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_whitespace_mode == next {
            return false;
        }

        self.diff_whitespace_mode = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.whitespace_mode = next;
        });
        self.popover_host
            .update(cx, |host, cx| host.sync_diff_whitespace_mode(next, cx));
        self.schedule_ui_settings_persist(cx);
        true
    }

    pub(in crate::view) fn sync_diff_whitespace_mode_from_pane(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.apply_diff_whitespace_mode_preference(next, cx);
    }

    pub(in crate::view) fn set_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.apply_diff_whitespace_mode_preference(next, cx) {
            return;
        }

        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_whitespace_mode(next, cx));
    }

    pub(super) fn apply_diff_reveal_whitespace_chars_preference(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_reveal_whitespace_chars == next {
            return false;
        }

        self.diff_reveal_whitespace_chars = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.reveal_whitespace_chars = next;
        });
        self.popover_host.update(cx, |host, cx| {
            host.sync_diff_reveal_whitespace_chars(next, cx)
        });
        self.schedule_ui_settings_persist(cx);
        true
    }

    pub(in crate::view) fn sync_diff_reveal_whitespace_chars_from_pane(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.apply_diff_reveal_whitespace_chars_preference(next, cx);
    }

    pub(in crate::view) fn set_diff_reveal_whitespace_chars(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.apply_diff_reveal_whitespace_chars_preference(next, cx) {
            return;
        }

        self.main_pane.update(cx, |pane, cx| {
            pane.set_diff_reveal_whitespace_chars(next, cx)
        });
    }

    pub(super) fn apply_diff_word_wrap_preference(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_word_wrap == next {
            return false;
        }

        self.diff_word_wrap = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.word_wrap = next;
        });
        self.popover_host
            .update(cx, |host, cx| host.sync_diff_word_wrap(next, cx));
        self.schedule_ui_settings_persist(cx);
        true
    }

    pub(in crate::view) fn sync_diff_word_wrap_from_pane(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.apply_diff_word_wrap_preference(next, cx);
    }

    pub(in crate::view) fn set_diff_word_wrap(&mut self, next: bool, cx: &mut gpui::Context<Self>) {
        if !self.apply_diff_word_wrap_preference(next, cx) {
            return;
        }

        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_word_wrap(next, cx));
    }

    pub(super) fn apply_diff_show_line_numbers_preference(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.diff_show_line_numbers == next {
            return false;
        }

        self.diff_show_line_numbers = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.diff.show_line_numbers = next;
        });
        self.popover_host
            .update(cx, |host, cx| host.sync_diff_show_line_numbers(next, cx));
        self.schedule_ui_settings_persist(cx);
        true
    }

    pub(in crate::view) fn sync_diff_show_line_numbers_from_pane(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.apply_diff_show_line_numbers_preference(next, cx);
    }

    pub(in crate::view) fn set_diff_show_line_numbers(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.apply_diff_show_line_numbers_preference(next, cx) {
            return;
        }

        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_show_line_numbers(next, cx));
    }

    /// Show the file the main pane has open in the sidebar's file explorer,
    /// expanding the folders on the way to it and scrolling it into view.
    ///
    /// Switches the sidebar to Files when it is showing Branches — the action is
    /// reachable from the menu, the palette and a shortcut, so the tree it acts
    /// on may not even be visible.
    pub(crate) fn locate_open_file_in_explorer(&mut self, cx: &mut gpui::Context<Self>) {
        if self.sidebar_collapsed {
            self.set_sidebar_collapsed(false, cx);
        }
        self.sidebar_pane
            .update(cx, |pane, cx| pane.locate_open_file(cx));
        cx.notify();
    }

    /// Mirrors the settings window's auto-save toggle into the pane that owns
    /// the file editor. The main window never writes this back (the settings
    /// window is the only writer), so there is no persist call here.
    pub(in crate::view) fn set_auto_save_file_edits(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.auto_save_file_edits == next {
            return;
        }
        self.auto_save_file_edits = next;
        self.update_ui_preferences(cx, move |preferences| {
            preferences.file_editing.auto_save = next;
        });
        self.main_pane
            .update(cx, |pane, cx| pane.set_auto_save_file_edits(next, cx));
    }

    pub(in crate::view) fn set_history_column_preferences(
        &mut self,
        show_graph: bool,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_ui_preferences(cx, move |preferences| {
            preferences.history.show_graph = show_graph;
            preferences.history.show_author = show_author;
            preferences.history.show_date = show_date;
            preferences.history.show_sha = show_sha;
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.set_history_column_preferences(show_graph, show_author, show_date, show_sha, cx);
        });
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn reset_history_column_widths(&mut self, cx: &mut gpui::Context<Self>) {
        self.main_pane
            .update(cx, |pane, cx| pane.reset_history_column_widths(cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_history_highlight_commit_chain(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_ui_preferences(cx, move |preferences| {
            preferences.history.highlight_commit_chain = enabled;
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.set_history_highlight_commit_chain(enabled, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn set_history_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_ui_preferences(cx, move |preferences| {
            preferences.history.relative_dates = enabled;
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.set_history_relative_dates(enabled, cx);
        });
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_history_tag_preferences(
        &mut self,
        show_tags: bool,
        tag_fetch_mode: gitcomet_state::model::GitLogTagFetchMode,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_ui_preferences(cx, move |preferences| {
            preferences.history.show_tags = show_tags;
            preferences.history.tag_fetch_mode = tag_fetch_mode;
        });
        let auto_fetch_tags_on_repo_activation = matches!(
            tag_fetch_mode,
            gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
        );
        self.main_pane.update(cx, |pane, cx| {
            pane.set_history_tag_preferences(show_tags, auto_fetch_tags_on_repo_activation, cx);
        });
        self.store.dispatch(Msg::SetGitLogSettings {
            show_history_tags: show_tags,
            tag_fetch_mode,
        });
        if show_tags
            && auto_fetch_tags_on_repo_activation
            && let Some(repo) = self.main_pane.read(cx).active_repo()
        {
            if matches!(repo.tags, Loadable::NotLoaded | Loadable::Error(_)) {
                self.store.dispatch(Msg::LoadTags { repo_id: repo.id });
            }
            if matches!(repo.remote_tags, Loadable::NotLoaded | Loadable::Error(_)) {
                self.store
                    .dispatch(Msg::LoadRemoteTags { repo_id: repo.id });
            }
        }
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_default_tag_type_preference(
        &mut self,
        tag_type: DefaultTagType,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_ui_preferences(cx, move |preferences| {
            preferences.repository.default_tag_type = tag_type;
        });
        self.store.dispatch(Msg::SetDefaultTagType(tag_type));
    }

    pub(in crate::view) fn set_default_history_mode_preference(
        &mut self,
        mode: gitcomet_core::domain::HistoryMode,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_ui_preferences(cx, move |preferences| {
            preferences.history.default_mode = mode;
        });
    }
}
