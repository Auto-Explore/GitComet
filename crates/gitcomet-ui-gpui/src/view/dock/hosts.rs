//! The extra views a rearranged layout needs. The window always has one
//! sidebar pane and one details pane; these are the panes for the second
//! sidebar-like half, for reference sections moved into Changes or a custom
//! panel, and the slots that draw change sections outside Changes.
//!
//! The renderer asks for a host by key each frame. A sidebar pane stays
//! while the layout still places what it draws, even in a frame that does
//! not draw it (another tab of its half shows, or its dock is collapsed), so
//! its scroll, filter and selection survive; a layout that goes back to the
//! factory one drops them all and costs nothing.

use super::*;
use gitcomet_state::model::SidebarDataRequest;
use std::collections::{BTreeMap, BTreeSet};

/// Which view a host stands for.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::view) enum HostKey {
    /// The sidebar tabs of one dock half.
    Tabs(DockSide, DockHalf),
    /// The reference sections placed in one panel other than Branches.
    Sections(PanelId),
}

impl HostKey {
    /// Whether `layout` still places what this pane draws: a sidebar panel
    /// in its half (other than the main pane's half, which holds Branches),
    /// or reference sections in its panel.
    fn still_placed(&self, layout: &DockLayout) -> bool {
        match self {
            Self::Tabs(side, half) => {
                let mut sidebar = false;
                for (panel, placement) in &layout.panels {
                    if placement.side != *side || placement.half != *half {
                        continue;
                    }
                    if panel.as_str() == panel_ids::BRANCHES {
                        return false;
                    }
                    sidebar |= PanelKind::of(panel).is_sidebar();
                }
                sidebar
            }
            Self::Sections(panel) => layout
                .sections_in(panel)
                .iter()
                .any(|section| section.kind() == SectionKind::References),
        }
    }
}

#[derive(Default)]
pub(in crate::view) struct DockHosts {
    sidebars: BTreeMap<HostKey, (Entity<SidebarPaneView>, gpui::Subscription)>,
    changes: BTreeMap<PanelId, Entity<ChangeSectionsView>>,
    used_sidebars: BTreeSet<HostKey>,
    used_changes: BTreeSet<PanelId>,
    /// The union of the sidebar panes' data requests the store was sent last.
    sent_sidebar_request: Option<(RepoId, SidebarDataRequest)>,
}

impl DockHosts {
    pub(in crate::view) fn begin_frame(&mut self) {
        self.used_sidebars.clear();
        self.used_changes.clear();
    }

    /// Drops the change slots this frame did not draw, and the sidebar panes
    /// it did not draw that `layout` no longer needs. Returns whether a
    /// sidebar pane went, so the window can send a smaller data request.
    pub(in crate::view) fn end_frame(&mut self, layout: &DockLayout) -> bool {
        let used = &self.used_sidebars;
        let before = self.sidebars.len();
        self.sidebars
            .retain(|key, _| used.contains(key) || key.still_placed(layout));
        let used = &self.used_changes;
        self.changes.retain(|key, _| used.contains(key));
        self.sidebars.len() != before
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
                let (pane, subscription) = self.new_dock_sidebar_pane(cx);
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

    /// A sidebar pane seeded from the main pane's collapse and pin state,
    /// redrawing the window when its header changes.
    pub(super) fn new_dock_sidebar_pane(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> (Entity<SidebarPaneView>, gpui::Subscription) {
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
        let subscription = cx.subscribe(&pane, |_, _, _: &panes::SidebarHeaderChanged, cx| {
            cx.notify()
        });
        (pane, subscription)
    }

    /// Sends the store what every sidebar pane's branch list needs, as one
    /// request. The store keeps a single request per repository and reloads
    /// only what it asks for, so panes sending their own would leave the
    /// last one's narrower request in charge.
    pub(in crate::view) fn dispatch_sidebar_data_request(&mut self, cx: &mut gpui::Context<Self>) {
        let mut union: Option<(RepoId, SidebarDataRequest)> = None;
        let panes = std::iter::once(&self.sidebar_pane)
            .chain(self.dock_hosts.sidebar_panes())
            .chain(
                self.dock_popped_out
                    .values()
                    .filter_map(PoppedOutPanel::sidebar_pane_ref),
            );
        for pane in panes {
            let Some((repo_id, request)) = pane.read(cx).sidebar_data_request() else {
                continue;
            };
            match &mut union {
                None => union = Some((repo_id, request)),
                Some((id, all)) if *id == repo_id => {
                    all.worktrees |= request.worktrees;
                    all.submodules |= request.submodules;
                    all.stashes |= request.stashes;
                    all.tags |= request.tags;
                }
                Some(_) => {}
            }
        }
        if union == self.dock_hosts.sent_sidebar_request {
            return;
        }
        self.dock_hosts.sent_sidebar_request = union;
        if let Some((repo_id, request)) = union {
            self.store
                .dispatch(Msg::EnsureSidebarData { repo_id, request });
        }
    }

    /// As [`Self::dispatch_sidebar_data_request`], after the current update:
    /// the panes it reads may be mid-update now.
    pub(in crate::view) fn schedule_sidebar_data_request(&self, cx: &mut gpui::Context<Self>) {
        let root = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = root.update(cx, |root, cx| root.dispatch_sidebar_data_request(cx));
        });
    }

    /// The slot that draws `sections` (change sections) inside `panel`.
    pub(in crate::view) fn dock_change_host(
        &mut self,
        panel: &PanelId,
        sections: Vec<&'static str>,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<ChangeSectionsView> {
        self.dock_hosts.used_changes.insert(panel.clone());
        match self.dock_hosts.changes.get(panel) {
            Some(view) => {
                view.update(cx, |view, cx| view.set_sections(sections, cx));
                view.clone()
            }
            None => {
                let details = self.details_pane.clone();
                let view = cx.new(|cx| ChangeSectionsView::new(details, sections, cx));
                self.dock_hosts.changes.insert(panel.clone(), view.clone());
                view
            }
        }
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
    pub(super) fn new(
        details: Entity<DetailsPaneView>,
        sections: Vec<&'static str>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&details, |_, _, cx| cx.notify());
        Self {
            details,
            sections,
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
