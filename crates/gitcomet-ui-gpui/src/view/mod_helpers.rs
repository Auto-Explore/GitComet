use super::*;
use gitcomet_core::path_utils::canonicalize_or_original;

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
    pub(super) streamed_ascii_monospace_cell_width: Option<Pixels>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy)]
pub(super) struct StatusSectionEntries<'a> {
    entries: &'a [FileStatus],
    filter: StatusSectionFilter,
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
        Some(Self { entries, filter })
    }

    pub(super) fn iter(self) -> StatusSectionIter<'a> {
        StatusSectionIter {
            inner: self.entries.iter(),
            filter: self.filter,
        }
    }

    pub(super) fn len(self) -> usize {
        self.iter().count()
    }

    pub(super) fn get(self, index: usize) -> Option<&'a FileStatus> {
        self.iter().nth(index)
    }

    pub(super) fn path_vec(self) -> Vec<std::path::PathBuf> {
        self.iter().map(|entry| entry.path.clone()).collect()
    }

    pub(super) fn contains_path(self, path: &std::path::Path) -> bool {
        self.iter().any(|entry| entry.path == path)
    }
}

pub(super) struct StatusSectionIter<'a> {
    inner: std::slice::Iter<'a, FileStatus>,
    filter: StatusSectionFilter,
}

impl<'a> Iterator for StatusSectionIter<'a> {
    type Item = &'a FileStatus;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .find(|entry| status_section_filter_matches(self.filter, entry))
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

#[derive(Clone, Debug)]
pub(super) struct ConflictResolverUiState {
    pub(super) repo_id: Option<RepoId>,
    pub(super) path: Option<std::path::PathBuf>,
    pub(super) shared_path: Option<gitcomet_state::msg::RepoPath>,
    pub(super) loaded_file: Option<gitcomet_state::model::ConflictFile>,
    pub(super) conflict_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(super) source_hash: Option<u64>,
    pub(super) current: Option<std::sync::Arc<str>>,
    pub(super) marker_segments: Vec<conflict_resolver::ConflictSegment>,
    /// Mapping from visible block index to `ConflictSession` region index.
    pub(super) conflict_region_indices: Vec<usize>,
    pub(super) active_conflict: usize,
    pub(super) hovered_conflict: Option<(usize, ThreeWayColumn)>,
    /// Streamed conflict state for the single conflict rendering/runtime path.
    pub(super) mode_state: ConflictModeState,
    pub(super) view_mode: ConflictResolverViewMode,
    /// Backing text for each three-way source side.
    pub(super) three_way_text: ThreeWaySides<SharedString>,
    /// Per-side line start offsets into `three_way_text`, materialized lazily.
    pub(super) three_way_line_starts: ThreeWaySides<DeferredLineStarts>,
    pub(super) three_way_len: usize,
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
    /// Visible-row indices used to measure horizontal width for the two-way split inputs.
    pub(super) two_way_horizontal_measure_rows: [usize; 2],
    pub(super) three_way_word_highlights: ThreeWaySides<conflict_resolver::WordHighlights>,
    pub(super) nav_anchor: Option<usize>,
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
            conflict_syntax_language: None,
            source_hash: None,
            current: None,
            marker_segments: Vec::new(),
            conflict_region_indices: Vec::new(),
            active_conflict: 0,
            hovered_conflict: None,
            mode_state: ConflictModeState::default(),
            view_mode: ConflictResolverViewMode::TwoWayDiff,
            three_way_text: ThreeWaySides::default(),
            three_way_line_starts: ThreeWaySides::default(),
            three_way_len: 0,
            three_way_visible_state_ready: false,
            three_way_conflict_ranges: ThreeWaySides::default(),
            three_way_horizontal_measure_rows: [0; 3],
            conflict_has_base: Vec::new(),
            conflict_choices: Vec::new(),
            two_way_horizontal_measure_rows: [0; 2],
            three_way_word_highlights: ThreeWaySides::default(),
            nav_anchor: None,
            hide_resolved: false,
            is_binary_conflict: false,
            binary_side_sizes: [None; 3],
            strategy: None,
            conflict_kind: None,
            last_autosolve_summary: None,
            conflict_rev: 0,
            resolver_pending_recompute_seq: 0,
            resolved_outline: ResolvedOutlineData::default(),
            resolved_outline_gutter_rows: Vec::new(),
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

impl ConflictResolverUiState {
    pub(super) fn matches_target(&self, repo_id: RepoId, path: &std::path::Path) -> bool {
        self.repo_id == Some(repo_id) && self.path.as_deref() == Some(path)
    }

    pub(super) fn dispatch_path(&self) -> Option<gitcomet_state::msg::RepoPath> {
        self.shared_path.clone()
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

    // ----- Two-way split dispatch (giant vs eager) -----

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
    pub(super) fn two_way_split_conflict_ix_for_visible(&self, visible_ix: usize) -> Option<usize> {
        match &self.mode_state {
            ConflictModeState::Streamed(s) => s
                .two_way_split_projection
                .get(visible_ix)
                .and_then(|(_, ci)| ci),
        }
    }

    /// Build unresolved conflict navigation entries for two-way split view.
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

    // ----- Unified two-way dispatch (Split + Inline, giant vs eager) -----

    /// Build unresolved conflict navigation entries for the current two-way
    /// conflict diff view.
    pub(super) fn two_way_nav_entries(&self) -> Vec<usize> {
        self.two_way_split_nav_entries()
    }

    /// Map a two-way visible index to its conflict index.
    pub(super) fn two_way_conflict_ix_for_visible(&self, visible_ix: usize) -> Option<usize> {
        self.two_way_split_conflict_ix_for_visible(visible_ix)
    }

    /// Find the first visible index for a conflict in the current two-way diff
    /// view.
    pub(super) fn two_way_visible_ix_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
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
        if has_hidden_resolved_blocks {
            return self.compute_three_way_horizontal_measure_rows_from_visible_projection();
        }

        let mut best_rows = [0usize; 3];
        let mut best_lens = [0usize; 3];
        let mut base_line = 0usize;
        let mut ours_line = 0usize;
        let mut theirs_line = 0usize;

        let mut update_best = |slot: usize, row_ix: usize, width: usize| {
            if width > best_lens[slot] {
                best_lens[slot] = width;
                best_rows[slot] = row_ix;
            }
        };

        for segment in &self.marker_segments {
            match segment {
                conflict_resolver::ConflictSegment::Text(text) => {
                    let stats = conflict_resolver::scan_text_line_stats(text.as_ref());
                    if let Some((line_ix, width)) = stats.widest_line() {
                        update_best(0, base_line + line_ix, width);
                        update_best(1, ours_line + line_ix, width);
                        update_best(2, theirs_line + line_ix, width);
                    }
                    base_line = base_line.saturating_add(stats.line_count);
                    ours_line = ours_line.saturating_add(stats.line_count);
                    theirs_line = theirs_line.saturating_add(stats.line_count);
                }
                conflict_resolver::ConflictSegment::Block(block) => {
                    if let Some(base) = block.base.as_deref() {
                        let stats = conflict_resolver::scan_text_line_stats(base);
                        if let Some((line_ix, width)) = stats.widest_line() {
                            update_best(0, base_line + line_ix, width);
                        }
                        base_line = base_line.saturating_add(stats.line_count);
                    }

                    let ours_stats = conflict_resolver::scan_text_line_stats(block.ours.as_ref());
                    if let Some((line_ix, width)) = ours_stats.widest_line() {
                        update_best(1, ours_line + line_ix, width);
                    }
                    ours_line = ours_line.saturating_add(ours_stats.line_count);

                    let theirs_stats =
                        conflict_resolver::scan_text_line_stats(block.theirs.as_ref());
                    if let Some((line_ix, width)) = theirs_stats.widest_line() {
                        update_best(2, theirs_line + line_ix, width);
                    }
                    theirs_line = theirs_line.saturating_add(theirs_stats.line_count);
                }
            }
        }

        best_rows
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
                    let width = self.three_way_line_text(side, line_ix).map_or(0, str::len);
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
    /// Returns `None` in giant mode (word highlights are computed on-the-fly
    /// via `compute_word_highlights_for_row` at render time instead).
    pub(super) fn two_way_split_word_highlight(
        &self,
        _row_ix: usize,
    ) -> Option<&conflict_resolver::TwoWayWordHighlightPair> {
        None
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
        let three_way_visible_projection =
            conflict_resolver::build_three_way_visible_projection_with_resolved_flags(
                self.three_way_len,
                &maps.conflict_ranges[1],
                &maps.conflict_resolved,
                self.hide_resolved,
            );
        self.apply_three_way_conflict_maps(maps);
        match &mut self.mode_state {
            ConflictModeState::Streamed(s) => {
                s.three_way_visible_projection = three_way_visible_projection;
            }
        }
        self.three_way_visible_state_ready = true;
        self.refresh_three_way_horizontal_measure_rows();
    }

    /// Rebuild two-way visible state from current marker segments.
    /// Rebuilds the streamed split row index and projection.
    pub(super) fn rebuild_two_way_visible_state(&mut self) {
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
        ConflictResolverUiState, DeferredLineStarts, Loadable, ThreeWayColumn, ThreeWaySides,
    };
    use crate::view::conflict_resolver::{
        self, ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
        ConflictSplitRowIndex, ThreeWayVisibleItem, TwoWaySplitProjection,
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
            }),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".into()),
                ours: "ours2\n".into(),
                theirs: "theirs2\n".into(),
                choice: ConflictChoice::Both,
                resolved: true,
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
    fn rebuild_three_way_visible_state_streamed_mode() {
        let mut state = ConflictResolverUiState::default();
        state.marker_segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".into(),
            theirs: "c\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PopoverKind {
    RepoPicker,
    RecentRepositoryPicker,
    BranchPicker,
    CreateBranch,
    CreateBranchFromRefPrompt {
        repo_id: RepoId,
        target: String,
    },
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
    ResetPrompt {
        repo_id: RepoId,
        target: String,
        mode: ResetMode,
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
    ChangeTrackingSettings,
    UiScalePicker,
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
    canonicalize_or_original(path)
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

pub(super) fn renders_full_chrome(view_mode: GitCometViewMode) -> bool {
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

pub struct GitCometView {
    pub(super) store: Arc<AppStore>,
    pub(super) state: Arc<AppState>,
    pub(super) window_handle: gpui::AnyWindowHandle,
    pub(super) _ui_model: Entity<AppUiModel>,
    pub(super) _poller: Poller,
    pub(super) _ui_model_subscription: gpui::Subscription,
    pub(super) _activation_subscription: gpui::Subscription,
    pub(super) _appearance_subscription: gpui::Subscription,
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
    pub(super) popover_host: Entity<PopoverHost>,
    pub(super) focused_mergetool_bootstrap: Option<FocusedMergetoolBootstrap>,
    pub(super) deferred_repo_bootstrap: Option<DeferredRepoBootstrap>,
    pub(super) startup_repo_bootstrap_pending: bool,
    pub(super) splash_backdrop_image: Arc<gpui::Image>,

    pub(super) last_window_size: Size<Pixels>,
    pub(super) ui_window_size_last_seen: Size<Pixels>,
    pub(super) ui_settings_persist_seq: u64,

    pub(super) date_time_format: DateTimeFormat,
    pub(super) timezone: Timezone,
    pub(super) show_timezone: bool,
    pub(super) change_tracking_view: ChangeTrackingView,
    pub(super) diff_scroll_sync: DiffScrollSync,
    pub(super) ui_scale_percent: u32,

    pub(super) open_repo_panel: bool,
    pub(super) open_repo_input: Entity<components::TextInput>,

    pub(super) hover_resize_edge: Option<ResizeEdge>,

    pub(super) sidebar_collapsed: bool,
    pub(super) details_collapsed: bool,
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
    pub(super) pending_pull_reconcile_prompt: Option<RepoId>,
    pub(super) pending_force_delete_branch_prompt: Option<(RepoId, String)>,
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
