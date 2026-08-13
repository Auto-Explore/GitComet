use super::*;
use gitcomet_core::services::InteractiveRebaseAction;

mod add_repo_menu;
mod app_menu;
mod branch_picker;
mod checkout_remote_branch_prompt;
mod cherry_pick_commit_confirm;
mod clone_repo;
mod commit_prompt;
pub(in super::super) mod context_menu;
mod create_branch_from_ref_prompt;
mod create_tag_prompt;
mod delete_remote_branch_confirm;
mod discard_changes_confirm;
mod file_history;
mod fingerprint;
mod force_delete_branch_confirm;
mod force_push_confirm;
mod force_remove_worktree_confirm;
mod merge_abort_confirm;
mod picker_nav;
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
    _worktree_picker_search_input_subscription: Option<gpui::Subscription>,
    _workspace_picker_search_input_subscription: Option<gpui::Subscription>,
    _submodule_picker_search_input_subscription: Option<gpui::Subscription>,
    _file_history_search_input_subscription: Option<gpui::Subscription>,
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
    sidebar_pane: Entity<SidebarPaneView>,
    /// Mirror of the sidebar pane's pinned branches, keyed by repository
    /// workdir. Kept here because context menus are built from click handlers
    /// that already hold the sidebar pane's update borrow, so its entity can't
    /// be read at that point.
    pinned_branches_by_repo:
        std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,

    popover: Option<PopoverKind>,
    popover_anchor: Option<PopoverAnchor>,
    /// Explicit 1-based mainline selected for the currently open single
    /// merge-commit cherry-pick confirmation. Reset every time that dialog
    /// opens; drafts are intentionally session-local.
    cherry_pick_mainline: Option<usize>,
    context_menu_focus_handle: FocusHandle,
    /// Focus held by the App/Add Repository menu invoker, restored when that
    /// menu is dismissed without replacing it with another prompt.
    menu_invoker_focus: Option<FocusHandle>,
    prompt_tab_group_focus_handle: FocusHandle,
    prompt_tab_wrap_end_focus_handle: FocusHandle,
    context_menu_selected_ix: Option<usize>,
    repo_picker_selected_index: Option<usize>,
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
    repo_picker_row_menu: Option<repo_picker::RepoPickerRowMenu>,
    branch_picker_selected_index: Option<usize>,
    worktree_picker_selected_index: Option<usize>,
    workspace_picker_selected_index: Option<usize>,
    /// Path/reference the workspace badge's create row hands to the Add-worktree
    /// dialog. Consumed (and cleared) when that dialog opens, so a later
    /// open from elsewhere still starts blank.
    pending_worktree_add_prefill: Option<(String, String)>,
    submodule_picker_selected_index: Option<usize>,
    file_history_selected_index: Option<usize>,
    /// Row models for the pickers that build one row per repository, ref or
    /// worktree, rebuilt only when the data behind them changes rather than on
    /// every frame. See [`rows_cache`] — a hover moving between rows re-renders
    /// this whole view.
    branch_picker_rows_cache: rows_cache::RowsCache<branch_picker::BranchPickerNavTarget>,
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
    commit_prompt_message_drafts: HashMap<RepoId, SharedString>,
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
            s.bg(theme.colors.focus_ring_bg)
                .border_color(theme.colors.focus_ring)
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
            | PopoverKind::MarkdownLinkMenu { .. }
            | PopoverKind::DiffActionMenu
            | PopoverKind::InteractiveRebaseActionMenu { .. }
            | PopoverKind::InteractiveRebaseAutosquashMenu
            | PopoverKind::MergetoolSettingsMenu
            | PopoverKind::HistoryBranchFilter { .. }
            | PopoverKind::DiffContentModeSettings
            | PopoverKind::ChangeTrackingSettings
            | PopoverKind::UiScalePicker
            | PopoverKind::TerminalMenu { .. }
            | PopoverKind::DiffHunkMenu { .. }
            | PopoverKind::DiffEditorMenu { .. }
            | PopoverKind::ConflictResolverInputRowMenu { .. }
            | PopoverKind::ConflictResolverChunkMenu { .. }
            | PopoverKind::ConflictResolverOutputMenu { .. }
            | PopoverKind::CommitMenu { .. }
            | PopoverKind::TagMenu { .. }
            | PopoverKind::TagRefMenu { .. }
            | PopoverKind::StatusFileMenu { .. }
            | PopoverKind::BranchMenu { .. }
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
            | PopoverKind::BrowseHistoryMenu { .. }
    )
}

fn popover_is_confirm_dialog(kind: &PopoverKind) -> bool {
    matches!(
        kind,
        PopoverKind::StashDropConfirm { .. }
            | PopoverKind::ForcePushConfirm { .. }
            | PopoverKind::CherryPickCommitConfirm { .. }
            | PopoverKind::MergeAbortConfirm { .. }
            | PopoverKind::RebaseOntoConfirm { .. }
            | PopoverKind::RebaseReword { .. }
            | PopoverKind::ForceDeleteBranchConfirm { .. }
            | PopoverKind::ForceRemoveWorktreeConfirm { .. }
            | PopoverKind::DiscardChangesConfirm { .. }
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
        .text_color(theme.colors.text_muted)
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
    div().border_t_1().border_color(theme.colors.border)
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
                .text_color(theme.colors.text_muted)
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
                .text_color(theme.colors.text_muted)
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
                        .text_color(theme.colors.text_muted)
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
                .text_color(theme.colors.text_muted)
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
        .text_color(theme.colors.text_muted)
        .child(label)
}

fn popover_anchor_corner(kind: &PopoverKind) -> Anchor {
    match kind {
        PopoverKind::PullPicker
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
        | PopoverKind::MergeAbortConfirm { .. }
        | PopoverKind::ForceDeleteBranchConfirm { .. }
        | PopoverKind::ForceRemoveWorktreeConfirm { .. }
        | PopoverKind::PullReconcilePrompt { .. }
        | PopoverKind::RebaseOntoConfirm { .. }
        | PopoverKind::RebaseReword { .. }
        | PopoverKind::CommitOptionsMenu { .. }
        | PopoverKind::PreviousCommitMessagesMenu { .. }
        | PopoverKind::RepoTabMenu { .. }
        | PopoverKind::MarkdownLinkMenu { .. }
        | PopoverKind::DiffActionMenu
        | PopoverKind::MergetoolSettingsMenu
        | PopoverKind::HistoryBranchFilter { .. }
        | PopoverKind::DiffContentModeSettings
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
        } => Some(LARGE_PICKER_WIDTH),
        PopoverKind::StashPrompt
        | PopoverKind::CommitPrompt { .. }
        | PopoverKind::StashPickerPrompt { .. }
        | PopoverKind::CloneRepo
        | PopoverKind::CreateTagPrompt { .. }
        | PopoverKind::SquashPrompt { .. } => Some(DIALOG_420_WIDTH),
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
        | PopoverKind::DiscardChangesConfirm { .. }
        | PopoverKind::StageConflictMarkersConfirm { .. } => Some(DIALOG_420_WIDTH),
        PopoverKind::PushSetUpstreamPrompt { .. } => Some(DIALOG_320_WIDTH),
        PopoverKind::ResetPrompt { .. }
        | PopoverKind::RebaseOntoConfirm { .. }
        | PopoverKind::CherryPickCommitConfirm { .. } => Some(DIALOG_380_WIDTH),
        PopoverKind::MergeAbortConfirm { .. } => Some(DIALOG_360_WIDTH),
        PopoverKind::ForceRemoveWorktreeConfirm { .. } => Some(DIALOG_460_WIDTH),
        PopoverKind::PullReconcilePrompt { .. } => Some(DIALOG_440_WIDTH),
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
        PopoverKind::MarkdownLinkMenu { .. } | PopoverKind::DiffActionMenu => {
            Some(DIFF_ACTION_MENU_WIDTH)
        }
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
        | PopoverKind::BrowseHistoryMenu { .. } => Some(DEFAULT_CONTEXT_MENU_WIDTH),
        PopoverKind::RepoTabMenu { .. } => Some(REPO_TAB_MENU_WIDTH),
        PopoverKind::HistoryBranchFilter { .. }
        | PopoverKind::DiffContentModeSettings
        | PopoverKind::UiScalePicker
        | PopoverKind::DiffHunkMenu { .. } => Some(NARROW_CONTEXT_MENU_WIDTH),
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

impl PopoverHost {
    #[cfg(test)]
    pub(in crate::view) fn create_branch_input_focus_handle_for_test(
        &self,
        app: &App,
    ) -> FocusHandle {
        self.create_branch_input.read(app).focus_handle()
    }

    fn sync_titlebar_app_menu_state(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        let app_menu_open = matches!(self.popover, Some(PopoverKind::AppMenu));
        let repo_picker_open = matches!(self.popover, Some(PopoverKind::RepoPicker));
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.title_bar.update(cx, |title_bar, cx| {
                    title_bar.set_app_menu_open(app_menu_open, cx);
                    title_bar.set_repo_picker_open(repo_picker_open, cx);
                });
            });
        });
    }

    fn clear_active_context_menu_invoker(&self, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_active_context_menu_invoker(None, cx);
            });
        });
    }

    fn history_refs_menu_active(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.root_view
            .update(cx, |root, _cx| {
                root.active_context_menu_invoker
                    .as_ref()
                    .is_some_and(|invoker| {
                        invoker.as_ref().starts_with(
                            crate::view::history_refs_hover::HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX,
                        )
                    })
            })
            .unwrap_or(false)
    }

    /// Subscription that submits a prompt when Enter is pressed in one of its
    /// inputs. Escape is consumed here; prompt dismissal is handled by the
    /// PopoverPrompt key context.
    fn prompt_enter_subscription(
        input: &Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
        is_active: fn(&Self) -> bool,
        submit: fn(&mut Self, &mut Window, &mut gpui::Context<Self>),
    ) -> gpui::Subscription {
        cx.observe_in(input, window, move |this, input, window, cx| {
            let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
            let _ = input.update(cx, |input, _| input.take_escape_pressed());

            if !is_active(this) {
                return;
            }

            if enter_pressed {
                submit(this, window, cx);
                return;
            }

            cx.notify();
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        theme_mode: ThemeMode,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        change_tracking_view: ChangeTrackingView,
        commit_push_after_enabled: bool,
        diff_content_mode: DiffContentMode,
        diff_whitespace_mode: DiffWhitespaceMode,
        diff_reveal_whitespace_chars: bool,
        diff_word_wrap: bool,
        diff_show_line_numbers: bool,
        root_view: WeakEntity<GitCometView>,
        root_view_mode: GitCometViewMode,
        tooltip_host: WeakEntity<TooltipHost>,
        main_pane: Entity<MainPaneView>,
        details_pane: Entity<DetailsPaneView>,
        sidebar_pane: Entity<SidebarPaneView>,
        pinned_branches_by_repo: std::collections::BTreeMap<
            std::path::PathBuf,
            std::collections::BTreeSet<String>,
        >,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            this.state = Arc::clone(&model.read(cx).state);
            this.commit_prompt_message_drafts
                .retain(|repo_id, _| this.state.repos.iter().any(|repo| repo.id == *repo_id));

            // Prefill the squash prompt from the message preview when it lands,
            // rather than in the render path, so the generated message never
            // clobbers text the user typed while it was loading.
            this.sync_squash_prompt_prefill(cx);

            let Some(popover) = this.popover.as_ref() else {
                return;
            };

            let next_fingerprint = fingerprint::notify_fingerprint(&this.state, popover);
            if next_fingerprint != this.notify_fingerprint {
                this.notify_fingerprint = next_fingerprint;
                cx.notify();
            }
        });

        let clone_repo_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let clone_repo_parent_dir_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/parent/folder".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let rebase_onto_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "origin/main".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let create_tag_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "v1.0.0".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let create_tag_message_scroll = ScrollHandle::new();
        let create_tag_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Annotation message (optional)".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 3,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(create_tag_message_scroll.clone()));
            input
        });

        let squash_message_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Commit message".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let squash_description_scroll = ScrollHandle::new();
        let squash_description_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Description (optional)".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 4,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(squash_description_scroll.clone()));
            input
        });

        let remote_name_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "origin".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let remote_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let remote_url_edit_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let create_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-name".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let stash_message_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Stash message".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        // The subject input re-renders the host on every keystroke so the
        // Squash button's disabled state (driven by whether the message is
        // empty) stays current, and submits on Enter.
        let squash_message_input_subscription =
            cx.observe(&squash_message_input, |this, input, cx| {
                let enter_pressed = input.update(cx, |input, _| input.take_enter_pressed());
                let _ = input.update(cx, |input, _| input.take_escape_pressed());

                if !matches!(this.popover, Some(PopoverKind::SquashPrompt { .. })) {
                    return;
                }

                if enter_pressed {
                    this.submit_squash(cx);
                    return;
                }

                cx.notify();
            });

        // The multiline description input only needs to re-render the host (it
        // does not affect the button state, and Enter inserts a newline).
        let squash_description_input_subscription =
            cx.observe(&squash_description_input, |this, _input, cx| {
                if !matches!(this.popover, Some(PopoverKind::SquashPrompt { .. })) {
                    return;
                }
                cx.notify();
            });

        let commit_prompt_message_scroll = ScrollHandle::new();
        let commit_prompt_message_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Commit message".into(),
                    multiline: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(commit_prompt_message_scroll.clone()));
            input
        });

        let push_upstream_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-name".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let worktree_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/worktree".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let worktree_ref_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-or-commit".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_url_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "https://example.com/org/repo.git".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_path_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "path/in/repo".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_ref_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "branch-or-commit".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_name_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "submodule-logical-name".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let submodule_branch_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "feature".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        let rebase_reword_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Commit subject".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        let rebase_reword_description_scroll = ScrollHandle::new();
        let rebase_reword_description_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Description (optional)".into(),
                    multiline: true,
                    soft_wrap: true,
                    min_lines: 4,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(rebase_reword_description_scroll.clone()));
            input
        });

        let mut prompt_input_subscriptions = Vec::new();
        prompt_input_subscriptions.push(cx.observe(
            &commit_prompt_message_input,
            |this, _input, cx| {
                if matches!(this.popover, Some(PopoverKind::CommitPrompt { .. })) {
                    cx.notify();
                }
            },
        ));
        for input in [&clone_repo_url_input, &clone_repo_parent_dir_input] {
            prompt_input_subscriptions.push(Self::prompt_enter_subscription(
                input,
                window,
                cx,
                |this| matches!(this.popover, Some(PopoverKind::CloneRepo)),
                |this, _window, cx| this.submit_clone_repo(cx),
            ));
        }
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &create_tag_input,
            window,
            cx,
            |this| matches!(this.popover, Some(PopoverKind::CreateTagPrompt { .. })),
            |this, _window, cx| this.submit_create_tag(cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &create_branch_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                        | Some(PopoverKind::RenameBranchPrompt { .. })
                        | Some(PopoverKind::CheckoutRemoteBranchPrompt { .. })
                )
            },
            |this, window, cx| {
                if matches!(
                    this.popover,
                    Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                ) {
                    this.submit_create_branch(window, cx);
                } else if matches!(this.popover, Some(PopoverKind::RenameBranchPrompt { .. })) {
                    this.submit_rename_branch(window, cx);
                } else {
                    this.submit_checkout_remote_branch(cx);
                }
            },
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &stash_message_input,
            window,
            cx,
            |this| matches!(this.popover, Some(PopoverKind::StashPrompt)),
            |this, window, cx| this.submit_stash(window, cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &submodule_ref_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::Repo {
                        kind: RepoPopoverKind::Submodule(
                            SubmodulePopoverKind::ChangePointerPrompt { .. }
                        ),
                        ..
                    })
                )
            },
            |this, window, cx| this.submit_submodule_change_pointer(window, cx),
        ));
        for input in [&remote_name_input, &remote_url_input] {
            prompt_input_subscriptions.push(Self::prompt_enter_subscription(
                input,
                window,
                cx,
                |this| {
                    matches!(
                        this.popover,
                        Some(PopoverKind::Repo {
                            kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                            ..
                        })
                    )
                },
                |this, _window, cx| this.submit_remote_add(cx),
            ));
        }
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &remote_url_edit_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::Repo {
                        kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { .. }),
                        ..
                    })
                )
            },
            |this, _window, cx| this.submit_remote_edit_url(cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &push_upstream_branch_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::PushSetUpstreamPrompt { .. })
                )
            },
            |this, _window, cx| this.submit_push_set_upstream(cx),
        ));
        prompt_input_subscriptions.push(Self::prompt_enter_subscription(
            &worktree_path_input,
            window,
            cx,
            |this| {
                matches!(
                    this.popover,
                    Some(PopoverKind::Repo {
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                        ..
                    })
                )
            },
            |this, _window, cx| this.submit_worktree_add(cx),
        ));
        for input in [
            &submodule_url_input,
            &submodule_path_input,
            &submodule_branch_input,
            &submodule_name_input,
        ] {
            prompt_input_subscriptions.push(Self::prompt_enter_subscription(
                input,
                window,
                cx,
                |this| {
                    matches!(
                        this.popover,
                        Some(PopoverKind::Repo {
                            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                            ..
                        })
                    )
                },
                |this, _window, cx| this.submit_submodule_add(cx),
            ));
        }

        let context_menu_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let prompt_tab_group_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let prompt_tab_wrap_end_focus_handle = cx.focus_handle().tab_index(1).tab_stop(false);
        let create_branch_from_ref_checkout_focus_handle =
            cx.focus_handle().tab_index(0).tab_stop(true);
        let create_branch_from_ref_focus = DialogFocus::new(cx);
        let checkout_remote_branch_focus = DialogFocus::new(cx);
        let stash_focus = DialogFocus::new(cx);
        let commit_prompt_focus = DialogFocus::new(cx);
        let clone_repo_browse_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let squash_cancel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let squash_submit_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let rebase_onto_submit_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let clone_repo_focus = DialogFocus::new(cx);
        let create_tag_focus = DialogFocus::new(cx);
        let create_tag_annotated_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let remote_add_focus = DialogFocus::new(cx);
        let remote_edit_focus = DialogFocus::new(cx);
        let push_upstream_focus = DialogFocus::new(cx);
        let worktree_browse_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let worktree_focus = DialogFocus::new(cx);
        let submodule_advanced_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let submodule_force_focus_handle = cx.focus_handle().tab_index(0).tab_stop(true);
        let submodule_focus = DialogFocus::new(cx);

        Self {
            store,
            state,
            theme,
            theme_mode,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            commit_amend_enabled: false,
            commit_push_after_enabled,
            diff_content_mode,
            diff_whitespace_mode,
            diff_reveal_whitespace_chars,
            diff_word_wrap,
            diff_show_line_numbers,
            _ui_model_subscription: subscription,
            _repo_picker_search_input_subscription: None,
            _branch_picker_search_input_subscription: None,
            _worktree_picker_search_input_subscription: None,
            _workspace_picker_search_input_subscription: None,
            _submodule_picker_search_input_subscription: None,
            _file_history_search_input_subscription: None,
            _stash_picker_search_input_subscription: None,
            _squash_message_input_subscription: squash_message_input_subscription,
            _squash_description_input_subscription: squash_description_input_subscription,
            _prompt_input_subscriptions: prompt_input_subscriptions,
            notify_fingerprint: 0,
            root_view,
            root_view_mode,
            tooltip_host,
            main_pane,
            details_pane,
            sidebar_pane,
            pinned_branches_by_repo,
            popover: None,
            popover_anchor: None,
            cherry_pick_mainline: None,
            context_menu_focus_handle,
            menu_invoker_focus: None,
            prompt_tab_group_focus_handle,
            prompt_tab_wrap_end_focus_handle,
            context_menu_selected_ix: None,
            repo_picker_selected_index: None,
            cached_recent_repos: Vec::new(),
            cached_pinned_repos: Vec::new(),
            cached_collapsed_picker_sections: std::collections::BTreeSet::new(),
            repo_picker_sort: repo_picker::RepoPickerSort::default(),
            repo_picker_sort_menu_open: false,
            repo_picker_row_menu: None,
            branch_picker_selected_index: None,
            worktree_picker_selected_index: None,
            workspace_picker_selected_index: None,
            pending_worktree_add_prefill: None,
            submodule_picker_selected_index: None,
            file_history_selected_index: None,
            branch_picker_rows_cache: rows_cache::RowsCache::default(),
            workspace_picker_rows_cache: rows_cache::RowsCache::default(),
            repo_picker_rows_cache: rows_cache::RowsCache::default(),
            stash_picker_rows_cache: rows_cache::RowsCache::default(),
            file_history_rows_cache: rows_cache::RowsCache::default(),
            submodule_picker_rows_cache: rows_cache::RowsCache::default(),
            worktree_picker_rows_cache: rows_cache::RowsCache::default(),
            branch_ref_rows_cache: rows_cache::RowsCache::default(),
            repo_picker_search_input: None,
            branch_picker_search_input: None,
            remote_picker_search_input: None,
            file_history_search_input: None,
            worktree_picker_search_input: None,
            workspace_picker_search_input: None,
            submodule_picker_search_input: None,
            picker_prompt_scroll: ScrollHandle::new(),
            clone_repo_url_input,
            clone_repo_parent_dir_input,
            rebase_onto_input,
            create_tag_input,
            create_tag_message_input,
            create_tag_message_scroll,
            squash_message_input,
            squash_description_input,
            squash_description_scroll,
            squash_prompt_prefilled_range: None,
            remote_name_input,
            remote_url_input,
            remote_url_edit_input,
            create_branch_input,
            create_branch_checkout_enabled: true,
            create_branch_source_target: String::new(),
            worktree_ref_source_target: String::new(),
            suppress_worktree_submit_after_ref_enter: false,
            create_branch_from_ref_checkout_focus_handle,
            create_branch_from_ref_focus,
            create_tag_annotated: false,
            create_tag_annotated_focus_handle,
            checkout_remote_branch_focus,
            stash_message_input,
            stash_focus,
            stash_picker_prompt_selected_index: None,
            stash_picker_search_input: None,
            commit_prompt_message_drafts: HashMap::default(),
            commit_prompt_message_input,
            commit_prompt_message_scroll,
            commit_prompt_focus,
            clone_repo_browse_focus_handle,
            squash_cancel_focus_handle,
            squash_submit_focus_handle,
            rebase_onto_submit_focus_handle,
            clone_repo_focus,
            create_tag_focus,
            remote_add_focus,
            remote_edit_focus,
            push_upstream_focus,
            worktree_browse_focus_handle,
            worktree_focus,
            submodule_advanced_focus_handle,
            submodule_force_focus_handle,
            submodule_focus,
            push_upstream_branch_input,
            worktree_path_input,
            worktree_ref_input,
            submodule_url_input,
            submodule_path_input,
            submodule_ref_input,
            submodule_branch_input,
            submodule_name_input,
            submodule_add_advanced_expanded: false,
            submodule_force_enabled: false,
            rebase_reword_input,
            rebase_reword_description_input,
            rebase_reword_description_scroll,
        }
    }

    /// Every text input owned by the host, including the lazily created
    /// picker search inputs that currently exist.
    fn all_text_inputs(&self) -> impl Iterator<Item = &Entity<components::TextInput>> {
        [
            &self.clone_repo_url_input,
            &self.clone_repo_parent_dir_input,
            &self.rebase_onto_input,
            &self.create_tag_input,
            &self.create_tag_message_input,
            &self.squash_message_input,
            &self.squash_description_input,
            &self.remote_name_input,
            &self.remote_url_input,
            &self.remote_url_edit_input,
            &self.create_branch_input,
            &self.stash_message_input,
            &self.commit_prompt_message_input,
            &self.push_upstream_branch_input,
            &self.worktree_path_input,
            &self.worktree_ref_input,
            &self.submodule_url_input,
            &self.submodule_path_input,
            &self.submodule_ref_input,
            &self.submodule_branch_input,
            &self.submodule_name_input,
            &self.rebase_reword_input,
            &self.rebase_reword_description_input,
        ]
        .into_iter()
        .chain(
            [
                &self.repo_picker_search_input,
                &self.branch_picker_search_input,
                &self.remote_picker_search_input,
                &self.file_history_search_input,
                &self.worktree_picker_search_input,
                &self.workspace_picker_search_input,
                &self.submodule_picker_search_input,
                &self.stash_picker_search_input,
            ]
            .into_iter()
            .flatten(),
        )
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;

        let inputs: Vec<_> = self.all_text_inputs().cloned().collect();
        for input in inputs {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }

        cx.notify();
    }

    pub(in super::super) fn is_kind_open(&self, kind: &PopoverKind) -> bool {
        self.popover.as_ref() == Some(kind)
    }

    #[cfg(test)]
    pub(in super::super) fn popover_kind_for_tests(&self) -> Option<PopoverKind> {
        self.popover.clone()
    }

    #[cfg(test)]
    pub(in super::super) fn worktree_path_input_text_for_tests(&self, app: &gpui::App) -> String {
        self.worktree_path_input.read(app).text().to_string()
    }

    #[cfg(test)]
    pub(in super::super) fn worktree_ref_source_target_for_tests(&self) -> &str {
        &self.worktree_ref_source_target
    }

    /// Whether the unsaved-edits confirmation is the popover on screen.
    ///
    /// Asked by the close/quit path instead of a mirrored bool: that dialog
    /// blocks every further close while it is up, and a mirror that missed a
    /// dismissal wedged the window shut for the rest of the session.
    pub(in super::super) fn showing_unsaved_file_edits_prompt(&self) -> bool {
        matches!(self.popover, Some(PopoverKind::UnsavedFileEditsConfirm(_)))
    }

    pub(in super::super) fn close_popover(&mut self, cx: &mut gpui::Context<Self>) {
        let dismissing_unsaved_prompt = self.showing_unsaved_file_edits_prompt();
        self.save_commit_prompt_draft(cx);
        self.clear_truncated_tooltip(cx);
        crate::view::tooltip::set_tooltips_suppressed_by_overlay(false, cx);
        self.popover = None;
        self.popover_anchor = None;
        self.context_menu_selected_ix = None;
        self.repo_picker_row_menu = None;
        self.menu_invoker_focus = None;
        self.notify_fingerprint = 0;
        self.sync_titlebar_app_menu_state(cx);
        self.clear_active_context_menu_invoker(cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                if dismissing_unsaved_prompt {
                    root.clear_pending_unsaved_file_edits_prompt(cx);
                }
                root.set_history_refs_hover_item_menu_open(false, cx);
            });
        });
        cx.notify();
    }

    /// Validates the repo's current multi-selection against its loaded log and
    /// HEAD, returning a squash plan when the selection is eligible. Shared by
    /// the squash prompt's render, prefill, and submit paths so they always
    /// agree on the range.
    pub(in super::super) fn squash_plan_for_repo_id(
        &self,
        repo_id: RepoId,
    ) -> Option<gitcomet_core::squash::SquashPlan> {
        let repo = self.state.repos.iter().find(|r| r.id == repo_id)?;
        let Loadable::Ready(page) = &repo.log else {
            return None;
        };
        let head = repo.head_commit_id()?;
        gitcomet_core::squash::squash_eligibility(
            &page.commits,
            &repo.history_state.multi_selection.commits,
            &head,
        )
    }

    /// Populates the squash prompt's inputs from the loaded message preview.
    /// Only fires when the preview matches the live plan's range (never a stale
    /// preview from an earlier selection) and only while both inputs are still
    /// empty for a range not yet prefilled (never over the user's own text).
    fn sync_squash_prompt_prefill(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::SquashPrompt { repo_id }) = self.popover else {
            return;
        };
        let Some(plan) = self.squash_plan_for_repo_id(repo_id) else {
            return;
        };
        let repo = self.state.repos.iter().find(|r| r.id == repo_id);
        let Some(Loadable::Ready(preview)) = repo.map(|repo| &repo.history_state.squash_preview)
        else {
            return;
        };
        // The preview must belong to the range currently planned, not a leftover
        // from a previous prompt whose PrepareSquash dispatch has not landed yet.
        if preview.oldest != plan.oldest || preview.head != plan.head {
            return;
        }
        let range = (plan.oldest.clone(), plan.head.clone());
        if self.squash_prompt_prefilled_range.as_ref() == Some(&range) {
            return;
        }
        // Empty inputs mean the user has not typed anything for this range yet;
        // if they had, we must not overwrite it.
        let inputs_empty = self
            .squash_message_input
            .read_with(cx, |input, _| input.text().is_empty())
            && self
                .squash_description_input
                .read_with(cx, |input, _| input.text().is_empty());
        if !inputs_empty {
            return;
        }

        let subject = preview.subject.clone();
        let body = preview.body.clone();
        self.squash_prompt_prefilled_range = Some(range);
        self.squash_message_input.update(cx, |input, cx| {
            input.set_text(subject, cx);
            cx.notify();
        });
        self.squash_description_input.update(cx, |input, cx| {
            input.set_text(body, cx);
            cx.notify();
        });
    }

    /// Reads the squash prompt inputs, builds the final message, and dispatches
    /// the squash against the live plan. No-ops if the selection is no longer
    /// eligible or the subject is empty.
    fn submit_squash(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::SquashPrompt { repo_id }) = self.popover else {
            return;
        };
        let Some(plan) = self.squash_plan_for_repo_id(repo_id) else {
            return;
        };
        let subject = self
            .squash_message_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if subject.is_empty() {
            return;
        }
        let body = self
            .squash_description_input
            .read_with(cx, |input, _| input.text().to_string());
        let message = if body.trim().is_empty() {
            subject
        } else {
            format!("{subject}\n\n{}", body.trim_end())
        };
        self.store.dispatch(Msg::SquashCommits {
            repo_id,
            oldest: plan.oldest,
            expected_head: plan.head,
            message,
            count: plan.commit_count,
        });
        self.close_popover(cx);
    }

    pub(in super::super) fn close_popover_and_restore_focus(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let menu_invoker_focus = self.menu_invoker_focus.take();
        let restore_diff_panel_focus = matches!(
            self.popover,
            Some(
                PopoverKind::ChangeTrackingSettings
                    | PopoverKind::DiffContentModeSettings
                    | PopoverKind::MarkdownLinkMenu { .. }
                    | PopoverKind::DiffActionMenu
                    | PopoverKind::MergetoolSettingsMenu
                    | PopoverKind::DiffHunkMenu { .. }
                    | PopoverKind::DiffEditorMenu { .. }
            )
        );
        self.close_popover(cx);
        if restore_diff_panel_focus {
            let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
        } else if let Some(focus) = menu_invoker_focus {
            window.focus(&focus, cx);
        }
    }

    pub(in super::super) fn is_open(&self) -> bool {
        self.popover.is_some()
    }

    fn prompt_tab_navigation_enabled(&self) -> bool {
        matches!(
            self.popover,
            Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                | Some(PopoverKind::RenameBranchPrompt { .. })
                | Some(PopoverKind::CheckoutRemoteBranchPrompt { .. })
                | Some(PopoverKind::StashPrompt)
                | Some(PopoverKind::CommitPrompt { .. })
                | Some(PopoverKind::CloneRepo)
                | Some(PopoverKind::CreateTagPrompt { .. })
                | Some(PopoverKind::SquashPrompt { .. })
                | Some(PopoverKind::PushSetUpstreamPrompt { .. })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { .. }),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(
                        SubmodulePopoverKind::ChangePointerPrompt { .. }
                    ),
                    ..
                })
        ) || self.popover.as_ref().is_some_and(popover_is_confirm_dialog)
    }

    fn wrap_prompt_focus(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if forward {
            window.focus(&self.prompt_tab_group_focus_handle, cx);
            window.focus_next(cx);
        } else {
            window.focus(&self.prompt_tab_wrap_end_focus_handle, cx);
            window.focus_prev(cx);
        }
    }

    fn focus_next_prompt_field(
        &mut self,
        _: &crate::view::PopoverPromptTabNext,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.prompt_tab_navigation_enabled() {
            return;
        }

        window.focus_next(cx);
        if !self
            .prompt_tab_group_focus_handle
            .contains_focused(window, cx)
        {
            self.wrap_prompt_focus(true, window, cx);
        }
        cx.stop_propagation();
    }

    fn focus_prev_prompt_field(
        &mut self,
        _: &crate::view::PopoverPromptTabPrev,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.prompt_tab_navigation_enabled() {
            return;
        }

        window.focus_prev(cx);
        if !self
            .prompt_tab_group_focus_handle
            .contains_focused(window, cx)
        {
            self.wrap_prompt_focus(false, window, cx);
        }
        cx.stop_propagation();
    }

    pub(in super::super) fn dismiss_prompt_popover(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.popover.as_ref().is_some_and(popover_is_confirm_dialog) {
            self.close_popover(cx);
            return;
        }
        match self.popover.as_ref() {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. })
            | Some(PopoverKind::RenameBranchPrompt { .. })
            | Some(PopoverKind::StashPrompt)
            | Some(PopoverKind::CommitPrompt { .. })
            | Some(PopoverKind::StashPickerPrompt { .. })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
                ..
            }) => self.dismiss_inline_popover(window, cx),
            Some(PopoverKind::CloneRepo)
            | Some(PopoverKind::CreateTagPrompt { .. })
            | Some(PopoverKind::SquashPrompt { .. })
            | Some(PopoverKind::CheckoutRemoteBranchPrompt { .. })
            | Some(PopoverKind::PushSetUpstreamPrompt { .. })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                ..
            })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { .. }),
                ..
            })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            })
            | Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                ..
            }) => self.close_popover(cx),
            _ => {}
        }
    }

    fn dismiss_prompt(
        &mut self,
        _: &crate::view::PopoverPromptDismiss,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.prompt_tab_navigation_enabled() {
            return;
        }

        self.dismiss_prompt_popover(window, cx);
        cx.stop_propagation();
    }

    fn dismiss_inline_popover(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.save_commit_prompt_draft(cx);
        self.clear_truncated_tooltip(cx);
        self.popover = None;
        self.popover_anchor = None;
        self.clear_active_context_menu_invoker(cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_history_refs_hover_item_menu_open(false, cx);
            });
        });
        let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    fn clear_truncated_tooltip(&self, cx: &mut gpui::Context<Self>) {
        let _ = self.tooltip_host.update(cx, |host, cx| {
            host.clear_tooltip(cx);
        });
    }

    fn can_submit_create_tag(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(self.popover, Some(PopoverKind::CreateTagPrompt { .. }))
            && self
                .create_tag_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn can_submit_clone_repo(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(self.popover, Some(PopoverKind::CloneRepo))
            && self
                .clone_repo_url_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
            && self
                .clone_repo_parent_dir_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn can_submit_submodule_change_pointer(&self, cx: &mut gpui::Context<Self>) -> bool {
        matches!(
            self.popover,
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
                ..
            })
        ) && self
            .submodule_ref_input
            .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn submit_create_tag(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::CreateTagPrompt { repo_id, target }) = self.popover.clone() else {
            return;
        };

        let name = self
            .create_tag_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if name.is_empty() {
            return;
        }

        let annotated = self.create_tag_annotated;
        let message = if annotated {
            let msg = self
                .create_tag_message_input
                .read_with(cx, |input, _| input.text().trim().to_string());
            Some(msg)
        } else {
            None
        };

        self.store.dispatch(Msg::CreateTag {
            repo_id,
            name,
            target,
            message,
            annotated,
        });
        self.close_popover(cx);
    }

    fn submit_clone_repo(&mut self, cx: &mut gpui::Context<Self>) {
        if !matches!(self.popover, Some(PopoverKind::CloneRepo)) {
            return;
        }

        let url = self
            .clone_repo_url_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        let parent = self
            .clone_repo_parent_dir_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if url.is_empty() || parent.is_empty() {
            return;
        }

        let repo_name = clone_repo_name_from_url(&url);
        let dest = std::path::PathBuf::from(parent).join(repo_name);
        self.store.dispatch(Msg::CloneRepo { url, dest });
        self.close_popover(cx);
    }

    fn submit_submodule_change_pointer(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { path }),
        }) = self.popover.clone()
        else {
            return;
        };

        let reference = self
            .submodule_ref_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if reference.is_empty() {
            return;
        }

        self.store.dispatch(Msg::ChangeSubmodulePointer {
            repo_id,
            path,
            reference,
        });
        self.dismiss_inline_popover(window, cx);
    }

    fn inline_branch_picker_active(&self) -> bool {
        matches!(
            self.popover,
            Some(PopoverKind::BranchPicker { .. })
                | Some(PopoverKind::CreateBranchFromRefPrompt {
                    source_selectable: true,
                    ..
                })
                | Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                })
        )
    }

    fn handle_inline_branch_picker_escape(&mut self, cx: &mut gpui::Context<Self>) {
        match &self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. }) => {
                self.branch_picker_selected_index = None;
                if let Some(input) = &self.branch_picker_search_input {
                    let target = self.create_branch_source_target.clone();
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(target, cx);
                        cx.notify();
                    });
                }
                cx.notify();
            }
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            }) => {
                self.branch_picker_selected_index = None;
                if let Some(input) = &self.branch_picker_search_input {
                    let target = self.worktree_ref_source_target.clone();
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(target, cx);
                        cx.notify();
                    });
                }
                cx.notify();
            }
            _ => {
                self.close_popover(cx);
            }
        }
    }

    fn handle_inline_branch_picker_select(
        &mut self,
        name: String,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match &self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. }) => {
                self.create_branch_source_target = name;
                if let Some(input) = &self.branch_picker_search_input {
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(self.create_branch_source_target.clone(), cx);
                        cx.notify();
                    });
                }
                self.branch_picker_selected_index = None;
                cx.defer_in(window, |this, window, cx| {
                    if matches!(
                        this.popover,
                        Some(PopoverKind::CreateBranchFromRefPrompt { .. })
                    ) {
                        let focus = this
                            .create_branch_input
                            .read_with(cx, |input, _| input.focus_handle());
                        window.focus(&focus, cx);
                        cx.notify();
                    }
                });
                cx.notify();
            }
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            }) => {
                self.worktree_ref_source_target = name;
                if let Some(input) = &self.branch_picker_search_input {
                    let theme = self.theme;
                    input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(self.worktree_ref_source_target.clone(), cx);
                        cx.notify();
                    });
                }
                self.branch_picker_selected_index = None;
                // Hand focus to Add once the keystroke that picked the ref has
                // finished dispatching, so it cannot land on the button it just
                // moved to; `suppress_worktree_submit_after_ref_enter` covers
                // the same Enter until the next frame is on screen.
                cx.defer_in(window, |this, window, cx| {
                    if matches!(
                        this.popover,
                        Some(PopoverKind::Repo {
                            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                            ..
                        })
                    ) {
                        let focus = if this.can_submit_worktree_add(cx) {
                            this.worktree_focus.submit.clone()
                        } else {
                            this.worktree_path_input
                                .read_with(cx, |input, _| input.focus_handle())
                        };
                        window.focus(&focus, cx);
                        cx.notify();
                    }
                    cx.on_next_frame(window, |this, _window, cx| {
                        this.suppress_worktree_submit_after_ref_enter = false;
                        cx.notify();
                    });
                });
                cx.notify();
            }
            Some(PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::Delete,
            }) => {
                let is_centered = matches!(self.popover_anchor, Some(PopoverAnchor::Centered));
                let _ = self.root_view.update(cx, |root, _| {
                    root.pending_force_delete_branch_centered = is_centered;
                });
                self.store.dispatch(Msg::DeleteBranch { repo_id, name });
                self.close_popover(cx);
            }
            Some(PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::RebaseOnto,
            }) => {
                self.open_popover_centered(
                    PopoverKind::RebaseOntoConfirm {
                        repo_id,
                        onto: name,
                    },
                    window,
                    cx,
                );
            }
            _ => {
                self.store.dispatch(Msg::CheckoutBranch { repo_id, name });
                self.close_popover(cx);
            }
        }
    }

    fn can_submit_create_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.create_branch_prompt_repo_and_target().is_some()
            && self
                .create_branch_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn create_branch_prompt_repo_and_target(&self) -> Option<(RepoId, String)> {
        match &self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                source_selectable: true,
                ..
            }) => {
                let target = self.create_branch_source_target.clone();
                if target.is_empty() {
                    None
                } else {
                    Some((*repo_id, target))
                }
            }
            Some(PopoverKind::CreateBranchFromRefPrompt {
                repo_id, target, ..
            }) => Some((*repo_id, target.clone())),
            _ => None,
        }
    }

    fn submit_create_branch(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, target)) = self.create_branch_prompt_repo_and_target() else {
            return;
        };
        let name = self
            .create_branch_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if name.is_empty() {
            return;
        }

        let checkout = match self.popover {
            Some(PopoverKind::CreateBranchFromRefPrompt { .. }) => {
                self.create_branch_checkout_enabled
            }
            _ => return,
        };

        if checkout {
            self.store.dispatch(Msg::CreateBranchAndCheckout {
                repo_id,
                name,
                target,
            });
        } else {
            self.store.dispatch(Msg::CreateBranch {
                repo_id,
                name,
                target,
            });
        }
        self.dismiss_inline_popover(window, cx);
    }

    fn can_submit_rename_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(PopoverKind::RenameBranchPrompt { name, .. }) = &self.popover else {
            return false;
        };
        self.create_branch_input.read_with(cx, |input, _| {
            let new_name = input.text().trim();
            !new_name.is_empty() && new_name != name
        })
    }

    fn submit_rename_branch(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::RenameBranchPrompt { repo_id, name, .. }) = self.popover.clone()
        else {
            return;
        };
        let new_name = self
            .create_branch_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if new_name.is_empty() || new_name == name {
            return;
        }
        self.store.dispatch(Msg::RenameBranch {
            repo_id,
            old_name: name,
            new_name,
        });
        self.dismiss_inline_popover(window, cx);
    }

    fn can_submit_stash(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.active_repo_id().is_some()
            && self
                .stash_message_input
                .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    pub(super) fn submit_commit_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.can_submit_commit_prompt(cx) {
            return;
        }
        let Some(PopoverKind::CommitPrompt { repo_id }) = self.popover.clone() else {
            return;
        };
        let message = self
            .commit_prompt_message_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if message.is_empty() {
            return;
        }
        self.store.dispatch(Msg::Commit {
            repo_id,
            message,
            push_after_commit: false,
        });
        self.commit_prompt_message_drafts.remove(&repo_id);
        self.commit_prompt_message_input
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.commit_prompt_message_scroll
            .set_offset(point(px(0.0), px(0.0)));
        self.dismiss_inline_popover(window, cx);
    }

    fn save_commit_prompt_draft(&mut self, cx: &gpui::Context<Self>) {
        let Some(PopoverKind::CommitPrompt { repo_id }) = self.popover else {
            return;
        };
        let draft: SharedString = self
            .commit_prompt_message_input
            .read(cx)
            .text()
            .to_string()
            .into();
        if draft.is_empty() {
            self.commit_prompt_message_drafts.remove(&repo_id);
        } else {
            self.commit_prompt_message_drafts.insert(repo_id, draft);
        }
    }

    pub(super) fn can_submit_commit_prompt(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.active_repo().is_some_and(|repo| {
            repo.staged_status_entries()
                .is_some_and(|entries| !entries.is_empty())
                || matches!(repo.merge_commit_message, Loadable::Ready(Some(_)))
        }) && self
            .commit_prompt_message_input
            .read_with(cx, |input, _| !input.text().trim().is_empty())
    }

    fn submit_stash(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let message = self
            .stash_message_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if message.is_empty() {
            return;
        }

        self.store.dispatch(Msg::Stash {
            repo_id,
            message,
            include_untracked: true,
        });
        self.dismiss_inline_popover(window, cx);
    }

    pub(super) fn can_submit_remote_add(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.remote_name_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .remote_url_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_remote_add(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_remote_add(cx) {
            return;
        }
        let name = self
            .remote_name_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let url = self
            .remote_url_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::AddRemote { repo_id, name, url });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_remote_edit_url(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.remote_url_edit_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_remote_edit_url(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { name, kind }),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_remote_edit_url(cx) {
            return;
        }
        let url = self
            .remote_url_edit_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_push_set_upstream(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.push_upstream_branch_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_push_set_upstream(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::PushSetUpstreamPrompt { repo_id, remote }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_push_set_upstream(cx) {
            return;
        }
        let branch = self
            .push_upstream_branch_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_checkout_remote_branch(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.create_branch_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_checkout_remote_branch(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::CheckoutRemoteBranchPrompt {
            repo_id,
            remote,
            branch,
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_checkout_remote_branch(cx) {
            return;
        }
        let local_branch = self
            .create_branch_input
            .read_with(cx, |i, _| i.text().trim().to_string());

        let local_branch_exists = self
            .state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .and_then(|repo| match &repo.branches {
                Loadable::Ready(branches) => {
                    Some(branches.iter().any(|b| b.name == local_branch.as_str()))
                }
                _ => None,
            })
            .unwrap_or(false);
        if local_branch_exists {
            self.push_toast(
                components::ToastKind::Error,
                format!("Branch already exists: {local_branch}"),
                cx,
            );
            return;
        }

        self.store.dispatch(Msg::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.rebuild_diff_cache(cx);
            cx.notify();
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_worktree_add(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.worktree_path_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_worktree_add(&mut self, cx: &mut gpui::Context<Self>) {
        if self.suppress_worktree_submit_after_ref_enter {
            return;
        }
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_worktree_add(cx) {
            return;
        }
        let folder = self
            .worktree_path_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let reference = self.worktree_ref_source_target.trim().to_string();
        let reference = (!reference.is_empty()).then_some(reference);
        self.store.dispatch(Msg::AddWorktree {
            repo_id,
            path: std::path::PathBuf::from(folder),
            reference,
        });
        self.close_popover(cx);
    }

    pub(super) fn can_submit_submodule_add(&self, cx: &mut gpui::Context<Self>) -> bool {
        self.submodule_url_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .submodule_path_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(super) fn submit_submodule_add(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_submodule_add(cx) {
            return;
        }
        let url = self
            .submodule_url_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let path_text = self
            .submodule_path_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let branch = self.submodule_branch_input.read_with(cx, |i, _| {
            let text = i.text().trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        });
        let name = self.submodule_name_input.read_with(cx, |i, _| {
            let text = i.text().trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        });
        let force = self.submodule_force_enabled;
        self.store.dispatch(Msg::AddSubmodule {
            repo_id,
            url,
            path: std::path::PathBuf::from(path_text),
            branch,
            name,
            force,
        });
        self.close_popover(cx);
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Point(anchor), window, cx);
    }

    pub(in super::super) fn open_popover_centered(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Centered, window, cx);
    }

    pub(in super::super) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_popover(kind, PopoverAnchor::Bounds(anchor_bounds), window, cx);
    }

    fn request_lazy_popover_repo_data(&self, kind: &PopoverKind) {
        let repo_id = match kind {
            PopoverKind::TagMenu { repo_id, .. } | PopoverKind::TagRefMenu { repo_id, .. } => {
                Some(*repo_id)
            }
            PopoverKind::PreviousCommitMessagesMenu { repo_id } => Some(*repo_id),
            PopoverKind::CommitOptionsMenu { repo_id } => Some(*repo_id),
            PopoverKind::BranchPicker { .. } => self.state.active_repo,
            _ => None,
        };
        let Some(repo_id) = repo_id else {
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return;
        };

        if matches!(kind, PopoverKind::BranchPicker { .. }) {
            // Decorates the checkout picker's rows; load once, retry on error.
            if matches!(repo.ref_metadata, Loadable::NotLoaded | Loadable::Error(_)) {
                self.store.dispatch(Msg::LoadRefMetadata { repo_id });
            }
            // Remote branches arrive with the repo's normal refresh; the picker
            // just omits the Remote section until they do.
            return;
        }

        if matches!(
            kind,
            PopoverKind::PreviousCommitMessagesMenu { .. } | PopoverKind::CommitOptionsMenu { .. }
        ) {
            if matches!(
                repo.recent_commit_messages,
                Loadable::NotLoaded | Loadable::Error(_)
            ) {
                self.store
                    .dispatch(Msg::LoadRecentCommitMessages { repo_id, limit: 10 });
            }
            return;
        }

        if matches!(repo.tags, Loadable::NotLoaded | Loadable::Error(_)) {
            self.store.dispatch(Msg::LoadTags { repo_id });
        }
        if matches!(repo.remote_tags, Loadable::NotLoaded | Loadable::Error(_)) {
            self.store.dispatch(Msg::LoadRemoteTags { repo_id });
        }
    }

    fn open_popover(
        &mut self,
        kind: PopoverKind,
        anchor: PopoverAnchor,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.save_commit_prompt_draft(cx);
        self.clear_truncated_tooltip(cx);
        // The anchor stays hovered behind the opened surface; keep its
        // tooltip from re-showing on top of the popover.
        crate::view::tooltip::set_tooltips_suppressed_by_overlay(true, cx);
        self.request_lazy_popover_repo_data(&kind);
        if matches!(&kind, PopoverKind::CherryPickCommitConfirm { .. }) {
            self.cherry_pick_mainline = None;
        }
        self.menu_invoker_focus =
            if matches!(&kind, PopoverKind::AppMenu | PopoverKind::AddRepoMenu) {
                window.focused(cx)
            } else {
                None
            };
        let is_context_menu = popover_is_context_menu(&kind);
        let keep_active_invoker = is_context_menu
            || matches!(
                &kind,
                PopoverKind::CreateBranchFromRefPrompt { .. }
                    | PopoverKind::RenameBranchPrompt { .. }
                    | PopoverKind::StashPrompt
                    | PopoverKind::CommitPrompt { .. }
                    | PopoverKind::StashPickerPrompt { .. }
                    // Action-bar badges stay lit while their picker is open.
                    // Scoped to Checkout: the Delete picker is opened from the
                    // sidebar context menu, whose invoker must still be cleared.
                    | PopoverKind::BranchPicker {
                        purpose: BranchPickerPurpose::Checkout,
                    }
                    | PopoverKind::Repo {
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                        ..
                    }
            );
        if !keep_active_invoker {
            self.clear_active_context_menu_invoker(cx);
        }

        self.popover_anchor = Some(anchor);
        self.context_menu_selected_ix = None;
        self.repo_picker_selected_index = None;
        // Belongs with the reset above, not with the RepoPicker arm below: every
        // popover kind draws `row_menu_layer`, so a menu left over from a closed
        // picker would spread its occluding scrim over an unrelated popover.
        self.repo_picker_row_menu = None;
        self.branch_picker_selected_index = None;
        self.worktree_picker_selected_index = None;
        self.workspace_picker_selected_index = None;
        self.submodule_picker_selected_index = None;
        self.file_history_selected_index = None;
        // Rows are keyed by the data they were built from, so a stale slot can
        // only be reused when that data is unchanged. Dropping them on open still
        // keeps the memory from outliving the picker that needed it.
        self.branch_picker_rows_cache.clear();
        self.workspace_picker_rows_cache.clear();
        self.repo_picker_rows_cache.clear();
        self.stash_picker_rows_cache.clear();
        self.file_history_rows_cache.clear();
        self.submodule_picker_rows_cache.clear();
        self.worktree_picker_rows_cache.clear();
        self.branch_ref_rows_cache.clear();
        if is_context_menu {
            self.popover = Some(kind);
            self.context_menu_selected_ix = self
                .popover
                .as_ref()
                .and_then(|kind| self.context_menu_model(kind, cx))
                .and_then(|m| m.first_selectable());
            window.focus(&self.context_menu_focus_handle, cx);
        } else {
            match &kind {
                PopoverKind::RepoPicker => {
                    let ui_session = session::load();
                    self.repo_picker_sort = repo_picker::sort_from_session(&ui_session);
                    self.cached_recent_repos = ui_session.recent_repos;
                    self.cached_pinned_repos = ui_session.pinned_repos;
                    self.cached_collapsed_picker_sections =
                        ui_session.repo_picker_collapsed_sections;
                    self.repo_picker_sort_menu_open = false;
                    let _ = self.ensure_repo_picker_search_input(window, cx);
                }
                PopoverKind::BranchPicker { .. } => {
                    let _ = self.ensure_branch_picker_search_input(window, cx);
                }
                PopoverKind::CreateBranchFromRefPrompt {
                    source_selectable,
                    target,
                    ..
                } => {
                    let theme = self.theme;
                    self.create_branch_checkout_enabled = true;
                    self.create_branch_source_target = target.clone();
                    if *source_selectable {
                        let _ = self.ensure_branch_picker_search_input(window, cx);
                        if let Some(input) = &self.branch_picker_search_input {
                            input.update(cx, |input, cx| {
                                input.set_text(target.clone(), cx);
                            });
                        }
                    }
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::RenameBranchPrompt { name, .. } => {
                    let theme = self.theme;
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(name.clone(), cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |input, _| input.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::CheckoutRemoteBranchPrompt { branch, .. } => {
                    let theme = self.theme;
                    self.create_branch_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(branch.clone(), cx);
                        cx.notify();
                    });
                    let focus = self
                        .create_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::StashPrompt => {
                    let theme = self.theme;
                    self.stash_message_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .stash_message_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::CommitPrompt { repo_id } => {
                    let theme = self.theme;
                    let draft = self
                        .commit_prompt_message_drafts
                        .get(repo_id)
                        .cloned()
                        .unwrap_or_default();
                    self.commit_prompt_message_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(draft.to_string(), cx);
                        cx.notify();
                    });
                    self.commit_prompt_message_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                    let focus = self
                        .commit_prompt_message_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::StashPickerPrompt { .. } => {
                    let _ = self.ensure_stash_picker_search_input(window, cx);
                    self.stash_picker_prompt_selected_index = Some(0);
                }
                PopoverKind::CloneRepo => {
                    let theme = self.theme;
                    let url_text = self
                        .clone_repo_url_input
                        .read_with(cx, |i, _| i.text().to_string());
                    let parent_text = self
                        .clone_repo_parent_dir_input
                        .read_with(cx, |i, _| i.text().to_string());
                    self.clone_repo_url_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(url_text, cx);
                        cx.notify();
                    });
                    self.clone_repo_parent_dir_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(parent_text, cx);
                        cx.notify();
                    });
                    let focus = self
                        .clone_repo_url_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::SquashPrompt { .. } => {
                    let theme = self.theme;
                    self.squash_prompt_prefilled_range = None;
                    self.squash_message_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.squash_description_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    // The preview may already be Ready (e.g. reopening the same
                    // range); prefill immediately rather than waiting for the
                    // next model update.
                    self.sync_squash_prompt_prefill(cx);
                    let focus = self
                        .squash_message_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::CreateTagPrompt { .. } => {
                    let theme = self.theme;
                    self.create_tag_annotated =
                        matches!(self.state.default_tag_type, DefaultTagType::Annotated);
                    self.create_tag_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.create_tag_message_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self.create_tag_input.read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    self.remote_name_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.remote_url_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .remote_name_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    repo_id,
                    kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { name, .. }),
                } => {
                    let theme = self.theme;
                    let text = self
                        .state
                        .repos
                        .iter()
                        .find(|r| r.id == *repo_id)
                        .and_then(|r| match &r.remotes {
                            Loadable::Ready(remotes) => remotes
                                .iter()
                                .find(|remote| remote.name.as_str() == name.as_str())
                                .and_then(|remote| remote.url.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    self.remote_url_edit_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(text, cx);
                        cx.notify();
                    });
                    let focus = self
                        .remote_url_edit_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    let (path_prefill, reference_prefill) =
                        self.pending_worktree_add_prefill.take().unwrap_or_default();
                    self.worktree_path_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(path_prefill, cx);
                        cx.notify();
                    });
                    self.worktree_ref_source_target = reference_prefill.clone();
                    self.suppress_worktree_submit_after_ref_enter = false;
                    let ref_input = self.ensure_branch_picker_search_input(window, cx);
                    // `ensure_*` blanks the input, so the prefilled ref has to be
                    // written back afterwards or the box would read empty while
                    // submit still used the reference.
                    if !reference_prefill.is_empty() {
                        ref_input.update(cx, |input, cx| {
                            input.set_text(reference_prefill, cx);
                            cx.notify();
                        });
                    }
                    let focus = self
                        .worktree_path_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    repo_id,
                    kind:
                        RepoPopoverKind::Worktree(
                            WorktreePopoverKind::OpenPicker | WorktreePopoverKind::RemovePicker,
                        ),
                } => {
                    let _ = self.ensure_worktree_picker_search_input(window, cx);
                    self.store
                        .dispatch(Msg::LoadWorktrees { repo_id: *repo_id });
                }
                PopoverKind::Repo {
                    repo_id,
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                } => {
                    let _ = self.ensure_workspace_picker_search_input(window, cx);
                    self.store
                        .dispatch(Msg::LoadWorktrees { repo_id: *repo_id });
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::AddPrompt),
                    ..
                } => {
                    let theme = self.theme;
                    self.submodule_add_advanced_expanded = false;
                    self.submodule_force_enabled = false;
                    self.submodule_url_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.submodule_path_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.submodule_branch_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    self.submodule_name_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .submodule_url_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind:
                        RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt { .. }),
                    ..
                } => {
                    let theme = self.theme;
                    self.submodule_ref_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text("", cx);
                        cx.notify();
                    });
                    let focus = self
                        .submodule_ref_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::Repo {
                    kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::TrustConfirm),
                    ..
                } => {}
                PopoverKind::Repo {
                    repo_id,
                    kind:
                        RepoPopoverKind::Submodule(
                            SubmodulePopoverKind::OpenPicker | SubmodulePopoverKind::RemovePicker,
                        ),
                } => {
                    let _ = self.ensure_submodule_picker_search_input(window, cx);
                    self.store
                        .dispatch(Msg::LoadSubmodules { repo_id: *repo_id });
                }
                PopoverKind::FileHistory { repo_id, path } => {
                    self.ensure_file_history_search_input(window, cx);
                    self.store.dispatch(Msg::LoadFileHistory {
                        repo_id: *repo_id,
                        path: path.clone(),
                        limit: 200,
                    });
                }
                PopoverKind::PushSetUpstreamPrompt { repo_id, .. } => {
                    let theme = self.theme;
                    let current_text = self
                        .push_upstream_branch_input
                        .read_with(cx, |i, _| i.text().to_string());
                    let text = self
                        .state
                        .repos
                        .iter()
                        .find(|r| r.id == *repo_id)
                        .and_then(|repo| match &repo.head_branch {
                            Loadable::Ready(head) if !head.is_empty() => Some(head.clone()),
                            _ => None,
                        })
                        .unwrap_or(current_text);
                    self.push_upstream_branch_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(text, cx);
                        cx.notify();
                    });
                    let focus = self
                        .push_upstream_branch_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::RebaseReword {
                    ix: _,
                    original_action: _,
                    original_message,
                } => {
                    let theme = self.theme;
                    let (subject, body) = original_message
                        .split_once("\n\n")
                        .map(|(s, b)| (s.to_owned(), b.to_owned()))
                        .unwrap_or_else(|| (original_message.clone(), String::new()));
                    self.rebase_reword_input.update(cx, |input, cx| {
                        input.clear_transient_key_presses();
                        input.set_theme(theme, cx);
                        input.set_text(subject, cx);
                        cx.notify();
                    });
                    self.rebase_reword_description_input
                        .update(cx, |input, cx| {
                            input.clear_transient_key_presses();
                            input.set_theme(theme, cx);
                            input.set_text(body, cx);
                            cx.notify();
                        });
                    self.rebase_reword_description_scroll
                        .set_offset(point(px(0.0), px(0.0)));
                    let focus = self
                        .rebase_reword_input
                        .read_with(cx, |i, _| i.focus_handle());
                    window.focus(&focus, cx);
                }
                PopoverKind::RebaseOntoConfirm { .. } => {
                    // Focus the primary (Rebase) button so Enter confirms and
                    // Tab/Esc still reach Cancel.
                    window.focus(&self.rebase_onto_submit_focus_handle, cx);
                }
                k if popover_is_confirm_dialog(k) => {
                    window.focus(&self.prompt_tab_group_focus_handle, cx);
                }
                _ => {}
            }
            self.popover = Some(kind);
        }
        if let Some(popover) = self.popover.as_ref() {
            self.notify_fingerprint = fingerprint::notify_fingerprint(&self.state, popover);
        }
        self.sync_titlebar_app_menu_state(cx);
        cx.notify();
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn set_pinned_branches(
        &mut self,
        pinned: std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.pinned_branches_by_repo == pinned {
            return;
        }
        self.pinned_branches_by_repo = pinned;
        cx.notify();
    }

    pub(in super::super) fn is_branch_pinned(
        &self,
        repo_id: RepoId,
        section: BranchSection,
        name: &str,
    ) -> bool {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        let key = crate::view::branch_sidebar::branch_pin_storage_key(section, name);
        self.pinned_branches_by_repo
            .get(&repo.spec.workdir)
            .is_some_and(|items| items.contains(&key))
    }

    pub(in super::super) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_date_time_format(next, cx));
        self.sync_details_pane_date_settings(cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(in super::super) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_timezone(next, cx));
        self.sync_details_pane_date_settings(cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(in super::super) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.show_timezone == enabled {
            return;
        }
        self.show_timezone = enabled;
        self.main_pane
            .update(cx, |pane, cx| pane.set_show_timezone(enabled, cx));
        self.sync_details_pane_date_settings(cx);
        self.schedule_ui_settings_persist(cx);
    }

    fn sync_details_pane_date_settings(&mut self, cx: &mut gpui::Context<Self>) {
        let (format, timezone, show_timezone) =
            (self.date_time_format, self.timezone, self.show_timezone);
        self.details_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx);
        });
    }

    pub(in super::super) fn set_theme_mode(
        &mut self,
        next: ThemeMode,
        appearance: gpui::WindowAppearance,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == next {
            return;
        }

        self.theme_mode = next.clone();
        self.set_theme(next.resolve_theme(appearance), cx);
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.set_theme_mode(next.clone(), appearance, cx);
            });
        });
    }

    fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let mode = self.theme_mode.clone();
        let fmt = self.date_time_format;
        let tz = self.timezone;
        let show_tz = self.show_timezone;
        let root_view = self.root_view.clone();
        cx.spawn(
            async move |_host: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let _ = root_view.update(cx, |root, cx| {
                    root.theme_mode = mode;
                    root.date_time_format = fmt;
                    root.timezone = tz;
                    root.show_timezone = show_tz;
                    root.schedule_ui_settings_persist(cx);
                });
            },
        )
        .detach();
    }

    pub(in super::super) fn sync_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        if matches!(self.popover, Some(PopoverKind::ChangeTrackingSettings)) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_commit_push_after_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_push_after_enabled == enabled {
            return;
        }

        self.commit_push_after_enabled = enabled;
        if matches!(self.popover, Some(PopoverKind::CommitOptionsMenu { .. })) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_commit_amend_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_amend_enabled == enabled {
            return;
        }

        self.commit_amend_enabled = enabled;
        if matches!(self.popover, Some(PopoverKind::CommitOptionsMenu { .. })) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_diff_content_mode(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_content_mode == next {
            return;
        }

        self.diff_content_mode = next;
        if matches!(self.popover, Some(PopoverKind::DiffContentModeSettings)) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_whitespace_mode == next {
            return;
        }

        self.diff_whitespace_mode = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_diff_reveal_whitespace_chars(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_reveal_whitespace_chars == next {
            return;
        }

        self.diff_reveal_whitespace_chars = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_diff_word_wrap(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap == next {
            return;
        }

        self.diff_word_wrap = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in super::super) fn sync_diff_show_line_numbers(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_show_line_numbers == next {
            return;
        }

        self.diff_show_line_numbers = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn install_linux_desktop_integration(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.install_linux_desktop_integration(cx);
        });
    }

    fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.push_toast(kind, message, cx);
        });
    }
}

impl Render for PopoverHost {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(kind) = self.popover.clone() else {
            return div().into_any_element();
        };

        let history_refs_menu_active = self.history_refs_menu_active(cx);
        let close = cx.listener(|this, _e: &MouseDownEvent, window, cx| {
            this.close_popover_and_restore_focus(window, cx);
        });

        let popover = self.popover_view(kind, window, cx).into_any_element();
        let is_centered = matches!(self.popover_anchor, Some(PopoverAnchor::Centered));
        let mut layer = div()
            .id("popover_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full();
        if !history_refs_menu_active && !is_centered {
            let scrim = div()
                .id("popover_scrim")
                .debug_selector(|| "repo_popover_close".to_string())
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(gpui::rgba(0x00000000))
                .occlude()
                .on_any_mouse_down(close);
            layer = layer.child(scrim);
        }
        layer = layer.child(popover);
        // Painted after the popover, so it hit-tests above the picker it floats
        // over and its own scrim intercepts the click that would otherwise
        // reach `popover_scrim` and close the whole picker.
        if let Some(row_menu) = repo_picker::row_menu_layer(self, window, cx) {
            layer = layer.child(row_menu);
        }
        layer.into_any_element()
    }
}
impl PopoverHost {
    pub(in super::super) fn popover_view(
        &mut self,
        kind: PopoverKind,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale = popover_ui_scale(cx);
        let ui_scale_percent = ui_scale.percent();
        let scaled_px = |value: f32| popover_scaled_px(value, ui_scale);
        let anchor_source = self
            .popover_anchor
            .clone()
            .unwrap_or_else(|| PopoverAnchor::Point(point(px(64.0), px(64.0))));
        let anchor_is_bounds = matches!(&anchor_source, PopoverAnchor::Bounds(_));
        let window_bounds = window.window_bounds().get_bounds();
        let window_w = window_bounds.size.width;
        let window_h = window_bounds.size.height;
        let margin_x = scaled_px(16.0);
        let margin_y = scaled_px(16.0);

        let is_app_menu = matches!(&kind, PopoverKind::AppMenu);
        let is_context_menu = popover_is_context_menu(&kind);
        let mut anchor_corner = popover_anchor_corner(&kind);

        let anchor_for_corner = |corner: Anchor| match &anchor_source {
            PopoverAnchor::Point(point) => *point,
            PopoverAnchor::Bounds(bounds) => match corner {
                Anchor::TopRight => bounds.bottom_right(),
                Anchor::BottomLeft => bounds.origin,
                Anchor::BottomRight => bounds.top_right(),
                _ => bounds.bottom_left(),
            },
            PopoverAnchor::Centered => point(px(0.0), px(0.0)),
        };

        // Some popovers have large minimum widths. If the anchor is close to the edge, the popover
        // can end up constrained to a very narrow width (making inputs unusably small). Prefer the
        // side with more horizontal space in those cases.
        let mut anchor = anchor_for_corner(anchor_corner);
        let preferred_width = popover_preferred_anchor_width(&kind, ui_scale);
        let space_left = (anchor.x - margin_x).max(px(0.0));
        let space_right = (window_w - margin_x - anchor.x).max(px(0.0));
        anchor_corner =
            choose_popover_anchor_corner(anchor_corner, space_left, space_right, preferred_width);
        anchor = anchor_for_corner(anchor_corner);

        let panel = match kind {
            PopoverKind::RepoPicker => repo_picker::panel(self, cx),
            PopoverKind::BranchPicker { .. } => branch_picker::panel(self, cx),
            PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                target,
                source_selectable,
            } => create_branch_from_ref_prompt::panel(
                self,
                repo_id,
                target,
                source_selectable,
                window,
                cx,
            ),
            PopoverKind::RenameBranchPrompt {
                repo_id,
                name,
                is_current_branch,
            } => rename_branch_prompt::panel(self, repo_id, name, is_current_branch, cx),
            PopoverKind::CheckoutRemoteBranchPrompt {
                repo_id,
                remote,
                branch,
            } => checkout_remote_branch_prompt::panel(self, repo_id, remote, branch, cx),
            PopoverKind::StashPrompt => stash_prompt::panel(self, cx),
            PopoverKind::CommitPrompt { repo_id } => commit_prompt::panel(self, repo_id, cx),
            PopoverKind::StashPickerPrompt { repo_id, purpose } => {
                stash_picker_prompt::panel(self, repo_id, purpose, cx)
            }
            PopoverKind::StashDropConfirm {
                repo_id,
                index,
                message,
            } => stash_drop_confirm::panel(self, repo_id, index, message, cx),
            PopoverKind::CloneRepo => clone_repo::panel(self, cx),
            PopoverKind::ResetPrompt {
                repo_id,
                target,
                mode,
            } => reset_prompt::panel(self, repo_id, target, mode, cx),
            PopoverKind::SquashPrompt { repo_id } => squash_prompt::panel(self, repo_id, cx),
            PopoverKind::CreateTagPrompt { repo_id, target } => {
                create_tag_prompt::panel(self, repo_id, target, cx)
            }
            PopoverKind::Repo { repo_id, kind } => match kind {
                RepoPopoverKind::Remote(remote_kind) => match remote_kind {
                    RemotePopoverKind::AddPrompt => remote_add_prompt::panel(self, repo_id, cx),
                    RemotePopoverKind::EditUrlPrompt { name, kind } => {
                        remote_edit_url_prompt::panel(self, repo_id, name, kind, cx)
                    }
                    RemotePopoverKind::RemoveConfirm { name } => {
                        remote_remove_confirm::panel(self, repo_id, name, cx)
                    }
                    RemotePopoverKind::DeleteBranchConfirm { remote, branch } => {
                        delete_remote_branch_confirm::panel(self, repo_id, remote, branch, cx)
                    }
                    RemotePopoverKind::Menu { name } => self.context_menu_view(
                        PopoverKind::remote(repo_id, RemotePopoverKind::Menu { name }),
                        cx,
                    ),
                },
                RepoPopoverKind::Worktree(worktree_kind) => match worktree_kind {
                    WorktreePopoverKind::SectionMenu => self.context_menu_view(
                        PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu),
                        cx,
                    ),
                    WorktreePopoverKind::Menu { path, branch } => self.context_menu_view(
                        PopoverKind::worktree(repo_id, WorktreePopoverKind::Menu { path, branch }),
                        cx,
                    ),
                    WorktreePopoverKind::AddPrompt => {
                        worktree_add_prompt::panel(self, repo_id, window, cx)
                    }
                    WorktreePopoverKind::OpenPicker => {
                        worktree_picker::panel(self, repo_id, false, cx)
                    }
                    WorktreePopoverKind::RemovePicker => {
                        worktree_picker::panel(self, repo_id, true, cx)
                    }
                    WorktreePopoverKind::BadgePicker => workspace_picker::panel(self, repo_id, cx),
                    WorktreePopoverKind::RemoveConfirm { path, branch } => {
                        worktree_remove_confirm::panel(self, repo_id, path, branch, cx)
                    }
                },
                RepoPopoverKind::Submodule(submodule_kind) => match submodule_kind {
                    SubmodulePopoverKind::SectionMenu => self.context_menu_view(
                        PopoverKind::submodule(repo_id, SubmodulePopoverKind::SectionMenu),
                        cx,
                    ),
                    SubmodulePopoverKind::Menu { path } => self.context_menu_view(
                        PopoverKind::submodule(repo_id, SubmodulePopoverKind::Menu { path }),
                        cx,
                    ),
                    SubmodulePopoverKind::AddPrompt => {
                        submodule_add_prompt::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::ChangePointerPrompt { path } => {
                        submodule_change_pointer_prompt::panel(self, repo_id, &path, cx)
                    }
                    SubmodulePopoverKind::TrustConfirm => {
                        submodule_trust_confirm::panel(self, repo_id, cx)
                    }
                    SubmodulePopoverKind::OpenPicker => {
                        submodule_picker::panel(self, repo_id, false, cx)
                    }
                    SubmodulePopoverKind::RemovePicker => {
                        submodule_picker::panel(self, repo_id, true, cx)
                    }
                    SubmodulePopoverKind::RemoveConfirm { path } => {
                        submodule_remove_confirm::panel(self, repo_id, path, cx)
                    }
                },
            },
            PopoverKind::FileHistory { repo_id, path } => {
                file_history::panel(self, repo_id, path, cx)
            }
            PopoverKind::PushSetUpstreamPrompt { repo_id, remote } => {
                push_set_upstream_prompt::panel(self, repo_id, remote, cx)
            }
            PopoverKind::ForcePushConfirm { repo_id } => {
                force_push_confirm::panel(self, repo_id, cx)
            }
            PopoverKind::CherryPickCommitConfirm { repo_id, commit_id } => {
                cherry_pick_commit_confirm::panel(self, repo_id, commit_id, cx)
            }
            PopoverKind::MergeAbortConfirm { repo_id } => {
                merge_abort_confirm::panel(self, repo_id, cx)
            }
            PopoverKind::ForceDeleteBranchConfirm { repo_id, name } => {
                force_delete_branch_confirm::panel(self, repo_id, name, cx)
            }
            PopoverKind::ForceRemoveWorktreeConfirm {
                repo_id,
                path,
                branch,
            } => force_remove_worktree_confirm::panel(self, repo_id, path, branch, cx),
            PopoverKind::DiscardChangesConfirm {
                repo_id,
                area,
                path,
            } => discard_changes_confirm::panel(self, repo_id, area, path.clone(), cx),
            PopoverKind::StageConflictMarkersConfirm {
                repo_id,
                paths,
                unresolved,
                clear_selection,
            } => stage_conflict_markers_confirm::panel(
                self,
                repo_id,
                paths.clone(),
                unresolved.clone(),
                clear_selection,
                cx,
            ),
            PopoverKind::PullReconcilePrompt { repo_id } => {
                pull_reconcile_prompt::panel(self, repo_id, cx)
            }
            PopoverKind::DiffActionMenu => self.context_menu_view(PopoverKind::DiffActionMenu, cx),
            PopoverKind::MarkdownLinkMenu { url } => {
                self.context_menu_view(PopoverKind::MarkdownLinkMenu { url }, cx)
            }
            PopoverKind::MergetoolSettingsMenu => {
                self.context_menu_view(PopoverKind::MergetoolSettingsMenu, cx)
            }
            PopoverKind::TerminalMenu { repo_id, context } => {
                self.context_menu_view(PopoverKind::TerminalMenu { repo_id, context }, cx)
            }
            PopoverKind::HistoryBranchFilter { repo_id } => {
                self.context_menu_view(PopoverKind::HistoryBranchFilter { repo_id }, cx)
            }
            PopoverKind::DiffContentModeSettings => {
                self.context_menu_view(PopoverKind::DiffContentModeSettings, cx)
            }
            PopoverKind::ChangeTrackingSettings => {
                self.context_menu_view(PopoverKind::ChangeTrackingSettings, cx)
            }
            PopoverKind::UiScalePicker => self.context_menu_view(PopoverKind::UiScalePicker, cx),
            PopoverKind::PullPicker => self.context_menu_view(PopoverKind::PullPicker, cx),
            PopoverKind::PushPicker => self.context_menu_view(PopoverKind::PushPicker, cx),
            PopoverKind::CommitOptionsMenu { repo_id } => {
                self.context_menu_view(PopoverKind::CommitOptionsMenu { repo_id }, cx)
            }
            PopoverKind::PreviousCommitMessagesMenu { repo_id } => {
                self.context_menu_view(PopoverKind::PreviousCommitMessagesMenu { repo_id }, cx)
            }
            PopoverKind::RepoTabMenu { repo_id } => {
                self.context_menu_view(PopoverKind::RepoTabMenu { repo_id }, cx)
            }
            PopoverKind::CommitMenu { repo_id, commit_id } => {
                self.context_menu_view(PopoverKind::CommitMenu { repo_id, commit_id }, cx)
            }
            PopoverKind::TagMenu { repo_id, commit_id } => {
                self.context_menu_view(PopoverKind::TagMenu { repo_id, commit_id }, cx)
            }
            PopoverKind::TagRefMenu {
                repo_id,
                commit_id,
                name,
            } => self.context_menu_view(
                PopoverKind::TagRefMenu {
                    repo_id,
                    commit_id,
                    name,
                },
                cx,
            ),
            PopoverKind::DiffHunkMenu { repo_id, src_ix } => {
                self.context_menu_view(PopoverKind::DiffHunkMenu { repo_id, src_ix }, cx)
            }
            PopoverKind::DiffEditorMenu {
                repo_id,
                area,
                path,
                hunk_patch,
                hunks_count,
                lines_patch,
                discard_lines_patch,
                lines_count,
                copy_text,
                copy_target,
            } => self.context_menu_view(
                PopoverKind::DiffEditorMenu {
                    repo_id,
                    area,
                    path,
                    hunk_patch,
                    hunks_count,
                    lines_patch,
                    discard_lines_patch,
                    lines_count,
                    copy_text,
                    copy_target,
                },
                cx,
            ),
            PopoverKind::ConflictResolverInputRowMenu {
                line_label,
                line_target,
                chunk_label,
                chunk_target,
            } => self.context_menu_view(
                PopoverKind::ConflictResolverInputRowMenu {
                    line_label,
                    line_target,
                    chunk_label,
                    chunk_target,
                },
                cx,
            ),
            PopoverKind::ConflictResolverChunkMenu {
                conflict_ix,
                has_base,
                is_three_way,
                selected_choices,
                output_line_ix,
                split_selection_rows,
                join_previous_region,
                join_next_region,
                alignment_marked_columns,
                has_manual_alignments,
                output_is_protected,
            } => self.context_menu_view(
                PopoverKind::ConflictResolverChunkMenu {
                    conflict_ix,
                    has_base,
                    is_three_way,
                    selected_choices,
                    output_line_ix,
                    split_selection_rows,
                    join_previous_region,
                    join_next_region,
                    alignment_marked_columns,
                    has_manual_alignments,
                    output_is_protected,
                },
                cx,
            ),
            PopoverKind::ConflictResolverOutputMenu {
                cursor_line,
                selected_text,
                has_source_a,
                has_source_b,
                has_source_c,
                is_three_way,
            } => self.context_menu_view(
                PopoverKind::ConflictResolverOutputMenu {
                    cursor_line,
                    selected_text,
                    has_source_a,
                    has_source_b,
                    has_source_c,
                    is_three_way,
                },
                cx,
            ),
            PopoverKind::StatusFileMenu {
                repo_id,
                area,
                path,
            } => self.context_menu_view(
                PopoverKind::StatusFileMenu {
                    repo_id,
                    area,
                    path,
                },
                cx,
            ),
            PopoverKind::BranchMenu {
                repo_id,
                section,
                name,
            } => self.context_menu_view(
                PopoverKind::BranchMenu {
                    repo_id,
                    section,
                    name,
                },
                cx,
            ),
            PopoverKind::BranchSectionMenu { repo_id, section } => {
                self.context_menu_view(PopoverKind::BranchSectionMenu { repo_id, section }, cx)
            }
            PopoverKind::StashMenu {
                repo_id,
                index,
                message,
            } => self.context_menu_view(
                PopoverKind::StashMenu {
                    repo_id,
                    index,
                    message,
                },
                cx,
            ),
            PopoverKind::CommitFileMenu {
                repo_id,
                commit_id,
                path,
            } => self.context_menu_view(
                PopoverKind::CommitFileMenu {
                    repo_id,
                    commit_id,
                    path,
                },
                cx,
            ),
            PopoverKind::FileBrowserFileMenu { repo_id, path } => {
                self.context_menu_view(PopoverKind::FileBrowserFileMenu { repo_id, path }, cx)
            }
            PopoverKind::BrowseHistoryMenu { repo_id } => {
                self.context_menu_view(PopoverKind::BrowseHistoryMenu { repo_id }, cx)
            }
            PopoverKind::SubmoduleInnerDiffMenu {
                repo_id,
                submodule_repo_path,
                target,
            } => self.context_menu_view(
                PopoverKind::SubmoduleInnerDiffMenu {
                    repo_id,
                    submodule_repo_path,
                    target,
                },
                cx,
            ),
            kind @ (PopoverKind::AppMenu | PopoverKind::AddRepoMenu) => {
                self.context_menu_view(kind, cx)
            }
            PopoverKind::RebaseOntoConfirm { repo_id, onto } => {
                rebase_onto_confirm::panel(self, repo_id, onto, cx)
            }
            PopoverKind::InteractiveRebaseActionMenu { .. }
            | PopoverKind::InteractiveRebaseAutosquashMenu => {
                self.context_menu_view(kind.clone(), cx)
            }
            PopoverKind::RebaseReword {
                ix,
                original_action,
                original_message: _,
            } => {
                let theme = self.theme;
                let submit_button_id = "reword_save";
                let main_pane = self.main_pane.clone();
                let submit = cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                    let subject = this
                        .rebase_reword_input
                        .read_with(cx, |input, _| input.text().to_string());
                    let body = this
                        .rebase_reword_description_input
                        .read_with(cx, |input, _| input.text().to_string());
                    let new_message = if body.trim().is_empty() {
                        subject.clone()
                    } else {
                        format!("{subject}\n\n{body}")
                    };
                    main_pane.update(cx, |pane, cx| {
                        if subject.is_empty() {
                            // Empty subject → discard any previous override and revert
                            // the action. Use set_rebase_action so side-effects
                            // (squash-target cleanup, notify) are handled consistently.
                            if let Some(entry) = pane
                                .active_irebase_mut()
                                .and_then(|st| st.entries.get_mut(ix))
                            {
                                entry.new_message = None;
                            }
                            pane.set_rebase_action(ix, original_action, cx);
                        } else if let Some(entry) = pane
                            .active_irebase_mut()
                            .and_then(|st| st.entries.get_mut(ix))
                        {
                            entry.action = InteractiveRebaseAction::Reword;
                            entry.new_message = Some(new_message);
                            cx.notify();
                        }
                    });
                    this.close_popover_and_restore_focus(window, cx);
                });
                let cancel = cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.set_rebase_action(ix, original_action, cx);
                    });
                    this.close_popover_and_restore_focus(window, cx);
                });

                div()
                    .flex()
                    .flex_col()
                    .w(scaled_px(440.0))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("Reword commit message"),
                    )
                    .child(div().border_t_1().border_color(theme.colors.border))
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("Commit message"),
                            )
                            .child(self.rebase_reword_input.clone()),
                    )
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .pb_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("Description"),
                            )
                            .child(
                                components::ScrollContainer::vertical(
                                    "rebase_reword_description_scroll_surface",
                                    "rebase_reword_description_scrollbar",
                                    self.rebase_reword_description_scroll.clone(),
                                    scaled_px(180.0),
                                )
                                .debug_selector("rebase_reword_description_scroll_surface")
                                .render(theme, self.rebase_reword_description_input.clone()),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child(
                                "Clear the message and save to keep the original commit message.",
                            ),
                    )
                    .child(div().border_t_1().border_color(theme.colors.border))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                components::Button::new("reword_cancel", "Cancel")
                                    .separated_end_slot(hotkey_hint(
                                        theme,
                                        "reword_cancel_hint",
                                        "Esc",
                                    ))
                                    .style(components::ButtonStyle::Outlined)
                                    .render(theme, ui_scale_percent)
                                    .on_click(cancel),
                            )
                            .child(
                                components::Button::new(submit_button_id, "Save message")
                                    .style(components::ButtonStyle::Filled)
                                    .render(theme, ui_scale_percent)
                                    .on_click(submit),
                            ),
                    )
            }
            PopoverKind::TerminalShutdownConfirm(prompt) => {
                terminal_shutdown_confirm::panel(self, prompt, cx)
            }
            PopoverKind::UnsavedFileEditsConfirm(prompt) => {
                unsaved_file_edits_confirm::panel(self, prompt, cx)
            }
        };

        let is_right = matches!(anchor_corner, Anchor::TopRight | Anchor::BottomRight);
        let popover_border_color = theme.colors.border;
        let gap_y = if is_app_menu {
            crate::view::chrome::title_bar_height(ui_scale_percent)
        } else if anchor_is_bounds {
            px(1.0)
        } else if is_right {
            scaled_px(10.0)
        } else {
            scaled_px(8.0)
        };

        let mut context_menu_max_panel_h: Option<Pixels> = None;
        if is_context_menu {
            let (below_anchor_y, above_anchor_y) = match &anchor_source {
                PopoverAnchor::Point(_) => (anchor.y, anchor.y),
                PopoverAnchor::Bounds(bounds) => (bounds.bottom_left().y, bounds.origin.y),
                PopoverAnchor::Centered => (anchor.y, anchor.y),
            };
            let below = (window_h - margin_y) - (below_anchor_y + gap_y);
            let above = (above_anchor_y - gap_y) - margin_y;
            if below < scaled_px(240.0) && above > below {
                anchor_corner = match anchor_corner {
                    Anchor::TopLeft => Anchor::BottomLeft,
                    Anchor::TopRight => Anchor::BottomRight,
                    corner => corner,
                };
            }
            if anchor_is_bounds {
                anchor = anchor_for_corner(anchor_corner);
            }

            let popover_edge_y = match anchor_corner {
                Anchor::BottomLeft | Anchor::BottomRight => anchor.y - gap_y,
                _ => anchor.y + gap_y,
            };
            let max_popover_h = match anchor_corner {
                Anchor::BottomLeft | Anchor::BottomRight => popover_edge_y - margin_y,
                _ => (window_h - margin_y) - popover_edge_y,
            }
            .max(px(0.0));
            let max_panel_h = (max_popover_h - scaled_px(12.0)).max(px(0.0));
            context_menu_max_panel_h = Some(max_panel_h);
        }

        let offset_y = match anchor_corner {
            Anchor::BottomLeft | Anchor::BottomRight => -gap_y,
            _ => gap_y,
        };

        let panel = if let Some(max_panel_h) = context_menu_max_panel_h {
            restrict_scroll_to_vertical_axis(
                div()
                    .id("context_menu_scroll")
                    .min_h(px(0.0))
                    .max_h(max_panel_h)
                    .overflow_y_scroll(),
            )
            .child(panel)
            .into_any_element()
        } else {
            panel.into_any_element()
        };

        let prompt_tab_navigation_enabled = self.prompt_tab_navigation_enabled();
        let panel = if prompt_tab_navigation_enabled {
            div()
                .track_focus(&self.prompt_tab_group_focus_handle)
                .tab_group()
                .child(panel)
                .child(
                    div()
                        .track_focus(&self.prompt_tab_wrap_end_focus_handle)
                        .w(px(0.0))
                        .h(px(0.0)),
                )
                .into_any_element()
        } else {
            panel
        };

        // Centered prompts are modal dialogs; anchored popovers (menus,
        // pickers) float just above the content and take the lighter lift.
        let is_centered = matches!(self.popover_anchor, Some(PopoverAnchor::Centered));
        let popover_surface = if is_centered {
            components::modal_surface(theme)
        } else {
            components::popover_surface(theme).border_color(popover_border_color)
        };
        let mut popover_container = popover_surface
            .id("app_popover")
            .debug_selector(|| "app_popover".to_string())
            .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
            // `occlude` keeps the root view's mouse-move listener from firing
            // over the popover, so the tooltip host would otherwise anchor
            // truncated-text tooltips to wherever the pointer was before the
            // popover opened. Feed it positions from inside the popover.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                let _ = this
                    .tooltip_host
                    .update(cx, |host, cx| host.on_mouse_moved(e.position, cx));
            }))
            .occlude()
            .p_1()
            .child(panel);

        if prompt_tab_navigation_enabled {
            popover_container = popover_container
                .key_context("PopoverPrompt")
                .on_action(cx.listener(Self::dismiss_prompt))
                .on_action(cx.listener(Self::focus_next_prompt_field))
                .on_action(cx.listener(Self::focus_prev_prompt_field));
        }

        if is_centered {
            let top_offset = scaled_px(80.0);
            let scrim_close = cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.close_popover_and_restore_focus(window, cx);
            });
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(components::modal_scrim(theme).on_mouse_down(MouseButton::Left, scrim_close))
                .child(
                    div()
                        .absolute()
                        .top(top_offset)
                        .left_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(div().child(popover_container)),
                )
                .into_any_element()
        } else {
            anchored()
                .position(anchor)
                .anchor(anchor_corner)
                .offset(point(px(0.0), offset_y))
                .child(popover_container)
                .into_any_element()
        }
    }
}

fn clone_repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last).trim();
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests;
