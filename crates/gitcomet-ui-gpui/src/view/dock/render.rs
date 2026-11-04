//! Draws the docks from the layout. The factory layout takes the native
//! paths below, so today's window looks and behaves as it always has:
//!
//! - a half holding only Branches, Files and File History is the sidebar
//!   pane with its own tab strip;
//! - a half holding only Changes is the details pane;
//! - a bottom half holding only Terminal and Reflog is the bottom panel.
//!
//! Any other mix gets a plain tab strip over the shown panel's content.

use super::hosts::HostKey;
use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use std::rc::Rc;

/// The drag that resizes the two halves of one dock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DockSplitDrag(pub(in crate::view) DockSide);

const DOCK_SPLIT_HANDLE_PX: f32 = 5.0;

/// How a panel is drawn when it shares a half only with its own kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Family {
    Sidebar,
    Changes,
    Bottom,
    Other,
}

fn family(panel: &PanelId) -> Family {
    let kind = PanelKind::of(panel);
    if kind.is_sidebar() {
        Family::Sidebar
    } else if kind.is_bottom_kind() {
        Family::Bottom
    } else if kind == PanelKind::Changes {
        Family::Changes
    } else {
        Family::Other
    }
}

/// One half with something to draw.
pub(in crate::view) struct DockHalfTabs {
    pub(in crate::view) half: DockHalf,
    pub(in crate::view) tabs: Vec<PanelId>,
    pub(in crate::view) shown: PanelId,
}

impl GitCometView {
    /// Whether the window can draw `panel` now. Terminal and Reflog exist
    /// while open and File History while it has a file, as today.
    pub(in crate::view) fn dock_panel_present(&self, panel: &PanelId, cx: &gpui::App) -> bool {
        if self.is_dock_panel_popped_out(panel) {
            return false;
        }
        let repo_id = self.active_repo_id();
        match PanelKind::of(panel) {
            PanelKind::Terminal => repo_id.is_some_and(|repo_id| {
                self.bottom_panel_providers.iter().any(|provider| {
                    !matches!(provider.tab(), BottomPanelTab::Reflog)
                        && provider.is_open(self, repo_id, cx)
                })
            }),
            PanelKind::Reflog => {
                repo_id.is_some_and(|repo_id| self.reflog_panel_is_open(repo_id, cx))
            }
            PanelKind::FileHistory => self
                .file_history_pane
                .read(cx)
                .target
                .as_ref()
                .is_some_and(|(target, _)| Some(*target) == repo_id),
            PanelKind::Unknown => false,
            PanelKind::Branches | PanelKind::Files | PanelKind::Changes | PanelKind::Custom => true,
        }
    }

    /// The halves of `side` that have a panel to draw, ignoring whether the
    /// dock is collapsed.
    pub(in crate::view) fn dock_halves(&self, side: DockSide, cx: &gpui::App) -> Vec<DockHalfTabs> {
        DockHalf::ALL
            .into_iter()
            .filter_map(|half| {
                let tabs: Vec<PanelId> = self
                    .dock_layout
                    .panels_in(side, half)
                    .into_iter()
                    .filter(|panel| self.dock_panel_present(panel, cx))
                    .collect();
                let active = self.dock_layout.dock(side).active_in(half);
                let shown = active
                    .filter(|active| tabs.contains(active))
                    .or_else(|| tabs.first())?
                    .clone();
                // A split half the user closed stays closed until a panel
                // is shown there again.
                if half == DockHalf::Split && self.dock_layout.dock(side).active_split.is_none() {
                    return None;
                }
                Some(DockHalfTabs { half, tabs, shown })
            })
            .collect()
    }

    /// Whether `side` draws anything: shown and holding a present panel.
    pub(in crate::view) fn dock_has_content(&self, side: DockSide, cx: &gpui::App) -> bool {
        !self.dock_halves(side, cx).is_empty()
    }

    /// Where the main sidebar pane lives: the half that holds Branches.
    fn main_sidebar_half(&self) -> Option<(DockSide, DockHalf)> {
        self.dock_layout
            .placement(&PanelId::from(panel_ids::BRANCHES))
            .map(|placement| (placement.side, placement.half))
    }

    /// Whether the collapsed-sidebar rail belongs on the left: it lists the
    /// Branches sections, so it shows only while Branches is docked there.
    pub(in crate::view) fn dock_left_has_rail(&self) -> bool {
        self.main_sidebar_half()
            .is_some_and(|(side, _)| side == DockSide::Left)
    }

    /// Shows panels that just became present (a terminal opened, a file's
    /// history was asked for) and keeps the old collapse flags in step.
    pub(in crate::view) fn sync_dock_presence(&mut self, cx: &mut gpui::Context<Self>) {
        let present: Vec<PanelId> = BUILTIN_PANELS
            .iter()
            .map(|panel| PanelId::from(*panel))
            .filter(|panel| self.dock_panel_present(panel, cx))
            .collect();
        let mut changed = false;
        for panel in &present {
            if !self.dock_present.contains(panel)
                && matches!(
                    PanelKind::of(panel),
                    PanelKind::Terminal | PanelKind::Reflog | PanelKind::FileHistory
                )
            {
                changed |= self.dock_layout.show_panel(panel);
            }
        }
        if present != self.dock_present {
            self.dock_present = present;
        }
        if changed {
            self.sync_collapse_flags_from_layout(cx);
        }
    }

    /// The reference sections the Branches panel lists, as header keys, or
    /// `None` for today's sections in today's order.
    fn branches_sections(&self) -> Option<Rc<[&'static str]>> {
        let sections = self.reference_sections_in(&PanelId::from(panel_ids::BRANCHES));
        let factory: Vec<&'static str> = DEFAULT_BRANCHES_SECTIONS
            .iter()
            .filter_map(|id| branch_sidebar::section_storage_key(id))
            .collect();
        (sections != factory).then(|| sections.into())
    }

    fn reference_sections_in(&self, panel: &PanelId) -> Vec<&'static str> {
        self.dock_layout
            .sections_in(panel)
            .iter()
            .filter(|section| section.kind() == SectionKind::References)
            .filter_map(|section| branch_sidebar::section_storage_key(section.as_str()))
            .collect()
    }

    fn change_sections_in(&self, panel: &PanelId) -> Vec<&'static str> {
        self.dock_layout
            .sections_in(panel)
            .iter()
            .filter(|section| section.kind() == SectionKind::Changes)
            .filter_map(static_section_id)
            .collect()
    }

    /// Tells the main panes which sections they draw.
    pub(in crate::view) fn sync_dock_section_hosts(&mut self, cx: &mut gpui::Context<Self>) {
        let changes = self.change_sections_in(&PanelId::from(panel_ids::CHANGES));
        self.details_pane
            .update(cx, |pane, cx| pane.set_change_sections(changes, cx));
    }

    /// The whole dock on `side`, or `None` when it has nothing to draw.
    /// `zoomed` draws it to fill the window.
    pub(in crate::view) fn render_dock(
        &mut self,
        side: DockSide,
        zoomed: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let halves = self.dock_halves(side, cx);
        if halves.is_empty() {
            return None;
        }
        // Today's bottom panel sizes itself; anything else in the bottom
        // dock gets the bottom dock's height.
        let native_bottom = side == DockSide::Bottom
            && !zoomed
            && halves.len() == 1
            && halves[0]
                .tabs
                .iter()
                .all(|tab| family(tab) == Family::Bottom);
        let fill = !native_bottom;
        let mut elements: Vec<(DockHalf, AnyElement)> = Vec::new();
        for DockHalfTabs { half, tabs, shown } in halves {
            let element = self.render_dock_half(side, half, &tabs, &shown, fill, window, cx);
            elements.push((half, element));
        }
        let element = if elements.len() == 1 {
            elements.pop().map(|(_, element)| element)?
        } else {
            self.render_split_dock(side, elements, cx)
        };
        if side == DockSide::Bottom && fill && !zoomed {
            let tab_bar = self.ui_scale().px(30.0);
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .h(self.terminal_panel_height + tab_bar)
                    .min_h(px(0.0))
                    .child(element)
                    .into_any_element(),
            );
        }
        Some(element)
    }

    fn render_split_dock(
        &mut self,
        side: DockSide,
        elements: Vec<(DockHalf, AnyElement)>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let share = f32::from(self.dock_layout.dock(side).split_permille) / 1000.0;
        let vertical = side != DockSide::Bottom;
        let mut container = div()
            .id(("dock_split", side as usize))
            .flex()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .on_drag_move(cx.listener(
                move |this, e: &gpui::DragMoveEvent<DockSplitDrag>, _window, cx| {
                    if e.drag(cx).0 != side {
                        return;
                    }
                    let (offset, total) = if vertical {
                        (e.event.position.y - e.bounds.origin.y, e.bounds.size.height)
                    } else {
                        (e.event.position.x - e.bounds.origin.x, e.bounds.size.width)
                    };
                    let total: f32 = total.into();
                    if total <= 1.0 {
                        return;
                    }
                    let offset: f32 = offset.into();
                    let permille = ((offset / total) * 1000.0).round().clamp(0.0, 1000.0) as u16;
                    if this.dock_layout.set_dock_split(side, permille) {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                },
            ));
        container = if vertical {
            container.flex_col()
        } else {
            container.flex_row()
        };
        for (index, (_, element)) in elements.into_iter().enumerate() {
            let grow = if index == 0 { share } else { 1.0 - share };
            let mut slot = div()
                .flex()
                .flex_col()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .overflow_hidden()
                .child(element);
            slot.style().flex_grow = Some(grow.max(0.01));
            slot.style().flex_shrink = Some(1.0);
            slot.style().flex_basis = Some(relative(0.0).into());
            if index > 0 {
                container = container.child(self.dock_split_handle(side, theme, cx));
            }
            container = container.child(slot);
        }
        container.into_any_element()
    }

    fn dock_split_handle(
        &self,
        side: DockSide,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let vertical = side != DockSide::Bottom;
        let size = self.ui_scale().px(DOCK_SPLIT_HANDLE_PX);
        let id: SharedString = format!("dock_split_handle_{}", side_label(side, DockHalf::Primary))
            .to_lowercase()
            .into();
        let handle = div()
            .id(id.clone())
            .debug_selector({
                let id = id.clone();
                move || id.to_string()
            })
            .flex_none()
            .group(id.clone())
            .bg(theme.colors.stroke.subtle)
            .on_resize_drag(DockSplitDrag(side))
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, e: &gpui::ClickEvent, _window, cx| {
                    // Double-click puts the halves back to equal shares.
                    if e.click_count() == 2
                        && this
                            .dock_layout
                            .set_dock_split(side, gitcomet_state::session::DEFAULT_SPLIT_PERMILLE)
                    {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            );
        if vertical {
            handle
                .h(size)
                .w_full()
                .cursor(CursorStyle::ResizeUpDown)
                .into_any_element()
        } else {
            handle
                .w(size)
                .h_full()
                .cursor(CursorStyle::ResizeLeftRight)
                .into_any_element()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_dock_half(
        &mut self,
        side: DockSide,
        half: DockHalf,
        tabs: &[PanelId],
        shown: &PanelId,
        fill: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let families: Vec<Family> = tabs.iter().map(family).collect();
        let all = |wanted: Family| families.iter().all(|family| *family == wanted);
        if all(Family::Sidebar) {
            return self.render_sidebar_half(side, half, tabs, shown, window, cx);
        }
        if all(Family::Changes) {
            return self.render_changes_panel(window, cx);
        }
        if all(Family::Bottom)
            && let Some(element) = self.render_bottom_kinds(tabs, fill, window, cx)
        {
            return element;
        }
        let strip = self.render_dock_tab_strip(side, half, tabs, shown, cx);
        let content = self.render_panel_content(side, half, shown, window, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .child(strip)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(content),
            )
            .into_any_element()
    }

    /// The sidebar pane showing `tabs`: the main pane for the half holding
    /// Branches, another pane for any other.
    fn render_sidebar_half(
        &mut self,
        side: DockSide,
        half: DockHalf,
        tabs: &[PanelId],
        shown: &PanelId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let set = panes::SidebarTabSet::from_panels(tabs.iter().map(PanelId::as_str));
        let is_main = self.main_sidebar_half() == Some((side, half));
        let config = panes::SidebarDockConfig {
            tabs: set,
            sections: self.branches_sections(),
            header: true,
            extension_tabs: is_main,
        };
        let pane = if is_main {
            self.sidebar_pane
                .update(cx, |pane, cx| pane.set_dock_config(config, cx));
            self.sidebar_pane.clone()
        } else {
            self.dock_sidebar_host(HostKey::Tabs(side, half), config, cx)
        };
        // The pane's own tab strip picks the tab; the layout records it.
        if let Some(showing) = pane.read(cx).shown_dock_panel(cx)
            && showing != shown.as_str()
            && tabs.len() > 1
        {
            let _ = self.dock_layout.show_panel(&PanelId::from(showing));
        }
        let extension_sections = is_main
            .then(|| self.sidebar_section_content(window, cx))
            .flatten();
        let mut column = div().flex().flex_col().size_full().min_h(px(0.0)).child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .child(stable_cached_fill_view(pane.clone())),
        );
        if let Some(sections) = extension_sections {
            column = column.child(sections);
        }
        // Change sections placed in Branches draw under its list.
        if pane.read(cx).shown_dock_panel(cx) == Some(panel_ids::BRANCHES) {
            let branches = PanelId::from(panel_ids::BRANCHES);
            let changes = self.change_sections_in(&branches);
            if !changes.is_empty() {
                let host = self.dock_change_host(&branches, changes, cx);
                column = column.child(
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .border_t_1()
                        .border_color(self.theme.colors.stroke.subtle)
                        .child(stable_cached_fill_view(host)),
                );
            }
        }
        column.into_any_element()
    }

    /// Changes: today's details pane, with any reference sections moved
    /// into it drawn above.
    fn render_changes_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let changes = PanelId::from(panel_ids::CHANGES);
        let references = self.reference_sections_in(&changes);
        let details = match self.details_tab_content(cx) {
            Some(content) => content,
            None => div()
                .flex_1()
                .min_h(px(0.0))
                .child(stable_cached_fill_view(self.details_pane.clone()))
                .into_any_element(),
        };
        if references.is_empty() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .min_h(px(0.0))
                .child(details)
                .into_any_element();
        }
        let host = self.dock_sidebar_host(
            HostKey::Sections(changes),
            panes::SidebarDockConfig::sections_host(references.into()),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .border_b_1()
                    .border_color(self.theme.colors.stroke.subtle)
                    .child(stable_cached_fill_view(host)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .child(details),
            )
            .into_any_element()
    }

    /// Terminal and Reflog as the bottom panel draws them, limited to `tabs`.
    /// Extension bottom panels follow the terminal.
    fn render_bottom_kinds(
        &mut self,
        tabs: &[PanelId],
        fill: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let terminal = tabs.iter().any(|tab| tab.as_str() == panel_ids::TERMINAL);
        let reflog = tabs.iter().any(|tab| tab.as_str() == panel_ids::REFLOG);
        let theme = self.theme;
        self.render_bottom_panel_filtered(
            theme,
            &move |tab| match tab {
                BottomPanelTab::Reflog => reflog,
                _ => terminal,
            },
            fill,
            window,
            cx,
        )
    }

    fn render_panel_content(
        &mut self,
        side: DockSide,
        half: DockHalf,
        panel: &PanelId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        match family(panel) {
            Family::Sidebar => {
                self.render_sidebar_half(side, half, std::slice::from_ref(panel), panel, window, cx)
            }
            Family::Changes => self.render_changes_panel(window, cx),
            Family::Bottom => self
                .render_bottom_kinds(std::slice::from_ref(panel), true, window, cx)
                .unwrap_or_else(|| div().into_any_element()),
            Family::Other if PanelKind::of(panel) == PanelKind::Custom => {
                self.render_custom_panel(panel, cx)
            }
            Family::Other => components::empty_state(
                self.theme,
                panel_title(&self.dock_layout, panel),
                "This panel's extension is not loaded.",
            )
            .into_any_element(),
        }
    }

    /// A panel the user made: reference sections above change sections.
    fn render_custom_panel(&mut self, panel: &PanelId, cx: &mut gpui::Context<Self>) -> AnyElement {
        let references = self.reference_sections_in(panel);
        let changes = self.change_sections_in(panel);
        if references.is_empty() && changes.is_empty() {
            return components::empty_state(
                self.theme,
                panel_title(&self.dock_layout, panel),
                "Drag a section header here, or use a section's menu to move it here.",
            )
            .into_any_element();
        }
        let mut column = div().flex().flex_col().size_full().min_h(px(0.0));
        if !references.is_empty() {
            let host = self.dock_sidebar_host(
                HostKey::Sections(panel.clone()),
                panes::SidebarDockConfig::sections_host(references.into()),
                cx,
            );
            column = column.child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(stable_cached_fill_view(host)),
            );
        }
        if !changes.is_empty() {
            let host = self.dock_change_host(panel, changes, cx);
            column = column.child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .border_t_1()
                    .border_color(self.theme.colors.stroke.subtle)
                    .child(stable_cached_fill_view(host)),
            );
        }
        column.into_any_element()
    }

    /// The plain tab strip of a mixed half. Tabs select on click, drag to
    /// another dock, and open the panel menu on right-click.
    fn render_dock_tab_strip(
        &mut self,
        side: DockSide,
        half: DockHalf,
        tabs: &[PanelId],
        shown: &PanelId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let mut strip = div()
            .id(("dock_tab_strip", side as usize * 2 + half as usize))
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(ui_scale.px(2.0))
            .px(ui_scale.px(4.0))
            .py(ui_scale.px(4.0))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .overflow_x_hidden();
        for panel in tabs {
            let title = panel_title(&self.dock_layout, panel);
            let id: SharedString = format!("dock_tab_{panel}").into();
            let selected = panel == shown;
            let drag = DockPanelDrag {
                panel: panel.clone(),
                title: title.clone(),
            };
            let for_click = panel.clone();
            let for_menu = panel.clone();
            strip = strip.child(
                components::panel_tab(id, theme, ui_scale, panel_icon(panel), title, selected)
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &gpui::ClickEvent, window, cx| {
                            this.run_dock_command(
                                DockCommand::ShowPanel(for_click.clone()),
                                Some(window),
                                cx,
                            );
                        }),
                    )
                    .on_drag(drag, move |drag, _offset, _window, cx| {
                        cx.new(|_| DockDragPreview {
                            title: drag.title.clone(),
                            theme,
                        })
                    })
                    .on_pointer_click(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_dock_menu(
                                DockMenuTarget::Panel(for_menu.clone()),
                                e.position,
                                window,
                                cx,
                            );
                        }),
                    ),
            );
        }
        strip.into_any_element()
    }

    /// While a panel tab or a section header is dragged: where it can land.
    pub(in crate::view) fn render_dock_drop_overlay(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let drag = cx.active_drag()?;
        if drag.value.downcast_ref::<DockPanelDrag>().is_some() {
            return Some(self.render_panel_drop_zones(cx));
        }
        if drag.value.downcast_ref::<DockSectionDrag>().is_some() {
            return Some(self.render_section_drop_zones(cx));
        }
        None
    }

    fn render_panel_drop_zones(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let zone = |side: DockSide, half: DockHalf, cx: &mut gpui::Context<Self>| {
            let label = side_label(side, half);
            div()
                .id(SharedString::from(format!("dock_drop_{label}")))
                .debug_selector(move || format!("dock_drop_{label}"))
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .m(ui_scale.px(4.0))
                .rounded(px(theme.radii.row))
                .border_1()
                .border_color(with_alpha(theme.colors.accent.foreground, 0.5))
                .bg(with_alpha(theme.colors.surface.panel, 0.85))
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(label)
                .on_drag_over::<DockPanelDrag>(move |style, _, _, _| {
                    style
                        .bg(with_alpha(theme.colors.accent.foreground, 0.25))
                        .border_color(theme.colors.accent.foreground)
                })
                .on_drop(cx.listener(move |this, drag: &DockPanelDrag, window, cx| {
                    this.run_dock_command(
                        DockCommand::MovePanel {
                            panel: drag.panel.clone(),
                            side,
                            half,
                        },
                        Some(window),
                        cx,
                    );
                }))
        };
        let column = |side: DockSide, cx: &mut gpui::Context<Self>| {
            div()
                .flex()
                .flex_col()
                .w(relative(0.22))
                .h_full()
                .child(zone(side, DockHalf::Primary, cx))
                .child(zone(side, DockHalf::Split, cx))
        };
        let bottom = div()
            .flex()
            .flex_row()
            .h(relative(0.25))
            .w_full()
            .child(zone(DockSide::Bottom, DockHalf::Primary, cx))
            .child(zone(DockSide::Bottom, DockHalf::Split, cx));
        div()
            .id("dock_drop_overlay")
            .debug_selector(|| "dock_drop_overlay".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .justify_between()
                    .child(column(DockSide::Left, cx))
                    .child(column(DockSide::Right, cx)),
            )
            .child(bottom)
            .into_any_element()
    }

    fn render_section_drop_zones(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let mut hosts: Vec<PanelId> = vec![
            PanelId::from(panel_ids::BRANCHES),
            PanelId::from(panel_ids::CHANGES),
        ];
        hosts.extend(
            self.dock_layout
                .custom_panels
                .iter()
                .map(|custom| custom.id.clone()),
        );
        let target = |id: SharedString,
                      label: SharedString,
                      on_drop: Box<dyn Fn(&DockSectionDrag) -> DockCommand>,
                      cx: &mut gpui::Context<Self>| {
            let selector = id.clone();
            div()
                .id(id)
                .debug_selector(move || selector.to_string())
                .flex()
                .items_center()
                .justify_center()
                .px(ui_scale.px(16.0))
                .py(ui_scale.px(12.0))
                .rounded(px(theme.radii.row))
                .border_1()
                .border_color(with_alpha(theme.colors.accent.foreground, 0.5))
                .bg(theme.colors.surface.panel)
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.primary)
                .child(label)
                .on_drag_over::<DockSectionDrag>(move |style, _, _, _| {
                    style
                        .bg(with_alpha(theme.colors.accent.foreground, 0.25))
                        .border_color(theme.colors.accent.foreground)
                })
                .on_drop(
                    cx.listener(move |this, drag: &DockSectionDrag, window, cx| {
                        let command = on_drop(drag);
                        this.run_dock_command(command, Some(window), cx);
                    }),
                )
        };
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(ui_scale.px(8.0))
            .p(ui_scale.px(12.0))
            .rounded(px(theme.radii.row))
            .bg(with_alpha(theme.colors.surface.chrome, 0.92))
            .border_1()
            .border_color(theme.colors.stroke.default);
        for panel in hosts {
            let label: SharedString =
                format!("Move to {}", panel_title(&self.dock_layout, &panel)).into();
            let id: SharedString = format!("dock_section_drop_{panel}").into();
            let target_panel = panel.clone();
            row = row.child(target(
                id,
                label,
                Box::new(move |drag| DockCommand::MoveSection {
                    section: drag.section.clone(),
                    panel: target_panel.clone(),
                }),
                cx,
            ));
        }
        row = row.child(target(
            "dock_section_drop_new_panel".into(),
            "New Panel".into(),
            Box::new(|drag| DockCommand::NewPanelWithSection(drag.section.clone())),
            cx,
        ));
        row = row.child(target(
            "dock_section_drop_hide".into(),
            "Hide Section".into(),
            Box::new(|drag| DockCommand::HideSection(drag.section.clone())),
            cx,
        ));
        div()
            .id("dock_section_drop_overlay")
            .debug_selector(|| "dock_section_drop_overlay".to_string())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_end()
            .justify_center()
            .pb(ui_scale.px(24.0))
            .child(row)
            .into_any_element()
    }
}
