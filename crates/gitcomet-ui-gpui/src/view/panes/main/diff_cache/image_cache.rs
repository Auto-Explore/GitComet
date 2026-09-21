use super::*;
use crate::view::diff_utils::{fill_svg_viewport_white, image_format_for_path};
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
static IMAGE_DIFF_SVG_USVG_OPTIONS: std::sync::LazyLock<resvg::usvg::Options<'static>> =
    std::sync::LazyLock::new(resvg::usvg::Options::default);
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
    render: Option<Arc<gpui::RenderImage>>,
    cached_path: Option<std::path::PathBuf>,
}

fn image_rs_format_for_diff_preview(format: gpui::ImageFormat) -> Option<image::ImageFormat> {
    match format {
        gpui::ImageFormat::Png => Some(image::ImageFormat::Png),
        gpui::ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
        gpui::ImageFormat::Gif => Some(image::ImageFormat::Gif),
        gpui::ImageFormat::Webp => Some(image::ImageFormat::WebP),
        gpui::ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
        gpui::ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
        gpui::ImageFormat::Ico => Some(image::ImageFormat::Ico),
        gpui::ImageFormat::Svg => None,
        gpui::ImageFormat::Pnm => Some(image::ImageFormat::Pnm),
    }
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

fn configure_raster_decoder(
    decoder: &mut impl image::ImageDecoder,
    policy: RasterPreviewPolicy,
) -> image::ImageResult<()> {
    let (width, height) = decoder.dimensions();
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            image::ImageError::Limits(image::error::LimitError::from_kind(
                image::error::LimitErrorKind::InsufficientMemory,
            ))
        })?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(policy.max_decode_bytes);
    limits.reserve(decoder.total_bytes())?;
    limits.reserve(rgba_bytes)?;
    decoder.set_limits(limits)
}

fn decode_oriented_static_raster_frame(
    mut decoder: impl image::ImageDecoder,
    policy: RasterPreviewPolicy,
) -> Option<image::Frame> {
    configure_raster_decoder(&mut decoder, policy).ok()?;
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
    let image_format = image_rs_format_for_diff_preview(format)?;
    let frames = match format {
        gpui::ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
            configure_raster_decoder(&mut decoder, policy).ok()?;
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
            configure_raster_decoder(&mut decoder, policy).ok()?;
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
    svg_bytes: &[u8],
) -> Option<Arc<gpui::RenderImage>> {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &IMAGE_DIFF_SVG_USVG_OPTIONS).ok()?;
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
    let mut pixmap = resvg::tiny_skia::Pixmap::new(raster_width, raster_height)?;
    fill_svg_viewport_white(&mut pixmap);
    let transform = resvg::tiny_skia::Transform::from_scale(
        raster_width as f32 / svg_width,
        raster_height as f32 / svg_height,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut buffer = image::ImageBuffer::from_raw(pixmap.width(), pixmap.height(), pixmap.take())?;
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
    match format {
        gpui::ImageFormat::Svg => {
            if let Some(render) = render_svg_image_diff_preview(bytes) {
                return DecodedImageDiffPreview {
                    render: Some(render),
                    cached_path: None,
                };
            }
            DecodedImageDiffPreview {
                render: None,
                cached_path: cached_image_diff_path(bytes, "svg"),
            }
        }
        _ => DecodedImageDiffPreview {
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
    let format = image_format_for_path(&file.path);
    let file_path = Some(if file.path.is_absolute() {
        file.path.to_path_buf()
    } else {
        workdir.join(&file.path)
    });

    let Some(format) = format else {
        return ImageDiffCacheRebuild {
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
        file_path,
        old: old_preview.render,
        new: new_preview.render,
        old_svg_path: old_preview.cached_path,
        new_svg_path: new_preview.cached_path,
        failed,
    }
}

impl MainPaneView {
    fn advance_file_image_preview_side(
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
        let now = std::time::Instant::now();
        let animate = window.is_window_active() && !cx.reduce_motion();
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

        if deadline != self.file_image_preview_animation.scheduled_deadline {
            self.file_image_preview_animation_task = None;
            self.file_image_preview_animation.generation =
                self.file_image_preview_animation.generation.wrapping_add(1);
            self.file_image_preview_animation.scheduled_deadline = deadline;
            if let Some(deadline) = deadline {
                let generation = self.file_image_preview_animation.generation;
                let delay = deadline.saturating_duration_since(now);
                self.file_image_preview_animation_task = Some(cx.spawn(
                    async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                        cx.background_executor().timer(delay).await;
                        let _ = view.update(cx, |this, cx| {
                            if this.file_image_preview_animation.generation != generation {
                                return;
                            }
                            this.file_image_preview_animation.scheduled_deadline = None;
                            cx.notify();
                        });
                    },
                ));
            }
        }

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
        self.file_image_diff_cache_old_svg_path = None;
        self.file_image_diff_cache_new_svg_path = None;
        self.file_image_preview_animation = FileImagePreviewAnimation::default();
        self.file_image_preview_animation_task = None;
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
                self.file_image_diff_cache_old = rebuild.old;
                self.file_image_diff_cache_new = rebuild.new;
                self.file_image_diff_cache_old_svg_path = rebuild.old_svg_path;
                self.file_image_diff_cache_new_svg_path = rebuild.new_svg_path;
                cx.notify();
            }
            return;
        }

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
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
mod tests {
    use super::*;
    use std::path::Path;

    struct TestDecoder {
        dimensions: (u32, u32),
        orientation_error: bool,
        read_called: Arc<std::sync::atomic::AtomicBool>,
    }

    impl image::ImageDecoder for TestDecoder {
        fn dimensions(&self) -> (u32, u32) {
            self.dimensions
        }

        fn color_type(&self) -> image::ColorType {
            image::ColorType::Rgba8
        }

        fn orientation(&mut self) -> image::ImageResult<image::metadata::Orientation> {
            if self.orientation_error {
                Err(image::ImageError::IoError(std::io::Error::other(
                    "metadata read failed",
                )))
            } else {
                Ok(image::metadata::Orientation::NoTransforms)
            }
        }

        fn read_image(self, buffer: &mut [u8]) -> image::ImageResult<()> {
            self.read_called
                .store(true, std::sync::atomic::Ordering::Release);
            for pixel in buffer.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[10, 20, 30, 255]);
            }
            Ok(())
        }

        fn read_image_boxed(self: Box<Self>, buffer: &mut [u8]) -> image::ImageResult<()> {
            (*self).read_image(buffer)
        }
    }

    fn solid_rect_svg(width: u32, height: u32) -> Vec<u8> {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<rect width="{width}" height="{height}" fill="#00aaff"/>
</svg>"##
        )
        .into_bytes()
    }

    fn inset_rect_svg(width: u32, height: u32, inset_x: u32, inset_y: u32) -> Vec<u8> {
        let inner_width = width.saturating_sub(inset_x.saturating_mul(2));
        let inner_height = height.saturating_sub(inset_y.saturating_mul(2));
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<rect x="{inset_x}" y="{inset_y}" width="{inner_width}" height="{inner_height}" fill="#00aaff"/>
</svg>"##
        )
        .into_bytes()
    }

    fn render_pixel_bgra(render: &gpui::RenderImage, x: usize, y: usize) -> [u8; 4] {
        let size = render.size(0);
        let width = size.width.0 as usize;
        let offset = (y.saturating_mul(width).saturating_add(x)).saturating_mul(4);
        let bytes = render.as_bytes(0).expect("render bytes");
        [
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]
    }

    fn write_test_file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write test file");
        path
    }

    fn rotate_90_exif() -> Vec<u8> {
        vec![
            b'I', b'I', 42, 0, // Little-endian TIFF header.
            8, 0, 0, 0, // Offset to the first IFD.
            1, 0, // One directory entry.
            0x12, 0x01, // Orientation tag (0x0112).
            3, 0, // SHORT.
            1, 0, 0, 0, // One value.
            6, 0, 0, 0, // Rotate 90 degrees clockwise.
            0, 0, 0, 0, // No next IFD.
        ]
    }

    fn oriented_tiff_fixture() -> Vec<u8> {
        fn push_ifd_entry(bytes: &mut Vec<u8>, tag: u16, field_type: u16, value: u32) {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&field_type.to_le_bytes());
            bytes.extend_from_slice(&1_u32.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }

        const ENTRY_COUNT: u16 = 9;
        const PIXEL_OFFSET: u32 = 8 + 2 + ENTRY_COUNT as u32 * 12 + 4;
        let mut bytes = Vec::with_capacity(PIXEL_OFFSET as usize + 2);
        bytes.extend_from_slice(b"II");
        bytes.extend_from_slice(&42_u16.to_le_bytes());
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&ENTRY_COUNT.to_le_bytes());
        push_ifd_entry(&mut bytes, 256, 3, 2); // ImageWidth.
        push_ifd_entry(&mut bytes, 257, 3, 1); // ImageLength.
        push_ifd_entry(&mut bytes, 258, 3, 8); // BitsPerSample.
        push_ifd_entry(&mut bytes, 259, 3, 1); // No compression.
        push_ifd_entry(&mut bytes, 262, 3, 1); // BlackIsZero.
        push_ifd_entry(&mut bytes, 273, 4, PIXEL_OFFSET); // StripOffsets.
        push_ifd_entry(&mut bytes, 274, 3, 6); // Orientation: rotate 90.
        push_ifd_entry(&mut bytes, 278, 4, 1); // RowsPerStrip.
        push_ifd_entry(&mut bytes, 279, 4, 2); // StripByteCounts.
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 255]);
        bytes
    }

    fn oriented_raster_fixture(format: gpui::ImageFormat) -> Vec<u8> {
        use image::ImageEncoder as _;

        let mut encoded = Vec::new();
        let exif = rotate_90_exif();
        match format {
            gpui::ImageFormat::Jpeg => {
                let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut encoded);
                encoder.set_exif_metadata(exif).expect("JPEG EXIF metadata");
                encoder
                    .write_image(
                        &[255, 0, 0, 0, 255, 0],
                        2,
                        1,
                        image::ExtendedColorType::Rgb8,
                    )
                    .expect("encode oriented JPEG");
            }
            gpui::ImageFormat::Png => {
                let mut encoder = image::codecs::png::PngEncoder::new(&mut encoded);
                encoder.set_exif_metadata(exif).expect("PNG EXIF metadata");
                encoder
                    .write_image(
                        &[255, 0, 0, 255, 0, 255, 0, 255],
                        2,
                        1,
                        image::ExtendedColorType::Rgba8,
                    )
                    .expect("encode oriented PNG");
            }
            gpui::ImageFormat::Tiff => return oriented_tiff_fixture(),
            gpui::ImageFormat::Webp => {
                let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut encoded);
                encoder.set_exif_metadata(exif).expect("WebP EXIF metadata");
                encoder
                    .write_image(
                        &[255, 0, 0, 255, 0, 255, 0, 255],
                        2,
                        1,
                        image::ExtendedColorType::Rgba8,
                    )
                    .expect("encode oriented WebP");
            }
            _ => panic!("unsupported oriented raster fixture format: {format:?}"),
        }
        encoded
    }

    #[test]
    fn transparent_edge_extension_keeps_alpha_and_uses_alpha_weighted_rgb() {
        let mut buffer = image::RgbaImage::from_raw(
            5,
            1,
            vec![
                9, 8, 7, 0, // Transparent left edge.
                255, 0, 0, 255, // Opaque red.
                0, 0, 0, 0, // Between red and blue.
                0, 0, 255, 128, // Half-alpha blue.
                6, 5, 4, 0, // Transparent right edge.
            ],
        )
        .expect("test buffer");

        extend_transparent_edge_rgb(&mut buffer);

        assert_eq!(buffer.get_pixel(0, 0).0, [255, 0, 0, 0]);
        assert_eq!(buffer.get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert_eq!(buffer.get_pixel(2, 0).0, [170, 0, 85, 0]);
        assert_eq!(buffer.get_pixel(3, 0).0, [0, 0, 255, 128]);
        assert_eq!(buffer.get_pixel(4, 0).0, [0, 0, 255, 0]);
    }

    #[test]
    fn transparent_edge_extension_is_one_texel_and_leaves_empty_images_unchanged() {
        let mut buffer = image::RgbaImage::from_raw(
            5,
            1,
            vec![
                7, 8, 9, 0, 0, 0, 0, 0, 40, 80, 120, 255, 0, 0, 0, 0, 3, 2, 1, 0,
            ],
        )
        .expect("test buffer");

        extend_transparent_edge_rgb(&mut buffer);

        assert_eq!(buffer.get_pixel(0, 0).0, [7, 8, 9, 0]);
        assert_eq!(buffer.get_pixel(1, 0).0, [40, 80, 120, 0]);
        assert_eq!(buffer.get_pixel(3, 0).0, [40, 80, 120, 0]);
        assert_eq!(buffer.get_pixel(4, 0).0, [3, 2, 1, 0]);

        let mut empty = image::RgbaImage::from_pixel(2, 2, image::Rgba([3, 4, 5, 0]));
        extend_transparent_edge_rgb(&mut empty);
        assert!(empty.pixels().all(|pixel| pixel.0 == [3, 4, 5, 0]));
    }

    #[test]
    fn alpha_correct_thumbnail_keeps_straight_edge_color() {
        let source = image::RgbaImage::from_raw(2, 1, vec![200, 100, 50, 255, 0, 0, 0, 0])
            .expect("test buffer");

        let resized = alpha_correct_thumbnail(source, 1);
        let pixel = resized.get_pixel(0, 0).0;

        assert_eq!(resized.dimensions(), (1, 1));
        assert!(pixel[0].abs_diff(200) <= 1, "red={}", pixel[0]);
        assert!(pixel[1].abs_diff(100) <= 1, "green={}", pixel[1]);
        assert!(pixel[2].abs_diff(50) <= 1, "blue={}", pixel[2]);
        assert!(pixel[3].abs_diff(128) <= 1, "alpha={}", pixel[3]);
    }

    #[test]
    fn decode_allocation_limit_is_checked_before_reading_pixels() {
        let read_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let decoder = TestDecoder {
            dimensions: (10, 10),
            orientation_error: false,
            read_called: Arc::clone(&read_called),
        };
        let policy = RasterPreviewPolicy {
            max_edge_px: 10,
            max_frames: 1,
            max_retained_bytes: usize::MAX,
            // Decoder RGBA output plus the projected prepared RGBA buffer is
            // 800 bytes, so this must reject without calling read_image.
            max_decode_bytes: 799,
        };

        assert!(decode_oriented_static_raster_frame(decoder, policy).is_none());
        assert!(!read_called.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn orientation_metadata_failure_falls_back_to_unrotated_decode() {
        let read_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let decoder = TestDecoder {
            dimensions: (1, 1),
            orientation_error: true,
            read_called: Arc::clone(&read_called),
        };
        let policy = RasterPreviewPolicy {
            max_edge_px: 1,
            max_frames: 1,
            max_retained_bytes: 4,
            max_decode_bytes: 8,
        };

        let frame = decode_oriented_static_raster_frame(decoder, policy)
            .expect("pixel decode should survive metadata failure");
        assert!(read_called.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(frame.buffer().get_pixel(0, 0).0, [30, 20, 10, 255]);
    }

    #[test]
    fn prepared_raster_frame_preserves_timing_and_extends_bgra_edge_color() {
        let delay = image::Delay::from_numer_denom_ms(17, 1);
        let frame = image::Frame::from_parts(
            image::RgbaImage::from_raw(3, 1, vec![0, 0, 0, 0, 12, 34, 56, 128, 0, 0, 0, 0])
                .expect("test buffer"),
            2,
            3,
            delay,
        );

        let prepared = prepare_raster_preview_frame(frame, FILE_DIFF_RASTER_PREVIEW_POLICY);

        assert_eq!(prepared.delay(), delay);
        assert_eq!(prepared.left(), 2);
        assert_eq!(prepared.top(), 3);
        assert_eq!(prepared.buffer().get_pixel(0, 0).0, [56, 34, 12, 0]);
        assert_eq!(prepared.buffer().get_pixel(1, 0).0, [56, 34, 12, 128]);
        assert_eq!(prepared.buffer().get_pixel(2, 0).0, [56, 34, 12, 0]);
    }

    #[test]
    fn raster_preview_png_uploads_transparent_edge_with_extended_bgra() {
        let image = image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 0, 0, 0, 0])
                .expect("test buffer"),
        );
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode png");

        let render =
            render_raster_image_diff_preview(gpui::ImageFormat::Png, &encoded.into_inner())
                .expect("render image");

        assert_eq!(render_pixel_bgra(&render, 0, 0), [30, 20, 10, 255]);
        assert_eq!(render_pixel_bgra(&render, 1, 0), [30, 20, 10, 0]);
    }

    #[test]
    fn raster_preview_gif_preserves_frames_delays_and_transparent_edge_color() {
        let frames = [
            image::Frame::from_parts(
                image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 0, 0, 0, 0])
                    .expect("first GIF frame"),
                0,
                0,
                image::Delay::from_numer_denom_ms(40, 1),
            ),
            image::Frame::from_parts(
                image::RgbaImage::from_raw(2, 1, vec![70, 80, 90, 255, 0, 0, 0, 0])
                    .expect("second GIF frame"),
                0,
                0,
                image::Delay::from_numer_denom_ms(70, 1),
            ),
        ];
        let mut encoded = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut encoded)
            .encode_frames(frames)
            .expect("encode GIF frames");

        let render =
            render_raster_image_diff_preview(gpui::ImageFormat::Gif, &encoded).expect("render GIF");

        assert_eq!(render.frame_count(), 2);
        assert_eq!(render.delay(0).numer_denom_ms(), (40, 1));
        assert_eq!(render.delay(1).numer_denom_ms(), (70, 1));
        assert_eq!(render_pixel_bgra(&render, 0, 0), [30, 20, 10, 255]);
        assert_eq!(render_pixel_bgra(&render, 1, 0), [30, 20, 10, 0]);
        assert_eq!(
            render.as_bytes(1).expect("second frame")[0..4],
            [90, 80, 70, 255]
        );
        assert_eq!(
            render.as_bytes(1).expect("second frame")[4..8],
            [90, 80, 70, 0]
        );
    }

    #[test]
    fn animation_frame_preparation_stops_at_frame_and_byte_limits() {
        let decoded_for_frame_limit = std::cell::Cell::new(0_usize);
        let frames = (0..10).map(|_| {
            decoded_for_frame_limit.set(decoded_for_frame_limit.get() + 1);
            Ok(image::Frame::new(image::RgbaImage::from_pixel(
                2,
                2,
                image::Rgba([10, 20, 30, 255]),
            )))
        });
        let prepared = prepare_raster_preview_animation_frames(
            frames,
            (2, 2),
            image::metadata::Orientation::NoTransforms,
            RasterPreviewPolicy {
                max_edge_px: 2,
                max_frames: 2,
                max_retained_bytes: usize::MAX,
                max_decode_bytes: u64::MAX,
            },
        );

        assert_eq!(prepared.len(), 2);
        assert_eq!(decoded_for_frame_limit.get(), 2);

        let decoded_for_byte_limit = std::cell::Cell::new(0_usize);
        let frames = (0..10).map(|_| {
            decoded_for_byte_limit.set(decoded_for_byte_limit.get() + 1);
            Ok(image::Frame::new(image::RgbaImage::from_pixel(
                2,
                2,
                image::Rgba([10, 20, 30, 255]),
            )))
        });
        let prepared = prepare_raster_preview_animation_frames(
            frames,
            (2, 2),
            image::metadata::Orientation::NoTransforms,
            RasterPreviewPolicy {
                max_edge_px: 2,
                max_frames: usize::MAX,
                max_retained_bytes: 32,
                max_decode_bytes: u64::MAX,
            },
        );
        let retained_bytes = prepared
            .iter()
            .map(|frame| frame.buffer().as_raw().len())
            .sum::<usize>();

        assert_eq!(prepared.len(), 2);
        assert_eq!(retained_bytes, 32);
        assert_eq!(decoded_for_byte_limit.get(), 2);
    }

    #[test]
    fn animation_frame_preparation_stops_after_first_decode_error() {
        let polls = std::cell::Cell::new(0_usize);
        let frames = std::iter::from_fn(|| {
            polls.set(polls.get() + 1);
            Some(Err(image::ImageError::IoError(std::io::Error::other(
                "corrupt frame",
            ))))
        });

        let prepared = prepare_raster_preview_animation_frames(
            frames,
            (1, 1),
            image::metadata::Orientation::NoTransforms,
            FILE_DIFF_RASTER_PREVIEW_POLICY,
        );

        assert!(prepared.is_empty());
        assert_eq!(polls.get(), 1);
    }

    #[test]
    fn animation_frame_preparation_applies_orientation_to_every_frame() {
        let frames = (0..2).map(|_| {
            Ok(image::Frame::new(image::RgbaImage::from_pixel(
                2,
                1,
                image::Rgba([10, 20, 30, 255]),
            )))
        });
        let prepared = prepare_raster_preview_animation_frames(
            frames,
            (2, 1),
            image::metadata::Orientation::Rotate90,
            RasterPreviewPolicy {
                max_edge_px: 2,
                max_frames: 2,
                max_retained_bytes: 16,
                max_decode_bytes: 16,
            },
        );

        assert_eq!(prepared.len(), 2);
        assert!(
            prepared
                .iter()
                .all(|frame| frame.buffer().dimensions() == (1, 2))
        );
    }

    #[test]
    fn alpha_correct_thumbnail_never_upscales_or_changes_empty_images() {
        let source = image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]));
        assert_eq!(alpha_correct_thumbnail(source, 1920).dimensions(), (4, 2));

        let empty = image::RgbaImage::new(0, 0);
        assert_eq!(alpha_correct_thumbnail(empty, 1920).dimensions(), (0, 0));
    }

    #[test]
    fn raster_preview_gif_caps_retained_frame_count() {
        let mut encoded = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut encoded)
            .encode_frames(
                (0..IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES + 5).map(|index| {
                    image::Frame::from_parts(
                        image::RgbaImage::from_pixel(1, 1, image::Rgba([index as u8, 0, 0, 255])),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(10, 1),
                    )
                }),
            )
            .expect("encode many-frame GIF");

        let render = render_raster_image_diff_preview(gpui::ImageFormat::Gif, &encoded)
            .expect("render capped GIF");

        assert_eq!(
            render.frame_count(),
            IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES
        );
    }

    #[test]
    fn conflict_raster_preview_keeps_only_a_512px_first_frame() {
        let frames = (0..2).map(|index| {
            image::Frame::from_parts(
                image::RgbaImage::from_pixel(1024, 512, image::Rgba([index as u8, 20, 30, 255])),
                0,
                0,
                image::Delay::from_numer_denom_ms(40, 1),
            )
        });
        let mut encoded = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut encoded)
            .encode_frames(frames)
            .expect("encode conflict GIF");

        let render = render_raster_conflict_preview(gpui::ImageFormat::Gif, &encoded)
            .expect("render conflict GIF");
        assert_eq!(render.frame_count(), 1);
        assert_eq!(render.size(0).width.0, 512);
        assert_eq!(render.size(0).height.0, 256);
    }

    #[test]
    fn file_animation_state_follows_delays_and_resets_for_new_sources() {
        let image = Arc::new(gpui::RenderImage::new(vec![
            image::Frame::from_parts(
                image::RgbaImage::new(1, 1),
                0,
                0,
                image::Delay::from_numer_denom_ms(40, 1),
            ),
            image::Frame::from_parts(
                image::RgbaImage::new(1, 1),
                0,
                0,
                image::Delay::from_numer_denom_ms(70, 1),
            ),
        ]));
        let start = std::time::Instant::now();
        let mut state = FileImagePreviewAnimationSide::default();

        assert_eq!(
            MainPaneView::advance_file_image_preview_side(&mut state, Some(&image), start, true,),
            Some(start + std::time::Duration::from_millis(40))
        );
        assert_eq!(state.frame_index, 0);
        MainPaneView::advance_file_image_preview_side(
            &mut state,
            Some(&image),
            start + std::time::Duration::from_millis(40),
            true,
        );
        assert_eq!(state.frame_index, 1);

        let replacement = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )]));
        MainPaneView::advance_file_image_preview_side(
            &mut state,
            Some(&replacement),
            start + std::time::Duration::from_millis(50),
            true,
        );
        assert_eq!(state.image_id, Some(replacement.id));
        assert_eq!(state.frame_index, 0);
        assert!(state.frame_started_at.is_none());
    }

    #[test]
    fn static_raster_previews_apply_metadata_orientation() {
        for format in [
            gpui::ImageFormat::Jpeg,
            gpui::ImageFormat::Png,
            gpui::ImageFormat::Tiff,
            gpui::ImageFormat::Webp,
        ] {
            let encoded = oriented_raster_fixture(format);
            let render = render_raster_image_diff_preview(format, &encoded)
                .unwrap_or_else(|| panic!("render oriented {format:?}"));
            let size = render.size(0);

            assert_eq!(
                (size.width.0, size.height.0),
                (1, 2),
                "{format:?} preview should apply its rotate-90 metadata"
            );
        }
    }

    #[test]
    fn build_file_image_diff_cache_rebuild_decodes_ico_with_alpha_correct_edges() {
        let mut source = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 0]));
        *source.get_pixel_mut(0, 0) = image::Rgba([10, 20, 30, 255]);
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut encoded, image::ImageFormat::Ico)
            .expect("encode ICO");
        let bytes = encoded.into_inner();
        let file = gitcomet_core::domain::FileDiffImage {
            path: Path::new("images/sample.ico").to_path_buf(),
            old: Some(bytes.clone()),
            new: Some(bytes),
        };

        let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
        let old = rebuild.old.expect("old ICO preview");
        let new = rebuild.new.expect("new ICO preview");

        assert!(Arc::ptr_eq(&old, &new));
        assert_eq!(render_pixel_bgra(&old, 0, 0), [30, 20, 10, 255]);
        assert_eq!(render_pixel_bgra(&old, 1, 0), [30, 20, 10, 0]);
        assert!(rebuild.old_svg_path.is_none());
        assert!(rebuild.new_svg_path.is_none());
    }

    #[test]
    fn file_image_diff_signature_changes_with_payload() {
        let base = gitcomet_core::domain::FileDiffImage {
            path: Path::new("image.png").to_path_buf(),
            old: Some(vec![1, 2, 3]),
            new: Some(vec![4, 5, 6]),
        };
        let changed = gitcomet_core::domain::FileDiffImage {
            path: Path::new("image.png").to_path_buf(),
            old: Some(vec![1, 2, 3, 4]),
            new: Some(vec![4, 5, 6]),
        };

        assert_ne!(
            file_image_diff_signature(&base),
            file_image_diff_signature(&changed)
        );
    }

    #[test]
    fn build_file_image_diff_cache_rebuild_resolves_absolute_preview_path() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = gitcomet_core::domain::FileDiffImage {
            path: Path::new("images/sample.png").to_path_buf(),
            old: Some(vec![1, 2, 3]),
            new: Some(vec![4, 5, 6]),
        };

        let rebuild = build_file_image_diff_cache_rebuild(&file, temp_dir.path());
        let expected = temp_dir.path().join("images/sample.png");
        assert_eq!(rebuild.file_path.as_deref(), Some(expected.as_path()));
        assert!(rebuild.old.is_none());
        assert!(rebuild.new.is_none());
    }

    #[test]
    fn decode_file_image_diff_preview_side_clamps_large_png_to_preview_bounds() {
        let width = IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX * 2;
        let height = IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX;
        let image = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
            width,
            height,
            image::Rgba([12, 34, 56, 255]),
        ));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode png");

        let preview =
            decode_file_image_diff_preview_side(gpui::ImageFormat::Png, &encoded.into_inner());
        let render = preview.render.expect("preview render image");
        let size = render.size(0);
        assert_eq!(size.width.0, IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX as i32);
        assert_eq!(
            size.height.0,
            (IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX / 2) as i32
        );
        assert!(preview.cached_path.is_none());
    }

    #[test]
    fn decode_file_image_diff_preview_side_rasterizes_svg_without_path_fallback() {
        let svg = solid_rect_svg(4096, 2048);
        let preview = decode_file_image_diff_preview_side(gpui::ImageFormat::Svg, &svg);
        let render = preview.render.expect("svg render image");
        let size = render.size(0);
        assert_eq!(size.width.0, IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX as i32);
        assert_eq!(
            size.height.0,
            (IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / 2.0) as i32
        );
        assert!(preview.cached_path.is_none());
    }

    #[test]
    fn render_svg_image_diff_preview_fills_transparent_viewport_white() {
        let svg = inset_rect_svg(4, 4, 1, 1);
        let render = render_svg_image_diff_preview(&svg).expect("svg render image");
        let size = render.size(0);

        assert_eq!(render_pixel_bgra(&render, 0, 0), [255, 255, 255, 255]);
        assert_eq!(
            render_pixel_bgra(
                &render,
                (size.width.0 as usize) / 2,
                (size.height.0 as usize) / 2,
            ),
            [255, 170, 0, 255]
        );
    }

    #[test]
    fn decode_file_image_diff_preview_side_upscales_small_svg_to_target_width() {
        let svg = solid_rect_svg(32, 16);
        let preview = decode_file_image_diff_preview_side(gpui::ImageFormat::Svg, &svg);
        let render = preview.render.expect("svg render image");
        let size = render.size(0);
        assert_eq!(size.width.0, IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX as i32);
        assert_eq!(
            size.height.0,
            (IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX / 2.0) as i32
        );
        assert!(preview.cached_path.is_none());
    }

    #[test]
    fn decode_file_image_diff_preview_side_keeps_svg_path_fallback_for_invalid_svg() {
        let preview =
            decode_file_image_diff_preview_side(gpui::ImageFormat::Svg, b"<not-valid-svg>");
        assert!(preview.render.is_none());
        assert!(preview.cached_path.is_some());
        assert!(preview.cached_path.unwrap().exists());
    }

    #[test]
    fn build_file_image_diff_cache_rebuild_reuses_identical_render_preview() {
        let image = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
            64,
            32,
            image::Rgba([200, 100, 50, 255]),
        ));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode png");
        let bytes = encoded.into_inner();

        let file = gitcomet_core::domain::FileDiffImage {
            path: Path::new("images/sample.png").to_path_buf(),
            old: Some(bytes.clone()),
            new: Some(bytes),
        };

        let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
        let old = rebuild.old.expect("old preview");
        let new = rebuild.new.expect("new preview");
        assert!(Arc::ptr_eq(&old, &new));
    }

    #[test]
    fn build_file_image_diff_cache_rebuild_reuses_identical_svg_render_preview() {
        let svg = solid_rect_svg(2048, 1024);
        let file = gitcomet_core::domain::FileDiffImage {
            path: Path::new("images/sample.svg").to_path_buf(),
            old: Some(svg.clone()),
            new: Some(svg),
        };

        let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
        let old = rebuild.old.expect("old preview");
        let new = rebuild.new.expect("new preview");
        assert!(Arc::ptr_eq(&old, &new));
        assert!(rebuild.old_svg_path.is_none());
        assert!(rebuild.new_svg_path.is_none());
    }

    #[test]
    fn build_file_image_diff_cache_rebuild_rasterizes_distinct_svg_sides_without_fallback_paths() {
        let file = gitcomet_core::domain::FileDiffImage {
            path: Path::new("images/sample.svg").to_path_buf(),
            old: Some(solid_rect_svg(4096, 2048)),
            new: Some(solid_rect_svg(2048, 4096)),
        };

        let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
        let old = rebuild.old.expect("old preview");
        let new = rebuild.new.expect("new preview");
        assert_eq!(
            old.size(0).width.0,
            IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX as i32
        );
        assert_eq!(
            old.size(0).height.0,
            (IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / 2.0) as i32
        );
        assert_eq!(
            new.size(0).width.0,
            (IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / 2.0) as i32
        );
        assert_eq!(
            new.size(0).height.0,
            IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX as i32
        );
        assert!(rebuild.old_svg_path.is_none());
        assert!(rebuild.new_svg_path.is_none());
    }

    #[test]
    fn build_file_image_diff_cache_rebuild_uses_fallback_paths_for_invalid_distinct_svg_sides() {
        let file = gitcomet_core::domain::FileDiffImage {
            path: Path::new("images/sample.svg").to_path_buf(),
            old: Some(b"<not-valid-svg-old>".to_vec()),
            new: Some(b"<not-valid-svg-new>".to_vec()),
        };

        let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
        assert!(rebuild.old.is_none());
        assert!(rebuild.new.is_none());
        assert!(
            rebuild
                .old_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        assert!(
            rebuild
                .new_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
    }

    #[test]
    fn image_format_for_path_detects_known_extensions_case_insensitively() {
        assert_eq!(
            image_format_for_path(Path::new("x.PNG")),
            Some(gpui::ImageFormat::Png)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.JpEg")),
            Some(gpui::ImageFormat::Jpeg)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.GiF")),
            Some(gpui::ImageFormat::Gif)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.webp")),
            Some(gpui::ImageFormat::Webp)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.BMP")),
            Some(gpui::ImageFormat::Bmp)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.TiFf")),
            Some(gpui::ImageFormat::Tiff)
        );
        assert_eq!(
            image_format_for_path(Path::new("x.ICO")),
            Some(gpui::ImageFormat::Ico)
        );
    }

    #[test]
    fn image_format_for_path_returns_none_for_unknown_or_missing_extension() {
        assert_eq!(image_format_for_path(Path::new("x.heic")), None);
        assert_eq!(image_format_for_path(Path::new("x")), None);
    }

    #[test]
    fn decode_file_image_diff_bytes_keeps_non_svg_bytes() {
        let bytes = [1_u8, 2, 3, 4, 5];
        let mut svg_path = None;
        let image =
            decode_file_image_diff_bytes(gpui::ImageFormat::Png, &bytes, Some(&mut svg_path))
                .unwrap();
        assert_eq!(image.format(), gpui::ImageFormat::Png);
        assert_eq!(image.bytes(), bytes);
        assert!(svg_path.is_none());
    }

    #[test]
    fn decode_file_image_diff_bytes_uses_cached_svg_file_for_valid_svg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
<rect width="16" height="16" fill="#00aaff"/>
</svg>"##;
        let mut svg_path = None;
        let image = decode_file_image_diff_bytes(gpui::ImageFormat::Svg, svg, Some(&mut svg_path));
        assert!(image.is_none());
        let svg_path = svg_path.expect("svg should produce a cached file path");
        assert!(svg_path.exists());
        assert_eq!(svg_path.extension().and_then(|s| s.to_str()), Some("svg"));
    }

    #[test]
    fn decode_file_image_diff_bytes_keeps_svg_path_fallback_for_invalid_svg() {
        let mut svg_path = None;
        let image = decode_file_image_diff_bytes(
            gpui::ImageFormat::Svg,
            b"<not-valid-svg>",
            Some(&mut svg_path),
        );
        assert!(image.is_none());
        assert!(svg_path.is_some());
        assert!(svg_path.unwrap().exists());
    }

    #[test]
    fn image_diff_cache_reuses_the_content_addressed_file() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = [0_u8, 0, 1, 0, 1, 0, 16, 16];

        let path = image_diff_cache_path(dir.path(), &bytes, "ico").expect("cached path");
        let reused = image_diff_cache_path(dir.path(), &bytes, "ico").expect("second cached path");

        assert_eq!(path, reused);
        assert_eq!(path.extension().and_then(|s| s.to_str()), Some("ico"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(std::fs::symlink_metadata(&path).unwrap().is_file());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "a cache hit must not write a second file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn image_diff_cache_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        // A content-addressed name is guessable, so the directory holding it
        // must not be one other users can create entries in.
        let dir = image_diff_cache_dir().expect("cache dir");
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn image_diff_cache_dir_rejects_a_world_writable_directory() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(IMAGE_DIFF_CACHE_DIR_NAME);
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();

        let result = image_diff_cache_dir_in(root.path());

        assert!(result.is_err(), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn image_diff_cache_does_not_reuse_a_planted_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();

        // Plant the symlink under the exact name this content hashes to.
        let planted = image_diff_cache_path(dir.path(), b"preview", "svg").unwrap();
        std::fs::remove_file(&planted).unwrap();
        std::os::unix::fs::symlink(&victim, &planted).unwrap();

        let err = image_diff_cache_path(dir.path(), b"preview", "svg")
            .expect_err("a planted symlink must never be served as a cache hit");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    }

    #[test]
    fn cleanup_image_diff_cache_dir_removes_stale_prefixed_files() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let stale = write_test_file(
            temp_dir.path(),
            "gitcomet-image-diff-stale.svg",
            b"old-cache",
        );
        let non_cache = write_test_file(temp_dir.path(), "keep-me.txt", b"keep");

        cleanup_image_diff_cache_dir(
            temp_dir.path(),
            std::time::Duration::from_secs(60),
            u64::MAX,
            std::time::SystemTime::now() + std::time::Duration::from_secs(60 * 60),
        )
        .expect("cleanup");

        assert!(!stale.exists());
        assert!(non_cache.exists());
    }

    #[test]
    fn cleanup_image_diff_cache_dir_prunes_to_max_total_size() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let a = write_test_file(temp_dir.path(), "gitcomet-image-diff-a.svg", b"1234");
        let b = write_test_file(temp_dir.path(), "gitcomet-image-diff-b.svg", b"1234");
        let c = write_test_file(temp_dir.path(), "gitcomet-image-diff-c.svg", b"1234");
        let non_cache = write_test_file(temp_dir.path(), "unrelated.bin", b"1234567890");

        cleanup_image_diff_cache_dir(
            temp_dir.path(),
            std::time::Duration::from_secs(60 * 60 * 24),
            8,
            std::time::SystemTime::now(),
        )
        .expect("cleanup");

        let cache_paths = [&a, &b, &c];
        let remaining_count = cache_paths.iter().filter(|path| path.exists()).count();
        assert_eq!(remaining_count, 2);

        let remaining_total = cache_paths
            .iter()
            .filter(|path| path.exists())
            .map(|path| std::fs::metadata(path).expect("metadata").len())
            .sum::<u64>();
        assert!(remaining_total <= 8);
        assert!(non_cache.exists());
    }
}
