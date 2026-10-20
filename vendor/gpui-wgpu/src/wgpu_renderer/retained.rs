use super::resources::WgpuResources;
use gpui::Scene;
use gpui::profiler::gpu;
use gpui_render::damage::{Damage, SceneSnapshot};
use gpui_render::retained::{GpuMemoryReservation, RetainedScene, reserve_retained_bytes};
use gpui_render::shaders::{blur::BlurUniforms, interface::bytes_of};
use std::sync::Arc;

pub(super) struct RetainedFrame {
    enabled: bool,
    target: Option<Target>,
    scene: RetainedScene,
    #[cfg(test)]
    last_damage: &'static str,
}
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
    clear_color: wgpu::Color,
    copy_source: bool,
    /// Samples the target onto the presentation texture when it isn't copied.
    /// Owned here: a shared cache would keep the texture alive after a release.
    blit: wgpu::BindGroup,
    // Held for its drop, which releases the device budget.
    _reservation: GpuMemoryReservation,
}
pub(super) struct RetainedPlan {
    pub view: Option<wgpu::TextureView>,
    pub copy_texture: Option<wgpu::Texture>,
    pub blit: Option<wgpu::BindGroup>,
    pub damage: Damage,
    pub snapshot: Option<SceneSnapshot>,
    pub scene_revision: u64,
}
impl RetainedFrame {
    pub fn new() -> Self {
        Self {
            // Measured on Windows and Linux; the web and macOS builds draw in full.
            enabled: cfg!(any(
                target_os = "windows",
                target_os = "linux",
                target_os = "freebsd"
            )),
            target: None,
            scene: RetainedScene::default(),
            #[cfg(test)]
            last_damage: "full",
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    #[cfg(test)]
    pub fn enable_for_tests(&mut self) {
        self.enabled = true;
    }
    #[cfg(test)]
    pub fn disable_for_tests(&mut self) {
        self.enabled = false;
        self.invalidate();
    }
    #[cfg(test)]
    pub fn last_damage(&self) -> &'static str {
        self.last_damage
    }
    /// Forget the retained frame and free its target and budget at once: wgpu
    /// keeps a texture alive while submitted work uses it, and an idle or closed
    /// window would never poll a completion callback.
    pub fn invalidate(&mut self) {
        self.scene.forget();
        self.target = None;
    }
    pub fn prepare(
        &mut self,
        scene: &Scene,
        atlas: u64,
        width: u32,
        height: u32,
        copy_source: bool,
        resources: &WgpuResources,
        format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        clear_color: wgpu::Color,
    ) -> RetainedPlan {
        let context = gpu::gpu_frame_context();
        let device = &resources.device;
        let full = || RetainedPlan {
            view: None,
            copy_texture: None,
            blit: None,
            damage: Damage::Full,
            snapshot: None,
            scene_revision: context.scene_revision,
        };
        if !self.enabled {
            return full();
        }
        let bytes = retained_bytes(format, width, height);
        if !context.interactive
            || !alpha_supported(alpha_mode)
            // The damage-clear Quad must reproduce the raw target clear exactly.
            // Colored translucent clears would be premultiplied again by its shader;
            // arbitrary opaque colors also lack an exact HSL round-trip proof.
            || !retained_clear_supported(clear_color)
            || bytes.is_none()
        {
            self.invalidate();
            return full();
        }
        if self.target.as_ref().is_some_and(|target| {
            target.width != width
                || target.height != height
                || target.format != format
                || target.alpha_mode != alpha_mode
                || target.clear_color != clear_color
                || target.copy_source != copy_source
        }) {
            self.invalidate();
        }
        // Reserve before hashing, so a window the budget can't hold doesn't
        // capture its scene every frame only to draw in full.
        let reservation = if self.target.is_none() {
            let reservation = bytes.and_then(|bytes| {
                reserve_retained_bytes("wgpu", Arc::as_ptr(device) as usize, bytes)
            });
            if reservation.is_none() {
                self.invalidate();
                return full();
            }
            reservation
        } else {
            None
        };
        let Some((mut damage, snapshot)) =
            self.scene
                .plan(scene, context.scene_revision, atlas, width, height)
        else {
            self.invalidate();
            return full();
        };
        if let Some(reservation) = reservation {
            #[cfg(not(target_family = "wasm"))]
            let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gpui_retained_frame"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | if copy_source {
                        wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            });
            // The target is optional: without memory for it, draw in full, as
            // with an exhausted budget, instead of failing the device. The web
            // never retains frames.
            #[cfg(not(target_family = "wasm"))]
            if gpui::block_on(scope.pop()).is_some() {
                self.invalidate();
                return full();
            }
            let view = texture.create_view(&Default::default());
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpui_retained_blit"),
                size: std::mem::size_of::<BlurUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let size = [width as f32, height as f32];
            resources
                .queue
                .write_buffer(&uniforms, 0, bytes_of(&BlurUniforms::copy(size)));
            let blit = resources.bind_group_layouts.create_blur(
                device,
                uniforms.as_entire_buffer_binding(),
                &view,
                &resources.surface_sampler,
            );
            self.target = Some(Target {
                texture,
                view,
                width,
                height,
                format,
                alpha_mode,
                clear_color,
                copy_source,
                blit,
                _reservation: reservation,
            });
            // A new target holds nothing yet.
            damage = Damage::Full;
        }
        let target = self.target.as_ref().unwrap();
        RetainedPlan {
            view: Some(target.view.clone()),
            copy_texture: copy_source.then(|| target.texture.clone()),
            blit: Some(target.blit.clone()),
            damage,
            snapshot,
            scene_revision: context.scene_revision,
        }
    }
    pub fn complete(&mut self, plan: RetainedPlan, success: bool) {
        #[cfg(test)]
        {
            self.last_damage = plan.damage.kind();
        }
        if success {
            self.scene.complete(plan.snapshot, plan.scene_revision);
        } else {
            self.invalidate();
        }
    }
}

fn retained_clear_supported(clear: wgpu::Color) -> bool {
    // Both linear and sRGB targets preserve the black/white endpoints exactly.
    // Black RGB stays black through either premultiplied or straight-alpha output.
    let black = clear.r == 0.0 && clear.g == 0.0 && clear.b == 0.0;
    let opaque_white = clear.r == 1.0 && clear.g == 1.0 && clear.b == 1.0 && clear.a == 1.0;
    (black && clear.a.is_finite() && (0.0..=1.0).contains(&clear.a)) || opaque_white
}

/// The alpha modes partial redraw is verified with, copied or blitted.
pub(super) fn alpha_supported(alpha: wgpu::CompositeAlphaMode) -> bool {
    matches!(
        alpha,
        wgpu::CompositeAlphaMode::Opaque | wgpu::CompositeAlphaMode::PreMultiplied
    )
}

fn retained_bytes(format: wgpu::TextureFormat, width: u32, height: u32) -> Option<u64> {
    if width == 0
        || height == 0
        || !matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
                | wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
        )
    {
        return None;
    }
    (width as u64).checked_mul(height as u64)?.checked_mul(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_clear_requires_exact_shader_endpoint_representation() {
        assert!(retained_clear_supported(wgpu::Color::TRANSPARENT));
        assert!(retained_clear_supported(wgpu::Color::BLACK));
        assert!(retained_clear_supported(wgpu::Color::WHITE));
        assert!(retained_clear_supported(wgpu::Color {
            a: 0.25,
            ..wgpu::Color::BLACK
        }));
        for alpha in [0.0, 0.25, 1.0] {
            assert!(!retained_clear_supported(wgpu::Color {
                r: 0.04,
                g: 0.03,
                b: 0.02,
                a: alpha,
            }));
        }
        assert!(!retained_clear_supported(wgpu::Color {
            a: f64::NAN,
            ..wgpu::Color::BLACK
        }));
        assert!(!retained_clear_supported(wgpu::Color {
            a: -0.25,
            ..wgpu::Color::BLACK
        }));
    }

    #[test]
    fn only_opaque_and_premultiplied_targets_retain() {
        assert!(alpha_supported(wgpu::CompositeAlphaMode::Opaque));
        assert!(alpha_supported(wgpu::CompositeAlphaMode::PreMultiplied));
        assert!(!alpha_supported(wgpu::CompositeAlphaMode::PostMultiplied));
        assert!(!alpha_supported(wgpu::CompositeAlphaMode::Inherit));
    }

    #[test]
    fn retained_budget_rejects_hdr_unknown_formats_zero_extent_and_overflow() {
        assert_eq!(
            retained_bytes(wgpu::TextureFormat::Bgra8Unorm, 4096, 2048),
            Some(4096 * 2048 * 4)
        );
        assert_eq!(
            retained_bytes(wgpu::TextureFormat::Rgba16Float, 4096, 2048),
            None
        );
        assert_eq!(retained_bytes(wgpu::TextureFormat::Rgba8Unorm, 0, 8), None);
        assert_eq!(
            retained_bytes(wgpu::TextureFormat::Rgba8Unorm, u32::MAX, u32::MAX),
            None
        );
    }
}
