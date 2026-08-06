use super::canvas::keyed_canvas;
use super::diff_text::{
    PreparedDocumentByteRangeHighlights, build_cached_diff_query_overlay_styled_text,
    build_cached_diff_styled_text, build_cached_diff_styled_text_from_relative_highlights,
    hash_rgba_bits as hash_rgba, slice_cached_diff_styled_text,
    syntax_highlights_for_streamed_line_slice_heuristic, whitespace_visible_line_styled_text,
    whitespace_visible_line_styled_text_for_raw, whitespace_visible_line_text,
    whitespace_visible_styled_text,
};
use super::*;
use crate::view::panes::main::DiffHorizontalScrollColumn;
use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};
use gitcomet_core::domain::{DiffArea, DiffLineKind};
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, HighlightStyle, Hitbox, HitboxBehavior, Hsla, Pixels,
    Styled, TextRun, TextStyle, TransformationMatrix, TruncateFrom, Window, fill, point, px, size,
};
use rustc_hash::{FxHashMap, FxHasher};
use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;

const DIFF_FONT_SCALE: f32 = 0.80;

const GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 16_384;
const STREAMED_DIFF_TEXT_MIN_BYTES: usize = LARGE_DIFF_TEXT_MIN_BYTES;
const STREAMED_DIFF_TEXT_OVERSCAN_COLUMNS: usize = 64;
const STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE: &str = "0000000000";
const DIFF_TEXT_WRAP_WIDTH_SAMPLE: &str = "WWWWWWWWWW";
const DIFF_ROW_HEIGHT_PX: f32 = 20.0;
/// Width of a line-number cell (excluding the shared horizontal padding).
/// Sized to fit six digits of the diff monospace font; numbers right-align
/// toward the content so any slack sits before the digits, not between the
/// digits and the code.
const DIFF_GUTTER_BASE_WIDTH_PX: f32 = 38.0;
const DIFF_ROW_HORIZONTAL_PADDING_PX: f32 = 8.0;
const DIFF_ROW_TEXT_TRAILING_PADDING_PX: f32 = 16.0;
const DIFF_CHANGE_BAR_WIDTH_PX: f32 = 3.0;
const DIFF_ROW_BACKGROUND_OVERDRAW_PX: f32 = 1.0;

/// Default width of the blame/annotate column shown to the left of the diff
/// content when annotate is enabled. The live width is stored on the view and
/// is user-resizable; this is only the initial value and clamp reference.
pub(in crate::view) const DIFF_ANNOTATION_COLUMN_WIDTH_PX: f32 = 300.0;
/// Min/max bounds the annotation column can be dragged to.
pub(in crate::view) const DIFF_ANNOTATION_MIN_WIDTH_PX: f32 = 170.0;
pub(in crate::view) const DIFF_ANNOTATION_MAX_WIDTH_PX: f32 = 640.0;
/// Width of the recency "heat" bar at the far left of the annotation column.
pub(in crate::view) const DIFF_ANNOTATION_BORDER_WIDTH_PX: f32 = 3.0;
/// Gap between annotation sub-columns.
pub(in crate::view) const DIFF_ANNOTATION_GAP_PX: f32 = 6.0;
/// Fixed width of the "X ago" sub-column.
pub(in crate::view) const DIFF_ANNOTATION_WHEN_WIDTH_PX: f32 = 82.0;
/// Fixed width of the author-initials sub-column.
pub(in crate::view) const DIFF_ANNOTATION_INITIALS_WIDTH_PX: f32 = 22.0;
/// Cell width reserved for each trailing action icon.
pub(in crate::view) const DIFF_ANNOTATION_ICON_WIDTH_PX: f32 = 16.0;
/// Rendered (square) size of each action icon within its cell.
pub(in crate::view) const DIFF_ANNOTATION_ICON_GLYPH_PX: f32 = 13.0;
/// Diameter of the "viewing file at this commit" dot in the trailing slot.
pub(in crate::view) const DIFF_ANNOTATION_DOT_DIAMETER_PX: f32 = 6.0;

pub(in crate::view) const DIFF_ANNOTATION_PRIOR_ICON: &str = "icons/undo.svg";
pub(in crate::view) const DIFF_ANNOTATION_BROWSE_ICON: &str = "icons/history.svg";

/// Size of the hover-revealed stage/unstage button painted over the empty slack
/// at the far left of a change row. It overlays the line-number gutter rather
/// than reserving a column of its own, so enabling it never shifts diff text.
const DIFF_STAGE_GUTTER_CELL_PX: f32 = 16.0;
/// Rendered (square) size of the icon within the button.
const DIFF_STAGE_GUTTER_GLYPH_PX: f32 = 11.0;
const DIFF_STAGE_GUTTER_STAGE_ICON: &str = "icons/plus.svg";
const DIFF_STAGE_GUTTER_UNSTAGE_ICON: &str = "icons/minus.svg";

type HighlightSpans = Arc<[(Range<usize>, HighlightStyle)]>;

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

pub(in crate::view) fn blame_column_layout(
    column_left: Pixels,
    column_width: Pixels,
    row_top: Pixels,
    row_height: Pixels,
    ui_scale_percent: u32,
) -> BlameColumnLayout {
    let border_w = diff_scaled_px(DIFF_ANNOTATION_BORDER_WIDTH_PX, ui_scale_percent);
    let gap = diff_scaled_px(DIFF_ANNOTATION_GAP_PX, ui_scale_percent);
    let when_w = diff_scaled_px(DIFF_ANNOTATION_WHEN_WIDTH_PX, ui_scale_percent);
    let initials_w = diff_scaled_px(DIFF_ANNOTATION_INITIALS_WIDTH_PX, ui_scale_percent);
    let icon_w = diff_scaled_px(DIFF_ANNOTATION_ICON_WIDTH_PX, ui_scale_percent);

    let right = column_left + column_width;
    let cell = |x: Pixels, w: Pixels| Bounds::new(point(x, row_top), size(w, row_height));

    let browse_x = right - gap - icon_w;
    let prior_x = browse_x - gap - icon_w;
    let when_x = column_left + border_w + gap;
    let initials_x = when_x + when_w + gap;
    let message_x = initials_x + initials_w + gap;
    let message_w = (prior_x - gap - message_x).max(px(0.0));

    BlameColumnLayout {
        border: cell(column_left, border_w),
        when_x,
        initials_x,
        message: cell(message_x, message_w),
        prior_icon: cell(prior_x, icon_w),
        browse_icon: cell(browse_x, icon_w),
    }
}

/// Fold blame content into a row's canvas revision key so the cached canvas
/// repaints when the blame attribution (color/text/commit/width) or this row's
/// own hover highlight changes.
fn mix_blame_revision(
    base: u64,
    annotation_width: Pixels,
    hover: Option<AnnotArea>,
    blame: Option<&RowBlamePaint>,
) -> u64 {
    let mut hasher = FxHasher::default();
    base.hash(&mut hasher);
    f32::from(annotation_width).to_bits().hash(&mut hasher);
    // Only this row's own hover state feeds the cache key (not the raw cursor
    // position), so the cached canvas is invalidated only when the highlighted
    // sub-area of this specific row changes — not on every mouse move.
    hover.hash(&mut hasher);
    if let Some(blame) = blame {
        hash_rgba(&mut hasher, blame.border);
        blame.show_text.hash(&mut hasher);
        // `when` is time-relative ("3 days ago") so it changes for the same commit
        // as time passes — it must stay in the key. `initials`, `summary` and
        // `body` are NOT hashed: they are pure functions of `commit_id` (a given
        // commit always yields the same author/summary/body), so the commit id
        // hashed below already covers them. This avoids re-hashing a potentially
        // multi-KB commit body for every annotated row on every frame.
        hash_shared_string(&mut hasher, &blame.when);
        blame.commit_id.0.as_ref().hash(&mut hasher);
        blame.prior_exists.hash(&mut hasher);
        blame
            .prior_commit
            .as_ref()
            .map(|c| c.0.as_ref())
            .hash(&mut hasher);
        // Folded in so the trailing slot repaints when the viewed revision
        // changes (it toggles the browse icon vs. the "viewing here" dot).
        blame.is_viewed_commit.hash(&mut hasher);
    } else {
        u8::MAX.hash(&mut hasher);
    }
    hasher.finish()
}

/// Fold a row's stage-gutter button into its canvas revision key so the cached
/// canvas repaints when the button appears, disappears, or flips direction
/// (staging vs. unstaging). `hovered` is this row's own stored hover state, not
/// the raw cursor position, so mouse movement invalidates only the two rows
/// whose button visibility actually changed.
fn mix_stage_gutter_revision(
    base: u64,
    specs: &[Option<StageGutterSpec>],
    stage_hover: Option<DiffStageHover>,
    visible_ix: usize,
) -> u64 {
    if specs.iter().all(Option::is_none) {
        return base;
    }
    let mut hasher = FxHasher::default();
    base.hash(&mut hasher);
    for spec in specs {
        match spec {
            Some(spec) => {
                spec.area.hash(&mut hasher);
                spec.slot.hash(&mut hasher);
                // The kind decides which patch line a click resolves to, and the
                // paint closure captures it, so a cached canvas must not outlive
                // a change to it.
                std::mem::discriminant(&spec.kind).hash(&mut hasher);
                stage_hover
                    .filter(|hover| hover.visible_ix == visible_ix && hover.slot == spec.slot)
                    .map(|hover| hover.on_button)
                    .hash(&mut hasher);
            }
            None => u8::MAX.hash(&mut hasher),
        }
    }
    hasher.finish()
}

/// Paint a single line of text truncated to `max_width` with a trailing "…".
fn paint_truncated_text(
    text: &SharedString,
    x: Pixels,
    y: Pixels,
    max_width: Pixels,
    color: gpui::Rgba,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() || max_width <= px(0.0) {
        return;
    }
    let mut style = diff_text_style(window);
    style.color = color.into();
    let runs = vec![style.to_run(text.len())];
    let mut wrapper = window
        .text_system()
        .line_wrapper(style.font(), metrics.font_size);
    let (truncated, runs) =
        wrapper.truncate_line(text.clone(), max_width, "…", &runs, TruncateFrom::End);
    let shaped = window
        .text_system()
        .shape_line(truncated, metrics.font_size, runs.as_ref(), None);
    let _ = shaped.paint(
        point(x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// Whether a blame entry points at a real commit. Working-tree blame surfaces
/// uncommitted lines with an empty or all-zero object id ("Not Committed Yet");
/// those have no commit to open, so their action icons and click handlers are
/// suppressed.
fn blame_commit_is_navigable(commit_id: &gitcomet_core::domain::CommitId) -> bool {
    !commit_id.is_uncommitted()
}

/// Paint the annotation column for one row: a recency border bar and, on run
/// starts, the "X ago | initials | summary" sub-columns plus two action icons.
/// When `hovered` is Some, that area gets a highlight + underline (message) or
/// brighter color (icons).
#[allow(clippy::too_many_arguments)]
fn paint_blame_annotation(
    blame: &RowBlamePaint,
    layout: &BlameColumnLayout,
    y: Pixels,
    text_color: gpui::Rgba,
    theme: AppTheme,
    metrics: LineMetrics,
    when_metrics: LineMetrics,
    hovered: Option<AnnotArea>,
    prior_enabled: bool,
    browse_enabled: bool,
    ui_scale_percent: u32,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(layout.border, blame.border));

    if !blame.show_text {
        return;
    }

    paint_gutter_text(
        &blame.when,
        layout.when_x,
        y,
        text_color,
        when_metrics,
        window,
        cx,
    );
    paint_gutter_text(
        &blame.initials,
        layout.initials_x,
        y,
        text_color,
        metrics,
        window,
        cx,
    );
    window.paint_layer(layout.message, |window| {
        let message_color = if hovered == Some(AnnotArea::Message) {
            theme.colors.accent
        } else {
            text_color
        };
        paint_truncated_text(
            &blame.summary,
            layout.message.left(),
            y,
            layout.message.size.width,
            message_color,
            metrics,
            window,
            cx,
        );
        if hovered == Some(AnnotArea::Message) {
            let underline_y = y + metrics.line_height + px(0.5);
            window.paint_quad(fill(
                Bounds::new(
                    point(layout.message.left(), underline_y),
                    size(layout.message.size.width, px(1.0)),
                ),
                theme.colors.accent,
            ));
        }
    });

    // The "view file at parent commit" icon. Shown for committed lines whose
    // parent has the file, and for uncommitted ("Now") lines that carry a base
    // revision (the committed state before the local change).
    if prior_enabled {
        let icon_color = if hovered == Some(AnnotArea::PriorIcon) {
            theme.colors.accent
        } else {
            crate::theme::with_alpha(text_color, 0.6)
        };
        paint_blame_icon(
            DIFF_ANNOTATION_PRIOR_ICON,
            layout.prior_icon,
            icon_color,
            ui_scale_percent,
            window,
            cx,
        );
    }
    if blame.is_viewed_commit {
        // The file is currently open at this line's commit: mark it with a dot in
        // the trailing slot (where the browse icon would otherwise sit) instead of
        // the "view file at this commit" icon, which would be a no-op here.
        paint_blame_dot(
            layout.browse_icon,
            crate::theme::with_alpha(theme.colors.accent, 0.7),
            ui_scale_percent,
            window,
        );
    } else if browse_enabled {
        let icon_color = if hovered == Some(AnnotArea::BrowseIcon) {
            theme.colors.accent
        } else {
            crate::theme::with_alpha(text_color, 0.6)
        };
        paint_blame_icon(
            DIFF_ANNOTATION_BROWSE_ICON,
            layout.browse_icon,
            icon_color,
            ui_scale_percent,
            window,
            cx,
        );
    }
}

/// Paint an action icon centered within its cell.
fn paint_blame_icon(
    path: &'static str,
    cell: Bounds<Pixels>,
    color: gpui::Rgba,
    ui_scale_percent: u32,
    window: &mut Window,
    cx: &mut App,
) {
    paint_centered_svg_icon(
        path,
        cell,
        diff_scaled_px(DIFF_ANNOTATION_ICON_GLYPH_PX, ui_scale_percent),
        color,
        window,
        cx,
    );
}

/// Paint `path` as a square icon of `glyph` size, centered in `cell` and clamped
/// so it never spills out of it.
fn paint_centered_svg_icon(
    path: &'static str,
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
        Hsla::from(color),
        cx,
    );
}

/// Paint a small filled dot centered within `cell`, marking the line's commit as
/// the revision the file is currently being viewed at.
fn paint_blame_dot(
    cell: Bounds<Pixels>,
    color: gpui::Rgba,
    ui_scale_percent: u32,
    window: &mut Window,
) {
    let diameter = diff_scaled_px(DIFF_ANNOTATION_DOT_DIAMETER_PX, ui_scale_percent)
        .min(cell.size.width)
        .min(cell.size.height);
    let ox = cell.left() + (cell.size.width - diameter) * 0.5;
    let oy = cell.top() + (cell.size.height - diameter) * 0.5;
    let bounds = Bounds::new(point(ox, oy), size(diameter, diameter));
    window.paint_quad(fill(bounds, color).corner_radii(diameter * 0.5));
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::view) enum AnnotArea {
    Message,
    PriorIcon,
    BrowseIcon,
}

/// Hitboxes for annotation sub-column interactive areas.
#[derive(Clone, Debug)]
struct AnnotHitboxes {
    message: Hitbox,
    prior_icon: Hitbox,
    browse_icon: Hitbox,
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
            DiffArea::Unstaged => theme.colors.diff_add_text,
            DiffArea::Staged => theme.colors.diff_remove_text,
        }
    }

    fn tooltip(self) -> SharedString {
        match self.area {
            DiffArea::Unstaged => SharedString::from("Stage line"),
            DiffArea::Staged => SharedString::from("Unstage line"),
        }
    }
}

/// Cell the stage/unstage button occupies: the far left of a row, which with
/// line numbers shown is the empty slack ahead of the right-aligned number.
/// Shared by painting and hit-testing so the two cannot drift apart.
///
/// With line numbers hidden there is no such slack — the diff text starts here
/// — so the button overlaps its first characters and takes the clicks landing on
/// them. That is deliberate: reaching the button everywhere is worth more than
/// the couple of characters it sits on, and the chip is painted opaque so what
/// it covers reads as covered rather than as garbled text.
fn stage_gutter_cell(
    content_left: Pixels,
    row_top: Pixels,
    row_height: Pixels,
    ui_scale_percent: u32,
) -> Bounds<Pixels> {
    let pad = diff_row_horizontal_padding(ui_scale_percent);
    let width = diff_scaled_px(DIFF_STAGE_GUTTER_CELL_PX, ui_scale_percent);
    let height = width.min(row_height);
    Bounds::new(
        point(
            content_left + pad * 0.5,
            row_top + (row_height - height) * 0.5,
        ),
        size(width, height),
    )
}

#[derive(Clone, Debug)]
struct StageGutterPrepaint {
    spec: StageGutterSpec,
    cell: Bounds<Pixels>,
    hitbox: Hitbox,
}

/// Reserve the button's hitbox during prepaint. Called after the text hitbox so
/// the button's pointer cursor — and its clicks — win over the text I-beam, and
/// clipped to the content mask so a scrolled-away button cannot be hovered.
fn build_stage_gutter(
    window: &mut Window,
    spec: Option<StageGutterSpec>,
    content_left: Pixels,
    row_bounds: Bounds<Pixels>,
    ui_scale_percent: u32,
) -> Option<StageGutterPrepaint> {
    let spec = spec?;
    let cell = stage_gutter_cell(
        content_left,
        row_bounds.top(),
        row_bounds.size.height,
        ui_scale_percent,
    );
    let visible = cell.intersect(&window.content_mask().bounds);
    if visible.size.width <= px(0.0) || visible.size.height <= px(0.0) {
        return None;
    }
    Some(StageGutterPrepaint {
        spec,
        cell,
        hitbox: window.insert_hitbox(visible, HitboxBehavior::Normal),
    })
}

/// Paint a row's stage/unstage button and register its hover handling, returning
/// the click routing for it. Hovering anywhere in the row reveals the button;
/// hovering the button itself brightens it. The hover state comes from the view
/// (never from the live cursor) so it matches the value folded into the canvas
/// revision key. Clicks are handled by `install_diff_row_mouse_handlers`.
#[allow(clippy::too_many_arguments)]
fn paint_stage_gutter(
    prepaint: Option<&StageGutterPrepaint>,
    visible_ix: usize,
    theme: AppTheme,
    row_bg: gpui::Rgba,
    ui_scale_percent: u32,
    row_hitbox: &Hitbox,
    column_bounds: Option<Bounds<Pixels>>,
    view: &Entity<MainPaneView>,
    window: &mut Window,
    cx: &mut App,
) -> Option<StageGutterMouse> {
    let prepaint = prepaint?;
    let spec = prepaint.spec;
    window.set_cursor_style(CursorStyle::PointingHand, &prepaint.hitbox);

    let hover = view.update(cx, |this, _cx| {
        this.set_diff_stage_gutter_cell(visible_ix, spec.slot, prepaint.cell);
        this.diff_stage_gutter_hover
            .filter(|hover| hover.visible_ix == visible_ix && hover.slot == spec.slot)
    });

    if let Some(hover) = hover {
        let color = spec.color(theme);
        // The chip is opaque so it masks any line-number digits it covers. Over
        // the row it stays quiet; under the pointer it takes a tint of the
        // action it performs.
        let (chip, icon) = if hover.on_button {
            (
                crate::theme::composite_over(row_bg, with_alpha(color, 0.22)),
                color,
            )
        } else {
            (row_bg, with_alpha(color, 0.75))
        };
        window.paint_quad(fill(prepaint.cell, chip).corner_radii(px(theme.radii.control)));
        paint_centered_svg_icon(
            spec.icon(),
            prepaint.cell,
            diff_scaled_px(DIFF_STAGE_GUTTER_GLYPH_PX, ui_scale_percent),
            icon,
            window,
            cx,
        );
    }

    install_stage_gutter_hover_handler(
        window,
        view,
        visible_ix,
        spec,
        &prepaint.hitbox,
        row_hitbox,
        column_bounds,
    );

    Some(StageGutterMouse {
        hitbox: prepaint.hitbox.clone(),
        kind: spec.kind,
    })
}

/// Register hover handling for a row's stage-gutter button: on every mouse move
/// it resolves whether the cursor is over this row (which reveals the button) and
/// whether it is on the button itself (which brightens it and shows a tooltip),
/// updating the view only when that changes. `column_bounds` narrows the row to
/// one side in split views, where both columns paint their own button.
fn install_stage_gutter_hover_handler(
    window: &mut Window,
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    spec: StageGutterSpec,
    hitbox: &Hitbox,
    row_hitbox: &Hitbox,
    column_bounds: Option<Bounds<Pixels>>,
) {
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        let row_hitbox = row_hitbox.clone();
        move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            // Hover follows the hit test, so a panel painted over the diff hides
            // the button underneath instead of revealing it through the overlay.
            let on_row = row_hitbox.is_hovered(window)
                && column_bounds.is_none_or(|bounds| bounds.contains(&event.position));
            let next = on_row.then(|| DiffStageHover {
                visible_ix,
                slot: spec.slot,
                on_button: hitbox.is_hovered(window),
            });

            // Cheap gate so plain mouse movement doesn't borrow/notify the view
            // for every visible row: only act when this button's hover changes,
            // and never clear a hover that belongs to a different button.
            let current = view.read(cx).diff_stage_gutter_hover;
            if current == next {
                return;
            }
            if next.is_none()
                && !current
                    .is_some_and(|hover| hover.visible_ix == visible_ix && hover.slot == spec.slot)
            {
                return;
            }

            let tooltip = next.filter(|hover| hover.on_button).map(|_| spec.tooltip());
            view.update(cx, |this, cx| {
                this.update_diff_stage_gutter_hover(next, tooltip, cx);
            });
        }
    });
}

/// Register click handling for a row's annotation column.
#[allow(clippy::too_many_arguments)]
fn install_blame_annotation_mouse_handler(
    window: &mut Window,
    view: &Entity<MainPaneView>,
    message_hitbox: &Hitbox,
    prior_icon_hitbox: &Hitbox,
    browse_icon_hitbox: &Hitbox,
    commit_id: gitcomet_core::domain::CommitId,
    path: std::sync::Arc<std::path::Path>,
    source_path: Option<std::sync::Arc<std::path::Path>>,
    prior_commit: Option<gitcomet_core::domain::CommitId>,
    message_enabled: bool,
    prior_enabled: bool,
    browse_enabled: bool,
) {
    window.on_mouse_event({
        let view = view.clone();
        let message_hitbox = message_hitbox.clone();
        let prior_icon_hitbox = prior_icon_hitbox.clone();
        let browse_icon_hitbox = browse_icon_hitbox.clone();
        move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != gpui::MouseButton::Left {
                return;
            }
            let commit_id = commit_id.clone();
            let path = path.clone();
            // For renamed files, navigate to the historical name at this commit
            // rather than the current path (which may not exist in that tree).
            let historical_path = source_path
                .as_deref()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf());
            // `is_hovered`, not `contains`: this is a window-level listener, so
            // it runs even for clicks that landed on something painted over the
            // diff. Only the hit test knows what actually owns the pointer.
            let action = if browse_enabled && browse_icon_hitbox.is_hovered(window) {
                BlameClickAction::Browse
            } else if prior_enabled && prior_icon_hitbox.is_hovered(window) {
                BlameClickAction::PriorRevision
            } else if message_enabled && message_hitbox.is_hovered(window) {
                BlameClickAction::OpenDetails
            } else {
                return;
            };
            let prior_commit = prior_commit.clone();
            view.update(cx, |this, cx| {
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let msg = match action {
                    BlameClickAction::Browse => Msg::OpenFileAtCommit {
                        repo_id,
                        commit_id,
                        path: historical_path,
                    },
                    // An uncommitted ("Now") line's prior is the base revision it
                    // was edited from: open that commit directly. A committed line
                    // resolves and opens its commit's parent.
                    BlameClickAction::PriorRevision => match prior_commit {
                        Some(base) => Msg::OpenFileAtCommit {
                            repo_id,
                            commit_id: base,
                            path: historical_path,
                        },
                        None => Msg::OpenFileAtCommitParent {
                            repo_id,
                            commit_id,
                            path: historical_path,
                        },
                    },
                    BlameClickAction::OpenDetails => Msg::SelectCommit { repo_id, commit_id },
                };
                this.store.dispatch(msg);
                cx.notify();
            });
        }
    });
}

enum BlameClickAction {
    OpenDetails,
    PriorRevision,
    Browse,
}

/// Paint the annotation column for one row: border bar, text, icons, and
/// hover effects. Installs click handlers and sets cursor + tooltip via hitboxes.
#[allow(clippy::too_many_arguments)]
fn render_blame_column(
    blame: &RowBlamePaint,
    row_bounds: Bounds<Pixels>,
    annot_w: Pixels,
    y: Pixels,
    theme: AppTheme,
    metrics: LineMetrics,
    when_metrics: LineMetrics,
    ui_scale_percent: u32,
    visible_ix: usize,
    annot_hitboxes: Option<&AnnotHitboxes>,
    view: &Entity<MainPaneView>,
    window: &mut Window,
    cx: &mut App,
) {
    let layout = blame_column_layout(
        row_bounds.left(),
        annot_w,
        row_bounds.top(),
        row_bounds.size.height,
        ui_scale_percent,
    );

    let navigable = blame_commit_is_navigable(&blame.commit_id);
    // "View file at parent commit": committed lines whose parent has the file, or
    // uncommitted ("Now") lines carrying a base revision (the state before the
    // local change).
    let prior_enabled = (blame.prior_exists && navigable) || blame.prior_commit.is_some();
    // The commit message opens commit details — only for real commits.
    let message_enabled = navigable;
    // Hide "view file at this commit" when already viewing that commit (a dot is
    // painted there instead) and for uncommitted lines (no commit to browse).
    let browse_enabled = navigable && !blame.is_viewed_commit;
    // Drive the hover highlight from stored hover state (updated by the hover
    // handler on real hover transitions) rather than the live cursor position,
    // so this matches the value folded into the canvas revision key.
    let hovered = view
        .read(cx)
        .blame_annot_hover
        .and_then(|(ix, area)| (ix == visible_ix).then_some(area));

    if let Some(hb) = annot_hitboxes {
        if message_enabled {
            window.set_cursor_style(CursorStyle::PointingHand, &hb.message);
        }
        if prior_enabled {
            window.set_cursor_style(CursorStyle::PointingHand, &hb.prior_icon);
        }
        if browse_enabled {
            window.set_cursor_style(CursorStyle::PointingHand, &hb.browse_icon);
        }
    }

    paint_blame_annotation(
        blame,
        &layout,
        y,
        theme.colors.text_muted,
        theme,
        metrics,
        when_metrics,
        hovered,
        prior_enabled,
        browse_enabled,
        ui_scale_percent,
        window,
        cx,
    );

    if blame.show_text
        && (message_enabled || prior_enabled || browse_enabled)
        && let Some(hb) = annot_hitboxes
    {
        install_blame_annotation_mouse_handler(
            window,
            view,
            &hb.message,
            &hb.prior_icon,
            &hb.browse_icon,
            blame.commit_id.clone(),
            blame.path.clone(),
            blame.source_path.clone(),
            blame.prior_commit.clone(),
            message_enabled,
            prior_enabled,
            browse_enabled,
        );
        install_blame_annotation_hover_handler(
            window,
            view,
            visible_ix,
            hb,
            blame.summary.clone(),
            blame.body.clone(),
            message_enabled,
            prior_enabled,
            browse_enabled,
        );
    }
}

/// Register hover handling for a row's annotation column. On every mouse move it
/// resolves which sub-area (message / prior icon / browse icon) the cursor is
/// over and, only when that changes for this row, updates the view's hover state
/// (which drives the accent highlight on repaint) and the shared tooltip host.
fn install_blame_annotation_hover_handler(
    window: &mut Window,
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    hitboxes: &AnnotHitboxes,
    summary: SharedString,
    body: Option<SharedString>,
    message_enabled: bool,
    prior_enabled: bool,
    browse_enabled: bool,
) {
    window.on_mouse_event({
        let view = view.clone();
        let message = hitboxes.message.clone();
        let prior_icon = hitboxes.prior_icon.clone();
        let browse_icon = hitboxes.browse_icon.clone();
        move |_event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            // Hover follows the hit test, so a panel painted over the diff hides
            // the annotations underneath instead of highlighting them through it.
            let area = if message_enabled && message.is_hovered(window) {
                Some(AnnotArea::Message)
            } else if prior_enabled && prior_icon.is_hovered(window) {
                Some(AnnotArea::PriorIcon)
            } else if browse_enabled && browse_icon.is_hovered(window) {
                Some(AnnotArea::BrowseIcon)
            } else {
                None
            };

            // Cheap gate so plain mouse movement doesn't borrow/notify the view
            // for every visible row: only act when this row's hover changes, and
            // never clear a hover that belongs to a different row.
            let next = area.map(|a| (visible_ix, a));
            let current = view.read(cx).blame_annot_hover;
            if current == next {
                return;
            }
            if next.is_none() && !matches!(current, Some((ix, _)) if ix == visible_ix) {
                return;
            }

            let tooltip = match area {
                Some(AnnotArea::Message) => Some(body.clone().unwrap_or_else(|| summary.clone())),
                Some(AnnotArea::PriorIcon) => {
                    Some(SharedString::from("View file prior this change"))
                }
                Some(AnnotArea::BrowseIcon) => Some(SharedString::from("View file at this commit")),
                None => None,
            };

            view.update(cx, |this, cx| {
                this.update_blame_annot_hover(next, tooltip, cx);
            });
        }
    });
}

struct DiffTextPaintPayload {
    text: SharedString,
    highlights: HighlightSpans,
    highlights_hash: u64,
    text_hash: u64,
    offset_map: Option<DiffTextOffsetMap>,
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
    pub(super) word_ranges: Arc<[Range<usize>]>,
    pub(super) word_color: Option<gpui::Rgba>,
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
    (bg != theme.colors.window_bg).then_some(bg)
}

fn focused_row_outline_color(theme: AppTheme, bg: gpui::Rgba) -> gpui::Rgba {
    with_alpha(bg, if theme.is_dark { 0.72 } else { 0.56 })
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(in crate::view) struct DiffPaintRecord {
    pub(in crate::view) visible_ix: usize,
    pub(in crate::view) region: DiffTextRegion,
    pub(in crate::view) text: SharedString,
    pub(in crate::view) highlights: Vec<(Range<usize>, Option<gpui::Hsla>, Option<gpui::Hsla>)>,
    pub(in crate::view) row_bg: Option<gpui::Rgba>,
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

fn should_stream_diff_text(spec: Option<&StreamedDiffTextPaintSpec>) -> bool {
    let Some(spec) = spec else {
        return false;
    };
    is_streamable_diff_text(&spec.raw_text)
}

fn streamed_diff_text_cell_width_cache_key(base_style: &TextStyle, font_size: Pixels) -> u64 {
    let mut hasher = FxHasher::default();
    font_size.hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_weight.hash(&mut hasher);
    hasher.finish()
}

fn streamed_diff_text_ascii_cell_width(
    base_style: &TextStyle,
    font_size: Pixels,
    window: &mut Window,
) -> Pixels {
    let key = streamed_diff_text_cell_width_cache_key(base_style, font_size);
    if let Some(width) =
        STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE.with(|cache| cache.borrow().get(&key).copied())
    {
        return width;
    }

    let run = base_style.to_run(STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.len());
    let layout = window.text_system().shape_line(
        STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.into(),
        font_size,
        &[run],
        None,
    );
    let width = if STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.is_empty() {
        px(0.0)
    } else {
        layout.width / STREAMED_DIFF_TEXT_CELL_WIDTH_SAMPLE.len() as f32
    };
    STREAMED_DIFF_TEXT_CELL_WIDTH_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, width);
    });
    width
}

fn streamed_diff_text_visible_slice_range(
    bounds: Bounds<Pixels>,
    clip_bounds: Bounds<Pixels>,
    total_len: usize,
    cell_width: Pixels,
    overscan_columns: usize,
) -> Range<usize> {
    if total_len == 0 || cell_width <= px(0.0) {
        return 0..0;
    }

    let visible = bounds.intersect(&clip_bounds);
    let left = if visible.size.width > px(0.0) {
        (visible.left() - bounds.left()).max(px(0.0))
    } else {
        px(0.0)
    };
    let right = if visible.size.width > px(0.0) {
        (visible.right() - bounds.left()).max(left)
    } else {
        left
    };

    let start = ((left / cell_width).floor() as usize).saturating_sub(overscan_columns);
    let mut end = ((right / cell_width).ceil() as usize)
        .saturating_add(overscan_columns)
        .min(total_len);
    if end <= start {
        end = (start + 1).min(total_len);
    }
    start.min(total_len)..end
}

fn clip_ranges_to_slice(ranges: &[Range<usize>], slice_range: &Range<usize>) -> Vec<Range<usize>> {
    if ranges.is_empty() || slice_range.is_empty() {
        return Vec::new();
    }

    let mut clipped = Vec::new();
    for range in ranges {
        let start = range.start.max(slice_range.start);
        let end = range.end.min(slice_range.end);
        if start < end {
            clipped.push(
                start.saturating_sub(slice_range.start)..end.saturating_sub(slice_range.start),
            );
        }
    }
    clipped
}

fn push_or_extend_highlight(
    merged: &mut Vec<(Range<usize>, HighlightStyle)>,
    range: Range<usize>,
    style: HighlightStyle,
) {
    if range.is_empty() {
        return;
    }

    if let Some(last) = merged.last_mut()
        && last.0.end == range.start
        && last.1 == style
    {
        last.0.end = range.end;
        return;
    }

    merged.push((range, style));
}

fn hash_range(hasher: &mut FxHasher, range: &Range<usize>) {
    range.start.hash(hasher);
    range.end.hash(hasher);
}

fn streamed_diff_text_text_hash(spec: &StreamedDiffTextPaintSpec) -> u64 {
    spec.raw_text.identity_hash_without_loading()
}

fn streamed_diff_text_visible_text_hash(
    spec: &StreamedDiffTextPaintSpec,
    reveal_whitespace_chars: bool,
) -> u64 {
    let base = streamed_diff_text_text_hash(spec);
    if !reveal_whitespace_chars {
        return base;
    }

    let mut hasher = FxHasher::default();
    base.hash(&mut hasher);
    reveal_whitespace_chars.hash(&mut hasher);
    hasher.finish()
}

fn whitespace_marker_len(ch: char) -> usize {
    match ch {
        ' ' => '\u{00B7}'.len_utf8(),
        '\t' => '\u{2192}'.len_utf8(),
        '\r' => '\u{240D}'.len_utf8(),
        '\n' => '\u{21B5}'.len_utf8(),
        _ if ch.is_whitespace() => '\u{2420}'.len_utf8(),
        _ => ch.len_utf8(),
    }
}

fn diff_display_source_len_for_char(ch: char) -> usize {
    match ch {
        '\t' => 4,
        _ => ch.len_utf8(),
    }
}

pub(in crate::view) fn whitespace_visible_diff_offset_map(
    text: &str,
    append_eol_marker: bool,
) -> DiffTextOffsetMap {
    let source_len = crate::view::diff_utils::diff_text_display_len(text);
    let mut display_len = text.chars().map(whitespace_marker_len).sum::<usize>();
    let append_synthetic_eol = append_eol_marker && !text.ends_with('\n');
    if append_synthetic_eol {
        display_len = display_len.saturating_add('\u{21B5}'.len_utf8());
    }

    let mut display_to_source = vec![0usize; display_len.saturating_add(1)];
    let mut source_to_display = vec![0usize; source_len.saturating_add(1)];
    let mut source = 0usize;
    let mut display = 0usize;

    for ch in text.chars() {
        let source_start = source;
        let display_start = display;
        source = source.saturating_add(diff_display_source_len_for_char(ch));
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

fn display_range_for_source_range(
    map: &DiffTextOffsetMap,
    source_range: &Range<usize>,
) -> Range<usize> {
    let source_start = source_range.start.min(map.source_len());
    let source_end = source_range.end.min(map.source_len());
    let display_start = map.display_offset_for_source(source_start);
    let display_end = if source_end >= map.source_len() {
        map.display_len()
    } else {
        map.display_offset_for_source(source_end)
    };
    display_start.min(map.display_len())..display_end.min(map.display_len())
}

fn slice_diff_text_offset_map(
    map: &DiffTextOffsetMap,
    display_range: Range<usize>,
    source_range: Range<usize>,
) -> DiffTextOffsetMap {
    let display_start = display_range.start.min(map.display_len());
    let display_end = display_range.end.min(map.display_len()).max(display_start);
    let source_start = source_range.start.min(map.source_len());
    let source_end = source_range.end.min(map.source_len()).max(source_start);
    let display_len = display_end.saturating_sub(display_start);
    let source_len = source_end.saturating_sub(source_start);

    let display_to_source = (0..=display_len)
        .map(|offset| {
            map.source_offset_for_display(display_start.saturating_add(offset))
                .clamp(source_start, source_end)
                .saturating_sub(source_start)
        })
        .collect::<Vec<_>>();
    let source_to_display = (0..=source_len)
        .map(|offset| {
            map.display_offset_for_source(source_start.saturating_add(offset))
                .clamp(display_start, display_end)
                .saturating_sub(display_start)
        })
        .collect::<Vec<_>>();

    DiffTextOffsetMap {
        display_to_source: Arc::from(display_to_source),
        source_to_display: Arc::from(source_to_display),
    }
}

fn streamed_diff_text_highlights_hash(spec: &StreamedDiffTextPaintSpec) -> u64 {
    let mut hasher = FxHasher::default();
    spec.query.as_ref().hash(&mut hasher);
    spec.query_options.hash(&mut hasher);
    for range in spec.word_ranges.iter() {
        hash_range(&mut hasher, range);
    }
    if let Some(color) = spec.word_color {
        hash_rgba(&mut hasher, color);
    }
    match &spec.syntax {
        StreamedDiffTextSyntaxSource::None => {
            0u8.hash(&mut hasher);
        }
        StreamedDiffTextSyntaxSource::Heuristic { language, mode } => {
            1u8.hash(&mut hasher);
            language.hash(&mut hasher);
            mode.hash(&mut hasher);
        }
        StreamedDiffTextSyntaxSource::Prepared {
            document_text,
            line_starts,
            language,
            line_ix,
            ..
        } => {
            2u8.hash(&mut hasher);
            language.hash(&mut hasher);
            line_ix.hash(&mut hasher);
            (document_text.as_ptr() as usize).hash(&mut hasher);
            document_text.len().hash(&mut hasher);
            (line_starts.as_ptr() as usize).hash(&mut hasher);
            line_starts.len().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn hash_overlay_ranges(
    base_highlights_hash: u64,
    ranges: &[Range<usize>],
    background_color: gpui::Hsla,
) -> u64 {
    let mut hasher = FxHasher::default();
    base_highlights_hash.hash(&mut hasher);
    hash_rgba(&mut hasher, background_color.into());
    for range in ranges {
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
    }
    hasher.finish()
}

fn overlay_background_ranges_on_styled_text(
    base: &CachedDiffStyledText,
    ranges: &[Range<usize>],
    background_color: gpui::Hsla,
) -> CachedDiffStyledText {
    if ranges.is_empty() || base.text.is_empty() {
        return base.clone();
    }

    let base_highlights = base.highlights.as_ref();
    if base_highlights.is_empty() {
        let mut merged = Vec::with_capacity(ranges.len());
        for range in ranges.iter().cloned() {
            push_or_extend_highlight(
                &mut merged,
                range,
                HighlightStyle {
                    background_color: Some(background_color),
                    ..HighlightStyle::default()
                },
            );
        }
        return CachedDiffStyledText {
            text: base.text.clone(),
            highlights: Arc::from(merged),
            highlights_hash: hash_overlay_ranges(base.highlights_hash, ranges, background_color),
            text_hash: base.text_hash,
        };
    }

    let mut merged = Vec::with_capacity(base_highlights.len() + ranges.len() * 2);
    let mut base_ix = 0usize;
    let mut range_ix = 0usize;
    let mut cursor = 0usize;
    let text_len = base.text.len();
    let default_style = HighlightStyle::default();

    while cursor < text_len {
        while base_ix < base_highlights.len() && base_highlights[base_ix].0.end <= cursor {
            base_ix += 1;
        }
        while range_ix < ranges.len() && ranges[range_ix].end <= cursor {
            range_ix += 1;
        }

        let active_base = base_highlights
            .get(base_ix)
            .filter(|(range, _)| range.start <= cursor && range.end > cursor);
        let active_range = ranges
            .get(range_ix)
            .filter(|range| range.start <= cursor && range.end > cursor);

        let mut next_boundary = text_len;
        if let Some((range, _)) = active_base {
            next_boundary = next_boundary.min(range.end.min(text_len));
        } else if let Some((range, _)) = base_highlights.get(base_ix) {
            next_boundary = next_boundary.min(range.start.min(text_len));
        }
        if let Some(range) = active_range {
            next_boundary = next_boundary.min(range.end.min(text_len));
        } else if let Some(range) = ranges.get(range_ix) {
            next_boundary = next_boundary.min(range.start.min(text_len));
        }

        if next_boundary <= cursor {
            break;
        }

        let mut style = active_base.map(|(_, style)| *style).unwrap_or_default();
        if active_range.is_some() {
            style.background_color = Some(background_color);
        }

        if style != default_style {
            push_or_extend_highlight(&mut merged, cursor..next_boundary, style);
        }

        cursor = next_boundary;
    }

    CachedDiffStyledText {
        text: base.text.clone(),
        highlights: Arc::from(merged),
        highlights_hash: hash_overlay_ranges(base.highlights_hash, ranges, background_color),
        text_hash: base.text_hash,
    }
}

fn should_apply_query_overlay_to_streamed_slice(
    options: DiffSearchOptions,
    slice_range: &Range<usize>,
    total_len: usize,
) -> bool {
    if !options.whole_word && !options.regex {
        return true;
    }

    slice_range.start == 0 && slice_range.end >= total_len
}

fn streamed_diff_text_relative_prepared_highlights(
    theme: AppTheme,
    spec: &StreamedDiffTextPaintSpec,
    slice_range: &Range<usize>,
) -> Option<PreparedDocumentByteRangeHighlights> {
    let StreamedDiffTextSyntaxSource::Prepared {
        document_text,
        line_starts,
        document,
        language,
        line_ix,
    } = &spec.syntax
    else {
        return None;
    };

    let text_len = document_text.len();
    let line_start = line_starts
        .get(*line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let abs_start = line_start.saturating_add(slice_range.start).min(text_len);
    let abs_end = line_start.saturating_add(slice_range.end).min(text_len);
    if abs_start >= abs_end {
        return Some(PreparedDocumentByteRangeHighlights::default());
    }

    rows::request_syntax_highlights_for_prepared_document_byte_range(
        theme,
        document_text.as_ref(),
        line_starts.as_ref(),
        *document,
        *language,
        abs_start..abs_end,
    )
}

fn build_streamed_diff_slice_styled_text(
    theme: AppTheme,
    spec: &StreamedDiffTextPaintSpec,
    requested_slice_range: &Range<usize>,
) -> (CachedDiffStyledText, bool, Range<usize>) {
    let (slice_text, resolved_slice_range) = spec
        .raw_text
        .slice_text_resolved(requested_slice_range.clone())
        .unwrap_or((Cow::Borrowed(""), 0..0));
    let slice_text_ref = slice_text.as_ref();

    let mut pending = false;
    let mut base = match &spec.syntax {
        StreamedDiffTextSyntaxSource::None => build_cached_diff_styled_text(
            theme,
            slice_text_ref,
            &[],
            "",
            None,
            rows::DiffSyntaxMode::HeuristicOnly,
            None,
        ),
        StreamedDiffTextSyntaxSource::Heuristic { language, mode } => {
            match syntax_highlights_for_streamed_line_slice_heuristic(
                theme,
                &spec.raw_text,
                *language,
                requested_slice_range.clone(),
                resolved_slice_range.clone(),
            ) {
                Some(highlights) => build_cached_diff_styled_text_from_relative_highlights(
                    slice_text_ref,
                    highlights.as_slice(),
                ),
                None => build_cached_diff_styled_text(
                    theme,
                    slice_text_ref,
                    &[],
                    "",
                    Some(*language),
                    *mode,
                    None,
                ),
            }
        }
        StreamedDiffTextSyntaxSource::Prepared { language, .. } => {
            match streamed_diff_text_relative_prepared_highlights(
                theme,
                spec,
                &resolved_slice_range,
            ) {
                Some(result) => {
                    pending = result.pending;
                    let StreamedDiffTextSyntaxSource::Prepared {
                        line_starts,
                        line_ix,
                        ..
                    } = &spec.syntax
                    else {
                        unreachable!();
                    };
                    let line_start = line_starts
                        .get(*line_ix)
                        .copied()
                        .unwrap_or_default()
                        .saturating_add(resolved_slice_range.start);
                    let mut relative = Vec::with_capacity(result.highlights.len());
                    for (range, style) in result.highlights {
                        let start = range.start.max(line_start);
                        let end = range
                            .end
                            .min(line_start.saturating_add(resolved_slice_range.len()));
                        if start < end {
                            relative.push((
                                start.saturating_sub(line_start)..end.saturating_sub(line_start),
                                style,
                            ));
                        }
                    }
                    if relative.is_empty() {
                        match syntax_highlights_for_streamed_line_slice_heuristic(
                            theme,
                            &spec.raw_text,
                            *language,
                            requested_slice_range.clone(),
                            resolved_slice_range.clone(),
                        ) {
                            Some(highlights) => {
                                build_cached_diff_styled_text_from_relative_highlights(
                                    slice_text_ref,
                                    highlights.as_slice(),
                                )
                            }
                            None => build_cached_diff_styled_text(
                                theme,
                                slice_text_ref,
                                &[],
                                "",
                                Some(*language),
                                rows::DiffSyntaxMode::HeuristicOnly,
                                None,
                            ),
                        }
                    } else {
                        build_cached_diff_styled_text_from_relative_highlights(
                            slice_text_ref,
                            relative.as_slice(),
                        )
                    }
                }
                None => build_cached_diff_styled_text(
                    theme,
                    slice_text_ref,
                    &[],
                    "",
                    Some(*language),
                    rows::DiffSyntaxMode::HeuristicOnly,
                    None,
                ),
            }
        }
    };

    if !spec.word_ranges.is_empty()
        && let Some(mut color) = spec.word_color
    {
        let clipped = clip_ranges_to_slice(spec.word_ranges.as_ref(), &resolved_slice_range);
        if !clipped.is_empty() {
            color.a = if theme.is_dark { 0.22 } else { 0.16 };
            base =
                overlay_background_ranges_on_styled_text(&base, clipped.as_slice(), color.into());
        }
    }

    if let Some(matcher) = spec.query_matcher.as_deref()
        && should_apply_query_overlay_to_streamed_slice(
            spec.query_options,
            &resolved_slice_range,
            spec.raw_text.len(),
        )
    {
        base = build_cached_diff_query_overlay_styled_text(theme, &base, matcher);
    }

    (base, pending, resolved_slice_range)
}

fn diff_text_paint_payload(
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<&StreamedDiffTextPaintSpec>,
    raw_text: Option<&str>,
    reveal_whitespace_chars: bool,
    region: DiffTextRegion,
    wrap: Option<DiffTextWrapSlice>,
) -> DiffTextPaintPayload {
    if reveal_whitespace_chars {
        if should_stream_diff_text(streamed_spec) {
            let spec = streamed_spec.expect("streamed spec checked above");
            return DiffTextPaintPayload {
                text: SharedString::default(),
                highlights: empty_highlights(),
                highlights_hash: streamed_diff_text_highlights_hash(spec),
                text_hash: streamed_diff_text_visible_text_hash(spec, true),
                offset_map: None,
            };
        }

        let mut offset_map: Option<DiffTextOffsetMap> = None;
        let styled = if let Some(styled) = styled {
            let visible = if let Some(raw_text) = raw_text {
                offset_map = Some(whitespace_visible_diff_offset_map(raw_text, true));
                whitespace_visible_line_styled_text_for_raw(styled, raw_text)
            } else {
                offset_map = Some(whitespace_visible_diff_offset_map(
                    styled.text.as_ref(),
                    true,
                ));
                whitespace_visible_line_styled_text(styled)
            };
            Some(visible)
        } else if let Some(spec) = streamed_spec {
            let raw_text = spec.raw_text.as_ref();
            offset_map = Some(whitespace_visible_diff_offset_map(raw_text, true));
            let text = whitespace_visible_line_text(raw_text);
            let text_hash = {
                let mut hasher = FxHasher::default();
                text.as_ref().hash(&mut hasher);
                hasher.finish()
            };
            Some(CachedDiffStyledText {
                text,
                highlights: empty_highlights(),
                highlights_hash: 0,
                text_hash,
            })
        } else {
            None
        };

        let wrapped;
        let styled = if let (Some(styled), Some(wrap)) = (styled.as_ref(), wrap) {
            let source_range = wrap.range_for_region(region);
            let display_range = offset_map
                .as_ref()
                .map(|map| display_range_for_source_range(map, &source_range))
                .unwrap_or_else(|| source_range.clone());
            wrapped = slice_cached_diff_styled_text(styled, display_range.clone());
            offset_map = offset_map
                .as_ref()
                .map(|map| slice_diff_text_offset_map(map, display_range, source_range));
            Some(&wrapped)
        } else {
            styled.as_ref()
        };
        let text = styled.map(|s| s.text.clone()).unwrap_or_default();
        let highlights = styled
            .map(|s| Arc::clone(&s.highlights))
            .unwrap_or_else(empty_highlights);
        let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
        let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);
        return DiffTextPaintPayload {
            text,
            highlights,
            highlights_hash,
            text_hash,
            offset_map,
        };
    }

    if should_stream_diff_text(streamed_spec) {
        let spec = streamed_spec.expect("streamed spec checked above");
        return DiffTextPaintPayload {
            text: SharedString::default(),
            highlights: empty_highlights(),
            highlights_hash: streamed_diff_text_highlights_hash(spec),
            text_hash: streamed_diff_text_visible_text_hash(spec, false),
            offset_map: None,
        };
    }

    let wrapped;
    let styled = if let (Some(styled), Some(wrap)) = (styled, wrap) {
        wrapped = slice_cached_diff_styled_text(styled, wrap.range_for_region(region));
        Some(&wrapped)
    } else {
        styled
    };
    let text = styled.map(|s| s.text.clone()).unwrap_or_default();
    let highlights = styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
    let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);
    DiffTextPaintPayload {
        text,
        highlights,
        highlights_hash,
        text_hash,
        offset_map: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_diff_line_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
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
    raw_text: Option<&str>,
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
    let canvas_id: gpui::ElementId = ("diff_row_canvas_inline", visible_ix).into();
    let test_row_bg = semantic_diff_row_bg(theme, bg);

    keyed_canvas(
        (canvas_id, format!("{revision:016x}")),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = if show_line_numbers {
                gutter_cell_total_width(pad, ui_scale_percent)
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
            let line_metrics = line_metrics(window);
            let when_metrics = line_metrics_annot_when(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            // Selection must not tint the annotation sidebar: fill it with a
            // neutral panel color and fill the content area with the row bg.
            paint_row_bg_with_annotation(
                window,
                prepaint.bounds,
                prepaint.annot_w,
                bg,
                theme.colors.surface_bg,
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
                    window,
                    cx,
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
                    window,
                    cx,
                );
            }

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
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
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .bg(bg)
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn split_diff_line_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
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
    left_raw_text: Option<&str>,
    right_raw_text: Option<&str>,
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
        left_styled,
        left_streamed_spec.as_ref(),
        left_raw_text,
        reveal_whitespace_chars,
        DiffTextRegion::SplitLeft,
        wrap,
    );
    let right_payload = diff_text_paint_payload(
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
    let canvas_id: gpui::ElementId = ("diff_row_canvas_split", visible_ix).into();
    let left_test_row_bg = semantic_diff_row_bg(theme, left_bg);
    let right_test_row_bg = semantic_diff_row_bg(theme, right_bg);

    keyed_canvas(
        (canvas_id, format!("{revision:016x}")),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = if show_line_numbers {
                gutter_cell_total_width(pad, ui_scale_percent)
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
            let line_metrics = line_metrics(window);
            let when_metrics = line_metrics_annot_when(window);
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
                    theme.colors.surface_bg,
                );
            }
            window.paint_quad(fill(row_bg_fill_bounds(prepaint.left_col), left_bg));
            window.paint_quad(fill(
                row_bg_fill_bounds(prepaint.sep_bounds),
                theme.colors.border,
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
                    window,
                    cx,
                );
            }

            if show_line_numbers {
                let gutter_total = gutter_cell_total_width(prepaint.pad, ui_scale_percent);
                paint_gutter_text_right_aligned(
                    &old,
                    prepaint.left_col.left() + gutter_total - prepaint.pad,
                    y,
                    left_gutter,
                    line_metrics,
                    window,
                    cx,
                );
                paint_gutter_text_right_aligned(
                    &new,
                    prepaint.right_col.left() + gutter_total - prepaint.pad,
                    y,
                    right_gutter,
                    line_metrics,
                    window,
                    cx,
                );
            }

            window.paint_layer(prepaint.left_text_bounds, |window| {
                paint_selectable_diff_text(
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
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn patch_split_column_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
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
    raw_text: Option<&str>,
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
    let canvas_id: gpui::ElementId = (
        match column {
            super::diff::PatchSplitColumn::Left => "diff_row_canvas_file_split_left",
            super::diff::PatchSplitColumn::Right => "diff_row_canvas_file_split_right",
        },
        visible_ix,
    )
        .into();
    let test_row_bg = semantic_diff_row_bg(theme, bg);

    keyed_canvas(
        (canvas_id, format!("{revision:016x}")),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = if show_line_numbers {
                gutter_cell_total_width(pad, ui_scale_percent)
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
            let line_metrics = line_metrics(window);
            let when_metrics = line_metrics_annot_when(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            // Selection must not tint the annotation sidebar: fill it with a
            // neutral panel color and fill the content area with the row bg.
            paint_row_bg_with_annotation(
                window,
                prepaint.bounds,
                prepaint.annot_w,
                bg,
                theme.colors.surface_bg,
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
                    window,
                    cx,
                );
            }

            if show_line_numbers {
                let gutter_total = gutter_cell_total_width(prepaint.pad, ui_scale_percent);
                paint_gutter_text_right_aligned(
                    &line_no,
                    prepaint.bounds.left() + prepaint.annot_w + gutter_total - prepaint.pad,
                    y,
                    gutter_fg,
                    line_metrics,
                    window,
                    cx,
                );
            }

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
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
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn worktree_preview_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    ui_scale_percent: u32,
    ix: usize,
    min_width: Pixels,
    annotation_width: Pixels,
    blame: Option<RowBlamePaint>,
    bar_color: Option<gpui::Rgba>,
    line_no: SharedString,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<StreamedDiffTextPaintSpec>,
    raw_text: Option<&str>,
    reveal_whitespace_chars: bool,
) -> AnyElement {
    let paint_payload = diff_text_paint_payload(
        styled,
        streamed_spec.as_ref(),
        raw_text,
        reveal_whitespace_chars,
        DiffTextRegion::Inline,
        None,
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
            let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
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
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            // Reserve the annotation sidebar with the neutral panel color (matching
            // the diff renderers) so the blame column background is consistent with
            // the diff view; fill the content area with the row background.
            paint_row_bg_with_annotation(
                window,
                bounds,
                prepaint.annot_w,
                theme.colors.window_bg,
                theme.colors.surface_bg,
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
                let when_metrics = line_metrics_annot_when(window);
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
                    window,
                    cx,
                );
            }

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            paint_gutter_text_right_aligned(
                &line_no,
                prepaint.inner.left() + gutter_cell_total_width(prepaint.pad, ui_scale_percent)
                    - prepaint.pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
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
                    theme.colors.text,
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
                                cx,
                            );
                            cx.notify();
                        });
                    } else if event.button == gpui::MouseButton::Right {
                        view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                ix,
                                DiffTextRegion::Inline,
                                event.position,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            });
        },
    )
    .h(diff_row_height(ui_scale_percent))
    .min_w(min_width + annotation_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

fn build_annot_hitboxes(
    window: &mut Window,
    row_bounds: Bounds<Pixels>,
    annot_w: Pixels,
    ui_scale_percent: u32,
) -> Option<AnnotHitboxes> {
    if annot_w <= px(0.0) {
        return None;
    }
    let layout = blame_column_layout(
        row_bounds.left(),
        annot_w,
        row_bounds.top(),
        row_bounds.size.height,
        ui_scale_percent,
    );
    let clip = window.content_mask().bounds;
    Some(AnnotHitboxes {
        message: window.insert_hitbox(layout.message.intersect(&clip), HitboxBehavior::Normal),
        prior_icon: window
            .insert_hitbox(layout.prior_icon.intersect(&clip), HitboxBehavior::Normal),
        browse_icon: window
            .insert_hitbox(layout.browse_icon.intersect(&clip), HitboxBehavior::Normal),
    })
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

#[derive(Clone, Debug)]
enum DiffRowTextRegions {
    Single {
        region: DiffTextRegion,
        bounds: Bounds<Pixels>,
    },
    Split {
        left_bounds: Bounds<Pixels>,
        right_bounds: Bounds<Pixels>,
    },
}

impl DiffRowTextRegions {
    fn single(region: DiffTextRegion, bounds: Bounds<Pixels>) -> Self {
        Self::Single { region, bounds }
    }

    fn split(left_bounds: Bounds<Pixels>, right_bounds: Bounds<Pixels>) -> Self {
        Self::Split {
            left_bounds,
            right_bounds,
        }
    }

    fn region_at(&self, position: gpui::Point<Pixels>) -> Option<DiffTextRegion> {
        match self {
            Self::Single { region, bounds } => bounds.contains(&position).then_some(*region),
            Self::Split {
                left_bounds,
                right_bounds,
            } => {
                if left_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitLeft)
                } else if right_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitRight)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffRowRightClickBehavior {
    OpenContextMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffRowMouseUpBehavior {
    None,
    HandlePatchRowClick,
}

#[derive(Clone, Debug)]
struct DiffRowMouseHandlers {
    row_hitbox: Hitbox,
    regions: DiffRowTextRegions,
    right_click: DiffRowRightClickBehavior,
    mouse_up: DiffRowMouseUpBehavior,
    /// Stage/unstage buttons painted in this row's gutter(s): one per column, so
    /// at most two in split views and at most one anywhere else.
    stage: Vec<StageGutterMouse>,
}

/// Click routing for one stage/unstage button.
#[derive(Clone, Debug)]
struct StageGutterMouse {
    hitbox: Hitbox,
    kind: DiffLineKind,
}

impl StageGutterMouse {
    fn hovered(buttons: &[Self], window: &Window) -> Option<DiffLineKind> {
        buttons
            .iter()
            .find(|button| button.hitbox.is_hovered(window))
            .map(|button| button.kind)
    }
}

/// Row mouse handlers are window-level listeners: they see every event, whatever
/// is painted over the diff. Asking the row's hitbox (rather than its bounds)
/// whether it is hovered defers to the hit test, so a click on a panel floating
/// above the diff — the collapsed sidebar's section popover, say — stops there
/// instead of also landing on the row beneath it.
fn should_handle_row_mouse_event(
    phase: DispatchPhase,
    row_hitbox: &Hitbox,
    window: &Window,
) -> bool {
    phase == DispatchPhase::Bubble && row_hitbox.is_hovered(window)
}

fn install_diff_row_mouse_handlers(
    window: &mut Window,
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    handlers: DiffRowMouseHandlers,
) {
    let DiffRowMouseHandlers {
        row_hitbox,
        regions,
        right_click,
        mouse_up,
        stage,
    } = handlers;
    let row_hitbox_for_down = row_hitbox.clone();
    let regions = regions.clone();
    let stage_for_down = stage.clone();
    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if !should_handle_row_mouse_event(phase, &row_hitbox_for_down, window) {
                return;
            }

            let region = regions.region_at(event.position);

            if event.button == gpui::MouseButton::Left {
                let focus = view.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                // The gutter button owns the whole press: staging happens here,
                // and neither text selection nor row selection may see it.
                // Claiming the press is what stands the release handlers down —
                // staging reloads the diff, so the release lands on a repainted
                // row whose fresh handlers would otherwise read it as an
                // ordinary click. The claim outlives the release and is cleared
                // by the next press, so it cannot swallow a later click.
                // A repeat click of a double-click stages nothing: the first one
                // is still in flight, and window-activating clicks
                // (`first_mouse`) must not act at all.
                if let Some(kind) = StageGutterMouse::hovered(&stage_for_down, window) {
                    crate::press_gesture::claim_press(cx);
                    cx.stop_propagation();
                    if event.click_count > 1 || event.first_mouse {
                        return;
                    }
                    view.update(cx, |this, cx| {
                        this.stage_or_unstage_diff_line(visible_ix, kind, cx);
                        cx.notify();
                    });
                    return;
                }
                if let Some(region) = region {
                    let click_count = event.click_count;
                    let position = event.position;
                    view.update(cx, |this, cx| {
                        this.handle_diff_text_mouse_down(
                            visible_ix,
                            region,
                            position,
                            click_count,
                            cx,
                        );
                        cx.notify();
                    });
                }
            } else if event.button == gpui::MouseButton::Right
                && let Some(region) = region
            {
                match right_click {
                    DiffRowRightClickBehavior::OpenContextMenu => {
                        view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                visible_ix,
                                region,
                                event.position,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            }
        }
    });

    if mouse_up == DiffRowMouseUpBehavior::None {
        return;
    }

    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseUpEvent, phase, window, cx| {
            if event.button != gpui::MouseButton::Left
                || !should_handle_row_mouse_event(phase, &row_hitbox, window)
            {
                return;
            }

            // A canvas cannot lean on `on_click` to pair press and release, so
            // it asks who owns the press instead.
            if crate::press_gesture::is_press_claimed(cx) {
                return;
            }

            // A release over a gutter button belongs to that button (it already
            // staged on press): it must not also move the row selection.
            if StageGutterMouse::hovered(&stage, window).is_some() {
                return;
            }

            let shift = event.modifiers.shift;
            view.update(cx, |this, cx| {
                if this.consume_suppress_click_after_drag() {
                    cx.notify();
                    return;
                }
                this.handle_patch_row_click(visible_ix, DiffClickKind::Line, shift);
                cx.notify();
            });
        }
    });
}

#[derive(Clone, Copy, Debug)]
struct LineMetrics {
    font_size: Pixels,
    line_height: Pixels,
}

fn diff_text_style(window: &Window) -> TextStyle {
    let mut style = window.text_style();
    style.font_weight = FontWeight::NORMAL;
    style
}

fn line_metrics(window: &Window) -> LineMetrics {
    line_metrics_scaled(window, 1.0)
}

/// Diff-text metrics at `extra_scale` times the base diff font size (1.0 = the
/// regular row text; the annotation "when" column uses a slightly smaller scale).
fn line_metrics_scaled(window: &Window, extra_scale: f32) -> LineMetrics {
    let style = diff_text_style(window);
    let font_size = style.font_size.to_pixels(window.rem_size()) * DIFF_FONT_SCALE * extra_scale;
    let line_height = style
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    LineMetrics {
        font_size,
        line_height,
    }
}

/// Smaller font metrics for the "X ago" sub-column in the annotation panel.
fn line_metrics_annot_when(window: &Window) -> LineMetrics {
    line_metrics_scaled(window, 0.85)
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
) -> Pixels {
    let mut style = diff_text_style(window);
    style.font_family = editor_font_family.into();
    let font_size = style.font_size.to_pixels(window.rem_size()) * DIFF_FONT_SCALE;
    let run = style.to_run(DIFF_TEXT_WRAP_WIDTH_SAMPLE.len());
    let layout = window.text_system().shape_line(
        DIFF_TEXT_WRAP_WIDTH_SAMPLE.into(),
        font_size,
        &[run],
        None,
    );
    if DIFF_TEXT_WRAP_WIDTH_SAMPLE.is_empty() {
        px(1.0)
    } else {
        (layout.width / DIFF_TEXT_WRAP_WIDTH_SAMPLE.len() as f32).max(px(1.0))
    }
}

fn center_text_y(bounds: Bounds<Pixels>, line_height: Pixels) -> Pixels {
    let extra = (bounds.size.height - line_height).max(px(0.0));
    bounds.top() + extra * 0.5
}

fn px_2(window: &Window) -> Pixels {
    crate::ui_scale::design_px_from_window(DIFF_ROW_HORIZONTAL_PADDING_PX, window)
}

pub(in crate::view) fn diff_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

pub(in crate::view) fn diff_row_height(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_ROW_HEIGHT_PX, ui_scale_percent)
}

pub(in crate::view) fn diff_row_horizontal_padding(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_ROW_HORIZONTAL_PADDING_PX, ui_scale_percent)
}

pub(super) fn diff_gutter_total_width(ui_scale_percent: u32) -> Pixels {
    gutter_cell_total_width(
        diff_row_horizontal_padding(ui_scale_percent),
        ui_scale_percent,
    )
}

pub(in crate::view) fn diff_single_column_text_start(ui_scale_percent: u32) -> Pixels {
    diff_gutter_total_width(ui_scale_percent) + diff_row_horizontal_padding(ui_scale_percent)
}

pub(in crate::view) fn diff_inline_text_start(ui_scale_percent: u32) -> Pixels {
    diff_gutter_total_width(ui_scale_percent) * 2.0 + diff_row_horizontal_padding(ui_scale_percent)
}

fn gutter_cell_total_width(pad: Pixels, ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_GUTTER_BASE_WIDTH_PX, ui_scale_percent) + pad * 2.0
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
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let shaped = shaped_gutter_line(text, color, metrics, window);
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
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let shaped = shaped_gutter_line(text, color, metrics, window);
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
    window: &mut Window,
) -> gpui::ShapedLine {
    let mut style = diff_text_style(window);
    style.color = color.into();
    let key = {
        let mut hasher = FxHasher::default();
        text.as_ref().hash(&mut hasher);
        metrics.font_size.hash(&mut hasher);
        style.font_family.hash(&mut hasher);
        style.font_weight.hash(&mut hasher);
        color.r.to_bits().hash(&mut hasher);
        color.g.to_bits().hash(&mut hasher);
        color.b.to_bits().hash(&mut hasher);
        color.a.to_bits().hash(&mut hasher);
        hasher.finish()
    };

    let shaped = GUTTER_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned());
    shaped.unwrap_or_else(|| {
        let run = style.to_run(text.len());
        let shaped = window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None);

        GUTTER_TEXT_LAYOUT_CACHE.with(|cache| {
            cache.borrow_mut().put(key, shaped.clone());
        });

        shaped
    })
}

#[allow(clippy::too_many_arguments)]
fn paint_selectable_diff_text(
    view: &Entity<MainPaneView>,
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
    base_style.color = base_fg.into();
    base_style.white_space = gpui::WhiteSpace::Nowrap;
    base_style.text_overflow = None;

    let pad = px_2(window);
    let gutter_total = gutter_cell_total_width(pad, ui_scale_percent);
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
    let selection = view
        .read(cx)
        .diff_text_local_selection_range(visible_ix, region);

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
            build_streamed_diff_slice_styled_text(theme, spec, &slice_range);
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

    #[cfg(test)]
    record_diff_paint_for_tests(visible_ix, region, paint_text, paint_highlights, row_bg);
    #[cfg(not(test))]
    let _ = row_bg;

    if let Some(r) = selection {
        let (x0, x1) = if let Some(cell_width) = hitbox_cell_width {
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
        };

        if x1 > x0 {
            let color = view.read(cx).diff_text_selection_color();
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left() + x0, bounds.top()),
                    point(bounds.left() + x1, bounds.bottom()),
                ),
                color,
            ));
        }
    }

    let hitbox = DiffTextHitbox {
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
        streamed_ascii_monospace_cell_width: hitbox_cell_width,
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

    let _ = layout.paint_background(
        point(paint_x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
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
    base_fg.r.to_bits().hash(&mut hasher);
    base_fg.g.to_bits().hash(&mut hasher);
    base_fg.b.to_bits().hash(&mut hasher);
    base_fg.a.to_bits().hash(&mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
fn ensure_layout_cached(
    view: &Entity<MainPaneView>,
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
            .shape_line(text.clone(), metrics.font_size, &[run], None)
    } else {
        let runs = compute_runs(text.as_ref(), base_style, highlights);
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &runs, None)
    };
    (layout_key, shaped.clone(), Some(shaped))
}

fn compute_runs(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.len()),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

fn empty_highlights() -> HighlightSpans {
    static EMPTY: OnceLock<HighlightSpans> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(r: f32, g: f32, b: f32) -> gpui::Rgba {
        gpui::Rgba { r, g, b, a: 1.0 }
    }

    fn test_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    fn streamed_query_spec(
        raw_text: &str,
        query: &str,
        query_options: DiffSearchOptions,
    ) -> StreamedDiffTextPaintSpec {
        StreamedDiffTextPaintSpec {
            raw_text: gitcomet_core::file_diff::FileDiffLineText::from(raw_text),
            query: query.to_owned().into(),
            query_options,
            query_matcher: (!query.is_empty())
                .then(|| Arc::new(DiffSearchMatcher::new(query, query_options))),
            word_ranges: Arc::from(Vec::<Range<usize>>::new()),
            word_color: None,
            syntax: StreamedDiffTextSyntaxSource::None,
        }
    }

    fn highlight_ranges(styled: &CachedDiffStyledText) -> Vec<Range<usize>> {
        styled
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect()
    }

    #[test]
    fn diff_text_paint_payload_reveals_whitespace_markers() {
        let style = HighlightStyle::default();
        let styled = CachedDiffStyledText {
            text: "a b\t".into(),
            highlights: Arc::from(vec![(1..4, style)]),
            highlights_hash: 7,
            text_hash: 11,
        };

        let payload = diff_text_paint_payload(
            Some(&styled),
            None,
            Some("a b\t"),
            true,
            DiffTextRegion::Inline,
            None,
        );

        assert_eq!(payload.text.as_ref(), "a·b→↵");
        assert_eq!(payload.highlights[0].0, 1..7);
        assert_ne!(payload.text_hash, styled.text_hash);

        let offset_map = payload.offset_map.expect("reveal whitespace offset map");
        assert_eq!(offset_map.source_offset_for_display(0), 0);
        assert_eq!(offset_map.source_offset_for_display("a·".len()), 2);
        assert_eq!(offset_map.source_offset_for_display("a·b→".len()), 7);
        assert_eq!(offset_map.display_offset_for_source(2), "a·".len());
        assert_eq!(offset_map.display_offset_for_source(7), "a·b→".len());
    }

    #[test]
    fn diff_text_paint_payload_keeps_streamed_whitespace_rows_unmaterialized() {
        let raw = "a ".repeat((STREAMED_DIFF_TEXT_MIN_BYTES / 2).saturating_add(1));
        let spec = streamed_query_spec(raw.as_str(), "", DiffSearchOptions::default());

        assert!(should_stream_diff_text(Some(&spec)));

        let payload =
            diff_text_paint_payload(None, Some(&spec), None, true, DiffTextRegion::Inline, None);

        assert!(payload.text.is_empty());
        assert!(payload.highlights.is_empty());
        assert_ne!(payload.text_hash, 0);
        assert!(payload.offset_map.is_none());
    }

    #[test]
    fn row_bg_fill_bounds_overdraws_bottom_without_changing_origin_or_width() {
        let bounds = test_bounds(4.0, 8.0, 120.0, 20.0);
        let painted = row_bg_fill_bounds(bounds);

        assert_eq!(painted.origin, bounds.origin);
        assert_eq!(painted.size.width, bounds.size.width);
        assert_eq!(
            painted.size.height,
            bounds.size.height + px(DIFF_ROW_BACKGROUND_OVERDRAW_PX)
        );
    }

    #[test]
    fn diff_row_text_regions_single_only_hits_inside_text() {
        let regions =
            DiffRowTextRegions::single(DiffTextRegion::Inline, test_bounds(5.0, 5.0, 20.0, 10.0));

        assert_eq!(
            regions.region_at(point(px(10.0), px(10.0))),
            Some(DiffTextRegion::Inline)
        );
        assert_eq!(regions.region_at(point(px(1.0), px(10.0))), None);
    }

    #[test]
    fn diff_row_text_regions_split_maps_left_and_right_regions() {
        let regions = DiffRowTextRegions::split(
            test_bounds(0.0, 0.0, 40.0, 20.0),
            test_bounds(41.0, 0.0, 40.0, 20.0),
        );

        assert_eq!(
            regions.region_at(point(px(10.0), px(10.0))),
            Some(DiffTextRegion::SplitLeft)
        );
        assert_eq!(
            regions.region_at(point(px(60.0), px(10.0))),
            Some(DiffTextRegion::SplitRight)
        );
        assert_eq!(regions.region_at(point(px(40.5), px(10.0))), None);
    }

    #[test]
    fn inline_row_canvas_revision_key_tracks_rendered_payload() {
        let base = inline_row_canvas_revision_key(
            &"1".into(),
            &"2".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            11,
            17,
        );

        assert_eq!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"2".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                11,
                17,
            )
        );
        assert_ne!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"3".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                11,
                17,
            )
        );
        assert_ne!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"2".into(),
                rgba(1.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                11,
                17,
            )
        );
        assert_ne!(
            base,
            inline_row_canvas_revision_key(
                &"1".into(),
                &"2".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                12,
                17,
            )
        );
    }

    #[test]
    fn split_row_canvas_revision_key_tracks_both_sides() {
        let base = split_row_canvas_revision_key(
            &"10".into(),
            &"20".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            3,
            5,
            7,
            11,
        );

        assert_ne!(
            base,
            split_row_canvas_revision_key(
                &"10".into(),
                &"20".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                4,
                5,
                7,
                11,
            )
        );
        assert_ne!(
            base,
            split_row_canvas_revision_key(
                &"10".into(),
                &"20".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                3,
                5,
                7,
                11,
            )
        );
        assert_ne!(
            base,
            split_row_canvas_revision_key(
                &"10".into(),
                &"21".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                3,
                5,
                7,
                11,
            )
        );
    }

    #[test]
    fn patch_split_row_canvas_revision_key_tracks_line_number_and_style() {
        let base = patch_split_row_canvas_revision_key(
            &"42".into(),
            rgba(0.0, 0.0, 0.0),
            rgba(1.0, 1.0, 1.0),
            rgba(1.0, 1.0, 1.0),
            13,
            17,
        );

        assert_ne!(
            base,
            patch_split_row_canvas_revision_key(
                &"43".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                13,
                17,
            )
        );
        assert_ne!(
            base,
            patch_split_row_canvas_revision_key(
                &"42".into(),
                rgba(0.0, 1.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                13,
                17,
            )
        );
        assert_ne!(
            base,
            patch_split_row_canvas_revision_key(
                &"42".into(),
                rgba(0.0, 0.0, 0.0),
                rgba(1.0, 1.0, 1.0),
                rgba(1.0, 1.0, 1.0),
                14,
                17,
            )
        );
    }

    #[test]
    fn streamed_query_overlay_skips_whole_word_on_partial_slice() {
        let theme = AppTheme::gitcomet_dark();
        let spec = streamed_query_spec(
            "foo_suffix",
            "foo",
            DiffSearchOptions {
                whole_word: true,
                ..DiffSearchOptions::default()
            },
        );

        let (styled, _, resolved) = build_streamed_diff_slice_styled_text(theme, &spec, &(0..3));

        assert_eq!(resolved, 0..3);
        assert_eq!(styled.text.as_ref(), "foo");
        assert!(styled.highlights.is_empty());
    }

    #[test]
    fn streamed_query_overlay_skips_regex_anchor_on_partial_slice() {
        let theme = AppTheme::gitcomet_dark();
        let spec = streamed_query_spec(
            "prefixfoo suffix",
            r"^foo",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );

        let (styled, _, resolved) = build_streamed_diff_slice_styled_text(theme, &spec, &(6..9));

        assert_eq!(resolved, 6..9);
        assert_eq!(styled.text.as_ref(), "foo");
        assert!(styled.highlights.is_empty());
    }

    #[test]
    fn streamed_query_overlay_keeps_boundary_sensitive_matches_on_full_slice() {
        let theme = AppTheme::gitcomet_dark();
        let spec = streamed_query_spec(
            "foo suffix",
            r"^foo",
            DiffSearchOptions {
                regex: true,
                ..DiffSearchOptions::default()
            },
        );

        let (styled, _, resolved) =
            build_streamed_diff_slice_styled_text(theme, &spec, &(0.."foo suffix".len()));

        assert_eq!(resolved, 0.."foo suffix".len());
        assert_eq!(highlight_ranges(&styled), vec![0..3]);
    }
}
