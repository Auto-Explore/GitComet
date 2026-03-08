use super::*;

pub(super) fn toast_fade_in_duration() -> Duration {
    Duration::from_millis(TOAST_FADE_IN_MS)
}

pub(super) fn toast_fade_out_duration() -> Duration {
    Duration::from_millis(TOAST_FADE_OUT_MS)
}

pub(super) fn toast_total_lifetime(ttl: Duration) -> Duration {
    toast_fade_in_duration() + ttl + toast_fade_out_duration()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HistoryColResizeHandle {
    Branch,
    Graph,
    Author,
    Date,
    Sha,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct HistoryColResizeState {
    pub(super) handle: HistoryColResizeHandle,
    pub(super) start_x: Pixels,
    pub(super) start_branch: Pixels,
    pub(super) start_graph: Pixels,
    pub(super) start_author: Pixels,
    pub(super) start_date: Pixels,
    pub(super) start_sha: Pixels,
}

pub(super) struct HistoryColResizeDragGhost;

impl Render for HistoryColResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

pub(super) fn should_hide_unified_diff_header_line(line: &AnnotatedDiffLine) -> bool {
    matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
        && (line.text.starts_with("index ")
            || line.text.starts_with("--- ")
            || line.text.starts_with("+++ "))
}

pub(super) fn absolute_scroll_y(handle: &ScrollHandle) -> Pixels {
    let raw = handle.offset().y;
    if raw < px(0.0) { -raw } else { raw }
}

pub(super) fn scroll_is_near_bottom(handle: &ScrollHandle, threshold: Pixels) -> bool {
    let max_offset = handle.max_offset().height.max(px(0.0));
    if max_offset <= px(0.0) {
        return true;
    }

    let scroll_y = absolute_scroll_y(handle).max(px(0.0)).min(max_offset);
    (max_offset - scroll_y) <= threshold
}

pub(super) fn is_svg_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

pub(super) fn should_bypass_text_file_preview_for_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    ext.eq_ignore_ascii_case("png")
        || ext.eq_ignore_ascii_case("jpg")
        || ext.eq_ignore_ascii_case("jpeg")
        || ext.eq_ignore_ascii_case("webp")
        || ext.eq_ignore_ascii_case("ico")
        || ext.eq_ignore_ascii_case("svg")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffViewMode {
    Inline,
    Split,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SvgDiffViewMode {
    Image,
    Code,
}

/// Preview mode for the conflict resolver merge-input pane.
///
/// When the conflicted file supports a visual preview (e.g. SVG images),
/// the user can toggle between the normal text diff view and a rendered
/// preview of each conflict side.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ConflictResolverPreviewMode {
    /// Normal text/diff view with syntax highlighting.
    #[default]
    Text,
    /// Rendered preview (image for SVG files, syntax-highlighted view for markdown).
    Preview,
}

pub(super) fn is_markdown_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn"
            )
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PaneResizeHandle {
    Sidebar,
    Details,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PaneResizeState {
    pub(super) handle: PaneResizeHandle,
    pub(super) start_x: Pixels,
    pub(super) start_sidebar: Pixels,
    pub(super) start_details: Pixels,
}

pub(super) struct PaneResizeDragGhost;

impl Render for PaneResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DiffSplitResizeState {
    pub(super) handle: DiffSplitResizeHandle,
    pub(super) start_x: Pixels,
    pub(super) start_ratio: f32,
}

pub(super) struct DiffSplitResizeDragGhost;

impl Render for DiffSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConflictVSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ConflictVSplitResizeState {
    pub(super) start_y: Pixels,
    pub(super) start_ratio: f32,
}

pub(super) struct ConflictVSplitResizeDragGhost;

impl Render for ConflictVSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConflictHSplitResizeHandle {
    First,
    Second,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ConflictHSplitResizeState {
    pub(super) handle: ConflictHSplitResizeHandle,
    pub(super) start_x: Pixels,
    pub(super) start_ratios: [f32; 2],
}

pub(super) struct ConflictHSplitResizeDragGhost;

impl Render for ConflictHSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConflictDiffSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ConflictDiffSplitResizeState {
    pub(super) start_x: Pixels,
    pub(super) start_ratio: f32,
}

pub(super) struct ConflictDiffSplitResizeDragGhost;

impl Render for ConflictDiffSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum DiffTextRegion {
    Inline,
    SplitLeft,
    SplitRight,
}

impl DiffTextRegion {
    pub(super) fn order(self) -> u8 {
        match self {
            DiffTextRegion::Inline | DiffTextRegion::SplitLeft => 0,
            DiffTextRegion::SplitRight => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiffTextPos {
    pub(super) visible_ix: usize,
    pub(super) region: DiffTextRegion,
    pub(super) offset: usize,
}

impl DiffTextPos {
    pub(super) fn cmp_key(self) -> (usize, u8, usize) {
        (self.visible_ix, self.region.order(), self.offset)
    }
}

pub(super) struct DiffTextHitbox {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) layout_key: u64,
    pub(super) text_len: usize,
}

#[derive(Clone)]
pub(super) struct ToastState {
    pub(super) id: u64,
    pub(super) kind: components::ToastKind,
    pub(super) input: Entity<components::TextInput>,
    pub(super) is_code_message: bool,
    pub(super) action_url: Option<String>,
    pub(super) action_label: Option<String>,
    pub(super) ttl: Option<Duration>,
}

#[derive(Clone, Debug)]
pub(super) struct CommitDetailsDelayState {
    pub(super) repo_id: RepoId,
    pub(super) commit_id: CommitId,
    pub(super) show_loading: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct StatusMultiSelection {
    pub(super) unstaged: Vec<std::path::PathBuf>,
    pub(super) unstaged_anchor: Option<std::path::PathBuf>,
    pub(super) staged: Vec<std::path::PathBuf>,
    pub(super) staged_anchor: Option<std::path::PathBuf>,
}

pub(super) fn reconcile_status_multi_selection(
    selection: &mut StatusMultiSelection,
    status: &RepoStatus,
) {
    let mut unstaged_paths: HashSet<&std::path::Path> =
        HashSet::with_capacity_and_hasher(status.unstaged.len(), Default::default());
    for entry in &status.unstaged {
        unstaged_paths.insert(entry.path.as_path());
    }

    selection
        .unstaged
        .retain(|p| unstaged_paths.contains(&p.as_path()));
    if selection
        .unstaged_anchor
        .as_ref()
        .is_some_and(|a| !unstaged_paths.contains(&a.as_path()))
    {
        selection.unstaged_anchor = None;
    }

    let mut staged_paths: HashSet<&std::path::Path> =
        HashSet::with_capacity_and_hasher(status.staged.len(), Default::default());
    for entry in &status.staged {
        staged_paths.insert(entry.path.as_path());
    }

    selection
        .staged
        .retain(|p| staged_paths.contains(&p.as_path()));
    if selection
        .staged_anchor
        .as_ref()
        .is_some_and(|a| !staged_paths.contains(&a.as_path()))
    {
        selection.staged_anchor = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ThreeWayColumn {
    Base,
    Ours,
    Theirs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ResolvedOutputConflictMarker {
    pub(super) conflict_ix: usize,
    pub(super) range_start: usize,
    pub(super) range_end: usize,
    pub(super) is_start: bool,
    pub(super) is_end: bool,
    pub(super) unresolved: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ConflictResolverUiState {
    pub(super) repo_id: Option<RepoId>,
    pub(super) path: Option<std::path::PathBuf>,
    pub(super) conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(super) source_hash: Option<u64>,
    pub(super) current: Option<String>,
    pub(super) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    /// Mapping from visible block index to `ConflictSession` region index.
    pub(super) conflict_region_indices: Vec<usize>,
    pub(super) active_conflict: usize,
    pub(super) hovered_conflict: Option<(usize, ThreeWayColumn)>,
    pub(super) view_mode: ConflictResolverViewMode,
    pub(super) diff_rows: Vec<FileDiffRow>,
    pub(super) inline_rows: Vec<ConflictInlineRow>,
    pub(super) three_way_base_lines: Vec<SharedString>,
    pub(super) three_way_ours_lines: Vec<SharedString>,
    pub(super) three_way_theirs_lines: Vec<SharedString>,
    pub(super) three_way_len: usize,
    pub(super) three_way_conflict_ranges: Vec<Range<usize>>,
    pub(super) three_way_base_line_conflict_map: Vec<Option<usize>>,
    pub(super) three_way_ours_line_conflict_map: Vec<Option<usize>>,
    pub(super) three_way_theirs_line_conflict_map: Vec<Option<usize>>,
    pub(super) conflict_has_base: Vec<bool>,
    pub(super) three_way_word_highlights_base: conflict_resolver::WordHighlights,
    pub(super) three_way_word_highlights_ours: conflict_resolver::WordHighlights,
    pub(super) three_way_word_highlights_theirs: conflict_resolver::WordHighlights,
    pub(super) diff_word_highlights_split: conflict_resolver::TwoWayWordHighlights,
    pub(super) diff_mode: ConflictDiffMode,
    pub(super) nav_anchor: Option<usize>,
    pub(super) hide_resolved: bool,
    pub(super) three_way_visible_map: Vec<conflict_resolver::ThreeWayVisibleItem>,
    pub(super) diff_row_conflict_map: Vec<Option<usize>>,
    pub(super) inline_row_conflict_map: Vec<Option<usize>>,
    pub(super) diff_visible_row_indices: Vec<usize>,
    pub(super) inline_visible_row_indices: Vec<usize>,
    /// True when any conflict side contains non-UTF8 binary data.
    pub(super) is_binary_conflict: bool,
    /// Byte sizes of the three conflict sides (for binary UI display).
    pub(super) binary_side_sizes: [Option<usize>; 3],
    /// The resolver strategy for the current conflict (set during sync).
    pub(super) strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
    /// The conflict kind for the current file (set during sync).
    pub(super) conflict_kind: Option<gitcomet_core::domain::FileConflictKind>,
    /// Last autosolve trace summary shown in resolver UI.
    pub(super) last_autosolve_summary: Option<SharedString>,
    /// Tracks the last-seen `conflict_rev` from state so we can detect
    /// state-side session changes (e.g. hide-resolved, bulk picks, autosolve)
    /// that don't change the underlying file content.
    pub(super) conflict_rev: u64,
    /// Sequence token for debounced resolved-output outline recompute tasks.
    pub(super) resolver_pending_recompute_seq: u64,
    /// Per-line provenance metadata for the resolved output outline.
    pub(super) resolved_line_meta: Vec<ResolvedLineMeta>,
    /// Per-line conflict marker metadata for resolved output gutter markers.
    pub(super) resolved_output_conflict_markers: Vec<Option<ResolvedOutputConflictMarker>>,
    /// Set of source line keys currently represented in resolved output (for dedupe/plus-icon).
    pub(super) resolved_output_line_sources_index: HashSet<SourceLineKey>,
    /// Preview mode for the merge-input pane (Text vs rendered Preview).
    pub(super) resolver_preview_mode: ConflictResolverPreviewMode,
}

impl Default for ConflictResolverUiState {
    fn default() -> Self {
        Self {
            repo_id: None,
            path: None,
            conflict_syntax_language: None,
            source_hash: None,
            current: None,
            marker_segments: Vec::new(),
            conflict_region_indices: Vec::new(),
            active_conflict: 0,
            hovered_conflict: None,
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            diff_rows: Vec::new(),
            inline_rows: Vec::new(),
            three_way_base_lines: Vec::new(),
            three_way_ours_lines: Vec::new(),
            three_way_theirs_lines: Vec::new(),
            three_way_len: 0,
            three_way_conflict_ranges: Vec::new(),
            three_way_base_line_conflict_map: Vec::new(),
            three_way_ours_line_conflict_map: Vec::new(),
            three_way_theirs_line_conflict_map: Vec::new(),
            conflict_has_base: Vec::new(),
            three_way_word_highlights_base: Vec::new(),
            three_way_word_highlights_ours: Vec::new(),
            three_way_word_highlights_theirs: Vec::new(),
            diff_word_highlights_split: Vec::new(),
            diff_mode: ConflictDiffMode::Split,
            nav_anchor: None,
            hide_resolved: false,
            three_way_visible_map: Vec::new(),
            diff_row_conflict_map: Vec::new(),
            inline_row_conflict_map: Vec::new(),
            diff_visible_row_indices: Vec::new(),
            inline_visible_row_indices: Vec::new(),
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: None,
            conflict_kind: None,
            last_autosolve_summary: None,
            conflict_rev: 0,
            resolver_pending_recompute_seq: 0,
            resolved_line_meta: Vec::new(),
            resolved_output_conflict_markers: Vec::new(),
            resolved_output_line_sources_index: HashSet::default(),
            resolver_preview_mode: ConflictResolverPreviewMode::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)]
pub(super) enum ResolverPickTarget {
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
    /// Append a specific line from the 2-way inline resolver pane.
    TwoWayInlineLine { row_ix: usize },
    /// Pick a full conflict chunk for the requested side.
    Chunk {
        conflict_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        /// Optional resolved-output line that initiated this pick.
        /// When present, chunk pick scopes to the marker chunk at this line.
        output_line_ix: Option<usize>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PopoverKind {
    RepoPicker,
    BranchPicker,
    CreateBranch,
    CheckoutRemoteBranchPrompt {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    StashPrompt,
    StashDropConfirm {
        repo_id: RepoId,
        index: usize,
        message: String,
    },
    StashMenu {
        repo_id: RepoId,
        index: usize,
        message: String,
    },
    CloneRepo,
    Settings,
    OpenSourceLicenses,
    ResetPrompt {
        repo_id: RepoId,
        target: String,
        mode: ResetMode,
    },
    RebasePrompt {
        repo_id: RepoId,
    },
    CreateTagPrompt {
        repo_id: RepoId,
        target: String,
    },
    RemoteAddPrompt {
        repo_id: RepoId,
    },
    #[allow(dead_code)]
    RemoteUrlPicker {
        repo_id: RepoId,
        kind: RemoteUrlKind,
    },
    #[allow(dead_code)]
    RemoteRemovePicker {
        repo_id: RepoId,
    },
    RemoteBranchDeletePicker {
        repo_id: RepoId,
        remote: Option<String>,
    },
    RemoteEditUrlPrompt {
        repo_id: RepoId,
        name: String,
        kind: RemoteUrlKind,
    },
    RemoteRemoveConfirm {
        repo_id: RepoId,
        name: String,
    },
    RemoteMenu {
        repo_id: RepoId,
        name: String,
    },
    WorktreeSectionMenu {
        repo_id: RepoId,
    },
    WorktreeMenu {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    SubmoduleSectionMenu {
        repo_id: RepoId,
    },
    SubmoduleMenu {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    WorktreeAddPrompt {
        repo_id: RepoId,
    },
    WorktreeOpenPicker {
        repo_id: RepoId,
    },
    WorktreeRemovePicker {
        repo_id: RepoId,
    },
    WorktreeRemoveConfirm {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    SubmoduleAddPrompt {
        repo_id: RepoId,
    },
    SubmoduleOpenPicker {
        repo_id: RepoId,
    },
    SubmoduleRemovePicker {
        repo_id: RepoId,
    },
    SubmoduleRemoveConfirm {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    FileHistory {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    Blame {
        repo_id: RepoId,
        path: std::path::PathBuf,
        rev: Option<String>,
    },
    PushSetUpstreamPrompt {
        repo_id: RepoId,
        remote: String,
    },
    ForcePushConfirm {
        repo_id: RepoId,
    },
    MergeAbortConfirm {
        repo_id: RepoId,
    },
    ConflictSaveStageConfirm {
        repo_id: RepoId,
        path: std::path::PathBuf,
        has_conflict_markers: bool,
        unresolved_blocks: usize,
    },
    ForceDeleteBranchConfirm {
        repo_id: RepoId,
        name: String,
    },
    DeleteRemoteBranchConfirm {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    DiscardChangesConfirm {
        repo_id: RepoId,
        area: DiffArea,
        path: Option<std::path::PathBuf>,
    },
    PullReconcilePrompt {
        repo_id: RepoId,
    },
    PullPicker,
    PushPicker,
    AppMenu,
    DiffHunks,
    DiffHunkMenu {
        repo_id: RepoId,
        src_ix: usize,
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
    },
    #[allow(dead_code)]
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
    CommitFileMenu {
        repo_id: RepoId,
        commit_id: CommitId,
        path: std::path::PathBuf,
    },
    TagMenu {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    HistoryBranchFilter {
        repo_id: RepoId,
    },
    HistoryColumnSettings,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RemoteRow {
    Header(String),
    Branch { remote: String, name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffClickKind {
    Line,
    HunkHeader,
    FileHeader,
}

#[derive(Clone, Debug)]
pub(super) enum PatchSplitRow {
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
    #[allow(dead_code)]
    FocusedMergetool,
}

#[derive(Clone, Debug, Default)]
pub struct GitCometViewConfig {
    pub initial_path: Option<std::path::PathBuf>,
    pub view_mode: GitCometViewMode,
    pub focused_mergetool: Option<FocusedMergetoolViewConfig>,
    pub focused_mergetool_exit_code: Option<Arc<AtomicI32>>,
    pub startup_crash_report: Option<StartupCrashReport>,
}

impl GitCometViewConfig {
    pub fn normal(
        initial_path: Option<std::path::PathBuf>,
        startup_crash_report: Option<StartupCrashReport>,
    ) -> Self {
        Self {
            initial_path,
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
pub(super) struct FocusedMergetoolBootstrap {
    pub(super) repo_path: std::path::PathBuf,
    pub(super) target_path: std::path::PathBuf,
}

impl FocusedMergetoolBootstrap {
    pub(super) fn from_view_config(config: FocusedMergetoolViewConfig) -> Self {
        let repo_path = normalize_bootstrap_repo_path(config.repo_path);
        let target_path = focused_mergetool_target_path(&repo_path, &config.conflicted_file_path);
        Self {
            repo_path,
            target_path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FocusedMergetoolBootstrapAction {
    OpenRepo(std::path::PathBuf),
    SetActiveRepo(RepoId),
    SelectDiff {
        repo_id: RepoId,
        target: DiffTarget,
    },
    LoadConflictFile {
        repo_id: RepoId,
        path: std::path::PathBuf,
    },
    Complete,
}

pub(super) fn normalize_bootstrap_repo_path(path: std::path::PathBuf) -> std::path::PathBuf {
    let path = if path.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    } else {
        path
    };
    canonicalize_path(path)
}

pub(super) fn focused_mergetool_target_path(
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

pub(super) fn canonicalize_path(path: std::path::PathBuf) -> std::path::PathBuf {
    strip_windows_verbatim_prefix(std::fs::canonicalize(&path).unwrap_or(path))
}

#[cfg(windows)]
pub(super) fn strip_windows_verbatim_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(stripped) = path_text.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{stripped}"));
    }
    if let Some(stripped) = path_text.strip_prefix(r"\\?\") {
        return std::path::PathBuf::from(stripped);
    }
    path
}

#[cfg(not(windows))]
pub(super) fn strip_windows_verbatim_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    path
}

pub(super) fn focused_mergetool_bootstrap_action(
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
    if repo.diff_target.as_ref() != Some(&target) {
        return Some(FocusedMergetoolBootstrapAction::SelectDiff {
            repo_id: repo.id,
            target,
        });
    }

    let has_conflict_file_target = repo.conflict_file_path.as_ref() == Some(&bootstrap.target_path);
    if !has_conflict_file_target || matches!(repo.conflict_file, Loadable::NotLoaded) {
        return Some(FocusedMergetoolBootstrapAction::LoadConflictFile {
            repo_id: repo.id,
            path: bootstrap.target_path.clone(),
        });
    }

    Some(FocusedMergetoolBootstrapAction::Complete)
}

pub(super) fn renders_full_chrome(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub struct GitCometView {
    pub(super) store: Arc<AppStore>,
    pub(super) state: Arc<AppState>,
    pub(super) _ui_model: Entity<AppUiModel>,
    pub(super) _poller: Poller,
    pub(super) _ui_model_subscription: gpui::Subscription,
    pub(super) _activation_subscription: gpui::Subscription,
    pub(super) _appearance_subscription: gpui::Subscription,
    pub(super) view_mode: GitCometViewMode,
    pub(super) theme: AppTheme,
    pub(super) title_bar: Entity<TitleBarView>,
    pub(super) sidebar_pane: Entity<SidebarPaneView>,
    pub(super) main_pane: Entity<MainPaneView>,
    pub(super) details_pane: Entity<DetailsPaneView>,
    pub(super) repo_tabs_bar: Entity<RepoTabsBarView>,
    pub(super) action_bar: Entity<ActionBarView>,
    pub(super) tooltip_host: Entity<TooltipHost>,
    pub(super) toast_host: Entity<ToastHost>,
    pub(super) popover_host: Entity<PopoverHost>,
    pub(super) focused_mergetool_bootstrap: Option<FocusedMergetoolBootstrap>,

    pub(super) last_window_size: Size<Pixels>,
    pub(super) ui_window_size_last_seen: Size<Pixels>,
    pub(super) ui_settings_persist_seq: u64,

    pub(super) date_time_format: DateTimeFormat,
    pub(super) timezone: Timezone,
    pub(super) show_timezone: bool,

    pub(super) open_repo_panel: bool,
    pub(super) open_repo_input: Entity<components::TextInput>,

    pub(super) hover_resize_edge: Option<ResizeEdge>,

    pub(super) sidebar_width: Pixels,
    pub(super) details_width: Pixels,
    pub(super) pane_resize: Option<PaneResizeState>,

    pub(super) last_mouse_pos: Point<Pixels>,
    pub(super) pending_pull_reconcile_prompt: Option<RepoId>,
    pub(super) pending_force_delete_branch_prompt: Option<(RepoId, String)>,
    pub(super) startup_crash_report: Option<StartupCrashReport>,

    pub(super) error_banner_input: Entity<components::TextInput>,
    pub(super) active_context_menu_invoker: Option<SharedString>,
}

pub(super) struct DiffTextLayoutCacheEntry {
    pub(super) layout: ShapedLine,
    pub(super) last_used_epoch: u64,
}
