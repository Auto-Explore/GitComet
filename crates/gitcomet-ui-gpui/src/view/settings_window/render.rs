use super::*;

impl Render for SettingsWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        self.terminal_external_program_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.terminal_external_args_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        let decorations = window.window_decorations();
        let show_custom_window_chrome =
            crate::linux_gui_env::LinuxGuiEnvironment::should_render_custom_window_chrome(
                decorations,
            );
        let (tiling, client_inset) = match decorations {
            Decorations::Client { tiling } => (
                Some(tiling),
                settings_window_client_inset_for_scale(self.ui_scale_percent),
            ),
            Decorations::Server => (None, px(0.0)),
        };
        window.set_client_inset(client_inset);

        let cursor = self
            .hover_resize_edge
            .map(chrome::cursor_style_for_resize_edge)
            .unwrap_or(CursorStyle::Arrow);
        let is_macos = cfg!(target_os = "macos");
        let header_bg = if window.is_window_active() {
            with_alpha(
                theme.colors.surface.panel,
                if theme.is_dark { 0.98 } else { 0.94 },
            )
        } else {
            theme.colors.surface.panel
        };
        let header_border = if window.is_window_active() {
            theme.colors.stroke.default
        } else {
            with_alpha(theme.colors.stroke.default, 0.7)
        };

        let drag_region = div()
            .id("settings_window_header_drag")
            .debug_selector(|| "settings_window_header_drag".to_string())
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .px(px(12.0))
            .window_control_area(WindowControlArea::Drag)
            .when(is_macos, |this| {
                this.pl(settings_window_traffic_lights_safe_inset(
                    self.ui_scale_percent,
                ))
            })
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                if !chrome::should_handle_titlebar_double_click(e.click_count(), e.standard_click())
                {
                    return;
                }

                this.title_drag_state.clear();
                cx.stop_propagation();
                chrome::handle_titlebar_double_click(window);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|_this, e: &MouseUpEvent, window, cx| {
                    chrome::show_titlebar_secondary_menu(e.position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _window, cx| {
                    this.title_drag_state.on_left_mouse_down(e.click_count);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, _e, window, _cx| {
                if this.title_drag_state.take_move_request() {
                    crate::app::begin_window_move(window);
                }
            }))
            .child(
                div()
                    .overflow_hidden()
                    .text_size(px(13.0))
                    .line_height(px(16.0))
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .child(SETTINGS_WINDOW_TITLE),
            );

        let min = chrome::titlebar_control_button(
            self.ui_scale_percent,
            "settings_window_min_btn",
            "icons/generic_minimize.svg",
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("settings_window_min")
        .debug_selector(|| "settings_window_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            window.minimize_window();
        }));

        let max_icon = if window.is_maximized() {
            "icons/generic_restore.svg"
        } else {
            "icons/generic_maximize.svg"
        };
        let max = chrome::titlebar_control_button(
            self.ui_scale_percent,
            "settings_window_max_btn",
            max_icon,
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("settings_window_max")
        .debug_selector(|| "settings_window_max".to_string())
        .window_control_area(WindowControlArea::Max)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            crate::app::toggle_window_zoom(window);
            cx.notify();
        }));

        let close = chrome::titlebar_control_button(
            self.ui_scale_percent,
            "settings_window_close_btn",
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            theme.colors.status.danger.foreground,
        )
        .id("settings_window_close_btn")
        .debug_selector(|| "settings_window_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .on_click(cx.listener(|_this, _e: &ClickEvent, window, cx| {
            cx.stop_propagation();
            crate::app::mark_clean_shutdown_if_last_window_from_view(cx);
            window.remove_window();
        }));

        let frame_rounding = chrome::client_frame_corner_rounding(theme, window);
        let header = div()
            .id("settings_window_header")
            .h(chrome::title_bar_height(self.ui_scale_percent))
            .w_full()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(header_border)
            .bg(header_bg)
            .when_some(
                chrome::client_frame_corner_rounding(theme, window),
                |d, rounding| {
                    d.when(rounding.top_left, |d| d.rounded_tl(rounding.radius))
                        .when(rounding.top_right, |d| d.rounded_tr(rounding.radius))
                },
            )
            .child(drag_region)
            .when(!is_macos, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pr_2()
                        .child(min)
                        .child(max)
                        .child(close),
                )
            });

        self.git_executable_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.external_editor_custom_path_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.external_editor_custom_arguments_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.search_input
            .update(cx, |input, cx| input.set_theme(theme, cx));

        #[cfg(test)]
        let show_overflow_probe =
            self.overflow_probe && matches!(self.current_view, SettingsView::Root);
        #[cfg(not(test))]
        let show_overflow_probe = false;

        let content = if show_overflow_probe {
            self.overflow_probe_content(theme).into_any_element()
        } else {
            match self.current_view {
                SettingsView::Root => {
                    let no_separator = gpui::rgba(0x00000000);
                    let theme_row = self
                        .summary_row(
                            "settings_window_theme",
                            "Theme",
                            self.theme_mode.label().into(),
                            self.expanded_section == Some(SettingsSection::Theme),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::Theme, cx);
                        }));

                    let date_format_row = self
                        .summary_row(
                            "settings_window_date_format",
                            "Date format",
                            self.date_time_format.label().into(),
                            self.expanded_section == Some(SettingsSection::DateFormat),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::DateFormat, cx);
                        }));

                    let ui_scale_row = self
                        .summary_row(
                            "settings_window_ui_scale",
                            "UI scale",
                            ui_scale::label(self.ui_scale_percent).into(),
                            self.expanded_section == Some(SettingsSection::UiScale),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::UiScale, cx);
                        }));

                    let ui_font_row = self
                        .summary_row(
                            "settings_window_ui_font",
                            "UI Font",
                            crate::font_preferences::display_label(&self.ui_font_family).into(),
                            self.expanded_section == Some(SettingsSection::UiFont),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::UiFont, cx);
                        }));

                    let editor_font_row = self
                        .summary_row(
                            "settings_window_editor_font",
                            "Editor Font",
                            crate::font_preferences::display_label(&self.editor_font_family).into(),
                            self.expanded_section == Some(SettingsSection::EditorFont),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::EditorFont, cx);
                        }));

                    let font_ligatures_row = self
                        .toggle_row(
                            "settings_window_use_font_ligatures",
                            "Use font ligatures",
                            self.use_font_ligatures,
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_use_font_ligatures(!this.use_font_ligatures, cx);
                        }));

                    let external_editor_row = self
                        .summary_row(
                            "settings_window_external_code_editor",
                            "External code editor",
                            crate::external_editor::label_for_setting(
                                self.external_editor_setting.as_ref(),
                            )
                            .into(),
                            self.expanded_section == Some(SettingsSection::ExternalCodeEditor),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::ExternalCodeEditor, cx);
                        }));

                    let timezone_row = self
                        .summary_row(
                            "settings_window_timezone",
                            "Date timezone",
                            self.timezone.label().into(),
                            self.expanded_section == Some(SettingsSection::Timezone),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::Timezone, cx);
                        }));

                    let show_timezone_row = self
                        .toggle_row(
                            "settings_window_show_timezone",
                            "Show timezone",
                            self.show_timezone,
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_show_timezone(!this.show_timezone, cx);
                        }));

                    let terminal_external_row = self
                        .summary_row(
                            "settings_window_terminal_external",
                            "External terminal",
                            self.terminal_preferences.external_summary().into(),
                            self.expanded_section == Some(SettingsSection::TerminalExternal),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::TerminalExternal, cx);
                        }));

                    let terminal_action_bar_row = self
                        .summary_row(
                            "settings_window_terminal_action_bar",
                            "Action bar terminal button opens",
                            self.terminal_preferences
                                .action_bar_terminal_target
                                .label()
                                .into(),
                            self.expanded_section == Some(SettingsSection::TerminalActionBar),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::TerminalActionBar, cx);
                        }));

                    let change_tracking_row = self
                        .summary_row(
                            "settings_window_change_tracking",
                            "Untracked files",
                            self.change_tracking_view.settings_label().into(),
                            self.expanded_section == Some(SettingsSection::ChangeTracking),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::ChangeTracking, cx);
                        }));

                    let diff_scroll_sync_row = self
                        .summary_row(
                            "settings_window_diff_scroll_sync",
                            "Scroll sync",
                            self.diff_scroll_sync.label().into(),
                            self.expanded_section == Some(SettingsSection::Diff),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::Diff, cx);
                        }));

                    let diff_content_mode_row = self
                        .summary_row(
                            "settings_window_diff_content_mode",
                            "Diff mode",
                            self.diff_content_mode.settings_label().into(),
                            self.expanded_section == Some(SettingsSection::DiffContentMode),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::DiffContentMode, cx);
                        }));

                    let diff_whitespace_mode_row = self
                        .toggle_row(
                            "settings_window_diff_whitespace_mode",
                            "Show whitespace changes",
                            self.diff_whitespace_mode == DiffWhitespaceMode::Show,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_diff_whitespace_mode(this.diff_whitespace_mode.toggled(), cx);
                        }));

                    let diff_reveal_whitespace_chars_row = self
                        .toggle_row(
                            "settings_window_diff_reveal_whitespace_chars",
                            "Reveal whitespace characters",
                            self.diff_reveal_whitespace_chars,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_diff_reveal_whitespace_chars(
                                !this.diff_reveal_whitespace_chars,
                                cx,
                            );
                        }));

                    let diff_word_wrap_row = self
                        .toggle_row(
                            "settings_window_diff_word_wrap",
                            "Word wrap",
                            self.diff_word_wrap,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_diff_word_wrap(!this.diff_word_wrap, cx);
                        }));

                    let diff_show_line_numbers_row = self
                        .toggle_row(
                            "settings_window_diff_show_line_numbers",
                            "Show line numbers",
                            self.diff_show_line_numbers,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_diff_show_line_numbers(!this.diff_show_line_numbers, cx);
                        }));

                    let history_default_mode_row = self
                        .summary_row(
                            "settings_window_git_log_default_mode",
                            "Default history mode",
                            crate::view::history_mode::history_mode_label(
                                self.default_history_mode,
                            )
                            .into(),
                            self.expanded_section == Some(SettingsSection::GitLogDefaultMode),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::GitLogDefaultMode, cx);
                        }));

                    let history_columns_row = self
                        .summary_row(
                            "settings_window_git_log_columns",
                            "History columns",
                            history_columns_settings_label(
                                self.history_show_graph,
                                self.history_show_author,
                                self.history_show_date,
                                self.history_show_sha,
                            ),
                            self.expanded_section == Some(SettingsSection::GitLogColumns),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::GitLogColumns, cx);
                        }));

                    // "Lane", not "chain": what this dims is every lane but the
                    // selected commit's own. A merge's second parent sits on a
                    // lane of its own and washes out with the rest, so the old
                    // label promised an ancestry walk the graph no longer does.
                    let highlight_commit_chain_row = self
                        .toggle_row(
                            "settings_window_git_log_highlight_commit_chain",
                            "Highlight selected commit lane",
                            self.history_highlight_commit_chain,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_history_highlight_commit_chain(
                                !this.history_highlight_commit_chain,
                                cx,
                            );
                        }));

                    let relative_dates_row = self
                        .toggle_row(
                            "settings_window_git_log_relative_dates",
                            "Relative dates in history view",
                            self.history_relative_dates,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_history_relative_dates(!this.history_relative_dates, cx);
                        }));

                    let show_history_tags_row = self
                        .toggle_row(
                            "settings_window_git_log_show_tags",
                            "Show tags in history view",
                            self.history_show_tags,
                            theme,
                        )
                        .border_color(if self.history_show_tags {
                            settings_row_separator_color(theme)
                        } else {
                            no_separator
                        })
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_history_show_tags(!this.history_show_tags, cx);
                        }));

                    let auto_fetch_tags_row = self
                        .summary_row(
                            "settings_window_git_log_tag_fetch_mode",
                            "Automatically fetch tags",
                            git_log_tag_fetch_mode_label(self.history_tag_fetch_mode).into(),
                            self.expanded_section == Some(SettingsSection::GitLogTagFetch),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            if this.history_show_tags {
                                this.toggle_section(SettingsSection::GitLogTagFetch, cx);
                            }
                        }));

                    let mut general_card = self
                        .card("settings_window_general", "General", theme)
                        .child(self.subsection_heading(
                            "settings_window_general_appearance",
                            "Appearance",
                            theme,
                        ))
                        .child(theme_row);

                    if self.expanded_section == Some(SettingsSection::Theme) {
                        let theme_mode_count = settings_theme_modes().len();
                        let list = uniform_list(
                            "settings_window_theme_list",
                            theme_mode_count,
                            cx.processor(Self::render_theme_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.theme_scroll)
                        .on_scroll_wheel({
                            let scroll = self.theme_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        general_card = general_card.child(self.dropdown_list_container(
                            "settings_window_theme_list_container",
                            "settings_window_theme_scrollbar",
                            self.theme_scroll.clone(),
                            theme_mode_count,
                            SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                            list,
                            theme,
                        ));
                        general_card = general_card.child(
                            self.detail_container("settings_window_theme_links_container", theme)
                                // Above the folder link, so a theme that is
                                // missing from the list above is explained right
                                // next to the way to go and fix it.
                                .children(self.rejected_theme_rows(theme))
                                .child(
                                    self.link_row(
                                        "settings_window_theme_custom_folder",
                                        "Open custom theme folder",
                                        self.custom_theme_folder_detail(),
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        |this, _e: &ClickEvent, _window, cx| {
                                            this.open_custom_theme_folder(cx);
                                        },
                                    )),
                                )
                                .child(
                                    self.link_row(
                                        "settings_window_theme_guide",
                                        "Theme guide",
                                        THEMES_GUIDE_URL.into(),
                                        theme,
                                    )
                                    .border_color(no_separator)
                                    .on_click(|_, _, cx| {
                                        cx.open_url(THEMES_GUIDE_URL);
                                    }),
                                ),
                        );
                    }

                    general_card = general_card.child(ui_scale_row);
                    if self.expanded_section == Some(SettingsSection::UiScale) {
                        let mut detail =
                            self.detail_container("settings_window_ui_scale_container", theme);
                        for percent in ui_scale::UI_SCALE_PRESETS.iter().copied() {
                            let detail_text = match percent {
                                ui_scale::DEFAULT_UI_SCALE_PERCENT => Some("Default scale".into()),
                                80 | 90 => Some("Fit more on screen".into()),
                                110 | 125 | 150 => Some("Larger controls and text".into()),
                                _ => None,
                            };
                            detail = detail.child(
                                self.option_row(
                                    format!("settings_window_ui_scale_{percent}"),
                                    ui_scale::label(percent),
                                    detail_text,
                                    self.ui_scale_percent == percent,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |this, _e: &ClickEvent, window, cx| {
                                        this.set_ui_scale_percent(percent, window, cx);
                                    },
                                )),
                            );
                        }
                        general_card = general_card.child(
                            detail.child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child("Shortcut: Ctrl/Cmd +, -, and 0."),
                            ),
                        );
                    }

                    general_card = general_card.child(ui_font_row);
                    if self.expanded_section == Some(SettingsSection::UiFont) {
                        let list = if self.ui_font_options.is_empty() {
                            self.empty_dropdown_list("No fonts available.", theme)
                        } else {
                            restrict_scroll_to_vertical_axis(uniform_list(
                                "settings_window_ui_font_list",
                                self.ui_font_options.len(),
                                cx.processor(Self::render_ui_font_option_rows),
                            )
                            .w_full()
                            .min_w(px(0.0))
                            .h_full()
                            .min_h(px(0.0))
                            .track_scroll(&self.ui_font_scroll)
                            .on_scroll_wheel({
                                let scroll = self.ui_font_scroll.clone();
                                move |event, window, cx| {
                                    if uniform_list_should_stop_scroll_propagation(
                                        &scroll, event, window,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                }
                            })
                            )
                            .into_any_element()
                        };
                        general_card = general_card
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(self.font_options_hint(self.ui_font_family.as_str())),
                            )
                            .child(self.dropdown_list_container(
                                "settings_window_ui_font_list_container",
                                "settings_window_ui_font_scrollbar",
                                self.ui_font_scroll.clone(),
                                self.ui_font_options.len(),
                                SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                                0.0,
                                list,
                                theme,
                            ));
                    }

                    general_card = general_card.child(editor_font_row);
                    if self.expanded_section == Some(SettingsSection::EditorFont) {
                        let list = if self.editor_font_options.is_empty() {
                            self.empty_dropdown_list("No fonts available.", theme)
                        } else {
                            restrict_scroll_to_vertical_axis(uniform_list(
                                "settings_window_editor_font_list",
                                self.editor_font_options.len(),
                                cx.processor(Self::render_editor_font_option_rows),
                            )
                            .w_full()
                            .min_w(px(0.0))
                            .h_full()
                            .min_h(px(0.0))
                            .track_scroll(&self.editor_font_scroll)
                            .on_scroll_wheel({
                                let scroll = self.editor_font_scroll.clone();
                                move |event, window, cx| {
                                    if uniform_list_should_stop_scroll_propagation(
                                        &scroll, event, window,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                }
                            })
                            )
                            .into_any_element()
                        };
                        general_card = general_card
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(
                                        self.font_options_hint(self.editor_font_family.as_str()),
                                    ),
                            )
                            .child(self.dropdown_list_container(
                                "settings_window_editor_font_list_container",
                                "settings_window_editor_font_scrollbar",
                                self.editor_font_scroll.clone(),
                                self.editor_font_options.len(),
                                SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                                0.0,
                                list,
                                theme,
                            ));
                    }

                    general_card = general_card.child(font_ligatures_row);

                    general_card = general_card
                        .child(self.subsection_heading(
                            "settings_window_general_integrations",
                            "Integrations",
                            theme,
                        ))
                        .child(external_editor_row);
                    if self.expanded_section == Some(SettingsSection::ExternalCodeEditor) {
                        let (item_count, list) = if self.external_editor_options_loading() {
                            (
                                1,
                                self.empty_dropdown_list("Detecting installed editors…", theme),
                            )
                        } else {
                            (
                                self.external_editor_options.len(),
                                uniform_list(
                                    "settings_window_external_code_editor_list",
                                    self.external_editor_options.len(),
                                    cx.processor(Self::render_external_editor_option_rows),
                                )
                                .w_full()
                                .min_w(px(0.0))
                                .h_full()
                                .min_h(px(0.0))
                                .track_scroll(&self.external_editor_scroll)
                                .on_scroll_wheel({
                                    let scroll = self.external_editor_scroll.clone();
                                    move |event, window, cx| {
                                        if uniform_list_should_stop_scroll_propagation(
                                            &scroll, event, window,
                                        ) {
                                            cx.stop_propagation();
                                        }
                                    }
                                })
                                .into_any_element(),
                            )
                        };
                        general_card = general_card.child(self.dropdown_list_container(
                            "settings_window_external_code_editor_list_container",
                            "settings_window_external_code_editor_scrollbar",
                            self.external_editor_scroll.clone(),
                            item_count,
                            SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                            list,
                            theme,
                        ));
                    }

                    if self.external_editor_is_custom() {
                        let browse_button = components::Button::new(
                            "settings_window_external_code_editor_browse",
                            "Browse",
                        )
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |_this, _e, window, cx| {
                            let view = cx.weak_entity();
                            let rx = cx.prompt_for_paths(custom_external_editor_path_prompt_options());

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
                                    let _ = view.update(cx, |this, cx| {
                                        this.apply_browsed_external_editor_path(path, cx);
                                    });
                                })
                                .detach();
                        });

                        general_card = general_card.child(
                            self.detail_container(
                                "settings_window_external_code_editor_custom_container",
                                theme,
                            )
                            .child(
                                div()
                                    .px_2()
                                    .pt_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child("Custom editor executable"),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .child(self.external_editor_custom_path_input.clone()),
                                    )
                                    .child(browse_button),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .pt_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child("Arguments"),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .child(
                                        self.external_editor_custom_arguments_input.clone(),
                                    ),
                            ),
                        );
                    }

                    general_card = general_card
                        .child(self.subsection_heading(
                            "settings_window_general_date_time",
                            "Date & Time",
                            theme,
                        ))
                        .child(date_format_row);
                    if self.expanded_section == Some(SettingsSection::DateFormat) {
                        let list = uniform_list(
                            "settings_window_date_format_list",
                            DateTimeFormat::all().len(),
                            cx.processor(Self::render_date_format_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.date_format_scroll)
                        .on_scroll_wheel({
                            let scroll = self.date_format_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        general_card = general_card.child(self.dropdown_list_container(
                            "settings_window_date_format_list_container",
                            "settings_window_date_format_scrollbar",
                            self.date_format_scroll.clone(),
                            DateTimeFormat::all().len(),
                            SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                            list,
                            theme,
                        ));
                    }

                    general_card = general_card.child(timezone_row);
                    if self.expanded_section == Some(SettingsSection::Timezone) {
                        let list = uniform_list(
                            "settings_window_timezone_list",
                            Timezone::all().len(),
                            cx.processor(Self::render_timezone_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.timezone_scroll)
                        .on_scroll_wheel({
                            let scroll = self.timezone_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        general_card = general_card.child(self.dropdown_list_container(
                            "settings_window_timezone_list_container",
                            "settings_window_timezone_scrollbar",
                            self.timezone_scroll.clone(),
                            Timezone::all().len(),
                            SETTINGS_DROPDOWN_DENSE_DETAIL_ROW_HEIGHT_PX,
                            0.0,
                            list,
                            theme,
                        ));
                    }

                    general_card = general_card.child(show_timezone_row);

                    let mut terminal_card =
                        self.card("settings_window_terminal_card", "Terminal", theme);

                    terminal_card = terminal_card.child(terminal_external_row);
                    if self.expanded_section == Some(SettingsSection::TerminalExternal) {
                        terminal_card = terminal_card
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(
                                        "System default is best effort. Use a custom launcher for predictable cross-platform behavior.",
                                    ),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        self.option_row(
                                            "settings_window_terminal_external_default",
                                            ExternalTerminalMode::SystemDefault.label(),
                                            Some("Use the platform default when possible".into()),
                                            self.terminal_preferences.external_terminal_mode
                                                == ExternalTerminalMode::SystemDefault,
                                            theme,
                                        )
                                        .on_click(cx.listener(
                                            |this, _e: &ClickEvent, _window, cx| {
                                                this.set_external_terminal_mode(
                                                    ExternalTerminalMode::SystemDefault,
                                                    cx,
                                                );
                                            },
                                        )),
                                    )
                                    .child(
                                        self.option_row(
                                            "settings_window_terminal_external_custom",
                                            ExternalTerminalMode::CustomProgram.label(),
                                            Some("Choose a launcher and explicit arguments".into()),
                                            self.terminal_preferences.external_terminal_mode
                                                == ExternalTerminalMode::CustomProgram,
                                            theme,
                                        )
                                        .on_click(cx.listener(
                                            |this, _e: &ClickEvent, _window, cx| {
                                                this.set_external_terminal_mode(
                                                    ExternalTerminalMode::CustomProgram,
                                                    cx,
                                                );
                                            },
                                        )),
                                    ),
                            );

                        if self.terminal_preferences.external_terminal_mode
                            == ExternalTerminalMode::CustomProgram
                        {
                            terminal_card = terminal_card
                                .child(
                                    div()
                                        .px_2()
                                        .pt_1()
                                        .text_xs()
                                        .text_color(theme.colors.foreground.secondary)
                                        .child("Program"),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .pb_1()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div().flex_1().min_w(px(0.0)).child(
                                                self.terminal_external_program_input.clone(),
                                            ),
                                        )
                                        .child(
                                            components::Button::new(
                                                "settings_window_terminal_external_browse",
                                                "Browse",
                                            )
                                            .style(components::ButtonStyle::Outlined)
                                            .on_click(theme, cx, |this, _e, window, cx| {
                                                this.browse_terminal_program_input(
                                                    TerminalProgramInputTarget::ExternalTerminal,
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .pt_1()
                                        .text_xs()
                                        .text_color(theme.colors.foreground.secondary)
                                        .child("Arguments"),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .pb_1()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .child(self.terminal_external_args_input.clone()),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .pb_1()
                                        .text_xs()
                                        .text_color(theme.colors.foreground.secondary)
                                        .child("One argument per line. Use {cwd} and {repo_name} placeholders."),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .pb_1()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            components::Button::new(
                                                "settings_window_terminal_external_save",
                                                "Save",
                                            )
                                            .style(components::ButtonStyle::Filled)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.save_terminal_external_draft(cx);
                                            }),
                                        )
                                        .child(
                                            components::Button::new(
                                                "settings_window_terminal_external_reset",
                                                "Reset",
                                            )
                                            .style(components::ButtonStyle::Outlined)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.reset_terminal_external_draft(cx);
                                            }),
                                        )
                                        .child(
                                            components::Button::new(
                                                "settings_window_terminal_external_test",
                                                "Test launch",
                                            )
                                            .style(components::ButtonStyle::Outlined)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.test_terminal_launch_from_draft(cx);
                                            }),
                                        ),
                                );
                        }
                    }

                    terminal_card = terminal_card.child(terminal_action_bar_row);
                    if self.expanded_section == Some(SettingsSection::TerminalActionBar) {
                        terminal_card = terminal_card
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(
                                        "Choose what the action bar terminal button opens. Global shortcuts for each can be configured separately.",
                                    ),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        self.option_row(
                                            "settings_window_terminal_action_bar_embedded",
                                            ActionBarTerminalTarget::Embedded.label(),
                                            Some("Toggle the embedded terminal panel".into()),
                                            self.terminal_preferences.action_bar_terminal_target
                                                == ActionBarTerminalTarget::Embedded,
                                            theme,
                                        )
                                        .on_click(cx.listener(
                                            |this, _e: &ClickEvent, _window, cx| {
                                                this.set_action_bar_terminal_target(
                                                    ActionBarTerminalTarget::Embedded,
                                                    cx,
                                                );
                                            },
                                        )),
                                    )
                                    .child(
                                        self.option_row(
                                            "settings_window_terminal_action_bar_external",
                                            ActionBarTerminalTarget::External.label(),
                                            Some("Launch the external terminal".into()),
                                            self.terminal_preferences.action_bar_terminal_target
                                                == ActionBarTerminalTarget::External,
                                            theme,
                                        )
                                        .on_click(cx.listener(
                                            |this, _e: &ClickEvent, _window, cx| {
                                                this.set_action_bar_terminal_target(
                                                    ActionBarTerminalTarget::External,
                                                    cx,
                                                );
                                            },
                                        )),
                                    ),
                            );
                    }

                    if let Some(status) = self.terminal_status.clone() {
                        terminal_card = terminal_card.child(
                            div()
                                .px_2()
                                .pt_1()
                                .text_xs()
                                .text_color(if status.is_error {
                                    theme.colors.status.danger.foreground
                                } else {
                                    theme.colors.status.success.foreground
                                })
                                .child(status.text),
                        );
                    }

                    let mut change_tracking_card = self
                        .card(
                            "settings_window_change_tracking_card",
                            "Change tracking",
                            theme,
                        )
                        .child(change_tracking_row);

                    if self.expanded_section == Some(SettingsSection::ChangeTracking) {
                        let list = uniform_list(
                            "settings_window_change_tracking_list",
                            CHANGE_TRACKING_OPTIONS.len(),
                            cx.processor(Self::render_change_tracking_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.change_tracking_scroll)
                        .on_scroll_wheel({
                            let scroll = self.change_tracking_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        change_tracking_card =
                            change_tracking_card.child(self.dropdown_list_container(
                                "settings_window_change_tracking_list_container",
                                "settings_window_change_tracking_scrollbar",
                                self.change_tracking_scroll.clone(),
                                CHANGE_TRACKING_OPTIONS.len(),
                                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                                list,
                                theme,
                            ));
                    }

                    let mut diff_card = self
                        .card("settings_window_diff_card", "Diff", theme)
                        .child(diff_content_mode_row);

                    if self.expanded_section == Some(SettingsSection::DiffContentMode) {
                        let list = uniform_list(
                            "settings_window_diff_content_mode_list",
                            DIFF_CONTENT_MODE_OPTIONS.len(),
                            cx.processor(Self::render_diff_content_mode_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.diff_content_mode_scroll)
                        .on_scroll_wheel({
                            let scroll = self.diff_content_mode_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        diff_card = diff_card.child(self.dropdown_list_container(
                            "settings_window_diff_content_mode_list_container",
                            "settings_window_diff_content_mode_scrollbar",
                            self.diff_content_mode_scroll.clone(),
                            DIFF_CONTENT_MODE_OPTIONS.len(),
                            SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                            list,
                            theme,
                        ));
                    }

                    let diff_view_mode_row = self
                        .summary_row(
                            "settings_window_diff_view_mode",
                            "View mode",
                            self.diff_view_mode.settings_label().into(),
                            self.expanded_section == Some(SettingsSection::DiffViewMode),
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.toggle_section(SettingsSection::DiffViewMode, cx);
                        }));

                    diff_card = diff_card.child(diff_view_mode_row);

                    if self.expanded_section == Some(SettingsSection::DiffViewMode) {
                        let list = uniform_list(
                            "settings_window_diff_view_mode_list",
                            DIFF_VIEW_MODE_OPTIONS.len(),
                            cx.processor(Self::render_diff_view_mode_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.diff_view_mode_scroll)
                        .on_scroll_wheel({
                            let scroll = self.diff_view_mode_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        diff_card = diff_card.child(self.dropdown_list_container(
                            "settings_window_diff_view_mode_list_container",
                            "settings_window_diff_view_mode_scrollbar",
                            self.diff_view_mode_scroll.clone(),
                            DIFF_VIEW_MODE_OPTIONS.len(),
                            SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                            list,
                            theme,
                        ));
                    }

                    diff_card = diff_card
                        .child(diff_whitespace_mode_row)
                        .child(diff_reveal_whitespace_chars_row)
                        .child(diff_word_wrap_row)
                        .child(diff_show_line_numbers_row);

                    diff_card = diff_card.child(diff_scroll_sync_row);

                    if self.expanded_section == Some(SettingsSection::Diff) {
                        let list = uniform_list(
                            "settings_window_diff_scroll_sync_list",
                            DIFF_SCROLL_SYNC_OPTIONS.len(),
                            cx.processor(Self::render_diff_scroll_sync_option_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.diff_scroll_sync_scroll)
                        .on_scroll_wheel({
                            let scroll = self.diff_scroll_sync_scroll.clone();
                            move |event, window, cx| {
                                if uniform_list_should_stop_scroll_propagation(
                                    &scroll, event, window,
                                ) {
                                    cx.stop_propagation();
                                }
                            }
                        });
                        let list = restrict_scroll_to_vertical_axis(list).into_any_element();
                        diff_card = diff_card.child(self.dropdown_list_container(
                            "settings_window_diff_scroll_sync_list_container",
                            "settings_window_diff_scroll_sync_scrollbar",
                            self.diff_scroll_sync_scroll.clone(),
                            DIFF_SCROLL_SYNC_OPTIONS.len(),
                            SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                            SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX + 18.0,
                            list,
                            theme,
                        ));
                    }

                    let file_editing_card = self
                        .card(
                            "settings_window_file_editing_card",
                            "File editing",
                            theme,
                        )
                        .child(
                            self.toggle_row(
                                "settings_window_auto_save_file_edits",
                                "Auto-save edits",
                                self.auto_save_file_edits,
                                theme,
                            )
                            .border_color(no_separator)
                            .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_auto_save_file_edits(!this.auto_save_file_edits, cx);
                            })),
                        );

                    let mut git_log_card = self
                        .card("settings_window_git_log_card", "Git log", theme)
                        .child(history_default_mode_row);

                    if self.expanded_section == Some(SettingsSection::GitLogDefaultMode) {
                        let mut mode_container = self.detail_container(
                            "settings_window_git_log_default_mode_container",
                            theme,
                        );
                        for spec in crate::view::history_mode::history_mode_ui_specs() {
                            let mode = spec.mode;
                            mode_container = mode_container.child(
                                self.option_row(
                                    spec.settings_row_id,
                                    spec.label,
                                    Some(spec.settings_description.into()),
                                    self.default_history_mode == mode,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    move |this, _e: &ClickEvent, _window, cx| {
                                        this.set_default_history_mode(mode, cx);
                                    },
                                )),
                            );
                        }
                        git_log_card = git_log_card.child(
                            mode_container.child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(
                                        "Applies when opening repositories that do not already have a saved history mode.",
                                    ),
                            ),
                        );
                    }

                    git_log_card = git_log_card.child(history_columns_row);

                    if self.expanded_section == Some(SettingsSection::GitLogColumns) {
                        git_log_card = git_log_card.child(
                            self.detail_container(
                                "settings_window_git_log_columns_container",
                                theme,
                            )
                            .child(
                                self.toggle_row(
                                    "settings_window_git_log_column_graph",
                                    "Graph",
                                    self.history_show_graph,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    |this, _e: &ClickEvent, _window, cx| {
                                        this.set_history_column_preferences(
                                            !this.history_show_graph,
                                            this.history_show_author,
                                            this.history_show_date,
                                            this.history_show_sha,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                self.toggle_row(
                                    "settings_window_git_log_column_author",
                                    "Author",
                                    self.history_show_author,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    |this, _e: &ClickEvent, _window, cx| {
                                        this.set_history_column_preferences(
                                            this.history_show_graph,
                                            !this.history_show_author,
                                            this.history_show_date,
                                            this.history_show_sha,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                self.toggle_row(
                                    "settings_window_git_log_column_date",
                                    "Commit date",
                                    self.history_show_date,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    |this, _e: &ClickEvent, _window, cx| {
                                        this.set_history_column_preferences(
                                            this.history_show_graph,
                                            this.history_show_author,
                                            !this.history_show_date,
                                            this.history_show_sha,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                self.toggle_row(
                                    "settings_window_git_log_column_sha",
                                    "SHA",
                                    self.history_show_sha,
                                    theme,
                                )
                                .on_click(cx.listener(
                                    |this, _e: &ClickEvent, _window, cx| {
                                        this.set_history_column_preferences(
                                            this.history_show_graph,
                                            this.history_show_author,
                                            this.history_show_date,
                                            !this.history_show_sha,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child("Columns may auto-hide in narrow windows."),
                            )
                            .child(
                                self.link_row(
                                    "settings_window_git_log_reset_widths",
                                    "Reset column widths",
                                    "Reset".into(),
                                    theme,
                                )
                                .border_color(no_separator)
                                .on_click(cx.listener(
                                    |this, _e: &ClickEvent, _window, cx| {
                                        this.update_main_windows(cx, |view, _window, cx| {
                                            view.reset_history_column_widths(cx);
                                        });
                                        cx.notify();
                                    },
                                )),
                            ),
                        );
                    }

                    git_log_card = git_log_card.child(highlight_commit_chain_row);
                    git_log_card = git_log_card.child(relative_dates_row);
                    git_log_card = git_log_card.child(show_history_tags_row);
                    if self.history_show_tags {
                        git_log_card = git_log_card.child(auto_fetch_tags_row);

                        if self.expanded_section == Some(SettingsSection::GitLogTagFetch) {
                            git_log_card = git_log_card.child(
                                self.detail_container(
                                    "settings_window_git_log_tag_fetch_container",
                                    theme,
                                )
                                .child(
                                    self.option_row(
                                        "settings_window_git_log_tag_fetch_mode_activation",
                                        "On repository activation",
                                        Some(
                                            "Fetch local and remote tags in the background when a repository becomes active."
                                                .into(),
                                        ),
                                        self.history_tag_fetch_mode
                                            == GitLogTagFetchMode::OnRepositoryActivation,
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        |this, _e: &ClickEvent, _window, cx| {
                                            this.set_history_tag_fetch_mode(
                                                GitLogTagFetchMode::OnRepositoryActivation,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                                .child(
                                    self.option_row(
                                        "settings_window_git_log_tag_fetch_mode_disabled",
                                        "Disabled",
                                        Some(
                                            "Skip automatic tag fetching on repository activation."
                                                .into(),
                                        ),
                                        self.history_tag_fetch_mode == GitLogTagFetchMode::Disabled,
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        |this, _e: &ClickEvent, _window, cx| {
                                            this.set_history_tag_fetch_mode(
                                                GitLogTagFetchMode::Disabled,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                            );
                        }
                    }

                    let remotes_card = self
                        .card("settings_window_remotes_card", "Remotes", theme)
                        .child(
                            self.toggle_row(
                                "settings_window_prune_deleted_remote_branches",
                                "Automatically prune deleted remote branches on every fetch",
                                self.prune_deleted_remote_branches_on_fetch,
                                theme,
                            )
                            .border_color(no_separator)
                            .on_click(cx.listener(
                                |this, _e: &ClickEvent, _window, cx| {
                                    this.set_prune_deleted_remote_branches_on_fetch(
                                        !this.prune_deleted_remote_branches_on_fetch,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            div()
                                .px_2()
                                .pb_2()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(
                                    "Also applies to the fetch performed by Pull and Pull into current. Local branches whose fetched upstream was deleted are unlinked, but local branches and tags are never deleted.",
                                ),
                        );

                    let tags_card = self
                        .card("settings_window_tags_card", "Tags", theme)
                        .child(
                            self.setting_option_row(
                                "settings_window_tags_default_lightweight",
                                "Lightweight",
                                Some(
                                    "A simple tag pointing directly to a commit. No message, no GPG signing."
                                        .into(),
                                ),
                                self.default_tag_type == DefaultTagType::Lightweight,
                                theme,
                            )
                            .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_default_tag_type(DefaultTagType::Lightweight, cx);
                            })),
                        )
                        .child(
                            self.setting_option_row(
                                "settings_window_tags_default_annotated",
                                "Annotated",
                                Some(
                                    "Stores tag author, date, and an optional message. Supports GPG signing."
                                        .into(),
                                ),
                                self.default_tag_type == DefaultTagType::Annotated,
                                theme,
                            )
                            .border_color(no_separator)
                            .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_default_tag_type(DefaultTagType::Annotated, cx);
                            })),
                        );

                    let system_git_row = self
                        .setting_option_row(
                            "settings_window_git_executable_system",
                            "System PATH",
                            Some(
                                "Use the first `git` executable available in the current PATH."
                                    .into(),
                            ),
                            self.git_executable_mode == GitExecutableMode::SystemPath,
                            theme,
                        )
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_git_executable_mode(GitExecutableMode::SystemPath, cx);
                        }));

                    let custom_git_row = self
                    .setting_option_row(
                        "settings_window_git_executable_custom",
                        "Custom executable",
                        Some(
                            "Use a specific Git binary and add its directory when Git resolves helper tools."
                                .into(),
                        ),
                        self.git_executable_mode == GitExecutableMode::Custom,
                        theme,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_git_executable_mode(GitExecutableMode::Custom, cx);
                    }));

                    let mut git_executable_card = self
                        .card("settings_window_git_executable", "Git executable", theme)
                        .child(
                            div()
                                .id("settings_window_git_executable_scope_note")
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(git_executable_scope_note()),
                        )
                        .child(system_git_row)
                        .child(custom_git_row);

                    if self.git_executable_mode == GitExecutableMode::Custom {
                        let browse_button = components::Button::new(
                            "settings_window_git_executable_browse",
                            "Browse",
                        )
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |_this, _e, window, cx| {
                            let view = cx.weak_entity();
                            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: false,
                                prompt: Some("Select Git executable".into()),
                            });

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
                                    let _ = view.update(cx, |this, cx| {
                                        let next = path.display().to_string();
                                        this.git_custom_path_draft = next.clone();
                                        this.git_executable_input
                                            .update(cx, |input, cx| input.set_text(next, cx));
                                        this.apply_git_executable_settings(cx);
                                    });
                                })
                                .detach();
                        });

                        let use_path_button = components::Button::new(
                            "settings_window_git_executable_apply",
                            "Use Path",
                        )
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _window, cx| {
                            this.apply_git_executable_settings(cx);
                        });

                        git_executable_card = git_executable_card.child(
                        self.detail_container(
                            "settings_window_git_executable_custom_container",
                            theme,
                        )
                        .child(
                            div()
                                .px_2()
                                .pt_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child("Custom Git executable"),
                        )
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .w_full()
                                .min_w(px(0.0))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(self.git_executable_input.clone()),
                                )
                                .child(browse_button)
                                .child(use_path_button),
                        )
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(
                                    "Press Enter after editing the path to apply it immediately.",
                                ),
                        ),
                    );
                    }

                    git_executable_card = git_executable_card.child(self.git_runtime_row(theme));

                    if let Some(detail) = self.runtime_info.git.detail.clone() {
                        git_executable_card = git_executable_card.child(
                            div()
                                .id("settings_window_git_runtime_detail")
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(detail),
                        );
                    }

                    let environment_card = self
                        .card("settings_window_environment", "Environment", theme)
                        .child(self.info_row(
                            "settings_window_build",
                            "Build",
                            self.runtime_info.app_version_display.clone(),
                            theme,
                        ))
                        .child(self.info_row(
                            "settings_window_os",
                            "Operating system",
                            self.runtime_info.operating_system.clone(),
                            theme,
                        ).border_color(no_separator));

                    let links_card = self
                        .card("settings_window_links", "Links", theme)
                        .child(
                            self.link_row(
                                "settings_window_links_theme_guide",
                                "Theme guide",
                                "docs/themes.md".into(),
                                theme,
                            )
                            .on_click(|_, _, cx| {
                                cx.open_url(THEMES_GUIDE_URL);
                            }),
                        )
                        .child(
                            self.link_row(
                                "settings_window_github",
                                "GitHub",
                                "Auto-Explore/GitComet".into(),
                                theme,
                            )
                            .on_click(|_, _, cx| {
                                cx.open_url(GITHUB_URL);
                            }),
                        )
                        .child(
                            self.link_row(
                                "settings_window_license",
                                "License",
                                LICENSE_NAME.into(),
                                theme,
                            )
                            .on_click(|_, _, cx| {
                                cx.open_url(LICENSE_URL);
                            }),
                        )
                        .child(
                            self.link_row(
                                "settings_window_professional_edition_waitlist",
                                "Professional Edition waitlist",
                                "gitcomet.dev".into(),
                                theme,
                            )
                            .on_click(|_, _, cx| {
                                cx.open_url(EDITIONS_URL);
                            }),
                        )
                        .child(
                            self.link_row(
                                "settings_window_open_source_licenses",
                                "Open source licenses",
                                "Show".into(),
                                theme,
                            )
                            .border_color(no_separator)
                            .on_click(cx.listener(
                                |this, _e: &ClickEvent, _window, cx| {
                                    this.show_open_source_licenses(cx);
                                },
                            )),
                        );

                    // The visible page follows the selected nav category.
                    // Expanding a row can only happen from within its owning
                    // category, so deriving from an expanded section keeps the
                    // page and the expanded row consistent.
                    let active_category = self
                        .expanded_section
                        .map(SettingsSection::category)
                        .unwrap_or(self.selected_category);

                    let active_card = match active_category {
                        SettingsCategory::General => general_card,
                        SettingsCategory::Terminal => terminal_card,
                        SettingsCategory::ChangeTracking => change_tracking_card,
                        SettingsCategory::Diff => diff_card,
                        SettingsCategory::FileEditing => file_editing_card,
                        SettingsCategory::GitLog => git_log_card,
                        SettingsCategory::Remotes => remotes_card,
                        SettingsCategory::Tags => tags_card,
                        SettingsCategory::GitExecutable => git_executable_card,
                        SettingsCategory::Environment => environment_card,
                        SettingsCategory::Links => links_card,
                    };

                    let scroll_surface = restrict_scroll_to_vertical_axis(
                        div()
                            .id("settings_window_scroll")
                            .debug_selector(|| "settings_window_scroll".to_string())
                            .w_full()
                            .h_full()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .track_scroll(&self.settings_window_scroll),
                    )
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .child(active_card);

                    let content_pane = div()
                        .id("settings_window_content_pane")
                        .debug_selector(|| "settings_window_content_pane".to_string())
                        .relative()
                        .flex_1()
                        .h_full()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .bg(theme.colors.surface.canvas)
                        .child(
                            div()
                                .w_full()
                                .flex_1()
                                .h_full()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .pr(components::Scrollbar::visible_gutter(
                                    self.settings_window_scroll.clone(),
                                    components::ScrollbarAxis::Vertical,
                                ))
                                .child(scroll_surface),
                        )
                        .child(
                            {
                                let scrollbar = components::Scrollbar::new(
                                    "settings_window_scrollbar",
                                    self.settings_window_scroll.clone(),
                                )
                                .always_visible();
                                #[cfg(test)]
                                let scrollbar =
                                    scrollbar.debug_selector("settings_window_scrollbar");
                                scrollbar
                            }
                            .render(theme),
                        );

                    div()
                        .id("settings_window_root_view")
                        .debug_selector(|| "settings_window_root_view".to_string())
                        .w_full()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .flex()
                        .flex_row()
                        .child(self.render_settings_nav(active_category, theme, cx))
                        .child(content_pane)
                }
                SettingsView::OpenSourceLicenses => {
                    let rows = crate::view::open_source_licenses_data::open_source_license_rows();
                    let breadcrumb = div()
                        .id("settings_window_breadcrumb")
                        .w_full()
                        .px_2()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("settings_window_breadcrumb_settings")
                                .debug_selector(|| {
                                    "settings_window_breadcrumb_settings".to_string()
                                })
                                .px_2()
                                .py_1()
                                .rounded(px(theme.radii.row))
                                .cursor(CursorStyle::PointingHand)
                                .hover(move |s| s.bg(theme.colors.interaction.hover_background))
                                .active(move |s| s.bg(theme.colors.interaction.pressed_background))
                                .text_sm()
                                .text_color(theme.colors.accent.foreground)
                                .child("< Settings")
                                .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                    this.show_root(cx);
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.colors.foreground.secondary)
                                .child("/"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child("Open source licenses"),
                        );

                    let list = if rows.is_empty() {
                        div()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .text_color(theme.colors.foreground.secondary)
                            .child("No dependency licenses found.")
                            .into_any_element()
                    } else {
                        restrict_scroll_to_vertical_axis(uniform_list(
                            "settings_window_open_source_licenses_list",
                            rows.len(),
                            cx.processor(Self::render_open_source_license_rows),
                        )
                        .w_full()
                        .min_w(px(0.0))
                        .h_full()
                        .min_h(px(0.0))
                        .track_scroll(&self.open_source_licenses_scroll))
                        .into_any_element()
                    };

                    let list_container = div()
                        .id("settings_window_open_source_licenses_list_container")
                        .w_full()
                        .min_w(px(0.0))
                        .relative()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .w_full()
                                .flex_1()
                                .h_full()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .pr(components::Scrollbar::visible_gutter(
                                    self.open_source_licenses_scroll.clone(),
                                    components::ScrollbarAxis::Vertical,
                                ))
                                .child(list),
                        )
                        .child(
                            {
                                let scrollbar = components::Scrollbar::new(
                                    "settings_window_open_source_licenses_scrollbar",
                                    self.open_source_licenses_scroll.clone(),
                                )
                                .always_visible();
                                #[cfg(test)]
                                let scrollbar = scrollbar.debug_selector(
                                    "settings_window_open_source_licenses_scrollbar",
                                );
                                scrollbar
                            }
                            .render(theme),
                        );

                    let licenses_card = self
                        .card(
                            "settings_window_open_source_licenses_card",
                            "Open source licenses",
                            theme,
                        )
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(format!("{} third-party crates listed", rows.len())),
                        )
                        .child(
                            div()
                                .id("settings_window_open_source_licenses_columns")
                                .debug_selector(|| {
                                    "settings_window_open_source_licenses_columns".to_string()
                                })
                                .px_2()
                                .py_1()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().w(px(200.0)).child("Crate"))
                                .child(div().w(px(90.0)).child("Version"))
                                .child(div().flex_1().min_w(px(0.0)).child("License")),
                        )
                        .child(list_container);

                    div()
                        .id("settings_window_open_source_licenses_view")
                        .w_full()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_3()
                        .child(breadcrumb)
                        .child(licenses_card)
                }
            }
            .into_any_element()
        };

        let body = div()
            .id("settings_window_content")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .when_some(frame_rounding, |d, rounding| {
                d.when(rounding.bottom_left, |d| d.rounded_bl(rounding.radius))
                    .when(rounding.bottom_right, |d| d.rounded_br(rounding.radius))
            })
            .font(gpui::Font {
                family: crate::font_preferences::applied_ui_font_family(&self.ui_font_family)
                    .into(),
                features: crate::font_preferences::applied_font_features(self.use_font_ligatures),
                fallbacks: None,
                weight: gpui::FontWeight::default(),
                style: gpui::FontStyle::default(),
            })
            .text_color(theme.colors.foreground.primary);

        let body = if show_custom_window_chrome {
            body.child(header).child(content)
        } else {
            body.child(content)
        };

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.foreground.primary)
            .relative()
            // Any click anywhere hides visible tooltips.
            .capture_any_mouse_down(cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                crate::view::tooltip::dismiss_tooltips_on_mouse_down(cx);
            }));

        root = root.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            let Decorations::Client { tiling } = window.window_decorations() else {
                if this.hover_resize_edge.is_some() {
                    this.hover_resize_edge = None;
                    cx.notify();
                }
                return;
            };

            let size = window.viewport_size();
            let next = chrome::resize_edge(
                e.position,
                settings_window_client_inset_for_scale(this.ui_scale_percent),
                size,
                tiling,
            );
            if next != this.hover_resize_edge {
                this.hover_resize_edge = next;
                cx.notify();
            }
        }));

        if tiling.is_some() {
            root = root.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    let Decorations::Client { tiling } = window.window_decorations() else {
                        return;
                    };

                    let size = window.viewport_size();
                    let edge = chrome::resize_edge(
                        e.position,
                        settings_window_client_inset_for_scale(this.ui_scale_percent),
                        size,
                        tiling,
                    );
                    let Some(edge) = edge else {
                        return;
                    };

                    cx.stop_propagation();
                    crate::app::begin_window_resize(window, edge);
                }),
            );
        } else {
            self.hover_resize_edge = None;
        }

        root.child(settings_window_frame(
            theme,
            decorations,
            body.into_any_element(),
            self.ui_scale_percent,
        ))
    }
}
