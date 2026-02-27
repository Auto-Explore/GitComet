use crate::{theme::AppTheme, zed_port as zed};
use gitgpui_core::diff::{AnnotatedDiffLine, annotate_unified};
use gitgpui_core::domain::{
    Branch, Commit, CommitId, DiffArea, DiffTarget, FileStatus, FileStatusKind, RepoStatus, Tag,
    UpstreamDivergence,
};
use gitgpui_core::file_diff::FileDiffRow;
use gitgpui_core::services::{PullMode, RemoteUrlKind, ResetMode};
use gitgpui_state::model::{
    AppNotificationKind, AppState, CloneOpState, CloneOpStatus, DiagnosticKind, Loadable, RepoId,
    RepoState,
};
use gitgpui_state::msg::{Msg, RepoExternalChange, StoreEvent};
use gitgpui_state::session;
use gitgpui_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Corner, CursorStyle,
    Decorations, Element, ElementId, Entity, FocusHandle, FontWeight, GlobalElementId,
    InspectorElementId, IsZero, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Render, ResizeEdge, ScrollHandle, ShapedLine, SharedString, Size,
    Style, TextRun, Tiling, Timer, UniformListScrollHandle, WeakEntity, Window,
    WindowControlArea, anchored, div, fill, point, px, relative, size, uniform_list,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

mod app_model;
mod branch_sidebar;
mod caches;
mod chrome;
mod color;
mod conflict_resolver;
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
};
use date_time::{DateTimeFormat, Timezone, format_datetime};
#[cfg(test)]
use date_time::format_datetime_utc;
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
    matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
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
    kind: zed::ToastKind,
    input: Entity<zed::TextInput>,
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

#[derive(Clone, Debug)]
struct ConflictResolverUiState {
    repo_id: Option<RepoId>,
    path: Option<std::path::PathBuf>,
    source_hash: Option<u64>,
    current: Option<String>,
    marker_segments: Vec<conflict_resolver::ConflictSegment>,
    active_conflict: usize,
    view_mode: ConflictResolverViewMode,
    diff_rows: Vec<FileDiffRow>,
    inline_rows: Vec<ConflictInlineRow>,
    three_way_base_lines: Vec<SharedString>,
    three_way_ours_lines: Vec<SharedString>,
    three_way_theirs_lines: Vec<SharedString>,
    three_way_len: usize,
    three_way_conflict_ranges: Vec<Range<usize>>,
    three_way_word_highlights_base: Vec<Option<Vec<Range<usize>>>>,
    three_way_word_highlights_ours: Vec<Option<Vec<Range<usize>>>>,
    three_way_word_highlights_theirs: Vec<Option<Vec<Range<usize>>>>,
    diff_word_highlights_split: Vec<Option<(Vec<Range<usize>>, Vec<Range<usize>>)>>,
    diff_mode: ConflictDiffMode,
    nav_anchor: Option<usize>,
    split_selected: std::collections::BTreeSet<(usize, ConflictPickSide)>,
    inline_selected: std::collections::BTreeSet<usize>,
}

impl Default for ConflictResolverUiState {
    fn default() -> Self {
        Self {
            repo_id: None,
            path: None,
            source_hash: None,
            current: None,
            marker_segments: Vec::new(),
            active_conflict: 0,
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            diff_rows: Vec::new(),
            inline_rows: Vec::new(),
            three_way_base_lines: Vec::new(),
            three_way_ours_lines: Vec::new(),
            three_way_theirs_lines: Vec::new(),
            three_way_len: 0,
            three_way_conflict_ranges: Vec::new(),
            three_way_word_highlights_base: Vec::new(),
            three_way_word_highlights_ours: Vec::new(),
            three_way_word_highlights_theirs: Vec::new(),
            diff_word_highlights_split: Vec::new(),
            diff_mode: ConflictDiffMode::Split,
            nav_anchor: None,
            split_selected: std::collections::BTreeSet::new(),
            inline_selected: std::collections::BTreeSet::new(),
        }
    }
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
    CloneRepo,
    Settings,
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

pub struct GitGpuiView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    _ui_model: Entity<AppUiModel>,
    _poller: Poller,
    _ui_model_subscription: gpui::Subscription,
    _activation_subscription: gpui::Subscription,
    _appearance_subscription: gpui::Subscription,
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

    last_window_size: Size<Pixels>,
    ui_window_size_last_seen: Size<Pixels>,
    ui_settings_persist_seq: u64,

    date_time_format: DateTimeFormat,
    timezone: Timezone,

    open_repo_panel: bool,
    open_repo_input: Entity<zed::TextInput>,

    hover_resize_edge: Option<ResizeEdge>,

    sidebar_width: Pixels,
    details_width: Pixels,
    pane_resize: Option<PaneResizeState>,

    last_mouse_pos: Point<Pixels>,
    pending_pull_reconcile_prompt: Option<RepoId>,
    pending_force_delete_branch_prompt: Option<(RepoId, String)>,

    error_banner_input: Entity<zed::TextInput>,
    active_context_menu_invoker: Option<SharedString>,
}

struct DiffTextLayoutCacheEntry {
    layout: ShapedLine,
    last_used_epoch: u64,
}

impl GitGpuiView {
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
            let _ = sidebar_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            let _ = main_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            let _ = details_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            let _ = action_bar.update(cx, |bar, cx| {
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
        let store = Arc::new(store);
        let initial_theme = AppTheme::default_for_window_appearance(window.appearance());

        let mut ui_session = session::load();
        if let Some(path) = initial_path.as_ref() {
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

        // Only auto-restore/open on startup if the store hasn't already been preloaded.
        // This avoids re-opening repos (and changing RepoIds) when the UI is attached to an
        // already-initialized store (notably in `gpui::test` setup).
        let store_preloaded = !store.snapshot().repos.is_empty();
        let should_auto_restore = {
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
            zed::TextInput::new(
                zed::TextInputOptions {
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
            zed::TextInput::new(
                zed::TextInputOptions {
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
            error_banner_input,
            active_context_menu_invoker: None,
        };

        view.set_theme(initial_theme, cx);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        view.maybe_auto_install_linux_desktop_integration(cx);

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

    fn push_toast(&mut self, kind: zed::ToastKind, message: String, cx: &mut gpui::Context<Self>) {
        self.toast_host
            .update(cx, |host, cx| host.push_toast(kind, message, cx));
    }

    #[cfg(test)]
    pub(crate) fn is_popover_open(&self, app: &App) -> bool {
        self.popover_host.read(app).is_open()
    }
}

impl Render for GitGpuiView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        self.last_window_size = window.window_bounds().get_bounds().size;
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

        let mut body = div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.colors.text)
            .child(self.title_bar.clone())
            .child(
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
                    ),
            );

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

            let dismiss = zed::Button::new("repo_error_banner_close", "✕")
                .style(zed::ButtonStyle::Transparent)
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

            let size = window.window_bounds().get_bounds().size;
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

                    let size = window.window_bounds().get_bounds().size;
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
    use gitgpui_core::domain::{Branch, CommitId, Remote, RemoteBranch, RepoSpec, Upstream};
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

        let rows = GitGpuiView::remote_rows(&repo);
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

        let rows = GitGpuiView::branch_sidebar_rows(&repo);
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

        let rows = GitGpuiView::branch_sidebar_rows(&repo);
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
}
