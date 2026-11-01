use gpui::{BorderStyle, Bounds, Pixels, Rgba, Window, point, px, size};

/// Whole device pixels, halves up and at least one. gpui rounds halves toward
/// zero, which thins a 1.5px stroke to 1px at 1x.
pub(super) fn device_px(value: Pixels, scale_factor: f32) -> Pixels {
    px((f32::from(value) * scale_factor).round().max(1.0) / scale_factor)
}

/// History's circles and pills are SDF quads. gpui snaps a quad's edges to
/// device pixels but keeps its radius, so a circle whose bounds fall between
/// pixels paints as a lens or a rounded square: snap first, then fit the radius.
pub(super) fn paint_rounded_rect(
    bounds: Bounds<Pixels>,
    radius: Pixels,
    background: Rgba,
    border_width: Pixels,
    border_color: Rgba,
    window: &mut Window,
) {
    let snapped = Bounds::from_corners(
        window.pixel_snap_point(bounds.origin),
        window.pixel_snap_point(bounds.bottom_right()),
    );
    if snapped.is_empty() {
        return;
    }
    let half = snapped.size.width.min(snapped.size.height) * 0.5;
    // A fully round shape stays fully round when snapping grows it.
    let radius = if radius * 2.0 >= bounds.size.width.min(bounds.size.height) {
        half
    } else {
        radius.max(px(0.0)).min(half)
    };
    window.paint_quad(
        gpui::PaintQuad::new(snapped)
            .corner_radii(radius)
            .background(background)
            .border_widths(border_width)
            .border_color(border_color)
            .border_style(BorderStyle::Solid),
    );
}

/// A circle takes a whole device-pixel diameter before its position snaps, so
/// it stays round and a dotted line keeps one dot size. Returns what it painted.
pub(super) fn paint_disc(
    bounds: Bounds<Pixels>,
    color: Rgba,
    window: &mut Window,
) -> Bounds<Pixels> {
    let diameter = device_px(bounds.size.width, window.scale_factor());
    let center = bounds.center();
    let disc = Bounds::new(
        window.pixel_snap_point(point(center.x - diameter * 0.5, center.y - diameter * 0.5)),
        size(diameter, diameter),
    );
    paint_rounded_rect(disc, diameter * 0.5, color, px(0.0), color, window);
    disc
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Styled;
    use std::rc::Rc;

    const DEVICE_SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

    struct PaintView(Rc<dyn Fn(&mut Window)>);

    impl gpui::Render for PaintView {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let paint = self.0.clone();
            gpui::canvas(|_, _, _| (), move |_, (), window, _| paint(window)).size_full()
        }
    }

    fn painted(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<PaintView>,
        scale_factor: f32,
        paint: impl Fn(&mut Window) + 'static,
    ) -> Vec<gpui::Quad> {
        cx.update(|window, app| {
            window.set_scale_factor(scale_factor);
            view.update(app, |view, cx| {
                view.0 = Rc::new(paint);
                cx.notify();
            });
            let _ = window.draw(app);
            window.painted_quads()
        })
    }

    fn assert_whole(value: f32, what: &str) {
        assert!((value - value.round()).abs() < 1e-3, "{what} = {value}");
    }

    /// A pill or circle whose bounds fall between device pixels must still be
    /// one quad whose radius fits its snapped size, or its arcs overlap.
    #[gpui::test]
    fn fractional_shapes_paint_quads_whose_radius_fits_the_snapped_size(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|_, _| PaintView(Rc::new(|_| {})));
        for scale_factor in DEVICE_SCALES {
            for (width, height) in [(21.43, 21.43), (7.0, 7.0), (60.75, 17.3)] {
                let quads = painted(cx, &view, scale_factor, move |window| {
                    paint_rounded_rect(
                        Bounds::new(point(px(40.3), px(20.45)), size(px(width), px(height))),
                        px(height * 0.5),
                        gpui::rgba(0x202020ff),
                        px(1.0),
                        gpui::rgba(0xabcdefff),
                        window,
                    );
                });
                assert_eq!(quads.len(), 1, "scale={scale_factor} size={width}x{height}");
                let quad = &quads[0];
                let bounds = quad.bounds;
                for (edge, what) in [
                    (bounds.left(), "left"),
                    (bounds.top(), "top"),
                    (bounds.right(), "right"),
                    (bounds.bottom(), "bottom"),
                ] {
                    assert_whole(edge.0, what);
                }
                let half = bounds.size.width.0.min(bounds.size.height.0) * 0.5;
                let radius = quad.corner_radii.top_left.0;
                assert!(
                    (radius - half).abs() < 1e-3,
                    "scale={scale_factor} size={width}x{height}: radius {radius} for half size {half}",
                );
            }
        }
    }

    /// A dotted leader's dots share one round device size wherever they land.
    #[gpui::test]
    fn dots_share_one_device_size_and_stay_round(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| PaintView(Rc::new(|_| {})));
        for (scale_factor, expected) in [(1.0, 2.0), (1.25, 2.0), (1.5, 2.0), (2.0, 3.0)] {
            let quads = painted(cx, &view, scale_factor, |window| {
                for index in 0..6 {
                    let dot = Bounds::new(
                        point(px(50.3 + index as f32 * 3.7), px(20.45)),
                        size(px(1.5), px(1.5)),
                    );
                    paint_disc(dot, gpui::rgba(0xabcdefff), window);
                }
            });
            assert_eq!(quads.len(), 6, "scale={scale_factor}");
            for quad in &quads {
                let bounds = quad.bounds;
                assert!(
                    (bounds.size.width.0 - expected).abs() < 1e-3,
                    "scale={scale_factor}"
                );
                assert!(
                    (bounds.size.height.0 - expected).abs() < 1e-3,
                    "scale={scale_factor}"
                );
                assert_whole(bounds.left().0, "dot left");
                assert_whole(bounds.top().0, "dot top");
                assert!((quad.corner_radii.top_left.0 * 2.0 - expected).abs() < 1e-3);
            }
        }
    }
}
