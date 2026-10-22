use super::*;

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
    fn header(
        &self,
        cx: &mut App,
    ) -> (
        components::PanelHeader,
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
        let mut tabs: Vec<(
            SidebarTabTarget,
            SharedString,
            SharedString,
            Option<SharedString>,
            SharedString,
        )> = Vec::new();
        if self
            .extension_tabs
            .as_ref()
            .is_none_or(|tabs| tabs.host_tabs)
        {
            tabs.push((
                SidebarTabTarget::Branches,
                "sidebar_tab_branches".into(),
                "Branches".into(),
                Some("icons/git_branch.svg".into()),
                "Branches".into(),
            ));
            tabs.push((
                SidebarTabTarget::Files,
                "sidebar_tab_files".into(),
                "Files".into(),
                Some("icons/folder.svg".into()),
                "Files".into(),
            ));
        }
        if self.file_history_visible(cx) {
            let title = self
                .file_history_pane
                .read(cx)
                .target
                .as_ref()
                .map(|(_, path)| path.display().to_string())
                .unwrap_or_default();
            tabs.push((
                SidebarTabTarget::History,
                "sidebar_tab_file_history".into(),
                "History".into(),
                Some("icons/history.svg".into()),
                title.into(),
            ));
        }
        for (index, title, icon) in self.extension_tabs.iter().flat_map(|tabs| &tabs.tabs) {
            tabs.push((
                SidebarTabTarget::Extension(*index),
                format!("sidebar_tab_extension_{index}").into(),
                title.clone(),
                icon.clone(),
                title.clone(),
            ));
        }
        let mut targets = Vec::with_capacity(tabs.len());
        let mut header = components::PanelHeader::new("sidebar_view_toggle", cx);
        for (target, id, title, icon, tooltip) in tabs {
            header = header.tab(id, title, icon, active == target, tooltip);
            targets.push(target);
        }
        (header.tools(active.tool_count()), targets, active)
    }

    pub(in crate::view) fn header_minimum_width(
        &self,
        theme: AppTheme,
        scale: ui_scale::UiScale,
        window: &mut Window,
        cx: &mut App,
    ) -> Pixels {
        let (header, _, _) = self.header(cx);
        header.minimum_width(theme, scale, window)
    }

    pub(super) fn render_tab_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = ui_scale::scaler(ui_scale_percent);
        let (header, targets, active) = self.header(cx);
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

        let mut tools_row: Vec<AnyElement> = Vec::new();
        if tools {
            tools_row.push(
                self.render_search_toggle(mode == SidebarMode::Files, "sidebar_search_toggle", cx)
                    .into_any_element(),
            );
        }
        if mode == SidebarMode::Files && tools {
            tools_row.push(
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
                    .debug_selector(|| "sidebar_explorer_settings".to_string())
                    .into_any_element(),
            );
        }
        if mode == SidebarMode::Branches && tools {
            let tooltip = active_local_branch_name.map_or_else(
                || SharedString::from("No active local branch to show"),
                |name| {
                    SharedString::from(format!("Show and select the active local branch: {name}"))
                },
            );
            tools_row.push(
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
                    .debug_selector(|| "sidebar_locate_active_branch".to_string())
                    .into_any_element(),
            );
        }
        if mode == SidebarMode::Files && tools {
            tools_row.push(
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
                    .debug_selector(|| "sidebar_locate_open_file".to_string())
                    .into_any_element(),
            );
        }
        header
            .render(
                theme,
                ui_scale,
                bg,
                tools_row,
                cx,
                move |this, index, window, cx| {
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
                },
            )
            .id("sidebar_header")
            .debug_selector(|| "sidebar_header".to_string())
    }
}
