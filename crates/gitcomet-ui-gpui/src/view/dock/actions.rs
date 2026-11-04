//! Runs layout commands and builds the dock menus.
//!
//! Every change goes through [`GitCometView::update_dock_layout`]: it keeps
//! the old sidebar and details collapse flags (and so the bottom bar's
//! toggles and the collapsed rail) in step with the layout, tells the panes
//! which sections they draw, and saves the window.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MENU_SEQ: AtomicU64 = AtomicU64::new(1);

impl GitCometView {
    /// Applies `change` to the layout; returns whether it changed anything.
    pub(in crate::view) fn update_dock_layout(
        &mut self,
        change: impl FnOnce(&mut DockLayout) -> bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !change(&mut self.dock_layout) {
            return false;
        }
        self.sync_collapse_flags_from_layout(cx);
        self.sync_dock_section_hosts(cx);
        if self
            .dock_zoomed
            .is_some_and(|side| !self.dock_has_content(side, cx))
        {
            self.dock_zoomed = None;
        }
        self.schedule_ui_settings_persist(cx);
        cx.notify();
        true
    }

    /// The left and right docks' hidden flags are the old collapse flags.
    pub(in crate::view) fn sync_collapse_flags_from_layout(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let left = self.dock_layout.dock(DockSide::Left).hidden;
        if self.sidebar_collapsed != left {
            self.set_sidebar_collapsed(left, cx);
        }
        let right = self.dock_layout.dock(DockSide::Right).hidden;
        if self.details_collapsed != right {
            self.set_details_collapsed(right, cx);
        }
    }

    /// The centre shows history and a diff together when the layout says so.
    pub(in crate::view) fn sync_centre_split(&mut self, cx: &mut gpui::Context<Self>) {
        let centre = self.dock_layout.centre.clone();
        self.main_pane
            .update(cx, |pane, cx| pane.set_centre_layout(centre, cx));
    }

    pub(in crate::view) fn run_dock_command(
        &mut self,
        command: DockCommand,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        match command {
            DockCommand::TogglePanel(panel) => {
                if !self.dock_panel_present(&panel, cx) {
                    self.open_absent_panel(&panel, window, cx);
                    return;
                }
                let shows = self.dock_layout.is_panel_visible(&panel);
                if shows {
                    self.update_dock_layout(|layout| layout.hide_panel(&panel), cx);
                } else {
                    self.show_dock_panel(&panel, window, cx);
                }
            }
            DockCommand::ShowPanel(panel) => {
                if !self.dock_panel_present(&panel, cx) {
                    self.open_absent_panel(&panel, window, cx);
                    return;
                }
                self.show_dock_panel(&panel, window, cx);
            }
            DockCommand::HidePanel(panel) => {
                self.update_dock_layout(|layout| layout.hide_panel(&panel), cx);
            }
            DockCommand::MovePanel { panel, side, half } => {
                self.update_dock_layout(
                    |layout| {
                        let moved = layout.move_panel(&panel, side, half, None);
                        layout.show_panel(&panel) || moved
                    },
                    cx,
                );
                self.show_dock_panel(&panel, window, cx);
            }
            DockCommand::RestorePanel(panel) => {
                self.update_dock_layout(|layout| layout.restore_panel(&panel), cx);
                self.show_dock_panel(&panel, window, cx);
            }
            DockCommand::RestoreDefault => {
                self.dock_zoomed = None;
                self.update_dock_layout(DockLayout::restore_default, cx);
                cx.notify();
            }
            DockCommand::HideAll => {
                self.dock_zoomed = None;
                self.update_dock_layout(DockLayout::toggle_hide_all, cx);
            }
            DockCommand::Maximize => self.toggle_dock_zoom(cx),
            DockCommand::ShowSection(section) => self.show_dock_section(&section, window, cx),
            DockCommand::HideSection(section) => {
                self.update_dock_layout(|layout| layout.hide_section(&section), cx);
            }
            DockCommand::MoveSection { section, panel } => {
                self.update_dock_layout(
                    |layout| {
                        let moved = layout.move_section(&section, &panel, None);
                        layout.show_panel(&panel) || moved
                    },
                    cx,
                );
                self.show_dock_panel(&panel, window, cx);
            }
            DockCommand::MoveSectionBy { section, delta } => {
                self.update_dock_layout(|layout| move_section_by(layout, &section, delta), cx);
            }
            DockCommand::NewPanel(side) => {
                self.update_dock_layout(
                    |layout| {
                        layout.add_custom_panel("", side);
                        true
                    },
                    cx,
                );
            }
            DockCommand::NewPanelWithSection(section) => {
                self.update_dock_layout(
                    |layout| {
                        let side = layout
                            .section_panel(&section)
                            .and_then(|panel| layout.placement(&panel).map(|p| p.side))
                            .unwrap_or(DockSide::Left);
                        let title = section.title().to_owned();
                        let panel = layout.add_custom_panel(&title, side);
                        layout.move_section(&section, &panel, None);
                        true
                    },
                    cx,
                );
            }
            DockCommand::RemovePanel(panel) => {
                self.update_dock_layout(|layout| layout.remove_custom_panel(&panel), cx);
            }
            DockCommand::ApplyPreset(preset) => {
                self.dock_zoomed = None;
                self.update_dock_layout(|layout| layout.apply_preset(preset), cx);
            }
            DockCommand::SaveLayout => self.save_named_layout(cx),
            DockCommand::ApplySaved(index) => {
                let name = self
                    .named_layouts
                    .names()
                    .get(index.saturating_sub(1))
                    .cloned();
                match name {
                    Some(name) => self.apply_named_layout(&name, cx),
                    None => self.push_toast(
                        components::ToastKind::Warning,
                        format!("No saved layout {index}. Save one with Save Current Layout."),
                        cx,
                    ),
                }
            }
            DockCommand::DeleteSaved(index) => {
                let name = self
                    .named_layouts
                    .names()
                    .get(index.saturating_sub(1))
                    .cloned();
                if let Some(name) = name
                    && self.named_layouts.delete(&name)
                {
                    self.persist_named_layouts(cx);
                    self.push_toast(
                        components::ToastKind::Success,
                        format!("Deleted the saved layout {name}."),
                        cx,
                    );
                }
            }
            DockCommand::SetCentre(mode) => {
                self.update_dock_layout(
                    |layout| {
                        let changed = layout.centre.mode != mode;
                        layout.centre.mode = mode;
                        changed
                    },
                    cx,
                );
                self.sync_centre_split(cx);
            }
            DockCommand::PopOut(panel) => self.pop_out_dock_panel(&panel, cx),
            DockCommand::ResetDockSize(side) => self.reset_dock_size(side, cx),
            DockCommand::ResizeDock { side, grow } => self.resize_dock(side, grow, cx),
        }
    }

    /// Shows a panel and switches the pane that hosts it to its tab.
    fn show_dock_panel(
        &mut self,
        panel: &PanelId,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_dock_layout(|layout| layout.show_panel(panel), cx);
        if let Some(placement) = self.dock_layout.placement(panel) {
            self.dock_focus_side = placement.side;
        }
        if !PanelKind::of(panel).is_sidebar() {
            return;
        }
        let Some(window) = window else {
            return;
        };
        let panel_id = panel.as_str().to_owned();
        let is_main = self.dock_layout.placement(panel).map(|p| (p.side, p.half))
            == self
                .dock_layout
                .placement(&PanelId::from(panel_ids::BRANCHES))
                .map(|p| (p.side, p.half));
        let pane = if is_main {
            Some(self.sidebar_pane.clone())
        } else {
            self.dock_layout.placement(panel).and_then(|placement| {
                self.dock_hosts
                    .sidebar_for(&super::hosts::HostKey::Tabs(placement.side, placement.half))
            })
        };
        if let Some(pane) = pane {
            pane.update(cx, |pane, cx| pane.show_dock_panel(&panel_id, window, cx));
        }
    }

    /// Terminal and Reflog are opened, not shown: open them for the active
    /// repository. File History needs a file, so it says how to get one.
    fn open_absent_panel(
        &mut self,
        panel: &PanelId,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        match PanelKind::of(panel) {
            PanelKind::Terminal => {
                if let Some(window) = window {
                    self.toggle_terminal_for_active_repo(window, cx);
                }
            }
            PanelKind::Reflog => {
                if let Some(repo_id) = self.active_repo_id() {
                    self.open_reflog_panel(repo_id, cx);
                }
            }
            PanelKind::FileHistory => self.push_toast(
                components::ToastKind::Warning,
                "File History opens from a file's menu: choose Show File History.".into(),
                cx,
            ),
            _ => {}
        }
    }

    /// Shows a section: puts a hidden one back in its default panel and
    /// brings its panel up.
    fn show_dock_section(
        &mut self,
        section: &SectionId,
        window: Option<&mut Window>,
        cx: &mut gpui::Context<Self>,
    ) {
        let default_panel = section.default_panel();
        self.update_dock_layout(|layout| layout.show_section(section, &default_panel), cx);
        if let Some(panel) = self.dock_layout.section_panel(section) {
            self.show_dock_panel(&panel, window, cx);
        }
    }

    fn toggle_dock_zoom(&mut self, cx: &mut gpui::Context<Self>) {
        if self.dock_zoomed.take().is_none() {
            let side = self.dock_focus_side;
            let side = if self.dock_has_content(side, cx) {
                Some(side)
            } else {
                DockSide::ALL
                    .into_iter()
                    .find(|side| self.dock_has_content(*side, cx))
            };
            if let Some(side) = side {
                self.update_dock_layout(|layout| layout.set_dock_hidden(side, false), cx);
                self.dock_zoomed = Some(side);
            }
        }
        cx.notify();
    }

    fn reset_dock_size(&mut self, side: DockSide, cx: &mut gpui::Context<Self>) {
        let scale = self.ui_scale();
        match side {
            DockSide::Left => {
                self.set_sidebar_width_from_pixels(scale.px(DEFAULT_LEFT_DOCK_WIDTH));
                if !self.sidebar_collapsed {
                    self.sidebar_render_width = self.sidebar_width;
                }
            }
            DockSide::Right => {
                self.set_details_width_from_pixels(scale.px(DEFAULT_RIGHT_DOCK_WIDTH));
                if !self.details_collapsed {
                    self.details_render_width = self.details_width;
                }
            }
            DockSide::Bottom => {
                self.terminal_panel_height = px(TERMINAL_PANEL_DEFAULT_HEIGHT_PX);
            }
        }
        self.clamp_pane_widths_to_window();
        self.dock_layout
            .set_dock_split(side, gitcomet_state::session::DEFAULT_SPLIT_PERMILLE);
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    /// One keyboard step: 40 design units, within the docks' limits.
    fn resize_dock(&mut self, side: DockSide, grow: bool, cx: &mut gpui::Context<Self>) {
        let step = self.ui_scale().px(if grow { 40.0 } else { -40.0 });
        match side {
            DockSide::Left => {
                let width = (self.sidebar_width + step).max(self.sidebar_min_width());
                self.set_sidebar_width_from_pixels(width);
                if !self.sidebar_collapsed {
                    self.sidebar_render_width = self.sidebar_width;
                }
            }
            DockSide::Right => {
                let width = (self.details_width + step).max(self.details_min_width());
                self.set_details_width_from_pixels(width);
                if !self.details_collapsed {
                    self.details_render_width = self.details_width;
                }
            }
            DockSide::Bottom => {
                self.terminal_panel_height = (self.terminal_panel_height + step).max(px(
                    crate::view::terminal_panel::TERMINAL_PANEL_MIN_HEIGHT_PX,
                ));
            }
        }
        self.clamp_pane_widths_to_window();
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    /// Once per install: says that panels move and how to get today's
    /// window back.
    pub(in crate::view) fn maybe_show_dock_hint(&mut self, cx: &mut gpui::Context<Self>) {
        if self.named_layouts.hint_seen || !crate::ui_runtime::current().persists_ui_settings() {
            return;
        }
        self.named_layouts.hint_seen = true;
        self.persist_named_layouts(cx);
        let root = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = root.update(cx, |this, cx| {
                this.push_toast(
                    components::ToastKind::Success,
                    "Panels and sections can move now: drag a panel's tab or a section header, \
                     or right-click it. Restore Today's Sidebar, in the Branches menu or the \
                     command palette, brings the old layout back."
                        .into(),
                    cx,
                );
            });
        });
    }

    fn save_named_layout(&mut self, cx: &mut gpui::Context<Self>) {
        let index = self.named_layouts.layouts.len() + 1;
        let name = format!("Layout {index}");
        let layout = self.dock_layout.clone();
        let saved = self.named_layouts.save(&name, &layout).unwrap_or(name);
        self.persist_named_layouts(cx);
        let position = self
            .named_layouts
            .names()
            .iter()
            .position(|other| *other == saved)
            .map_or(index, |ix| ix + 1);
        let shortcut = if cfg!(target_os = "macos") {
            format!("Cmd+Alt+{position}")
        } else {
            format!("Ctrl+Alt+{position}")
        };
        self.push_toast(
            components::ToastKind::Success,
            format!("Saved the layout as {saved}. {shortcut} brings it back."),
            cx,
        );
    }

    fn apply_named_layout(&mut self, name: &str, cx: &mut gpui::Context<Self>) {
        let Some(layout) = self.named_layouts.get(name).cloned() else {
            return;
        };
        self.dock_zoomed = None;
        self.update_dock_layout(
            |current| {
                let changed = *current != layout;
                *current = layout;
                changed
            },
            cx,
        );
        self.sync_centre_split(cx);
    }

    fn persist_named_layouts(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().persists_ui_settings() {
            return;
        }
        let layouts = self.named_layouts.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = gitcomet_state::session::persist_named_layouts(&layouts);
            })
            .detach();
    }

    /// Opens the menu for a panel or a section at `position`.
    pub(in crate::view) fn open_dock_menu(
        &mut self,
        target: DockMenuTarget,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let menu = build_dock_menu(&self.dock_layout, &target);
        self.open_popover_at(PopoverKind::DockMenu(Arc::new(menu)), position, window, cx);
    }

    /// The window's dock actions, for the root element.
    pub(in crate::view) fn register_dock_actions<E: InteractiveElement>(
        &self,
        element: E,
        cx: &mut gpui::Context<Self>,
    ) -> E {
        element
            .on_action(cx.listener(|this, action: &ToggleDockPanel, window, cx| {
                if let Some(panel) = action
                    .index
                    .checked_sub(1)
                    .and_then(|ix| BUILTIN_PANELS.get(ix))
                {
                    this.run_dock_command(
                        DockCommand::TogglePanel(PanelId::from(*panel)),
                        Some(window),
                        cx,
                    );
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, action: &ApplySavedLayout, window, cx| {
                this.run_dock_command(DockCommand::ApplySaved(action.index), Some(window), cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &HideAllDocks, window, cx| {
                this.run_dock_command(DockCommand::HideAll, Some(window), cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &MaximizeDock, window, cx| {
                this.run_dock_command(DockCommand::Maximize, Some(window), cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &RestoreDefaultLayout, window, cx| {
                this.run_dock_command(DockCommand::RestoreDefault, Some(window), cx);
                cx.stop_propagation();
            }))
    }
}

/// Width the left dock starts at, in design units (today's sidebar).
pub(in crate::view) const DEFAULT_LEFT_DOCK_WIDTH: f32 = 280.0;
/// Width the right dock starts at, in design units (today's details pane).
pub(in crate::view) const DEFAULT_RIGHT_DOCK_WIDTH: f32 = 420.0;

/// Moves a section one place up (`delta < 0`) or down within its panel.
fn move_section_by(layout: &mut DockLayout, section: &SectionId, delta: i32) -> bool {
    let Some(panel) = layout.section_panel(section) else {
        return false;
    };
    let sections = layout.sections_in(&panel).to_vec();
    let Some(index) = sections.iter().position(|id| id == section) else {
        return false;
    };
    let before = if delta < 0 {
        match index.checked_sub(1) {
            Some(previous) => Some(sections[previous].clone()),
            None => return false,
        }
    } else {
        if index + 1 >= sections.len() {
            return false;
        }
        sections.get(index + 2).cloned()
    };
    layout.move_section(section, &panel, before.as_ref())
}

fn item(label: impl Into<SharedString>, command: DockCommand) -> DockMenuEntry {
    DockMenuEntry::Item {
        label: label.into(),
        command,
        checked: false,
        disabled: false,
    }
}

/// The section hosts a section can move to, in menu order.
fn section_hosts(layout: &DockLayout) -> Vec<PanelId> {
    let mut hosts = vec![
        PanelId::from(panel_ids::BRANCHES),
        PanelId::from(panel_ids::CHANGES),
    ];
    hosts.extend(layout.custom_panels.iter().map(|custom| custom.id.clone()));
    hosts
}

pub(in crate::view) fn build_dock_menu(layout: &DockLayout, target: &DockMenuTarget) -> DockMenu {
    let entries = match target {
        DockMenuTarget::Panel(panel) => panel_menu(layout, panel),
        DockMenuTarget::Section(section) => section_menu(layout, section),
    };
    DockMenu {
        seq: NEXT_MENU_SEQ.fetch_add(1, Ordering::Relaxed),
        entries,
    }
}

fn panel_menu(layout: &DockLayout, panel: &PanelId) -> Vec<DockMenuEntry> {
    let title = panel_title(layout, panel);
    let placement = layout.placement(panel).cloned();
    let mut entries = vec![
        DockMenuEntry::Header(title.clone()),
        DockMenuEntry::Separator,
    ];
    for side in DockSide::ALL {
        for half in DockHalf::ALL {
            let here = placement
                .as_ref()
                .is_some_and(|placement| placement.side == side && placement.half == half);
            entries.push(DockMenuEntry::Item {
                label: format!("Move to {}", side_label(side, half)).into(),
                command: DockCommand::MovePanel {
                    panel: panel.clone(),
                    side,
                    half,
                },
                checked: here,
                disabled: here,
            });
        }
    }
    entries.push(DockMenuEntry::Separator);
    entries.push(DockMenuEntry::Item {
        label: format!("Hide {title}").into(),
        command: DockCommand::HidePanel(panel.clone()),
        checked: false,
        disabled: !layout.is_panel_visible(panel),
    });
    entries.push(item("Maximize Dock", DockCommand::Maximize));
    if super::commands::can_pop_out(panel) {
        entries.push(item(
            "Open in New Window",
            DockCommand::PopOut(panel.clone()),
        ));
    }
    match PanelKind::of(panel) {
        PanelKind::Custom => {
            entries.push(item(
                "Remove Panel",
                DockCommand::RemovePanel(panel.clone()),
            ));
        }
        PanelKind::Branches => entries.push(item(
            "Restore Today's Sidebar",
            DockCommand::RestorePanel(panel.clone()),
        )),
        PanelKind::Unknown => {}
        _ => entries.push(item(
            format!("Restore {title} to Its Default Place"),
            DockCommand::RestorePanel(panel.clone()),
        )),
    }
    if let Some(placement) = placement.as_ref() {
        entries.push(item(
            format!("New Panel on the {}", placement.side.label()),
            DockCommand::NewPanel(placement.side),
        ));
    }
    if layout.is_section_host(panel) {
        entries.push(DockMenuEntry::Separator);
        entries.push(DockMenuEntry::Header("Sections".into()));
        for section in layout.sections_in(panel) {
            entries.push(item(
                format!("Hide {}", section.title()),
                DockCommand::HideSection(section.clone()),
            ));
        }
        for section in gitcomet_state::session::builtin_sections() {
            if layout.section_panel(&section).as_ref() == Some(panel) {
                continue;
            }
            entries.push(item(
                format!("Add {} Here", section.title()),
                DockCommand::MoveSection {
                    section,
                    panel: panel.clone(),
                },
            ));
        }
    }
    entries.push(DockMenuEntry::Separator);
    entries.push(item("Restore Default Layout", DockCommand::RestoreDefault));
    entries
}

fn section_menu(layout: &DockLayout, section: &SectionId) -> Vec<DockMenuEntry> {
    let current = layout.section_panel(section);
    let siblings = current
        .as_ref()
        .map(|panel| layout.sections_in(panel).to_vec())
        .unwrap_or_default();
    let index = siblings.iter().position(|id| id == section);
    let mut entries = vec![
        DockMenuEntry::Header(section.title().to_owned().into()),
        DockMenuEntry::Separator,
        DockMenuEntry::Item {
            label: "Move Up".into(),
            command: DockCommand::MoveSectionBy {
                section: section.clone(),
                delta: -1,
            },
            checked: false,
            disabled: index.is_none_or(|ix| ix == 0),
        },
        DockMenuEntry::Item {
            label: "Move Down".into(),
            command: DockCommand::MoveSectionBy {
                section: section.clone(),
                delta: 1,
            },
            checked: false,
            disabled: index.is_none_or(|ix| ix + 1 >= siblings.len()),
        },
        DockMenuEntry::Separator,
    ];
    for panel in section_hosts(layout) {
        let here = current.as_ref() == Some(&panel);
        entries.push(DockMenuEntry::Item {
            label: format!("Move to {}", panel_title(layout, &panel)).into(),
            command: DockCommand::MoveSection {
                section: section.clone(),
                panel,
            },
            checked: here,
            disabled: here,
        });
    }
    entries.push(item(
        "Move to a New Panel",
        DockCommand::NewPanelWithSection(section.clone()),
    ));
    entries.push(item(
        format!("Hide {}", section.title()),
        DockCommand::HideSection(section.clone()),
    ));
    entries.push(DockMenuEntry::Separator);
    let restore = match section.kind() {
        SectionKind::References => (
            "Restore Today's Sidebar",
            PanelId::from(panel_ids::BRANCHES),
        ),
        SectionKind::Changes => (
            "Restore the Changes Panel",
            PanelId::from(panel_ids::CHANGES),
        ),
    };
    entries.push(item(restore.0, DockCommand::RestorePanel(restore.1)));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(menu: &DockMenu) -> Vec<String> {
        menu.entries
            .iter()
            .filter_map(|entry| match entry {
                DockMenuEntry::Item {
                    label, disabled, ..
                } if !disabled => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_branches_menu_can_rebuild_todays_sidebar_and_add_tags() {
        let layout = DockLayout::factory();
        let menu = build_dock_menu(
            &layout,
            &DockMenuTarget::Panel(PanelId::from(panel_ids::BRANCHES)),
        );
        let labels = labels(&menu);
        assert!(
            labels
                .iter()
                .any(|label| label == "Restore Today's Sidebar")
        );
        assert!(labels.iter().any(|label| label == "Add Tags Here"));
        assert!(labels.iter().any(|label| label == "Hide Stashes"));
        assert!(!labels.iter().any(|label| label == "Move to Left"));
    }

    #[test]
    fn moving_a_section_down_and_up_returns_it_to_its_place() {
        let mut layout = DockLayout::factory();
        let stashes = SectionId::from(section_ids::STASHES);
        let local = SectionId::from(section_ids::LOCAL_BRANCHES);
        assert!(!move_section_by(&mut layout, &stashes, 1));
        assert!(move_section_by(&mut layout, &local, 1));
        let branches = PanelId::from(panel_ids::BRANCHES);
        assert_eq!(layout.sections_in(&branches)[1], local);
        assert!(move_section_by(&mut layout, &local, -1));
        assert_eq!(layout, DockLayout::factory());
    }

    #[test]
    fn the_section_menu_offers_every_host_but_its_own() {
        let layout = DockLayout::factory();
        let menu = build_dock_menu(
            &layout,
            &DockMenuTarget::Section(SectionId::from(section_ids::STAGED)),
        );
        let labels = labels(&menu);
        assert!(labels.iter().any(|label| label == "Move to Branches"));
        assert!(!labels.iter().any(|label| label == "Move to Changes"));
        assert!(labels.iter().any(|label| label == "Move to a New Panel"));
    }
}
