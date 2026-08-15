//! kdiff3-style minimap column for the merge resolver.
//!
//! Port of kdiff3's `Overview` widget (`Overview.cpp`): a narrow column beside
//! the input panes that paints the whole file's change structure at a glance,
//! frames the current viewport, and jumps the panes when clicked.
//!
//! Band classification lives in `gitcomet_core::merge::minimap`; this module
//! only paints the bands and handles the mouse.

use crate::kit::scrollbar::{ScrollbarAxis, ScrollbarDriver};
use crate::theme::AppTheme;
use gitcomet_core::merge::MinimapRowKind;
use gpui::prelude::*;
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, ElementId, Hitbox, HitboxBehavior, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Window, canvas, div, fill, point, px,
    size,
};
use std::sync::Arc;

/// Column width, matching kdiff3's `setFixedWidth(20)`.
pub const MINIMAP_COLUMN_WIDTH_PX: f32 = 20.0;

/// Where the viewport sits within the whole content, as fractions in `[0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MinimapViewport {
    /// Fraction of the content above the viewport.
    pub start: f32,
    /// Fraction of the content the viewport covers.
    pub extent: f32,
}

impl MinimapViewport {
    /// Derive the viewport frame from a scroll driver and the column height.
    ///
    /// The column is laid out beside the lists at the same height, so its
    /// height is the lists' viewport height and the content height follows
    /// from the driver's maximum scroll offset.
    pub fn from_driver(driver: &dyn ScrollbarDriver, viewport_height: Pixels) -> Self {
        let axis = ScrollbarAxis::Vertical;
        let max_offset = driver.max_offset(axis).max(px(0.0));
        let content = viewport_height + max_offset;
        if content <= px(0.0) {
            return Self {
                start: 0.0,
                extent: 1.0,
            };
        }
        let raw = driver.raw_offset(axis);
        let scroll = if raw < px(0.0) { -raw } else { raw }.clamp(px(0.0), max_offset);
        Self {
            start: (scroll / content).clamp(0.0, 1.0),
            extent: (viewport_height / content).clamp(0.0, 1.0),
        }
    }
}

type JumpHandler = Arc<dyn Fn(f32, &mut Window, &mut App) + 'static>;

/// The minimap column element.
///
/// `bands` is the merge classification, one entry per quantized slice of the
/// file, painted across the full column width.
#[derive(Clone)]
pub struct MinimapColumn {
    id: ElementId,
    bands: Arc<[MinimapRowKind]>,
    driver: Option<Arc<dyn ScrollbarDriver>>,
    on_jump: Option<JumpHandler>,
}

#[derive(Default)]
struct MinimapInteractionState {
    dragging: bool,
}

impl MinimapColumn {
    pub fn new(id: impl Into<ElementId>, bands: Arc<[MinimapRowKind]>) -> Self {
        Self {
            id: id.into(),
            bands,
            driver: None,
            on_jump: None,
        }
    }

    /// Scroll model the viewport frame follows. The column is laid out at the
    /// panes' height, so its own bounds give the viewport extent.
    pub fn driver(mut self, driver: impl ScrollbarDriver) -> Self {
        self.driver = Some(Arc::new(driver));
        self
    }

    /// Called with the clicked position as a fraction of the content height.
    pub fn on_jump(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_jump = Some(Arc::new(handler));
        self
    }

    pub fn render(self, theme: AppTheme) -> impl IntoElement {
        let Self {
            id,
            bands,
            driver,
            on_jump,
        } = self;

        let state_id = id.clone();
        let paint = canvas(
            move |bounds, window, _cx| {
                window.insert_hitbox(bounds, HitboxBehavior::BlockMouseExceptScroll)
            },
            move |bounds, hitbox: Hitbox, window, cx| {
                let interaction = window.use_keyed_state(
                    (state_id.clone(), "minimap_interaction"),
                    cx,
                    |_window, _cx| MinimapInteractionState::default(),
                );

                window.paint_quad(fill(bounds, theme.colors.surface.panel));
                // kdiff3 rules each column off with a line; keep the width at
                // exactly 20px by painting the edge instead of using a border.
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.right() - px(1.0), bounds.top()),
                        size(px(1.0), bounds.size.height),
                    ),
                    theme.colors.stroke.default,
                ));

                paint_bands(
                    window,
                    theme,
                    bounds,
                    bounds.left(),
                    bounds.size.width,
                    &bands,
                );

                if let Some(driver) = driver.as_ref() {
                    let viewport =
                        MinimapViewport::from_driver(driver.as_ref(), bounds.size.height);
                    paint_viewport_frame(window, theme, bounds, viewport);
                }

                window.set_cursor_style(CursorStyle::Arrow, &hitbox);

                let Some(on_jump) = on_jump.clone() else {
                    return;
                };
                let height = bounds.size.height;
                let fraction_at = move |y: Pixels| {
                    if height <= px(0.0) {
                        return 0.0;
                    }
                    ((y - bounds.top()) / height).clamp(0.0, 1.0)
                };

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    let on_jump = on_jump.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        if !bounds.contains(&event.position) {
                            return;
                        }
                        interaction.update(cx, |state, _cx| state.dragging = true);
                        on_jump(fraction_at(event.position.y), window, cx);
                        window.refresh();
                        cx.stop_propagation();
                    }
                });

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    let on_jump = on_jump.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || !event.dragging() {
                            return;
                        }
                        if !interaction.read(cx).dragging {
                            return;
                        }
                        on_jump(fraction_at(event.position.y), window, cx);
                        window.refresh();
                        cx.stop_propagation();
                    }
                });

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    move |event: &MouseUpEvent, phase, _window, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        if interaction.read(cx).dragging {
                            interaction.update(cx, |state, _cx| state.dragging = false);
                        }
                    }
                });
            },
        )
        .size_full();

        div()
            .id(id)
            .h_full()
            .w(px(MINIMAP_COLUMN_WIDTH_PX))
            .flex_shrink_0()
            .child(paint)
    }
}

fn band_color(theme: AppTheme, kind: MinimapRowKind) -> Option<gpui::Rgba> {
    let changed_alpha = if theme.is_dark { 0.85 } else { 0.70 };
    // A settled conflict drops to a muted neutral, so the red that remains is
    // exactly the work left and it reads as neither side's change color.
    let resolved_alpha = if theme.is_dark { 0.45 } else { 0.35 };
    let (mut color, alpha) = match kind {
        MinimapRowKind::Unchanged => return None,
        MinimapRowKind::LocalChanged => (theme.colors.diff.added.foreground, changed_alpha),
        MinimapRowKind::RemoteChanged => (theme.colors.accent.foreground, changed_alpha),
        MinimapRowKind::ResolvedConflict => (theme.colors.foreground.secondary, resolved_alpha),
        MinimapRowKind::Conflict => (theme.colors.status.danger.foreground, changed_alpha),
    };
    color.alpha = alpha;
    Some(color)
}

/// Paint one classification column, coalescing runs of equal bands so the
/// number of quads follows the change count rather than the band count.
fn paint_bands(
    window: &mut Window,
    theme: AppTheme,
    bounds: Bounds<Pixels>,
    x: Pixels,
    width: Pixels,
    bands: &[MinimapRowKind],
) {
    let count = bands.len();
    if count == 0 || width <= px(0.0) {
        return;
    }
    let height = bounds.size.height;
    let band_height = height / count as f32;

    let mut ix = 0usize;
    while ix < count {
        let kind = bands[ix];
        let start = ix;
        ix += 1;
        while ix < count && bands[ix] == kind {
            ix += 1;
        }
        let Some(color) = band_color(theme, kind) else {
            continue;
        };
        let y0 = bounds.top() + band_height * start as f32;
        // Keep a single changed line visible on tall columns.
        let h = (band_height * (ix - start) as f32).max(px(1.0));
        window.paint_quad(fill(Bounds::new(point(x, y0), size(width, h)), color));
    }
}

/// Frame the current viewport, as kdiff3's `paintEvent` does with `drawRect`.
fn paint_viewport_frame(
    window: &mut Window,
    theme: AppTheme,
    bounds: Bounds<Pixels>,
    viewport: MinimapViewport,
) {
    if viewport.extent >= 1.0 {
        return;
    }
    let height = bounds.size.height;
    let y0 = bounds.top() + height * viewport.start;
    let frame_h = (height * viewport.extent).max(px(2.0));
    let y1 = (y0 + frame_h).min(bounds.bottom());
    let width = bounds.size.width;
    let mut edge = theme.colors.foreground.primary;
    edge.alpha = if theme.is_dark { 0.95 } else { 0.70 };
    let thickness = px(1.0);

    // A 1px rectangle around the page, as kdiff3's `drawRect` does, plus a
    // faint wash so a page that is only a few pixels tall still reads.
    let mut shade = theme.colors.foreground.primary;
    shade.alpha = if theme.is_dark { 0.14 } else { 0.09 };
    window.paint_quad(fill(
        Bounds::new(point(bounds.left(), y0), size(width, y1 - y0)),
        shade,
    ));
    for edge_bounds in [
        Bounds::new(point(bounds.left(), y0), size(width, thickness)),
        Bounds::new(point(bounds.left(), y1 - thickness), size(width, thickness)),
        Bounds::new(point(bounds.left(), y0), size(thickness, y1 - y0)),
        Bounds::new(
            point(bounds.right() - thickness, y0),
            size(thickness, y1 - y0),
        ),
    ] {
        window.paint_quad(fill(edge_bounds, edge));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedDriver {
        max: Pixels,
        raw: Pixels,
    }

    impl ScrollbarDriver for FixedDriver {
        fn max_offset(&self, _axis: ScrollbarAxis) -> Pixels {
            self.max
        }
        fn raw_offset(&self, _axis: ScrollbarAxis) -> Pixels {
            self.raw
        }
        fn set_axis_offset(&self, _axis: ScrollbarAxis, _offset: Pixels) {}
    }

    #[test]
    fn viewport_covers_everything_when_content_fits() {
        let driver = FixedDriver {
            max: px(0.0),
            raw: px(0.0),
        };
        let viewport = MinimapViewport::from_driver(&driver, px(400.0));
        assert_eq!(viewport.start, 0.0);
        assert_eq!(viewport.extent, 1.0);
    }

    #[test]
    fn viewport_tracks_scroll_position() {
        // 400px viewport over 1600px of content: a quarter of the file.
        let driver = FixedDriver {
            max: px(1200.0),
            raw: px(-600.0),
        };
        let viewport = MinimapViewport::from_driver(&driver, px(400.0));
        assert!((viewport.extent - 0.25).abs() < 1e-5);
        assert!((viewport.start - 0.375).abs() < 1e-5);
    }

    #[test]
    fn viewport_accepts_either_offset_sign() {
        let negative = MinimapViewport::from_driver(
            &FixedDriver {
                max: px(1200.0),
                raw: px(-1200.0),
            },
            px(400.0),
        );
        let positive = MinimapViewport::from_driver(
            &FixedDriver {
                max: px(1200.0),
                raw: px(1200.0),
            },
            px(400.0),
        );
        assert_eq!(negative, positive);
        assert!((negative.start - 0.75).abs() < 1e-5);
    }

    #[test]
    fn unchanged_bands_are_not_painted() {
        let theme = crate::theme::AppTheme::gitcomet_dark();
        assert!(band_color(theme, MinimapRowKind::Unchanged).is_none());
        assert!(band_color(theme, MinimapRowKind::Conflict).is_some());
        assert!(band_color(theme, MinimapRowKind::LocalChanged).is_some());
        assert!(band_color(theme, MinimapRowKind::RemoteChanged).is_some());
    }

    #[test]
    fn a_resolved_conflict_is_painted_but_not_in_the_conflict_color() {
        for theme in [
            crate::theme::AppTheme::gitcomet_dark(),
            crate::theme::AppTheme::gitcomet_light(),
        ] {
            let resolved = band_color(theme, MinimapRowKind::ResolvedConflict)
                .expect("a resolved conflict still marks its rows");
            let open =
                band_color(theme, MinimapRowKind::Conflict).expect("an open conflict paints");
            assert_ne!(
                (resolved.red, resolved.green, resolved.blue),
                (open.red, open.green, open.blue),
                "a resolved conflict must not keep the open-conflict hue",
            );
            assert!(
                resolved.alpha < open.alpha,
                "a resolved conflict should recede: {resolved:?} vs {open:?}",
            );
        }
    }
}
