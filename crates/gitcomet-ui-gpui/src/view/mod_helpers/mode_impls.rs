use super::*;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResolverPickTarget {
    /// Append a specific line from the 3-way resolver pane.
    ThreeWayLine {
        line_ix: usize,
        choice: conflict_resolver::ConflictChoice,
    },
    /// Append a specific line from the 2-way split resolver pane.
    TwoWaySplitLine {
        row_ix: usize,
        side: conflict_resolver::ConflictPickSide,
    },
    /// Pick a full conflict chunk for the requested side.
    Chunk {
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        /// Optional resolved-output line that initiated this pick.
        /// When present, chunk pick scopes to the marker chunk at this line.
        output_line_ix: Option<usize>,
    },
}

/// Identity captured when a conflict-region Join entry is built. The action
/// is accepted only while this exact resolver revision remains current, so an
/// open menu cannot join a different pair after region indices shift.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConflictResolverJoinTarget {
    pub(crate) repo_id: RepoId,
    pub(crate) path: gitcomet_state::msg::RepoPath,
    pub(crate) conflict_rev: u64,
    pub(crate) first_region_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TerminalMenuContext {
    pub(crate) has_session: bool,
    pub(crate) has_selection: bool,
    pub(crate) connected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BranchPickerPurpose {
    Checkout,
    Delete,
    RebaseOnto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StashPickerPurpose {
    Pop,
    Apply,
    Drop,
}

/// Auto-squash strategy: which commit in each identical-message group survives,
/// the others being folded (fixup) into it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AutosquashMode {
    /// Fold each duplicate group into its newest (top) commit.
    ToTop,
    /// Only merge duplicates that are already adjacent in the list.
    Neighbor,
    /// Fold each duplicate group into its oldest (bottom) commit.
    ToBottom,
}

impl AutosquashMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            AutosquashMode::ToTop => "To Top Commit",
            AutosquashMode::Neighbor => "Neighboring Commit",
            AutosquashMode::ToBottom => "To Bottom Commit",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PopoverKind {
    HookActivity {
        repo_id: RepoId,
        operation_id: Option<GitOperationId>,
    },
    RepoPicker,
    BranchPicker {
        purpose: BranchPickerPurpose,
    },
    CreateBranchFromRefPrompt {
        repo_id: RepoId,
        target: String,
        source_selectable: bool,
        /// Text the name field opens with, so "create a branch in this group"
        /// can hand over `feat/` and leave the user typing only the leaf.
        ///
        /// Carried on the kind rather than kept beside it on the host because
        /// two prompts differing only by prefix are different popovers; sharing
        /// a value would make them compare equal.
        name_prefix: String,
    },
    RenameBranchPrompt {
        repo_id: RepoId,
        name: String,
        is_current_branch: bool,
    },
    CheckoutRemoteBranchPrompt {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    CommitPrompt {
        repo_id: RepoId,
    },
    StashPrompt,
    StashDropConfirm {
        repo_id: RepoId,
        index: usize,
        message: String,
    },
    StashPickerPrompt {
        repo_id: RepoId,
        purpose: StashPickerPurpose,
    },
    StashMenu {
        repo_id: RepoId,
        index: usize,
        message: String,
    },
    CloneRepo,
    ResetPrompt {
        repo_id: RepoId,
        target: String,
        mode: ResetMode,
    },
    SquashPrompt {
        repo_id: RepoId,
    },
    CreateTagPrompt {
        repo_id: RepoId,
        target: String,
    },
    Repo {
        repo_id: RepoId,
        kind: RepoPopoverKind,
    },
    FileHistory {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    /// Right-click menu on a reflog panel row: the same reset actions the
    /// history log's commit context menu offers, targeting the commit the
    /// clicked reflog entry points at.
    ReflogEntryMenu {
        repo_id: RepoId,
        target: CommitId,
        selector: SharedString,
    },
    PushSetUpstreamPrompt {
        repo_id: RepoId,
        remote: String,
    },
    ForcePushConfirm {
        repo_id: RepoId,
    },
    CherryPickCommitConfirm {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    MergeCommitConfirm {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    MergeAbortConfirm {
        repo_id: RepoId,
    },
    ForceDeleteBranchConfirm {
        repo_id: RepoId,
        name: String,
    },
    ForceRemoveWorktreeConfirm {
        repo_id: RepoId,
        path: std::path::PathBuf,
        branch: Option<String>,
    },
    DiscardChangesConfirm {
        repo_id: RepoId,
        area: DiffArea,
        path: Option<std::path::PathBuf>,
    },
    /// Add the clicked status path — or its folder, or its extension — to the
    /// repo-root `.gitignore`.
    ///
    /// `path` is the clicked row only. The multi-selection it may stand for is
    /// re-derived when the dialog opens and consumed only on submit, so
    /// cancelling leaves the selection intact.
    AddToGitignorePrompt {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
    },
    /// Staging would mark files resolved that still contain conflict markers.
    /// `paths` is the stage request as issued (empty means everything);
    /// `unresolved` is what the user is being warned about.
    ///
    /// `clear_selection` says whether `paths` came out of the status row
    /// selection. The selection is deliberately left intact while this dialog is
    /// up — cancelling must not cost it — so going ahead is what consumes it.
    StageConflictMarkersConfirm {
        repo_id: RepoId,
        paths: Vec<std::path::PathBuf>,
        unresolved: Vec<std::path::PathBuf>,
        clear_selection: bool,
    },
    PullReconcilePrompt {
        repo_id: RepoId,
    },
    PullPicker,
    PushPicker,
    CommitOptionsMenu {
        repo_id: RepoId,
    },
    CommitFileSortMenu,
    PreviousCommitMessagesMenu {
        repo_id: RepoId,
    },
    RepoTabMenu {
        repo_id: RepoId,
    },
    AppMenu,
    AddRepoMenu,
    TerminalShutdownConfirm(TerminalShutdownPrompt),
    UnsavedFileEditsConfirm(UnsavedFileEditsPrompt),
    TerminalMenu {
        repo_id: RepoId,
        context: TerminalMenuContext,
    },
    DiffActionMenu,
    MergetoolSettingsMenu,
    DiffHunkMenu {
        repo_id: RepoId,
        src_ix: usize,
    },
    /// Actions for a web link clicked in the rendered markdown preview or in a
    /// commit message.
    WebLinkMenu {
        url: SharedString,
    },
    /// Actions for a commit id clicked in a commit message or a SHA field.
    CommitShaLinkMenu {
        repo_id: RepoId,
        commit_id: CommitId,
        /// A commit's own SHA field cannot navigate to itself.
        allow_navigate: bool,
    },
    DiffEditorMenu {
        repo_id: RepoId,
        area: DiffArea,
        path: Option<std::path::PathBuf>,
        hunk_patch: Option<String>,
        hunks_count: usize,
        lines_patch: Option<String>,
        discard_lines_patch: Option<String>,
        lines_count: usize,
        copy_text: Option<String>,
        copy_target: Option<(usize, DiffTextRegion)>,
    },
    ConflictResolverInputRowMenu {
        line_label: SharedString,
        line_target: ResolverPickTarget,
        chunk_label: SharedString,
        chunk_target: ResolverPickTarget,
    },
    ConflictResolverChunkMenu {
        conflict_ix: usize,
        has_base: bool,
        is_three_way: bool,
        selected_choices: Vec<conflict_resolver::ConflictChoice>,
        output_line_ix: Option<usize>,
        /// section 30 split: row count of a valid split selection in this block, or
        /// `None` when there is no splittable selection (hides the entry).
        split_selection_rows: Option<usize>,
        /// Revision-bound target for joining this chunk with its previous
        /// neighbour, when one exists.
        join_previous_region: Option<ConflictResolverJoinTarget>,
        /// Revision-bound target for joining this chunk with its next
        /// neighbour, when one exists.
        join_next_region: Option<ConflictResolverJoinTarget>,
        /// kdiff3 manual diff help: how many source columns carry a pending
        /// alignment mark. Zero hides the "align" entry.
        alignment_marked_columns: usize,
        /// Whether this file already has pinned alignments to clear.
        has_manual_alignments: bool,
        /// Whether the merged output is the untouched worktree payload rather
        /// than our projection. Every resolution action refuses to run in that
        /// state, so the entries grey out instead of silently doing nothing —
        /// the toolbar already gates the same picks this way.
        output_is_protected: bool,
    },
    ConflictResolverOutputMenu {
        cursor_line: usize,
        selected_text: Option<String>,
        has_source_a: bool,
        has_source_b: bool,
        has_source_c: bool,
        is_three_way: bool,
    },
    CommitMenu {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    StatusFileMenu {
        repo_id: RepoId,
        area: DiffArea,
        path: std::path::PathBuf,
    },
    BranchMenu {
        repo_id: RepoId,
        section: BranchSection,
        name: String,
    },
    BranchSectionMenu {
        repo_id: RepoId,
        section: BranchSection,
    },
    /// Menu for a `/`-prefix group row in the branch tree (`feat/`).
    BranchGroupMenu {
        repo_id: RepoId,
        section: BranchSection,
        /// The owning remote for a remote group; `None` for a local one.
        remote: Option<String>,
        /// Full slash path with no trailing separator (`feat`, `feat/sub`).
        path: String,
    },
    /// Menu for the "Pinned Local/Remote Branches" header row.
    PinnedSectionMenu {
        repo_id: RepoId,
        section: BranchSection,
    },
    /// Confirms deleting every branch in a group. Carries the resolved member
    /// list so the dialog names what it is about to remove, rather than
    /// re-deriving it and risking a different answer than the menu showed.
    DeleteBranchesConfirm {
        repo_id: RepoId,
        section: BranchSection,
        remote: Option<String>,
        group_label: String,
        names: Vec<String>,
    },
    CommitFileMenu {
        repo_id: RepoId,
        commit_id: CommitId,
        path: std::path::PathBuf,
    },
    FileBrowserFileMenu {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    FileBrowserFolderMenu {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    BrowseHistoryMenu {
        repo_id: RepoId,
    },
    SubmoduleInnerDiffMenu {
        repo_id: RepoId,
        submodule_repo_path: std::path::PathBuf,
        target: DiffTarget,
    },
    #[allow(dead_code)]
    TagMenu {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    TagRefMenu {
        repo_id: RepoId,
        commit_id: CommitId,
        name: String,
    },
    HistoryBranchFilter {
        repo_id: RepoId,
    },
    HistoryAuthorFilter {
        repo_id: RepoId,
    },
    DiffContentModeSettings,
    ChangeTrackingSettings,
    UiScalePicker,
    RebaseOntoConfirm {
        repo_id: RepoId,
        onto: String,
    },
    RebaseReword {
        ix: usize,
        original_action: InteractiveRebaseAction,
        original_message: String,
    },
    InteractiveRebaseActionMenu {
        ix: usize,
        can_squash: bool,
        can_drop: bool,
    },
    InteractiveRebaseAutosquashMenu,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RepoPopoverKind {
    Remote(RemotePopoverKind),
    Worktree(WorktreePopoverKind),
    Submodule(SubmodulePopoverKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RemotePopoverKind {
    AddPrompt,
    EditUrlPrompt { name: String, kind: RemoteUrlKind },
    RemoveConfirm { name: String },
    Menu { name: String },
    DeleteBranchConfirm { remote: String, branch: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorktreePopoverKind {
    SectionMenu,
    Menu {
        path: std::path::PathBuf,
        branch: Option<String>,
    },
    AddPrompt,
    OpenPicker,
    RemovePicker,
    /// The action bar's workspace badge picker: every worktree including the
    /// current one, plus a create row. Distinct from `OpenPicker`, which hides
    /// the current worktree and has no create affordance.
    BadgePicker,
    RemoveConfirm {
        path: std::path::PathBuf,
        branch: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubmodulePopoverKind {
    SectionMenu,
    Menu { path: std::path::PathBuf },
    AddPrompt,
    ChangePointerPrompt { path: std::path::PathBuf },
    TrustConfirm,
    OpenPicker,
    RemovePicker,
    RemoveConfirm { path: std::path::PathBuf },
}

impl PopoverKind {
    pub(crate) fn remote(repo_id: RepoId, kind: RemotePopoverKind) -> Self {
        Self::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(kind),
        }
    }

    pub(crate) fn worktree(repo_id: RepoId, kind: WorktreePopoverKind) -> Self {
        Self::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(kind),
        }
    }

    pub(crate) fn submodule(repo_id: RepoId, kind: SubmodulePopoverKind) -> Self {
        Self::Repo {
            repo_id,
            kind: RepoPopoverKind::Submodule(kind),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteRow {
    Header(String),
    Branch { remote: String, name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffClickKind {
    Line,
    HunkHeader,
    FileHeader,
}

#[derive(Clone, Debug)]
pub(crate) enum PatchSplitRow {
    Raw {
        src_ix: usize,
        click_kind: DiffClickKind,
    },
    Aligned {
        row: FileDiffRow,
        old_src_ix: Option<usize>,
        new_src_ix: Option<usize>,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GitCometViewMode {
    #[default]
    Normal,
    FocusedMergetool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InitialRepositoryLaunchMode {
    #[default]
    RestoreSession,
    OpenExplicitly,
}

#[derive(Clone, Debug, Default)]
pub struct GitCometViewConfig {
    pub initial_path: Option<std::path::PathBuf>,
    pub initial_repository_launch_mode: InitialRepositoryLaunchMode,
    pub view_mode: GitCometViewMode,
    pub focused_mergetool: Option<FocusedMergetoolViewConfig>,
    pub focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub startup_crash_report: Option<StartupCrashReport>,
}

impl GitCometViewConfig {
    pub fn normal(startup_crash_report: Option<StartupCrashReport>) -> Self {
        Self {
            initial_path: None,
            initial_repository_launch_mode: InitialRepositoryLaunchMode::RestoreSession,
            view_mode: GitCometViewMode::Normal,
            focused_mergetool: None,
            focused_mergetool_exit_code: None,
            startup_crash_report,
        }
    }

    pub fn normal_with_initial_repository(
        initial_path: std::path::PathBuf,
        startup_crash_report: Option<StartupCrashReport>,
    ) -> Self {
        Self {
            initial_path: Some(initial_path),
            initial_repository_launch_mode: InitialRepositoryLaunchMode::OpenExplicitly,
            view_mode: GitCometViewMode::Normal,
            focused_mergetool: None,
            focused_mergetool_exit_code: None,
            startup_crash_report,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupCrashReport {
    pub issue_url: String,
    pub summary: String,
    pub crash_log_path: std::path::PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusedMergetoolLabels {
    pub local: String,
    pub remote: String,
    pub base: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusedMergetoolViewConfig {
    pub repo_path: std::path::PathBuf,
    pub conflicted_file_path: std::path::PathBuf,
    pub labels: FocusedMergetoolLabels,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FocusedMergetoolBootstrap {
    pub(crate) repo_path: std::path::PathBuf,
    pub(crate) target_path: std::path::PathBuf,
}

impl FocusedMergetoolBootstrap {
    pub(crate) fn from_view_config(config: FocusedMergetoolViewConfig) -> Self {
        let repo_path = normalize_bootstrap_repo_path(config.repo_path);
        let target_path = focused_mergetool_target_path(&repo_path, &config.conflicted_file_path);
        Self {
            repo_path,
            target_path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FocusedMergetoolBootstrapAction {
    OpenRepo(std::path::PathBuf),
    SetActiveRepo(RepoId),
    SelectConflictDiff {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    LoadConflictFile {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DeferredRepoBootstrap {
    RestoreSession {
        open_repos: Vec<std::path::PathBuf>,
        active_repo: Option<std::path::PathBuf>,
    },
    OpenRepo(std::path::PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubmoduleDiffBootstrap {
    pub(crate) repo_path: std::path::PathBuf,
    pub(crate) target: DiffTarget,
}

impl SubmoduleDiffBootstrap {
    pub(crate) fn new(repo_path: std::path::PathBuf, target: DiffTarget) -> Self {
        let repo_path = normalize_bootstrap_repo_path(repo_path);
        let target = normalize_bootstrap_diff_target(&repo_path, target);
        Self { repo_path, target }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubmoduleDiffBootstrapAction {
    OpenRepo(std::path::PathBuf),
    SetActiveRepo(RepoId),
    SelectDiff { repo_id: RepoId, target: DiffTarget },
    Complete,
}

pub(crate) fn normalize_bootstrap_repo_path(path: std::path::PathBuf) -> std::path::PathBuf {
    let path = if path.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    } else {
        path
    };
    canonicalize_path(path)
}

pub(crate) fn normalize_bootstrap_target_path(
    repo_path: &std::path::Path,
    target_path: std::path::PathBuf,
) -> std::path::PathBuf {
    if target_path.is_relative() {
        return target_path;
    }

    if let Ok(relative) = target_path.strip_prefix(repo_path) {
        return relative.to_path_buf();
    }

    canonicalize_path(target_path.clone())
        .strip_prefix(repo_path)
        .map(std::path::Path::to_path_buf)
        .unwrap_or(target_path)
}

pub(crate) fn normalize_bootstrap_diff_target(
    repo_path: &std::path::Path,
    target: DiffTarget,
) -> DiffTarget {
    match target {
        DiffTarget::WorkingTree { path, area } => DiffTarget::WorkingTree {
            path: normalize_bootstrap_target_path(repo_path, path),
            area,
        },
        DiffTarget::Commit { commit_id, path } => DiffTarget::Commit {
            commit_id,
            path: path.map(|path| normalize_bootstrap_target_path(repo_path, path)),
        },
        DiffTarget::CommitRange {
            from_commit_id,
            to_commit_id,
            path,
        } => DiffTarget::CommitRange {
            from_commit_id,
            to_commit_id,
            path: path.map(|path| normalize_bootstrap_target_path(repo_path, path)),
        },
    }
}

pub(crate) fn focused_mergetool_target_path(
    repo_path: &std::path::Path,
    conflicted_file_path: &std::path::Path,
) -> std::path::PathBuf {
    if conflicted_file_path.is_relative() {
        return conflicted_file_path.to_path_buf();
    }

    if let Ok(relative) = conflicted_file_path.strip_prefix(repo_path) {
        return relative.to_path_buf();
    }

    let normalized_conflicted = canonicalize_path(conflicted_file_path.to_path_buf());
    if let Ok(relative) = normalized_conflicted.strip_prefix(repo_path) {
        return relative.to_path_buf();
    }

    conflicted_file_path.to_path_buf()
}

pub(crate) fn canonicalize_path(path: std::path::PathBuf) -> std::path::PathBuf {
    canonicalize_or_original(path)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalTextMetrics {
    pub(crate) font_size: Pixels,
    pub(crate) line_height: Pixels,
    pub(crate) cell_width: Pixels,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalGridSize {
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) pixel_width: u16,
    pub(crate) pixel_height: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalLayoutKey {
    pub(crate) font_size_bits: u32,
    pub(crate) line_height_bits: u32,
    pub(crate) cell_width_bits: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalLayoutCache {
    pub(crate) rem_size: Pixels,
    pub(crate) key: TerminalLayoutKey,
    pub(crate) base_style: gpui::TextStyle,
    pub(crate) metrics: TerminalTextMetrics,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalCachedRow {
    pub(crate) fingerprint: u64,
    pub(crate) layout_key: TerminalLayoutKey,
    pub(crate) shaped: Option<ShapedLine>,
    pub(crate) background_rects: Vec<super::terminal_alacritty::TerminalBackgroundRect>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalViewportCacheKey {
    pub(crate) content_epoch: u64,
    pub(crate) scrollback: usize,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) layout_key: TerminalLayoutKey,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalRenderCache {
    pub(crate) viewport_key: Option<TerminalViewportCacheKey>,
    pub(crate) rows: Vec<TerminalCachedRow>,
}

pub(crate) struct TerminalViewportView {
    pub(crate) theme: AppTheme,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) term_lock: Option<AlacrittyTermLock>,
    pub(crate) pty_sender: Option<super::terminal_alacritty::PtySender>,
    pub(crate) layout_cache: Option<TerminalLayoutCache>,
    pub(crate) render_cache: TerminalRenderCache,
    pub(crate) cursor_blink_visible: bool,
    pub(crate) cursor_blink_hold_until: Instant,
    pub(crate) cursor_blink_active: bool,
    pub(crate) cursor_blink_task_scheduled: bool,
    pub(crate) cursor_blink_seq: u64,
    pub(crate) content_epoch: u64,
    pub(crate) last_content: Option<super::terminal_alacritty::TerminalContent>,
    pub(crate) viewport_bounds: Option<Bounds<Pixels>>,
    pub(crate) pressed_mouse_button: Option<gpui::MouseButton>,
    /// Last grid cell reported to the PTY for mouse-motion tracking. Used to
    /// dedupe motion reports so a TUI in any-event mode (1003) receives at most
    /// one report per cell instead of one per pixel-level move event.
    pub(crate) last_motion_cell: Option<TerminalGridPoint>,
    pub(crate) was_focused: bool,
    /// Selection endpoints in grid coordinates. Note these are *not* rotated
    /// when the PTY emits output: alacritty shifts existing content to
    /// more-negative rows as lines scroll off, so text can slide under a
    /// stationary highlight during a drag. Autoscroll itself is safe because
    /// `scroll_display` moves the viewport, not the content.
    pub(crate) selection_start: Option<TerminalGridPoint>,
    pub(crate) selection_end: Option<TerminalGridPoint>,
    /// Set by "select all" so Copy grabs the entire buffer through the trimming
    /// `copy_entire_buffer` path. Cleared as soon as a manual selection begins.
    pub(crate) select_all_active: bool,
    /// True while the left button is held down for a selection drag. Drives the
    /// window-level `TerminalSelectionTracker` listeners and the autoscroll
    /// ticker, both of which keep working after the pointer leaves the viewport.
    pub(crate) selecting: bool,
    /// Most recent pointer position seen during a drag. The autoscroll ticker
    /// re-reads it every frame so scrolling continues while the pointer is held
    /// still outside the viewport.
    pub(crate) selection_last_mouse_pos: Point<Pixels>,
    /// Whether the current drag has actually moved (pointer motion, a wheel
    /// scroll, or an autoscroll step). The ticker refuses to re-resolve the free
    /// end until it has: otherwise the first tick after a double- or
    /// triple-click would drag that word/line selection back to the press cell.
    pub(crate) selection_drag_moved: bool,
    /// Bumped whenever a drag starts or ends so a stale autoscroll ticker exits.
    pub(crate) selection_autoscroll_seq: u64,
    pub(crate) ime_state: Option<super::terminal_alacritty::TerminalImeState>,
}

/// A single terminal (one PTY + alacritty + rendered viewport). A repo can hold
/// several of these as tabs.
pub(crate) struct TerminalInstance {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) pty_sender: Option<super::terminal_alacritty::PtySender>,
    pub(crate) child_pid: Option<u32>,
    pub(crate) events_rx:
        Option<smol::channel::Receiver<super::terminal_alacritty::TerminalBackendEvent>>,
    pub(crate) connected: bool,
    pub(crate) viewport: Entity<TerminalViewportView>,
    pub(crate) session_seq: u64,
    pub(crate) title: String,
}

pub(crate) struct RepoTerminalSession {
    pub(crate) workdir: std::path::PathBuf,
    pub(crate) repo_name: String,
    pub(crate) instances: Vec<TerminalInstance>,
    pub(crate) active_index: usize,
}

impl RepoTerminalSession {
    pub(crate) fn active_instance(&self) -> Option<&TerminalInstance> {
        self.instances.get(self.active_index)
    }

    pub(crate) fn instance_by_seq(&self, seq: u64) -> Option<&TerminalInstance> {
        self.instances.iter().find(|i| i.session_seq == seq)
    }

    pub(crate) fn instance_by_seq_mut(&mut self, seq: u64) -> Option<&mut TerminalInstance> {
        self.instances.iter_mut().find(|i| i.session_seq == seq)
    }

    /// Tab index for a stable session sequence. Backend events and delayed
    /// confirmations resolve through this at close time, never by a stored
    /// index that sibling closings may have shifted.
    pub(crate) fn index_by_seq(&self, seq: u64) -> Option<usize> {
        self.instances
            .iter()
            .position(|instance| instance.session_seq == seq)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalShutdownSummary {
    pub(crate) terminal_count: usize,
    pub(crate) running_command_count: usize,
    pub(crate) repo_names: Vec<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum TerminalShutdownAction {
    CloseRepo { repo_id: RepoId },
    CloseTerminalForRepo { repo_id: RepoId },
    CloseTerminalTab { repo_id: RepoId, session_seq: u64 },
    CloseWindow,
    QuitApp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct TerminalShutdownPrompt {
    pub(in crate::view) action: TerminalShutdownAction,
    pub(in crate::view) summary: TerminalShutdownSummary,
}

/// What the window was about to do when unsaved edits were found.
///
/// Only the two irreversible ones: switching files keeps the buffer, so it
/// needs no prompt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum UnsavedFileEditsAction {
    /// Carries the window that asked: the retry can run seconds later, after a
    /// slow write drains, by which time "the active window" may be another one.
    CloseWindow(gpui::WindowId),
    QuitApp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct UnsavedFileEditsPrompt {
    pub(in crate::view) action: UnsavedFileEditsAction,
    /// Display labels, repo-qualified when the list spans more than one repo.
    pub(in crate::view) files: Vec<SharedString>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalPanelResizeState {
    pub(crate) start_y: Pixels,
    pub(crate) start_height: Pixels,
}

/// Which content the bottom panel currently shows for a repository, when more
/// than one of its panels (terminal, reflog, …) is open at once. A tab strip
/// only appears once a second panel is available; with just one open, that
/// panel fills the area exactly like before this switcher existed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BottomPanelTab {
    Terminal,
    Reflog,
}

/// A cell in alacritty's grid coordinate space. `row` is a `Line`: `0` is the
/// top of the visible screen at the live tail, and scrollback history is
/// negative down to `-history_size`. Field order matters — the derived `Ord`
/// gives row-major ordering, which is what normalises a selection's
/// `start`/`end` pair.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TerminalGridPoint {
    pub(crate) row: i32,
    pub(crate) col: u16,
}

impl TerminalGridPoint {
    pub(crate) fn new(row: i32, col: u16) -> Self {
        Self { row, col }
    }
}

pub(crate) fn focused_mergetool_bootstrap_action(
    state: &AppState,
    bootstrap: &FocusedMergetoolBootstrap,
) -> Option<FocusedMergetoolBootstrapAction> {
    let Some(repo) = state
        .repos
        .iter()
        .find(|r| r.spec.workdir == bootstrap.repo_path)
    else {
        return Some(FocusedMergetoolBootstrapAction::OpenRepo(
            bootstrap.repo_path.clone(),
        ));
    };

    if state.active_repo != Some(repo.id) {
        return Some(FocusedMergetoolBootstrapAction::SetActiveRepo(repo.id));
    }

    if !matches!(repo.open, Loadable::Ready(())) {
        return None;
    }

    let target = DiffTarget::WorkingTree {
        area: DiffArea::Unstaged,
        path: bootstrap.target_path.clone(),
    };
    if repo.diff_state.diff_target.as_ref() != Some(&target) {
        return Some(FocusedMergetoolBootstrapAction::SelectConflictDiff {
            repo_id: repo.id,
            path: bootstrap.target_path.clone(),
        });
    }

    let has_conflict_file_target =
        repo.conflict_state.conflict_file_path.as_ref() == Some(&bootstrap.target_path);
    if !has_conflict_file_target || matches!(repo.conflict_state.conflict_file, Loadable::NotLoaded)
    {
        return Some(FocusedMergetoolBootstrapAction::LoadConflictFile {
            repo_id: repo.id,
            path: bootstrap.target_path.clone(),
        });
    }

    Some(FocusedMergetoolBootstrapAction::Complete)
}

pub(crate) fn submodule_diff_bootstrap_action(
    state: &AppState,
    bootstrap: &SubmoduleDiffBootstrap,
) -> Option<SubmoduleDiffBootstrapAction> {
    let Some(repo) = state
        .repos
        .iter()
        .find(|r| r.spec.workdir == bootstrap.repo_path)
    else {
        return Some(SubmoduleDiffBootstrapAction::OpenRepo(
            bootstrap.repo_path.clone(),
        ));
    };

    if state.active_repo != Some(repo.id) {
        return Some(SubmoduleDiffBootstrapAction::SetActiveRepo(repo.id));
    }

    if !matches!(repo.open, Loadable::Ready(())) {
        return None;
    }

    if repo.diff_state.diff_target.as_ref() != Some(&bootstrap.target) {
        return Some(SubmoduleDiffBootstrapAction::SelectDiff {
            repo_id: repo.id,
            target: bootstrap.target.clone(),
        });
    }

    Some(SubmoduleDiffBootstrapAction::Complete)
}

pub(crate) fn renders_full_chrome(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(crate) fn show_diff_file_navigation(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(crate) fn show_titlebar_repo_tabs(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(crate) fn command_palette_available(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(crate) fn should_seed_initial_repository_from_session(
    view_mode: GitCometViewMode,
    initial_path: Option<&std::path::Path>,
    initial_repository_launch_mode: InitialRepositoryLaunchMode,
    has_saved_open_repos: bool,
) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
        && initial_path.is_some()
        && (matches!(
            initial_repository_launch_mode,
            InitialRepositoryLaunchMode::OpenExplicitly
        ) || has_saved_open_repos)
}

pub(crate) fn repository_entry_interstitial_active(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
) -> bool {
    matches!(view_mode, GitCometViewMode::Normal) && !has_repo_tabs
}

pub(crate) fn should_show_startup_repository_loading_screen(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
    startup_repo_bootstrap_pending: bool,
) -> bool {
    repository_entry_interstitial_active(view_mode, has_repo_tabs) && startup_repo_bootstrap_pending
}

pub(crate) fn should_show_splash_screen(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
    startup_repo_bootstrap_pending: bool,
) -> bool {
    repository_entry_interstitial_active(view_mode, has_repo_tabs)
        && !startup_repo_bootstrap_pending
}

pub(crate) fn titlebar_workspace_actions_enabled(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
) -> bool {
    !repository_entry_interstitial_active(view_mode, has_repo_tabs)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ThemeMode {
    #[default]
    Automatic,
    Named(String),
}

impl ThemeMode {
    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Automatic => "automatic",
            Self::Named(key) => key,
        }
    }

    pub(crate) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "automatic" => Some(Self::Automatic),
            "light" => Some(Self::Named(
                crate::theme::DEFAULT_LIGHT_THEME_KEY.to_string(),
            )),
            "dark" => Some(Self::Named(
                crate::theme::DEFAULT_DARK_THEME_KEY.to_string(),
            )),
            _ if crate::theme::has_theme_key(raw) => Some(Self::Named(raw.to_string())),
            _ => None,
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Automatic => "Automatic".to_string(),
            Self::Named(key) => crate::theme::theme_label(key).unwrap_or_else(|| key.clone()),
        }
    }

    pub(crate) fn resolve_theme(&self, appearance: gpui::WindowAppearance) -> AppTheme {
        match self {
            Self::Automatic => AppTheme::default_for_window_appearance(appearance),
            Self::Named(key) => crate::theme::AppTheme::from_key(key)
                .unwrap_or_else(|| AppTheme::default_for_window_appearance(appearance)),
        }
    }

    pub(crate) const fn is_automatic(&self) -> bool {
        matches!(self, Self::Automatic)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ChangeTrackingView {
    #[default]
    Combined,
    SplitUntracked,
}

impl ChangeTrackingView {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Combined => "combined",
            Self::SplitUntracked => "split_untracked",
        }
    }

    pub(crate) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "combined" => Some(Self::Combined),
            "split_untracked" => Some(Self::SplitUntracked),
            _ => None,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Combined => "Combined with Unstaged",
            Self::SplitUntracked => "Separate section",
        }
    }

    pub(crate) const fn menu_label(self) -> &'static str {
        match self {
            Self::Combined => "Combine with Unstaged",
            Self::SplitUntracked => "Show separate Untracked block",
        }
    }

    pub(crate) const fn settings_label(self) -> &'static str {
        match self {
            Self::Combined => "Combined",
            Self::SplitUntracked => "Separate section",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DiffScrollSync {
    Vertical,
    Horizontal,
    None,
    #[default]
    Both,
}

impl DiffScrollSync {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::None => "none",
            Self::Both => "both",
        }
    }

    pub(crate) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "vertical" => Some(Self::Vertical),
            "horizontal" => Some(Self::Horizontal),
            "none" => Some(Self::None),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
            Self::None => "None",
            Self::Both => "Both",
        }
    }

    pub(crate) const fn includes_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    pub(crate) const fn includes_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DiffContentMode {
    #[default]
    Full,
    Collapsed,
}

impl DiffContentMode {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Full => "content",
            Self::Collapsed => "changed_lines_only",
        }
    }

    pub(crate) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "content" => Some(Self::Full),
            "changed_lines_only" => Some(Self::Collapsed),
            _ => None,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Collapsed => "Collapsed",
        }
    }

    pub(crate) const fn settings_label(self) -> &'static str {
        self.label()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DiffWhitespaceMode {
    #[default]
    Show,
    Ignore,
}

impl DiffWhitespaceMode {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Ignore => "ignore",
        }
    }

    pub(crate) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "show" => Some(Self::Show),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }

    pub(crate) const fn toggled(self) -> Self {
        match self {
            Self::Show => Self::Ignore,
            Self::Ignore => Self::Show,
        }
    }
}
