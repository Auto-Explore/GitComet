use super::canvas::keyed_canvas;
use super::canvas_text;
use super::canvas_text::LineMetrics;
use super::canvas_text::center_text_y;
use super::canvas_text::compute_runs;
use super::canvas_text::diff_text_style;
use super::canvas_text::line_metrics;
use super::diff_text::hash_rgba_bits as hash_rgba;
use super::diff_text::whitespace_visible_styled_text;
use super::*;
use crate::view::panes::diff::diff_search::DiffSearchMatcher;
use crate::view::panes::diff::diff_search::DiffSearchOptions;
use crate::view::panes::diff::helpers::state::DiffChangeSide;
use crate::view::panes::diff::helpers::state::DiffHorizontalScrollColumn;
use crate::view::panes::diff::helpers::state::FocusedChangeBlockRow;
use annotation::AnnotHitboxes;
use annotation::build_annot_hitboxes;
use annotation::line_metrics_annot_when;
use annotation::mix_blame_revision;
use annotation::render_blame_column;
use gitcomet_core::domain::DiffArea;
use gitcomet_core::domain::DiffLineKind;
use gitcomet_ui_kit::text_layout::TextLayoutExt as _;
use gpui::App;
use gpui::Bounds;
use gpui::CursorStyle;
use gpui::DispatchPhase;
use gpui::HighlightStyle;
use gpui::Hitbox;
use gpui::HitboxBehavior;
use gpui::Pixels;
use gpui::Styled;
use gpui::TextStyle;
use gpui::TransformationMatrix;
use gpui::Window;
use gpui::fill;
use gpui::point;
use gpui::px;
use gpui::size;
use interaction::DiffRowMouseHandlers;
use interaction::DiffRowMouseUpBehavior;
use interaction::DiffRowRightClickBehavior;
use interaction::DiffRowTextRegions;
use interaction::diff_canvas_click_target;
use interaction::install_diff_row_mouse_handlers;
use palette::IntoColor;
use rustc_hash::FxHashMap;
use rustc_hash::FxHasher;
use stage_gutter::DIFF_STAGE_GUTTER_STAGE_ICON;
use stage_gutter::DIFF_STAGE_GUTTER_UNSTAGE_ICON;
use stage_gutter::StageGutterPrepaint;
use stage_gutter::build_stage_gutter;
use stage_gutter::mix_stage_gutter_revision;
use stage_gutter::paint_stage_gutter;
use std::cell::RefCell;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Range;
use std::sync::Arc;
use streamed_text::STREAMED_DIFF_TEXT_MIN_BYTES;
use streamed_text::STREAMED_DIFF_TEXT_OVERSCAN_COLUMNS;
use streamed_text::build_streamed_diff_slice_styled_text;
use streamed_text::diff_text_paint_payload;
use streamed_text::should_stream_diff_text;
use streamed_text::streamed_diff_text_ascii_cell_width;
use streamed_text::streamed_diff_text_visible_slice_range;
use streamed_text::whitespace_marker_len;

pub(super) mod annotation;
mod interaction;
mod stage_gutter;
mod streamed_text;

const GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 16_384;

const DIFF_TEXT_WRAP_WIDTH_SAMPLE: &str = "WWWWWWWWWW";

/// Width of a line-number cell (excluding the shared horizontal padding).
/// Sized to fit six digits of the diff monospace font; numbers right-align
/// toward the content so any slack sits before the digits, not between the
/// digits and the code.
const DIFF_GUTTER_BASE_WIDTH_PX: f32 = 38.0;

const DIFF_ROW_HORIZONTAL_PADDING_PX: f32 = 8.0;

const DIFF_ROW_TEXT_TRAILING_PADDING_PX: f32 = 16.0;

const DIFF_CHANGE_BAR_WIDTH_PX: f32 = 3.0;

/// Outline around the change block F2/F3 landed on.
const FOCUSED_CHANGE_BLOCK_OUTLINE_WIDTH_PX: f32 = 1.0;

const DIFF_ROW_BACKGROUND_OVERDRAW_PX: f32 = 1.0;

/// Per-row blame data prepared for painting in the annotation column.
#[derive(Clone)]
pub(in crate::view) struct RowBlamePaint {
    /// Recency "heat" color for the left border bar (older → newer).
    pub(in crate::view) border: gpui::Rgba,
    /// Whether to paint the textual annotation + action icons. `false` on
    /// interior lines of a same-commit run (only the border bar is painted).
    pub(in crate::view) show_text: bool,
    /// Relative time ("3 days ago").
    pub(in crate::view) when: SharedString,
    /// Author initials.
    pub(in crate::view) initials: SharedString,
    /// Commit summary (truncated with an ellipsis at paint time).
    pub(in crate::view) summary: SharedString,
    /// Commit message body (everything after the first line), used for tooltips.
    pub(in crate::view) body: Option<SharedString>,
    /// Commit attributed to this line, used for click handling.
    pub(in crate::view) commit_id: gitcomet_core::domain::CommitId,
    /// File path of the annotated view, used by the "view prior change" action.
    pub(in crate::view) path: std::sync::Arc<std::path::Path>,
    /// The file's path at `commit_id` when it differs from `path` because the
    /// file was renamed at/after that commit. Navigation actions ("view file at
    /// this commit" / "prior revision") use this historical name so they don't
    /// look up the current path in an older tree where it doesn't exist.
    pub(in crate::view) source_path: Option<std::sync::Arc<std::path::Path>>,
    /// Whether the file existed in this commit's parent. When `false`, the
    /// "view file at parent commit" icon is hidden and non-interactive (the
    /// commit introduced the file, so there is no prior revision to show).
    pub(in crate::view) prior_exists: bool,
    /// For an uncommitted ("Now") line, the base revision the working-tree change
    /// was made against (git blame porcelain `previous`). When `Some`, the
    /// "view file at parent commit" icon is shown on the local-change row and
    /// navigating opens that revision directly. `None` for committed lines (which
    /// resolve their parent from `commit_id`) and for newly-added files.
    pub(in crate::view) prior_commit: Option<gitcomet_core::domain::CommitId>,
    /// Whether this line's commit is the revision currently being viewed. When
    /// `true`, the "view file at this commit" icon is hidden and non-interactive
    /// (navigating there would be a no-op).
    pub(in crate::view) is_viewed_commit: bool,
}

/// Fixed sub-column geometry for the annotation column, shared by painting and
/// hit-testing so they stay in sync.
pub(in crate::view) struct BlameColumnLayout {
    pub(in crate::view) border: Bounds<Pixels>,
    pub(in crate::view) when_x: Pixels,
    pub(in crate::view) initials_x: Pixels,
    pub(in crate::view) message: Bounds<Pixels>,
    pub(in crate::view) prior_icon: Bounds<Pixels>,
    pub(in crate::view) browse_icon: Bounds<Pixels>,
}

/// Paint `path` as a square icon of `glyph` size, centered in `cell` and clamped
/// so it never spills out of it.
pub(super) fn paint_centered_svg_icon(
    path: impl Into<SharedString>,
    cell: Bounds<Pixels>,
    glyph: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
    cx: &mut App,
) {
    let glyph = glyph.min(cell.size.width).min(cell.size.height);
    let ox = cell.left() + (cell.size.width - glyph) * 0.5;
    let oy = cell.top() + (cell.size.height - glyph) * 0.5;
    let bounds = Bounds::new(point(ox, oy), size(glyph, glyph));
    let _ = window.paint_svg(
        bounds,
        path.into(),
        None,
        TransformationMatrix::unit(),
        color.into_color(),
        cx,
    );
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::view) enum AnnotArea {
    Message,
    PriorIcon,
    BrowseIcon,
}

/// Which column's left gutter a stage/unstage button belongs to. Split views
/// paint one button per column for the same row, so the row index alone cannot
/// identify a button.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(in crate::view) enum DiffStageSlot {
    Inline,
    SplitLeft,
    SplitRight,
}

/// The stage-gutter button the pointer is currently on or near. Hovering
/// anywhere in the row reveals its button; hovering the button itself is tracked
/// separately so it can render brighter and show a tooltip.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(in crate::view) struct DiffStageHover {
    pub(in crate::view) visible_ix: usize,
    pub(in crate::view) slot: DiffStageSlot,
    pub(in crate::view) on_button: bool,
}

/// What a row's stage-gutter button does. Rows that get no button (context
/// lines, headers, diffs that are not worktree diffs) pass `None` instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct StageGutterSpec {
    /// Which side of the index the shown diff is: an unstaged diff stages the
    /// line, a staged diff unstages it.
    pub(in crate::view) area: DiffArea,
    pub(in crate::view) slot: DiffStageSlot,
    /// The change this button acts on, used to resolve the row back to a single
    /// patch source line on click.
    pub(in crate::view) kind: DiffLineKind,
}

impl StageGutterSpec {
    fn icon(self) -> &'static str {
        match self.area {
            DiffArea::Unstaged => DIFF_STAGE_GUTTER_STAGE_ICON,
            DiffArea::Staged => DIFF_STAGE_GUTTER_UNSTAGE_ICON,
        }
    }

    fn color(self, theme: AppTheme) -> gpui::Rgba {
        match self.area {
            DiffArea::Unstaged => theme.colors.diff.added.foreground,
            DiffArea::Staged => theme.colors.diff.removed.foreground,
        }
    }

    pub(super) fn tooltip(self) -> SharedString {
        match self.area {
            DiffArea::Unstaged => SharedString::from("Stage line"),
            DiffArea::Staged => SharedString::from("Unstage line"),
        }
    }
}

thread_local! {
    static GUTTER_TEXT_LAYOUT_CACHE: RefCell<FxLruCache<u64, gpui::ShapedLine>> =
        RefCell::new(new_fx_lru_cache(GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES));
    static STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE: RefCell<FxHashMap<u64, Pixels>> =
        RefCell::new(FxHashMap::default());
}

#[derive(Clone)]
pub(super) enum StreamedDiffTextSyntaxSource {
    None,
    Heuristic {
        language: rows::DiffSyntaxLanguage,
        mode: rows::DiffSyntaxMode,
    },
    Prepared {
        document_text: Arc<str>,
        line_starts: Arc<[usize]>,
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_ix: usize,
    },
}

#[derive(Clone)]
pub(super) struct StreamedDiffTextPaintSpec {
    pub(super) raw_text: gitcomet_core::file_diff::FileDiffLineText,
    pub(super) query: SharedString,
    pub(super) query_options: DiffSearchOptions,
    pub(super) query_matcher: Option<Arc<DiffSearchMatcher>>,
    /// Only the read-only file view sets this; the diff and conflict views mark
    /// their current match by selecting the row instead.
    pub(super) query_emphasis: DiffSearchMatchEmphasis,
    pub(super) word_ranges: Arc<[Range<usize>]>,
    pub(super) word_kind: Option<crate::theme::DiffColorKind>,
    pub(super) syntax: StreamedDiffTextSyntaxSource,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct DiffWrapByteRange {
    pub(in crate::view) start: usize,
    pub(in crate::view) end: usize,
}

impl DiffWrapByteRange {
    pub(in crate::view) fn from_range(range: Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }

    pub(in crate::view) fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffTextWrapSlice {
    pub(in crate::view) wrap_ix: usize,
    pub(in crate::view) wrap_columns: usize,
    pub(in crate::view) primary_range: DiffWrapByteRange,
    pub(in crate::view) secondary_range: DiffWrapByteRange,
}

impl DiffTextWrapSlice {
    pub(in crate::view) fn range_for_region(self, region: DiffTextRegion) -> Range<usize> {
        match region {
            DiffTextRegion::Inline | DiffTextRegion::SplitLeft => self.primary_range.range(),
            DiffTextRegion::SplitRight => self.secondary_range.range(),
        }
    }
}

fn hash_shared_string(hasher: &mut FxHasher, text: &SharedString) {
    text.as_ref().hash(hasher);
}

fn row_bg_fill_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        bounds.origin,
        size(
            bounds.size.width,
            bounds.size.height + px(DIFF_ROW_BACKGROUND_OVERDRAW_PX),
        ),
    )
}

/// Paint the neutral sidebar quad behind the annotation column (width `annot_w`)
/// so selection tint does not bleed into it.
fn paint_annotation_sidebar(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    annot_w: Pixels,
    sidebar_bg: gpui::Rgba,
) {
    window.paint_quad(fill(
        row_bg_fill_bounds(Bounds::new(
            bounds.origin,
            size(annot_w, bounds.size.height),
        )),
        sidebar_bg,
    ));
}

/// Fill a single-content-area row background, reserving a neutral annotation
/// sidebar (width `annot_w`) at the left. With `annot_w == 0` this simply fills
/// `bounds` with `bg`.
fn paint_row_bg_with_annotation(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    annot_w: Pixels,
    bg: gpui::Rgba,
    sidebar_bg: gpui::Rgba,
) {
    if annot_w > px(0.0) {
        paint_annotation_sidebar(window, bounds, annot_w, sidebar_bg);
        window.paint_quad(fill(row_bg_fill_bounds(inset_left(bounds, annot_w)), bg));
    } else {
        window.paint_quad(fill(row_bg_fill_bounds(bounds), bg));
    }
}

/// One integer id per (row, revision) instead of a `NamedChild` carrying a
/// freshly formatted hex `String` (an allocation plus an `Arc` per visible
/// row per frame). The identity still changes exactly when the revision does.
fn diff_row_canvas_id(name: &'static str, visible_ix: usize, revision: u64) -> gpui::ElementId {
    let mut hasher = FxHasher::default();
    visible_ix.hash(&mut hasher);
    revision.hash(&mut hasher);
    (name, hasher.finish()).into()
}

fn inline_row_canvas_revision_key(
    old: &SharedString,
    new: &SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    text_hash: u64,
    highlights_hash: u64,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_shared_string(&mut hasher, old);
    hash_shared_string(&mut hasher, new);
    hash_rgba(&mut hasher, bg);
    hash_rgba(&mut hasher, fg);
    hash_rgba(&mut hasher, gutter_fg);
    text_hash.hash(&mut hasher);
    highlights_hash.hash(&mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
fn split_row_canvas_revision_key(
    old: &SharedString,
    new: &SharedString,
    left_bg: gpui::Rgba,
    left_fg: gpui::Rgba,
    left_gutter: gpui::Rgba,
    right_bg: gpui::Rgba,
    right_fg: gpui::Rgba,
    right_gutter: gpui::Rgba,
    left_text_hash: u64,
    left_highlights_hash: u64,
    right_text_hash: u64,
    right_highlights_hash: u64,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_shared_string(&mut hasher, old);
    hash_shared_string(&mut hasher, new);
    hash_rgba(&mut hasher, left_bg);
    hash_rgba(&mut hasher, left_fg);
    hash_rgba(&mut hasher, left_gutter);
    hash_rgba(&mut hasher, right_bg);
    hash_rgba(&mut hasher, right_fg);
    hash_rgba(&mut hasher, right_gutter);
    left_text_hash.hash(&mut hasher);
    left_highlights_hash.hash(&mut hasher);
    right_text_hash.hash(&mut hasher);
    right_highlights_hash.hash(&mut hasher);
    hasher.finish()
}

fn patch_split_row_canvas_revision_key(
    line_no: &SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    text_hash: u64,
    highlights_hash: u64,
) -> u64 {
    let mut hasher = FxHasher::default();
    hash_shared_string(&mut hasher, line_no);
    hash_rgba(&mut hasher, bg);
    hash_rgba(&mut hasher, fg);
    hash_rgba(&mut hasher, gutter_fg);
    text_hash.hash(&mut hasher);
    highlights_hash.hash(&mut hasher);
    hasher.finish()
}

fn semantic_diff_row_bg(theme: AppTheme, bg: gpui::Rgba) -> Option<gpui::Rgba> {
    (bg != theme.colors.surface.canvas).then_some(bg)
}

fn focused_row_outline_color(theme: AppTheme, bg: gpui::Rgba) -> gpui::Rgba {
    with_alpha(bg, if theme.is_dark { 0.72 } else { 0.56 })
}

/// Marks a row of the change block F2/F3 landed on: the accent bar the
/// conflict resolver puts on its active conflict, plus an outline in the
/// change's colour that the block's first and last rows close. `left..right`
/// is the column; both edges are pinned to the visible area so horizontal
/// scrolling keeps the marks in view.
#[allow(clippy::too_many_arguments)]
fn paint_focused_change_block_marks(
    window: &mut Window,
    row_bounds: Bounds<Pixels>,
    left: Pixels,
    right: Pixels,
    row: FocusedChangeBlockRow,
    outline: Option<DiffChangeSide>,
    theme: AppTheme,
    ui_scale_percent: u32,
) {
    let clip = window.content_mask().bounds;
    let left = left.max(clip.left());
    let right = right.min(clip.right());
    if right <= left {
        return;
    }
    let top = row_bounds.top();
    let height = row_bounds.size.height;
    // Overdrawn like the row fill so stacked rows join, but not past the block.
    let run_height = if row.bottom {
        height
    } else {
        height + px(DIFF_ROW_BACKGROUND_OVERDRAW_PX)
    };

    if let Some(side) = outline {
        let color = match side {
            DiffChangeSide::Removed => theme.colors.diff.removed.foreground,
            DiffChangeSide::Added => theme.colors.diff.added.foreground,
        };
        let line_w = diff_scaled_px(FOCUSED_CHANGE_BLOCK_OUTLINE_WIDTH_PX, ui_scale_percent);
        let width = right - left;
        window.paint_quad(fill(
            Bounds::new(point(right - line_w, top), size(line_w, run_height)),
            color,
        ));
        if row.top {
            window.paint_quad(fill(
                Bounds::new(point(left, top), size(width, line_w)),
                color,
            ));
        }
        if row.bottom {
            window.paint_quad(fill(
                Bounds::new(point(left, top + height - line_w), size(width, line_w)),
                color,
            ));
        }
    }

    // The bar is the outline's left edge, painted last so it caps the corners.
    window.paint_quad(fill(
        Bounds::new(
            point(left, top),
            size(
                diff_scaled_px(DIFF_CHANGE_BAR_WIDTH_PX, ui_scale_percent),
                run_height,
            ),
        ),
        theme.colors.accent.foreground,
    ));
}

/// One column of one row painting the focused change block's marks.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct FocusedChangeBlockPaint {
    pub(in crate::view) visible_ix: usize,
    pub(in crate::view) region: DiffTextRegion,
    pub(in crate::view) outline: Option<DiffChangeSide>,
    pub(in crate::view) top: bool,
    pub(in crate::view) bottom: bool,
}

#[cfg(test)]
thread_local! {
    static FOCUSED_CHANGE_BLOCK_PAINT_LOG: RefCell<Vec<FocusedChangeBlockPaint>> =
        const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_focused_change_block_for_tests(
    visible_ix: usize,
    region: DiffTextRegion,
    row: FocusedChangeBlockRow,
    outline: Option<DiffChangeSide>,
) {
    FOCUSED_CHANGE_BLOCK_PAINT_LOG.with(|log| {
        log.borrow_mut().push(FocusedChangeBlockPaint {
            visible_ix,
            region,
            outline,
            top: row.top,
            bottom: row.bottom,
        })
    });
}

#[cfg(test)]
pub(in crate::view) fn clear_focused_change_block_paint_log_for_tests() {
    FOCUSED_CHANGE_BLOCK_PAINT_LOG.with(|log| log.borrow_mut().clear());
}

/// Focused change block marks painted since the last clear.
#[cfg(test)]
pub(in crate::view) fn focused_change_block_paint_log_for_tests() -> Vec<FocusedChangeBlockPaint> {
    FOCUSED_CHANGE_BLOCK_PAINT_LOG.with(|log| log.borrow().clone())
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(in crate::view) struct DiffPaintRecord {
    pub(in crate::view) visible_ix: usize,
    pub(in crate::view) region: DiffTextRegion,
    pub(in crate::view) text: SharedString,
    pub(in crate::view) highlights: Vec<(Range<usize>, Option<gpui::Hsla>, Option<gpui::Hsla>)>,
    pub(in crate::view) row_bg: Option<gpui::Rgba>,
    /// Local offset ranges the matching-pair quad was painted for. Recorded as
    /// ranges rather than x coordinates: `#[gpui::test]` shapes on a
    /// `NoopTextSystem`, so a painted x proves nothing about which character
    /// it covered.
    pub(in crate::view) pair_quads: Vec<Range<usize>>,
    /// Local offset ranges the occurrence quads were painted for.
    pub(in crate::view) occurrence_quads: Vec<Range<usize>>,
}

#[cfg(test)]
thread_local! {
    static DIFF_PAINT_LOG: RefCell<Vec<DiffPaintRecord>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_diff_paint_for_tests(
    visible_ix: usize,
    region: DiffTextRegion,
    text: &SharedString,
    highlights: &[(Range<usize>, HighlightStyle)],
    row_bg: Option<gpui::Rgba>,
    pair_quads: &[Range<usize>],
    occurrence_quads: &[Range<usize>],
) {
    DIFF_PAINT_LOG.with(|log| {
        log.borrow_mut().push(DiffPaintRecord {
            visible_ix,
            region,
            text: text.clone(),
            highlights: highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
            row_bg,
            pair_quads: pair_quads.to_vec(),
            occurrence_quads: occurrence_quads.to_vec(),
        });
    });
}

#[cfg(test)]
pub(in crate::view) fn clear_diff_paint_log_for_tests() {
    DIFF_PAINT_LOG.with(|log| log.borrow_mut().clear());
}

#[cfg(test)]
pub(in crate::view) fn diff_paint_log_for_tests() -> Vec<DiffPaintRecord> {
    DIFF_PAINT_LOG.with(|log| log.borrow().clone())
}

pub(in crate::view) fn is_streamable_diff_text(
    text: &gitcomet_core::file_diff::FileDiffLineText,
) -> bool {
    text.len() >= STREAMED_DIFF_TEXT_MIN_BYTES && !text.has_tabs_without_loading()
}

/// A row's text as its producer holds it. The paint payload reads a
/// source-backed line only when it needs the whole line.
#[derive(Clone, Copy)]
pub(super) enum DiffRowText<'a> {
    Str(&'a str),
    Line(&'a gitcomet_core::file_diff::FileDiffLineText),
}

impl<'a> DiffRowText<'a> {
    fn as_str(self) -> &'a str {
        match self {
            Self::Str(text) => text,
            Self::Line(line) => line.as_ref(),
        }
    }
}

pub(in crate::view) fn whitespace_visible_diff_offset_map(
    tab_width: usize,
    text: &str,
    append_eol_marker: bool,
) -> DiffTextOffsetMap {
    let source_len = crate::view::diff_utils::diff_text_display_len(tab_width, text);
    let mut display_len = text.chars().map(whitespace_marker_len).sum::<usize>();
    let append_synthetic_eol = append_eol_marker && !text.ends_with('\n');
    if append_synthetic_eol {
        display_len = display_len.saturating_add('\u{21B5}'.len_utf8());
    }

    let mut display_to_source = vec![0usize; display_len.saturating_add(1)];
    let mut source_to_display = vec![0usize; source_len.saturating_add(1)];
    let mut source = 0usize;
    let mut display = 0usize;
    let mut column = 0usize;

    for ch in text.chars() {
        let source_start = source;
        let display_start = display;
        source = source.saturating_add(crate::view::tab_width::char_expanded_len(
            tab_width, ch, column,
        ));
        column += crate::view::tab_width::char_columns(tab_width, ch, column);
        display = display.saturating_add(whitespace_marker_len(ch));

        if let Some(slot) = display_to_source.get_mut(display_start) {
            *slot = source_start;
        }
        for slot in display_to_source
            .iter_mut()
            .take(display.saturating_add(1))
            .skip(display_start.saturating_add(1))
        {
            *slot = source;
        }

        if let Some(slot) = source_to_display.get_mut(source_start) {
            *slot = display_start;
        }
        for slot in source_to_display
            .iter_mut()
            .take(source.saturating_add(1))
            .skip(source_start.saturating_add(1))
        {
            *slot = display;
        }
    }

    if append_synthetic_eol {
        let display_start = display;
        display = display.saturating_add('\u{21B5}'.len_utf8());
        if let Some(slot) = display_to_source.get_mut(display_start) {
            *slot = source;
        }
        for slot in display_to_source
            .iter_mut()
            .take(display.saturating_add(1))
            .skip(display_start.saturating_add(1))
        {
            *slot = source;
        }
    }

    DiffTextOffsetMap {
        display_to_source: Arc::from(display_to_source),
        source_to_display: Arc::from(source_to_display),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_diff_line_row_canvas(
    tab_width: usize,
    theme: AppTheme,
    view: Entity<DiffRenderer>,
    ui_scale_percent: u32,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    old: SharedString,
    new: SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
    raw_text: Option<DiffRowText<'_>>,
    reveal_whitespace_chars: bool,
    show_line_numbers: bool,
    wrap: Option<DiffTextWrapSlice>,
    annotation_width: Pixels,
    blame: Option<RowBlamePaint>,
    annot_hover: Option<(usize, AnnotArea)>,
    stage: Option<StageGutterSpec>,
    stage_hover: Option<DiffStageHover>,
) -> AnyElement {
    let paint_payload = diff_text_paint_payload(
        tab_width,
        styled,
        streamed_spec.as_ref(),
        raw_text,
        reveal_whitespace_chars,
        DiffTextRegion::Inline,
        wrap,
    );
    let revision = inline_row_canvas_revision_key(
        &old,
        &new,
        bg,
        fg,
        gutter_fg,
        paint_payload.text_hash,
        paint_payload.highlights_hash,
    );
    let row_hover = annot_hover.and_then(|(ix, area)| (ix == visible_ix).then_some(area));
    let revision = mix_blame_revision(revision, annotation_width, row_hover, blame.as_ref());
    let revision = mix_stage_gutter_revision(revision, &[stage], stage_hover, visible_ix);
    let text = paint_payload.text;
    let highlights = paint_payload.highlights;
    let highlights_hash = paint_payload.highlights_hash;
    let text_hash = paint_payload.text_hash;
    let offset_map = paint_payload.offset_map;
    let canvas_id = diff_row_canvas_id("diff_row_canvas_inline", visible_ix, revision);
    let test_row_bg = semantic_diff_row_bg(theme, bg);

    keyed_canvas(
        canvas_id,
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = if show_line_numbers {
                gutter_cell_total_width(
                    pad,
                    ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                )
            } else {
                px(0.0)
            };
            let content_bounds = inset_left(bounds, annotation_width);
            let text_bounds = inline_text_bounds(content_bounds, gutter_total, pad);
            // Everything but the annotation column, which owns its own clicks.
            let row_hitbox = window.insert_hitbox(content_bounds, HitboxBehavior::Normal);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            let annot_hitboxes =
                build_annot_hitboxes(window, bounds, annotation_width, ui_scale_percent);
            let stage_gutter = build_stage_gutter(
                window,
                stage,
                content_bounds.left(),
                bounds,
                ui_scale_percent,
            );

            InlineRowPrepaintState {
                bounds,
                pad,
                gutter_total,
                annot_w: annotation_width,
                text_bounds,
                row_hitbox,
                text_hitbox,
                annot_hitboxes,
                stage_gutter,
            }
        },
        move |bounds, prepaint, window, cx| {
            let gutter_style = diff_text_style(window);
            let line_metrics = line_metrics(window, theme);
            let when_metrics = line_metrics_annot_when(window, theme);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            // Selection must not tint the annotation sidebar: fill it with a
            // neutral panel color and fill the content area with the row bg.
            paint_row_bg_with_annotation(
                window,
                prepaint.bounds,
                prepaint.annot_w,
                bg,
                theme.colors.surface.panel,
            );

            if let Some(blame) = &blame {
                render_blame_column(
                    blame,
                    prepaint.bounds,
                    prepaint.annot_w,
                    y,
                    theme,
                    line_metrics,
                    when_metrics,
                    ui_scale_percent,
                    visible_ix,
                    prepaint.annot_hitboxes.as_ref(),
                    &view,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            if let Some(row) = view.read(cx).diff_focused_change_block_row(visible_ix) {
                let outline = Some(row.inline_outline());
                paint_focused_change_block_marks(
                    window,
                    prepaint.bounds,
                    prepaint.bounds.left() + prepaint.annot_w,
                    prepaint.bounds.right(),
                    row,
                    outline,
                    theme,
                    ui_scale_percent,
                );
                #[cfg(test)]
                record_focused_change_block_for_tests(
                    visible_ix,
                    DiffTextRegion::Inline,
                    row,
                    outline,
                );
            }

            if show_line_numbers {
                paint_gutter_text_right_aligned(
                    &old,
                    prepaint.bounds.left() + prepaint.annot_w + prepaint.gutter_total
                        - prepaint.pad,
                    y,
                    gutter_fg,
                    line_metrics,
                    &gutter_style,
                    window,
                    cx,
                );
                paint_gutter_text_right_aligned(
                    &new,
                    prepaint.bounds.left() + prepaint.annot_w + prepaint.gutter_total * 2.0
                        - prepaint.pad,
                    y,
                    gutter_fg,
                    line_metrics,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    tab_width,
                    &view,
                    visible_ix,
                    DiffTextRegion::Inline,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    streamed_spec.as_ref(),
                    test_row_bg,
                    highlights_hash,
                    text_hash,
                    offset_map.as_ref(),
                    reveal_whitespace_chars,
                    y,
                    fg,
                    line_metrics,
                    ui_scale_percent,
                    show_line_numbers,
                    wrap,
                    theme,
                    window,
                    cx,
                );
            });

            let stage_buttons = paint_stage_gutter(
                prepaint.stage_gutter.as_ref(),
                visible_ix,
                theme,
                bg,
                ui_scale_percent,
                &prepaint.row_hitbox,
                None,
                &view,
                window,
                cx,
            )
            .into_iter()
            .collect();

            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_hitbox: prepaint.row_hitbox.clone(),
                    regions: DiffRowTextRegions::single(
                        DiffTextRegion::Inline,
                        visible_text_bounds,
                    ),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                    stage: stage_buttons,
                },
                cx,
            );

            if selected {
                window.paint_quad(gpui::outline(
                    inset_left(bounds, prepaint.annot_w),
                    focused_row_outline_color(theme, bg),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(theme.editor_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .bg(bg)
    .text_size(theme.editor_font_size(ui_scale_percent))
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn split_diff_line_row_canvas(
    tab_width: usize,
    theme: AppTheme,
    view: Entity<DiffRenderer>,
    ui_scale_percent: u32,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    old: SharedString,
    new: SharedString,
    left_bg: gpui::Rgba,
    left_fg: gpui::Rgba,
    left_gutter: gpui::Rgba,
    right_bg: gpui::Rgba,
    right_fg: gpui::Rgba,
    right_gutter: gpui::Rgba,
    left_styled: Option<&CachedDiffStyledText>,
    right_styled: Option<&CachedDiffStyledText>,
    left_streamed_spec: Option<StreamedDiffTextPaintSpec>,
    right_streamed_spec: Option<StreamedDiffTextPaintSpec>,
    left_raw_text: Option<DiffRowText<'_>>,
    right_raw_text: Option<DiffRowText<'_>>,
    reveal_whitespace_chars: bool,
    show_line_numbers: bool,
    wrap: Option<DiffTextWrapSlice>,
    annotation_width: Pixels,
    blame: Option<RowBlamePaint>,
    annot_hover: Option<(usize, AnnotArea)>,
    stage_left: Option<StageGutterSpec>,
    stage_right: Option<StageGutterSpec>,
    stage_hover: Option<DiffStageHover>,
) -> AnyElement {
    let left_payload = diff_text_paint_payload(
        tab_width,
        left_styled,
        left_streamed_spec.as_ref(),
        left_raw_text,
        reveal_whitespace_chars,
        DiffTextRegion::SplitLeft,
        wrap,
    );
    let right_payload = diff_text_paint_payload(
        tab_width,
        right_styled,
        right_streamed_spec.as_ref(),
        right_raw_text,
        reveal_whitespace_chars,
        DiffTextRegion::SplitRight,
        wrap,
    );
    let revision = split_row_canvas_revision_key(
        &old,
        &new,
        left_bg,
        left_fg,
        left_gutter,
        right_bg,
        right_fg,
        right_gutter,
        left_payload.text_hash,
        left_payload.highlights_hash,
        right_payload.text_hash,
        right_payload.highlights_hash,
    );
    let row_hover = annot_hover.and_then(|(ix, area)| (ix == visible_ix).then_some(area));
    let revision = mix_blame_revision(revision, annotation_width, row_hover, blame.as_ref());
    let revision = mix_stage_gutter_revision(
        revision,
        &[stage_left, stage_right],
        stage_hover,
        visible_ix,
    );
    let left_text = left_payload.text;
    let left_highlights = left_payload.highlights;
    let left_highlights_hash = left_payload.highlights_hash;
    let left_text_hash = left_payload.text_hash;
    let left_offset_map = left_payload.offset_map;
    let right_text = right_payload.text;
    let right_highlights = right_payload.highlights;
    let right_highlights_hash = right_payload.highlights_hash;
    let right_text_hash = right_payload.text_hash;
    let right_offset_map = right_payload.offset_map;
    let canvas_id = diff_row_canvas_id("diff_row_canvas_split", visible_ix, revision);
    let left_test_row_bg = semantic_diff_row_bg(theme, left_bg);
    let right_test_row_bg = semantic_diff_row_bg(theme, right_bg);

    keyed_canvas(
        canvas_id,
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = if show_line_numbers {
                gutter_cell_total_width(
                    pad,
                    ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                )
            } else {
                px(0.0)
            };
            let content_bounds = inset_left(bounds, annotation_width);
            let (left_col, sep_bounds, right_col) = split_columns(content_bounds);
            let left_text_bounds = column_text_bounds(left_col, gutter_total, pad);
            let right_text_bounds = column_text_bounds(right_col, gutter_total, pad);

            // Everything but the annotation column, which owns its own clicks.
            let row_hitbox = window.insert_hitbox(content_bounds, HitboxBehavior::Normal);
            let left_hitbox = window.insert_hitbox(left_text_bounds, HitboxBehavior::Normal);
            let right_hitbox = window.insert_hitbox(right_text_bounds, HitboxBehavior::Normal);
            let annot_hitboxes =
                build_annot_hitboxes(window, bounds, annotation_width, ui_scale_percent);
            let left_stage_gutter = build_stage_gutter(
                window,
                stage_left,
                left_col.left(),
                bounds,
                ui_scale_percent,
            );
            let right_stage_gutter = build_stage_gutter(
                window,
                stage_right,
                right_col.left(),
                bounds,
                ui_scale_percent,
            );

            SplitRowPrepaintState {
                bounds,
                pad,
                annot_w: annotation_width,
                left_col,
                sep_bounds,
                right_col,
                left_text_bounds,
                right_text_bounds,
                row_hitbox,
                left_hitbox,
                right_hitbox,
                annot_hitboxes,
                left_stage_gutter,
                right_stage_gutter,
            }
        },
        move |bounds, prepaint, window, cx| {
            let gutter_style = diff_text_style(window);
            let line_metrics = line_metrics(window, theme);
            let when_metrics = line_metrics_annot_when(window, theme);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.left_hitbox);
            window.set_cursor_style(CursorStyle::IBeam, &prepaint.right_hitbox);

            // Neutral panel bg under the annotation column so selection does
            // not tint it.
            if prepaint.annot_w > px(0.0) {
                paint_annotation_sidebar(
                    window,
                    prepaint.bounds,
                    prepaint.annot_w,
                    theme.colors.surface.panel,
                );
            }
            window.paint_quad(fill(row_bg_fill_bounds(prepaint.left_col), left_bg));
            window.paint_quad(fill(
                row_bg_fill_bounds(prepaint.sep_bounds),
                theme.colors.stroke.default,
            ));
            window.paint_quad(fill(row_bg_fill_bounds(prepaint.right_col), right_bg));

            if let Some(blame) = &blame {
                render_blame_column(
                    blame,
                    prepaint.bounds,
                    prepaint.annot_w,
                    y,
                    theme,
                    line_metrics,
                    when_metrics,
                    ui_scale_percent,
                    visible_ix,
                    prepaint.annot_hitboxes.as_ref(),
                    &view,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            if let Some(row) = view.read(cx).diff_focused_change_block_row(visible_ix) {
                for (column, old_side) in [(prepaint.left_col, true), (prepaint.right_col, false)] {
                    let outline = row.column_outline(old_side);
                    paint_focused_change_block_marks(
                        window,
                        prepaint.bounds,
                        column.left(),
                        column.right(),
                        row,
                        outline,
                        theme,
                        ui_scale_percent,
                    );
                    #[cfg(test)]
                    record_focused_change_block_for_tests(
                        visible_ix,
                        if old_side {
                            DiffTextRegion::SplitLeft
                        } else {
                            DiffTextRegion::SplitRight
                        },
                        row,
                        outline,
                    );
                }
            }

            if show_line_numbers {
                let gutter_total = gutter_cell_total_width(
                    prepaint.pad,
                    ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                );
                paint_gutter_text_right_aligned(
                    &old,
                    prepaint.left_col.left() + gutter_total - prepaint.pad,
                    y,
                    left_gutter,
                    line_metrics,
                    &gutter_style,
                    window,
                    cx,
                );
                paint_gutter_text_right_aligned(
                    &new,
                    prepaint.right_col.left() + gutter_total - prepaint.pad,
                    y,
                    right_gutter,
                    line_metrics,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            window.paint_layer(prepaint.left_text_bounds, |window| {
                paint_selectable_diff_text(
                    tab_width,
                    &view,
                    visible_ix,
                    DiffTextRegion::SplitLeft,
                    prepaint.left_text_bounds,
                    &left_text,
                    &left_highlights,
                    left_streamed_spec.as_ref(),
                    left_test_row_bg,
                    left_highlights_hash,
                    left_text_hash,
                    left_offset_map.as_ref(),
                    reveal_whitespace_chars,
                    y,
                    left_fg,
                    line_metrics,
                    ui_scale_percent,
                    show_line_numbers,
                    wrap,
                    theme,
                    window,
                    cx,
                );
            });

            window.paint_layer(prepaint.right_text_bounds, |window| {
                paint_selectable_diff_text(
                    tab_width,
                    &view,
                    visible_ix,
                    DiffTextRegion::SplitRight,
                    prepaint.right_text_bounds,
                    &right_text,
                    &right_highlights,
                    right_streamed_spec.as_ref(),
                    right_test_row_bg,
                    right_highlights_hash,
                    right_text_hash,
                    right_offset_map.as_ref(),
                    reveal_whitespace_chars,
                    y,
                    right_fg,
                    line_metrics,
                    ui_scale_percent,
                    show_line_numbers,
                    wrap,
                    theme,
                    window,
                    cx,
                );
            });

            let stage_buttons = paint_stage_gutter(
                prepaint.left_stage_gutter.as_ref(),
                visible_ix,
                theme,
                left_bg,
                ui_scale_percent,
                &prepaint.row_hitbox,
                Some(prepaint.left_col),
                &view,
                window,
                cx,
            )
            .into_iter()
            .chain(paint_stage_gutter(
                prepaint.right_stage_gutter.as_ref(),
                visible_ix,
                theme,
                right_bg,
                ui_scale_percent,
                &prepaint.row_hitbox,
                Some(prepaint.right_col),
                &view,
                window,
                cx,
            ))
            .collect();

            let left_text_bounds = prepaint.left_text_bounds;
            let right_text_bounds = prepaint.right_text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_left_text_bounds = left_text_bounds.intersect(&clip_bounds);
            let visible_right_text_bounds = right_text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_hitbox: prepaint.row_hitbox.clone(),
                    regions: DiffRowTextRegions::split(
                        visible_left_text_bounds,
                        visible_right_text_bounds,
                    ),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                    stage: stage_buttons,
                },
                cx,
            );

            if selected {
                window.paint_quad(gpui::outline(
                    inset_left(bounds, prepaint.annot_w),
                    focused_row_outline_color(theme, left_bg),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(theme.editor_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_size(theme.editor_font_size(ui_scale_percent))
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn patch_split_column_row_canvas(
    tab_width: usize,
    theme: AppTheme,
    view: Entity<DiffRenderer>,
    ui_scale_percent: u32,
    column: super::diff::PatchSplitColumn,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    line_no: SharedString,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
    raw_text: Option<DiffRowText<'_>>,
    reveal_whitespace_chars: bool,
    show_line_numbers: bool,
    wrap: Option<DiffTextWrapSlice>,
    annotation_width: Pixels,
    blame: Option<RowBlamePaint>,
    annot_hover: Option<(usize, AnnotArea)>,
    stage: Option<StageGutterSpec>,
    stage_hover: Option<DiffStageHover>,
) -> AnyElement {
    let region = match column {
        super::diff::PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        super::diff::PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };
    let paint_payload = diff_text_paint_payload(
        tab_width,
        styled,
        streamed_spec.as_ref(),
        raw_text,
        reveal_whitespace_chars,
        region,
        wrap,
    );
    let text = paint_payload.text;
    let highlights = paint_payload.highlights;
    let highlights_hash = paint_payload.highlights_hash;
    let text_hash = paint_payload.text_hash;
    let offset_map = paint_payload.offset_map;
    let revision = patch_split_row_canvas_revision_key(
        &line_no,
        bg,
        fg,
        gutter_fg,
        text_hash,
        highlights_hash,
    );
    let row_hover = annot_hover.and_then(|(ix, area)| (ix == visible_ix).then_some(area));
    let revision = mix_blame_revision(revision, annotation_width, row_hover, blame.as_ref());
    let revision = mix_stage_gutter_revision(revision, &[stage], stage_hover, visible_ix);
    let canvas_id = diff_row_canvas_id(
        match column {
            super::diff::PatchSplitColumn::Left => "diff_row_canvas_file_split_left",
            super::diff::PatchSplitColumn::Right => "diff_row_canvas_file_split_right",
        },
        visible_ix,
        revision,
    );
    let test_row_bg = semantic_diff_row_bg(theme, bg);

    keyed_canvas(
        canvas_id,
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = if show_line_numbers {
                gutter_cell_total_width(
                    pad,
                    ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                )
            } else {
                px(0.0)
            };
            let content_bounds = inset_left(bounds, annotation_width);
            let text_bounds = single_column_text_bounds(content_bounds, gutter_total, pad);
            // Everything but the annotation column, which owns its own clicks.
            let row_hitbox = window.insert_hitbox(content_bounds, HitboxBehavior::Normal);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            let annot_hitboxes =
                build_annot_hitboxes(window, bounds, annotation_width, ui_scale_percent);
            let stage_gutter = build_stage_gutter(
                window,
                stage,
                content_bounds.left(),
                bounds,
                ui_scale_percent,
            );
            SingleColumnRowPrepaintState {
                bounds,
                pad,
                annot_w: annotation_width,
                text_bounds,
                row_hitbox,
                text_hitbox,
                annot_hitboxes,
                stage_gutter,
            }
        },
        move |bounds, prepaint, window, cx| {
            let gutter_style = diff_text_style(window);
            let line_metrics = line_metrics(window, theme);
            let when_metrics = line_metrics_annot_when(window, theme);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            // Selection must not tint the annotation sidebar: fill it with a
            // neutral panel color and fill the content area with the row bg.
            paint_row_bg_with_annotation(
                window,
                prepaint.bounds,
                prepaint.annot_w,
                bg,
                theme.colors.surface.panel,
            );

            if let Some(blame) = &blame {
                render_blame_column(
                    blame,
                    prepaint.bounds,
                    prepaint.annot_w,
                    y,
                    theme,
                    line_metrics,
                    when_metrics,
                    ui_scale_percent,
                    visible_ix,
                    prepaint.annot_hitboxes.as_ref(),
                    &view,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            if let Some(row) = view.read(cx).diff_focused_change_block_row(visible_ix) {
                let outline = row.column_outline(region == DiffTextRegion::SplitLeft);
                paint_focused_change_block_marks(
                    window,
                    prepaint.bounds,
                    prepaint.bounds.left() + prepaint.annot_w,
                    prepaint.bounds.right(),
                    row,
                    outline,
                    theme,
                    ui_scale_percent,
                );
                #[cfg(test)]
                record_focused_change_block_for_tests(visible_ix, region, row, outline);
            }

            if show_line_numbers {
                let gutter_total = gutter_cell_total_width(
                    prepaint.pad,
                    ui_scale::UiScale::from_percent(ui_scale_percent)
                        .with_appearance(theme.metrics),
                );
                paint_gutter_text_right_aligned(
                    &line_no,
                    prepaint.bounds.left() + prepaint.annot_w + gutter_total - prepaint.pad,
                    y,
                    gutter_fg,
                    line_metrics,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    tab_width,
                    &view,
                    visible_ix,
                    region,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    streamed_spec.as_ref(),
                    test_row_bg,
                    highlights_hash,
                    text_hash,
                    offset_map.as_ref(),
                    reveal_whitespace_chars,
                    y,
                    fg,
                    line_metrics,
                    ui_scale_percent,
                    show_line_numbers,
                    wrap,
                    theme,
                    window,
                    cx,
                );
            });

            let stage_buttons = paint_stage_gutter(
                prepaint.stage_gutter.as_ref(),
                visible_ix,
                theme,
                bg,
                ui_scale_percent,
                &prepaint.row_hitbox,
                None,
                &view,
                window,
                cx,
            )
            .into_iter()
            .collect();

            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_hitbox: prepaint.row_hitbox.clone(),
                    regions: DiffRowTextRegions::single(region, visible_text_bounds),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                    stage: stage_buttons,
                },
                cx,
            );

            if selected {
                window.paint_quad(gpui::outline(
                    inset_left(bounds, prepaint.annot_w),
                    focused_row_outline_color(theme, bg),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(theme.editor_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_size(theme.editor_font_size(ui_scale_percent))
    .whitespace_nowrap()
    .into_any_element()
}

/// The blame column for one row of the file editor's gutter.
///
/// The editor's gutter is otherwise plain divs, but blame is not a label — it
/// has a hover highlight, a tooltip carrying the commit body, and three click
/// targets (the message opens commit details, the two icons walk to the file at
/// that commit and at its parent). All of that lives in [`render_blame_column`],
/// so the gutter hands it a canvas sized to exactly the annotation column and
/// gets the same behaviour the diff and preview have.
///
/// A row whose `blame.show_text` is false paints only its recency bar and
/// installs no handlers — which is how the editor represents blame it cannot
/// stand behind, on the interior lines of a same-commit run and over a buffer
/// with unsaved edits.
pub(in crate::view) fn blame_gutter_row_canvas(
    theme: AppTheme,
    view: Entity<DiffRenderer>,
    ui_scale_percent: u32,
    visual_ix: usize,
    row_height: Pixels,
    annotation_width: Pixels,
    annot_hover: Option<(usize, AnnotArea)>,
    blame: Option<RowBlamePaint>,
) -> AnyElement {
    // Same revision key the diff rows build, for the same reason: GPUI keys
    // element state by id, so a canvas identified by its row index alone keeps
    // last frame's hitboxes and hover after the blame under it changes.
    let row_hover = annot_hover.and_then(|(ix, area)| (ix == visual_ix).then_some(area));
    let revision = mix_blame_revision(0, annotation_width, row_hover, blame.as_ref());
    keyed_canvas(
        diff_row_canvas_id("file_editor_blame_row_canvas", visual_ix, revision),
        move |bounds, window, _cx| {
            build_annot_hitboxes(window, bounds, annotation_width, ui_scale_percent)
        },
        move |bounds, annot_hitboxes, window, cx| {
            let gutter_style = diff_text_style(window);
            let line_metrics = line_metrics(window, theme);
            let y = center_text_y(bounds, line_metrics.line_height);

            // The whole canvas *is* the annotation column, so it takes the
            // sidebar colour the diff and preview rows give theirs — not the
            // editor canvas, which left the strip with no edge at all.
            window.paint_quad(fill(bounds, theme.colors.surface.panel));

            if let Some(blame) = &blame {
                let when_metrics = line_metrics_annot_when(window, theme);
                render_blame_column(
                    blame,
                    bounds,
                    annotation_width,
                    y,
                    theme,
                    line_metrics,
                    when_metrics,
                    ui_scale_percent,
                    visual_ix,
                    annot_hitboxes.as_ref(),
                    &view,
                    &gutter_style,
                    window,
                    cx,
                );
            }
        },
    )
    .w(annotation_width)
    .h(row_height)
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn worktree_preview_row_canvas(
    tab_width: usize,
    theme: AppTheme,
    view: Entity<DiffRenderer>,
    ui_scale_percent: u32,
    ix: usize,
    min_width: Pixels,
    annotation_width: Pixels,
    blame: Option<RowBlamePaint>,
    bar_color: Option<gpui::Rgba>,
    line_no: SharedString,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
    raw_text: Option<DiffRowText<'_>>,
    reveal_whitespace_chars: bool,
    wrap: Option<DiffTextWrapSlice>,
) -> AnyElement {
    let paint_payload = diff_text_paint_payload(
        tab_width,
        styled,
        streamed_spec.as_ref(),
        raw_text,
        reveal_whitespace_chars,
        DiffTextRegion::Inline,
        wrap,
    );
    let text = paint_payload.text;
    let highlights = paint_payload.highlights;
    let highlights_hash = paint_payload.highlights_hash;
    let text_hash = paint_payload.text_hash;
    let offset_map = paint_payload.offset_map;

    keyed_canvas(
        ("worktree_preview_row_canvas", ix),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(
                pad,
                ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
            );
            let bar_w = if bar_color.is_some() {
                diff_scaled_px(DIFF_CHANGE_BAR_WIDTH_PX, ui_scale_percent)
            } else {
                px(0.0)
            };
            // Inline annotate reserves a fixed column at the left edge of the row;
            // the content (change bar, gutter, text) is inset past it.
            let content = inset_left(bounds, annotation_width);
            let inner = Bounds::new(
                point(content.left() + bar_w, content.top()),
                size(
                    (content.size.width - bar_w).max(px(0.0)),
                    content.size.height,
                ),
            );
            let text_bounds = single_column_text_bounds(inner, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            let annot_hitboxes =
                build_annot_hitboxes(window, bounds, annotation_width, ui_scale_percent);
            WorktreePreviewRowPrepaintState {
                inner,
                pad,
                bar_w,
                text_bounds,
                text_hitbox,
                annot_w: annotation_width,
                annot_hitboxes,
            }
        },
        move |bounds, prepaint, window, cx| {
            let gutter_style = diff_text_style(window);
            let line_metrics = line_metrics(window, theme);
            let y = center_text_y(bounds, line_metrics.line_height);

            // Reserve the annotation sidebar with the neutral panel color (matching
            // the diff renderers) so the blame column background is consistent with
            // the diff view; fill the content area with the row background.
            paint_row_bg_with_annotation(
                window,
                bounds,
                prepaint.annot_w,
                theme.colors.surface.canvas,
                theme.colors.surface.panel,
            );
            if let Some(color) = bar_color
                && prepaint.bar_w > px(0.0)
            {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left() + prepaint.annot_w, bounds.top()),
                        size(prepaint.bar_w, bounds.size.height),
                    ),
                    color,
                ));
            }

            if let Some(blame) = &blame {
                let when_metrics = line_metrics_annot_when(window, theme);
                render_blame_column(
                    blame,
                    bounds,
                    prepaint.annot_w,
                    y,
                    theme,
                    line_metrics,
                    when_metrics,
                    ui_scale_percent,
                    ix,
                    prepaint.annot_hitboxes.as_ref(),
                    &view,
                    &gutter_style,
                    window,
                    cx,
                );
            }

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            paint_gutter_text_right_aligned(
                &line_no,
                prepaint.inner.left()
                    + gutter_cell_total_width(
                        prepaint.pad,
                        ui_scale::UiScale::from_percent(ui_scale_percent)
                            .with_appearance(theme.metrics),
                    )
                    - prepaint.pad,
                y,
                theme.colors.foreground.secondary,
                line_metrics,
                &gutter_style,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    tab_width,
                    &view,
                    ix,
                    DiffTextRegion::Inline,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    streamed_spec.as_ref(),
                    None,
                    highlights_hash,
                    text_hash,
                    offset_map.as_ref(),
                    reveal_whitespace_chars,
                    y,
                    theme.colors.foreground.primary,
                    line_metrics,
                    ui_scale_percent,
                    true,
                    None,
                    theme,
                    window,
                    cx,
                );
            });

            window.on_mouse_event({
                let view = view.clone();
                // The hitbox covers the same text area, but consulting it rather
                // than the bounds keeps clicks on anything painted over the
                // preview (a floating popover, a menu) from reaching this row.
                let text_hitbox = prepaint.text_hitbox.clone();
                move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || !text_hitbox.is_hovered(window) {
                        return;
                    }

                    if event.button == gpui::MouseButton::Left {
                        let focus = view.read(cx).diff_panel_focus_handle.clone();
                        window.focus(&focus, cx);
                        let click_count = event.click_count;
                        let position = event.position;
                        view.update(cx, |this, cx| {
                            this.handle_diff_text_mouse_down(
                                ix,
                                DiffTextRegion::Inline,
                                position,
                                click_count,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            });
            let target = diff_canvas_click_target(&view, ix, "preview-context", cx);
            let view = view.clone();
            crate::kit::click::on_canvas_click(
                window,
                target,
                &prepaint.text_hitbox,
                gpui::MouseButton::Right,
                true,
                move |event, window, cx| {
                    view.update(cx, |this, cx| {
                        this.open_diff_editor_context_menu(
                            ix,
                            DiffTextRegion::Inline,
                            event.position(),
                            window,
                            cx,
                        );
                        cx.notify();
                    })
                },
            );
        },
    )
    .h(theme.editor_row_height(ui_scale_percent))
    .min_w(min_width + annotation_width)
    .w_full()
    .text_size(theme.editor_font_size(ui_scale_percent))
    .whitespace_nowrap()
    .into_any_element()
}

#[derive(Clone, Debug)]
struct InlineRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    gutter_total: Pixels,
    annot_w: Pixels,
    text_bounds: Bounds<Pixels>,
    row_hitbox: Hitbox,
    text_hitbox: Hitbox,
    annot_hitboxes: Option<AnnotHitboxes>,
    stage_gutter: Option<StageGutterPrepaint>,
}

#[derive(Clone, Debug)]
struct SplitRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    annot_w: Pixels,
    left_col: Bounds<Pixels>,
    sep_bounds: Bounds<Pixels>,
    right_col: Bounds<Pixels>,
    left_text_bounds: Bounds<Pixels>,
    right_text_bounds: Bounds<Pixels>,
    row_hitbox: Hitbox,
    left_hitbox: Hitbox,
    right_hitbox: Hitbox,
    annot_hitboxes: Option<AnnotHitboxes>,
    left_stage_gutter: Option<StageGutterPrepaint>,
    right_stage_gutter: Option<StageGutterPrepaint>,
}

#[derive(Clone, Debug)]
struct SingleColumnRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    annot_w: Pixels,
    text_bounds: Bounds<Pixels>,
    row_hitbox: Hitbox,
    text_hitbox: Hitbox,
    annot_hitboxes: Option<AnnotHitboxes>,
    stage_gutter: Option<StageGutterPrepaint>,
}

#[derive(Clone, Debug)]
struct WorktreePreviewRowPrepaintState {
    inner: Bounds<Pixels>,
    pad: Pixels,
    bar_w: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
    annot_w: Pixels,
    annot_hitboxes: Option<AnnotHitboxes>,
}

/// Width of one wrapped diff-text column, measured in `editor_font_family`.
///
/// The family must be passed in rather than taken from the ambient text style:
/// wrap columns are computed while the diff pane builds its element tree, which
/// is before the rows container pushes `.font_family(editor_font)` onto the
/// window text style stack. `window.text_style()` still resolves to the
/// proportional UI font at that point, and measuring the `W` sample there
/// overestimates the column width by ~1.5x (IBM Plex Sans `W` is 0.891em vs
/// Lilex 0.600em), so every line wrapped at about two thirds of the width it
/// actually had.
pub(in crate::view) fn diff_text_wrap_char_width(
    window: &mut Window,
    editor_font_family: impl Into<gpui::SharedString>,
    editor_font_size_px: u32,
) -> Pixels {
    let mut style = diff_text_style(window);
    style.font_family = editor_font_family.into();
    let font_size = crate::ui_scale::design_px_from_window(editor_font_size_px as f32, window);
    let run = style.to_run(DIFF_TEXT_WRAP_WIDTH_SAMPLE.len());
    let layout =
        window
            .text_system()
            .shape_line(DIFF_TEXT_WRAP_WIDTH_SAMPLE.into(), font_size, &[run]);
    if DIFF_TEXT_WRAP_WIDTH_SAMPLE.is_empty() {
        px(1.0)
    } else {
        (layout.width / DIFF_TEXT_WRAP_WIDTH_SAMPLE.len() as f32).max(px(1.0))
    }
}

fn px_2(window: &Window) -> Pixels {
    crate::ui_scale::design_px_from_window(DIFF_ROW_HORIZONTAL_PADDING_PX, window)
}

pub(in crate::view) fn diff_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

pub(in crate::view) fn diff_row_horizontal_padding(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_ROW_HORIZONTAL_PADDING_PX, ui_scale_percent)
}

pub(super) fn diff_gutter_total_width(scale: impl Into<ui_scale::UiScale>) -> Pixels {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    gutter_cell_total_width(diff_row_horizontal_padding(ui_scale_percent), scale)
}

/// Width of the bar marking a wholly added or removed file, which the row's
/// content is inset past.
pub(in crate::view) fn diff_change_bar_width(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_CHANGE_BAR_WIDTH_PX, ui_scale_percent)
}

pub(in crate::view) fn diff_single_column_text_start(
    scale: impl Into<ui_scale::UiScale>,
) -> Pixels {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    diff_gutter_total_width(scale) + diff_row_horizontal_padding(ui_scale_percent)
}

pub(in crate::view) fn diff_inline_text_start(scale: impl Into<ui_scale::UiScale>) -> Pixels {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    diff_gutter_total_width(scale) * 2.0 + diff_row_horizontal_padding(ui_scale_percent)
}

fn gutter_cell_total_width(pad: Pixels, scale: impl Into<ui_scale::UiScale>) -> Pixels {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    diff_scaled_px(
        DIFF_GUTTER_BASE_WIDTH_PX * scale.appearance.editor_font_size_px as f32 / 13.0,
        ui_scale_percent,
    ) + pad * 2.0
}

fn inline_text_bounds(bounds: Bounds<Pixels>, gutter_total: Pixels, pad: Pixels) -> Bounds<Pixels> {
    let left = bounds.left() + gutter_total * 2.0 + pad;
    let width = (bounds.size.width - gutter_total * 2.0 - pad * 2.0).max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

/// Shrink `bounds` from the left by `dx`, reserving that space (e.g. for the
/// annotation column). Width is clamped to zero.
fn inset_left(bounds: Bounds<Pixels>, dx: Pixels) -> Bounds<Pixels> {
    Bounds::new(
        point(bounds.left() + dx, bounds.top()),
        size((bounds.size.width - dx).max(px(0.0)), bounds.size.height),
    )
}

fn single_column_text_bounds(
    bounds: Bounds<Pixels>,
    gutter_total: Pixels,
    pad: Pixels,
) -> Bounds<Pixels> {
    let left = bounds.left() + gutter_total + pad;
    let width = (bounds.size.width - gutter_total - pad * 2.0).max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

fn split_columns(bounds: Bounds<Pixels>) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    let sep = px(1.0);
    let total_w = bounds.size.width.max(px(0.0));
    let inner_w = (total_w - sep).max(px(0.0));
    let left_w = (inner_w * 0.5).floor();
    let right_w = (inner_w - left_w).max(px(0.0));
    let left = Bounds::new(bounds.origin, size(left_w, bounds.size.height));
    let sep_bounds = Bounds::new(
        point(bounds.left() + left_w, bounds.top()),
        size(sep, bounds.size.height),
    );
    let right = Bounds::new(
        point(bounds.left() + left_w + sep, bounds.top()),
        size(right_w, bounds.size.height),
    );
    (left, sep_bounds, right)
}

fn column_text_bounds(col: Bounds<Pixels>, gutter_total: Pixels, pad: Pixels) -> Bounds<Pixels> {
    single_column_text_bounds(col, gutter_total, pad)
}

/// Paints `text` with its right edge at `right`; used for line numbers so the
/// digits hug the content edge of their gutter cell.
#[allow(clippy::too_many_arguments)]
fn paint_gutter_text_right_aligned(
    text: &SharedString,
    right: Pixels,
    y: Pixels,
    color: gpui::Rgba,
    metrics: LineMetrics,
    style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let shaped = shaped_gutter_line(text, color, metrics, style, window);
    let _ = shaped.paint(
        point(right - shaped.width, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_gutter_text(
    text: &SharedString,
    x: Pixels,
    y: Pixels,
    color: gpui::Rgba,
    metrics: LineMetrics,
    style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let shaped = shaped_gutter_line(text, color, metrics, style, window);
    let _ = shaped.paint(
        point(x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn shaped_gutter_line(
    text: &SharedString,
    color: gpui::Rgba,
    metrics: LineMetrics,
    style: &TextStyle,
    window: &mut Window,
) -> gpui::ShapedLine {
    GUTTER_TEXT_LAYOUT_CACHE
        .with(|cache| canvas_text::shaped_gutter_line(text, color, metrics, style, cache, window))
}

#[allow(clippy::too_many_arguments)]
fn paint_selectable_diff_text(
    tab_width: usize,
    view: &Entity<DiffRenderer>,
    visible_ix: usize,
    region: DiffTextRegion,
    bounds: Bounds<Pixels>,
    text: &SharedString,
    highlights: &Arc<[(Range<usize>, HighlightStyle)]>,
    streamed_spec: Option<&StreamedDiffTextPaintSpec>,
    row_bg: Option<gpui::Rgba>,
    highlights_hash: u64,
    text_hash: u64,
    offset_map: Option<&DiffTextOffsetMap>,
    reveal_whitespace_chars: bool,
    y: Pixels,
    base_fg: gpui::Rgba,
    metrics: LineMetrics,
    ui_scale_percent: u32,
    show_line_numbers: bool,
    wrap: Option<DiffTextWrapSlice>,
    theme: AppTheme,
    window: &mut Window,
    cx: &mut App,
) {
    let mut base_style = diff_text_style(window);
    base_style.color = base_fg.into_color();
    base_style.white_space = gpui::WhiteSpace::Nowrap;
    base_style.text_overflow = None;

    let pad = px_2(window);
    let gutter_total = gutter_cell_total_width(
        pad,
        ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
    );
    let row_extra = match region {
        DiffTextRegion::Inline if show_line_numbers => gutter_total * 2.0 + pad * 2.0,
        DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight if show_line_numbers => {
            gutter_total + pad * 2.0
        }
        _ => pad * 2.0,
    };
    let total_text_len = streamed_spec
        .filter(|spec| should_stream_diff_text(Some(spec)))
        .map(|spec| spec.raw_text.len())
        .unwrap_or_else(|| text.len());
    let source_text_len = offset_map
        .map(DiffTextOffsetMap::source_len)
        .unwrap_or(total_text_len);
    let (source_visible_ix, visual_text_range) = view
        .read(cx)
        .diff_text_visual_source_range_for_region(visible_ix, region);
    // Resolved once and shared by all three row queries below: the selection,
    // the delimiter pair and the name's other uses each need it, and its
    // non-wrapped arm re-fetches the row model to measure the row.
    let resolved_row = (source_visible_ix, visual_text_range.clone());
    let selection = view
        .read(cx)
        .diff_text_local_selection_range_in(region, resolved_row.clone());

    let mut streamed_styled = None;
    let mut streamed_slice_range = None;
    let mut streamed_slice_is_wrap = false;
    let mut paint_x = bounds.left();
    let mut hitbox_cell_width = None;
    let mut pending_prepared_syntax = false;

    let (layout_key, layout, shaped_new, required_row_w) = if let Some(spec) =
        streamed_spec.filter(|spec| should_stream_diff_text(Some(spec)))
    {
        let cell_width =
            streamed_diff_text_ascii_cell_width(&base_style, metrics.font_size, window);
        let clip_bounds = window.content_mask().bounds;
        let overscan_columns = STREAMED_DIFF_TEXT_OVERSCAN_COLUMNS.max(spec.query.as_ref().len());
        let wrap_range = wrap.map(|wrap| wrap.range_for_region(region));
        streamed_slice_is_wrap = wrap_range.is_some();
        let slice_range = wrap_range.clone().unwrap_or_else(|| {
            streamed_diff_text_visible_slice_range(
                bounds,
                clip_bounds,
                spec.raw_text.len(),
                cell_width,
                overscan_columns,
            )
        });
        let (mut slice_styled, pending, resolved_slice_range) =
            build_streamed_diff_slice_styled_text(tab_width, theme, spec, &slice_range);
        if reveal_whitespace_chars {
            let append_eol_marker = resolved_slice_range.end >= spec.raw_text.len();
            slice_styled = whitespace_visible_styled_text(&slice_styled, append_eol_marker);
        }
        let (layout_key, layout, shaped_new) = ensure_layout_cached(
            view,
            slice_styled.text_hash,
            &slice_styled.text,
            &base_style,
            base_fg,
            slice_styled.highlights.as_ref(),
            slice_styled.highlights_hash,
            metrics,
            window,
            cx,
        );
        paint_x = if wrap_range.is_some() {
            bounds.left()
        } else {
            bounds.left() + cell_width * resolved_slice_range.start as f32
        };
        hitbox_cell_width = Some(cell_width);
        pending_prepared_syntax = pending;
        streamed_slice_range = Some(resolved_slice_range);
        let total_text_cells = spec
            .raw_text
            .len()
            .saturating_add(usize::from(reveal_whitespace_chars));
        let required_row_w = (row_extra
            + cell_width * total_text_cells as f32
            + diff_scaled_px(DIFF_ROW_TEXT_TRAILING_PADDING_PX, ui_scale_percent))
        .round();
        streamed_styled = Some(slice_styled);
        (layout_key, layout, shaped_new, required_row_w)
    } else {
        let (layout_key, layout, shaped_new) = ensure_layout_cached(
            view,
            text_hash,
            text,
            &base_style,
            base_fg,
            highlights.as_ref(),
            highlights_hash,
            metrics,
            window,
            cx,
        );
        let required_row_w = (row_extra
            + layout.width
            + diff_scaled_px(DIFF_ROW_TEXT_TRAILING_PADDING_PX, ui_scale_percent))
        .round();
        (layout_key, layout, shaped_new, required_row_w)
    };

    let paint_text = streamed_styled
        .as_ref()
        .map(|styled| &styled.text)
        .unwrap_or(text);
    let paint_highlights = streamed_styled
        .as_ref()
        .map(|styled| styled.highlights.as_ref())
        .unwrap_or_else(|| highlights.as_ref());

    let pair_ranges = view
        .read(cx)
        .diff_text_local_pair_ranges_in(region, resolved_row.clone());
    let occurrence_ranges = view
        .read(cx)
        .diff_text_local_occurrence_ranges_in(region, resolved_row);

    #[cfg(test)]
    record_diff_paint_for_tests(
        visible_ix,
        region,
        paint_text,
        paint_highlights,
        row_bg,
        &pair_ranges,
        &occurrence_ranges,
    );
    #[cfg(not(test))]
    let _ = row_bg;

    // Shared by the selection and matching-pair quads: three coordinate regimes
    // (streamed monospace, tab-expanded via the offset map, and plain shaped
    // text) that both must agree on, so neither gets its own copy.
    let x_range_for_local = |r: &Range<usize>| -> (Pixels, Pixels) {
        if let Some(cell_width) = hitbox_cell_width {
            let (start, end) = if streamed_slice_is_wrap {
                (r.start.min(total_text_len), r.end.min(total_text_len))
            } else {
                let start = streamed_slice_range
                    .as_ref()
                    .map(|slice_range| r.start.max(slice_range.start))
                    .unwrap_or(r.start)
                    .min(total_text_len);
                let end = streamed_slice_range
                    .as_ref()
                    .map(|slice_range| r.end.min(slice_range.end))
                    .unwrap_or(r.end)
                    .min(total_text_len);
                (start, end)
            };
            (cell_width * start as f32, cell_width * end as f32)
        } else if let Some(offset_map) = offset_map {
            let start = offset_map.display_offset_for_source(r.start.min(source_text_len));
            let end = offset_map.display_offset_for_source(r.end.min(source_text_len));
            (layout.x_for_index(start), layout.x_for_index(end))
        } else {
            (
                layout.x_for_index(r.start.min(total_text_len)),
                layout.x_for_index(r.end.min(total_text_len)),
            )
        }
    };
    let paint_row_quad = |x0: Pixels, x1: Pixels, color: gpui::Rgba, window: &mut Window| {
        if x1 > x0 {
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left() + x0, bounds.top()),
                    point(bounds.left() + x1, bounds.bottom()),
                ),
                color,
            ));
        }
    };

    // Styled run backgrounds (including rendered Markdown's inline-code wash)
    // go below interactive overlays. Painting them after selection would mask
    // that part of the selection completely; this order intentionally blends
    // the translucent selection with the run background instead.
    if !paint_text.is_empty() && !paint_highlights.is_empty() {
        let _ = layout.paint_background(
            point(paint_x, y),
            metrics.line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    // Occurrences go down first, then the pair, then the selection. One click
    // produces both a pair and a set of occurrences, and the clicked name is in
    // both: painting the pair second means the delimiters stay legible where
    // the two overlap.
    if !occurrence_ranges.is_empty() {
        let color = view.read(cx).diff_text_occurrence_color();
        for range in &occurrence_ranges {
            let (x0, x1) = x_range_for_local(range);
            paint_row_quad(x0, x1, color, window);
        }
    }

    if !pair_ranges.is_empty() {
        let color = view.read(cx).diff_text_pair_match_color();
        for range in &pair_ranges {
            let (x0, x1) = x_range_for_local(range);
            paint_row_quad(x0, x1, color, window);
        }
    }

    if let Some(r) = selection {
        let (x0, x1) = x_range_for_local(&r);
        let color = view.read(cx).diff_text_selection_color();
        paint_row_quad(x0, x1, color, window);
    }

    let hitbox = DiffTextHitbox {
        atomic: false,
        bounds,
        layout_key,
        source_visible_ix,
        text_start_offset: if streamed_slice_is_wrap {
            streamed_slice_range
                .as_ref()
                .map(|range| range.start)
                .unwrap_or(visual_text_range.start)
        } else {
            visual_text_range.start
        },
        text_len: if streamed_slice_is_wrap {
            streamed_slice_range
                .as_ref()
                .map(|range| range.end.saturating_sub(range.start))
                .unwrap_or(text.len())
        } else if let Some(offset_map) = offset_map {
            offset_map.display_len()
        } else {
            total_text_len
        },
        offset_map: offset_map.cloned(),
        painted_text: paint_text.clone(),
        streamed_ascii_monospace_cell_width: hitbox_cell_width,
        wrapped: None,
        cells: Vec::new(),
    };

    view.update(cx, |this, cx| {
        this.set_diff_text_hitbox(visible_ix, region, hitbox);
        this.touch_diff_text_layout_cache(layout_key, shaped_new);
        if pending_prepared_syntax {
            this.ensure_prepared_syntax_chunk_poll(cx);
        }
        let column = match region {
            DiffTextRegion::Inline | DiffTextRegion::SplitLeft => {
                DiffHorizontalScrollColumn::Primary
            }
            DiffTextRegion::SplitRight => DiffHorizontalScrollColumn::SplitRight,
        };
        this.record_diff_horizontal_content_width_for_column(column, required_row_w, cx);
    });

    if paint_text.is_empty() {
        return;
    }

    if paint_highlights.is_empty() {
        let _ = layout.paint(
            point(paint_x, y),
            metrics.line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
        return;
    }

    let _ = layout.paint(
        point(paint_x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn diff_layout_base_key(
    text_hash: u64,
    base_style: &TextStyle,
    base_fg: gpui::Rgba,
    metrics: LineMetrics,
) -> u64 {
    let mut hasher = FxHasher::default();
    text_hash.hash(&mut hasher);
    metrics.font_size.hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_weight.hash(&mut hasher);
    base_fg.red.to_bits().hash(&mut hasher);
    base_fg.green.to_bits().hash(&mut hasher);
    base_fg.blue.to_bits().hash(&mut hasher);
    base_fg.alpha.to_bits().hash(&mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
fn ensure_layout_cached(
    view: &Entity<DiffRenderer>,
    text_hash: u64,
    text: &SharedString,
    base_style: &TextStyle,
    base_fg: gpui::Rgba,
    highlights: &[(Range<usize>, HighlightStyle)],
    highlights_hash: u64,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) -> (u64, gpui::ShapedLine, Option<gpui::ShapedLine>) {
    let base_key = diff_layout_base_key(text_hash, base_style, base_fg, metrics);

    let layout_key = if highlights.is_empty() {
        base_key
    } else {
        let mut hasher = FxHasher::default();
        base_key.hash(&mut hasher);
        highlights_hash.hash(&mut hasher);
        highlights.len().hash(&mut hasher);
        hasher.finish()
    };

    if let Some(entry) = view.read(cx).diff_text_layout_cache.get(&layout_key) {
        return (layout_key, entry.layout.clone(), None);
    }

    let shaped = if highlights.is_empty() {
        let run = base_style.to_run(text.len());
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run])
    } else {
        let runs = compute_runs(text.as_ref(), base_style, highlights);
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &runs)
    };
    (layout_key, shaped.clone(), Some(shaped))
}

#[cfg(test)]
mod tests;
