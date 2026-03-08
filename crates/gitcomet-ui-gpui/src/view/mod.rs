use crate::theme::AppTheme;
use gitcomet_core::diff::{AnnotatedDiffLine, annotate_unified};
use gitcomet_core::domain::{
    Branch, Commit, CommitId, DiffArea, DiffTarget, FileStatus, FileStatusKind, RepoStatus, Tag,
    UpstreamDivergence,
};
use gitcomet_core::file_diff::FileDiffRow;
use gitcomet_core::services::{PullMode, RemoteUrlKind, ResetMode};
use gitcomet_state::model::{
    AppNotificationKind, AppState, CloneOpState, CloneOpStatus, DiagnosticKind, Loadable, RepoId,
    RepoState,
};
use gitcomet_state::msg::{Msg, RepoExternalChange, StoreEvent};
use gitcomet_state::session;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Corner, CursorStyle, Decorations,
    Element, ElementId, Entity, FocusHandle, FontWeight, GlobalElementId, InspectorElementId,
    IsZero, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    Render, ResizeEdge, ScrollHandle, ShapedLine, SharedString, Size, Style, TextRun, Tiling,
    Timer, UniformListScrollHandle, WeakEntity, Window, WindowControlArea, anchored, div, fill,
    point, px, relative, size, uniform_list,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::time::Duration;

mod app_model;
mod branch_sidebar;
mod caches;
mod chrome;
mod color;
pub(crate) mod components;
pub(crate) mod conflict_resolver;
mod date_time;
mod diff_navigation;
mod diff_preview;
mod diff_text_model;
mod diff_text_selection;
mod diff_utils;
mod fingerprint;
mod history_graph;
mod icons;
mod linux_desktop_integration;
mod panels;
mod panes;
mod patch_split;
mod path_display;
mod perf;
mod poller;
mod repo_open;
pub(crate) mod rows;
mod state_apply;
mod toast_host;
mod tooltip;
mod tooltip_host;
mod word_diff;

use app_model::AppUiModel;
use branch_sidebar::{BranchSection, BranchSidebarRow};
use caches::{
    BranchSidebarCache, HistoryCache, HistoryCacheRequest, HistoryCommitRowVm,
    HistoryStashIdsCache, HistoryWorktreeSummaryCache,
};
use chrome::{
    CLIENT_SIDE_DECORATION_INSET, TitleBarView, cursor_style_for_resize_edge, resize_edge,
};
use conflict_resolver::{
    ConflictDiffMode, ConflictInlineRow, ConflictPickSide, ConflictResolverViewMode,
    ResolvedLineMeta, SourceLineKey,
};
#[cfg(test)]
use date_time::format_datetime_utc;
use date_time::{DateTimeFormat, Timezone, format_datetime};
use diff_preview::{build_deleted_file_preview_from_diff, build_new_file_preview_from_diff};
use patch_split::build_patch_split_rows;
use poller::Poller;
use word_diff::capped_word_diff_ranges;

use diff_text_model::{CachedDiffStyledText, CachedDiffTextSegment, SyntaxTokenKind};
use diff_text_selection::{DiffTextSelectionOverlay, DiffTextSelectionTracker};
use diff_utils::{
    build_unified_patch_for_hunks, build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard,
    compute_diff_file_for_src_ix, compute_diff_file_stats, compute_diff_word_highlights,
    context_menu_selection_range_from_diff_text, diff_content_text, enclosing_hunk_src_ix,
    image_format_for_path, parse_diff_git_header_path, parse_unified_hunk_header_for_display,
    scrollbar_markers_from_flags,
};
use panels::{ActionBarView, PopoverHost, RepoTabsBarView};
use panes::{DetailsPaneView, HistoryView, MainPaneView, SidebarPaneView};
use toast_host::ToastHost;
use tooltip_host::TooltipHost;

pub(crate) use chrome::window_frame;
use color::with_alpha;
use icons::{svg_icon, svg_spinner};

const HISTORY_COL_BRANCH_PX: f32 = 130.0;
const HISTORY_COL_GRAPH_PX: f32 = 80.0;
const HISTORY_COL_GRAPH_MAX_PX: f32 = 240.0;
const HISTORY_COL_AUTHOR_PX: f32 = 140.0;
const HISTORY_COL_DATE_PX: f32 = 160.0;
const HISTORY_COL_SHA_PX: f32 = 88.0;
const HISTORY_COL_HANDLE_PX: f32 = 8.0;

const HISTORY_COL_BRANCH_MIN_PX: f32 = 60.0;
const HISTORY_COL_GRAPH_MIN_PX: f32 = 44.0;
const HISTORY_COL_AUTHOR_MIN_PX: f32 = 80.0;
const HISTORY_COL_DATE_MIN_PX: f32 = 110.0;
const HISTORY_COL_SHA_MIN_PX: f32 = 60.0;

const HISTORY_GRAPH_COL_GAP_PX: f32 = 16.0;
const HISTORY_GRAPH_MARGIN_X_PX: f32 = 10.0;

const PANE_RESIZE_HANDLE_PX: f32 = 8.0;
const SIDEBAR_MIN_PX: f32 = 200.0;
const DETAILS_MIN_PX: f32 = 280.0;
const MAIN_MIN_PX: f32 = 280.0;

const DIFF_SPLIT_COL_MIN_PX: f32 = 160.0;

const DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 4000;
const DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE: usize = 256;
const TOAST_FADE_IN_MS: u64 = 180;
const TOAST_FADE_OUT_MS: u64 = 220;
const TOAST_SLIDE_PX: f32 = 12.0;

fn toast_fade_in_duration() -> Duration {
    Duration::from_millis(TOAST_FADE_IN_MS)
}

fn toast_fade_out_duration() -> Duration {
    Duration::from_millis(TOAST_FADE_OUT_MS)
}

fn toast_total_lifetime(ttl: Duration) -> Duration {
    toast_fade_in_duration() + ttl + toast_fade_out_duration()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryColResizeHandle {
    Branch,
    Graph,
    Author,
    Date,
    Sha,
}

#[derive(Clone, Copy, Debug)]
struct HistoryColResizeState {
    handle: HistoryColResizeHandle,
    start_x: Pixels,
    start_branch: Pixels,
    start_graph: Pixels,
    start_author: Pixels,
    start_date: Pixels,
    start_sha: Pixels,
}

struct HistoryColResizeDragGhost;

impl Render for HistoryColResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

fn should_hide_unified_diff_header_line(line: &AnnotatedDiffLine) -> bool {
    matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
        && (line.text.starts_with("index ")
            || line.text.starts_with("--- ")
            || line.text.starts_with("+++ "))
}

fn absolute_scroll_y(handle: &ScrollHandle) -> Pixels {
    let raw = handle.offset().y;
    if raw < px(0.0) { -raw } else { raw }
}

fn scroll_is_near_bottom(handle: &ScrollHandle, threshold: Pixels) -> bool {
    let max_offset = handle.max_offset().height.max(px(0.0));
    if max_offset <= px(0.0) {
        return true;
    }

    let scroll_y = absolute_scroll_y(handle).max(px(0.0)).min(max_offset);
    (max_offset - scroll_y) <= threshold
}

fn is_svg_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

fn should_bypass_text_file_preview_for_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    ext.eq_ignore_ascii_case("png")
        || ext.eq_ignore_ascii_case("jpg")
        || ext.eq_ignore_ascii_case("jpeg")
        || ext.eq_ignore_ascii_case("webp")
        || ext.eq_ignore_ascii_case("svg")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffViewMode {
    Inline,
    Split,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SvgDiffViewMode {
    Image,
    Code,
}

/// Preview mode for the conflict resolver merge-input pane.
///
/// When the conflicted file supports a visual preview (e.g. SVG images),
/// the user can toggle between the normal text diff view and a rendered
/// preview of each conflict side.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ConflictResolverPreviewMode {
    /// Normal text/diff view with syntax highlighting.
    #[default]
    Text,
    /// Rendered preview (image for SVG files, syntax-highlighted view for markdown).
    Preview,
}

fn is_markdown_path(path: &std::path::Path) -> bool {
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
enum PaneResizeHandle {
    Sidebar,
    Details,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PaneResizeState {
    handle: PaneResizeHandle,
    start_x: Pixels,
    start_sidebar: Pixels,
    start_details: Pixels,
}

struct PaneResizeDragGhost;

impl Render for PaneResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
struct DiffSplitResizeState {
    handle: DiffSplitResizeHandle,
    start_x: Pixels,
    start_ratio: f32,
}

struct DiffSplitResizeDragGhost;

impl Render for DiffSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConflictVSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
struct ConflictVSplitResizeState {
    start_y: Pixels,
    start_ratio: f32,
}

struct ConflictVSplitResizeDragGhost;

impl Render for ConflictVSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConflictHSplitResizeHandle {
    First,
    Second,
}

#[derive(Clone, Copy, Debug)]
struct ConflictHSplitResizeState {
    handle: ConflictHSplitResizeHandle,
    start_x: Pixels,
    start_ratios: [f32; 2],
}

struct ConflictHSplitResizeDragGhost;

impl Render for ConflictHSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConflictDiffSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
struct ConflictDiffSplitResizeState {
    start_x: Pixels,
    start_ratio: f32,
}

struct ConflictDiffSplitResizeDragGhost;

impl Render for ConflictDiffSplitResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum DiffTextRegion {
    Inline,
    SplitLeft,
    SplitRight,
}

impl DiffTextRegion {
    fn order(self) -> u8 {
        match self {
            DiffTextRegion::Inline | DiffTextRegion::SplitLeft => 0,
            DiffTextRegion::SplitRight => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiffTextPos {
    visible_ix: usize,
    region: DiffTextRegion,
    offset: usize,
}

impl DiffTextPos {
    fn cmp_key(self) -> (usize, u8, usize) {
        (self.visible_ix, self.region.order(), self.offset)
    }
}

struct DiffTextHitbox {
    bounds: Bounds<Pixels>,
    layout_key: u64,
    text_len: usize,
}

#[derive(Clone)]
struct ToastState {
    id: u64,
    kind: components::ToastKind,
    input: Entity<components::TextInput>,
    is_code_message: bool,
    ttl: Option<Duration>,
}

#[derive(Clone, Debug)]
struct CommitDetailsDelayState {
    repo_id: RepoId,
    commit_id: CommitId,
    show_loading: bool,
}

#[derive(Clone, Debug, Default)]
struct StatusMultiSelection {
    unstaged: Vec<std::path::PathBuf>,
    unstaged_anchor: Option<std::path::PathBuf>,
    staged: Vec<std::path::PathBuf>,
    staged_anchor: Option<std::path::PathBuf>,
}

fn reconcile_status_multi_selection(selection: &mut StatusMultiSelection, status: &RepoStatus) {
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
enum ThreeWayColumn {
    Base,
    Ours,
    Theirs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolvedOutputConflictMarker {
    conflict_ix: usize,
    range_start: usize,
    range_end: usize,
    is_start: bool,
    is_end: bool,
    unresolved: bool,
}

#[derive(Clone, Debug)]
struct ConflictResolverUiState {
    repo_id: Option<RepoId>,
    path: Option<std::path::PathBuf>,
    conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    source_hash: Option<u64>,
    current: Option<String>,
    marker_segments: Vec<conflict_resolver::ConflictSegment>,
    /// Mapping from visible block index to `ConflictSession` region index.
    conflict_region_indices: Vec<usize>,
    active_conflict: usize,
    hovered_conflict: Option<(usize, ThreeWayColumn)>,
    view_mode: ConflictResolverViewMode,
    diff_rows: Vec<FileDiffRow>,
    inline_rows: Vec<ConflictInlineRow>,
    three_way_base_lines: Vec<SharedString>,
    three_way_ours_lines: Vec<SharedString>,
    three_way_theirs_lines: Vec<SharedString>,
    three_way_len: usize,
    three_way_conflict_ranges: Vec<Range<usize>>,
    three_way_base_line_conflict_map: Vec<Option<usize>>,
    three_way_ours_line_conflict_map: Vec<Option<usize>>,
    three_way_theirs_line_conflict_map: Vec<Option<usize>>,
    conflict_has_base: Vec<bool>,
    three_way_word_highlights_base: conflict_resolver::WordHighlights,
    three_way_word_highlights_ours: conflict_resolver::WordHighlights,
    three_way_word_highlights_theirs: conflict_resolver::WordHighlights,
    diff_word_highlights_split: conflict_resolver::TwoWayWordHighlights,
    diff_mode: ConflictDiffMode,
    nav_anchor: Option<usize>,
    hide_resolved: bool,
    three_way_visible_map: Vec<conflict_resolver::ThreeWayVisibleItem>,
    diff_row_conflict_map: Vec<Option<usize>>,
    inline_row_conflict_map: Vec<Option<usize>>,
    diff_visible_row_indices: Vec<usize>,
    inline_visible_row_indices: Vec<usize>,
    /// True when any conflict side contains non-UTF8 binary data.
    is_binary_conflict: bool,
    /// Byte sizes of the three conflict sides (for binary UI display).
    binary_side_sizes: [Option<usize>; 3],
    /// The resolver strategy for the current conflict (set during sync).
    strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
    /// The conflict kind for the current file (set during sync).
    conflict_kind: Option<gitcomet_core::domain::FileConflictKind>,
    /// Last autosolve trace summary shown in resolver UI.
    last_autosolve_summary: Option<SharedString>,
    /// Tracks the last-seen `conflict_rev` from state so we can detect
    /// state-side session changes (e.g. hide-resolved, bulk picks, autosolve)
    /// that don't change the underlying file content.
    conflict_rev: u64,
    /// Sequence token for debounced resolved-output outline recompute tasks.
    resolver_pending_recompute_seq: u64,
    /// Per-line provenance metadata for the resolved output outline.
    resolved_line_meta: Vec<ResolvedLineMeta>,
    /// Per-line conflict marker metadata for resolved output gutter markers.
    resolved_output_conflict_markers: Vec<Option<ResolvedOutputConflictMarker>>,
    /// Set of source line keys currently represented in resolved output (for dedupe/plus-icon).
    resolved_output_line_sources_index: HashSet<SourceLineKey>,
    /// Preview mode for the merge-input pane (Text vs rendered Preview).
    resolver_preview_mode: ConflictResolverPreviewMode,
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
enum ResolverPickTarget {
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
enum PopoverKind {
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
enum RemoteRow {
    Header(String),
    Branch { remote: String, name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffClickKind {
    Line,
    HunkHeader,
    FileHeader,
}

#[derive(Clone, Debug)]
enum PatchSplitRow {
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
struct FocusedMergetoolBootstrap {
    repo_path: std::path::PathBuf,
    target_path: std::path::PathBuf,
}

impl FocusedMergetoolBootstrap {
    fn from_view_config(config: FocusedMergetoolViewConfig) -> Self {
        let repo_path = normalize_bootstrap_repo_path(config.repo_path);
        let target_path = focused_mergetool_target_path(&repo_path, &config.conflicted_file_path);
        Self {
            repo_path,
            target_path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FocusedMergetoolBootstrapAction {
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

fn normalize_bootstrap_repo_path(path: std::path::PathBuf) -> std::path::PathBuf {
    let path = if path.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    } else {
        path
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn focused_mergetool_target_path(
    repo_path: &std::path::Path,
    conflicted_file_path: &std::path::Path,
) -> std::path::PathBuf {
    if conflicted_file_path.is_relative() {
        return conflicted_file_path.to_path_buf();
    }

    if let Ok(relative) = conflicted_file_path.strip_prefix(repo_path) {
        return relative.to_path_buf();
    }

    let normalized_conflicted = std::fs::canonicalize(conflicted_file_path)
        .unwrap_or_else(|_| conflicted_file_path.to_path_buf());
    if let Ok(relative) = normalized_conflicted.strip_prefix(repo_path) {
        return relative.to_path_buf();
    }

    conflicted_file_path.to_path_buf()
}

fn focused_mergetool_bootstrap_action(
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

fn renders_full_chrome(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub struct GitCometView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    _ui_model: Entity<AppUiModel>,
    _poller: Poller,
    _ui_model_subscription: gpui::Subscription,
    _activation_subscription: gpui::Subscription,
    _appearance_subscription: gpui::Subscription,
    view_mode: GitCometViewMode,
    theme: AppTheme,
    title_bar: Entity<TitleBarView>,
    sidebar_pane: Entity<SidebarPaneView>,
    main_pane: Entity<MainPaneView>,
    details_pane: Entity<DetailsPaneView>,
    repo_tabs_bar: Entity<RepoTabsBarView>,
    action_bar: Entity<ActionBarView>,
    tooltip_host: Entity<TooltipHost>,
    toast_host: Entity<ToastHost>,
    popover_host: Entity<PopoverHost>,
    focused_mergetool_bootstrap: Option<FocusedMergetoolBootstrap>,

    last_window_size: Size<Pixels>,
    ui_window_size_last_seen: Size<Pixels>,
    ui_settings_persist_seq: u64,

    date_time_format: DateTimeFormat,
    timezone: Timezone,

    open_repo_panel: bool,
    open_repo_input: Entity<components::TextInput>,

    hover_resize_edge: Option<ResizeEdge>,

    sidebar_width: Pixels,
    details_width: Pixels,
    pane_resize: Option<PaneResizeState>,

    last_mouse_pos: Point<Pixels>,
    pending_pull_reconcile_prompt: Option<RepoId>,
    pending_force_delete_branch_prompt: Option<(RepoId, String)>,
    startup_crash_report: Option<StartupCrashReport>,

    error_banner_input: Entity<components::TextInput>,
    active_context_menu_invoker: Option<SharedString>,
}

struct DiffTextLayoutCacheEntry {
    layout: ShapedLine,
    last_used_epoch: u64,
}

impl GitCometView {
    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.popover_host.update(cx, |host, cx| {
            host.open_popover_at(kind, anchor, window, cx)
        });
    }

    pub(in crate::view) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.popover_host.update(cx, |host, cx| {
            host.open_popover_for_bounds(kind, anchor_bounds, window, cx)
        });
    }

    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();

        let sidebar_pane = self.sidebar_pane.clone();
        let main_pane = self.main_pane.clone();
        let details_pane = self.details_pane.clone();
        let action_bar = self.action_bar.clone();

        cx.defer(move |cx| {
            sidebar_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            main_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            details_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            action_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
        });
    }

    pub fn new(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        initial_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self::new_with_config(
            store,
            events,
            GitCometViewConfig::normal(initial_path, None),
            window,
            cx,
        )
    }

    pub fn new_with_config(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        config: GitCometViewConfig,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let GitCometViewConfig {
            mut initial_path,
            view_mode,
            focused_mergetool,
            focused_mergetool_exit_code,
            startup_crash_report,
        } = config;
        if initial_path.is_none() {
            initial_path = focused_mergetool.as_ref().map(|cfg| cfg.repo_path.clone());
        }
        let focused_mergetool_labels = focused_mergetool.as_ref().map(|cfg| cfg.labels.clone());
        let focused_mergetool_bootstrap = if view_mode == GitCometViewMode::FocusedMergetool {
            focused_mergetool
                .clone()
                .map(FocusedMergetoolBootstrap::from_view_config)
        } else {
            None
        };
        let store = Arc::new(store);
        let initial_theme = AppTheme::default_for_window_appearance(window.appearance());

        let mut ui_session = session::load();
        if view_mode == GitCometViewMode::Normal
            && let Some(path) = initial_path.as_ref()
        {
            if !ui_session.open_repos.iter().any(|p| p == path) {
                ui_session.open_repos.push(path.clone());
            }
            ui_session.active_repo = Some(path.clone());
        }

        let restored_sidebar_width = ui_session.sidebar_width;
        let restored_details_width = ui_session.details_width;
        let date_time_format = ui_session
            .date_time_format
            .as_deref()
            .and_then(DateTimeFormat::from_key)
            .unwrap_or(DateTimeFormat::YmdHm);
        let timezone = ui_session
            .timezone
            .as_deref()
            .and_then(Timezone::from_key)
            .unwrap_or_default();

        let history_show_author = ui_session.history_show_author.unwrap_or(true);
        let history_show_date = ui_session.history_show_date.unwrap_or(true);
        let history_show_sha = ui_session.history_show_sha.unwrap_or(false);
        let conflict_enable_whitespace_autosolve = ui_session
            .conflict_enable_whitespace_autosolve
            .unwrap_or(false);
        let conflict_enable_regex_autosolve =
            ui_session.conflict_enable_regex_autosolve.unwrap_or(false);
        let conflict_enable_history_autosolve = ui_session
            .conflict_enable_history_autosolve
            .unwrap_or(false);

        // Only auto-restore/open on startup if the store hasn't already been preloaded.
        // This avoids re-opening repos (and changing RepoIds) when the UI is attached to an
        // already-initialized store (notably in `gpui::test` setup).
        let store_preloaded = !store.snapshot().repos.is_empty();
        let should_auto_restore = if view_mode == GitCometViewMode::FocusedMergetool {
            false
        } else {
            #[cfg(test)]
            {
                false
            }
            #[cfg(not(test))]
            {
                !store_preloaded
            }
        };

        if should_auto_restore {
            if !ui_session.open_repos.is_empty() {
                store.dispatch(Msg::RestoreSession {
                    open_repos: ui_session.open_repos,
                    active_repo: ui_session.active_repo,
                });
            } else if let Ok(path) = std::env::current_dir() {
                store.dispatch(Msg::OpenRepo(path));
            }
        } else if store_preloaded {
            if let Some(path) = initial_path.as_ref() {
                store.dispatch(Msg::OpenRepo(path.clone()));
            }
        } else if let Some(path) = initial_path.as_ref() {
            store.dispatch(Msg::OpenRepo(path.clone()));
        }

        let initial_state = Arc::new(store.snapshot());
        let ui_model = cx.new(|_cx| AppUiModel::new(Arc::clone(&initial_state)));

        let ui_model_subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let should_notify = this.apply_state_snapshot(next, cx);
            if should_notify {
                cx.notify();
            }
        });

        let weak_view = cx.weak_entity();
        let poller = Poller::start(Arc::clone(&store), events, ui_model.downgrade(), window, cx);

        let title_bar = cx.new(|_cx| TitleBarView::new(initial_theme, weak_view.clone()));
        let tooltip_host = cx.new(|_cx| TooltipHost::new(initial_theme));
        let toast_host = cx.new(|_cx| ToastHost::new(initial_theme, tooltip_host.downgrade()));
        let repo_tabs_bar = cx.new(|cx| {
            RepoTabsBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });
        let action_bar = cx.new(|cx| {
            ActionBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });

        let sidebar_pane = cx.new(|cx| {
            SidebarPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });
        let main_pane = cx.new(|cx| {
            MainPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                date_time_format,
                timezone,
                history_show_author,
                history_show_date,
                history_show_sha,
                conflict_enable_whitespace_autosolve,
                conflict_enable_regex_autosolve,
                conflict_enable_history_autosolve,
                view_mode,
                focused_mergetool_labels,
                focused_mergetool_exit_code.clone(),
                weak_view.clone(),
                tooltip_host.downgrade(),
                window,
                cx,
            )
        });
        let details_pane = cx.new(|cx| {
            DetailsPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                tooltip_host.downgrade(),
                window,
                cx,
            )
        });

        let popover_host = cx.new(|cx| {
            PopoverHost::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                date_time_format,
                timezone,
                weak_view.clone(),
                toast_host.downgrade(),
                main_pane.clone(),
                details_pane.clone(),
                window,
                cx,
            )
        });

        let activation_subscription = cx.observe_window_activation(window, |this, window, _cx| {
            if !window.is_window_active() {
                return;
            }
            if let Some(repo) = this.active_repo()
                && matches!(repo.open, Loadable::Ready(_))
            {
                this.store.dispatch(Msg::RepoExternallyChanged {
                    repo_id: repo.id,
                    change: RepoExternalChange::GitState,
                });
            }
        });

        let appearance_subscription = {
            let view = cx.weak_entity();
            let mut first = true;
            window.observe_window_appearance(move |window, app| {
                if first {
                    first = false;
                    return;
                }
                let theme = AppTheme::default_for_window_appearance(window.appearance());
                let _ = view.update(app, |this, cx| {
                    this.set_theme(theme, cx);
                    cx.notify();
                });
            })
        };

        let open_repo_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/repo".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let error_banner_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let mut view = Self {
            state: Arc::clone(&initial_state),
            _ui_model: ui_model,
            store,
            _poller: poller,
            _ui_model_subscription: ui_model_subscription,
            _activation_subscription: activation_subscription,
            _appearance_subscription: appearance_subscription,
            view_mode,
            theme: initial_theme,
            title_bar,
            sidebar_pane,
            main_pane,
            details_pane,
            repo_tabs_bar,
            action_bar,
            tooltip_host,
            toast_host,
            popover_host,
            focused_mergetool_bootstrap,
            last_window_size: size(px(0.0), px(0.0)),
            ui_window_size_last_seen: size(px(0.0), px(0.0)),
            ui_settings_persist_seq: 0,
            date_time_format,
            timezone,
            open_repo_panel: false,
            open_repo_input,
            hover_resize_edge: None,
            sidebar_width: restored_sidebar_width
                .map(|w| px(w as f32))
                .unwrap_or(px(280.0))
                .max(px(SIDEBAR_MIN_PX)),
            details_width: restored_details_width
                .map(|w| px(w as f32))
                .unwrap_or(px(420.0))
                .max(px(DETAILS_MIN_PX)),
            pane_resize: None,
            last_mouse_pos: point(px(0.0), px(0.0)),
            pending_pull_reconcile_prompt: None,
            pending_force_delete_branch_prompt: None,
            startup_crash_report,
            error_banner_input,
            active_context_menu_invoker: None,
        };

        view.set_theme(initial_theme, cx);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        view.maybe_auto_install_linux_desktop_integration(cx);

        view.drive_focused_mergetool_bootstrap();

        view
    }

    fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.title_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.sidebar_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.main_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.details_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.repo_tabs_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.action_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.tooltip_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.toast_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.popover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.open_repo_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.error_banner_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
    }

    fn pane_resize_handle(
        &self,
        theme: AppTheme,
        id: &'static str,
        handle: PaneResizeHandle,
        cx: &gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .w(px(PANE_RESIZE_HANDLE_PX))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.65)))
            .active(move |s| s.bg(theme.colors.active))
            .child(div().w(px(1.0)).h_full().bg(theme.colors.border))
            .on_drag(handle, |_handle, _offset, _window, cx| {
                cx.new(|_cx| PaneResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    this.pane_resize = Some(PaneResizeState {
                        handle,
                        start_x: e.position.x,
                        start_sidebar: this.sidebar_width,
                        start_details: this.details_width,
                    });
                    cx.notify();
                }),
            )
            .on_drag_move(cx.listener(
                move |this, e: &gpui::DragMoveEvent<PaneResizeHandle>, _w, cx| {
                    let Some(state) = this.pane_resize else {
                        return;
                    };
                    if state.handle != *e.drag(cx) {
                        return;
                    }

                    let total_w = this.last_window_size.width;
                    let handles_w = px(PANE_RESIZE_HANDLE_PX) * 2.0;
                    let main_min = px(MAIN_MIN_PX);
                    let sidebar_min = px(SIDEBAR_MIN_PX);
                    let details_min = px(DETAILS_MIN_PX);

                    let dx = e.event.position.x - state.start_x;
                    match state.handle {
                        PaneResizeHandle::Sidebar => {
                            let max_sidebar =
                                (total_w - state.start_details - main_min - handles_w)
                                    .max(sidebar_min);
                            this.sidebar_width =
                                (state.start_sidebar + dx).max(sidebar_min).min(max_sidebar);
                        }
                        PaneResizeHandle::Details => {
                            let max_details =
                                (total_w - state.start_sidebar - main_min - handles_w)
                                    .max(details_min);
                            this.details_width =
                                (state.start_details - dx).max(details_min).min(max_details);
                        }
                    }
                    cx.notify();
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.pane_resize = None;
                    this.schedule_ui_settings_persist(cx);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.pane_resize = None;
                    this.schedule_ui_settings_persist(cx);
                    cx.notify();
                }),
            )
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn drive_focused_mergetool_bootstrap(&mut self) {
        let Some(bootstrap) = self.focused_mergetool_bootstrap.as_ref() else {
            return;
        };
        let Some(action) = focused_mergetool_bootstrap_action(&self.state, bootstrap) else {
            return;
        };

        match action {
            FocusedMergetoolBootstrapAction::OpenRepo(path) => {
                self.store.dispatch(Msg::OpenRepo(path))
            }
            FocusedMergetoolBootstrapAction::SetActiveRepo(repo_id) => {
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            FocusedMergetoolBootstrapAction::SelectDiff { repo_id, target } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
            }
            FocusedMergetoolBootstrapAction::LoadConflictFile { repo_id, path } => {
                self.store.dispatch(Msg::LoadConflictFile { repo_id, path });
            }
            FocusedMergetoolBootstrapAction::Complete => {
                self.focused_mergetool_bootstrap = None;
            }
        }
    }

    fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    #[cfg(test)]
    fn remote_rows(repo: &RepoState) -> Vec<RemoteRow> {
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();

        if let Loadable::Ready(remote_branches) = &repo.remote_branches {
            for branch in remote_branches.iter() {
                grouped
                    .entry(branch.remote.clone())
                    .or_default()
                    .push(branch.name.clone());
            }
        }

        if grouped.is_empty()
            && let Loadable::Ready(remotes) = &repo.remotes
        {
            for remote in remotes.iter() {
                grouped.entry(remote.name.clone()).or_default();
            }
        }

        let mut rows = Vec::new();
        for (remote, mut branches) in grouped {
            branches.sort();
            branches.dedup();
            rows.push(RemoteRow::Header(remote.clone()));
            for name in branches {
                rows.push(RemoteRow::Branch {
                    remote: remote.clone(),
                    name,
                });
            }
        }

        rows
    }

    fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        self.toast_host
            .update(cx, |host, cx| host.push_toast(kind, message, cx));
    }

    fn open_external_url(&mut self, url: &str) -> Result<(), std::io::Error> {
        if url.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "URL is empty",
            ));
        }

        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(url).spawn()?;
            return Ok(());
        }

        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", ""])
                .arg(url)
                .spawn()?;
            return Ok(());
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            match std::process::Command::new("xdg-open").arg(url).spawn() {
                Ok(_) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    let _ = std::process::Command::new("gio")
                        .args(["open"])
                        .arg(url)
                        .spawn()?;
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }

        #[cfg(not(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux",
            target_os = "freebsd"
        )))]
        {
            let _ = url;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Opening URLs is not supported on this platform",
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn is_popover_open(&self, app: &App) -> bool {
        self.popover_host.read(app).is_open()
    }
}

impl Render for GitCometView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        debug_assert!(matches!(
            self.view_mode,
            GitCometViewMode::Normal | GitCometViewMode::FocusedMergetool
        ));
        self.last_window_size = window.viewport_size();
        self.clamp_pane_widths_to_window();
        if self.last_window_size != self.ui_window_size_last_seen {
            self.ui_window_size_last_seen = self.last_window_size;
            self.schedule_ui_settings_persist(cx);
        }

        if let Some(repo_id) = self.pending_pull_reconcile_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::PullReconcilePrompt { repo_id },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        if let Some((repo_id, name)) = self.pending_force_delete_branch_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::ForceDeleteBranchConfirm { repo_id, name },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        let decorations = window.window_decorations();
        let (tiling, client_inset) = match decorations {
            Decorations::Client { tiling } => (Some(tiling), CLIENT_SIDE_DECORATION_INSET),
            Decorations::Server => (None, px(0.0)),
        };
        window.set_client_inset(client_inset);

        let cursor = self
            .hover_resize_edge
            .map(cursor_style_for_resize_edge)
            .unwrap_or(CursorStyle::Arrow);

        let center_content = if renders_full_chrome(self.view_mode) {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(self.repo_tabs_bar.clone())
                .child(self.open_repo_panel(cx))
                .child(self.action_bar.clone())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .id("sidebar_pane")
                                .w(self.sidebar_width)
                                .min_h(px(0.0))
                                .bg(theme.colors.surface_bg)
                                .child(self.sidebar_pane.clone()),
                        )
                        .child(self.pane_resize_handle(
                            theme,
                            "pane_resize_sidebar",
                            PaneResizeHandle::Sidebar,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .child(self.main_pane.clone()),
                        )
                        .child(self.pane_resize_handle(
                            theme,
                            "pane_resize_details",
                            PaneResizeHandle::Details,
                            cx,
                        ))
                        .child(
                            div()
                                .id("details_pane")
                                .w(self.details_width)
                                .min_h(px(0.0))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h(px(0.0))
                                        .child(self.details_pane.clone()),
                                ),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(self.main_pane.clone()),
                )
                .into_any_element()
        };

        let mut body = div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.colors.text)
            .child(self.title_bar.clone())
            .child(center_content);

        if let Some(report) = self.startup_crash_report.clone()
            && self.view_mode == GitCometViewMode::Normal
        {
            let issue_url = report.issue_url.clone();
            let summary = report.summary.clone();

            let report_button =
                components::Button::new("startup_crash_report_open", "Report Issue")
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        match this.open_external_url(&issue_url) {
                            Ok(()) => {
                                this.push_toast(
                                    components::ToastKind::Success,
                                    "Opened crash report page in your browser.".to_string(),
                                    cx,
                                );
                                this.startup_crash_report = None;
                            }
                            Err(err) => {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    format!("Failed to open browser: {err}"),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });

            let dismiss_button = components::Button::new("startup_crash_report_dismiss", "Dismiss")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.startup_crash_report = None;
                    cx.notify();
                });

            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .bg(with_alpha(theme.colors.warning, 0.13))
                    .border_1()
                    .border_color(with_alpha(theme.colors.warning, 0.30))
                    .rounded(px(theme.radii.panel))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child("GitComet recovered from program crash"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.colors.text_muted)
                                    .child(
                                        "Would you like to contribute by reporting issue to GitComet GitHub repository?",
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(format!("Summary: {summary}")),
                            )
                            .child(
                                div()
                                    .pt_1()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(report_button)
                                    .child(dismiss_button),
                            ),
                    ),
            );
        }

        if let Some(repo_id) = self.active_repo_id()
            && let Some(repo) = self.active_repo()
            && let Some(err) = repo.last_error.as_ref()
        {
            let err_text: &str = err.as_ref();
            let (error_command, display_error): (Option<SharedString>, SharedString) = (|| {
                let lines: Vec<&str> = err_text.lines().collect();
                let Some(cmd_start) = lines.iter().position(|line| line.starts_with("    git "))
                else {
                    return (None, err.clone().into());
                };

                let mut cmd_end = cmd_start;
                while cmd_end < lines.len() && lines[cmd_end].starts_with("    ") {
                    cmd_end += 1;
                }

                let command = lines[cmd_start..cmd_end]
                    .iter()
                    .map(|line| line.strip_prefix("    ").unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n");

                let mut body_lines: Vec<String> = Vec::with_capacity(lines.len());
                for line in &lines[..cmd_start] {
                    body_lines.push((*line).to_string());
                }
                for line in &lines[cmd_end..] {
                    body_lines.push(line.strip_prefix("    ").unwrap_or(line).to_string());
                }

                let mut collapsed: Vec<String> = Vec::with_capacity(body_lines.len());
                let mut prev_blank = false;
                for line in body_lines {
                    let blank = line.trim().is_empty();
                    if blank && prev_blank {
                        continue;
                    }
                    collapsed.push(line);
                    prev_blank = blank;
                }

                (Some(command.into()), collapsed.join("\n").into())
            })(
            );
            self.error_banner_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text(display_error.clone(), cx);
                input.set_read_only(true, cx);
            });

            let dismiss = components::Button::new("repo_error_banner_close", "")
                .start_slot(svg_icon(
                    "icons/generic_close.svg",
                    theme.colors.text_muted,
                    px(12.0),
                ))
                .style(components::ButtonStyle::Transparent)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::DismissRepoError { repo_id });
                    cx.notify();
                });

            let command_block = error_command.as_ref().map(|command| {
                div()
                    .id("repo_error_banner_command")
                    .font_family("monospace")
                    .bg(with_alpha(
                        theme.colors.window_bg,
                        if theme.is_dark { 0.28 } else { 0.75 },
                    ))
                    .rounded(px(theme.radii.row))
                    .px_2()
                    .py_1()
                    .child(command.clone())
            });

            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .pr(px(40.0))
                    .bg(with_alpha(theme.colors.danger, 0.15))
                    .border_1()
                    .border_color(with_alpha(theme.colors.danger, 0.3))
                    .rounded(px(theme.radii.panel))
                    .child(
                        div()
                            .id("repo_error_banner_scroll")
                            .max_h(px(140.0))
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .when_some(command_block, |this, command_block| {
                                        this.child(command_block)
                                    })
                                    .child(self.error_banner_input.clone()),
                            ),
                    )
                    .child(div().absolute().top(px(6.0)).right(px(6.0)).child(dismiss)),
            );
        }

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.text);
        root = root.relative();

        root = root.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            this.last_mouse_pos = e.position;
            this.tooltip_host
                .update(cx, |tooltip, cx| tooltip.on_mouse_moved(e.position, cx));

            let Decorations::Client { tiling } = window.window_decorations() else {
                if this.hover_resize_edge.is_some() {
                    this.hover_resize_edge = None;
                    cx.notify();
                }
                return;
            };

            let size = window.viewport_size();
            let next = resize_edge(e.position, CLIENT_SIDE_DECORATION_INSET, size, tiling);
            if next != this.hover_resize_edge {
                this.hover_resize_edge = next;
                cx.notify();
            }
        }));
        if tiling.is_some() {
            root = root.on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, e: &MouseDownEvent, window, cx| {
                    let Decorations::Client { tiling } = window.window_decorations() else {
                        return;
                    };

                    let size = window.viewport_size();
                    let edge = resize_edge(e.position, CLIENT_SIDE_DECORATION_INSET, size, tiling);
                    let Some(edge) = edge else {
                        return;
                    };

                    cx.stop_propagation();
                    window.start_window_resize(edge);
                }),
            );
        } else if self.hover_resize_edge.is_some() {
            self.hover_resize_edge = None;
        }

        root = root.child(window_frame(theme, decorations, body.into_any_element()));

        root = root.child(self.toast_host.clone());

        root = root.child(self.popover_host.clone());

        root = root.child(self.tooltip_host.clone());

        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, Remote, RemoteBranch, RepoSpec, Upstream};
    use std::path::PathBuf;

    #[test]
    fn toast_total_lifetime_includes_fade_in_and_out() {
        let ttl = Duration::from_secs(6);
        assert_eq!(
            toast_total_lifetime(ttl),
            ttl + Duration::from_millis(TOAST_FADE_IN_MS + TOAST_FADE_OUT_MS)
        );
    }

    #[test]
    fn reconcile_status_multi_selection_prunes_missing_paths_and_anchors() {
        let a = PathBuf::from("a.txt");
        let b = PathBuf::from("b.txt");
        let c = PathBuf::from("c.txt");

        let status = RepoStatus {
            staged: vec![],
            unstaged: vec![FileStatus {
                path: a.clone(),
                kind: FileStatusKind::Modified,
                conflict: None,
            }],
        };

        let mut selection = StatusMultiSelection {
            unstaged: vec![a.clone(), b.clone()],
            unstaged_anchor: Some(b),
            staged: vec![c.clone()],
            staged_anchor: Some(c),
        };

        reconcile_status_multi_selection(&mut selection, &status);

        assert_eq!(selection.unstaged, vec![a]);
        assert!(selection.unstaged_anchor.is_none());
        assert!(selection.staged.is_empty());
        assert!(selection.staged_anchor.is_none());
    }

    #[test]
    fn remote_rows_groups_and_sorts() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::new(),
            },
        );
        repo.remote_branches = Loadable::Ready(Arc::new(vec![
            RemoteBranch {
                remote: "origin".to_string(),
                name: "b".to_string(),
                target: CommitId("b0".to_string()),
            },
            RemoteBranch {
                remote: "origin".to_string(),
                name: "a".to_string(),
                target: CommitId("a0".to_string()),
            },
            RemoteBranch {
                remote: "upstream".to_string(),
                name: "main".to_string(),
                target: CommitId("c0".to_string()),
            },
        ]));

        let rows = GitCometView::remote_rows(&repo);
        assert_eq!(
            rows,
            vec![
                RemoteRow::Header("origin".to_string()),
                RemoteRow::Branch {
                    remote: "origin".to_string(),
                    name: "a".to_string()
                },
                RemoteRow::Branch {
                    remote: "origin".to_string(),
                    name: "b".to_string()
                },
                RemoteRow::Header("upstream".to_string()),
                RemoteRow::Branch {
                    remote: "upstream".to_string(),
                    name: "main".to_string()
                },
            ]
        );
    }

    #[test]
    fn remote_headers_include_remotes_with_no_branches() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::new(),
            },
        );

        repo.remotes = Loadable::Ready(Arc::new(vec![
            Remote {
                name: "origin".to_string(),
                url: Some("https://example.com/origin.git".to_string()),
            },
            Remote {
                name: "upstream".to_string(),
                url: Some("https://example.com/upstream.git".to_string()),
            },
        ]));
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: CommitId("deadbeef".to_string()),
        }]));

        let rows = GitCometView::branch_sidebar_rows(&repo);
        let mut headers = rows
            .iter()
            .filter_map(|r| match r {
                BranchSidebarRow::RemoteHeader { name } => Some(name.as_ref().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>();
        headers.sort();
        headers.dedup();

        assert!(
            headers.contains(&"origin".to_string()),
            "expected origin remote header"
        );
        assert!(
            headers.contains(&"upstream".to_string()),
            "expected upstream remote header"
        );
    }

    #[test]
    fn remote_upstream_branch_is_marked() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::new(),
            },
        );

        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".to_string()),
            upstream: Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            divergence: None,
        }]));
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: CommitId("deadbeef".to_string()),
        }]));

        let rows = GitCometView::branch_sidebar_rows(&repo);
        let upstream_row = rows.iter().find(|r| {
            matches!(
                r,
                BranchSidebarRow::Branch {
                    section: BranchSection::Remote,
                    name,
                    is_upstream: true,
                    ..
                } if name.as_ref() == "origin/main"
            )
        });
        assert!(
            upstream_row.is_some(),
            "expected origin/main to be marked as upstream"
        );
    }

    #[test]
    fn resize_edge_detects_edges_and_corners() {
        let window_size = size(px(100.0), px(100.0));
        let tiling = Tiling::default();
        let inset = px(10.0);

        assert_eq!(
            resize_edge(point(px(0.0), px(0.0)), inset, window_size, tiling),
            Some(ResizeEdge::TopLeft)
        );
        assert_eq!(
            resize_edge(point(px(99.0), px(0.0)), inset, window_size, tiling),
            Some(ResizeEdge::TopRight)
        );
        assert_eq!(
            resize_edge(point(px(0.0), px(99.0)), inset, window_size, tiling),
            Some(ResizeEdge::BottomLeft)
        );
        assert_eq!(
            resize_edge(point(px(99.0), px(99.0)), inset, window_size, tiling),
            Some(ResizeEdge::BottomRight)
        );

        assert_eq!(
            resize_edge(point(px(50.0), px(0.0)), inset, window_size, tiling),
            Some(ResizeEdge::Top)
        );
        assert_eq!(
            resize_edge(point(px(50.0), px(99.0)), inset, window_size, tiling),
            Some(ResizeEdge::Bottom)
        );
        assert_eq!(
            resize_edge(point(px(0.0), px(50.0)), inset, window_size, tiling),
            Some(ResizeEdge::Left)
        );
        assert_eq!(
            resize_edge(point(px(99.0), px(50.0)), inset, window_size, tiling),
            Some(ResizeEdge::Right)
        );

        assert_eq!(
            resize_edge(point(px(50.0), px(50.0)), inset, window_size, tiling),
            None
        );
    }

    #[test]
    fn resize_edge_respects_tiling() {
        let window_size = size(px(100.0), px(100.0));
        let inset = px(10.0);
        let tiling = Tiling {
            top: true,
            left: false,
            right: false,
            bottom: false,
        };

        assert_eq!(
            resize_edge(point(px(0.0), px(0.0)), inset, window_size, tiling),
            Some(ResizeEdge::Left)
        );
        assert_eq!(
            resize_edge(point(px(50.0), px(0.0)), inset, window_size, tiling),
            None
        );
        assert_eq!(
            resize_edge(point(px(0.0), px(50.0)), inset, window_size, tiling),
            Some(ResizeEdge::Left)
        );
    }

    #[test]
    fn cursor_style_matches_resize_edge() {
        assert_eq!(
            cursor_style_for_resize_edge(ResizeEdge::Left),
            CursorStyle::ResizeLeftRight
        );
        assert_eq!(
            cursor_style_for_resize_edge(ResizeEdge::Top),
            CursorStyle::ResizeUpDown
        );
        assert_eq!(
            cursor_style_for_resize_edge(ResizeEdge::TopLeft),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            cursor_style_for_resize_edge(ResizeEdge::TopRight),
            CursorStyle::ResizeUpRightDownLeft
        );
    }

    #[test]
    fn is_markdown_path_detects_common_extensions() {
        use std::path::Path;
        assert!(is_markdown_path(Path::new("README.md")));
        assert!(is_markdown_path(Path::new("doc.markdown")));
        assert!(is_markdown_path(Path::new("notes.mdown")));
        assert!(is_markdown_path(Path::new("CHANGES.mkd")));
        assert!(is_markdown_path(Path::new("file.mkdn")));
        assert!(is_markdown_path(Path::new("file.mdwn")));
        assert!(is_markdown_path(Path::new("UPPER.MD")));
    }

    #[test]
    fn is_markdown_path_rejects_non_markdown() {
        use std::path::Path;
        assert!(!is_markdown_path(Path::new("file.txt")));
        assert!(!is_markdown_path(Path::new("file.rs")));
        assert!(!is_markdown_path(Path::new("file")));
    }

    #[test]
    fn conflict_resolver_preview_mode_defaults_to_text() {
        assert_eq!(
            ConflictResolverPreviewMode::default(),
            ConflictResolverPreviewMode::Text
        );
    }

    fn focused_bootstrap(
        repo_path: PathBuf,
        conflicted_file_path: PathBuf,
    ) -> FocusedMergetoolBootstrap {
        FocusedMergetoolBootstrap::from_view_config(FocusedMergetoolViewConfig {
            repo_path,
            conflicted_file_path,
            labels: FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            },
        })
    }

    fn open_repo_state_with_workdir(workdir: &str) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: normalize_bootstrap_repo_path(PathBuf::from(workdir)),
            },
        );
        repo.open = Loadable::Ready(());
        repo
    }

    #[test]
    fn focused_mergetool_target_path_prefers_repo_relative_path() {
        let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
        let target = focused_mergetool_target_path(&repo, &repo.join("src/conflict.txt"));
        assert_eq!(target, PathBuf::from("src/conflict.txt"));
    }

    #[test]
    fn focused_mergetool_bootstrap_requests_open_repo_when_missing() {
        let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
        let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
        let state = AppState::default();

        assert_eq!(
            focused_mergetool_bootstrap_action(&state, &bootstrap),
            Some(FocusedMergetoolBootstrapAction::OpenRepo(repo))
        );
    }

    #[test]
    fn focused_mergetool_bootstrap_selects_worktree_diff_target() {
        let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
        let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
        let mut state = AppState::default();
        state.active_repo = Some(RepoId(1));
        state
            .repos
            .push(open_repo_state_with_workdir(&repo.to_string_lossy()));

        assert_eq!(
            focused_mergetool_bootstrap_action(&state, &bootstrap),
            Some(FocusedMergetoolBootstrapAction::SelectDiff {
                repo_id: RepoId(1),
                target: DiffTarget::WorkingTree {
                    area: DiffArea::Unstaged,
                    path: PathBuf::from("src/conflict.txt"),
                },
            })
        );
    }

    #[test]
    fn focused_mergetool_bootstrap_loads_conflict_file_after_diff_target() {
        let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
        let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
        let mut state = AppState::default();
        state.active_repo = Some(RepoId(1));
        let mut repo_state = open_repo_state_with_workdir(&repo.to_string_lossy());
        repo_state.diff_target = Some(DiffTarget::WorkingTree {
            area: DiffArea::Unstaged,
            path: PathBuf::from("src/conflict.txt"),
        });
        state.repos.push(repo_state);

        assert_eq!(
            focused_mergetool_bootstrap_action(&state, &bootstrap),
            Some(FocusedMergetoolBootstrapAction::LoadConflictFile {
                repo_id: RepoId(1),
                path: PathBuf::from("src/conflict.txt"),
            })
        );
    }

    #[test]
    fn focused_mergetool_bootstrap_completes_after_conflict_file_target_set() {
        let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
        let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
        let mut state = AppState::default();
        state.active_repo = Some(RepoId(1));
        let mut repo_state = open_repo_state_with_workdir(&repo.to_string_lossy());
        repo_state.diff_target = Some(DiffTarget::WorkingTree {
            area: DiffArea::Unstaged,
            path: PathBuf::from("src/conflict.txt"),
        });
        repo_state.conflict_file_path = Some(PathBuf::from("src/conflict.txt"));
        repo_state.conflict_file = Loadable::Loading;
        state.repos.push(repo_state);

        assert_eq!(
            focused_mergetool_bootstrap_action(&state, &bootstrap),
            Some(FocusedMergetoolBootstrapAction::Complete)
        );
    }

    #[test]
    fn focused_mergetool_mode_hides_full_chrome() {
        assert!(renders_full_chrome(GitCometViewMode::Normal));
        assert!(!renders_full_chrome(GitCometViewMode::FocusedMergetool));
    }
}
