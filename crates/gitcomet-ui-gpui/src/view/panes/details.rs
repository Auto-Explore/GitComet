use super::super::path_display;
use super::super::*;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

pub(in super::super) struct DetailsPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) change_tracking_view: ChangeTrackingView,
    pub(in super::super) ui_scale_percent: u32,
    _ui_model_subscription: gpui::Subscription,
    _commit_message_input_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: u64,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,
    change_tracking_height_design: Option<f32>,
    untracked_height_design: Option<f32>,
    pub(in super::super) change_tracking_height: Option<Pixels>,
    pub(in super::super) untracked_height: Option<Pixels>,
    pub(in super::super) status_sections_bounds_ref:
        std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>>,
    pub(in super::super) change_tracking_stack_bounds_ref:
        std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>>,
    pub(in super::super) status_section_resize: Option<StatusSectionResizeState>,

    pub(in super::super) untracked_scroll: UniformListScrollHandle,
    pub(in super::super) unstaged_scroll: UniformListScrollHandle,
    pub(in super::super) staged_scroll: UniformListScrollHandle,
    pub(in super::super) commit_files_scroll: UniformListScrollHandle,
    pub(in super::super) commit_message_scroll: ScrollHandle,
    pub(in super::super) commit_scroll: ScrollHandle,

    pub(in super::super) commit_message_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_message_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_sha_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_date_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_parent_input: Entity<components::TextInput>,
    pub(in super::super) commit_message_drafts: HashMap<RepoId, SharedString>,
    pub(in super::super) commit_message_user_edited: bool,
    pub(in super::super) commit_message_last_text: SharedString,
    pub(in super::super) commit_message_programmatic_change: bool,

    pub(in super::super) status_multi_selection: HashMap<RepoId, StatusMultiSelection>,
    pub(in super::super) status_multi_selection_last_status: HashMap<RepoId, (u64, u64)>,

    pub(in super::super) commit_details_delay: Option<CommitDetailsDelayState>,
    pub(in super::super) commit_details_delay_seq: u64,

    path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,
    commit_file_rows:
        std::cell::RefCell<crate::view::rows::CommitFileRowPresentationCache<(RepoId, u64)>>,
}

pub(in super::super) struct DetailsPaneInit {
    pub(in super::super) theme: AppTheme,
    pub(in super::super) change_tracking_view: ChangeTrackingView,
    pub(in super::super) change_tracking_height: Option<u32>,
    pub(in super::super) untracked_height: Option<u32>,
    pub(in super::super) ui_scale_percent: u32,
    pub(in super::super) root_view: WeakEntity<GitCometView>,
    pub(in super::super) tooltip_host: WeakEntity<TooltipHost>,
}

pub(in super::super) struct StatusSectionResizeTracker {
    pub(in super::super) view: Entity<DetailsPaneView>,
}

impl IntoElement for StatusSectionResizeTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for StatusSectionResizeTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(0.0).into();
        style.size.height = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let pane = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Capture {
                return;
            }

            let active = pane.update(cx, |this, cx| {
                if this.status_section_resize.is_some() {
                    this.update_status_section_resize(event.position.y, cx);
                    true
                } else {
                    false
                }
            });
            if active {
                window.refresh();
                cx.stop_propagation();
            }
        });

        let pane = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != gpui::DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }

            let finished = pane.update(cx, |this, cx| this.finish_status_section_resize(cx));
            if finished {
                window.refresh();
                cx.stop_propagation();
            }
        });
    }
}

impl DetailsPaneView {
    fn notify_fingerprint(state: &AppState) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.worktree_status_cache_rev().hash(&mut hasher);
            repo.staged_status_cache_rev().hash(&mut hasher);
            repo.ops_rev.hash(&mut hasher);
            repo.history_state.selected_commit_rev.hash(&mut hasher);
            repo.history_state.commit_details_rev.hash(&mut hasher);
            repo.merge_message_rev.hash(&mut hasher);
        }

        hasher.finish()
    }

    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        init: DetailsPaneInit,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let DetailsPaneInit {
            theme,
            change_tracking_view,
            change_tracking_height,
            untracked_height,
            ui_scale_percent,
            root_view,
            tooltip_host,
        } = init;
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

        let commit_message_scroll = ScrollHandle::new();
        let commit_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Enter commit message".into(),
                    multiline: true,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: true,
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(commit_message_scroll.clone()));
            input
        });

        let commit_details_message_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
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

        let commit_details_sha_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: false,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let commit_details_date_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: false,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let commit_details_parent_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: false,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: false,
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
            change_tracking_view,
            ui_scale_percent,
            _ui_model_subscription: subscription,
            _commit_message_input_subscription: commit_message_subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            change_tracking_height_design: Self::sanitized_restored_change_tracking_height_design(
                change_tracking_view,
                change_tracking_height,
            ),
            untracked_height_design: Self::sanitized_restored_untracked_height_design(
                untracked_height,
            ),
            change_tracking_height: None,
            untracked_height: None,
            status_sections_bounds_ref: std::rc::Rc::new(std::cell::RefCell::new(None)),
            change_tracking_stack_bounds_ref: std::rc::Rc::new(std::cell::RefCell::new(None)),
            status_section_resize: None,
            untracked_scroll: UniformListScrollHandle::default(),
            unstaged_scroll: UniformListScrollHandle::default(),
            staged_scroll: UniformListScrollHandle::default(),
            commit_files_scroll: UniformListScrollHandle::default(),
            commit_message_scroll,
            commit_scroll: ScrollHandle::new(),
            commit_message_input,
            commit_details_message_input,
            commit_details_sha_input,
            commit_details_date_input,
            commit_details_parent_input,
            commit_message_drafts: HashMap::default(),
            commit_message_user_edited: false,
            commit_message_last_text: SharedString::default(),
            commit_message_programmatic_change: false,
            status_multi_selection: HashMap::default(),
            status_multi_selection_last_status: HashMap::default(),
            commit_details_delay: None,
            commit_details_delay_seq: 0,
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            commit_file_rows: std::cell::RefCell::new(
                crate::view::rows::CommitFileRowPresentationCache::default(),
            ),
        };
        pane.sync_scaled_section_heights_from_design();
        pane.set_theme(theme, cx);
        pane
    }

    pub(in super::super) fn current_status_sections_bounds(&self) -> Option<Bounds<Pixels>> {
        *self.status_sections_bounds_ref.borrow()
    }

    pub(in super::super) fn current_change_tracking_stack_bounds(&self) -> Option<Bounds<Pixels>> {
        *self.change_tracking_stack_bounds_ref.borrow()
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.commit_message_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.commit_details_message_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.commit_details_sha_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.commit_details_date_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.commit_details_parent_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    pub(in crate::view) fn ui_scale(&self) -> ui_scale::UiScale {
        ui_scale::UiScale::from_percent(self.ui_scale_percent)
    }

    fn change_tracking_height_design(&self) -> Option<f32> {
        self.change_tracking_height_design.or_else(|| {
            self.ui_scale()
                .design_units_from_optional_pixels(self.change_tracking_height)
        })
    }

    fn untracked_height_design(&self) -> Option<f32> {
        self.untracked_height_design.or_else(|| {
            self.ui_scale()
                .design_units_from_optional_pixels(self.untracked_height)
        })
    }

    fn sync_scaled_section_heights_from_design(&mut self) {
        let change_tracking_height_design = self.change_tracking_height_design();
        let untracked_height_design = self.untracked_height_design();
        let scale = self.ui_scale();
        self.change_tracking_height_design = change_tracking_height_design;
        self.untracked_height_design = untracked_height_design;
        self.change_tracking_height = scale.pixels_from_design_units(change_tracking_height_design);
        self.untracked_height = scale.pixels_from_design_units(untracked_height_design);
    }

    pub(in super::super) fn set_change_tracking_height_from_pixels(
        &mut self,
        height: Option<Pixels>,
    ) {
        self.change_tracking_height = height;
        self.change_tracking_height_design =
            self.ui_scale().design_units_from_optional_pixels(height);
    }

    pub(in super::super) fn set_untracked_height_from_pixels(&mut self, height: Option<Pixels>) {
        self.untracked_height = height;
        self.untracked_height_design = self.ui_scale().design_units_from_optional_pixels(height);
    }

    pub(in super::super) fn set_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        self.status_section_resize = None;
        self.status_multi_selection.clear();
        cx.notify();
    }

    pub(in super::super) fn saved_status_section_heights(&self) -> (Option<u32>, Option<u32>) {
        let scale = self.ui_scale();
        (
            ui_scale::stored_design_units(
                scale
                    .design_units_from_optional_pixels(self.change_tracking_height)
                    .or(self.change_tracking_height_design),
            ),
            ui_scale::stored_design_units(
                scale
                    .design_units_from_optional_pixels(self.untracked_height)
                    .or(self.untracked_height_design),
            ),
        )
    }

    pub(in super::super) fn apply_ui_scale_percent(
        &mut self,
        _previous_percent: u32,
        next_percent: u32,
        change_tracking_view: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.ui_scale_percent == next_percent {
            return;
        }

        let (change_tracking_height, untracked_height) = self.saved_status_section_heights();
        self.ui_scale_percent = next_percent;
        self.status_section_resize = None;
        self.change_tracking_height_design = Self::sanitized_restored_change_tracking_height_design(
            change_tracking_view,
            change_tracking_height,
        );
        self.untracked_height_design =
            Self::sanitized_restored_untracked_height_design(untracked_height);
        self.sync_scaled_section_heights_from_design();
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

    pub(in super::super) fn cached_path_display(&self, path: &std::path::Path) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    pub(in super::super) fn cached_commit_file_rows(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[crate::view::rows::CommitFileRowPresentation]> {
        let mut cache = self.commit_file_rows.borrow_mut();
        cache.rows_for(&(repo_id, commit_details_rev), files)
    }

    fn apply_state_snapshot(&mut self, next: Arc<AppState>, cx: &mut gpui::Context<Self>) {
        let prev_active_repo_id = self.state.active_repo;
        let prev_selected_commit = prev_active_repo_id.and_then(|repo_id| {
            self.state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.history_state.selected_commit.clone())
        });
        let prev_merge_message = prev_active_repo_id.and_then(|repo_id| {
            self.state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| match &r.merge_commit_message {
                    Loadable::Ready(Some(message)) => Some(message.clone()),
                    _ => None,
                })
        });

        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
        let next_selected_commit = next_repo.and_then(|r| r.history_state.selected_commit.clone());
        let next_merge_message = next_repo.and_then(|r| match &r.merge_commit_message {
            Loadable::Ready(Some(message)) => Some(message.clone()),
            _ => None,
        });

        self.state = next;
        self.commit_message_drafts
            .retain(|repo_id, _| self.state.repos.iter().any(|repo| repo.id == *repo_id));

        let repos = &self.state.repos;
        let last_status = &mut self.status_multi_selection_last_status;
        self.status_multi_selection.retain(|repo_id, selection| {
            let Some(repo) = repos.iter().find(|r| r.id == *repo_id) else {
                last_status.remove(repo_id);
                return false;
            };

            if selection.is_empty() {
                last_status.remove(repo_id);
                return false;
            }

            let status_key = (
                repo.worktree_status_cache_rev(),
                repo.staged_status_cache_rev(),
            );
            let status_changed = match last_status.get(repo_id) {
                Some(prev) => *prev != status_key,
                None => true,
            };
            if status_changed {
                last_status.insert(*repo_id, status_key);
                reconcile_status_multi_selection_with_repo(selection, repo);
            }

            if selection.is_empty() {
                last_status.remove(repo_id);
                return false;
            }

            true
        });

        let switched_repo = prev_active_repo_id != next_repo_id;
        let mut restored_commit_message: Option<SharedString> = None;
        if switched_repo {
            if let Some(prev_repo_id) = prev_active_repo_id {
                let current: SharedString =
                    self.commit_message_input.read(cx).text().to_string().into();
                if current.is_empty() {
                    self.commit_message_drafts.remove(&prev_repo_id);
                } else {
                    self.commit_message_drafts.insert(prev_repo_id, current);
                }
            }

            self.unstaged_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.staged_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.commit_message_scroll
                .set_offset(point(px(0.0), px(0.0)));
            self.commit_scroll.set_offset(point(px(0.0), px(0.0)));
            self.commit_files_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            let restore = next_repo_id
                .and_then(|repo_id| self.commit_message_drafts.get(&repo_id).cloned())
                .unwrap_or_default();
            restored_commit_message = Some(restore.clone());
            self.commit_message_user_edited = false;
            self.commit_message_programmatic_change = true;
            self.commit_message_input
                .update(cx, |input, cx| input.set_text(restore.to_string(), cx));
            self.commit_message_last_text = restore;
        } else if prev_selected_commit != next_selected_commit {
            self.commit_scroll.set_offset(point(px(0.0), px(0.0)));
            self.commit_files_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }

        let merge_started = match (prev_active_repo_id, next_repo_id) {
            (Some(prev), Some(next)) if prev == next => {
                prev_merge_message.is_none() && next_merge_message.is_some()
            }
            _ => next_merge_message.is_some(),
        };
        let restored_is_empty = restored_commit_message
            .as_ref()
            .map(|message| message.trim().is_empty())
            .unwrap_or(true);
        let apply_merge_message = if switched_repo {
            restored_is_empty
        } else {
            true
        };
        if merge_started
            && apply_merge_message
            && let Some(message) = next_merge_message
        {
            self.commit_message_user_edited = false;
            self.commit_message_programmatic_change = true;
            self.commit_message_last_text = message.clone().into();
            self.commit_message_input
                .update(cx, |input, cx| input.set_text(message, cx));
            self.commit_message_scroll
                .set_offset(point(px(0.0), px(0.0)));
        }

        self.update_commit_details_delay(cx);
    }

    fn update_commit_details_delay(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, selected_id, ready_for_selected, is_error)) = (|| {
            let repo = self.active_repo()?;
            let selected_id = repo.history_state.selected_commit.clone()?;
            let ready_for_selected = matches!(
                &repo.history_state.commit_details,
                Loadable::Ready(details) if details.id == selected_id
            );
            let is_error = matches!(&repo.history_state.commit_details, Loadable::Error(_));
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
                smol::Timer::after(Duration::from_millis(100)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.commit_details_delay_seq != seq {
                        return;
                    }
                    let Some(repo) = this.active_repo() else {
                        return;
                    };
                    let Some(current_selected) = repo.history_state.selected_commit.clone() else {
                        return;
                    };
                    if repo.id != repo_id {
                        return;
                    }

                    let ready_for_selected = matches!(
                        &repo.history_state.commit_details,
                        Loadable::Ready(details) if details.id == current_selected
                    );
                    if ready_for_selected
                        || matches!(&repo.history_state.commit_details, Loadable::Error(_))
                    {
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

    pub(in super::super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.schedule_ui_settings_persist(cx);
        });
    }

    pub(in super::super) fn focus_diff_panel(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            let handle = root.main_pane.read(cx).diff_panel_focus_handle.clone();
            window.focus(&handle, cx);
        });
    }
}

impl Render for DetailsPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(self.commit_details_view(cx))
            .child(StatusSectionResizeTracker { view: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_state(id: RepoId, path: &str) -> RepoState {
        RepoState::new_opening(
            id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from(path),
            },
        )
    }

    #[test]
    fn notify_fingerprint_ignores_inactive_repo_revisions() {
        let active = repo_state(RepoId(1), "/tmp/active");
        let inactive = repo_state(RepoId(2), "/tmp/inactive");
        let mut state = AppState {
            repos: vec![active, inactive],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = DetailsPaneView::notify_fingerprint(&state);

        state.repos[1].worktree_status_rev = 1;
        state.repos[1].staged_status_rev = 1;
        state.repos[1].ops_rev = 1;
        state.repos[1].history_state.selected_commit_rev = 1;
        state.repos[1].history_state.commit_details_rev = 1;
        state.repos[1].merge_message_rev = 1;

        assert_eq!(DetailsPaneView::notify_fingerprint(&state), initial);
    }

    #[test]
    fn notify_fingerprint_tracks_active_repo_relevant_revisions() {
        let mut state = AppState {
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = DetailsPaneView::notify_fingerprint(&state);

        state.repos[0].worktree_status_rev = 1;
        let after_status = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(after_status, initial);

        state.repos[0].ops_rev = 1;
        let after_ops = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(after_ops, after_status);

        state.repos[0].history_state.selected_commit_rev = 1;
        let after_selected = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(after_selected, after_ops);

        state.repos[0].history_state.commit_details_rev = 1;
        let after_details = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(after_details, after_selected);

        state.repos[0].merge_message_rev = 1;
        assert_ne!(DetailsPaneView::notify_fingerprint(&state), after_details);
    }
}
