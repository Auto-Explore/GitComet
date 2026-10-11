//! Compare the production connector optimization with the previous GPU paths.
use super::*;
use gpui::{AppContext, Context, HeadlessAppContext, Render, TestTextSystem, canvas};
use std::sync::Arc;

struct Connectors {
    optimized: bool,
    ui_scale: u32,
    display_scale: f32,
    offset: f32,
    color: gpui::Rgba,
}

impl Render for Connectors {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let optimized = self.optimized;
        let display_scale = self.display_scale;
        let ui_scale = self.ui_scale;
        let offset = self.offset;
        let color = self.color;
        canvas(
            |_, _, _| (),
            move |viewport, (), window, _| {
                window.paint_quad(fill(viewport, gpui::rgb(0xeeeeee)));
                let grid =
                    GraphGrid::for_scale(display_scale, ui_scale::UiScale::from_percent(ui_scale));
                // Express the same device-pixel geometry on TestPlatform's 2x
                // display. This covers native fractional display scales as well
                // as fractional endpoints that must keep their original path.
                let convert = |value: Pixels| value * (display_scale / window.scale_factor());
                let p = |x, y| {
                    point(
                        convert(grid.snap(px(x)) + px(offset / display_scale)),
                        convert(grid.snap(px(y)) + px(offset / display_scale)),
                    )
                };
                let stroke = convert(grid.stroke_width);
                let connectors = [
                    (p(10.0, 20.0), p(35.0, 20.0)),
                    (p(35.0, 30.0), p(10.0, 30.0)),
                    (p(45.0, 10.0), p(45.0, 40.0)),
                    (p(55.0, 40.0), p(55.0, 10.0)),
                    // Nearly collinear and diagonal strokes keep their endpoints.
                    (p(65.0, 10.0), p(65.25, 40.0)),
                    (p(75.0, 10.0), p(95.0, 40.0)),
                    // Cross fractional content-mask edges in both directions.
                    (p(1.0, 50.0), p(170.0, 50.0)),
                    (p(110.0, -5.0), p(110.0, 115.0)),
                ];
                window.with_content_mask(
                    Some(gpui::ContentMask {
                        bounds: Bounds::new(point(px(2.25), px(2.75)), size(px(150.0), px(100.0))),
                        ..Default::default()
                    }),
                    |window| {
                        for (from, to) in connectors {
                            if optimized {
                                paint_straight_connector(from, to, stroke, color, window);
                            } else {
                                let mut path = gpui::PathBuilder::stroke(stroke);
                                path.move_to(from);
                                path.line_to(to);
                                window.paint_path(path.build().unwrap(), color);
                            }
                        }
                    },
                );
            },
        )
        .size_full()
    }
}

#[test]
#[ignore = "requires native hardware WGPU rendering; run explicitly"]
fn pixel_aligned_connectors_match_original_gpu_paths() {
    let _guard = crate::test_support::lock_visual_test();
    let mut cx = HeadlessAppContext::with_platform(Arc::new(TestTextSystem), Arc::new(()), || {
        Some(Box::new(
            gpui_wgpu::WgpuHeadlessRenderer::new().expect("hardware renderer"),
        ))
    });
    let window = cx
        .open_window(size(px(180.0), px(120.0)), |_, cx| {
            cx.new(|_| Connectors {
                optimized: false,
                ui_scale: 100,
                display_scale: 1.0,
                offset: 0.0,
                color: gpui::rgb(0x336699),
            })
        })
        .unwrap();
    for display_scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        for ui_scale in [80, 100, 125, 150, 175, 200] {
            for offset in [0.0, 0.25, 0.5, 0.75] {
                for color in [gpui::rgb(0x336699), gpui::rgba(0x33669988)] {
                    let mut reference = None;
                    for optimized in [false, true] {
                        cx.update_window(window.into(), |view, window, cx| {
                            view.downcast::<Connectors>()
                                .unwrap()
                                .update(cx, |view, _| {
                                    *view = Connectors {
                                        optimized,
                                        ui_scale,
                                        display_scale,
                                        offset,
                                        color,
                                    };
                                });
                            window.refresh();
                            window.draw(cx).clear(cx);
                        })
                        .unwrap();
                        let pixels = cx.capture_screenshot(window.into()).unwrap();
                        if let Some(reference) = reference.as_ref() {
                            let reference: &image::RgbaImage = reference;
                            let max_difference = pixels
                                .as_raw()
                                .iter()
                                .zip(reference.as_raw())
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap();
                            if max_difference > 1 {
                                let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                                    .join(
                                        "../../target/performance/gpu-efficiency/connector-pixels",
                                    );
                                std::fs::create_dir_all(&directory).unwrap();
                                reference.save(directory.join("reference.png")).unwrap();
                                pixels.save(directory.join("candidate.png")).unwrap();
                            }
                            assert!(
                                max_difference <= 1,
                                "display={display_scale} ui={ui_scale} offset={offset} color={color:?}: max channel difference {max_difference}"
                            );
                        } else {
                            reference = Some(pixels);
                        }
                    }
                }
            }
        }
    }
}
