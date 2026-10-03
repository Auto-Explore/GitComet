//! Settings › Workspaces: name, title-bar colour and theme per workspace.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_state::session::{Workspace, WorkspaceColor, WorkspaceId};

/// Open workspaces first, then most recently used.
fn sorted_workspaces(cx: &App) -> Vec<Workspace> {
    let mut workspaces = crate::workspaces::workspaces(cx);
    crate::workspaces::sort_workspaces(&mut workspaces);
    workspaces
}

impl SettingsWindowView {
    pub(super) fn select_workspace(&mut self, id: WorkspaceId, cx: &mut gpui::Context<Self>) {
        if self.selected_workspace == Some(id) {
            return;
        }
        self.commit_workspace_name(cx);
        self.selected_workspace = Some(id);
        self.load_workspace_name_draft(cx);
        self.workspace_delete_confirm = None;
        if self.expanded_section == Some(SettingsSection::WorkspaceTheme) {
            self.expanded_section = None;
        }
        cx.notify();
    }

    fn load_workspace_name_draft(&mut self, cx: &mut gpui::Context<Self>) {
        self.workspace_name_draft = self
            .selected_workspace
            .and_then(|id| crate::workspaces::workspace(cx, id))
            .and_then(|workspace| workspace.custom_name)
            .unwrap_or_default();
        let draft = self.workspace_name_draft.clone();
        self.workspace_name_input
            .update(cx, |input, cx| input.set_text(draft, cx));
    }

    /// Keep the selection valid when workspaces change behind this window.
    pub(super) fn reconcile_selected_workspace(&mut self, cx: &mut gpui::Context<Self>) {
        let workspaces = sorted_workspaces(cx);
        if self
            .selected_workspace
            .is_some_and(|id| workspaces.iter().any(|workspace| workspace.id == id))
        {
            return;
        }
        self.selected_workspace = workspaces.first().map(|workspace| workspace.id);
        self.workspace_delete_confirm = None;
        self.load_workspace_name_draft(cx);
    }

    pub(super) fn commit_workspace_name(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(id) = self.selected_workspace else {
            return;
        };
        if crate::workspaces::set_workspace_name(cx, id, &self.workspace_name_draft) {
            crate::app::notify_workspace_changed_from_view(cx, id);
        }
    }

    fn set_workspace_color(
        &mut self,
        id: WorkspaceId,
        color: Option<WorkspaceColor>,
        cx: &mut gpui::Context<Self>,
    ) {
        if crate::workspaces::set_workspace_color(cx, id, color) {
            crate::app::notify_workspace_changed_from_view(cx, id);
        }
        cx.notify();
    }

    /// Keeps the theme grid open so themes can be compared pick after pick.
    pub(super) fn set_workspace_theme(
        &mut self,
        id: WorkspaceId,
        mode: Option<ThemeMode>,
        cx: &mut gpui::Context<Self>,
    ) {
        let key = mode.map(|mode| mode.key().to_string());
        if crate::workspaces::set_workspace_theme_mode(cx, id, key) {
            crate::app::notify_workspace_changed_from_view(cx, id);
        }
        cx.notify();
    }

    fn delete_workspace(&mut self, id: WorkspaceId, cx: &mut gpui::Context<Self>) {
        // Closes its window first when one is open; the workspace observer
        // moves the selection once it is gone.
        crate::app::delete_workspace_from_view(cx, id);
        self.workspace_delete_confirm = None;
        cx.notify();
    }

    fn color_swatches(
        &self,
        id: WorkspaceId,
        current: Option<WorkspaceColor>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let ring = theme.colors.foreground.primary;
        div().px_2().py_1().flex().flex_wrap().gap_2().children(
            crate::workspaces::WORKSPACE_COLORS
                .into_iter()
                .map(|(color, label)| {
                    let selector =
                        format!("settings_window_workspace_color_{}", label.to_lowercase());
                    let debug_selector = selector.clone();
                    let selected = current == color;
                    // Default and Blue share a dot; only Blue tints the bar.
                    let tooltip: SharedString = if color.is_none() {
                        "Default (no title-bar tint)".into()
                    } else {
                        label.into()
                    };
                    div()
                        .id(SharedString::from(selector))
                        .debug_selector(move || debug_selector.clone())
                        .size(px(22.0))
                        .rounded_full()
                        .border_2()
                        .border_color(if selected {
                            ring
                        } else {
                            gpui::rgba(0x00000000)
                        })
                        .p(px(2.0))
                        .cursor(CursorStyle::PointingHand)
                        .child(
                            div()
                                .size_full()
                                .rounded_full()
                                .bg(crate::view::chrome::workspace_color(color)),
                        )
                        .gitcomet_tooltip(theme, tooltip)
                        .control_interaction(
                            crate::view::components::InteractionStyle::new(theme),
                            crate::view::components::InteractionState::default(),
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                                this.set_workspace_color(id, color, cx);
                            }),
                        )
                }),
        )
    }

    fn field_label(&self, text: &'static str, theme: AppTheme) -> gpui::Div {
        div()
            .px_2()
            .pt_2()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(text)
    }

    pub(super) fn workspaces_card(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        self.workspace_name_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        let workspaces = sorted_workspaces(cx);
        let new_window_shortcut = crate::view::shortcut_labels::secondary_shortcut("N");
        let open_workspace_shortcut = crate::view::shortcut_labels::secondary_shortcut("Shift+R");
        let new_workspace_button =
            components::Button::new("settings_window_workspace_new", "New Workspace")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |_this, _e, _window, cx| {
                    cx.defer(crate::app::open_new_empty_window);
                });
        let mut card = self
            .card("settings_window_workspaces", "Workspaces", theme)
            .child(
                div()
                    .id("settings_window_workspaces_intro")
                    .debug_selector(|| "settings_window_workspaces_intro".to_string())
                    .px_2()
                    .pb_2()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(format!(
                                "Each window is a workspace. Open a new one with New Workspace \
                                 or New Window ({new_window_shortcut}); it is saved once a \
                                 repository is open in it. Give a workspace a name, a title-bar \
                                 colour or its own theme and it is kept even after its last \
                                 repository closes. Reopen saved ones with Open Workspace \
                                 ({open_workspace_shortcut})."
                            )),
                    )
                    .child(new_workspace_button),
            );
        if workspaces.is_empty() {
            return card.child(
                div()
                    .id("settings_window_workspaces_empty")
                    .debug_selector(|| "settings_window_workspaces_empty".to_string())
                    .px_2()
                    .py_2()
                    .text_size(theme.ui_text(13.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("No workspaces yet. Open a repository and its window becomes one."),
            );
        }

        let ui_scale = self.row_scale(theme);
        let mut list = self
            .detail_container("settings_window_workspaces_list", theme)
            .p(ui_scale.px(4.0));
        for workspace in &workspaces {
            let id = workspace.id;
            let item = crate::view::workspace_picker::workspace_picker_item(workspace);
            let row_selector: SharedString = format!("settings_window_workspace_{id}").into();
            list = list.child(
                components::picker_row(
                    theme,
                    ui_scale,
                    &item,
                    components::PickerRowSpec {
                        id: row_selector.clone().into(),
                        selector_prefix: "settings_window",
                        key: components::PickerRowKey::Text(id.to_string().into()),
                        row_selector: Some(row_selector),
                        selected: self.selected_workspace == Some(id),
                        marked: false,
                        match_range: None,
                        leading_icon: None,
                    },
                    None,
                    cx,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        this.select_workspace(id, cx);
                    }),
                ),
            );
        }
        card = card.child(list);

        let Some(selected) = self
            .selected_workspace
            .and_then(|id| workspaces.iter().find(|workspace| workspace.id == id))
            .cloned()
        else {
            return card;
        };
        let id = selected.id;

        card = card
            .child(self.subsection_heading(
                "settings_window_workspace_details",
                "Selected workspace",
                theme,
            ))
            .child(
                self.detail_container("settings_window_workspace_actions", theme)
                    .child(
                        self.option_row(
                            "settings_window_workspace_open",
                            "Open workspace",
                            Some("Focus its window, or open one with its repositories.".into()),
                            false,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(move |_this, _e: &ClickEvent, _window, cx| {
                                crate::app::activate_workspace_from_view(cx, id);
                            }),
                        ),
                    ),
            )
            .child(self.field_label("Name", theme))
            .child(
                div()
                    .id("settings_window_workspace_name")
                    .debug_selector(|| "settings_window_workspace_name".to_string())
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
                            .child(self.workspace_name_input.clone()),
                    )
                    .child(
                        // Enter also saves; the button makes that discoverable.
                        components::Button::new("settings_window_workspace_name_save", "Save")
                            .style(components::ButtonStyle::Filled)
                            .disabled(
                                self.workspace_name_draft.trim()
                                    == selected.custom_name.as_deref().unwrap_or(""),
                            )
                            .on_click(theme, cx, |this, _e, _window, cx| {
                                this.commit_workspace_name(cx);
                                cx.notify();
                            }),
                    ),
            )
            .child(self.field_label("Title bar color", theme))
            .child(self.color_swatches(id, selected.color, theme, cx));

        let theme_expanded = self.expanded_section == Some(SettingsSection::WorkspaceTheme);
        // One read of the themes folder serves every lookup below. An unknown
        // key (a deleted user theme) is no override, as in the window itself.
        let themes = crate::theme::ThemeCatalog::load();
        let override_mode = selected
            .theme_mode
            .as_deref()
            .and_then(|raw| ThemeMode::from_catalog_key(raw, &themes));
        // The orb of what the window actually shows: the override, else the app theme.
        let orb = override_mode
            .as_ref()
            .and_then(|mode| super::theme_grid::OrbPaint::for_mode(mode, &themes))
            .unwrap_or_else(|| self.app_theme_orb(&themes, theme));
        let orb = super::theme_grid::theme_orb(orb, self.row_scale(theme).px(16.0));
        let label: SharedString = override_mode.as_ref().map_or_else(
            || "Follow app theme".into(),
            |mode| mode.catalog_label(&themes).into(),
        );
        card = card.child(
            self.summary_row_with_value_prefix(
                "settings_window_workspace_theme",
                "Theme",
                Some(orb.into_any_element()),
                label,
                theme_expanded,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::WorkspaceTheme, cx);
                }),
            ),
        );
        if theme_expanded {
            card = card.child(
                self.detail_container("settings_window_workspace_theme_container", theme)
                    .child(self.workspace_theme_tile_grid(&themes, id, override_mode, theme, cx)),
            );
        }

        card.child(self.workspace_delete_zone(&selected, theme, cx))
    }

    /// Delete sits apart at the bottom, well away from Open, and asks once.
    fn workspace_delete_zone(
        &self,
        workspace: &Workspace,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let id = workspace.id;
        let danger = theme.colors.status.danger.foreground;
        let icon_size = self.row_scale(theme).px(14.0);
        let trash = || crate::view::icons::svg_icon("icons/trash.svg", danger, icon_size);
        let note = |text: String| {
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(text)
        };
        let zone = div()
            .id("settings_window_workspace_delete_zone")
            .debug_selector(|| "settings_window_workspace_delete_zone".to_string())
            .mt_4()
            .pt_4()
            .px_2()
            .border_t_1()
            .border_color(theme.colors.stroke.subtle)
            .flex()
            .flex_col()
            .items_start()
            .gap_2();
        if self.workspace_delete_confirm == Some(id) {
            return zone
                .child(note(format!(
                    "Delete \u{201c}{}\u{201d}? Its name, colour, theme and layout are \
                     forgotten. Repositories and their files are not touched.",
                    workspace.display_name()
                )))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            components::Button::new(
                                "settings_window_workspace_delete_confirm",
                                "Delete",
                            )
                            .style(components::ButtonStyle::Danger)
                            .start_slot(trash())
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _window, cx| {
                                    this.delete_workspace(id, cx);
                                },
                            ),
                        )
                        .child(
                            components::Button::new(
                                "settings_window_workspace_delete_cancel",
                                "Cancel",
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(
                                theme,
                                cx,
                                |this, _e, _window, cx| {
                                    this.workspace_delete_confirm = None;
                                    cx.notify();
                                },
                            ),
                        ),
                );
        }
        zone.child(
            components::Button::new("settings_window_workspace_delete", "Delete workspace")
                .style(components::ButtonStyle::Danger)
                .start_slot(trash())
                .on_click(theme, cx, move |this, _e, _window, cx| {
                    this.workspace_delete_confirm = Some(id);
                    cx.notify();
                }),
        )
        .child(note(
            "Closes its window (the last window returns to Home) and forgets its name, \
             colour, theme and layout. Repositories and their files are not touched."
                .to_string(),
        ))
    }
}
