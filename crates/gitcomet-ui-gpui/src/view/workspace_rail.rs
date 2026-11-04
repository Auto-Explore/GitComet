//! The workspace rail: with workspaces opening in the current window, a slim
//! column at the left of each window lists every workspace, and picking one
//! (or Alt+1…9) shows it in that window. A workspace stays in at most one
//! window: one shown elsewhere is focused there, or moved here on request.

use super::terminal_panel::shutdown_terminal_instance;
use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_state::session::{Workspace, WorkspaceId};

/// The rail's width at 100% scale.
const RAIL_WIDTH_PX: f32 = 48.0;
const TILE_PX: f32 = 32.0;

/// Two letters naming a workspace on its tile: the first letters of its first
/// two words, the capitals of a single CamelCase word, or its first two
/// letters.
pub(in crate::view) fn workspace_initials(name: &str) -> String {
    let words = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let initials: String = match words.as_slice() {
        [] => "?".to_string(),
        [word] => {
            let capitals = word
                .chars()
                .filter(|c| c.is_uppercase())
                .take(2)
                .collect::<String>();
            if capitals.chars().count() == 2 {
                capitals
            } else {
                word.chars().take(2).collect()
            }
        }
        [first, second, ..] => first
            .chars()
            .take(1)
            .chain(second.chars().take(1))
            .collect(),
    };
    initials.to_uppercase()
}

/// One workspace on the rail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct RailEntry {
    pub(in crate::view) id: WorkspaceId,
    pub(in crate::view) name: String,
    pub(in crate::view) initials: String,
    pub(in crate::view) color: Option<gitcomet_state::session::WorkspaceColor>,
    /// Alt+N reaches it.
    pub(in crate::view) slot: Option<usize>,
    /// This window shows it.
    pub(in crate::view) current: bool,
    /// The number of another window showing it.
    pub(in crate::view) elsewhere: Option<usize>,
}

pub(in crate::view) fn rail_entries(
    workspaces: &[Workspace],
    window_id: gpui::WindowId,
    windows: &[(gpui::WindowId, WorkspaceId, usize)],
) -> Vec<RailEntry> {
    workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            let name = workspace.display_name();
            let holder = windows.iter().find(|(_, id, _)| *id == workspace.id);
            RailEntry {
                id: workspace.id,
                initials: workspace_initials(&name),
                name,
                color: workspace.color,
                slot: (index < crate::app::WORKSPACE_SWITCH_SLOTS).then_some(index + 1),
                current: holder.is_some_and(|(window, _, _)| *window == window_id),
                elsewhere: holder
                    .filter(|(window, _, _)| *window != window_id)
                    .map(|(_, _, number)| *number),
            }
        })
        .collect()
}

/// What hovering a tile says.
pub(in crate::view) fn rail_tooltip(entry: &RailEntry) -> String {
    let shortcut = entry
        .slot
        .map(|slot| format!(" (Alt+{slot})"))
        .unwrap_or_default();
    match (entry.current, entry.elsewhere) {
        (true, _) => format!("{}{shortcut}, shown in this window", entry.name),
        (false, Some(number)) => format!(
            "{}{shortcut} is open in window {number}. Click to go there, Shift+click to show it here.",
            entry.name
        ),
        (false, None) => format!("Show {} in this window{shortcut}", entry.name),
    }
}

pub(in crate::view) struct WorkspaceRail {
    theme: AppTheme,
    window_id: gpui::WindowId,
    _manager_subscription: gpui::Subscription,
}

impl WorkspaceRail {
    pub(in crate::view) fn new(
        theme: AppTheme,
        window_id: gpui::WindowId,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let subscription =
            cx.observe_global::<crate::workspaces::WorkspaceManager>(|_this, cx| cx.notify());
        Self {
            theme,
            window_id,
            _manager_subscription: subscription,
        }
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }

    fn tile(
        &self,
        entry: &RailEntry,
        scaled_px: &impl Fn(f32) -> Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let color = chrome::workspace_color(entry.color);
        let id = entry.id;
        let window_id = self.window_id;
        let selector = format!("workspace_rail_entry_{id}");
        let tile = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(scaled_px(TILE_PX))
            .rounded(px(theme.radii.control))
            .text_size(theme.ui_text(12.0))
            .font_weight(FontWeight::BOLD)
            .map(|tile| {
                if entry.current {
                    tile.bg(color).text_color(theme.colors.surface.canvas)
                } else {
                    tile.bg(with_alpha(color, if theme.is_dark { 0.16 } else { 0.12 }))
                        .border_1()
                        .border_color(with_alpha(color, 0.35))
                        .text_color(color)
                }
            })
            .child(entry.initials.clone())
            .when_some(entry.elsewhere, |tile, number| {
                tile.child(
                    div()
                        .debug_selector(move || format!("workspace_rail_window_badge_{id}"))
                        .absolute()
                        .top(scaled_px(-5.0))
                        .right(scaled_px(-5.0))
                        .size(scaled_px(15.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.colors.foreground.primary)
                        .text_color(theme.colors.surface.chrome)
                        .text_size(theme.ui_text(9.0))
                        .font_weight(FontWeight::BOLD)
                        .child(number.to_string()),
                )
            });
        let label = entry.slot.map(|slot| format!("Alt {slot}"));
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(scaled_px(2.0))
            .py(scaled_px(2.0))
            .control_interaction(
                crate::view::components::InteractionStyle::new(theme),
                crate::view::components::InteractionState::default(),
            )
            .when(entry.current, |entry_row| {
                entry_row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(scaled_px(6.0))
                        .w(scaled_px(3.0))
                        .h(scaled_px(TILE_PX - 4.0))
                        .rounded_r(scaled_px(2.0))
                        .bg(color),
                )
            })
            .child(tile)
            .when_some(label, |entry_row, label| {
                entry_row.child(
                    div()
                        .text_size(theme.ui_text(9.0))
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.foreground.secondary)
                        .child(label),
                )
            })
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |_this, event: &ClickEvent, _window, cx| {
                    let move_here = event.modifiers().shift;
                    crate::app::switch_workspace_from_view(cx, window_id, id, move_here);
                }),
            )
            .gitcomet_tooltip(theme, rail_tooltip(entry).into())
    }
}

impl Render for WorkspaceRail {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if !crate::workspaces::open_mode(cx).shows_rail() {
            return div().id("workspace_rail_hidden");
        }
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let entries = rail_entries(
            &crate::workspaces::rail_workspaces(cx),
            self.window_id,
            &crate::workspaces::window_numbers(cx),
        );
        let mut column = div()
            .id("workspace_rail")
            .debug_selector(|| "workspace_rail".to_string())
            .flex_none()
            .h_full()
            .w(scaled_px(RAIL_WIDTH_PX))
            .flex()
            .flex_col()
            .items_center()
            .gap(scaled_px(6.0))
            .py_2()
            .overflow_y_scroll()
            .bg(theme.colors.surface.chrome)
            .border_r_1()
            .border_color(theme.colors.stroke.subtle);
        for entry in &entries {
            column = column.child(self.tile(entry, &scaled_px, cx));
        }
        let window_id = self.window_id;
        column.child(
            div()
                .id("workspace_rail_new")
                .debug_selector(|| "workspace_rail_new".to_string())
                .flex()
                .items_center()
                .justify_center()
                .size(scaled_px(TILE_PX))
                .rounded(px(theme.radii.control))
                .border_1()
                .border_dashed()
                .border_color(theme.colors.stroke.default)
                .control_interaction(
                    crate::view::components::InteractionStyle::new(theme),
                    crate::view::components::InteractionState::default(),
                )
                .child(svg_icon(
                    "icons/plus.svg",
                    theme.colors.foreground.secondary,
                    scaled_px(14.0),
                ))
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |_this, _event: &ClickEvent, _window, cx| {
                        crate::app::release_workspace_from_view(cx, window_id);
                    }),
                )
                .gitcomet_tooltip(
                    theme,
                    "New workspace in this window. The current one stays saved.".into(),
                ),
        )
    }
}

/// A workspace switch under way: the window's repositories are closing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct PendingWorkspaceSwitch {
    /// The workspace to show once they closed, or Home without one.
    pub(in crate::view) target: Option<WorkspaceId>,
    /// The workspace the window let go of.
    pub(in crate::view) released: Option<WorkspaceId>,
    /// A window to show the released workspace in afterwards.
    pub(in crate::view) hand_off: Option<gpui::WindowId>,
}

impl GitCometView {
    /// Show `target` in this window, or Home without one, once the window's
    /// close guards agree; its current workspace stays saved. Returns whether
    /// a prompt took the switch over.
    pub(crate) fn request_switch_workspace(
        &mut self,
        target: Option<WorkspaceId>,
        hand_off: Option<gpui::WindowId>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.pending_workspace_switch.is_some() {
            return true;
        }
        let window_id = self.window_handle.window_id();
        self.flush_workspace_environment(cx);
        if self.request_unsaved_file_edits_prompt(
            UnsavedFileEditsAction::SwitchWorkspace {
                window_id,
                target,
                hand_off,
            },
            cx,
        ) {
            return true;
        }
        let action = TerminalShutdownAction::SwitchWorkspace { target, hand_off };
        if self.request_terminal_shutdown_action(action.clone(), cx)
            || self.request_late_close_guards(&action, cx)
        {
            return true;
        }
        self.end_terminals_for_workspace_switch();
        self.begin_workspace_switch(target, hand_off, cx);
        false
    }

    fn end_terminals_for_workspace_switch(&mut self) {
        for session in self.terminal_sessions.values() {
            for instance in &session.instances {
                shutdown_terminal_instance(instance, true);
            }
        }
    }

    /// Every guard passed: save the workspace, let it go, and close its
    /// repositories. The switch completes once the store has closed them.
    pub(in crate::view) fn begin_workspace_switch(
        &mut self,
        target: Option<WorkspaceId>,
        hand_off: Option<gpui::WindowId>,
        cx: &mut gpui::Context<Self>,
    ) {
        let window_id = self.window_handle.window_id();
        // The workspace keeps the repositories it shows now.
        self.sync_workspace_and_registry(cx);
        let released = self.workspace_id;
        crate::workspaces::release_window_workspace(cx, window_id);
        self.set_workspace_id(None, cx);
        self.pending_repo_open_reservations.clear();
        self.pending_repo_open_active = None;
        self.startup_repo_bootstrap_pending = false;
        self.deferred_repo_bootstrap = None;
        self.pending_workspace_switch = Some(PendingWorkspaceSwitch {
            target,
            released,
            hand_off,
        });
        let repo_ids: Vec<RepoId> = self
            .store
            .snapshot()
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect();
        if repo_ids.is_empty() {
            self.finish_workspace_switch(cx);
        } else {
            self.store.dispatch(Msg::CloseRepos {
                repo_ids,
                activate_after: None,
            });
        }
        cx.notify();
    }

    /// While a switch closes the old repositories, the window's syncs must
    /// not write them into the workspace being opened. Returns whether the
    /// sync should stop here.
    pub(in crate::view) fn workspace_switch_holds_sync(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.pending_workspace_switch.is_none() {
            return false;
        }
        if !self.state.repos.is_empty() {
            return true;
        }
        self.finish_workspace_switch(cx);
        true
    }

    fn finish_workspace_switch(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(pending) = self.pending_workspace_switch.take() else {
            return;
        };
        let window_id = self.window_handle.window_id();
        let workspace = pending
            .target
            .filter(|id| !crate::workspaces::is_open_in_a_window(cx, *id))
            .and_then(|id| crate::workspaces::workspace(cx, id));
        match workspace {
            Some(workspace) => self.adopt_workspace(workspace, cx),
            None => {
                self.sync_workspace_and_registry(cx);
                self.workspace_changed(cx);
            }
        }
        if let (Some(released), Some(hand_off)) = (pending.released, pending.hand_off)
            && hand_off != window_id
        {
            cx.defer(move |cx| crate::app::switch_workspace_in_window(cx, hand_off, released));
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_come_from_words_or_capitals() {
        assert_eq!(workspace_initials("GitComet"), "GC");
        assert_eq!(workspace_initials("GitComet Pro"), "GP");
        assert_eq!(workspace_initials("auto-explore"), "AE");
        assert_eq!(workspace_initials("gitcomet_web"), "GW");
        assert_eq!(workspace_initials("agent"), "AG");
        assert_eq!(workspace_initials("x"), "X");
        assert_eq!(workspace_initials("  "), "?");
    }

    fn workspace(name: &str) -> Workspace {
        let mut workspace = Workspace::new(Vec::new());
        workspace.custom_name = Some(name.to_string());
        workspace
    }

    #[test]
    fn entries_mark_this_window_and_number_the_others() {
        let workspaces = (0..11)
            .map(|index| workspace(&format!("Space {index}")))
            .collect::<Vec<_>>();
        let here = gpui::WindowId::from(7u64);
        let there = gpui::WindowId::from(9u64);
        let windows = vec![(here, workspaces[1].id, 1), (there, workspaces[2].id, 2)];
        let entries = rail_entries(&workspaces, here, &windows);
        assert_eq!(entries.len(), 11);
        assert!(!entries[0].current && entries[0].elsewhere.is_none());
        assert!(entries[1].current);
        assert_eq!(entries[1].elsewhere, None);
        assert!(!entries[2].current);
        assert_eq!(entries[2].elsewhere, Some(2));
        assert_eq!(entries[0].slot, Some(1));
        assert_eq!(entries[8].slot, Some(9));
        assert_eq!(entries[9].slot, None);
        assert_eq!(entries[1].initials, "S1");
    }

    #[test]
    fn tooltips_say_what_a_click_does() {
        let workspaces = vec![workspace("GitComet"), workspace("Other")];
        let here = gpui::WindowId::from(1u64);
        let there = gpui::WindowId::from(2u64);
        let entries = rail_entries(&workspaces, here, &[(there, workspaces[1].id, 2)]);
        assert_eq!(
            rail_tooltip(&entries[0]),
            "Show GitComet in this window (Alt+1)"
        );
        assert_eq!(
            rail_tooltip(&entries[1]),
            "Other (Alt+2) is open in window 2. Click to go there, Shift+click to show it here."
        );
    }
}
