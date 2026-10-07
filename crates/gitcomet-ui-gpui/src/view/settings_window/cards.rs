//! Settings category cards. Each card is its own function and only the
//! visible one is built, which keeps the render stack frame small in
//! unoptimized builds.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};

impl SettingsWindowView {
    pub(super) fn category_card(
        &mut self,
        category: SettingsCategory,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        crate::view::perf::settings_page_rendered();
        match category {
            SettingsCategory::General => self.general_card(theme, cx),
            SettingsCategory::Appearance => self.appearance_card(theme, cx),
            SettingsCategory::Workspaces => self.workspaces_card(theme, cx),
            SettingsCategory::SecurityPrivacy => self.security_privacy_card(theme, cx),
            SettingsCategory::Terminal => self.terminal_card(theme, cx),
            SettingsCategory::ChangeTracking => self.change_tracking_card(theme, cx),
            SettingsCategory::Diff => self.diff_card(theme, cx),
            SettingsCategory::FileEditing => self.file_editing_card(theme, cx),
            SettingsCategory::GitLog => self.git_log_card(theme, cx),
            SettingsCategory::Remotes => self.remotes_card(theme, cx),
            SettingsCategory::LargeFiles => self.large_files_card(theme, cx),
            SettingsCategory::Tags => self.tags_card(theme, cx),
            SettingsCategory::Maintenance => self.maintenance_card(theme, cx),
            SettingsCategory::GitExecutable => self.git_executable_card(theme, cx),
            SettingsCategory::Environment => self.environment_card(theme, cx),
            SettingsCategory::Links => self.links_card(theme, cx),
        }
    }

    fn general_card(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let card = self.card("settings_window_general", "General", theme);
        let card = self.general_integration_rows(card, theme, cx);
        self.general_date_time_rows(card, theme, cx)
    }

    fn general_integration_rows(
        &self,
        mut general_card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let external_editor_row = self
            .summary_row(
                "settings_window_external_code_editor",
                "External code editor",
                crate::external_editor::label_for_setting(self.external_editor_setting.as_ref())
                    .into(),
                self.expanded_section == Some(SettingsSection::ExternalCodeEditor),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::ExternalCodeEditor, cx);
                }),
            );

        let browser_open_target_row = self
            .summary_row(
                "settings_window_browser_open_target",
                "Command-line repository opens",
                self.browser_open_target.label().into(),
                self.expanded_section == Some(SettingsSection::BrowserOpenTarget),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::BrowserOpenTarget, cx);
                }),
            );

        general_card = general_card.child(self.subsection_heading(
            "settings_window_general_integrations",
            "Integrations",
            theme,
        ));
        general_card = general_card.child(browser_open_target_row);
        if self.expanded_section == Some(SettingsSection::BrowserOpenTarget) {
            let mut detail =
                self.detail_container("settings_window_browser_open_target_container", theme);
            for target in crate::app::BrowserOpenTarget::ALL {
                detail = detail.child(
                    self.option_row(
                        format!("settings_window_browser_open_target_{}", target.key()),
                        target.label(),
                        Some(target.detail().into()),
                        self.browser_open_target == target,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_browser_open_target(target, cx);
                        }),
                    ),
                );
            }
            general_card = general_card.child(detail);
        }

        general_card = general_card.child(external_editor_row);
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
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
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
            let browse_button =
                components::Button::new("settings_window_external_code_editor_browse", "Browse")
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
                        .text_size(theme.ui_text(12.0))
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
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child("Arguments"),
                )
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .w_full()
                        .min_w(px(0.0))
                        .child(self.external_editor_custom_arguments_input.clone()),
                ),
            );
        }
        general_card
    }

    fn general_date_time_rows(
        &self,
        mut general_card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let date_format_row = self
            .summary_row(
                "settings_window_date_format",
                "Date format",
                self.date_time_format.label().into(),
                self.expanded_section == Some(SettingsSection::DateFormat),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DateFormat, cx);
                }),
            );

        let timezone_row = self
            .summary_row(
                "settings_window_timezone",
                "Date timezone",
                self.timezone.label().into(),
                self.expanded_section == Some(SettingsSection::Timezone),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::Timezone, cx);
                }),
            );

        let show_timezone_row = self
            .toggle_row(
                "settings_window_show_timezone",
                "Show timezone",
                self.show_timezone,
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_show_timezone(!this.show_timezone, cx);
                }),
            );

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
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
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
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
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
        general_card
    }

    fn terminal_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let terminal_external_row = self
            .summary_row(
                "settings_window_terminal_external",
                "External terminal",
                self.terminal_preferences.external_summary().into(),
                self.expanded_section == Some(SettingsSection::TerminalExternal),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::TerminalExternal, cx);
                }),
            );

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
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::TerminalActionBar, cx);
                }),
            );

        let mut terminal_card = self.card("settings_window_terminal_card", "Terminal", theme);

        terminal_card = terminal_card.child(terminal_external_row);
        if self.expanded_section == Some(SettingsSection::TerminalExternal) {
            terminal_card = terminal_card
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
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
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
                            .text_size(theme.ui_text(12.0))
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
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(self.terminal_external_program_input.clone()),
                            )
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_browse",
                                    "Browse",
                                )
                                .style(components::ButtonStyle::Outlined)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, window, cx| {
                                        this.browse_terminal_program_input(
                                            TerminalProgramInputTarget::ExternalTerminal,
                                            window,
                                            cx,
                                        );
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(theme.ui_text(12.0))
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
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(
                                "One argument per line. Use {cwd} and {repo_name} placeholders.",
                            ),
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
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, _w, cx| {
                                        this.save_terminal_external_draft(cx);
                                    },
                                ),
                            )
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_reset",
                                    "Reset",
                                )
                                .style(components::ButtonStyle::Outlined)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, _w, cx| {
                                        this.reset_terminal_external_draft(cx);
                                    },
                                ),
                            )
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_test",
                                    "Test launch",
                                )
                                .style(components::ButtonStyle::Outlined)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, _w, cx| {
                                        this.test_terminal_launch_from_draft(cx);
                                    },
                                ),
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
                        .text_size(theme.ui_text(12.0))
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
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
                    .text_size(theme.ui_text(12.0))
                    .text_color(if status.is_error {
                        theme.colors.status.danger.foreground
                    } else {
                        theme.colors.status.success.foreground
                    })
                    .child(status.text),
            );
        }
        terminal_card
    }

    fn security_privacy_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let allowed_remote_protocols_row = self
            .summary_row(
                "settings_window_allowed_remote_protocols",
                "Allowed remote protocols",
                remote_url_policy_settings_label(self.remote_url_policy).into(),
                self.expanded_section == Some(SettingsSection::AllowedRemoteProtocols),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::AllowedRemoteProtocols, cx);
                }),
            );

        let remote_markdown_images_row = self
            .summary_row(
                "settings_window_remote_markdown_images",
                "Remote Markdown images",
                self.remote_markdown_image_policy.settings_label().into(),
                self.expanded_section == Some(SettingsSection::RemoteMarkdownImages),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::RemoteMarkdownImages, cx);
                }),
            );

        let update_check_locked = crate::view::update_checks_disabled_by_environment();
        let update_check_effective = self.check_for_updates_on_startup && !update_check_locked;
        let mut update_check_row = self
            .toggle_row(
                "settings_window_check_for_updates_on_startup",
                "Automatically check for updates on startup",
                update_check_effective,
                theme,
            )
            .border_color(no_separator);
        if update_check_locked {
            update_check_row = update_check_row.opacity(0.6).cursor(CursorStyle::Arrow);
        } else {
            update_check_row = update_check_row.on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_check_for_updates_on_startup(!this.check_for_updates_on_startup, cx);
                }),
            );
        }

        let mut security_privacy_card = self
            .card(
                "settings_window_security_privacy_card",
                "Security / Privacy",
                theme,
            )
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_3()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "Only selected built-in URL protocols may be passed to Git. Custom remote helpers stay blocked. Local paths and SCP-style SSH locations remain available.",
                    ),
            )
            .child(allowed_remote_protocols_row);

        if self.expanded_section == Some(SettingsSection::AllowedRemoteProtocols) {
            let list = uniform_list(
                "settings_window_remote_protocols_list",
                REMOTE_PROTOCOL_OPTIONS.len(),
                cx.processor(Self::render_remote_protocol_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.remote_protocols_scroll)
            .on_scroll_wheel({
                let scroll = self.remote_protocols_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            security_privacy_card = security_privacy_card.child(self.dropdown_list_container(
                "settings_window_remote_protocols_list_container",
                "settings_window_remote_protocols_scrollbar",
                self.remote_protocols_scroll.clone(),
                REMOTE_PROTOCOL_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        security_privacy_card = security_privacy_card
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_3()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "Repository-controlled HTTP/HTTPS images can act as tracking pixels and reveal that you opened a document. Safe relative images from the repository continue to load in every mode.",
                    ),
            )
            .child(remote_markdown_images_row);

        if self.expanded_section == Some(SettingsSection::RemoteMarkdownImages) {
            let list = uniform_list(
                "settings_window_remote_markdown_images_list",
                REMOTE_MARKDOWN_IMAGE_OPTIONS.len(),
                cx.processor(Self::render_remote_markdown_image_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.remote_markdown_images_scroll)
            .on_scroll_wheel({
                let scroll = self.remote_markdown_images_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            security_privacy_card = security_privacy_card.child(self.dropdown_list_container(
                "settings_window_remote_markdown_images_list_container",
                "settings_window_remote_markdown_images_scrollbar",
                self.remote_markdown_images_scroll.clone(),
                REMOTE_MARKDOWN_IMAGE_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        // A product without an update source has nothing to toggle.
        if !crate::view::update_checks_available() {
            return security_privacy_card;
        }
        security_privacy_card = security_privacy_card.child(update_check_row);
        if update_check_locked {
            security_privacy_card = security_privacy_card.child(
                div()
                    .id("settings_window_update_check_environment_note")
                    .px_2()
                    .pb_3()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        format!(
                            "Disabled by GITCOMET_NO_UPDATE_CHECK. Remove the environment variable and restart {} to change this setting.",
                            crate::view::product_name()
                        ),
                    ),
            );
        }
        security_privacy_card
    }

    fn change_tracking_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let change_tracking_row = self
            .summary_row(
                "settings_window_change_tracking",
                "Untracked files",
                self.change_tracking_view.settings_label().into(),
                self.expanded_section == Some(SettingsSection::ChangeTracking),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::ChangeTracking, cx);
                }),
            );

        let file_list_layout_row = self
            .summary_row(
                "settings_window_file_list_layout",
                "Changed-file lists",
                self.file_list_layout.label().into(),
                self.expanded_section == Some(SettingsSection::FileListLayout),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::FileListLayout, cx);
                }),
            );

        let file_list_sort_row = self
            .summary_row(
                "settings_window_file_list_sort",
                "Changed-file sort",
                self.file_list_sort.label().into(),
                self.expanded_section == Some(SettingsSection::FileListSort),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::FileListSort, cx);
                }),
            );

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
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            change_tracking_card = change_tracking_card.child(self.dropdown_list_container(
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

        change_tracking_card = change_tracking_card.child(file_list_layout_row);

        if self.expanded_section == Some(SettingsSection::FileListLayout) {
            let list = uniform_list(
                "settings_window_file_list_layout_list",
                FILE_LIST_LAYOUT_OPTIONS.len(),
                cx.processor(Self::render_file_list_layout_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.file_list_layout_scroll)
            .on_scroll_wheel({
                let scroll = self.file_list_layout_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            change_tracking_card = change_tracking_card.child(self.dropdown_list_container(
                "settings_window_file_list_layout_list_container",
                "settings_window_file_list_layout_scrollbar",
                self.file_list_layout_scroll.clone(),
                FILE_LIST_LAYOUT_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        change_tracking_card = change_tracking_card.child(file_list_sort_row);

        if self.expanded_section == Some(SettingsSection::FileListSort) {
            let list = uniform_list(
                "settings_window_file_list_sort_list",
                FILE_LIST_SORT_OPTIONS.len(),
                cx.processor(Self::render_file_list_sort_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.file_list_sort_scroll)
            .on_scroll_wheel({
                let scroll = self.file_list_sort_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            change_tracking_card = change_tracking_card.child(self.dropdown_list_container(
                "settings_window_file_list_sort_list_container",
                "settings_window_file_list_sort_scrollbar",
                self.file_list_sort_scroll.clone(),
                FILE_LIST_SORT_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }
        change_tracking_card
    }

    fn diff_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let diff_scroll_sync_row = self
            .summary_row(
                "settings_window_diff_scroll_sync",
                "Scroll sync",
                self.diff_scroll_sync.label().into(),
                self.expanded_section == Some(SettingsSection::Diff),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::Diff, cx);
                }),
            );

        let diff_content_mode_row = self
            .summary_row(
                "settings_window_diff_content_mode",
                "Diff mode",
                self.diff_content_mode.settings_label().into(),
                self.expanded_section == Some(SettingsSection::DiffContentMode),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DiffContentMode, cx);
                }),
            );

        let diff_whitespace_mode_row = self
            .toggle_row(
                "settings_window_diff_whitespace_mode",
                "Show whitespace changes",
                self.diff_whitespace_mode == DiffWhitespaceMode::Show,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_whitespace_mode(this.diff_whitespace_mode.toggled(), cx);
                }),
            );

        let diff_reveal_whitespace_chars_row = self
            .toggle_row(
                "settings_window_diff_reveal_whitespace_chars",
                "Reveal whitespace characters",
                self.diff_reveal_whitespace_chars,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_reveal_whitespace_chars(!this.diff_reveal_whitespace_chars, cx);
                }),
            );

        let diff_word_wrap_row = self
            .toggle_row(
                "settings_window_diff_word_wrap",
                "Word wrap",
                self.diff_word_wrap,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_word_wrap(!this.diff_word_wrap, cx);
                }),
            );

        let diff_tab_size_row = self
            .summary_row(
                "settings_window_diff_tab_size",
                "Tab size",
                format!("{} spaces", self.diff_tab_size).into(),
                self.expanded_section == Some(SettingsSection::DiffTabSize),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DiffTabSize, cx);
                }),
            );

        let diff_show_line_numbers_row = self
            .toggle_row(
                "settings_window_diff_show_line_numbers",
                "Show line numbers",
                self.diff_show_line_numbers,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_show_line_numbers(!this.diff_show_line_numbers, cx);
                }),
            );

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
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
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
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DiffViewMode, cx);
                }),
            );

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
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
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
            .child(diff_tab_size_row);

        if self.expanded_section == Some(SettingsSection::DiffTabSize) {
            let list = uniform_list(
                "settings_window_diff_tab_size_list",
                DIFF_TAB_SIZE_OPTIONS.len(),
                cx.processor(Self::render_diff_tab_size_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.diff_tab_size_scroll)
            .on_scroll_wheel({
                let scroll = self.diff_tab_size_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            diff_card = diff_card.child(self.dropdown_list_container(
                "settings_window_diff_tab_size_list_container",
                "settings_window_diff_tab_size_scrollbar",
                self.diff_tab_size_scroll.clone(),
                DIFF_TAB_SIZE_OPTIONS.len(),
                SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        diff_card = diff_card.child(diff_show_line_numbers_row);

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
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
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
        diff_card
    }

    fn file_editing_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_file_editing_card", "File editing", theme)
            .child(
                self.toggle_row(
                    "settings_window_auto_save_file_edits",
                    "Auto-save edits",
                    self.auto_save_file_edits,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_auto_save_file_edits(!this.auto_save_file_edits, cx);
                    }),
                ),
            )
    }

    fn git_log_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let history_default_mode_row = self
            .summary_row(
                "settings_window_git_log_default_mode",
                "Default history mode",
                crate::view::history_mode::history_mode_label(self.default_history_mode).into(),
                self.expanded_section == Some(SettingsSection::GitLogDefaultMode),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogDefaultMode, cx);
                }),
            );

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
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogColumns, cx);
                }),
            );

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
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_highlight_commit_chain(
                        !this.history_highlight_commit_chain,
                        cx,
                    );
                }),
            );

        let files_follow_selected_commit_row = self
            .toggle_row(
                "settings_window_git_log_files_follow_selected_commit",
                "Follow selected commit while file browsing",
                self.files_follow_selected_commit,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_files_follow_selected_commit(!this.files_follow_selected_commit, cx);
                }),
            );

        let relative_dates_row = self
            .toggle_row(
                "settings_window_git_log_relative_dates",
                "Relative dates in history view",
                self.history_relative_dates,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_relative_dates(!this.history_relative_dates, cx);
                }),
            );

        let verify_commit_signatures_row = self
            .toggle_row(
                "settings_window_git_log_verify_signatures",
                "Verify commit signatures",
                self.history_verify_commit_signatures,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_verify_commit_signatures(!this.history_verify_commit_signatures, cx);
                }),
            );

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
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_show_tags(!this.history_show_tags, cx);
                }),
            );

        let auto_fetch_tags_row = self
            .summary_row(
                "settings_window_git_log_tag_fetch_mode",
                "Automatically fetch tags",
                git_log_tag_fetch_mode_label(self.history_tag_fetch_mode).into(),
                self.expanded_section == Some(SettingsSection::GitLogTagFetch),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    if this.history_show_tags {
                        this.toggle_section(SettingsSection::GitLogTagFetch, cx);
                    }
                }),
            );

        let mut git_log_card = self
            .card("settings_window_git_log_card", "Git log", theme)
            .child(history_default_mode_row);

        if self.expanded_section == Some(SettingsSection::GitLogDefaultMode) {
            let mut mode_container =
                self.detail_container("settings_window_git_log_default_mode_container", theme);
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
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_default_history_mode(mode, cx);
                        }),
                    ),
                );
            }
            git_log_card = git_log_card.child(
                mode_container.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(
                            "Applies when opening repositories that do not already have a saved history mode.",
                        ),
                ),
            );
        }

        git_log_card = git_log_card.child(
            self.summary_row(
                "settings_window_git_log_branch_names",
                "Branch names",
                self.history_branch_names.settings_label().into(),
                self.expanded_section == Some(SettingsSection::GitLogBranchNames),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogBranchNames, cx);
                }),
            ),
        );
        if self.expanded_section == Some(SettingsSection::GitLogBranchNames) {
            let mut options =
                self.detail_container("settings_window_git_log_branch_names_container", theme);
            for (mode, id) in [
                (
                    HistoryBranchNamesMode::SeparateColumn,
                    "settings_window_git_log_branch_names_separate",
                ),
                (
                    HistoryBranchNamesMode::Inline,
                    "settings_window_git_log_branch_names_inline",
                ),
            ] {
                options = options.child(
                    self.option_row(
                        id,
                        mode.settings_label(),
                        None,
                        self.history_branch_names == mode,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_history_branch_names(mode, cx);
                        }),
                    ),
                );
            }
            git_log_card = git_log_card.child(options);
        }

        git_log_card = git_log_card.child(history_columns_row);

        if self.expanded_section == Some(SettingsSection::GitLogColumns) {
            git_log_card = git_log_card.child(
                self.detail_container("settings_window_git_log_columns_container", theme)
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_graph",
                            "Graph",
                            self.history_show_graph,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    !this.history_show_graph,
                                    this.history_show_author,
                                    this.history_show_date,
                                    this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_author",
                            "Author",
                            self.history_show_author,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    this.history_show_graph,
                                    !this.history_show_author,
                                    this.history_show_date,
                                    this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_date",
                            "Commit date",
                            self.history_show_date,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    this.history_show_graph,
                                    this.history_show_author,
                                    !this.history_show_date,
                                    this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_sha",
                            "SHA",
                            self.history_show_sha,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    this.history_show_graph,
                                    this.history_show_author,
                                    this.history_show_date,
                                    !this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_size(theme.ui_text(12.0))
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
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.update_main_windows(cx, |view, _window, cx| {
                                    view.reset_history_column_widths(cx);
                                });
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }

        git_log_card = git_log_card.child(highlight_commit_chain_row);
        git_log_card = git_log_card.child(files_follow_selected_commit_row);
        git_log_card = git_log_card.child(relative_dates_row);
        git_log_card = git_log_card.child(verify_commit_signatures_row);
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
                        .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
                        .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
        git_log_card
    }

    fn remotes_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self
            .card("settings_window_remotes_card", "Remotes", theme)
            .child(
                self.toggle_row(
                    "settings_window_prune_deleted_remote_branches",
                    "Automatically prune deleted remote branches on every fetch",
                    self.prune_deleted_remote_branches_on_fetch,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(false, controls::ControlActivation::Action, cx.listener(
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
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "Also applies to the fetch performed by Pull and Pull into current. Local branches whose fetched upstream was deleted are unlinked, but local branches and tags are never deleted.",
                    ),
            )
    }

    fn large_files_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let large_files = self.large_file_settings;
        let large_file_toggle =
            |id: &'static str,
             label: &'static str,
             enabled: bool,
             update: fn(&mut gitcomet_state::model::LargeFileSettings)| {
                self.toggle_row(id, label, enabled, theme).on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        let mut next = this.large_file_settings;
                        update(&mut next);
                        this.set_large_file_settings(next, cx);
                    }),
                )
            };
        let large_file_note = |text: &'static str| {
            div()
                .px_2()
                .pb_2()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(text)
        };
        self.card("settings_window_large_files_card", "Large files", theme)
            .child(large_file_toggle(
                "settings_window_annex_hide_refs",
                "Hide git-annex bookkeeping branches",
                large_files.hide_annex_refs,
                |s| s.hide_annex_refs = !s.hide_annex_refs,
            ))
            .child(large_file_note(
                "The git-annex branch and synced/* branches are managed by git-annex itself.",
            ))
            .child(large_file_toggle(
                "settings_window_annex_pull_push",
                "Use git-annex pull and push on adjusted branches",
                large_files.annex_pull_push,
                |s| s.annex_pull_push = !s.annex_pull_push,
            ))
            .child(large_file_note(
                "A plain merge into an adjusted branch commits the adjusted files to the wrong branch; git-annex propagates changes to the base branch instead.",
            ))
            .child(large_file_toggle(
                "settings_window_annex_sync_content",
                "git-annex pull, push and sync also move file content",
                large_files.annex_sync_content,
                |s| s.annex_sync_content = !s.annex_sync_content,
            ))
    }

    fn maintenance_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_maintenance_card", "Maintenance", theme)
            .child(
                self.toggle_row(
                    "settings_window_recommend_maintenance",
                    "Recommend repository maintenance",
                    self.recommend_repo_maintenance,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(false, controls::ControlActivation::Action, cx.listener(
                    |this, _e: &ClickEvent, _window, cx| {
                        this.set_recommend_repo_maintenance(
                            !this.recommend_repo_maintenance,
                            cx,
                        );
                    },
                )),
            )
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(format!(
                        "Once a day {name} asks Git whether a repository has collected enough loose objects or packs to slow it down (gc.auto, gc.autoPackLimit) and, if so, offers to run git maintenance. {name} never starts maintenance on its own. To skip one repository, set maintenance.auto to false in its Git config.",
                        name = crate::view::product_name()
                    )),
            )
    }

    fn tags_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_tags_card", "Tags", theme)
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
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_default_tag_type(DefaultTagType::Lightweight, cx);
                    }),
                ),
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
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_default_tag_type(DefaultTagType::Annotated, cx);
                    }),
                ),
            )
    }

    fn git_executable_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let system_git_row = self
            .setting_option_row(
                "settings_window_git_executable_system",
                "System PATH",
                Some("Use the first `git` executable available in the current PATH.".into()),
                self.git_executable_mode == GitExecutableMode::SystemPath,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_git_executable_mode(GitExecutableMode::SystemPath, cx);
                }),
            );

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
        .on_activate(false, controls::ControlActivation::Action, cx.listener(|this, _e: &ClickEvent, _window, cx| {
            this.set_git_executable_mode(GitExecutableMode::Custom, cx);
        }));

        let mut git_executable_card = self
            .card("settings_window_git_executable", "Executables", theme)
            .child(
                div()
                    .id("settings_window_git_executable_scope_note")
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(git_executable_scope_note()),
            )
            .child(system_git_row)
            .child(custom_git_row)
            .child(
                components::Button::new("settings_window_recheck_executables", "Recheck")
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, |_this, _e, _window, cx| {
                        super::super::runtime_probe::request(cx, true);
                    }),
            );

        if self.git_executable_mode == GitExecutableMode::Custom {
            let browse_button =
                components::Button::new("settings_window_git_executable_browse", "Browse")
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

            let use_path_button =
                components::Button::new("settings_window_git_executable_apply", "Use Path")
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, |this, _e, _window, cx| {
                        this.apply_git_executable_settings(cx);
                    });

            git_executable_card = git_executable_card.child(
                self.detail_container("settings_window_git_executable_custom_container", theme)
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(theme.ui_text(12.0))
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
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child("Press Enter after editing the path to apply it immediately."),
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
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(detail),
            );
        }

        let signing_tools = self.runtime_info.signing_tools.as_ref();
        let large_file_tools = self.runtime_info.large_file_tools.as_ref();
        for (row_id, detail_id, label, description, info) in [
            (
                "settings_window_gpg_runtime",
                "settings_window_gpg_runtime_detail",
                "GPG",
                GPG_DESCRIPTION,
                gpg_info(signing_tools),
            ),
            (
                "settings_window_ssh_keygen_runtime",
                "settings_window_ssh_keygen_runtime_detail",
                "ssh-keygen",
                SSH_KEYGEN_DESCRIPTION,
                ssh_keygen_info(signing_tools),
            ),
            (
                "settings_window_git_lfs_runtime",
                "settings_window_git_lfs_runtime_detail",
                "Git LFS",
                GIT_LFS_DESCRIPTION,
                git_lfs_info(large_file_tools),
            ),
            (
                "settings_window_git_annex_runtime",
                "settings_window_git_annex_runtime_detail",
                "git-annex",
                GIT_ANNEX_DESCRIPTION,
                git_annex_info(large_file_tools),
            ),
        ] {
            git_executable_card = git_executable_card.child(self.signing_tool_row(
                row_id,
                label,
                description,
                &info,
                theme,
            ));
            if let Some(detail) = info.detail {
                git_executable_card = git_executable_card.child(
                    div()
                        .id(detail_id)
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(detail),
                );
            }
        }

        if let Some(url) = signature_guide_url() {
            git_executable_card = git_executable_card.child(
                self.link_row(
                    "settings_window_signature_guide",
                    "Signature verification guide",
                    "docs/commit-signatures.md".into(),
                    theme,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    move |_, _, cx| {
                        crate::view::platform_open::open_url_later(&url, cx);
                    },
                ),
            );
        }
        git_executable_card
    }

    fn links_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_links", "Links", theme)
            .when_some(themes_guide_url(), |card, url| {
                card.child(
                    self.link_row(
                        "settings_window_links_theme_guide",
                        "Theme guide",
                        "docs/themes.md".into(),
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            crate::view::platform_open::open_url_later(&url, cx);
                        },
                    ),
                )
            })
            .when_some(repository_url(), |card, url| {
                let label = if url.contains("://github.com/") {
                    "GitHub"
                } else {
                    "Source code"
                };
                card.child(
                    self.link_row("settings_window_github", label, url_label(url), theme)
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            move |_, _, cx| {
                                crate::view::platform_open::open_url_later(url, cx);
                            },
                        ),
                )
            })
            .when_some(license_link(), |card, license| {
                card.child(
                    self.link_row(
                        "settings_window_license",
                        "License",
                        license.name.to_string().into(),
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            crate::view::platform_open::open_url_later(&license.url, cx);
                        },
                    ),
                )
            })
            .when_some(crate::view::editions_url(), |card, url| {
                let host = url_label(url);
                let host: SharedString = host
                    .split(['/', '#'])
                    .next()
                    .unwrap_or_default()
                    .to_string()
                    .into();
                card.child(
                    self.link_row(
                        "settings_window_professional_edition_waitlist",
                        "Professional Edition waitlist",
                        host,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            crate::view::platform_open::open_url_later(url, cx);
                        },
                    ),
                )
            })
            .child(
                self.link_row(
                    "settings_window_open_source_licenses",
                    "Open source licenses",
                    "Show".into(),
                    theme,
                )
                .border_color(no_separator)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.show_open_source_licenses(cx);
                    }),
                ),
            )
    }
}
