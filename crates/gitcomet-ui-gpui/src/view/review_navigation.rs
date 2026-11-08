//! Focus transitions for reviewing a commit, its files, and a diff.
use super::*;
use crate::view::panes::main::MainPaneContent;

pub(in crate::view) fn hint(
    theme: AppTheme,
    scale: crate::ui_scale::UiScale,
    text: &'static str,
) -> gpui::Div {
    div()
        .flex_shrink_0()
        .px(scale.px(8.0))
        .py(scale.px(3.0))
        .text_size(scale.px(10.0))
        .text_color(theme.colors.foreground.secondary)
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text)
}

#[derive(Clone, Copy)]
pub(in crate::view) enum ReviewNavigation {
    Files { from_graph: bool },
    Diff,
    Graph,
    WorkingCopy,
    Cycle(bool),
}

pub(in crate::view) fn defer(
    root: WeakEntity<GitCometView>,
    navigation: ReviewNavigation,
    window: &Window,
    cx: &mut App,
) {
    let handle = window.window_handle();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            let _ = root.update(cx, |root, cx| root.navigate_review(navigation, window, cx));
        });
    });
}

impl GitCometView {
    fn mounted_main_focus(&self, cx: &App) -> Option<FocusHandle> {
        if self.documents_active
            || self
                .repository_views
                .as_ref()
                .zip(self.active_repo())
                .is_some_and(|(router, repo)| router.active_view(repo).is_some())
        {
            return Some(self.repository_view_focus.clone());
        }
        let main = self.main_pane.read(cx);
        match main.active_content(cx) {
            MainPaneContent::History => Some(
                main.history_view
                    .read(cx)
                    .history_panel_focus_handle
                    .clone(),
            ),
            MainPaneContent::Diff => {
                Some(self.diff_renderer.read(cx).diff_panel_focus_handle.clone())
            }
            MainPaneContent::InteractiveRebase => None,
        }
    }

    pub(crate) fn defer_review_cycle(&self, backwards: bool, cx: &mut gpui::Context<Self>) {
        let root = cx.weak_entity();
        let window = self.window_handle;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = root.update(cx, |root, cx| {
                    root.navigate_review(ReviewNavigation::Cycle(backwards), window, cx)
                });
            });
        });
    }

    fn focus_review_graph(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.diff_renderer.update(cx, |pane, cx| {
            if let Some(repo_id) = pane.active_repo_id() {
                pane.remember_review_position();
                pane.clear_diff_selection_or_exit(repo_id, cx);
            }
        });
        let history = &self.main_pane.read(cx).history_view;
        let focus = history.read(cx).history_panel_focus_handle.clone();
        window.focus(&focus, cx);
    }

    pub(in crate::view) fn navigate_review(
        &mut self,
        navigation: ReviewNavigation,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.view_mode != GitCometViewMode::Normal
            || self.background_hovers_blocked(cx)
            || self.focused_terminal_viewport(window, cx).is_some()
        {
            return;
        }
        self.dismiss_hover_popovers(cx);
        match navigation {
            ReviewNavigation::Files { from_graph } => {
                let focused = self.details_pane.update(cx, |pane, cx| {
                    let focused = if from_graph {
                        pane.review_details_dismissed = false;
                        pane.focus_review_files(None, window, cx)
                    } else {
                        pane.restore_review_file_focus(window, cx)
                    };
                    cx.notify();
                    focused
                });
                if focused {
                    self.set_details_collapsed(false, cx);
                } else if !from_graph {
                    self.focus_review_graph(window, cx);
                } else if let Some(focus) = self.mounted_main_focus(cx) {
                    window.focus(&focus, cx);
                }
            }
            ReviewNavigation::Diff => {
                // The selected file may still be arriving from the store.
                // Its focus handle is stable across loading and loaded views.
                let focus = self.diff_renderer.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
            }
            ReviewNavigation::Graph => {
                self.focus_review_graph(window, cx);
            }
            ReviewNavigation::WorkingCopy => {
                self.set_details_collapsed(false, cx);
                let focus = self.details_pane.update(cx, |pane, cx| {
                    pane.review_details_dismissed = true;
                    cx.notify();
                    pane.visible_review_focus_handles().into_iter().next()
                });
                if let Some(focus) = focus.or_else(|| self.mounted_main_focus(cx)) {
                    window.focus(&focus, cx);
                }
            }
            ReviewNavigation::Cycle(backwards) => {
                let main = self.mounted_main_focus(cx);
                let mut handles = Vec::new();
                if !self.sidebar_collapsed {
                    handles.push(self.sidebar_pane.read(cx).sidebar_focus.clone());
                }
                let fallback = if main.is_some() { handles.len() } else { 0 };
                handles.extend(main);
                if !self.details_collapsed && self.active_details_view().is_none() {
                    handles.extend(self.details_pane.read(cx).visible_review_focus_handles());
                }
                let current = handles
                    .iter()
                    .position(|focus| focus.contains_focused(window, cx));
                let next = match current {
                    Some(ix) if backwards => (ix + handles.len() - 1) % handles.len(),
                    Some(ix) => (ix + 1) % handles.len(),
                    None => fallback,
                };
                if let Some(focus) = handles.get(next) {
                    window.focus(focus, cx);
                }
            }
        }
        cx.notify();
    }
}
