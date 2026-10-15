use super::*;
use std::time::{Duration, Instant};

const ICON_DIAMETER_PX: f32 = 16.0;
const AUTHOR_DIAMETER_PX: f32 = 20.0;
const AUTHOR_DOT_DIAMETER_PX: f32 = 7.0;
const RING_PX: f32 = 1.5;

/// The node ring rounds half up (2px at 1x); chip borders keep gpui's
/// half-toward-zero like every other 1px border.
pub(super) fn ring_width(scale: ui_scale::UiScale, scale_factor: f32) -> Pixels {
    super::history_shapes::device_px(scale.px(RING_PX), scale_factor)
}

fn extra_inset(appearance: crate::appearance::Appearance) -> f32 {
    appearance.ramp(1.0, 2.0)
}

pub(super) fn icon_diameter(scale: ui_scale::UiScale) -> Pixels {
    scale.ui_text(ICON_DIAMETER_PX) + scale.px(extra_inset(scale.appearance) * 2.0)
}

pub(super) fn author_diameter(scale: ui_scale::UiScale, initials_reveal: f32) -> Pixels {
    let dot = scale.px(AUTHOR_DOT_DIAMETER_PX);
    let expanded =
        scale.ui_text(AUTHOR_DIAMETER_PX) + scale.px(extra_inset(scale.appearance) * 2.0);
    dot + (expanded - dot) * initials_reveal
}

pub(in crate::view) fn graph_font_growth(scale: ui_scale::UiScale) -> Pixels {
    (scale.ui_text(AUTHOR_DIAMETER_PX) - scale.px(AUTHOR_DIAMETER_PX)).max(px(0.0))
}

pub(in crate::view) fn graph_min_width(scale: ui_scale::UiScale) -> Pixels {
    scale.px(HISTORY_COL_GRAPH_MIN_PX) + graph_font_growth(scale)
}

const SELECTION_DURATION: Duration = Duration::from_millis(180);

#[derive(Clone, Copy, Debug)]
struct SelectionTransition {
    from: f32,
    selected: bool,
    started: Instant,
}

impl SelectionTransition {
    fn settled(selected: bool, now: Instant) -> Self {
        Self {
            from: if selected { 1.0 } else { 0.0 },
            selected,
            started: now,
        }
    }

    fn value(self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.started).as_secs_f32()
            / SELECTION_DURATION.as_secs_f32())
        .min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        let target = if self.selected { 1.0 } else { 0.0 };
        if t == 1.0 {
            target
        } else {
            self.from + (target - self.from) * eased
        }
    }

    fn update(&mut self, selected: bool, now: Instant, animate: bool) -> f32 {
        if !animate {
            *self = Self::settled(selected, now);
        } else if selected != self.selected {
            self.from = self.value(now);
            self.selected = selected;
            self.started = now;
        }
        self.value(now)
    }
}

/// State belongs to a visible node's stable identity. Scrolling a selected node
/// into view starts settled; only a change of selection animates, including a
/// reversal before the previous transition finishes. Like other pane
/// animations, it settles at once under reduced motion or a deterministic runtime.
pub(super) fn selection_progress(
    id: &gpui::GlobalElementId,
    selected: bool,
    window: &mut Window,
    cx: &App,
) -> f32 {
    let now = cx.background_executor().now();
    let animate = crate::ui_runtime::current().uses_pane_animations() && !cx.reduce_motion();
    window.with_element_state(id, |state: Option<SelectionTransition>, window| {
        let mut state = state.unwrap_or_else(|| SelectionTransition::settled(selected, now));
        let progress = state.update(selected, now, animate);
        if progress != if selected { 1.0 } else { 0.0 } {
            window.request_animation_frame();
        }
        (progress, state)
    })
}

#[derive(Clone, Copy)]
struct GlowLayer {
    bounds: Bounds<Pixels>,
    alpha: f32,
}

// A finite radial falloff keeps the halo visible in compact rows without
// squeezing most of a Gaussian into the first pixel or clipping its long tail.
// One ring per device pixel keeps every ring concentric with the even node.
fn selection_glow_layers(
    theme: AppTheme,
    bounds: Bounds<Pixels>,
    cell_bounds: Bounds<Pixels>,
    progress: f32,
    scale: ui_scale::UiScale,
    device_scale: f32,
) -> impl Iterator<Item = GlowLayer> {
    // Reserve a physical pixel for antialiasing/outward geometry rounding.
    let clearance = (bounds.left() - cell_bounds.left())
        .min(cell_bounds.right() - bounds.right())
        .min(bounds.top() - cell_bounds.top())
        .min(cell_bounds.bottom() - bounds.bottom())
        - px(1.0 / device_scale);
    let rings = if progress > 0.0 {
        (f32::from(scale.px(6.0).min(clearance)) * device_scale)
            .floor()
            .max(0.0) as usize
    } else {
        0
    };
    let strength = progress * if theme.is_dark { 0.65 } else { 0.42 };
    let mut previous_alpha = 0.0;
    (0..rings).map(move |ring| {
        let dilation = rings - ring;
        // Each ring sets the falloff at the middle of the pixel band inside its
        // outer edge, so the outermost band fades out.
        let t = 1.0 - (dilation as f32 - 0.5) / rings as f32;
        let target_alpha = strength * t * t * (3.0 - 2.0 * t);
        // Concentric translucent rings overlap. Compensate so their combined
        // opacity follows the smooth falloff rather than accumulating a rim.
        let alpha = (target_alpha - previous_alpha) / (1.0 - previous_alpha);
        previous_alpha = target_alpha;
        GlowLayer {
            bounds: bounds.dilate(px(dilation as f32 / device_scale)),
            alpha,
        }
    })
}

/// One soft lane-coloured halo for commit, merge, stash and change nodes.
/// It fades out inside the row and graph column; the node's interior stays
/// transparent so the halo cannot tint its fill or author glyph.
pub(super) fn paint_selection_glow(
    theme: AppTheme,
    bounds: Bounds<Pixels>,
    cell_bounds: Bounds<Pixels>,
    lane: gpui::Rgba,
    progress: f32,
    window: &mut Window,
) {
    // Fit the visible column width, but the row's full height: scrolling clips
    // the halo like the node instead of resizing it.
    let mask = window.content_mask().bounds;
    let left = cell_bounds.left().max(mask.left());
    let visible_cell = Bounds::from_corners(
        point(left, cell_bounds.top()),
        point(
            cell_bounds.right().min(mask.right()).max(left),
            cell_bounds.bottom(),
        ),
    );
    for layer in selection_glow_layers(
        theme,
        bounds,
        visible_cell,
        progress,
        ui_scale::UiScale::from_window(window),
        window.scale_factor(),
    ) {
        super::history_shapes::paint_rounded_rect(
            layer.bounds,
            layer.bounds.size.width * 0.5,
            // Match the transparent fill's RGB to the border: antialiasing
            // interpolates both, and transparent black leaves a dark fringe.
            with_alpha(lane, 0.0),
            (layer.bounds.size.width - bounds.size.width) * 0.5,
            with_alpha(lane, layer.alpha),
            window,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_glow_fits_inside_rows_and_pinned_columns() {
        use crate::appearance::{Appearance, UiDensity};

        let theme = AppTheme::gitcomet_dark();
        for density in UiDensity::ALL {
            for percent in [80, 100, 125, 150, 200] {
                let scale = ui_scale::UiScale::from_percent(percent).with_appearance(Appearance {
                    density,
                    ..Appearance::default()
                });
                for device_scale in [1.0, 1.25, 1.5, 2.0] {
                    for width in [HISTORY_COL_GRAPH_MIN_PX, HISTORY_COL_GRAPH_PX] {
                        let cell = Bounds::new(
                            point(px(40.25), px(20.5)),
                            size(
                                scale.px(width),
                                super::super::history::history_row_height(scale),
                            ),
                        );
                        for center_x in [
                            HISTORY_GRAPH_MARGIN_X_PX,
                            width - HISTORY_GRAPH_MARGIN_RIGHT_PX,
                        ] {
                            for diameter in [
                                scale.px(7.0),
                                icon_diameter(scale),
                                author_diameter(scale, 1.0),
                            ] {
                                let node = Bounds::new(
                                    point(
                                        cell.left() + scale.px(center_x) - diameter * 0.5,
                                        cell.center().y - diameter * 0.5,
                                    ),
                                    size(diameter, diameter),
                                );
                                let layers: Vec<_> = selection_glow_layers(
                                    theme,
                                    node,
                                    cell,
                                    1.0,
                                    scale,
                                    device_scale,
                                )
                                .collect();
                                assert!(!layers.is_empty());
                                for layer in layers {
                                    assert!(layer.alpha > 0.0 && layer.alpha < 1.0);
                                    // Whole device-pixel rings stay concentric once snapped.
                                    let ring = f32::from(layer.bounds.size.width - node.size.width)
                                        * 0.5
                                        * device_scale;
                                    assert!(
                                        ring >= 1.0 - 1e-3 && (ring - ring.round()).abs() < 1e-3
                                    );
                                    assert!(
                                        (f32::from(layer.bounds.center().x - node.center().x))
                                            .abs()
                                            < 1e-3
                                    );
                                    let tail = layer.bounds.dilate(px(1.0 / device_scale));
                                    let epsilon = px(0.001);
                                    assert!(tail.left() >= cell.left() - epsilon);
                                    assert!(tail.right() <= cell.right() + epsilon);
                                    assert!(tail.top() >= cell.top() - epsilon);
                                    assert!(tail.bottom() <= cell.bottom() + epsilon);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    struct ClippedGlowView {
        mask_top: Pixels,
    }

    impl Render for ClippedGlowView {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let mask_top = self.mask_top;
            gpui::canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let cell = Bounds::new(point(px(10.0), px(10.0)), size(px(40.0), px(28.0)));
                    let node = Bounds::new(point(px(22.0), px(16.0)), size(px(16.0), px(16.0)));
                    let mask = Bounds::new(point(px(0.0), mask_top), size(px(200.0), px(200.0)));
                    let mask = gpui::ContentMask {
                        bounds: mask,
                        ..Default::default()
                    };
                    window.with_content_mask(Some(mask), |window| {
                        paint_selection_glow(
                            AppTheme::gitcomet_dark(),
                            node,
                            cell,
                            gpui::rgba(0x3b82f6ff),
                            1.0,
                            window,
                        );
                    });
                },
            )
            .size_full()
        }
    }

    /// A row half scrolled out of the list keeps the halo it had, clipped like
    /// its node, instead of dropping it or shrinking it frame by frame.
    #[gpui::test]
    fn history_glow_is_clipped_not_resized_by_scrolling(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| ClippedGlowView { mask_top: px(0.0) });
        let rings = |cx: &mut gpui::VisualTestContext, mask_top: f32| {
            cx.update(|window, app| {
                view.update(app, |view, cx| {
                    view.mask_top = px(mask_top);
                    cx.notify();
                });
                let _ = window.draw(app);
                window
                    .painted_quads()
                    .iter()
                    .map(|quad| quad.bounds)
                    .collect::<Vec<_>>()
            })
        };
        let visible = rings(cx, 0.0);
        // 5.5px of clearance at the test window's 2x is 11 device-pixel rings.
        assert_eq!(visible.len(), 11, "every computed ring is painted");
        for mask_top in [12.0, 20.0, 30.0] {
            assert_eq!(rings(cx, mask_top), visible, "mask top at {mask_top}px");
        }
    }

    #[test]
    fn compact_history_glow_has_a_visible_soft_falloff() {
        let node = Bounds::new(point(px(5.0), px(3.0)), size(px(22.0), px(22.0)));
        let layers = selection_glow_layers(
            AppTheme::gitcomet_dark(),
            node,
            Bounds::new(point(px(0.0), px(0.0)), size(px(34.0), px(28.0))),
            1.0,
            ui_scale::UiScale::from_percent(100),
            1.0,
        )
        .collect::<Vec<_>>();
        let opacity = 1.0
            - layers
                .iter()
                .map(|layer| 1.0 - layer.alpha)
                .product::<f32>();
        assert!(opacity > 0.3, "halo stays visible beside the node");
        assert!(layers[0].alpha < opacity / 2.0, "outer edge fades");
    }

    /// Six device pixels of room is a 1x Comfortable row: its outermost ring
    /// must fade out instead of ending the halo on a visible step.
    #[test]
    fn history_glow_outer_ring_fades_out() {
        let node = Bounds::new(point(px(7.0), px(7.0)), size(px(24.0), px(24.0)));
        let layers = selection_glow_layers(
            AppTheme::gitcomet_dark(),
            node,
            Bounds::new(point(px(0.0), px(0.0)), size(px(38.0), px(38.0))),
            1.0,
            ui_scale::UiScale::from_percent(100),
            1.0,
        )
        .collect::<Vec<_>>();
        assert_eq!(layers.len(), 6);
        assert!(
            layers[0].alpha < 0.02,
            "outer ring alpha {}",
            layers[0].alpha
        );
    }

    #[test]
    fn history_glow_skips_unselected_and_fully_clipped_nodes() {
        let theme = AppTheme::gitcomet_dark();
        let node = Bounds::new(point(px(10.0), px(10.0)), size(px(20.0), px(20.0)));
        let scale = ui_scale::UiScale::from_percent(100);
        assert!(
            selection_glow_layers(theme, node, node.dilate(px(8.0)), 0.0, scale, 1.0)
                .next()
                .is_none()
        );
        assert!(
            selection_glow_layers(theme, node, node, 1.0, scale, 1.0)
                .next()
                .is_none()
        );
    }

    #[test]
    fn selection_transition_finishes_and_reverses_without_jumping() {
        let now = Instant::now();
        let mut state = SelectionTransition::settled(false, now);
        assert_eq!(state.update(true, now, true), 0.0);
        let halfway = now + SELECTION_DURATION / 2;
        let expanded = state.update(true, halfway, true);
        assert!(expanded > 0.5 && expanded < 1.0);
        assert_eq!(state.update(false, halfway, true), expanded);
        assert_eq!(state.update(false, halfway + SELECTION_DURATION, true), 0.0);
        state.update(true, halfway + SELECTION_DURATION, true);
        assert_eq!(
            state.update(true, halfway + SELECTION_DURATION * 2, true),
            1.0
        );
    }

    #[test]
    fn reduced_motion_and_newly_visible_nodes_start_settled() {
        let now = Instant::now();
        let mut state = SelectionTransition::settled(true, now);
        assert_eq!(state.value(now), 1.0);
        assert_eq!(state.update(false, now, false), 0.0);
        assert_eq!(state.update(true, now, false), 1.0);
    }
}
