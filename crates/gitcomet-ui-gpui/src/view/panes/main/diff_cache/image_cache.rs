use super::*;
use crate::view::diff_utils::image_format_for_path;
use std::hash::Hasher;
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE_DIFF_CACHE_FILE_PREFIX: &str = "gitcomet-image-diff-";
const IMAGE_DIFF_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);
const IMAGE_DIFF_CACHE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_DIFF_CACHE_CLEANUP_WRITE_INTERVAL: usize = 16;
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

impl MainPaneView {
    pub(in crate::view) fn ensure_file_image_diff_cache(&mut self, _cx: &mut gpui::Context<Self>) {
        struct Rebuild {
            repo_id: RepoId,
            diff_file_rev: u64,
            diff_target: Option<DiffTarget>,
            file_path: Option<std::path::PathBuf>,
            old: Option<Arc<gpui::Image>>,
            new: Option<Arc<gpui::Image>>,
            old_svg_path: Option<std::path::PathBuf>,
            new_svg_path: Option<std::path::PathBuf>,
        }

        enum Action {
            Clear,
            Noop,
            Reset {
                repo_id: RepoId,
                diff_file_rev: u64,
                diff_target: Option<DiffTarget>,
            },
            Rebuild(Rebuild),
        }

        let action = (|| {
            let Some(repo) = self.active_repo() else {
                return Action::Clear;
            };

            if !Self::is_file_diff_target(repo.diff_state.diff_target.as_ref()) {
                return Action::Clear;
            }

            if self.file_image_diff_cache_repo_id == Some(repo.id)
                && self.file_image_diff_cache_rev == repo.diff_state.diff_file_rev
                && self.file_image_diff_cache_target.as_ref()
                    == repo.diff_state.diff_target.as_ref()
            {
                return Action::Noop;
            }

            let repo_id = repo.id;
            let diff_file_rev = repo.diff_state.diff_file_rev;
            let diff_target = repo.diff_state.diff_target.clone();

            let Loadable::Ready(file_opt) = &repo.diff_state.diff_file_image else {
                return Action::Reset {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                };
            };
            let Some(file) = file_opt.as_ref() else {
                return Action::Reset {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                };
            };

            let format = image_format_for_path(&file.path);
            let is_ico = file
                .path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"));
            let workdir = &repo.spec.workdir;
            let file_path = Some(if file.path.is_absolute() {
                file.path.clone()
            } else {
                workdir.join(&file.path)
            });

            let mut old_svg_path = None;
            let mut new_svg_path = None;
            let old = file.old.as_ref().and_then(|bytes| {
                if is_ico {
                    old_svg_path = cached_image_diff_path(bytes, "ico");
                    None
                } else {
                    format.and_then(|format| {
                        decode_file_image_diff_bytes(format, bytes, Some(&mut old_svg_path))
                    })
                }
            });
            let new = file.new.as_ref().and_then(|bytes| {
                if is_ico {
                    new_svg_path = cached_image_diff_path(bytes, "ico");
                    None
                } else {
                    format.and_then(|format| {
                        decode_file_image_diff_bytes(format, bytes, Some(&mut new_svg_path))
                    })
                }
            });

            Action::Rebuild(Rebuild {
                repo_id,
                diff_file_rev,
                diff_target,
                file_path,
                old,
                new,
                old_svg_path,
                new_svg_path,
            })
        })();

        match action {
            Action::Noop => {}
            Action::Clear => {
                self.file_image_diff_cache_repo_id = None;
                self.file_image_diff_cache_target = None;
                self.file_image_diff_cache_rev = 0;
                self.file_image_diff_cache_path = None;
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;
            }
            Action::Reset {
                repo_id,
                diff_file_rev,
                diff_target,
            } => {
                self.file_image_diff_cache_repo_id = Some(repo_id);
                self.file_image_diff_cache_rev = diff_file_rev;
                self.file_image_diff_cache_target = diff_target;
                self.file_image_diff_cache_path = None;
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;
            }
            Action::Rebuild(rebuild) => {
                self.file_image_diff_cache_repo_id = Some(rebuild.repo_id);
                self.file_image_diff_cache_rev = rebuild.diff_file_rev;
                self.file_image_diff_cache_target = rebuild.diff_target;
                self.file_image_diff_cache_path = rebuild.file_path;
                self.file_image_diff_cache_old = rebuild.old;
                self.file_image_diff_cache_new = rebuild.new;
                self.file_image_diff_cache_old_svg_path = rebuild.old_svg_path;
                self.file_image_diff_cache_new_svg_path = rebuild.new_svg_path;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_test_file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write test file");
        path
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
