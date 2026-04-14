use gpui::SharedString;
#[cfg(windows)]
use rustc_hash::FxHashMap as HashMap;
#[cfg(not(windows))]
use rustc_hash::FxHashMap as HashMap;
#[cfg(not(windows))]
use rustc_hash::FxHasher;
#[cfg(not(windows))]
use smallvec::SmallVec;
#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::cell::Cell;
#[cfg(not(windows))]
use std::hash::{Hash, Hasher};
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
type PathDisplayCacheBucket = SmallVec<[SharedString; 1]>;
#[cfg(not(windows))]
type PathDisplayCacheMap = HashMap<u64, PathDisplayCacheBucket>;
#[cfg(windows)]
type PathDisplayCacheMap = HashMap<PathBuf, SharedString>;

/// A bounded two-generation cache. Keeping the previous generation avoids the
/// all-or-nothing cliff when a large repo first crosses the cache cap.
pub(super) struct PathDisplayCache {
    recent: PathDisplayCacheMap,
    previous: PathDisplayCacheMap,
    recent_entries: usize,
    previous_entries: usize,
    #[cfg(not(windows))]
    present_hash_counts: HashMap<u64, u16>,
    #[cfg(not(windows))]
    overflow_tail_active: bool,
}

impl Default for PathDisplayCache {
    fn default() -> Self {
        Self {
            #[cfg(not(windows))]
            recent: HashMap::with_capacity_and_hasher(Self::RECENT_MAX_ENTRIES, Default::default()),
            #[cfg(windows)]
            recent: HashMap::with_capacity_and_hasher(Self::RECENT_MAX_ENTRIES, Default::default()),
            #[cfg(not(windows))]
            previous: HashMap::with_capacity_and_hasher(
                Self::RECENT_MAX_ENTRIES,
                Default::default(),
            ),
            #[cfg(windows)]
            previous: HashMap::with_capacity_and_hasher(
                Self::RECENT_MAX_ENTRIES,
                Default::default(),
            ),
            recent_entries: 0,
            previous_entries: 0,
            #[cfg(not(windows))]
            present_hash_counts: HashMap::with_capacity_and_hasher(
                Self::MAX_ENTRIES,
                Default::default(),
            ),
            #[cfg(not(windows))]
            overflow_tail_active: false,
        }
    }
}

impl PathDisplayCache {
    const MAX_ENTRIES: usize = 8_192;
    const RECENT_MAX_ENTRIES: usize = Self::MAX_ENTRIES / 2;

    pub(super) fn len(&self) -> usize {
        self.recent_entries + self.previous_entries
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[allow(dead_code)]
    pub(in crate::view) fn clear(&mut self) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_clear);

        self.recent.clear();
        self.previous.clear();
        self.recent_entries = 0;
        self.previous_entries = 0;

        #[cfg(not(windows))]
        {
            self.present_hash_counts.clear();
            self.overflow_tail_active = false;
        }
    }

    fn rotate_generations(&mut self) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_clear);
        #[cfg(not(windows))]
        {
            remove_generation_hashes(&self.previous, &mut self.present_hash_counts);
            self.overflow_tail_active = false;
        }
        self.previous.clear();
        self.previous_entries = 0;
        std::mem::swap(&mut self.previous, &mut self.recent);
        std::mem::swap(&mut self.previous_entries, &mut self.recent_entries);
        debug_assert!(self.recent.is_empty());
        debug_assert_eq!(self.recent_entries, 0);
    }
}

#[cfg(not(windows))]
#[inline]
fn path_display_cache_key(path_key: &str) -> u64 {
    let mut hasher = FxHasher::default();
    path_key.hash(&mut hasher);
    hasher.finish()
}

#[cfg(not(windows))]
#[inline]
fn bucket_find<'a>(bucket: &'a PathDisplayCacheBucket, path_key: &str) -> Option<&'a SharedString> {
    bucket.iter().find(|label| label.as_ref() == path_key)
}

#[cfg(not(windows))]
#[inline]
fn hash_counts_contains(hash_counts: &HashMap<u64, u16>, path_key_hash: u64) -> bool {
    hash_counts.contains_key(&path_key_hash)
}

#[cfg(not(windows))]
#[inline]
fn hash_counts_insert(hash_counts: &mut HashMap<u64, u16>, path_key_hash: u64) {
    hash_counts
        .entry(path_key_hash)
        .and_modify(|count| *count = count.saturating_add(1))
        .or_insert(1);
}

#[cfg(not(windows))]
#[inline]
fn hash_counts_remove(
    hash_counts: &mut HashMap<u64, u16>,
    path_key_hash: u64,
    removed_entries: usize,
) {
    let Ok(removed_entries) = u16::try_from(removed_entries) else {
        hash_counts.remove(&path_key_hash);
        return;
    };
    let Some(current_count) = hash_counts.get(&path_key_hash).copied() else {
        return;
    };
    if current_count <= removed_entries {
        hash_counts.remove(&path_key_hash);
    } else if let Some(count) = hash_counts.get_mut(&path_key_hash) {
        *count = current_count - removed_entries;
    }
}

#[cfg(not(windows))]
fn remove_generation_hashes(cache: &PathDisplayCacheMap, hash_counts: &mut HashMap<u64, u16>) {
    for (&path_key_hash, bucket) in cache.iter() {
        hash_counts_remove(hash_counts, path_key_hash, bucket.len());
    }
}

#[cfg(not(windows))]
#[inline]
fn cache_lookup(
    cache: &PathDisplayCacheMap,
    path_key_hash: u64,
    path_key: &str,
) -> Option<SharedString> {
    cache
        .get(&path_key_hash)
        .and_then(|bucket| bucket_find(bucket, path_key))
        .cloned()
}

#[cfg(not(windows))]
fn cache_remove(
    cache: &mut PathDisplayCacheMap,
    entry_count: &mut usize,
    hash_counts: &mut HashMap<u64, u16>,
    path_key_hash: u64,
    path_key: &str,
) -> Option<SharedString> {
    let (remove_bucket, position) = {
        let bucket = cache.get(&path_key_hash)?;
        let position = bucket.iter().position(|label| label.as_ref() == path_key)?;
        (bucket.len() == 1, position)
    };

    *entry_count = entry_count.saturating_sub(1);
    hash_counts_remove(hash_counts, path_key_hash, 1);
    if remove_bucket {
        return cache
            .remove(&path_key_hash)
            .and_then(|mut bucket| bucket.pop());
    }

    cache
        .get_mut(&path_key_hash)
        .map(|bucket| bucket.swap_remove(position))
}

#[cfg(not(windows))]
#[inline]
fn cache_insert(
    cache: &mut PathDisplayCacheMap,
    entry_count: &mut usize,
    hash_counts: &mut HashMap<u64, u16>,
    path_key_hash: u64,
    label: SharedString,
) {
    cache.entry(path_key_hash).or_default().push(label);
    *entry_count = entry_count.saturating_add(1);
    hash_counts_insert(hash_counts, path_key_hash);
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

pub(in crate::view) fn repo_path_name(path: &Path) -> SharedString {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(SharedString::new)
        .unwrap_or_else(|| path_display_shared_fast(path))
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
    let path_key_hash = path_display_cache_key(path_key);
    #[cfg(not(windows))]
    let full_two_generation_cache = cache.recent_entries >= PathDisplayCache::RECENT_MAX_ENTRIES
        && cache.previous_entries != 0
        && cache.len() >= PathDisplayCache::MAX_ENTRIES;
    #[cfg(not(windows))]
    if full_two_generation_cache
        && cache.overflow_tail_active
        && !hash_counts_contains(&cache.present_hash_counts, path_key_hash)
    {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_miss);
        return SharedString::new(path_key);
    }

    #[cfg(not(windows))]
    if let Some(s) = cache_lookup(&cache.recent, path_key_hash, path_key) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        cache.overflow_tail_active = false;
        return s;
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
    if cache.previous_entries != 0
        && let Some(s) = cache_remove(
            &mut cache.previous,
            &mut cache.previous_entries,
            &mut cache.present_hash_counts,
            path_key_hash,
            path_key,
        )
    {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        cache.overflow_tail_active = false;
        // Promote to recent so subsequent lookups hit the fast path.
        // Check capacity before inserting to maintain the size invariant.
        if cache.recent_entries >= PathDisplayCache::RECENT_MAX_ENTRIES {
            cache.rotate_generations();
        }
        cache_insert(
            &mut cache.recent,
            &mut cache.recent_entries,
            &mut cache.present_hash_counts,
            path_key_hash,
            s.clone(),
        );
        return s;
    }

    #[cfg(windows)]
    if cache.previous_entries != 0
        && let Some(s) = cache.previous.remove(path)
    {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        cache.previous_entries = cache.previous_entries.saturating_sub(1);
        // Promote to recent so subsequent lookups hit the fast path.
        // Check capacity before inserting to maintain the size invariant.
        if cache.recent_entries >= PathDisplayCache::RECENT_MAX_ENTRIES {
            cache.rotate_generations();
        }
        cache.recent.insert(path.to_path_buf(), s.clone());
        cache.recent_entries = cache.recent_entries.saturating_add(1);
        return s;
    }

    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_miss);
    if cache.recent_entries >= PathDisplayCache::RECENT_MAX_ENTRIES {
        if cache.previous_entries == 0 || cache.len() < PathDisplayCache::MAX_ENTRIES {
            cache.rotate_generations();
        } else {
            // Once both generations are full, keep the hot working set and
            // skip caching one-off overflow misses instead of invalidating the
            // entire previous generation on every long unique tail.
            #[cfg(not(windows))]
            {
                cache.overflow_tail_active = true;
                return SharedString::new(path_key);
            }
            #[cfg(windows)]
            {
                return path_display_shared_fast(path);
            }
        }
    }
    #[cfg(not(windows))]
    {
        cache.overflow_tail_active = false;
    }

    #[cfg(not(windows))]
    let s = SharedString::new(path_key);

    #[cfg(windows)]
    let s = path_display_shared_fast(path);

    #[cfg(not(windows))]
    cache_insert(
        &mut cache.recent,
        &mut cache.recent_entries,
        &mut cache.present_hash_counts,
        path_key_hash,
        s.clone(),
    );

    #[cfg(windows)]
    {
        cache.recent.insert(path.to_path_buf(), s.clone());
        cache.recent_entries = cache.recent_entries.saturating_add(1);
    }

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
        cached_path_display, format_windows_path_for_display, path_display_string, repo_path_name,
    };
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    #[cfg(not(windows))]
    fn cache_contains(cache_map: &super::PathDisplayCacheMap, path: &Path) -> bool {
        let path_key = path.to_str().expect("test paths should be utf-8");
        let path_key_hash = super::path_display_cache_key(path_key);
        cache_map
            .get(&path_key_hash)
            .is_some_and(|bucket| bucket.iter().any(|label| label.as_ref() == path_key))
    }

    #[cfg(not(windows))]
    fn cache_contains_recent(cache: &PathDisplayCache, path: &Path) -> bool {
        cache_contains(&cache.recent, path)
    }

    #[cfg(windows)]
    fn cache_contains_recent(cache: &PathDisplayCache, path: &Path) -> bool {
        cache.recent.contains_key(path)
    }

    #[cfg(not(windows))]
    fn cache_contains_previous(cache: &PathDisplayCache, path: &Path) -> bool {
        cache_contains(&cache.previous, path)
    }

    #[cfg(windows)]
    fn cache_contains_previous(cache: &PathDisplayCache, path: &Path) -> bool {
        cache.previous.contains_key(path)
    }

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
    fn utf8_cache_reuses_shared_string_for_entry_and_return_value() {
        let mut cache = PathDisplayCache::default();
        let path = PathBuf::from("src/lib.rs");
        let display = cached_path_display(&mut cache, &path);

        let cached = cache
            .recent
            .values()
            .next()
            .and_then(|bucket| bucket.first())
            .cloned()
            .expect("cached recent label");
        let cached_arc: Arc<str> = cached.into();
        let display_arc: Arc<str> = display.into();

        assert!(Arc::ptr_eq(&cached_arc, &display_arc));
    }

    #[test]
    fn previous_generation_hit_promotes_into_recent() {
        let mut cache = PathDisplayCache::default();
        let promoted = PathBuf::from("src/promoted.rs");

        for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
            let path = if ix == 0 {
                promoted.clone()
            } else {
                PathBuf::from(format!("src/previous_{ix}.rs"))
            };
            let _ = cached_path_display(&mut cache, &path);
        }
        let _ = cached_path_display(&mut cache, Path::new("src/rotate.rs"));

        assert!(
            !cache_contains_recent(&cache, &promoted),
            "promoted path should start in the previous generation"
        );
        assert!(cache_contains_previous(&cache, &promoted));

        let display = cached_path_display(&mut cache, &promoted);
        assert_eq!(display.as_ref(), promoted.to_str().unwrap_or_default());
        assert!(
            cache_contains_recent(&cache, &promoted),
            "previous-generation hits should be promoted into recent"
        );
        assert!(
            !cache_contains_previous(&cache, &promoted),
            "promoted entries should be removed from the previous generation"
        );
    }

    #[test]
    fn overflow_miss_preserves_full_two_generation_hot_set() {
        bench_reset();

        let mut cache = PathDisplayCache::default();
        let previous_hot = PathBuf::from("src/previous_hot.rs");
        let recent_hot = PathBuf::from("src/recent_hot.rs");

        for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
            let path = if ix == 0 {
                previous_hot.clone()
            } else {
                PathBuf::from(format!("src/previous_{ix}.rs"))
            };
            let _ = cached_path_display(&mut cache, &path);
        }
        for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
            let path = if ix == 0 {
                recent_hot.clone()
            } else {
                PathBuf::from(format!("src/recent_{ix}.rs"))
            };
            let _ = cached_path_display(&mut cache, &path);
        }

        let before = bench_snapshot();
        assert_eq!(cache.len(), PathDisplayCache::MAX_ENTRIES);
        assert!(cache_contains_previous(&cache, &previous_hot));
        assert!(cache_contains_recent(&cache, &recent_hot));

        let overflow = PathBuf::from("src/overflow.rs");
        let _ = cached_path_display(&mut cache, &overflow);

        let after = bench_snapshot();
        assert_eq!(cache.len(), PathDisplayCache::MAX_ENTRIES);
        assert_eq!(
            after.cache_clears, before.cache_clears,
            "overflow misses should not rotate a full two-generation cache"
        );
        assert_eq!(after.cache_misses, before.cache_misses + 1);
        assert!(cache_contains_previous(&cache, &previous_hot));
        assert!(cache_contains_recent(&cache, &recent_hot));
        assert!(!cache_contains_recent(&cache, &overflow));
        assert!(!cache_contains_previous(&cache, &overflow));
    }

    #[test]
    fn overflow_tail_misses_still_allow_cached_hits() {
        bench_reset();

        let mut cache = PathDisplayCache::default();
        let previous_hot = PathBuf::from("src/previous_hot.rs");
        let recent_hot = PathBuf::from("src/recent_hot.rs");

        for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
            let path = if ix == 0 {
                previous_hot.clone()
            } else {
                PathBuf::from(format!("src/previous_{ix}.rs"))
            };
            let _ = cached_path_display(&mut cache, &path);
        }
        for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
            let path = if ix == 0 {
                recent_hot.clone()
            } else {
                PathBuf::from(format!("src/recent_{ix}.rs"))
            };
            let _ = cached_path_display(&mut cache, &path);
        }

        let _ = cached_path_display(&mut cache, Path::new("src/overflow_a.rs"));
        let _ = cached_path_display(&mut cache, Path::new("src/overflow_b.rs"));

        let display = cached_path_display(&mut cache, &recent_hot);
        assert_eq!(display.as_ref(), recent_hot.to_str().unwrap_or_default());
        assert!(cache_contains_recent(&cache, &recent_hot));
        assert!(cache_contains_previous(&cache, &previous_hot));
    }

    #[test]
    fn repo_path_name_uses_last_path_component_when_available() {
        assert_eq!(
            repo_path_name(Path::new("/tmp/repo-name")).as_ref(),
            "repo-name"
        );
    }

    #[test]
    fn repo_path_name_falls_back_to_full_path_when_file_name_is_missing() {
        #[cfg(not(windows))]
        let path = Path::new("/");
        #[cfg(windows)]
        let path = Path::new(r"C:\");

        assert_eq!(repo_path_name(path).as_ref(), path_display_string(path));
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
