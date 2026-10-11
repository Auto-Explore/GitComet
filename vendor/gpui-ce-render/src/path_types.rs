//! Host-side ABI for path primitives, shared by every renderer backend.
//!
//! Layouts must match the WGSL in `shaders::paths`; stride assertions here and the
//! Naga layout checks in `build.rs` enforce that.

use crate::shaders::interface::{BufferData, StorageAbi, storage_abi};
use gpui::{Background, Bounds, Path, ScaledPixels};

#[derive(Clone, Debug)]
#[repr(C)]
pub struct PathSprite {
    pub bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct PathRasterizationVertex {
    pub xy_position: gpui::Point<ScaledPixels>,
    pub curve_position: gpui::Point<f32>,
    pub color: Background,
    pub bounds: Bounds<ScaledPixels>,
}

unsafe impl BufferData for PathSprite {
    const WGSL_TYPE: &'static str = "PathSprite";
}
unsafe impl BufferData for PathRasterizationVertex {
    const WGSL_TYPE: &'static str = "PathRasterizationVertex";
}

pub const STORAGE_ABI: &[StorageAbi] = &[
    storage_abi::<PathSprite>(),
    storage_abi::<PathRasterizationVertex>(),
];

/// Texture extent from the viewport origin through the visible path bounds.
/// Round up to 64 pixels to avoid reallocating for small geometry changes.
/// Rasterization and gradient coordinates remain in window pixels; sampling
/// uses the attachment's actual dimensions independently of the viewport.
pub fn path_target_extent(
    bounds: impl Iterator<Item = Bounds<ScaledPixels>>,
    viewport_width: u32,
    viewport_height: u32,
) -> (u32, u32) {
    let (right, bottom) = bounds.fold((0.0_f32, 0.0_f32), |(right, bottom), bounds| {
        if bounds.right().0 <= 0.0
            || bounds.bottom().0 <= 0.0
            || bounds.left().0 >= viewport_width as f32
            || bounds.top().0 >= viewport_height as f32
        {
            return (right, bottom);
        }
        (right.max(bounds.right().0), bottom.max(bounds.bottom().0))
    });
    let padded = |value: f32, limit: u32| {
        (value.ceil() as u32)
            .max(1)
            .saturating_add(63)
            .div_euclid(64)
            .saturating_mul(64)
            .min(limit.max(1))
    };
    (
        padded(right, viewport_width),
        padded(bottom, viewport_height),
    )
}

pub fn rasterization_vertex_count(paths: &[Path<ScaledPixels>]) -> usize {
    paths.iter().map(|path| path.vertices.len()).sum()
}

pub fn rasterization_vertices(
    paths: &[Path<ScaledPixels>],
) -> impl Iterator<Item = PathRasterizationVertex> + '_ {
    paths.iter().flat_map(|path| {
        let bounds = path.clipped_bounds();
        path.vertices
            .iter()
            .map(move |vertex| PathRasterizationVertex {
                xy_position: vertex.xy_position,
                curve_position: vertex.st_position,
                color: path.color,
                bounds,
            })
    })
}

pub fn sprites(paths: &[Path<ScaledPixels>]) -> PathSprites<'_> {
    let combined = paths.first().and_then(|first| {
        (paths.last().is_some_and(|path| path.order != first.order)).then(|| {
            paths
                .iter()
                .skip(1)
                .fold(first.clipped_bounds(), |bounds, path| {
                    bounds.union(&path.clipped_bounds())
                })
        })
    });
    PathSprites {
        paths: combined.is_none().then_some(paths.iter()),
        combined,
    }
}

pub struct PathSprites<'a> {
    paths: Option<std::slice::Iter<'a, Path<ScaledPixels>>>,
    combined: Option<Bounds<ScaledPixels>>,
}

impl Iterator for PathSprites<'_> {
    type Item = PathSprite;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(bounds) = self.combined.take() {
            return Some(PathSprite { bounds });
        }
        self.paths.as_mut()?.next().map(|path| PathSprite {
            bounds: path.clipped_bounds(),
        })
    }
}

pub fn sprite_count(paths: &[Path<ScaledPixels>]) -> usize {
    if paths.is_empty() {
        0
    } else if paths
        .last()
        .is_some_and(|path| path.order == paths[0].order)
    {
        paths.len()
    } else {
        1
    }
}

const _: () = {
    assert!(std::mem::size_of::<PathSprite>() == 16);
    assert!(std::mem::size_of::<PathRasterizationVertex>() == 104);
};
