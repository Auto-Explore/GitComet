use super::{WgpuRenderer, pipelines::SmoothingRequirements};
use crate::WgpuContext;
use gpui::{
    AtlasKey, AtlasTile, BackdropFilter, Bounds, ContentMask, Corners, DevicePixels,
    FilterBoundary, ImageId, PlatformAtlas, PolychromeSprite, Quad, RenderImageParams,
    ScaledFilter, ScaledPixels, Scene, Shadow, point, size, solid_background,
};
use std::borrow::Cow;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

fn handles(renderer: &WgpuRenderer) -> [Option<wgpu::RenderPipeline>; 4] {
    let pipelines = &renderer.resources().pipelines;
    [
        &pipelines.smoothed_quads,
        &pipelines.smoothed_shadows,
        &pipelines.smoothed_polychrome_sprites,
        &pipelines.smoothed_blur_composite,
    ]
    .map(|pipeline| {
        pipeline
            .as_ref()
            .map(|pipeline| std::ops::Deref::deref(pipeline).clone())
    })
}

fn image_tile(renderer: &WgpuRenderer) -> anyhow::Result<AtlasTile> {
    renderer
        .atlas
        .get_or_insert_with(
            &AtlasKey::Image(RenderImageParams {
                image_id: ImageId(9081),
                frame_index: 0,
            }),
            &mut || {
                Ok(Some((
                    size(DevicePixels(8), DevicePixels(8)),
                    Cow::Owned(vec![255; 8 * 8 * 4]),
                )))
            },
        )?
        .ok_or_else(|| anyhow::anyhow!("test image must have pixels"))
}

fn scene(kind: usize, tile: AtlasTile) -> Scene {
    let mut scene = Scene::default();
    let shape = bounds(20.0, 20.0, 56.0, 56.0);
    let content_mask = ContentMask {
        bounds: bounds(0.0, 0.0, 96.0, 96.0),
        ..Default::default()
    };
    let corner_radii = Corners::all(ScaledPixels(18.0));
    match kind {
        0 => scene.insert_primitive(Quad {
            bounds: shape,
            content_mask,
            corner_radii,
            corner_smoothing: 0.75,
            background: solid_background(gpui::white()),
            ..Default::default()
        }),
        1 => scene.insert_primitive(Shadow {
            order: 0,
            bounds: shape,
            element_bounds: shape,
            content_mask,
            corner_radii,
            element_corner_radii: corner_radii,
            corner_smoothing: 0.75,
            color: solid_background(gpui::white()),
            blur_radius: ScaledPixels(5.0),
            inset: false.into(),
        }),
        2 => scene.insert_primitive(PolychromeSprite {
            order: 0,
            bounds: shape,
            content_mask,
            corner_radii,
            corner_smoothing: 0.75,
            grayscale: false.into(),
            opacity: 1.0,
            tile,
        }),
        3 => {
            scene.insert_primitive(Quad {
                bounds: shape,
                content_mask,
                background: gpui::checkerboard(gpui::white(), 2.0),
                ..Default::default()
            });
            scene.raise_order_floor();
            scene.insert_primitive(BackdropFilter {
                order: 0,
                bounds: shape,
                content_mask,
                corner_radii,
                corner_smoothing: 0.75,
                filters: smallvec::smallvec![ScaledFilter::Blur(ScaledPixels(5.0))],
                opacity: 1.0,
            });
        }
        4 | 5 => {
            let boundary = FilterBoundary {
                order: 0,
                bounds: shape,
                content_mask,
                corner_radii,
                corner_smoothing: 0.75,
                filters: if kind == 4 {
                    smallvec::smallvec![ScaledFilter::Blur(ScaledPixels(5.0))]
                } else {
                    // Upstream also composites zero-blur opacity groups.
                    smallvec::smallvec![]
                },
                opacity: 1.0,
                is_start: true,
            };
            scene.insert_primitive(boundary.clone());
            scene.raise_order_floor();
            scene.insert_primitive(Quad {
                bounds: shape,
                content_mask,
                background: solid_background(gpui::white()),
                ..Default::default()
            });
            scene.raise_order_floor();
            scene.insert_primitive(FilterBoundary {
                is_start: false,
                ..boundary
            });
        }
        _ => unreachable!(),
    }
    scene.finish();
    scene
}

#[test]
fn smoothing_pipelines_are_absent_until_first_use_and_match_prepared_pixels() -> anyhow::Result<()>
{
    let _gpu_test_guard = crate::test_gpu::guard();
    let context = WgpuContext::new_headless(None)?;
    let target = size(DevicePixels(96), DevicePixels(96));
    let mut lazy = WgpuRenderer::new_headless(&context, target)?;
    let mut prepared = WgpuRenderer::new_headless(&context, target)?;
    lazy.retained.disable_for_tests();
    prepared.retained.disable_for_tests();
    let tile = image_tile(&lazy)?;
    assert_eq!(tile, image_tile(&prepared)?);
    assert!(handles(&lazy).iter().all(Option::is_none));

    let mut ordinary = Scene::default();
    ordinary.insert_primitive(Quad {
        bounds: bounds(0.0, 0.0, 96.0, 96.0),
        content_mask: ContentMask {
            bounds: bounds(0.0, 0.0, 96.0, 96.0),
            ..Default::default()
        },
        background: solid_background(gpui::white()),
        ..Default::default()
    });
    ordinary.finish();
    assert_eq!(
        lazy.render_to_image(&ordinary)?.get_pixel(48, 48).0,
        [255; 4]
    );
    assert!(
        handles(&lazy).iter().all(Option::is_none),
        "ordinary frames need no smoothing"
    );

    prepared.resources_mut().pipelines.prepare_smoothing(
        &context.device,
        SmoothingRequirements {
            quads: true,
            shadows: true,
            polychrome_sprites: true,
            blur_composite: true,
        },
    )?;
    for kind in 0..6 {
        if kind >= 4 {
            // Target/device resource rebuilds must not retain old pipeline handles.
            lazy.rebuild_pipelines();
            assert!(handles(&lazy).iter().all(Option::is_none));
        }
        let scene = scene(kind, tile);
        let first_use = lazy.render_to_image(&scene)?;
        let reference = prepared.render_to_image(&scene)?;
        assert_eq!(
            first_use, reference,
            "first-use pixels for smoothing kind {kind}"
        );
        assert!(
            first_use.pixels().any(|pixel| pixel.0[..3] != [0; 3]),
            "kind {kind} must draw"
        );
        if kind == 0 || kind == 2 {
            assert_eq!(first_use.get_pixel(48, 48).0, [255; 4]);
            assert_eq!(
                first_use.get_pixel(20, 20).0,
                [0, 0, 0, 255],
                "the first smoothed fill/image must exclude its corner"
            );
        }
        let created = handles(&lazy);
        let expected = if kind >= 4 {
            [false, false, false, true]
        } else {
            std::array::from_fn(|index| index <= kind)
        };
        assert_eq!(created.each_ref().map(Option::is_some), expected);
        assert_eq!(lazy.render_to_image(&scene)?, first_use);
        assert_eq!(
            handles(&lazy),
            created,
            "second use must reuse pipeline handles"
        );
    }
    assert!(context.uncaptured_error_slot().lock().unwrap().is_none());
    Ok(())
}
