use super::*;
use crate::view::diff_utils::image_format_for_path;
use crate::view::image_preview::{limit_decoder, svg};
use image::{AnimationDecoder as _, ImageDecoder as _};
use rustc_hash::FxHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE_DIFF_CACHE_DIR_NAME: &str = "gitcomet-image-diff";
const IMAGE_DIFF_CACHE_FILE_PREFIX: &str = "gitcomet-image-diff-";
const IMAGE_DIFF_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);
const IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL: usize = 16;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX: u32 = 1920;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES: usize = 120;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;
const CONFLICT_RASTER_PREVIEW_MAX_EDGE_PX: u32 = 512;
const CONFLICT_RASTER_PREVIEW_MAX_BYTES: usize = 512 * 512 * 4;
const CONFLICT_RASTER_PREVIEW_MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;
const IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX: f32 = 640.0;
const IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX: f32 = 1024.0;
static IMAGE_DIFF_CACHE_STARTUP_CLEANUP: std::sync::Once = std::sync::Once::new();
static IMAGE_DIFF_CACHE_WRITE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug)]
struct RasterPreviewPolicy {
    max_edge_px: u32,
    max_frames: usize,
    max_retained_bytes: usize,
    max_decode_bytes: u64,
}

const FILE_DIFF_RASTER_PREVIEW_POLICY: RasterPreviewPolicy = RasterPreviewPolicy {
    max_edge_px: IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX,
    max_frames: IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES,
    max_retained_bytes: IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_BYTES,
    max_decode_bytes: IMAGE_DIFF_RASTER_PREVIEW_MAX_DECODE_BYTES,
};

const CONFLICT_RASTER_PREVIEW_POLICY: RasterPreviewPolicy = RasterPreviewPolicy {
    max_edge_px: CONFLICT_RASTER_PREVIEW_MAX_EDGE_PX,
    max_frames: 1,
    max_retained_bytes: CONFLICT_RASTER_PREVIEW_MAX_BYTES,
    max_decode_bytes: CONFLICT_RASTER_PREVIEW_MAX_DECODE_BYTES,
};

#[derive(Debug)]
struct ImageDiffCacheEntry {
    path: std::path::PathBuf,
    modified: std::time::SystemTime,
    size: u64,
}

fn cleanup_image_diff_cache_startup_once() {
    IMAGE_DIFF_CACHE_STARTUP_CLEANUP.call_once(cleanup_image_diff_cache_now);
}

fn maybe_cleanup_image_diff_cache_on_write() {
    let write_count = IMAGE_DIFF_CACHE_WRITE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if write_count.is_multiple_of(IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL) {
        cleanup_image_diff_cache_now();
    }
}

/// Previews live in an owner-only directory, not the shared temp root: the file
/// name is a content hash, so anyone able to create an entry there could plant
/// a name and have it served as a cache hit. The temp root is world-creatable,
/// so a directory already there is used only if nobody else can write to it.
fn image_diff_cache_dir() -> std::io::Result<std::path::PathBuf> {
    image_diff_cache_dir_in(&std::env::temp_dir())
}

fn image_diff_cache_dir_in(base: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let dir = base.join(IMAGE_DIFF_CACHE_DIR_NAME);
    gitcomet_core::fs_utils::ensure_private_dir(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if std::fs::metadata(&dir)?.permissions().mode() & 0o022 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "image diff cache dir is writable by others: {}",
                    dir.display()
                ),
            ));
        }
    }
    Ok(dir)
}

fn cleanup_image_diff_cache_now() {
    let Ok(cache_dir) = image_diff_cache_dir() else {
        return;
    };
    let _ = cleanup_image_diff_cache_dir(
        &cache_dir,
        IMAGE_DIFF_CACHE_MAX_AGE,
        IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES,
        std::time::SystemTime::now(),
    );
}

fn cleanup_image_diff_cache_dir(
    cache_dir: &std::path::Path,
    max_age: std::time::Duration,
    max_total_bytes: u64,
    now: std::time::SystemTime,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    let mut cache_entries = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };

        let file_name = entry.file_name();
        let Some(file_name_text) = file_name.to_str() else {
            continue;
        };
        if !file_name_text.starts_with(IMAGE_DIFF_CACHE_FILE_PREFIX) {
            continue;
        }

        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        if !metadata.is_file() {
            continue;
        }

        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
        let age = now.duration_since(modified).unwrap_or_default();
        if age > max_age {
            let _ = std::fs::remove_file(path);
            continue;
        }

        cache_entries.push(ImageDiffCacheEntry {
            path,
            modified,
            size: metadata.len(),
        });
    }

    let mut total_size = cache_entries
        .iter()
        .fold(0_u64, |acc, entry| acc.saturating_add(entry.size));
    if total_size <= max_total_bytes {
        return Ok(());
    }

    cache_entries.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });

    for entry in cache_entries {
        if total_size <= max_total_bytes {
            break;
        }
        if std::fs::remove_file(&entry.path).is_ok() {
            total_size = total_size.saturating_sub(entry.size);
        }
    }

    Ok(())
}

#[cfg(test)]
fn decode_file_image_diff_bytes(
    format: gpui::ImageFormat,
    bytes: &[u8],
    cached_path: Option<&mut Option<std::path::PathBuf>>,
) -> Option<Arc<gpui::Image>> {
    match format {
        gpui::ImageFormat::Svg => {
            if let Some(path) = cached_path {
                *path = Some(cached_image_diff_path(bytes, "svg")?);
            }
            None
        }
        _ => Some(Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))),
    }
}

#[derive(Clone, Default)]
struct DecodedImageDiffPreview {
    source: Option<Arc<crate::view::image_preview::Source>>,
    render: Option<Arc<gpui::RenderImage>>,
    cached_path: Option<std::path::PathBuf>,
}

fn swap_rgba_to_bgra(color: &mut [u8]) {
    color.swap(0, 2);
}

fn swap_rgba_pa_to_bgra(color: &mut [u8]) {
    swap_rgba_to_bgra(color);
    if color[3] > 0 {
        let a = color[3] as f32 / 255.0;
        color[0] = (color[0] as f32 / a).min(255.0) as u8;
        color[1] = (color[1] as f32 / a).min(255.0) as u8;
        color[2] = (color[2] as f32 / a).min(255.0) as u8;
    }
}

fn render_image_from_bgra8(buffer: image::RgbaImage) -> Arc<gpui::RenderImage> {
    Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buffer)]))
}

fn extend_transparent_edge_rgb(buffer: &mut image::RgbaImage) {
    let (width, height) = buffer.dimensions();
    if width == 0 || height == 0 {
        return;
    }

    // GPUI samples image-atlas textures with a linear filter and then blends
    // the sampled value as straight alpha. A transparent black texel beside a
    // coloured edge therefore attenuates RGB once during interpolation and a
    // second time during blending. Keep alpha untouched, but extend the edge's
    // straight RGB into the one-texel transparent neighbourhood that the
    // bilinear sampler can reach.
    //
    for y in 0..height {
        for x in 0..width {
            if buffer.get_pixel(x, y).0[3] != 0 {
                continue;
            }

            let mut weighted_rgb = [0_u32; 3];
            let mut total_alpha = 0_u32;
            let min_y = y.saturating_sub(1);
            let max_y = y.saturating_add(1).min(height - 1);
            let min_x = x.saturating_sub(1);
            let max_x = x.saturating_add(1).min(width - 1);

            for neighbour_y in min_y..=max_y {
                for neighbour_x in min_x..=max_x {
                    let neighbour = buffer.get_pixel(neighbour_x, neighbour_y).0;
                    let alpha = u32::from(neighbour[3]);
                    if alpha == 0 {
                        continue;
                    }
                    total_alpha += alpha;
                    for (weighted, channel) in weighted_rgb.iter_mut().zip(neighbour[..3].iter()) {
                        *weighted += u32::from(*channel) * alpha;
                    }
                }
            }

            let Some(total_alpha) = std::num::NonZeroU32::new(total_alpha) else {
                continue;
            };
            let divisor = total_alpha.get();
            let pixel = buffer.get_pixel_mut(x, y);
            for (channel, weighted) in pixel.0[..3].iter_mut().zip(weighted_rgb) {
                *channel = ((weighted + divisor / 2) / divisor) as u8;
            }
        }
    }
}

struct PremultipliedRgbaView<'a>(&'a image::RgbaImage);

impl image::GenericImageView for PremultipliedRgbaView<'_> {
    type Pixel = image::Rgba<u32>;

    fn dimensions(&self) -> (u32, u32) {
        self.0.dimensions()
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let pixel = self.0.get_pixel(x, y).0;
        let alpha = u32::from(pixel[3]);
        image::Rgba([
            u32::from(pixel[0]) * alpha,
            u32::from(pixel[1]) * alpha,
            u32::from(pixel[2]) * alpha,
            alpha,
        ])
    }
}

fn alpha_correct_thumbnail(buffer: image::RgbaImage, max_edge_px: u32) -> image::RgbaImage {
    let (width, height) = buffer.dimensions();
    if width == 0 || height == 0 || max_edge_px == 0 || width.max(height) <= max_edge_px {
        return buffer;
    }

    let scale =
        (f64::from(max_edge_px) / f64::from(width)).min(f64::from(max_edge_px) / f64::from(height));
    let resized_width = (f64::from(width) * scale).round().max(1.0) as u32;
    let resized_height = (f64::from(height) * scale).round().max(1.0) as u32;

    // The integer box sampler visits each source pixel once and allocates only
    // its output. RGB is multiplied by alpha lazily so transparent edge colors
    // cannot bleed into the downscaled result.
    let resized = image::imageops::thumbnail(
        &PremultipliedRgbaView(&buffer),
        resized_width,
        resized_height,
    );

    let mut straight_samples = Vec::with_capacity(resized.as_raw().len());
    for pixel in resized.pixels() {
        let alpha = pixel.0[3].min(255);
        for channel in 0..3 {
            let straight = (pixel.0[channel] + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
            straight_samples.push(straight);
        }
        straight_samples.push(alpha as u8);
    }

    image::RgbaImage::from_raw(resized_width, resized_height, straight_samples)
        .expect("RGBA sample count follows the resized dimensions")
}

fn raster_preview_dimensions(width: u32, height: u32, max_edge_px: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || max_edge_px == 0 {
        return None;
    }
    if width.max(height) <= max_edge_px {
        return Some((width, height));
    }
    let scale =
        (f64::from(max_edge_px) / f64::from(width)).min(f64::from(max_edge_px) / f64::from(height));
    Some((
        (f64::from(width) * scale).round().max(1.0) as u32,
        (f64::from(height) * scale).round().max(1.0) as u32,
    ))
}

fn raster_preview_frame_bytes(
    width: u32,
    height: u32,
    policy: RasterPreviewPolicy,
) -> Option<usize> {
    let (width, height) = raster_preview_dimensions(width, height, policy.max_edge_px)?;
    usize::try_from(
        u64::from(width)
            .checked_mul(u64::from(height))?
            .checked_mul(4)?,
    )
    .ok()
}

fn prepare_raster_preview_frame(
    mut frame: image::Frame,
    policy: RasterPreviewPolicy,
) -> image::Frame {
    let delay = frame.delay();
    let left = frame.left();
    let top = frame.top();
    let oversized = frame.buffer().width().max(frame.buffer().height()) > policy.max_edge_px;

    let mut buffer = if oversized {
        alpha_correct_thumbnail(frame.into_buffer(), policy.max_edge_px)
    } else {
        std::mem::take(frame.buffer_mut())
    };

    let mut has_fully_transparent_pixel = false;
    for pixel in buffer.as_chunks_mut::<4>().0 {
        has_fully_transparent_pixel |= pixel[3] == 0;
        swap_rgba_to_bgra(pixel);
    }
    if has_fully_transparent_pixel {
        extend_transparent_edge_rgb(&mut buffer);
    }

    image::Frame::from_parts(buffer, left, top, delay)
}

fn prepare_raster_preview_animation_frames(
    mut decoded_frames: impl Iterator<Item = image::ImageResult<image::Frame>>,
    dimensions: (u32, u32),
    orientation: image::metadata::Orientation,
    policy: RasterPreviewPolicy,
) -> Vec<image::Frame> {
    let Some(frame_bytes) = raster_preview_frame_bytes(dimensions.0, dimensions.1, policy) else {
        return Vec::new();
    };
    let max_frames = policy
        .max_frames
        .min(policy.max_retained_bytes / frame_bytes.max(1));
    let mut frames = Vec::new();
    let mut retained_bytes = 0_usize;

    // Pull and prepare one full-canvas frame at a time. In particular, do not
    // collect the decoder first: GIF and WebP decoders yield RGBA canvases, so
    // buffering all source frames can consume gigabytes before the preview
    // resize has a chance to run.
    while frames.len() < max_frames {
        let Some(decoded) = decoded_frames.next() else {
            break;
        };
        let Ok(mut frame) = decoded else {
            break;
        };
        if orientation != image::metadata::Orientation::NoTransforms {
            let delay = frame.delay();
            let mut decoded = image::DynamicImage::ImageRgba8(frame.into_buffer());
            decoded.apply_orientation(orientation);
            frame = image::Frame::from_parts(decoded.into_rgba8(), 0, 0, delay);
        }
        let frame = prepare_raster_preview_frame(frame, policy);
        let Some(next_retained_bytes) = retained_bytes.checked_add(frame.buffer().as_raw().len())
        else {
            break;
        };
        if next_retained_bytes > policy.max_retained_bytes {
            break;
        }
        retained_bytes = next_retained_bytes;
        frames.push(frame);
    }

    frames
}

fn decode_oriented_static_raster_frame(
    mut decoder: impl image::ImageDecoder,
    policy: RasterPreviewPolicy,
) -> Option<image::Frame> {
    limit_decoder(&mut decoder, policy.max_decode_bytes).ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).ok()?;
    decoded.apply_orientation(orientation);
    Some(prepare_raster_preview_frame(
        image::Frame::new(decoded.into_rgba8()),
        policy,
    ))
}

fn decode_raster_preview_frames(
    format: gpui::ImageFormat,
    bytes: &[u8],
    policy: RasterPreviewPolicy,
) -> Option<Vec<image::Frame>> {
    let image_format = crate::view::image_preview::raster_format(format)?;
    let frames = match format {
        gpui::ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
            limit_decoder(&mut decoder, policy.max_decode_bytes).ok()?;
            let dimensions = decoder.dimensions();
            prepare_raster_preview_animation_frames(
                decoder.into_frames(),
                dimensions,
                image::metadata::Orientation::NoTransforms,
                policy,
            )
        }
        gpui::ImageFormat::Webp => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            limit_decoder(&mut decoder, policy.max_decode_bytes).ok()?;
            if decoder.has_animation() {
                let dimensions = decoder.dimensions();
                let orientation = decoder
                    .orientation()
                    .unwrap_or(image::metadata::Orientation::NoTransforms);
                let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                prepare_raster_preview_animation_frames(
                    decoder.into_frames(),
                    dimensions,
                    orientation,
                    policy,
                )
            } else {
                vec![decode_oriented_static_raster_frame(decoder, policy)?]
            }
        }
        _ => {
            let decoder = image::ImageReader::with_format(Cursor::new(bytes), image_format)
                .into_decoder()
                .ok()?;
            vec![decode_oriented_static_raster_frame(decoder, policy)?]
        }
    };

    (!frames.is_empty()).then_some(frames)
}

pub(in crate::view) fn render_svg_image_diff_preview(
    tree: &resvg::usvg::Tree,
) -> Option<Arc<gpui::RenderImage>> {
    let svg_size = tree.size();
    let svg_width = svg_size.width();
    let svg_height = svg_size.height();
    if !svg_width.is_finite() || !svg_height.is_finite() || svg_width <= 0.0 || svg_height <= 0.0 {
        return None;
    }

    let upscale = if svg_width < IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX {
        IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX / svg_width
    } else {
        1.0
    };
    let mut raster_width = (svg_width * upscale).round();
    let mut raster_height = (svg_height * upscale).round();
    let max_edge = raster_width.max(raster_height);
    if max_edge > IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX {
        let downscale = IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / max_edge;
        raster_width = (raster_width * downscale).round();
        raster_height = (raster_height * downscale).round();
    }

    let raster_width = raster_width.max(1.0) as u32;
    let raster_height = raster_height.max(1.0) as u32;
    let mut buffer = svg::raster_region(
        tree,
        (raster_width, raster_height),
        (0, 0, raster_width, raster_height),
    )
    .ok()?;
    for pixel in buffer.as_chunks_mut::<4>().0 {
        swap_rgba_pa_to_bgra(pixel);
    }

    Some(render_image_from_bgra8(buffer))
}

pub(in crate::view) fn render_raster_image_diff_preview(
    format: gpui::ImageFormat,
    bytes: &[u8],
) -> Option<Arc<gpui::RenderImage>> {
    let frames = decode_raster_preview_frames(format, bytes, FILE_DIFF_RASTER_PREVIEW_POLICY)?;
    Some(Arc::new(gpui::RenderImage::new(frames)))
}

pub(in crate::view) fn render_raster_conflict_preview(
    format: gpui::ImageFormat,
    bytes: &[u8],
) -> Option<Arc<gpui::RenderImage>> {
    let frames = decode_raster_preview_frames(format, bytes, CONFLICT_RASTER_PREVIEW_POLICY)?;
    Some(Arc::new(gpui::RenderImage::new(frames)))
}

fn decode_file_image_diff_preview_side(
    format: gpui::ImageFormat,
    bytes: &[u8],
) -> DecodedImageDiffPreview {
    let source = crate::view::image_preview::Source::new(
        format,
        bytes,
        IMAGE_DIFF_RASTER_PREVIEW_MAX_DECODE_BYTES,
    );
    match format {
        gpui::ImageFormat::Svg => {
            if let Some(render) = source
                .as_ref()
                .and_then(|source| render_svg_image_diff_preview(source.svg()?))
            {
                return DecodedImageDiffPreview {
                    source,
                    render: Some(render),
                    cached_path: None,
                };
            }
            DecodedImageDiffPreview {
                source,
                render: None,
                cached_path: cached_image_diff_path(bytes, "svg"),
            }
        }
        _ => DecodedImageDiffPreview {
            source,
            render: render_raster_image_diff_preview(format, bytes),
            cached_path: None,
        },
    }
}

fn file_image_diff_signature(file: &gitcomet_core::domain::FileDiffImage) -> u64 {
    let mut hasher = FxHasher::default();
    file.path.hash(&mut hasher);
    file.old.hash(&mut hasher);
    file.new.hash(&mut hasher);
    hasher.finish()
}

fn cached_image_diff_path(bytes: &[u8], extension: &str) -> Option<std::path::PathBuf> {
    cleanup_image_diff_cache_startup_once();
    image_diff_cache_path(&image_diff_cache_dir().ok()?, bytes, extension).ok()
}

/// Content-addressed cache file, written once and reused on later rebuilds.
fn image_diff_cache_path(
    cache_dir: &std::path::Path,
    bytes: &[u8],
    extension: &str,
) -> std::io::Result<std::path::PathBuf> {
    let mut hasher = FxHasher::default();
    hasher.write(bytes);
    hasher.write(extension.as_bytes());
    let path = cache_dir.join(format!(
        "{IMAGE_DIFF_CACHE_FILE_PREFIX}{:016x}-{}.{}",
        hasher.finish(),
        bytes.len(),
        extension
    ));

    if !is_regular_file(&path) {
        write_image_diff_cache_file(&path, bytes)?;
        maybe_cleanup_image_diff_cache_on_write();
        // O_EXCL leaves a planted symlink in place; never serve that as an image.
        if !is_regular_file(&path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "image diff cache path is not a regular file: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(path)
}

fn is_regular_file(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

fn write_image_diff_cache_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = tempfile::Builder::new()
        .prefix(IMAGE_DIFF_CACHE_FILE_PREFIX)
        .suffix(".tmp")
        .tempfile_in(path.parent().unwrap_or_else(|| std::path::Path::new(".")))?;
    file.write_all(bytes)?;
    file.as_file().sync_data()?;
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        // Another writer produced the same content first.
        Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err.error),
    }
}

struct ImageDiffCacheRebuild {
    sources: [Option<Arc<crate::view::image_preview::Source>>; 2],
    file_path: Option<std::path::PathBuf>,
    old: Option<Arc<gpui::RenderImage>>,
    new: Option<Arc<gpui::RenderImage>>,
    old_svg_path: Option<std::path::PathBuf>,
    new_svg_path: Option<std::path::PathBuf>,
    failed: bool,
}

fn decode_file_image_diff_preview_pair(
    format: gpui::ImageFormat,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) -> (DecodedImageDiffPreview, DecodedImageDiffPreview) {
    if old.is_some() && old == new {
        let preview = old
            .map(|bytes| decode_file_image_diff_preview_side(format, bytes))
            .unwrap_or_default();
        return (preview.clone(), preview);
    }

    std::thread::scope(|scope| {
        let old_task = old.map(|bytes| {
            std::thread::Builder::new().spawn_scoped(scope, move || {
                decode_file_image_diff_preview_side(format, bytes)
            })
        });
        let new_task = new.map(|bytes| {
            std::thread::Builder::new().spawn_scoped(scope, move || {
                decode_file_image_diff_preview_side(format, bytes)
            })
        });

        let old_preview = old_task.map_or_else(DecodedImageDiffPreview::default, |task| {
            task.map_or_else(
                |_| {
                    old.map(|bytes| decode_file_image_diff_preview_side(format, bytes))
                        .unwrap_or_default()
                },
                |task| task.join().unwrap_or_default(),
            )
        });
        let new_preview = new_task.map_or_else(DecodedImageDiffPreview::default, |task| {
            task.map_or_else(
                |_| {
                    new.map(|bytes| decode_file_image_diff_preview_side(format, bytes))
                        .unwrap_or_default()
                },
                |task| task.join().unwrap_or_default(),
            )
        });
        (old_preview, new_preview)
    })
}

fn build_file_image_diff_cache_rebuild(
    file: &gitcomet_core::domain::FileDiffImage,
    workdir: &std::path::Path,
) -> ImageDiffCacheRebuild {
    // A name without a known extension leaves the format to the bytes.
    let format = image_format_for_path(&file.path).or_else(|| {
        [file.new.as_deref(), file.old.as_deref()]
            .into_iter()
            .flatten()
            .find_map(crate::view::diff_utils::image_format_for_bytes)
    });
    let file_path = Some(if file.path.is_absolute() {
        file.path.to_path_buf()
    } else {
        workdir.join(&file.path)
    });

    let Some(format) = format else {
        return ImageDiffCacheRebuild {
            sources: Default::default(),
            file_path,
            old: None,
            new: None,
            old_svg_path: None,
            new_svg_path: None,
            failed: file.old.is_some() || file.new.is_some(),
        };
    };

    let (old_preview, new_preview) =
        decode_file_image_diff_preview_pair(format, file.old.as_deref(), file.new.as_deref());
    let failed =
        (file.old.is_some() && old_preview.render.is_none() && old_preview.cached_path.is_none())
            || (file.new.is_some()
                && new_preview.render.is_none()
                && new_preview.cached_path.is_none());
    ImageDiffCacheRebuild {
        sources: [old_preview.source, new_preview.source],
        file_path,
        old: old_preview.render,
        new: new_preview.render,
        old_svg_path: old_preview.cached_path,
        new_svg_path: new_preview.cached_path,
        failed,
    }
}

impl DiffRenderer {
    pub(in crate::view) fn advance_file_image_preview_side(
        state: &mut FileImagePreviewAnimationSide,
        image: Option<&Arc<gpui::RenderImage>>,
        now: std::time::Instant,
        animate: bool,
    ) -> Option<std::time::Instant> {
        let Some(image) = image else {
            *state = FileImagePreviewAnimationSide::default();
            return None;
        };
        if state.image_id != Some(image.id) {
            *state = FileImagePreviewAnimationSide {
                image_id: Some(image.id),
                frame_index: 0,
                frame_started_at: None,
            };
        }

        let frame_count = image.frame_count();
        if !animate || frame_count <= 1 {
            state.frame_index = 0;
            state.frame_started_at = None;
            return None;
        }

        let started = state.frame_started_at.get_or_insert(now);
        let mut elapsed = now.saturating_duration_since(*started);
        for _ in 0..frame_count {
            let delay = std::time::Duration::from(image.delay(state.frame_index))
                .max(std::time::Duration::from_millis(16));
            if elapsed < delay {
                return Some(*started + delay);
            }
            elapsed -= delay;
            *started += delay;
            state.frame_index = (state.frame_index + 1) % frame_count;
        }

        // A window can be inactive or suspended for many complete animation
        // cycles. Preserve the current frame but discard whole-cycle lag so a
        // resume never schedules a run of immediate catch-up renders.
        *started = now;
        let delay = std::time::Duration::from(image.delay(state.frame_index))
            .max(std::time::Duration::from_millis(16));
        Some(*started + delay)
    }

    pub(in crate::view) fn update_file_image_preview_animation(
        &mut self,
        old: Option<&Arc<gpui::RenderImage>>,
        new: Option<&Arc<gpui::RenderImage>>,
        window: &gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> [usize; 2] {
        let timer: fn(&mut Self) -> &mut _ = |this| &mut this.file_image_preview_timer;
        if self.image_preview_group.read(cx).paused {
            self.file_image_preview_animation.old.frame_started_at = None;
            self.file_image_preview_animation.new.frame_started_at = None;
            self.file_image_preview_timer.schedule(None, timer, cx);
            return [
                self.file_image_preview_animation.old.frame_index,
                self.file_image_preview_animation.new.frame_index,
            ];
        }
        let now = std::time::Instant::now();
        let animate = crate::view::image_preview::animates(window, cx);
        let old_deadline = Self::advance_file_image_preview_side(
            &mut self.file_image_preview_animation.old,
            old,
            now,
            animate,
        );
        let new_deadline = Self::advance_file_image_preview_side(
            &mut self.file_image_preview_animation.new,
            new,
            now,
            animate,
        );
        let deadline = match (old_deadline, new_deadline) {
            (Some(old), Some(new)) => Some(old.min(new)),
            (old, new) => old.or(new),
        };
        self.file_image_preview_timer.schedule(deadline, timer, cx);

        [
            self.file_image_preview_animation.old.frame_index,
            self.file_image_preview_animation.new.frame_index,
        ]
    }

    fn reset_file_image_diff_cache_data(&mut self, cx: &mut gpui::Context<Self>) {
        let old = self.file_image_diff_cache_old.take();
        let new = self.file_image_diff_cache_new.take();
        if old.is_some() || new.is_some() {
            cx.defer(move |cx| {
                if let Some(old) = old {
                    let duplicate = new.as_ref().is_some_and(|new| new.id == old.id);
                    cx.drop_image(old, None);
                    if duplicate {
                        return;
                    }
                }
                if let Some(new) = new {
                    cx.drop_image(new, None);
                }
            });
        }
        self.file_image_diff_cache_content_signature = None;
        self.file_image_diff_cache_inflight = None;
        self.file_image_diff_cache_complete = false;
        self.file_image_diff_cache_failed = false;
        self.file_image_diff_cache_path = None;
        self.file_image_sources = Default::default();
        self.file_image_diff_cache_old_svg_path = None;
        self.file_image_diff_cache_new_svg_path = None;
        self.file_image_preview_animation = FileImagePreviewAnimation::default();
        self.file_image_preview_timer = Default::default();
    }

    pub(in crate::view) fn ensure_file_image_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, diff_file_rev, diff_target, workdir, expected_abs_path, file)) =
            (|| {
                let (repo_id, diff_file_rev, diff_target, workdir, expected_abs_path) =
                    self.rendered_file_diff_identity()?;
                let file: Option<Arc<gitcomet_core::domain::FileDiffImage>> =
                    match self.rendered_file_image_diff_loadable()? {
                        Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                        _ => None,
                    };

                Some((
                    repo_id,
                    diff_file_rev,
                    diff_target,
                    workdir,
                    expected_abs_path,
                    file,
                ))
            })()
        else {
            self.file_image_diff_cache_repo_id = None;
            self.file_image_diff_cache_target = None;
            self.file_image_diff_cache_rev = 0;
            self.reset_file_image_diff_cache_data(cx);
            return;
        };

        let diff_target_for_task = diff_target.clone();
        let file_content_signature = file
            .as_ref()
            .map(|file| file_image_diff_signature(file.as_ref()));
        let same_repo_and_target = self.file_image_diff_cache_repo_id == Some(repo_id)
            && self.file_image_diff_cache_target == Some(diff_target.clone())
            && self.file_image_diff_cache_path.as_ref() == Some(&expected_abs_path);

        if same_repo_and_target && self.file_image_diff_cache_rev == diff_file_rev {
            return;
        }

        if same_repo_and_target
            && let Some(signature) = file_content_signature
            && self.file_image_diff_cache_content_signature == Some(signature)
        {
            if self.file_image_diff_cache_inflight.is_none() {
                self.file_image_diff_cache_rev = diff_file_rev;
            }
            return;
        }

        self.file_image_diff_cache_repo_id = Some(repo_id);
        self.file_image_diff_cache_rev = diff_file_rev;
        self.file_image_diff_cache_target = Some(diff_target);
        self.reset_file_image_diff_cache_data(cx);

        let Some(file) = file else {
            return;
        };
        let content_signature =
            file_content_signature.unwrap_or_else(|| file_image_diff_signature(file.as_ref()));

        self.file_image_diff_cache_seq = self.file_image_diff_cache_seq.wrapping_add(1);
        let seq = self.file_image_diff_cache_seq;
        self.file_image_diff_cache_inflight = Some(seq);

        if !crate::ui_runtime::current().uses_background_compute() {
            let rebuild = build_file_image_diff_cache_rebuild(file.as_ref(), &workdir);
            if self.file_image_diff_cache_inflight == Some(seq)
                && self.file_image_diff_cache_repo_id == Some(repo_id)
                && self.file_image_diff_cache_rev == diff_file_rev
                && self.file_image_diff_cache_target == Some(diff_target_for_task.clone())
            {
                self.file_image_diff_cache_inflight = None;
                self.file_image_diff_cache_complete = true;
                self.file_image_diff_cache_failed = rebuild.failed;
                self.file_image_diff_cache_content_signature = Some(content_signature);
                self.file_image_diff_cache_path = rebuild.file_path;
                self.file_image_sources = rebuild.sources;
                self.file_image_diff_cache_old = rebuild.old;
                self.file_image_diff_cache_new = rebuild.new;
                self.file_image_diff_cache_old_svg_path = rebuild.old_svg_path;
                self.file_image_diff_cache_new_svg_path = rebuild.new_svg_path;
                cx.notify();
            }
            return;
        }

        cx.spawn(
            async move |view: WeakEntity<DiffRenderer>, cx: &mut gpui::AsyncApp| {
                let rebuild = smol::unblock(move || {
                    build_file_image_diff_cache_rebuild(file.as_ref(), &workdir)
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_image_diff_cache_inflight != Some(seq) {
                        return;
                    }
                    if this.file_image_diff_cache_repo_id != Some(repo_id)
                        || this.file_image_diff_cache_rev != diff_file_rev
                        || this.file_image_diff_cache_target != Some(diff_target_for_task.clone())
                    {
                        return;
                    }

                    this.file_image_diff_cache_inflight = None;
                    this.file_image_diff_cache_complete = true;
                    this.file_image_diff_cache_failed = rebuild.failed;
                    this.file_image_diff_cache_content_signature = Some(content_signature);
                    this.file_image_diff_cache_path = rebuild.file_path;
                    this.file_image_sources = rebuild.sources;
                    this.file_image_diff_cache_old = rebuild.old;
                    this.file_image_diff_cache_new = rebuild.new;
                    this.file_image_diff_cache_old_svg_path = rebuild.old_svg_path;
                    this.file_image_diff_cache_new_svg_path = rebuild.new_svg_path;
                    cx.notify();
                });
            },
        )
        .detach();
    }
}

#[cfg(test)]
mod tests;
