//! Built-in and extension panels share registration, selection and closing.
use super::*;
use std::rc::Rc;

pub(in crate::view) struct PanelLabels {
    pub id: SharedString,
    pub title: SharedString,
    pub icon: SharedString,
}
pub(in crate::view) trait BottomPanelProvider {
    fn tab(&self) -> BottomPanelTab;
    fn labels(&self) -> PanelLabels;
    fn is_open(&self, root: &GitCometView, repo: RepoId, cx: &App) -> bool;
    fn render(
        &self,
        root: &mut GitCometView,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<GitCometView>,
    ) -> Option<AnyElement>;
    fn close(&self, root: &mut GitCometView, repo: RepoId, cx: &mut gpui::Context<GitCometView>);
    fn owns_height(&self) -> bool {
        false
    }
}
struct Terminal;
impl BottomPanelProvider for Terminal {
    fn tab(&self) -> BottomPanelTab {
        BottomPanelTab::Terminal
    }
    fn labels(&self) -> PanelLabels {
        PanelLabels {
            id: "bottom_panel_tab_terminal".into(),
            title: "Terminal".into(),
            icon: "icons/terminal.svg".into(),
        }
    }
    fn is_open(&self, root: &GitCometView, repo: RepoId, _: &App) -> bool {
        root.terminal_sessions
            .get(&repo)
            .and_then(|session| session.active_instance())
            .is_some()
    }
    fn render(
        &self,
        root: &mut GitCometView,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<GitCometView>,
    ) -> Option<AnyElement> {
        root.render_terminal_panel(theme, window, cx)
    }
    fn close(&self, root: &mut GitCometView, repo: RepoId, cx: &mut gpui::Context<GitCometView>) {
        if !root.request_close_terminal_for_repo(repo, cx) {
            root.close_terminal_for_repo(repo, cx);
        }
    }
    fn owns_height(&self) -> bool {
        true
    }
}
struct Reflog;
impl BottomPanelProvider for Reflog {
    fn tab(&self) -> BottomPanelTab {
        BottomPanelTab::Reflog
    }
    fn labels(&self) -> PanelLabels {
        PanelLabels {
            id: "bottom_panel_tab_reflog".into(),
            title: "Reflog".into(),
            icon: "icons/history.svg".into(),
        }
    }
    fn is_open(&self, root: &GitCometView, repo: RepoId, cx: &App) -> bool {
        root.reflog_panel_is_open(repo, cx)
    }
    fn render(
        &self,
        root: &mut GitCometView,
        _: AppTheme,
        _: &mut Window,
        _: &mut gpui::Context<GitCometView>,
    ) -> Option<AnyElement> {
        Some(root.reflog_pane.clone().into_any_element())
    }
    fn close(&self, root: &mut GitCometView, repo: RepoId, cx: &mut gpui::Context<GitCometView>) {
        root.close_reflog_panel(repo, cx);
    }
}
struct ExtensionPanel {
    index: usize,
    title: SharedString,
    icon: SharedString,
}
impl BottomPanelProvider for ExtensionPanel {
    fn tab(&self) -> BottomPanelTab {
        BottomPanelTab::Extension(self.index)
    }
    fn labels(&self) -> PanelLabels {
        PanelLabels {
            id: format!("bottom_panel_tab_extension_{}", self.index).into(),
            title: self.title.clone(),
            icon: self.icon.clone(),
        }
    }
    fn is_open(&self, root: &GitCometView, _: RepoId, _: &App) -> bool {
        root.extension_panel_view(self.index).is_some()
    }
    fn render(
        &self,
        root: &mut GitCometView,
        _: AppTheme,
        _: &mut Window,
        _: &mut gpui::Context<GitCometView>,
    ) -> Option<AnyElement> {
        Some(
            div()
                .size_full()
                .child(root.extension_panel_view(self.index)?)
                .into_any_element(),
        )
    }
    fn close(&self, root: &mut GitCometView, _: RepoId, cx: &mut gpui::Context<GitCometView>) {
        root.close_extension_bottom_panel(self.index, cx);
    }
}

pub(in crate::view) type Providers = Rc<[Rc<dyn BottomPanelProvider>]>;
pub(in crate::view) fn registered(cx: &App) -> Providers {
    let mut panels: Vec<Rc<dyn BottomPanelProvider>> = vec![Rc::new(Terminal), Rc::new(Reflog)];
    if let Some(registry) = super::extension_host::registry(cx) {
        panels.extend(registry.bottom_panels().iter().enumerate().map(
            |(index, (_, descriptor))| {
                Rc::new(ExtensionPanel {
                    index,
                    title: descriptor.title.clone(),
                    icon: descriptor.icon.clone(),
                }) as Rc<dyn BottomPanelProvider>
            },
        ));
    }
    panels.into()
}
