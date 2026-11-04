//! The extra views a rearranged layout needs. The window always has one
//! sidebar pane and one details pane; these are the panes for the second
//! sidebar-like half, for reference sections moved into Changes or a custom
//! panel, and the slots that draw change sections outside Changes.
//!
//! The renderer asks for a host by key each frame; hosts it did not ask
//! for are dropped at the end of the frame, so a layout that goes back to
//! the factory one costs nothing.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Which view a host stands for.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::view) enum HostKey {
    /// The sidebar tabs of one dock half.
    Tabs(DockSide, DockHalf),
    /// The reference sections placed in one panel other than Branches.
    Sections(PanelId),
}

#[derive(Default)]
pub(in crate::view) struct DockHosts {
    sidebars: BTreeMap<HostKey, (Entity<SidebarPaneView>, gpui::Subscription)>,
    changes: BTreeMap<PanelId, Entity<ChangeSectionsView>>,
    used_sidebars: BTreeSet<HostKey>,
    used_changes: BTreeSet<PanelId>,
}

impl DockHosts {
    pub(in crate::view) fn begin_frame(&mut self) {
        self.used_sidebars.clear();
        self.used_changes.clear();
    }

    /// Drops the hosts this frame did not draw.
    pub(in crate::view) fn end_frame(&mut self) {
        let used = &self.used_sidebars;
        self.sidebars.retain(|key, _| used.contains(key));
        let used = &self.used_changes;
        self.changes.retain(|key, _| used.contains(key));
    }

    pub(in crate::view) fn sidebar_panes(&self) -> impl Iterator<Item = &Entity<SidebarPaneView>> {
        self.sidebars.values().map(|(pane, _)| pane)
    }

    pub(in crate::view) fn sidebar_for(&self, key: &HostKey) -> Option<Entity<SidebarPaneView>> {
        self.sidebars.get(key).map(|(pane, _)| pane.clone())
    }
}

impl GitCometView {
    /// A sidebar pane for `key`, made on first use from the main pane's
    /// collapse and pin state.
    pub(in crate::view) fn dock_sidebar_host(
        &mut self,
        key: HostKey,
        config: panes::SidebarDockConfig,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<SidebarPaneView> {
        self.dock_hosts.used_sidebars.insert(key.clone());
        let pane = match self.dock_hosts.sidebars.get(&key) {
            Some((pane, _)) => pane.clone(),
            None => {
                let (collapsed, pinned) = self.sidebar_pane.read(cx).collapse_and_pin_state();
                let store = Arc::clone(&self.store);
                let ui_model = self.ui_model.clone();
                let theme = self.theme;
                let root = cx.entity().downgrade();
                let tooltip_host = self.tooltip_host.downgrade();
                let file_history_pane = self.file_history_pane.clone();
                let pane = cx.new(|cx| {
                    SidebarPaneView::new(
                        store,
                        ui_model,
                        theme,
                        collapsed,
                        pinned,
                        root,
                        tooltip_host,
                        file_history_pane,
                        cx,
                    )
                });
                let subscription = cx
                    .subscribe(&pane, |_, _, _: &panes::SidebarHeaderChanged, cx| {
                        cx.notify()
                    });
                self.dock_hosts
                    .sidebars
                    .insert(key, (pane.clone(), subscription));
                pane
            }
        };
        let theme = self.theme;
        pane.update(cx, |pane, cx| {
            if pane.theme() != theme {
                pane.set_theme(theme, cx);
            }
            pane.set_dock_config(config, cx);
        });
        pane
    }

    /// The slot that draws `sections` (change sections) inside `panel`.
    pub(in crate::view) fn dock_change_host(
        &mut self,
        panel: &PanelId,
        sections: Vec<&'static str>,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<ChangeSectionsView> {
        self.dock_hosts.used_changes.insert(panel.clone());
        let view = match self.dock_hosts.changes.get(panel) {
            Some(view) => view.clone(),
            None => {
                let details = self.details_pane.clone();
                let view = cx.new(|cx| ChangeSectionsView::new(details, cx));
                self.dock_hosts.changes.insert(panel.clone(), view.clone());
                view
            }
        };
        view.update(cx, |view, cx| view.set_sections(sections, cx));
        view
    }
}

/// Draws some of the details pane's change sections somewhere else. The
/// sections keep their state (selection, scroll, the commit message) in the
/// details pane; this view only places them.
pub(in crate::view) struct ChangeSectionsView {
    details: Entity<DetailsPaneView>,
    sections: Vec<&'static str>,
    _details_changed: gpui::Subscription,
}

impl ChangeSectionsView {
    fn new(details: Entity<DetailsPaneView>, cx: &mut gpui::Context<Self>) -> Self {
        let subscription = cx.observe(&details, |_, _, cx| cx.notify());
        Self {
            details,
            sections: Vec::new(),
            _details_changed: subscription,
        }
    }

    fn set_sections(&mut self, sections: Vec<&'static str>, cx: &mut gpui::Context<Self>) {
        if self.sections != sections {
            self.sections = sections;
            cx.notify();
        }
    }
}

impl Render for ChangeSectionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let sections = self.sections.clone();
        let content = self.details.update(cx, |details, cx| {
            details.render_change_sections(&sections, cx)
        });
        div()
            .size_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(content)
    }
}
