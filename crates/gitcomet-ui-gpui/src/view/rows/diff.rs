use super::diff_canvas;
use super::diff_text::*;
use super::*;
use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};
use crate::view::panes::main::{
    CollapsedDiffExpansionKind, CollapsedDiffHunk, CollapsedDiffVisibleRow,
    DiffHorizontalScrollColumn,
};
use crate::view::panes::main::{
    VersionedCachedDiffStyledText, versioned_query_cached_diff_styled_text_is_current,
};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;

const COLLAPSED_DIFF_INLINE_HUNK_SHELL_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_shell";
const COLLAPSED_DIFF_INLINE_HUNK_GUTTER_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_gutter";
const COLLAPSED_DIFF_INLINE_HUNK_UP_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_up";
const COLLAPSED_DIFF_INLINE_HUNK_DOWN_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_down";
const COLLAPSED_DIFF_INLINE_HUNK_SHORT_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_short";
const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHELL_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_shell";
const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_GUTTER_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_gutter";
const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_UP_DEBUG_SELECTOR: &str = "collapsed_diff_split_left_hunk_up";
const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_DOWN_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_down";
const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHORT_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_short";
const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHELL_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_shell";
const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_GUTTER_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_gutter";
const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_UP_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_up";
const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_DOWN_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_down";
const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHORT_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_short";
const COLLAPSED_HUNK_BACKGROUND_OVERDRAW_PX: f32 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollapsedHunkRevealAction {
    Up,
    Down,
    DownBefore,
    Short,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CollapsedHunkRevealClick {
    action: CollapsedHunkRevealAction,
    src_ix: usize,
}

fn diff_row_height(ui_scale_percent: u32) -> Pixels {
    crate::view::panes::main::diff_row_height_for_ui_scale(ui_scale_percent)
}

fn diff_file_header_height(ui_scale_percent: u32) -> Pixels {
    crate::view::panes::main::diff_file_header_height_for_ui_scale(ui_scale_percent)
}

fn diff_hunk_header_height(ui_scale_percent: u32) -> Pixels {
    crate::view::panes::main::diff_hunk_header_height_for_ui_scale(ui_scale_percent)
}

fn collapsed_hunk_header_row_height(ui_scale_percent: u32) -> Pixels {
    diff_row_height(ui_scale_percent)
}

fn collapsed_hunk_shell_width(
    handle: &gpui::UniformListScrollHandle,
    fallback_width: Pixels,
) -> Pixels {
    let width = handle
        .0
        .borrow()
        .base_handle
        .bounds()
        .size
        .width
        .max(px(0.0));
    if width > px(0.0) {
        width
    } else {
        fallback_width.max(px(0.0))
    }
}

fn scroll_pinned_hunk_shell(
    scroll_handle: gpui::UniformListScrollHandle,
    background: Option<gpui::Rgba>,
    child: AnyElement,
) -> ScrollPinnedHunkShell {
    ScrollPinnedHunkShell {
        child,
        scroll_handle,
        background,
    }
}

fn collapsed_hunk_bg_fill_bounds(bounds: gpui::Bounds<Pixels>) -> gpui::Bounds<Pixels> {
    gpui::Bounds::new(
        bounds.origin,
        gpui::size(
            bounds.size.width,
            bounds.size.height + px(COLLAPSED_HUNK_BACKGROUND_OVERDRAW_PX),
        ),
    )
}

fn collapsed_hunk_header_bg(theme: AppTheme) -> gpui::Rgba {
    with_alpha(
        theme.colors.text_muted,
        if theme.is_dark { 0.14 } else { 0.10 },
    )
}

fn focused_diff_row_alpha(theme: AppTheme) -> f32 {
    if theme.is_dark { 0.30 } else { 0.20 }
}

fn focused_diff_neutral_row_bg(theme: AppTheme) -> gpui::Rgba {
    with_alpha(
        theme.colors.text_muted,
        if theme.is_dark { 0.26 } else { 0.16 },
    )
}

fn focused_diff_line_bg(theme: AppTheme, kind: DiffLineKind) -> gpui::Rgba {
    match kind {
        DiffLineKind::Add => with_alpha(theme.colors.success, focused_diff_row_alpha(theme)),
        DiffLineKind::Remove => with_alpha(theme.colors.danger, focused_diff_row_alpha(theme)),
        DiffLineKind::Context | DiffLineKind::Header | DiffLineKind::Hunk => {
            focused_diff_neutral_row_bg(theme)
        }
    }
}

fn focused_collapsed_hunk_bg(theme: AppTheme, _hunk: Option<CollapsedDiffHunk>) -> gpui::Rgba {
    with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 })
}

fn collapsed_inline_hunk_bg(
    theme: AppTheme,
    _hunk: Option<CollapsedDiffHunk>,
    _expansion_kind: CollapsedDiffExpansionKind,
) -> gpui::Rgba {
    collapsed_hunk_header_bg(theme)
}

fn collapsed_inline_hunk_fg(theme: AppTheme, _hunk: Option<CollapsedDiffHunk>) -> gpui::Rgba {
    theme.colors.text_muted
}

fn collapsed_split_hunk_bg(
    theme: AppTheme,
    _hunk: Option<CollapsedDiffHunk>,
    _column: PatchSplitColumn,
) -> gpui::Rgba {
    collapsed_hunk_header_bg(theme)
}

fn collapsed_split_hunk_fg(theme: AppTheme, _column: PatchSplitColumn) -> gpui::Rgba {
    theme.colors.text_muted
}

fn collapsed_hunk_reveal_button(
    id: impl Into<gpui::ElementId>,
    debug_selector: &'static str,
    theme: AppTheme,
    enabled: bool,
    icon: &'static str,
    tooltip: &'static str,
    icon_color: gpui::Rgba,
    click: CollapsedHunkRevealClick,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let mut button = div()
        .id(id)
        .debug_selector(move || debug_selector.to_string())
        .w(px(18.0))
        .h(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radii.row));

    if enabled {
        button = button
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.55)))
            .active(move |s| s.bg(theme.colors.active))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                cx.stop_propagation();
                match click.action {
                    CollapsedHunkRevealAction::Up => {
                        this.collapsed_diff_reveal_hunk_up(click.src_ix, cx);
                    }
                    CollapsedHunkRevealAction::Down => {
                        this.collapsed_diff_reveal_hunk_down(click.src_ix, cx);
                    }
                    CollapsedHunkRevealAction::DownBefore => {
                        this.collapsed_diff_reveal_hunk_down_before(click.src_ix, cx);
                    }
                    CollapsedHunkRevealAction::Short => {
                        this.collapsed_diff_reveal_hunk_short(click.src_ix, cx);
                    }
                }
            }));
    }

    button
        .child(svg_icon(icon, icon_color, px(10.0)))
        .gitcomet_tooltip(theme, tooltip.into())
        .into_any_element()
}

struct ScrollPinnedHunkShell {
    child: AnyElement,
    scroll_handle: gpui::UniformListScrollHandle,
    background: Option<gpui::Rgba>,
}

impl gpui::IntoElement for ScrollPinnedHunkShell {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for ScrollPinnedHunkShell {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        let scroll_x = -self.scroll_handle.0.borrow().base_handle.offset().x;
        self.child.prepaint_at(
            gpui::point(bounds.origin.x + scroll_x, bounds.origin.y),
            window,
            cx,
        );
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint_state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        if let Some(background) = self.background {
            window.paint_quad(gpui::fill(
                collapsed_hunk_bg_fill_bounds(bounds),
                background,
            ));
        }
        self.child.paint(window, cx);
    }
}

/// Returns the word-highlight color for a diff line kind.
fn diff_line_word_color(kind: DiffLineKind, theme: AppTheme) -> Option<gpui::Rgba> {
    match kind {
        DiffLineKind::Add => Some(theme.colors.diff_add_text),
        DiffLineKind::Remove => Some(theme.colors.diff_remove_text),
        _ => None,
    }
}

/// Returns the word-highlight color for a file diff split column.
/// Left highlights Remove/Modify; Right highlights Add/Modify.
fn file_diff_split_word_color(
    column: PatchSplitColumn,
    kind: FileDiffRowKind,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    match column {
        PatchSplitColumn::Left => matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify)
            .then_some(theme.colors.diff_remove_text),
        PatchSplitColumn::Right => matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify)
            .then_some(theme.colors.diff_add_text),
    }
}

fn diff_placeholder_row(
    id: impl Into<gpui::ElementId>,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> AnyElement {
    div()
        .id(id)
        .h(diff_row_height(ui_scale_percent))
        .px_2()
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child("")
        .into_any_element()
}

fn streamed_diff_text_spec_with_syntax(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    word_ranges: Vec<Range<usize>>,
    word_color: Option<gpui::Rgba>,
    syntax: diff_canvas::StreamedDiffTextSyntaxSource,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    diff_canvas::is_streamable_diff_text(&raw_text).then(|| {
        diff_canvas::StreamedDiffTextPaintSpec {
            raw_text,
            query: query.clone(),
            query_options,
            query_matcher,
            word_ranges: Arc::from(word_ranges),
            word_color,
            syntax,
        }
    })
}

fn heuristic_streamed_diff_text_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    word_ranges: Vec<Range<usize>>,
    word_color: Option<gpui::Rgba>,
    language: Option<rows::DiffSyntaxLanguage>,
    mode: rows::DiffSyntaxMode,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    let syntax = match language {
        Some(language) => diff_canvas::StreamedDiffTextSyntaxSource::Heuristic { language, mode },
        None => diff_canvas::StreamedDiffTextSyntaxSource::None,
    };
    streamed_diff_text_spec_with_syntax(
        raw_text,
        query,
        query_options,
        query_matcher,
        word_ranges,
        word_color,
        syntax,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepared_streamed_diff_text_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    word_ranges: Vec<Range<usize>>,
    word_color: Option<gpui::Rgba>,
    language: Option<rows::DiffSyntaxLanguage>,
    fallback_mode: rows::DiffSyntaxMode,
    document_text: Arc<str>,
    line_starts: Arc<[usize]>,
    prepared_line: rows::PreparedDiffSyntaxLine,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    let syntax = match (language, prepared_line.document) {
        (Some(language), Some(document)) => diff_canvas::StreamedDiffTextSyntaxSource::Prepared {
            document_text,
            line_starts,
            document,
            language,
            line_ix: prepared_line.line_ix,
        },
        (Some(language), None) => diff_canvas::StreamedDiffTextSyntaxSource::Heuristic {
            language,
            mode: fallback_mode,
        },
        (None, _) => diff_canvas::StreamedDiffTextSyntaxSource::None,
    };
    streamed_diff_text_spec_with_syntax(
        raw_text,
        query,
        query_options,
        query_matcher,
        word_ranges,
        word_color,
        syntax,
    )
}

fn build_file_diff_cached_styled_text(
    theme: AppTheme,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    word_ranges: &[Range<usize>],
    context_prefix: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    if should_truncate_file_diff_display(raw_text) {
        let display = file_diff_display_text(raw_text);
        return build_cached_diff_styled_text(
            theme,
            display.as_ref(),
            &[],
            context_prefix,
            None,
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
    }

    build_cached_diff_styled_text(
        theme,
        raw_text.as_ref(),
        word_ranges,
        context_prefix,
        language,
        syntax_mode,
        word_color,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
    theme: AppTheme,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    word_ranges: &[Range<usize>],
    context_prefix: &str,
    syntax: DiffSyntaxConfig,
    word_color: Option<gpui::Rgba>,
    projected: rows::PreparedDiffSyntaxLine,
) -> (CachedDiffStyledText, bool) {
    if should_truncate_file_diff_display(raw_text) {
        let display = file_diff_display_text(raw_text);
        return (
            build_cached_diff_styled_text(
                theme,
                display.as_ref(),
                &[],
                context_prefix,
                None,
                DiffSyntaxMode::HeuristicOnly,
                None,
            ),
            false,
        );
    }

    build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
        theme,
        raw_text.as_ref(),
        word_ranges,
        context_prefix,
        syntax,
        word_color,
        projected,
    )
    .into_parts()
}

fn file_diff_split_side_text(
    row: &FileDiffRow,
    is_left: bool,
) -> Option<&gitcomet_core::file_diff::FileDiffLineText> {
    if is_left {
        row.old.as_ref()
    } else {
        row.new.as_ref()
    }
}

fn file_diff_split_side_text_owned(
    row: &FileDiffRow,
    is_left: bool,
) -> Option<gitcomet_core::file_diff::FileDiffLineText> {
    file_diff_split_side_text(row, is_left).cloned()
}

fn file_diff_split_side_line(row: &FileDiffRow, is_left: bool) -> Option<u32> {
    if is_left { row.old_line } else { row.new_line }
}

/// Snapshot of blame data needed to render the annotation column for one diff
/// render pass. Owns its data (an `Arc` clone of the loaded blame plus the
/// recency range and a single `now` timestamp) so it does not borrow the view.
pub(super) struct BlameRenderCtx {
    lines: std::sync::Arc<Vec<gitcomet_core::services::BlameLine>>,
    range: Option<(i64, i64)>,
    now: std::time::SystemTime,
    path: std::sync::Arc<std::path::Path>,
    /// The commit currently being viewed (when blaming a specific revision), used
    /// to hide the "view file at this commit" action on lines from that commit.
    viewed_commit: Option<std::sync::Arc<str>>,
    /// The working-tree area being blamed (`Some` for staged/unstaged diffs),
    /// used to classify uncommitted lines as staged vs unstaged. `None` when
    /// blaming a committed revision, where that distinction has no meaning.
    area: Option<gitcomet_core::domain::DiffArea>,
}

/// Staged vs unstaged classification for an uncommitted blame row when blaming a
/// working-tree area.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LocalChange {
    Staged,
    Unstaged,
}

/// Classify an uncommitted (or removal) blame row as staged vs unstaged from
/// whether it is a genuine *context* line and the blamed [`DiffArea`].
///
/// `Staged` area: every local line is staged. `Unstaged` area: an unchanged
/// *context* line (identical in index and worktree but differing from `HEAD`) is
/// a *staged* change; any actual change on the new side — an add or a
/// modification — is an *unstaged* change, which therefore overrides staged when
/// a line carries both staged and unstaged edits. Removals are also changes and
/// are classified with `is_context == false`. Views without diff sidedness (full
/// file content) pass `is_context == false`, yielding the area default
/// (`Staged → Staged`, `Unstaged → Unstaged`).
fn classify_local_change(area: gitcomet_core::domain::DiffArea, is_context: bool) -> LocalChange {
    use gitcomet_core::domain::DiffArea;
    match area {
        DiffArea::Staged => LocalChange::Staged,
        DiffArea::Unstaged if is_context => LocalChange::Staged,
        DiffArea::Unstaged => LocalChange::Unstaged,
    }
}

/// Run-grouping identity for a blamed row. Consecutive rows in the same group
/// collapse into one attribution run (the textual label is painted only on the
/// run start). Local lines all share the all-zero commit id, so the
/// staged/unstaged kind is part of the group to break a run at a staged↔unstaged
/// boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlameGroup {
    Committed,
    /// A local change; `None` when blaming a revision (legacy "Now" rendering).
    Local(Option<LocalChange>),
}

/// Previous rendered blamed row's state, threaded across a row sequence so run
/// starts are computed against the previously *rendered* line and group.
#[derive(Clone, Copy, Default)]
pub(super) struct BlamePrev {
    new_line: Option<u32>,
    group: Option<BlameGroup>,
}

/// Border color for a local-change group, reusing the diff add/remove palette.
fn local_change_color(theme: AppTheme, local: Option<LocalChange>) -> gpui::Rgba {
    match local {
        Some(LocalChange::Staged) => crate::theme::blame_staged_color(theme),
        Some(LocalChange::Unstaged) => crate::theme::blame_unstaged_color(theme),
        None => crate::theme::blame_local_change_color(theme.is_dark),
    }
}

/// Run-start label for a local-change group.
fn local_change_label(local: Option<LocalChange>) -> &'static str {
    match local {
        Some(LocalChange::Staged) => "Staged",
        Some(LocalChange::Unstaged) => "Unstaged",
        None => "Now",
    }
}

/// Build the per-row annotation paint data for a diff row, given its old-side
/// and new-side (1-based) line numbers. Returns `None` for rows that carry no
/// annotation (e.g. headers, or pure deletions outside a working-tree blame).
///
/// `is_context` is `true` only for genuinely unchanged lines (so a staged-only
/// edit, which appears as context in the unstaged diff, is labeled "Staged"); any
/// add/modify/removal passes `false`. `old_line` is used only to recognize a pure
/// removal worth a bar. Full file-content views without diff sidedness pass
/// `is_context = false`, falling back to the area default.
pub(super) fn build_row_blame_paint(
    ctx: &BlameRenderCtx,
    is_context: bool,
    old_line: Option<u32>,
    new_line: Option<u32>,
    prev_new_line: Option<u32>,
    theme: AppTheme,
) -> Option<diff_canvas::RowBlamePaint> {
    let prev = BlamePrev {
        new_line: prev_new_line,
        group: None,
    };
    build_row_blame_paint_inner(ctx, is_context, old_line, new_line, prev, theme)
        .map(|(paint, _)| paint)
}

/// Core blame-paint builder shared by the tracked and untracked entry points.
/// Returns the paint plus the row's [`BlameGroup`] so the caller can thread it
/// into the next row's run-start decision.
fn build_row_blame_paint_inner(
    ctx: &BlameRenderCtx,
    is_context: bool,
    old_line: Option<u32>,
    new_line: Option<u32>,
    prev: BlamePrev,
    theme: AppTheme,
) -> Option<(diff_canvas::RowBlamePaint, BlameGroup)> {
    // A pure removal (no new-side line) has no `BlameLine` to attribute, but in a
    // working-tree blame it is still a local change worth a colored bar so a
    // deleted chunk is visible in the annotation column. A removal is a change,
    // so it never counts as context.
    if new_line.is_none() {
        let area = ctx.area?;
        old_line?;
        let local = classify_local_change(area, false);
        let group = BlameGroup::Local(Some(local));
        let is_run_start = prev.group != Some(group);
        return Some((removal_blame_paint(ctx, local, is_run_start, theme), group));
    }

    let annotation = super::blame::blame_for_new_line(&ctx.lines, new_line, prev.new_line)?;
    let line = annotation.line;
    // Working-tree blame surfaces not-yet-committed lines with an empty or
    // all-zero object id. These render as a distinct local-change row: a
    // staged/unstaged-colored bar, the matching label, no author initials, no
    // summary, and no action icons.
    let uncommitted = gitcomet_core::domain::is_uncommitted_commit_id(&line.commit_id);
    let group = if uncommitted {
        BlameGroup::Local(ctx.area.map(|area| classify_local_change(area, is_context)))
    } else {
        BlameGroup::Committed
    };
    // Break a run when the classification group changes from the previous
    // rendered row, even when the underlying commit id is identical (all local
    // lines share the all-zero id), so a staged chunk and an adjacent unstaged
    // chunk each get their own label. Only force this when a previous group is
    // actually known (the tracked diff path); the untracked file-content path
    // leaves `prev.group == None` and relies on `annotation.is_run_start` alone.
    let is_run_start =
        annotation.is_run_start || (prev.group.is_some() && prev.group != Some(group));

    let border = match group {
        BlameGroup::Local(local) => local_change_color(theme, local),
        BlameGroup::Committed => {
            let t = match (line.author_time_unix, ctx.range) {
                (Some(ts), Some(range)) => super::blame::blame_recency_t(ts, range),
                _ => 1.0,
            };
            crate::theme::blame_heat_color(theme.is_dark, t)
        }
    };
    let (when, initials, summary, body) = if !is_run_start {
        (
            SharedString::default(),
            SharedString::default(),
            SharedString::default(),
            None,
        )
    } else if let BlameGroup::Local(local) = group {
        (
            SharedString::from(local_change_label(local)),
            SharedString::default(),
            SharedString::default(),
            None,
        )
    } else {
        let when = line
            .author_time_unix
            .map(|ts| crate::view::date_time::format_relative_time(ts, ctx.now))
            .unwrap_or_else(|| "unknown".to_string());
        (
            SharedString::from(when),
            SharedString::from(super::blame::author_initials(&line.author)),
            // `Arc<str>` -> `SharedString` is a refcount bump, not a byte copy,
            // so this avoids re-allocating the summary/body every render pass.
            SharedString::from(line.summary.clone()),
            line.body.clone().map(SharedString::from),
        )
    };
    let paint = diff_canvas::RowBlamePaint {
        border,
        show_text: is_run_start,
        when,
        initials,
        summary,
        body,
        commit_id: gitcomet_core::domain::CommitId(line.commit_id.clone()),
        path: std::sync::Arc::clone(&ctx.path),
        source_path: line.source_path.as_deref().map(std::sync::Arc::from),
        prior_exists: line.prior_exists,
        prior_commit: line
            .prior_commit
            .clone()
            .map(gitcomet_core::domain::CommitId),
        is_viewed_commit: ctx.viewed_commit.as_deref() == Some(line.commit_id.as_ref()),
    };
    Some((paint, group))
}

/// Build the annotation paint for a pure-removal row, which has no `BlameLine`:
/// just a colored bar plus the staged/unstaged label on the run start.
fn removal_blame_paint(
    ctx: &BlameRenderCtx,
    local: LocalChange,
    is_run_start: bool,
    theme: AppTheme,
) -> diff_canvas::RowBlamePaint {
    let when = if is_run_start {
        SharedString::from(local_change_label(Some(local)))
    } else {
        SharedString::default()
    };
    diff_canvas::RowBlamePaint {
        border: local_change_color(theme, Some(local)),
        show_text: is_run_start,
        when,
        initials: SharedString::default(),
        summary: SharedString::default(),
        body: None,
        commit_id: gitcomet_core::domain::CommitId(std::sync::Arc::from("")),
        path: std::sync::Arc::clone(&ctx.path),
        source_path: None,
        prior_exists: false,
        prior_commit: None,
        is_viewed_commit: false,
    }
}

/// Build the blame paint for a row while tracking the previous blamed row's
/// new-side line number and group in `prev`, so run starts are computed against
/// the previously *rendered* line and break at staged↔unstaged boundaries.
/// `prev` must be threaded once per rendered row sequence (one cell per map).
fn build_row_blame_paint_tracked(
    ctx: &BlameRenderCtx,
    is_context: bool,
    old_line: Option<u32>,
    new_line: Option<u32>,
    prev: &std::cell::Cell<BlamePrev>,
    theme: AppTheme,
) -> Option<diff_canvas::RowBlamePaint> {
    let prev_state = prev.get();
    let result =
        build_row_blame_paint_inner(ctx, is_context, old_line, new_line, prev_state, theme);
    prev.set(BlamePrev {
        new_line: new_line.or(prev_state.new_line),
        group: match &result {
            Some((_, group)) => Some(*group),
            None => prev_state.group,
        },
    });
    result.map(|(paint, _)| paint)
}

impl MainPaneView {
    /// Build a blame render context when annotate is enabled and blame for the
    /// current target is loaded; otherwise `None`.
    pub(super) fn blame_render_ctx(&mut self) -> Option<BlameRenderCtx> {
        if !self.annotation_active() {
            return None;
        }
        let repo = self.active_repo()?;
        let gitcomet_state::model::Loadable::Ready(lines) = &repo.history_state.blame else {
            return None;
        };
        let path: std::sync::Arc<std::path::Path> =
            std::sync::Arc::from(repo.history_state.blame_path.as_deref()?);
        // When blaming a specific commit, that commit is the one currently being
        // viewed; "view file at this commit" on its own lines would be a no-op.
        let viewed_commit = match &repo.history_state.blame_source {
            Some(gitcomet_core::domain::BlameSource::Revision(Some(rev))) => {
                Some(std::sync::Arc::<str>::from(rev.as_str()))
            }
            _ => None,
        };
        // The blamed working-tree area, used to classify uncommitted lines as
        // staged vs unstaged. `None` for revision blame (no such distinction).
        let area = match &repo.history_state.blame_source {
            Some(gitcomet_core::domain::BlameSource::WorkingTree(area)) => Some(*area),
            _ => None,
        };
        let lines = std::sync::Arc::clone(lines);
        // The time range never changes for a given loaded blame, so memoize it by
        // the blame Arc's identity instead of rescanning every frame. Compare by
        // `ptr_eq` against a held Arc clone: keeping the cached allocation alive
        // means a reloaded blame can't reuse the same address and alias a stale
        // range (an ABA hazard a bare pointer key would have).
        let range = match &self.blame_time_range_cache {
            Some((cached, range)) if std::sync::Arc::ptr_eq(cached, &lines) => *range,
            _ => {
                let range = super::blame::blame_time_range(&lines);
                self.blame_time_range_cache = Some((std::sync::Arc::clone(&lines), range));
                range
            }
        };
        Some(BlameRenderCtx {
            lines,
            range,
            now: std::time::SystemTime::now(),
            path,
            viewed_commit,
            area,
        })
    }

    fn diff_text_segments_cache_get_for_query(
        &mut self,
        key: usize,
        query: &str,
        options: DiffSearchOptions,
        syntax_epoch: u64,
    ) -> Option<&CachedDiffStyledText> {
        if query.is_empty() {
            return self.diff_text_segments_cache_get(key, syntax_epoch);
        }

        self.sync_diff_text_query_overlay_cache(query, options);
        let query_generation = self.diff_text_query_cache_generation;
        if self.diff_text_query_segments_cache.len() <= key {
            self.diff_text_query_segments_cache
                .resize_with(key + 1, || None);
        }

        if versioned_query_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
            query_generation,
        )
        .is_none()
        {
            let base = self
                .diff_text_segments_cache_get(key, syntax_epoch)?
                .clone();
            let overlaid = build_cached_diff_query_overlay_styled_text(
                self.theme,
                &base,
                self.diff_text_query_cache_matcher.as_ref()?,
            );
            self.diff_text_query_segments_cache[key] = Some(VersionedCachedDiffStyledText {
                syntax_epoch,
                query_generation,
                styled: overlaid,
            });
        }

        versioned_query_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
            query_generation,
        )
    }

    pub(in super::super) fn render_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        let min_width = this.diff_horizontal_layout_min_width(DiffHorizontalScrollColumn::Primary);
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = (!query.as_ref().is_empty())
            .then(|| Arc::new(DiffSearchMatcher::new(query.as_ref(), query_options)));
        let reveal_whitespace_chars = this.reveal_whitespace_chars;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let annotation_width = if this.annotation_active() {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        let blame_ctx = this.blame_render_ctx();

        if this.is_collapsed_diff_projection_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            let old_document_text: Arc<str> = this.file_diff_old_text.clone().into();
            let old_line_starts = Arc::clone(&this.file_diff_old_line_starts);
            let new_document_text: Arc<str> = this.file_diff_new_text.clone().into();
            let new_line_starts = Arc::clone(&this.file_diff_new_line_starts);
            let pinned_hunk_shell_width = collapsed_hunk_shell_width(&this.diff_scroll, min_width);
            let pinned_hunk_shell_scroll = this.diff_scroll.clone();

            let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
                    let Some(source_visible_ix) =
                        this.diff_source_visible_ix_for_visible_ix(visible_ix)
                    else {
                        return diff_placeholder_row(
                            ("collapsed_diff_missing", visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };
                    let Some(row) = this.collapsed_visible_row(source_visible_ix) else {
                        return diff_placeholder_row(
                            ("collapsed_diff_missing", visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };

                    match row {
                        CollapsedDiffVisibleRow::HunkHeader {
                            src_ix,
                            expansion_kind,
                            hidden_rows,
                            ..
                        } => {
                            let display_src_ix = row.header_display_src_ix();
                            let display = display_src_ix
                                .and_then(|display_src_ix| {
                                    this.collapsed_diff_hunk_header_display(display_src_ix)
                                })
                                .unwrap_or_default();
                            let context_menu_active = display_src_ix.is_some()
                                && this.active_repo_id().is_some_and(|repo_id| {
                                    let invoker: SharedString =
                                        format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                                    this.active_context_menu_invoker.as_ref() == Some(&invoker)
                                });
                            let collapsed_hunk = this.collapsed_diff_hunk_for_src_ix(src_ix);

                            collapsed_inline_header_row(
                                theme,
                                ui_scale_percent,
                                visible_ix,
                                DiffClickKind::HunkHeader,
                                selected,
                                min_width,
                                pinned_hunk_shell_width,
                                pinned_hunk_shell_scroll.clone(),
                                collapsed_hunk,
                                None,
                                display,
                                None,
                                context_menu_active,
                                src_ix,
                                expansion_kind,
                                hidden_rows,
                                cx,
                            )
                        }
                        CollapsedDiffVisibleRow::FileRow { row_ix } => {
                            let row_word_ranges = this.file_diff_inline_word_ranges(row_ix);
                            let Some(row) = this.file_diff_inline_render_data(row_ix) else {
                                return diff_placeholder_row(
                                    ("collapsed_diff_oob", visible_ix),
                                    theme,
                                    ui_scale_percent,
                                );
                            };
                            let visual_kind = this.file_diff_inline_visual_kind(row_ix);
                            let line = AnnotatedDiffLine {
                                kind: row.kind,
                                text: "".into(),
                                old_line: row.old_line,
                                new_line: row.new_line,
                            };
                            let streamed_spec = {
                                let line_language = matches!(
                                    row.kind,
                                    DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                                )
                                .then_some(language)
                                .flatten();
                                let word_color = diff_line_word_color(visual_kind, theme);
                                let prepared_line = match row.kind {
                                    DiffLineKind::Remove => {
                                        rows::prepared_diff_syntax_line_for_one_based_line(
                                            this.file_diff_split_prepared_syntax_document(
                                                DiffTextRegion::SplitLeft,
                                            ),
                                            row.old_line,
                                        )
                                    }
                                    DiffLineKind::Add | DiffLineKind::Context => {
                                        rows::prepared_diff_syntax_line_for_one_based_line(
                                            this.file_diff_split_prepared_syntax_document(
                                                DiffTextRegion::SplitRight,
                                            ),
                                            row.new_line,
                                        )
                                    }
                                    DiffLineKind::Header | DiffLineKind::Hunk => {
                                        rows::prepared_diff_syntax_line_for_one_based_line(
                                            None, None,
                                        )
                                    }
                                };
                                let (document_text, line_starts) = match row.kind {
                                    DiffLineKind::Remove => (
                                        Arc::clone(&old_document_text),
                                        Arc::clone(&old_line_starts),
                                    ),
                                    DiffLineKind::Add | DiffLineKind::Context => (
                                        Arc::clone(&new_document_text),
                                        Arc::clone(&new_line_starts),
                                    ),
                                    DiffLineKind::Header | DiffLineKind::Hunk => (
                                        Arc::clone(&new_document_text),
                                        Arc::clone(&new_line_starts),
                                    ),
                                };
                                let syntax_mode = DiffSyntaxMode::Auto;
                                prepared_streamed_diff_text_spec(
                                    row.text.clone(),
                                    &query,
                                    query_options,
                                    query_matcher.clone(),
                                    row_word_ranges.clone(),
                                    word_color,
                                    line_language,
                                    syntax_mode,
                                    document_text,
                                    line_starts,
                                    prepared_line,
                                )
                            };

                            let styled = if streamed_spec.is_some() {
                                None
                            } else {
                                let cache_epoch =
                                    this.file_diff_style_cache_epochs.inline_epoch(row.kind);
                                if this
                                    .diff_text_segments_cache_get(row_ix, cache_epoch)
                                    .is_none()
                                {
                                    let word_color = diff_line_word_color(visual_kind, theme);
                                    let is_content_line = matches!(
                                        line.kind,
                                        DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                                    );
                                    let line_language =
                                        is_content_line.then_some(language).flatten();
                                    let projected = this.file_diff_inline_projected_syntax(&line);
                                    let syntax_mode = DiffSyntaxMode::Auto;
                                    let (styled, is_pending) =
                                        build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                                            theme,
                                            &row.text,
                                            row_word_ranges.as_slice(),
                                            "",
                                            DiffSyntaxConfig {
                                                language: line_language,
                                                mode: syntax_mode,
                                            },
                                            word_color,
                                            projected,
                                        );
                                    if is_pending {
                                        this.ensure_prepared_syntax_chunk_poll(cx);
                                    }
                                    this.diff_text_segments_cache_set(row_ix, cache_epoch, styled);
                                }
                                this.diff_text_segments_cache_get_for_query(
                                    row_ix,
                                    query.as_ref(),
                                    query_options,
                                    cache_epoch,
                                )
                            };

                            diff_row(
                                theme,
                                ui_scale_percent,
                                visible_ix,
                                DiffClickKind::Line,
                                selected,
                                DiffViewMode::Inline,
                                min_width,
                                &line,
                                visual_kind,
                                None,
                                None,
                                styled,
                                streamed_spec,
                                Some(row.text.as_ref()),
                                reveal_whitespace_chars,
                                false,
                                show_line_numbers,
                                wrap,
                                annotation_width,
                                blame_ctx
                                    .as_ref()
                                    .and_then(|ctx| build_row_blame_paint_tracked(ctx, matches!(visual_kind, DiffLineKind::Context), line.old_line, line.new_line, &blame_prev_nl, theme)),
                                annot_hover,
                                cx,
                            )
                        }
                    }
                })
                .collect();
        }

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            let old_document_text: Arc<str> = this.file_diff_old_text.clone().into();
            let old_line_starts = Arc::clone(&this.file_diff_old_line_starts);
            let new_document_text: Arc<str> = this.file_diff_new_text.clone().into();
            let new_line_starts = Arc::clone(&this.file_diff_new_line_starts);
            // Inline syntax is now projected from the real old/new (split)
            // documents instead of parsing a synthetic mixed inline stream.
            // syntax_mode is determined per-row based on projection availability.
            if let Some(language) = language {
                struct SyntaxOnlyBatchRow {
                    inline_ix: usize,
                    cache_epoch: u64,
                    line: AnnotatedDiffLine,
                    text: gitcomet_core::file_diff::FileDiffLineText,
                }

                let mut syntax_only_rows = Vec::new();
                for visible_ix in range.clone() {
                    let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        continue;
                    };
                    let Some(row) = this.file_diff_inline_render_data(inline_ix) else {
                        continue;
                    };
                    if diff_canvas::is_streamable_diff_text(&row.text) {
                        continue;
                    }
                    if should_truncate_file_diff_display(&row.text) {
                        continue;
                    }
                    let line = AnnotatedDiffLine {
                        kind: row.kind,
                        text: "".into(),
                        old_line: row.old_line,
                        new_line: row.new_line,
                    };
                    let cache_epoch = this.file_diff_style_cache_epochs.inline_epoch(row.kind);
                    if this
                        .diff_text_segments_cache_get(inline_ix, cache_epoch)
                        .is_some()
                    {
                        continue;
                    }
                    if !matches!(
                        line.kind,
                        DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                    ) {
                        continue;
                    }
                    if this.file_diff_inline_modify_pair_texts(inline_ix).is_some() {
                        continue;
                    }
                    syntax_only_rows.push(SyntaxOnlyBatchRow {
                        inline_ix,
                        cache_epoch,
                        line,
                        text: row.text,
                    });
                }

                if !syntax_only_rows.is_empty() {
                    let batch_rows = syntax_only_rows
                        .iter()
                        .map(|row| InlineDiffSyntaxOnlyRow {
                            text: row.text.as_ref(),
                            line: &row.line,
                        })
                        .collect::<Vec<_>>();
                    let batched_styles =
                        build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
                            theme,
                            Some(language),
                            PreparedDiffSyntaxTextSource {
                                document: this.file_diff_split_prepared_syntax_document(
                                    DiffTextRegion::SplitLeft,
                                ),
                            },
                            PreparedDiffSyntaxTextSource {
                                document: this.file_diff_split_prepared_syntax_document(
                                    DiffTextRegion::SplitRight,
                                ),
                            },
                            batch_rows.as_slice(),
                            DiffSyntaxMode::Auto,
                        );
                    let mut pending_batch = false;
                    for (row, prepared) in syntax_only_rows.iter().zip(batched_styles) {
                        let (styled, is_pending) = prepared.into_parts();
                        pending_batch |= is_pending;
                        this.diff_text_segments_cache_set(row.inline_ix, row.cache_epoch, styled);
                    }
                    if pending_batch {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                    }
                }
            }

            let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

                    let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row(
                            ("diff_missing", visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };
                    let row_word_ranges = this.file_diff_inline_word_ranges(inline_ix);
                    let visual_kind = this.file_diff_inline_visual_kind(inline_ix);
                    let render_data = this.file_diff_inline_render_data(inline_ix);
                    let streamed_spec = render_data.as_ref().and_then(|row| {
                        let line_language = matches!(
                            row.kind,
                            DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                        )
                        .then_some(language)
                        .flatten();
                        let word_color = diff_line_word_color(visual_kind, theme);
                        let prepared_line = match row.kind {
                            DiffLineKind::Remove => rows::prepared_diff_syntax_line_for_one_based_line(
                                this.file_diff_split_prepared_syntax_document(
                                    DiffTextRegion::SplitLeft,
                                ),
                                row.old_line,
                            ),
                            DiffLineKind::Add | DiffLineKind::Context => {
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    this.file_diff_split_prepared_syntax_document(
                                        DiffTextRegion::SplitRight,
                                    ),
                                    row.new_line,
                                )
                            }
                            DiffLineKind::Header | DiffLineKind::Hunk => {
                                rows::prepared_diff_syntax_line_for_one_based_line(None, None)
                            }
                        };
                        let (document_text, line_starts) = match row.kind {
                            DiffLineKind::Remove => (
                                Arc::clone(&old_document_text),
                                Arc::clone(&old_line_starts),
                            ),
                            DiffLineKind::Add | DiffLineKind::Context => (
                                Arc::clone(&new_document_text),
                                Arc::clone(&new_line_starts),
                            ),
                            DiffLineKind::Header | DiffLineKind::Hunk => (
                                Arc::clone(&new_document_text),
                                Arc::clone(&new_line_starts),
                            ),
                        };
                        let syntax_mode = DiffSyntaxMode::Auto;
                        prepared_streamed_diff_text_spec(
                            row.text.clone(),
                            &query,
                            query_options,
                            query_matcher.clone(),
                            row_word_ranges.clone(),
                            word_color,
                            line_language,
                            syntax_mode,
                            document_text,
                            line_starts,
                            prepared_line,
                        )
                    });

                    let (line, cache_epoch, styled) = if let Some(row) = render_data.as_ref() {
                        let line = AnnotatedDiffLine {
                            kind: row.kind,
                            text: "".into(),
                            old_line: row.old_line,
                            new_line: row.new_line,
                        };
                        let cache_epoch = this.file_diff_style_cache_epochs.inline_epoch(row.kind);
                        if streamed_spec.is_none()
                            && this
                                .diff_text_segments_cache_get(inline_ix, cache_epoch)
                                .is_none()
                            {
                                let word_color = diff_line_word_color(visual_kind, theme);
                                let is_content_line = matches!(
                                    line.kind,
                                    DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                                );
                                let line_language = is_content_line.then_some(language).flatten();
                                let projected = this.file_diff_inline_projected_syntax(&line);
                                let syntax_mode = DiffSyntaxMode::Auto;
                                let (styled, is_pending) =
                                    build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                                        theme,
                                        &row.text,
                                        row_word_ranges.as_slice(),
                                        "",
                                        DiffSyntaxConfig {
                                            language: line_language,
                                            mode: syntax_mode,
                                        },
                                        word_color,
                                        projected,
                                    );
                                if is_pending {
                                    this.ensure_prepared_syntax_chunk_poll(cx);
                                }
                                this.diff_text_segments_cache_set(inline_ix, cache_epoch, styled);
                            }
                        let styled = if streamed_spec.is_none() {
                            this.diff_text_segments_cache_get_for_query(
                                inline_ix,
                                query.as_ref(),
                                query_options,
                                cache_epoch,
                            )
                        } else {
                            None
                        };
                        debug_assert!(
                            streamed_spec.is_some() || styled.is_some(),
                            "diff text segment cache missing for inline row {inline_ix} after populate"
                        );
                        (line, cache_epoch, styled)
                    } else {
                        let Some(line) = this.file_diff_inline_row(inline_ix) else {
                            return diff_placeholder_row(
                                ("diff_oob", visible_ix),
                                theme,
                                ui_scale_percent,
                            );
                        };
                        let cache_epoch = this.file_diff_inline_style_cache_epoch(&line);
                        if this
                            .diff_text_segments_cache_get(inline_ix, cache_epoch)
                            .is_none()
                        {
                            let word_color = diff_line_word_color(visual_kind, theme);
                            let is_content_line = matches!(
                                line.kind,
                                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                            );
                            let line_language = is_content_line.then_some(language).flatten();
                            let projected = this.file_diff_inline_projected_syntax(&line);
                            let syntax_mode = DiffSyntaxMode::Auto;
                            let (styled, is_pending) =
                                build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                                    theme,
                                    diff_content_text(&line),
                                    row_word_ranges.as_slice(),
                                    "",
                                    DiffSyntaxConfig {
                                        language: line_language,
                                        mode: syntax_mode,
                                    },
                                    word_color,
                                    projected,
                                )
                                .into_parts();
                            if is_pending {
                                this.ensure_prepared_syntax_chunk_poll(cx);
                            }
                            this.diff_text_segments_cache_set(inline_ix, cache_epoch, styled);
                        }
                        let styled = this.diff_text_segments_cache_get_for_query(
                            inline_ix,
                            query.as_ref(),
                            query_options,
                            cache_epoch,
                        );
                        debug_assert!(
                            styled.is_some(),
                            "diff text segment cache missing for inline row {inline_ix} after populate"
                        );
                        (line, cache_epoch, styled)
                    };
                    let _ = cache_epoch;

                    diff_row(
                        theme,
                        ui_scale_percent,
                        visible_ix,
                        DiffClickKind::Line,
                        selected,
                        DiffViewMode::Inline,
                        min_width,
                        &line,
                        visual_kind,
                        None,
                        None,
                        styled,
                        streamed_spec,
                        render_data
                            .as_ref()
                            .map(|row| row.text.as_ref())
                            .or_else(|| Some(diff_content_text(&line))),
                        reveal_whitespace_chars,
                        false,
                        show_line_numbers,
                        wrap,
                        annotation_width,
                        blame_ctx
                            .as_ref()
                            .and_then(|ctx| build_row_blame_paint_tracked(ctx, matches!(visual_kind, DiffLineKind::Context), line.old_line, line.new_line, &blame_prev_nl, theme)),
                        annot_hover,
                        cx,
                    )
                })
                .collect();
        }

        let theme = this.theme;
        let cache_epoch = 0u64;
        let repo_id_for_context_menu = this.active_repo_id();
        let active_context_menu_invoker = this.active_context_menu_invoker.clone();
        let syntax_mode = this.patch_diff_syntax_mode();
        let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                let show_line_numbers = this.diff_show_line_numbers;
                let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

                let Some(src_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return diff_placeholder_row(
                        ("diff_missing", visible_ix),
                        theme,
                        ui_scale_percent,
                    );
                };
                let click_kind = this
                    .diff_click_kinds
                    .get(src_ix)
                    .copied()
                    .unwrap_or(DiffClickKind::Line);

                this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                let word_ranges: &[Range<usize>] = this
                    .diff_word_highlights
                    .get(src_ix)
                    .and_then(|r| r.as_ref().map(Vec::as_slice))
                    .unwrap_or(&[]);

                let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);

                let language = this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                let Some(line) = this.patch_diff_row(src_ix) else {
                    return diff_placeholder_row(("diff_oob", visible_ix), theme, ui_scale_percent);
                };
                let visual_kind = this.patch_visual_line_kind(src_ix);
                let streamed_spec = matches!(click_kind, DiffClickKind::Line)
                    .then(|| {
                        heuristic_streamed_diff_text_spec(
                            crate::view::diff_utils::diff_content_line_text(&line),
                            &query,
                            query_options,
                            query_matcher.clone(),
                            word_ranges.to_vec(),
                            diff_line_word_color(visual_kind, theme),
                            language,
                            syntax_mode,
                        )
                    })
                    .flatten();

                let should_style = matches!(click_kind, DiffClickKind::Line) || !query.is_empty();
                if should_style
                    && streamed_spec.is_none()
                    && this
                        .diff_text_segments_cache_get(src_ix, cache_epoch)
                        .is_none()
                {
                    let computed = if matches!(click_kind, DiffClickKind::Line) {
                        let word_color = diff_line_word_color(visual_kind, theme);
                        let content_text = diff_content_text(&line);

                        build_cached_diff_styled_text_with_source_identity(
                            theme,
                            content_text,
                            Some(DiffTextSourceIdentity::from_str(content_text)),
                            word_ranges,
                            "",
                            language,
                            syntax_mode,
                            word_color,
                        )
                    } else {
                        let display =
                            this.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        build_cached_diff_styled_text(
                            theme,
                            display.as_ref(),
                            &[] as &[Range<usize>],
                            "",
                            None,
                            syntax_mode,
                            None,
                        )
                    };
                    this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                }

                let header_display = matches!(
                    click_kind,
                    DiffClickKind::FileHeader | DiffClickKind::HunkHeader
                )
                .then(|| this.diff_header_display_cache.get(&src_ix).cloned())
                .flatten();
                let context_menu_active = click_kind == DiffClickKind::HunkHeader
                    && repo_id_for_context_menu.is_some_and(|repo_id| {
                        let invoker: SharedString =
                            format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                        active_context_menu_invoker.as_ref() == Some(&invoker)
                    });
                let styled = if should_style && streamed_spec.is_none() {
                    this.diff_text_segments_cache_get_for_query(
                        src_ix,
                        query.as_ref(),
                        query_options,
                        cache_epoch,
                    )
                } else {
                    None
                };
                diff_row(
                    theme,
                    ui_scale_percent,
                    visible_ix,
                    click_kind,
                    selected,
                    DiffViewMode::Inline,
                    min_width,
                    &line,
                    visual_kind,
                    file_stat,
                    header_display,
                    styled,
                    streamed_spec,
                    Some(if matches!(click_kind, DiffClickKind::Line) {
                        diff_content_text(&line)
                    } else {
                        line.text.as_ref()
                    }),
                    reveal_whitespace_chars,
                    context_menu_active,
                    show_line_numbers,
                    wrap,
                    annotation_width,
                    blame_ctx.as_ref().and_then(|ctx| {
                        build_row_blame_paint_tracked(
                            ctx,
                            matches!(visual_kind, DiffLineKind::Context),
                            line.old_line,
                            line.new_line,
                            &blame_prev_nl,
                            theme,
                        )
                    }),
                    annot_hover,
                    cx,
                )
            })
            .collect()
    }

    pub(in super::super) fn render_diff_split_left_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        Self::render_diff_split_rows(this, PatchSplitColumn::Left, range, annot_hover, cx)
    }

    pub(in super::super) fn render_diff_split_right_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        Self::render_diff_split_rows(this, PatchSplitColumn::Right, range, annot_hover, cx)
    }

    fn render_diff_split_rows(
        this: &mut Self,
        column: PatchSplitColumn,
        range: Range<usize>,
        annot_hover: Option<(usize, AnnotArea)>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width =
            this.diff_horizontal_layout_min_width(if matches!(column, PatchSplitColumn::Right) {
                DiffHorizontalScrollColumn::SplitRight
            } else {
                DiffHorizontalScrollColumn::Primary
            });
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = (!query.as_ref().is_empty())
            .then(|| Arc::new(DiffSearchMatcher::new(query.as_ref(), query_options)));
        let reveal_whitespace_chars = this.reveal_whitespace_chars;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();

        let is_left = matches!(column, PatchSplitColumn::Left);
        // The annotation column is only drawn in the left split column. Reserve
        // its width whenever annotate mode is on — even before blame data is
        // ready — so the column space is stable and content does not shift when
        // blame finishes loading. Only the left column needs the blame context;
        // building it for the right column would clone the blame data and rescan
        // the time range for nothing.
        let blame_ctx = if is_left {
            this.blame_render_ctx()
        } else {
            None
        };
        let annotation_width = if this.annotation_active() && is_left {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        let region = if is_left {
            DiffTextRegion::SplitLeft
        } else {
            DiffTextRegion::SplitRight
        };
        // Static ID tags to avoid format!/String allocation in element IDs.
        let (id_missing, id_oob, id_src_oob, id_hidden) = if is_left {
            (
                "diff_split_left_missing",
                "diff_split_left_oob",
                "diff_split_left_src_oob",
                "diff_split_left_hidden_header",
            )
        } else {
            (
                "diff_split_right_missing",
                "diff_split_right_oob",
                "diff_split_right_src_oob",
                "diff_split_right_hidden_header",
            )
        };

        if this.is_collapsed_diff_projection_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            let cache_epoch = this.file_diff_split_style_cache_epoch(region);
            let syntax_document = this.file_diff_split_prepared_syntax_document(region);
            let syntax_mode = DiffSyntaxMode::Auto;
            let document_text: Arc<str> = if is_left {
                this.file_diff_old_text.clone().into()
            } else {
                this.file_diff_new_text.clone().into()
            };
            let line_starts = if is_left {
                Arc::clone(&this.file_diff_old_line_starts)
            } else {
                Arc::clone(&this.file_diff_new_line_starts)
            };
            let pinned_hunk_shell_width = if is_left {
                collapsed_hunk_shell_width(&this.diff_scroll, min_width)
            } else {
                collapsed_hunk_shell_width(&this.diff_split_right_scroll, min_width)
            };
            let pinned_hunk_shell_scroll = if is_left {
                this.diff_scroll.clone()
            } else {
                this.diff_split_right_scroll.clone()
            };

            let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
                    let Some(source_visible_ix) =
                        this.diff_source_visible_ix_for_visible_ix(visible_ix)
                    else {
                        return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
                    };
                    let Some(visible_row) = this.collapsed_visible_row(source_visible_ix) else {
                        return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
                    };

                    match visible_row {
                        CollapsedDiffVisibleRow::HunkHeader {
                            src_ix,
                            expansion_kind,
                            hidden_rows,
                            ..
                        } => {
                            let display_src_ix = visible_row.header_display_src_ix();
                            let display = display_src_ix
                                .and_then(|display_src_ix| {
                                    this.collapsed_diff_hunk_header_display(display_src_ix)
                                })
                                .unwrap_or_default();
                            let context_menu_active = display_src_ix.is_some()
                                && this.active_repo_id().is_some_and(|repo_id| {
                                    let invoker: SharedString =
                                        format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                                    this.active_context_menu_invoker.as_ref() == Some(&invoker)
                                });
                            let collapsed_hunk = this.collapsed_diff_hunk_for_src_ix(src_ix);

                            collapsed_split_header_row(
                                theme,
                                ui_scale_percent,
                                column,
                                visible_ix,
                                DiffClickKind::HunkHeader,
                                selected,
                                min_width,
                                pinned_hunk_shell_width,
                                pinned_hunk_shell_scroll.clone(),
                                collapsed_hunk,
                                None,
                                display,
                                None,
                                context_menu_active,
                                src_ix,
                                expansion_kind,
                                hidden_rows,
                                cx,
                            )
                        }
                        CollapsedDiffVisibleRow::FileRow { row_ix } => {
                            let Some(row) = this.file_diff_split_render_data(row_ix) else {
                                return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
                            };
                            let visual_kind = this.file_diff_split_visual_kind(row_ix);
                            let row_word_ranges =
                                this.file_diff_split_word_ranges(row_ix, region);
                            let row_word_color =
                                file_diff_split_word_color(column, visual_kind, theme);
                            let streamed_spec =
                                file_diff_split_side_text_owned(&row, is_left).and_then(
                                    |raw_text| {
                                        prepared_streamed_diff_text_spec(
                                            raw_text,
                                            &query,
                                            query_options,
                                            query_matcher.clone(),
                                            row_word_ranges.clone(),
                                            row_word_color,
                                            language,
                                            syntax_mode,
                                            Arc::clone(&document_text),
                                            Arc::clone(&line_starts),
                                            rows::prepared_diff_syntax_line_for_one_based_line(
                                                syntax_document,
                                                file_diff_split_side_line(&row, is_left),
                                            ),
                                        )
                                    },
                                );
                            let key = this.file_diff_split_cache_key(row_ix, region);
                            if let Some(key) = key
                                && streamed_spec.is_none()
                                && this.diff_text_segments_cache_get(key, cache_epoch).is_none()
                            {
                                let raw_text = file_diff_split_side_text(&row, is_left);
                                if let Some(raw_text) = raw_text {
                                    let (styled, is_pending) =
                                        build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                                            theme,
                                            raw_text,
                                            row_word_ranges.as_slice(),
                                            "",
                                            DiffSyntaxConfig {
                                                language,
                                                mode: syntax_mode,
                                            },
                                            row_word_color,
                                            rows::prepared_diff_syntax_line_for_one_based_line(
                                                syntax_document,
                                                file_diff_split_side_line(&row, is_left),
                                            ),
                                        );
                                    if is_pending {
                                        this.ensure_prepared_syntax_chunk_poll(cx);
                                    }
                                    this.diff_text_segments_cache_set(key, cache_epoch, styled);
                                }
                            }

                            let row_has_content = file_diff_split_side_text(&row, is_left).is_some();
                            let styled = if row_has_content && streamed_spec.is_none() {
                                if let Some(key) = key {
                                    this.diff_text_segments_cache_get_for_query(
                                        key,
                                        query.as_ref(),
                                        query_options,
                                        cache_epoch,
                                    )
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            patch_split_column_row(
                                theme,
                                ui_scale_percent,
                                column,
                                visible_ix,
                                selected,
                                min_width,
                                &row,
                                visual_kind,
                                styled,
                                streamed_spec,
                                reveal_whitespace_chars,
                                show_line_numbers,
                                wrap,
                                annotation_width,
                                if is_left {
                                    blame_ctx.as_ref().and_then(|ctx| {
                                        build_row_blame_paint_tracked(ctx, matches!(visual_kind, FileDiffRowKind::Context), row.old_line, row.new_line, &blame_prev_nl, theme)
                                    })
                                } else {
                                    None
                                },
                                annot_hover,
                                cx,
                            )
                        }
                    }
                })
                .collect();
        }

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            let cache_epoch = this.file_diff_split_style_cache_epoch(region);
            let syntax_document = this.file_diff_split_prepared_syntax_document(region);
            let syntax_mode = DiffSyntaxMode::Auto;
            let document_text: Arc<str> = if is_left {
                this.file_diff_old_text.clone().into()
            } else {
                this.file_diff_new_text.clone().into()
            };
            let line_starts = if is_left {
                Arc::clone(&this.file_diff_old_line_starts)
            } else {
                Arc::clone(&this.file_diff_new_line_starts)
            };

            let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

                    let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
                    };
                    let Some(row) = this.file_diff_split_render_data(row_ix) else {
                        return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
                    };
                    let visual_kind = this.file_diff_split_visual_kind(row_ix);
                    let row_word_ranges = this.file_diff_split_word_ranges(row_ix, region);
                    let row_word_color = file_diff_split_word_color(column, visual_kind, theme);
                    let streamed_spec = file_diff_split_side_text_owned(&row, is_left).and_then(
                        |raw_text| {
                            prepared_streamed_diff_text_spec(
                                raw_text,
                                &query,
                                query_options,
                                query_matcher.clone(),
                                row_word_ranges.clone(),
                                row_word_color,
                                language,
                                syntax_mode,
                                Arc::clone(&document_text),
                                Arc::clone(&line_starts),
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    syntax_document,
                                    file_diff_split_side_line(&row, is_left),
                                ),
                            )
                        },
                    );
                    let key = this.file_diff_split_cache_key(row_ix, region);
                    if let Some(key) = key
                        && streamed_spec.is_none()
                        && this.diff_text_segments_cache_get(key, cache_epoch).is_none()
                    {
                        let raw_text = file_diff_split_side_text(&row, is_left);
                        if let Some(raw_text) = raw_text {
                            let (styled, is_pending) = build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                                theme,
                                raw_text,
                                row_word_ranges.as_slice(),
                                "",
                                DiffSyntaxConfig {
                                    language,
                                    mode: syntax_mode,
                                },
                                row_word_color,
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    syntax_document,
                                    file_diff_split_side_line(&row, is_left),
                                ),
                            );
                            if is_pending {
                                this.ensure_prepared_syntax_chunk_poll(cx);
                            }
                            this.diff_text_segments_cache_set(key, cache_epoch, styled);
                        }
                    }

                    let row_has_content = file_diff_split_side_text(&row, is_left).is_some();
                    let styled = if row_has_content && streamed_spec.is_none() {
                        if let Some(key) = key {
                            this.diff_text_segments_cache_get_for_query(
                                key,
                                query.as_ref(),
                                query_options,
                                cache_epoch,
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    debug_assert!(
                        !row_has_content
                            || key.is_none()
                            || streamed_spec.is_some()
                            || styled.is_some(),
                        "diff text segment cache missing for split-{column:?} row {row_ix} after populate"
                    );

                    patch_split_column_row(
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        selected,
                        min_width,
                        &row,
                        visual_kind,
                        styled,
                        streamed_spec,
                        reveal_whitespace_chars,
                        show_line_numbers,
                        wrap,
                        annotation_width,
                        if is_left {
                            blame_ctx
                                .as_ref()
                                .and_then(|ctx| build_row_blame_paint_tracked(ctx, matches!(visual_kind, FileDiffRowKind::Context), row.old_line, row.new_line, &blame_prev_nl, theme))
                        } else {
                        None
                    },
                    annot_hover,
                    cx,
                )
                })
                .collect();
        }

        let theme = this.theme;
        let cache_epoch = 0u64;
        let syntax_mode = this.patch_diff_syntax_mode();
        let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                let show_line_numbers = this.diff_show_line_numbers;
                let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

                let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
                };
                let Some(row) = this.patch_diff_split_row(row_ix) else {
                    return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
                };

                match row {
                    PatchSplitRow::Aligned {
                        row,
                        old_src_ix,
                        new_src_ix,
                    } => {
                        let src_ix = if is_left { old_src_ix } else { new_src_ix };
                        let old_changed = old_src_ix.is_some_and(|src_ix| {
                            matches!(this.patch_visual_line_kind(src_ix), DiffLineKind::Remove)
                        });
                        let new_changed = new_src_ix.is_some_and(|src_ix| {
                            matches!(this.patch_visual_line_kind(src_ix), DiffLineKind::Add)
                        });
                        let visual_kind = match (old_changed, new_changed) {
                            (true, true) => FileDiffRowKind::Modify,
                            (true, false) => FileDiffRowKind::Remove,
                            (false, true) => FileDiffRowKind::Add,
                            (false, false) => FileDiffRowKind::Context,
                        };
                        let (streamed_spec, styled) = if let Some(src_ix) = src_ix {
                            let language =
                                this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                            this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                            let word_ranges = this
                                .diff_word_highlights
                                .get(src_ix)
                                .and_then(|r| r.as_ref().cloned())
                                .unwrap_or_default();
                            let word_color =
                                diff_line_word_color(this.patch_visual_line_kind(src_ix), theme);
                            let streamed_spec = file_diff_split_side_text_owned(&row, is_left)
                                .and_then(|raw_text| {
                                    heuristic_streamed_diff_text_spec(
                                        raw_text,
                                        &query,
                                        query_options,
                                        query_matcher.clone(),
                                        word_ranges.clone(),
                                        word_color,
                                        language,
                                        syntax_mode,
                                    )
                                });
                            if streamed_spec.is_none()
                                && this
                                    .diff_text_segments_cache_get(src_ix, cache_epoch)
                                    .is_none()
                            {
                                let computed = if let Some(raw_text) =
                                    file_diff_split_side_text(&row, is_left)
                                {
                                    build_file_diff_cached_styled_text(
                                        theme,
                                        raw_text,
                                        word_ranges.as_slice(),
                                        "",
                                        language,
                                        syntax_mode,
                                        word_color,
                                    )
                                } else {
                                    build_cached_diff_styled_text(
                                        theme,
                                        "",
                                        word_ranges.as_slice(),
                                        "",
                                        language,
                                        syntax_mode,
                                        word_color,
                                    )
                                };
                                this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                            }

                            let styled = if streamed_spec.is_none() {
                                this.diff_text_segments_cache_get_for_query(
                                    src_ix,
                                    query.as_ref(),
                                    query_options,
                                    cache_epoch,
                                )
                            } else {
                                None
                            };
                            (streamed_spec, styled)
                        } else {
                            (None, None)
                        };

                        patch_split_column_row(
                            theme,
                            ui_scale_percent,
                            column,
                            visible_ix,
                            selected,
                            min_width,
                            &row,
                            visual_kind,
                            styled,
                            streamed_spec,
                            reveal_whitespace_chars,
                            show_line_numbers,
                            wrap,
                            annotation_width,
                            if is_left {
                                blame_ctx.as_ref().and_then(|ctx| {
                                    build_row_blame_paint_tracked(
                                        ctx,
                                        matches!(visual_kind, FileDiffRowKind::Context),
                                        row.old_line,
                                        row.new_line,
                                        &blame_prev_nl,
                                        theme,
                                    )
                                })
                            } else {
                                None
                            },
                            annot_hover,
                            cx,
                        )
                    }
                    PatchSplitRow::Raw { src_ix, click_kind } => {
                        if this.patch_diff_row(src_ix).is_none() {
                            return diff_placeholder_row(
                                (id_src_oob, visible_ix),
                                theme,
                                ui_scale_percent,
                            );
                        };
                        let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);
                        let should_style = !query.is_empty();
                        if should_style
                            && this
                                .diff_text_segments_cache_get(src_ix, cache_epoch)
                                .is_none()
                        {
                            let display = this.diff_text_line_for_region(visible_ix, region);
                            let computed = build_cached_diff_styled_text(
                                theme,
                                display.as_ref(),
                                &[],
                                "",
                                None,
                                syntax_mode,
                                None,
                            );
                            this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                        }
                        let Some(line) = this.patch_diff_row(src_ix) else {
                            return diff_placeholder_row(
                                (id_src_oob, visible_ix),
                                theme,
                                ui_scale_percent,
                            );
                        };
                        if should_hide_unified_diff_header_line(&line) {
                            return div()
                                .id((id_hidden, visible_ix))
                                .h(px(0.0))
                                .into_any_element();
                        }
                        let context_menu_active = click_kind == DiffClickKind::HunkHeader
                            && this.active_repo_id().is_some_and(|repo_id| {
                                let invoker: SharedString =
                                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                                this.active_context_menu_invoker.as_ref() == Some(&invoker)
                            });
                        let header_display = this.diff_header_display_cache.get(&src_ix).cloned();
                        let styled = if should_style {
                            this.diff_text_segments_cache_get_for_query(
                                src_ix,
                                query.as_ref(),
                                query_options,
                                cache_epoch,
                            )
                        } else {
                            None
                        };
                        patch_split_header_row(
                            theme,
                            ui_scale_percent,
                            column,
                            visible_ix,
                            click_kind,
                            selected,
                            min_width,
                            &line,
                            file_stat,
                            header_display,
                            styled,
                            context_menu_active,
                            cx,
                        )
                    }
                }
            })
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    mode: DiffViewMode,
    min_width: Pixels,
    line: &AnnotatedDiffLine,
    visual_kind: DiffLineKind,
    file_stat: Option<(usize, usize)>,
    header_display: Option<SharedString>,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<diff_canvas::StreamedDiffTextPaintSpec>,
    raw_text: Option<&str>,
    reveal_whitespace_chars: bool,
    context_menu_active: bool,
    show_line_numbers: bool,
    wrap: Option<diff_canvas::DiffTextWrapSlice>,
    annotation_width: Pixels,
    row_blame: Option<diff_canvas::RowBlamePaint>,
    annot_hover: Option<(usize, AnnotArea)>,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, click_kind, e.modifiers().shift);
        cx.notify();
    });

    if matches!(click_kind, DiffClickKind::FileHeader) {
        let file =
            header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));
        let mut row = div()
            .id(("diff_file_hdr", visible_ix))
            .h(diff_file_header_height(ui_scale_percent))
            .w_full()
            .min_w(min_width)
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .bg(theme.colors.surface_bg_elevated)
            .border_b_1()
            .border_color(theme.colors.border)
            .text_sm()
            .font_weight(FontWeight::BOLD)
            .child(selectable_cached_diff_text(
                visible_ix,
                DiffTextRegion::Inline,
                DiffClickKind::FileHeader,
                theme.colors.text,
                None,
                file,
                cx,
            ))
            .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                let (a, r) = file_stat.unwrap_or_default();
                this.child(components::diff_stat(theme, a, r))
            })
            .on_click(on_click);

        if selected {
            row = row.bg(focused_diff_neutral_row_bg(theme));
        }

        return row.into_any_element();
    }

    if matches!(click_kind, DiffClickKind::HunkHeader) {
        let display =
            header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));

        let mut row = div()
            .id(("diff_hunk_hdr", visible_ix))
            .h(diff_hunk_header_height(ui_scale_percent))
            .w_full()
            .min_w(min_width)
            .flex()
            .items_center()
            .px_2()
            .bg(with_alpha(
                theme.colors.accent,
                if theme.is_dark { 0.10 } else { 0.07 },
            ))
            .border_b_1()
            .border_color(with_alpha(
                theme.colors.accent,
                if theme.is_dark { 0.28 } else { 0.22 },
            ))
            .text_xs()
            .text_color(theme.colors.text_muted)
            .child(selectable_cached_diff_text(
                visible_ix,
                DiffTextRegion::Inline,
                DiffClickKind::HunkHeader,
                theme.colors.text_muted,
                None,
                display,
                cx,
            ))
            .on_click(on_click);
        let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            if this.is_inline_submodule_diff_active() {
                return;
            }
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let Some(src_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return;
            };
            let context_menu_invoker: SharedString =
                format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
            this.activate_context_menu_invoker(context_menu_invoker, cx);
            this.open_popover_at(
                PopoverKind::DiffHunkMenu { repo_id, src_ix },
                e.position,
                window,
                cx,
            );
        });
        row = row.on_mouse_down(MouseButton::Right, on_right_click);

        if selected {
            row = row.bg(focused_diff_neutral_row_bg(theme));
        }
        if context_menu_active {
            row = row.bg(theme.colors.active);
        }

        return row.into_any_element();
    }

    let (mut bg, fg, gutter_fg) = diff_line_colors(theme, visual_kind);
    if selected {
        bg = focused_diff_line_bg(theme, visual_kind);
    }

    let show_row_numbers = wrap.is_none_or(|wrap| wrap.wrap_ix == 0);
    let old = if show_row_numbers {
        line_number_string(line.old_line)
    } else {
        SharedString::default()
    };
    let new = if show_row_numbers {
        line_number_string(line.new_line)
    } else {
        SharedString::default()
    };

    match mode {
        DiffViewMode::Inline => diff_canvas::inline_diff_line_row_canvas(
            theme,
            cx.entity(),
            ui_scale_percent,
            visible_ix,
            min_width,
            selected,
            old,
            new,
            bg,
            fg,
            gutter_fg,
            styled,
            streamed_spec,
            raw_text,
            reveal_whitespace_chars,
            show_line_numbers,
            wrap,
            annotation_width,
            row_blame,
            annot_hover,
        ),
        DiffViewMode::Split => {
            let left_kind = if visual_kind == DiffLineKind::Remove {
                DiffLineKind::Remove
            } else {
                DiffLineKind::Context
            };
            let right_kind = if visual_kind == DiffLineKind::Add {
                DiffLineKind::Add
            } else {
                DiffLineKind::Context
            };

            let (mut left_bg, left_fg, left_gutter) = diff_line_colors(theme, left_kind);
            let (mut right_bg, right_fg, right_gutter) = diff_line_colors(theme, right_kind);
            if selected {
                left_bg = focused_diff_line_bg(theme, left_kind);
                right_bg = focused_diff_line_bg(theme, right_kind);
            }

            let (left_text, right_text) = match line.kind {
                DiffLineKind::Remove => (styled, None),
                DiffLineKind::Add => (None, styled),
                DiffLineKind::Context => (styled, styled),
                _ => (styled, None),
            };
            let left_streamed_spec = match line.kind {
                DiffLineKind::Remove | DiffLineKind::Context => streamed_spec.clone(),
                _ => None,
            };
            let right_streamed_spec = match line.kind {
                DiffLineKind::Add | DiffLineKind::Context => streamed_spec,
                _ => None,
            };
            let left_raw_text = match line.kind {
                DiffLineKind::Remove | DiffLineKind::Context => raw_text,
                _ => None,
            };
            let right_raw_text = match line.kind {
                DiffLineKind::Add | DiffLineKind::Context => raw_text,
                _ => None,
            };

            diff_canvas::split_diff_line_row_canvas(
                theme,
                cx.entity(),
                ui_scale_percent,
                visible_ix,
                min_width,
                selected,
                old,
                new,
                left_bg,
                left_fg,
                left_gutter,
                right_bg,
                right_fg,
                right_gutter,
                left_text,
                right_text,
                left_streamed_spec,
                right_streamed_spec,
                left_raw_text,
                right_raw_text,
                reveal_whitespace_chars,
                show_line_numbers,
                wrap,
                annotation_width,
                row_blame,
                annot_hover,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collapsed_inline_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    pinned_hunk_shell_width: Pixels,
    pinned_hunk_shell_scroll: gpui::UniformListScrollHandle,
    collapsed_hunk: Option<CollapsedDiffHunk>,
    file_stat: Option<(usize, usize)>,
    display: SharedString,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    src_ix: usize,
    expansion_kind: CollapsedDiffExpansionKind,
    hidden_rows: usize,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    match click_kind {
        DiffClickKind::FileHeader => {
            let mut row = div()
                .id(("collapsed_diff_file_hdr", visible_ix))
                .h(diff_file_header_height(ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    DiffTextRegion::Inline,
                    DiffClickKind::FileHeader,
                    theme.colors.text,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, a, r))
                });

            if selected {
                row = row.bg(focused_diff_neutral_row_bg(theme));
            }

            row.into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let gutter_w = diff_canvas::diff_inline_text_start(ui_scale_percent);
            let trailing_pad = diff_canvas::diff_row_horizontal_padding(ui_scale_percent);
            let text_color = collapsed_inline_hunk_fg(theme, collapsed_hunk);
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.is_inline_submodule_diff_active() {
                    return;
                }
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                this.activate_context_menu_invoker(context_menu_invoker, cx);
                this.open_popover_at(
                    PopoverKind::DiffHunkMenu { repo_id, src_ix },
                    e.position,
                    window,
                    cx,
                );
            });
            let button_color = if hidden_rows > 0 {
                text_color
            } else {
                with_alpha(text_color, 0.45)
            };
            let controls = match expansion_kind {
                CollapsedDiffExpansionKind::Up => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_up", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_UP_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Down => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_down", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_DOWN_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Down,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Both => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_down", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_DOWN_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::DownBefore,
                            src_ix,
                        },
                        cx,
                    ))
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_up", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_UP_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Short => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_short", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_SHORT_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/plus.svg",
                        "Show hidden lines",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Short,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::None => div().into_any_element(),
            };

            let row_bg = collapsed_inline_hunk_bg(theme, collapsed_hunk, expansion_kind);
            let painted_row_bg = if selected {
                focused_collapsed_hunk_bg(theme, collapsed_hunk)
            } else {
                row_bg
            };
            let mut row = div()
                .id(("collapsed_diff_hunk_hdr", visible_ix))
                .debug_selector(|| COLLAPSED_DIFF_INLINE_HUNK_SHELL_DEBUG_SELECTOR.to_string())
                .h(collapsed_hunk_header_row_height(ui_scale_percent))
                .w(pinned_hunk_shell_width)
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .bg(painted_row_bg)
                .text_xs()
                .text_color(text_color);
            row = row
                .child(
                    div()
                        .debug_selector(|| {
                            COLLAPSED_DIFF_INLINE_HUNK_GUTTER_DEBUG_SELECTOR.to_string()
                        })
                        .w(gutter_w)
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(controls),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .pr(trailing_pad)
                        .overflow_hidden()
                        .child(selectable_cached_diff_text(
                            visible_ix,
                            DiffTextRegion::Inline,
                            DiffClickKind::HunkHeader,
                            text_color,
                            styled,
                            display,
                            cx,
                        )),
                )
                .on_mouse_down(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(painted_row_bg);
            }
            if context_menu_active {
                row = row.bg(theme.colors.active);
            }

            div()
                .h(collapsed_hunk_header_row_height(ui_scale_percent))
                .min_w(min_width)
                .bg(painted_row_bg)
                .child(scroll_pinned_hunk_shell(
                    pinned_hunk_shell_scroll,
                    Some(painted_row_bg),
                    row.into_any_element(),
                ))
                .into_any_element()
        }
        DiffClickKind::Line => diff_placeholder_row(
            ("collapsed_diff_invalid", visible_ix),
            theme,
            ui_scale_percent,
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PatchSplitColumn {
    Left,
    Right,
}

#[allow(clippy::too_many_arguments)]
fn patch_split_column_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    min_width: Pixels,
    row: &gitcomet_core::file_diff::FileDiffRow,
    visual_kind: FileDiffRowKind,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<diff_canvas::StreamedDiffTextPaintSpec>,
    reveal_whitespace_chars: bool,
    show_line_numbers: bool,
    wrap: Option<diff_canvas::DiffTextWrapSlice>,
    annotation_width: Pixels,
    row_blame: Option<diff_canvas::RowBlamePaint>,
    annot_hover: Option<(usize, AnnotArea)>,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let line_kind = match (column, visual_kind) {
        (PatchSplitColumn::Left, FileDiffRowKind::Remove | FileDiffRowKind::Modify) => {
            DiffLineKind::Remove
        }
        (PatchSplitColumn::Right, FileDiffRowKind::Add | FileDiffRowKind::Modify) => {
            DiffLineKind::Add
        }
        _ => DiffLineKind::Context,
    };
    let (mut bg, fg, gutter_fg) = diff_line_colors(theme, line_kind);
    if selected {
        bg = focused_diff_line_bg(theme, line_kind);
    }

    let show_row_number = wrap.is_none_or(|wrap| wrap.wrap_ix == 0);
    let line_no = if show_row_number {
        match column {
            PatchSplitColumn::Left => line_number_string(row.old_line),
            PatchSplitColumn::Right => line_number_string(row.new_line),
        }
    } else {
        SharedString::default()
    };

    diff_canvas::patch_split_column_row_canvas(
        theme,
        cx.entity(),
        ui_scale_percent,
        column,
        visible_ix,
        min_width,
        selected,
        bg,
        fg,
        gutter_fg,
        line_no,
        styled,
        streamed_spec,
        match column {
            PatchSplitColumn::Left => row.old.as_ref(),
            PatchSplitColumn::Right => row.new.as_ref(),
        }
        .map(|text| text.as_ref()),
        reveal_whitespace_chars,
        show_line_numbers,
        wrap,
        annotation_width,
        row_blame,
        annot_hover,
    )
}

#[allow(clippy::too_many_arguments)]
fn patch_split_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    line: &AnnotatedDiffLine,
    file_stat: Option<(usize, usize)>,
    header_display: Option<SharedString>,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, click_kind, e.modifiers().shift);
        cx.notify();
    });
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    match click_kind {
        DiffClickKind::FileHeader => {
            let display =
                header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));
            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_file_hdr",
                        PatchSplitColumn::Right => "diff_split_right_file_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_file_header_height(ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::FileHeader,
                    theme.colors.text,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, a, r))
                })
                .on_click(on_click);

            if selected {
                row = row.bg(focused_diff_neutral_row_bg(theme));
            }

            row.into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let display =
                header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));

            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_hunk_hdr",
                        PatchSplitColumn::Right => "diff_split_right_hunk_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_hunk_header_height(ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .px_2()
                .bg(with_alpha(
                    theme.colors.accent,
                    if theme.is_dark { 0.10 } else { 0.07 },
                ))
                .border_b_1()
                .border_color(with_alpha(
                    theme.colors.accent,
                    if theme.is_dark { 0.28 } else { 0.22 },
                ))
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::HunkHeader,
                    theme.colors.text_muted,
                    styled,
                    display,
                    cx,
                ))
                .on_click(on_click);
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.is_inline_submodule_diff_active() {
                    return;
                }
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return;
                };
                let Some(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::HunkHeader,
                }) = this.patch_diff_split_row(row_ix)
                else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                this.activate_context_menu_invoker(context_menu_invoker, cx);
                this.open_popover_at(
                    PopoverKind::DiffHunkMenu { repo_id, src_ix },
                    e.position,
                    window,
                    cx,
                );
            });
            row = row.on_mouse_down(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(focused_diff_neutral_row_bg(theme));
            }
            if context_menu_active {
                row = row.bg(theme.colors.active);
            }

            row.into_any_element()
        }
        DiffClickKind::Line => patch_split_meta_row(
            theme,
            ui_scale_percent,
            column,
            visible_ix,
            selected,
            line,
            cx,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn collapsed_split_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    pinned_hunk_shell_width: Pixels,
    pinned_hunk_shell_scroll: gpui::UniformListScrollHandle,
    collapsed_hunk: Option<CollapsedDiffHunk>,
    file_stat: Option<(usize, usize)>,
    display: SharedString,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    src_ix: usize,
    expansion_kind: CollapsedDiffExpansionKind,
    hidden_rows: usize,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    match click_kind {
        DiffClickKind::FileHeader => {
            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "collapsed_diff_split_left_file_hdr",
                        PatchSplitColumn::Right => "collapsed_diff_split_right_file_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_file_header_height(ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::FileHeader,
                    theme.colors.text,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, a, r))
                });

            if selected {
                row = row.bg(focused_diff_neutral_row_bg(theme));
            }

            row.into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let gutter_w = diff_canvas::diff_single_column_text_start(ui_scale_percent);
            let trailing_pad = diff_canvas::diff_row_horizontal_padding(ui_scale_percent);
            let text_color = collapsed_split_hunk_fg(theme, column);
            let (
                row_id,
                shell_debug_selector,
                gutter_debug_selector,
                up_id,
                up_debug_selector,
                down_id,
                down_debug_selector,
                short_id,
                short_debug_selector,
            ) = match column {
                PatchSplitColumn::Left => (
                    "collapsed_diff_split_left_hunk_hdr",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHELL_DEBUG_SELECTOR,
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_GUTTER_DEBUG_SELECTOR,
                    "collapsed_diff_split_left_hunk_up",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_UP_DEBUG_SELECTOR,
                    "collapsed_diff_split_left_hunk_down",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_DOWN_DEBUG_SELECTOR,
                    "collapsed_diff_split_left_hunk_short",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHORT_DEBUG_SELECTOR,
                ),
                PatchSplitColumn::Right => (
                    "collapsed_diff_split_right_hunk_hdr",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHELL_DEBUG_SELECTOR,
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_GUTTER_DEBUG_SELECTOR,
                    "collapsed_diff_split_right_hunk_up",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_UP_DEBUG_SELECTOR,
                    "collapsed_diff_split_right_hunk_down",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_DOWN_DEBUG_SELECTOR,
                    "collapsed_diff_split_right_hunk_short",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHORT_DEBUG_SELECTOR,
                ),
            };
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.is_inline_submodule_diff_active() {
                    return;
                }
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                this.activate_context_menu_invoker(context_menu_invoker, cx);
                this.open_popover_at(
                    PopoverKind::DiffHunkMenu { repo_id, src_ix },
                    e.position,
                    window,
                    cx,
                );
            });
            let button_color = if hidden_rows > 0 {
                text_color
            } else {
                with_alpha(text_color, 0.45)
            };
            let controls = match expansion_kind {
                CollapsedDiffExpansionKind::Up => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (up_id, visible_ix),
                        up_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Down => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (down_id, visible_ix),
                        down_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Down,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Both => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (down_id, visible_ix),
                        down_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::DownBefore,
                            src_ix,
                        },
                        cx,
                    ))
                    .child(collapsed_hunk_reveal_button(
                        (up_id, visible_ix),
                        up_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Short => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (short_id, visible_ix),
                        short_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/plus.svg",
                        "Show hidden lines",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Short,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::None => div().into_any_element(),
            };

            let row_bg = collapsed_split_hunk_bg(theme, collapsed_hunk, column);
            let painted_row_bg = if selected {
                focused_collapsed_hunk_bg(theme, collapsed_hunk)
            } else {
                row_bg
            };
            let mut row = div()
                .id((row_id, visible_ix))
                .debug_selector(move || shell_debug_selector.to_string())
                .h(collapsed_hunk_header_row_height(ui_scale_percent))
                .w(pinned_hunk_shell_width)
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .bg(painted_row_bg)
                .text_xs()
                .text_color(text_color)
                .child(
                    div()
                        .debug_selector(move || gutter_debug_selector.to_string())
                        .w(gutter_w)
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(controls),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .pr(trailing_pad)
                        .overflow_hidden()
                        .child(selectable_cached_diff_text(
                            visible_ix,
                            region,
                            DiffClickKind::HunkHeader,
                            text_color,
                            styled,
                            display,
                            cx,
                        )),
                )
                .on_mouse_down(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(painted_row_bg);
            }
            if context_menu_active {
                row = row.bg(theme.colors.active);
            }

            div()
                .h(collapsed_hunk_header_row_height(ui_scale_percent))
                .min_w(min_width)
                .bg(painted_row_bg)
                .child(scroll_pinned_hunk_shell(
                    pinned_hunk_shell_scroll,
                    Some(painted_row_bg),
                    row.into_any_element(),
                ))
                .into_any_element()
        }
        DiffClickKind::Line => diff_placeholder_row(
            (
                match column {
                    PatchSplitColumn::Left => "collapsed_diff_split_left_invalid",
                    PatchSplitColumn::Right => "collapsed_diff_split_right_invalid",
                },
                visible_ix,
            ),
            theme,
            ui_scale_percent,
        ),
    }
}

fn patch_split_meta_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    line: &AnnotatedDiffLine,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, DiffClickKind::Line, e.modifiers().shift);
        cx.notify();
    });
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    let (bg, fg, _) = diff_line_colors(theme, line.kind);
    let mut row = div()
        .id((
            match column {
                PatchSplitColumn::Left => "diff_split_left_meta",
                PatchSplitColumn::Right => "diff_split_right_meta",
            },
            visible_ix,
        ))
        .h(diff_row_height(ui_scale_percent))
        .flex()
        .items_center()
        .px_2()
        .text_xs()
        .bg(bg)
        .text_color(fg)
        .whitespace_nowrap()
        .child(selectable_cached_diff_text(
            visible_ix,
            region,
            DiffClickKind::Line,
            fg,
            None,
            SharedString::from(line.text.as_ref().to_owned()),
            cx,
        ))
        .on_click(on_click);

    if selected {
        row = row.bg(focused_diff_line_bg(theme, line.kind));
    }

    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collapsed_hunk(has_removals: bool, has_additions: bool) -> CollapsedDiffHunk {
        CollapsedDiffHunk {
            src_ix: 0,
            base_row_start: 0,
            base_row_end_exclusive: 1,
            has_additions,
            has_removals,
            reveal_up_lines: 0,
            reveal_down_lines: 0,
        }
    }

    fn uncommitted_blame_line() -> gitcomet_core::services::BlameLine {
        gitcomet_core::services::BlameLine {
            commit_id: std::sync::Arc::from("0000000000000000000000000000000000000000"),
            author: std::sync::Arc::from("Not Committed Yet"),
            author_time_unix: None,
            summary: std::sync::Arc::from("Not Committed Yet"),
            body: None,
            line: String::new(),
            prior_exists: false,
            source_path: None,
            prior_commit: None,
        }
    }

    fn committed_blame_line(sha: &str) -> gitcomet_core::services::BlameLine {
        gitcomet_core::services::BlameLine {
            commit_id: std::sync::Arc::from(sha),
            author: std::sync::Arc::from("Jane Doe"),
            author_time_unix: Some(1_700_000_000),
            summary: std::sync::Arc::from("a commit"),
            body: None,
            line: String::new(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        }
    }

    fn blame_ctx(
        lines: Vec<gitcomet_core::services::BlameLine>,
        area: Option<gitcomet_core::domain::DiffArea>,
    ) -> BlameRenderCtx {
        BlameRenderCtx {
            lines: std::sync::Arc::new(lines),
            range: None,
            now: std::time::SystemTime::now(),
            path: std::sync::Arc::from(std::path::Path::new("file.rs")),
            viewed_commit: None,
            area,
        }
    }

    #[test]
    fn classify_local_change_matrix() {
        use gitcomet_core::domain::DiffArea;
        // Staged area: every local line is staged regardless of context-ness.
        assert_eq!(
            classify_local_change(DiffArea::Staged, true),
            LocalChange::Staged
        );
        assert_eq!(
            classify_local_change(DiffArea::Staged, false),
            LocalChange::Staged
        );
        // Unstaged area: an unchanged context line is a staged change; any actual
        // change (add / modify / remove) is an unstaged change.
        assert_eq!(
            classify_local_change(DiffArea::Unstaged, true),
            LocalChange::Staged
        );
        assert_eq!(
            classify_local_change(DiffArea::Unstaged, false),
            LocalChange::Unstaged
        );
    }

    #[test]
    fn unstaged_diff_separates_staged_context_from_unstaged_add() {
        use gitcomet_core::domain::DiffArea;
        let theme = AppTheme::gitcomet_dark();
        let ctx = blame_ctx(
            vec![uncommitted_blame_line(), uncommitted_blame_line()],
            Some(DiffArea::Unstaged),
        );
        // Uncommitted context line -> staged: green diff-add bar.
        let (staged, _) =
            build_row_blame_paint_inner(&ctx, true, Some(1), Some(1), BlamePrev::default(), theme)
                .unwrap();
        assert_eq!(staged.border, theme.colors.diff_add_text);
        assert_eq!(staged.when.as_ref(), "Staged");
        // Added line -> unstaged: red diff-remove bar.
        let (unstaged, _) =
            build_row_blame_paint_inner(&ctx, false, None, Some(2), BlamePrev::default(), theme)
                .unwrap();
        assert_eq!(unstaged.border, theme.colors.diff_remove_text);
        assert_eq!(unstaged.when.as_ref(), "Unstaged");
    }

    #[test]
    fn modified_line_with_both_sides_is_unstaged_not_staged() {
        use gitcomet_core::domain::DiffArea;
        let theme = AppTheme::gitcomet_dark();
        let ctx = blame_ctx(vec![uncommitted_blame_line()], Some(DiffArea::Unstaged));
        // A split `Modify` row has both old and new line numbers, but it is a
        // change, not unchanged context. Unstaged must override staged here.
        let (paint, _) =
            build_row_blame_paint_inner(&ctx, false, Some(1), Some(1), BlamePrev::default(), theme)
                .unwrap();
        assert_eq!(paint.border, theme.colors.diff_remove_text);
        assert_eq!(paint.when.as_ref(), "Unstaged");
    }

    #[test]
    fn staged_area_labels_all_local_as_staged() {
        use gitcomet_core::domain::DiffArea;
        let theme = AppTheme::gitcomet_dark();
        let ctx = blame_ctx(vec![uncommitted_blame_line()], Some(DiffArea::Staged));
        // Even an added line is "Staged" when viewing the staged area.
        let (paint, _) =
            build_row_blame_paint_inner(&ctx, false, None, Some(1), BlamePrev::default(), theme)
                .unwrap();
        assert_eq!(paint.border, theme.colors.diff_add_text);
        assert_eq!(paint.when.as_ref(), "Staged");
    }

    #[test]
    fn revision_blame_keeps_now_label() {
        let theme = AppTheme::gitcomet_dark();
        // No working-tree area (revision blame) -> legacy generic local change.
        let ctx = blame_ctx(vec![uncommitted_blame_line()], None);
        let (paint, _) =
            build_row_blame_paint_inner(&ctx, true, Some(1), Some(1), BlamePrev::default(), theme)
                .unwrap();
        assert_eq!(
            paint.border,
            crate::theme::blame_local_change_color(theme.is_dark)
        );
        assert_eq!(paint.when.as_ref(), "Now");
    }

    #[test]
    fn run_breaks_at_staged_unstaged_boundary() {
        use gitcomet_core::domain::DiffArea;
        let theme = AppTheme::gitcomet_dark();
        let ctx = blame_ctx(
            vec![
                uncommitted_blame_line(),
                uncommitted_blame_line(),
                uncommitted_blame_line(),
            ],
            Some(DiffArea::Unstaged),
        );
        let prev = std::cell::Cell::new(BlamePrev::default());
        // Line 1: staged context -> run start, label shown.
        let p1 = build_row_blame_paint_tracked(&ctx, true, Some(1), Some(1), &prev, theme).unwrap();
        assert!(p1.show_text);
        assert_eq!(p1.when.as_ref(), "Staged");
        // Line 2: staged context, contiguous -> not a run start (label not repeated).
        let p2 = build_row_blame_paint_tracked(&ctx, true, Some(2), Some(2), &prev, theme).unwrap();
        assert!(!p2.show_text);
        // Line 3: unstaged add. The new-side line is still contiguous, but the
        // staged->unstaged group change must start a new run with its label.
        let p3 = build_row_blame_paint_tracked(&ctx, false, None, Some(3), &prev, theme).unwrap();
        assert!(p3.show_text);
        assert_eq!(p3.when.as_ref(), "Unstaged");
    }

    #[test]
    fn untracked_content_view_collapses_consecutive_same_commit_lines() {
        // The full file-content view uses the untracked `build_row_blame_paint`
        // (no threaded group). Consecutive lines of the same commit must collapse
        // into one run: the message shows on the first line only, not repeated.
        let theme = AppTheme::gitcomet_dark();
        let sha = "1111111111111111111111111111111111111111";
        let ctx = blame_ctx(
            vec![committed_blame_line(sha), committed_blame_line(sha)],
            Some(gitcomet_core::domain::DiffArea::Unstaged),
        );
        // Line 1 (no previous rendered line) starts the run -> shows the summary.
        let first = build_row_blame_paint(&ctx, false, None, Some(1), None, theme).unwrap();
        assert!(first.show_text);
        assert_eq!(first.summary.as_ref(), "a commit");
        // Line 2 is contiguous and same commit -> not a run start, no repeated text.
        let second = build_row_blame_paint(&ctx, false, None, Some(2), Some(1), theme).unwrap();
        assert!(!second.show_text);
        assert!(second.when.as_ref().is_empty());
        assert!(second.summary.as_ref().is_empty());
    }

    #[test]
    fn removal_rows_get_a_local_bar_only_in_working_tree_blame() {
        use gitcomet_core::domain::DiffArea;
        let theme = AppTheme::gitcomet_dark();
        // Pure removal (old side only) in the unstaged area -> unstaged bar + label,
        // even though there is no `BlameLine` for a deleted line.
        let ctx = blame_ctx(Vec::new(), Some(DiffArea::Unstaged));
        let prev = std::cell::Cell::new(BlamePrev::default());
        let removal =
            build_row_blame_paint_tracked(&ctx, false, Some(5), None, &prev, theme).unwrap();
        assert_eq!(removal.border, theme.colors.diff_remove_text);
        assert_eq!(removal.when.as_ref(), "Unstaged");
        // Revision blame has no staged/unstaged concept, so removals get no bar.
        let ctx_rev = blame_ctx(Vec::new(), None);
        let prev_rev = std::cell::Cell::new(BlamePrev::default());
        assert!(
            build_row_blame_paint_tracked(&ctx_rev, false, Some(5), None, &prev_rev, theme)
                .is_none()
        );
    }

    #[test]
    fn focused_diff_row_backgrounds_are_semantic_and_not_text_selection() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let text_selection_bg =
                with_alpha(theme.colors.accent, if theme.is_dark { 0.28 } else { 0.18 });
            let add_focus = focused_diff_line_bg(theme, DiffLineKind::Add);
            let remove_focus = focused_diff_line_bg(theme, DiffLineKind::Remove);
            let neutral_focus = focused_diff_line_bg(theme, DiffLineKind::Context);
            let (add_bg, _, _) = diff_line_colors(theme, DiffLineKind::Add);
            let (remove_bg, _, _) = diff_line_colors(theme, DiffLineKind::Remove);
            let (context_bg, _, _) = diff_line_colors(theme, DiffLineKind::Context);

            assert_ne!(add_focus, text_selection_bg);
            assert_ne!(remove_focus, text_selection_bg);
            assert_ne!(neutral_focus, text_selection_bg);
            assert_ne!(
                neutral_focus,
                with_alpha(theme.colors.warning, focused_diff_row_alpha(theme))
            );
            assert_ne!(add_focus, add_bg);
            assert_ne!(remove_focus, remove_bg);
            assert_ne!(neutral_focus, context_bg);
            assert_ne!(add_focus, remove_focus);
            assert_ne!(add_focus, neutral_focus);
            assert_ne!(remove_focus, neutral_focus);

            let collapsed_focus = focused_collapsed_hunk_bg(theme, None);
            let expected_collapsed_focus =
                with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 });
            assert_eq!(collapsed_focus, expected_collapsed_focus);
            assert_ne!(
                collapsed_focus,
                with_alpha(theme.colors.accent, focused_diff_row_alpha(theme))
            );
            assert_ne!(collapsed_focus, text_selection_bg);
            assert_eq!(
                focused_collapsed_hunk_bg(theme, Some(collapsed_hunk(false, true))),
                collapsed_focus
            );
            assert_eq!(
                focused_collapsed_hunk_bg(theme, Some(collapsed_hunk(true, false))),
                collapsed_focus
            );
            assert_eq!(
                focused_collapsed_hunk_bg(theme, Some(collapsed_hunk(true, true))),
                collapsed_focus
            );
            assert_eq!(focused_collapsed_hunk_bg(theme, None), collapsed_focus);
        }
    }

    #[test]
    fn collapsed_hunk_headers_use_uniform_diff_row_height() {
        for ui_scale_percent in [75, 100, 125, 150, 200] {
            assert_eq!(
                collapsed_hunk_header_row_height(ui_scale_percent),
                diff_row_height(ui_scale_percent)
            );
            assert_ne!(
                collapsed_hunk_header_row_height(ui_scale_percent),
                diff_hunk_header_height(ui_scale_percent)
            );
        }
    }

    #[test]
    fn collapsed_inline_hunk_headers_use_neutral_colors() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let neutral = collapsed_hunk_header_bg(theme);

            assert_eq!(
                collapsed_inline_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, false)),
                    CollapsedDiffExpansionKind::Up,
                ),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_bg(
                    theme,
                    Some(collapsed_hunk(false, true)),
                    CollapsedDiffExpansionKind::Up,
                ),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, true)),
                    CollapsedDiffExpansionKind::Up,
                ),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, true)),
                    CollapsedDiffExpansionKind::Both,
                ),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, true)),
                    CollapsedDiffExpansionKind::Short,
                ),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, true)),
                    CollapsedDiffExpansionKind::Down,
                ),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_bg(theme, None, CollapsedDiffExpansionKind::Up),
                neutral
            );
            assert_eq!(
                collapsed_inline_hunk_fg(theme, Some(collapsed_hunk(true, false))),
                theme.colors.text_muted
            );
            assert_eq!(
                collapsed_inline_hunk_fg(theme, Some(collapsed_hunk(false, true))),
                theme.colors.text_muted
            );
            assert_eq!(
                collapsed_inline_hunk_fg(theme, Some(collapsed_hunk(true, true))),
                theme.colors.text_muted
            );
            assert_eq!(
                collapsed_inline_hunk_fg(theme, None),
                theme.colors.text_muted
            );
        }
    }

    #[test]
    fn collapsed_split_hunk_headers_use_neutral_colors_for_both_sides() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let neutral = collapsed_hunk_header_bg(theme);

            assert_eq!(
                collapsed_split_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, false)),
                    PatchSplitColumn::Left,
                ),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, false)),
                    PatchSplitColumn::Right,
                ),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(
                    theme,
                    Some(collapsed_hunk(false, true)),
                    PatchSplitColumn::Left,
                ),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(
                    theme,
                    Some(collapsed_hunk(false, true)),
                    PatchSplitColumn::Right,
                ),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, true)),
                    PatchSplitColumn::Left,
                ),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(
                    theme,
                    Some(collapsed_hunk(true, true)),
                    PatchSplitColumn::Right,
                ),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(theme, None, PatchSplitColumn::Left),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_bg(theme, None, PatchSplitColumn::Right),
                neutral
            );
            assert_eq!(
                collapsed_split_hunk_fg(theme, PatchSplitColumn::Left),
                theme.colors.text_muted
            );
            assert_eq!(
                collapsed_split_hunk_fg(theme, PatchSplitColumn::Right),
                theme.colors.text_muted
            );
        }
    }
}
