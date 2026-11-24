use gpui_shared_string::SharedString;
use rustc_hash::FxHashMap;
#[cfg(not(windows))]
use rustc_hash::FxHasher;
#[cfg(not(windows))]
use smallvec::SmallVec;
#[cfg(any(test, feature = "benchmarks"))]
use std::cell::Cell;
#[cfg(not(windows))]
use std::hash::{Hash, Hasher};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PathDisplayBenchSnapshot {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_clears: u64,
}

#[cfg(not(windows))]
type PathDisplayCacheBucket = SmallVec<[SharedString; 1]>;
#[cfg(not(windows))]
type PathDisplayCacheMap = FxHashMap<u64, PathDisplayCacheBucket>;
#[cfg(windows)]
type PathDisplayCacheMap = FxHashMap<PathBuf, SharedString>;

/// A bounded two-generation cache. Keeping the previous generation avoids the
/// all-or-nothing cliff when a large repo first crosses the cache cap.
#[derive(Default)]
pub struct PathDisplayCache {
    recent: PathDisplayCacheMap,
    previous: PathDisplayCacheMap,
    recent_entries: usize,
    previous_entries: usize,
    #[cfg(not(windows))]
    present_hash_counts: FxHashMap<u64, u16>,
    #[cfg(not(windows))]
    overflow_tail_active: bool,
}

impl PathDisplayCache {
    const MAX_ENTRIES: usize = 8_192;
    const RECENT_MAX_ENTRIES: usize = Self::MAX_ENTRIES / 2;
    const INITIAL_CAPACITY: usize = 256;

    pub fn len(&self) -> usize {
        self.recent_entries + self.previous_entries
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[allow(dead_code)]
    pub fn clear(&mut self) {
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

    fn reserve_recent(&mut self, capacity: usize) {
        self.recent.reserve(capacity - self.recent.len());
        #[cfg(not(windows))]
        self.present_hash_counts
            .reserve((capacity * 2).saturating_sub(self.present_hash_counts.len()));
    }

    fn rotate_generations(&mut self) {
        #[cfg(any(test, feature = "benchmarks"))]
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
        self.reserve_recent(Self::RECENT_MAX_ENTRIES);
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
fn hash_counts_contains(hash_counts: &FxHashMap<u64, u16>, path_key_hash: u64) -> bool {
    hash_counts.contains_key(&path_key_hash)
}

#[cfg(not(windows))]
#[inline]
fn hash_counts_insert(hash_counts: &mut FxHashMap<u64, u16>, path_key_hash: u64) {
    hash_counts
        .entry(path_key_hash)
        .and_modify(|count| *count = count.saturating_add(1))
        .or_insert(1);
}

#[cfg(not(windows))]
#[inline]
fn hash_counts_remove(
    hash_counts: &mut FxHashMap<u64, u16>,
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
fn remove_generation_hashes(cache: &PathDisplayCacheMap, hash_counts: &mut FxHashMap<u64, u16>) {
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
    hash_counts: &mut FxHashMap<u64, u16>,
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
    hash_counts: &mut FxHashMap<u64, u16>,
    path_key_hash: u64,
    label: SharedString,
) {
    cache.entry(path_key_hash).or_default().push(label);
    *entry_count = entry_count.saturating_add(1);
    hash_counts_insert(hash_counts, path_key_hash);
}

#[cfg(windows)]
pub fn path_display_string(path: &Path) -> String {
    format_windows_path_for_display(path.display().to_string())
}

#[cfg(not(windows))]
pub fn path_display_string(path: &Path) -> String {
    path.to_str()
        .map(str::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn path_display_shared(path: &Path) -> SharedString {
    path_display_string(path).into()
}

pub fn repo_path_name(path: &Path) -> SharedString {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(SharedString::new)
        .unwrap_or_else(|| path_display_shared_fast(path))
}

/// Fast path that skips the intermediate `String` allocation on non-Windows
/// by constructing `SharedString` directly from `&str` → `Arc<str>`.
#[cfg(not(windows))]
pub fn path_display_shared_fast(path: &Path) -> SharedString {
    match path.to_str() {
        Some(s) => SharedString::new(s),
        None => path_display_shared(path),
    }
}

#[cfg(windows)]
pub fn path_display_shared_fast(path: &Path) -> SharedString {
    path_display_shared(path)
}

pub fn cached_path_display(cache: &mut PathDisplayCache, path: &Path) -> SharedString {
    #[cfg(not(windows))]
    let Some(path_key) = path.to_str() else {
        #[cfg(any(test, feature = "benchmarks"))]
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
        #[cfg(any(test, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_miss);
        return SharedString::new(path_key);
    }

    #[cfg(not(windows))]
    if let Some(s) = cache_lookup(&cache.recent, path_key_hash, path_key) {
        #[cfg(any(test, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        cache.overflow_tail_active = false;
        return s;
    }

    #[cfg(windows)]
    if let Some(s) = cache.recent.get(path) {
        #[cfg(any(test, feature = "benchmarks"))]
        PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::record_hit);
        return s.clone();
    }

    match cache.recent_entries {
        0 => cache.reserve_recent(PathDisplayCache::INITIAL_CAPACITY),
        PathDisplayCache::INITIAL_CAPACITY => {
            cache.reserve_recent(PathDisplayCache::RECENT_MAX_ENTRIES)
        }
        _ => {}
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
        #[cfg(any(test, feature = "benchmarks"))]
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
        #[cfg(any(test, feature = "benchmarks"))]
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

    #[cfg(any(test, feature = "benchmarks"))]
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
pub fn bench_snapshot() -> PathDisplayBenchSnapshot {
    PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::snapshot)
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn bench_reset() {
    PATH_DISPLAY_BENCH_COUNTERS.with(PathDisplayBenchCounters::reset);
}

#[cfg(any(test, feature = "benchmarks"))]
struct PathDisplayBenchCounters {
    cache_hits: Cell<u64>,
    cache_misses: Cell<u64>,
    cache_clears: Cell<u64>,
}

#[cfg(any(test, feature = "benchmarks"))]
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

    fn snapshot(&self) -> PathDisplayBenchSnapshot {
        PathDisplayBenchSnapshot {
            cache_hits: self.cache_hits.get(),
            cache_misses: self.cache_misses.get(),
            cache_clears: self.cache_clears.get(),
        }
    }

    fn reset(&self) {
        self.cache_hits.set(0);
        self.cache_misses.set(0);
        self.cache_clears.set(0);
    }
}

#[cfg(any(test, feature = "benchmarks"))]
thread_local! {
    static PATH_DISPLAY_BENCH_COUNTERS: PathDisplayBenchCounters = PathDisplayBenchCounters::new();
}

#[cfg(any(windows, test))]
fn format_windows_path_for_display(mut path: String) -> String {
    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        path = format!(r"\\{stripped}");
    } else if let Some(stripped) = path.strip_prefix(r"\\?\") {
        path = stripped.to_string();
    }
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests;
