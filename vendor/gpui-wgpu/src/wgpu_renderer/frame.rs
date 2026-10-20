use super::{
    WgpuRenderer, begin_color_render_pass,
    buffers::{InstanceTransport, InstanceUpload},
    filters::{FILTER_UNIFORMS_PER_COMPOSITE, FrameUniformRequirements},
    path_types,
    pipelines::SmoothingRequirements,
};
use gpui::{
    FilterRenderTarget, MonochromeSprite, PolychromeSprite, PrimitiveBatch, Quad, RenderCommand,
    Scene, ScenePlanRequirements, Shadow, SubpixelSprite, Underline,
};
use gpui_render::TargetStack;
use gpui_render::blur::ScissorRectangle;
use gpui_render::blur::{FilterCompositeClip, FilterCompositeParameters};
use gpui_render::damage::{Damage, batch_intersects, clear_quad};
use gpui_render::shaders::{
    common::{FontRasterizationUniforms, GlobalUniforms, ShaderBool},
    interface as shader_interface,
};

pub(super) fn render_to_view(
    renderer: &mut WgpuRenderer,
    scene: &Scene,
    frame_view: &wgpu::TextureView,
    frame_texture: &wgpu::Texture,
    readback: Option<ReadbackCopy<'_>>,
) -> Option<wgpu::SubmissionIndex> {
    if !begin_frame(renderer) {
        return None;
    }
    let atlas_generation = if renderer.retained.enabled() {
        gpui::PlatformAtlas::generation(renderer.atlas.as_ref())
    } else {
        0
    };
    let mut retained = renderer.retained.prepare(
        scene,
        atlas_generation,
        renderer.target.width(),
        renderer.target.height(),
        renderer.target.retained_copy_enabled(),
        renderer.resources.as_ref()?,
        renderer.target.format(),
        renderer.target.alpha_mode(),
        renderer.target.clear_color(),
    );
    let transport = renderer.resources().instances.transport();
    let requirements = FrameRequirements::for_damage(scene, &retained.damage, transport);
    let resources = renderer.resources_mut();
    if let Err(error) = resources
        .pipelines
        .prepare_smoothing(&resources.device, requirements.smoothing)
    {
        let message = format!("creating smoothing pipelines: {error}");
        if matches!(
            error,
            wgpu::Error::OutOfMemory { .. } | wgpu::Error::Internal { .. }
        ) {
            *renderer.faults.fatal_error.lock().unwrap() = Some(message.clone());
        }
        // Let the next begin_frame consume this error. Handling it here and
        // clearing the slot would reset the retry counter at the next frame.
        *renderer.faults.pending_error.lock().unwrap() = Some(message);
        renderer.retained.complete(retained, false);
        renderer.target.request_redraw();
        return None;
    }
    let mut targets = PreparedTargets::prepare(renderer, scene, frame_view, &requirements)?;
    // The retained texture matches the target by construction. The frame
    // texture may not: it can predate a resize or lack COPY_DST.
    let retained_copy = (retained.copy_texture.take())
        .filter(|source| {
            targets.offscreen.is_none()
                && frame_texture
                    .usage()
                    .contains(wgpu::TextureUsages::COPY_DST)
                && frame_texture.size() == source.size()
                && frame_texture.format() == source.format()
        })
        .map(|source| (source, frame_texture.clone()));
    if let Some(view) = &retained.view {
        targets.active = view.clone();
        // Create the overwrite-clear PSO with the target's first full frame, so
        // its first partial frame doesn't compile a pipeline on hover.
        if retained.damage != Damage::Reuse {
            let device = renderer.resources().device.clone();
            renderer
                .resources_mut()
                .pipelines
                .ensure_damage_clear(&device);
        }
    }
    // Prune once against the complete scene. Doing this inside each draw batch would evict a
    // texture that reappears after an intervening primitive and recreate its platform view.
    renderer.retain_surface_cache(&scene.surfaces);

    let retained_blit = retained.blit.take().filter(|_| retained_copy.is_none());
    #[cfg(test)]
    {
        renderer.last_retained_presentation = if retained_copy.is_some() {
            "copy"
        } else if retained_blit.is_some() {
            "blit"
        } else {
            "direct"
        };
    }
    let result = FrameEncoder::new(
        renderer,
        scene,
        targets,
        retained.damage,
        retained_blit,
        retained_copy,
    )
    .encode(readback);
    renderer.retained.complete(retained, result.is_ok());
    match result {
        Ok(command_buffer) => {
            let submission = renderer.resources().queue.submit([command_buffer]);
            // A cleanup skipped a frame. Only a real submission without a new validation
            // error can reset that retry budget; merely starting the next frame cannot.
            if renderer.faults.cleaned_up_failed_frames
                && renderer.faults.pending_error.lock().unwrap().is_none()
            {
                renderer.faults.consecutive_failed_frames = 0;
                renderer.faults.cleaned_up_failed_frames = false;
            }
            Some(submission)
        }
        Err(DrawError::ExternalSurface) => None,
        Err(DrawError::CapacityPlanningInvariant) => {
            log::error!("frame storage exceeded its precomputed capacity");
            None
        }
        Err(DrawError::MissingIntermediateTarget) => {
            log::error!("frame preparation did not create a required intermediate target");
            None
        }
    }
}

pub(super) struct ReadbackCopy<'a> {
    pub(super) texture: &'a wgpu::Texture,
    pub(super) buffer: &'a wgpu::Buffer,
    pub(super) bytes_per_row: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

struct PreparedTargets {
    active: wgpu::TextureView,
    presentation: wgpu::TextureView,
    offscreen: Option<wgpu::TextureView>,
    instances: InstanceUpload,
}

impl PreparedTargets {
    fn prepare(
        renderer: &mut WgpuRenderer,
        scene: &Scene,
        frame_view: &wgpu::TextureView,
        requirements: &FrameRequirements,
    ) -> Option<Self> {
        let device = renderer.resources().device.clone();
        let resources = renderer.resources_mut();
        if !resources.instances.ensure_capacity(
            &device,
            &resources.bind_group_layouts,
            requirements.storage_bytes,
            requirements.instance_batches,
        ) {
            return None;
        }
        let instances = {
            let resources = renderer.resources();
            resources.instances.begin_upload(
                &resources.queue,
                requirements.storage_bytes,
                requirements.instance_batches,
            )?
        };
        if !renderer.ensure_uniform_capacity(requirements.uniforms) {
            return None;
        }

        if requirements.planned.uses_path_target {
            renderer.ensure_path_textures(scene);
        }
        if requirements.planned.uses_offscreen_target {
            renderer.ensure_filter_textures(&requirements.planned);
        }
        write_shader_globals(renderer);

        if requirements.planned.uses_offscreen_target {
            let resources = renderer.resources();
            let offscreen = resources
                .scene_color_view
                .as_ref()
                .expect("blur texture preparation must create a scene target")
                .clone();
            Some(Self {
                active: offscreen.clone(),
                presentation: frame_view.clone(),
                offscreen: Some(offscreen),
                instances,
            })
        } else {
            Some(Self {
                active: frame_view.clone(),
                presentation: frame_view.clone(),
                offscreen: None,
                instances,
            })
        }
    }
}

fn begin_frame(renderer: &mut WgpuRenderer) -> bool {
    // Release the callback mutex before atlas uploads can report another GPU error.
    let pending_error = renderer.faults.pending_error.lock().unwrap().take();
    let Some(error) = pending_error else {
        if !renderer.faults.cleaned_up_failed_frames {
            renderer.faults.consecutive_failed_frames = 0;
        }
        renderer.atlas.before_frame();
        return true;
    };

    handle_gpu_error(renderer, error)
}

pub(super) fn handle_gpu_error(renderer: &mut WgpuRenderer, error: String) -> bool {
    renderer.release_retained_frame();
    renderer.faults.consecutive_failed_frames += 1;
    log::error!(
        "GPU error during frame (failure {} of 10): {error}",
        renderer.faults.consecutive_failed_frames
    );
    if renderer.faults.consecutive_failed_frames > 10 {
        if renderer.reports_terminal_errors() {
            renderer.faults.terminal_error = Some(format!(
                "too many consecutive GPU errors; last error: {error}"
            ));
            return false;
        }
        panic!("too many consecutive GPU errors; last error: {error}");
    }
    if renderer.faults.consecutive_failed_frames > 5 && !renderer.faults.cleaned_up_failed_frames {
        if let Some(resources) = renderer.resources.as_mut() {
            resources.invalidate_intermediate_textures();
        }
        renderer.atlas.clear();
        renderer.target.request_redraw();
        renderer.faults.cleaned_up_failed_frames = true;
        if !renderer.reports_terminal_errors() {
            // Preserve the existing retry policy for other platform integrations.
            renderer.faults.consecutive_failed_frames = 0;
            renderer.faults.cleaned_up_failed_frames = false;
        }
        return false;
    }

    renderer.atlas.before_frame();
    true
}

#[cfg(all(test, feature = "test-support", target_os = "windows"))]
mod failure_tests {
    use super::*;

    #[test]
    fn persistent_dx12_errors_become_terminal_and_a_healthy_frame_resets_the_count() {
        let context = crate::WgpuContext::new_headless(None).unwrap();
        let mut renderer = WgpuRenderer::new_headless(
            &context,
            gpui::size(gpui::DevicePixels(16), gpui::DevicePixels(16)),
        )
        .unwrap();
        for _ in 0..18 {
            handle_gpu_error(&mut renderer, "injected validation failure".into());
        }
        assert_eq!(renderer.faults.consecutive_failed_frames, 0);
        assert!(renderer.terminal_error().is_none());
        renderer.native_backend = Some(crate::NativeBackend::Dx12);
        for _ in 0..6 {
            handle_gpu_error(&mut renderer, "injected validation failure".into());
        }
        assert!(renderer.faults.cleaned_up_failed_frames);
        assert!(begin_frame(&mut renderer));
        assert_eq!(renderer.faults.consecutive_failed_frames, 6);
        let mut scene = Scene::default();
        scene.finish();
        renderer.render_to_image(&scene).unwrap();
        assert_eq!(renderer.faults.consecutive_failed_frames, 0);
        for _ in 0..10 {
            handle_gpu_error(&mut renderer, "injected validation failure".into());
        }
        assert!(renderer.terminal_error().is_none());
        assert!(!handle_gpu_error(
            &mut renderer,
            "injected validation failure".into()
        ));
        assert!(
            renderer
                .terminal_error()
                .unwrap()
                .contains("validation failure")
        );
        // The error can be consumed without holding a callback/GPU mutex across application notification.
        assert!(renderer.faults.fatal_error.try_lock().is_ok());
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct GlobalUniformState {
    globals: GlobalUniforms,
    path_globals: GlobalUniforms,
    font_rasterization: FontRasterizationUniforms,
}

fn write_shader_globals(renderer: &mut WgpuRenderer) {
    let font = renderer.rendering_params.font_rasterization;
    let font_rasterization = FontRasterizationUniforms {
        gamma_ratios: wgsl_rs::std::vec4f(
            font.gamma_ratios[0],
            font.gamma_ratios[1],
            font.gamma_ratios[2],
            font.gamma_ratios[3],
        ),
        grayscale_enhanced_contrast: font.grayscale_enhanced_contrast,
        subpixel_enhanced_contrast: font.subpixel_enhanced_contrast,
        uses_blue_green_red_subpixel_order: ShaderBool::from(
            renderer.subpixel_order == super::SubpixelOrder::BlueGreenRed,
        ),
        padding: 0,
    };
    let globals = GlobalUniforms {
        viewport_size: wgsl_rs::std::vec2f(
            renderer.target.width() as f32,
            renderer.target.height() as f32,
        ),
        premultiplied_alpha: ShaderBool::from(
            renderer.target.alpha_mode() == wgpu::CompositeAlphaMode::PreMultiplied,
        ),
        padding: 0,
    };
    let path_globals = GlobalUniforms {
        viewport_size: renderer
            .resources()
            .path_intermediate_texture
            .as_ref()
            .map(|texture| wgsl_rs::std::vec2f(texture.width() as f32, texture.height() as f32))
            .unwrap_or(globals.viewport_size),
        premultiplied_alpha: ShaderBool::Disabled,
        ..globals
    };
    let state = GlobalUniformState {
        globals,
        path_globals,
        font_rasterization,
    };
    if renderer.uploaded_globals == Some(state) {
        return;
    }

    let resources = renderer.resources();
    let globals_size = std::mem::size_of::<GlobalUniforms>();
    let font_size = std::mem::size_of::<FontRasterizationUniforms>();
    let upload_size = renderer.globals.font_offset + font_size as u64;
    let mut upload = resources
        .queue
        .write_buffer_with(
            &resources.globals_buffer,
            0,
            std::num::NonZeroU64::new(upload_size).expect("global uniforms are non-empty"),
        )
        .expect("global uniform upload must fit its buffer");
    upload.slice(..).fill(0);
    upload
        .slice(..globals_size)
        .copy_from_slice(shader_interface::bytes_of(&globals));
    upload
        .slice(
            renderer.globals.path_offset as usize
                ..renderer.globals.path_offset as usize + globals_size,
        )
        .copy_from_slice(shader_interface::bytes_of(&path_globals));
    upload
        .slice(
            renderer.globals.font_offset as usize
                ..renderer.globals.font_offset as usize + font_size,
        )
        .copy_from_slice(shader_interface::bytes_of(&font_rasterization));
    drop(upload);
    renderer.uploaded_globals = Some(state);
}

#[derive(Clone, Copy, Default)]
pub(super) struct FrameRequirements {
    storage_bytes: u64,
    /// Instance batches this frame; one downlevel range-uniform slot per batch.
    instance_batches: u64,
    pub(super) uniforms: FrameUniformRequirements,
    planned: ScenePlanRequirements,
    smoothing: SmoothingRequirements,
}

impl FrameRequirements {
    /// What a frame uploads: the whole scene, the batches a partial frame
    /// repaints, or nothing when it reuses the retained frame.
    fn for_damage(scene: &Scene, damage: &Damage, transport: InstanceTransport) -> Self {
        match damage {
            Damage::Full => Self::for_scene(scene, transport),
            Damage::Partial(region) => Self::for_partial_scene(scene, *region, transport),
            Damage::Reuse => Self::default(),
        }
    }

    pub(super) fn for_scene(scene: &Scene, transport: InstanceTransport) -> Self {
        let planned = scene.render_plan().requirements();
        let mut requirements = Self::scene_metadata(scene);

        for command in scene.render_commands() {
            if let RenderCommand::Batch(batch) = command {
                requirements.reserve_batch(batch, transport);
            }
            requirements.include_filter_smoothing(scene, command);
        }
        debug_assert_eq!(
            requirements.instance_batches as usize,
            planned.instance_batch_count
        );

        requirements
    }

    fn scene_metadata(scene: &Scene) -> Self {
        let planned = scene.render_plan().requirements();
        Self {
            uniforms: FrameUniformRequirements {
                filter_count: FILTER_UNIFORMS_PER_COMPOSITE
                    * (planned.backdrop_filter_count + planned.isolated_filter_count) as u64
                    + u64::from(planned.uses_offscreen_target)
                    + if planned.isolated_target_count > 0 {
                        (planned.backdrop_filter_count * (planned.isolated_target_count + 1)) as u64
                    } else {
                        0
                    },
                surface_count: planned.surface_count as u64,
            },
            planned: *planned,
            ..Default::default()
        }
    }

    fn for_partial_scene(
        scene: &Scene,
        region: ScissorRectangle,
        transport: InstanceTransport,
    ) -> Self {
        let mut requirements = Self::scene_metadata(scene);
        // Encoding prepends a clear and skips nonintersecting batches. Either
        // changes the alignment of every following type, so a full-frame
        // estimate can't be reused.
        requirements.reserve(std::mem::size_of::<Quad>(), 1, transport);
        for command in scene.render_commands() {
            if let RenderCommand::Batch(batch) = command {
                if !batch_intersects(scene, batch, region) {
                    continue;
                }
                requirements.reserve_batch(batch, transport);
            }
            requirements.include_filter_smoothing(scene, command);
        }
        requirements
    }

    fn include_filter_smoothing(&mut self, scene: &Scene, command: &RenderCommand) {
        if self.smoothing.blur_composite {
            return;
        }
        self.smoothing.blur_composite = match command {
            RenderCommand::Batch(PrimitiveBatch::BackdropFilters(range)) => scene.backdrop_filters
                [range.clone()]
            .iter()
            .any(|filter| filter.corner_smoothing > 0.0 && filter.max_blur_radius() > 0.0),
            RenderCommand::EndFilter {
                boundary_index,
                target: FilterRenderTarget::Isolated(_),
                ..
            } => {
                let boundary = &scene.filter_boundaries[*boundary_index];
                // Isolated groups composite even without blur (for example, opacity
                // groups), and select the smoothing variant from this value alone.
                boundary.corner_smoothing > 0.0
            }
            _ => false,
        };
    }

    fn reserve(&mut self, element_size: usize, count: usize, transport: InstanceTransport) {
        if count > 0 {
            let stride = element_size as u64;
            self.storage_bytes = self
                .storage_bytes
                .next_multiple_of(transport.batch_alignment(stride));
            self.storage_bytes = self
                .storage_bytes
                .saturating_add(stride.saturating_mul(count as u64));
            self.instance_batches += 1;
        }
    }

    fn reserve_batch(&mut self, batch: &PrimitiveBatch, transport: InstanceTransport) {
        match batch {
            PrimitiveBatch::Shadows { range, smoothed } => {
                self.smoothing.shadows |= *smoothed && !range.is_empty();
                self.reserve(std::mem::size_of::<Shadow>(), range.len(), transport)
            }
            PrimitiveBatch::Quads { range, smoothed } => {
                self.smoothing.quads |= *smoothed && !range.is_empty();
                self.reserve(std::mem::size_of::<Quad>(), range.len(), transport)
            }
            PrimitiveBatch::Paths {
                rasterization_vertex_count,
                sprite_count,
                ..
            } if *rasterization_vertex_count > 0 => {
                self.reserve(
                    std::mem::size_of::<path_types::PathRasterizationVertex>(),
                    *rasterization_vertex_count,
                    transport,
                );
                self.reserve(
                    std::mem::size_of::<path_types::PathSprite>(),
                    *sprite_count,
                    transport,
                );
            }
            PrimitiveBatch::Underlines(range) => {
                self.reserve(std::mem::size_of::<Underline>(), range.len(), transport)
            }
            PrimitiveBatch::MonochromeSprites { range, .. } => self.reserve(
                std::mem::size_of::<MonochromeSprite>(),
                range.len(),
                transport,
            ),
            PrimitiveBatch::SubpixelSprites { range, .. } => self.reserve(
                std::mem::size_of::<SubpixelSprite>(),
                range.len(),
                transport,
            ),
            PrimitiveBatch::PolychromeSprites {
                range, smoothed, ..
            } => {
                self.smoothing.polychrome_sprites |= *smoothed && !range.is_empty();
                self.reserve(
                    std::mem::size_of::<PolychromeSprite>(),
                    range.len(),
                    transport,
                )
            }
            PrimitiveBatch::Paths { .. }
            | PrimitiveBatch::Surfaces(_)
            | PrimitiveBatch::BackdropFilters(_)
            | PrimitiveBatch::FilterBoundary(_) => {}
        }
    }
}

#[cfg(test)]
mod planning_tests {
    use super::*;

    #[test]
    fn partial_alignment_includes_the_clear() {
        let transport = InstanceTransport::StorageBuffer;
        let sequence = [
            (std::mem::size_of::<Shadow>(), 1),
            (std::mem::size_of::<Quad>(), 1),
            (std::mem::size_of::<Shadow>(), 1),
            (std::mem::size_of::<Quad>(), 1),
            (
                std::mem::size_of::<path_types::PathRasterizationVertex>(),
                3,
            ),
            (std::mem::size_of::<path_types::PathSprite>(), 1),
            (std::mem::size_of::<MonochromeSprite>(), 1),
            (std::mem::size_of::<Quad>(), 1),
        ];
        let mut full = FrameRequirements::default();
        for &(stride, count) in &sequence {
            full.reserve(stride, count, transport);
        }
        assert_eq!(full.storage_bytes, 1984);
        let old_estimate = full.storage_bytes + 2 * std::mem::size_of::<Quad>() as u64;
        assert_eq!(old_estimate, 2480);
        let mut partial = FrameRequirements::default();
        partial.reserve(std::mem::size_of::<Quad>(), 1, transport);
        for &(stride, count) in &sequence {
            partial.reserve(stride, count, transport);
        }
        assert_eq!(partial.storage_bytes, 2728);
        assert_eq!(partial.instance_batches, 9);
        assert!(partial.storage_bytes > old_estimate);
    }

    #[test]
    fn partial_planner_counts_only_intersecting_batches_and_clear_slots() {
        use gpui::{Bounds, ContentMask, ScaledPixels, point, size};
        let bounds = |x, y| {
            Bounds::new(
                point(ScaledPixels(x), ScaledPixels(y)),
                size(ScaledPixels(10.0), ScaledPixels(10.0)),
            )
        };
        let mask = ContentMask {
            bounds: Bounds::new(
                point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size(ScaledPixels(100.0), ScaledPixels(100.0)),
            ),
            ..Default::default()
        };
        let mut scene = Scene::default();
        for (index, (x, y)) in [(0.0, 0.0), (20.0, 20.0)].into_iter().enumerate() {
            scene.insert_primitive(Quad {
                bounds: bounds(x, y),
                content_mask: mask,
                corner_smoothing: if index == 0 { 0.0 } else { 0.65 },
                ..Default::default()
            });
        }
        scene.finish();
        let region = |x, y| ScissorRectangle {
            x,
            y,
            width: 5,
            height: 5,
        };
        for transport in [
            InstanceTransport::StorageBuffer,
            InstanceTransport::DataTexture,
        ] {
            // The clear, then only the batch under the region: the second quad's.
            let requirements =
                FrameRequirements::for_partial_scene(&scene, region(24, 24), transport);
            let mut expected = FrameRequirements::default();
            expected.reserve(std::mem::size_of::<Quad>(), 1, transport);
            expected.reserve(std::mem::size_of::<Quad>(), 1, transport);
            assert_eq!(requirements.instance_batches, 2);
            assert_eq!(requirements.storage_bytes, expected.storage_bytes);
            assert!(!requirements.planned.uses_path_target);
            assert!(!requirements.planned.uses_offscreen_target);
            assert!(requirements.smoothing.quads);
            assert_eq!(
                FrameRequirements::for_partial_scene(&scene, region(2, 2), transport).smoothing,
                SmoothingRequirements::default(),
                "an ordinary batch must not prepare the distant smoothing pipeline"
            );
            // A region over neither quad replays only the clear.
            let empty = FrameRequirements::for_partial_scene(&scene, region(60, 60), transport);
            assert_eq!(empty.instance_batches, 1);
            assert_eq!(empty.smoothing, SmoothingRequirements::default());
            // A frame uploads once, sized by its damage; a reused frame uploads nothing.
            let partial =
                FrameRequirements::for_damage(&scene, &Damage::Partial(region(24, 24)), transport);
            assert_eq!(partial.storage_bytes, requirements.storage_bytes);
            let reuse = FrameRequirements::for_damage(&scene, &Damage::Reuse, transport);
            assert_eq!((reuse.storage_bytes, reuse.instance_batches), (0, 0));
            assert_eq!(reuse.smoothing, SmoothingRequirements::default());
        }
    }

    #[test]
    fn smoothing_planner_includes_backdrop_and_isolated_filter_composites() {
        use gpui::{
            BackdropFilter, Bounds, ContentMask, Corners, FilterBoundary, ScaledFilter,
            ScaledPixels, point, size,
        };
        let bounds = Bounds::new(
            point(ScaledPixels(0.0), ScaledPixels(0.0)),
            size(ScaledPixels(40.0), ScaledPixels(40.0)),
        );
        let content_mask = ContentMask {
            bounds,
            ..Default::default()
        };
        for isolated in [false, true] {
            for (smoothing, radius, expected) in
                [(0.0, 4.0, false), (0.7, 0.0, isolated), (0.7, 4.0, true)]
            {
                let mut scene = Scene::default();
                let filters = smallvec::smallvec![ScaledFilter::Blur(ScaledPixels(radius))];
                if isolated {
                    let boundary = FilterBoundary {
                        order: 0,
                        bounds,
                        content_mask,
                        corner_radii: Corners::all(ScaledPixels(8.0)),
                        corner_smoothing: smoothing,
                        filters,
                        opacity: 1.0,
                        is_start: true,
                    };
                    scene.insert_primitive(boundary.clone());
                    scene.raise_order_floor();
                    scene.insert_primitive(Quad {
                        bounds,
                        content_mask,
                        background: gpui::white().into(),
                        ..Default::default()
                    });
                    scene.raise_order_floor();
                    scene.insert_primitive(FilterBoundary {
                        is_start: false,
                        ..boundary
                    });
                } else {
                    scene.insert_primitive(BackdropFilter {
                        order: 0,
                        bounds,
                        content_mask,
                        corner_radii: Corners::all(ScaledPixels(8.0)),
                        corner_smoothing: smoothing,
                        filters,
                        opacity: 1.0,
                    });
                }
                scene.finish();
                let required =
                    FrameRequirements::for_scene(&scene, InstanceTransport::StorageBuffer);
                assert_eq!(
                    required.smoothing.blur_composite, expected,
                    "isolated={isolated}, smoothing={smoothing}, radius={radius}"
                );
                assert!(!required.smoothing.quads);
                assert_eq!(
                    FrameRequirements::for_damage(
                        &scene,
                        &Damage::Reuse,
                        InstanceTransport::StorageBuffer
                    )
                    .smoothing,
                    SmoothingRequirements::default()
                );
            }
        }
    }
}

struct FrameEncoder<'a> {
    renderer: &'a WgpuRenderer,
    scene: &'a Scene,
    encoder: wgpu::CommandEncoder,
    targets: TargetStack<wgpu::TextureView>,
    offscreen: Option<wgpu::TextureView>,
    presentation: wgpu::TextureView,
    instances: InstanceUpload,
    damage: Damage,
    retained_blit: Option<wgpu::BindGroup>,
    retained_copy: Option<(wgpu::Texture, wgpu::Texture)>,
}

impl<'a> FrameEncoder<'a> {
    fn new(
        renderer: &'a WgpuRenderer,
        scene: &'a Scene,
        targets: PreparedTargets,
        damage: Damage,
        retained_blit: Option<wgpu::BindGroup>,
        retained_copy: Option<(wgpu::Texture, wgpu::Texture)>,
    ) -> Self {
        let encoder =
            renderer
                .resources()
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("gpui_frame"),
                });
        Self {
            renderer,
            scene,
            encoder,
            targets: TargetStack::new(targets.active),
            offscreen: targets.offscreen,
            presentation: targets.presentation,
            instances: targets.instances,
            damage,
            retained_blit,
            retained_copy,
        }
    }

    fn encode(
        mut self,
        readback: Option<ReadbackCopy<'_>>,
    ) -> Result<wgpu::CommandBuffer, DrawError> {
        let result = self.encode_commands();
        if result.is_ok() {
            if let Some((source, destination)) = &self.retained_copy {
                self.encoder.copy_texture_to_texture(
                    source.as_image_copy(),
                    destination.as_image_copy(),
                    wgpu::Extent3d {
                        width: self.renderer.target.width(),
                        height: self.renderer.target.height(),
                        depth_or_array_layers: 1,
                    },
                );
            }
            if let Some(bind_group) = &self.retained_blit {
                self.renderer
                    .draw_blit(&mut self.encoder, bind_group, 0, &self.presentation);
            }
            if let Some(offscreen) = &self.offscreen {
                self.renderer
                    .blit_to_frame(&mut self.encoder, offscreen, &self.presentation);
            }
            if let Some(readback) = readback {
                self.encoder.copy_texture_to_buffer(
                    readback.texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: readback.buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(readback.bytes_per_row),
                            rows_per_image: Some(readback.height),
                        },
                    },
                    wgpu::Extent3d {
                        width: readback.width,
                        height: readback.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        self.instances.finish(&mut self.encoder);
        self.renderer.resources().finish_frame_uploads();
        let command_buffer = self.encoder.finish();
        result.map(|()| command_buffer)
    }

    fn encode_commands(&mut self) -> DrawResult {
        match std::mem::replace(&mut self.damage, Damage::Reuse) {
            Damage::Reuse => Ok(()),
            Damage::Partial(region) => self.encode_region(Some(region)),
            Damage::Full => self.encode_region(None),
        }
    }
    fn encode_region(&mut self, region: Option<ScissorRectangle>) -> DrawResult {
        let mut pass = begin_scene_render_pass(
            self.renderer,
            &mut self.encoder,
            "main_pass",
            self.targets.current(),
            if region.is_some() {
                wgpu::LoadOp::Load
            } else {
                wgpu::LoadOp::Clear(self.renderer.target.clear_color())
            },
            region,
        );
        if let Some(region) = region {
            let clear = self.renderer.target.clear_color();
            let clear = [clear.r, clear.g, clear.b, clear.a].map(|channel| channel as f32);
            self.renderer.draw_damage_clear(
                &[clear_quad(region, clear)],
                &mut self.instances,
                &mut pass,
            )?;
        }

        for command in self.scene.render_commands() {
            if let (Some(region), RenderCommand::Batch(batch)) = (region, command) {
                if !batch_intersects(self.scene, batch, region) {
                    continue;
                }
            }
            match command {
                RenderCommand::Batch(PrimitiveBatch::Paths {
                    range,
                    rasterization_vertex_count,
                    ..
                }) => {
                    if *rasterization_vertex_count == 0 {
                        continue;
                    }
                    let paths = &self.scene.paths[range.clone()];
                    drop(pass);
                    let rasterized = self.renderer.draw_paths_to_intermediate(
                        &mut self.encoder,
                        paths,
                        &mut self.instances,
                    );
                    pass = begin_scene_render_pass(
                        self.renderer,
                        &mut self.encoder,
                        "after_paths",
                        self.targets.current(),
                        wgpu::LoadOp::Load,
                        region,
                    );
                    rasterized?;
                    self.renderer.draw_paths_from_intermediate(
                        paths,
                        &mut self.instances,
                        &mut pass,
                    )?;
                }
                RenderCommand::Batch(PrimitiveBatch::BackdropFilters(range)) => {
                    drop(pass);
                    for filter in &self.scene.backdrop_filters[range.clone()] {
                        let snapshot = self.targets.inherits_backdrop().then(|| {
                            self.renderer.snapshot_backdrop(
                                &mut self.encoder,
                                self.targets.backdrop_layers(),
                            )
                        });
                        let source = snapshot.as_ref().unwrap_or_else(|| self.targets.current());
                        self.renderer.blur_and_composite(
                            &mut self.encoder,
                            source,
                            self.targets.current(),
                            FilterCompositeParameters {
                                bounds: filter.bounds,
                                content_mask: filter.content_mask.bounds,
                                corner_radii: filter.corner_radii,
                                corner_smoothing: filter.corner_smoothing,
                                blur_radius: filter.max_blur_radius(),
                                opacity: filter.opacity,
                                clip: FilterCompositeClip::RoundedBounds,
                            },
                        );
                    }
                    pass = begin_scene_render_pass(
                        self.renderer,
                        &mut self.encoder,
                        "after_backdrop_filter",
                        self.targets.current(),
                        wgpu::LoadOp::Load,
                        region,
                    );
                }
                RenderCommand::Batch(PrimitiveBatch::FilterBoundary(_)) => {
                    unreachable!("filter boundaries must be compiled into render commands")
                }
                RenderCommand::Batch(batch) => encode_inline_batch(
                    self.renderer,
                    self.scene,
                    batch,
                    &mut self.instances,
                    &mut pass,
                )?,
                RenderCommand::BeginFilter {
                    boundary_index,
                    target: FilterRenderTarget::Isolated(index),
                } => {
                    drop(pass);
                    let target =
                        self.renderer.resources().filter_group_views[index.as_usize()].clone();
                    self.targets.enter(
                        target,
                        self.scene.filter_boundaries[*boundary_index]
                            .filters
                            .is_empty(),
                    );
                    pass = begin_scene_render_pass(
                        self.renderer,
                        &mut self.encoder,
                        "filter_group",
                        self.targets.current(),
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        region,
                    );
                }
                RenderCommand::EndFilter {
                    boundary_index,
                    target: FilterRenderTarget::Isolated(_),
                    ..
                } => {
                    drop(pass);
                    let (filtered, parent) = self.targets.exit();
                    let boundary = &self.scene.filter_boundaries[*boundary_index];
                    self.renderer.blur_and_composite(
                        &mut self.encoder,
                        &filtered,
                        parent,
                        FilterCompositeParameters {
                            bounds: boundary.bounds,
                            content_mask: boundary.content_mask.bounds,
                            corner_radii: boundary.corner_radii,
                            corner_smoothing: boundary.corner_smoothing,
                            blur_radius: boundary.max_blur_radius(),
                            opacity: boundary.opacity,
                            clip: FilterCompositeClip::ContentShape,
                        },
                    );
                    pass = begin_scene_render_pass(
                        self.renderer,
                        &mut self.encoder,
                        "after_content_filter",
                        self.targets.current(),
                        wgpu::LoadOp::Load,
                        region,
                    );
                }
                RenderCommand::BeginFilter {
                    target: FilterRenderTarget::Inline,
                    ..
                }
                | RenderCommand::EndFilter {
                    target: FilterRenderTarget::Inline,
                    ..
                } => {}
            }
        }
        drop(pass);
        self.targets.assert_balanced();
        Ok(())
    }
}

fn begin_scene_render_pass<'a>(
    renderer: &'a WgpuRenderer,
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'a str,
    target: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    region: Option<ScissorRectangle>,
) -> wgpu::RenderPass<'a> {
    let mut pass = begin_color_render_pass(encoder, label, target, load);
    if let Some(region) = region {
        pass.set_scissor_rect(region.x, region.y, region.width, region.height);
    }
    pass.set_bind_group(
        shader_interface::GLOBAL_BIND_GROUP,
        &renderer.resources().globals_bind_group,
        &[],
    );
    pass
}

#[derive(Debug)]
pub(super) enum DrawError {
    CapacityPlanningInvariant,
    ExternalSurface,
    MissingIntermediateTarget,
}

pub(super) type DrawResult = Result<(), DrawError>;

fn encode_inline_batch(
    renderer: &WgpuRenderer,
    scene: &Scene,
    batch: &PrimitiveBatch,
    instances: &mut InstanceUpload,
    pass: &mut wgpu::RenderPass<'_>,
) -> DrawResult {
    match batch {
        PrimitiveBatch::Quads { range, smoothed } => {
            renderer.draw_quads(&scene.quads[range.clone()], *smoothed, instances, pass)
        }
        PrimitiveBatch::Shadows { range, smoothed } => {
            renderer.draw_shadows(&scene.shadows[range.clone()], *smoothed, instances, pass)
        }
        PrimitiveBatch::Underlines(range) => {
            renderer.draw_underlines(&scene.underlines[range.clone()], instances, pass)
        }
        PrimitiveBatch::MonochromeSprites { texture_id, range } => renderer
            .draw_monochrome_sprites(
                &scene.monochrome_sprites[range.clone()],
                *texture_id,
                instances,
                pass,
            ),
        PrimitiveBatch::SubpixelSprites { texture_id, range } => renderer.draw_subpixel_sprites(
            &scene.subpixel_sprites[range.clone()],
            *texture_id,
            instances,
            pass,
        ),
        PrimitiveBatch::PolychromeSprites {
            texture_id,
            range,
            smoothed,
        } => renderer.draw_polychrome_sprites(
            &scene.polychrome_sprites[range.clone()],
            *texture_id,
            *smoothed,
            instances,
            pass,
        ),
        PrimitiveBatch::Surfaces(range) => {
            renderer.draw_surfaces(&scene.surfaces[range.clone()], pass)
        }
        PrimitiveBatch::Paths { .. }
        | PrimitiveBatch::BackdropFilters(_)
        | PrimitiveBatch::FilterBoundary(_) => {
            unreachable!("pass-interrupting batches are handled by FrameEncoder")
        }
    }
}
