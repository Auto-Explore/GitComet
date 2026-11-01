//! The workspace rail: an optional column down the window's left edge, one
//! tile per workspace, that switches this window between workspaces in place.
//!
//! A switch closes the window's tabs, as closing its window would, and then
//! adopts the chosen workspace once the store has reduced the close. A
//! workspace that another window holds is focused there instead, and a window
//! with something a close would lose (unsaved edits, a running command or Git
//! operation) opens the chosen workspace in a new window.

use super::*;
use crate::kit::interaction as controls;
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};
use gitcomet_state::session::{Workspace, WorkspaceId};

actions!(
    workspace_rail,
    [
        SwitchWorkspace1,
        SwitchWorkspace2,
        SwitchWorkspace3,
        SwitchWorkspace4,
        SwitchWorkspace5,
        SwitchWorkspace6,
        SwitchWorkspace7,
        SwitchWorkspace8,
        SwitchWorkspace9,
    ]
);

const RAIL_WIDTH_PX: f32 = 44.0;
const TILE_SLOT_PX: f32 = 34.0;
const TILE_PX: f32 = 28.0;

/// The Alt+1..9 bindings. Text inputs and terminals keep those keys: on many
/// layouts Alt+digit types a character, such as `@` on a Finnish Mac.
pub(crate) fn key_bindings() -> Vec<gpui::KeyBinding> {
    use gpui::{KeyBinding, NoAction};
    let mut bindings = vec![
        KeyBinding::new("alt-1", SwitchWorkspace1, None),
        KeyBinding::new("alt-2", SwitchWorkspace2, None),
        KeyBinding::new("alt-3", SwitchWorkspace3, None),
        KeyBinding::new("alt-4", SwitchWorkspace4, None),
        KeyBinding::new("alt-5", SwitchWorkspace5, None),
        KeyBinding::new("alt-6", SwitchWorkspace6, None),
        KeyBinding::new("alt-7", SwitchWorkspace7, None),
        KeyBinding::new("alt-8", SwitchWorkspace8, None),
        KeyBinding::new("alt-9", SwitchWorkspace9, None),
    ];
    // Registered after the bindings above, so they win the tie at equal depth.
    for context in ["TextInput", "Terminal"] {
        for digit in 1..=9 {
            bindings.push(KeyBinding::new(
                &format!("alt-{digit}"),
                NoAction,
                Some(context),
            ));
        }
    }
    bindings
}

/// A rail switch waiting for this window's tabs to close. `None` is the
/// rail's new-workspace tile: the window returns to Home.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingWorkspaceSwitch {
    pub(crate) target: Option<WorkspaceId>,
}

/// Workspaces in rail order: the order they were made in, so a tile (and its
/// shortcut) stays put however the workspaces are used.
pub(crate) fn rail_workspaces(cx: &App) -> Vec<Workspace> {
    let mut workspaces = crate::workspaces::workspaces(cx);
    workspaces.sort_by_key(|workspace| (workspace.created_at.unwrap_or(0), workspace.id));
    workspaces
}

/// One or two letters for a tile: the first letters of the first two words,
/// or the first two letters of a single word. An unnamed workspace's "+N"
/// suffix (its other repositories) is left out.
pub(crate) fn workspace_initials(name: &str) -> String {
    let name = match name.rsplit_once(" +") {
        Some((base, count)) if count.chars().all(|c| c.is_ascii_digit()) => base,
        _ => name,
    };
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut initials = String::new();
    match words.as_slice() {
        [] => initials.push('W'),
        [word] => {
            let mut letters = word.chars();
            if let Some(first) = letters.next() {
                initials.extend(first.to_uppercase());
            }
            if let Some(second) = letters.next() {
                initials.extend(second.to_lowercase());
            }
        }
        [first, second, ..] => {
            for word in [first, second] {
                if let Some(letter) = word.chars().next() {
                    initials.extend(letter.to_uppercase());
                }
            }
        }
    }
    initials
}

impl GitCometView {
    pub(super) fn workspace_rail_shown(&self) -> bool {
        self.workspace_rail_enabled && self.view_mode == GitCometViewMode::Normal
    }

    /// The width the rail takes from the panes.
    pub(super) fn workspace_rail_width(&self) -> Pixels {
        if self.workspace_rail_shown() {
            self.ui_scale().px(RAIL_WIDTH_PX)
        } else {
            px(0.0)
        }
    }

    /// The window width the sidebar, main pane and details pane share.
    pub(super) fn panes_total_width(&self) -> Pixels {
        (self.last_window_size.width - self.workspace_rail_width()).max(px(0.0))
    }

    /// The workspace the rail marks: the one a pending switch is heading to.
    fn rail_current_workspace(&self) -> Option<WorkspaceId> {
        match self.pending_workspace_switch {
            Some(pending) => pending.target,
            None => self.workspace_id,
        }
    }

    pub(super) fn workspace_rail(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let scale = self.ui_scale();
        let current = self.rail_current_workspace();
        let workspaces = rail_workspaces(cx);

        let tiles = workspaces.iter().enumerate().map(|(ix, workspace)| {
            let id = workspace.id;
            let is_current = current == Some(id);
            let elsewhere = !is_current && crate::workspaces::is_open_in_a_window(cx, id);
            let color = chrome::workspace_color(workspace.color);
            let name = workspace.display_name();
            let mut tooltip = format!(
                "{name}\n{}",
                crate::workspaces::repository_count_label(workspace.repositories.len())
            );
            if elsewhere {
                tooltip.push_str(" · open in another window");
            }
            if ix < 9 {
                tooltip.push('\n');
                tooltip.push_str(&crate::kit::components::alt_shortcut(&(ix + 1).to_string()));
            }
            let tile = div()
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .size(scale.px(TILE_PX))
                .rounded(px(theme.radii.control))
                .bg(with_alpha(
                    color,
                    match (is_current, theme.is_dark) {
                        (true, true) => 0.32,
                        (true, false) => 0.22,
                        (false, true) => 0.14,
                        (false, false) => 0.10,
                    },
                ))
                .border_1()
                .border_color(if is_current {
                    color
                } else {
                    with_alpha(color, 0.0)
                })
                .text_size(theme.ui_text(12.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if is_current {
                    theme.colors.foreground.primary
                } else {
                    theme.colors.foreground.secondary
                })
                .child(workspace_initials(&name))
                .when(elsewhere, |tile| {
                    // Held by another window: clicking focuses it there.
                    tile.child(
                        div()
                            .absolute()
                            .right(scale.px(-2.0))
                            .bottom(scale.px(-2.0))
                            .size(scale.px(7.0))
                            .rounded_full()
                            .border_1()
                            .border_color(theme.colors.surface.panel)
                            .bg(theme.colors.foreground.secondary),
                    )
                });
            div()
                .id(("workspace_rail_tile", ix))
                .debug_selector(move || format!("workspace_rail_tile_{ix}"))
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(scale.px(TILE_SLOT_PX))
                .rounded(px(theme.radii.control))
                .control_interaction(
                    InteractionStyle::new(theme),
                    InteractionState::default().open(is_current),
                )
                .when(is_current, |slot| {
                    slot.child(
                        div()
                            .absolute()
                            .left(scale.px(-5.0))
                            .top(scale.px(9.0))
                            .bottom(scale.px(9.0))
                            .w(scale.px(3.0))
                            .rounded_full()
                            .bg(theme.colors.accent.foreground),
                    )
                })
                .child(tile)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        this.switch_workspace_from_rail(Some(id), cx);
                    }),
                )
                .gitcomet_tooltip(theme, tooltip.into())
        });

        let new_workspace = div()
            .id("workspace_rail_new")
            .debug_selector(|| "workspace_rail_new".to_string())
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(scale.px(TILE_SLOT_PX))
            .rounded(px(theme.radii.control))
            .control_interaction(
                InteractionStyle::new(theme),
                InteractionState::default().open(current.is_none()),
            )
            .child(svg_icon(
                "icons/plus.svg",
                theme.colors.foreground.secondary,
                scale.px(14.0),
            ))
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.switch_workspace_from_rail(None, cx);
                }),
            )
            .gitcomet_tooltip(theme, "New workspace in this window".into());

        div()
            .id("workspace_rail")
            .debug_selector(|| "workspace_rail".to_string())
            .flex_none()
            .w(self.workspace_rail_width())
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(scale.px(6.0))
            .pb(scale.px(6.0))
            .gap(scale.px(2.0))
            .overflow_y_scroll()
            .children(tiles)
            .child(new_workspace)
            .into_any_element()
    }

    /// Alt+N: the Nth tile, while the rail is on. Otherwise the key goes on.
    pub(super) fn rail_shortcut(&mut self, index: usize, cx: &mut gpui::Context<Self>) {
        if !self.workspace_rail_shown() {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
        if let Some(workspace) = rail_workspaces(cx).get(index) {
            self.switch_workspace_from_rail(Some(workspace.id), cx);
        }
    }

    /// Why this window cannot let go of its tabs right now, if anything.
    fn workspace_switch_blocker(&self, cx: &App) -> Option<SharedString> {
        if !self
            .ui_model
            .read(cx)
            .documents
            .unsaved_documents()
            .is_empty()
            || self.filesystem_writes_pending(cx)
        {
            return Some("This window has unsaved edits.".into());
        }
        if self
            .terminal_shutdown_summary_for_action(&TerminalShutdownAction::CloseWindow)
            .running_command_count
            > 0
        {
            return Some("A terminal command is still running.".into());
        }
        self.late_close_guard_reasons(&TerminalShutdownAction::CloseWindow, cx)
            .into_iter()
            .next()
    }

    /// Show `target` in this window, or Home for `None`.
    pub(crate) fn switch_workspace_from_rail(
        &mut self,
        target: Option<WorkspaceId>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.pending_workspace_switch.is_some() || target == self.workspace_id {
            return;
        }
        let window_id = self.window_handle.window_id();
        if let Some(id) = target
            && crate::workspaces::is_open_in_a_window(cx, id)
        {
            // Another window holds it; a workspace lives in one window.
            cx.defer(move |cx| crate::app::open_workspace_in_window(cx, window_id, id));
            return;
        }
        if self.synced_repo_paths.is_empty() {
            // Nothing to close.
            match target {
                Some(id) => {
                    cx.defer(move |cx| crate::app::open_workspace_in_window(cx, window_id, id));
                }
                None => {
                    crate::workspaces::release_window_workspace(cx, window_id);
                    self.set_workspace_id(None, cx);
                    self.sync_workspace_and_registry(cx);
                    self.workspace_changed(cx);
                }
            }
            return;
        }
        if let Some(reason) = self.workspace_switch_blocker(cx) {
            match target {
                Some(id) => {
                    self.push_toast(
                        components::ToastKind::Warning,
                        format!("{reason} The workspace opened in a new window."),
                        cx,
                    );
                    crate::app::activate_workspace_from_view(cx, id);
                }
                None => self.push_toast(
                    components::ToastKind::Warning,
                    format!("{reason} Finish it before leaving this workspace."),
                    cx,
                ),
            }
            return;
        }

        // Leave the current workspace as closing its window would: saved,
        // with its layout, and not reopened at launch.
        self.flush_workspace_environment(cx);
        crate::workspaces::release_window_workspace(cx, window_id);
        self.set_workspace_id(None, cx);
        self.pending_repo_open_reservations.clear();
        self.pending_repo_open_active = None;
        self.pending_workspace_switch = Some(PendingWorkspaceSwitch { target });
        let repo_ids: Vec<RepoId> = self.state.repos.iter().map(|repo| repo.id).collect();
        if repo_ids.is_empty() {
            self.finish_workspace_switch(cx);
        } else {
            self.store.dispatch(Msg::CloseRepos {
                repo_ids,
                activate_after: None,
            });
        }
        self.workspace_changed(cx);
        cx.notify();
    }

    /// The tabs have closed: take the chosen workspace.
    pub(super) fn finish_workspace_switch(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PendingWorkspaceSwitch { target }) = self.pending_workspace_switch.take() else {
            return;
        };
        let window_id = self.window_handle.window_id();
        match target.and_then(|id| crate::workspaces::workspace(cx, id)) {
            Some(workspace) if crate::workspaces::is_open_in_a_window(cx, workspace.id) => {
                // Another window took it meanwhile; this one stays on Home.
                let id = workspace.id;
                self.sync_workspace_and_registry(cx);
                cx.defer(move |cx| crate::app::open_workspace_in_window(cx, window_id, id));
            }
            Some(workspace) => self.adopt_workspace(workspace, cx),
            None => self.sync_workspace_and_registry(cx),
        }
        self.workspace_changed(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::workspace_initials;

    #[test]
    fn initials_take_two_words_or_two_letters() {
        assert_eq!(workspace_initials("Client work"), "CW");
        assert_eq!(workspace_initials("gitcomet +2"), "Gi");
        assert_eq!(workspace_initials("gitcomet"), "Gi");
        assert_eq!(workspace_initials("my-app backend"), "MA");
        assert_eq!(workspace_initials("  "), "W");
    }
}
