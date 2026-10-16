use super::super::*;
use super::diff::input::DiffPaneInput;
use gitcomet_core::domain::LogScope;
use rustc_hash::FxHashMap;

mod interactive_rebase;
mod state;
use state::{ICommitEditorMode, IRebaseDragState, IRebaseViewState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::view) enum MainPaneContent {
    History,
    Diff,
    InteractiveRebase,
}

pub(crate) struct MainPaneView {
    pub(in crate::view) diff_renderer: Entity<DiffRenderer>,
    pub(in crate::view) history_view: Entity<HistoryView>,
    state: Arc<AppState>,
    store: Arc<AppStore>,
    root_view: WeakEntity<GitCometView>,
    theme: AppTheme,
    view_mode: GitCometViewMode,
    active_context_menu_invoker: Option<SharedString>,
    pub(in crate::view) interactive_rebase_states: FxHashMap<RepoId, IRebaseViewState>,
    _subscription: gpui::Subscription,
}

impl MainPaneView {
    pub(in crate::view) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        diff_renderer: Entity<DiffRenderer>,
        root_view: WeakEntity<GitCometView>,
        tooltip_host: WeakEntity<TooltipHost>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = ui_model.read(cx).state.clone();
        let preferences = ui_model.read(cx).preferences.clone();
        let theme = diff_renderer.read(cx).theme;
        let view_mode = diff_renderer.read(cx).view_mode;
        let history_view = cx.new(|cx| {
            HistoryView::new(
                store.clone(),
                ui_model.clone(),
                theme,
                ui_scale::current(cx).percent,
                preferences.appearance.date_time_format,
                preferences.appearance.timezone,
                preferences.appearance.show_timezone,
                preferences.history.relative_dates,
                preferences.history.highlight_commit_chain,
                preferences.history.show_graph,
                preferences.history.show_graph_gradient,
                preferences.history.show_author,
                preferences.history.show_date,
                preferences.history.show_sha,
                preferences.history.show_tags,
                matches!(
                    preferences.history.tag_fetch_mode,
                    gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
                ),
                root_view.clone(),
                tooltip_host,
                window.viewport_size(),
                window,
                cx,
            )
        });
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = model.read(cx).state.clone();
            let changed =
                Self::notify_fingerprint_for(&this.state) != Self::notify_fingerprint_for(&next);
            if this.state.active_repo != next.active_repo {
                this.history_view.update(cx, |history, _| {
                    history
                        .history_scroll
                        .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                });
            }
            let diff_changed = this.diff_renderer.update(cx, |pane, cx| {
                pane.apply_input(DiffPaneInput::selected(next.clone()), cx)
            });
            this.state = next;
            this.sync_interactive_commit_editor_states();
            if changed || diff_changed {
                cx.notify();
            }
        });
        Self {
            diff_renderer,
            history_view,
            state,
            store,
            root_view,
            theme,
            view_mode,
            active_context_menu_invoker: None,
            interactive_rebase_states: FxHashMap::default(),
            _subscription: subscription,
        }
    }

    fn notify_fingerprint_for(state: &AppState) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        state.active_repo.hash(&mut hasher);
        if let Some(repo) = state
            .active_repo
            .and_then(|id| state.repos.iter().find(|repo| repo.id == id))
        {
            match &repo.interactive_rebase_setup {
                Some(setup) => {
                    1u8.hash(&mut hasher);
                    setup.base.hash(&mut hasher);
                    match &setup.entries {
                        Loadable::NotLoaded => 0u8.hash(&mut hasher),
                        Loadable::Loading => 1u8.hash(&mut hasher),
                        Loadable::Ready(_) => 2u8.hash(&mut hasher),
                        Loadable::Error(err) => {
                            3u8.hash(&mut hasher);
                            err.hash(&mut hasher);
                        }
                    }
                }
                None => {
                    0u8.hash(&mut hasher);
                }
            }
            match &repo.interactive_cherry_pick_setup {
                Some(setup) => {
                    1u8.hash(&mut hasher);
                    setup.entries.len().hash(&mut hasher);
                    for entry in &setup.entries {
                        entry.commit_id.hash(&mut hasher);
                        entry.summary.hash(&mut hasher);
                    }
                    setup.source_colors.hash(&mut hasher);
                    match &setup.full_messages {
                        Loadable::NotLoaded => 0u8.hash(&mut hasher),
                        Loadable::Loading => 1u8.hash(&mut hasher),
                        Loadable::Ready(()) => 2u8.hash(&mut hasher),
                        Loadable::Error(error) => {
                            3u8.hash(&mut hasher);
                            error.hash(&mut hasher);
                        }
                    }
                }
                None => 0u8.hash(&mut hasher),
            }
        }
        hasher.finish()
    }

    pub(in crate::view) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }
    pub(in crate::view) fn active_repo(&self) -> Option<&RepoState> {
        let id = self.active_repo_id()?;
        self.state.repos.iter().find(|repo| repo.id == id)
    }
    pub(in crate::view) fn active_content(&self, cx: &App) -> MainPaneContent {
        if self.diff_renderer.read(cx).rendered_diff_target().is_some() {
            MainPaneContent::Diff
        } else if self.active_repo().is_some_and(|repo| {
            repo.interactive_rebase_setup.is_some() || repo.interactive_cherry_pick_setup.is_some()
        }) {
            MainPaneContent::InteractiveRebase
        } else {
            MainPaneContent::History
        }
    }
    pub(in crate::view) fn history_is_active_surface(&self, cx: &App) -> bool {
        self.active_repo().is_some() && self.active_content(cx) == MainPaneContent::History
    }
    pub(in crate::view) fn open_search_for_active_view(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.history_is_active_surface(cx) {
            self.history_view
                .update(cx, |history, cx| history.open_history_find(window, cx));
            return true;
        }
        self.diff_renderer
            .update(cx, |pane, cx| pane.open_search_for_active_view(window, cx))
    }
    pub(in crate::view) fn navigate_search_match(
        &mut self,
        forward: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.history_is_active_surface(cx) {
            return self.history_view.update(cx, |history, cx| {
                history.history_find_is_open() && history.history_find_step(forward, cx)
            });
        }
        self.diff_renderer.update(cx, |pane, cx| {
            if forward {
                pane.navigate_next_search_match_or_diff_change(cx)
            } else {
                pane.navigate_prev_search_match_or_diff_change(cx)
            }
        })
    }
    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.history_view
            .update(cx, |view, cx| view.set_theme(theme, cx));
        cx.notify();
    }
    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        value: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.active_context_menu_invoker = value.clone();
        self.history_view.update(cx, |view, cx| {
            view.set_active_context_menu_invoker(value, cx)
        });
        cx.notify();
    }
    pub(in crate::view) fn sync_interactive_commit_editor_states(&mut self) {
        let repos_with_setup: Vec<RepoId> = self
            .state
            .repos
            .iter()
            .filter(|r| {
                r.interactive_rebase_setup.is_some() || r.interactive_cherry_pick_setup.is_some()
            })
            .map(|r| r.id)
            .collect();
        self.interactive_rebase_states
            .retain(|repo_id, _| repos_with_setup.contains(repo_id));
        for repo in self.state.repos.iter() {
            if let Some(setup) = repo.interactive_rebase_setup.as_ref() {
                let Loadable::Ready(entries) = &setup.entries else {
                    continue;
                };
                let replace = self
                    .interactive_rebase_states
                    .get(&repo.id)
                    .is_none_or(|st| {
                        st.mode != ICommitEditorMode::Rebase || st.original_entries != *entries
                    });
                if replace {
                    self.interactive_rebase_states.insert(
                        repo.id,
                        IRebaseViewState {
                            mode: ICommitEditorMode::Rebase,
                            entries: entries.clone(),
                            original_entries: entries.clone(),
                            ..Default::default()
                        },
                    );
                }
            } else if let Some(setup) = repo.interactive_cherry_pick_setup.as_ref() {
                if !matches!(setup.full_messages, Loadable::Ready(())) {
                    // Do not retain subject-only view-local entries from this
                    // or a replaced setup while full messages are pending.
                    self.interactive_rebase_states.remove(&repo.id);
                    continue;
                }
                let source_colors = setup
                    .source_colors
                    .iter()
                    .cloned()
                    .collect::<FxHashMap<_, _>>();
                // A repeated state application for the same setup must not
                // replace view-local reordering or action edits. A different
                // id set is a genuinely new setup.
                let same_plan =
                    self.interactive_rebase_states.get(&repo.id).is_some_and(
                        |st: &IRebaseViewState| {
                            st.mode == ICommitEditorMode::CherryPick
                                && st.original_entries.len() == setup.entries.len()
                                && st.original_entries.iter().zip(setup.entries.iter()).all(
                                    |(current, incoming)| current.commit_id == incoming.commit_id,
                                )
                        },
                    );
                if same_plan {
                    let st = self
                        .interactive_rebase_states
                        .get_mut(&repo.id)
                        .expect("same_plan implies the state exists");
                    for (current, incoming) in
                        st.original_entries.iter_mut().zip(setup.entries.iter())
                    {
                        current.message = incoming.message.clone();
                    }
                    for entry in st.entries.iter_mut() {
                        if let Some(incoming) = setup
                            .entries
                            .iter()
                            .find(|incoming| incoming.commit_id == entry.commit_id)
                        {
                            entry.message = incoming.message.clone();
                        }
                    }
                } else {
                    self.interactive_rebase_states.insert(
                        repo.id,
                        IRebaseViewState {
                            mode: ICommitEditorMode::CherryPick,
                            entries: setup.entries.clone(),
                            original_entries: setup.entries.clone(),
                            source_colors,
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }

    pub(in crate::view) fn reveal_history_commit(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.view_mode == GitCometViewMode::FocusedMergetool {
            self.diff_renderer.update(cx, |pane, cx| {
                pane.clear_diff_selection_or_exit(repo_id, cx)
            });
            return;
        }

        self.diff_renderer.update(cx, |pane, cx| {
            pane.clear_diff_selection_or_exit(repo_id, cx)
        });
        // Resolve and show the commit immediately; the history walk below only
        // has to find its row. Without this the details pane would sit on the
        // working tree — or flip in and out of it — for the whole walk.
        self.store.dispatch(Msg::RevealCommit {
            repo_id,
            reference: commit_id.clone(),
        });
        self.history_view.update(cx, |view, cx| {
            view.request_reveal_commit(repo_id, commit_id, fallback_scope, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn reveal_history_worktree(
        &mut self,
        repo_id: RepoId,
        worktree_path: std::path::PathBuf,
        is_current: bool,
        head: Option<CommitId>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.reveal_worktree(repo_id, worktree_path, is_current, head, cx);
        });
    }

    pub(in crate::view) fn reveal_history_branch_commit(
        &mut self,
        repo_id: RepoId,
        target: BranchMenuTarget,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_selected_branch(repo_id, target, cx);
        });
        self.reveal_history_commit(repo_id, commit_id, fallback_scope, cx);
    }

    pub(in crate::view) fn set_history_highlight_commit_chain(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_highlight_commit_chain(enabled, cx)
        });
        cx.notify();
    }

    pub(in crate::view) fn history_highlight_commit_chain(&self, cx: &App) -> bool {
        self.history_view.read(cx).history_highlight_commit_chain
    }

    pub(in crate::view) fn set_history_show_graph_gradient(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_show_graph_gradient(enabled, cx)
        });
        cx.notify();
    }

    pub(in crate::view) fn history_show_graph_gradient(&self, cx: &App) -> bool {
        self.history_view.read(cx).history_show_graph_gradient
    }

    pub(in crate::view) fn set_history_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_history_relative_dates(enabled, cx));
        cx.notify();
    }

    pub(in crate::view) fn history_relative_dates(&self, cx: &App) -> bool {
        self.history_view.read(cx).history_relative_dates
    }

    pub(in crate::view) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        self.history_view
            .update(cx, |view, cx| view.set_timezone(next, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_show_timezone(enabled, cx));
        cx.notify();
    }

    pub(in crate::view) fn history_visible_column_preferences(
        &self,
        cx: &gpui::App,
    ) -> (bool, bool, bool, bool) {
        self.history_view
            .read(cx)
            .history_visible_column_preferences()
    }

    pub(in crate::view) fn history_tag_preferences(&self, cx: &gpui::App) -> (bool, bool) {
        self.history_view.read(cx).history_tag_preferences()
    }

    pub(in crate::view) fn set_history_branch_names(
        &mut self,
        next: HistoryBranchNamesMode,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_history_branch_names(next, cx));
        cx.notify();
    }

    pub(in crate::view) fn set_history_column_preferences(
        &mut self,
        show_graph: bool,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_column_preferences(show_graph, show_author, show_date, show_sha, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn set_history_tag_preferences(
        &mut self,
        show_tags: bool,
        auto_fetch_tags_on_repo_activation: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view.update(cx, |view, cx| {
            view.set_history_tag_preferences(show_tags, auto_fetch_tags_on_repo_activation, cx);
        });
        cx.notify();
    }

    pub(in crate::view) fn reset_history_column_widths(&mut self, cx: &mut gpui::Context<Self>) {
        self.history_view.update(cx, |view, cx| {
            view.reset_history_column_widths();
            cx.notify();
        });
        cx.notify();
    }
}

impl Render for MainPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let width = self.diff_renderer.update(cx, |pane, cx| {
            pane.last_window_size = window.viewport_size();
            pane.sync_layout_snapshot(cx);
            pane.main_pane_content_width(cx)
        });
        self.history_view.update(cx, |history, _| {
            history.set_last_window_size(window.viewport_size());
            history.set_history_content_width(width);
        });
        match self.active_content(cx) {
            MainPaneContent::History => self.history_view.clone().into_any_element(),
            MainPaneContent::Diff => self.diff_renderer.clone().into_any_element(),
            MainPaneContent::InteractiveRebase => {
                self.interactive_rebase_view(window, cx).into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests;
