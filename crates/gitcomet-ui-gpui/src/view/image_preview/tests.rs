use super::*;
use std::io::Cursor;

mod controls;
mod painting;

pub(super) fn source(width: u32, height: u32) -> Arc<Source> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 81, 255])
    });
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    Source::new(gpui::ImageFormat::Png, bytes.get_ref(), 512 * 1024 * 1024).unwrap()
}

fn inputs(a: &Arc<Source>, b: &Arc<Source>) -> [Input; 3] {
    [
        Input {
            source: Some(a.clone()),
            ..Default::default()
        },
        Input {
            source: Some(b.clone()),
            ..Default::default()
        },
        Input::default(),
    ]
}

#[gpui::test]
fn linked_cameras_share_source_coordinates_and_refresh_preserves_zoom(
    cx: &mut gpui::TestAppContext,
) {
    let a = source(200, 100);
    let b = source(100, 200);
    let group = cx.new(Group::new);
    group.update(cx, |group, cx| {
        group.set_inputs("first".into(), inputs(&a, &b), false, cx);
        group.sides[0].device_size = (400.0, 300.0);
        group.sides[1].device_size = (400.0, 300.0);
        group.layout(0);
        assert_eq!(group.sides[0].camera.scale, 1.5);
        assert_eq!(group.sides[1].camera.origin((400.0, 300.0)), (50.0, 0.0));
        group.toggle_linked(cx);
        group.zoom(Zoom::Fit, None, cx);
        assert_eq!(group.sides[0].camera.scale, 2.0);
        group.toggle_linked(cx);
        assert_eq!(
            group.sides[1].camera.scale, 2.0,
            "relink adopts the fitted pane's actual zoom"
        );
        group.zoom(Zoom::Scale(4.0), None, cx);
        group.pan(0, (-60.0, -80.0), cx);
        assert_eq!(group.sides[0].camera.center, group.sides[1].camera.center);
        group.toggle_linked(cx);
        let other = group.sides[1].camera;
        group.zoom(Zoom::Scale(8.0), None, cx);
        assert_eq!(group.sides[1].camera.scale, other.scale);
        group.toggle_linked(cx);
        assert_eq!(group.sides[1].camera.scale, 8.0);
        group.set_inputs("first".into(), inputs(&b, &a), false, cx);
        assert_eq!(
            group.sides[0].camera.scale, 8.0,
            "same target refresh keeps the camera"
        );
        group.set_inputs("second".into(), inputs(&a, &b), false, cx);
        assert!(group.linked && group.sides.iter().all(|side| side.camera.fit));
    });
}

#[gpui::test]
fn preferences_initialize_once_and_notify_all_window_groups(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        Preferences::initialize(
            &session::UiSession {
                image_sampling_mode: Some("smooth".into()),
                image_smooth_filter: Some("hann".into()),
                ..Default::default()
            },
            cx,
        )
    });
    let first = cx.new(Group::new);
    let second = cx.new(Group::new);
    cx.update(|cx| {
        Preferences::initialize(&session::UiSession::default(), cx);
        assert_eq!(
            Preferences::current(cx),
            Preferences {
                mode: ImageSamplingMode::Smooth,
                filter: ImageSmoothFilter::Hann
            }
        );
        cx.set_global(Preferences {
            mode: ImageSamplingMode::Pixels,
            filter: ImageSmoothFilter::Robidoux,
        });
    });
    cx.run_until_parked();
    for group in [first, second] {
        group.read_with(cx, |group, _| {
            assert!(group.sides[0].generation.load(Ordering::Relaxed) > 0)
        });
    }
    assert_eq!(
        ImageSamplingMode::parse(Some("future")),
        ImageSamplingMode::Auto
    );
    assert_eq!(
        ImageSmoothFilter::parse(Some("future")),
        ImageSmoothFilter::Spline36
    );
}

#[test]
fn paused_animation_decodes_only_selected_composited_frame_and_cancels() {
    let mut bytes = Vec::new();
    let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
    image::codecs::gif::GifEncoder::new(&mut bytes)
        .encode_frames(
            colors.map(|color| {
                image::Frame::new(image::RgbaImage::from_pixel(9, 7, image::Rgba(color)))
            }),
        )
        .unwrap();
    let source = Source::new(gpui::ImageFormat::Gif, &bytes, 128 * 1024 * 1024).unwrap();
    let generation = AtomicU64::new(1);
    let source::Decoded::Raster(frame) = source.decode(1, &generation, 1).unwrap().unwrap() else {
        panic!()
    };
    assert_eq!(frame.dimensions(), (9, 7));
    assert_eq!(frame.get_pixel(0, 0).0, colors[1]);
    assert!(source.decode(2, &generation, 0).unwrap().is_none());
}

#[test]
fn original_decoding_applies_orientation_and_limits() {
    use image::ImageEncoder as _;
    let exif = vec![
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder.set_exif_metadata(exif).unwrap();
    encoder
        .write_image(
            &[255, 0, 0, 255, 0, 0, 255, 255],
            2,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let source = Source::new(gpui::ImageFormat::Png, &bytes, 1024 * 1024).unwrap();
    assert_eq!(source.dimensions, (1, 2));
    let source::Decoded::Raster(image) = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap()
    else {
        panic!()
    };
    assert_eq!(image.dimensions(), (1, 2));
    assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert!(Source::new(gpui::ImageFormat::Png, &bytes, 1).is_none());
}

#[test]
fn adjacent_tiles_equal_whole_image_including_fractional_scale_and_edges() {
    let image = image::RgbaImage::from_fn(413, 297, |x, y| {
        image::Rgba([
            (x % 256) as u8,
            (y % 256) as u8,
            ((x * y) % 256) as u8,
            if x % 17 == 0 { 0 } else { 191 },
        ])
    });
    for filter in ImageSmoothFilter::ALL {
        let sampling = filter::Sampling::Smooth(*filter);
        let output = (997, 717);
        let whole = resample::raster_region(&image, output, (0, 0, 997, 717), sampling).unwrap();
        for (x, y, w, h) in [
            (0, 0, 512, 512),
            (512, 0, 485, 512),
            (0, 512, 512, 205),
            (512, 512, 485, 205),
        ] {
            let tile = resample::raster_region(&image, output, (x, y, w, h), sampling).unwrap();
            for j in 0..h {
                for i in 0..w {
                    for (a, b) in tile
                        .get_pixel(i, j)
                        .0
                        .into_iter()
                        .zip(whole.get_pixel(x + i, y + j).0)
                    {
                        assert!(
                            a.abs_diff(b) <= 1,
                            "{filter:?} seam at {},{}: {a} vs {b}",
                            x + i,
                            y + j
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn downscaling_widens_support_to_avoid_checkerboard_aliasing() {
    let image = image::RgbaImage::from_fn(400, 400, |x, y| {
        let c = if (x + y) % 2 == 0 { 0 } else { 255 };
        image::Rgba([c, c, c, 255])
    });
    for filter in ImageSmoothFilter::ALL {
        let result = resample::raster_region(
            &image,
            (4, 4),
            (0, 0, 4, 4),
            filter::Sampling::Smooth(*filter),
        )
        .unwrap();
        assert!(result.pixels().all(|p| p[0].abs_diff(128) <= 1));
    }
}

#[test]
fn svg_rerasterization_has_white_background_and_continuous_tiles() {
    let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100"><path d="M0 0L200 100" stroke="#ff0000" stroke-width="1"/></svg>"##;
    let source = Source::new(gpui::ImageFormat::Svg, bytes, 1024 * 1024).unwrap();
    let decoded = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap();
    let left = resample::tile(&decoded, (800, 400), 0, 0, filter::Sampling::Nearest).unwrap();
    let right = resample::tile(&decoded, (800, 400), 1, 0, filter::Sampling::Nearest).unwrap();
    assert_eq!(left.get_pixel(511, 100).0, [255, 255, 255, 255]);
    assert_eq!(right.get_pixel(0, 100).0, [255, 255, 255, 255]);
    assert_eq!(left.get_pixel(511, 255).0[2], 255);
    assert_eq!(right.get_pixel(0, 256).0[2], 255);
}

fn text_svg(family: &str) -> Vec<u8> {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80">
        <text x="10" y="55" font-family="{family}" font-size="40">Labels</text></svg>"#
    )
    .into_bytes()
}

#[test]
fn svg_text_survives_detailed_image_preview() {
    for family in [
        "IBM Plex Sans",
        "Lilex",
        "sans-serif",
        "monospace",
        "system-ui",
    ] {
        let bytes = text_svg(family);
        let source = Source::new(gpui::ImageFormat::Svg, &bytes, 1024 * 1024).unwrap();
        let decoded = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap();
        let pixels = resample::tile(&decoded, (240, 80), 0, 0, filter::Sampling::Nearest).unwrap();
        assert!(
            pixels.pixels().filter(|p| p[0] < 128).count() > 100,
            "SVG text disappeared with font-family={family}"
        );
    }
}

#[test]
fn svg_text_survives_bounded_image_preview() {
    for family in [
        "IBM Plex Sans",
        "Lilex",
        "sans-serif",
        "monospace",
        "system-ui",
    ] {
        let tree = svg::parse(&text_svg(family)).unwrap();
        let preview = panes::main::diff_cache::render_svg_image_diff_preview(&tree).unwrap();
        assert!(
            preview
                .as_bytes(0)
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] < 128)
                .count()
                > 100,
            "bounded SVG preview lost font-family={family}"
        );
    }
}

pub(super) const OFFSET_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1100" height="300">
            <defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="1100" height="300">
            <feOffset dx="1000"/></filter></defs>
            <rect y="80" width="100" height="100" fill="#ef2010" filter="url(#f)"/>
        </svg>"##;

#[test]
fn filtered_svg_image_preview_tiles_match_whole_rendering() {
    let fixtures = [
        (OFFSET_SVG, (1100, 300)),
        (
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="180">
            <defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="400" height="180">
            <feGaussianBlur stdDeviation="12"/><feOffset dx="80" dy="20"/></filter></defs>
            <g opacity="0.8"><rect x="40" y="30" width="100" height="80" fill="#2010ef" filter="url(#f)"/></g>
        </svg>"##,
            (1200, 540),
        ),
    ];
    for (svg, output) in fixtures {
        let source = Source::new(gpui::ImageFormat::Svg, svg.as_bytes(), 1024 * 1024).unwrap();
        let decoded = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap();
        let source::Decoded::Vector(tree, _) = &decoded else {
            panic!()
        };
        let mut whole = resvg::tiny_skia::Pixmap::new(output.0, output.1).unwrap();
        whole.fill(resvg::tiny_skia::Color::WHITE);
        resvg::render(
            tree,
            resvg::tiny_skia::Transform::from_scale(
                output.0 as f32 / tree.size().width(),
                output.1 as f32 / tree.size().height(),
            ),
            &mut whole.as_mut(),
        );
        for row in 0..output.1.div_ceil(resample::TILE) {
            for column in 0..output.0.div_ceil(resample::TILE) {
                let tile = resample::tile(&decoded, output, column, row, filter::Sampling::Nearest)
                    .unwrap();
                for (x, y, pixel) in tile.enumerate_pixels() {
                    let x = column * resample::TILE + x;
                    let y = row * resample::TILE + y;
                    let expected = whole.pixel(x, y).unwrap();
                    for (actual, expected) in pixel.0.into_iter().zip([
                        expected.blue(),
                        expected.green(),
                        expected.red(),
                        expected.alpha(),
                    ]) {
                        assert!(
                            actual.abs_diff(expected) <= 1,
                            "filtered SVG tile differs at ({x}, {y}): {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn unfiltered_svg_image_preview_keeps_detail_at_large_zoom() {
    let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1100" height="300">
        <rect width="1100" height="300" fill="#ef2010"/></svg>"##;
    let source = Source::new(gpui::ImageFormat::Svg, bytes, 1024 * 1024).unwrap();
    let decoded = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap();
    let tile =
        resample::tile(&decoded, (70400, 19200), 137, 37, filter::Sampling::Nearest).unwrap();
    assert!(tile.pixels().all(|p| p.0 == [16, 32, 239, 255]));
}

struct Harness {
    group: Entity<Group>,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(ImageViewport {
            group: self.group.clone(),
            side: 0,
            theme: AppTheme::gitcomet_dark(),
        })
    }
}

#[gpui::test]
fn image_preview_regression_svg_conflict_keeps_encoded_fallback(cx: &mut gpui::TestAppContext) {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1100" height="300">
        <defs><filter id="blur"><feGaussianBlur stdDeviation="2"/></filter></defs>
        <rect width="1100" height="300" fill="#c86432" filter="url(#blur)"/>
    </svg>"##;
    let source = Source::new(gpui::ImageFormat::Svg, svg, 128 * 1024 * 1024).unwrap();
    let decoded = source.decode(0, &AtomicU64::new(0), 0).unwrap().unwrap();
    assert!(
        decoded.for_output((4400, 1200)).is_err(),
        "400% exceeds the SVG detail limit"
    );
    let encoded = Arc::new(gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        crate::view::diff_utils::rasterize_svg_preview_png(svg).unwrap(),
    ));
    let expected = image::load_from_memory(encoded.bytes()).unwrap().to_rgba8();
    let mut expected_center = expected.get_pixel(550, 150).0;
    expected_center.swap(0, 2);
    assert!(expected_center[0] < 128 && expected_center[3] == 255);
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    // Let the encoded asset finish loading before checking the handoff to
    // the image viewport, so a missing fallback fails deterministically.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let ready =
            cx.update(|window, app| encoded.clone().get_render_image(window, app).is_some());
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "encoded preview did not load"
        );
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let group = cx.update(|window, app| {
        root.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.conflict_resolver.image_preview.sources[0] = Some(source);
            pane.conflict_resolver.image_preview.images.base =
                Loadable::Ready(Some(ConflictPreviewImage::Encoded(encoded)));
            pane.prepare_conflict_image_preview(window, cx)
        })
    });
    cx.update(|window, app| {
        window.replace_root(app, |_, _| Harness {
            group: group.clone(),
        });
        group.update(app, |group, cx| group.zoom(Zoom::Scale(4.0), None, cx));
        let preview = group.read(app).sides[0].input.preview.as_ref().expect(
            "encoded SVG conflict preview must remain renderable when detail rendering fails",
        );
        assert_eq!(
            preview.size(0),
            gpui::size(gpui::DevicePixels(1100), gpui::DevicePixels(300))
        );
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        crate::view::test_support::redraw(cx);
        cx.run_until_parked();
        if cx.update(|_, app| group.read(app).sides[0].error.is_some()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "SVG detail request did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.update(|_, app| {
        let side = &group.read(app).sides[0];
        assert!(
            side.error
                .as_ref()
                .unwrap()
                .contains("detailed preview limit")
        );
        let preview = side
            .input
            .preview
            .as_ref()
            .expect("fallback survives the detail error");
        let center = (150 * 1100 + 550) * 4;
        assert_eq!(
            &preview.as_bytes(0).unwrap()[center..center + 4],
            &expected_center,
            "fallback preserves the encoded, filtered preview pixels"
        );
    });
}

#[gpui::test]
fn viewport_wheel_zoom_pan_drag_and_keyboard(cx: &mut gpui::TestAppContext) {
    let input = source(1000, 1000);
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        group: cx.new(|cx| {
            let mut group = Group::new(cx);
            group.set_inputs(
                "image".into(),
                [
                    Input {
                        source: Some(input),
                        ..Default::default()
                    },
                    Input::default(),
                    Input::default(),
                ],
                false,
                cx,
            );
            group
        }),
    });
    crate::view::test_support::redraw(cx);
    let group = cx.update(|_, app| view.read(app).group.clone());
    let bounds = cx.debug_bounds("image_viewport").unwrap();
    cx.update(|_, app| group.update(app, |group, cx| group.zoom(Zoom::Scale(4.0), None, cx)));
    crate::view::test_support::redraw(cx);
    let before = cx.update(|_, app| group.read(app).sides[0].camera.center);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-60.0))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|_, app| {
        let camera = group.read(app).sides[0].camera;
        assert_ne!(camera.center, before);
        assert_eq!(camera.scale, 4.0);
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(60.0))),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|_, app| assert_eq!(group.read(app).sides[0].camera.scale, 8.0));
    let outside = bounds.bottom_right() + gpui::point(px(30.0), px(30.0));
    cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(outside, Some(gpui::MouseButton::Left), Default::default());
    cx.simulate_mouse_up(outside, gpui::MouseButton::Left, Default::default());
    cx.update(|_, app| assert!(group.read(app).dragging.is_none()));
    cx.simulate_keystrokes("1");
    cx.update(|_, app| assert_eq!(group.read(app).sides[0].camera.scale, 1.0));
    cx.simulate_keystrokes("0");
    cx.update(|_, app| assert!(group.read(app).sides[0].camera.fit));
}

/// rustc-hash 2.x `hash_bytes`: a 16-byte block whose first word equals the
/// running state multiplies by zero, so its second word never reaches the hash.
fn fx_colliding_ppm(fill: u8) -> Vec<u8> {
    let mut bytes = b"P6\n4 4\n255\n".to_vec();
    bytes.extend([128; 48]);
    bytes[16..24].copy_from_slice(&0x13198a2e03707344u64.to_le_bytes());
    bytes[24..32].fill(fill);
    bytes
}

/// Source ids were unseeded FxHash of the file bytes, so two crafted images
/// shared one id and the cache showed the first one's pixels for both.
#[test]
fn crafted_hash_collision_keeps_each_images_pixels() {
    let black = Source::new(gpui::ImageFormat::Pnm, &fx_colliding_ppm(0), 1 << 20).unwrap();
    let white = Source::new(gpui::ImageFormat::Pnm, &fx_colliding_ppm(255), 1 << 20).unwrap();
    let cache = Cache::default();
    cache.show(&[], &[black.id, white.id]);
    let key = |source: &Source| TileKey {
        source: source.id,
        frame: 0,
        output: (4, 4),
        sampling: filter::Sampling::Nearest,
        column: 0,
        row: 0,
    };
    for source in [&black, &white] {
        cache
            .render(source, &[key(source)], &AtomicU64::new(0), 0)
            .unwrap();
    }
    // Pixel (1, 1) lies in the block that the hash ignores.
    let tile = cache.peek(&key(&white)).unwrap();
    let pixel = &tile.as_bytes(0).unwrap()[(4 + 1) * 4..(4 + 1) * 4 + 3];
    assert_eq!(
        pixel,
        [255, 255, 255],
        "white image shows another image's pixels"
    );
}

/// Any wheel pan left Fit, even when the clamp cancelled it, so later
/// resizes stopped refitting the image.
#[gpui::test]
fn clamped_pan_keeps_fit(cx: &mut gpui::TestAppContext) {
    let image = source(200, 100);
    let group = cx.new(Group::new);
    group.update(cx, |group, cx| {
        group.set_inputs(
            "fit".into(),
            [
                Input {
                    source: Some(image),
                    ..Default::default()
                },
                Input::default(),
                Input::default(),
            ],
            false,
            cx,
        );
        group.sides[0].device_size = (400.0, 300.0);
        group.layout(0);
        let label = group.zoom_label();
        group.pan(0, (0.0, -60.0), cx);
        assert!(group.sides[0].camera.fit, "a clamped pan left Fit");
        assert_eq!(group.zoom_label(), label);
    });
}

/// Every Ctrl/Cmd+scroll event was a zoom step, so a gentle trackpad gesture
/// went from 100% to 6400%.
#[gpui::test]
fn trackpad_ctrl_scroll_steps_by_distance(cx: &mut gpui::TestAppContext) {
    let input = source(1000, 1000);
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        group: cx.new(|cx| {
            let mut group = Group::new(cx);
            group.set_inputs(
                "image".into(),
                [
                    Input {
                        source: Some(input),
                        ..Default::default()
                    },
                    Input::default(),
                    Input::default(),
                ],
                false,
                cx,
            );
            group
        }),
    });
    crate::view::test_support::redraw(cx);
    let group = cx.update(|_, app| view.read(app).group.clone());
    let bounds = cx.debug_bounds("image_viewport").unwrap();
    cx.update(|_, app| group.update(app, |group, cx| group.zoom(Zoom::Scale(1.0), None, cx)));
    crate::view::test_support::redraw(cx);
    let ctrl = gpui::Modifiers {
        control: true,
        ..Default::default()
    };
    // A gentle trackpad gesture: many small pixel deltas, 80 px in total.
    for _ in 0..20 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(4.0))),
            modifiers: ctrl,
            touch_phase: gpui::TouchPhase::Moved,
        });
    }
    cx.update(|_, app| assert_eq!(group.read(app).zoom_label(), "150%"));
    // A Windows wheel notch scrolls three lines and is still one step.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Lines(gpui::point(0.0, 3.0)),
        modifiers: ctrl,
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.update(|_, app| assert_eq!(group.read(app).zoom_label(), "200%"));
}

/// Conflict previews parsed SVG without fonts, so their text disappeared
/// until detail tiles arrived.
#[test]
fn svg_text_survives_conflict_preview() {
    for family in [
        "IBM Plex Sans",
        "Lilex",
        "sans-serif",
        "monospace",
        "system-ui",
    ] {
        let png = crate::view::diff_utils::rasterize_svg_preview_png(&text_svg(family)).unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert!(
            image.pixels().filter(|p| p[0] < 128).count() > 100,
            "conflict SVG preview lost font-family={family}"
        );
    }
}

/// Each pane decoded and kept its own copy of the same image.
#[gpui::test]
fn panes_share_decoded_originals(cx: &mut gpui::TestAppContext) {
    let mut image = source(64, 64);
    let (done, decoded) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    resume.send(()).unwrap();
    resume.send(()).unwrap();
    Arc::get_mut(&mut image).unwrap().decode_gate = Some(Arc::new(source::DecodeGate {
        done,
        resume: std::sync::Mutex::new(paused),
    }));
    let panes = [cx.new(Group::new), cx.new(Group::new)];
    for (pane, output) in panes.iter().zip([(64, 64), (128, 128)]) {
        let cache = pane.update(cx, |group, cx| {
            group.set_inputs(
                "image".into(),
                [
                    Input {
                        source: Some(image.clone()),
                        ..Default::default()
                    },
                    Input::default(),
                    Input::default(),
                ],
                false,
                cx,
            );
            group.cache.clone()
        });
        let key = TileKey {
            source: image.id,
            frame: 0,
            output,
            sampling: filter::Sampling::Nearest,
            column: 0,
            row: 0,
        };
        cache.render(&image, &[key], &AtomicU64::new(0), 0).unwrap();
    }
    assert_eq!(
        decoded.try_iter().count(),
        1,
        "each pane decoded the same image"
    );
}
