use std::sync::Arc;

use gpui::{DevicePixels, Scene, Size};

use crate::{WgpuAtlas, WgpuContext};

use super::{WgpuRenderer, WgpuSurfaceConfig};

struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bytes_per_row: u32,
    size: Size<DevicePixels>,
}

impl WgpuRenderer {
    pub(super) fn new_headless(
        context: &WgpuContext,
        size: Size<DevicePixels>,
    ) -> anyhow::Result<Self> {
        let mut renderer = Self::new_internal(
            None,
            context,
            None,
            WgpuSurfaceConfig {
                size,
                transparent: false,
                preferred_present_mode: None,
            },
            None,
            None,
            Arc::new(WgpuAtlas::from_context(context)),
        )?;

        renderer.target.clear_color = wgpu::Color::BLACK;

        Ok(renderer)
    }

    fn create_offscreen_target(&self) -> OffscreenTarget {
        let width = self.target.width();
        let height = self.target.height();
        let padded_bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let texture = self
            .resources()
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("gpui_offscreen_target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.target.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | if self.target.retained_copy_enabled() {
                        wgpu::TextureUsages::COPY_DST
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = self
            .resources()
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpui_offscreen_readback"),
                size: u64::from(padded_bytes_per_row) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        OffscreenTarget {
            texture,
            view,
            readback,
            padded_bytes_per_row,
            size: self.viewport_size(),
        }
    }

    fn read_offscreen_target(
        &self,
        target: &OffscreenTarget,
        submission: wgpu::SubmissionIndex,
    ) -> anyhow::Result<image::RgbaImage> {
        let width = target.size.width.0 as u32;
        let height = target.size.height.0 as u32;
        let bytes_per_row = width * 4;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        target
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.resources()
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| anyhow::anyhow!("failed to poll offscreen readback: {error}"))?;
        receiver
            .recv()
            .map_err(|error| anyhow::anyhow!("offscreen readback callback dropped: {error}"))?
            .map_err(|error| anyhow::anyhow!("failed to map offscreen readback: {error}"))?;

        let mapped = target.readback.slice(..).get_mapped_range();
        let mut pixels = Vec::with_capacity(bytes_per_row as usize * height as usize);
        for row in mapped.chunks_exact(target.padded_bytes_per_row as usize) {
            pixels.extend_from_slice(&row[..bytes_per_row as usize]);
        }
        drop(mapped);
        target.readback.unmap();
        if self.target.format() == wgpu::TextureFormat::Bgra8Unorm {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        image::RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| anyhow::anyhow!("offscreen readback dimensions did not match its data"))
    }

    /// Renders through the normal scene path and reads back without presenting.
    pub fn render_to_image(&mut self, scene: &Scene) -> anyhow::Result<image::RgbaImage> {
        let target = self.create_offscreen_target();
        let submission = self
            .render_to_view_with_readback(scene, &target.view, target.readback_copy())
            .ok_or_else(|| anyhow::anyhow!("failed to render scene into the offscreen target"))?;
        self.read_offscreen_target(&target, submission)
    }
}

impl OffscreenTarget {
    fn readback_copy(&self) -> super::frame::ReadbackCopy<'_> {
        super::frame::ReadbackCopy {
            texture: &self.texture,
            buffer: &self.readback,
            bytes_per_row: self.padded_bytes_per_row,
            width: self.size.width.0 as u32,
            height: self.size.height.0 as u32,
        }
    }
}

/// Surface-free renderer used by GPUI visual tests and benchmarks.
pub struct WgpuHeadlessRenderer {
    renderer: WgpuRenderer,
    target: Option<OffscreenTarget>,
}

impl WgpuHeadlessRenderer {
    pub fn new() -> anyhow::Result<Self> {
        let context = WgpuContext::new_headless(None)?;
        let renderer = WgpuRenderer::new_headless(
            &context,
            Size {
                width: DevicePixels(1),
                height: DevicePixels(1),
            },
        )?;
        Ok(Self {
            renderer,
            target: None,
        })
    }

    fn ensure_target(&mut self, size: Size<DevicePixels>) -> anyhow::Result<()> {
        anyhow::ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "headless render target must have positive dimensions"
        );
        if self
            .target
            .as_ref()
            .is_some_and(|target| target.size == size)
        {
            return Ok(());
        }
        self.renderer.update_drawable_size(size);
        self.target = Some(self.renderer.create_offscreen_target());
        Ok(())
    }

    /// Renders through the normal submission path and waits for that work to finish.
    pub fn render_scene_and_wait(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> anyhow::Result<()> {
        self.ensure_target(size)?;
        let target = self.target.as_ref().expect("target was just ensured");
        let submission = super::frame::render_to_view(
            &mut self.renderer,
            scene,
            &target.view,
            &target.texture,
            None,
        )
        .ok_or_else(|| anyhow::anyhow!("failed to render headless scene"))?;
        self.renderer
            .resources()
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| anyhow::anyhow!("failed to wait for headless render: {error}"))?;
        Ok(())
    }
}

impl gpui::PlatformHeadlessRenderer for WgpuHeadlessRenderer {
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> anyhow::Result<image::RgbaImage> {
        self.ensure_target(size)?;
        let target = self.target.as_ref().expect("target was just ensured");
        let submission = self
            .renderer
            .render_to_view_with_readback(scene, &target.view, target.readback_copy())
            .ok_or_else(|| anyhow::anyhow!("failed to render headless scene"))?;
        self.renderer.read_offscreen_target(target, submission)
    }

    fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> anyhow::Result<()> {
        self.ensure_target(size)?;
        let target = self.target.as_ref().expect("target was just ensured");
        anyhow::ensure!(
            self.renderer
                .render_to_texture(scene, &target.texture, &target.view),
            "failed to render headless scene"
        );
        Ok(())
    }

    fn sprite_atlas(&self) -> Arc<dyn gpui::PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }
}

#[cfg(test)]
mod path_target_tests {
    use super::*;
    use gpui::{
        AtlasKey, AtlasTile, Bounds, ContentMask, FontId, GlyphId, GlyphRenderMode,
        MonochromeSprite, PathBuilder, PlatformAtlas, PreparedRasterStyle, Quad,
        RasterizedGlyphFormat, RenderGlyphParams, ScaledPixels, Shadow, point, px, size,
        solid_background,
    };
    use std::borrow::Cow;

    fn scaled(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(width), ScaledPixels(height)),
        )
    }
    fn full_mask() -> ContentMask<ScaledPixels> {
        ContentMask {
            bounds: scaled(0.0, 0.0, 200.0, 100.0),
            ..Default::default()
        }
    }

    /// A clipped quad covers the same pixels as a straight underline, which
    /// only fragment masking clips. Both follow a rasterized rectangle's rule
    /// at the mask: a pixel centered on its left or top edge is inside, on its
    /// right or bottom edge outside.
    #[test]
    fn clipped_quads_and_underlines_share_mask_edge_pixels() -> anyhow::Result<()> {
        fn render(
            renderer: &mut WgpuRenderer,
            primitive: gpui::Primitive,
        ) -> anyhow::Result<image::RgbaImage> {
            let mut scene = Scene::default();
            scene.insert_primitive(primitive);
            scene.finish();
            renderer.render_to_image(&scene)
        }
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let mut renderer =
            WgpuRenderer::new_headless(&context, size(DevicePixels(64), DevicePixels(64)))?;
        renderer.retained.disable_for_tests();
        let geometry = scaled(4.0, 4.0, 56.0, 56.0);
        // Every edge runs through pixel centers, at 8.5 and 24.5.
        let mask = ContentMask {
            bounds: scaled(8.5, 8.5, 16.0, 16.0),
            ..Default::default()
        };
        let quad = Quad {
            bounds: geometry,
            content_mask: mask,
            background: solid_background(gpui::white()),
            ..Default::default()
        };
        let underline = gpui::Underline {
            order: 0,
            padding: 0,
            bounds: geometry,
            content_mask: mask,
            color: gpui::white().into(),
            thickness: ScaledPixels(1.0),
            wavy: Default::default(),
        };
        let quad = render(&mut renderer, quad.into())?;
        let underline = render(&mut renderer, underline.into())?;
        let (white, black) = (
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 0, 255]),
        );
        for (what, image) in [("quad", &quad), ("underline", &underline)] {
            assert_eq!(*image.get_pixel(8, 12), white, "{what}: left edge");
            assert_eq!(*image.get_pixel(12, 8), white, "{what}: top edge");
            assert_eq!(*image.get_pixel(24, 12), black, "{what}: right edge");
            assert_eq!(*image.get_pixel(12, 24), black, "{what}: bottom edge");
        }
        assert!(
            quad == underline,
            "the quad and the underline cover different pixels"
        );
        Ok(())
    }

    /// Vertex clipping to the content mask must draw exactly what fragment
    /// masking alone draws: every pixel centered inside the half-open mask
    /// (its left and top edges included, right and bottom excluded) matches
    /// the unmasked frame, and every other pixel is untouched. Mask edges land
    /// on pixel centers whenever an odd logical coordinate is scaled by 1.5.
    #[test]
    fn vertex_clipping_matches_fragment_masking_on_pixel_center_edges() -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let mut renderer =
            WgpuRenderer::new_headless(&context, size(DevicePixels(200), DevicePixels(100)))?;
        renderer.retained.disable_for_tests();
        let element = scaled(40.0, 30.0, 100.0, 40.0);
        let radii = gpui::Corners::all(ScaledPixels(8.0));
        let scene = |kind: usize, content_mask: Option<ContentMask<ScaledPixels>>| {
            let mut scene = Scene::default();
            scene.insert_primitive(Quad {
                bounds: scaled(0.0, 0.0, 200.0, 100.0),
                content_mask: full_mask(),
                background: solid_background(gpui::rgb(0x18212b)),
                ..Default::default()
            });
            if let Some(content_mask) = content_mask {
                let shadow = Shadow {
                    order: 0,
                    blur_radius: ScaledPixels(6.0),
                    bounds: element,
                    corner_radii: radii,
                    content_mask,
                    color: gpui::hsla(0.6, 0.5, 0.1, 0.7).into(),
                    element_bounds: element,
                    element_corner_radii: radii,
                    inset: Default::default(),
                    corner_smoothing: 0.0,
                };
                match kind {
                    0 => scene.insert_primitive(Quad {
                        bounds: scaled(4.0, 3.0, 190.0, 92.0),
                        content_mask,
                        background: solid_background(gpui::hsla(0.33, 0.7, 0.55, 0.8)),
                        corner_radii: radii,
                        ..Default::default()
                    }),
                    1 => scene.insert_primitive(shadow),
                    2 => scene.insert_primitive(Shadow {
                        blur_radius: ScaledPixels(0.0),
                        ..shadow
                    }),
                    // A 1px inset ring, as interaction outlines paint.
                    3 => scene.insert_primitive(Shadow {
                        blur_radius: ScaledPixels(0.0),
                        bounds: scaled(41.0, 31.0, 98.0, 38.0),
                        inset: true.into(),
                        ..shadow
                    }),
                    _ => scene.insert_primitive(Shadow {
                        blur_radius: ScaledPixels(3.0),
                        bounds: scaled(44.0, 34.0, 92.0, 32.0),
                        inset: true.into(),
                        ..shadow
                    }),
                }
            }
            scene.finish();
            scene
        };
        let background = renderer.render_to_image(&scene(0, None))?;
        let mut failures = Vec::new();
        for kind in 0..5 {
            let full = renderer.render_to_image(&scene(kind, Some(full_mask())))?;
            // Both frames share the shader, so check its blur reaches past the
            // shadow's bounds, 3px left of the element here.
            if kind == 1 {
                assert_ne!(full.get_pixel(37, 50), background.get_pixel(37, 50));
            }
            for mask in [
                scaled(10.5, 10.5, 100.0, 50.0),
                scaled(52.5, 37.5, 30.0, 21.0),
                scaled(36.0, 26.0, 40.5, 50.5),
                scaled(37.25, 27.75, 60.0, 30.0),
                scaled(250.0, 140.0, 1.0, 1.0),
            ] {
                let masked = renderer.render_to_image(&scene(
                    kind,
                    Some(ContentMask {
                        bounds: mask,
                        ..Default::default()
                    }),
                ))?;
                let (left, top) = (mask.origin.x.0, mask.origin.y.0);
                let (right, bottom) = (left + mask.size.width.0, top + mask.size.height.0);
                let difference = masked.enumerate_pixels().find(|(x, y, pixel)| {
                    let (cx, cy) = (*x as f32 + 0.5, *y as f32 + 0.5);
                    let inside = cx >= left && cx < right && cy >= top && cy < bottom;
                    if inside {
                        *pixel != full.get_pixel(*x, *y)
                    } else {
                        *pixel != background.get_pixel(*x, *y)
                    }
                });
                if let Some((x, y, _)) = difference {
                    failures.push(format!("kind={kind} mask={mask:?} first pixel=({x}, {y})"));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
        Ok(())
    }

    fn triangle(
        x: f32,
        y: f32,
        edge: f32,
        mask: ContentMask<ScaledPixels>,
    ) -> gpui::Path<ScaledPixels> {
        let mut builder = PathBuilder::fill();
        builder.move_to(point(px(x), px(y)));
        builder.line_to(point(px(x + edge), px(y)));
        builder.line_to(point(px(x), px(y + edge)));
        builder.close();
        let mut path = builder.build().unwrap().scale(1.0);
        path.content_mask = mask;
        path.color = gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(gpui::rgb(0xff0000), 0.0),
            gpui::linear_color_stop(gpui::rgb(0x00ff00), 1.0),
        );
        path
    }

    fn hover_scene(row: usize, resting_fills: bool) -> Scene {
        let mut scene = Scene::default();
        // The headless target clears to black. Leave uncovered pixels around each
        // translucent row so a wrong retained clear color is observable.
        for index in 0..3 {
            if !resting_fills && index != row {
                continue;
            }
            scene.insert_primitive(Quad {
                bounds: scaled(4.0, 10.0 + index as f32 * 25.0, 192.0, 14.0),
                content_mask: full_mask(),
                background: solid_background(if row == index {
                    gpui::hsla(0.6, 0.4, 0.4, 0.7)
                } else {
                    gpui::hsla(0.6, 0.1, 0.15, 0.7)
                }),
                ..Default::default()
            });
        }
        let mut graph = triangle(
            20.0,
            8.0,
            18.0,
            ContentMask {
                bounds: scaled(20.0, 10.0, 18.0, 14.0),
                ..Default::default()
            },
        );
        graph.color = solid_background(if row == 0 {
            gpui::hsla(0.6, 0.4, 0.4, 0.7)
        } else {
            gpui::black()
        });
        scene.insert_primitive(graph);
        scene.insert_primitive(Quad {
            bounds: scaled(80.0, 5.0, 15.0, 70.0),
            content_mask: ContentMask {
                bounds: scaled(82.0, 0.0, 10.0, 100.0),
                ..Default::default()
            },
            background: solid_background(gpui::hsla(0.02, 0.8, 0.6, 0.4)),
            corner_radii: gpui::Corners::all(ScaledPixels(4.0)),
            ..Default::default()
        });
        scene.finish();
        scene
    }

    #[test]
    fn retained_hover_matches_full_pixels_for_rows_graph_clip_and_invalidation()
    -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        for resting_fills in [true, false] {
            let target = size(DevicePixels(200), DevicePixels(400));
            let mut full = WgpuRenderer::new_headless(&context, target)?;
            full.retained.disable_for_tests();
            let mut retained = WgpuRenderer::new_headless(&context, target)?;
            retained.retained.enable_for_tests();
            let mut revision = 0;
            let mut previous = None;
            for row in [0, 0, 1, 2, 0] {
                if previous != Some(row) {
                    revision += 1;
                }
                let scene = hover_scene(row, resting_fills);
                let _scope = gpui::profiler::gpu::set_gpu_frame_context(true, revision);
                let expected = full.render_to_image(&scene)?;
                let actual = retained.render_to_image(&scene)?;
                assert_eq!(
                    actual, expected,
                    "retained pixels differ for row {row}, revision {revision}, resting fills {resting_fills}"
                );
                assert_eq!(
                    retained.retained.last_damage(),
                    if previous.is_none() {
                        "full"
                    } else if previous == Some(row) {
                        "reuse"
                    } else {
                        "partial"
                    },
                    "damage selection for row {row}, previous {previous:?}, revision {revision}, resting fills {resting_fills}"
                );
                previous = Some(row);
            }
            let scene = hover_scene(0, resting_fills);
            let _scope = gpui::profiler::gpu::set_gpu_frame_context(true, revision);
            retained.atlas.clear();
            assert_eq!(
                retained.render_to_image(&scene)?,
                full.render_to_image(&scene)?
            );
            assert_eq!(
                retained.retained.last_damage(),
                "full",
                "atlas invalidates unchanged scene revision"
            );
            let target = size(DevicePixels(190), DevicePixels(100));
            retained.update_drawable_size(target);
            full.update_drawable_size(target);
            assert_eq!(
                retained.render_to_image(&scene)?,
                full.render_to_image(&scene)?
            );
            assert_eq!(
                retained.retained.last_damage(),
                "full",
                "resize invalidates unchanged scene revision"
            );
            retained.update_transparency(true);
            full.update_transparency(true);
            assert_eq!(
                retained.render_to_image(&scene)?,
                full.render_to_image(&scene)?
            );
            assert_eq!(
                retained.retained.last_damage(),
                "full",
                "appearance transition invalidates the previous opaque target"
            );
            assert_eq!(
                retained.render_to_image(&scene)?,
                full.render_to_image(&scene)?
            );
            assert_eq!(retained.retained.last_damage(), "reuse");
        }
        Ok(())
    }

    #[test]
    fn retained_regrouped_coincident_paths_match_full_pixels() -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let target = size(DevicePixels(512), DevicePixels(512));
        let mut full = WgpuRenderer::new_headless(&context, target)?;
        let mut retained = WgpuRenderer::new_headless(&context, target)?;
        full.retained.disable_for_tests();
        retained.retained.enable_for_tests();
        let mask = ContentMask {
            bounds: scaled(0.0, 0.0, 512.0, 512.0),
            ..Default::default()
        };
        // A quad in a tile of its own splits four paths into two batches.
        // Coincident paths in one batch share MSAA samples; in two batches
        // they composite twice, so regrouping them changes their edges.
        let scene = |paths: [gpui::Path<ScaledPixels>; 4]| {
            let mut scene = Scene::default();
            // A layer each gives every primitive its own draw order.
            let mut paint = |primitive: gpui::Primitive| {
                scene.push_layer(mask.bounds);
                scene.insert_primitive(primitive);
                scene.pop_layer();
            };
            for (index, path) in paths.into_iter().enumerate() {
                if index == 2 {
                    paint(
                        Quad {
                            bounds: scaled(420.0, 420.0, 20.0, 20.0),
                            content_mask: mask,
                            background: solid_background(gpui::white()),
                            ..Default::default()
                        }
                        .into(),
                    );
                }
                paint(path.into());
            }
            scene.finish();
            scene
        };
        let moved = |x| triangle(x, 12.25, 24.75, mask);
        // One wide triangle clipped to the left or the right: every path has
        // the same bounds, so only the masks tell the batches apart.
        let clipped = |left| {
            let mask = ContentMask {
                bounds: scaled(left, 0.0, 56.0, 256.0),
                ..Default::default()
            };
            triangle(12.25, 12.25, 288.0, mask)
        };
        for (what, paired, crossed) in [
            (
                "moved",
                scene([moved(12.25), moved(12.25), moved(204.25), moved(204.25)]),
                scene([moved(12.25), moved(204.25), moved(12.25), moved(204.25)]),
            ),
            (
                "clipped",
                scene([clipped(0.0), clipped(0.0), clipped(200.0), clipped(200.0)]),
                scene([clipped(0.0), clipped(200.0), clipped(0.0), clipped(200.0)]),
            ),
        ] {
            retained.release_retained_frame();
            let before = assert_retained_rgba(&mut full, &mut retained, &paired, 1, "full", what)?;
            let after =
                assert_retained_rgba(&mut full, &mut retained, &crossed, 2, "partial", what)?;
            assert_ne!(before, after, "{what}: regrouping must change the edges");
        }
        Ok(())
    }

    #[test]
    fn retained_moves_shadows_rings_paths_and_rotated_sprites_exactly() -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let target = size(DevicePixels(512), DevicePixels(512));
        let mut full = WgpuRenderer::new_headless(&context, target)?;
        let mut retained = WgpuRenderer::new_headless(&context, target)?;
        full.retained.disable_for_tests();
        retained.retained.enable_for_tests();
        let glyph = retained_glyph_tile(&full)?;
        assert_eq!(glyph, retained_glyph_tile(&retained)?);
        let mask = ContentMask {
            bounds: scaled(0.0, 0.0, 512.0, 512.0),
            ..Default::default()
        };
        // Each step moves a blurred shadow, an inset ring and a path by a
        // fractional offset and rotates a sprite, as floating surfaces, focus
        // rings, graph edges and spinners do.
        let scene = |step: usize| {
            let offset = step as f32 * 3.25;
            let mut scene = Scene::default();
            scene.insert_primitive(Quad {
                bounds: scaled(0.0, 0.0, 512.0, 512.0),
                content_mask: mask,
                background: solid_background(gpui::rgb(0x18212b)),
                ..Default::default()
            });
            let card = scaled(100.25 + offset, 100.5, 30.0, 20.0);
            scene.insert_primitive(Shadow {
                order: 0,
                blur_radius: ScaledPixels(4.5),
                bounds: card,
                corner_radii: gpui::Corners::all(ScaledPixels(4.0)),
                content_mask: mask,
                color: solid_background(gpui::hsla(0.0, 0.0, 0.0, 0.6)),
                element_bounds: card,
                element_corner_radii: gpui::Corners::all(ScaledPixels(4.0)),
                inset: gpui::ShaderBool::Disabled,
                corner_smoothing: 0.0,
            });
            let ring = scaled(100.5, 300.25 + offset, 40.0, 16.0);
            scene.insert_primitive(Shadow {
                order: 0,
                blur_radius: ScaledPixels(0.0),
                bounds: scaled(101.5, 301.25 + offset, 38.0, 14.0),
                corner_radii: gpui::Corners::all(ScaledPixels(3.0)),
                content_mask: mask,
                color: solid_background(gpui::hsla(0.6, 0.8, 0.6, 1.0)),
                element_bounds: ring,
                element_corner_radii: gpui::Corners::all(ScaledPixels(4.0)),
                inset: gpui::ShaderBool::Enabled,
                corner_smoothing: 0.0,
            });
            let mut path = gpui::Path::new(point(px(300.25 + offset), px(100.5)));
            path.line_to(point(px(320.5 + offset), px(110.25)));
            path.line_to(point(px(305.75 + offset), px(125.5)));
            let mut path = path.scale(1.0);
            path.content_mask = mask;
            for vertex in &mut path.vertices {
                vertex.content_mask = mask;
            }
            path.color = solid_background(gpui::hsla(0.3, 0.7, 0.6, 0.8));
            scene.insert_primitive(path);
            let center = point(ScaledPixels(304.0), ScaledPixels(304.0));
            scene.insert_primitive(MonochromeSprite {
                order: 0,
                padding: 0,
                bounds: scaled(300.0, 300.0, 8.0, 8.0),
                content_mask: mask,
                color: gpui::hsla(0.1, 0.6, 0.7, 1.0).into(),
                tile: glyph,
                transformation: gpui::TransformationMatrix::unit()
                    .translate(center)
                    .rotate(gpui::radians(step as f32 * 0.7))
                    .translate(point(ScaledPixels(-304.0), ScaledPixels(-304.0))),
            });
            scene.finish();
            scene
        };
        for step in 0..4 {
            assert_retained_rgba(
                &mut full,
                &mut retained,
                &scene(step),
                step as u64 + 1,
                if step == 0 { "full" } else { "partial" },
                &format!("step {step}"),
            )?;
        }
        Ok(())
    }

    fn assert_retained_rgba(
        full: &mut WgpuRenderer,
        retained: &mut WgpuRenderer,
        scene: &Scene,
        revision: u64,
        damage: &str,
        context: &str,
    ) -> anyhow::Result<image::RgbaImage> {
        let _scope = gpui::profiler::gpu::set_gpu_frame_context(true, revision);
        let expected = full
            .render_to_image(scene)
            .map_err(|error| anyhow::anyhow!("{context}: full renderer: {error:#}"))?;
        let actual = retained
            .render_to_image(scene)
            .map_err(|error| anyhow::anyhow!("{context}: retained renderer: {error:#}"))?;
        assert_eq!(actual.dimensions(), expected.dimensions(), "{context}");
        let first_difference = actual
            .as_raw()
            .iter()
            .zip(expected.as_raw())
            .enumerate()
            .find(|(_, (actual, expected))| actual != expected);
        assert!(
            first_difference.is_none(),
            "{context}: exact RGBA comparison failed at byte {first_difference:?}"
        );
        assert_eq!(
            retained.retained.last_damage(),
            damage,
            "{context}: the requested retained path must actually execute"
        );
        Ok(actual)
    }

    fn empty_transparent_hover_scene(row: Option<usize>) -> Scene {
        let mut scene = Scene::default();
        if let Some(row) = row {
            scene.insert_primitive(Quad {
                bounds: scaled(12.25, 10.5 + row as f32 * 25.0, 120.5, 13.25),
                content_mask: full_mask(),
                background: gpui::rgba(0x4488cc70).into(),
                corner_radii: gpui::Corners::all(ScaledPixels(3.25)),
                ..Default::default()
            });
        }
        // A zero-alpha anchor preserves an exact following painter slot when the
        // only visible primitive moves; every resting-frame pixel is still clear.
        scene.insert_primitive(Quad {
            bounds: scaled(190.0, 90.0, 2.0, 2.0),
            content_mask: full_mask(),
            background: gpui::rgba(0x00000000).into(),
            ..Default::default()
        });
        scene.finish();
        scene
    }

    #[test]
    fn retained_transparent_clear_insert_move_erase_is_exact_rgba() -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let target = size(DevicePixels(200), DevicePixels(400));
        let mut full = WgpuRenderer::new_headless(&context, target)?;
        let mut retained = WgpuRenderer::new_headless(&context, target)?;
        full.retained.disable_for_tests();
        retained.retained.enable_for_tests();
        full.update_transparency(true);
        retained.update_transparency(true);
        let mut revision = 0;
        let mut previous = None;
        for (index, row) in [None, None, Some(0), Some(0), Some(1), Some(1), None, None]
            .into_iter()
            .enumerate()
        {
            if index == 0 || row != previous {
                revision += 1;
            }
            let image = assert_retained_rgba(
                &mut full,
                &mut retained,
                &empty_transparent_hover_scene(row),
                revision,
                if index == 0 {
                    "full"
                } else if row == previous {
                    "reuse"
                } else {
                    "partial"
                },
                &format!("empty transparent target, step {index}, row {row:?}"),
            )?;
            if row.is_none() {
                assert!(
                    image.as_raw().iter().all(|byte| *byte == 0),
                    "initial clear and erased hover must leave transparent black RGBA"
                );
            } else {
                assert!(image.pixels().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
            }
            previous = row;
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn retained_damage_clear_prewarm_precedes_partial_and_tracks_pipeline_generation()
    -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let adapter = context.adapter.get_info();
        assert_eq!(adapter.backend, wgpu::Backend::Dx12);
        let target = size(DevicePixels(201), DevicePixels(403));
        let mut full = WgpuRenderer::new_headless(&context, target)?;
        let mut retained = WgpuRenderer::new_headless(&context, target)?;
        full.retained.disable_for_tests();
        retained.retained.disable_for_tests();
        full.update_transparency(true);
        retained.update_transparency(true);
        let resting = empty_transparent_hover_scene(None);
        assert_retained_rgba(
            &mut full,
            &mut retained,
            &resting,
            1,
            "full",
            "retained off",
        )?;
        assert!(
            retained.resources().pipelines.damage_clear_quads.is_none(),
            "full frames must not create the damage-clear PSO"
        );
        retained.retained.enable_for_tests();
        {
            let _scope = gpui::profiler::gpu::set_gpu_frame_context(false, 1);
            assert_eq!(
                retained.render_to_image(&resting)?,
                full.render_to_image(&resting)?
            );
        }
        assert!(
            retained.resources().pipelines.damage_clear_quads.is_none(),
            "inactive-window fallback must not prewarm"
        );
        {
            let _budget = gpui_render::retained::reserve_retained_bytes(
                "wgpu",
                Arc::as_ptr(&context.device) as usize,
                gpui_render::retained::RETAINED_DEVICE_LIMIT,
            )
            .expect("the private device has no retained textures yet");
            assert_retained_rgba(
                &mut full,
                &mut retained,
                &resting,
                1,
                "full",
                "budget fallback",
            )?;
            assert!(
                retained.resources().pipelines.damage_clear_quads.is_none(),
                "budget fallback must not create a PSO for an unallocated retained texture"
            );
        }
        assert_retained_rgba(
            &mut full,
            &mut retained,
            &resting,
            1,
            "full",
            "qualified retained initialization",
        )?;
        let warmed = (**retained
            .resources()
            .pipelines
            .damage_clear_quads
            .as_ref()
            .expect("the qualified Full must create the clear PSO before any Partial"))
        .clone();
        let hovered = empty_transparent_hover_scene(Some(0));
        for damage in ["partial", "reuse"] {
            assert_retained_rgba(
                &mut full,
                &mut retained,
                &hovered,
                2,
                damage,
                "warm first hover",
            )?;
            assert_eq!(
                &**retained
                    .resources()
                    .pipelines
                    .damage_clear_quads
                    .as_ref()
                    .unwrap(),
                &warmed,
                "Partial/Reuse must reuse the prewarmed pipeline handle"
            );
        }
        for renderer in [&mut full, &mut retained] {
            renderer.update_drawable_size(size(DevicePixels(203), DevicePixels(405)));
        }
        assert_retained_rgba(
            &mut full,
            &mut retained,
            &hovered,
            2,
            "full",
            "same-format resize",
        )?;
        assert_eq!(
            &**retained
                .resources()
                .pipelines
                .damage_clear_quads
                .as_ref()
                .unwrap(),
            &warmed,
            "same-format resize must not recreate the clear PSO"
        );
        for renderer in [&mut full, &mut retained] {
            renderer
                .target
                .set_format_for_tests(wgpu::TextureFormat::Rgba8Unorm);
            renderer.rebuild_pipelines();
        }
        assert!(retained.resources().pipelines.damage_clear_quads.is_none());
        assert_retained_rgba(
            &mut full,
            &mut retained,
            &hovered,
            2,
            "full",
            "format generation rebuild",
        )?;
        let rebuilt = (**retained
            .resources()
            .pipelines
            .damage_clear_quads
            .as_ref()
            .unwrap())
        .clone();
        assert_ne!(
            rebuilt, warmed,
            "format rebuild must create a pipeline for the new target format"
        );
        assert_retained_rgba(
            &mut full,
            &mut retained,
            &empty_transparent_hover_scene(Some(1)),
            3,
            "partial",
            "new format first hover",
        )?;
        assert_eq!(
            &**retained
                .resources()
                .pipelines
                .damage_clear_quads
                .as_ref()
                .unwrap(),
            &rebuilt
        );

        let fresh_context = WgpuContext::new_headless(None)?;
        let adapter = fresh_context.adapter.get_info();
        assert_eq!(adapter.backend, wgpu::Backend::Dx12);
        let mut recreated =
            WgpuRenderer::new_headless(&fresh_context, size(DevicePixels(203), DevicePixels(405)))?;
        recreated.retained.enable_for_tests();
        recreated.update_transparency(true);
        assert!(
            recreated.resources().pipelines.damage_clear_quads.is_none(),
            "a new device must not reuse a pipeline belonging to the previous device"
        );
        assert_retained_rgba(
            &mut full,
            &mut recreated,
            &resting,
            1,
            "full",
            "device recreation qualified Full",
        )?;
        let recreated_pipeline = (**recreated
            .resources()
            .pipelines
            .damage_clear_quads
            .as_ref()
            .unwrap())
        .clone();
        assert_ne!(recreated_pipeline, rebuilt);
        assert_retained_rgba(
            &mut full,
            &mut recreated,
            &hovered,
            2,
            "partial",
            "recreated device first hover",
        )?;
        assert_eq!(
            &**recreated
                .resources()
                .pipelines
                .damage_clear_quads
                .as_ref()
                .unwrap(),
            &recreated_pipeline
        );
        Ok(())
    }

    #[test]
    fn replaced_retained_targets_leave_no_blit_bind_groups_behind() -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let mut renderer =
            WgpuRenderer::new_headless(&context, size(DevicePixels(64), DevicePixels(64)))?;
        renderer.retained.enable_for_tests();
        // A surface without COPY_DST blits on DX12 too.
        renderer
            .target
            .update_surface_copy_support(wgpu::TextureUsages::RENDER_ATTACHMENT, &[]);
        let mut scene = Scene::default();
        scene.finish();
        for revision in 1..=3 {
            let _scope = gpui::profiler::gpu::set_gpu_frame_context(true, revision);
            renderer.render_to_image(&scene)?;
            assert_eq!(renderer.last_retained_presentation, "blit");
            // A cached bind group would keep each released target alive.
            renderer.release_retained_frame();
        }
        assert_eq!(renderer.resources().blur_bind_group_count(), 0);
        Ok(())
    }

    #[test]
    fn retained_targets_return_their_budget_at_once() -> anyhow::Result<()> {
        use gpui_render::retained::{RETAINED_DEVICE_LIMIT, reserve_retained_bytes};

        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let device_key = Arc::as_ptr(&context.device) as usize;
        let mut scene = Scene::default();
        scene.finish();
        let _scope = gpui::profiler::gpu::set_gpu_frame_context(true, 1);
        let target_bytes = 64 * 64 * 4;
        for leave in ["release", "unfocused repaint", "close"] {
            let mut renderer =
                WgpuRenderer::new_headless(&context, size(DevicePixels(64), DevicePixels(64)))?;
            renderer.retained.enable_for_tests();
            let target = renderer.create_offscreen_target();
            assert!(renderer.render_to_texture(&scene, &target.texture, &target.view));
            // Other windows hold the rest of the device budget.
            let _others =
                reserve_retained_bytes("wgpu", device_key, RETAINED_DEVICE_LIMIT - target_bytes)
                    .expect("the retained target fits beside the other windows");
            assert!(reserve_retained_bytes("wgpu", device_key, 1).is_none());
            match leave {
                "release" => renderer.release_retained_frame(),
                // The pointer left an unfocused window, which repaints once.
                "unfocused repaint" => {
                    let _scope = gpui::profiler::gpu::set_gpu_frame_context(false, 2);
                    assert!(renderer.render_to_texture(&scene, &target.texture, &target.view));
                }
                _ => drop(renderer),
            }
            // An idle or closed window renders and polls nothing more.
            assert!(
                reserve_retained_bytes("wgpu", device_key, target_bytes).is_some(),
                "{leave}"
            );
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn retained_copy_full_partial_reuse_preserves_exact_rgba_and_odd_extents() -> anyhow::Result<()>
    {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let adapter = context.adapter.get_info();
        assert_eq!(
            adapter.backend,
            wgpu::Backend::Dx12,
            "this fixture must exercise the native DX12 copy implementation"
        );
        for format in [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ] {
            for transparent in [false, true] {
                let target = size(DevicePixels(201), DevicePixels(403));
                let mut full = WgpuRenderer::new_headless(&context, target)?;
                let mut retained = WgpuRenderer::new_headless(&context, target)?;
                full.retained.disable_for_tests();
                retained.retained.enable_for_tests();
                for renderer in [&mut full, &mut retained] {
                    renderer.target.set_format_for_tests(format);
                    renderer.rebuild_pipelines();
                    renderer.update_transparency(transparent);
                }
                assert!(retained.target.retained_copy_enabled());
                let glyph = retained_glyph_tile(&full)?;
                assert_eq!(glyph, retained_glyph_tile(&retained)?);
                let mut revision = 0;
                let mut previous = None;
                for (index, row) in [None, None, Some(0), Some(0), Some(2), Some(1), None, None]
                    .into_iter()
                    .enumerate()
                {
                    if index == 0 || row != previous {
                        revision += 1;
                    }
                    let scene =
                        transparent_composite_hover_scene(row, 0.25, 0.65, 0.45, glyph, Some(true));
                    let label = format!(
                        "copy format={format:?} alpha={transparent}, step={index}, row={row:?}"
                    );
                    let image = assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        if index == 0 {
                            "full"
                        } else if row == previous {
                            "reuse"
                        } else {
                            "partial"
                        },
                        &label,
                    )?;
                    assert_eq!(
                        retained.last_retained_presentation, "copy",
                        "{label}: no silent shader fallback"
                    );
                    if transparent {
                        assert_eq!(
                            *image.get_pixel(200, 102),
                            image::Rgba([0, 0, 0, 0]),
                            "{label}: untouched transparent corner"
                        );
                        assert!(image.pixels().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
                    }
                    previous = row;
                }
                let scene =
                    transparent_composite_hover_scene(None, 0.25, 0.65, 0.45, glyph, Some(true));
                full.update_drawable_size(size(DevicePixels(203), DevicePixels(405)));
                retained.update_drawable_size(size(DevicePixels(203), DevicePixels(405)));
                for damage in ["full", "reuse"] {
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        damage,
                        "odd resize invalidation",
                    )?;
                    assert_eq!(retained.last_retained_presentation, "copy");
                }
                full.update_transparency(!transparent);
                retained.update_transparency(!transparent);
                for damage in ["full", "reuse"] {
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        damage,
                        "alpha transition",
                    )?;
                    assert_eq!(retained.last_retained_presentation, "copy");
                }
                retained.release_retained_frame();
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &scene,
                    revision,
                    "full",
                    "released texture rebuild",
                )?;
                assert_eq!(retained.last_retained_presentation, "copy");

                let _scope = gpui::profiler::gpu::set_gpu_frame_context(true, revision);
                // A texture made while the surface lacked COPY_DST.
                let copy = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST;
                retained
                    .target
                    .update_surface_copy_support(wgpu::TextureUsages::RENDER_ATTACHMENT, &[format]);
                let target = retained.create_offscreen_target();
                retained.target.update_surface_copy_support(copy, &[format]);
                assert!(
                    !target
                        .texture
                        .usage()
                        .contains(wgpu::TextureUsages::COPY_DST)
                );
                let submission = super::super::frame::render_to_view(
                    &mut retained,
                    &scene,
                    &target.view,
                    &target.texture,
                    Some(target.readback_copy()),
                )
                .unwrap();
                assert_eq!(
                    retained.read_offscreen_target(&target, submission)?,
                    full.render_to_image(&scene)?
                );
                assert_eq!(
                    retained.last_retained_presentation, "blit",
                    "a texture without COPY_DST must not be copied"
                );

                // Surface replacement must drop unsupported COPY_DST usage and capability.
                retained
                    .target
                    .update_surface_copy_support(wgpu::TextureUsages::RENDER_ATTACHMENT, &[format]);
                assert!(!retained.target.retained_copy_enabled());
                assert!(
                    !retained
                        .target
                        .configuration()
                        .usage
                        .contains(wgpu::TextureUsages::COPY_DST)
                );
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &scene,
                    revision,
                    "full",
                    "replacement without copy support",
                )?;
                assert_eq!(retained.last_retained_presentation, "blit");
                retained.target.update_surface_copy_support(copy, &[format]);
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &scene,
                    revision,
                    "full",
                    "replacement restores copy support",
                )?;
                assert_eq!(retained.last_retained_presentation, "copy");
            }
        }
        Ok(())
    }

    #[test]
    fn retained_raw_clear_endpoints_and_colored_fallback_match_full_rgba() -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        let target = size(DevicePixels(200), DevicePixels(400));
        let mut full = WgpuRenderer::new_headless(&context, target)?;
        let mut retained = WgpuRenderer::new_headless(&context, target)?;
        full.retained.disable_for_tests();
        retained.retained.enable_for_tests();
        for transparent in [false, true] {
            full.update_transparency(transparent);
            retained.update_transparency(transparent);
            for (clear, supported, label) in [
                (wgpu::Color::TRANSPARENT, true, "transparent black"),
                (
                    wgpu::Color {
                        a: 0.25,
                        ..wgpu::Color::BLACK
                    },
                    true,
                    "translucent black",
                ),
                (wgpu::Color::BLACK, true, "opaque black"),
                (wgpu::Color::WHITE, true, "opaque white"),
                (
                    wgpu::Color {
                        r: 0.04,
                        g: 0.03,
                        b: 0.02,
                        a: 0.25,
                    },
                    false,
                    "translucent colored clear",
                ),
                (
                    wgpu::Color {
                        r: 0.04,
                        g: 0.03,
                        b: 0.02,
                        a: 0.0,
                    },
                    false,
                    "zero-alpha colored raw clear",
                ),
                (
                    wgpu::Color {
                        r: 0.04,
                        g: 0.03,
                        b: 0.02,
                        a: 1.0,
                    },
                    false,
                    "opaque colored raw clear",
                ),
            ] {
                full.target.clear_color = clear;
                retained.target.clear_color = clear;
                retained.retained.invalidate();
                for (index, row) in [None, Some(0), Some(1), None].into_iter().enumerate() {
                    let scene = empty_transparent_hover_scene(row);
                    let revision = index as u64 + 1;
                    let label = format!("alpha={transparent}, clear={label}, step={index}");
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        if supported && index != 0 {
                            "partial"
                        } else {
                            "full"
                        },
                        &label,
                    )?;
                    // Unsupported clears must never retain faulty pixels, including
                    // an unchanged repeat immediately after an inserted/erased fill.
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        if supported { "reuse" } else { "full" },
                        &format!("repeat: {label}"),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn retained_glyph_tile(renderer: &WgpuRenderer) -> anyhow::Result<AtlasTile> {
        let key = AtlasKey::Glyph {
            params: RenderGlyphParams {
                font_id: FontId(0),
                glyph_id: GlyphId(65),
                font_size: px(8.0),
                subpixel_variant: point(0, 0),
                scale_factor: 1.0,
                raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
            },
            format: RasterizedGlyphFormat::AlphaMask,
        };
        // Deterministic glyph-shaped coverage exercises the real monochrome atlas
        // pipeline without depending on a machine's font rasterizer or font files.
        let coverage = [
            0, 0, 96, 255, 255, 96, 0, 0, 0, 64, 255, 96, 96, 255, 64, 0, 0, 128, 255, 0, 0, 255,
            128, 0, 32, 224, 160, 0, 0, 160, 224, 32, 96, 255, 255, 255, 255, 255, 255, 96, 160,
            224, 64, 0, 0, 64, 224, 160, 224, 160, 0, 0, 0, 0, 160, 224, 255, 96, 0, 0, 0, 0, 96,
            255,
        ];
        renderer
            .atlas
            .get_or_insert_with(&key, &mut || {
                Ok(Some((
                    size(DevicePixels(8), DevicePixels(8)),
                    Cow::Borrowed(&coverage),
                )))
            })?
            .ok_or_else(|| anyhow::anyhow!("retained glyph fixture must produce an atlas tile"))
    }

    fn retained_tiled_glyph(
        renderer: &WgpuRenderer,
        mode: GlyphRenderMode,
    ) -> anyhow::Result<AtlasTile> {
        let key = AtlasKey::Glyph {
            params: RenderGlyphParams {
                font_id: FontId(0),
                glyph_id: GlyphId(90),
                font_size: px(8.0),
                subpixel_variant: point(0, 0),
                scale_factor: 1.0,
                raster_style: PreparedRasterStyle::independent(mode),
            },
            format: mode.rasterized_format(),
        };
        let coverage: Vec<u8> = (0..64)
            .map(|index| match (index / 8, index % 8) {
                (4, 1..=6) | (1..=6, 1 | 6) => 255,
                (_, 2 | 5) => 96,
                _ => 0,
            })
            .collect();
        let bytes = if mode == GlyphRenderMode::Subpixel {
            coverage
                .iter()
                .flat_map(|&value| [value / 2, value, value.saturating_add(24), 255])
                .collect()
        } else {
            coverage
        };
        renderer
            .atlas
            .get_or_insert_with(&key, &mut || {
                Ok(Some((
                    size(DevicePixels(8), DevicePixels(8)),
                    Cow::Borrowed(&bytes),
                )))
            })?
            .ok_or_else(|| anyhow::anyhow!("tiled glyph fixture must allocate an atlas tile"))
    }

    fn tiled_history_scene(
        hover: Option<usize>,
        fraction: f32,
        glyph: AtlasTile,
        mode: GlyphRenderMode,
    ) -> Scene {
        let mut scene = Scene::default();
        let pane = scaled(20.0, 12.0, 460.0, 840.0);
        let mask = ContentMask {
            bounds: pane,
            ..Default::default()
        };
        scene.insert_primitive(Shadow {
            order: 0,
            blur_radius: ScaledPixels(2.25),
            bounds: pane,
            corner_radii: gpui::Corners::all(ScaledPixels(8.25)),
            content_mask: ContentMask {
                bounds: scaled(0.0, 0.0, 512.0, 1024.0),
                ..Default::default()
            },
            color: gpui::rgba(0x17243850).into(),
            element_bounds: pane,
            element_corner_radii: gpui::Corners::all(ScaledPixels(8.25)),
            inset: false.into(),
            corner_smoothing: 0.65,
        });
        scene.insert_primitive(Quad {
            bounds: pane,
            content_mask: mask,
            background: gpui::rgba(0x223344bb).into(),
            corner_radii: gpui::Corners::all(ScaledPixels(8.25)),
            corner_smoothing: 0.65,
            ..Default::default()
        });
        // Stable graph raster groups precede ordinary bounds-tree row paints.
        // No per-row push_layer/raise_order_floor masks the live rank permutation.
        for row in [0, 5, 10, 19] {
            scene.insert_primitive(triangle(
                24.25 + fraction,
                18.25 + row as f32 * 40.0,
                10.5,
                mask,
            ));
        }
        let paint_glyph = |scene: &mut Scene, x: f32, y: f32, color: gpui::Hsla| {
            if mode == GlyphRenderMode::Subpixel {
                scene.insert_primitive(gpui::SubpixelSprite {
                    order: 0,
                    padding: 0,
                    bounds: scaled(x, y, 8.5, 10.25),
                    content_mask: mask,
                    color: color.into(),
                    tile: glyph,
                    transformation: Default::default(),
                });
            } else {
                scene.insert_primitive(MonochromeSprite {
                    order: 0,
                    padding: 0,
                    bounds: scaled(x, y, 8.5, 10.25),
                    content_mask: mask,
                    color: color.into(),
                    tile: glyph,
                    transformation: Default::default(),
                });
            }
        };
        for row in 0..20 {
            let y = 16.25 + fraction + row as f32 * 40.0;
            if hover == Some(row) {
                scene.insert_primitive(Quad {
                    bounds: scaled(20.0, y, 460.0, 28.5),
                    content_mask: mask,
                    background: gpui::rgba(0x55779944).into(),
                    ..Default::default()
                });
            }
            // Live graph cutouts have unchanged geometry but hover-dependent
            // backing colors. Color changes must not consume structural edits.
            for (x, width) in [(134.0, 66.0), (200.0, 4.0), (410.0, 24.0)] {
                scene.insert_primitive(Quad {
                    bounds: scaled(x, y, width, 28.5),
                    content_mask: mask,
                    background: if hover == Some(row) {
                        gpui::rgba(0x55779999)
                    } else {
                        gpui::rgba(0x22334499)
                    }
                    .into(),
                    ..Default::default()
                });
            }
            for index in 0..20 {
                paint_glyph(
                    &mut scene,
                    32.25 + fraction + index as f32 * 18.5,
                    y + 3.25,
                    gpui::hsla(0.12, 0.6, 0.65, 0.85),
                );
            }
            if hover == Some(row) {
                for index in 0..7 + row % 3 {
                    paint_glyph(
                        &mut scene,
                        282.25 + fraction + index as f32 * 7.0,
                        y + 15.0,
                        gpui::hsla(0.6, 0.4, 0.6, 0.6),
                    );
                }
            }
        }
        scene.insert_primitive(Quad {
            bounds: scaled(250.25 + fraction, 14.5, 8.5, 832.25),
            content_mask: mask,
            background: gpui::rgba(0xda624d60).into(),
            ..Default::default()
        });
        scene.finish();
        scene
    }

    #[test]
    fn retained_tiled_history_bounds_tree_matches_exact_rgba_for_rank_permutations()
    -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        for mode in [GlyphRenderMode::Grayscale, GlyphRenderMode::Subpixel] {
            for (transparent, fraction) in [(false, 0.0), (true, 0.25), (true, 0.75)] {
                let target = size(DevicePixels(512), DevicePixels(1024));
                let mut full = WgpuRenderer::new_headless(&context, target)?;
                let mut retained = WgpuRenderer::new_headless(&context, target)?;
                full.retained.disable_for_tests();
                retained.retained.enable_for_tests();
                full.update_transparency(transparent);
                retained.update_transparency(transparent);
                let glyph = retained_tiled_glyph(&full, mode)?;
                assert_eq!(glyph, retained_tiled_glyph(&retained, mode)?);
                let first = tiled_history_scene(Some(0), fraction, glyph, mode);
                let last = tiled_history_scene(Some(1), fraction, glyph, mode);
                let first_snapshot =
                    gpui_render::damage::SceneSnapshot::capture(&first, 1, 512, 1024, Vec::new())
                        .unwrap();
                let last_snapshot =
                    gpui_render::damage::SceneSnapshot::capture(&last, 1, 512, 1024, Vec::new())
                        .unwrap();
                assert!(
                    matches!(
                        last_snapshot.compare(&first_snapshot),
                        gpui_render::damage::Damage::Partial(_)
                    ),
                    "rank permutations must stay partial"
                );
                let mut previous = None;
                let mut revision = 0;
                // One box spans every changed tile. A jump across the list repaints
                // in full, and so does starting or ending a hover: that raises the
                // draw order of the strip crossing every row, which moves it among
                // paints it doesn't touch, so all its tiles repaint.
                for (index, (row, damage)) in [
                    (None, "full"),
                    (None, "reuse"),
                    (Some(0), "full"),
                    (Some(0), "reuse"),
                    (Some(1), "partial"),
                    (Some(2), "partial"),
                    (Some(19), "full"),
                    (Some(18), "partial"),
                    (None, "full"),
                    (None, "reuse"),
                ]
                .into_iter()
                .enumerate()
                {
                    if index == 0 || row != previous {
                        revision += 1;
                    }
                    let scene = tiled_history_scene(row, fraction, glyph, mode);
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        damage,
                        &format!(
                            "tiled history mode={mode:?}, transparent={transparent}, fraction={fraction}, step={index}, row={row:?}"
                        ),
                    )?;
                    previous = row;
                }

                // Identical overlapping descriptors around a third translucent
                // paint must change pixels, rather than being mistaken for Reuse.
                let mask = ContentMask {
                    bounds: scaled(0.0, 0.0, 512.0, 1024.0),
                    ..Default::default()
                };
                let a = Quad {
                    bounds: scaled(62.25, 62.25, 24.5, 24.5),
                    content_mask: mask,
                    background: gpui::rgba(0xff000080).into(),
                    ..Default::default()
                };
                let b = Quad {
                    background: gpui::rgba(0x0000ff80).into(),
                    ..a
                };
                let mut overlap = Scene::default();
                for quad in [a, a, b] {
                    overlap.insert_primitive(quad);
                }
                overlap.finish();
                // Recreate targets to ensure this independent order proof starts
                // with an initial Full rather than an unrelated previous scene.
                retained.retained.invalidate();
                revision += 1;
                let original = assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &overlap,
                    revision,
                    "full",
                    "tiled overlap initial",
                )?;
                overlap.quads.swap(1, 2); // Counts/pipelines stay unchanged.
                revision += 1;
                let changed = assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &overlap,
                    revision,
                    "partial",
                    "tiled overlap reordered",
                )?;
                assert_ne!(
                    original.as_raw(),
                    changed.as_raw(),
                    "overlapping painter reorder must visibly change RGBA"
                );
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &overlap,
                    revision,
                    "reuse",
                    "tiled overlap repeated",
                )?;
            }
        }
        Ok(())
    }

    fn transparent_composite_hover_scene(
        row: Option<usize>,
        fraction: f32,
        smoothing: f32,
        backing_alpha: f32,
        glyph: AtlasTile,
        badge_before_graph: Option<bool>,
    ) -> Scene {
        let mut scene = Scene::default();
        let backing_bounds = scaled(4.25 + fraction, 3.5 + fraction, 190.5, 90.25);
        let radii = gpui::Corners::all(ScaledPixels(9.25));
        scene.insert_primitive(Shadow {
            order: 0,
            blur_radius: ScaledPixels(2.5),
            bounds: backing_bounds,
            corner_radii: radii,
            content_mask: full_mask(),
            color: gpui::rgba(0x17243850).into(),
            element_bounds: backing_bounds,
            element_corner_radii: radii,
            inset: false.into(),
            corner_smoothing: smoothing,
        });
        scene.raise_order_floor();
        scene.insert_primitive(Quad {
            bounds: backing_bounds,
            content_mask: full_mask(),
            background: solid_background(gpui::hsla(0.62, 0.3, 0.35, backing_alpha)),
            corner_radii: radii,
            corner_smoothing: smoothing,
            ..Default::default()
        });
        scene.raise_order_floor();
        // A shifted/shrunk inset hole is far from the first hovered row. Its
        // actual raster geometry is the whole element, so pruning by the hole
        // bounds would lose static inset pixels when that row is damage-cleared.
        scene.insert_primitive(Shadow {
            order: 0,
            blur_radius: ScaledPixels(0.5),
            bounds: scaled(140.25 + fraction, 62.5, 26.5, 14.0),
            corner_radii: gpui::Corners::all(ScaledPixels(2.0)),
            content_mask: full_mask(),
            color: gpui::rgba(0x10172870).into(),
            element_bounds: backing_bounds,
            element_corner_radii: radii,
            inset: true.into(),
            corner_smoothing: smoothing,
        });
        scene.raise_order_floor();
        if let Some(row) = row {
            scene.insert_primitive(Quad {
                bounds: scaled(
                    10.25 + fraction,
                    12.5 + row as f32 * 24.0,
                    165.5 - row as f32 * 0.75,
                    13.25,
                ),
                content_mask: ContentMask {
                    bounds: scaled(
                        11.5 + fraction,
                        12.75 + row as f32 * 24.0,
                        162.5 - row as f32 * 0.25,
                        12.5,
                    ),
                    fade_out: gpui::Edges {
                        left: ScaledPixels(1.25),
                        right: ScaledPixels(1.75),
                        ..Default::default()
                    },
                },
                background: gpui::rgba(0x4f98df70).into(),
                corner_radii: gpui::Corners::all(ScaledPixels(3.5 + row as f32 * 0.25)),
                corner_smoothing: smoothing,
                ..Default::default()
            });
        }
        let paint_badge = |scene: &mut Scene| {
            if let Some(row) = row {
                // The live history row adds an entire lane-name badge only while
                // hovered. Variable run lengths and distant row moves must repaint
                // both the old and new badge without losing the static text anchor.
                for index in 0..7 + row {
                    scene.insert_primitive(MonochromeSprite {
                        order: 0,
                        padding: 0,
                        bounds: scaled(
                            112.25 + fraction + index as f32 * 6.5,
                            14.5 + row as f32 * 24.0,
                            6.5,
                            10.75,
                        ),
                        content_mask: full_mask(),
                        color: gpui::hsla(0.6, 0.4, 0.6, 0.6).into(),
                        tile: glyph,
                        transformation: Default::default(),
                    });
                }
            }
        };
        scene.raise_order_floor();
        if badge_before_graph == Some(true) {
            paint_badge(&mut scene);
            scene.raise_order_floor();
        }
        scene.insert_primitive(triangle(
            17.25 + fraction,
            10.5,
            17.5,
            ContentMask {
                bounds: scaled(19.5 + fraction, 13.25, 13.75, 12.5),
                fade_out: gpui::Edges::all(ScaledPixels(0.75)),
            },
        ));
        scene.raise_order_floor();
        if badge_before_graph == Some(false) {
            paint_badge(&mut scene);
        }
        scene.raise_order_floor();
        scene.insert_primitive(MonochromeSprite {
            order: 0,
            padding: 0,
            bounds: scaled(44.25 + fraction, 14.5, 12.5, 10.75),
            content_mask: ContentMask {
                bounds: scaled(45.0 + fraction, 14.25, 11.5, 11.5),
                fade_out: gpui::Edges::all(ScaledPixels(0.75)),
            },
            color: gpui::hsla(0.12, 0.8, 0.65, 0.85).into(),
            tile: glyph,
            transformation: Default::default(),
        });
        scene.raise_order_floor();
        scene.insert_primitive(Quad {
            bounds: scaled(80.25 + fraction, 6.5, 16.5, 78.25),
            content_mask: full_mask(),
            background: gpui::rgba(0xda624d60).into(),
            corner_radii: gpui::Corners::all(ScaledPixels(4.25)),
            corner_smoothing: smoothing,
            ..Default::default()
        });
        scene.finish();
        scene
    }

    #[test]
    fn retained_transparent_composite_hover_matches_exact_rgba_and_invalidates_appearance()
    -> anyhow::Result<()> {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None)?;
        for badge_before_graph in [None, Some(false), Some(true)] {
            for (fraction, smoothing, backing_alpha) in [
                (0.0, 0.0, 0.45),
                (0.25, 0.65, 1.0),
                (0.5, 0.0, 1.0),
                (0.75, 0.65, 0.45),
            ] {
                let target = size(DevicePixels(200), DevicePixels(400));
                let mut full = WgpuRenderer::new_headless(&context, target)?;
                let mut retained = WgpuRenderer::new_headless(&context, target)?;
                full.retained.disable_for_tests();
                retained.retained.enable_for_tests();
                full.update_transparency(true);
                retained.update_transparency(true);
                let glyph = retained_glyph_tile(&full)?;
                assert_eq!(glyph, retained_glyph_tile(&retained)?);
                let label = format!(
                    "fraction={fraction}, smoothing={smoothing}, backing={backing_alpha}, badge_before_graph={badge_before_graph:?}"
                );
                let mut revision = 0;
                let mut previous = None;
                let resting = transparent_composite_hover_scene(
                    None,
                    fraction,
                    smoothing,
                    backing_alpha,
                    glyph,
                    badge_before_graph,
                );
                let mut resting_pixels = None;
                for (index, row) in [
                    None,
                    None,
                    Some(0),
                    Some(0),
                    Some(2),
                    Some(0),
                    Some(1),
                    Some(2),
                    None,
                    None,
                ]
                .into_iter()
                .enumerate()
                {
                    if index == 0 || row != previous {
                        revision += 1;
                    }
                    let scene = transparent_composite_hover_scene(
                        row,
                        fraction,
                        smoothing,
                        backing_alpha,
                        glyph,
                        badge_before_graph,
                    );
                    if badge_before_graph == Some(true)
                        && let Some(row) = row
                    {
                        assert!(scene.render_commands().windows(2).any(|commands|
                            matches!((&commands[0], &commands[1]),
                                (gpui::RenderCommand::Batch(gpui::PrimitiveBatch::Quads { range: quad, .. }),
                                 gpui::RenderCommand::Batch(gpui::PrimitiveBatch::MonochromeSprites { range: glyphs, .. }))
                                if quad.len() == 1 && glyphs.len() == 7 + row)),
                            "fixture must prove fill directly precedes the changed badge: {label}");
                    }
                    let image = assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &scene,
                        revision,
                        if index == 0 {
                            "full"
                        } else if row == previous {
                            "reuse"
                        } else {
                            "partial"
                        },
                        &format!("{label}, step={index}, row={row:?}"),
                    )?;
                    assert!(image.pixels().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
                    assert!(
                        image.get_pixel(0, 0)[3] < 255,
                        "rounded outside corner: {label}"
                    );
                    if backing_alpha == 1.0 {
                        assert_eq!(image.get_pixel(150, 85)[3], 255, "opaque interior: {label}");
                    }
                    if index == 0 {
                        resting_pixels = Some(image.clone());
                    } else if row.is_none() {
                        assert_eq!(
                            Some(&image),
                            resting_pixels.as_ref(),
                            "erase restores every byte: {label}"
                        );
                    } else {
                        assert_ne!(
                            Some(&image),
                            resting_pixels.as_ref(),
                            "hover must visibly alter the fixture: {label}"
                        );
                    }
                    previous = row;
                }
                // Invalidate independently of scene revision: resize, alpha mode,
                // clear color, and atlas generation must never reuse stale pixels.
                let resized = size(DevicePixels(193), DevicePixels(97));
                full.update_drawable_size(resized);
                retained.update_drawable_size(resized);
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &resting,
                    revision,
                    "full",
                    &format!("resize: {label}"),
                )?;
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &resting,
                    revision,
                    "reuse",
                    &format!("resized reuse: {label}"),
                )?;
                for transparent in [false, true] {
                    full.update_transparency(transparent);
                    retained.update_transparency(transparent);
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &resting,
                        revision,
                        "full",
                        &format!("alpha transition to {transparent}: {label}"),
                    )?;
                    assert_retained_rgba(
                        &mut full,
                        &mut retained,
                        &resting,
                        revision,
                        "reuse",
                        &format!("alpha reuse {transparent}: {label}"),
                    )?;
                }
                // A direct clear-color change must be detected by the retained
                // appearance key even when no public alpha toggle fires.
                full.target.clear_color = wgpu::Color {
                    r: 0.04,
                    g: 0.03,
                    b: 0.02,
                    a: 0.25,
                };
                retained.target.clear_color = full.target.clear_color;
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &resting,
                    revision,
                    "full",
                    &format!("clear-color transition: {label}"),
                )?;
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &resting,
                    revision,
                    "full",
                    &format!("unsupported clear-color fallback: {label}"),
                )?;
                // Restore an exact endpoint so atlas invalidation still tests
                // a qualified retained target rather than the unsupported clear.
                full.target.clear_color = wgpu::Color::TRANSPARENT;
                retained.target.clear_color = full.target.clear_color;
                full.atlas.clear();
                retained.atlas.clear();
                let glyph = retained_glyph_tile(&full)?;
                assert_eq!(glyph, retained_glyph_tile(&retained)?);
                let scene = transparent_composite_hover_scene(
                    None,
                    fraction,
                    smoothing,
                    backing_alpha,
                    glyph,
                    badge_before_graph,
                );
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &scene,
                    revision,
                    "full",
                    &format!("atlas invalidation: {label}"),
                )?;
                assert_retained_rgba(
                    &mut full,
                    &mut retained,
                    &scene,
                    revision,
                    "reuse",
                    &format!("atlas reuse: {label}"),
                )?;
            }
        }
        Ok(())
    }

    #[test]
    fn cropped_path_targets_match_full_viewport_pixels_when_growing_and_resizing() {
        let _gpu_test_guard = crate::test_gpu::guard();
        let context = WgpuContext::new_headless(None).expect("hardware or software GPU");
        let target = size(DevicePixels(512), DevicePixels(320));
        let mask = ContentMask {
            bounds: Bounds::new(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(1024.0), ScaledPixels(640.0)),
            ),
            ..Default::default()
        };
        let mut full = WgpuRenderer::new_headless(&context, target).unwrap();
        let mut cropped = WgpuRenderer::new_headless(&context, target).unwrap();
        for (x, y, target) in [
            (16.0, 16.0, target),
            (180.0, 120.0, target),
            (16.0, 16.0, target),
            (40.0, 35.0, size(DevicePixels(360), DevicePixels(240))),
        ] {
            full.update_drawable_size(target);
            cropped.update_drawable_size(target);
            let mut force_full = Scene::default();
            force_full.paths.push(triangle(
                0.0,
                0.0,
                target.width.0.max(target.height.0) as f32,
                mask,
            ));
            full.ensure_path_textures(&force_full);

            let mut scene = Scene::default();
            scene.insert_primitive(Quad {
                bounds: mask.bounds,
                content_mask: mask,
                background: solid_background(gpui::black()),
                ..Default::default()
            });
            scene.insert_primitive(triangle(x, y, 47.5, mask));
            // A later path beyond the viewport makes the combined sprite span
            // empty space past the cropped target. That space must stay black.
            scene.insert_primitive(Quad {
                bounds: Bounds::new(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(1.0), ScaledPixels(1.0)),
                ),
                content_mask: mask,
                background: solid_background(gpui::black()),
                ..Default::default()
            });
            scene.insert_primitive(triangle(800.0, 10.0, 40.0, mask));
            // Negative geometry and a clipped curve exercise coordinate/clip
            // preservation independently of the viewport's dimensions.
            let mut curve = PathBuilder::stroke(px(1.6));
            curve.move_to(point(px(-10.0), px(30.0)));
            curve.cubic_bezier_to(
                point(px(40.0), px(70.0)),
                point(px(24.0), px(22.0)),
                point(px(5.0), px(60.0)),
            );
            let mut curve = curve.build().unwrap().scale(1.0);
            curve.content_mask = ContentMask {
                bounds: Bounds::new(
                    point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size(ScaledPixels(30.0), ScaledPixels(60.0)),
                ),
                ..Default::default()
            };
            curve.color = solid_background(gpui::white());
            scene.insert_primitive(curve);
            scene.finish();
            let expected = full.render_to_image(&scene).expect("full target frame");
            let actual = cropped
                .render_to_image(&scene)
                .expect("cropped target frame");
            assert!(
                actual.get_pixel(x as u32 + 8, y as u32 + 8)[0] > 100,
                "triangle must render"
            );
            assert_eq!(
                &actual.get_pixel(300, 200).0[..3],
                &[0, 0, 0],
                "no clamped edge smear"
            );
            let max_diff = actual
                .as_raw()
                .iter()
                .zip(expected.as_raw())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                max_diff <= 1,
                "cropped rendering changed a pixel by {max_diff}"
            );
            let texture = cropped
                .resources()
                .path_intermediate_texture
                .as_ref()
                .unwrap();
            assert!(
                texture.width() < target.width.0 as u32,
                "narrow scene must keep a narrow target"
            );
            assert!(texture.height() < target.height.0 as u32);
            if let Ok(directory) = std::env::var("GPUI_PATH_TEST_IMAGES") {
                actual
                    .save(
                        std::path::Path::new(&directory)
                            .join(format!("cropped-{x}-{y}-{}.png", target.width.0)),
                    )
                    .unwrap();
                expected
                    .save(
                        std::path::Path::new(&directory)
                            .join(format!("full-{x}-{y}-{}.png", target.width.0)),
                    )
                    .unwrap();
            }
        }
    }
}
