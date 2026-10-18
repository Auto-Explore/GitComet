use super::super::canvas_text::LineMetrics;
use super::super::canvas_text::line_metrics_scaled;
use super::super::diff_text::hash_rgba_bits as hash_rgba;
use super::super::*;
use super::AnnotArea;
use super::BlameColumnLayout;
use super::RowBlamePaint;
use super::diff_scaled_px;
use super::hash_shared_string;
use super::interaction::diff_canvas_click_target;
use super::interaction::install_blame_annotation_hover_handler;
use super::interaction::install_blame_annotation_mouse_handler;
use super::paint_centered_svg_icon;
use super::paint_gutter_text;
use gpui::App;
use gpui::Bounds;
use gpui::CursorStyle;
use gpui::Hitbox;
use gpui::HitboxBehavior;
use gpui::PaintQuad;
use gpui::Pixels;
use gpui::TextStyle;
use gpui::Window;
use gpui::point;
use gpui::px;
use gpui::size;
use palette::IntoColor;
use rustc_hash::FxHasher;
use std::hash::Hash;
use std::hash::Hasher;

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
pub(super) fn mix_blame_revision(
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

/// Paint a single line of text truncated to `max_width` with a trailing "…".
///
/// Goes through the shared truncation cache: the blame summary is repainted
/// every frame, and building a line wrapper, truncating and shaping it per
/// row per frame was the one uncached text path in this file.
#[allow(clippy::too_many_arguments)]
fn paint_truncated_text(
    text: &SharedString,
    x: Pixels,
    y: Pixels,
    max_width: Pixels,
    color: gpui::Rgba,
    metrics: LineMetrics,
    style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() || max_width <= px(0.0) {
        return;
    }
    let mut style = style.clone();
    style.color = color.into_color();
    style.font_size = metrics.font_size.into();
    style.line_height = metrics.line_height.into();
    let layout = crate::kit::text_truncation::shape_truncated_line_cached(
        window,
        cx,
        &style,
        text,
        Some(max_width),
        crate::kit::text_truncation::TextTruncationProfile::End,
        &[],
        None,
    );
    let _ = layout.shaped_line.paint(
        point(x, y),
        metrics.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

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
    style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(PaintQuad::new(layout.border).background(blame.border));

    if !blame.show_text {
        return;
    }

    paint_gutter_text(
        &blame.when,
        layout.when_x,
        y,
        text_color,
        when_metrics,
        style,
        window,
        cx,
    );
    paint_gutter_text(
        &blame.initials,
        layout.initials_x,
        y,
        text_color,
        metrics,
        style,
        window,
        cx,
    );
    window.paint_layer(layout.message, |window| {
        let message_color = if hovered == Some(AnnotArea::Message) {
            theme.colors.accent.foreground
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
            style,
            window,
            cx,
        );
        if hovered == Some(AnnotArea::Message) {
            let underline_y = y + metrics.line_height + px(0.5);
            window.paint_quad(
                PaintQuad::new(Bounds::new(
                    point(layout.message.left(), underline_y),
                    size(layout.message.size.width, px(1.0)),
                ))
                .background(theme.colors.accent.foreground),
            );
        }
    });

    // The "view file at parent commit" icon. Shown for committed lines whose
    // parent has the file, and for uncommitted ("Now") lines that carry a base
    // revision (the committed state before the local change).
    if prior_enabled {
        let icon_color = if hovered == Some(AnnotArea::PriorIcon) {
            theme.colors.accent.foreground
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
            crate::theme::with_alpha(theme.colors.accent.foreground, 0.7),
            ui_scale_percent,
            window,
        );
    } else if browse_enabled {
        let icon_color = if hovered == Some(AnnotArea::BrowseIcon) {
            theme.colors.accent.foreground
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
    window.paint_quad(
        PaintQuad::new(bounds)
            .background(color)
            .corner_radii(diameter * 0.5),
    );
}

/// Hitboxes for annotation sub-column interactive areas.
#[derive(Clone, Debug)]
pub(super) struct AnnotHitboxes {
    pub(super) message: Hitbox,
    pub(super) prior_icon: Hitbox,
    pub(super) browse_icon: Hitbox,
}

/// Paint the annotation column for one row: border bar, text, icons, and
/// hover effects. Installs click handlers and sets cursor + tooltip via hitboxes.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_blame_column(
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
    view: &Entity<DiffRenderer>,
    style: &TextStyle,
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
        theme.colors.foreground.secondary,
        theme,
        metrics,
        when_metrics,
        hovered,
        prior_enabled,
        browse_enabled,
        ui_scale_percent,
        style,
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
            diff_canvas_click_target(view, visible_ix, "blame", cx),
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

pub(super) fn build_annot_hitboxes(
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

/// Smaller font metrics for the "X ago" sub-column in the annotation panel.
pub(super) fn line_metrics_annot_when(window: &Window, theme: AppTheme) -> LineMetrics {
    line_metrics_scaled(window, theme, 0.85)
}
