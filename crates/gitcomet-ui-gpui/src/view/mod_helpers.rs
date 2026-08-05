use super::*;
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::InteractiveRebaseAction;
use rustc_hash::FxHashMap;

type AlacrittyTermLock = super::terminal_alacritty::AlacrittyTermLock;

pub(super) fn toast_fade_in_duration() -> Duration {
    Duration::from_millis(TOAST_FADE_IN_MS)
}

pub(super) fn toast_fade_out_duration() -> Duration {
    Duration::from_millis(TOAST_FADE_OUT_MS)
}

pub(super) fn toast_total_lifetime(ttl: Duration) -> Duration {
    toast_fade_in_duration() + ttl + toast_fade_out_duration()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct SelectedBranch {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) section: BranchSection,
    pub(in crate::view) name: String,
}

pub(in crate::view) fn selected_branch_label_color(theme: AppTheme) -> gpui::Rgba {
    theme.colors.emphasis_text
}

pub(in crate::view) fn selected_branch_row_bg(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.text, if theme.is_dark { 0.16 } else { 0.10 })
}

pub(in crate::view) fn selected_branch_history_entry_text(
    selected_branch: Option<&SelectedBranch>,
    repo_id: RepoId,
    is_head: bool,
    selected: bool,
) -> Option<SharedString> {
    if !selected {
        return None;
    }

    let selected_branch = selected_branch?;
    if selected_branch.repo_id != repo_id {
        return None;
    }

    match selected_branch.section {
        BranchSection::Local if is_head => Some(format!("HEAD → {}", selected_branch.name).into()),
        BranchSection::Local | BranchSection::Remote => {
            Some(SharedString::from(selected_branch.name.clone()))
        }
    }
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
    pub(super) start_width: Pixels,
    pub(super) current_width: Pixels,
    pub(super) drag_delta_sign: f32,
    pub(super) min_width: Pixels,
    pub(super) static_max_width: Pixels,
    pub(super) other_fixed_width: Pixels,
    pub(super) bounds_available_width: Pixels,
    pub(super) max_width: Pixels,
    pub(super) visible_columns: (bool, bool, bool),
}

pub(super) struct ResizeDragGhost;

impl Render for ResizeDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}

pub(super) use ResizeDragGhost as HistoryColResizeDragGhost;

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
    let max_offset = handle.max_offset().y.max(px(0.0));
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
    image_format_for_path(path).is_some()
        || path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RenderableConflictFile {
    Loading,
    Error(SharedString),
    Missing,
    File(gitcomet_state::model::ConflictFile),
}

pub(super) fn conflict_file_is_binary(file: &gitcomet_state::model::ConflictFile) -> bool {
    let has_non_text = |bytes: &Option<std::sync::Arc<[u8]>>,
                        text: &Option<std::sync::Arc<str>>| {
        bytes.is_some() && text.is_none()
    };
    has_non_text(&file.base_bytes, &file.base)
        || has_non_text(&file.ours_bytes, &file.ours)
        || has_non_text(&file.theirs_bytes, &file.theirs)
        || has_non_text(&file.current_bytes, &file.current)
}

pub(super) fn renderable_conflict_file(
    repo: &RepoState,
    conflict_resolver: &ConflictResolverUiState,
    target_path: &std::path::Path,
) -> RenderableConflictFile {
    match &repo.conflict_state.conflict_file {
        Loadable::Ready(Some(file)) if file.path == target_path => {
            RenderableConflictFile::File(file.clone())
        }
        Loadable::Ready(Some(_)) => RenderableConflictFile::Loading,
        Loadable::Loading | Loadable::NotLoaded => conflict_resolver
            .cached_loaded_file_for_target(repo.id, target_path)
            .cloned()
            .map(RenderableConflictFile::File)
            .unwrap_or(RenderableConflictFile::Loading),
        Loadable::Error(error) => RenderableConflictFile::Error(error.clone().into()),
        Loadable::Ready(None) => RenderableConflictFile::Missing,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffViewMode {
    Inline,
    Split,
}

impl DiffViewMode {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Split => "split",
        }
    }

    pub(super) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "inline" => Some(Self::Inline),
            "split" => Some(Self::Split),
            _ => None,
        }
    }

    pub(super) const fn settings_label(self) -> &'static str {
        match self {
            Self::Inline => "Inline",
            Self::Split => "Split",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum RenderedPreviewKind {
    Svg,
    Markdown,
}

impl RenderedPreviewKind {
    pub(super) fn rendered_label(self) -> &'static str {
        match self {
            Self::Svg => "Image",
            Self::Markdown => "Preview",
        }
    }

    pub(super) fn source_label(self) -> &'static str {
        match self {
            Self::Svg => "Code",
            Self::Markdown => "Text",
        }
    }

    pub(super) fn rendered_button_id(self) -> &'static str {
        match self {
            Self::Svg => "svg_diff_view_image",
            Self::Markdown => "markdown_diff_view_preview",
        }
    }

    pub(super) fn toggle_id(self) -> &'static str {
        match self {
            Self::Svg => "svg_diff_view_toggle",
            Self::Markdown => "markdown_diff_view_toggle",
        }
    }

    pub(super) fn source_button_id(self) -> &'static str {
        match self {
            Self::Svg => "svg_diff_view_code",
            Self::Markdown => "markdown_diff_view_text",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RenderedPreviewMode {
    Rendered,
    Source,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RenderedPreviewModes {
    pub(super) svg: RenderedPreviewMode,
    pub(super) markdown: RenderedPreviewMode,
}

impl Default for RenderedPreviewModes {
    fn default() -> Self {
        Self {
            svg: RenderedPreviewMode::Rendered,
            markdown: RenderedPreviewMode::Rendered,
        }
    }
}

impl RenderedPreviewModes {
    pub(super) fn get(self, kind: RenderedPreviewKind) -> RenderedPreviewMode {
        match kind {
            RenderedPreviewKind::Svg => self.svg,
            RenderedPreviewKind::Markdown => self.markdown,
        }
    }

    pub(super) fn set(&mut self, kind: RenderedPreviewKind, mode: RenderedPreviewMode) {
        match kind {
            RenderedPreviewKind::Svg => self.svg = mode,
            RenderedPreviewKind::Markdown => self.markdown = mode,
        }
    }
}

/// Preview mode for the conflict resolver merge-input pane.
///
/// When the conflicted file supports a rendered preview (for example, SVG or
/// markdown), the user can toggle between the normal text diff view and a
/// rendered preview of each conflict side.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ConflictResolverPreviewMode {
    /// Normal text/diff view with syntax highlighting.
    #[default]
    Text,
    /// Rendered preview (image for SVG files, rendered rows for markdown).
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

pub(super) fn preview_path_rendered_kind(path: &std::path::Path) -> Option<RenderedPreviewKind> {
    if is_svg_path(path) {
        Some(RenderedPreviewKind::Svg)
    } else if is_markdown_path(path) {
        Some(RenderedPreviewKind::Markdown)
    } else {
        None
    }
}

pub(super) fn diff_target_rendered_preview_kind(
    target: Option<&DiffTarget>,
) -> Option<RenderedPreviewKind> {
    let path = match target? {
        DiffTarget::WorkingTree { path, .. } => path.as_path(),
        DiffTarget::Commit {
            path: Some(path), ..
        } => path.as_path(),
        _ => return None,
    };
    preview_path_rendered_kind(path)
}

pub(super) fn main_diff_rendered_preview_toggle_kind(
    wants_file_diff: bool,
    is_file_preview: bool,
    preview_kind: Option<RenderedPreviewKind>,
) -> Option<RenderedPreviewKind> {
    match preview_kind? {
        RenderedPreviewKind::Svg if wants_file_diff => Some(RenderedPreviewKind::Svg),
        RenderedPreviewKind::Markdown if wants_file_diff || is_file_preview => {
            Some(RenderedPreviewKind::Markdown)
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PaneResizeHandle {
    Sidebar,
    Details,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PaneResizeState {
    pub(super) handle: PaneResizeHandle,
    pub(super) start_x: Pixels,
    pub(super) start_width: Pixels,
    pub(super) other_width: Pixels,
    pub(super) drag_delta_sign: f32,
    pub(super) bounds_total_w: Pixels,
    pub(super) bounds_sidebar_collapsed: bool,
    pub(super) bounds_details_collapsed: bool,
    pub(super) min_width: Pixels,
    pub(super) max_width: Pixels,
}

impl PaneResizeState {
    #[inline]
    pub(super) fn new(
        handle: PaneResizeHandle,
        start_x: Pixels,
        start_sidebar: Pixels,
        start_details: Pixels,
        total_w: Pixels,
        sidebar_collapsed: bool,
        details_collapsed: bool,
    ) -> Self {
        let (min_width, start_width, other_width, other_collapsed, drag_delta_sign) = match handle {
            PaneResizeHandle::Sidebar => (
                px(super::SIDEBAR_MIN_PX),
                start_sidebar,
                start_details,
                details_collapsed,
                1.0,
            ),
            PaneResizeHandle::Details => (
                px(super::DETAILS_MIN_PX),
                start_details,
                start_sidebar,
                sidebar_collapsed,
                -1.0,
            ),
        };
        let (_, max_width) = super::pane_resize_drag_width_bounds_for_other_pane(
            min_width,
            other_width,
            other_collapsed,
            total_w,
            sidebar_collapsed,
            details_collapsed,
        );
        Self {
            handle,
            start_x,
            start_width,
            other_width,
            drag_delta_sign,
            bounds_total_w: total_w,
            bounds_sidebar_collapsed: sidebar_collapsed,
            bounds_details_collapsed: details_collapsed,
            min_width,
            max_width,
        }
    }

    #[inline]
    pub(super) fn drag_width_bounds(
        &self,
        total_w: Pixels,
        sidebar_collapsed: bool,
        details_collapsed: bool,
    ) -> (Pixels, Pixels) {
        if self.bounds_total_w == total_w
            && self.bounds_sidebar_collapsed == sidebar_collapsed
            && self.bounds_details_collapsed == details_collapsed
        {
            (self.min_width, self.max_width)
        } else {
            let other_collapsed = match self.handle {
                PaneResizeHandle::Sidebar => details_collapsed,
                PaneResizeHandle::Details => sidebar_collapsed,
            };
            super::pane_resize_drag_width_bounds_for_other_pane(
                self.min_width,
                self.other_width,
                other_collapsed,
                total_w,
                sidebar_collapsed,
                details_collapsed,
            )
        }
    }
}

pub(super) use ResizeDragGhost as PaneResizeDragGhost;

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

pub(super) use ResizeDragGhost as DiffSplitResizeDragGhost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum AnnotateResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct AnnotateResizeState {
    pub(in crate::view) start_x: Pixels,
    pub(in crate::view) start_width: f32,
}

pub(in crate::view) use ResizeDragGhost as AnnotateResizeDragGhost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConflictVSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ConflictVSplitResizeState {
    pub(super) start_y: Pixels,
    pub(super) start_ratio: f32,
}

pub(super) use ResizeDragGhost as ConflictVSplitResizeDragGhost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusSectionResizeHandle {
    ChangeTrackingAndStaged,
    UntrackedAndUnstaged,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct StatusSectionResizeState {
    pub(super) handle: StatusSectionResizeHandle,
    pub(super) start_y: Pixels,
    pub(super) start_height: Pixels,
}

#[allow(unused_imports)]
pub(super) use ResizeDragGhost as StatusSectionResizeDragGhost;

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

pub(super) use ResizeDragGhost as ConflictHSplitResizeDragGhost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConflictDiffSplitResizeHandle {
    Divider,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ConflictDiffSplitResizeState {
    pub(super) start_x: Pixels,
    pub(super) start_ratio: f32,
}

pub(super) use ResizeDragGhost as ConflictDiffSplitResizeDragGhost;

#[cfg(test)]
mod resize_drag_ghost_tests {
    use super::{
        ConflictDiffSplitResizeDragGhost, ConflictHSplitResizeDragGhost,
        ConflictVSplitResizeDragGhost, DiffSplitResizeDragGhost, HistoryColResizeDragGhost,
        PaneResizeDragGhost, ResizeDragGhost, StatusSectionResizeDragGhost,
    };
    use std::any::TypeId;

    #[test]
    fn all_resize_drag_ghost_aliases_use_shared_type() {
        let shared = TypeId::of::<ResizeDragGhost>();

        assert_eq!(TypeId::of::<HistoryColResizeDragGhost>(), shared);
        assert_eq!(TypeId::of::<PaneResizeDragGhost>(), shared);
        assert_eq!(TypeId::of::<DiffSplitResizeDragGhost>(), shared);
        assert_eq!(TypeId::of::<ConflictVSplitResizeDragGhost>(), shared);
        assert_eq!(TypeId::of::<StatusSectionResizeDragGhost>(), shared);
        assert_eq!(TypeId::of::<ConflictHSplitResizeDragGhost>(), shared);
        assert_eq!(TypeId::of::<ConflictDiffSplitResizeDragGhost>(), shared);
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
    pub(super) source_visible_ix: usize,
    pub(super) region: DiffTextRegion,
    pub(super) offset: usize,
}

impl DiffTextPos {
    pub(super) fn cmp_key(self) -> (usize, u8, usize) {
        (self.source_visible_ix, self.region.order(), self.offset)
    }
}

pub(super) struct DiffTextHitbox {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) layout_key: u64,
    pub(super) source_visible_ix: usize,
    pub(super) text_start_offset: usize,
    pub(super) text_len: usize,
    pub(super) offset_map: Option<DiffTextOffsetMap>,
    pub(super) streamed_ascii_monospace_cell_width: Option<Pixels>,
}

#[derive(Clone, Debug)]
pub(super) struct DiffTextOffsetMap {
    pub(super) display_to_source: Arc<[usize]>,
    pub(super) source_to_display: Arc<[usize]>,
}

impl DiffTextOffsetMap {
    pub(super) fn display_len(&self) -> usize {
        self.display_to_source.len().saturating_sub(1)
    }

    pub(super) fn source_len(&self) -> usize {
        self.source_to_display.len().saturating_sub(1)
    }

    pub(super) fn source_offset_for_display(&self, offset: usize) -> usize {
        self.display_to_source
            .get(offset.min(self.display_len()))
            .copied()
            .unwrap_or_else(|| self.source_len())
    }

    pub(super) fn display_offset_for_source(&self, offset: usize) -> usize {
        self.source_to_display
            .get(offset.min(self.source_len()))
            .copied()
            .unwrap_or_else(|| self.display_len())
    }
}

#[derive(Clone)]
pub(super) struct ToastState {
    pub(super) id: u64,
    pub(super) kind: components::ToastKind,
    pub(super) input: Entity<components::TextInput>,
    pub(super) is_code_message: bool,
    pub(super) actions: Vec<ToastAction>,
    pub(super) dismiss_behavior: ToastDismissBehavior,
    pub(super) ttl: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ToastAction {
    OpenUrl {
        url: String,
        label: String,
    },
    OpenSurvey {
        survey_id: String,
        survey_name: String,
        url: String,
        label: String,
    },
    PostponeSurvey {
        survey_id: String,
        survey_name: String,
        postpone_seconds: u64,
        label: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum ToastDismissBehavior {
    #[default]
    Remove,
    PostponeSurvey {
        survey_id: String,
        survey_name: String,
        postpone_seconds: u64,
    },
}

#[derive(Clone, Debug)]
pub(super) struct CommitDetailsDelayState {
    pub(super) repo_id: RepoId,
    pub(super) commit_id: CommitId,
    pub(super) show_loading: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum StatusSection {
    CombinedUnstaged,
    Untracked,
    Unstaged,
    Staged,
}

impl StatusSection {
    pub(super) const fn diff_area(self) -> DiffArea {
        match self {
            Self::CombinedUnstaged | Self::Untracked | Self::Unstaged => DiffArea::Unstaged,
            Self::Staged => DiffArea::Staged,
        }
    }

    pub(super) const fn id_label(self) -> &'static str {
        match self {
            Self::CombinedUnstaged | Self::Unstaged => "unstaged",
            Self::Untracked => "untracked",
            Self::Staged => "staged",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StatusSectionFilter {
    All,
    UntrackedOnly,
    ExcludeUntracked,
}

#[derive(Clone)]
pub(super) struct StatusSectionEntries<'a> {
    entries: &'a [FileStatus],
    indexes: StatusSectionIndexes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StatusSectionIndexes {
    All,
    Filtered(Vec<usize>),
}

impl<'a> StatusSectionEntries<'a> {
    pub(super) fn from_repo(repo: &'a RepoState, section: StatusSection) -> Option<Self> {
        let (entries, filter) = match section {
            StatusSection::CombinedUnstaged => {
                (repo.worktree_status_entries()?, StatusSectionFilter::All)
            }
            StatusSection::Untracked => (
                repo.worktree_status_entries()?,
                StatusSectionFilter::UntrackedOnly,
            ),
            StatusSection::Unstaged => (
                repo.worktree_status_entries()?,
                StatusSectionFilter::ExcludeUntracked,
            ),
            StatusSection::Staged => (repo.staged_status_entries()?, StatusSectionFilter::All),
        };
        let indexes = match filter {
            StatusSectionFilter::All => StatusSectionIndexes::All,
            StatusSectionFilter::UntrackedOnly | StatusSectionFilter::ExcludeUntracked => {
                StatusSectionIndexes::Filtered(
                    entries
                        .iter()
                        .enumerate()
                        .filter_map(|(ix, entry)| {
                            status_section_filter_matches(filter, entry).then_some(ix)
                        })
                        .collect(),
                )
            }
        };
        Some(Self { entries, indexes })
    }

    pub(super) fn iter(&self) -> StatusSectionIter<'a, '_> {
        let inner = match &self.indexes {
            StatusSectionIndexes::All => StatusSectionIterInner::All(self.entries.iter()),
            StatusSectionIndexes::Filtered(indexes) => StatusSectionIterInner::Filtered {
                entries: self.entries,
                indexes: indexes.iter(),
            },
        };
        StatusSectionIter { inner }
    }

    pub(super) fn len(&self) -> usize {
        match &self.indexes {
            StatusSectionIndexes::All => self.entries.len(),
            StatusSectionIndexes::Filtered(indexes) => indexes.len(),
        }
    }

    pub(super) fn get(&self, index: usize) -> Option<&'a FileStatus> {
        match &self.indexes {
            StatusSectionIndexes::All => self.entries.get(index),
            StatusSectionIndexes::Filtered(indexes) => indexes
                .get(index)
                .and_then(|source_ix| self.entries.get(*source_ix)),
        }
    }

    pub(super) fn path_vec(&self) -> Vec<std::path::PathBuf> {
        self.iter().map(|entry| entry.path.clone()).collect()
    }

    pub(super) fn contains_path(&self, path: &std::path::Path) -> bool {
        self.iter().any(|entry| entry.path == path)
    }
}

pub(super) struct StatusSectionIter<'a, 'indexes> {
    inner: StatusSectionIterInner<'a, 'indexes>,
}

enum StatusSectionIterInner<'a, 'indexes> {
    All(std::slice::Iter<'a, FileStatus>),
    Filtered {
        entries: &'a [FileStatus],
        indexes: std::slice::Iter<'indexes, usize>,
    },
}

impl<'a, 'indexes> Iterator for StatusSectionIter<'a, 'indexes> {
    type Item = &'a FileStatus;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            StatusSectionIterInner::All(iter) => iter.next(),
            StatusSectionIterInner::Filtered { entries, indexes } => {
                indexes.next().and_then(|ix| entries.get(*ix))
            }
        }
    }
}

fn status_section_filter_matches(filter: StatusSectionFilter, entry: &FileStatus) -> bool {
    match filter {
        StatusSectionFilter::All => true,
        StatusSectionFilter::UntrackedOnly => entry.kind == FileStatusKind::Untracked,
        StatusSectionFilter::ExcludeUntracked => entry.kind != FileStatusKind::Untracked,
    }
}

pub(super) fn status_section_rev(repo: &RepoState, section: StatusSection) -> u64 {
    match section {
        StatusSection::Staged => repo.staged_status_cache_rev(),
        StatusSection::CombinedUnstaged | StatusSection::Untracked | StatusSection::Unstaged => {
            repo.worktree_status_cache_rev()
        }
    }
}

pub(super) fn status_section_is_loading(repo: &RepoState, section: StatusSection) -> bool {
    match section {
        StatusSection::Staged => repo.staged_status_is_loading(),
        StatusSection::CombinedUnstaged | StatusSection::Untracked | StatusSection::Unstaged => {
            repo.worktree_status_is_loading()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct StatusMultiSelection {
    pub(super) untracked: Vec<std::path::PathBuf>,
    pub(super) untracked_anchor: Option<std::path::PathBuf>,
    pub(super) unstaged: Vec<std::path::PathBuf>,
    pub(super) unstaged_anchor: Option<std::path::PathBuf>,
    pub(super) unstaged_anchor_index: Option<usize>,
    pub(super) unstaged_anchor_status_rev: Option<u64>,
    pub(super) staged: Vec<std::path::PathBuf>,
    pub(super) staged_anchor: Option<std::path::PathBuf>,
    pub(super) staged_anchor_index: Option<usize>,
    pub(super) staged_anchor_status_rev: Option<u64>,
}

impl StatusMultiSelection {
    pub(super) fn is_empty(&self) -> bool {
        self.untracked.is_empty() && self.unstaged.is_empty() && self.staged.is_empty()
    }

    pub(super) fn selected_paths_for_area(&self, area: DiffArea) -> &[std::path::PathBuf] {
        match area {
            DiffArea::Unstaged => {
                if !self.unstaged.is_empty() {
                    self.unstaged.as_slice()
                } else {
                    self.untracked.as_slice()
                }
            }
            DiffArea::Staged => self.staged.as_slice(),
        }
    }

    pub(super) fn selected_count_for_area(&self, area: DiffArea) -> usize {
        self.selected_paths_for_area(area).len()
    }

    pub(super) fn first_selected_for_area(&self, area: DiffArea) -> Option<&std::path::PathBuf> {
        self.selected_paths_for_area(area).first()
    }

    pub(super) fn take_selected_paths_for_area(self, area: DiffArea) -> Vec<std::path::PathBuf> {
        match area {
            DiffArea::Unstaged => {
                if !self.unstaged.is_empty() {
                    self.unstaged
                } else {
                    self.untracked
                }
            }
            DiffArea::Staged => self.staged,
        }
    }
}

#[cfg(test)]
pub(super) fn reconcile_status_multi_selection(
    selection: &mut StatusMultiSelection,
    status: &gitcomet_core::domain::RepoStatus,
) {
    let mut untracked_paths: HashSet<&std::path::Path> =
        HashSet::with_capacity_and_hasher(status.unstaged.len(), Default::default());
    let mut unstaged_paths: HashSet<&std::path::Path> =
        HashSet::with_capacity_and_hasher(status.unstaged.len(), Default::default());
    for entry in &status.unstaged {
        unstaged_paths.insert(entry.path.as_path());
        if entry.kind == FileStatusKind::Untracked {
            untracked_paths.insert(entry.path.as_path());
        }
    }

    selection
        .untracked
        .retain(|p| untracked_paths.contains(&p.as_path()));
    if selection
        .untracked_anchor
        .as_ref()
        .is_some_and(|a| !untracked_paths.contains(&a.as_path()))
    {
        selection.untracked_anchor = None;
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
        selection.unstaged_anchor_index = None;
        selection.unstaged_anchor_status_rev = None;
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
        selection.staged_anchor_index = None;
        selection.staged_anchor_status_rev = None;
    }
}

pub(super) fn reconcile_status_multi_selection_with_repo(
    selection: &mut StatusMultiSelection,
    repo: &RepoState,
) {
    if let Some(worktree) = repo.worktree_status_entries() {
        let mut untracked_paths: HashSet<&std::path::Path> =
            HashSet::with_capacity_and_hasher(worktree.len(), Default::default());
        let mut unstaged_paths: HashSet<&std::path::Path> =
            HashSet::with_capacity_and_hasher(worktree.len(), Default::default());
        for entry in worktree {
            unstaged_paths.insert(entry.path.as_path());
            if entry.kind == FileStatusKind::Untracked {
                untracked_paths.insert(entry.path.as_path());
            }
        }

        selection
            .untracked
            .retain(|p| untracked_paths.contains(&p.as_path()));
        if selection
            .untracked_anchor
            .as_ref()
            .is_some_and(|a| !untracked_paths.contains(&a.as_path()))
        {
            selection.untracked_anchor = None;
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
            selection.unstaged_anchor_index = None;
            selection.unstaged_anchor_status_rev = None;
        }
    }

    if let Some(staged) = repo.staged_status_entries() {
        let mut staged_paths: HashSet<&std::path::Path> =
            HashSet::with_capacity_and_hasher(staged.len(), Default::default());
        for entry in staged {
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
            selection.staged_anchor_index = None;
            selection.staged_anchor_status_rev = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ThreeWayColumn {
    Base,
    Ours,
    Theirs,
}

impl ThreeWayColumn {
    /// Index into `[base, ours, theirs]` arrays and the aligned map.
    pub(super) fn side_index(self) -> usize {
        match self {
            ThreeWayColumn::Base => 0,
            ThreeWayColumn::Ours => 1,
            ThreeWayColumn::Theirs => 2,
        }
    }

    pub(super) const ALL: [ThreeWayColumn; 3] = [
        ThreeWayColumn::Base,
        ThreeWayColumn::Ours,
        ThreeWayColumn::Theirs,
    ];
}

#[derive(Clone, Debug, Default)]
pub(super) struct ThreeWaySides<T> {
    pub(super) base: T,
    pub(super) ours: T,
    pub(super) theirs: T,
}

impl<T> std::ops::Index<ThreeWayColumn> for ThreeWaySides<T> {
    type Output = T;
    fn index(&self, side: ThreeWayColumn) -> &T {
        match side {
            ThreeWayColumn::Base => &self.base,
            ThreeWayColumn::Ours => &self.ours,
            ThreeWayColumn::Theirs => &self.theirs,
        }
    }
}

impl<T> std::ops::IndexMut<ThreeWayColumn> for ThreeWaySides<T> {
    fn index_mut(&mut self, side: ThreeWayColumn) -> &mut T {
        match side {
            ThreeWayColumn::Base => &mut self.base,
            ThreeWayColumn::Ours => &mut self.ours,
            ThreeWayColumn::Theirs => &mut self.theirs,
        }
    }
}

fn deferred_line_starts_for_text(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
    starts.push(0);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            starts.push(ix.saturating_add(1));
        }
    }
    starts
}

/// Lazily materialized line starts for one merge-input side.
///
/// Large conflict bootstrap only needs stable line counts up front. The full
/// byte-offset index is built on demand when a consumer actually needs random
/// line access for that side.
#[derive(Clone, Debug, Default)]
pub(super) struct DeferredLineStarts {
    line_count: usize,
    starts: std::sync::Arc<std::sync::OnceLock<std::sync::Arc<[usize]>>>,
}

impl DeferredLineStarts {
    pub(super) fn with_line_count(line_count: usize) -> Self {
        Self {
            line_count,
            starts: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub(super) fn line_count(&self) -> usize {
        self.line_count
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.line_count == 0
    }

    #[cfg(test)]
    pub(super) fn is_materialized(&self) -> bool {
        self.starts.get().is_some()
    }

    pub(super) fn starts<'a>(&'a self, text: &str) -> &'a [usize] {
        self.starts
            .get_or_init(|| std::sync::Arc::from(deferred_line_starts_for_text(text)))
            .as_ref()
    }

    pub(super) fn shared_starts(&self, text: &str) -> std::sync::Arc<[usize]> {
        std::sync::Arc::clone(
            self.starts
                .get_or_init(|| std::sync::Arc::from(deferred_line_starts_for_text(text))),
        )
    }

    fn materialized_with_count(line_starts: std::sync::Arc<[usize]>, line_count: usize) -> Self {
        let starts = std::sync::OnceLock::new();
        assert!(
            starts.set(line_starts).is_ok(),
            "fresh OnceLock should accept line starts"
        );
        Self {
            line_count,
            starts: std::sync::Arc::new(starts),
        }
    }
}

impl From<Vec<usize>> for DeferredLineStarts {
    fn from(starts: Vec<usize>) -> Self {
        let line_count = starts.len();
        Self::materialized_with_count(std::sync::Arc::from(starts), line_count)
    }
}

impl From<std::sync::Arc<[usize]>> for DeferredLineStarts {
    fn from(starts: std::sync::Arc<[usize]>) -> Self {
        let line_count = starts.len();
        Self::materialized_with_count(starts, line_count)
    }
}

pub(super) type LoadableMarkdownDoc =
    Loadable<Arc<crate::view::markdown_preview::MarkdownPreviewDocument>>;

pub(super) type LoadableMarkdownDiff =
    Loadable<Arc<crate::view::markdown_preview::MarkdownPreviewDiff>>;

pub(super) type LoadableImagePreview = Loadable<Option<Arc<gpui::Image>>>;

#[derive(Clone, Debug)]
pub(super) struct ConflictResolverMarkdownPreviewState {
    pub(super) source_hash: Option<u64>,
    pub(super) documents: ThreeWaySides<LoadableMarkdownDoc>,
}

impl Default for ConflictResolverMarkdownPreviewState {
    fn default() -> Self {
        Self {
            source_hash: None,
            documents: ThreeWaySides {
                base: Loadable::NotLoaded,
                ours: Loadable::NotLoaded,
                theirs: Loadable::NotLoaded,
            },
        }
    }
}

impl ConflictResolverMarkdownPreviewState {
    pub(super) fn document(&self, side: ThreeWayColumn) -> &LoadableMarkdownDoc {
        &self.documents[side]
    }
}

#[derive(Clone, Debug)]
pub(super) struct ConflictResolverImagePreviewState {
    pub(super) source_hash: Option<u64>,
    pub(super) path: Option<std::path::PathBuf>,
    pub(super) images: ThreeWaySides<LoadableImagePreview>,
}

impl Default for ConflictResolverImagePreviewState {
    fn default() -> Self {
        Self {
            source_hash: None,
            path: None,
            images: ThreeWaySides {
                base: Loadable::NotLoaded,
                ours: Loadable::NotLoaded,
                theirs: Loadable::NotLoaded,
            },
        }
    }
}

impl ConflictResolverImagePreviewState {
    pub(super) fn image(&self, side: ThreeWayColumn) -> &LoadableImagePreview {
        &self.images[side]
    }
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

/// Resolved-output outline metadata: per-line provenance, conflict markers, and source index.
/// Shared between visible state (`ConflictResolverUiState`) and incremental-recompute stash.
#[derive(Clone, Debug, Default)]
pub(super) struct ResolvedOutlineData {
    /// Per-line provenance metadata.
    pub(super) meta: Vec<conflict_resolver::ResolvedLineMeta>,
    /// Per-line conflict marker metadata for gutter markers.
    pub(super) markers: Vec<Option<ResolvedOutputConflictMarker>>,
    /// Source line keys currently represented in resolved output (for dedupe/plus-icon).
    pub(super) sources_index: HashSet<conflict_resolver::SourceLineKey>,
}

/// Mode-specific state for streamed (giant-file) conflict resolution.
///
/// Uses lazy paged access and span-based projections instead of
/// eagerly materializing all rows.
#[derive(Clone, Debug, Default)]
pub(super) struct StreamedConflictState {
    pub(super) three_way_visible_projection: conflict_resolver::ThreeWayVisibleProjection,
    pub(super) split_row_index: conflict_resolver::ConflictSplitRowIndex,
    pub(super) two_way_split_projection: conflict_resolver::TwoWaySplitProjection,
}

#[derive(Clone, Debug)]
pub(super) enum ConflictModeState {
    Streamed(StreamedConflictState),
}

impl Default for ConflictModeState {
    fn default() -> Self {
        Self::Streamed(StreamedConflictState::default())
    }
}

/// section 30 split: a drag selection of aligned rows within one conflict block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ConflictRowSelection {
    /// Visible conflict block the selection is anchored in.
    pub(super) conflict_ix: usize,
    /// Aligned row where the drag started.
    pub(super) anchor_row: usize,
    /// Aligned row under the cursor (clamped to the block).
    pub(super) head_row: usize,
    /// True while the drag is in progress.
    pub(super) selecting: bool,
}

impl ConflictRowSelection {
    /// Inclusive aligned-row range covered, normalized so start <= end.
    pub(super) fn row_range(&self) -> std::ops::RangeInclusive<usize> {
        let lo = self.anchor_row.min(self.head_row);
        let hi = self.anchor_row.max(self.head_row);
        lo..=hi
    }
}

/// KDiff3 manual diff help: lines marked in one source column, pending the
/// Ctrl+Y that pins them against the other columns' marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AlignmentLineSelection {
    /// Line where the mark started.
    pub(super) anchor: usize,
    /// Line last marked.
    pub(super) head: usize,
}

impl AlignmentLineSelection {
    /// Half-open line range covered, normalized so start <= end.
    pub(super) fn line_range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head) + 1
    }

    pub(super) fn contains(self, line: usize) -> bool {
        self.line_range().contains(&line)
    }
}

#[derive(Clone, Debug)]
pub(super) struct ConflictResolverUiState {
    pub(super) repo_id: Option<RepoId>,
    pub(super) path: Option<std::path::PathBuf>,
    pub(super) shared_path: Option<gitcomet_state::msg::RepoPath>,
    pub(super) loaded_file: Option<gitcomet_state::model::ConflictFile>,
    pub(super) conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(super) source_hash: Option<u64>,
    /// The editable output contains preserved worktree text whose conflict
    /// spans could not be mapped safely onto the stage projection.
    pub(super) output_is_protected: bool,
    /// Marker-backed geometry used for reset and source-region rendering.
    pub(super) current: Option<std::sync::Arc<str>>,
    pub(super) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    /// section 30 collapsed context mode: fold unchanged runs in the source columns.
    pub(super) collapse_context: bool,
    /// Per-fold reveal state for collapsed context mode, keyed by fold id.
    pub(super) context_fold_reveals:
        std::collections::HashMap<usize, conflict_resolver::ConflictFoldReveal>,
    /// section 30 collapsed context mode for the resolved output pane: fold
    /// projection in output line space. `None` ⇒ pass-through (one row per
    /// line). Rebuilt lazily after its inputs change.
    pub(super) resolved_output_visible: Option<conflict_resolver::ThreeWayVisibleProjection>,
    pub(super) resolved_output_visible_dirty: bool,
    /// Per-fold reveal state for resolved-output folds (output-line fold ids).
    pub(super) output_context_fold_reveals:
        std::collections::HashMap<usize, conflict_resolver::ConflictFoldReveal>,
    /// Mapping from visible block index to `ConflictSession` region index.
    pub(super) conflict_region_indices: Vec<usize>,
    /// Mapping from visible marker block index to its semantic merge-plan
    /// block. Empty for marker-only/fallback sessions.
    pub(super) display_plan_block_indices: Vec<usize>,
    /// Whether each raw session region includes a diff3 base marker. This is
    /// kept separate from display blocks, whose base may be populated from
    /// the shared ancestor for picking.
    pub(super) conflict_region_marker_has_base: Vec<bool>,
    /// Actionable conflict block currently selected in the displayed marker
    /// projection. Semantic targets without a displayed block leave this unset.
    pub(super) active_conflict: Option<usize>,
    /// Ordered semantic resolver navigation targets.
    pub(super) nav_targets: Vec<conflict_resolver::ConflictNavTarget>,
    /// Aligned source rows retained for every original session region before
    /// manual/automatic resolutions are materialized into plain display text.
    pub(super) original_region_aligned_ranges: Vec<Option<Range<usize>>>,
    pub(super) hovered_conflict: Option<(usize, ThreeWayColumn)>,
    /// section 30 split: in-progress or completed drag selection of aligned rows
    /// inside one conflict block, used to split that block at the selection
    /// boundary. Cleared whenever the conflict source rebuilds.
    pub(super) row_selection: Option<ConflictRowSelection>,
    /// KDiff3 manual diff help: lines marked per source column, independent of
    /// the block-scoped `row_selection` because a manual alignment exists
    /// precisely to pin lines the automatic alignment put in different blocks.
    pub(super) alignment_selection: ThreeWaySides<Option<AlignmentLineSelection>>,
    /// Streamed conflict state for the single conflict rendering/runtime path.
    pub(super) mode_state: ConflictModeState,
    pub(super) view_mode: ConflictResolverViewMode,
    /// Backing text for each three-way source side.
    pub(super) three_way_text: ThreeWaySides<SharedString>,
    /// Per-side line start offsets into `three_way_text`, materialized lazily.
    pub(super) three_way_line_starts: ThreeWaySides<DeferredLineStarts>,
    pub(super) three_way_len: usize,
    /// section 30 aligned row space: maps visual rows to per-side lines. Identity
    /// (row == line) when alignment is unavailable.
    pub(super) three_way_aligned: conflict_resolver::ThreeWayAlignedMap,
    /// kdiff3-style minimap column bands, in visible-row space. Empty when no
    /// alignment is available, which hides the column.
    pub(super) minimap_bands: Arc<[gitcomet_core::merge::MinimapRowKind]>,
    /// Exact merge-plan row ranges for the currently visible marker blocks.
    ///
    /// `None` is the legacy/current-only fallback where ranges must be
    /// estimated from marker text.
    pub(super) merge_plan_aligned_conflict_ranges: Option<Vec<Range<usize>>>,
    /// Whether the three-way visible projection/ranges have been built at
    /// least once for the current conflict source.
    pub(super) three_way_visible_state_ready: bool,
    /// Per-side conflict ranges for O(log n) binary-search lookups and
    /// conflict-to-visible mapping. The ours ranges remain the anchor space for
    /// legacy three-way visible projections.
    pub(super) three_way_conflict_ranges: ThreeWaySides<Vec<Range<usize>>>,
    /// Visible-row indices used to measure horizontal width for each three-way input column.
    pub(super) three_way_horizontal_measure_rows: [usize; 3],
    pub(super) conflict_has_base: Vec<bool>,
    /// Current choice for each conflict block, cached to avoid rebuilding it
    /// from `marker_segments` on every render.
    pub(super) conflict_choices: Vec<conflict_resolver::ConflictChoice>,
    /// Ignore-whitespace visual row kinds by two-way split source row.
    pub(super) two_way_split_visual_kind_cache:
        HashMap<usize, gitcomet_core::file_diff::FileDiffRowKind>,
    /// Visible-row indices used to measure horizontal width for the two-way split inputs.
    pub(super) two_way_horizontal_measure_rows: [usize; 2],
    pub(super) three_way_word_highlights: ThreeWaySides<conflict_resolver::WordHighlights>,
    /// Aligned two-way (ours↔theirs) word highlights keyed by aligned row,
    /// precomputed once per rebuild and shared by both diff columns.
    pub(super) two_way_aligned_word_highlights:
        FxHashMap<usize, conflict_resolver::TwoWayWordHighlightPair>,
    /// Bounded on-demand word highlights for giant block-local two-way rows.
    pub(super) two_way_split_word_highlight_cache:
        conflict_resolver::ConflictSplitWordHighlightCache,
    pub(super) nav_anchor: Option<conflict_resolver::ConflictNavAnchor>,
    pub(super) hide_resolved: bool,
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
    /// KDiff3-style report captured when this resolver file opened.
    ///
    /// This stays fixed while the user makes manual picks, so the toast
    /// describes the open-time state rather than a later live state.
    pub(super) open_summary_counts: Option<conflict_resolver::ConflictSummaryCounts>,
    /// True once the one-shot open-summary toast (total / auto-solved /
    /// unsolved, kdiff3-style) has been pushed for this resolver open.
    pub(super) open_summary_announced: bool,
    /// Tracks the last-seen `conflict_rev` from state so we can detect
    /// state-side session changes (e.g. hide-resolved, bulk picks, autosolve)
    /// that don't change the underlying file content.
    pub(super) conflict_rev: u64,
    /// Sequence token for debounced resolved-output outline recompute tasks.
    pub(super) resolver_pending_recompute_seq: u64,
    /// Resolved-output outline metadata (provenance, conflict markers, source index).
    pub(super) resolved_outline: ResolvedOutlineData,
    /// Cached per-line gutter render state for resolved-output preview rows.
    pub(super) resolved_outline_gutter_rows: Vec<conflict_resolver::ResolvedOutputGutterRow>,
    /// Conflict-to-output anchors used by scroll sync. Rebuilt lazily after
    /// either the source projection or resolved outline changes.
    pub(super) conflict_output_row_anchors: Arc<[(f32, f32)]>,
    pub(super) conflict_output_row_anchors_dirty: bool,
    /// Cached rendered markdown previews for the merge-input sides.
    pub(super) markdown_preview: ConflictResolverMarkdownPreviewState,
    /// Cached image previews for the merge-input sides.
    pub(super) image_preview: ConflictResolverImagePreviewState,
    /// Preview mode for the merge-input pane (Text vs rendered Preview).
    pub(super) resolver_preview_mode: ConflictResolverPreviewMode,
}

impl Default for ConflictResolverUiState {
    fn default() -> Self {
        Self {
            repo_id: None,
            path: None,
            shared_path: None,
            loaded_file: None,
            collapse_context: false,
            context_fold_reveals: std::collections::HashMap::default(),
            conflict_syntax_language: None,
            source_hash: None,
            output_is_protected: false,
            current: None,
            marker_segments: Vec::new(),
            conflict_region_indices: Vec::new(),
            display_plan_block_indices: Vec::new(),
            conflict_region_marker_has_base: Vec::new(),
            active_conflict: None,
            nav_targets: Vec::new(),
            original_region_aligned_ranges: Vec::new(),
            hovered_conflict: None,
            row_selection: None,
            alignment_selection: ThreeWaySides::default(),
            mode_state: ConflictModeState::default(),
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            three_way_text: ThreeWaySides::default(),
            three_way_line_starts: ThreeWaySides::default(),
            three_way_len: 0,
            three_way_aligned: conflict_resolver::ThreeWayAlignedMap::default(),
            minimap_bands: Arc::from([]),
            merge_plan_aligned_conflict_ranges: None,
            three_way_visible_state_ready: false,
            three_way_conflict_ranges: ThreeWaySides::default(),
            three_way_horizontal_measure_rows: [0; 3],
            conflict_has_base: Vec::new(),
            conflict_choices: Vec::new(),
            two_way_split_visual_kind_cache: HashMap::default(),
            two_way_horizontal_measure_rows: [0; 2],
            three_way_word_highlights: ThreeWaySides::default(),
            two_way_aligned_word_highlights: FxHashMap::default(),
            two_way_split_word_highlight_cache: Default::default(),
            nav_anchor: None,
            hide_resolved: false,
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: None,
            conflict_kind: None,
            last_autosolve_summary: None,
            open_summary_counts: None,
            open_summary_announced: false,
            conflict_rev: 0,
            resolver_pending_recompute_seq: 0,
            resolved_outline: ResolvedOutlineData::default(),
            resolved_outline_gutter_rows: Vec::new(),
            resolved_output_visible: None,
            resolved_output_visible_dirty: true,
            output_context_fold_reveals: std::collections::HashMap::default(),
            conflict_output_row_anchors: Arc::from([(0.0, 0.0)]),
            conflict_output_row_anchors_dirty: true,
            markdown_preview: ConflictResolverMarkdownPreviewState::default(),
            image_preview: ConflictResolverImagePreviewState::default(),
            resolver_preview_mode: ConflictResolverPreviewMode::default(),
        }
    }
}

fn indexed_line_text<'a>(text: &'a str, line_starts: &[usize], line_ix: usize) -> Option<&'a str> {
    if text.is_empty() {
        return None;
    }
    let text_len = text.len();
    let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
    if start >= text_len {
        return None;
    }
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    Some(text.get(start..end).unwrap_or(""))
}

fn append_conflict_row_without_whitespace(
    row: &gitcomet_core::file_diff::FileDiffRow,
    old_out: &mut String,
    new_out: &mut String,
) {
    use gitcomet_core::file_diff::FileDiffRowKind as RK;

    match row.kind {
        RK::Context => {}
        RK::Remove => {
            if let Some(text) = row.old.as_ref() {
                old_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
        }
        RK::Add => {
            if let Some(text) = row.new.as_ref() {
                new_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
        }
        RK::Modify => {
            if let Some(text) = row.old.as_ref() {
                old_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
            if let Some(text) = row.new.as_ref() {
                new_out.extend(text.as_ref().chars().filter(|ch| !ch.is_whitespace()));
            }
        }
    }
}

impl ConflictResolverUiState {
    pub(super) fn matches_target(&self, repo_id: RepoId, path: &std::path::Path) -> bool {
        self.repo_id == Some(repo_id) && self.path.as_deref() == Some(path)
    }

    pub(super) fn dispatch_path(&self) -> Option<gitcomet_state::msg::RepoPath> {
        self.shared_path.clone()
    }

    pub(super) fn selected_nav_target_index(&self) -> Option<usize> {
        let anchor = self.nav_anchor?;
        self.nav_targets
            .iter()
            .position(|target| target.id == anchor.id)
    }

    pub(super) fn nav_target_index_for_aligned_row(&self, row: usize) -> Option<usize> {
        self.nav_targets.iter().position(|target| {
            target
                .aligned_rows
                .as_ref()
                .is_some_and(|range| range.contains(&row))
        })
    }

    pub(super) fn selected_nav_target_contains_aligned_row(&self, row: usize) -> bool {
        self.selected_nav_target_index()
            .and_then(|index| self.nav_targets.get(index))
            .and_then(|target| target.aligned_rows.as_ref())
            .is_some_and(|range| range.contains(&row))
    }

    fn nav_target_matches_display(
        &self,
        target: &conflict_resolver::ConflictNavTarget,
        display_conflict_index: usize,
    ) -> bool {
        target.display_conflict_index == Some(display_conflict_index)
            || target.region_index.is_some_and(|region_index| {
                self.conflict_region_indices
                    .get(display_conflict_index)
                    .copied()
                    == Some(region_index)
            })
            || matches!(
                target.id,
                conflict_resolver::ConflictNavTargetId::DisplayBlock(index)
                    if index == display_conflict_index
            )
    }

    pub(super) fn select_nav_target(&mut self, target_index: usize) -> bool {
        let Some(target) = self.nav_targets.get(target_index) else {
            return false;
        };
        self.nav_anchor = Some(target.anchor());
        self.active_conflict = target.display_conflict_index;
        true
    }

    pub(super) fn select_display_conflict(&mut self, display_conflict_index: usize) -> bool {
        let Some(target_index) = self
            .nav_targets
            .iter()
            .position(|target| self.nav_target_matches_display(target, display_conflict_index))
        else {
            return false;
        };
        self.nav_anchor = Some(self.nav_targets[target_index].anchor());
        self.active_conflict = Some(display_conflict_index);
        true
    }

    pub(super) fn reconcile_nav_targets(
        &mut self,
        targets: Vec<conflict_resolver::ConflictNavTarget>,
    ) {
        let previous_targets = std::mem::replace(&mut self.nav_targets, targets);
        let previous_active = self.active_conflict;
        let selected = conflict_resolver::reconcile_conflict_nav_target_index(
            self.nav_anchor,
            &previous_targets,
            &self.nav_targets,
        );
        let Some(selected) = selected else {
            self.nav_anchor = None;
            self.active_conflict = None;
            return;
        };
        let target = &self.nav_targets[selected];
        self.nav_anchor = Some(target.anchor());
        self.active_conflict = previous_active
            .filter(|display| self.nav_target_matches_display(target, *display))
            .or(target.display_conflict_index);
    }

    pub(super) fn output_line_for_nav_target_provenance(
        &self,
        target: &conflict_resolver::ConflictNavTarget,
    ) -> Option<usize> {
        let aligned_rows = target.aligned_rows.as_ref()?;
        self.resolved_outline.meta.iter().find_map(|meta| {
            let side = match (self.view_mode, meta.source) {
                (ConflictResolverViewMode::ThreeWay, conflict_resolver::ResolvedLineSource::A) => {
                    ThreeWayColumn::Base
                }
                (ConflictResolverViewMode::ThreeWay, conflict_resolver::ResolvedLineSource::B) => {
                    ThreeWayColumn::Ours
                }
                (ConflictResolverViewMode::ThreeWay, conflict_resolver::ResolvedLineSource::C) => {
                    ThreeWayColumn::Theirs
                }
                (
                    ConflictResolverViewMode::TwoWayDiff,
                    conflict_resolver::ResolvedLineSource::A,
                ) => ThreeWayColumn::Ours,
                (
                    ConflictResolverViewMode::TwoWayDiff,
                    conflict_resolver::ResolvedLineSource::B,
                ) => ThreeWayColumn::Theirs,
                (
                    ConflictResolverViewMode::TwoWayDiff,
                    conflict_resolver::ResolvedLineSource::C,
                )
                | (_, conflict_resolver::ResolvedLineSource::Manual) => return None,
            };
            let source_line = usize::try_from(meta.input_line?).ok()?.checked_sub(1)?;
            let aligned_row = self.three_way_row_for_side_line(side, source_line);
            (aligned_rows.contains(&aligned_row)
                || (aligned_rows.is_empty() && aligned_rows.start == aligned_row))
                .then_some(meta.output_line as usize)
        })
    }

    pub(super) fn cached_loaded_file_for_target(
        &self,
        repo_id: RepoId,
        path: &std::path::Path,
    ) -> Option<&gitcomet_state::model::ConflictFile> {
        self.matches_target(repo_id, path)
            .then_some(self.loaded_file.as_ref())
            .flatten()
    }

    // ----- Mode accessors -----

    /// Return the rendering mode enum (for tracing / external APIs that expect it).
    #[cfg(test)]
    pub(super) fn rendering_mode(&self) -> conflict_resolver::ConflictRenderingMode {
        conflict_resolver::ConflictRenderingMode::StreamedLargeFile
    }

    /// Access the streamed conflict state.
    #[cfg(test)]
    #[track_caller]
    pub(super) fn streamed(&self) -> &StreamedConflictState {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s,
        }
    }

    /// Mutably access the streamed conflict state.
    #[cfg(test)]
    #[track_caller]
    pub(super) fn streamed_mut(&mut self) -> &mut StreamedConflictState {
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => s,
        }
    }

    pub(super) fn split_row_index(&self) -> Option<&conflict_resolver::ConflictSplitRowIndex> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => Some(&s.split_row_index),
        }
    }

    pub(super) fn two_way_split_projection(
        &self,
    ) -> Option<&conflict_resolver::TwoWaySplitProjection> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => Some(&s.two_way_split_projection),
        }
    }

    pub(super) fn three_way_visible_projection(
        &self,
    ) -> &conflict_resolver::ThreeWayVisibleProjection {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => &s.three_way_visible_projection,
        }
    }

    #[track_caller]
    #[allow(unused_variables)]
    pub(super) fn debug_assert_rendering_mode_invariants(&self) {}

    pub(super) fn three_way_line_count(&self, side: ThreeWayColumn) -> usize {
        self.three_way_line_starts[side].line_count()
    }

    pub(super) fn three_way_line_starts_ref(&self, side: ThreeWayColumn) -> &[usize] {
        self.three_way_line_starts[side].starts(self.three_way_text[side].as_ref())
    }

    pub(super) fn three_way_shared_line_starts(&self, side: ThreeWayColumn) -> Arc<[usize]> {
        self.three_way_line_starts[side].shared_starts(self.three_way_text[side].as_ref())
    }

    pub(super) fn three_way_line_text(&self, side: ThreeWayColumn, line_ix: usize) -> Option<&str> {
        indexed_line_text(
            &self.three_way_text[side],
            self.three_way_line_starts_ref(side),
            line_ix,
        )
    }

    /// The side line rendered at an aligned visual row (section 30 aligned row
    /// space), or `None` for padding rows.
    pub(super) fn three_way_side_line_for_row(
        &self,
        side: ThreeWayColumn,
        row: usize,
    ) -> Option<usize> {
        self.three_way_aligned
            .side_line_for_row(side.side_index(), row)
    }

    /// Text of the side line rendered at an aligned visual row; `None` for
    /// padding rows and rows past the side's end.
    pub(super) fn three_way_row_text(&self, side: ThreeWayColumn, row: usize) -> Option<&str> {
        let line_ix = self.three_way_side_line_for_row(side, row)?;
        self.three_way_line_text(side, line_ix)
    }

    /// section 30 R11 (kdiff3 change colours): whether the side columns can tint
    /// rows by their own change vs base — needs a real base and a
    /// non-identity alignment (both-added and unaligned files keep the
    /// marker-region tint).
    pub(super) fn three_way_per_side_change_rows(&self) -> bool {
        !self.three_way_aligned.is_identity() && !self.three_way_text.base.is_empty()
    }

    /// section 30 R11: whether `column`'s line at aligned `row` differs from the
    /// base line paired at the same row. A line on one side of a padding row
    /// counts as a change; the base column itself is never "changed".
    pub(super) fn three_way_row_differs_from_base(
        &self,
        column: ThreeWayColumn,
        row: usize,
    ) -> bool {
        if matches!(column, ThreeWayColumn::Base) {
            return false;
        }
        self.three_way_row_text(column, row) != self.three_way_row_text(ThreeWayColumn::Base, row)
    }

    /// The aligned visual row at which a side line renders.
    pub(super) fn three_way_row_for_side_line(&self, side: ThreeWayColumn, line: usize) -> usize {
        self.three_way_aligned
            .row_for_side_line(side.side_index(), line)
    }

    /// section 30 split: whether row selection / split is available for the current
    /// conflict. Requires a real aligned row space (so rows map consistently
    /// across columns) and a full-text resolver strategy on non-binary data.
    pub(super) fn conflict_row_selection_enabled(&self) -> bool {
        !self.three_way_aligned.is_identity()
            && !self.is_binary_conflict
            && self.strategy
                == Some(gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver)
    }

    /// section 30 split: whether aligned `row` is inside the current row selection
    /// (highlighted in every source column since rows are shared).
    pub(super) fn conflict_row_is_selected(&self, row: usize) -> bool {
        self.row_selection
            .is_some_and(|sel| sel.row_range().contains(&row))
    }

    /// KDiff3 manual diff help: whether the resolver can pin alignments at all.
    ///
    /// Shares the row-selection preconditions: a real aligned row space and a
    /// full-text resolver on non-binary data.
    pub(super) fn manual_alignment_enabled(&self) -> bool {
        self.conflict_row_selection_enabled()
    }

    /// Mark `line` in `column` for a manual alignment.
    ///
    /// `extend` grows the column's existing mark from its anchor; otherwise it
    /// starts a fresh single-line mark. Each column is marked independently —
    /// that is the whole point, since a manual alignment pins lines the
    /// automatic alignment placed on different rows.
    pub(super) fn set_alignment_selection(
        &mut self,
        column: ThreeWayColumn,
        line: usize,
        extend: bool,
    ) {
        let anchor = match self.alignment_selection[column] {
            Some(selection) if extend => selection.anchor,
            _ => line,
        };
        self.alignment_selection[column] = Some(AlignmentLineSelection { anchor, head: line });
    }

    /// Drop every pending alignment mark. Returns whether anything was marked.
    pub(super) fn clear_alignment_selections(&mut self) -> bool {
        let had_any = self.has_alignment_selection();
        self.alignment_selection = ThreeWaySides::default();
        had_any
    }

    pub(super) fn has_alignment_selection(&self) -> bool {
        ThreeWayColumn::ALL
            .iter()
            .any(|column| self.alignment_selection[*column].is_some())
    }

    /// Whether `line` of `column` carries a pending alignment mark.
    pub(super) fn alignment_line_is_selected(&self, column: ThreeWayColumn, line: usize) -> bool {
        self.alignment_selection[column].is_some_and(|selection| selection.contains(line))
    }

    /// Build the entry a Ctrl+Y would pin from the current marks.
    ///
    /// A column the user left unmarked still needs a position, or the entry
    /// could not be ordered against the others. The aligned row where the
    /// marked columns begin gives it one, and it pins an empty range there —
    /// "the marked lines align against nothing on this side", which is how a
    /// one-sided block gets forced.
    ///
    /// Returns `None` when nothing is marked or the plan cannot be pinned.
    pub(super) fn manual_alignment_from_selections(
        &self,
        has_base: bool,
    ) -> Option<gitcomet_core::merge::ManualAlignment> {
        if !self.manual_alignment_enabled() || !self.has_alignment_selection() {
            return None;
        }
        let anchor_row = ThreeWayColumn::ALL
            .iter()
            .filter_map(|column| {
                let selection = self.alignment_selection[*column]?;
                Some(
                    self.three_way_aligned
                        .aligned_range_for_side_range(column.side_index(), selection.line_range())
                        .start,
                )
            })
            .min()?;
        let range_for = |column: ThreeWayColumn| match self.alignment_selection[column] {
            Some(selection) => selection.line_range(),
            None => {
                let line = self
                    .three_way_aligned
                    .side_line_lower_bound(column.side_index(), anchor_row);
                line..line
            }
        };
        let base = if has_base {
            range_for(ThreeWayColumn::Base)
        } else {
            0..0
        };
        Some(gitcomet_core::merge::ManualAlignment::new(
            base,
            range_for(ThreeWayColumn::Ours),
            range_for(ThreeWayColumn::Theirs),
        ))
    }

    /// section 30 split: the shared aligned-row range of conflict block `conflict_ix`
    /// (all source columns share it after `rebuild_three_way_visible_state`).
    pub(super) fn three_way_block_aligned_range(
        &self,
        conflict_ix: usize,
    ) -> Option<std::ops::Range<usize>> {
        self.three_way_conflict_ranges[ThreeWayColumn::Ours]
            .get(conflict_ix)
            .cloned()
    }

    /// section 30 split: clamp aligned `row` into conflict block `conflict_ix`.
    pub(super) fn clamp_row_to_conflict_block(&self, conflict_ix: usize, row: usize) -> usize {
        match self.three_way_block_aligned_range(conflict_ix) {
            Some(range) if !range.is_empty() => row.clamp(range.start, range.end - 1),
            _ => row,
        }
    }

    /// section 30 split: convert a normalized row selection inside a conflict block
    /// into block-local per-side split boundaries and the target region index.
    /// Returns `None` when selection/split is unavailable, the selection is
    /// degenerate (covers the whole block or nothing), or the block maps to a
    /// non-unique session region.
    pub(super) fn split_boundaries_for_selection(
        &self,
    ) -> Option<(
        usize,
        gitcomet_core::conflict_session::ConflictRegionSplitBoundaries,
    )> {
        let selection = self.row_selection?;
        if !self.conflict_row_selection_enabled() {
            return None;
        }
        // Custom/manual resolutions can replace a raw region with display
        // text, shifting every later display-side range away from the
        // immutable source alignment. Only split while display blocks retain
        // a one-to-one, in-order mapping to raw session regions.
        if self.conflict_region_indices.len() != self.conflict_region_marker_has_base.len()
            || self
                .conflict_region_indices
                .iter()
                .enumerate()
                .any(|(block_index, &region_index)| block_index != region_index)
        {
            return None;
        }
        let conflict_ix = selection.conflict_ix;
        let block = self.three_way_block_aligned_range(conflict_ix)?;
        if block.is_empty() {
            return None;
        }
        let row_range = selection.row_range();
        let sel_start = (*row_range.start()).max(block.start);
        let sel_end_inclusive = (*row_range.end()).min(block.end - 1);
        if sel_start > sel_end_inclusive {
            return None;
        }
        // A selection covering the whole block cannot split it.
        if sel_start <= block.start && sel_end_inclusive >= block.end - 1 {
            return None;
        }

        let marker_block = self
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .nth(conflict_ix)?;
        let line_count = |text: &str| {
            if text.is_empty() {
                0
            } else {
                text.as_bytes()
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count()
                    + usize::from(!text.ends_with('\n'))
            }
        };

        let side_bounds = |side: usize| -> Option<([usize; 2], usize)> {
            // The aligned map is built from the actual staged sides. Its position
            // at the block's first aligned row remains correct when clean context
            // before this block exists on only one side; marker Text segments do
            // not retain enough information to reconstruct that position.
            let base = self
                .three_way_aligned
                .side_line_lower_bound(side, block.start);
            let b0 = self
                .three_way_aligned
                .side_line_lower_bound(side, sel_start)
                .saturating_sub(base);
            let b1 = self
                .three_way_aligned
                .side_line_lower_bound(side, sel_end_inclusive + 1)
                .saturating_sub(base);
            let len = match side {
                0 => line_count(marker_block.base.as_deref().unwrap_or_default()),
                1 => line_count(&marker_block.ours),
                2 => line_count(&marker_block.theirs),
                _ => return None,
            };
            let b0 = b0.min(len);
            let b1 = b1.clamp(b0, len);
            Some(([b0, b1], len))
        };

        let region_index = self.conflict_region_indices.get(conflict_ix).copied()?;
        if self
            .conflict_region_indices
            .iter()
            .filter(|&&index| index == region_index)
            .take(2)
            .count()
            != 1
        {
            return None;
        }
        let has_base = self
            .conflict_region_marker_has_base
            .get(region_index)
            .copied()?;
        let (ours, ours_len) = side_bounds(ThreeWayColumn::Ours.side_index())?;
        let (theirs, theirs_len) = side_bounds(ThreeWayColumn::Theirs.side_index())?;
        let base = if has_base {
            Some(side_bounds(ThreeWayColumn::Base.side_index())?)
        } else {
            None
        };

        // Alignment can contain padding/base-only rows that have no content in
        // the serialized marker block. Do not advertise a split unless the
        // selection owns at least one serialized line and leaves at least one
        // serialized line outside the new region.
        let selected_has_content = ours[0] < ours[1]
            || theirs[0] < theirs[1]
            || base.is_some_and(|(bounds, _)| bounds[0] < bounds[1]);
        let has_content_outside = ours[0] > 0
            || ours[1] < ours_len
            || theirs[0] > 0
            || theirs[1] < theirs_len
            || base.is_some_and(|(bounds, len)| bounds[0] > 0 || bounds[1] < len);
        if !selected_has_content || !has_content_outside {
            return None;
        }

        let boundaries = gitcomet_core::conflict_session::ConflictRegionSplitBoundaries {
            ours,
            theirs,
            base: base.map(|(bounds, _)| bounds),
        };
        Some((region_index, boundaries))
    }

    /// Whether two consecutive displayed marker blocks can be joined without
    /// crossing malformed marker-looking context. This mirrors the core
    /// surgery guard so an enabled menu item does not silently no-op.
    pub(super) fn conflict_blocks_have_joinable_context(
        &self,
        first_conflict_ix: usize,
        second_conflict_ix: usize,
    ) -> bool {
        if first_conflict_ix.checked_add(1) != Some(second_conflict_ix) {
            return false;
        }
        let markerish = |text: &str| {
            text.lines().any(|line| {
                line.starts_with("<<<<<<<")
                    || line.starts_with("=======")
                    || line.starts_with(">>>>>>>")
                    || line.starts_with("|||||||")
            })
        };
        let mut conflict_ix = 0usize;
        let mut between = false;
        for segment in &self.marker_segments {
            match segment {
                conflict_resolver::ConflictSegment::Block(_) => {
                    if conflict_ix == second_conflict_ix {
                        return between;
                    }
                    between = conflict_ix == first_conflict_ix;
                    conflict_ix = conflict_ix.saturating_add(1);
                }
                conflict_resolver::ConflictSegment::Text(text) if between => {
                    if markerish(text.as_str()) {
                        return false;
                    }
                }
                conflict_resolver::ConflictSegment::Text(_) => {}
            }
        }
        false
    }

    pub(super) fn three_way_has_line(&self, side: ThreeWayColumn, line_ix: usize) -> bool {
        self.three_way_line_text(side, line_ix).is_some()
    }

    /// Return source-pane text for a conflict pick choice at a global line index.
    ///
    /// This reads from the indexed merge-input texts directly so callers do not
    /// depend on eager diff rows or streamed page generation.
    pub(super) fn source_line_text_for_choice(
        &self,
        choice: conflict_resolver::ConflictChoice,
        line_ix: usize,
    ) -> Option<&str> {
        match choice {
            conflict_resolver::ConflictChoice::Base
                if self.view_mode == ConflictResolverViewMode::ThreeWay =>
            {
                self.three_way_line_text(ThreeWayColumn::Base, line_ix)
            }
            conflict_resolver::ConflictChoice::Ours => {
                self.three_way_line_text(ThreeWayColumn::Ours, line_ix)
            }
            conflict_resolver::ConflictChoice::Theirs => {
                self.three_way_line_text(ThreeWayColumn::Theirs, line_ix)
            }
            conflict_resolver::ConflictChoice::Base | conflict_resolver::ConflictChoice::Both => {
                None
            }
            _ => None,
        }
    }

    /// Look up the visible item at `visible_ix`, dispatching between the eager
    /// map (small files) and the span-based projection (giant files).
    pub(super) fn three_way_visible_item(
        &self,
        visible_ix: usize,
    ) -> Option<conflict_resolver::ThreeWayVisibleItem> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s.three_way_visible_projection.get(visible_ix),
        }
    }

    /// Number of visible rows in the three-way view.
    pub(super) fn three_way_visible_len(&self) -> usize {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s.three_way_visible_projection.len(),
        }
    }

    /// Look up the conflict index for a given line on a given side.
    /// Uses binary search on per-side ranges in giant mode, O(1) array lookup otherwise.
    pub(super) fn conflict_index_for_side_line(
        &self,
        side: ThreeWayColumn,
        line_ix: usize,
    ) -> Option<usize> {
        let ranges = &self.three_way_conflict_ranges[side];
        conflict_resolver::conflict_index_for_line(ranges, line_ix)
    }

    /// Find the visible index for a conflict range, using the projection in giant mode.
    pub(super) fn visible_index_for_conflict(&self, range_ix: usize) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.three_way_visible_projection.visible_index_for_conflict(
                    &self.three_way_conflict_ranges[ThreeWayColumn::Ours],
                    range_ix,
                )
            }
        }
    }

    /// Find the visible row for an aligned merge-plan row. Context hidden by a
    /// fold maps to the fold summary row.
    pub(super) fn visible_index_for_aligned_row(&self, row: usize) -> Option<usize> {
        self.three_way_visible_projection()
            .visible_index_for_source_line(row)
    }

    // ----- Two-way split dispatch (giant vs eager) -----

    /// section 30 aligned row space: whether the two-way view renders the shared
    /// aligned whole-file rows (full mode) instead of the block-local
    /// `ConflictSplitRowIndex` rows (giant files / sides not loaded).
    pub(super) fn two_way_uses_aligned_rows(&self) -> bool {
        !self.three_way_aligned.is_identity()
    }

    /// Number of visible rows in the two-way view (aligned or block-local).
    pub(super) fn two_way_visible_len(&self) -> usize {
        if self.two_way_uses_aligned_rows() {
            self.three_way_visible_len()
        } else {
            self.two_way_split_visible_len()
        }
    }

    /// Number of visible rows in the two-way split view.
    pub(super) fn two_way_split_visible_len(&self) -> usize {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s.two_way_split_projection.visible_len(),
        }
    }

    /// Retrieve a materialized split row for the given visible index,
    /// dispatching between the paged index (giant) and the eager `diff_rows`
    /// array (small).
    pub(super) fn two_way_split_visible_row(
        &self,
        visible_ix: usize,
    ) -> Option<conflict_resolver::TwoWaySplitVisibleRow> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                let (source_row_ix, conflict_ix) = s.two_way_split_projection.get(visible_ix)?;
                let row = s
                    .split_row_index
                    .row_at(&self.marker_segments, source_row_ix)?;
                Some(conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix,
                    row,
                    conflict_ix,
                })
            }
        }
    }

    /// Retrieve a split row by source row index (not visible index).
    pub(super) fn two_way_split_row_by_source(
        &self,
        row_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffRow> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.split_row_index.row_at(&self.marker_segments, row_ix)
            }
        }
    }

    pub(super) fn two_way_split_visual_kind_at(
        &mut self,
        row_ix: usize,
        row: &gitcomet_core::file_diff::FileDiffRow,
        whitespace_mode: DiffWhitespaceMode,
    ) -> gitcomet_core::file_diff::FileDiffRowKind {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        if whitespace_mode == DiffWhitespaceMode::Show || matches!(row.kind, RK::Context) {
            return row.kind;
        }

        if let Some(kind) = self.two_way_split_visual_kind_cache.get(&row_ix).copied() {
            return kind;
        }

        self.cache_two_way_split_visual_kind_run(row_ix);
        self.two_way_split_visual_kind_cache
            .get(&row_ix)
            .copied()
            .unwrap_or(row.kind)
    }

    fn cache_two_way_split_visual_kind_run(&mut self, row_ix: usize) {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        let mut start = row_ix;
        while start > 0 {
            let Some(prev) = self.two_way_split_row_by_source(start - 1) else {
                break;
            };
            if matches!(prev.kind, RK::Context) {
                break;
            }
            start -= 1;
        }

        let mut old_stripped = String::new();
        let mut new_stripped = String::new();
        let mut end = start;
        while let Some(next) = self.two_way_split_row_by_source(end) {
            if matches!(next.kind, RK::Context) {
                break;
            }
            append_conflict_row_without_whitespace(&next, &mut old_stripped, &mut new_stripped);
            end += 1;
        }

        if start == end {
            return;
        }

        if old_stripped == new_stripped {
            for ix in start..end {
                self.two_way_split_visual_kind_cache.insert(ix, RK::Context);
            }
            return;
        }

        for ix in start..end {
            if let Some(row) = self.two_way_split_row_by_source(ix) {
                self.two_way_split_visual_kind_cache.insert(ix, row.kind);
            }
        }
    }

    /// Find the first visible index for a conflict in two-way split view.
    pub(super) fn two_way_split_visible_ix_for_conflict(
        &self,
        conflict_ix: usize,
    ) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s
                .two_way_split_projection
                .visible_index_for_conflict(conflict_ix),
        }
    }

    /// Map a two-way split visible index back to its conflict index.
    #[cfg(test)]
    pub(super) fn two_way_split_conflict_ix_for_visible(&self, visible_ix: usize) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s
                .two_way_split_projection
                .get(visible_ix)
                .and_then(|(_, ci)| ci),
        }
    }

    /// Build unresolved conflict navigation entries for two-way split view.
    #[cfg(test)]
    pub(super) fn two_way_split_nav_entries(&self) -> Vec<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => {
                conflict_resolver::unresolved_conflict_indices(&self.marker_segments)
                    .into_iter()
                    .filter_map(|ci| s.two_way_split_projection.visible_index_for_conflict(ci))
                    .collect()
            }
        }
    }

    // ----- Unified two-way dispatch (aligned vs block-local) -----

    /// Build unresolved conflict navigation entries for the current two-way
    /// conflict diff view.
    #[cfg(test)]
    pub(super) fn two_way_nav_entries(&self) -> Vec<usize> {
        if self.two_way_uses_aligned_rows() {
            return conflict_resolver::unresolved_conflict_indices(&self.marker_segments)
                .into_iter()
                .filter_map(|ci| self.visible_index_for_conflict(ci))
                .collect();
        }
        self.two_way_split_nav_entries()
    }

    /// Map a two-way visible index to its conflict index.
    #[cfg(test)]
    pub(super) fn two_way_conflict_ix_for_visible(&self, visible_ix: usize) -> Option<usize> {
        if self.two_way_uses_aligned_rows() {
            return match self.three_way_visible_item(visible_ix)? {
                conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(ri) => Some(ri),
                conflict_resolver::ThreeWayVisibleItem::Line(row) => {
                    // Conflict ranges are aligned-row ranges shared by all
                    // columns, so any side works for the lookup.
                    self.conflict_index_for_side_line(ThreeWayColumn::Ours, row)
                }
                conflict_resolver::ThreeWayVisibleItem::CollapsedContext { .. } => None,
            };
        }
        self.two_way_split_conflict_ix_for_visible(visible_ix)
    }

    /// Find the first visible index for a conflict in the current two-way diff
    /// view.
    pub(super) fn two_way_visible_ix_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
        if self.two_way_uses_aligned_rows() {
            return self.visible_index_for_conflict(conflict_ix);
        }
        self.two_way_split_visible_ix_for_conflict(conflict_ix)
    }

    /// Return (diff_row_count, inline_row_count) for trace recording.
    pub(super) fn two_way_row_counts(&self) -> (usize, usize) {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => (s.split_row_index.total_rows(), 0),
        }
    }

    pub(super) fn three_way_horizontal_measure_row(&self, side: ThreeWayColumn) -> usize {
        match side {
            ThreeWayColumn::Base => self.three_way_horizontal_measure_rows[0],
            ThreeWayColumn::Ours => self.three_way_horizontal_measure_rows[1],
            ThreeWayColumn::Theirs => self.three_way_horizontal_measure_rows[2],
        }
    }

    pub(super) fn two_way_horizontal_measure_row(
        &self,
        side: conflict_resolver::ConflictPickSide,
    ) -> usize {
        // Aligned two-way rows share the three-way row space, so the
        // three-way per-column measurements apply directly.
        if self.two_way_uses_aligned_rows() {
            return match side {
                conflict_resolver::ConflictPickSide::Ours => {
                    self.three_way_horizontal_measure_row(ThreeWayColumn::Ours)
                }
                conflict_resolver::ConflictPickSide::Theirs => {
                    self.three_way_horizontal_measure_row(ThreeWayColumn::Theirs)
                }
            };
        }
        match side {
            conflict_resolver::ConflictPickSide::Ours => self.two_way_horizontal_measure_rows[0],
            conflict_resolver::ConflictPickSide::Theirs => self.two_way_horizontal_measure_rows[1],
        }
    }

    fn refresh_three_way_horizontal_measure_rows(&mut self) {
        self.three_way_horizontal_measure_rows = self.compute_three_way_horizontal_measure_rows();
    }

    fn refresh_two_way_horizontal_measure_rows(&mut self) {
        self.two_way_horizontal_measure_rows = self.compute_two_way_horizontal_measure_rows();
    }

    fn compute_three_way_horizontal_measure_rows(&self) -> [usize; 3] {
        let has_hidden_resolved_blocks = self.hide_resolved
            && self.marker_segments.iter().any(|segment| {
                matches!(
                    segment,
                    conflict_resolver::ConflictSegment::Block(block) if block.resolved
                )
            });
        if self.collapse_context || has_hidden_resolved_blocks {
            // This helper already returns indices in the compact visible
            // projection. Mapping them as side-line indices would apply the
            // alignment a second time and can select an unrelated row. Context
            // folding likewise changes visible indices even without a hidden
            // resolved block.
            return self.compute_three_way_horizontal_measure_rows_from_visible_projection();
        }

        let rows = self.compute_three_way_horizontal_measure_side_lines();
        // The scan yields indices in each stage's own text; width measurement
        // wants their corresponding aligned rows.
        [
            self.three_way_row_for_side_line(ThreeWayColumn::Base, rows[0]),
            self.three_way_row_for_side_line(ThreeWayColumn::Ours, rows[1]),
            self.three_way_row_for_side_line(ThreeWayColumn::Theirs, rows[2]),
        ]
    }

    fn compute_three_way_horizontal_measure_side_lines(&self) -> [usize; 3] {
        // Marker text is the merge result, not any one index stage. Clean
        // changes outside conflict markers can therefore add or remove lines
        // on only one side. Walking marker segments and advancing all three
        // counters together produces invalid stage coordinates (and can make
        // a column measure a short row instead of its widest row). Scan each
        // actual stage text independently instead.
        [
            conflict_resolver::scan_text_line_stats(self.three_way_text.base.as_ref())
                .widest_line()
                .map_or(0, |(line_ix, _)| line_ix),
            conflict_resolver::scan_text_line_stats(self.three_way_text.ours.as_ref())
                .widest_line()
                .map_or(0, |(line_ix, _)| line_ix),
            conflict_resolver::scan_text_line_stats(self.three_way_text.theirs.as_ref())
                .widest_line()
                .map_or(0, |(line_ix, _)| line_ix),
        ]
    }

    fn compute_three_way_horizontal_measure_rows_from_visible_projection(&self) -> [usize; 3] {
        let mut best_rows = [0usize; 3];
        let mut best_lens = [0usize; 3];

        for span in self.three_way_visible_projection().spans() {
            let conflict_resolver::ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } = *span
            else {
                continue;
            };

            for offset in 0..len {
                let visible_ix = visible_start + offset;
                let line_ix = source_line_start + offset;

                for (slot, side) in [
                    ThreeWayColumn::Base,
                    ThreeWayColumn::Ours,
                    ThreeWayColumn::Theirs,
                ]
                .into_iter()
                .enumerate()
                {
                    let width = self.three_way_row_text(side, line_ix).map_or(0, str::len);
                    if width > best_lens[slot] {
                        best_lens[slot] = width;
                        best_rows[slot] = visible_ix;
                    }
                }
            }
        }

        best_rows
    }

    fn compute_two_way_horizontal_measure_rows(&self) -> [usize; 2] {
        let Some(split_row_index) = self.split_row_index() else {
            return [0; 2];
        };
        let Some(projection) = self.two_way_split_projection() else {
            return [0; 2];
        };

        let [ours_source_row, theirs_source_row] = split_row_index
            .widest_source_rows_by_text_len(&self.marker_segments, self.hide_resolved);

        [
            ours_source_row
                .and_then(|row_ix| projection.source_to_visible(row_ix))
                .unwrap_or(0),
            theirs_source_row
                .and_then(|row_ix| projection.source_to_visible(row_ix))
                .unwrap_or(0),
        ]
    }

    /// Pre-computed word highlights for a source row in the two-way split view.
    /// Return an already-computed giant-mode word highlight pair.
    pub(super) fn two_way_split_word_highlight(
        &self,
        row_ix: usize,
    ) -> Option<Arc<conflict_resolver::TwoWayWordHighlightPair>> {
        self.two_way_split_word_highlight_cache.get(row_ix)
    }

    /// Cache a giant-mode word highlight pair so the other split column and
    /// later frames reuse the same word diff.
    pub(super) fn cache_two_way_split_word_highlight(
        &mut self,
        row_ix: usize,
        highlights: conflict_resolver::TwoWayWordHighlightPair,
    ) -> Arc<conflict_resolver::TwoWayWordHighlightPair> {
        self.two_way_split_word_highlight_cache
            .insert(row_ix, highlights)
    }

    pub(super) fn two_way_split_word_highlight_for_row(
        &mut self,
        row_ix: usize,
        row: &gitcomet_core::file_diff::FileDiffRow,
    ) -> Option<Arc<conflict_resolver::TwoWayWordHighlightPair>> {
        self.two_way_split_word_highlight(row_ix).or_else(|| {
            conflict_resolver::compute_word_highlights_for_row(row)
                .map(|highlights| self.cache_two_way_split_word_highlight(row_ix, highlights))
        })
    }

    /// Rebuild three-way visible state (conflict maps + visible map/projection)
    /// from current marker segments and line counts.
    pub(super) fn rebuild_three_way_visible_state(&mut self) {
        let maps = conflict_resolver::build_three_way_conflict_maps_without_line_maps(
            &self.marker_segments,
            self.three_way_line_count(ThreeWayColumn::Base),
            self.three_way_line_count(ThreeWayColumn::Ours),
            self.three_way_line_count(ThreeWayColumn::Theirs),
        );
        let block_count = maps.conflict_ranges[1].len();
        let exact_plan_ranges = self
            .merge_plan_aligned_conflict_ranges
            .as_ref()
            .filter(|ranges| {
                ranges.len() == block_count
                    && ranges
                        .iter()
                        .all(|range| range.start <= range.end && range.end <= self.three_way_len)
                    && ranges.windows(2).all(|pair| pair[0].end <= pair[1].start)
            })
            .cloned();
        let aligned_ranges = exact_plan_ranges.unwrap_or_else(|| {
            // Legacy/current-only fallback: project marker-text offsets back
            // through the side alignment. Marker text is output space rather
            // than source space, so this is necessarily an estimate.
            conflict_resolver::project_conflict_ranges_to_aligned_rows(
                &self.marker_segments,
                &self.three_way_aligned,
                [
                    self.three_way_line_count(ThreeWayColumn::Base),
                    self.three_way_line_count(ThreeWayColumn::Ours),
                    self.three_way_line_count(ThreeWayColumn::Theirs),
                ],
            )
        });
        let three_way_visible_projection =
            conflict_resolver::build_three_way_visible_projection_with_options(
                self.three_way_len,
                &aligned_ranges,
                &maps.conflict_resolved,
                conflict_resolver::ThreeWayVisibleOptions {
                    hide_resolved: self.hide_resolved,
                    collapse_context: self.collapse_context,
                    context_fold_reveals: Some(&self.context_fold_reveals),
                },
            );
        self.apply_three_way_conflict_maps(maps);
        // All columns share the aligned conflict ranges.
        self.three_way_conflict_ranges = ThreeWaySides {
            base: aligned_ranges.clone(),
            ours: aligned_ranges.clone(),
            theirs: aligned_ranges,
        };
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.three_way_visible_projection = three_way_visible_projection;
            }
        }
        self.three_way_visible_state_ready = true;
        self.conflict_output_row_anchors_dirty = true;
        self.refresh_three_way_horizontal_measure_rows();
        self.rebuild_minimap_bands();
    }

    /// Recompute the minimap column's bands for the current projection.
    ///
    /// Runs from `rebuild_three_way_visible_state`, after the aligned conflict
    /// ranges are in place, so a pick recolors the band it settles.
    pub(super) fn rebuild_minimap_bands(&mut self) {
        let projection = match &self.mode_state {
            ConflictModeState::Streamed(s) => &s.three_way_visible_projection,
        };
        let resolved =
            conflict_resolver::resolved_conflict_flags_from_segments(&self.marker_segments);
        self.minimap_bands = conflict_resolver::build_minimap_bands(
            &self.three_way_aligned,
            projection,
            &self.three_way_conflict_ranges[ThreeWayColumn::Ours],
            &resolved,
            conflict_resolver::CONFLICT_BOTTOM_OVERSCROLL_ROWS,
        )
        .into();
    }

    /// Whether the minimap column has anything to show.
    pub(super) fn has_minimap(&self) -> bool {
        !self.minimap_bands.is_empty()
    }

    /// Rebuild two-way visible state from current marker segments.
    /// Rebuilds the streamed split row index and projection.
    pub(super) fn rebuild_two_way_visible_state(&mut self) {
        self.two_way_split_visual_kind_cache.clear();
        self.two_way_split_word_highlight_cache.clear();
        let ConflictModeState::Streamed(s) = &mut self.mode_state;
        s.split_row_index = conflict_resolver::ConflictSplitRowIndex::new(
            &self.marker_segments,
            conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        self.rebuild_two_way_visible_projections();
    }

    /// Rebuild streamed two-way visible projections from the current split-row index.
    pub(super) fn rebuild_two_way_visible_projections(&mut self) {
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.two_way_split_projection = conflict_resolver::TwoWaySplitProjection::new(
                    &s.split_row_index,
                    &self.marker_segments,
                    self.hide_resolved,
                );
            }
        }
        self.conflict_output_row_anchors_dirty = true;
        self.debug_assert_rendering_mode_invariants();
        self.refresh_two_way_horizontal_measure_rows();
    }

    /// Apply three-way conflict maps to state fields.
    pub(super) fn apply_three_way_conflict_maps(
        &mut self,
        maps: conflict_resolver::ThreeWayConflictMaps,
    ) {
        let [base_ranges, ours_ranges, theirs_ranges] = maps.conflict_ranges;
        self.three_way_conflict_ranges = ThreeWaySides {
            base: base_ranges,
            ours: ours_ranges,
            theirs: theirs_ranges,
        };
        self.conflict_has_base = maps.conflict_has_base;
        self.refresh_conflict_choices_from_segments();
    }

    pub(super) fn refresh_conflict_has_base_from_segments(&mut self) {
        self.conflict_has_base = self
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.base.is_some()),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
        self.refresh_conflict_choices_from_segments();
    }

    pub(super) fn refresh_conflict_choices_from_segments(&mut self) {
        self.conflict_choices = self
            .marker_segments
            .iter()
            .filter_map(|segment| match segment {
                conflict_resolver::ConflictSegment::Block(block) => Some(block.choice),
                conflict_resolver::ConflictSegment::Text(_) => None,
            })
            .collect();
    }

    pub(super) fn has_three_way_visible_state_ready(&self) -> bool {
        self.three_way_visible_state_ready
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default, clippy::single_range_in_vec_init)]
mod conflict_resolver_ui_state_tests {
    use super::{
        ConflictResolverUiState, ConflictRowSelection, DeferredLineStarts, DiffWhitespaceMode,
        Loadable, ThreeWayColumn, ThreeWaySides,
    };
    use crate::view::conflict_resolver::{
        self, ConflictBlock, ConflictChoice, ConflictNavTarget, ConflictNavTargetId,
        ConflictResolverViewMode, ConflictSegment, ConflictSplitRowIndex, ResolvedLineMeta,
        ResolvedLineSource, ThreeWayVisibleItem, TwoWaySplitProjection,
    };

    #[test]
    fn default_groups_three_way_side_fields() {
        let state = ConflictResolverUiState::default();

        assert!(state.three_way_text.base.is_empty());
        assert!(state.three_way_text.ours.is_empty());
        assert!(state.three_way_text.theirs.is_empty());
        assert!(state.rendering_mode().is_streamed_large_file());
        assert!(state.three_way_line_starts.base.is_empty());
        assert!(state.three_way_line_starts.ours.is_empty());
        assert!(state.three_way_line_starts.theirs.is_empty());
        assert!(state.three_way_conflict_ranges.base.is_empty());
        assert!(state.three_way_word_highlights.base.is_empty());
        assert!(state.split_row_index().is_some());
        assert!(state.two_way_split_projection().is_some());
        assert!(matches!(
            state.markdown_preview.documents.base,
            Loadable::NotLoaded
        ));
    }

    #[test]
    fn three_way_sides_keep_each_column_separate() {
        let mut sides = ThreeWaySides {
            base: vec![1],
            ours: vec![2],
            theirs: vec![3],
        };

        sides.base.push(10);
        sides.ours.push(20);
        sides.theirs.push(30);

        assert_eq!(sides.base, vec![1, 10]);
        assert_eq!(sides.ours, vec![2, 20]);
        assert_eq!(sides.theirs, vec![3, 30]);
    }

    #[test]
    fn three_way_sides_index_by_column() {
        let mut sides = ThreeWaySides {
            base: 10,
            ours: 20,
            theirs: 30,
        };

        assert_eq!(sides[ThreeWayColumn::Base], 10);
        assert_eq!(sides[ThreeWayColumn::Ours], 20);
        assert_eq!(sides[ThreeWayColumn::Theirs], 30);

        sides[ThreeWayColumn::Ours] = 42;
        assert_eq!(sides.ours, 42);
    }

    #[test]
    fn source_line_text_for_choice_reads_two_way_inputs_from_indexed_text() {
        let mut state = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            ..Default::default()
        };
        state.three_way_text.ours = "o0\no1\n".into();
        state.three_way_text.theirs = "t0\nt1\n".into();
        state.three_way_line_starts.ours = vec![0, 3].into();
        state.three_way_line_starts.theirs = vec![0, 3].into();

        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Ours, 1),
            Some("o1")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Theirs, 0),
            Some("t0")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Base, 0),
            None
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Both, 0),
            None
        );
    }

    #[test]
    fn source_line_text_for_choice_reads_base_only_in_three_way_mode() {
        let mut state = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::ThreeWay,
            ..Default::default()
        };
        state.three_way_text.base = "b0\nb1\n".into();
        state.three_way_text.ours = "o0\no1\n".into();
        state.three_way_text.theirs = "t0\nt1\n".into();
        state.three_way_line_starts.base = vec![0, 3].into();
        state.three_way_line_starts.ours = vec![0, 3].into();
        state.three_way_line_starts.theirs = vec![0, 3].into();

        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Base, 1),
            Some("b1")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Ours, 0),
            Some("o0")
        );
        assert_eq!(
            state.source_line_text_for_choice(ConflictChoice::Theirs, 1),
            Some("t1")
        );
    }

    #[test]
    fn apply_three_way_conflict_maps_distributes_ranges_and_flags() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".into()),
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
            whitespace_only: false,
        })];
        let maps = conflict_resolver::ThreeWayConflictMaps {
            conflict_ranges: [vec![0..3], vec![0..5], vec![0..4]],
            line_conflict_maps: [vec![Some(0); 3], vec![Some(0); 5], vec![Some(0); 4]],
            conflict_has_base: vec![true],
            conflict_resolved: vec![true],
        };
        state.apply_three_way_conflict_maps(maps.clone());

        assert_eq!(
            state.three_way_conflict_ranges.base,
            maps.conflict_ranges[0]
        );
        assert_eq!(
            state.three_way_conflict_ranges.ours,
            maps.conflict_ranges[1]
        );
        assert_eq!(
            state.three_way_conflict_ranges.theirs,
            maps.conflict_ranges[2]
        );
        assert_eq!(state.conflict_has_base, maps.conflict_has_base);
        assert_eq!(state.conflict_choices, vec![ConflictChoice::Theirs]);
    }

    #[test]
    fn merge_plan_ranges_override_marker_output_offset_estimates() {
        let block = |ours: &str, theirs: &str| {
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: ours.into(),
                theirs: theirs.into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            })
        };
        let exact_ranges = vec![1..3, 4..5, 6..8];
        let mut state = ConflictResolverUiState {
            // These text segments are merged-output projections whose line
            // counts do not represent positions in both immutable sources.
            marker_segments: vec![
                ConflictSegment::Text("one-sided resolved output\n".into()),
                block("local-a\nlocal-b\n", "remote-a\n"),
                ConflictSegment::Text("another selected-side line\n".into()),
                block("local-c\n", "remote-c\nremote-extra\n"),
                ConflictSegment::Text("selected output before final block\n".into()),
                block("local-d\nlocal-e\n", "remote-d\n"),
            ],
            three_way_len: 9,
            merge_plan_aligned_conflict_ranges: Some(exact_ranges.clone()),
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        assert_eq!(state.three_way_conflict_ranges.base, exact_ranges);
        assert_eq!(
            state.three_way_conflict_ranges.ours,
            state.three_way_conflict_ranges.base
        );
        assert_eq!(
            state.three_way_conflict_ranges.theirs,
            state.three_way_conflict_ranges.base
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 1),
            Some(0)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 3),
            None
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 4),
            Some(1)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 7),
            Some(2)
        );
    }

    #[test]
    fn refresh_conflict_has_base_from_segments_refreshes_choice_cache() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours\n".into(),
                theirs: "theirs\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".into()),
                ours: "ours2\n".into(),
                theirs: "theirs2\n".into(),
                choice: ConflictChoice::Both,
                resolved: true,
                whitespace_only: false,
            }),
        ];

        state.refresh_conflict_has_base_from_segments();

        assert_eq!(state.conflict_has_base, vec![false, true]);
        assert_eq!(
            state.conflict_choices,
            vec![ConflictChoice::Ours, ConflictChoice::Both]
        );
    }

    #[test]
    fn ignored_whitespace_visual_kind_caches_entire_change_run() {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "let x = 1\nabc  \n".into(),
            theirs: "let x=1\nabc\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        state.rebuild_two_way_visible_state();

        let first_row = state.two_way_split_row_by_source(0).unwrap();
        assert_eq!(
            state.two_way_split_visual_kind_at(0, &first_row, DiffWhitespaceMode::Ignore),
            RK::Context
        );

        assert_eq!(state.two_way_split_visual_kind_cache.len(), 2);
        assert_eq!(
            state.two_way_split_visual_kind_cache.get(&1).copied(),
            Some(RK::Context)
        );
    }

    #[test]
    fn giant_two_way_word_highlights_are_shared_between_column_renders() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "let local_name = value;\n".into(),
            theirs: "let remote_name = value;\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        state.rebuild_two_way_visible_state();
        let row = state.two_way_split_row_by_source(0).unwrap();

        let left = state
            .two_way_split_word_highlight_for_row(0, &row)
            .expect("modified row should have word highlights");
        let right = state
            .two_way_split_word_highlight_for_row(0, &row)
            .expect("second column should reuse word highlights");

        assert!(std::sync::Arc::ptr_eq(&left, &right));
    }

    #[test]
    fn rebuild_three_way_visible_state_streamed_mode() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        state.three_way_text.ours = "a\nb\n".into();
        state.three_way_text.theirs = "c\n".into();
        state.three_way_line_starts.ours = vec![0, 2].into();
        state.three_way_line_starts.theirs = vec![0].into();
        state.three_way_len = 2;

        state.rebuild_three_way_visible_state();

        assert!(state.streamed().three_way_visible_projection.len() > 0);
        assert_eq!(
            state.three_way_visible_len(),
            state.streamed().three_way_visible_projection.len()
        );
        assert!(!state.three_way_conflict_ranges.ours.is_empty());
    }

    #[test]
    fn three_way_measure_rows_do_not_materialize_deferred_line_starts() {
        let mut state = ConflictResolverUiState::default();
        let base_text = "ctx\nbase 1234567890\nend\n";
        let ours_text = "ctx\nours abcdefghij\nend\n";
        let theirs_text = "ctx\ntheirs klmnopqrstuv\nend\n";

        state.marker_segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base 1234567890\n".into()),
                ours: "ours abcdefghij\n".into(),
                theirs: "theirs klmnopqrstuv\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
            ConflictSegment::Text("end\n".into()),
        ];
        state.three_way_text = ThreeWaySides {
            base: base_text.into(),
            ours: ours_text.into(),
            theirs: theirs_text.into(),
        };
        state.three_way_line_starts = ThreeWaySides {
            base: DeferredLineStarts::with_line_count(3),
            ours: DeferredLineStarts::with_line_count(3),
            theirs: DeferredLineStarts::with_line_count(3),
        };
        state.three_way_len = 3;

        state.rebuild_three_way_visible_state();

        assert_eq!(
            state.three_way_horizontal_measure_row(ThreeWayColumn::Base),
            1
        );
        assert_eq!(
            state.three_way_horizontal_measure_row(ThreeWayColumn::Ours),
            1
        );
        assert_eq!(
            state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs),
            1
        );
        assert!(
            !state.three_way_line_starts.base.is_materialized(),
            "base line starts should stay deferred when selecting measure rows"
        );
        assert!(
            !state.three_way_line_starts.ours.is_materialized(),
            "ours line starts should stay deferred when selecting measure rows"
        );
        assert!(
            !state.three_way_line_starts.theirs.is_materialized(),
            "theirs line starts should stay deferred when selecting measure rows"
        );
    }

    #[test]
    fn three_way_measure_rows_use_each_stage_coordinates_when_clean_context_diverges() {
        let base = "ctx\nbase conflict\ntail\n";
        let ours = "ctx\nclean ours insertion\nours conflict\ntail\n";
        let long_theirs = "theirs conflict line that must drive the remote column width";
        let theirs = format!("ctx\n{long_theirs}\ntail\n");
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                &theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            // The clean insertion is present in the merge result's context,
            // but not in the base or remote index stages.
            marker_segments: vec![
                ConflictSegment::Text("ctx\nclean ours insertion\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("base conflict\n".into()),
                    ours: "ours conflict\n".into(),
                    theirs: format!("{long_theirs}\n").into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.clone().into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(&theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        let measure_row = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
        assert_eq!(
            state.three_way_row_text(ThreeWayColumn::Theirs, measure_row),
            Some(long_theirs),
            "remote width measurement must select the widest line in stage :3"
        );
    }

    #[test]
    fn hidden_resolved_measure_row_is_not_remapped_as_a_side_line() {
        let base = "head\nb1\nb2\ntail\nbase widest visible line\n";
        let ours = "head\no1\nours insertion\no2\ntail\nours widest visible line\n";
        let long_theirs = "theirs widest visible line after a collapsed conflict";
        let theirs = format!("head\nt1\nt2\ntail\n{long_theirs}\n");
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                &theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("head\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("b1\nb2\n".into()),
                    ours: "o1\nours insertion\no2\n".into(),
                    theirs: "t1\nt2\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: true,
                    whitespace_only: false,
                }),
                ConflictSegment::Text(format!("tail\n{long_theirs}\n").into()),
            ],
            hide_resolved: true,
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.clone().into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(&theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        let measure_visible_ix = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
        let Some(ThreeWayVisibleItem::Line(aligned_row)) =
            state.three_way_visible_item(measure_visible_ix)
        else {
            panic!("remote measure row should be a visible source line");
        };
        assert_eq!(
            state.three_way_row_text(ThreeWayColumn::Theirs, aligned_row),
            Some(long_theirs),
        );
    }

    #[test]
    fn collapsed_context_measure_row_uses_the_compact_visible_index() {
        let prefix = (0..20)
            .map(|ix| format!("context {ix}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let base = format!("{prefix}base conflict\ntail\n");
        let ours = format!("{prefix}ours conflict\ntail\n");
        let long_theirs = "theirs conflict line wide enough to be the measurement row";
        let theirs = format!("{prefix}{long_theirs}\ntail\n");
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                &base,
                &ours,
                &theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text(prefix.into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("base conflict\n".into()),
                    ours: "ours conflict\n".into(),
                    theirs: format!("{long_theirs}\n").into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            collapse_context: true,
            three_way_text: ThreeWaySides {
                base: base.clone().into(),
                ours: ours.clone().into(),
                theirs: theirs.clone().into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(&base).into(),
                ours: super::deferred_line_starts_for_text(&ours).into(),
                theirs: super::deferred_line_starts_for_text(&theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };

        state.rebuild_three_way_visible_state();

        let measure_visible_ix = state.three_way_horizontal_measure_row(ThreeWayColumn::Theirs);
        let Some(ThreeWayVisibleItem::Line(aligned_row)) =
            state.three_way_visible_item(measure_visible_ix)
        else {
            panic!("remote measure row should survive context folding");
        };
        assert_eq!(
            state.three_way_row_text(ThreeWayColumn::Theirs, aligned_row),
            Some(long_theirs),
        );
        assert!(
            measure_visible_ix < aligned_row,
            "folded projection should compact the source row index"
        );
    }

    #[test]
    fn streamed_conflict_index_for_side_line_uses_grouped_side_ranges() {
        let mut state = ConflictResolverUiState::default();
        state.three_way_conflict_ranges = ThreeWaySides {
            base: vec![0..1, 4..6],
            ours: vec![2..5, 8..9],
            theirs: vec![1..3, 7..10],
        };

        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Base, 4),
            Some(1)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Ours, 3),
            Some(0)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Theirs, 8),
            Some(1)
        );
        assert_eq!(
            state.conflict_index_for_side_line(ThreeWayColumn::Base, 2),
            None
        );
    }

    #[test]
    fn streamed_mode_dispatch_uses_projection() {
        let mut state = ConflictResolverUiState::default();
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\nc\nd\ne\n".into(),
            theirs: "a\nb\nc\nd\ne\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        })];
        let ranges = vec![0..5];
        state.streamed_mut().three_way_visible_projection =
            conflict_resolver::build_three_way_visible_projection(5, &ranges, &segments, false);

        assert_eq!(state.three_way_visible_len(), 5);
        assert_eq!(
            state.three_way_visible_item(2),
            Some(ThreeWayVisibleItem::Line(2))
        );
    }

    fn streamed_state_with_one_conflict() -> ConflictResolverUiState {
        let segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\nb\n".into(),
                theirs: "c\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            }),
        ];
        let index = ConflictSplitRowIndex::new(&segments, 3);
        let projection = TwoWaySplitProjection::new(&index, &segments, false);

        let mut state = ConflictResolverUiState::default();
        state.marker_segments = segments;
        state.mode_state = super::ConflictModeState::Streamed(super::StreamedConflictState {
            split_row_index: index,
            two_way_split_projection: projection,
            ..super::StreamedConflictState::default()
        });
        state
    }

    #[test]
    fn two_way_row_counts_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let (diff_count, inline_count) = streamed.two_way_row_counts();
        assert!(diff_count > 0);
        assert_eq!(inline_count, 0);
    }

    #[test]
    fn two_way_split_conflict_ix_for_visible_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let vis_len = streamed.two_way_split_visible_len();
        let mut found_conflict = false;
        for ix in 0..vis_len {
            if streamed.two_way_split_conflict_ix_for_visible(ix) == Some(0) {
                found_conflict = true;
                break;
            }
        }
        assert!(found_conflict);
    }

    #[test]
    fn two_way_split_visible_row_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let visible_ix = streamed
            .two_way_visible_ix_for_conflict(0)
            .expect("streamed visible row should exist for the unresolved conflict");
        let visible_row = streamed
            .two_way_split_visible_row(visible_ix)
            .expect("streamed visible row should resolve through the projection");
        assert_eq!(visible_row.conflict_ix, Some(0));
        assert!(visible_row.row.old.is_some() || visible_row.row.new.is_some());
        assert!(visible_row.source_row_ix < streamed.two_way_row_counts().0);
    }

    #[test]
    fn two_way_split_nav_entries_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        assert_eq!(streamed.two_way_split_nav_entries().len(), 1);
    }

    #[test]
    fn two_way_nav_entries_uses_split_projection() {
        let streamed = streamed_state_with_one_conflict();
        assert_eq!(streamed.two_way_nav_entries().len(), 1);
    }

    #[test]
    fn two_way_conflict_ix_for_visible_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        let vis_len = streamed.two_way_split_visible_len();
        let mut found = false;
        for ix in 0..vis_len {
            if streamed.two_way_conflict_ix_for_visible(ix) == Some(0) {
                found = true;
                break;
            }
        }
        assert!(found);
    }

    #[test]
    fn two_way_visible_ix_for_conflict_dispatch() {
        let streamed = streamed_state_with_one_conflict();
        assert!(streamed.two_way_visible_ix_for_conflict(0).is_some());
        assert_eq!(streamed.two_way_visible_ix_for_conflict(99), None);
    }

    #[test]
    fn default_mode_state_is_streamed() {
        let state = ConflictResolverUiState::default();
        assert!(state.rendering_mode().is_streamed_large_file());
        assert!(state.split_row_index().is_some());
    }

    fn split_ready_state() -> ConflictResolverUiState {
        let base = "ctx\nb1\nb2\nb3\ntail\n";
        let ours = "ctx\no1\no2\no3\ntail\n";
        let theirs = "ctx\nt1\nt2\nt3\ntail\n";
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("ctx\n".into()),
                // Display blocks may have a base populated from the ancestor
                // even though the raw marker block is two-sided.
                ConflictSegment::Block(ConflictBlock {
                    base: Some("b1\nb2\nb3\n".into()),
                    ours: "o1\no2\no3\n".into(),
                    theirs: "t1\nt2\nt3\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            conflict_region_indices: vec![0],
            conflict_region_marker_has_base: vec![false],
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: vec![0, 4, 7, 10, 13].into(),
                ours: vec![0, 4, 7, 10, 13].into(),
                theirs: vec![0, 4, 7, 10, 13].into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };
        state.rebuild_three_way_visible_state();
        assert_eq!(state.three_way_block_aligned_range(0), Some(1..4));
        state
    }

    fn split_ready_state_with_synthetic_base(
        base: &str,
        block_base: &str,
    ) -> ConflictResolverUiState {
        let ours = "ctx\nshared1\nshared2\ntail\n";
        let theirs = ours;
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(
            &gitcomet_core::merge::align_three_way(
                base,
                ours,
                theirs,
                gitcomet_core::merge::DiffAlgorithm::Myers,
            ),
        );
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("ctx\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    // Synthetic display base populated from the ancestor; the
                    // serialized marker remains the ordinary two-marker form.
                    base: Some(block_base.to_string().into()),
                    ours: "shared1\nshared2\n".into(),
                    theirs: "shared1\nshared2\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            conflict_region_indices: vec![0],
            conflict_region_marker_has_base: vec![false],
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            three_way_text: ThreeWaySides {
                base: base.to_string().into(),
                ours: ours.into(),
                theirs: theirs.into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };
        state.rebuild_three_way_visible_state();
        state
    }

    #[test]
    fn conflict_row_selection_normalizes_and_clamps_to_its_block() {
        let state = split_ready_state();
        let reverse = ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 3,
            head_row: 1,
            selecting: true,
        };
        assert_eq!(reverse.row_range(), 1..=3);
        assert_eq!(state.clamp_row_to_conflict_block(0, 0), 1);
        assert_eq!(state.clamp_row_to_conflict_block(0, usize::MAX), 3);
    }

    #[test]
    fn alignment_marks_are_independent_per_column_and_extend_from_their_anchor() {
        let mut state = split_ready_state();
        assert!(state.manual_alignment_enabled());
        assert!(!state.has_alignment_selection());

        state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);
        state.set_alignment_selection(ThreeWayColumn::Theirs, 1, false);
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 2));
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Theirs, 1));
        assert!(
            !state.alignment_line_is_selected(ThreeWayColumn::Ours, 1),
            "marking one column must not mark the same line in another"
        );

        // Extending backwards from the anchor normalizes the range.
        state.set_alignment_selection(ThreeWayColumn::Ours, 1, true);
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 1));
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 2));
        assert!(!state.alignment_line_is_selected(ThreeWayColumn::Ours, 3));

        // Without extend the mark restarts at the clicked line.
        state.set_alignment_selection(ThreeWayColumn::Ours, 3, false);
        assert!(!state.alignment_line_is_selected(ThreeWayColumn::Ours, 1));
        assert!(state.alignment_line_is_selected(ThreeWayColumn::Ours, 3));

        assert!(state.clear_alignment_selections());
        assert!(!state.has_alignment_selection());
        assert!(!state.clear_alignment_selections());
    }

    #[test]
    fn an_unmarked_column_pins_an_empty_range_at_its_aligned_position() {
        let mut state = split_ready_state();
        state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);
        state.set_alignment_selection(ThreeWayColumn::Theirs, 1, false);

        let entry = state
            .manual_alignment_from_selections(true)
            .expect("two marked columns are enough to pin");
        assert_eq!(entry.local, 2..3);
        assert_eq!(entry.remote, 1..2);
        assert!(
            entry.base.is_empty(),
            "the unmarked base column pins nothing, not a guessed range"
        );
        assert_eq!(
            entry.base.start,
            state
                .three_way_aligned
                .side_line_lower_bound(ThreeWayColumn::Base.side_index(), 1),
            "its empty range still sits where the marked columns start"
        );
    }

    #[test]
    fn a_two_input_pin_leaves_the_base_range_at_the_origin() {
        let mut state = split_ready_state();
        state.set_alignment_selection(ThreeWayColumn::Base, 2, false);
        state.set_alignment_selection(ThreeWayColumn::Ours, 2, false);

        let entry = state
            .manual_alignment_from_selections(false)
            .expect("marked columns are enough to pin");
        assert_eq!(
            entry.base,
            0..0,
            "without a base the plan maps ours/theirs onto A/B, so the base range must stay inert"
        );
        assert_eq!(entry.local, 2..3);
    }

    #[test]
    fn nothing_marked_pins_nothing() {
        let state = split_ready_state();
        assert!(state.manual_alignment_from_selections(true).is_none());
    }

    #[test]
    fn a_conflict_without_aligned_rows_cannot_be_pinned() {
        let mut state = ConflictResolverUiState {
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            ..Default::default()
        };
        assert!(
            !state.manual_alignment_enabled(),
            "the identity map has no shared row space to express a pin in"
        );
        state.set_alignment_selection(ThreeWayColumn::Ours, 0, false);
        assert!(state.manual_alignment_from_selections(true).is_none());
    }

    #[test]
    fn split_boundaries_support_forward_reverse_and_single_row_selections() {
        let mut state = split_ready_state();

        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 1,
            head_row: 2,
            selecting: false,
        });
        let (region_index, forward) = state.split_boundaries_for_selection().expect("forward");
        assert_eq!(region_index, 0);
        assert_eq!(forward.ours, [0, 2]);
        assert_eq!(forward.theirs, [0, 2]);
        assert_eq!(
            forward.base, None,
            "raw two-sided markers need no base cuts"
        );

        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 2,
            head_row: 1,
            selecting: false,
        });
        assert_eq!(
            state.split_boundaries_for_selection().unwrap().1,
            forward,
            "reverse drags normalize to the same boundaries",
        );

        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 3,
            head_row: 3,
            selecting: false,
        });
        let single = state
            .split_boundaries_for_selection()
            .expect("single row")
            .1;
        assert_eq!(single.ours, [2, 3]);
        assert_eq!(single.theirs, [2, 3]);
    }

    #[test]
    fn split_boundaries_use_staged_positions_after_one_sided_clean_context() {
        let base = "ctx\nb1\nb2\ntail\n";
        let ours = "ctx\nours clean insertion\no1\no2\ntail\n";
        let theirs = "ctx\nt1\nt2\ntail\n";
        use gitcomet_core::merge::{AlignedRun, AlignedRunKind};
        let aligned = conflict_resolver::ThreeWayAlignedMap::from_alignment(&[
            AlignedRun {
                base: 0..1,
                ours: 0..1,
                theirs: 0..1,
                kind: AlignedRunKind::Unchanged,
            },
            AlignedRun {
                base: 1..1,
                ours: 1..2,
                theirs: 1..1,
                kind: AlignedRunKind::OursChanged,
            },
            AlignedRun {
                base: 1..3,
                ours: 2..4,
                theirs: 1..3,
                kind: AlignedRunKind::Conflict,
            },
            AlignedRun {
                base: 3..4,
                ours: 4..5,
                theirs: 3..4,
                kind: AlignedRunKind::Unchanged,
            },
        ]);
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                ConflictSegment::Text("ctx\nours clean insertion\n".into()),
                ConflictSegment::Block(ConflictBlock {
                    base: Some("b1\nb2\n".into()),
                    ours: "o1\no2\n".into(),
                    theirs: "t1\nt2\n".into(),
                    choice: ConflictChoice::Ours,
                    resolved: false,
                    whitespace_only: false,
                }),
                ConflictSegment::Text("tail\n".into()),
            ],
            conflict_region_indices: vec![0],
            conflict_region_marker_has_base: vec![true],
            strategy: Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::FullTextResolver,
            ),
            three_way_text: ThreeWaySides {
                base: base.into(),
                ours: ours.into(),
                theirs: theirs.into(),
            },
            three_way_line_starts: ThreeWaySides {
                base: super::deferred_line_starts_for_text(base).into(),
                ours: super::deferred_line_starts_for_text(ours).into(),
                theirs: super::deferred_line_starts_for_text(theirs).into(),
            },
            three_way_len: aligned.aligned_len(),
            three_way_aligned: aligned,
            ..Default::default()
        };
        state.rebuild_three_way_visible_state();
        let first_conflict_row = state
            .three_way_block_aligned_range(0)
            .unwrap()
            .find(|&row| {
                state.three_way_row_text(ThreeWayColumn::Base, row) == Some("b1")
                    && state.three_way_row_text(ThreeWayColumn::Ours, row) == Some("o1")
                    && state.three_way_row_text(ThreeWayColumn::Theirs, row) == Some("t1")
            })
            .expect("first conflict row");
        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: first_conflict_row,
            head_row: first_conflict_row,
            selecting: false,
        });

        let boundaries = state.split_boundaries_for_selection().unwrap().1;
        assert_eq!(boundaries.base, Some([0, 1]));
        assert_eq!(boundaries.ours, [0, 1]);
        assert_eq!(boundaries.theirs, [0, 1]);
    }

    #[test]
    fn split_boundaries_reject_whole_block_and_ambiguous_region_maps() {
        let mut state = split_ready_state();
        state.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: 1,
            head_row: 3,
            selecting: false,
        });
        assert!(state.split_boundaries_for_selection().is_none());

        state.row_selection.as_mut().unwrap().head_row = 2;
        state.conflict_region_indices = vec![0, 0];
        assert!(state.split_boundaries_for_selection().is_none());

        state.conflict_region_indices.clear();
        assert!(state.split_boundaries_for_selection().is_none());

        state.conflict_region_indices = vec![1];
        assert!(state.split_boundaries_for_selection().is_none());
    }

    #[test]
    fn split_boundaries_reject_synthetic_base_only_and_serialized_whole_block_selections() {
        let mut interior = split_ready_state_with_synthetic_base(
            "ctx\nshared1\nbase-only\nshared2\ntail\n",
            "shared1\nbase-only\nshared2\n",
        );
        let interior_range = interior.three_way_block_aligned_range(0).unwrap();
        let interior_padding = interior_range
            .clone()
            .find(|&row| {
                interior
                    .three_way_side_line_for_row(ThreeWayColumn::Base, row)
                    .is_some()
                    && interior
                        .three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                        .is_none()
                    && interior
                        .three_way_side_line_for_row(ThreeWayColumn::Theirs, row)
                        .is_none()
            })
            .expect("base-only aligned row");
        interior.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: interior_padding,
            head_row: interior_padding,
            selecting: false,
        });
        assert!(
            interior.split_boundaries_for_selection().is_none(),
            "a row absent from every serialized marker side cannot become its own conflict",
        );

        let mut edge = split_ready_state_with_synthetic_base(
            "ctx\nbase-only\nshared1\nshared2\ntail\n",
            "base-only\nshared1\nshared2\n",
        );
        let edge_range = edge.three_way_block_aligned_range(0).unwrap();
        let serialized_rows: Vec<usize> = edge_range
            .clone()
            .filter(|&row| {
                edge.three_way_side_line_for_row(ThreeWayColumn::Ours, row)
                    .is_some()
                    || edge
                        .three_way_side_line_for_row(ThreeWayColumn::Theirs, row)
                        .is_some()
            })
            .collect();
        edge.row_selection = Some(ConflictRowSelection {
            conflict_ix: 0,
            anchor_row: *serialized_rows.first().expect("serialized row"),
            head_row: *serialized_rows.last().expect("serialized row"),
            selecting: false,
        });
        assert!(
            edge.split_boundaries_for_selection().is_none(),
            "selecting every serialized line remains a degenerate whole-block split",
        );
    }

    #[test]
    fn joinable_context_rejects_marker_looking_text_between_blocks() {
        let block = || {
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "ours\n".into(),
                theirs: "theirs\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
                whitespace_only: false,
            })
        };
        let mut state = ConflictResolverUiState {
            marker_segments: vec![
                block(),
                ConflictSegment::Text("clean context\n".into()),
                block(),
            ],
            ..Default::default()
        };
        assert!(state.conflict_blocks_have_joinable_context(0, 1));
        state.marker_segments[1] = ConflictSegment::Text("<<<<<<< malformed\n".into());
        assert!(!state.conflict_blocks_have_joinable_context(0, 1));
        assert!(!state.conflict_blocks_have_joinable_context(0, 2));
    }

    #[test]
    fn semantic_selection_retains_automatic_target_when_no_marker_block_exists() {
        let automatic_id = ConflictNavTargetId::PlanBlock(gitcomet_core::merge::MergeBlockId {
            fingerprint: 1,
            occurrence: 0,
        });
        let conflict_id = ConflictNavTargetId::PlanBlock(gitcomet_core::merge::MergeBlockId {
            fingerprint: 2,
            occurrence: 0,
        });
        let mut state = ConflictResolverUiState {
            conflict_region_indices: vec![0],
            nav_targets: vec![
                ConflictNavTarget {
                    id: automatic_id,
                    order: 0,
                    aligned_rows: Some(1..2),
                    region_index: None,
                    display_conflict_index: None,
                    is_delta: true,
                    original_conflict: false,
                    unresolved: false,
                },
                ConflictNavTarget {
                    id: conflict_id,
                    order: 1,
                    aligned_rows: Some(3..4),
                    region_index: Some(0),
                    display_conflict_index: Some(0),
                    is_delta: true,
                    original_conflict: true,
                    unresolved: true,
                },
            ],
            ..Default::default()
        };

        assert!(state.select_nav_target(0));
        assert_eq!(state.nav_anchor.unwrap().id, automatic_id);
        assert_eq!(state.selected_nav_target_index(), Some(0));
        assert_eq!(state.active_conflict, None);

        assert!(state.select_display_conflict(0));
        assert_eq!(state.nav_anchor.unwrap().id, conflict_id);
        assert_eq!(state.active_conflict, Some(0));
    }

    #[test]
    fn exact_provenance_projects_target_rows_after_output_line_shifts() {
        let target = ConflictNavTarget {
            id: ConflictNavTargetId::DisplayBlock(0),
            order: 0,
            aligned_rows: Some(2..4),
            region_index: None,
            display_conflict_index: None,
            is_delta: true,
            original_conflict: false,
            unresolved: false,
        };
        let anchor = target.anchor();
        let mut state = ConflictResolverUiState {
            view_mode: ConflictResolverViewMode::ThreeWay,
            resolved_outline: super::ResolvedOutlineData {
                meta: vec![ResolvedLineMeta {
                    output_line: 5,
                    source: ResolvedLineSource::B,
                    input_line: Some(3),
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(
            state.output_line_for_nav_target_provenance(&target),
            Some(5)
        );
        state.resolved_outline.meta[0].output_line = 11;
        assert_eq!(
            state.output_line_for_nav_target_provenance(&target),
            Some(11),
            "surrounding output insertions shift only the projection"
        );
        assert_eq!(target.anchor(), anchor, "the semantic anchor is unchanged");
    }

    #[test]
    fn deletion_and_untraceable_manual_output_have_no_output_projection() {
        let deletion = ConflictNavTarget {
            id: ConflictNavTargetId::DisplayBlock(0),
            order: 0,
            aligned_rows: Some(8..9),
            region_index: None,
            display_conflict_index: None,
            is_delta: true,
            original_conflict: false,
            unresolved: false,
        };
        let state = ConflictResolverUiState {
            resolved_outline: super::ResolvedOutlineData {
                meta: vec![ResolvedLineMeta {
                    output_line: 3,
                    source: ResolvedLineSource::Manual,
                    input_line: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(state.output_line_for_nav_target_provenance(&deletion), None);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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
pub(super) struct ConflictResolverJoinTarget {
    pub(super) repo_id: RepoId,
    pub(super) path: gitcomet_state::msg::RepoPath,
    pub(super) conflict_rev: u64,
    pub(super) first_region_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct TerminalMenuContext {
    pub(super) has_session: bool,
    pub(super) has_selection: bool,
    pub(super) connected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BranchPickerPurpose {
    Checkout,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StashPickerPurpose {
    Pop,
    Apply,
    Drop,
}

/// Auto-squash strategy: which commit in each identical-message group survives,
/// the others being folded (fixup) into it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AutosquashMode {
    /// Fold each duplicate group into its newest (top) commit.
    ToTop,
    /// Only merge duplicates that are already adjacent in the list.
    Neighbor,
    /// Fold each duplicate group into its oldest (bottom) commit.
    ToBottom,
}

impl AutosquashMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            AutosquashMode::ToTop => "To Top Commit",
            AutosquashMode::Neighbor => "Neighboring Commit",
            AutosquashMode::ToBottom => "To Bottom Commit",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PopoverKind {
    RepoPicker,
    RecentRepositoryPicker,
    BranchPicker {
        purpose: BranchPickerPurpose,
    },
    CreateBranchFromRefPrompt {
        repo_id: RepoId,
        target: String,
        source_selectable: bool,
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
    PullReconcilePrompt {
        repo_id: RepoId,
    },
    PullPicker,
    PushPicker,
    CommitOptionsMenu {
        repo_id: RepoId,
    },
    PreviousCommitMessagesMenu {
        repo_id: RepoId,
    },
    RepoTabMenu {
        repo_id: RepoId,
    },
    AppMenu,
    AddRepoMenu,
    TerminalShutdownConfirm(TerminalShutdownPrompt),
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
    FileBrowserFileMenu {
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
pub(super) enum RepoPopoverKind {
    Remote(RemotePopoverKind),
    Worktree(WorktreePopoverKind),
    Submodule(SubmodulePopoverKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RemotePopoverKind {
    AddPrompt,
    EditUrlPrompt { name: String, kind: RemoteUrlKind },
    RemoveConfirm { name: String },
    Menu { name: String },
    DeleteBranchConfirm { remote: String, branch: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WorktreePopoverKind {
    SectionMenu,
    Menu {
        path: std::path::PathBuf,
        branch: Option<String>,
    },
    AddPrompt,
    OpenPicker,
    RemovePicker,
    RemoveConfirm {
        path: std::path::PathBuf,
        branch: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SubmodulePopoverKind {
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
    pub(super) fn remote(repo_id: RepoId, kind: RemotePopoverKind) -> Self {
        Self::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(kind),
        }
    }

    pub(super) fn worktree(repo_id: RepoId, kind: WorktreePopoverKind) -> Self {
        Self::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(kind),
        }
    }

    pub(super) fn submodule(repo_id: RepoId, kind: SubmodulePopoverKind) -> Self {
        Self::Repo {
            repo_id,
            kind: RepoPopoverKind::Submodule(kind),
        }
    }
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
pub(super) enum DeferredRepoBootstrap {
    RestoreSession {
        open_repos: Vec<std::path::PathBuf>,
        active_repo: Option<std::path::PathBuf>,
    },
    OpenRepo(std::path::PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SubmoduleDiffBootstrap {
    pub(super) repo_path: std::path::PathBuf,
    pub(super) target: DiffTarget,
}

impl SubmoduleDiffBootstrap {
    pub(super) fn new(repo_path: std::path::PathBuf, target: DiffTarget) -> Self {
        let repo_path = normalize_bootstrap_repo_path(repo_path);
        let target = normalize_bootstrap_diff_target(&repo_path, target);
        Self { repo_path, target }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SubmoduleDiffBootstrapAction {
    OpenRepo(std::path::PathBuf),
    SetActiveRepo(RepoId),
    SelectDiff { repo_id: RepoId, target: DiffTarget },
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

fn normalize_bootstrap_target_path(
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

fn normalize_bootstrap_diff_target(repo_path: &std::path::Path, target: DiffTarget) -> DiffTarget {
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
    canonicalize_or_original(path)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TerminalTextMetrics {
    pub(super) font_size: Pixels,
    pub(super) line_height: Pixels,
    pub(super) cell_width: Pixels,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TerminalGridSize {
    pub(super) rows: u16,
    pub(super) cols: u16,
    pub(super) pixel_width: u16,
    pub(super) pixel_height: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TerminalLayoutKey {
    pub(super) font_size_bits: u32,
    pub(super) line_height_bits: u32,
    pub(super) cell_width_bits: u32,
}

#[derive(Clone, Debug)]
pub(super) struct TerminalLayoutCache {
    pub(super) rem_size: Pixels,
    pub(super) key: TerminalLayoutKey,
    pub(super) base_style: gpui::TextStyle,
    pub(super) metrics: TerminalTextMetrics,
}

#[derive(Clone, Debug, Default)]
pub(super) struct TerminalCachedRow {
    pub(super) fingerprint: u64,
    pub(super) layout_key: TerminalLayoutKey,
    pub(super) shaped: Option<ShapedLine>,
    pub(super) background_rects: Vec<super::terminal_alacritty::TerminalBackgroundRect>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TerminalViewportCacheKey {
    pub(super) content_epoch: u64,
    pub(super) scrollback: usize,
    pub(super) rows: u16,
    pub(super) cols: u16,
    pub(super) layout_key: TerminalLayoutKey,
}

#[derive(Clone, Debug, Default)]
pub(super) struct TerminalRenderCache {
    pub(super) viewport_key: Option<TerminalViewportCacheKey>,
    pub(super) rows: Vec<TerminalCachedRow>,
}

pub(super) struct TerminalViewportView {
    pub(super) theme: AppTheme,
    pub(super) focus_handle: FocusHandle,
    pub(super) term_lock: Option<AlacrittyTermLock>,
    pub(super) pty_sender: Option<super::terminal_alacritty::PtySender>,
    pub(super) layout_cache: Option<TerminalLayoutCache>,
    pub(super) render_cache: TerminalRenderCache,
    pub(super) cursor_blink_visible: bool,
    pub(super) cursor_blink_hold_until: Instant,
    pub(super) cursor_blink_active: bool,
    pub(super) cursor_blink_task_scheduled: bool,
    pub(super) cursor_blink_seq: u64,
    pub(super) content_epoch: u64,
    pub(super) last_content: Option<super::terminal_alacritty::TerminalContent>,
    pub(super) viewport_bounds: Option<Bounds<Pixels>>,
    pub(super) pressed_mouse_button: Option<gpui::MouseButton>,
    /// Last grid cell `(row, col)` reported to the PTY for mouse-motion tracking.
    /// Used to dedupe motion reports so a TUI in any-event mode (1003) receives at
    /// most one report per cell instead of one per pixel-level move event.
    pub(super) last_motion_cell: Option<(u16, u16)>,
    pub(super) was_focused: bool,
    pub(super) selection_start: Option<TerminalGridPoint>,
    pub(super) selection_end: Option<TerminalGridPoint>,
    /// Set by "select all" so Copy grabs the entire buffer (including scrollback
    /// history, which the `u16` grid-point selection cannot represent). Cleared
    /// as soon as a manual selection begins.
    pub(super) select_all_active: bool,
    pub(super) ime_state: Option<super::terminal_alacritty::TerminalImeState>,
}

/// A single terminal (one PTY + alacritty + rendered viewport). A repo can hold
/// several of these as tabs.
pub(super) struct TerminalInstance {
    pub(super) focus_handle: FocusHandle,
    pub(super) pty_sender: Option<super::terminal_alacritty::PtySender>,
    pub(super) child_pid: Option<u32>,
    pub(super) events_rx:
        Option<smol::channel::Receiver<super::terminal_alacritty::TerminalBackendEvent>>,
    pub(super) connected: bool,
    pub(super) exit_status: Option<String>,
    pub(super) viewport: Entity<TerminalViewportView>,
    pub(super) session_seq: u64,
    pub(super) title: String,
}

pub(super) struct RepoTerminalSession {
    pub(super) workdir: std::path::PathBuf,
    pub(super) repo_name: String,
    pub(super) instances: Vec<TerminalInstance>,
    pub(super) active_index: usize,
}

impl RepoTerminalSession {
    pub(super) fn active_instance(&self) -> Option<&TerminalInstance> {
        self.instances.get(self.active_index)
    }

    pub(super) fn instance_by_seq_mut(&mut self, seq: u64) -> Option<&mut TerminalInstance> {
        self.instances.iter_mut().find(|i| i.session_seq == seq)
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
    CloseTerminalTab { repo_id: RepoId, index: usize },
    CloseWindow,
    QuitApp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct TerminalShutdownPrompt {
    pub(in crate::view) action: TerminalShutdownAction,
    pub(in crate::view) summary: TerminalShutdownSummary,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalPanelResizeState {
    pub(super) start_y: Pixels,
    pub(super) start_height: Pixels,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TerminalGridPoint {
    pub(super) row: u16,
    pub(super) col: u16,
}

impl TerminalGridPoint {
    pub(super) fn new(row: u16, col: u16) -> Self {
        Self { row, col }
    }
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

pub(super) fn submodule_diff_bootstrap_action(
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

pub(super) fn renders_full_chrome(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(super) fn show_diff_file_navigation(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(super) fn show_titlebar_repo_tabs(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(super) fn command_palette_available(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(super) fn should_seed_initial_repository_from_session(
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

pub(super) fn repository_entry_interstitial_active(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
) -> bool {
    matches!(view_mode, GitCometViewMode::Normal) && !has_repo_tabs
}

pub(super) fn should_show_startup_repository_loading_screen(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
    startup_repo_bootstrap_pending: bool,
) -> bool {
    repository_entry_interstitial_active(view_mode, has_repo_tabs) && startup_repo_bootstrap_pending
}

pub(super) fn should_show_splash_screen(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
    startup_repo_bootstrap_pending: bool,
) -> bool {
    repository_entry_interstitial_active(view_mode, has_repo_tabs)
        && !startup_repo_bootstrap_pending
}

pub(super) fn titlebar_workspace_actions_enabled(
    view_mode: GitCometViewMode,
    has_repo_tabs: bool,
) -> bool {
    !repository_entry_interstitial_active(view_mode, has_repo_tabs)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum ThemeMode {
    #[default]
    Automatic,
    Named(String),
}

impl ThemeMode {
    pub(super) fn key(&self) -> &str {
        match self {
            Self::Automatic => "automatic",
            Self::Named(key) => key,
        }
    }

    pub(super) fn from_key(raw: &str) -> Option<Self> {
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

    pub(super) fn label(&self) -> String {
        match self {
            Self::Automatic => "Automatic".to_string(),
            Self::Named(key) => crate::theme::theme_label(key).unwrap_or_else(|| key.clone()),
        }
    }

    pub(super) fn resolve_theme(&self, appearance: gpui::WindowAppearance) -> AppTheme {
        match self {
            Self::Automatic => AppTheme::default_for_window_appearance(appearance),
            Self::Named(key) => crate::theme::AppTheme::from_key(key)
                .unwrap_or_else(|| AppTheme::default_for_window_appearance(appearance)),
        }
    }

    pub(super) const fn is_automatic(&self) -> bool {
        matches!(self, Self::Automatic)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ChangeTrackingView {
    #[default]
    Combined,
    SplitUntracked,
}

impl ChangeTrackingView {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Combined => "combined",
            Self::SplitUntracked => "split_untracked",
        }
    }

    pub(super) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "combined" => Some(Self::Combined),
            "split_untracked" => Some(Self::SplitUntracked),
            _ => None,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Combined => "Combined with Unstaged",
            Self::SplitUntracked => "Separate section",
        }
    }

    pub(super) const fn menu_label(self) -> &'static str {
        match self {
            Self::Combined => "Combine with Unstaged",
            Self::SplitUntracked => "Show separate Untracked block",
        }
    }

    pub(super) const fn settings_label(self) -> &'static str {
        match self {
            Self::Combined => "Combined",
            Self::SplitUntracked => "Separate section",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DiffScrollSync {
    Vertical,
    Horizontal,
    None,
    #[default]
    Both,
}

impl DiffScrollSync {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::None => "none",
            Self::Both => "both",
        }
    }

    pub(super) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "vertical" => Some(Self::Vertical),
            "horizontal" => Some(Self::Horizontal),
            "none" => Some(Self::None),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
            Self::None => "None",
            Self::Both => "Both",
        }
    }

    pub(super) const fn includes_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    pub(super) const fn includes_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DiffContentMode {
    #[default]
    Full,
    Collapsed,
}

impl DiffContentMode {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Full => "content",
            Self::Collapsed => "changed_lines_only",
        }
    }

    pub(super) fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "content" => Some(Self::Full),
            "changed_lines_only" => Some(Self::Collapsed),
            _ => None,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Collapsed => "Collapsed",
        }
    }

    pub(super) const fn settings_label(self) -> &'static str {
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

pub struct GitCometView {
    pub(super) store: Arc<AppStore>,
    pub(super) state: Arc<AppState>,
    pub(super) window_handle: gpui::AnyWindowHandle,
    pub(super) _ui_model: Entity<AppUiModel>,
    pub(super) _poller: Poller,
    pub(super) _ui_model_subscription: gpui::Subscription,
    pub(super) _activation_subscription: gpui::Subscription,
    pub(super) _appearance_subscription: gpui::Subscription,
    pub(super) _terminal_keystroke_interceptor: gpui::Subscription,
    pub(super) _auth_prompt_username_input_subscription: gpui::Subscription,
    pub(super) _auth_prompt_secret_input_subscription: gpui::Subscription,
    pub(super) _open_repo_input_subscription: gpui::Subscription,
    pub(super) view_mode: GitCometViewMode,
    pub(super) theme_mode: ThemeMode,
    pub(super) theme: AppTheme,
    pub(super) title_bar: Entity<TitleBarView>,
    pub(super) sidebar_pane: Entity<SidebarPaneView>,
    pub(super) main_pane: Entity<MainPaneView>,
    pub(super) details_pane: Entity<DetailsPaneView>,
    pub(super) repo_tabs_bar: Entity<RepoTabsBarView>,
    pub(super) action_bar: Entity<ActionBarView>,
    pub(super) bottom_status_bar: Entity<BottomStatusBarView>,
    pub(super) tooltip_host: Entity<TooltipHost>,
    pub(super) toast_host: Entity<ToastHost>,
    pub(super) history_refs_hover_host: Entity<HistoryRefsHoverHost>,
    pub(super) popover_host: Entity<PopoverHost>,
    pub(super) command_palette: super::command_palette::CommandPaletteState,
    pub(super) command_palette_open: bool,
    #[allow(dead_code)]
    pub(super) command_palette_subscription: Option<gpui::Subscription>,
    pub(super) pre_palette_focus: Option<FocusHandle>,
    pub(super) focused_mergetool_bootstrap: Option<FocusedMergetoolBootstrap>,
    pub(super) submodule_diff_bootstrap: Option<SubmoduleDiffBootstrap>,
    pub(super) deferred_repo_bootstrap: Option<DeferredRepoBootstrap>,
    pub(super) startup_repo_bootstrap_pending: bool,
    pub(super) splash_backdrop_image: Arc<gpui::Image>,

    pub(super) last_window_size: Size<Pixels>,
    pub(super) ui_window_size_last_seen: Size<Pixels>,
    pub(super) ui_settings_persist_seq: u64,
    pub(super) last_repo_activation_dispatch_at: HashMap<RepoId, Instant>,

    pub(super) date_time_format: DateTimeFormat,
    pub(super) timezone: Timezone,
    pub(super) show_timezone: bool,
    pub(super) change_tracking_view: ChangeTrackingView,
    pub(super) terminal_preferences: TerminalPreferences,
    pub(super) terminal_sessions: HashMap<RepoId, RepoTerminalSession>,
    pub(super) terminal_panel_height: Pixels,
    pub(super) terminal_panel_resize: Option<TerminalPanelResizeState>,
    pub(super) next_terminal_session_seq: u64,
    pub(super) terminal_cursor_blink_visible: bool,
    pub(super) terminal_cursor_blink_hold_until: Instant,
    pub(super) terminal_cursor_blink_active: bool,
    pub(super) terminal_cursor_blink_task_scheduled: bool,
    pub(super) terminal_cursor_blink_seq: u64,
    pub(super) commit_push_after_enabled: bool,
    pub(super) diff_scroll_sync: DiffScrollSync,
    pub(super) diff_content_mode: DiffContentMode,
    pub(super) diff_whitespace_mode: DiffWhitespaceMode,
    pub(super) diff_view_mode: DiffViewMode,
    pub(super) annotate_enabled: bool,
    /// View mode stashed when enabling blame forced Split → Inline; restored
    /// on toggle-off unless the user changed modes in the meantime.
    pub(super) diff_view_mode_before_annotate: Option<DiffViewMode>,
    pub(super) diff_reveal_whitespace_chars: bool,
    pub(super) diff_word_wrap: bool,
    pub(super) diff_show_line_numbers: bool,
    pub(super) ui_scale_percent: u32,

    pub(super) open_repo_panel: bool,
    pub(super) open_repo_input: Entity<components::TextInput>,

    pub(super) hover_resize_edge: Option<ResizeEdge>,

    pub(super) sidebar_collapsed: bool,
    pub(super) sidebar_collapsed_before_merge_view: Option<bool>,
    pub(super) details_collapsed: bool,
    pub(super) sidebar_width_design: f32,
    pub(super) details_width_design: f32,
    pub(super) sidebar_width: Pixels,
    pub(super) details_width: Pixels,
    pub(super) sidebar_render_width: Pixels,
    pub(super) details_render_width: Pixels,
    pub(super) sidebar_width_anim_seq: u64,
    pub(super) details_width_anim_seq: u64,
    pub(super) sidebar_width_animating: bool,
    pub(super) details_width_animating: bool,
    pub(super) pane_resize: Option<PaneResizeState>,

    pub(super) last_mouse_pos: Point<Pixels>,
    pub(super) pending_terminal_shutdown_prompt: Option<TerminalShutdownPrompt>,
    pub(super) pending_quit_other_views: Vec<gpui::WeakEntity<GitCometView>>,
    pub(super) pending_pull_reconcile_prompt: Option<RepoId>,
    pub(super) pending_force_delete_branch_prompt: Option<(RepoId, String)>,
    pub(super) pending_force_delete_branch_centered: bool,
    pub(super) pending_force_remove_worktree_prompt:
        Option<(RepoId, std::path::PathBuf, Option<String>)>,
    pub(super) pending_submodule_trust_prompt:
        Option<gitcomet_state::model::SubmoduleTrustPromptState>,
    pub(super) pending_worktree_branch_removals: HashMap<(RepoId, std::path::PathBuf), String>,
    pub(super) startup_crash_report: Option<StartupCrashReport>,
    #[cfg(target_os = "macos")]
    pub(super) recent_repos_menu_fingerprint: Vec<std::path::PathBuf>,

    pub(super) error_banner_input: Entity<components::TextInput>,
    pub(super) auth_prompt_username_input: Entity<components::TextInput>,
    pub(super) auth_prompt_secret_input: Entity<components::TextInput>,
    pub(super) auth_prompt_key: Option<String>,
    pub(super) active_context_menu_invoker: Option<SharedString>,
}

pub(super) struct DiffTextLayoutCacheEntry {
    pub(super) layout: ShapedLine,
    pub(super) last_used_epoch: u64,
}
