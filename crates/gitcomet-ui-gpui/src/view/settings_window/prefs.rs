use super::*;

impl SettingsWindowView {
    pub(super) fn persist_preferences(&self, cx: &mut gpui::Context<Self>) {
        let settings = self.preference_settings();

        cx.background_spawn(async move {
            let _ = session::persist_ui_settings(settings);
        })
        .detach();
    }

    pub(super) fn preference_settings(&self) -> session::UiSettings {
        let mut settings = session::UiSettings {
            repo_picker_sort: None,
            repo_picker_collapsed_sections: None,
            window_width: None,
            window_height: None,
            sidebar_width: None,
            details_width: None,
            sidebar_collapsed: None,
            repo_sidebar_collapsed_items: None,
            repo_sidebar_pinned_branches: None,
            theme_mode: Some(self.theme_mode.key().to_string()),
            ui_scale_percent: Some(self.ui_scale_percent),
            ui_font_family: Some(self.ui_font_family.clone()),
            editor_font_family: Some(self.editor_font_family.clone()),
            use_font_ligatures: Some(self.use_font_ligatures),
            date_time_format: Some(self.date_time_format.key().to_string()),
            timezone: Some(self.timezone.key()),
            show_timezone: Some(self.show_timezone),
            change_tracking_view: Some(self.change_tracking_view.key().to_string()),
            diff_scroll_sync: Some(self.diff_scroll_sync.key().to_string()),
            diff_content_mode: Some(self.diff_content_mode.key().to_string()),
            diff_whitespace_mode: Some(self.diff_whitespace_mode.key().to_string()),
            diff_view_mode: Some(self.diff_view_mode.key().to_string()),
            // Annotate is toggled from the diff toolbar, not the settings window,
            // so leave it untouched here (None never overwrites the stored value).
            annotate_enabled: None,
            diff_reveal_whitespace_chars: Some(self.diff_reveal_whitespace_chars),
            diff_word_wrap: Some(self.diff_word_wrap),
            diff_show_line_numbers: Some(self.diff_show_line_numbers),
            auto_save_file_edits: Some(self.auto_save_file_edits),
            // Merge tool settings are managed from the resolver's cog menu;
            // None never overwrites the stored values.
            mergetool_auto_advance: None,
            mergetool_collapse_unchanged: None,
            mergetool_output_scroll_sync: None,
            mergetool_show_line_numbers: None,
            mergetool_view_three_way: None,
            change_tracking_height: None,
            untracked_height: None,
            history_show_graph: Some(self.history_show_graph),
            history_show_author: Some(self.history_show_author),
            history_show_date: Some(self.history_show_date),
            history_show_sha: Some(self.history_show_sha),
            history_relative_dates: Some(self.history_relative_dates),
            history_highlight_commit_chain: Some(self.history_highlight_commit_chain),
            history_show_tags: Some(self.history_show_tags),
            history_tag_fetch_mode: Some(self.history_tag_fetch_mode),
            default_history_mode: Some(self.default_history_mode),
            default_tag_type: Some(self.default_tag_type),
            fetch_prune_deleted_remote_branches: Some(self.prune_deleted_remote_branches_on_fetch),
            commit_push_after_enabled: None,
            git_executable_path: Some(applied_git_executable_path(&self.runtime_info.git.runtime)),
            terminal_external_mode: None,
            terminal_external_program: None,
            terminal_external_args: None,
            terminal_action_bar_target: None,
            external_code_editor: None,
        };
        self.terminal_preferences
            .apply_to_ui_settings(&mut settings);
        settings
    }

    pub(super) fn apply_terminal_preferences_change(
        &mut self,
        next: TerminalPreferences,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.terminal_preferences == next {
            return;
        }

        self.terminal_preferences = next.clone();
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.apply_terminal_preferences(next.clone(), cx);
        });
        cx.notify();
    }

    pub(super) fn set_terminal_status(
        &mut self,
        is_error: bool,
        text: impl Into<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.terminal_status = Some(TerminalSettingsStatus {
            is_error,
            text: text.into(),
        });
        cx.notify();
    }

    pub(super) fn external_terminal_preferences_with_drafts(
        &self,
        cx: &gpui::Context<Self>,
    ) -> TerminalPreferences {
        let mut preferences = self.terminal_preferences.clone();
        preferences.external_terminal_program = self
            .terminal_external_program_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        let args_raw = self
            .terminal_external_args_input
            .read_with(cx, |input, _| input.text().to_string());
        preferences.external_terminal_args = parse_terminal_args_multiline(&args_raw);
        preferences
    }

    pub(super) fn set_external_terminal_mode(
        &mut self,
        mode: ExternalTerminalMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.terminal_preferences.external_terminal_mode == mode {
            return;
        }

        let mut next = self.terminal_preferences.clone();
        next.external_terminal_mode = mode;
        self.terminal_status = None;
        self.apply_terminal_preferences_change(next, cx);
    }

    pub(super) fn set_action_bar_terminal_target(
        &mut self,
        target: ActionBarTerminalTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.terminal_preferences.action_bar_terminal_target == target {
            return;
        }

        let mut next = self.terminal_preferences.clone();
        next.action_bar_terminal_target = target;
        self.terminal_status = None;
        self.apply_terminal_preferences_change(next, cx);
    }

    pub(super) fn save_terminal_external_draft(&mut self, cx: &mut gpui::Context<Self>) {
        let next = self.external_terminal_preferences_with_drafts(cx);
        self.apply_terminal_preferences_change(next, cx);
        self.set_terminal_status(false, "External terminal settings saved.", cx);
    }

    pub(super) fn reset_terminal_external_draft(&mut self, cx: &mut gpui::Context<Self>) {
        let program = self.terminal_preferences.external_terminal_program.clone();
        self.terminal_external_program_input
            .update(cx, |input, cx| input.set_text(program, cx));
        let args = self.terminal_preferences.external_args_multiline();
        self.terminal_external_args_input
            .update(cx, |input, cx| input.set_text(args, cx));
        self.set_terminal_status(false, "External terminal draft reset.", cx);
    }

    pub(super) fn browse_terminal_program_input(
        &mut self,
        target: TerminalProgramInputTarget,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let prompt = match target {
            TerminalProgramInputTarget::ExternalTerminal => "Select terminal launcher",
        };
        let allow_directories = cfg!(target_os = "macos");
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: allow_directories,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        let view = cx.weak_entity();

        window
            .spawn(cx, async move |cx| {
                let result = rx.await;
                let paths = match result {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return,
                    Ok(Err(_)) | Err(_) => return,
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let rendered = path.display().to_string();
                let _ = view.update(cx, |this, cx| {
                    match target {
                        TerminalProgramInputTarget::ExternalTerminal => {
                            this.terminal_external_program_input
                                .update(cx, |input, cx| input.set_text(rendered.clone(), cx));
                        }
                    }
                    this.terminal_status = None;
                    cx.notify();
                });
            })
            .detach();
    }

    pub(super) fn preferred_terminal_launch_context(
        &self,
        cx: &gpui::Context<Self>,
    ) -> ExternalTerminalLaunchContext {
        for handle in cx
            .windows()
            .into_iter()
            .filter_map(|window| window.downcast::<GitCometView>())
        {
            if let Ok(Some(context)) = handle.read_with(cx, |view, _cx| {
                view.terminal_launch_context_for_active_repo()
            }) {
                return context;
            }
        }

        ExternalTerminalLaunchContext {
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            repo_name: None,
        }
    }

    pub(super) fn test_terminal_launch_from_draft(&mut self, cx: &mut gpui::Context<Self>) {
        let preferences = self.external_terminal_preferences_with_drafts(cx);
        let context = self.preferred_terminal_launch_context(cx);
        let spec = match resolve_external_terminal_launch_spec(&preferences, &context) {
            Ok(spec) => spec,
            Err(err) => {
                self.set_terminal_status(true, format!("Test launch failed: {err}"), cx);
                return;
            }
        };

        super::platform_open::spawn_launch(
            cx,
            move || spec.launch().map_err(|err| err.to_string()),
            |this, result, cx| match result {
                Ok(()) => this.set_terminal_status(false, "Launch request sent.", cx),
                Err(err) => {
                    this.set_terminal_status(true, format!("Test launch failed: {err}"), cx)
                }
            },
        );
    }

    pub(super) fn show_root(&mut self, cx: &mut gpui::Context<Self>) {
        if self.current_view == SettingsView::Root {
            return;
        }

        self.current_view = SettingsView::Root;
        cx.notify();
    }

    pub(super) fn show_open_source_licenses(&mut self, cx: &mut gpui::Context<Self>) {
        if self.current_view == SettingsView::OpenSourceLicenses {
            return;
        }

        self.current_view = SettingsView::OpenSourceLicenses;
        self.expanded_section = None;
        cx.notify();
    }

    pub(super) fn custom_theme_folder_detail(&self) -> SharedString {
        session::user_themes_dir()
            .map(|path| path.display().to_string().into())
            .unwrap_or_else(|| "Unavailable".into())
    }

    pub(super) fn push_main_window_toast(
        &self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_main_windows(cx, move |view, _window, cx| {
            view.push_toast(kind, message.clone(), cx);
        });
    }

    pub(super) fn open_custom_theme_folder(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(path) = crate::theme::ensure_user_themes_dir_exists() else {
            self.push_main_window_toast(
                components::ToastKind::Error,
                "Custom theme folder is unavailable.".to_string(),
                cx,
            );
            return;
        };

        super::platform_open::spawn_launch(
            cx,
            move || super::platform_open::open_path_blocking(&path),
            |this, result, cx| {
                if let Err(err) = result {
                    this.push_main_window_toast(
                        components::ToastKind::Error,
                        format!("Failed to open custom theme folder: {err}"),
                        cx,
                    );
                }
            },
        );
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

        self.ui_scale_percent = percent;
        ui_scale::apply_to_window(window, percent);
        crate::app::ensure_window_respects_min_size(
            window,
            settings_window_min_size_for_percent(percent),
        );
        cx.notify();
    }

    pub(super) fn update_main_windows(
        &self,
        cx: &mut gpui::Context<Self>,
        f: impl FnMut(&mut GitCometView, &mut Window, &mut gpui::Context<GitCometView>) + 'static,
    ) {
        let handles: Vec<_> = cx
            .windows()
            .into_iter()
            .filter_map(|window| window.downcast::<GitCometView>())
            .collect();
        cx.spawn(
            async move |_view: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.update(move |cx| {
                    let mut f = f;
                    for handle in handles {
                        let _ = handle.update(cx, |view, window, cx| f(view, window, cx));
                    }
                });
            },
        )
        .detach();
    }
}

impl SettingsWindowView {
    pub(super) fn external_editor_is_custom(&self) -> bool {
        matches!(
            self.external_editor_setting,
            Some(ExternalCodeEditorSetting::Custom { .. })
        )
    }

    pub(super) fn custom_external_editor_setting_from_drafts(&self) -> ExternalCodeEditorSetting {
        let executable = self.external_editor_custom_path_draft.trim();
        let arguments = self.external_editor_custom_arguments_draft.trim();
        ExternalCodeEditorSetting::Custom {
            executable: if executable.is_empty() {
                PathBuf::new()
            } else {
                PathBuf::from(executable)
            },
            arguments: (!arguments.is_empty()).then(|| arguments.to_string()),
        }
    }

    pub(super) fn persist_external_editor_preference(&self, cx: &mut gpui::Context<Self>) {
        let setting = self.external_editor_setting.clone();
        crate::external_editor::set_configured_setting_override(setting.clone());
        let persist_queue = external_editor_preference_persist_queue().clone();
        let sequence = persist_queue.next_sequence();
        let setting_for_persist = setting.clone();
        cx.background_spawn(async move {
            let _ = persist_queue.persist_if_latest(sequence, setting_for_persist);
        })
        .detach();
        cx.defer(move |cx| {
            crate::app::refresh_external_editor_app_surfaces_for_setting(setting.as_ref(), cx);
        });
    }

    pub(super) fn apply_browsed_external_editor_path(
        &mut self,
        path: PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = path.display().to_string();
        self.external_editor_custom_path_draft = next.clone();
        self.external_editor_custom_path_input
            .update(cx, |input, cx| input.set_text(next, cx));
        self.persist_external_editor_from_custom_drafts(cx);
        self.notify_after_external_editor_browse(cx);
    }

    pub(super) fn notify_after_external_editor_browse(&mut self, cx: &mut gpui::Context<Self>) {
        #[cfg(test)]
        {
            self.external_editor_browse_notify_count += 1;
        }
        cx.notify();
    }

    pub(super) fn persist_external_editor_from_custom_drafts(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = self.custom_external_editor_setting_from_drafts();
        if self.external_editor_setting.as_ref() == Some(&next) {
            return;
        }
        self.external_editor_setting = Some(next);
        self.persist_external_editor_preference(cx);
    }

    pub(super) fn set_external_editor_setting(
        &mut self,
        next: Option<ExternalCodeEditorSetting>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.external_editor_setting == next {
            self.expanded_section = None;
            cx.notify();
            return;
        }

        self.external_editor_setting = next;
        self.expanded_section = None;
        self.persist_external_editor_preference(cx);
        cx.notify();
    }

    pub(super) fn select_custom_external_editor(&mut self, cx: &mut gpui::Context<Self>) {
        self.set_external_editor_setting(
            Some(self.custom_external_editor_setting_from_drafts()),
            cx,
        );
    }

    pub(super) fn font_option_detail(&self, family: &str) -> Option<SharedString> {
        match family {
            crate::font_preferences::UI_SYSTEM_FONT_FAMILY => {
                Some("Use GitComet's best match for the operating system UI font stack".into())
            }
            _ => None,
        }
    }

    pub(super) fn font_options_hint(&self, family: &str) -> SharedString {
        self.font_option_detail(family)
            .unwrap_or_else(|| "Choose from installed system fonts".into())
    }

    pub(super) fn font_option_row_for_family(
        &self,
        id_prefix: &'static str,
        ix: usize,
        family: &str,
        selected: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        self.option_row(
            format!("{id_prefix}_{ix}"),
            crate::font_preferences::display_label(family),
            None,
            selected,
            theme,
        )
    }

    pub(super) fn set_ui_scale_percent(
        &mut self,
        percent: u32,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let percent = ui_scale::set_current(cx, percent).percent;
        if self.ui_scale_percent == percent {
            return;
        }

        self.expanded_section = None;
        self.apply_ui_scale_percent(percent, window, cx);
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, root_window, cx| {
            view.apply_ui_scale_percent(percent, root_window, cx);
        });
        cx.notify();
    }

    pub(super) fn set_theme_mode(
        &mut self,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == mode {
            return;
        }

        self.theme_mode = mode.clone();
        self.theme = mode.resolve_theme(window.appearance());
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, root_window, cx| {
            view.popover_host.update(cx, |host, cx| {
                host.set_theme_mode(mode.clone(), root_window.appearance(), cx);
            });
        });
        cx.notify();
    }

    pub(super) fn set_ui_font_family(&mut self, family: String, cx: &mut gpui::Context<Self>) {
        if self.ui_font_family == family {
            return;
        }

        self.ui_font_family = family;
        self.expanded_section = None;
        crate::font_preferences::set_current(
            cx,
            self.ui_font_family.clone(),
            self.editor_font_family.clone(),
            self.use_font_ligatures,
        );
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.notify_font_preferences_changed(cx);
        });
        cx.notify();
    }

    pub(super) fn set_editor_font_family(&mut self, family: String, cx: &mut gpui::Context<Self>) {
        if self.editor_font_family == family {
            return;
        }

        self.editor_font_family = family;
        self.expanded_section = None;
        crate::font_preferences::set_current(
            cx,
            self.ui_font_family.clone(),
            self.editor_font_family.clone(),
            self.use_font_ligatures,
        );
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.notify_font_preferences_changed(cx);
        });
        cx.notify();
    }

    pub(super) fn set_use_font_ligatures(&mut self, enabled: bool, cx: &mut gpui::Context<Self>) {
        if self.use_font_ligatures == enabled {
            return;
        }

        self.use_font_ligatures = enabled;
        crate::font_preferences::set_current(
            cx,
            self.ui_font_family.clone(),
            self.editor_font_family.clone(),
            self.use_font_ligatures,
        );
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.notify_font_preferences_changed(cx);
        });
        cx.notify();
    }

    pub(super) fn set_date_time_format(
        &mut self,
        format: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == format {
            return;
        }

        self.date_time_format = format;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_date_time_format_preference(format, cx);
        });
        cx.notify();
    }

    pub(super) fn set_timezone(&mut self, timezone: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == timezone {
            return;
        }

        self.timezone = timezone;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_timezone_preference(timezone, cx);
        });
        cx.notify();
    }

    pub(super) fn set_show_timezone(&mut self, enabled: bool, cx: &mut gpui::Context<Self>) {
        if self.show_timezone == enabled {
            return;
        }

        self.show_timezone = enabled;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_show_timezone_preference(enabled, cx);
        });
        cx.notify();
    }

    pub(super) fn set_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_change_tracking_view(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_scroll_sync(
        &mut self,
        next: DiffScrollSync,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_scroll_sync == next {
            return;
        }

        self.diff_scroll_sync = next;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_scroll_sync(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_content_mode(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_content_mode == next {
            return;
        }

        self.diff_content_mode = next;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_content_mode(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_whitespace_mode == next {
            return;
        }

        self.diff_whitespace_mode = next;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_whitespace_mode(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_view_mode(&mut self, next: DiffViewMode, cx: &mut gpui::Context<Self>) {
        if self.diff_view_mode == next {
            return;
        }

        self.diff_view_mode = next;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_view_mode(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_reveal_whitespace_chars(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_reveal_whitespace_chars == next {
            return;
        }

        self.diff_reveal_whitespace_chars = next;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_reveal_whitespace_chars(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_word_wrap(&mut self, next: bool, cx: &mut gpui::Context<Self>) {
        if self.diff_word_wrap == next {
            return;
        }

        self.diff_word_wrap = next;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_word_wrap(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_diff_show_line_numbers(&mut self, next: bool, cx: &mut gpui::Context<Self>) {
        if self.diff_show_line_numbers == next {
            return;
        }

        self.diff_show_line_numbers = next;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_diff_show_line_numbers(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_auto_save_file_edits(&mut self, next: bool, cx: &mut gpui::Context<Self>) {
        if self.auto_save_file_edits == next {
            return;
        }

        self.auto_save_file_edits = next;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_auto_save_file_edits(next, cx);
        });
        cx.notify();
    }

    pub(super) fn set_history_column_preferences(
        &mut self,
        show_graph: bool,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_show_graph == show_graph
            && self.history_show_author == show_author
            && self.history_show_date == show_date
            && self.history_show_sha == show_sha
        {
            return;
        }

        self.history_show_graph = show_graph;
        self.history_show_author = show_author;
        self.history_show_date = show_date;
        self.history_show_sha = show_sha;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_history_column_preferences(show_graph, show_author, show_date, show_sha, cx);
        });
        cx.notify();
    }

    pub(super) fn set_history_highlight_commit_chain(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_highlight_commit_chain == enabled {
            return;
        }
        self.history_highlight_commit_chain = enabled;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_history_highlight_commit_chain(enabled, cx);
        });
        cx.notify();
    }

    pub(super) fn set_history_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_relative_dates == enabled {
            return;
        }

        self.history_relative_dates = enabled;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_history_relative_dates(enabled, cx);
        });
        cx.notify();
    }

    pub(super) fn set_history_show_tags(&mut self, enabled: bool, cx: &mut gpui::Context<Self>) {
        if self.history_show_tags == enabled {
            return;
        }

        self.history_show_tags = enabled;
        if !enabled && self.expanded_section == Some(SettingsSection::GitLogTagFetch) {
            self.expanded_section = None;
        }
        let tag_fetch_mode = self.history_tag_fetch_mode;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_history_tag_preferences(enabled, tag_fetch_mode, cx);
        });
        cx.notify();
    }

    pub(super) fn set_history_tag_fetch_mode(
        &mut self,
        mode: GitLogTagFetchMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_tag_fetch_mode == mode {
            return;
        }

        self.history_tag_fetch_mode = mode;
        self.expanded_section = None;
        let show_tags = self.history_show_tags;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_history_tag_preferences(show_tags, mode, cx);
        });
        cx.notify();
    }

    pub(super) fn set_default_history_mode(
        &mut self,
        mode: HistoryMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.default_history_mode == mode {
            return;
        }

        self.default_history_mode = mode;
        self.expanded_section = None;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_default_history_mode_preference(mode, cx);
        });
        cx.notify();
    }

    pub(super) fn set_default_tag_type(
        &mut self,
        tag_type: DefaultTagType,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.default_tag_type == tag_type {
            return;
        }

        self.default_tag_type = tag_type;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_default_tag_type_preference(tag_type, cx);
        });
        cx.notify();
    }

    pub(super) fn set_prune_deleted_remote_branches_on_fetch(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.prune_deleted_remote_branches_on_fetch == enabled {
            return;
        }

        self.prune_deleted_remote_branches_on_fetch = enabled;
        self.persist_preferences(cx);
        self.update_main_windows(cx, move |view, _window, cx| {
            view.set_remote_prune_preference(enabled, cx);
        });
        cx.notify();
    }
}
