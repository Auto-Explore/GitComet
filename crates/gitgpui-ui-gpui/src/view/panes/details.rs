use super::super::fingerprint;
use super::super::path_display;
use super::super::*;
use std::hash::{Hash, Hasher};

pub(in super::super) struct DetailsPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    _commit_message_input_subscription: gpui::Subscription,
    root_view: WeakEntity<GitGpuiView>,
    tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: u64,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,

    pub(in super::super) unstaged_scroll: UniformListScrollHandle,
    pub(in super::super) staged_scroll: UniformListScrollHandle,
    pub(in super::super) commit_files_scroll: UniformListScrollHandle,
    pub(in super::super) commit_scroll: ScrollHandle,

    pub(in super::super) commit_message_input: Entity<zed::TextInput>,
    pub(in super::super) commit_details_message_input: Entity<zed::TextInput>,
    pub(in super::super) commit_message_user_edited: bool,
    pub(in super::super) commit_message_last_text: SharedString,
    pub(in super::super) commit_message_programmatic_change: bool,

    pub(in super::super) status_multi_selection: HashMap<RepoId, StatusMultiSelection>,
    pub(in super::super) status_multi_selection_last_status: HashMap<RepoId, Arc<RepoStatus>>,

    pub(in super::super) commit_details_delay: Option<CommitDetailsDelayState>,
    pub(in super::super) commit_details_delay_seq: u64,

    path_display_cache: std::cell::RefCell<HashMap<std::path::PathBuf, SharedString>>,
}

impl DetailsPaneView {
    fn notify_fingerprint(state: &AppState) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            fingerprint::hash_loadable_arc(&repo.status, &mut hasher);
            repo.local_actions_in_flight.hash(&mut hasher);
            repo.commit_in_flight.hash(&mut hasher);
            repo.selected_commit.hash(&mut hasher);
            fingerprint::hash_loadable_arc(&repo.commit_details, &mut hasher);

            fingerprint::hash_loadable_kind(&repo.merge_commit_message, &mut hasher);
            fingerprint::hash_loadable_kind(&repo.rebase_in_progress, &mut hasher);
        }

        hasher.finish()
    }

    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        root_view: WeakEntity<GitGpuiView>,
        tooltip_host: WeakEntity<TooltipHost>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.apply_state_snapshot(next, cx);
            cx.notify();
        });

        let commit_message_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "Enter commit message".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let commit_details_message_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: true,
                },
                window,
                cx,
            )
        });

        let commit_message_subscription = cx.observe(&commit_message_input, |this, input, cx| {
            let next: SharedString = input.read(cx).text().to_string().into();
            if this.commit_message_programmatic_change {
                this.commit_message_programmatic_change = false;
                this.commit_message_last_text = next;
                return;
            }

            if this.commit_message_last_text != next {
                this.commit_message_last_text = next;
                this.commit_message_user_edited = true;
            }
        });

        let mut pane = Self {
            store,
            state,
            theme,
            _ui_model_subscription: subscription,
            _commit_message_input_subscription: commit_message_subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            unstaged_scroll: UniformListScrollHandle::default(),
            staged_scroll: UniformListScrollHandle::default(),
            commit_files_scroll: UniformListScrollHandle::default(),
            commit_scroll: ScrollHandle::new(),
            commit_message_input,
            commit_details_message_input,
            commit_message_user_edited: false,
            commit_message_last_text: SharedString::default(),
            commit_message_programmatic_change: false,
            status_multi_selection: HashMap::default(),
            status_multi_selection_last_status: HashMap::default(),
            commit_details_delay: None,
            commit_details_delay_seq: 0,
            path_display_cache: std::cell::RefCell::new(HashMap::default()),
        };
        pane.set_theme(theme, cx);
        pane
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.commit_message_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.commit_details_message_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next;
        cx.notify();
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn cached_path_display(&self, path: &std::path::PathBuf) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    fn apply_state_snapshot(&mut self, next: Arc<AppState>, cx: &mut gpui::Context<Self>) {
        let prev_active_repo_id = self.state.active_repo;
        let prev_selected_commit = prev_active_repo_id.and_then(|repo_id| {
            self.state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.selected_commit.clone())
        });

        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
        let next_selected_commit = next_repo.and_then(|r| r.selected_commit.clone());

        self.state = next;

        let repos = &self.state.repos;
        let last_status = &mut self.status_multi_selection_last_status;
        self.status_multi_selection.retain(|repo_id, selection| {
            let Some(repo) = repos.iter().find(|r| r.id == *repo_id) else {
                last_status.remove(repo_id);
                return false;
            };

            if selection.unstaged.is_empty() && selection.staged.is_empty() {
                last_status.remove(repo_id);
                return false;
            }

            let Loadable::Ready(status) = &repo.status else {
                return true;
            };

            let status_changed = match last_status.get(repo_id) {
                Some(prev) => !Arc::ptr_eq(prev, status),
                None => true,
            };
            if status_changed {
                last_status.insert(*repo_id, Arc::clone(status));
                reconcile_status_multi_selection(selection, status);
            }

            if selection.unstaged.is_empty() && selection.staged.is_empty() {
                last_status.remove(repo_id);
                return false;
            }

            true
        });

        if prev_active_repo_id != next_repo_id {
            self.unstaged_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.staged_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.commit_scroll.set_offset(point(px(0.0), px(0.0)));
            self.commit_files_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.commit_message_user_edited = false;
            self.commit_message_programmatic_change = false;
            self.commit_message_last_text =
                self.commit_message_input.read(cx).text().to_string().into();
        } else if prev_selected_commit != next_selected_commit {
            self.commit_scroll.set_offset(point(px(0.0), px(0.0)));
            self.commit_files_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }

        self.update_commit_details_delay(cx);
    }

    fn update_commit_details_delay(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, selected_id, ready_for_selected, is_error)) = (|| {
            let repo = self.active_repo()?;
            let selected_id = repo.selected_commit.clone()?;
            let ready_for_selected = matches!(
                &repo.commit_details,
                Loadable::Ready(details) if details.id == selected_id
            );
            let is_error = matches!(&repo.commit_details, Loadable::Error(_));
            Some((repo.id, selected_id, ready_for_selected, is_error))
        })() else {
            self.commit_details_delay = None;
            return;
        };

        if ready_for_selected || is_error {
            self.commit_details_delay = None;
            return;
        }

        let same_selection = self
            .commit_details_delay
            .as_ref()
            .is_some_and(|s| s.repo_id == repo_id && s.commit_id == selected_id);
        if same_selection {
            return;
        }

        self.commit_details_delay_seq = self.commit_details_delay_seq.wrapping_add(1);
        let seq = self.commit_details_delay_seq;
        self.commit_details_delay = Some(CommitDetailsDelayState {
            repo_id,
            commit_id: selected_id.clone(),
            show_loading: false,
        });

        let selected_id = selected_id.clone();
        cx.spawn(
            async move |view: WeakEntity<DetailsPaneView>, cx: &mut gpui::AsyncApp| {
                Timer::after(Duration::from_millis(100)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.commit_details_delay_seq != seq {
                        return;
                    }
                    let Some(repo) = this.active_repo() else {
                        return;
                    };
                    let Some(current_selected) = repo.selected_commit.clone() else {
                        return;
                    };
                    if repo.id != repo_id {
                        return;
                    }

                    let ready_for_selected = matches!(
                        &repo.commit_details,
                        Loadable::Ready(details) if details.id == current_selected
                    );
                    if ready_for_selected || matches!(&repo.commit_details, Loadable::Error(_)) {
                        return;
                    }

                    if let Some(state) = this.commit_details_delay.as_mut()
                        && state.repo_id == repo_id
                        && state.commit_id == selected_id
                        && !state.show_loading
                    {
                        state.show_loading = true;
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }

    pub(in super::super) fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
        false
    }

    pub(in super::super) fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
        false
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    pub(in super::super) fn focus_diff_panel(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            let handle = root.main_pane.read(cx).diff_panel_focus_handle.clone();
            window.focus(&handle);
        });
    }
}

impl Render for DetailsPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.commit_details_view(cx)
    }
}
