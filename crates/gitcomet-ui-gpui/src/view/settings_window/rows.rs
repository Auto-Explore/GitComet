use super::*;

impl SettingsWindowView {
    pub(super) fn option_row(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        detail: Option<SharedString>,
        selected: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let id: SharedString = id.into();
        let debug_id = id.clone();
        let text_color = if selected {
            theme.colors.foreground.primary
        } else {
            theme.colors.foreground.secondary
        };
        let selected_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.16 } else { 0.10 },
        );
        let hover_bg = theme.hover_overlay();
        let active_bg = theme.active_overlay();

        div()
            .id(id)
            .debug_selector(move || debug_id.to_string())
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_start()
            .gap_2()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .bg(if selected {
                selected_bg
            } else {
                gpui::rgba(0x00000000)
            })
            .hover(move |s| {
                if selected {
                    s.bg(selected_bg)
                } else {
                    s.bg(hover_bg)
                }
            })
            .active(move |s| {
                if selected {
                    s.bg(selected_bg)
                } else {
                    s.bg(active_bg)
                }
            })
            .child(
                div()
                    .w(px(16.0))
                    // Match the label's line box so the check mark centers on
                    // the first text line instead of hugging the row's top.
                    .h(px(20.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |d| {
                        d.child(svg_icon(
                            "icons/check.svg",
                            theme.colors.accent.foreground,
                            px(12.0),
                        ))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .line_height(px(20.0))
                            .text_color(text_color)
                            .child(label.into()),
                    )
                    .when_some(detail, |this, detail| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(detail),
                        )
                    }),
            )
    }

    pub(super) fn setting_option_row(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        detail: Option<SharedString>,
        selected: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        self.option_row(id, label, detail, selected, theme)
            .rounded(px(0.0))
            .pb_3()
            .border_b_1()
            .border_color(settings_row_separator_color(theme))
    }

    pub(super) fn dense_detail_option_row(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        selected: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let id: SharedString = id.into();
        let debug_id = id.clone();
        let text_color = if selected {
            theme.colors.foreground.primary
        } else {
            theme.colors.foreground.secondary
        };
        let selected_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.16 } else { 0.10 },
        );
        let hover_bg = theme.hover_overlay();
        let active_bg = theme.active_overlay();

        div()
            .id(id)
            .debug_selector(move || debug_id.to_string())
            .w_full()
            .min_h(px(SETTINGS_DROPDOWN_DENSE_DETAIL_ROW_HEIGHT_PX))
            .px_2()
            .py(px(2.0))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .bg(if selected {
                selected_bg
            } else {
                gpui::rgba(0x00000000)
            })
            .hover(move |s| {
                if selected {
                    s.bg(selected_bg)
                } else {
                    s.bg(hover_bg)
                }
            })
            .active(move |s| {
                if selected {
                    s.bg(selected_bg)
                } else {
                    s.bg(active_bg)
                }
            })
            .child(
                div()
                    .w(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |d| {
                        d.child(svg_icon(
                            "icons/check.svg",
                            theme.colors.accent.foreground,
                            px(12.0),
                        ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(text_color)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(label.into()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(detail.into()),
                    ),
            )
    }

    pub(super) fn empty_dropdown_list(&self, message: &'static str, theme: AppTheme) -> AnyElement {
        div()
            .w_full()
            .h_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .px_2()
            .py_1()
            .text_sm()
            .text_color(theme.colors.foreground.secondary)
            .child(message)
            .into_any_element()
    }

    pub(super) fn dropdown_list_container(
        &self,
        container_id: &'static str,
        scrollbar_id: &'static str,
        scroll: UniformListScrollHandle,
        item_count: usize,
        estimated_row_height_px: f32,
        extra_height_px: f32,
        list: AnyElement,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let height = settings_dropdown_height(
            item_count,
            estimated_row_height_px,
            extra_height_px,
            self.ui_scale_percent,
        );
        // `h` includes the 1px border on each edge, so keep the requested
        // dropdown height available to the inner list viewport.
        let outer_height = height + px(2.0);

        div()
            .id(container_id)
            .debug_selector(move || container_id.to_string())
            .w_full()
            .min_w(px(0.0))
            .relative()
            .h(outer_height)
            .min_h(outer_height)
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(settings_dropdown_border_color(theme))
            .bg(settings_dropdown_background(theme))
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .h_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list),
            )
            .child(
                components::Scrollbar::new(scrollbar_id, scroll)
                    .always_visible()
                    .render(theme),
            )
    }

    pub(super) fn detail_container(
        &self,
        container_id: &'static str,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(container_id)
            .debug_selector(move || container_id.to_string())
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(settings_dropdown_border_color(theme))
            .bg(settings_dropdown_background(theme))
            .overflow_hidden()
    }

    pub(super) fn summary_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: SharedString,
        expanded: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let label_debug_id = format!("{id}_label");
        let value_debug_id = format!("{id}_value");
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .pt_1()
            .pb_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(theme.radii.row))
            .border_b_1()
            .border_color(settings_row_separator_color(theme))
            .cursor(CursorStyle::PointingHand)
            .overflow_hidden()
            .hover(move |s| s.bg(theme.colors.interaction.hover_background))
            .active(move |s| s.bg(theme.colors.interaction.pressed_background))
            .child(
                div()
                    .debug_selector(move || label_debug_id.clone())
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(label),
                    ),
            )
            .child(
                div()
                    .debug_selector(move || value_debug_id.clone())
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w(px(0.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(value),
                    )
                    .child(div().flex_shrink_0().child(svg_icon(
                        if expanded {
                            "icons/chevron_down.svg"
                        } else {
                            "icons/arrow_right.svg"
                        },
                        theme.colors.foreground.secondary,
                        px(12.0),
                    ))),
            )
    }

    pub(super) fn toggle_row(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let label_debug_id = format!("{id}_label");
        let value_debug_id = format!("{id}_value");
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .pt_1()
            .pb_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(theme.radii.row))
            .border_b_1()
            .border_color(settings_row_separator_color(theme))
            .cursor(CursorStyle::PointingHand)
            .overflow_hidden()
            .hover(move |s| s.bg(theme.colors.interaction.hover_background))
            .active(move |s| s.bg(theme.colors.interaction.pressed_background))
            .child(
                div()
                    .debug_selector(move || label_debug_id.clone())
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(label),
                    ),
            )
            .child(
                div()
                    .debug_selector(move || value_debug_id.clone())
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(
                        // Toggle-switch visual; the whole row stays the click
                        // target, so this carries no handlers of its own.
                        div()
                            .w(px(28.0))
                            .h(px(16.0))
                            .rounded(px(theme.radii.pill))
                            .flex()
                            .items_center()
                            .p(px(2.0))
                            .when(enabled, |track| {
                                track.justify_end().bg(theme.colors.accent.foreground)
                            })
                            .when(!enabled, |track| {
                                track.justify_start().bg(with_alpha(
                                    theme.colors.foreground.secondary,
                                    if theme.is_dark { 0.35 } else { 0.30 },
                                ))
                            })
                            .child(
                                div()
                                    .size(px(12.0))
                                    .rounded(px(theme.radii.pill))
                                    .bg(gpui::rgba(0xFFFFFFF2)),
                            ),
                    ),
            )
    }

    pub(super) fn info_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: SharedString,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let label_debug_id = format!("{id}_label");
        let value_debug_id = format!("{id}_value");
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .pt_1()
            .pb_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(settings_row_separator_color(theme))
            .overflow_hidden()
            .child(
                div()
                    .debug_selector(move || label_debug_id.clone())
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(label),
                    ),
            )
            .child(
                div()
                    .debug_selector(move || value_debug_id.clone())
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w(px(0.0))
                            .text_sm()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(value),
                    ),
            )
    }

    pub(super) fn link_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: SharedString,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let label_debug_id = format!("{id}_label");
        let value_debug_id = format!("{id}_value");
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .pt_1()
            .pb_3()
            .flex()
            .flex_col()
            .items_stretch()
            .gap_0p5()
            .rounded(px(theme.radii.row))
            .border_b_1()
            .border_color(settings_row_separator_color(theme))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.colors.interaction.hover_background))
            .active(move |s| s.bg(theme.colors.interaction.pressed_background))
            .child(
                div()
                    .debug_selector(move || label_debug_id.clone())
                    .min_w(px(0.0))
                    .text_sm()
                    .child(label),
            )
            .child(
                div()
                    .debug_selector(move || value_debug_id.clone())
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .items_start()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.colors.accent.foreground)
                    .child(div().flex_1().min_w(px(0.0)).child(value))
                    .child(div().flex_shrink_0().child(svg_icon(
                        "icons/open_external.svg",
                        theme.colors.accent.foreground,
                        px(13.0),
                    ))),
            )
    }

    /// One row per theme file the loader refused, named and with its reason.
    ///
    /// A rejected file is otherwise silent: it simply is not in the picker, the
    /// app falls back to a bundled theme, and the account of why only ever
    /// reaches stderr. After a schema break every custom theme in the folder is
    /// rejected at once, and "my theme is gone" has to be answerable from here.
    pub(super) fn rejected_theme_rows(&self, theme: AppTheme) -> Vec<AnyElement> {
        crate::theme::runtime_theme_issues()
            .iter()
            .enumerate()
            .map(|(ix, issue)| {
                let name: SharedString = issue
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| issue.path.display().to_string())
                    .into();
                let message: SharedString = issue.message.clone().into();
                div()
                    .debug_selector(move || format!("settings_window_theme_rejected_{ix}"))
                    .w_full()
                    .min_w(px(0.0))
                    .px_2()
                    .pt_1()
                    .pb_3()
                    .flex()
                    .flex_col()
                    .items_stretch()
                    .gap_0p5()
                    .border_b_1()
                    .border_color(settings_row_separator_color(theme))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .child(svg_icon(
                                "icons/warning.svg",
                                theme.colors.status.warning.foreground,
                                px(13.0),
                            ))
                            .child(div().flex_1().min_w(px(0.0)).child(name)),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(theme.colors.foreground.secondary)
                            .child(message),
                    )
                    .into_any_element()
            })
            .collect()
    }

    pub(super) fn git_runtime_row(&self, theme: AppTheme) -> Stateful<gpui::Div> {
        let min_git_version = format!("{MIN_GIT_MAJOR}.{MIN_GIT_MINOR}");
        let (git_icon_path, git_icon_color, git_status_text): (
            &'static str,
            gpui::Rgba,
            SharedString,
        ) = match self.runtime_info.git.compatibility {
            GitCompatibility::Supported => (
                "icons/check.svg",
                theme.colors.status.success.foreground,
                format!("Git >= {min_git_version}").into(),
            ),
            GitCompatibility::TooOld => (
                "icons/warning.svg",
                theme.colors.status.warning.foreground,
                format!("Git < {min_git_version}").into(),
            ),
            GitCompatibility::Unknown => (
                "icons/warning.svg",
                theme.colors.status.warning.foreground,
                "Git version unknown".into(),
            ),
            GitCompatibility::Unavailable => (
                "icons/warning.svg",
                theme.colors.status.danger.foreground,
                "Unavailable".into(),
            ),
        };

        div()
            .id("settings_window_git_runtime")
            .debug_selector(|| "settings_window_git_runtime".to_string())
            .w_full()
            .px_2()
            .pt_1()
            .pb_3()
            .flex()
            .items_center()
            .gap_2()
            .overflow_hidden()
            .child(
                div()
                    .debug_selector(|| "settings_window_git_runtime_label".to_string())
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child("Detected runtime"),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "settings_window_git_runtime_value".to_string())
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .overflow_hidden()
                    .child(svg_icon(git_icon_path, git_icon_color, px(14.0)))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .text_sm()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(self.runtime_info.git.version_display.clone()),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .text_xs()
                            .text_color(git_icon_color)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .flex_shrink_0()
                            .child(git_status_text),
                    ),
            )
    }

    pub(super) fn overflow_probe_content(&self, theme: AppTheme) -> Stateful<gpui::Div> {
        div()
            .id("settings_window_overflow_probe_view")
            .w_full()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(
                self.card("settings_window_overflow_probe_card", "Overflow probe", theme)
                    .child(self.summary_row(
                        "settings_window_overflow_summary",
                        "Deliberately long summary label for overflow coverage",
                        "Extraordinarily long monospace-friendly summary value used to verify clipping"
                            .into(),
                        false,
                        theme,
                    ))
                    .child(self.toggle_row(
                        "settings_window_overflow_toggle",
                        "Deliberately long toggle label for overflow coverage",
                        true,
                        theme,
                    ))
                    .child(self.info_row(
                        "settings_window_overflow_info",
                        "Deliberately long info label for overflow coverage",
                        self.runtime_info.operating_system.clone(),
                        theme,
                    ))
                    .child(self.link_row(
                        "settings_window_overflow_link",
                        "Deliberately long link label for overflow coverage",
                        "https://github.com/Auto-Explore/GitComet/releases/tag/settings-overflow-regression"
                            .into(),
                        theme,
                    ))
                    .child(self.git_runtime_row(theme)),
            )
    }

    pub(super) fn open_source_license_row(
        &self,
        ix: usize,
        row: crate::view::open_source_licenses_data::OpenSourceLicenseRow,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(("settings_window_open_source_license_row", ix))
            .w_full()
            .px_2()
            .py_1()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(theme.colors.interaction.hover_background))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(200.0))
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(row.crate_name),
                    )
                    .child(
                        div()
                            .w(px(90.0))
                            .text_xs()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.foreground.secondary)
                            .whitespace_nowrap()
                            .child(row.version),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .font_family(UI_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(row.license),
                    ),
            )
    }

    pub(super) fn render_open_source_license_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let rows = crate::view::open_source_licenses_data::open_source_license_rows();
        let theme = this.theme;

        range
            .filter_map(|ix| rows.get(ix).copied().map(|row| (ix, row)))
            .map(|(ix, row)| {
                this.open_source_license_row(ix, row, theme)
                    .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_ui_font_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                this.ui_font_options
                    .get(ix)
                    .cloned()
                    .map(|family| (ix, family))
            })
            .map(|(ix, family)| {
                this.font_option_row_for_family(
                    "settings_window_ui_font",
                    ix,
                    family.as_str(),
                    this.ui_font_family == family,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_ui_font_family(family.clone(), cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_theme_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let modes = settings_theme_mode_options();
        range
            .filter_map(|ix| modes.get(ix).cloned())
            .map(|(mode, label)| {
                this.option_row(
                    format!("settings_window_theme_{}", mode.key()),
                    label,
                    None,
                    this.theme_mode == mode,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
                    this.set_theme_mode(mode.clone(), window, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_editor_font_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                this.editor_font_options
                    .get(ix)
                    .cloned()
                    .map(|family| (ix, family))
            })
            .map(|(ix, family)| {
                this.font_option_row_for_family(
                    "settings_window_editor_font",
                    ix,
                    family.as_str(),
                    this.editor_font_family == family,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_editor_font_family(family.clone(), cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_external_editor_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| this.external_editor_options.get(ix).cloned())
            .map(|option| {
                let selected = match &option.kind {
                    crate::external_editor::ExternalEditorOptionKind::None => {
                        this.external_editor_setting.is_none()
                    }
                    crate::external_editor::ExternalEditorOptionKind::Detected(setting) => {
                        this.external_editor_setting.as_ref() == Some(setting)
                    }
                    crate::external_editor::ExternalEditorOptionKind::Custom => {
                        this.external_editor_is_custom()
                    }
                };
                let row = this.option_row(
                    option.id.clone(),
                    option.label.clone(),
                    option.detail.clone().map(Into::into),
                    selected,
                    theme,
                );
                match option.kind {
                    crate::external_editor::ExternalEditorOptionKind::None => row
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.set_external_editor_setting(None, cx);
                        }))
                        .into_any_element(),
                    crate::external_editor::ExternalEditorOptionKind::Detected(setting) => row
                        .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_external_editor_setting(Some(setting.clone()), cx);
                        }))
                        .into_any_element(),
                    crate::external_editor::ExternalEditorOptionKind::Custom => row
                        .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.select_custom_external_editor(cx);
                        }))
                        .into_any_element(),
                }
            })
            .collect()
    }

    pub(super) fn render_date_format_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                DateTimeFormat::all()
                    .get(ix)
                    .copied()
                    .map(|format| (ix, format))
            })
            .map(|(_ix, format)| {
                this.option_row(
                    match format {
                        DateTimeFormat::YmdHm => "settings_window_date_format_ymd_hm",
                        DateTimeFormat::YmdHms => "settings_window_date_format_ymd_hms",
                        DateTimeFormat::DmyHm => "settings_window_date_format_dmy_hm",
                        DateTimeFormat::MdyHm => "settings_window_date_format_mdy_hm",
                    },
                    format.label(),
                    None,
                    this.date_time_format == format,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_date_time_format(format, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_timezone_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| {
                Timezone::all()
                    .get(ix)
                    .copied()
                    .map(|timezone| (ix, timezone))
            })
            .map(|(_ix, timezone)| {
                this.dense_detail_option_row(
                    format!("settings_window_timezone_{}", timezone.key()),
                    timezone.label(),
                    timezone.cities(),
                    this.timezone == timezone,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_timezone(timezone, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_change_tracking_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| CHANGE_TRACKING_OPTIONS.get(ix).copied())
            .map(|(id, option, detail)| {
                this.option_row(
                    id,
                    option.label(),
                    Some(detail.into()),
                    this.change_tracking_view == option,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_change_tracking_view(option, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_diff_scroll_sync_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| DIFF_SCROLL_SYNC_OPTIONS.get(ix).copied())
            .map(|(id, option, detail)| {
                this.option_row(
                    id,
                    option.label(),
                    Some(detail.into()),
                    this.diff_scroll_sync == option,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_scroll_sync(option, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_diff_view_mode_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| DIFF_VIEW_MODE_OPTIONS.get(ix).copied())
            .map(|(id, option, detail)| {
                this.option_row(
                    id,
                    option.settings_label(),
                    Some(detail.into()),
                    this.diff_view_mode == option,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_view_mode(option, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn render_diff_content_mode_option_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        range
            .filter_map(|ix| DIFF_CONTENT_MODE_OPTIONS.get(ix).copied())
            .map(|(id, option, detail)| {
                this.option_row(
                    id,
                    option.label(),
                    Some(detail.into()),
                    this.diff_content_mode == option,
                    theme,
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_content_mode(option, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(super) fn card(
        &self,
        id: &'static str,
        title: &'static str,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.foreground.primary)
                    .child(title),
            )
    }

    pub(super) fn subsection_heading(
        &self,
        id: &'static str,
        title: &'static str,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .px_2()
            .pt(px(24.0))
            .pb_2()
            .text_sm()
            .font_weight(FontWeight::BOLD)
            .text_color(theme.colors.foreground.primary)
            .child(title)
    }

    pub(super) fn settings_nav_item(
        &self,
        category: SettingsCategory,
        selected: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let icon_color = if selected {
            theme.colors.accent.foreground
        } else {
            theme.colors.foreground.secondary
        };
        div()
            .id(category.nav_id())
            .debug_selector(move || category.nav_id().to_string())
            .w_full()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .overflow_hidden()
            .when(selected, |d| {
                d.bg(theme.colors.interaction.pressed_background)
            })
            .when(!selected, |d| {
                d.hover(move |s| s.bg(theme.colors.interaction.hover_background))
            })
            .child(
                div()
                    .flex_shrink_0()
                    .child(svg_icon(category.icon(), icon_color, px(15.0))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                    .text_color(theme.colors.foreground.primary)
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(category.label()),
            )
            .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                this.select_category(category, cx);
            }))
    }

    pub(super) fn render_settings_nav(
        &self,
        active: SettingsCategory,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let query = self.search_query.clone();

        let mut list = div()
            .id("settings_window_nav_list")
            .debug_selector(|| "settings_window_nav_list".to_string())
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.nav_scroll)
            .flex()
            .flex_col()
            .gap(px(1.0));

        let mut any_match = false;
        for category in SettingsCategory::ALL.iter().copied() {
            if !category.matches_query(&query) {
                continue;
            }
            any_match = true;
            list = list.child(self.settings_nav_item(category, category == active, theme, cx));
        }

        if !any_match {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .child("No matching settings"),
            );
        }

        div()
            .id("settings_window_nav")
            .debug_selector(|| "settings_window_nav".to_string())
            .flex_none()
            .w(px(200.0))
            .h_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(theme.colors.surface.chrome)
            .child(
                div()
                    .id("settings_window_nav_search")
                    .flex_none()
                    .w_full()
                    .child(self.search_input.clone()),
            )
            .child(list)
    }
}
