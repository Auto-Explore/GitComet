use super::super::controls::viewport;
use super::*;
use crate::view::test_support::redraw;

struct ComparisonControls {
    group: Entity<Group>,
    count: usize,
    width: f32,
}

impl Render for ComparisonControls {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = AppTheme::gitcomet_dark();
        div()
            .w(px(self.width))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .children((0..self.count).map(|side| {
                        div()
                            .id(("comparison_pane", side))
                            .debug_selector(move || format!("comparison_pane_{side}"))
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .child(viewport(
                                self.group.clone(),
                                side,
                                theme,
                                cx,
                                |_, _, _, _, _| {},
                            ))
                    })),
            )
            .child(toolbar(self.group.clone(), theme, cx, |_, _, _, _, _| {}))
    }
}

fn comparison(
    cx: &mut gpui::TestAppContext,
    count: usize,
) -> (
    Entity<ComparisonControls>,
    Entity<Group>,
    &mut gpui::VisualTestContext,
) {
    let sources = [source(640, 480), source(320, 800), source(100, 100)];
    let group = cx.new(|cx| {
        let mut group = Group::new(cx);
        group.set_inputs(
            "comparison".into(),
            std::array::from_fn(|side| Input {
                source: (side < count).then(|| sources[side].clone()),
                ..Default::default()
            }),
            false,
            cx,
        );
        group
    });
    let (view, cx) = cx.add_window_view(|_, cx| {
        cx.observe(&group, |_, _, cx| cx.notify()).detach();
        ComparisonControls {
            group: group.clone(),
            count,
            width: 960.0,
        }
    });
    redraw(cx);
    (view, group, cx)
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Default::default());
    redraw(cx);
}

#[gpui::test]
fn linked_toolbar_controls_fit_narrow_panes(cx: &mut gpui::TestAppContext) {
    let (view, group, cx) = comparison(cx, 2);
    cx.update(|_, app| {
        // Include the extra animation control and a longer sampling label.
        app.set_global(Preferences {
            mode: ImageSamplingMode::Smooth,
            filter: ImageSmoothFilter::Robidoux,
        });
        group.update(app, |group, cx| {
            group.sides[0].input.preview = Some(Arc::new(gpui::RenderImage::new(vec![
                image::Frame::new(image::RgbaImage::new(16, 16)),
                image::Frame::new(image::RgbaImage::new(16, 16)),
            ])));
            cx.notify();
        });
    });
    for width in [576.0, 320.0, 220.0, 960.0] {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.width = width;
                cx.notify();
            });
        });
        redraw(cx);
        let strip = cx.debug_bounds("image_format_strip").unwrap();
        assert!(strip.left() >= px(0.0) && strip.right() <= px(width));
        for control in [
            "image_sampling",
            "image_zoom_out",
            "image_zoom",
            "image_zoom_in",
            "image_fit",
            "image_native",
            "image_link",
            "image_pause",
        ] {
            let bounds = cx.debug_bounds(control).unwrap();
            assert!(
                bounds.left() >= strip.left()
                    && bounds.right() <= strip.right()
                    && bounds.top() >= strip.top()
                    && bounds.bottom() <= strip.bottom(),
                "{control} must fit the {width}px toolbar: {bounds:?} vs {strip:?}"
            );
        }
        click(cx, "image_native");
        cx.update(|_, app| assert_eq!(group.read(app).zoom_label(), "100%"));
        click(cx, "image_fit");
        cx.update(|_, app| assert!(group.read(app).sides[0].camera.fit));
    }
}

#[gpui::test]
fn unlinked_controls_zoom_only_their_image_and_relink_to_the_last_adjustment(
    cx: &mut gpui::TestAppContext,
) {
    let (_, group, cx) = comparison(cx, 2);
    assert!(cx.debug_bounds("image_zoom").is_some());
    assert!(cx.debug_bounds("image_pane_controls_0").is_none());
    click(cx, "image_native");
    click(cx, "image_link");
    assert!(cx.debug_bounds("image_zoom").is_none());
    for side in 0..2 {
        let pane = cx
            .debug_bounds(format!("comparison_pane_{side}").leak())
            .unwrap();
        let controls = cx
            .debug_bounds(format!("image_pane_controls_{side}").leak())
            .unwrap();
        assert!(pane.contains(&controls.center()));
    }

    click(cx, "image_zoom_in_1");
    cx.update(|_, app| {
        let group = group.read(app);
        assert_eq!(group.zoom_label_for(0), "100%");
        assert_eq!(group.zoom_label_for(1), "125%");
        assert!(
            group.dragging.is_none(),
            "toolbar clicks must not start an image drag"
        );
    });
    click(cx, "image_zoom_out_0");
    cx.update(|_, app| {
        let group = group.read(app);
        assert_eq!(group.zoom_label_for(0), "75%");
        assert_eq!(group.zoom_label_for(1), "125%");
    });
    click(cx, "image_fit_0");
    cx.update(|_, app| {
        let group = group.read(app);
        assert!(group.sides[0].camera.fit);
        assert_eq!(group.zoom_label_for(1), "125%");
    });
    click(cx, "image_native_0");
    click(cx, "image_zoom_in_1");
    click(cx, "image_link");
    assert!(cx.debug_bounds("image_zoom").is_some());
    assert!(cx.debug_bounds("image_pane_controls_0").is_none());
    cx.update(|_, app| {
        let group = group.read(app);
        assert!(group.linked);
        assert_eq!(group.zoom_label_for(0), "150%");
        assert_eq!(group.zoom_label_for(1), "150%");
    });
}

#[gpui::test]
fn unlinked_zoom_menu_keeps_its_pane_and_does_not_apply_to_another_file(
    cx: &mut gpui::TestAppContext,
) {
    let (_, group, cx) = comparison(cx, 2);
    click(cx, "image_native");
    click(cx, "image_link");
    click(cx, "image_zoom_1");
    // Changing keyboard/pointer focus while a menu is open cannot retarget it.
    cx.update(|window, app| {
        group.update(app, |group, _| group.active = 0);
        apply_menu_zoom(Zoom::Scale(4.0), window, app);
        let group = group.read(app);
        assert_eq!(group.zoom_label_for(0), "100%");
        assert_eq!(group.zoom_label_for(1), "400%");
    });
    redraw(cx);
    click(cx, "image_zoom_0");
    cx.update(|window, app| {
        group.update(app, |group, cx| {
            let inputs = std::array::from_fn(|side| group.sides[side].input.clone());
            group.set_inputs("another file".into(), inputs, false, cx);
        });
        apply_menu_zoom(Zoom::Scale(8.0), window, app);
        assert!(group.read(app).sides.iter().all(|side| side.camera.fit));
    });
}

#[gpui::test]
fn three_way_controls_stay_in_their_panes_at_narrow_widths(cx: &mut gpui::TestAppContext) {
    let (view, group, cx) = comparison(cx, 3);
    click(cx, "image_native");
    click(cx, "image_link");
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.width = 660.0;
            cx.notify();
        });
    });
    redraw(cx);
    for side in 0..3 {
        let pane = cx
            .debug_bounds(format!("comparison_pane_{side}").leak())
            .unwrap();
        for control in [
            "image_zoom_out",
            "image_zoom",
            "image_zoom_in",
            "image_fit",
            "image_native",
        ] {
            let bounds = cx.debug_bounds(format!("{control}_{side}").leak()).unwrap();
            assert!(
                bounds.left() >= pane.left() && bounds.right() <= pane.right(),
                "{control}_{side} must fit its own pane: {bounds:?} vs {pane:?}"
            );
        }
    }
    click(cx, "image_zoom_in_2");
    cx.update(|_, app| {
        let group = group.read(app);
        assert_eq!(group.zoom_label_for(0), "100%");
        assert_eq!(group.zoom_label_for(1), "100%");
        assert_eq!(group.zoom_label_for(2), "125%");
    });
}
