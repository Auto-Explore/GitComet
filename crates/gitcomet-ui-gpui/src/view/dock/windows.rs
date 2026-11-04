//! Panels in a window of their own ("Open in New Window"). While a panel is
//! popped out its dock leaves it out; closing the window puts it back.

use super::hosts::HostKey;
use super::*;
use gpui::AnyWindowHandle;

/// A popped-out panel's window and the view it shows.
pub(in crate::view) struct PoppedOutPanel {
    pub(in crate::view) window: AnyWindowHandle,
    /// Panes made for the window, kept alive while it is open.
    _content: gpui::AnyView,
}

struct DockPopOutView {
    content: gpui::AnyView,
    _main_closed: gpui::Subscription,
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
            let _ = popped.window.update(cx, |_, window, _| window.activate());
            return;
        }
        let Some(content) = self.pop_out_content(panel, cx) else {
            return;
        };
        let main = self.window_handle.window_id();
        let title = panel_title(&self.dock_layout, panel);
        let (options, percent) = crate::view::pop_out::window_options(main, &title, cx);
        let root = cx.entity().downgrade();
        let closed_panel = panel.clone();
        let window_content = content.clone();
        let handle = cx.open_window(options, move |window, cx| {
            let id = window.window_handle().window_id();
            crate::ui_scale::follow_window(cx, id, main);
            crate::theme::follow_window_theme(cx, id, main);
            crate::ui_scale::apply_to_window(window, percent);
            let popped = window.window_handle();
            window.on_window_should_close(cx, move |_, cx| {
                let panel = closed_panel.clone();
                let _ = root.update(cx, |root, cx| root.return_popped_out_panel(&panel, cx));
                true
            });
            cx.new(|cx| DockPopOutView {
                content: window_content,
                _main_closed: cx.on_window_closed(move |cx, closed| {
                    if closed == main {
                        let _ = popped.update(cx, |_, window, _| window.remove_window());
                    }
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
                _content: content,
            },
        );
        cx.notify();
    }

    /// The window closed: the panel goes back to its dock.
    fn return_popped_out_panel(&mut self, panel: &PanelId, cx: &mut gpui::Context<Self>) {
        if self.dock_popped_out.remove(panel).is_some() {
            self.update_dock_layout(|layout| layout.show_panel(panel), cx);
            cx.notify();
        }
    }

    fn pop_out_content(
        &mut self,
        panel: &PanelId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::AnyView> {
        let kind = PanelKind::of(panel);
        if kind.is_sidebar() {
            let config = panes::SidebarDockConfig {
                tabs: panes::SidebarTabSet::from_panels([panel.as_str()]),
                sections: (kind == PanelKind::Branches)
                    .then(|| self.pop_out_branch_sections())
                    .flatten(),
                header: true,
                extension_tabs: false,
            };
            // Its own pane, so the window does not share one with the dock.
            let pane = self.dock_sidebar_host(
                HostKey::Sections(PanelId::new(format!("window:{panel}"))),
                config,
                cx,
            );
            return Some(pane.into());
        }
        match kind {
            PanelKind::Changes => {
                let sections = DEFAULT_CHANGES_SECTIONS.to_vec();
                let view =
                    self.dock_change_host(&PanelId::new(format!("window:{panel}")), sections, cx);
                Some(view.into())
            }
            PanelKind::Reflog => Some(self.reflog_pane.clone().into()),
            _ => None,
        }
    }

    fn pop_out_branch_sections(&self) -> Option<std::rc::Rc<[&'static str]>> {
        let sections: Vec<&'static str> = self
            .dock_layout
            .sections_in(&PanelId::from(panel_ids::BRANCHES))
            .iter()
            .filter(|section| section.kind() == SectionKind::References)
            .filter_map(|section| branch_sidebar::section_storage_key(section.as_str()))
            .collect();
        Some(sections.into())
    }
}
