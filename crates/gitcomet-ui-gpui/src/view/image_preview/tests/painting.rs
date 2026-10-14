use super::*;
use std::sync::Mutex;

// Capture GPUI's actual scene and atlas rectangles without requiring a GPU.
#[derive(Clone, Default)]
struct SceneCapture(Arc<Mutex<Vec<gpui::PolychromeSprite>>>, Arc<ImageAtlas>);

impl gpui::PlatformHeadlessRenderer for SceneCapture {
    fn render_scene_to_image(
        &mut self,
        scene: &gpui::Scene,
        size: gpui::Size<gpui::DevicePixels>,
    ) -> gpui::Result<image::RgbaImage> {
        self.render_scene(scene, size)?;
        Ok(image::RgbaImage::new(1, 1))
    }

    fn render_scene(
        &mut self,
        scene: &gpui::Scene,
        _: gpui::Size<gpui::DevicePixels>,
    ) -> gpui::Result<()> {
        *self.0.lock().unwrap() = scene.polychrome_sprites.clone();
        Ok(())
    }

    fn sprite_atlas(&self) -> Arc<dyn gpui::PlatformAtlas> {
        self.1.clone()
    }
}

#[derive(Default)]
struct ImageAtlas(Mutex<std::collections::HashSet<gpui::AtlasKey>>);
impl gpui::PlatformAtlas for ImageAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &gpui::AtlasKey,
        build: &mut dyn FnMut() -> gpui::Result<
            Option<(gpui::Size<gpui::DevicePixels>, std::borrow::Cow<'a, [u8]>)>,
        >,
    ) -> gpui::Result<Option<gpui::AtlasTile>> {
        Ok(build()?.map(|(size, _)| {
            self.0.lock().unwrap().insert(key.clone());
            gpui::AtlasTile {
                texture_id: gpui::AtlasTextureId {
                    index: 0,
                    kind: key.texture_kind(),
                },
                tile_id: gpui::TileId(0),
                padding: 0,
                bounds: Bounds::new(Default::default(), size),
            }
        }))
    }

    fn get_or_insert_glyph_with(
        &self,
        _: &gpui::RenderGlyphParams,
        _: &mut dyn FnMut() -> gpui::Result<gpui::ValidatedRasterizedGlyph>,
    ) -> gpui::Result<gpui::GlyphAtlasEntry> {
        unreachable!("the image-only test does not paint text")
    }

    fn remove(&self, key: &gpui::AtlasKey) {
        self.0.lock().unwrap().remove(key);
    }

    fn contains(&self, key: &gpui::AtlasKey) -> bool {
        self.0.lock().unwrap().contains(key)
    }
}

fn tile_key(source: &Source, output: (u32, u32)) -> TileKey {
    TileKey {
        source: source.id,
        frame: 0,
        output,
        sampling: filter::Sampling::Nearest,
        column: 0,
        row: 0,
    }
}

struct CloseCacheOnDraw(Arc<Cache>);
impl Render for CloseCacheOnDraw {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.close(cx);
        div().size_full()
    }
}

#[test]
fn image_preview_lifecycle_evicted_and_closed_tiles_leave_the_window_atlas() {
    let _guard = crate::test_support::lock_visual_test();
    for close in [false, true] {
        let capture = SceneCapture::default();
        let renderer = capture.clone();
        let mut cx = gpui::HeadlessAppContext::with_platform(
            Arc::new(gpui::TestTextSystem),
            Arc::new(()),
            move || Some(Box::new(renderer.clone())),
        );
        cx.update(|cx| {
            cx.set_global(Preferences {
                mode: ImageSamplingMode::Pixels,
                ..Default::default()
            })
        });
        let source = source(64, 64);
        let key = tile_key(&source, (64, 64));
        let group = cx.new(|cx| {
            let mut group = Group::new(cx);
            group.set_inputs(
                "painted".into(),
                [
                    Input {
                        source: Some(source.clone()),
                        ..Default::default()
                    },
                    Input::default(),
                    Input::default(),
                ],
                false,
                cx,
            );
            group.zoom(Zoom::Scale(1.0), None, cx);
            group
        });
        let cache = group.read_with(&cx, |group, _| group.cache.clone());
        cache
            .render(&source, &[key], &AtomicU64::new(0), 0)
            .unwrap();
        let painted = cache.peek(&key).unwrap();
        let window = cx
            .open_window(gpui::size(px(64.0), px(64.0)), |_, cx| {
                cx.new(|_| Harness {
                    group: group.clone(),
                })
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, app| {
            let _ = window.draw(app);
            assert!(
                window.has_image_atlas_entry(&painted),
                "the test painted a real atlas entry"
            );
        })
        .unwrap();
        if close {
            cx.update_window(window.into(), |_, window, app| {
                window.replace_root(app, |_, _| CloseCacheOnDraw(cache.clone()));
                let _ = window.draw(app);
            })
            .unwrap();
        } else {
            for width in 65..65 + cache::TILE_COUNT as u32 {
                cache
                    .render(
                        &source,
                        &[tile_key(&source, (width, 64))],
                        &AtomicU64::new(0),
                        0,
                    )
                    .unwrap();
            }
            assert!(cache.peek(&key).is_none(), "the painted tile was evicted");
            group.update(&mut cx, |group, cx| {
                group.set_inputs("empty".into(), Default::default(), false, cx)
            });
            cx.update_window(window.into(), |_, window, app| {
                let _ = window.draw(app);
            })
            .unwrap();
        }
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, _| {
            assert!(!window.has_image_atlas_entry(&painted), "cleanup during drawing must release the active window's GPU allocation; close={close}");
        }).unwrap();
    }
}

struct ZoomLabelProbe {
    group: Entity<Group>,
    labels: Arc<Mutex<Vec<String>>>,
    target: String,
    input: Input,
}
impl Render for ZoomLabelProbe {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.group.update(cx, |group, cx| {
            group.set_inputs(
                self.target.clone(),
                [self.input.clone(), Input::default(), Input::default()],
                false,
                cx,
            );
        });
        self.labels
            .lock()
            .unwrap()
            .push(self.group.read(cx).zoom_label());
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar(
                self.group.clone(),
                AppTheme::gitcomet_dark(),
                cx,
                |_, _, _, _, _| {},
            ))
            .child(div().flex_1().min_h(px(0.0)).child(ImageViewport {
                group: self.group.clone(),
                side: 0,
                theme: AppTheme::gitcomet_dark(),
            }))
    }
}

#[test]
fn image_preview_lifecycle_cached_fit_updates_the_rendered_zoom_label() {
    let _guard = crate::test_support::lock_visual_test();
    let mut cx = gpui::TestApp::with_text_system_and_assets(
        Arc::new(gpui::TestTextSystem),
        Arc::new(crate::assets::GitCometAssets::default()),
    );
    cx.set_global(Preferences {
        mode: ImageSamplingMode::Pixels,
        ..Default::default()
    });
    let first = source(200, 100);
    let second = source(1000, 1000);
    let group = cx.new_entity(|cx| {
        let mut group = Group::new(cx);
        group.set_inputs(
            "first".into(),
            [
                Input {
                    source: Some(first.clone()),
                    ..Default::default()
                },
                Input::default(),
                Input::default(),
            ],
            false,
            cx,
        );
        group
    });
    let cache = cx.read_entity(&group, |group, _| group.cache.clone());
    for (source, output) in [
        (&first, (250, 125)),
        (&first, (500, 250)),
        (&second, (160, 160)),
        (&second, (320, 320)),
    ] {
        cache
            .render(source, &[tile_key(source, output)], &AtomicU64::new(0), 0)
            .unwrap();
    }
    let labels = Arc::new(Mutex::new(Vec::new()));
    let mut window = cx.open_window_with_options(
        gpui::WindowOptions::new().window_bounds(Some(gpui::WindowBounds::Windowed(Bounds::new(
            gpui::point(px(0.0), px(0.0)),
            gpui::size(px(250.0), px(180.0)),
        )))),
        |_, cx| {
            cx.observe(&group, |_, _, cx| cx.notify()).detach();
            ZoomLabelProbe {
                group: group.clone(),
                labels: labels.clone(),
                target: "first".into(),
                input: Input {
                    source: Some(first.clone()),
                    ..Default::default()
                },
            }
        },
    );
    for _ in 0..3 {
        window.simulate_request_frame(gpui::RequestFrameOptions {
            require_presentation: true,
            ..Default::default()
        });
    }
    for source in [second, first] {
        labels.lock().unwrap().clear();
        window.update(|probe, _, cx| {
            probe.target = "second".into();
            probe.input = Input {
                source: Some(source),
                ..Default::default()
            };
            cx.notify();
        });
        // The first switch resets the target; the second updates the same
        // target's dimensions. Neither needs a tile worker notification.
        for _ in 0..3 {
            window.simulate_request_frame(gpui::RequestFrameOptions {
                require_presentation: true,
                ..Default::default()
            });
        }
        let expected = cx.read_entity(&group, |group, _| {
            assert!(
                group.sides[0].task.is_none(),
                "cached tiles cannot provide a later worker notification"
            );
            group.zoom_label()
        });
        assert_ne!(expected, "Fit · 100%");
        assert_eq!(
            labels.lock().unwrap().last(),
            Some(&expected),
            "the toolbar must repaint after the viewport determines its fit scale"
        );
    }
}

#[test]
fn playing_image_preview_preserves_texture_mapping_across_tiles_and_panning() {
    let _guard = crate::test_support::lock_visual_test();
    let capture = SceneCapture::default();
    let renderer = capture.clone();
    let mut cx = gpui::HeadlessAppContext::with_platform(
        Arc::new(gpui::TestTextSystem),
        Arc::new(()),
        move || Some(Box::new(renderer.clone())),
    );
    let source = source(6000, 40);
    let preview = Arc::new(gpui::RenderImage::new(vec![
        image::Frame::new(image::RgbaImage::new(1920, 12)),
        image::Frame::new(image::RgbaImage::new(1920, 12)),
    ]));
    let group = cx.new(|cx| {
        let mut group = Group::new(cx);
        group.set_inputs(
            "animated".into(),
            [
                Input {
                    source: Some(source),
                    preview: Some(preview),
                    frame: 1,
                },
                Input::default(),
                Input::default(),
            ],
            false,
            cx,
        );
        group
    });
    let window = cx
        .open_window(gpui::size(px(1024.0), px(128.0)), |_, cx| {
            cx.new(|_| Harness {
                group: group.clone(),
            })
        })
        .unwrap();
    let dpr = cx
        .update_window(window.into(), |_, window, app| {
            let _ = window.draw(app);
            window.scale_factor()
        })
        .unwrap();
    group.update(&mut cx, |group, cx| group.zoom(Zoom::Scale(64.0), None, cx));
    for pan in [0.0, -13.0, -201.0] {
        group.update(&mut cx, |group, cx| group.pan(0, (pan, 0.0), cx));
        cx.update_window(window.into(), |_, window, app| {
            let _ = window.draw(app);
        })
        .unwrap();
        cx.capture_screenshot(window.into()).unwrap();
        let sprites = capture.0.lock().unwrap();
        assert!(!sprites.is_empty());
        for sprite in sprites.iter() {
            assert_eq!(
                sprite.tile.bounds.size,
                gpui::size(gpui::DevicePixels(1920), gpui::DevicePixels(12)),
                "each clipped preview must retain its complete atlas rectangle"
            );
            assert_eq!(sprite.bounds.size.width.0, 6000.0 * 64.0);
            assert_eq!(sprite.bounds.size.height.0, 40.0 * 64.0);
        }
        let masked_area: f32 = sprites
            .iter()
            .map(|sprite| {
                let bounds = sprite.bounds.intersect(&sprite.content_mask.bounds);
                bounds.size.width.0 * bounds.size.height.0
            })
            .sum();
        assert_eq!(
            masked_area,
            1024.0 * 128.0 * dpr * dpr,
            "clipped previews cover the viewport exactly once"
        );
    }
}
