use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};

impl Render for SettingsWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        crate::view::perf::settings_rendered();
        let metrics = crate::appearance::current(cx);
        if self.appearance_metrics != metrics {
            self.appearance_metrics = metrics;
            for role in FontRole::ALL {
                self.font_size_inputs[role.index()].update(cx, |input, cx| {
                    input.set_text(metrics.size(role).to_string(), cx)
                });
            }
        }
        self.theme = self.theme.with_appearance(metrics);
        let theme = self.theme;
        for input in &self.font_size_inputs {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
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
        let frame_rounding = chrome::client_frame_corner_rounding(theme, window);

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

        if let Some(extension) = &self.extension_window {
            extension.set_theme(theme, cx);
        }
        let gate = match (&mut self.window_gates, &self.extension_window) {
            (Some(gates), Some(extension)) => gates.content(extension.host(), window, cx),
            _ => None,
        };
        let content = if let Some(gate) = gate {
            div().flex_1().min_h(px(0.0)).child(gate).into_any_element()
        } else if show_overflow_probe {
            self.overflow_probe_content(theme, cx).into_any_element()
        } else {
            match self.current_view {
                SettingsView::Root => self.root_page(theme, cx),
                SettingsView::OpenSourceLicenses => self.open_source_licenses_page(theme, cx),
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
            .text_size(theme.ui_text(16.0))
            .font(gpui::Font {
                family: crate::font_preferences::applied_ui_font_family(&self.ui_font_family)
                    .into(),
                features: crate::font_preferences::applied_font_features(self.use_font_ligatures),
                fallbacks: None,
                weight: gpui::FontWeight::default(),
                width: gpui::FontWidth::default(),
                style: gpui::FontStyle::default(),
            })
            .text_color(theme.colors.foreground.primary);

        let body = if show_custom_window_chrome {
            body.child(self.title_bar(window, theme, cx)).child(content)
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

        let root = root.child(settings_window_frame(
            theme,
            decorations,
            body.into_any_element(),
            self.ui_scale_percent,
        ));
        let ui_scale = crate::ui_scale::UiScale::from_percent(self.ui_scale_percent);
        let root = root.when_some(
            self.extension_notice.clone(),
            |root, (kind, notice, actions)| {
                use gitcomet_extension_api::NotificationKind;
                let kind = match kind {
                    NotificationKind::Success => components::ToastKind::Success,
                    NotificationKind::Error => components::ToastKind::Error,
                    _ => components::ToastKind::Warning,
                };
                let buttons = div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(actions.into_iter().enumerate().map(|(ix, action)| {
                        components::Button::new(
                            format!("settings_notice_action_{ix}"),
                            action.label().clone(),
                        )
                        .on_click(theme, cx, move |_, _, _, cx| action.invoke(cx))
                    }))
                    .child(
                        components::Button::new("settings_notice_close", "Dismiss").on_click(
                            theme,
                            cx,
                            |view, _, _, cx| {
                                view.extension_notice = None;
                                cx.notify();
                            },
                        ),
                    );
                root.child(
                    div()
                        .id("settings_notice")
                        .debug_selector(|| "settings_notice".to_string())
                        .absolute()
                        .bottom_4()
                        .right_4()
                        .occlude()
                        .child(components::toast(
                            theme,
                            ui_scale,
                            kind,
                            div().flex().flex_col().gap_2().child(notice).child(buttons),
                        )),
                )
            },
        );
        let dialog = self.extension_dialog.as_ref().map(|dialog| {
            (
                dialog.title.clone(),
                dialog.content.clone(),
                dialog.anchor,
                dialog.focus.clone(),
            )
        });
        root.when_some(dialog, |root, (title, view, anchor, focus)| {
            let bounds = window.viewport_size();
            // The main window's hosted dialogs use the same kit surfaces: a
            // scrim-backed modal, or a popover at its anchor.
            let surface = match anchor {
                None => components::modal_surface(theme),
                Some(_) => components::popover_surface(theme),
            };
            let dialog = surface
                .id("settings_hosted_dialog")
                .debug_selector(|| "settings_hosted_dialog".to_string())
                .track_focus(&focus)
                .tab_group()
                .tab_stop(false)
                .on_key_down(cx.listener(Self::hosted_dialog_key))
                .w(ui_scale.px(384.0))
                .max_h_full()
                .p(ui_scale.px(16.0))
                .flex()
                .flex_col()
                .gap(ui_scale.px(8.0))
                .text_color(theme.colors.foreground.primary)
                .child(title)
                .child(view)
                .child(
                    components::Button::new("settings_dialog_close", "Close").on_click(
                        theme,
                        cx,
                        |view, _, window, cx| view.close_hosted_dialog(window, cx),
                    ),
                );
            root.child(match anchor {
                None => components::modal_scrim(theme)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(dialog)
                    .into_any_element(),
                Some(anchor) => div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .child(
                        dialog
                            .absolute()
                            .left(
                                anchor
                                    .x
                                    .min((bounds.width - ui_scale.px(384.0)).max(px(0.0)))
                                    .max(px(0.0)),
                            )
                            .top(
                                anchor
                                    .y
                                    .min((bounds.height - ui_scale.px(120.0)).max(px(0.0)))
                                    .max(px(0.0)),
                            ),
                    )
                    .into_any_element(),
            })
        })
    }
}

fn settings_window_control(
    button: gpui::WindowButton,
    is_maximized: bool,
    theme: AppTheme,
    cx: &mut gpui::Context<SettingsWindowView>,
) -> AnyElement {
    match button {
        gpui::WindowButton::Minimize => chrome::titlebar_control_button(
            "settings_window_min_btn",
            "icons/generic_minimize.svg",
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("settings_window_min")
        .debug_selector(|| "settings_window_min".to_string())
        .window_control_area(WindowControlArea::Min)
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                window.minimize_window();
            }),
        )
        .into_any_element(),
        gpui::WindowButton::Maximize => chrome::titlebar_control_button(
            "settings_window_max_btn",
            if is_maximized {
                "icons/generic_restore.svg"
            } else {
                "icons/generic_maximize.svg"
            },
            theme.colors.foreground.secondary,
            theme.colors.foreground.primary,
        )
        .id("settings_window_max")
        .debug_selector(|| "settings_window_max".to_string())
        .window_control_area(WindowControlArea::Max)
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                crate::app::toggle_window_zoom(window);
                cx.notify();
            }),
        )
        .into_any_element(),
        gpui::WindowButton::Close => chrome::titlebar_control_button(
            "settings_window_close_btn",
            "icons/generic_close.svg",
            theme.colors.foreground.secondary,
            theme.colors.status.danger.foreground,
        )
        .id("settings_window_close_btn")
        .debug_selector(|| "settings_window_close".to_string())
        .window_control_area(WindowControlArea::Close)
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|this, _e: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.request_close(window, cx);
            }),
        )
        .into_any_element(),
    }
}

impl SettingsWindowView {
    fn title_bar(
        &self,
        window: &mut Window,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
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
            // The header is a window title bar, so it holds one size at every
            // UI scale, density and font size (see `chrome::chrome_scale`).
            .px(px(12.0))
            .window_control_area(WindowControlArea::Drag)
            .when(is_macos, |this| {
                this.pl(chrome::MACOS_TRAFFIC_LIGHTS_SAFE_INSET)
            })
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(|this, e: &ClickEvent, window, cx| {
                    this.title_drag_state.clear();
                    cx.notify();
                    if !chrome::should_handle_titlebar_double_click(
                        e.click_count(),
                        e.standard_click(),
                    ) {
                        return;
                    }

                    cx.stop_propagation();
                    chrome::handle_titlebar_double_click(window);
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(|_this, e: &MouseDownEvent, window, cx| {
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
                    .child(settings_window_title()),
            );

        let is_maximized = window.is_maximized();
        let control_layout = crate::window_controls::resolve_visibility(
            self.window_controls_mode,
            cx.button_layout(),
            cfg!(any(target_os = "linux", target_os = "freebsd")),
            window.window_decorations(),
            is_maximized,
        );
        let (left_controls, right_controls) = if is_macos {
            (Vec::new(), Vec::new())
        } else {
            let mut render = |button| settings_window_control(button, is_maximized, theme, cx);
            (
                control_layout
                    .left
                    .into_iter()
                    .flatten()
                    .map(&mut render)
                    .collect::<Vec<_>>(),
                control_layout
                    .right
                    .into_iter()
                    .flatten()
                    .map(render)
                    .collect::<Vec<_>>(),
            )
        };
        let has_left_controls = !left_controls.is_empty();
        div()
            .id("settings_window_header")
            .h(chrome::TITLE_BAR_HEIGHT)
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
            .when(has_left_controls, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .h_full()
                        .gap(px(4.0))
                        .pl(px(8.0))
                        .children(left_controls),
                )
            })
            .child(drag_region)
            .child(chrome::window_controls_cluster(right_controls))
    }

    fn root_page(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        // The visible page follows the selected nav category.
        // Expanding a row can only happen from within its owning
        // category, so deriving from an expanded section keeps the
        // page and the expanded row consistent.
        let active_category = self
            .expanded_section
            .map(SettingsSection::category)
            .unwrap_or(self.selected_category);
        let active_card = match self
            .expanded_section
            .is_none()
            .then(|| self.extension_page_card(theme))
            .flatten()
        {
            Some(card) => card,
            None => self.category_card(active_category, theme, cx),
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
                    let scrollbar = scrollbar.debug_selector("settings_window_scrollbar");
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

    fn open_source_licenses_page(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
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
                    .debug_selector(|| "settings_window_breadcrumb_settings".to_string())
                    .px_2()
                    .py_1()
                    .rounded(px(theme.radii.row))
                    .cursor(CursorStyle::PointingHand)
                    .control_interaction(
                        controls::InteractionStyle::new(theme),
                        controls::InteractionState::default(),
                    )
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.accent.foreground)
                    .child("< Settings")
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.show_root(cx);
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(theme.ui_text(14.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("/"),
            )
            .child(
                div()
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .child("Open source licenses"),
            );

        let list = if rows.is_empty() {
            div()
                .px_2()
                .py_1()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("No dependency licenses found.")
                .into_any_element()
        } else {
            restrict_scroll_to_vertical_axis(
                uniform_list(
                    "settings_window_open_source_licenses_list",
                    rows.len(),
                    cx.processor(Self::render_open_source_license_rows),
                )
                .w_full()
                .min_w(px(0.0))
                .h_full()
                .min_h(px(0.0))
                .track_scroll(&self.open_source_licenses_scroll),
            )
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
                    let scrollbar =
                        scrollbar.debug_selector("settings_window_open_source_licenses_scrollbar");
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
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(format!("{} third-party crates listed", rows.len())),
            )
            .child(
                div()
                    .id("settings_window_open_source_licenses_columns")
                    .debug_selector(|| "settings_window_open_source_licenses_columns".to_string())
                    .px_2()
                    .py_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(crate::ui_scale::design_px_from_percent(
                                super::rows::SETTINGS_LICENSE_NAME_COLUMN_PX,
                                self.ui_scale_percent,
                            ))
                            .child("Crate"),
                    )
                    .child(
                        div()
                            .w(crate::ui_scale::design_px_from_percent(
                                super::rows::SETTINGS_LICENSE_VERSION_COLUMN_PX,
                                self.ui_scale_percent,
                            ))
                            .child("Version"),
                    )
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
    pub(super) fn density_control(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let mut density = div()
            .debug_selector(|| "settings_window_density_controls".to_string())
            .w_full()
            .px_2()
            .pt_1()
            .pb_3()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(settings_row_separator_color(theme))
            .child(
                div()
                    .flex_1()
                    .text_size(theme.ui_text(14.0))
                    .child("UI density"),
            );
        for value in UiDensity::ALL {
            density = density.child(
                components::Button::new(format!("settings_density_{}", value.key()), value.label())
                    .selected(self.appearance_metrics.density == value)
                    .on_click(theme, cx, move |this, _, _, cx| this.set_density(value, cx)),
            );
        }
        density
    }

    pub(super) fn font_size_controls(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let mut rows = div()
            .debug_selector(|| "settings_window_font_size_controls".to_string())
            .flex()
            .flex_col()
            .gap_3()
            .p_2();
        for role in FontRole::ALL {
            let value = self.appearance_metrics.size(role);
            let control = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(div().flex_1().child(role.label()))
                .child(
                    components::Button::new(format!("font_size_{}_decrease", role.index()), "−")
                        .disabled(value <= *role.range().start())
                        .on_click(theme, cx, move |this, _, _, cx| {
                            this.set_font_size(role, value.saturating_sub(1), cx)
                        }),
                )
                .child(
                    div()
                        .w(ui_scale::design_px(56.0, cx))
                        .child(self.font_size_inputs[role.index()].clone()),
                )
                .child("px")
                .child(
                    components::Button::new(format!("font_size_{}_increase", role.index()), "+")
                        .disabled(value >= *role.range().end())
                        .on_click(theme, cx, move |this, _, _, cx| {
                            this.set_font_size(role, value + 1, cx)
                        }),
                )
                .child(
                    components::Button::new(format!("font_size_{}_reset", role.index()), "Reset")
                        .on_click(theme, cx, move |this, _, _, cx| {
                            this.set_font_size(role, role.default_size(), cx)
                        }),
                );
            let valid = self.font_size_inputs[role.index()]
                .read(cx)
                .text()
                .trim()
                .parse::<u32>()
                .ok()
                .is_some_and(|size| role.range().contains(&size));
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(control)
                    .when(!valid, |row| {
                        row.child(div().text_size(theme.ui_text(12.0)).child(format!(
                            "Enter a whole number from {} to {}.",
                            role.range().start(),
                            role.range().end()
                        )))
                    }),
            );
        }
        rows.child(div().text_size(theme.ui_text(12.0)).text_color(theme.colors.foreground.secondary)
            .child("Font sizes are measured at 100% UI scale. Each text area can be adjusted independently."))
    }
}
