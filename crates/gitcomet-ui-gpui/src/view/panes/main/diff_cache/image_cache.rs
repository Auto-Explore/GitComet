use super::*;
use crate::view::diff_utils::{fill_svg_viewport_white, image_format_for_path};
use image::AnimationDecoder as _;
use rustc_hash::FxHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE_DIFF_CACHE_FILE_PREFIX: &str = "gitcomet-image-diff-";
const IMAGE_DIFF_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);
const IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL: usize = 16;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX: u32 = 1920;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES: usize = 120;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_BYTES: usize = 64 * 1024 * 1024;
const IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX: f32 = 640.0;
const IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX: f32 = 1024.0;
static IMAGE_DIFF_SVG_USVG_OPTIONS: std::sync::LazyLock<resvg::usvg::Options<'static>> =
    std::sync::LazyLock::new(resvg::usvg::Options::default);
static IMAGE_DIFF_CACHE_STARTUP_CLEANUP: std::sync::Once = std::sync::Once::new();
static IMAGE_DIFF_CACHE_WRITE_COUNT: AtomicUsize = AtomicUsize::new(0);

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

fn cleanup_image_diff_cache_now() {
    let _ = cleanup_image_diff_cache_dir(
        &std::env::temp_dir(),
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
    // Read from a snapshot so the result cannot depend on scan direction and
    // so this remains a one-texel extension rather than a flood fill.
    let source = buffer.clone();
    for y in 0..height {
        for x in 0..width {
            if source.get_pixel(x, y).0[3] != 0 {
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
                    let neighbour = source.get_pixel(neighbour_x, neighbour_y).0;
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
    type Pixel = image::Rgba<f32>;

    fn dimensions(&self) -> (u32, u32) {
        self.0.dimensions()
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let pixel = self.0.get_pixel(x, y).0;
        let alpha = f32::from(pixel[3]) / 255.0;
        image::Rgba([
            f32::from(pixel[0]) / 255.0 * alpha,
            f32::from(pixel[1]) / 255.0 * alpha,
            f32::from(pixel[2]) / 255.0 * alpha,
            alpha,
        ])
    }
}

fn alpha_correct_thumbnail(buffer: image::RgbaImage, max_edge_px: u32) -> image::RgbaImage {
    let (width, height) = buffer.dimensions();
    let scale =
        (f64::from(max_edge_px) / f64::from(width)).min(f64::from(max_edge_px) / f64::from(height));
    let resized_width = (f64::from(width) * scale).round().max(1.0) as u32;
    let resized_height = (f64::from(height) * scale).round().max(1.0) as u32;

    // Feed image-rs a lazy premultiplied view. This operates on the exact byte
    // values the GPUI atlas samples without allocating another full-size float
    // copy of a potentially very large source image.
    let resized = image::imageops::resize(
        &PremultipliedRgbaView(&buffer),
        resized_width,
        resized_height,
        image::imageops::FilterType::Triangle,
    );

    let mut straight_samples = Vec::with_capacity(resized.as_raw().len());
    for pixel in resized.pixels() {
        let alpha = pixel.0[3].clamp(0.0, 1.0);
        for channel in 0..3 {
            let straight = if alpha > f32::EPSILON {
                (pixel.0[channel] / alpha).clamp(0.0, 1.0)
            } else {
                0.0
            };
            straight_samples.push((straight * 255.0).round() as u8);
        }
        straight_samples.push((alpha * 255.0).round() as u8);
    }

    image::RgbaImage::from_raw(resized_width, resized_height, straight_samples)
        .expect("RGBA sample count follows the resized dimensions")
}

fn prepare_raster_preview_frame(mut frame: image::Frame) -> image::Frame {
    let delay = frame.delay();
    let left = frame.left();
    let top = frame.top();
    let has_transparency = frame.buffer().pixels().any(|pixel| pixel.0[3] < 255);
    let oversized =
        frame.buffer().width().max(frame.buffer().height()) > IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX;

    let mut buffer = if oversized && has_transparency {
        alpha_correct_thumbnail(frame.into_buffer(), IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX)
    } else if oversized {
        image::DynamicImage::ImageRgba8(frame.into_buffer())
            .thumbnail(
                IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX,
                IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX,
            )
            .into_rgba8()
    } else {
        std::mem::take(frame.buffer_mut())
    };

    if has_transparency {
        extend_transparent_edge_rgb(&mut buffer);
    }
    for pixel in buffer.as_chunks_mut::<4>().0 {
        swap_rgba_to_bgra(pixel);
    }

    image::Frame::from_parts(buffer, left, top, delay)
}

fn prepare_raster_preview_animation_frames(
    mut decoded_frames: impl Iterator<Item = image::ImageResult<image::Frame>>,
    max_frames: usize,
    max_bytes: usize,
) -> Vec<image::Frame> {
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
        let Ok(frame) = decoded else {
            continue;
        };
        let frame = prepare_raster_preview_frame(frame);
        let frame_bytes = frame.buffer().as_raw().len();
        let Some(next_retained_bytes) = retained_bytes.checked_add(frame_bytes) else {
            break;
        };
        if next_retained_bytes > max_bytes {
            break;
        }

        retained_bytes = next_retained_bytes;
        frames.push(frame);
    }

    frames
}

fn decode_oriented_static_raster_frame(
    mut decoder: impl image::ImageDecoder,
) -> Option<image::Frame> {
    let orientation = decoder.orientation().ok()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).ok()?;
    decoded.apply_orientation(orientation);
    Some(prepare_raster_preview_frame(image::Frame::new(
        decoded.into_rgba8(),
    )))
}

fn decode_raster_preview_frames(
    format: gpui::ImageFormat,
    bytes: &[u8],
) -> Option<Vec<image::Frame>> {
    let image_format = image_rs_format_for_diff_preview(format)?;
    let frames = match format {
        gpui::ImageFormat::Gif => {
            let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
            prepare_raster_preview_animation_frames(
                decoder.into_frames(),
                IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES,
                IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_BYTES,
            )
        }
        gpui::ImageFormat::Webp => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if decoder.has_animation() {
                let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                prepare_raster_preview_animation_frames(
                    decoder.into_frames(),
                    IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES,
                    IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_BYTES,
                )
            } else {
                vec![decode_oriented_static_raster_frame(decoder)?]
            }
        }
        _ => {
            let decoder = image::ImageReader::with_format(Cursor::new(bytes), image_format)
                .into_decoder()
                .ok()?;
            vec![decode_oriented_static_raster_frame(decoder)?]
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
    let frames = decode_raster_preview_frames(format, bytes)?;
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
    use std::io::Write;

    cleanup_image_diff_cache_startup_once();

    let mut hasher = FxHasher::default();
    hasher.write(bytes);
    hasher.write(extension.as_bytes());
    let path = std::env::temp_dir().join(format!(
        "{IMAGE_DIFF_CACHE_FILE_PREFIX}{:016x}-{}.{}",
        hasher.finish(),
        bytes.len(),
        extension
    ));
    if path.is_file() {
        return Some(path);
    }

    let mut file = tempfile::Builder::new()
        .prefix(IMAGE_DIFF_CACHE_FILE_PREFIX)
        .suffix(".tmp")
        .tempfile_in(std::env::temp_dir())
        .ok()?;
    file.as_file_mut().write_all(bytes).ok()?;

    match file.persist_noclobber(&path) {
        Ok(_) => {
            maybe_cleanup_image_diff_cache_on_write();
            Some(path)
        }
        Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => Some(path),
        Err(_) => None,
    }
}

struct ImageDiffCacheRebuild {
    file_path: Option<std::path::PathBuf>,
    old: Option<Arc<gpui::RenderImage>>,
    new: Option<Arc<gpui::RenderImage>>,
    old_svg_path: Option<std::path::PathBuf>,
    new_svg_path: Option<std::path::PathBuf>,
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
    let is_ico = file
        .path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"));
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
        };
    };

    let (mut old_preview, mut new_preview) =
        decode_file_image_diff_preview_pair(format, file.old.as_deref(), file.new.as_deref());
    if is_ico {
        if old_preview.render.is_none() {
            old_preview.cached_path = file
                .old
                .as_deref()
                .and_then(|bytes| cached_image_diff_path(bytes, "ico"));
        }
        if new_preview.render.is_none() {
            new_preview.cached_path = file
                .new
                .as_deref()
                .and_then(|bytes| cached_image_diff_path(bytes, "ico"));
        }
    }
    ImageDiffCacheRebuild {
        file_path,
        old: old_preview.render,
        new: new_preview.render,
        old_svg_path: old_preview.cached_path,
        new_svg_path: new_preview.cached_path,
    }
}

impl MainPaneView {
    fn reset_file_image_diff_cache_data(&mut self) {
        self.file_image_diff_cache_content_signature = None;
        self.file_image_diff_cache_inflight = None;
        self.file_image_diff_cache_path = None;
        self.file_image_diff_cache_old = None;
        self.file_image_diff_cache_new = None;
        self.file_image_diff_cache_old_svg_path = None;
        self.file_image_diff_cache_new_svg_path = None;
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
            self.reset_file_image_diff_cache_data();
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
        self.reset_file_image_diff_cache_data();

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
    fn prepared_raster_frame_preserves_timing_and_extends_bgra_edge_color() {
        let delay = image::Delay::from_numer_denom_ms(17, 1);
        let frame = image::Frame::from_parts(
            image::RgbaImage::from_raw(3, 1, vec![0, 0, 0, 0, 12, 34, 56, 128, 0, 0, 0, 0])
                .expect("test buffer"),
            2,
            3,
            delay,
        );

        let prepared = prepare_raster_preview_frame(frame);

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
        let prepared = prepare_raster_preview_animation_frames(frames, 2, usize::MAX);

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
        let prepared = prepare_raster_preview_animation_frames(frames, usize::MAX, 32);
        let retained_bytes = prepared
            .iter()
            .map(|frame| frame.buffer().as_raw().len())
            .sum::<usize>();

        assert_eq!(prepared.len(), 2);
        assert_eq!(retained_bytes, 32);
        assert_eq!(decoded_for_byte_limit.get(), 3);
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
    fn cached_image_diff_path_writes_ico_cache_file() {
        let bytes = [0_u8, 0, 1, 0, 1, 0, 16, 16];
        let path = cached_image_diff_path(&bytes, "ico").expect("cached path");
        let same_path = cached_image_diff_path(&bytes, "ico").expect("second cached path");
        assert!(path.exists());
        assert_eq!(path, same_path);
        assert_eq!(path.extension().and_then(|s| s.to_str()), Some("ico"));
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
