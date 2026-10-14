use super::*;
use crate::view::image_preview::{Group, ImageViewport, Input, Source, raster_format};

pub(super) fn decode_image(path: &Path, bytes: &[u8]) -> Result<Input, String> {
    decode_image_with_limit(path, bytes, 512 * 1024 * 1024)
}

fn decode_image_with_limit(path: &Path, bytes: &[u8], detail_limit: u64) -> Result<Input, String> {
    let guessed = image::guess_format(bytes).ok();
    let format = match guessed {
        Some(format) => match format {
            image::ImageFormat::Png => Some(gpui::ImageFormat::Png),
            image::ImageFormat::Jpeg => Some(gpui::ImageFormat::Jpeg),
            image::ImageFormat::Gif => Some(gpui::ImageFormat::Gif),
            image::ImageFormat::WebP => Some(gpui::ImageFormat::Webp),
            image::ImageFormat::Bmp => Some(gpui::ImageFormat::Bmp),
            image::ImageFormat::Tiff => Some(gpui::ImageFormat::Tiff),
            image::ImageFormat::Ico => Some(gpui::ImageFormat::Ico),
            image::ImageFormat::Pnm => Some(gpui::ImageFormat::Pnm),
            _ => None,
        },
        None => image_format_for_path(path),
    };
    let Some((format, source)) = format
        .and_then(|format| Source::new(format, bytes, detail_limit).map(|source| (format, source)))
    else {
        // Match gpui::img's format detection and validate the header without
        // allocating the full raster. The document can use its legacy renderer
        // when detail decoding is over budget or needs another supported codec.
        let decoder = |format| {
            image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_decoder()
        };
        if let Some(format) = guessed {
            let decoder = decoder(format).map_err(|error| error.to_string())?;
            use image::ImageDecoder as _;
            if decoder.dimensions().0 == 0 || decoder.dimensions().1 == 0 {
                return Err("Empty image".into());
            }
        } else if let Err(error) = crate::view::image_preview::svg::parse(bytes) {
            // gpui::img renders unrecognized bytes as SVG, but a broken raster
            // file reports its own codec's error rather than an SVG one.
            return Err(match format.and_then(raster_format).map(decoder) {
                Some(Err(raster)) => raster.to_string(),
                _ => error.to_string(),
            });
        }
        return Ok(Input::default());
    };
    let preview = match source.svg() {
        Some(tree) => panes::main::diff_cache::render_svg_image_diff_preview(tree),
        None => panes::main::diff_cache::render_raster_image_diff_preview(format, bytes),
    };
    Ok(Input {
        source: Some(source),
        preview,
        frame: 0,
    })
}

impl StandaloneBuffer {
    pub(super) fn image_ready(&self) -> bool {
        self.image && !self.loading && self.error.is_none()
    }

    pub(super) fn render_image(
        &mut self,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        if self.image_input.source.is_none() {
            if let Some(group) = self.image_group.as_ref() {
                group.update(cx, |group, cx| {
                    group.set_inputs(
                        self.identity.0.to_string_lossy().into_owned(),
                        Default::default(),
                        false,
                        cx,
                    )
                });
            }
            let theme = self.theme;
            return (
                gpui::img(self.identity.0.clone())
                    .id("document_image_fallback")
                    .debug_selector(|| "document_image_fallback".into())
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain)
                    .with_fallback(move || {
                        components::empty_state_message(theme, "Preview unavailable.")
                            .into_any_element()
                    })
                    .into_any_element(),
                None,
            );
        }
        let group = if let Some(group) = self.image_group.as_ref() {
            group.clone()
        } else {
            // Documents share the main pane's tiles; one of them shows at a time.
            let owner = self
                .root
                .upgrade()
                .map(|root| root.read(cx).main_pane.read(cx).image_preview_group.clone());
            let group = cx.new(|cx| match owner {
                Some(owner) => Group::shared(&owner, cx),
                None => Group::new(cx),
            });
            cx.observe(&group, |_, _, cx| cx.notify()).detach();
            self.image_group = Some(group.clone());
            group
        };
        let paused = group.read(cx).paused;
        let deadline = if paused {
            self.image_animation.frame_started_at = None;
            None
        } else {
            MainPaneView::advance_file_image_preview_side(
                &mut self.image_animation,
                self.image_input.preview.as_ref(),
                std::time::Instant::now(),
                crate::view::image_preview::animates(window, cx),
            )
        };
        self.image_timer
            .schedule(deadline, |this: &mut Self| &mut this.image_timer, cx);
        self.image_input.frame = self.image_animation.frame_index;
        let target = self.identity.0.to_string_lossy().into_owned();
        let cap = preview_path_uses_scale_down(&self.identity.0);
        group.update(cx, |group, cx| {
            group.set_inputs(
                target,
                [self.image_input.clone(), Input::default(), Input::default()],
                cap,
                cx,
            )
        });
        let controls = crate::view::image_preview::toolbar(
            group.clone(),
            self.theme,
            cx,
            |this, request, position, window, cx| {
                let _ = this.root.update(cx, |root, cx| {
                    root.open_popover_at(request, position, window, cx)
                });
            },
        );
        (
            ImageViewport {
                group,
                side: 0,
                theme: self.theme,
            }
            .into_any_element(),
            Some(controls),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_preview_lifecycle_document_keeps_valid_images_over_the_detail_budget() {
        let pixels = image::RgbaImage::from_pixel(128, 128, image::Rgba([200, 100, 50, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(
            image::load_from_memory(bytes.get_ref()).is_ok(),
            "the legacy image decoder accepts this picture"
        );
        let input = decode_image_with_limit(Path::new("large.png"), bytes.get_ref(), 64 * 1024)
            .expect("a detail budget must not turn a valid picture into an error document");
        assert!(
            input.source.is_none(),
            "an over-budget image uses the legacy renderer"
        );
    }

    #[gpui::test]
    fn image_preview_lifecycle_document_keeps_legacy_decoders_for_mislabeled_files(
        cx: &mut gpui::TestAppContext,
    ) {
        let _guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
        let (root, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("qoi.png");
        let pixels = image::RgbaImage::from_pixel(16, 12, image::Rgba([200, 100, 50, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Qoi)
            .unwrap();
        assert!(image::load_from_memory(bytes.get_ref()).is_ok());
        std::fs::write(&path, bytes.get_ref()).unwrap();
        let buffer = open_image(&root, path, cx);
        wait_for_load(&buffer, cx);
        cx.update(|_, app| {
            let buffer = buffer.read(app);
            assert!(
                buffer.error.is_none(),
                "a format supported by gpui::img must stay viewable: {:?}",
                buffer.error
            );
            assert!(buffer.image_ready());
        });
        assert!(
            cx.debug_bounds("document_image_fallback").is_some(),
            "the document mounts the legacy image renderer"
        );
    }

    #[test]
    fn image_preview_regression_standalone_sniffs_raster_bytes() {
        let pixels = image::RgbaImage::from_pixel(16, 12, image::Rgba([200, 100, 50, 255]));
        for (encoding, format, filename) in [
            (image::ImageFormat::Png, gpui::ImageFormat::Png, "image.jpg"),
            (
                image::ImageFormat::Jpeg,
                gpui::ImageFormat::Jpeg,
                "image.png",
            ),
            (image::ImageFormat::Gif, gpui::ImageFormat::Gif, "image.png"),
            (
                image::ImageFormat::Bmp,
                gpui::ImageFormat::Bmp,
                "image.webp",
            ),
            (
                image::ImageFormat::Ico,
                gpui::ImageFormat::Ico,
                "image.tiff",
            ),
            (
                image::ImageFormat::WebP,
                gpui::ImageFormat::Webp,
                "image.svg",
            ),
            (
                image::ImageFormat::Tiff,
                gpui::ImageFormat::Tiff,
                "image.bmp",
            ),
            (image::ImageFormat::Pnm, gpui::ImageFormat::Pnm, "image.gif"),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            if encoding == image::ImageFormat::Jpeg {
                image::DynamicImage::ImageRgba8(pixels.clone())
                    .to_rgb8()
                    .write_to(&mut bytes, encoding)
                    .unwrap();
            } else {
                pixels.write_to(&mut bytes, encoding).unwrap();
            }
            let input = decode_image(Path::new(filename), bytes.get_ref())
                .unwrap_or_else(|error| panic!("{encoding:?} saved as {filename}: {error}"));
            let expected = Source::new(format, bytes.get_ref(), 512 * 1024 * 1024).unwrap();
            let source = input.source.unwrap();
            assert_eq!(source.id, expected.id, "detail decoder for {filename}");
            assert_eq!(source.dimensions, (16, 12));
            let preview = input.preview.expect("bounded raster preview");
            assert_eq!(
                preview.size(0),
                gpui::size(gpui::DevicePixels(16), gpui::DevicePixels(12))
            );
            assert!(
                preview.as_bytes(0).unwrap()[..4]
                    .iter()
                    .zip([50, 100, 200, 255])
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
            );
        }
    }

    #[test]
    fn standalone_image_preview_keeps_svg_and_rejects_invalid_bytes() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="12"><rect width="16" height="12" fill="#c86432"/></svg>"##;
        let input = decode_image(Path::new("image.svg"), svg).unwrap();
        assert!(input.source.unwrap().vector);
        assert!(input.preview.is_some());
        assert!(decode_image(Path::new("image.jpg"), b"invalid image").is_err());
    }

    fn open_image(
        root: &Entity<GitCometView>,
        path: PathBuf,
        cx: &mut gpui::VisualTestContext,
    ) -> Entity<StandaloneBuffer> {
        cx.update(|_, app| {
            root.update(app, |root, cx| {
                root.documents_active = true;
                cx.notify();
            });
            root.read(app).documents.clone().update(app, |docs, cx| {
                docs.open(path, true, cx);
                docs.buffers[&docs.active.unwrap()].clone()
            })
        })
    }

    fn wait_for_load(buffer: &Entity<StandaloneBuffer>, cx: &mut gpui::VisualTestContext) {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            cx.run_until_parked();
            crate::view::test_support::redraw(cx);
            if cx.update(|_, app| !buffer.read(app).loading) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "image document did not load"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[gpui::test]
    fn failed_image_preview_reload_releases_window_zoom(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
        let (root, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reload.png");
        let pixels = image::RgbaImage::from_pixel(32, 32, image::Rgba([200, 100, 50, 255]));
        pixels.save(&path).unwrap();
        let buffer = open_image(&root, path.clone(), cx);
        wait_for_load(&buffer, cx);
        let position = cx.debug_bounds("image_viewport").unwrap().center();
        let contains = |cx: &mut gpui::VisualTestContext| {
            crate::view::test_support::redraw(cx);
            cx.debug_bounds("image_viewport")
                .is_some_and(|bounds| bounds.contains(&position))
        };
        assert!(contains(cx));
        std::fs::write(&path, b"invalid PNG after reload").unwrap();
        cx.update(|_, app| buffer.update(app, |buffer, cx| buffer.reload(cx)));
        assert!(!contains(cx), "loading documents have no image viewport");
        wait_for_load(&buffer, cx);
        cx.update(|_, app| {
            assert!(buffer.read(app).error.is_some());
            assert!(
                buffer.read(app).image_group.is_some(),
                "a retained image group must not claim the wheel"
            );
        });
        assert!(cx.debug_bounds("image_viewport").is_none());
        let before = cx.update(|_, app| crate::ui_scale::current(app).percent);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(40.0))),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|_, app| {
            assert_eq!(
                crate::ui_scale::current(app).percent,
                crate::ui_scale::step_up(before),
                "window zoom must receive Ctrl/Cmd+scroll over the error panel"
            )
        });
        assert!(!contains(cx));
        pixels.save(&path).unwrap();
        cx.update(|_, app| buffer.update(app, |buffer, cx| buffer.reload(cx)));
        wait_for_load(&buffer, cx);
        assert!(contains(cx));
        let before = cx.update(|_, app| crate::ui_scale::current(app).percent);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Lines(gpui::point(0.0, 1.0)),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|_, app| {
            assert_eq!(
                crate::ui_scale::current(app).percent,
                before,
                "a recovered image takes Ctrl/Cmd+scroll again"
            );
            assert!(
                !buffer
                    .read(app)
                    .image_group
                    .as_ref()
                    .unwrap()
                    .read(app)
                    .zoom_label()
                    .starts_with("Fit")
            );
        });
    }

    #[gpui::test]
    fn standalone_image_preview_has_sampling_zoom_and_pause_controls(
        cx: &mut gpui::TestAppContext,
    ) {
        let _guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
        let (root, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let directory = tempfile::tempdir().unwrap();
        // A mismatched extension must preserve animation and pause controls.
        let path = directory.path().join("animated.png");
        let mut bytes = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut bytes)
            .encode_frames([[255, 0, 0, 255], [0, 0, 255, 255]].map(|color| {
                image::Frame::from_parts(
                    image::RgbaImage::from_pixel(16, 16, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                )
            }))
            .unwrap();
        std::fs::write(&path, bytes).unwrap();
        let buffer = open_image(&root, path, cx);
        wait_for_load(&buffer, cx);
        assert!(cx.debug_bounds("image_format_strip").is_some());
        assert!(cx.debug_bounds("image_sampling").is_some());
        let pause = cx.debug_bounds("image_pause").unwrap();
        cx.simulate_click(pause.center(), Default::default());
        let group = cx.update(|_, app| buffer.read(app).image_group.clone().unwrap());
        cx.update(|_, app| {
            assert!(group.read(app).paused);
            group.update(app, |group, cx| {
                group.zoom(crate::view::image_preview::Zoom::Scale(1.0), None, cx)
            });
        });
        crate::view::test_support::redraw(cx);
        let bounds = cx.debug_bounds("image_viewport").unwrap();
        let ui_scale = cx.update(|_, app| crate::ui_scale::current(app).percent);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(40.0))),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|_, app| {
            assert_eq!(group.read(app).zoom_label(), "125%");
            assert_eq!(crate::ui_scale::current(app).percent, ui_scale);
        });
    }

    fn animated_gif() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut bytes)
            .encode_frames([[255, 0, 0, 255], [0, 0, 255, 255]].map(|color| {
                image::Frame::from_parts(
                    image::RgbaImage::from_pixel(16, 16, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                )
            }))
            .unwrap();
        bytes
    }

    /// With reduced motion (or an inactive window) frames hold still, but the
    /// viewport still treated the image as playing and never drew detail.
    #[gpui::test]
    fn frozen_animation_gets_detail_tiles(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
        let (root, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        cx.update(|_, app| app.set_reduce_motion(true));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("animated.gif");
        std::fs::write(&path, animated_gif()).unwrap();
        let buffer = open_image(&root, path, cx);
        wait_for_load(&buffer, cx);
        let group = cx.update(|_, app| buffer.read(app).image_group.clone().unwrap());
        cx.update(|_, app| {
            group.update(app, |group, cx| {
                group.zoom(crate::view::image_preview::Zoom::Scale(8.0), None, cx)
            })
        });
        crate::view::test_support::redraw(cx);
        cx.run_until_parked();
        assert!(
            cx.update(|_, app| group.read(app).requested_tiles()) > 0,
            "an animation that reduce-motion holds still never gets detail tiles"
        );
        assert!(
            cx.debug_bounds("image_pause").is_none(),
            "nothing moves, so there is nothing to pause"
        );
    }

    /// Window zoom skipped the image's last-frame bounds, but an open menu's
    /// scrim kept the wheel from the image too, so Ctrl+scroll did nothing.
    #[gpui::test]
    fn window_zoom_reaches_an_image_covered_by_a_menu(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
        let (root, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("covered.png");
        image::RgbaImage::from_pixel(32, 32, image::Rgba([200, 100, 50, 255]))
            .save(&path)
            .unwrap();
        let buffer = open_image(&root, path, cx);
        wait_for_load(&buffer, cx);
        let image = cx.debug_bounds("image_viewport").unwrap();
        let chip = cx.debug_bounds("image_zoom").unwrap();
        cx.simulate_click(chip.center(), Default::default());
        crate::view::test_support::redraw(cx);
        assert!(
            cx.debug_bounds("context_menu_fit").is_some(),
            "the zoom menu is open"
        );
        let group = cx.update(|_, app| buffer.read(app).image_group.clone().unwrap());
        let scale = cx.update(|_, app| crate::ui_scale::current(app).percent);
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: image.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(40.0))),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|_, app| {
            assert!(group.read(app).zoom_label().starts_with("Fit"));
            assert_eq!(
                crate::ui_scale::current(app).percent,
                crate::ui_scale::step_up(scale),
                "Ctrl+scroll over the menu's scrim did nothing"
            );
        });
    }

    /// Unrecognized bytes fell back to SVG parsing whatever the extension, so
    /// a broken PNG reported an SVG parse error.
    #[test]
    fn corrupt_raster_reports_its_codec_error() {
        let error = decode_image(Path::new("photo.png"), b"not really a picture")
            .err()
            .unwrap();
        assert!(error.contains("Png") && !error.contains("SVG"), "{error}");
        let error = decode_image(Path::new("drawing.svg"), b"<svg")
            .err()
            .unwrap();
        assert!(error.contains("SVG"), "{error}");
    }
}
