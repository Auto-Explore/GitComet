//! The window's docks: panels the user can move, hide, split and fill with
//! sections, drawn from a [`DockLayout`].
//!
//! The layout model lives in `gitcomet_state::session::dock_layout`; this
//! module names the built-in panels, turns palette rows, menu entries and key
//! bindings into [`DockCommand`]s, and renders the docks (`render`). Every
//! change goes through [`GitCometView::update_dock_layout`], which keeps the
//! old sidebar and details collapse flags in step and persists the window.

use super::*;
pub(in crate::view) use gitcomet_state::session::{
    CentreSplitMode, DEFAULT_BRANCHES_SECTIONS, DEFAULT_CHANGES_SECTIONS, DockHalf, DockLayout,
    DockSide, LayoutPreset, NamedLayouts, PanelId, SectionId, SectionKind, panel_ids, section_ids,
};
use schemars::JsonSchema;
use serde::Deserialize;

mod actions;
mod commands;
mod hosts;
mod render;
mod windows;

pub(crate) use commands::DockCommand;
pub(in crate::view) use commands::palette_entries;
pub(in crate::view) use hosts::DockHosts;
pub(in crate::view) use windows::PoppedOutPanel;

/// Shows or hides the n-th built-in panel (Alt+1..9, Cmd+1..9 on macOS).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, gpui::Action)]
#[action(namespace = dock)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToggleDockPanel {
    pub(crate) index: usize,
}

/// Applies the n-th saved layout (Ctrl+Alt+1..9, Cmd+Alt+1..9 on macOS).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, gpui::Action)]
#[action(namespace = dock)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplySavedLayout {
    pub(crate) index: usize,
}

gpui::actions!(
    dock,
    [
        /// Hides every dock; again brings them back (IntelliJ's Ctrl+Shift+F12).
        HideAllDocks,
        /// Lets the dock with the shown panel fill the window, or undoes that.
        MaximizeDock,
        RestoreDefaultLayout,
    ]
);

/// How many built-in panels have a number shortcut.
pub(crate) const BUILTIN_PANEL_COUNT: usize = BUILTIN_PANELS.len();

/// The built-in panels in the order their shortcuts number them.
pub(in crate::view) const BUILTIN_PANELS: [&str; 6] = [
    panel_ids::BRANCHES,
    panel_ids::CHANGES,
    panel_ids::FILES,
    panel_ids::FILE_HISTORY,
    panel_ids::TERMINAL,
    panel_ids::REFLOG,
];

/// What draws a panel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum PanelKind {
    Branches,
    Files,
    FileHistory,
    Changes,
    Terminal,
    Reflog,
    Custom,
    /// Registered by an extension the window does not draw as a panel.
    Unknown,
}

impl PanelKind {
    pub(in crate::view) fn of(panel: &PanelId) -> Self {
        match panel.as_str() {
            panel_ids::BRANCHES => Self::Branches,
            panel_ids::FILES => Self::Files,
            panel_ids::FILE_HISTORY => Self::FileHistory,
            panel_ids::CHANGES => Self::Changes,
            panel_ids::TERMINAL => Self::Terminal,
            panel_ids::REFLOG => Self::Reflog,
            _ if panel.is_custom() => Self::Custom,
            _ => Self::Unknown,
        }
    }

    /// Drawn by a sidebar pane (today's left-hand tabs).
    pub(in crate::view) fn is_sidebar(self) -> bool {
        matches!(self, Self::Branches | Self::Files | Self::FileHistory)
    }

    /// Opened on demand, like today's bottom panel.
    pub(in crate::view) fn is_bottom_kind(self) -> bool {
        matches!(self, Self::Terminal | Self::Reflog)
    }
}

pub(in crate::view) fn panel_icon(panel: &PanelId) -> &'static str {
    match PanelKind::of(panel) {
        PanelKind::Branches => "icons/git_branch.svg",
        PanelKind::Files => "icons/folder.svg",
        PanelKind::FileHistory => "icons/history.svg",
        PanelKind::Changes => "icons/git_commit.svg",
        PanelKind::Terminal => "icons/terminal.svg",
        PanelKind::Reflog => "icons/history.svg",
        PanelKind::Custom | PanelKind::Unknown => "icons/list_groups.svg",
    }
}

pub(in crate::view) fn panel_title(layout: &DockLayout, panel: &PanelId) -> SharedString {
    match PanelKind::of(panel) {
        PanelKind::Branches => "Branches".into(),
        PanelKind::Files => "Files".into(),
        PanelKind::FileHistory => "File History".into(),
        PanelKind::Changes => "Changes".into(),
        PanelKind::Terminal => "Terminal".into(),
        PanelKind::Reflog => "Reflog".into(),
        PanelKind::Custom => layout
            .custom_panel_title(panel)
            .unwrap_or("Panel")
            .to_owned()
            .into(),
        PanelKind::Unknown => panel
            .as_str()
            .strip_prefix(gitcomet_state::session::EXTENSION_ID_PREFIX)
            .unwrap_or(panel.as_str())
            .to_owned()
            .into(),
    }
}

pub(in crate::view) fn side_label(side: DockSide, half: DockHalf) -> &'static str {
    match (side, half) {
        (DockSide::Left, DockHalf::Primary) => "Left",
        (DockSide::Left, DockHalf::Split) => "Left, lower half",
        (DockSide::Right, DockHalf::Primary) => "Right",
        (DockSide::Right, DockHalf::Split) => "Right, lower half",
        (DockSide::Bottom, DockHalf::Primary) => "Bottom",
        (DockSide::Bottom, DockHalf::Split) => "Bottom, right half",
    }
}

/// The section ids the dock layout knows, with their kind, as `'static`.
pub(in crate::view) fn static_section_id(section: &SectionId) -> Option<&'static str> {
    gitcomet_state::session::builtin_sections()
        .position(|id| &id == section)
        .and_then(|ix| ALL_SECTIONS.get(ix).copied())
}

/// Every built-in section in catalogue order (matches `builtin_sections`).
pub(in crate::view) const ALL_SECTIONS: [&str; 11] = [
    section_ids::LOCAL_BRANCHES,
    section_ids::REMOTE_BRANCHES,
    section_ids::WORKTREES,
    section_ids::SUBMODULES,
    section_ids::ANNEX,
    section_ids::STASHES,
    section_ids::TAGS,
    section_ids::UNSTAGED,
    section_ids::UNTRACKED,
    section_ids::STAGED,
    section_ids::COMMIT_MESSAGE,
];

/// A tab being dragged to another dock.
#[derive(Clone, Debug)]
pub(in crate::view) struct DockPanelDrag {
    pub(in crate::view) panel: PanelId,
    pub(in crate::view) title: SharedString,
}

/// A section header being dragged to another panel.
#[derive(Clone, Debug)]
pub(in crate::view) struct DockSectionDrag {
    pub(in crate::view) section: SectionId,
    pub(in crate::view) title: SharedString,
}

/// The pill that follows the pointer during a dock drag.
pub(in crate::view) struct DockDragPreview {
    pub(in crate::view) title: SharedString,
    pub(in crate::view) theme: AppTheme,
}

impl Render for DockDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let scale = ui_scale::UiScale::current(cx);
        div()
            .px(scale.px(10.0))
            .py(scale.px(4.0))
            .rounded(px(theme.radii.row))
            .bg(theme.colors.surface.panel)
            .border_1()
            .border_color(theme.colors.accent.foreground)
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.primary)
            .child(self.title.clone())
    }
}

/// One row of a dock menu, built by the root from the layout it shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DockMenuEntry {
    Header(SharedString),
    Separator,
    Item {
        label: SharedString,
        command: DockCommand,
        checked: bool,
        disabled: bool,
    },
}

/// A panel's or a section's menu: where to move it, how to hide it and how
/// to get the factory arrangement back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DockMenu {
    /// Tells two menus apart for the popover's render cache.
    pub(crate) seq: u64,
    pub(crate) entries: Vec<DockMenuEntry>,
}

/// What a dock menu is about.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DockMenuTarget {
    Panel(PanelId),
    Section(SectionId),
}
