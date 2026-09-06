//! Text painting shared by the diff and conflict canvases.
//!
//! Each canvas keeps its own layout caches, cache keys, clipping, and
//! invalidation behavior; the pieces here are byte-identical helpers the two
//! rows modules would otherwise duplicate.

use super::*;
use gpui::{Bounds, HighlightStyle, Pixels, TextRun, TextStyle, Window};
use palette::IntoColor;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, OnceLock};

pub(super) const DIFF_FONT_SCALE: f32 = 0.80;

pub(super) type HighlightSpans = Arc<[(Range<usize>, HighlightStyle)]>;

#[derive(Clone, Copy, Debug)]
pub(super) struct LineMetrics {
    pub(super) font_size: Pixels,
    pub(super) line_height: Pixels,
}

pub(super) fn diff_text_style(window: &Window) -> TextStyle {
    let mut style = window.text_style();
    style.font_weight = FontWeight::NORMAL;
    style
}

pub(super) fn line_metrics(window: &Window) -> LineMetrics {
    line_metrics_scaled(window, 1.0)
}

/// Metrics at `extra_scale` times the base diff font size (1.0 = the regular
/// row text; the annotation "when" column uses a slightly smaller scale).
pub(super) fn line_metrics_scaled(window: &Window, extra_scale: f32) -> LineMetrics {
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

pub(super) fn center_text_y(bounds: Bounds<Pixels>, line_height: Pixels) -> Pixels {
    let extra = (bounds.size.height - line_height).max(px(0.0));
    bounds.top() + extra * 0.5
}

/// Shapes gutter text, keyed on text, metrics, family/weight, and color.
///
/// The caller supplies its own cache so each canvas keeps its cache keys and
/// capacities separate, and the resolved text style (`diff_text_style`),
/// computed once per paint closure: `window.text_style()` re-merges the style
/// stack on every call, which added up across the gutter cells of a frame.
pub(super) fn shaped_gutter_line(
    text: &SharedString,
    color: gpui::Rgba,
    metrics: LineMetrics,
    style: &TextStyle,
    cache: &RefCell<FxLruCache<u64, gpui::ShapedLine>>,
    window: &mut Window,
) -> gpui::ShapedLine {
    let key = {
        let mut hasher = FxHasher::default();
        text.as_ref().hash(&mut hasher);
        metrics.font_size.hash(&mut hasher);
        style.font_family.hash(&mut hasher);
        style.font_weight.hash(&mut hasher);
        color.red.to_bits().hash(&mut hasher);
        color.green.to_bits().hash(&mut hasher);
        color.blue.to_bits().hash(&mut hasher);
        color.alpha.to_bits().hash(&mut hasher);
        hasher.finish()
    };

    let shaped = cache.borrow_mut().get(&key).cloned();
    shaped.unwrap_or_else(|| {
        let mut run = style.to_run(text.len());
        run.color = color.into_color();
        let shaped = window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None);

        cache.borrow_mut().put(key, shaped.clone());

        shaped
    })
}

/// Keep a line-number gutter at the visible edge of a horizontally scrolled
/// column. The row itself still moves so its measured width and scrollbar
/// range remain unchanged.
pub(super) fn sticky_gutter_bounds(
    column_bounds: Bounds<Pixels>,
    clip_bounds: Bounds<Pixels>,
    pad: Pixels,
    gap: Pixels,
    line_no_width: Pixels,
) -> Bounds<Pixels> {
    let visible_column = column_bounds.intersect(&clip_bounds);
    let width = (pad + line_no_width + gap).min(visible_column.size.width);
    Bounds::new(
        point(visible_column.left(), column_bounds.top()),
        size(width.max(px(0.0)), column_bounds.size.height),
    )
}

/// Clip the moving source text at the pinned gutter without changing the
/// text's paint origin. This makes content pass behind the gutter instead of
/// shifting as the horizontal offset changes.
pub(super) fn text_clip_bounds_behind_gutter(
    text_bounds: Bounds<Pixels>,
    gutter_bounds: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let left = text_bounds.left().max(gutter_bounds.right());
    Bounds::new(
        point(left, text_bounds.top()),
        size(
            (text_bounds.right() - left).max(px(0.0)),
            text_bounds.size.height,
        ),
    )
}

/// Paint the vertical divider between the line-number gutter and the code.
/// Sits at the right edge of the number cell (before the gap), so it stays
/// pinned with the sticky gutter as the column scrolls horizontally.
pub(super) fn paint_gutter_divider(
    gutter_bounds: Bounds<Pixels>,
    pad: Pixels,
    line_no_width: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
) {
    let x = gutter_bounds.left() + pad + line_no_width;
    window.paint_quad(fill(
        Bounds::new(
            point(x, gutter_bounds.top()),
            size(px(1.0), gutter_bounds.size.height),
        ),
        color,
    ));
}

pub(super) fn compute_runs(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<TextRun> {
    crate::text_runs::text_runs_for_highlights(text, default_style, highlights)
}

pub(super) fn empty_highlights() -> HighlightSpans {
    static EMPTY: OnceLock<HighlightSpans> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}
