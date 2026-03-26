use super::*;
use crate::view::diff_utils::image_format_for_path;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE_DIFF_CACHE_FILE_PREFIX: &str = "gitcomet-image-diff-";
const IMAGE_DIFF_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);
const IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL: usize = 16;
const IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX: u32 = 1920;
const IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX: f32 = 640.0;
const IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX: f32 = 1024.0;
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
        gpui::ImageFormat::Svg => None,
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

fn render_svg_image_diff_preview(svg_bytes: &[u8]) -> Option<Arc<gpui::RenderImage>> {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &resvg::usvg::Options::default()).ok()?;
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
    let transform = resvg::tiny_skia::Transform::from_scale(
        raster_width as f32 / svg_width,
        raster_height as f32 / svg_height,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut buffer = image::ImageBuffer::from_raw(pixmap.width(), pixmap.height(), pixmap.take())?;
    for pixel in buffer.chunks_exact_mut(4) {
        swap_rgba_pa_to_bgra(pixel);
    }

    Some(render_image_from_bgra8(buffer))
}

fn render_raster_image_diff_preview(
    format: gpui::ImageFormat,
    bytes: &[u8],
) -> Option<Arc<gpui::RenderImage>> {
    let image_format = image_rs_format_for_diff_preview(format)?;
    let decoded = image::load_from_memory_with_format(bytes, image_format).ok()?;
    let decoded = if decoded.width().max(decoded.height()) > IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX {
        decoded.thumbnail(
            IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX,
            IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX,
        )
    } else {
        decoded
    };

    let mut data = decoded.into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        swap_rgba_to_bgra(pixel);
    }

    Some(render_image_from_bgra8(data))
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
    let mut hasher = rustc_hash::FxHasher::default();
    file.path.hash(&mut hasher);
    file.old.hash(&mut hasher);
    file.new.hash(&mut hasher);
    hasher.finish()
}

fn cached_image_diff_path(bytes: &[u8], extension: &str) -> Option<std::path::PathBuf> {
    use std::io::Write;

    cleanup_image_diff_cache_startup_once();

    let mut hasher = rustc_hash::FxHasher::default();
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

fn cached_image_diff_path_pair(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    extension: &str,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    if old.is_some() && old == new {
        let path = old.and_then(|bytes| cached_image_diff_path(bytes, extension));
        return (path.clone(), path);
    }

    (
        old.and_then(|bytes| cached_image_diff_path(bytes, extension)),
        new.and_then(|bytes| cached_image_diff_path(bytes, extension)),
    )
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
        let old_task = old
            .map(|bytes| scope.spawn(move || decode_file_image_diff_preview_side(format, bytes)));
        let new_task = new
            .map(|bytes| scope.spawn(move || decode_file_image_diff_preview_side(format, bytes)));

        let old_preview = old_task.map_or_else(DecodedImageDiffPreview::default, |task| {
            task.join().unwrap_or_default()
        });
        let new_preview = new_task.map_or_else(DecodedImageDiffPreview::default, |task| {
            task.join().unwrap_or_default()
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
        file.path.clone()
    } else {
        workdir.join(&file.path)
    });

    if is_ico {
        let (old_svg_path, new_svg_path) =
            cached_image_diff_path_pair(file.old.as_deref(), file.new.as_deref(), "ico");
        return ImageDiffCacheRebuild {
            file_path,
            old: None,
            new: None,
            old_svg_path,
            new_svg_path,
        };
    }

    let Some(format) = format else {
        return ImageDiffCacheRebuild {
            file_path,
            old: None,
            new: None,
            old_svg_path: None,
            new_svg_path: None,
        };
    };

    let (old_preview, new_preview) =
        decode_file_image_diff_preview_pair(format, file.old.as_deref(), file.new.as_deref());
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
        let Some((repo_id, diff_file_rev, diff_target, workdir, file)) = (|| {
            let repo = self.active_repo()?;
            if !Self::is_file_diff_target(repo.diff_state.diff_target.as_ref()) {
                return None;
            }

            let file = match &repo.diff_state.diff_file_image {
                Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                _ => None,
            };

            Some((
                repo.id,
                repo.diff_state.diff_file_rev,
                repo.diff_state.diff_target.clone(),
                repo.spec.workdir.clone(),
                file,
            ))
        })() else {
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
            && self.file_image_diff_cache_target == diff_target;

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
        self.file_image_diff_cache_target = diff_target;
        self.reset_file_image_diff_cache_data();

        let Some(file) = file else {
            return;
        };
        let content_signature =
            file_content_signature.unwrap_or_else(|| file_image_diff_signature(file.as_ref()));

        self.file_image_diff_cache_seq = self.file_image_diff_cache_seq.wrapping_add(1);
        let seq = self.file_image_diff_cache_seq;
        self.file_image_diff_cache_inflight = Some(seq);

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
                        || this.file_image_diff_cache_target != diff_target_for_task
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

    fn write_test_file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write test file");
        path
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
    }

    #[test]
    fn image_format_for_path_returns_none_for_unknown_or_missing_extension() {
        assert_eq!(image_format_for_path(Path::new("x.heic")), None);
        assert_eq!(image_format_for_path(Path::new("x.ico")), None);
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
