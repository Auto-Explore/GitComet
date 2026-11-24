use super::super::path_display;
use super::super::*;
use crate::view::gitcomet_view_requests::{RootRequest, RootRequests as _};
use crate::view::pending_commit::PendingCommit;
use gitcomet_core::history_find::HistoryFindQuery;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

mod file_lists;
mod review;
mod review_search;

/// A commit message being edited in the commit details. Kept only while that
/// commit stays the one shown.
pub(in super::super) struct CommitRewordDraft {
    pub(in super::super) repo_id: RepoId,
    pub(in super::super) commit_id: CommitId,
    /// The commit's message when editing began.
    pub(in super::super) original: String,
    /// Whether the input holds a message "Update commit" would accept, so the
    /// pane repaints only when that flips rather than on every keystroke.
    pub(in super::super) submittable: bool,
    /// The message a first submit asked to confirm, for a published commit.
    pub(in super::super) confirming: Option<String>,
    /// Set from dispatch until the reword and any retry of it are over, so
    /// it is submitted once.
    pub(in super::super) submitting: Option<CommitRewordSubmission>,
    /// A paused update can be continued after its authentication prompt closes.
    pub(in super::super) retained_request: Option<gitcomet_state::model::RewordRequestId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct CommitRewordSubmission {
    pub(in super::super) request_id: gitcomet_state::model::RewordRequestId,
}

/// The cached [`WorktreeFileListInputs`] and the scan they were derived from:
/// repo, worktree-dirty revision, and the worktree's own path.
/// One status section's display order, keyed by the projection it was built for.
type StatusSectionOrderSlot = Option<(u64, Arc<[usize]>)>;

type WorktreeFileListInputsCacheEntry = (
    (RepoId, u64, std::path::PathBuf),
    Arc<WorktreeFileListInputs>,
);

/// A linked worktree's changed files, derived once per scan: the rows render from
/// `files`, and a click hands `entries` to the inline-diff machinery so the diff
/// view can step between them.
pub(in super::super) struct WorktreeFileListInputs {
    pub(in super::super) files: Vec<gitcomet_core::domain::CommitFileChange>,
    pub(in super::super) entries: Arc<[gitcomet_state::model::InlineSubmoduleDiffEntry]>,
}

pub(in crate::view) struct ComparisonOrderCache {
    pub key: u64,
    pub ids: Arc<Vec<CommitId>>,
    pub _selection: Arc<Vec<CommitId>>,
    pub _index: Option<gitcomet_core::history_index::HistoryIndexHandle>,
}

pub(in crate::view) struct ComparisonCardCache {
    pub key: u64,
    pub cards: std::rc::Rc<[crate::view::rows::CommitCard]>,
    pub _blocks: Vec<Arc<gitcomet_core::history_index::HistoryRange>>,
}

pub(in super::super) struct DetailsPaneView {
    pub(in crate::view) file_controllers: std::cell::RefCell<
        FxHashMap<
            (RepoId, crate::view::rows::FileListId),
            crate::view::changed_file_list::BuiltinFileList,
        >,
    >,
    pub(in super::super) store: Arc<AppStore>,
    pub(in super::super) state: Arc<AppState>,
    pub(in super::super) active_repo_index: ActiveRepoIndex,
    pub(in super::super) change_tracking_view: ChangeTrackingView,
    pub(in super::super) ui_scale_percent: u32,
    pub(in crate::view) appearance_metrics: crate::appearance::Appearance,
    pub(in super::super) date_time_format: crate::view::date_time::DateTimeFormat,
    pub(in super::super) timezone: crate::view::date_time::Timezone,
    pub(in super::super) show_timezone: bool,
    _ui_model_subscription: gpui::Subscription,
    _commit_reword_input_subscription: gpui::Subscription,
    _history_find_subscription: gpui::Subscription,
    /// The history find bar's query, highlighted in the commit details. It
    /// lives on the history view, so this copy is what repaints the pane.
    pub(in super::super) history_find_query: Option<HistoryFindQuery>,
    root_view: WeakEntity<GitCometView>,
    diff_renderer: WeakEntity<DiffRenderer>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
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
    pub(in super::super) commit_files_section_bounds_ref:
        std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>>,
    pub(in super::super) worktree_filter_bounds_ref:
        std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>>,
    pub(in super::super) range_filter_bounds_ref:
        std::rc::Rc<std::cell::RefCell<Option<Bounds<Pixels>>>>,
    pub(in super::super) status_section_resize: Option<StatusSectionResizeState>,
    status_section_focus_handles: [FocusHandle; 4],
    review_file_focus_handles: [FocusHandle; 3],
    /// Taken by any press in the pane that no child focuses, so Cmd/Ctrl-F
    /// opens this pane's file filter.
    details_focus: FocusHandle,
    review_cursor: Option<(
        RepoId,
        crate::view::rows::FileListId,
        review::ReviewRow,
        usize,
    )>,
    pub(in crate::view) review_search_input: Entity<components::TextInput>,
    /// The file filter bar's own layout: its width tier and overflow menu.
    pub(in crate::view) review_search_bar: std::rc::Rc<components::QuickSearchState>,
    review_search_list: Option<(RepoId, crate::view::rows::FileListId)>,
    _review_search_subscription: gpui::Subscription,
    pub(in crate::view) review_details_dismissed: bool,

    pub(in super::super) untracked_scroll: UniformListScrollHandle,
    pub(in super::super) unstaged_scroll: UniformListScrollHandle,
    pub(in super::super) staged_scroll: UniformListScrollHandle,
    pub(in super::super) commit_files_scroll: UniformListScrollHandle,
    pub(in super::super) commit_multi_scroll: UniformListScrollHandle,
    pub(in super::super) range_files_scroll: UniformListScrollHandle,
    pub(in super::super) worktree_files_scroll: UniformListScrollHandle,
    pub(in super::super) commit_message_scroll: ScrollHandle,
    pub(in super::super) commit_scroll: ScrollHandle,

    pub(in super::super) commit_message_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_message_input: Entity<components::TextInput>,
    /// The editable message while [`Self::commit_reword`] is set.
    pub(in super::super) commit_reword_input: Entity<components::TextInput>,
    pub(in super::super) commit_reword_scroll: ScrollHandle,
    pub(in super::super) commit_reword: Option<CommitRewordDraft>,
    pub(in super::super) commit_details_message_link_menu: Entity<components::CommitLinkMenu>,
    pub(in super::super) commit_message_drafts: FxHashMap<RepoId, SharedString>,
    pub(in super::super) commit_amend_enabled: bool,
    /// `--no-verify` for the commit and the push after it.
    pub(in super::super) commit_skip_hooks_enabled: bool,
    pub(in super::super) pending_commits: FxHashMap<RepoId, PendingCommit>,
    pending_amend_prefill: Option<RepoId>,
    /// Suggestion (repo and rev) already offered to the commit box, so git's
    /// prepared message is applied once and never re-applied.
    applied_commit_suggestions: rustc_hash::FxHashMap<RepoId, (u64, Option<String>)>,
    pub(in super::super) extension_commit_items: Vec<AnyView>,

    pub(in super::super) status_multi_selection: FxHashMap<RepoId, StatusMultiSelection>,
    pub(in super::super) status_multi_selection_last_status: FxHashMap<RepoId, (u64, u64)>,
    /// Row selection of the commit and comparison file lists.
    pub(in super::super) file_list_selection: FxHashMap<
        (RepoId, crate::view::rows::FileListId),
        crate::view::rows::FileListMultiSelection,
    >,

    pub(in super::super) commit_details_delay: Option<CommitDetailsDelayState>,
    pub(in super::super) commit_details_delay_seq: u64,

    path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,
    /// Comparison cards memoized on the revs that feed them; they walked
    /// the whole log page and cloned every selected commit 2-3 times per frame.
    pub(in super::super) range_comparison_commits_cache:
        std::cell::RefCell<Option<ComparisonCardCache>>,
    pub(in super::super) comparison_order: Option<ComparisonOrderCache>,
    pub(in super::super) comparison_order_pending: Option<u64>,
    pub(in super::super) commit_file_sort: crate::view::rows::CommitFileSort,
    /// What a list sorts by until it is sorted by hand.
    pub(in super::super) default_file_list_sort: crate::view::rows::CommitFileSort,
    pub(in super::super) commit_file_filter: crate::view::rows::CommitFileFilter,
    /// Global default; a list may override it until its context changes.
    pub(in super::super) file_list_layout: crate::view::FileListLayout,
    pub(in super::super) file_list_layout_override:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::FileListLayout>,
    /// One slot per status section: three of them render every frame and a
    /// single slot would have them evicting each other's sort.
    status_section_order: std::cell::RefCell<[StatusSectionOrderSlot; 4]>,
    pub(in crate::view) status_footer_position: std::cell::RefCell<
        crate::view::status_position_cache::StatusPositionCache<crate::view::rows::FileListPlan>,
    >,
    pub(in super::super) status_file_sort:
        FxHashMap<StatusSection, crate::view::rows::CommitFileSort>,
    /// Sort and filter for the lists without dedicated fields. Repo-keyed like
    /// the maps beside them, so a filter cannot hide files in another repo.
    list_sort:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::rows::CommitFileSort>,
    list_filter:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::rows::CommitFileFilter>,
    /// Keyed by the worktree as well as the scan revision: `rows_for` returns
    /// cached rows on a key match alone, and `worktree_dirty_rev` bumps per
    /// repo-wide scan, so without the path a different worktree would be served
    /// the previous one's rows.
    /// The per-file inputs the worktree file list is built from, derived once per
    /// scan rather than per frame. Same key as `worktree_file_rows`, and for the
    /// same reason.
    worktree_file_inputs: std::cell::RefCell<Option<WorktreeFileListInputsCacheEntry>>,
    pub(in super::super) untracked_path_alignment_group: components::PathTruncationAlignmentGroup,
    pub(in super::super) unstaged_path_alignment_group: components::PathTruncationAlignmentGroup,
    pub(in super::super) staged_path_alignment_group: components::PathTruncationAlignmentGroup,
    pub(in super::super) commit_files_path_alignment_group:
        components::PathTruncationAlignmentGroup,
    pub(in super::super) range_files_path_alignment_group: components::PathTruncationAlignmentGroup,
    pub(in super::super) worktree_files_path_alignment_group:
        components::PathTruncationAlignmentGroup,
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) render_count: usize,
}

pub(in super::super) struct DetailsPaneInit {
    pub(in super::super) root_view: WeakEntity<GitCometView>,
    pub(in super::super) diff_renderer: WeakEntity<DiffRenderer>,
    pub(in super::super) history_view: Entity<HistoryView>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
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
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
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
                cx.stop_propagation();
            }
        });

        let pane = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if phase != gpui::DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }

            let finished = pane.update(cx, |this, cx| this.finish_status_section_resize(cx));
            if finished {
                cx.stop_propagation();
            }
        });
    }
}

impl DetailsPaneView {
    #[cfg(test)]
    fn notify_fingerprint(state: &AppState) -> u64 {
        Self::fingerprint_with_repo(state, ActiveRepoIndex::default().get(state))
    }

    fn fingerprint_with_repo(state: &AppState, repo: Option<&RepoState>) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);
        if let Some(repo) = repo {
            repo.worktree_status_cache_rev().hash(&mut hasher);
            repo.repository_mode.hash(&mut hasher);
            repo.staged_status_cache_rev().hash(&mut hasher);
            // Counts arrive after the list is drawn; without these the pane
            // never repaints for them.
            repo.staged_line_stats_rev.hash(&mut hasher);
            repo.unstaged_line_stats_rev.hash(&mut hasher);
            // Large-file chips also arrive after the list.
            repo.large_files_rev.hash(&mut hasher);
            repo.lfs_locks_rev.hash(&mut hasher);
            repo.shared_preferences
                .as_ref()
                .map(|snapshot| snapshot.revision)
                .hash(&mut hasher);
            repo.ops_rev.hash(&mut hasher);
            repo.reword.revision.hash(&mut hasher);
            let history = &repo.history_state;
            history.selected_commit_rev.hash(&mut hasher);
            // Loaded history rows show up only as a comparison's cards.
            if Self::shows_comparison(repo) {
                repo.history_state.log_rev.hash(&mut hasher);
                history.indexed.rev.hash(&mut hasher);
            }
            // An older commit's place in that history, and HEAD's, decide
            // whether its message can be edited and whether it is published.
            if history.selected_commit.is_some() {
                repo.history_state.log_rev.hash(&mut hasher);
                history.indexed.rev.hash(&mut hasher);
                repo.detached_head_commit.hash(&mut hasher);
                repo.upstream_divergence_rev.hash(&mut hasher);
            }
            history.commit_details_rev.hash(&mut hasher);
            repo.stashes_rev.hash(&mut hasher);
            // Only the shown commit's signature, not each verified batch.
            if let Loadable::Ready(details) = &history.commit_details {
                history.commit_signatures.get(&details.id).hash(&mut hasher);
            }
            history.worktree_selection_rev.hash(&mut hasher);
            history.range_files_rev.hash(&mut hasher);
            repo.worktree_dirty_rev.hash(&mut hasher);
            repo.merge_message_rev.hash(&mut hasher);
            repo.suggested_commit_message_rev.hash(&mut hasher);
            repo.recent_commit_messages_rev.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.diff_state.diff_target_rev.hash(&mut hasher);
            // Linked worktree rows use the inline selection, which does not
            // bump diff_target_rev. Repaint when it changes, without invalidating
            // the file list for updates to the loaded diff content.
            repo.diff_state
                .inline_submodule_diff
                .as_ref()
                .map(|inline| (&inline.submodule_repo_path, inline.selected_ix))
                .hash(&mut hasher);
        }

        hasher.finish()
    }

    fn observe_model(&mut self, model: Entity<AppUiModel>, cx: &mut gpui::Context<Self>) {
        let next = Arc::clone(&model.read(cx).state);
        let next_fingerprint =
            Self::fingerprint_with_repo(&next, self.active_repo_index.get(&next));
        if self.advance_pending_commits(&next, cx) {
            cx.notify();
        }
        if next_fingerprint == self.notify_fingerprint {
            self.state = next;
            return;
        }

        self.notify_fingerprint = next_fingerprint;
        self.apply_state_snapshot(next, cx);
        cx.notify();
    }

    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        init: DetailsPaneInit,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let DetailsPaneInit {
            root_view,
            diff_renderer,
            history_view,
            tooltip_host,
        } = init;
        let preferences = ui_model.read(cx).preferences.clone();
        let change_tracking_view = preferences.change_tracking.view;
        let file_list_layout = preferences.file_lists.layout;
        let default_file_list_sort = preferences.file_lists.sort;
        let change_tracking_height = preferences.change_tracking.height;
        let untracked_height = preferences.change_tracking.untracked_height;
        let ui_scale_percent = preferences.appearance.ui_scale_percent;
        let date_time_format = preferences.appearance.date_time_format;
        let timezone = preferences.appearance.timezone;
        let show_timezone = preferences.appearance.show_timezone;
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint =
            Self::fingerprint_with_repo(&state, ui_model.read(cx).active_repo_index.get(&state));
        let subscription = cx.observe(&ui_model, Self::observe_model);

        let commit_message_scroll = ScrollHandle::new();
        let commit_scroll = ScrollHandle::new();
        let commit_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "What changed and why".into(),
                    multiline: true,
                    soft_wrap: true,
                    // The commit box draws the card around the input.
                    chromeless: true,
                    min_lines: 4,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_scrollable(
                commit_message_scroll.clone(),
                crate::view::panels::COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX,
                cx,
            );
            input
        });

        let commit_details_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions::selectable_multiline(),
                window,
                cx,
            );
            // The message scrolls inside a capped container. Without its
            // handle the input treats all of its height as visible and shapes
            // every line of a long message when a commit is selected.
            input.set_vertical_scroll_handle(Some(commit_scroll.clone()));
            input
        });
        let commit_reword_scroll = ScrollHandle::new();
        let commit_reword_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Enter commit message".into(),
                    multiline: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(commit_reword_scroll.clone()));
            input
        });
        let commit_reword_subscription = cx.observe(&commit_reword_input, |this, input, cx| {
            let Some(draft) = this.commit_reword.as_mut() else {
                return;
            };
            let text = input.read(cx).text();
            let submittable = crate::view::panels::commit_reword_submittable(&draft.original, text);
            // Editing the message withdraws a pending confirmation.
            let confirm_withdrawn = draft
                .confirming
                .as_deref()
                .is_some_and(|confirmed| confirmed != text);
            if confirm_withdrawn {
                draft.confirming = None;
            }
            if draft.submittable != submittable || confirm_withdrawn {
                draft.submittable = submittable;
                cx.notify();
            }
        });
        let commit_details_message_link_menu = cx.new(|_cx| {
            components::CommitLinkMenu::new(
                commit_details_message_input.clone(),
                RepoId(0),
                Arc::<[components::MessageLink]>::from([]),
                "commit_details_message_link_menu",
                root_view.clone(),
            )
        });

        let history_find_query = history_view.read(cx).history_find_query().cloned();
        let history_find_subscription = cx.observe(&history_view, |this, history, cx| {
            let query = history.read(cx).history_find_query();
            if this.history_find_query.as_ref() != query {
                this.history_find_query = query.cloned();
                cx.notify();
            }
        });
        let review_search_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Filter files".into(),
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        let review_search_subscription =
            cx.observe_in(&review_search_input, window, |this, input, window, cx| {
                let (escape, enter, down) = input.update(cx, |input, _| {
                    (
                        input.take_escape_pressed(),
                        input.take_enter_pressed(),
                        input.take_arrow_down_pressed(),
                    )
                });
                let Some((repo, list)) = this.review_search_list else {
                    return;
                };
                if escape {
                    this.review_search_escape(list, window, cx);
                    return;
                }
                if enter {
                    this.review_search_step(list, true, window, cx);
                    return;
                }
                if down {
                    // Keep the filter active while reviewing its results.
                    this.focus_review_files(Some(list), window, cx);
                    return;
                }
                let query: SharedString = input.read(cx).text().trim().to_owned().into();
                let controller = this.file_controller(repo, list);
                if controller.borrow().query == query {
                    return;
                }
                controller.borrow_mut().query = query;
                this.review_cursor = None;
                this.notify_commit_file_projection_dependents(cx);
                cx.notify();
            });
        let mut pane = Self {
            store,
            state,
            active_repo_index: ui_model.read(cx).active_repo_index.clone(),
            change_tracking_view,
            ui_scale_percent,
            appearance_metrics: crate::appearance::current(cx),
            date_time_format,
            timezone,
            show_timezone,
            _ui_model_subscription: subscription,
            _commit_reword_input_subscription: commit_reword_subscription,
            _history_find_subscription: history_find_subscription,
            history_find_query,
            root_view,
            diff_renderer,
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
            commit_files_section_bounds_ref: std::rc::Rc::new(std::cell::RefCell::new(None)),
            worktree_filter_bounds_ref: Default::default(),
            range_filter_bounds_ref: Default::default(),
            status_section_resize: None,
            status_section_focus_handles: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            review_file_focus_handles: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            details_focus: cx.focus_handle(),
            review_cursor: None,
            review_search_input,
            review_search_bar: components::QuickSearchState::new(),
            review_search_list: None,
            _review_search_subscription: review_search_subscription,
            review_details_dismissed: false,
            untracked_scroll: UniformListScrollHandle::default(),
            unstaged_scroll: UniformListScrollHandle::default(),
            staged_scroll: UniformListScrollHandle::default(),
            commit_files_scroll: UniformListScrollHandle::default(),
            commit_multi_scroll: UniformListScrollHandle::default(),
            range_files_scroll: UniformListScrollHandle::default(),
            worktree_files_scroll: UniformListScrollHandle::default(),
            commit_message_scroll,
            commit_scroll,
            commit_message_input,
            commit_details_message_input,
            commit_reword_input,
            commit_reword_scroll,
            commit_reword: None,
            commit_details_message_link_menu,
            commit_message_drafts: FxHashMap::default(),
            commit_amend_enabled: false,
            commit_skip_hooks_enabled: false,
            pending_commits: FxHashMap::default(),
            pending_amend_prefill: None,
            applied_commit_suggestions: rustc_hash::FxHashMap::default(),
            extension_commit_items: Vec::new(),
            status_multi_selection: FxHashMap::default(),
            status_multi_selection_last_status: FxHashMap::default(),
            file_list_selection: FxHashMap::default(),
            commit_details_delay: None,
            commit_details_delay_seq: 0,
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            range_comparison_commits_cache: std::cell::RefCell::new(None),
            comparison_order: None,
            comparison_order_pending: None,
            commit_file_sort: default_file_list_sort,
            default_file_list_sort,
            file_list_layout,
            file_list_layout_override: FxHashMap::default(),
            file_controllers: Default::default(),
            status_section_order: std::cell::RefCell::new(Default::default()),
            status_footer_position: Default::default(),
            status_file_sort: FxHashMap::default(),
            list_sort: FxHashMap::default(),
            list_filter: FxHashMap::default(),
            commit_file_filter: crate::view::rows::CommitFileFilter::default(),
            worktree_file_inputs: std::cell::RefCell::new(None),
            untracked_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            unstaged_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            staged_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            commit_files_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            range_files_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            worktree_files_path_alignment_group: components::PathTruncationAlignmentGroup::default(
            ),
            #[cfg(any(test, feature = "benchmarks"))]
            render_count: 0,
        };
        pane.sync_scaled_section_heights_from_design();
        pane
    }

    pub(in super::super) fn current_status_sections_bounds(&self) -> Option<Bounds<Pixels>> {
        *self.status_sections_bounds_ref.borrow()
    }

    pub(in super::super) fn current_change_tracking_stack_bounds(&self) -> Option<Bounds<Pixels>> {
        *self.change_tracking_stack_bounds_ref.borrow()
    }

    pub(in crate::view) fn ui_scale(&self) -> ui_scale::UiScale {
        ui_scale::UiScale::from_percent(self.ui_scale_percent)
            .with_appearance(self.appearance_metrics)
    }

    /// The "Stage all" buttons: drop the row selection, then stage — but confirm
    /// first if any of what is about to be staged still has conflict markers in
    /// the worktree, since staging is what tells git the conflict is resolved.
    /// An empty `paths` means everything, matching `Msg::StagePaths`.
    ///
    /// Shared so the combined and split change-tracking views cannot answer this
    /// question differently; they differ only in which section they stage.
    pub(in crate::view) fn stage_all_with_conflict_confirmation(
        &mut self,
        repo_id: RepoId,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let anchor = crate::view::conflict_markers::centered_dialog_anchor(window);
        self.stage_status_paths(repo_id, paths, true, anchor, window, cx);
    }

    pub(in crate::view) fn stage_status_paths(
        &mut self,
        repo_id: RepoId,
        paths: Vec<std::path::PathBuf>,
        clear_selection: bool,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let selection = clear_selection.then(|| self.status_multi_selection.get(&repo_id).cloned());
        crate::view::conflict_markers::check(
            self,
            repo_id,
            paths,
            crate::view::conflict_markers::StageIntent::Status { clear_selection },
            window,
            cx,
            move |this, paths, check, window, cx| {
                let clear_selection = selection.is_some_and(|before| {
                    before.as_ref() == this.status_multi_selection.get(&repo_id)
                });
                if let crate::view::conflict_markers::MarkerCheck::Marked(unresolved) = check {
                    this.open_popover_at(
                        PopoverKind::StageConflictMarkersConfirm {
                            repo_id,
                            paths,
                            unresolved,
                            clear_selection,
                        },
                        anchor,
                        window,
                        cx,
                    );
                } else if let crate::view::conflict_markers::MarkerCheck::Clear(guard) = check {
                    if clear_selection {
                        this.clear_status_multi_selection(repo_id);
                    }
                    crate::view::status_actions::stage_checked(
                        &this.store,
                        repo_id,
                        gitcomet_state::msg::StageSelection::Paths(paths.into()),
                        guard,
                    );
                }
                cx.notify();
            },
        );
    }

    pub(in super::super) fn set_date_settings(
        &mut self,
        format: crate::view::date_time::DateTimeFormat,
        timezone: crate::view::date_time::Timezone,
        show_timezone: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == format
            && self.timezone == timezone
            && self.show_timezone == show_timezone
        {
            return;
        }
        self.date_time_format = format;
        self.timezone = timezone;
        self.show_timezone = show_timezone;
        cx.notify();
    }

    /// Commit date for the details pane: the committer timestamp rendered per
    /// the user's date preferences, falling back to the backend-provided
    /// string when no timestamp is available.
    pub(in super::super) fn commit_details_date_display(
        &self,
        details: &gitcomet_core::domain::CommitDetails,
    ) -> String {
        if details.committed_at_unix == 0 {
            return details.committed_at.clone();
        }
        let mut buf = String::with_capacity(24);
        crate::view::date_time::format_datetime_into(
            &mut buf,
            crate::view::date_time::system_time_from_unix(details.committed_at_unix),
            self.date_time_format,
            self.timezone,
            self.show_timezone,
        );
        buf
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

    /// Update the fallback for lists without an explicit repository sort.
    pub(in super::super) fn set_default_file_list_sort(
        &mut self,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        self.default_file_list_sort = sort;
        self.commit_file_sort = sort;
        self.status_file_sort.clear();
        self.list_sort.clear();
        self.commit_files_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    /// A deliberate change to the global default wins over every list that was
    /// flipped by hand, so the setting is never a no-op somewhere off screen.
    pub(in super::super) fn set_file_list_layout(
        &mut self,
        layout: crate::view::FileListLayout,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_list_layout == layout && self.file_list_layout_override.is_empty() {
            return;
        }
        self.file_list_layout = layout;
        self.file_list_layout_override.clear();
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
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

    pub(in super::super) fn toggle_commit_amend(&mut self, cx: &mut gpui::Context<Self>) {
        self.commit_amend_enabled = !self.commit_amend_enabled;
        if self.commit_amend_enabled {
            self.prefill_commit_message_for_amend(cx);
        } else {
            self.pending_amend_prefill = None;
        }
        cx.notify();
    }

    pub(in super::super) fn toggle_commit_skip_hooks(&mut self, cx: &mut gpui::Context<Self>) {
        self.commit_skip_hooks_enabled = !self.commit_skip_hooks_enabled;
        cx.notify();
    }

    pub(in super::super) fn copy_commit_sha(
        &self,
        commit_id: CommitId,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.request(RootRequest::CopyCommitSha(commit_id), None, cx);
        });
    }

    pub(in super::super) fn reveal_commit_in_history(
        &self,
        repo_id: RepoId,
        commit_id: CommitId,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.reveal_commit_in_history(repo_id, commit_id, cx)
            });
        });
    }

    fn commit_message_is_empty(&self, cx: &gpui::Context<Self>) -> bool {
        self.commit_message_input.read(cx).text().trim().is_empty()
    }

    fn previous_commit_message(&self, repo_id: RepoId) -> Option<String> {
        let repo = self.state.repos.iter().find(|repo| repo.id == repo_id)?;
        match &repo.recent_commit_messages {
            Loadable::Ready(messages) => messages.first().map(|message| message.message.clone()),
            _ => None,
        }
    }

    fn set_commit_message_programmatically(
        &mut self,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        self.commit_message_input
            .update(cx, |input, cx| input.set_text(message, cx));
        self.commit_message_scroll
            .set_offset(point(px(0.0), px(0.0)));
    }

    /// An extension's message for the repository `repo_id` held at `lifetime`:
    /// in the box while that repository is on screen, otherwise as the draft
    /// it shows when selected. Focus stays where it is. A repository that has
    /// closed since the request gets nothing.
    pub(in super::super) fn set_extension_commit_message(
        &mut self,
        repo_id: RepoId,
        lifetime: u64,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self
            .state
            .repos
            .iter()
            .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
        {
            return;
        }
        if self.active_repo_id() == Some(repo_id) {
            self.set_commit_message_programmatically(message, cx);
            cx.notify();
            return;
        }
        if message.is_empty() {
            self.commit_message_drafts.remove(&repo_id);
        } else {
            self.commit_message_drafts.insert(repo_id, message.into());
        }
    }

    pub(in super::super) fn set_extension_commit_items(
        &mut self,
        items: Vec<AnyView>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.extension_commit_items = items;
        cx.notify();
    }

    fn prefill_commit_message_for_amend(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        if !self.commit_message_is_empty(cx) {
            self.pending_amend_prefill = None;
            return;
        }

        match self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| &repo.recent_commit_messages)
        {
            Some(Loadable::Ready(_)) => {
                self.pending_amend_prefill = None;
                if let Some(message) = self.previous_commit_message(repo_id) {
                    self.set_commit_message_programmatically(message, cx);
                }
            }
            _ => {
                self.pending_amend_prefill = Some(repo_id);
            }
        }
    }

    pub(in super::super) fn set_commit_message_from_history(
        &mut self,
        message: String,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_commit_message_programmatically(message, cx);
        let focus = self
            .commit_message_input
            .read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Follows dispatched commits to their result. Runs on every snapshot,
    /// before the store drops the result entries. Returns whether it changed
    /// something.
    fn advance_pending_commits(&mut self, next: &AppState, cx: &mut gpui::Context<Self>) -> bool {
        let mut pending_commits = std::mem::take(&mut self.pending_commits);
        let before = pending_commits.len();
        let mut changed = false;
        pending_commits.retain(|repo_id, pending| {
            // Without git the store drops the commit and never logs a result.
            if !next.git_runtime.is_available()
                || !next.repos.iter().any(|repo| repo.id == *repo_id)
            {
                return false;
            }
            let result = pending.advance(next);
            if result == Some(true) {
                if self.state.active_repo == Some(*repo_id) {
                    let unchanged = self.commit_message_input.read(cx).text().trim()
                        == pending.message.as_ref();
                    if unchanged {
                        self.set_commit_message_programmatically(String::new(), cx);
                        changed = true;
                    }
                    if pending.amend && self.commit_amend_enabled {
                        self.commit_amend_enabled = false;
                        changed = true;
                    }
                } else if self
                    .commit_message_drafts
                    .get(repo_id)
                    .is_some_and(|draft| draft.trim() == pending.message.as_ref())
                {
                    self.commit_message_drafts.remove(repo_id);
                }
            }
            !(result.is_some() && pending.settled())
        });
        changed |= pending_commits.len() != before;
        self.pending_commits = pending_commits;
        changed
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
        self.active_repo_index.get(&self.state)
    }

    pub(in super::super) fn cached_path_display(&self, path: &std::path::Path) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    fn apply_state_snapshot(&mut self, next: Arc<AppState>, cx: &mut gpui::Context<Self>) {
        self.file_controllers
            .borrow_mut()
            .retain(|(id, _), _| next.repos.iter().any(|repo| repo.id == *id));
        let prev_active_repo_id = self.state.active_repo;
        if prev_active_repo_id != next.active_repo || next.active_repo.is_none() {
            *self.status_footer_position.borrow_mut() = Default::default();
        }
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

        let prev_multi_commits = prev_active_repo_id.and_then(|repo_id| {
            self.state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .map(|r| r.history_state.multi_selection.commits.clone())
        });

        let prev_worktree_selection = prev_active_repo_id.and_then(|repo_id| {
            self.state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.history_state.worktree_selection.clone())
        });
        let prev_range_selection = prev_active_repo_id.and_then(|repo_id| {
            self.state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.history_state.range_selection.clone())
        });

        let previous_selection_rev = self
            .active_repo()
            .map(|repo| repo.history_state.selected_commit_rev);
        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
        let review_target = |repo: &RepoState| {
            repo.diff_state
                .inline_submodule_diff
                .as_ref()
                .map(|inline| &inline.target)
                .or(repo.diff_state.diff_target.as_ref())
                .cloned()
        };
        if self.active_repo().and_then(review_target) != next_repo.and_then(review_target) {
            self.review_cursor = None;
        }
        if previous_selection_rev != next_repo.map(|repo| repo.history_state.selected_commit_rev) {
            // Selecting the same commit again also reopens its details.
            self.review_details_dismissed = false;
        }
        let next_worktree_selection =
            next_repo.and_then(|r| r.history_state.worktree_selection.clone());
        let next_range_selection = next_repo.and_then(|r| r.history_state.range_selection.clone());
        let next_selected_commit = next_repo.and_then(|r| r.history_state.selected_commit.clone());
        let next_multi_commits = next_repo.map(|r| r.history_state.multi_selection.commits.clone());
        let next_merge_message = next_repo.and_then(|r| match &r.merge_commit_message {
            Loadable::Ready(Some(message)) => Some(message.clone()),
            _ => None,
        });

        self.state = next;
        self.advance_commit_reword_submission(cx);
        self.commit_message_drafts
            .retain(|repo_id, _| self.state.repos.iter().any(|repo| repo.id == *repo_id));

        // Nothing renders a worktree's file list once its row is deselected, and
        // the derived inputs are one entry per changed file -- worth releasing
        // rather than holding until some other worktree replaces them.
        if self.selected_worktree_summary().is_none() {
            self.worktree_file_inputs.borrow_mut().take();
        }

        let repos = &self.state.repos;
        let last_status = &mut self.status_multi_selection_last_status;
        self.status_multi_selection.retain(|repo_id, selection| {
            let Some(repo) = repos.iter().find(|r| r.id == *repo_id) else {
                last_status.remove(repo_id);
                return false;
            };

            if selection.is_empty() && selection.explicit_section.is_none() {
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

            if selection.is_empty() && selection.explicit_section.is_none() {
                last_status.remove(repo_id);
                return false;
            }

            true
        });

        let switched_repo = prev_active_repo_id != next_repo_id;
        let switched_commit = prev_selected_commit != next_selected_commit;
        if switched_repo
            || switched_commit
            || prev_worktree_selection != next_worktree_selection
            || prev_range_selection != next_range_selection
        {
            self.review_details_dismissed = false;
            self.review_cursor = None;
            self.reset_review_search(cx);
        }
        if switched_repo || switched_commit {
            self.commit_file_filter = crate::view::rows::CommitFileFilter::All;
            // A per-list layout flip lasts as long as the thing it was made for;
            // a different commit re-reads the global default.
            self.file_list_layout_override
                .retain(|(_, list), _| *list != crate::view::rows::FileListId::CommitFiles);
            self.file_controllers
                .borrow_mut()
                .retain(|(_, list), _| *list != crate::view::rows::FileListId::CommitFiles);
            self.file_list_selection
                .retain(|(_, list), _| *list != crate::view::rows::FileListId::CommitFiles);
        }
        if switched_repo || prev_worktree_selection != next_worktree_selection {
            self.forget_file_list_view_state(crate::view::rows::FileListId::WorktreeFiles);
        }
        if switched_repo || prev_range_selection != next_range_selection {
            self.forget_file_list_view_state(crate::view::rows::FileListId::RangeFiles);
        }
        if switched_repo {
            self.file_list_layout_override
                .retain(|(_, list), _| !matches!(list, crate::view::rows::FileListId::Status(_)));
            self.file_controllers
                .borrow_mut()
                .retain(|(_, list), _| !matches!(list, crate::view::rows::FileListId::Status(_)));
        }
        let mut restored_commit_message: Option<SharedString> = None;
        if switched_repo {
            self.commit_amend_enabled = false;
            self.pending_amend_prefill = None;
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
            self.commit_scroll.set_offset(point(px(0.0), px(0.0)));
            self.commit_files_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            let restore = next_repo_id
                .and_then(|repo_id| self.commit_message_drafts.get(&repo_id).cloned())
                .unwrap_or_default();
            restored_commit_message = Some(restore.clone());
            self.set_commit_message_programmatically(restore.to_string(), cx);
        } else if switched_commit {
            self.commit_scroll.set_offset(point(px(0.0), px(0.0)));
            self.commit_files_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }

        if switched_repo || prev_multi_commits != next_multi_commits {
            self.commit_multi_scroll
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
            self.set_commit_message_programmatically(message, cx);
        }

        self.apply_pending_amend_prefill(cx);
        self.apply_suggested_commit_message(cx);

        self.update_commit_details_delay(cx);
    }

    /// Offers Git's message once per suggestion and only when the box is empty.
    /// When an automatic commit consumes it, clear only the unchanged autofill.
    fn apply_suggested_commit_message(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let rev = repo.suggested_commit_message_rev;
        if self
            .applied_commit_suggestions
            .get(&repo_id)
            .is_some_and(|(seen, _)| *seen == rev)
        {
            return;
        }
        let message = repo.suggested_commit_message.clone();
        let previous = self.applied_commit_suggestions.insert(repo_id, (rev, None));
        match message {
            Some(message) if self.commit_message_is_empty(cx) => {
                self.set_commit_message_programmatically(message.clone(), cx);
                self.applied_commit_suggestions
                    .insert(repo_id, (rev, Some(message)));
            }
            Some(message) => {
                if previous.as_ref().and_then(|(_, text)| text.as_ref()) == Some(&message) {
                    self.applied_commit_suggestions
                        .insert(repo_id, (rev, Some(message)));
                }
            }
            None => {
                if let Some((_, Some(previous))) = previous
                    && self.commit_message_input.read(cx).text() == previous
                {
                    self.set_commit_message_programmatically(String::new(), cx);
                }
            }
        }
    }

    fn apply_pending_amend_prefill(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.pending_amend_prefill else {
            return;
        };
        if !self.commit_amend_enabled || self.active_repo_id() != Some(repo_id) {
            self.pending_amend_prefill = None;
            return;
        }

        let ready = matches!(
            self.state
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .map(|repo| &repo.recent_commit_messages),
            Some(Loadable::Ready(_))
        );
        if !ready {
            return;
        }

        self.pending_amend_prefill = None;
        if !self.commit_message_is_empty(cx) {
            return;
        }
        if let Some(message) = self.previous_commit_message(repo_id) {
            self.set_commit_message_programmatically(message, cx);
        }
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
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
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

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ =
                    root_view.request(RootRequest::OpenPopoverAt(kind, anchor), Some(window), cx);
            });
        });
    }

    pub(in super::super) fn open_popover_for_bounds(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.request(
                    RootRequest::OpenPopoverForBounds(kind, anchor_bounds),
                    Some(window),
                    cx,
                );
            });
        });
    }

    pub(in super::super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.schedule_ui_settings_persist(cx);
        });
    }

    pub(in crate::view) fn status_section_focus_handle(
        &self,
        section: StatusSection,
    ) -> &FocusHandle {
        &self.status_section_focus_handles[Self::status_section_alignment_key(section) as usize]
    }

    /// The shared action reads and clears our selection, so release the details
    /// pane before calling it. Keep section focus for the next Space or Ctrl+A.
    pub(in crate::view) fn defer_status_stage_and_advance(
        &self,
        repo_id: RepoId,
        section: StatusSection,
        target: DiffTarget,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let details_pane = cx.weak_entity();
        let diff_renderer = self.diff_renderer.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let still_selected = details_pane
                    .read_with(cx, |pane, _cx| {
                        pane.status_section_focus_handle(section).is_focused(window)
                            && pane
                                .status_section_stage_and_advance_target(repo_id, section)
                                .as_ref()
                                == Some(&target)
                    })
                    .unwrap_or(false);
                if !still_selected {
                    return;
                }
                let _ = diff_renderer.update(cx, |pane, cx| {
                    if pane.active_repo().is_some_and(|repo| {
                        repo.id == repo_id && repo.diff_state.diff_target.as_ref() == Some(&target)
                    }) && pane.toggle_stage_shown_file(window, cx)
                    {
                        cx.notify();
                    }
                });
            });
        });
    }

    pub(in crate::view) fn focus_status_section(
        &self,
        section: StatusSection,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        window.focus(self.status_section_focus_handle(section), cx);
    }

    pub(in super::super) fn status_section_container(
        &self,
        section: StatusSection,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        self.review_file_container(crate::view::rows::FileListId::Status(section), cx)
            .key_context("ChangedFiles StatusSection")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, window, cx| {
                    this.focus_status_section(section, window, cx);
                }),
            )
    }

    pub(in super::super) fn focus_diff_panel(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self
            .root_view
            .request(RootRequest::FocusDiffPanel, Some(window), cx);
    }

    pub(in super::super) fn focus_history_panel(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self
            .root_view
            .request(RootRequest::FocusHistoryPanel, Some(window), cx);
    }
}

impl DetailsPaneView {
    /// Whether keyboard focus is anywhere in this pane.
    pub(in crate::view) fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.details_focus.contains_focused(window, cx)
    }
}

impl Render for DetailsPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        #[cfg(any(test, feature = "benchmarks"))]
        {
            self.render_count += 1;
        }
        div()
            .size_full()
            .track_focus(&self.details_focus)
            // Capture runs before children, which may stop the press (a
            // scrollbar does); a child that focuses itself still wins.
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                if !this.details_focus.contains_focused(window, cx) {
                    window.focus(&this.details_focus, cx);
                }
            }))
            .child(self.commit_details_view(window, cx))
            .child(StatusSectionResizeTracker { view: cx.entity() })
    }
}

#[cfg(test)]
mod tests;
