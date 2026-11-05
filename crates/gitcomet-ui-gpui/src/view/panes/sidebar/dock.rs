//! How the dock layout shapes this pane: which of the built-in tabs it lists
//! and which reference sections its branch list draws, in what order.
//!
//! The window keeps one pane for the dock half that holds Branches (today's
//! sidebar) and adds panes for Files, File history or a group of sections
//! when the user moves them elsewhere. Each pane is told its share here.

use super::*;
use crate::view::gitcomet_view_requests::{RootRequest, RootRequests as _};
use gitcomet_state::session::panel_ids;

/// The built-in tabs a pane lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct SidebarTabSet {
    pub(in crate::view) branches: bool,
    pub(in crate::view) files: bool,
    pub(in crate::view) history: bool,
}

impl SidebarTabSet {
    pub(in crate::view) const ALL: Self = Self {
        branches: true,
        files: true,
        history: true,
    };

    pub(in crate::view) const fn only_branches() -> Self {
        Self {
            branches: true,
            files: false,
            history: false,
        }
    }

    /// The tabs for a list of dock panel ids; other ids are ignored.
    pub(in crate::view) fn from_panels<'a>(panels: impl IntoIterator<Item = &'a str>) -> Self {
        let mut set = Self {
            branches: false,
            files: false,
            history: false,
        };
        for panel in panels {
            match panel {
                panel_ids::BRANCHES => set.branches = true,
                panel_ids::FILES => set.files = true,
                panel_ids::FILE_HISTORY => set.history = true,
                _ => {}
            }
        }
        set
    }

    fn only_history(self) -> bool {
        self.history && !self.branches && !self.files
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct SidebarDockConfig {
    pub(in crate::view) tabs: SidebarTabSet,
    /// Header keys of the reference sections the branch list draws, in
    /// order; `None` draws today's sections in today's order.
    pub(in crate::view) sections: Option<Rc<[&'static str]>>,
    /// Whether the tab strip (with its search and locate tools) shows. A
    /// pane that only hosts sections moved into another panel hides it.
    pub(in crate::view) header: bool,
    /// Whether extension sidebar tabs list here; only the main pane takes them.
    pub(in crate::view) extension_tabs: bool,
}

impl Default for SidebarDockConfig {
    fn default() -> Self {
        Self {
            tabs: SidebarTabSet::ALL,
            sections: None,
            header: true,
            extension_tabs: true,
        }
    }
}

impl SidebarDockConfig {
    /// A pane showing only the reference sections moved into another panel.
    pub(in crate::view) fn sections_host(sections: Rc<[&'static str]>) -> Self {
        Self {
            tabs: SidebarTabSet::only_branches(),
            sections: Some(sections),
            header: false,
            extension_tabs: false,
        }
    }
}

impl SidebarPaneView {
    /// Apply the layout's share for this pane. Notifies only on a change:
    /// the root calls this on every render.
    pub(in crate::view) fn set_dock_config(
        &mut self,
        config: SidebarDockConfig,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.dock == config {
            return;
        }
        if self.dock.sections != config.sections {
            self.sticky_context = None;
            self.pending_sidebar_navigation = None;
            self.sidebar_scroll_animation = None;
        }
        self.dock = config;
        self.dispatch_sidebar_data_request_if_needed(cx);
        cx.emit(SidebarHeaderChanged);
        cx.notify();
    }

    pub(in crate::view) fn theme(&self) -> AppTheme {
        self.theme
    }

    /// The per-repository collapse and pin state, to seed another pane with.
    #[allow(clippy::type_complexity)]
    pub(in crate::view) fn collapse_and_pin_state(
        &self,
    ) -> (
        BTreeMap<std::path::PathBuf, BTreeSet<String>>,
        BTreeMap<std::path::PathBuf, BTreeSet<String>>,
    ) {
        (
            self.sidebar_collapsed_items_by_repo.clone(),
            self.sidebar_pinned_branches_by_repo.clone(),
        )
    }

    /// Branches or Files, as this pane shows them. A pane holding only one
    /// of the two always shows that one, whatever the window last asked for.
    pub(in crate::view) fn sidebar_mode(&self) -> SidebarMode {
        let tabs = self.dock.tabs;
        match (tabs.branches, tabs.files) {
            (true, false) => SidebarMode::Branches,
            (false, true) => SidebarMode::Files,
            _ => self.state.sidebar_mode,
        }
    }

    /// Whether the file history tab is the one showing.
    pub(super) fn dock_shows_history(&self, cx: &gpui::App) -> bool {
        self.dock.tabs.history
            && self.file_history_visible(cx)
            && (self.file_history_selected || self.dock.tabs.only_history())
    }

    /// The dock panel id of the tab this pane shows, for the layout to record
    /// which tab of a half is active. `None` while an extension tab shows.
    pub(in crate::view) fn shown_dock_panel(&self, cx: &gpui::App) -> Option<&'static str> {
        if self.dock_shows_history(cx) {
            return Some(panel_ids::FILE_HISTORY);
        }
        if self
            .extension_tabs
            .as_ref()
            .is_some_and(|tabs| tabs.selected.is_some() && tabs.view.is_some())
        {
            return None;
        }
        if !self.dock.tabs.branches && !self.dock.tabs.files {
            return None;
        }
        Some(match self.sidebar_mode() {
            SidebarMode::Branches => panel_ids::BRANCHES,
            SidebarMode::Files => panel_ids::FILES,
        })
    }

    /// Asks the window for the layout menu of a panel or a section. Deferred:
    /// opening it updates panes, this one among them.
    pub(in crate::view) fn open_dock_menu(
        &mut self,
        target: crate::view::dock::DockMenuTarget,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root = self.root_view.clone();
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                root.request(
                    RootRequest::OpenDockMenu { target, position },
                    Some(window),
                    cx,
                )
            });
        });
    }

    /// Select one of this pane's built-in tabs, as clicking it would.
    pub(in crate::view) fn show_dock_panel(
        &mut self,
        panel: &str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match panel {
            panel_ids::FILE_HISTORY => self.show_file_history(window, cx),
            panel_ids::BRANCHES | panel_ids::FILES => {
                let mode = if panel == panel_ids::FILES {
                    SidebarMode::Files
                } else {
                    SidebarMode::Branches
                };
                self.file_history_selected = false;
                if self.state.sidebar_mode != mode {
                    self.store.dispatch(Msg::SetSidebarMode { mode });
                }
                if self.dock.extension_tabs {
                    self.select_routed_tab(None, window, cx);
                }
                cx.emit(SidebarHeaderChanged);
                cx.notify();
            }
            _ => {}
        }
    }
}
