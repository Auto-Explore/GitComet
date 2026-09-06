use super::*;
use gitcomet_core::services::InteractiveRebaseAction;

mod add_repo_menu;
mod add_to_gitignore_prompt;
mod app_menu;
mod author_filter;
mod branch_exists_prompt;
mod branch_picker;
mod checkout_remote_branch_prompt;
mod cherry_pick_commit_confirm;
mod clone_repo;
mod commit_prompt;
pub(in super::super) mod context_menu;
mod create_branch_from_ref_prompt;
mod create_tag_prompt;
mod delete_branches_confirm;
mod delete_remote_branch_confirm;
mod discard_changes_confirm;
mod file_history;
mod fingerprint;
mod force_delete_branch_confirm;
mod force_push_confirm;
mod force_remove_worktree_confirm;
mod hook_activity;
mod merge_abort_confirm;
mod merge_commit_confirm;
mod picker_nav;
mod picker_row_menu;
mod pull_reconcile_prompt;
mod push_set_upstream_prompt;
mod rebase_onto_confirm;
mod remote_add_prompt;
mod remote_edit_url_prompt;
mod remote_remove_confirm;
mod rename_branch_prompt;
mod repo_picker;
mod reset_prompt;
mod rows_cache;
mod search_inputs;
mod squash_prompt;
mod stage_conflict_markers_confirm;
mod stash_drop_confirm;
mod stash_picker_prompt;
mod stash_prompt;
mod submodule_add_prompt;
mod submodule_change_pointer_prompt;
mod submodule_picker;
mod submodule_remove_confirm;
mod submodule_trust_confirm;
mod terminal_shutdown_confirm;
mod unsaved_file_edits_confirm;
mod upstream_picker;
mod workspace_picker;
mod worktree_add_prompt;
mod worktree_picker;
mod worktree_remove_confirm;

#[derive(Clone, Debug)]
enum PopoverAnchor {
    Point(Point<Pixels>),
    Bounds(Bounds<Pixels>),
    Centered,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in super::super) struct PopoverWidthSpec {
    preferred: f32,
    min: f32,
    max: f32,
}

impl PopoverWidthSpec {
    pub(in super::super) const fn fixed(width: f32) -> Self {
        Self {
            preferred: width,
            min: width,
            max: width,
        }
    }

    pub(in super::super) const fn range(preferred: f32, min: f32, max: f32) -> Self {
        Self {
            preferred,
            min,
            max,
        }
    }

    pub(in super::super) fn preferred_px(self, ui_scale: ui_scale::UiScale) -> Pixels {
        ui_scale.px(self.preferred)
    }

    pub(in super::super) fn min_px(self, ui_scale: ui_scale::UiScale) -> Pixels {
        ui_scale.px(self.min)
    }

    pub(in super::super) fn max_px(self, ui_scale: ui_scale::UiScale) -> Pixels {
        ui_scale.px(self.max)
    }
}

const DEFAULT_CONTEXT_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(260.0, 180.0, 380.0);
const NARROW_CONTEXT_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(220.0, 160.0, 220.0);
const REBASE_ACTION_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(110.0);
const REBASE_AUTOSQUASH_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(190.0);
const CHANGE_TRACKING_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(220.0, 220.0, 320.0);
const DIFF_ACTION_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(240.0, 200.0, 320.0);
const MERGETOOL_SETTINGS_MENU_WIDTH: PopoverWidthSpec =
    PopoverWidthSpec::range(320.0, 280.0, 420.0);
const DIFF_EDITOR_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(260.0, 200.0, 340.0);
const CONFLICT_INPUT_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(220.0, 180.0, 280.0);
const CONFLICT_CHUNK_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(320.0, 220.0, 360.0);
const CONFLICT_OUTPUT_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(240.0, 200.0, 300.0);
const STASH_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(220.0, 180.0, 360.0);
/// Wider than the sibling column menus: it carries a search box, and author
/// names run long — "Firstname Middlename Lastname" truncates at the menu
/// default.
const HISTORY_AUTHOR_FILTER_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(320.0, 240.0, 420.0);
const REPO_TAB_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(360.0);
const PICKER_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(420.0, 420.0, 820.0);
const LARGE_PICKER_WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(520.0, 520.0, 820.0);
const DIALOG_320_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(320.0);
const DIALOG_360_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(360.0);
const DIALOG_380_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(380.0);
const DIALOG_420_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(420.0);
const DIALOG_440_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(440.0);
const DIALOG_460_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(460.0);
const DIALOG_540_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(540.0);
const DIALOG_640_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(640.0);
const DIALOG_900_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(900.0);
// Leaves enough room for “Open in code editor” and its three-key shortcut
// badge to remain on one line on non-macOS platforms.
const APP_MENU_WIDTH: PopoverWidthSpec = PopoverWidthSpec::fixed(320.0);

/// Cancel/submit focus-handle pair shared by every prompt dialog.
pub(super) struct DialogFocus {
    pub(super) cancel: FocusHandle,
    pub(super) submit: FocusHandle,
}

impl DialogFocus {
    fn new(cx: &mut gpui::Context<PopoverHost>) -> Self {
        Self {
            cancel: cx.focus_handle().tab_index(0).tab_stop(true),
            submit: cx.focus_handle().tab_index(0).tab_stop(true),
        }
    }
}

pub(in super::super) struct PopoverHost {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    theme_mode: ThemeMode,
    date_time_format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    history_relative_dates: bool,
    change_tracking_view: ChangeTrackingView,
    commit_amend_enabled: bool,
    commit_push_after_enabled: bool,
    diff_content_mode: DiffContentMode,
    diff_whitespace_mode: DiffWhitespaceMode,
    diff_reveal_whitespace_chars: bool,
    diff_word_wrap: bool,
    diff_show_line_numbers: bool,
    _ui_model_subscription: gpui::Subscription,
    _repo_picker_search_input_subscription: Option<gpui::Subscription>,
    _branch_picker_search_input_subscription: Option<gpui::Subscription>,
    _upstream_picker_search_input_subscription: Option<gpui::Subscription>,
    _worktree_picker_search_input_subscription: Option<gpui::Subscription>,
    _workspace_picker_search_input_subscription: Option<gpui::Subscription>,
    _submodule_picker_search_input_subscription: Option<gpui::Subscription>,
    _file_history_search_input_subscription: Option<gpui::Subscription>,
    _history_author_filter_search_input_subscription: Option<gpui::Subscription>,
    _squash_message_input_subscription: gpui::Subscription,
    _squash_description_input_subscription: gpui::Subscription,
    _prompt_input_subscriptions: Vec<gpui::Subscription>,
    notify_fingerprint: u64,
    root_view: WeakEntity<GitCometView>,
    /// Mirror of the root view's mode, which is fixed for the window's lifetime.
    /// Held here because menu models are built while the root view's update
    /// borrow is active, so its entity can't be read at that point.
    root_view_mode: GitCometViewMode,
    tooltip_host: WeakEntity<TooltipHost>,
    main_pane: Entity<MainPaneView>,
    details_pane: Entity<DetailsPaneView>,
    reflog_pane: Entity<ReflogPaneView>,
    sidebar_pane: Entity<SidebarPaneView>,
    /// Mirror of the sidebar pane's pinned branches, keyed by repository
    /// workdir. Kept here because context menus are built from click handlers
    /// that already hold the sidebar pane's update borrow, so its entity can't
    /// be read at that point.
    pinned_branches_by_repo:
        std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
    /// Mirror of the sidebar's collapse set, kept here for the same reason as
    /// [`Self::pinned_branches_by_repo`]: the branch group menu is built while
    /// the sidebar pane's update borrow is already held.
    collapsed_items_by_repo:
        std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
    /// Mirror of the sidebar's branch filter, for the same reason.
    branch_filter_query: String,

    popover: Option<PopoverKind>,
    popover_anchor: Option<PopoverAnchor>,
    hook_activity_selected: Option<GitOperationId>,
    hook_activity_history_scroll: ScrollHandle,
    hook_activity_hooks_scroll: ScrollHandle,
    hook_activity_output_scroll: ScrollHandle,
    /// Explicit 1-based mainline selected for the currently open single
    /// merge-commit cherry-pick confirmation. Reset every time that dialog
    /// opens; drafts are intentionally session-local.
    cherry_pick_mainline: Option<usize>,
    context_menu_focus_handle: FocusHandle,
    /// Focus held by the App/Add Repository menu invoker, restored when that
    /// menu is dismissed without replacing it with another prompt.
    menu_invoker_focus: Option<FocusHandle>,
    /// Whether the open popover was invoked from inside the diff panel.
    ///
    /// Some menus — the web link menu above all — can be raised from either the
    /// diff panel or the commit details pane, and only the former should hand
    /// focus back to the diff panel when it closes.
    popover_opened_from_diff_panel: bool,
    prompt_tab_group_focus_handle: FocusHandle,
    prompt_tab_wrap_end_focus_handle: FocusHandle,
    context_menu_selected_ix: Option<usize>,
    repo_picker_selected_index: Option<usize>,
    /// Last trimmed query handled by the repository picker. Kept separately
    /// from the input so text edits can reset keyboard selection without
    /// mistaking cursor, focus, or styling notifications for query changes.
    repo_picker_search_query: String,
    /// Session recent repositories snapshotted when a repository picker opens,
    /// so the list can't shift under the user mid-interaction.
    cached_recent_repos: Vec<std::path::PathBuf>,
    /// Session pins snapshotted alongside `cached_recent_repos`. Held apart from
    /// the recents so a pin outlives the recents cap.
    cached_pinned_repos: Vec<std::path::PathBuf>,
    /// Storage keys of the repository picker sections the user folded away.
    cached_collapsed_picker_sections: std::collections::BTreeSet<String>,
    repo_picker_sort: repo_picker::RepoPickerSort,
    repo_picker_sort_menu_open: bool,
    /// Repository row whose context menu floats over the picker, and the window
    /// position it was invoked at. The picker stays open underneath it.
    picker_row_menu: Option<picker_row_menu::PickerRowMenu>,
    branch_picker_selected_index: Option<usize>,
    upstream_picker_selected_index: Option<usize>,
    worktree_picker_selected_index: Option<usize>,
    workspace_picker_selected_index: Option<usize>,
    /// Path/reference the workspace badge's create row hands to the Add-worktree
    /// dialog. Consumed (and cleared) when that dialog opens, so a later
    /// open from elsewhere still starts blank.
    pending_worktree_add_prefill: Option<(String, String)>,
    submodule_picker_selected_index: Option<usize>,
    file_history_selected_index: Option<usize>,
    history_author_filter_selected_index: Option<usize>,
    /// Author suggestions for the history author filter, keyed by repository and
    /// the log revision they were collected from. Collecting them walks the
    /// whole accumulated log, and the popover re-renders on every mouse move
    /// over it, so the result has to outlive the frame. See
    /// [`author_filter::suggestions`].
    history_author_suggestions: Option<(RepoId, u64, std::sync::Arc<[SharedString]>)>,
    /// Row models for the pickers that build one row per repository, ref or
    /// worktree, rebuilt only when the data behind them changes rather than on
    /// every frame. See [`rows_cache`] — a hover moving between rows re-renders
    /// this whole view.
    branch_picker_rows_cache: rows_cache::RowsCache<branch_picker::BranchPickerNavTarget>,
    upstream_picker_rows_cache: rows_cache::RowsCache<upstream_picker::UpstreamTarget>,
    workspace_picker_rows_cache: rows_cache::RowsCache<workspace_picker::WorkspaceRow>,
    repo_picker_rows_cache: rows_cache::RowsCache<repo_picker::RepoPickerEntry>,
    stash_picker_rows_cache: rows_cache::RowsCache<stash_picker_prompt::StashRow>,
    file_history_rows_cache: rows_cache::RowsCache<CommitId>,
    submodule_picker_rows_cache: rows_cache::RowsCache<std::path::PathBuf>,
    worktree_picker_rows_cache: rows_cache::RowsCache<std::path::PathBuf>,
    branch_ref_rows_cache: rows_cache::RowsCache<String>,

    repo_picker_search_input: Option<Entity<components::TextInput>>,
    branch_picker_search_input: Option<Entity<components::TextInput>>,
    remote_picker_search_input: Option<Entity<components::TextInput>>,
    file_history_search_input: Option<Entity<components::TextInput>>,
    history_author_filter_search_input: Option<Entity<components::TextInput>>,
    worktree_picker_search_input: Option<Entity<components::TextInput>>,
    workspace_picker_search_input: Option<Entity<components::TextInput>>,
    submodule_picker_search_input: Option<Entity<components::TextInput>>,
    picker_prompt_scroll: ScrollHandle,

    clone_repo_url_input: Entity<components::TextInput>,
    clone_repo_parent_dir_input: Entity<components::TextInput>,
    rebase_onto_input: Entity<components::TextInput>,
    create_tag_input: Entity<components::TextInput>,
    create_tag_message_input: Entity<components::TextInput>,
    create_tag_message_scroll: ScrollHandle,
    /// One `.gitignore` line per row. Multiline so a multi-file selection and a
    /// single file share one code path, and so the field reads like the file it
    /// is about to become.
    gitignore_patterns_input: Entity<components::TextInput>,
    gitignore_patterns_scroll: ScrollHandle,
    /// Which scope's patterns the input was last prefilled with. Only a prefill
    /// shortcut — submit reads the input, never this.
    gitignore_scope: gitcomet_core::gitignore::GitignoreScope,
    /// Computed once when the dialog opens, so a status refresh arriving
    /// mid-edit cannot change the offered scopes under the user.
    gitignore_suggestions: Option<gitcomet_core::gitignore::GitignoreSuggestions>,
    /// The paths the dialog is about, for the "Ignore <file>" body text.
    gitignore_paths: Vec<std::path::PathBuf>,
    squash_message_input: Entity<components::TextInput>,
    squash_description_input: Entity<components::TextInput>,
    squash_description_scroll: ScrollHandle,
    /// The `(oldest, head)` range the squash prompt's message inputs were last
    /// prefilled for. Prevents re-prefilling the same range (so a user who
    /// clears the fields keeps them cleared) and, together with the empty-input
    /// check, prevents clobbering text the user typed while the preview loaded.
    squash_prompt_prefilled_range: Option<(
        gitcomet_core::domain::CommitId,
        gitcomet_core::domain::CommitId,
    )>,
    remote_name_input: Entity<components::TextInput>,
    remote_url_input: Entity<components::TextInput>,
    remote_url_edit_input: Entity<components::TextInput>,
    create_branch_input: Entity<components::TextInput>,
    create_branch_checkout_enabled: bool,
    create_branch_source_target: String,
    worktree_ref_source_target: String,
    suppress_worktree_submit_after_ref_enter: bool,
    /// Set while a row menu floating over a picker runs one of its entries. The
    /// menu has already closed itself by then, and the popover underneath is the
    /// picker — which stays up so the next row can be acted on.
    suppress_popover_close_after_action: bool,
    create_branch_from_ref_checkout_focus_handle: FocusHandle,
    create_branch_from_ref_focus: DialogFocus,
    create_tag_annotated: bool,
    create_tag_annotated_focus_handle: FocusHandle,
    checkout_remote_branch_focus: DialogFocus,
    stash_message_input: Entity<components::TextInput>,
    stash_focus: DialogFocus,
    stash_picker_prompt_selected_index: Option<usize>,
    stash_picker_search_input: Option<Entity<components::TextInput>>,
    _stash_picker_search_input_subscription: Option<gpui::Subscription>,
    commit_prompt_message_drafts: FxHashMap<RepoId, SharedString>,
    commit_prompt_message_input: Entity<components::TextInput>,
    commit_prompt_message_scroll: ScrollHandle,
    commit_prompt_focus: DialogFocus,
    clone_repo_browse_focus_handle: FocusHandle,
    squash_cancel_focus_handle: FocusHandle,
    squash_submit_focus_handle: FocusHandle,
    rebase_onto_submit_focus_handle: FocusHandle,
    clone_repo_focus: DialogFocus,
    create_tag_focus: DialogFocus,
    remote_add_focus: DialogFocus,
    remote_edit_focus: DialogFocus,
    push_upstream_focus: DialogFocus,
    push_upstream_remote_focus_handle: FocusHandle,
    push_upstream_remote_menu_open: bool,
    push_upstream_remote_selected_index: Option<usize>,
    worktree_browse_focus_handle: FocusHandle,
    worktree_focus: DialogFocus,
    submodule_advanced_focus_handle: FocusHandle,
    submodule_force_focus_handle: FocusHandle,
    submodule_focus: DialogFocus,
    push_upstream_branch_input: Entity<components::TextInput>,
    worktree_path_input: Entity<components::TextInput>,
    worktree_ref_input: Entity<components::TextInput>,
    submodule_url_input: Entity<components::TextInput>,
    submodule_path_input: Entity<components::TextInput>,
    submodule_ref_input: Entity<components::TextInput>,
    submodule_branch_input: Entity<components::TextInput>,
    submodule_name_input: Entity<components::TextInput>,
    submodule_add_advanced_expanded: bool,
    submodule_force_enabled: bool,
    rebase_reword_input: Entity<components::TextInput>,
    rebase_reword_description_input: Entity<components::TextInput>,
    rebase_reword_description_scroll: ScrollHandle,
}

pub(in crate::view) struct PopoverHostInit {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) root_view: WeakEntity<GitCometView>,
    pub(in crate::view) root_view_mode: GitCometViewMode,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
    pub(in crate::view) main_pane: Entity<MainPaneView>,
    pub(in crate::view) details_pane: Entity<DetailsPaneView>,
    pub(in crate::view) reflog_pane: Entity<ReflogPaneView>,
    pub(in crate::view) sidebar_pane: Entity<SidebarPaneView>,
    pub(in crate::view) pinned_branches_by_repo:
        std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
    pub(in crate::view) collapsed_items_by_repo:
        std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
}

/// Rows the branch badge's checkout picker would show for `query`, for the
/// picker benchmarks. The builder is a pure function of the repository, so the
/// benchmark measures exactly what a frame used to rebuild.
#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_branch_checkout_rows(
    repo: &RepoState,
    query: &str,
    now: std::time::SystemTime,
) -> Vec<components::PickerPromptItem> {
    branch_picker::rows(repo, query, now).items
}

/// Rows the workspace badge's picker would show for `query`, for the picker
/// benchmarks.
#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_workspace_rows(
    repo: &RepoState,
    query: &str,
) -> Vec<components::PickerPromptItem> {
    workspace_picker::rows(repo, query).items
}

pub(in super::super) fn popover_ui_scale(cx: &mut gpui::Context<PopoverHost>) -> ui_scale::UiScale {
    ui_scale::UiScale::current(cx)
}

pub(in super::super) fn popover_ui_scale_percent(cx: &mut gpui::Context<PopoverHost>) -> u32 {
    popover_ui_scale(cx).percent()
}

pub(in super::super) fn popover_scaled_px(
    value: f32,
    ui_scale: impl Into<ui_scale::UiScale>,
) -> Pixels {
    ui_scale.into().px(value)
}

pub(in super::super) fn popover_scaled_px_from_percent(
    value: f32,
    ui_scale_percent: u32,
) -> Pixels {
    popover_scaled_px(value, ui_scale_percent)
}

/// One-line replacement for the per-panel `ui_scale_percent` + closure
/// preamble: returns a copyable `f32 -> Pixels` scaler for the current
/// UI scale.
pub(super) fn popover_scaled_px_fn(
    cx: &mut gpui::Context<PopoverHost>,
) -> impl Fn(f32) -> Pixels + Copy + use<> {
    let ui_scale = popover_ui_scale(cx);
    move |value: f32| ui_scale.px(value)
}

pub(in super::super) fn focusable_toggle_row<V: 'static>(
    id: &'static str,
    debug_selector: &'static str,
    theme: AppTheme,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<V>,
) -> gpui::Stateful<gpui::Div> {
    let focus_handle = focus_handle.clone().tab_index(0).tab_stop(true);
    let hover_bg = theme.hover_overlay();
    let active_bg = theme.active_overlay();
    div()
        .id(id)
        .debug_selector(move || debug_selector.to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .border_1()
        .border_color(gpui::transparent_black())
        .track_focus(&focus_handle)
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(hover_bg))
        .active(move |s| s.bg(active_bg))
        .focus(move |s| {
            s.bg(theme.colors.interaction.focus_background)
                .border_color(theme.colors.interaction.focus_ring)
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, _e: &MouseDownEvent, window, cx| {
                window.focus(&focus_handle, cx);
            }),
        )
}

fn popover_is_context_menu(kind: &PopoverKind) -> bool {
    matches!(
        kind,
        PopoverKind::AppMenu
            | PopoverKind::AddRepoMenu
            | PopoverKind::PullPicker
            | PopoverKind::PushPicker
            | PopoverKind::CommitOptionsMenu { .. }
            | PopoverKind::PreviousCommitMessagesMenu { .. }
            | PopoverKind::RepoTabMenu { .. }
            | PopoverKind::WebLinkMenu { .. }
            | PopoverKind::CommitShaLinkMenu { .. }
            | PopoverKind::DiffActionMenu
            | PopoverKind::InteractiveRebaseActionMenu { .. }
            | PopoverKind::InteractiveRebaseAutosquashMenu
            | PopoverKind::MergetoolSettingsMenu
            | PopoverKind::HistoryBranchFilter { .. }
            | PopoverKind::DiffContentModeSettings
            | PopoverKind::CommitFileSortMenu
            | PopoverKind::ChangeTrackingSettings
            | PopoverKind::UiScalePicker
            | PopoverKind::TerminalMenu { .. }
            | PopoverKind::DiffHunkMenu { .. }
            | PopoverKind::DiffEditorMenu { .. }
            | PopoverKind::ConflictResolverInputRowMenu { .. }
            | PopoverKind::ConflictResolverChunkMenu { .. }
            | PopoverKind::ConflictResolverOutputMenu { .. }
            | PopoverKind::CommitMenu { .. }
            | PopoverKind::ReflogEntryMenu { .. }
            | PopoverKind::TagMenu { .. }
            | PopoverKind::TagRefMenu { .. }
            | PopoverKind::StatusFileMenu { .. }
            | PopoverKind::BranchMenu { .. }
            | PopoverKind::BranchRefsMenu { .. }
            | PopoverKind::BranchSectionMenu { .. }
            | PopoverKind::SubmoduleInnerDiffMenu { .. }
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::Menu { .. }),
                ..
            }
            | PopoverKind::StashMenu { .. }
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(
                    WorktreePopoverKind::SectionMenu | WorktreePopoverKind::Menu { .. },
                ),
                ..
            }
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(
                    SubmodulePopoverKind::SectionMenu | SubmodulePopoverKind::Menu { .. },
                ),
                ..
            }
            | PopoverKind::CommitFileMenu { .. }
            | PopoverKind::FileBrowserFileMenu { .. }
            | PopoverKind::FileBrowserFolderMenu { .. }
            | PopoverKind::BranchGroupMenu { .. }
            | PopoverKind::PinnedSectionMenu { .. }
            | PopoverKind::BrowseHistoryMenu { .. }
    )
}

fn popover_is_confirm_dialog(kind: &PopoverKind) -> bool {
    matches!(
        kind,
        PopoverKind::StashDropConfirm { .. }
            | PopoverKind::ForcePushConfirm { .. }
            | PopoverKind::CherryPickCommitConfirm { .. }
            | PopoverKind::MergeCommitConfirm { .. }
            | PopoverKind::MergeAbortConfirm { .. }
            | PopoverKind::RebaseOntoConfirm { .. }
            | PopoverKind::RebaseReword { .. }
            | PopoverKind::BranchExistsPrompt { .. }
            | PopoverKind::ForceDeleteBranchConfirm { .. }
            | PopoverKind::DeleteBranchesConfirm { .. }
            | PopoverKind::ForceRemoveWorktreeConfirm { .. }
            | PopoverKind::DiscardChangesConfirm { .. }
            | PopoverKind::AddToGitignorePrompt { .. }
            | PopoverKind::StageConflictMarkersConfirm { .. }
            | PopoverKind::ResetPrompt { .. }
            | PopoverKind::PullReconcilePrompt { .. }
            | PopoverKind::TerminalShutdownConfirm(_)
            | PopoverKind::UnsavedFileEditsConfirm(_)
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::RemoveConfirm { .. }),
                ..
            }
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::DeleteBranchConfirm { .. }),
                ..
            }
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::RemoveConfirm { .. }),
                ..
            }
            | PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::RemoveConfirm { .. }),
                ..
            }
    )
}

pub(super) fn hotkey_hint(
    theme: AppTheme,
    debug_selector: &'static str,
    label: impl Into<SharedString>,
) -> gpui::Div {
    div()
        .debug_selector(move || debug_selector.to_string())
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .text_color(theme.colors.foreground.secondary)
        .child(label.into())
}

/// Shared Cancel button for confirm dialogs and prompt popovers: consistent
/// label, outlined style, and "Esc" hint. Attach the dismiss handler with
/// `.on_click(...)` at the call site.
pub(super) fn cancel_button_labeled(
    id: &'static str,
    hint_debug_selector: &'static str,
    label: impl Into<SharedString>,
    theme: AppTheme,
) -> components::Button {
    components::Button::new(id, label)
        .separated_end_slot(hotkey_hint(theme, hint_debug_selector, "Esc"))
        .style(components::ButtonStyle::Outlined)
}

pub(super) fn cancel_button(
    id: &'static str,
    hint_debug_selector: &'static str,
    theme: AppTheme,
) -> components::Button {
    cancel_button_labeled(id, hint_debug_selector, "Cancel", theme)
}

/// Cancel button whose click simply closes the popover.
pub(super) fn dialog_cancel_button(
    id: &'static str,
    hint_debug_selector: &'static str,
    theme: AppTheme,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    cancel_button(id, hint_debug_selector, theme).on_click(theme, cx, |this, _e, _w, cx| {
        this.close_popover(cx);
    })
}

pub(super) fn dialog_divider(theme: AppTheme) -> gpui::Div {
    div().border_t_1().border_color(theme.colors.stroke.default)
}

/// Shared scaffolding for confirm-style dialogs: title, divider, body
/// sections, divider, then a footer with a cancel button on the left and the
/// action button(s) on the right. Width comes from the same `PopoverWidthSpec`
/// constants used by `popover_width_spec`, so the two can't drift apart.
pub(super) struct ConfirmDialog {
    title: SharedString,
    width: PopoverWidthSpec,
    sections: Vec<AnyElement>,
}

impl ConfirmDialog {
    pub(super) fn new(title: impl Into<SharedString>, width: PopoverWidthSpec) -> Self {
        Self {
            title: title.into(),
            width,
            sections: Vec::new(),
        }
    }

    /// Muted body paragraph.
    pub(super) fn text(mut self, theme: AppTheme, text: impl Into<SharedString>) -> Self {
        self.sections.push(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.foreground.secondary)
                .child(text.into())
                .into_any_element(),
        );
        self
    }

    /// Smaller muted footnote.
    pub(super) fn note(mut self, theme: AppTheme, text: impl Into<SharedString>) -> Self {
        self.sections.push(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child(text.into())
                .into_any_element(),
        );
        self
    }

    /// Monospace value line (branch name, path, stash ref…).
    pub(super) fn mono_value(mut self, theme: AppTheme, text: impl Into<SharedString>) -> Self {
        self.sections.push(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .child(
                    div()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.foreground.secondary)
                        .child(text.into()),
                )
                .into_any_element(),
        );
        self
    }

    /// Monospace git command preview.
    pub(super) fn command(mut self, theme: AppTheme, text: impl Into<SharedString>) -> Self {
        self.sections.push(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.foreground.secondary)
                .child(text.into())
                .into_any_element(),
        );
        self
    }

    pub(super) fn divider(mut self, theme: AppTheme) -> Self {
        self.sections.push(dialog_divider(theme).into_any_element());
        self
    }

    /// Escape hatch for dialog-specific body content.
    pub(super) fn section(mut self, section: impl IntoElement) -> Self {
        self.sections.push(section.into_any_element());
        self
    }

    pub(super) fn render(
        self,
        theme: AppTheme,
        cancel: impl IntoElement,
        actions: impl IntoElement,
        cx: &mut gpui::Context<PopoverHost>,
    ) -> gpui::Div {
        let ui_scale = popover_ui_scale(cx);
        div()
            .flex()
            .flex_col()
            .min_w(self.width.preferred_px(ui_scale))
            .child(popover_title(self.title))
            .child(dialog_divider(theme))
            .children(self.sections)
            .child(dialog_divider(theme))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(cancel)
                    .child(actions),
            )
    }
}

/// Whether the create/rename prompt's name field holds something worth
/// submitting.
///
/// Not just "non-empty": the prompt can open pre-filled with a group prefix
/// (`feat/`), and git rejects a ref ending in `/`. Without this the Create
/// button would be live the instant that prompt opens, and pressing it would
/// produce an error toast instead of the prompt simply declining.
pub(super) fn is_submittable_branch_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.ends_with('/')
}

pub(super) fn popover_title(title: impl Into<SharedString>) -> gpui::Div {
    let title: SharedString = title.into();
    div()
        .px_2()
        .py_1()
        .text_sm()
        .font_weight(FontWeight::BOLD)
        .child(title)
}

pub(super) fn input_label(theme: AppTheme, label: &'static str) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.colors.foreground.secondary)
        .child(label)
}

/// Which corner of the popover is placed on its anchor.
///
/// Most menus hang off a button on the right of their row, so they open
/// leftwards. The link menus are the exception: their anchor is the box of a
/// span of text, and a menu that reads as belonging to that span has to start
/// where the span starts.
fn popover_anchor_corner(kind: &PopoverKind) -> Anchor {
    match kind {
        PopoverKind::PullPicker
        | PopoverKind::HookActivity { .. }
        | PopoverKind::PushPicker
        | PopoverKind::CreateBranchFromRefPrompt { .. }
        | PopoverKind::RenameBranchPrompt { .. }
        | PopoverKind::StashPrompt
        | PopoverKind::StashDropConfirm { .. }
        | PopoverKind::CloneRepo
        | PopoverKind::ResetPrompt { .. }
        | PopoverKind::CreateTagPrompt { .. }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Remote(
                    RemotePopoverKind::AddPrompt
                    | RemotePopoverKind::EditUrlPrompt { .. }
                    | RemotePopoverKind::RemoveConfirm { .. },
                ),
            ..
        }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Worktree(
                    WorktreePopoverKind::AddPrompt
                    | WorktreePopoverKind::OpenPicker
                    | WorktreePopoverKind::RemovePicker
                    | WorktreePopoverKind::RemoveConfirm { .. },
                ),
            ..
        }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Submodule(
                    SubmodulePopoverKind::AddPrompt
                    | SubmodulePopoverKind::ChangePointerPrompt { .. }
                    | SubmodulePopoverKind::TrustConfirm
                    | SubmodulePopoverKind::OpenPicker
                    | SubmodulePopoverKind::RemovePicker
                    | SubmodulePopoverKind::RemoveConfirm { .. },
                ),
            ..
        }
        | PopoverKind::PushSetUpstreamPrompt { .. }
        | PopoverKind::ForcePushConfirm { .. }
        | PopoverKind::CherryPickCommitConfirm { .. }
        | PopoverKind::MergeCommitConfirm { .. }
        | PopoverKind::MergeAbortConfirm { .. }
        | PopoverKind::BranchExistsPrompt { .. }
        | PopoverKind::ForceDeleteBranchConfirm { .. }
        | PopoverKind::ForceRemoveWorktreeConfirm { .. }
        | PopoverKind::PullReconcilePrompt { .. }
        | PopoverKind::RebaseOntoConfirm { .. }
        | PopoverKind::RebaseReword { .. }
        | PopoverKind::CommitOptionsMenu { .. }
        | PopoverKind::PreviousCommitMessagesMenu { .. }
        | PopoverKind::RepoTabMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::HistoryBranchFilter { .. }
        | PopoverKind::HistoryAuthorFilter { .. }
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::CommitFileSortMenu
        | PopoverKind::ChangeTrackingSettings
        | PopoverKind::TerminalMenu { .. }
        | PopoverKind::UiScalePicker => Anchor::TopRight,
        _ => Anchor::TopLeft,
    }
}

pub(in super::super) fn popover_width_spec(kind: &PopoverKind) -> Option<PopoverWidthSpec> {
    match kind {
        PopoverKind::RepoPicker
        | PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Delete | BranchPickerPurpose::RebaseOnto,
        } => Some(PICKER_WIDTH),
        PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Checkout,
        }
        | PopoverKind::UpstreamPicker { .. } => Some(LARGE_PICKER_WIDTH),
        PopoverKind::StashPrompt
        | PopoverKind::CommitPrompt { .. }
        | PopoverKind::StashPickerPrompt { .. }
        | PopoverKind::CloneRepo
        | PopoverKind::CreateTagPrompt { .. }
        | PopoverKind::SquashPrompt { .. } => Some(DIALOG_420_WIDTH),
        PopoverKind::HookActivity { .. } => Some(DIALOG_900_WIDTH),
        PopoverKind::CreateBranchFromRefPrompt { .. }
        | PopoverKind::RenameBranchPrompt { .. }
        | PopoverKind::CheckoutRemoteBranchPrompt { .. } => Some(DIALOG_540_WIDTH),
        PopoverKind::StashDropConfirm { .. }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Remote(
                    RemotePopoverKind::RemoveConfirm { .. }
                    | RemotePopoverKind::DeleteBranchConfirm { .. },
                ),
            ..
        }
        | PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::RemoveConfirm { .. }),
            ..
        }
        | PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::RemoveConfirm { .. }),
            ..
        }
        | PopoverKind::ForcePushConfirm { .. }
        | PopoverKind::ForceDeleteBranchConfirm { .. }
        | PopoverKind::DeleteBranchesConfirm { .. }
        | PopoverKind::DiscardChangesConfirm { .. }
        | PopoverKind::StageConflictMarkersConfirm { .. } => Some(DIALOG_420_WIDTH),
        PopoverKind::PushSetUpstreamPrompt { .. } => Some(DIALOG_320_WIDTH),
        PopoverKind::ResetPrompt { .. }
        | PopoverKind::RebaseOntoConfirm { .. }
        | PopoverKind::CherryPickCommitConfirm { .. }
        | PopoverKind::MergeCommitConfirm { .. } => Some(DIALOG_380_WIDTH),
        PopoverKind::BranchExistsPrompt { .. } => Some(DIALOG_540_WIDTH),
        PopoverKind::MergeAbortConfirm { .. } => Some(DIALOG_360_WIDTH),
        PopoverKind::ForceRemoveWorktreeConfirm { .. } => Some(DIALOG_460_WIDTH),
        PopoverKind::PullReconcilePrompt { .. } | PopoverKind::AddToGitignorePrompt { .. } => {
            Some(DIALOG_440_WIDTH)
        }
        PopoverKind::Repo {
            kind:
                RepoPopoverKind::Remote(
                    RemotePopoverKind::AddPrompt | RemotePopoverKind::EditUrlPrompt { .. },
                ),
            ..
        }
        | PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
            ..
        }
        | PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
            ..
        } => Some(DIALOG_640_WIDTH),
        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::TrustConfirm),
            ..
        } => Some(DIALOG_460_WIDTH),
        PopoverKind::Repo {
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
            ..
        } => Some(DIALOG_420_WIDTH),
        PopoverKind::Repo {
            kind:
                RepoPopoverKind::Worktree(
                    WorktreePopoverKind::OpenPicker
                    | WorktreePopoverKind::RemovePicker
                    | WorktreePopoverKind::BadgePicker,
                ),
            ..
        }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Submodule(
                    SubmodulePopoverKind::OpenPicker | SubmodulePopoverKind::RemovePicker,
                ),
            ..
        }
        | PopoverKind::FileHistory { .. } => Some(LARGE_PICKER_WIDTH),
        PopoverKind::AppMenu => Some(APP_MENU_WIDTH),
        PopoverKind::AddRepoMenu => Some(DEFAULT_CONTEXT_MENU_WIDTH),
        PopoverKind::TerminalShutdownConfirm(_) | PopoverKind::UnsavedFileEditsConfirm(_) => {
            Some(DIALOG_440_WIDTH)
        }
        PopoverKind::TerminalMenu { .. } => Some(DEFAULT_CONTEXT_MENU_WIDTH),
        PopoverKind::WebLinkMenu { .. } | PopoverKind::DiffActionMenu => {
            Some(DIFF_ACTION_MENU_WIDTH)
        }
        // Shares "Browse repository at this point" with the commit menu, and so
        // needs the same extra room.
        PopoverKind::CommitShaLinkMenu { .. } => Some(PopoverWidthSpec::range(300.0, 220.0, 400.0)),
        // "Browse repository at this point" needs more room than the default
        // context-menu width.
        PopoverKind::CommitMenu { .. } => Some(PopoverWidthSpec::range(300.0, 220.0, 400.0)),
        // Resolver settings have substantially longer labels than diff actions.
        // A dedicated preferred width also feeds the shared anchor-side chooser,
        // allowing the menu to flip toward the side where the full label fits.
        PopoverKind::MergetoolSettingsMenu => Some(MERGETOOL_SETTINGS_MENU_WIDTH),
        PopoverKind::PullPicker
        | PopoverKind::PushPicker
        | PopoverKind::CommitOptionsMenu { .. }
        | PopoverKind::PreviousCommitMessagesMenu { .. }
        | PopoverKind::TagMenu { .. }
        | PopoverKind::TagRefMenu { .. }
        | PopoverKind::StatusFileMenu { .. }
        | PopoverKind::BranchMenu { .. }
        | PopoverKind::BranchRefsMenu { .. }
        | PopoverKind::BranchSectionMenu { .. }
        | PopoverKind::SubmoduleInnerDiffMenu { .. }
        | PopoverKind::Repo {
            kind: RepoPopoverKind::Remote(RemotePopoverKind::Menu { .. }),
            ..
        }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Worktree(
                    WorktreePopoverKind::SectionMenu | WorktreePopoverKind::Menu { .. },
                ),
            ..
        }
        | PopoverKind::Repo {
            kind:
                RepoPopoverKind::Submodule(
                    SubmodulePopoverKind::SectionMenu | SubmodulePopoverKind::Menu { .. },
                ),
            ..
        }
        | PopoverKind::CommitFileMenu { .. }
        | PopoverKind::FileBrowserFileMenu { .. }
        | PopoverKind::FileBrowserFolderMenu { .. }
        | PopoverKind::BranchGroupMenu { .. }
        | PopoverKind::PinnedSectionMenu { .. }
        | PopoverKind::ReflogEntryMenu { .. }
        | PopoverKind::BrowseHistoryMenu { .. } => Some(DEFAULT_CONTEXT_MENU_WIDTH),
        PopoverKind::RepoTabMenu { .. } => Some(REPO_TAB_MENU_WIDTH),
        PopoverKind::HistoryBranchFilter { .. }
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::CommitFileSortMenu
        | PopoverKind::UiScalePicker
        | PopoverKind::DiffHunkMenu { .. } => Some(NARROW_CONTEXT_MENU_WIDTH),
        PopoverKind::HistoryAuthorFilter { .. } => Some(HISTORY_AUTHOR_FILTER_WIDTH),
        PopoverKind::ChangeTrackingSettings => Some(CHANGE_TRACKING_MENU_WIDTH),
        PopoverKind::DiffEditorMenu { .. } => Some(DIFF_EDITOR_MENU_WIDTH),
        PopoverKind::ConflictResolverInputRowMenu { .. } => Some(CONFLICT_INPUT_MENU_WIDTH),
        PopoverKind::ConflictResolverChunkMenu { .. } => Some(CONFLICT_CHUNK_MENU_WIDTH),
        PopoverKind::ConflictResolverOutputMenu { .. } => Some(CONFLICT_OUTPUT_MENU_WIDTH),
        PopoverKind::StashMenu { .. } => Some(STASH_MENU_WIDTH),
        PopoverKind::RebaseReword { .. } => Some(DIALOG_440_WIDTH),
        PopoverKind::InteractiveRebaseActionMenu { .. } => Some(REBASE_ACTION_MENU_WIDTH),
        PopoverKind::InteractiveRebaseAutosquashMenu => Some(REBASE_AUTOSQUASH_MENU_WIDTH),
    }
}

fn popover_preferred_anchor_width(kind: &PopoverKind, ui_scale: ui_scale::UiScale) -> Pixels {
    popover_width_spec(kind)
        .map(|spec| spec.preferred_px(ui_scale).max(spec.min_px(ui_scale)))
        .unwrap_or_else(|| ui_scale.px(640.0))
}

fn choose_popover_anchor_corner(
    anchor_corner: Anchor,
    space_left: Pixels,
    space_right: Pixels,
    preferred_width: Pixels,
) -> Anchor {
    match anchor_corner {
        Anchor::TopRight if space_left < preferred_width && space_right > space_left => {
            Anchor::TopLeft
        }
        Anchor::BottomRight if space_left < preferred_width && space_right > space_left => {
            Anchor::BottomLeft
        }
        Anchor::TopLeft if space_right < preferred_width && space_left > space_right => {
            Anchor::TopRight
        }
        Anchor::BottomLeft if space_right < preferred_width && space_left > space_right => {
            Anchor::BottomRight
        }
        _ => anchor_corner,
    }
}

/// The directory name to clone into, derived from the URL's last segment.
///
/// The result is joined onto the parent folder the user picked, so it must be
/// exactly one ordinary path component: `..`, `.`, an empty segment or a drive
/// prefix fall back to `repo` instead of resolving somewhere else.
fn clone_repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    let mut components = std::path::Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => name.to_string(),
        _ => "repo".to_string(),
    }
}

#[cfg(test)]
mod clone_repo_name_tests {
    use super::clone_repo_name_from_url;

    #[test]
    fn clone_repo_name_takes_the_last_url_segment() {
        assert_eq!(
            clone_repo_name_from_url("https://example.com/org/repo.git"),
            "repo"
        );
        assert_eq!(
            clone_repo_name_from_url("git@github.com:org/tools.git/"),
            "tools"
        );
        assert_eq!(
            clone_repo_name_from_url("C:\\src\\local-repo"),
            "local-repo"
        );
    }

    #[test]
    fn clone_repo_name_never_resolves_outside_the_parent_folder() {
        for url in [
            "https://example.com/org/..",
            "https://example.com/org/../",
            "https://example.com/org/.",
            "https://example.com/org/..git",
            "",
            "   ",
            "/",
        ] {
            assert_eq!(clone_repo_name_from_url(url), "repo", "{url:?}");
        }
    }
}

mod host;
mod host_render;
mod prompt_actions;

#[cfg(test)]
mod tests;

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_file_history_rows(
    page: &gitcomet_core::domain::LogPage,
    now: std::time::SystemTime,
) -> Vec<components::PickerPromptItem> {
    file_history::benchmark_rows(page, now)
}
