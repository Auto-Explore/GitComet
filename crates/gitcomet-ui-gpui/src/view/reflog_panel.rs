use super::*;

/// Bottom panel's minimum height while the reflog panel is the sole (or
/// active) content. Matches the terminal panel's own floor so the resize
/// handle behaves identically regardless of which content is showing.
const REFLOG_PANEL_MIN_HEIGHT_PX: f32 = 120.0;
/// Height of the Terminal/Reflog switcher shown above the bottom panel's
/// content once more than one of its panels is open for the active repo.
const BOTTOM_PANEL_TAB_BAR_HEIGHT_PX: f32 = 30.0;

impl GitCometView {
    /// Whether the reflog panel is open for `repo_id`. The panel itself owns
    /// that fact (and its filter text, scroll position, and selection); the
    /// root only decides where to put it.
    pub(super) fn reflog_panel_is_open(&self, repo_id: RepoId, cx: &gpui::App) -> bool {
        self.reflog_pane.read(cx).is_open(repo_id)
    }

    /// Opens (or re-focuses) the persistent reflog panel for `repo_id` and
    /// brings it to the front of the bottom panel's tab switcher.
    pub(super) fn open_reflog_panel(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.reflog_pane
            .update(cx, |pane, cx| pane.open(repo_id, cx));
        self.active_bottom_panel
            .insert(repo_id, BottomPanelTab::Reflog);
        cx.notify();
    }

    /// Opens the reflog panel for whichever repository is active. The entry
    /// point for the application menus and the command palette, none of which
    /// carry a repository of their own.
    pub(crate) fn open_reflog_panel_for_active_repo(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(repo_id) = self.active_repo_id() {
            self.open_reflog_panel(repo_id, cx);
        }
    }

    /// Closes the reflog panel for `repo_id` from outside the panel — the tab
    /// strip's close button. The panel drops its own state and calls back into
    /// [`Self::on_reflog_panel_closed`], so both entry points converge.
    pub(super) fn close_reflog_panel(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.reflog_pane
            .update(cx, |pane, cx| pane.close(repo_id, cx));
        cx.notify();
    }

    /// Closes whichever bottom-panel content the clicked tab stands for. The
    /// terminal keeps its shutdown prompt: a tab with a live process asks first,
    /// exactly as its own close button does.
    fn close_bottom_panel_tab(
        &mut self,
        repo_id: RepoId,
        tab: BottomPanelTab,
        cx: &mut gpui::Context<Self>,
    ) {
        match tab {
            BottomPanelTab::Terminal => {
                if !self.request_close_terminal_for_repo(repo_id, cx) {
                    self.close_terminal_for_repo(repo_id, cx);
                }
            }
            BottomPanelTab::Reflog => self.close_reflog_panel(repo_id, cx),
        }
    }

    /// The panel closed itself: fall the bottom panel back to the terminal.
    pub(super) fn on_reflog_panel_closed(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        if self.active_bottom_panel.get(&repo_id) == Some(&BottomPanelTab::Reflog) {
            self.active_bottom_panel
                .insert(repo_id, BottomPanelTab::Terminal);
        }
        cx.notify();
    }

    /// Drops the remembered bottom-panel tab for any repository that is no
    /// longer open, mirroring `sync_terminal_sessions_with_state`'s cleanup of
    /// terminal sessions. The reflog panel's own per-repo state is pruned by
    /// the panel itself, on the same state snapshot.
    pub(super) fn sync_reflog_panels_with_state(&mut self) {
        if self.active_bottom_panel.is_empty() {
            return;
        }
        let active_repo_ids: FxHashSet<RepoId> =
            self.state.repos.iter().map(|repo| repo.id).collect();
        self.active_bottom_panel
            .retain(|repo_id, _| active_repo_ids.contains(repo_id));
    }

    /// The bottom panel: the terminal, the reflog panel, or — when both are
    /// open for the active repo — a small tab switcher above whichever one is
    /// currently selected. Mirrors `render_terminal_panel`'s `None` contract,
    /// so a repo with neither open still renders nothing here.
    ///
    /// When the reflog panel isn't open this returns exactly what
    /// `render_terminal_panel` would have returned on its own: the terminal's
    /// behavior and shape are unchanged from before this panel existed.
    pub(super) fn render_bottom_panel(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let repo_id = self.active_repo_id()?;
        let reflog_open = self.reflog_panel_is_open(repo_id, cx);

        if !reflog_open {
            return self.render_terminal_panel(theme, window, cx);
        }

        let terminal_open = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .is_some();

        if !terminal_open {
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .h(self.terminal_panel_height)
                    .min_h(px(REFLOG_PANEL_MIN_HEIGHT_PX))
                    .child(self.reflog_pane.clone())
                    .into_any(),
            );
        }

        let active_tab = self
            .active_bottom_panel
            .get(&repo_id)
            .copied()
            .unwrap_or(BottomPanelTab::Reflog);

        let tab_bar = self.render_bottom_panel_tab_bar(theme, repo_id, active_tab, cx);
        let content = match active_tab {
            BottomPanelTab::Terminal => self.render_terminal_panel(theme, window, cx)?,
            BottomPanelTab::Reflog => self.reflog_pane.clone().into_any_element(),
        };

        Some(
            div()
                .flex()
                .flex_col()
                .h(self.terminal_panel_height + px(BOTTOM_PANEL_TAB_BAR_HEIGHT_PX))
                .min_h(px(
                    REFLOG_PANEL_MIN_HEIGHT_PX + BOTTOM_PANEL_TAB_BAR_HEIGHT_PX
                ))
                .child(tab_bar)
                .child(div().flex_1().min_h(px(0.0)).child(content))
                .into_any(),
        )
    }

    /// Minimal two-way switcher between the terminal and the reflog panel,
    /// styled after the terminal panel's own per-instance tab row (see
    /// `render_terminal_header` in `terminal_panel.rs`) rather than the
    /// browser-style `components::Tab`/`TabBar`, which is sized and shaped for
    /// the top-level repository tab strip and would look out of place this
    /// far down the chrome.
    fn render_bottom_panel_tab_bar(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        active_tab: BottomPanelTab,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let tab = |id: &'static str,
                   close_id: &'static str,
                   icon: &'static str,
                   label: &'static str,
                   close_tip: &'static str,
                   this_tab: BottomPanelTab,
                   cx: &mut gpui::Context<Self>| {
            let is_active = this_tab == active_tab;
            let bg = if is_active {
                theme.colors.interaction.selected_background
            } else {
                theme.colors.surface.panel
            };
            let text_color = if is_active {
                theme.colors.interaction.selected_foreground
            } else {
                theme.colors.foreground.secondary
            };
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(theme.radii.row))
                .bg(bg)
                .text_color(text_color)
                .text_size(px(12.0))
                .cursor(CursorStyle::PointingHand)
                .when(!is_active, |d| {
                    d.hover(move |s| s.bg(theme.colors.interaction.hover_background))
                })
                .child(svg_icon(icon, text_color, px(12.0)))
                .child(label)
                .child(bottom_panel_tab_close(
                    theme, close_id, close_tip, text_color, repo_id, this_tab, cx,
                ))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                        this.active_bottom_panel.insert(repo_id, this_tab);
                        cx.notify();
                    }),
                )
        };

        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(4.0))
            .h(px(BOTTOM_PANEL_TAB_BAR_HEIGHT_PX))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(tab(
                "bottom_panel_tab_terminal",
                "bottom_panel_tab_terminal_close",
                "icons/terminal.svg",
                "Terminal",
                "Close terminal",
                BottomPanelTab::Terminal,
                cx,
            ))
            .child(tab(
                "bottom_panel_tab_reflog",
                "bottom_panel_tab_reflog_close",
                "icons/history.svg",
                "Reflog",
                "Close reflog",
                BottomPanelTab::Reflog,
                cx,
            ))
            .into_any()
    }
}

/// The `×` on a bottom-panel tab. Same shape as the terminal's own per-instance
/// tab close (see `render_terminal_header`): 14px hit target, danger-tinted
/// hover, and `stop_propagation` so closing a tab never also selects it.
#[allow(clippy::too_many_arguments)]
fn bottom_panel_tab_close(
    theme: AppTheme,
    id: &'static str,
    tip: &'static str,
    text_color: gpui::Rgba,
    repo_id: RepoId,
    tab: BottomPanelTab,
    cx: &mut gpui::Context<GitCometView>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.0))
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(with_alpha(theme.colors.status.danger.foreground, 0.18)))
        .child(svg_icon("icons/generic_close.svg", text_color, px(10.0)))
        .gitcomet_tooltip(theme, tip.into())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
                this.close_bottom_panel_tab(repo_id, tab, cx);
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::test_support::NoopBackend as TestBackend;
    use gitcomet_state::model::RepoState;
    use gitcomet_state::store::AppStore;
    use std::sync::Arc;

    fn view_with_active_repo(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<GitCometView>, RepoId, &mut gpui::VisualTestContext) {
        let repo_id = RepoId(1);
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let state = Arc::new(AppState {
            repos: vec![RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: std::path::PathBuf::from("/tmp/bottom-panel-test"),
                },
            )],
            active_repo: Some(repo_id),
            ..AppState::default()
        });
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.apply_state_snapshot(Arc::clone(&state), cx);
            });
        });
        (view, repo_id, cx)
    }

    /// The tab strip's `×` is the same gesture as the panel's own: the panel
    /// goes away and the bottom area falls back to the terminal, rather than
    /// leaving the strip pointing at content that is no longer there.
    #[gpui::test]
    fn closing_the_reflog_tab_returns_the_bottom_panel_to_the_terminal(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, repo_id, cx) = view_with_active_repo(cx);

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.open_reflog_panel(repo_id, cx);
                assert!(this.reflog_panel_is_open(repo_id, cx));
                assert_eq!(
                    this.active_bottom_panel.get(&repo_id),
                    Some(&BottomPanelTab::Reflog)
                );
            });
        });

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.close_bottom_panel_tab(repo_id, BottomPanelTab::Reflog, cx);
            });
        });
        // The panel calls back into the root through `cx.defer`.
        cx.run_until_parked();

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                assert!(
                    !this.reflog_panel_is_open(repo_id, cx),
                    "closing the tab should close the panel"
                );
                assert_eq!(
                    this.active_bottom_panel.get(&repo_id),
                    Some(&BottomPanelTab::Terminal),
                    "the strip should fall back to the terminal"
                );
            });
        });
    }

    /// The menus and the command palette have no repository of their own, so
    /// they go through the active one — and do nothing at all without it.
    #[gpui::test]
    fn opening_from_a_menu_targets_the_active_repository(cx: &mut gpui::TestAppContext) {
        let (view, repo_id, cx) = view_with_active_repo(cx);

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.open_reflog_panel_for_active_repo(cx);
                assert!(this.reflog_panel_is_open(repo_id, cx));
            });
        });
    }

    #[gpui::test]
    fn opening_from_a_menu_without_a_repository_is_inert(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.open_reflog_panel_for_active_repo(cx);
                assert!(
                    this.active_bottom_panel.is_empty(),
                    "no repository means nothing to show a reflog for"
                );
            });
        });
    }
}
