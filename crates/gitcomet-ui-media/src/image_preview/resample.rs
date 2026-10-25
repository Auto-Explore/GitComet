use super::{filter::Sampling, source::Decoded};
use fast_image_resize::{ResizeOptions, Resizer};

pub(super) const TILE: u32 = 512;

/// Global output coordinates give adjacent tiles the same sample phase.
pub(super) fn raster_region(
    source: &image::RgbaImage,
    output: (u32, u32),
    region: (u32, u32, u32, u32),
    sampling: Sampling,
) -> Result<image::RgbaImage, String> {
    let (x, y, w, h) = region;
    let (sw, sh) = source.dimensions();
    if output.0 == 0
        || output.1 == 0
        || w == 0
        || h == 0
        || x.checked_add(w).is_none_or(|end| end > output.0)
        || y.checked_add(h).is_none_or(|end| end > output.1)
    {
        return Err("Invalid image tile dimensions".into());
    }
    if output == source.dimensions() {
        return Ok(image::imageops::crop_imm(source, x, y, w, h).to_image());
    }
    let dx = f64::from(sw) / f64::from(output.0);
    let dy = f64::from(sh) / f64::from(output.1);
    if sampling == Sampling::Nearest {
        return Ok(image::RgbaImage::from_fn(w, h, |i, j| {
            *source.get_pixel(
                (((f64::from(x + i) + 0.5) * dx) as u32).min(sw - 1),
                (((f64::from(y + j) + 0.5) * dy) as u32).min(sh - 1),
            )
        }));
    }
    let halo_x = sampling.support() * dx.max(1.0) + 2.0;
    let halo_y = sampling.support() * dy.max(1.0) + 2.0;
    let left = (f64::from(x) * dx - halo_x).floor().max(0.0) as u32;
    let top = (f64::from(y) * dy - halo_y).floor().max(0.0) as u32;
    let right = ((f64::from(x + w) * dx + halo_x).ceil() as u32).min(sw);
    let bottom = ((f64::from(y + h) * dy + halo_y).ceil() as u32).min(sh);
    if u64::from(right - left) * u64::from(bottom - top) * 16 > 32 * 1024 * 1024 && w == 1 && h == 1
    {
        // Even one output pixel can cover an enormous source. Accumulate it
        // directly instead of allocating an unbounded floating-point ROI.
        return Ok(image::RgbaImage::from_pixel(
            1,
            1,
            streaming_pixel(source, output, (x, y), sampling),
        ));
    }
    // Split the destination before materializing a large floating-point ROI.
    // Each stripe still includes its complete filter halo.
    if u64::from(right - left) * u64::from(bottom - top) * 16 > 32 * 1024 * 1024 && (w > 1 || h > 1)
    {
        let mut result = image::RgbaImage::new(w, h);
        let horizontal = w >= h;
        let first = if horizontal { w / 2 } else { h / 2 };
        let regions = if horizontal {
            [(x, y, first, h), (x + first, y, w - first, h)]
        } else {
            [(x, y, w, first), (x, y + first, w, h - first)]
        };
        for r in regions {
            let tile = raster_region(source, output, r, sampling)?;
            image::imageops::replace(&mut result, &tile, i64::from(r.0 - x), i64::from(r.1 - y));
        }
        return Ok(result);
    }
    let float_source: image::Rgba32FImage =
        image::ImageBuffer::from_fn(right - left, bottom - top, |i, j| {
            let p = source.get_pixel(left + i, top + j).0;
            let a = f32::from(p[3]) / 255.0;
            image::Rgba([
                f32::from(p[0]) / 255.0 * a,
                f32::from(p[1]) / 255.0 * a,
                f32::from(p[2]) / 255.0 * a,
                a,
            ])
        });
    let mut output_float = image::Rgba32FImage::new(w, h);
    let crop_x = f64::from(x) * dx - f64::from(left);
    let crop_y = f64::from(y) * dy - f64::from(top);
    // The last fractional tile can exceed the source by a few floating-point
    // ulps. Keep the crop inside the ROI without changing its sample phase.
    let crop_width = (f64::from(w) * dx).min(f64::from(right - left) - crop_x);
    let crop_height = (f64::from(h) * dy).min(f64::from(bottom - top) - crop_y);
    let options = ResizeOptions::new()
        .resize_alg(sampling.algorithm())
        .use_alpha(false)
        .crop(crop_x, crop_y, crop_width, crop_height);
    Resizer::new()
        .resize(&float_source, &mut output_float, Some(&options))
        .map_err(|e| e.to_string())?;
    Ok(image::RgbaImage::from_fn(w, h, |i, j| {
        let p = output_float.get_pixel(i, j).0;
        let a = p[3].clamp(0.0, 1.0);
        let channel = |c: f32| {
            if a > 1e-8 {
                (c / p[3] * 255.0).round().clamp(0.0, 255.0) as u8
            } else {
                0
            }
        };
        image::Rgba([
            channel(p[0]),
            channel(p[1]),
            channel(p[2]),
            (a * 255.0).round() as u8,
        ])
    }))
}

fn streaming_pixel(
    source: &image::RgbaImage,
    output: (u32, u32),
    pixel: (u32, u32),
    sampling: Sampling,
) -> image::Rgba<u8> {
    let dx = f64::from(source.width()) / f64::from(output.0);
    let dy = f64::from(source.height()) / f64::from(output.1);
    let center = (
        (f64::from(pixel.0) + 0.5) * dx,
        (f64::from(pixel.1) + 0.5) * dy,
    );
    let left = (center.0 - sampling.support() * dx.max(1.0))
        .floor()
        .max(0.0) as u32;
    let top = (center.1 - sampling.support() * dy.max(1.0))
        .floor()
        .max(0.0) as u32;
    let right = ((center.0 + sampling.support() * dx.max(1.0)).ceil() as u32).min(source.width());
    let bottom = ((center.1 + sampling.support() * dy.max(1.0)).ceil() as u32).min(source.height());
    let mut sum = [0.0; 4];
    let mut weights = 0.0;
    for y in top..bottom {
        let wy = sampling.weight((f64::from(y) + 0.5 - center.1) / dy.max(1.0));
        for x in left..right {
            let weight = wy * sampling.weight((f64::from(x) + 0.5 - center.0) / dx.max(1.0));
            let p = source.get_pixel(x, y).0;
            let a = f64::from(p[3]) / 255.0;
            for channel in 0..3 {
                sum[channel] += weight * a * f64::from(p[channel]);
            }
            sum[3] += weight * a;
            weights += weight;
        }
    }
    let channel = |i: usize| {
        if sum[3] > 1e-8 {
            (sum[i] / sum[3]).round().clamp(0.0, 255.0) as u8
        } else {
            0
        }
    };
    image::Rgba([
        channel(0),
        channel(1),
        channel(2),
        (sum[3] / weights * 255.0).round().clamp(0.0, 255.0) as u8,
    ])
}

pub(super) fn tile(
    source: &Decoded,
    output: (u32, u32),
    column: u32,
    row: u32,
    sampling: Sampling,
) -> Result<image::RgbaImage, String> {
    let x = column * TILE;
    let y = row * TILE;
    let w = TILE.min(output.0.saturating_sub(x));
    let h = TILE.min(output.1.saturating_sub(y));
    let mut pixels = match source {
        Decoded::Raster(image) => raster_region(image, output, (x, y, w, h), sampling)?,
        Decoded::Vector(tree, _) => super::svg::raster_region(tree, output, (x, y, w, h))?,
    };
    for p in pixels.pixels_mut() {
        p.0.swap(0, 2);
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_preview::filter::ImageSmoothFilter;
    #[test]
    fn bounded_memory_accumulation_matches_convolution() {
        let source = image::RgbaImage::from_fn(100, 83, |x, y| {
            image::Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                128,
                if x % 3 == 0 { 0 } else { 255 },
            ])
        });
        for filter in ImageSmoothFilter::ALL {
            let sampling = Sampling::Smooth(*filter);
            let whole = raster_region(&source, (7, 5), (0, 0, 7, 5), sampling).unwrap();
            for y in 0..5 {
                for x in 0..7 {
                    let streaming = streaming_pixel(&source, (7, 5), (x, y), sampling);
                    for (a, b) in streaming.0.into_iter().zip(whole.get_pixel(x, y).0) {
                        assert!(a.abs_diff(b) <= 1, "{filter:?}: {a} vs {b}");
                    }
                }
            }
        }
    }
    #[test]
    fn nearest_replicates_source_pixels_and_native_size_is_exact() {
        let src = image::RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba([(x * 127) as u8, (y * 127) as u8, 91, 255])
        });
        let scaled = raster_region(&src, (6, 6), (0, 0, 6, 6), Sampling::Nearest).unwrap();
        for y in 0..6 {
            for x in 0..6 {
                assert_eq!(scaled.get_pixel(x, y), src.get_pixel(x / 3, y / 3));
            }
        }
        for filter in ImageSmoothFilter::ALL {
            assert_eq!(
                raster_region(&src, (2, 2), (0, 0, 2, 2), Sampling::Smooth(*filter)).unwrap(),
                src
            );
        }
    }
    #[test]
    fn tiled_filters_match_whole_image_and_preserve_transparent_edge_color() {
        let src = image::RgbaImage::from_fn(47, 39, |x, y| {
            image::Rgba([255, 40, 0, if (x + y) % 3 == 0 { 0 } else { 255 }])
        });
        for filter in ImageSmoothFilter::ALL {
            for output in [(17, 13), (101, 83)] {
                let sampling = Sampling::Smooth(*filter);
                let whole =
                    raster_region(&src, output, (0, 0, output.0, output.1), sampling).unwrap();
                let part = raster_region(&src, output, (4, 3, 7, 6), sampling).unwrap();
                for y in 0..6 {
                    for x in 0..7 {
                        let p = part.get_pixel(x, y).0;
                        let reference = whole.get_pixel(x + 4, y + 3).0;
                        for c in 0..4 {
                            assert!(
                                p[c].abs_diff(reference[c]) <= 1,
                                "{filter:?}: {p:?} != {reference:?}"
                            );
                        }
                        if p[3] > 0 {
                            assert_eq!(p[0], 255);
                            assert!(p[1].abs_diff(40) <= 1);
                        }
                    }
                }
            }
        }
    }
}
