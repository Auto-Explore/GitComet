//! Settings › Workspaces: name, title-bar colour and theme per workspace.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_state::session::{Workspace, WorkspaceColor, WorkspaceId};

/// Open workspaces first, then most recently used.
fn sorted_workspaces(cx: &App) -> Vec<Workspace> {
    let mut workspaces = crate::workspaces::workspaces(cx);
    workspaces.sort_by_key(|workspace| {
        (
            !workspace.restore_on_launch,
            std::cmp::Reverse(workspace.last_activation_order),
        )
    });
    workspaces
}

fn workspace_summary(workspace: &Workspace) -> SharedString {
    let state = if workspace.restore_on_launch {
        "Open"
    } else {
        "Saved"
    };
    format!(
        "{state} · {}",
        crate::workspaces::repository_count_label(workspace.repositories.len())
    )
    .into()
}

fn workspace_theme_label(workspace: &Workspace) -> SharedString {
    workspace
        .theme_mode
        .as_deref()
        .and_then(ThemeMode::from_key)
        .map_or_else(|| "Follow app theme".into(), |mode| mode.label().into())
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

    fn set_workspace_theme(
        &mut self,
        id: WorkspaceId,
        mode: Option<ThemeMode>,
        cx: &mut gpui::Context<Self>,
    ) {
        let key = mode.map(|mode| mode.key().to_string());
        if crate::workspaces::set_workspace_theme_mode(cx, id, key) {
            crate::app::notify_workspace_changed_from_view(cx, id);
        }
        self.expanded_section = None;
        cx.notify();
    }

    fn delete_workspace(&mut self, id: WorkspaceId, cx: &mut gpui::Context<Self>) {
        crate::workspaces::discard_workspace(cx, id);
        // A live window drops its colour, theme and name on the next repaint.
        crate::app::notify_workspace_changed_from_view(cx, id);
        self.workspace_delete_confirm = None;
        self.reconcile_selected_workspace(cx);
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
                                .bg(crate::view::chrome::workspace_color(color, theme)),
                        )
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
        let mut card = self
            .card("settings_window_workspaces", "Workspaces", theme)
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "Each window is a workspace. Give one a name, a title-bar colour or \
                         its own theme and it is kept even after its last repository closes.",
                    ),
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

        let mut list = self.detail_container("settings_window_workspaces_list", theme);
        for workspace in &workspaces {
            let id = workspace.id;
            list = list.child(
                self.option_row(
                    format!("settings_window_workspace_{id}"),
                    workspace.display_name(),
                    Some(workspace_summary(workspace)),
                    self.selected_workspace == Some(id),
                    theme,
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
            .child(self.field_label("Name (press Enter to save)", theme))
            .child(
                div()
                    .id("settings_window_workspace_name")
                    .debug_selector(|| "settings_window_workspace_name".to_string())
                    .px_2()
                    .pb_1()
                    .w_full()
                    .min_w(px(0.0))
                    .child(self.workspace_name_input.clone()),
            )
            .child(self.field_label("Title bar color", theme))
            .child(self.color_swatches(id, selected.color, theme, cx));

        let theme_expanded = self.expanded_section == Some(SettingsSection::WorkspaceTheme);
        card = card.child(
            self.summary_row(
                "settings_window_workspace_theme",
                "Theme",
                workspace_theme_label(&selected),
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
            let current = selected.theme_mode.as_deref().and_then(ThemeMode::from_key);
            let options = std::iter::once((None, SharedString::from("Follow app theme"))).chain(
                settings_theme_mode_options()
                    .into_iter()
                    .map(|(mode, label)| (Some(mode), label)),
            );
            let mut detail =
                self.detail_container("settings_window_workspace_theme_container", theme);
            for (mode, label) in options {
                let key = mode
                    .as_ref()
                    .map_or("follow_app", |mode| mode.key())
                    .to_string();
                let selected_row = current == mode;
                detail = detail.child(
                    self.option_row(
                        format!("settings_window_workspace_theme_{key}"),
                        label,
                        None,
                        selected_row,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_workspace_theme(id, mode.clone(), cx);
                        }),
                    ),
                );
            }
            card = card.child(detail);
        }

        let mut actions = self
            .detail_container("settings_window_workspace_actions", theme)
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
            );
        if self.workspace_delete_confirm == Some(id) {
            actions = actions
                .child(
                    self.option_row(
                        "settings_window_workspace_delete_confirm",
                        "Confirm delete",
                        Some(
                            "Forgets its name, colour and theme. Repositories open in its \
                             window stay open."
                                .into(),
                        ),
                        false,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.delete_workspace(id, cx);
                        }),
                    ),
                )
                .child(
                    self.option_row(
                        "settings_window_workspace_delete_cancel",
                        "Cancel",
                        None,
                        false,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.workspace_delete_confirm = None;
                            cx.notify();
                        }),
                    ),
                );
        } else {
            actions = actions.child(
                self.option_row(
                    "settings_window_workspace_delete",
                    "Delete workspace…",
                    None,
                    false,
                    theme,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        this.workspace_delete_confirm = Some(id);
                        cx.notify();
                    }),
                ),
            );
        }
        card.child(actions)
    }
}
