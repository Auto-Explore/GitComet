use super::*;

const HEADER_PAD_X_PX: f32 = 4.0;
const HEADER_GAP_PX: f32 = 2.0;
const FALLBACK_TAB_ICON: &str = "icons/side_panel_left.svg";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidebarTabTarget {
    Branches,
    Files,
    History,
    Extension(usize),
}

impl SidebarTabTarget {
    fn tool_count(self) -> usize {
        match self {
            Self::Branches => 2,
            Self::Files => 3,
            Self::History | Self::Extension(_) => 0,
        }
    }
}

impl SidebarPaneView {
    fn header_control(
        &self,
        cx: &mut App,
    ) -> (
        components::SegmentedControl,
        Vec<SidebarTabTarget>,
        SidebarTabTarget,
    ) {
        let contributed = self
            .extension_tabs
            .as_ref()
            .and_then(SidebarExtensionTabs::showing);
        let active = if self.file_history_shown(cx) {
            SidebarTabTarget::History
        } else if let Some(index) = contributed {
            SidebarTabTarget::Extension(index)
        } else {
            match self.state.sidebar_mode {
                SidebarMode::Branches => SidebarTabTarget::Branches,
                SidebarMode::Files => SidebarTabTarget::Files,
            }
        };
        let font_preferences = crate::font_preferences::current(cx);
        let font = gpui::Font {
            family: crate::font_preferences::applied_ui_font_family(
                &font_preferences.ui_font_family,
                cx,
            )
            .into(),
            features: crate::font_preferences::applied_font_features(
                font_preferences.use_font_ligatures,
            ),
            ..Default::default()
        };
        let mut tabs = Vec::new();
        let mut add = |target,
                       id: SharedString,
                       title: SharedString,
                       icon: SharedString,
                       tooltip: SharedString| {
            tabs.push((
                target,
                components::Segment::new(id, title)
                    .icon(icon)
                    .selected(active == target)
                    .tooltip(tooltip, Vec::new()),
            ));
        };
        if self
            .extension_tabs
            .as_ref()
            .is_none_or(|tabs| tabs.host_tabs)
        {
            add(
                SidebarTabTarget::Branches,
                "sidebar_tab_branches".into(),
                "Branches".into(),
                "icons/git_branch.svg".into(),
                "Branches".into(),
            );
            add(
                SidebarTabTarget::Files,
                "sidebar_tab_files".into(),
                "Files".into(),
                "icons/folder.svg".into(),
                "Files".into(),
            );
        }
        if self.file_history_visible(cx) {
            let title = self
                .file_history_pane
                .read(cx)
                .target
                .as_ref()
                .map(|(_, path)| path.display().to_string())
                .unwrap_or_default();
            add(
                SidebarTabTarget::History,
                "sidebar_tab_file_history".into(),
                "History".into(),
                "icons/history.svg".into(),
                title.into(),
            );
        }
        for (index, title, icon) in self.extension_tabs.iter().flat_map(|tabs| &tabs.tabs) {
            add(
                SidebarTabTarget::Extension(*index),
                format!("sidebar_tab_extension_{index}").into(),
                title.clone(),
                icon.clone().unwrap_or_else(|| FALLBACK_TAB_ICON.into()),
                title.clone(),
            );
        }
        let (targets, segments): (Vec<_>, Vec<_>) = tabs.into_iter().unzip();
        let control = segments.into_iter().fold(
            components::SegmentedControl::new("sidebar_view_toggle")
                .adaptive()
                .font(font),
            |control, segment| control.segment(segment),
        );
        (control, targets, active)
    }

    pub(in crate::view) fn header_minimum_width(
        &self,
        theme: AppTheme,
        scale: ui_scale::UiScale,
        window: &mut Window,
        cx: &mut App,
    ) -> Pixels {
        let (control, _, active) = self.header_control(cx);
        control.minimum_width(theme, scale, window)
            + scale.px(HEADER_PAD_X_PX) * 2.0
            + (components::control_height(scale) + scale.px(HEADER_GAP_PX))
                * active.tool_count() as f32
    }

    pub(super) fn render_tab_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = ui_scale::scaler(ui_scale_percent);
        let (control, targets, active) = self.header_control(cx);
        let mode = match active {
            SidebarTabTarget::Files => SidebarMode::Files,
            _ => SidebarMode::Branches,
        };
        let tools = active.tool_count() > 0;
        let bg = crate::theme::content_header_bg(theme);
        let bg = if active == SidebarTabTarget::Files
            && self
                .active_repo()
                .is_some_and(|repo| repo.browsing_commit().is_some())
        {
            crate::theme::historical_header_bg(theme, bg)
        } else {
            bg
        };
        let tabs = control.render(theme, ui_scale, cx, move |this, index, window, cx| {
            match targets[index] {
                SidebarTabTarget::Branches | SidebarTabTarget::Files => {
                    let mode = if targets[index] == SidebarTabTarget::Branches {
                        SidebarMode::Branches
                    } else {
                        SidebarMode::Files
                    };
                    this.file_history_selected = false;
                    this.store.dispatch(Msg::SetSidebarMode { mode });
                    this.select_routed_tab(None, window, cx);
                }
                SidebarTabTarget::History => this.show_file_history(window, cx),
                SidebarTabTarget::Extension(index) => {
                    this.file_history_selected = false;
                    this.select_routed_tab(Some(index), window, cx);
                }
            }
            cx.emit(SidebarHeaderChanged);
            cx.notify();
        });
        let active_local_branch_name = self
            .section_locator_target(CollapsedSidebarSection::Local)
            .and_then(|target| match target {
                sticky::NavigationTarget::Branch(BranchMenuTarget::Local { name }) => Some(name),
                _ => None,
            });
        let can_locate_active_branch = active_local_branch_name.is_some();
        let can_locate_open_file = self
            .active_repo()
            .and_then(|repo| repo.open_file_path())
            .is_some();
        let explorer_settings = self
            .active_repo_id()
            .and_then(|repo_id| CollapsedSidebarSection::Files.section_menu(repo_id));
        let explorer_settings_open = explorer_settings
            .as_ref()
            .is_some_and(|(invoker, _)| self.active_context_menu_invoker.as_ref() == Some(invoker));

        let tools_bar = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(ui_scale.px(HEADER_GAP_PX))
            .when(tools, |slot| {
                slot.child(self.render_search_toggle(
                    mode == SidebarMode::Files,
                    "sidebar_search_toggle",
                    cx,
                ))
            })
            .when(mode == SidebarMode::Files && tools, |strip| {
                strip.child(
                    components::Button::new("sidebar_explorer_settings", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .open(explorer_settings_open)
                        .disabled(explorer_settings.is_none())
                        .selected_bg(with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.34 } else { 0.24 },
                        ))
                        .start_slot(crate::view::icons::svg_icon(
                            "icons/cog.svg",
                            if explorer_settings_open {
                                theme.colors.accent.foreground
                            } else if explorer_settings.is_some() {
                                theme.colors.foreground.secondary
                            } else {
                                with_alpha(theme.colors.foreground.secondary, 0.45)
                            },
                            scaled_px(13.0),
                        ))
                        .on_click(theme, cx, move |this, e, window, cx| {
                            if let Some((invoker, kind)) = explorer_settings.clone() {
                                this.open_popover_at(
                                    kind.invoked_by(invoker),
                                    e.position(),
                                    window,
                                    cx,
                                );
                            }
                        })
                        .w(components::control_height(ui_scale))
                        .h(components::control_height(ui_scale))
                        .gitcomet_tooltip(theme, "Files settings".into())
                        .debug_selector(|| "sidebar_explorer_settings".to_string()),
                )
            })
            .when(mode == SidebarMode::Branches && tools, |strip| {
                let tooltip = active_local_branch_name.map_or_else(
                    || SharedString::from("No active local branch to show"),
                    |name| {
                        SharedString::from(format!(
                            "Show and select the active local branch: {name}"
                        ))
                    },
                );
                strip.child(
                    components::Button::new("sidebar_locate_active_branch", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .disabled(!can_locate_active_branch)
                        .start_slot(crate::view::icons::svg_icon(
                            "icons/locate.svg",
                            if can_locate_active_branch {
                                theme.colors.foreground.secondary
                            } else {
                                with_alpha(theme.colors.foreground.secondary, 0.45)
                            },
                            scaled_px(13.0),
                        ))
                        .on_click(theme, cx, |this, _e, _window, cx| {
                            this.locate_active_local_branch(cx);
                        })
                        .w(components::control_height(ui_scale))
                        .h(components::control_height(ui_scale))
                        .gitcomet_tooltip(theme, tooltip)
                        .debug_selector(|| "sidebar_locate_active_branch".to_string()),
                )
            })
            .when(mode == SidebarMode::Files && tools, |strip| {
                strip.child(
                    components::Button::new("sidebar_locate_open_file", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .disabled(!can_locate_open_file)
                        .start_slot(crate::view::icons::svg_icon(
                            "icons/locate.svg",
                            if can_locate_open_file {
                                theme.colors.foreground.secondary
                            } else {
                                with_alpha(theme.colors.foreground.secondary, 0.45)
                            },
                            scaled_px(13.0),
                        ))
                        .on_click(theme, cx, |this, _e, _window, cx| {
                            this.locate_open_file(cx);
                        })
                        .w(components::control_height(ui_scale))
                        .h(components::control_height(ui_scale))
                        .gitcomet_tooltip(
                            theme,
                            if can_locate_open_file {
                                format!(
                                    "Show the open file in the explorer ({})",
                                    crate::kit::components::secondary_shortcut("Shift+L")
                                )
                                .into()
                            } else {
                                SharedString::from("No file is open")
                            },
                        )
                        .debug_selector(|| "sidebar_locate_open_file".to_string()),
                )
            });
        components::navigation_tab_strip(bg, ui_scale)
            .id("sidebar_header")
            .debug_selector(|| "sidebar_header".to_string())
            .min_w(px(0.0))
            .flex_none()
            .border_b_1()
            .border_color(theme.colors.stroke.default)
            .child(tabs)
            .when(tools, |header| header.child(tools_bar))
    }
}
