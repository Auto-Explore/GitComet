use super::super::path_display;
use super::super::*;
use crate::kit::text_truncation::path_alignment_visible_signature;
use gitcomet_state::model::{AuthRetryOperation, CommandLogEntry};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingCommitAmend {
    repo_id: RepoId,
    last_command_log_entry: Option<CommandLogEntry>,
}

/// The cached [`WorktreeFileListInputs`] and the scan they were derived from:
/// repo, worktree-dirty revision, and the worktree's own path.
/// One status section's display order, keyed by the projection it was built for.
type StatusSectionOrderSlot = Option<(u64, Arc<[usize]>)>;
type WorktreeFileProjectionCache = crate::view::rows::CommitFileProjectionCache<(
    RepoId,
    u64,
    std::path::PathBuf,
    crate::view::rows::CommitFileSort,
    crate::view::rows::CommitFileFilter,
)>;
type RangeFileProjectionCache = crate::view::rows::CommitFileProjectionCache<(
    RepoId,
    u64,
    crate::view::rows::CommitFileSort,
    crate::view::rows::CommitFileFilter,
)>;

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
    pub(in super::super) store: Arc<AppStore>,
    pub(in super::super) state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) change_tracking_view: ChangeTrackingView,
    pub(in super::super) ui_scale_percent: u32,
    pub(in crate::view) appearance_metrics: crate::appearance::Appearance,
    pub(in super::super) date_time_format: crate::view::date_time::DateTimeFormat,
    pub(in super::super) timezone: crate::view::date_time::Timezone,
    pub(in super::super) show_timezone: bool,
    _ui_model_subscription: gpui::Subscription,
    _commit_message_input_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    main_pane: WeakEntity<MainPaneView>,
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
    pub(in super::super) commit_details_message_link_menu: Entity<components::CommitLinkMenu>,
    pub(in super::super) commit_details_sha_link_menu: Entity<components::CommitLinkMenu>,
    pub(in super::super) commit_details_sha_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_date_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_parent_input: Entity<components::TextInput>,
    pub(in super::super) commit_details_parent_link_menu: Entity<components::CommitLinkMenu>,
    pub(in super::super) commit_message_drafts: FxHashMap<RepoId, SharedString>,
    pub(in super::super) commit_amend_enabled: bool,
    pub(in super::super) commit_push_after_enabled: bool,
    pending_commit_amend: Option<PendingCommitAmend>,
    pending_amend_prefill: Option<RepoId>,
    pub(in super::super) commit_message_user_edited: bool,
    pub(in super::super) commit_message_last_text: SharedString,
    pub(in super::super) commit_message_programmatic_change: bool,

    pub(in super::super) status_multi_selection: FxHashMap<RepoId, StatusMultiSelection>,
    pub(in super::super) status_multi_selection_last_status: FxHashMap<RepoId, (u64, u64)>,

    pub(in super::super) commit_details_delay: Option<CommitDetailsDelayState>,
    pub(in super::super) commit_details_delay_seq: u64,

    path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,
    /// Comparison cards memoized on the revs that feed them; they walked
    /// the whole log page and cloned every selected commit 2-3 times per frame.
    pub(in super::super) range_comparison_commits_cache:
        std::cell::RefCell<Option<ComparisonCardCache>>,
    pub(in super::super) comparison_order: Option<ComparisonOrderCache>,
    pub(in super::super) comparison_order_pending: Option<u64>,
    commit_file_rows:
        std::cell::RefCell<crate::view::rows::CommitFileRowPresentationCache<(RepoId, u64)>>,
    commit_file_projection: std::cell::RefCell<
        crate::view::rows::CommitFileProjectionCache<(
            RepoId,
            u64,
            crate::view::rows::CommitFileSort,
            crate::view::rows::CommitFileFilter,
        )>,
    >,
    pub(in super::super) commit_file_sort: crate::view::rows::CommitFileSort,
    pub(in super::super) commit_file_filter: crate::view::rows::CommitFileFilter,
    /// Global default; a list may override it until its context changes.
    pub(in super::super) file_list_layout: crate::view::FileListLayout,
    pub(in super::super) file_list_layout_override:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::FileListLayout>,
    pub(in super::super) file_list_collapsed:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::rows::CollapsedDirs>,
    commit_file_plan: std::cell::RefCell<crate::view::rows::FileListPlanCache>,
    /// One slot per status section: three of them render every frame and a
    /// single slot would have them evicting each other's sort.
    status_section_order: std::cell::RefCell<[StatusSectionOrderSlot; 4]>,
    status_file_plan: std::cell::RefCell<[crate::view::rows::FileListPlanCache; 4]>,
    pub(in super::super) status_file_sort:
        FxHashMap<StatusSection, crate::view::rows::CommitFileSort>,
    /// Sort and filter for the lists without dedicated fields. Repo-keyed like
    /// the maps beside them, so a filter cannot hide files in another repo.
    list_sort:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::rows::CommitFileSort>,
    list_filter:
        FxHashMap<(RepoId, crate::view::rows::FileListId), crate::view::rows::CommitFileFilter>,
    worktree_file_plan: std::cell::RefCell<crate::view::rows::FileListPlanCache>,
    range_file_plan: std::cell::RefCell<crate::view::rows::FileListPlanCache>,
    worktree_file_projection: std::cell::RefCell<WorktreeFileProjectionCache>,
    range_file_projection: std::cell::RefCell<RangeFileProjectionCache>,
    range_file_rows:
        std::cell::RefCell<crate::view::rows::CommitFileRowPresentationCache<(RepoId, u64)>>,
    /// Keyed by the worktree as well as the scan revision: `rows_for` returns
    /// cached rows on a key match alone, and `worktree_dirty_rev` bumps per
    /// repo-wide scan, so without the path a different worktree would be served
    /// the previous one's rows.
    worktree_file_rows: std::cell::RefCell<
        crate::view::rows::CommitFileRowPresentationCache<(RepoId, u64, std::path::PathBuf)>,
    >,
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
}

pub(in super::super) struct DetailsPaneInit {
    pub(in super::super) theme: AppTheme,
    pub(in super::super) root_view: WeakEntity<GitCometView>,
    pub(in super::super) main_pane: WeakEntity<MainPaneView>,
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
            // Counts arrive after the list is drawn; without these the pane
            // never repaints for them.
            repo.staged_line_stats_rev.hash(&mut hasher);
            repo.unstaged_line_stats_rev.hash(&mut hasher);
            repo.ops_rev.hash(&mut hasher);
            repo.history_state.selected_commit_rev.hash(&mut hasher);
            repo.log_rev.hash(&mut hasher);
            repo.history_state.indexed.rev.hash(&mut hasher);
            repo.history_state.commit_details_rev.hash(&mut hasher);
            repo.history_state.worktree_selection_rev.hash(&mut hasher);
            repo.history_state.range_files_rev.hash(&mut hasher);
            repo.worktree_dirty_rev.hash(&mut hasher);
            repo.merge_message_rev.hash(&mut hasher);
            repo.recent_commit_messages_rev.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.diff_state.diff_target_rev.hash(&mut hasher);
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
            root_view,
            main_pane,
            tooltip_host,
        } = init;
        let preferences = ui_model.read(cx).preferences.clone();
        let change_tracking_view = preferences.change_tracking.view;
        let file_list_layout = preferences.file_lists.layout;
        let change_tracking_height = preferences.change_tracking.height;
        let untracked_height = preferences.change_tracking.untracked_height;
        let ui_scale_percent = preferences.appearance.ui_scale_percent;
        let commit_push_after_enabled = preferences.repository.commit_push_after_enabled;
        let date_time_format = preferences.appearance.date_time_format;
        let timezone = preferences.appearance.timezone;
        let show_timezone = preferences.appearance.show_timezone;
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
                    soft_wrap: true,
                    ..Default::default()
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
                    multiline: true,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            )
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

        let commit_details_sha_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    read_only: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_display_truncation(Some(components::TextTruncationProfile::Middle), cx);
            input
        });

        let commit_details_date_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    read_only: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let commit_details_parent_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    read_only: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_display_truncation(Some(components::TextTruncationProfile::Middle), cx);
            input
        });
        let commit_details_parent_link_menu = cx.new(|_cx| {
            components::CommitLinkMenu::new(
                commit_details_parent_input.clone(),
                RepoId(0),
                Arc::<[components::MessageLink]>::from([]),
                "commit_details_parent_link_menu",
                root_view.clone(),
            )
        });

        let commit_details_sha_link_menu = cx.new(|_cx| {
            components::CommitLinkMenu::new(
                commit_details_sha_input.clone(),
                RepoId(0),
                Arc::<[components::MessageLink]>::from([]),
                "commit_details_sha_link_menu",
                root_view.clone(),
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
            appearance_metrics: crate::appearance::current(cx),
            date_time_format,
            timezone,
            show_timezone,
            _ui_model_subscription: subscription,
            _commit_message_input_subscription: commit_message_subscription,
            root_view,
            main_pane,
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
            status_section_focus_handles: std::array::from_fn(|_| cx.focus_handle()),
            untracked_scroll: UniformListScrollHandle::default(),
            unstaged_scroll: UniformListScrollHandle::default(),
            staged_scroll: UniformListScrollHandle::default(),
            commit_files_scroll: UniformListScrollHandle::default(),
            commit_multi_scroll: UniformListScrollHandle::default(),
            range_files_scroll: UniformListScrollHandle::default(),
            worktree_files_scroll: UniformListScrollHandle::default(),
            commit_message_scroll,
            commit_scroll: ScrollHandle::new(),
            commit_message_input,
            commit_details_message_input,
            commit_details_message_link_menu,
            commit_details_sha_link_menu,
            commit_details_sha_input,
            commit_details_date_input,
            commit_details_parent_input,
            commit_details_parent_link_menu,
            commit_message_drafts: FxHashMap::default(),
            commit_amend_enabled: false,
            commit_push_after_enabled,
            pending_commit_amend: None,
            pending_amend_prefill: None,
            commit_message_user_edited: false,
            commit_message_last_text: SharedString::default(),
            commit_message_programmatic_change: false,
            status_multi_selection: FxHashMap::default(),
            status_multi_selection_last_status: FxHashMap::default(),
            commit_details_delay: None,
            commit_details_delay_seq: 0,
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            range_comparison_commits_cache: std::cell::RefCell::new(None),
            comparison_order: None,
            comparison_order_pending: None,
            commit_file_rows: std::cell::RefCell::new(
                crate::view::rows::CommitFileRowPresentationCache::default(),
            ),
            commit_file_projection: std::cell::RefCell::new(
                crate::view::rows::CommitFileProjectionCache::default(),
            ),
            commit_file_sort: crate::view::rows::CommitFileSort::default(),
            file_list_layout,
            file_list_layout_override: FxHashMap::default(),
            file_list_collapsed: FxHashMap::default(),
            commit_file_plan: std::cell::RefCell::new(Default::default()),
            status_section_order: std::cell::RefCell::new(Default::default()),
            status_file_plan: std::cell::RefCell::new(Default::default()),
            status_file_sort: FxHashMap::default(),
            list_sort: FxHashMap::default(),
            list_filter: FxHashMap::default(),
            worktree_file_plan: std::cell::RefCell::new(Default::default()),
            range_file_plan: std::cell::RefCell::new(Default::default()),
            worktree_file_projection: std::cell::RefCell::new(Default::default()),
            range_file_projection: std::cell::RefCell::new(Default::default()),
            commit_file_filter: crate::view::rows::CommitFileFilter::default(),
            range_file_rows: std::cell::RefCell::new(
                crate::view::rows::CommitFileRowPresentationCache::default(),
            ),
            worktree_file_rows: std::cell::RefCell::new(
                crate::view::rows::CommitFileRowPresentationCache::default(),
            ),
            worktree_file_inputs: std::cell::RefCell::new(None),
            untracked_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            unstaged_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            staged_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            commit_files_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            range_files_path_alignment_group: components::PathTruncationAlignmentGroup::default(),
            worktree_files_path_alignment_group: components::PathTruncationAlignmentGroup::default(
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
        // The row selection is dropped because staging everything makes it
        // meaningless — but only once the staging is actually going ahead, so a
        // cancelled confirmation costs the user nothing.
        if let Some(confirm) = crate::view::conflict_markers::stage_confirm_popover(
            &self.state,
            repo_id,
            paths.clone(),
            true,
        ) {
            let anchor = crate::view::conflict_markers::centered_dialog_anchor(window);
            self.open_popover_at(confirm, anchor, window, cx);
            cx.notify();
            return;
        }
        self.clear_status_multi_selection(repo_id);
        self.store.dispatch(Msg::ClearDiffSelection { repo_id });
        self.store.dispatch(Msg::StagePaths {
            repo_id,
            paths: paths.into(),
        });
        cx.notify();
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

    pub(in super::super) fn set_commit_amend_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_amend_enabled == enabled {
            return;
        }

        self.commit_amend_enabled = enabled;
        if !enabled {
            self.pending_commit_amend = None;
            self.pending_amend_prefill = None;
        } else {
            self.prefill_commit_message_for_amend(cx);
        }
        cx.notify();
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
        self.commit_message_user_edited = false;
        self.commit_message_programmatic_change = true;
        self.commit_message_last_text = message.clone().into();
        self.commit_message_input
            .update(cx, |input, cx| input.set_text(message, cx));
        self.commit_message_scroll
            .set_offset(point(px(0.0), px(0.0)));
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

    pub(in super::super) fn set_commit_push_after_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_push_after_enabled == enabled {
            return;
        }

        self.commit_push_after_enabled = enabled;
        cx.notify();
    }

    pub(in super::super) fn set_commit_message_from_history(
        &mut self,
        message: String,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.commit_message_user_edited = true;
        self.commit_message_programmatic_change = true;
        self.commit_message_last_text = message.clone().into();
        self.commit_message_input
            .update(cx, |input, cx| input.set_text(message, cx));
        self.commit_message_scroll
            .set_offset(point(px(0.0), px(0.0)));
        let focus = self
            .commit_message_input
            .read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus, cx);
        cx.notify();
    }

    fn sync_commit_amend_enabled_to_root(&self, enabled: bool, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_commit_amend_enabled(enabled, cx);
            });
        });
    }

    fn should_preserve_pending_commit_amend_after_failed_log_entry(
        state: &AppState,
        repo_id: RepoId,
    ) -> bool {
        let auth_prompt_retries_amend = state.auth_prompt.as_ref().is_some_and(|prompt| {
            matches!(
                &prompt.operation,
                AuthRetryOperation::Commit {
                    repo_id: retry_repo_id,
                    amend: true,
                    ..
                } if *retry_repo_id == repo_id
            )
        });
        let commit_retry_in_flight = state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.pending
                    .commit_retry
                    .as_ref()
                    .is_some_and(|pending| pending.amend)
            });

        auth_prompt_retries_amend || commit_retry_in_flight
    }

    fn should_clear_pending_commit_amend_after_log_entry(
        state: &AppState,
        repo_id: RepoId,
        entry_ok: bool,
    ) -> bool {
        entry_ok
            || !Self::should_preserve_pending_commit_amend_after_failed_log_entry(state, repo_id)
    }

    pub(in super::super) fn mark_pending_commit_amend(&mut self, repo_id: RepoId) {
        let last_command_log_entry = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.feedback.command_log.last().cloned());
        self.pending_commit_amend = Some(PendingCommitAmend {
            repo_id,
            last_command_log_entry,
        });
    }

    fn pending_commit_amend_completed_entry<'a>(
        pending: &PendingCommitAmend,
        repo: &'a RepoState,
    ) -> Option<&'a CommandLogEntry> {
        let start = pending
            .last_command_log_entry
            .as_ref()
            .and_then(|last_seen| {
                repo.feedback
                    .command_log
                    .iter()
                    .rposition(|entry| entry == last_seen)
                    .map(|index| index + 1)
            })
            .unwrap_or(0);
        repo.feedback.command_log[start..]
            .iter()
            .rfind(|entry| entry.command == "Amend")
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

    pub(in super::super) fn cached_commit_file_projection(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::CommitFileProjection> {
        let sort = self.commit_file_sort;
        let filter = self.commit_file_filter;
        let mut cache = self.commit_file_projection.borrow_mut();
        cache.projection_for(
            &(repo_id, commit_details_rev, sort, filter),
            files,
            sort,
            filter,
        )
    }

    /// Drop a list's transient layout flip and collapse set, so the next time it
    /// is shown it starts from the global default again.
    fn forget_file_list_view_state(&mut self, list: crate::view::rows::FileListId) {
        self.file_list_layout_override
            .retain(|(_, entry), _| *entry != list);
        self.file_list_collapsed
            .retain(|(_, entry), _| *entry != list);
        // The filter too: a stale "Renamed" empties the list under a header
        // still counting changes.
        self.list_filter.retain(|(_, entry), _| *entry != list);
    }

    pub(in super::super) fn file_list_layout_for(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> crate::view::FileListLayout {
        self.file_list_layout_override
            .get(&(repo_id, list))
            .copied()
            .unwrap_or(self.file_list_layout)
    }

    pub(in super::super) fn toggle_file_list_layout(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = self.file_list_layout_for(repo_id, list).toggled();
        self.file_list_layout_override.insert((repo_id, list), next);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    pub(in super::super) fn file_list_collapsed_for(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> std::borrow::Cow<'_, crate::view::rows::CollapsedDirs> {
        self.file_list_collapsed
            .get(&(repo_id, list))
            .map(std::borrow::Cow::Borrowed)
            .unwrap_or_else(|| std::borrow::Cow::Owned(Default::default()))
    }

    pub(in super::super) fn toggle_file_list_dir(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        key: Arc<std::path::Path>,
        chain: Arc<[Arc<std::path::Path>]>,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let entry = self.file_list_collapsed.entry((repo_id, list)).or_default();
        if collapsed {
            entry.expand(&chain);
        } else {
            entry.collapse(key, &chain);
        }
        cx.notify();
    }

    pub(in super::super) fn cached_commit_file_plan(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let list = crate::view::rows::FileListId::CommitFiles;
        let layout = self.file_list_layout_for(repo_id, list);
        let sort = self.commit_file_sort;
        let projection = self.cached_commit_file_projection(repo_id, commit_details_rev, files);
        let key = crate::view::rows::file_list_projection_key(
            repo_id.0,
            commit_details_rev,
            sort,
            self.commit_file_filter,
        );
        let collapsed = self.file_list_collapsed_for(repo_id, list);
        let mut cache = self.commit_file_plan.borrow_mut();
        cache.plan_for(
            key,
            layout,
            &collapsed,
            projection.source_indices.len(),
            || {
                crate::view::rows::FileTree::build(
                    projection.source_indices.iter().filter_map(|source_ix| {
                        files
                            .get(*source_ix)
                            .map(|file| crate::view::rows::FileTreeItem {
                                path: file.path.as_path(),
                                additions: file.additions,
                                deletions: file.deletions,
                            })
                    }),
                    sort,
                )
            },
        )
    }

    pub(in super::super) fn cached_worktree_file_projection(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::CommitFileProjection> {
        let list = crate::view::rows::FileListId::WorktreeFiles;
        let sort = self.file_list_sort_for(list);
        let filter = self.file_list_filter_for(list);
        let mut cache = self.worktree_file_projection.borrow_mut();
        cache.projection_for(
            &(
                repo_id,
                worktree_dirty_rev,
                worktree_path.to_path_buf(),
                sort,
                filter,
            ),
            files,
            sort,
            filter,
        )
    }

    pub(in super::super) fn cached_range_file_projection(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::CommitFileProjection> {
        let list = crate::view::rows::FileListId::RangeFiles;
        let sort = self.file_list_sort_for(list);
        let filter = self.file_list_filter_for(list);
        let mut cache = self.range_file_projection.borrow_mut();
        cache.projection_for(
            &(repo_id, range_files_rev, sort, filter),
            files,
            sort,
            filter,
        )
    }

    fn file_list_plan_from_projection(
        &self,
        cache: &std::cell::RefCell<crate::view::rows::FileListPlanCache>,
        list: crate::view::rows::FileListId,
        repo_id: RepoId,
        rev: u64,
        // Separates lists sharing a `rev` — every worktree in one scan does.
        scope: Option<&std::path::Path>,
        projection: &crate::view::rows::CommitFileProjection,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let layout = self.file_list_layout_for(repo_id, list);
        let sort = self.file_list_sort_for(list);
        let key = crate::view::rows::file_list_projection_key_scoped(
            repo_id.0,
            rev,
            sort,
            self.file_list_filter_for(list),
            scope,
        );
        let collapsed = self.file_list_collapsed_for(repo_id, list);
        let mut cache = cache.borrow_mut();
        cache.plan_for(
            key,
            layout,
            &collapsed,
            projection.source_indices.len(),
            || {
                crate::view::rows::FileTree::build(
                    projection.source_indices.iter().filter_map(|source_ix| {
                        files
                            .get(*source_ix)
                            .map(|file| crate::view::rows::FileTreeItem {
                                path: file.path.as_path(),
                                additions: file.additions,
                                deletions: file.deletions,
                            })
                    }),
                    sort,
                )
            },
        )
    }

    pub(in super::super) fn cached_worktree_file_plan(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let projection =
            self.cached_worktree_file_projection(repo_id, worktree_dirty_rev, worktree_path, files);
        self.file_list_plan_from_projection(
            &self.worktree_file_plan,
            crate::view::rows::FileListId::WorktreeFiles,
            repo_id,
            worktree_dirty_rev,
            Some(worktree_path),
            &projection,
            files,
        )
    }

    pub(in super::super) fn cached_range_file_plan(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let projection = self.cached_range_file_projection(repo_id, range_files_rev, files);
        self.file_list_plan_from_projection(
            &self.range_file_plan,
            crate::view::rows::FileListId::RangeFiles,
            repo_id,
            range_files_rev,
            None,
            &projection,
            files,
        )
    }

    pub(in super::super) fn active_commit_file_source_indices(
        &self,
        repo_id: RepoId,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let Loadable::Ready(details) = &repo.history_state.commit_details else {
            return None;
        };
        let projection = self.cached_commit_file_projection(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let plan = self.cached_commit_file_plan(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        if !plan.is_tree() {
            return Some(projection.source_indices.clone());
        }
        // Tree order, and deliberately every file: a collapsed folder hides
        // rows, it does not narrow what prev/next file steps through.
        Some(
            plan.ordered()
                .iter()
                .filter_map(|ordinal| projection.source_indices.get(ordinal).copied())
                .collect(),
        )
    }

    /// A linked worktree's changed files as source indices in *drawn* order —
    /// the counterpart to [`Self::active_commit_file_source_indices`]. An
    /// inline worktree diff's `selected_ix` is a source index, so prev/next
    /// file has to step through this rather than through source order.
    pub(in super::super) fn active_worktree_file_source_indices(
        &self,
        repo_id: RepoId,
        worktree_path: &std::path::Path,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let worktree_dirty_rev = repo.worktree_dirty_rev;
        // The list on screen has to be the one the diff was opened from, or its
        // indices belong to another checkout's files.
        let summary = self
            .selected_worktree_summary()
            .filter(|summary| summary.path == worktree_path)?;
        let inputs = self.cached_worktree_file_inputs(repo_id, worktree_dirty_rev, summary);
        let projection = self.cached_worktree_file_projection(
            repo_id,
            worktree_dirty_rev,
            &summary.path,
            &inputs.files,
        );
        let plan = self.cached_worktree_file_plan(
            repo_id,
            worktree_dirty_rev,
            &summary.path,
            &inputs.files,
        );
        if !plan.is_tree() {
            return Some(projection.source_indices.clone());
        }
        // Tree order, and deliberately every file: a collapsed folder hides
        // rows, it does not narrow what prev/next file steps through.
        Some(
            plan.ordered()
                .iter()
                .filter_map(|ordinal| projection.source_indices.get(ordinal).copied())
                .collect(),
        )
    }

    pub(in super::super) fn set_commit_file_sort(
        &mut self,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_file_sort == sort {
            return;
        }
        self.commit_file_sort = sort;
        self.commit_files_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    pub(in super::super) fn set_commit_file_filter(
        &mut self,
        filter: crate::view::rows::CommitFileFilter,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_file_filter == filter {
            return;
        }
        self.commit_file_filter = filter;
        self.commit_files_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    /// The cached main pane reads this projection to render commit-diff file
    /// navigation, so local sort/filter changes must invalidate it explicitly.
    fn notify_commit_file_projection_dependents(&self, cx: &mut gpui::Context<Self>) {
        let main_pane = self.main_pane.clone();
        cx.defer(move |cx| {
            let _ = main_pane.update(cx, |_pane, cx| cx.notify());
        });
    }

    pub(in super::super) fn cached_range_file_rows(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[crate::view::rows::CommitFileRowPresentation]> {
        let mut cache = self.range_file_rows.borrow_mut();
        cache.rows_for(&(repo_id, range_files_rev), files)
    }

    /// The scan entry for the worktree row the history selection is on.
    pub(in super::super) fn selected_worktree_summary(
        &self,
    ) -> Option<&gitcomet_core::domain::WorktreeDirtySummary> {
        let repo = self.active_repo()?;
        let path = repo.history_state.worktree_selection.as_ref()?;
        let Loadable::Ready(dirty) = &repo.worktree_dirty else {
            return None;
        };
        dirty.iter().find(|summary| &summary.path == path)
    }

    /// The changed files of a linked worktree, in the shape the row builder and
    /// the inline-diff navigation need them.
    ///
    /// Derived once per scan and shared by every frame afterwards. Built inline it
    /// is O(all changed files) — three vectors and three `PathBuf` clones per file
    /// — on every layout pass of a list that only ever shows a screenful of rows.
    pub(in super::super) fn cached_worktree_file_inputs(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        summary: &gitcomet_core::domain::WorktreeDirtySummary,
    ) -> Arc<WorktreeFileListInputs> {
        let mut cache = self.worktree_file_inputs.borrow_mut();
        // Compared field by field rather than against a freshly built key: the hit
        // path runs every frame, and building the key clones a `PathBuf`.
        if let Some(((cached_repo, cached_rev, cached_path), inputs)) = cache.as_ref()
            && *cached_repo == repo_id
            && *cached_rev == worktree_dirty_rev
            && cached_path == &summary.path
        {
            return Arc::clone(inputs);
        }

        // Every file is an entry so the diff view can step between them with the
        // same navigation submodule diffs get. Built by the shared builder rather
        // than here: the reducer re-resolves an open diff's entries against each
        // new scan, and it has to arrive at the same order these rows are in.
        let entries: Arc<[_]> = gitcomet_state::model::worktree_inline_diff_entries(summary).into();
        let files = entries
            .iter()
            .map(|entry| {
                // The entry names its lane, so read the counts from that one.
                let stats = match &entry.target {
                    gitcomet_core::domain::DiffTarget::WorkingTree { path, area } => summary
                        .line_stats
                        .for_area(*area)
                        .get(path)
                        .copied()
                        .unwrap_or_default(),
                    _ => Default::default(),
                };
                gitcomet_core::domain::CommitFileChange {
                    path: entry.path.clone(),
                    kind: entry.kind,
                    is_submodule: false,
                    additions: stats.additions,
                    deletions: stats.deletions,
                }
            })
            .collect();

        let inputs = Arc::new(WorktreeFileListInputs { files, entries });
        *cache = Some((
            (repo_id, worktree_dirty_rev, summary.path.clone()),
            Arc::clone(&inputs),
        ));
        inputs
    }

    /// Presentation rows for a linked worktree's changed files. Keyed on the
    /// scan revision, so it rebuilds when the worktree is rescanned and is shared
    /// across renders otherwise.
    pub(in super::super) fn cached_worktree_file_rows(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[crate::view::rows::CommitFileRowPresentation]> {
        let mut cache = self.worktree_file_rows.borrow_mut();
        cache.rows_for(
            &(repo_id, worktree_dirty_rev, worktree_path.to_path_buf()),
            files,
        )
    }

    /// Path-truncation signature for a linked worktree's file rows.
    ///
    /// The scan revision bumps per repo-wide rescan, not per worktree, so the
    /// worktree's own path has to be in here: two worktrees with the same file
    /// count and the same visible range are otherwise indistinguishable, and the
    /// second one would inherit the first one's measured alignment.
    pub(in super::super) fn worktree_files_visible_signature(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo_id,
            worktree_dirty_rev,
            worktree_path,
            total_rows,
            range.start,
            range.end,
        ))
    }

    pub(in super::super) fn range_files_visible_signature(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo_id,
            range_files_rev,
            total_rows,
            range.start,
            range.end,
        ))
    }

    pub(in super::super) fn status_path_alignment_group(
        &self,
        section: StatusSection,
    ) -> &components::PathTruncationAlignmentGroup {
        match section {
            StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
                &self.unstaged_path_alignment_group
            }
            StatusSection::Untracked => &self.untracked_path_alignment_group,
            StatusSection::Staged => &self.staged_path_alignment_group,
        }
    }

    pub(in super::super) fn status_visible_signature(
        &self,
        repo: &RepoState,
        section: StatusSection,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo.id,
            Self::status_section_alignment_key(section),
            status_section_content_rev(repo, section),
            total_rows,
            range.start,
            range.end,
        ))
    }

    pub(in super::super) fn commit_files_visible_signature(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo_id,
            commit_details_rev,
            self.commit_file_sort,
            self.commit_file_filter,
            total_rows,
            range.start,
            range.end,
        ))
    }

    /// One entry point for the controls, whichever list they sit above.
    pub(in super::super) fn file_list_sort_for(
        &self,
        list: crate::view::rows::FileListId,
    ) -> crate::view::rows::CommitFileSort {
        match list {
            crate::view::rows::FileListId::Status(section) => self.status_file_sort_for(section),
            crate::view::rows::FileListId::CommitFiles => self.commit_file_sort,
            other => self
                .active_repo_id()
                .and_then(|repo_id| self.list_sort.get(&(repo_id, other)))
                .copied()
                .unwrap_or_default(),
        }
    }

    pub(in super::super) fn set_file_list_sort(
        &mut self,
        list: crate::view::rows::FileListId,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        match list {
            crate::view::rows::FileListId::Status(section) => {
                self.set_status_file_sort(section, sort, cx)
            }
            crate::view::rows::FileListId::CommitFiles => self.set_commit_file_sort(sort, cx),
            other => {
                let Some(repo_id) = self.active_repo_id() else {
                    return;
                };
                if self.file_list_sort_for(other) == sort {
                    return;
                }
                self.list_sort.insert((repo_id, other), sort);
                self.notify_commit_file_projection_dependents(cx);
                cx.notify();
            }
        }
    }

    pub(in super::super) fn file_list_filter_for(
        &self,
        list: crate::view::rows::FileListId,
    ) -> crate::view::rows::CommitFileFilter {
        match list {
            crate::view::rows::FileListId::CommitFiles => self.commit_file_filter,
            other => self
                .active_repo_id()
                .and_then(|repo_id| self.list_filter.get(&(repo_id, other)))
                .copied()
                .unwrap_or_default(),
        }
    }

    pub(in super::super) fn set_file_list_filter(
        &mut self,
        list: crate::view::rows::FileListId,
        filter: crate::view::rows::CommitFileFilter,
        cx: &mut gpui::Context<Self>,
    ) {
        if list == crate::view::rows::FileListId::CommitFiles {
            self.set_commit_file_filter(filter, cx);
            return;
        }
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        if self.file_list_filter_for(list) == filter {
            return;
        }
        self.list_filter.insert((repo_id, list), filter);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    pub(in super::super) fn status_file_sort_for(
        &self,
        section: StatusSection,
    ) -> crate::view::rows::CommitFileSort {
        self.status_file_sort
            .get(&section)
            .copied()
            .unwrap_or_default()
    }

    pub(in super::super) fn set_status_file_sort(
        &mut self,
        section: StatusSection,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.status_file_sort_for(section) == sort {
            return;
        }
        self.status_file_sort.insert(section, sort);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    /// A status section's entries in the backing slice's index space, filtered
    /// and in display order. Everything that renders, selects or navigates the
    /// section must come through here, or it walks a different order than the
    /// rows do.
    pub(in super::super) fn status_section_order(
        &self,
        repo: &RepoState,
        section: StatusSection,
    ) -> Option<Arc<[usize]>> {
        let sort = self.status_file_sort_for(section);
        let key = crate::view::rows::file_list_projection_key(
            repo.id.0,
            status_section_content_rev(repo, section),
            sort,
            crate::view::rows::CommitFileFilter::All,
        );
        let slot = Self::status_section_alignment_key(section) as usize;
        let mut cache = self.status_section_order.borrow_mut();
        if let Some((cached_key, indexes)) = cache[slot].as_ref()
            && *cached_key == key
        {
            return Some(Arc::clone(indexes));
        }
        let base = StatusSectionEntries::source_order_indexes(repo, section)?;
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries()?,
            _ => repo.worktree_status_entries()?,
        };
        let ordered = crate::view::rows::status_section_sorted_indexes(
            entries,
            &base,
            sort,
            status_section_line_stats(repo, section),
        );
        cache[slot] = Some((key, Arc::clone(&ordered)));
        Some(ordered)
    }

    pub(in super::super) fn status_path_display_position(
        &self,
        section: StatusSection,
        path: &std::path::Path,
    ) -> Option<usize> {
        let repo = self.active_repo()?;
        let order = self.active_status_section_order(repo.id, section)?;
        let entries = StatusSectionEntries::from_repo_with_order(repo, section, order)?;
        entries.iter().position(|entry| entry.path == path)
    }

    /// Expand whatever hides the file at drawn-order `position`, and return its
    /// row.
    pub(in super::super) fn reveal_status_row(
        &mut self,
        section: StatusSection,
        position: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        let repo_id = self.active_repo_id()?;
        let list = crate::view::rows::FileListId::Status(section);
        let plan = {
            let repo = self.active_repo()?;
            self.status_file_plan(repo, section)
        };
        if !plan.is_tree() {
            return Some(position);
        }
        let ordinal = crate::view::rows::FileOrdinal(plan.ordered().iter().nth(position)?);
        let chains = plan.reveal(ordinal);
        if !chains.is_empty() {
            let entry = self.file_list_collapsed.entry((repo_id, list)).or_default();
            for chain in chains {
                entry.expand(&chain);
            }
            cx.notify();
            let repo = self.active_repo()?;
            let plan = self.status_file_plan(repo, section);
            return plan.row_ix_for_ordinal(ordinal).map(|row| row.0);
        }
        plan.row_ix_for_ordinal(ordinal).map(|row| row.0)
    }

    /// Every file under `key`, including any hidden by a nested collapse —
    /// letting collapse narrow a stage would silently leave work behind.
    ///
    /// By prefix rather than the row's `subtree` range, which would have to be
    /// captured per row per frame and could go stale between paint and click.
    /// `Path::starts_with` is component-wise, so `src` never captures `src2`.
    /// The section's paths in drawn order. Shift-click spans a run of *rows*,
    /// and a tree's row order is not its projection order — resolving the range
    /// against the projection skips files displayed between the clicks.
    pub(in super::super) fn status_display_order_paths(
        &self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> Vec<std::path::PathBuf> {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return Vec::new();
        };
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries(),
            _ => repo.worktree_status_entries(),
        };
        let (Some(entries), Some(order)) =
            (entries, self.active_status_section_order(repo_id, section))
        else {
            return Vec::new();
        };
        order
            .iter()
            .filter_map(|source_ix| entries.get(*source_ix).map(|entry| entry.path.clone()))
            .collect()
    }

    /// Identity of the display order, for validating a cached shift-click
    /// anchor. Everything that reorders rows belongs here: sort and layout both
    /// do so without moving any status or line-stats rev.
    pub(in super::super) fn status_anchor_order_rev(
        &self,
        repo: &RepoState,
        section: StatusSection,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let list = crate::view::rows::FileListId::Status(section);
        let mut hasher = rustc_hash::FxHasher::default();
        status_section_content_rev(repo, section).hash(&mut hasher);
        self.file_list_sort_for(list).hash(&mut hasher);
        self.file_list_layout_for(repo.id, list)
            .key()
            .hash(&mut hasher);
        self.file_list_collapsed_for(repo.id, list)
            .rev()
            .hash(&mut hasher);
        hasher.finish()
    }

    pub(in super::super) fn status_folder_subtree_paths(
        &self,
        repo_id: RepoId,
        section: StatusSection,
        key: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return Vec::new();
        };
        let Some(entries) = self.status_section_entries(repo, section) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|entry| entry.path.starts_with(key))
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Expand whatever hides the commit file at `position` and return its row.
    /// Mirrors [`Self::reveal_status_row`]: in a tree the position among files
    /// is not the row index, and a collapsed file has no row at all.
    pub(in super::super) fn reveal_commit_file_row(
        &mut self,
        position: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        let repo_id = self.active_repo_id()?;
        let list = crate::view::rows::FileListId::CommitFiles;
        let plan = {
            let repo = self.active_repo()?;
            let Loadable::Ready(details) = &repo.history_state.commit_details else {
                return None;
            };
            self.cached_commit_file_plan(
                repo_id,
                repo.history_state.commit_details_rev,
                &details.files,
            )
        };
        if !plan.is_tree() {
            return Some(position);
        }
        // `position` indexes the tree-ordered list navigation walks.
        let ordinal = crate::view::rows::FileOrdinal(plan.ordered().iter().nth(position)?);
        let chains = plan.reveal(ordinal);
        if !chains.is_empty() {
            let entry = self.file_list_collapsed.entry((repo_id, list)).or_default();
            for chain in chains {
                entry.expand(&chain);
            }
            cx.notify();
            let repo = self.active_repo()?;
            let Loadable::Ready(details) = &repo.history_state.commit_details else {
                return None;
            };
            let plan = self.cached_commit_file_plan(
                repo_id,
                repo.history_state.commit_details_rev,
                &details.files,
            );
            return plan.row_ix_for_ordinal(ordinal).map(|row| row.0);
        }
        plan.row_ix_for_ordinal(ordinal).map(|row| row.0)
    }

    /// The section's backing-slice indexes in *drawn* order, for prev/next-file
    /// navigation. A tree hoists directories above files, so its rows are a
    /// permutation of the projection and stepping the projection would jump
    /// around the list.
    pub(in super::super) fn active_status_section_order(
        &self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let projection = self.status_section_order(repo, section)?;
        let plan = self.status_file_plan(repo, section);
        if !plan.is_tree() {
            return Some(projection);
        }
        Some(
            plan.ordered()
                .iter()
                .filter_map(|ordinal| projection.get(ordinal).copied())
                .collect(),
        )
    }

    pub(in super::super) fn status_section_entries<'a>(
        &self,
        repo: &'a RepoState,
        section: StatusSection,
    ) -> Option<StatusSectionEntries<'a>> {
        let order = self.status_section_order(repo, section)?;
        StatusSectionEntries::from_repo_with_order(repo, section, order)
    }

    pub(in super::super) fn status_file_plan(
        &self,
        repo: &RepoState,
        section: StatusSection,
    ) -> Arc<crate::view::rows::FileListPlan> {
        let list = crate::view::rows::FileListId::Status(section);
        let layout = self.file_list_layout_for(repo.id, list);
        let sort = self.status_file_sort_for(section);
        let order = self.status_section_order(repo, section).unwrap_or_default();
        let key = crate::view::rows::file_list_projection_key(
            repo.id.0,
            status_section_content_rev(repo, section),
            sort,
            crate::view::rows::CommitFileFilter::All,
        );
        let collapsed = self.file_list_collapsed_for(repo.id, list);
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries(),
            _ => repo.worktree_status_entries(),
        };
        let line_stats = status_section_line_stats(repo, section);
        let slot = Self::status_section_alignment_key(section) as usize;
        let mut cache = self.status_file_plan.borrow_mut();
        cache[slot].plan_for(key, layout, &collapsed, order.len(), || {
            crate::view::rows::FileTree::build(
                order.iter().filter_map(|source_ix| {
                    entries
                        .and_then(|entries| entries.get(*source_ix))
                        .map(|entry| {
                            let stats = line_stats
                                .and_then(|stats| stats.get(&entry.path))
                                .copied()
                                .unwrap_or_default();
                            crate::view::rows::FileTreeItem {
                                path: entry.path.as_path(),
                                additions: stats.additions,
                                deletions: stats.deletions,
                            }
                        })
                }),
                sort,
            )
        })
    }

    fn status_section_alignment_key(section: StatusSection) -> u8 {
        match section {
            StatusSection::CombinedUnstaged => 0,
            StatusSection::Untracked => 1,
            StatusSection::Unstaged => 2,
            StatusSection::Staged => 3,
        }
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

        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
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
        if switched_repo || switched_commit {
            self.commit_file_filter = crate::view::rows::CommitFileFilter::All;
            // A per-list layout flip lasts as long as the thing it was made for;
            // a different commit re-reads the global default.
            self.file_list_layout_override
                .retain(|(_, list), _| *list != crate::view::rows::FileListId::CommitFiles);
            self.file_list_collapsed
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
            self.file_list_collapsed
                .retain(|(_, list), _| !matches!(list, crate::view::rows::FileListId::Status(_)));
        }
        let mut restored_commit_message: Option<SharedString> = None;
        if switched_repo {
            let was_amend_enabled = self.commit_amend_enabled;
            self.commit_amend_enabled = false;
            self.pending_commit_amend = None;
            self.pending_amend_prefill = None;
            if was_amend_enabled {
                self.sync_commit_amend_enabled_to_root(false, cx);
            }
        } else if let Some((repo_id, entry_ok)) =
            self.pending_commit_amend.as_ref().and_then(|pending| {
                if Some(pending.repo_id) != next_repo_id {
                    return None;
                }
                let repo = self
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == pending.repo_id)?;
                let entry = Self::pending_commit_amend_completed_entry(pending, repo)?;
                Some((pending.repo_id, entry.ok))
            })
        {
            let clear_pending = Self::should_clear_pending_commit_amend_after_log_entry(
                &self.state,
                repo_id,
                entry_ok,
            );
            if entry_ok {
                self.commit_amend_enabled = false;
                self.sync_commit_amend_enabled_to_root(false, cx);
            }
            if clear_pending {
                self.pending_commit_amend = None;
            }
        }
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
            self.commit_message_user_edited = false;
            self.commit_message_programmatic_change = true;
            self.commit_message_last_text = message.clone().into();
            self.commit_message_input
                .update(cx, |input, cx| input.set_text(message, cx));
            self.commit_message_scroll
                .set_offset(point(px(0.0), px(0.0)));
        }

        self.apply_pending_amend_prefill(cx);

        self.update_commit_details_delay(cx);
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

    pub(in super::super) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
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

    pub(in crate::view) fn status_section_focus_handle(
        &self,
        section: StatusSection,
    ) -> &FocusHandle {
        &self.status_section_focus_handles[Self::status_section_alignment_key(section) as usize]
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
        cx: &gpui::Context<Self>,
    ) -> gpui::Div {
        div()
            .key_context("StatusSection")
            .track_focus(self.status_section_focus_handle(section))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, window, cx| {
                    this.focus_status_section(section, window, cx);
                }),
            )
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if this.handle_status_section_shortcut(section, &event.keystroke, window, cx) {
                        cx.stop_propagation();
                    }
                }),
            )
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
    use gitcomet_state::model::{AuthPromptState, PendingCommitRetry};
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn repo_state(id: RepoId, path: &str) -> RepoState {
        RepoState::new_opening(
            id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from(path),
            },
        )
    }

    fn command_log_entry(command: &str, ok: bool, seconds: u64) -> CommandLogEntry {
        CommandLogEntry {
            time: UNIX_EPOCH + Duration::from_secs(seconds),
            ok,
            command: command.to_string(),
            summary: format!("{command}: test"),
            stdout: "".into(),
            stderr: "".into(),
            announce_success: true,
            hook_operation_id: None,
        }
    }

    #[test]
    fn pending_amend_ignores_previous_amend_log_entry() {
        let repo_id = RepoId(1);
        let old_entry = command_log_entry("Amend", true, 1);
        let pending = PendingCommitAmend {
            repo_id,
            last_command_log_entry: Some(old_entry.clone()),
        };
        let mut repo = repo_state(repo_id, "/tmp/repo");
        repo.feedback.command_log.push(old_entry);

        assert!(DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo).is_none());
    }

    #[test]
    fn pending_amend_observes_later_amend_log_entry() {
        let repo_id = RepoId(1);
        let old_entry = command_log_entry("Amend", true, 1);
        let new_entry = command_log_entry("Amend", true, 2);
        let pending = PendingCommitAmend {
            repo_id,
            last_command_log_entry: Some(old_entry.clone()),
        };
        let mut repo = repo_state(repo_id, "/tmp/repo");
        repo.feedback.command_log.push(old_entry);
        repo.feedback.command_log.push(new_entry);

        assert_eq!(
            DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo)
                .map(|entry| entry.time),
            Some(UNIX_EPOCH + Duration::from_secs(2))
        );
    }

    #[test]
    fn pending_amend_observes_amend_before_later_push_log_entry() {
        let repo_id = RepoId(1);
        let old_entry = command_log_entry("Amend", true, 1);
        let new_entry = command_log_entry("Amend", true, 2);
        let push_entry = command_log_entry("Push after commit", true, 3);
        let pending = PendingCommitAmend {
            repo_id,
            last_command_log_entry: Some(old_entry.clone()),
        };
        let mut repo = repo_state(repo_id, "/tmp/repo");
        repo.feedback.command_log.push(old_entry);
        repo.feedback.command_log.push(new_entry);
        repo.feedback.command_log.push(push_entry);

        assert_eq!(
            DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo)
                .map(|entry| entry.time),
            Some(UNIX_EPOCH + Duration::from_secs(2))
        );
    }

    #[test]
    fn pending_amend_observes_successful_retry_after_failed_amend_log_entry() {
        let repo_id = RepoId(1);
        let marker_entry = command_log_entry("Commit", true, 1);
        let failed_amend = command_log_entry("Amend", false, 2);
        let successful_retry = command_log_entry("Amend", true, 3);
        let pending = PendingCommitAmend {
            repo_id,
            last_command_log_entry: Some(marker_entry.clone()),
        };
        let mut repo = repo_state(repo_id, "/tmp/repo");
        repo.feedback.command_log.push(marker_entry);
        repo.feedback.command_log.push(failed_amend);
        repo.feedback.command_log.push(successful_retry);

        let completed_entry =
            DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo);

        assert_eq!(
            completed_entry.map(|entry| (entry.ok, entry.time)),
            Some((true, UNIX_EPOCH + Duration::from_secs(3)))
        );
    }

    #[test]
    fn pending_amend_marker_survives_auth_prompt_retry() {
        let repo_id = RepoId(1);
        let state = AppState {
            repos: vec![repo_state(repo_id, "/tmp/repo")],
            active_repo: Some(repo_id),
            auth_prompt: Some(AuthPromptState {
                kind: AuthPromptKind::UsernamePassword,
                reason: "auth required".into(),
                operation: AuthRetryOperation::Commit {
                    repo_id,
                    message: "message".into(),
                    amend: true,
                    push_after_commit: false,
                },
            }),
            ..AppState::default()
        };

        assert!(
            DetailsPaneView::should_preserve_pending_commit_amend_after_failed_log_entry(
                &state, repo_id
            )
        );
        assert!(
            !DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(
                &state, repo_id, false
            )
        );
    }

    #[test]
    fn pending_amend_marker_survives_in_flight_auth_retry() {
        let repo_id = RepoId(1);
        let mut repo = repo_state(repo_id, "/tmp/repo");
        repo.pending.commit_retry = Some(PendingCommitRetry {
            message: "message".into(),
            amend: true,
            push_after_commit: false,
        });
        let state = AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..AppState::default()
        };

        assert!(
            DetailsPaneView::should_preserve_pending_commit_amend_after_failed_log_entry(
                &state, repo_id
            )
        );
        assert!(
            !DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(
                &state, repo_id, false
            )
        );
        assert!(
            DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(
                &state, repo_id, true
            )
        );
    }

    #[test]
    fn pending_amend_marker_is_not_preserved_for_non_retry_failure() {
        let repo_id = RepoId(1);
        let state = AppState {
            repos: vec![repo_state(repo_id, "/tmp/repo")],
            active_repo: Some(repo_id),
            ..AppState::default()
        };

        assert!(
            !DetailsPaneView::should_preserve_pending_commit_amend_after_failed_log_entry(
                &state, repo_id
            )
        );
        assert!(
            DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(
                &state, repo_id, false
            )
        );
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
    fn indexed_regression_details_fingerprint_tracks_comparison_metadata() {
        let mut state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo_state(RepoId(1), "/tmp/indexed-comparison")],
            ..Default::default()
        };
        let before = DetailsPaneView::notify_fingerprint(&state);
        state.repos[0].history_state.indexed.rev += 1;
        assert_ne!(before, DetailsPaneView::notify_fingerprint(&state));
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

        state.repos[0].history_state.range_files_rev = 1;
        let after_range_files = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(after_range_files, after_details);

        state.repos[0].merge_message_rev = 1;
        assert_ne!(
            DetailsPaneView::notify_fingerprint(&state),
            after_range_files
        );
    }

    #[test]
    fn notify_fingerprint_tracks_amend_availability_revisions() {
        let mut state = AppState {
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = DetailsPaneView::notify_fingerprint(&state);

        state.repos[0].head_branch_rev = 1;
        let after_head_branch = DetailsPaneView::notify_fingerprint(&state);
        assert_ne!(after_head_branch, initial);

        state.repos[0].branches_rev = 1;
        assert_ne!(
            DetailsPaneView::notify_fingerprint(&state),
            after_head_branch
        );
    }
}
