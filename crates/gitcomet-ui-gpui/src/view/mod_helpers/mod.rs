use super::*;
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::InteractiveRebaseAction;
use rustc_hash::{FxHashMap, FxHashSet};

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
    pub(in crate::view) target: BranchMenuTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum PushRequest {
    Push,
    SetUpstream { remote: String },
    NoRemotes,
    NotReady,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum PullRequest {
    Pull,
    NoRemotes,
    NotReady,
}

pub(in crate::view) fn head_is_detached(repo: &RepoState) -> bool {
    matches!(&repo.head_branch, Loadable::Ready(head) if head.is_empty() || head == "HEAD")
}

/// Whether the checked-out branch has a confirmed live remote-tracking
/// upstream. This remains false until the remote-branch list is loaded, and a
/// detached HEAD carries no branch to hold one.
pub(in crate::view) fn head_branch_has_live_upstream(repo: &RepoState) -> bool {
    let (Loadable::Ready(head), Loadable::Ready(branches)) = (&repo.head_branch, &repo.branches)
    else {
        return false;
    };
    let Some(upstream) = branches
        .iter()
        .find(|branch| branch.name == *head)
        .and_then(|branch| branch.upstream.as_ref())
    else {
        return false;
    };
    let Some(remote_branches) = repo.remote_branches.ready() else {
        return false;
    };
    remote_branches
        .iter()
        .any(|candidate| candidate.remote == upstream.remote && candidate.name == upstream.branch)
}

/// Decide whether Pull can run. A configured upstream is actionable only when
/// its exact remote-tracking ref exists; a future upstream configured by
/// "Create new" must be pushed before Pull is offered. For a branch with no
/// configured upstream, the backend can still use the preferred remote, while
/// detached HEAD falls back to Git's own diagnostics.
pub(in crate::view) fn pull_request(repo: &RepoState) -> PullRequest {
    let Loadable::Ready(head) = &repo.head_branch else {
        return PullRequest::NotReady;
    };
    if head.is_empty() || head == "HEAD" {
        return PullRequest::Pull;
    }
    let Loadable::Ready(branches) = &repo.branches else {
        return PullRequest::NotReady;
    };
    if branches
        .iter()
        .find(|branch| branch.name == *head)
        .is_some_and(|branch| branch.upstream.is_some())
    {
        return if head_branch_has_live_upstream(repo) {
            PullRequest::Pull
        } else {
            PullRequest::NotReady
        };
    }

    let Loadable::Ready(remotes) = &repo.remotes else {
        return PullRequest::NotReady;
    };
    if remotes.is_empty() {
        return PullRequest::NoRemotes;
    }
    PullRequest::Pull
}

/// Decide whether an interactive Push can run immediately or first needs the
/// existing set-upstream prompt. A configured upstream can name a branch that
/// has not been pushed yet; that still gives Push an exact destination.
pub(in crate::view) fn push_request(repo: &RepoState) -> PushRequest {
    let Loadable::Ready(head) = &repo.head_branch else {
        return PushRequest::NotReady;
    };
    // Preserve Git's own detached-HEAD diagnostics; there is no local branch
    // for which GitComet could offer to set an upstream.
    if head.is_empty() || head == "HEAD" {
        return PushRequest::Push;
    }
    let Loadable::Ready(branches) = &repo.branches else {
        return PushRequest::NotReady;
    };
    let Some(branch) = branches.iter().find(|branch| branch.name == *head) else {
        return PushRequest::NotReady;
    };
    if branch.upstream.is_some() {
        return PushRequest::Push;
    }

    let Loadable::Ready(remotes) = &repo.remotes else {
        return PushRequest::NotReady;
    };
    if remotes.is_empty() {
        return PushRequest::NoRemotes;
    }
    let remote = remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .unwrap_or(&remotes[0])
        .name
        .clone();
    PushRequest::SetUpstream { remote }
}

pub(in crate::view) fn selected_remote_branch_is_missing(
    state: &AppState,
    selected_branch: Option<&SelectedBranch>,
) -> bool {
    let Some(selected) = selected_branch else {
        return false;
    };
    let BranchMenuTarget::Remote { remote, branch } = &selected.target else {
        return false;
    };
    let Some(repo) = state.repos.iter().find(|repo| repo.id == selected.repo_id) else {
        return true;
    };
    let Loadable::Ready(branches) = &repo.remote_branches else {
        return false;
    };
    !branches
        .iter()
        .any(|candidate| candidate.remote == *remote && candidate.name == *branch)
}

pub(in crate::view) fn selected_branch_label_color(theme: AppTheme) -> gpui::Rgba {
    theme.colors.interaction.selected_foreground
}

pub(in crate::view) fn selected_branch_row_bg(theme: AppTheme) -> gpui::Rgba {
    theme.colors.interaction.selected_background
}

/// Which ref a history row should mark as the one the sidebar selected.
/// Carries the branch identity rather than its rendered label: the same branch
/// is drawn as `main` or `HEAD → main` depending on the row, so matching on
/// display text silently missed whichever form the row happened to use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct SelectedHistoryBranch {
    pub(in crate::view) target: BranchMenuTarget,
}

pub(in crate::view) fn selected_branch_for_history_row(
    selected_branch: Option<&SelectedBranch>,
    repo_id: RepoId,
    selected: bool,
) -> Option<SelectedHistoryBranch> {
    if !selected {
        return None;
    }

    let selected_branch = selected_branch?;
    if selected_branch.repo_id != repo_id {
        return None;
    }

    Some(SelectedHistoryBranch {
        target: selected_branch.target.clone(),
    })
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
    wants_collapsed_diff: bool,
    is_file_preview: bool,
    preview_kind: Option<RenderedPreviewKind>,
) -> Option<RenderedPreviewKind> {
    match preview_kind? {
        // Image/Code is orthogonal to the Full/Collapsed diff mode: the
        // rendered image is the whole file either way, and the source is a
        // normal text diff that both modes can show.
        // `is_file_preview` covers the content view an SVG gets when it is
        // opened from the file explorer: the picture is the whole file there
        // too, and Code is how you reach its source (and the editor).
        RenderedPreviewKind::Svg if wants_file_diff || wants_collapsed_diff || is_file_preview => {
            Some(RenderedPreviewKind::Svg)
        }
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

/// Which of the two real documents behind a diff a row's text came from.
///
/// A pair is always found in one document, so this is what says which line map
/// projects it back onto rows -- and why a pair can never span the two halves of
/// a split view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffTextPairSide {
    Old,
    New,
    /// The file preview, where rows are document lines one-to-one.
    Preview,
}

/// One end of a matched delimiter pair, projected onto a rendered row.
///
/// `range` is in the same tab-expanded display space as [`DiffTextPos::offset`],
/// so painting it reuses the coordinate machinery the selection quad already
/// has for wrapped, streamed and whitespace-revealed rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DiffTextPairSpan {
    pub(super) source_visible_ix: usize,
    pub(super) region: DiffTextRegion,
    pub(super) range: Range<usize>,
}

/// The delimiter pair a click selected, projected onto rows once at click time
/// rather than per row per frame.
///
/// One flat list rather than an open end and a close end: both are washed the
/// same colour, either can cover several rows (a start tag split across lines),
/// and either can be absent from the rendered rows entirely -- off the diff,
/// inside a collapsed hunk, or scrolled past. Whatever is on screen is painted;
/// half-lit says "the partner is elsewhere", where painting nothing would say
/// "there is no pair here", which is false.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DiffTextPairMatch {
    /// Which construct the pair delimits.
    ///
    /// Test-only, and deliberately so: the canvas washes brackets, tags and
    /// quotes with the same `diff_text_pair_match_color()`, so carrying the kind
    /// into a release build would be state nothing reads -- state that drifts.
    /// Assertions still want it, because "clicking the tag name paired the
    /// element" and "it paired some brackets nearby" are different outcomes with
    /// the same painted ranges.
    #[cfg(test)]
    pub(super) kind: crate::view::rows::SyntaxPairKind,
    pub(super) spans: Vec<DiffTextPairSpan>,
}

impl DiffTextPairMatch {
    /// The spans falling on one row, in ascending order.
    pub(super) fn ranges_on_row(
        &self,
        source_visible_ix: usize,
        region: DiffTextRegion,
    ) -> smallvec::SmallVec<[Range<usize>; 2]> {
        let mut out: smallvec::SmallVec<[Range<usize>; 2]> = smallvec::SmallVec::new();
        for span in &self.spans {
            if span.source_visible_ix == source_visible_ix
                && span.region == region
                && span.range.start < span.range.end
            {
                out.push(span.range.clone());
            }
        }
        out.sort_by_key(|range| range.start);
        out
    }
}

pub(super) struct DiffTextHitbox {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) layout_key: u64,
    pub(super) source_visible_ix: usize,
    pub(super) text_start_offset: usize,
    pub(super) text_len: usize,
    pub(super) offset_map: Option<DiffTextOffsetMap>,
    /// Exactly the text this row painted, tabs expanded and whitespace revealed
    /// as they were on screen.
    ///
    /// Offsets into it are the display offsets `x_for_index` wants, which is why
    /// the search reveal measures against this rather than re-deriving the row's
    /// text: the two do not always agree, and a row whose text cannot be found
    /// again reveals nothing.
    pub(super) painted_text: SharedString,
    pub(super) streamed_ascii_monospace_cell_width: Option<Pixels>,
    /// Set by rows that painted their text with wrapping. Those rows cover
    /// several visual lines, so a click resolves through the layout they were
    /// painted with rather than through an x offset along one shaped line.
    pub(super) wrapped: Option<DiffTextWrappedHit>,
}

/// A selectable document range painted by something other than text.
///
/// Flowing Markdown pictures and thematic breaks intentionally have no
/// [`DiffTextHitbox`]: their accessible copy text is not a run of glyphs that
/// can be mapped to an x coordinate. They still need a geometric target while
/// a drag crosses them, so this records the logical boundaries of the block
/// without making its invisible text directly clickable.
#[derive(Clone, Copy, Debug)]
pub(super) struct DiffTextMotionTarget {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) start: DiffTextPos,
    pub(super) end: DiffTextPos,
}

/// Where one merge-tool column row painted its text, and the line it shaped.
///
/// The conflict columns are their own canvases and register nothing in
/// [`DiffTextHitbox`], so quick search's sideways reveal measures against this
/// instead. `layout.text` is exactly what was painted, so offsets into it are
/// the display offsets `x_for_index` wants.
pub(super) struct ConflictTextHitbox {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) layout: gpui::ShapedLine,
}

/// The wrapped layout a row painted, plus what it takes to read offsets back
/// in row coordinates.
pub(super) struct DiffTextWrappedHit {
    pub(super) layout: gpui::TextLayout,
    /// The row's raw text, when tabs were expanded for painting.
    pub(super) untabbed: Option<SharedString>,
}

impl DiffTextWrappedHit {
    /// Offset in row coordinates for an offset in the painted text.
    pub(super) fn row_offset(&self, painted_offset: usize) -> usize {
        match &self.untabbed {
            Some(raw) => crate::view::rows::markdown_flow_row_offset(raw, painted_offset),
            None => painted_offset,
        }
    }

    /// Offset in the painted text for an offset in row coordinates — the
    /// inverse of [`Self::row_offset`].
    pub(super) fn painted_offset(&self, row_offset: usize) -> usize {
        match &self.untabbed {
            Some(raw) => crate::view::rows::markdown_flow_painted_offset(raw, row_offset),
            None => row_offset,
        }
    }
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

pub struct GitCometView {
    pub(super) store: Arc<AppStore>,
    pub(super) state: Arc<AppState>,
    pub(super) window_handle: gpui::AnyWindowHandle,
    pub(super) ui_model: Entity<AppUiModel>,
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
    pub(super) commit_message_hover_host: Entity<CommitMessageHoverHost>,
    pub(super) popover_host: Entity<PopoverHost>,
    pub(super) command_palette: Entity<super::command_palette::CommandPaletteView>,
    pub(super) command_palette_open: bool,
    pub(super) pre_palette_focus: Option<FocusHandle>,
    pub(super) focused_mergetool_bootstrap: Option<FocusedMergetoolBootstrap>,
    pub(super) submodule_diff_bootstrap: Option<SubmoduleDiffBootstrap>,
    pub(super) deferred_repo_bootstrap: Option<DeferredRepoBootstrap>,
    pub(super) startup_repo_bootstrap_pending: bool,
    pub(super) splash_backdrop_image: Arc<gpui::Image>,

    pub(super) last_window_size: Size<Pixels>,
    pub(super) ui_window_size_last_seen: Size<Pixels>,
    pub(super) ui_settings_persist_seq: u64,
    pub(super) last_repo_activation_dispatch_at: FxHashMap<RepoId, Instant>,
    /// Set when a deactivation was caused by a move/resize grab we requested, so
    /// the matching re-activation does not trigger a repo refresh.
    pub(super) window_grab_activation_suppressed_at: Option<Instant>,

    pub(super) date_time_format: DateTimeFormat,
    pub(super) timezone: Timezone,
    pub(super) show_timezone: bool,
    pub(super) change_tracking_view: ChangeTrackingView,
    pub(super) terminal_preferences: TerminalPreferences,
    pub(super) terminal_sessions: FxHashMap<RepoId, RepoTerminalSession>,
    pub(super) terminal_panel_height: Pixels,
    pub(super) terminal_panel_resize: Option<TerminalPanelResizeState>,
    pub(super) next_terminal_session_seq: u64,
    pub(super) terminal_cursor_blink_visible: bool,
    pub(super) terminal_cursor_blink_hold_until: Instant,
    pub(super) terminal_cursor_blink_active: bool,
    pub(super) terminal_cursor_blink_task_scheduled: bool,
    pub(super) terminal_cursor_blink_seq: u64,
    /// The reflog panel. It owns its own per-repository state (filter text,
    /// scroll, selection) — a separate entity so that hovering one of its rows
    /// repaints the panel instead of the whole application window.
    pub(super) reflog_pane: Entity<ReflogPaneView>,
    /// Which of the bottom panel's contents is currently visible for a repo,
    /// when more than one is open. Absent (and single-panel repos) fall back
    /// to whichever panel is actually open.
    pub(super) active_bottom_panel: FxHashMap<RepoId, BottomPanelTab>,
    pub(super) commit_push_after_enabled: bool,
    pub(super) diff_scroll_sync: DiffScrollSync,
    pub(super) diff_content_mode: DiffContentMode,
    pub(super) diff_whitespace_mode: DiffWhitespaceMode,
    pub(super) diff_view_mode: DiffViewMode,
    pub(super) annotate_enabled: bool,
    pub(super) diff_reveal_whitespace_chars: bool,
    pub(super) diff_word_wrap: bool,
    pub(super) diff_show_line_numbers: bool,
    pub(super) auto_save_file_edits: bool,
    pub(super) remote_markdown_image_policy: RemoteMarkdownImagePolicy,
    pub(super) remote_url_policy: RemoteUrlPolicy,
    pub(super) check_for_updates_on_startup: bool,
    pub(super) update_check_in_flight: bool,
    pub(super) update_check_manual_feedback_requested: bool,
    pub(super) ui_scale_percent: u32,

    pub(super) open_repo_panel: bool,
    pub(super) open_repo_input: Entity<components::TextInput>,
    pub(super) external_drag_paths: Option<gpui::ExternalPaths>,
    pub(super) external_drag_payload: Option<external_drag::ClassifiedExternalPaths>,
    pub(super) external_drag_classification_seq: u64,
    pub(super) external_drag_drop_pending: bool,

    pub(super) hover_resize_edge: Option<ResizeEdge>,

    pub(super) sidebar_collapsed: bool,
    /// Which sidebar section is currently shown in the collapsed-rail popover, if
    /// any. Only meaningful while `sidebar_collapsed` is true.
    pub(super) sidebar_collapsed_popover: Option<CollapsedSidebarSection>,
    /// A section whose popover is fading out. Kept mounted (invisible input) for
    /// the fade-out duration, then cleared by a timer keyed on the anim seq.
    pub(super) sidebar_collapsed_popover_closing: Option<CollapsedSidebarSection>,
    /// Bumped on every open/close transition; keys the fade animation (so it
    /// restarts each time) and guards the close timer against races.
    pub(super) sidebar_collapsed_popover_anim_seq: u64,
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
    pub(super) pending_unsaved_file_edits_prompt: Option<UnsavedFileEditsPrompt>,
    /// Waits for the dispatched writes to drain before the close/quit it was
    /// asked to retry.
    pub(super) pending_unsaved_file_edits_flush: Option<gpui::Task<()>>,
    pub(super) pending_quit_other_views: Vec<gpui::WeakEntity<GitCometView>>,
    pub(super) pending_pull_reconcile_prompt: Option<RepoId>,
    pub(super) pending_branch_exists_prompt: Option<BranchExistsPromptState>,
    pub(super) pending_force_delete_branch_prompt: Option<(RepoId, String)>,
    pub(super) pending_force_delete_branch_centered: bool,
    pub(super) pending_force_remove_worktree_prompt:
        Option<(RepoId, std::path::PathBuf, Option<String>)>,
    pub(super) pending_submodule_trust_prompt:
        Option<gitcomet_state::model::SubmoduleTrustPromptState>,
    pub(super) pending_submodule_trust_check:
        Option<gitcomet_state::model::SubmoduleTrustCheckState>,
    /// Hook chains queued to open once render has a `Window`. A chain is
    /// identified by the outer Git operation, so pre- and post-hooks from one
    /// command share the same presentation lifecycle.
    pub(super) pending_hook_activity_open: Option<(RepoId, GitOperationId)>,
    /// Active chains the user minimized, or that began behind another overlay.
    /// They stay represented by the compact progress toast and must not
    /// auto-open again when another hook in the same Git command starts.
    pub(super) minimized_hook_activity_chains: FxHashSet<(RepoId, GitOperationId)>,
    /// Repositories explicitly minimized by the user. Unlike the per-chain
    /// suppression above, this remains set across operations until Activity is
    /// opened again and closed with its X button.
    pub(super) minimized_hook_activity_repos: FxHashSet<RepoId>,
    pub(super) pending_worktree_branch_removals: FxHashMap<(RepoId, std::path::PathBuf), String>,
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

mod conflict_resolver_ui_state;
mod markdown_wrap_cache;
mod mode_impls;
mod status_sections;
mod three_way;
mod toasts;

pub(super) use conflict_resolver_ui_state::*;
pub(super) use markdown_wrap_cache::*;
pub use mode_impls::*;
pub(super) use status_sections::*;
pub(super) use three_way::*;
pub(super) use toasts::*;
