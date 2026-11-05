//! Panels in a window of their own ("Open in New Window"). While a panel is
//! popped out its dock leaves it out; closing the window puts it back.

use super::hosts::ChangeSectionsView;
use super::*;
use gpui::AnyWindowHandle;

/// A popped-out panel's window and the view it shows.
pub(in crate::view) struct PoppedOutPanel {
    pub(in crate::view) window: AnyWindowHandle,
    /// Panes made for the window, kept alive while it is open.
    content: PopOutContent,
}

/// What a popped-out window draws.
enum PopOutContent {
    /// A sidebar pane of its own: the window themes it and counts its data
    /// request with the docked panes'.
    Sidebar {
        pane: Entity<SidebarPaneView>,
        _header_changed: gpui::Subscription,
    },
    Other(gpui::AnyView),
}

impl PopOutContent {
    fn view(&self) -> gpui::AnyView {
        match self {
            Self::Sidebar { pane, .. } => pane.clone().into(),
            Self::Other(view) => view.clone(),
        }
    }
}

impl PoppedOutPanel {
    pub(in crate::view) fn sidebar_pane_ref(&self) -> Option<&Entity<SidebarPaneView>> {
        match &self.content {
            PopOutContent::Sidebar { pane, .. } => Some(pane),
            PopOutContent::Other(_) => None,
        }
    }

    pub(in crate::view) fn sidebar_pane(&self) -> Option<Entity<SidebarPaneView>> {
        self.sidebar_pane_ref().cloned()
    }
}

struct DockPopOutView {
    content: gpui::AnyView,
    _main_closed: gpui::Subscription,
    _released: gpui::Subscription,
}

impl Render for DockPopOutView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = AppTheme::current(cx);
        div()
            .id("dock_pop_out")
            .debug_selector(|| "dock_pop_out".to_string())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .text_color(theme.colors.foreground.primary)
            .child(self.content.clone())
    }
}

impl GitCometView {
    pub(in crate::view) fn is_dock_panel_popped_out(&self, panel: &PanelId) -> bool {
        self.dock_popped_out.contains_key(panel)
    }

    /// Opens `panel` in a window of its own, or brings that window forward.
    pub(in crate::view) fn pop_out_dock_panel(
        &mut self,
        panel: &PanelId,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(popped) = self.dock_popped_out.get(panel) {
            if popped
                .window
                .update(cx, |_, window, _| window.activate())
                .is_ok()
            {
                return;
            }
            // The window went without telling us: forget it and open anew.
            self.return_popped_out_panel(panel, cx);
        }
        let Some(content) = self.pop_out_content(panel, cx) else {
            return;
        };
        let main = self.window_handle.window_id();
        let title = panel_title(&self.dock_layout, panel);
        let (options, percent) = crate::view::pop_out::window_options(main, &title, cx);
        let root = cx.entity().downgrade();
        let closed_panel = panel.clone();
        let window_content = content.view();
        let handle = cx.open_window(options, move |window, cx| {
            let id = window.window_handle().window_id();
            crate::ui_scale::follow_window(cx, id, main);
            crate::theme::follow_window_theme(cx, id, main);
            crate::ui_scale::apply_to_window(window, percent);
            let popped = window.window_handle();
            cx.new(|cx| DockPopOutView {
                content: window_content,
                _main_closed: cx.on_window_closed(move |cx, closed| {
                    if closed == main {
                        let _ = popped.update(cx, |_, window, _| window.remove_window());
                    }
                }),
                // However the window goes (its close button, Ctrl+W, its main
                // window closing), the panel goes back to its dock. Only an
                // OS close asks `on_window_should_close` first.
                _released: cx.on_release(move |_: &mut DockPopOutView, cx| {
                    let _ = root.update(cx, |root, cx| {
                        root.return_popped_out_panel(&closed_panel, cx)
                    });
                }),
            })
        });
        let Ok(handle) = handle else {
            return;
        };
        self.dock_popped_out.insert(
            panel.clone(),
            PoppedOutPanel {
                window: handle.into(),
                content,
            },
        );
        cx.notify();
    }

    /// The window closed: the panel goes back to its dock.
    fn return_popped_out_panel(&mut self, panel: &PanelId, cx: &mut gpui::Context<Self>) {
        let Some(popped) = self.dock_popped_out.remove(panel) else {
            return;
        };
        self.update_dock_layout(|layout| layout.show_panel(panel), cx);
        if popped.sidebar_pane_ref().is_some() {
            self.dispatch_sidebar_data_request(cx);
        }
        cx.notify();
    }

    fn pop_out_content(
        &mut self,
        panel: &PanelId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<PopOutContent> {
        let kind = PanelKind::of(panel);
        if kind.is_sidebar() {
            let config = panes::SidebarDockConfig {
                tabs: panes::SidebarTabSet::from_panels([panel.as_str()]),
                sections: (kind == PanelKind::Branches)
                    .then(|| self.reference_sections_in(panel).into()),
                header: true,
                extension_tabs: false,
            };
            // Its own pane, so the window does not share one with the dock.
            let (pane, header_changed) = self.new_dock_sidebar_pane(cx);
            pane.update(cx, |pane, cx| pane.set_dock_config(config, cx));
            return Some(PopOutContent::Sidebar {
                pane,
                _header_changed: header_changed,
            });
        }
        match kind {
            PanelKind::Changes => {
                let sections = self.change_sections_in(panel);
                let details = self.details_pane.clone();
                let view = cx.new(|cx| ChangeSectionsView::new(details, sections, cx));
                Some(PopOutContent::Other(view.into()))
            }
            PanelKind::Reflog => Some(PopOutContent::Other(self.reflog_pane.clone().into())),
            _ => None,
        }
    }
}
