use gpui::SharedString;
use rustc_hash::FxHashMap as HashMap;
#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::cell::Cell;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct PathDisplayBenchSnapshot {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_clears: u64,
}

#[cfg(not(windows))]
type PathDisplayCacheMap = HashMap<SharedString, SharedString>;
#[cfg(windows)]
type PathDisplayCacheMap = HashMap<PathBuf, SharedString>;

/// A bounded two-generation cache. Keeping the previous generation avoids the
/// all-or-nothing cliff when a large repo first crosses the cache cap.
pub(super) struct PathDisplayCache {
    recent: PathDisplayCacheMap,
    previous: PathDisplayCacheMap,
}

impl Default for PathDisplayCache {
    fn default() -> Self {
        Self {
            recent: HashMap::with_capacity_and_hasher(Self::RECENT_MAX_ENTRIES, Default::default()),
            previous: HashMap::with_capacity_and_hasher(
                Self::RECENT_MAX_ENTRIES,
                Default::default(),
            ),
        }
    }
}

impl PathDisplayCache {
    const MAX_ENTRIES: usize = 8_192;
    const RECENT_MAX_ENTRIES: usize = Self::MAX_ENTRIES / 2;

    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn clear(&mut self) {
        self.recent.clear();
        self.previous.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.recent.len() + self.previous.len()
    }

    fn rotate_generations(&mut self) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_clear);
        self.previous.clear();
        std::mem::swap(&mut self.previous, &mut self.recent);
        debug_assert!(self.recent.is_empty());
    }
}

#[cfg(windows)]
pub(super) fn path_display_string(path: &Path) -> String {
    format_windows_path_for_display(path.display().to_string())
}

#[cfg(not(windows))]
pub(super) fn path_display_string(path: &Path) -> String {
    path.to_str()
        .map(str::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub(super) fn path_display_shared(path: &Path) -> SharedString {
    path_display_string(path).into()
}

/// Fast path that skips the intermediate `String` allocation on non-Windows
/// by constructing `SharedString` directly from `&str` → `Arc<str>`.
#[cfg(not(windows))]
pub(in crate::view) fn path_display_shared_fast(path: &Path) -> SharedString {
    match path.to_str() {
        Some(s) => SharedString::new(s),
        None => path_display_shared(path),
    }
}

#[cfg(windows)]
pub(in crate::view) fn path_display_shared_fast(path: &Path) -> SharedString {
    path_display_shared(path)
}

pub(super) fn cached_path_display(cache: &mut PathDisplayCache, path: &Path) -> SharedString {
    #[cfg(not(windows))]
    let Some(path_key) = path.to_str() else {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_miss);
        return path_display_shared(path);
    };

    #[cfg(not(windows))]
    if let Some(s) = cache.recent.get(path_key) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        return s.clone();
    }

    #[cfg(windows)]
    if let Some(s) = cache.recent.get(path) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        return s.clone();
    }

    // Skip the previous-generation lookup entirely when it is empty.
    // This avoids a redundant hash + probe on cold caches and after clear().
    #[cfg(not(windows))]
    if !cache.previous.is_empty()
        && let Some(s) = cache.previous.remove(path_key)
    {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        // Promote to recent so subsequent lookups hit the fast path.
        // Check capacity before inserting to maintain the size invariant.
        if cache.recent.len() >= PathDisplayCache::RECENT_MAX_ENTRIES {
            cache.rotate_generations();
        }
        cache.recent.insert(s.clone(), s.clone());
        return s;
    }

    #[cfg(windows)]
    if !cache.previous.is_empty()
        && let Some(s) = cache.previous.remove(path)
    {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        // Promote to recent so subsequent lookups hit the fast path.
        // Check capacity before inserting to maintain the size invariant.
        if cache.recent.len() >= PathDisplayCache::RECENT_MAX_ENTRIES {
            cache.rotate_generations();
        }
        cache.recent.insert(path.to_path_buf(), s.clone());
        return s;
    }

    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_miss);
    if cache.recent.len() >= PathDisplayCache::RECENT_MAX_ENTRIES {
        if cache.previous.is_empty() || cache.len() < PathDisplayCache::MAX_ENTRIES {
            cache.rotate_generations();
        } else {
            // Once both generations are full, keep the hot working set and
            // skip caching one-off overflow misses instead of invalidating the
            // entire previous generation on every long unique tail.
            return path_display_shared_fast(path);
        }
    }

    #[cfg(not(windows))]
    let s = SharedString::new(path_key);

    #[cfg(windows)]
    let s = path_display_shared_fast(path);

    #[cfg(not(windows))]
    cache.recent.insert(s.clone(), s.clone());

    #[cfg(windows)]
    cache.recent.insert(path.to_path_buf(), s.clone());

    s
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn bench_snapshot() -> PathDisplayBenchSnapshot {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::snapshot)
    }
    #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
    {
        PathDisplayBenchSnapshot::default()
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn bench_reset() {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::reset);
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
struct PathDisplayBenchCounters {
    cache_hits: Cell<u64>,
    cache_misses: Cell<u64>,
    cache_clears: Cell<u64>,
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
impl PathDisplayBenchCounters {
    fn new() -> Self {
        Self {
            cache_hits: Cell::new(0),
            cache_misses: Cell::new(0),
            cache_clears: Cell::new(0),
        }
    }

    fn record_hit(&self) {
        self.cache_hits.set(self.cache_hits.get().saturating_add(1));
    }

    fn record_miss(&self) {
        self.cache_misses
            .set(self.cache_misses.get().saturating_add(1));
    }

    fn record_clear(&self) {
        self.cache_clears
            .set(self.cache_clears.get().saturating_add(1));
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn snapshot(&self) -> PathDisplayBenchSnapshot {
        PathDisplayBenchSnapshot {
            cache_hits: self.cache_hits.get(),
            cache_misses: self.cache_misses.get(),
            cache_clears: self.cache_clears.get(),
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn reset(&self) {
        self.cache_hits.set(0);
        self.cache_misses.set(0);
        self.cache_clears.set(0);
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
thread_local! {
    static PATH_DISPLAY_BENCH_COUNTERS: PathDisplayBenchCounters = PathDisplayBenchCounters::new();
}

#[cfg(windows)]
fn format_windows_path_for_display(mut path: String) -> String {
    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        path = format!(r"\\{stripped}");
    } else if let Some(stripped) = path.strip_prefix(r"\\?\") {
        path = stripped.to_string();
    }
    path.replace('\\', "/")
}

#[cfg(not(windows))]
#[allow(dead_code)] // cross-platform stub; only called from tests on non-windows
fn format_windows_path_for_display(path: String) -> String {
    path
}

#[cfg(test)]
mod tests {
    use super::{
        PathDisplayBenchSnapshot, PathDisplayCache, bench_reset, bench_snapshot,
        cached_path_display, format_windows_path_for_display,
    };
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_disk_prefix_and_uses_forward_slashes() {
        let formatted =
            format_windows_path_for_display(r"\\?\C:\Users\sanni\git\GitComet".to_string());
        assert_eq!(formatted, "C:/Users/sanni/git/GitComet");
    }

    #[cfg(windows)]
    #[test]
    fn strips_verbatim_unc_prefix_and_uses_forward_slashes() {
        let formatted = format_windows_path_for_display(r"\\?\UNC\server\share\repo".to_string());
        assert_eq!(formatted, "//server/share/repo");
    }

    #[cfg(not(windows))]
    #[test]
    fn leaves_non_windows_path_unchanged() {
        let formatted = format_windows_path_for_display("/tmp/repo".to_string());
        assert_eq!(formatted, "/tmp/repo");
    }

    #[test]
    fn bench_counters_track_hits_misses_and_clears() {
        bench_reset();

        let mut cache = PathDisplayCache::default();
        let path = PathBuf::from("src/lib.rs");
        let _ = cached_path_display(&mut cache, &path);
        let _ = cached_path_display(&mut cache, &path);

        for ix in 0..8_193 {
            let extra = PathBuf::from(format!("src/module_{ix}/file_{ix}.rs"));
            let _ = cached_path_display(&mut cache, &extra);
        }

        assert_eq!(
            bench_snapshot(),
            PathDisplayBenchSnapshot {
                cache_hits: 1,
                cache_misses: 8_194,
                cache_clears: 1,
            }
        );
        assert!(cache.len() <= PathDisplayCache::MAX_ENTRIES);
    }

    #[cfg(not(windows))]
    #[test]
    fn utf8_cache_reuses_shared_string_for_key_and_value() {
        let mut cache = PathDisplayCache::default();
        let path = PathBuf::from("src/lib.rs");
        let display = cached_path_display(&mut cache, &path);

        let (cached_key, cached_value) = cache.recent.iter().next().unwrap();
        let key_arc: Arc<str> = cached_key.clone().into();
        let value_arc: Arc<str> = cached_value.clone().into();
        let display_arc: Arc<str> = display.into();

        assert!(Arc::ptr_eq(&key_arc, &value_arc));
        assert!(Arc::ptr_eq(&value_arc, &display_arc));
    }

    #[test]
    fn bench_counters_are_isolated_per_thread() {
        bench_reset();

        let ready = Arc::new(Barrier::new(2));
        let ready_thread = ready.clone();
        let handle = std::thread::spawn(move || {
            bench_reset();
            let mut cache = PathDisplayCache::default();
            let path = PathBuf::from("src/thread.rs");
            let _ = cached_path_display(&mut cache, &path);
            let _ = cached_path_display(&mut cache, &path);
            ready_thread.wait();
            assert_eq!(
                bench_snapshot(),
                PathDisplayBenchSnapshot {
                    cache_hits: 1,
                    cache_misses: 1,
                    cache_clears: 0,
                }
            );
        });

        let mut cache = PathDisplayCache::default();
        let path = PathBuf::from("src/main.rs");
        let _ = cached_path_display(&mut cache, &path);
        ready.wait();
        assert_eq!(
            bench_snapshot(),
            PathDisplayBenchSnapshot {
                cache_hits: 0,
                cache_misses: 1,
                cache_clears: 0,
            }
        );

        handle.join().expect("join path_display test thread");
    }
}
