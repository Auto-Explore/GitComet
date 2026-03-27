use super::*;
use std::cell::RefCell;
use std::hash::BuildHasherDefault;
use std::num::NonZeroUsize;

pub(in crate::view) const MAX_LINES_FOR_SYNTAX_HIGHLIGHTING: usize = 4_000;
const MAX_CACHED_LINE_NUMBER: usize = 16_384;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct LruCacheMetrics {
    pub(in crate::view) hits: u64,
    pub(in crate::view) misses: u64,
    pub(in crate::view) evictions: u64,
    pub(in crate::view) clears: u64,
}

#[derive(Debug)]
pub(in crate::view) struct InstrumentedLruCache<
    K: std::hash::Hash + Eq,
    V,
    S: std::hash::BuildHasher = lru::DefaultHasher,
> {
    cache: lru::LruCache<K, V, S>,
    metrics: LruCacheMetrics,
}

impl<K: std::hash::Hash + Eq, V> InstrumentedLruCache<K, V> {
    pub(in crate::view) fn new(cap: usize) -> Self {
        Self {
            cache: lru::LruCache::new(non_zero_lru_capacity(cap)),
            metrics: LruCacheMetrics::default(),
        }
    }
}

impl<K: std::hash::Hash + Eq, V, S: std::hash::BuildHasher> InstrumentedLruCache<K, V, S> {
    pub(in crate::view) fn with_hasher(cap: usize, hash_builder: S) -> Self {
        Self {
            cache: lru::LruCache::with_hasher(non_zero_lru_capacity(cap), hash_builder),
            metrics: LruCacheMetrics::default(),
        }
    }

    pub(in crate::view) fn get(&mut self, key: &K) -> Option<&V> {
        let value = self.cache.get(key);
        if value.is_some() {
            self.metrics.hits = self.metrics.hits.saturating_add(1);
        } else {
            self.metrics.misses = self.metrics.misses.saturating_add(1);
        }
        value
    }

    #[cfg(test)]
    pub(in crate::view) fn peek(&self, key: &K) -> Option<&V> {
        self.cache.peek(key)
    }

    pub(in crate::view) fn put(&mut self, key: K, value: V) -> Option<V> {
        let will_evict =
            self.cache.peek(&key).is_none() && self.cache.len() >= self.cache.cap().get();
        let previous = self.cache.put(key, value);
        if will_evict {
            self.metrics.evictions = self.metrics.evictions.saturating_add(1);
        }
        previous
    }

    #[cfg(test)]
    pub(in crate::view) fn len(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub(in crate::view) fn clear(&mut self) {
        self.cache.clear();
        self.metrics.clears = self.metrics.clears.saturating_add(1);
    }

    #[cfg(test)]
    pub(in crate::view) fn metrics(&self) -> LruCacheMetrics {
        self.metrics
    }
}

fn non_zero_lru_capacity(cap: usize) -> NonZeroUsize {
    NonZeroUsize::new(cap).expect("LRU cache capacity must be > 0")
}

pub(in crate::view) type LruCache<K, V> = InstrumentedLruCache<K, V>;
/// LRU cache backed by FxHasher for fast hashing of u64 keys (text layout caches).
pub(in crate::view) type FxLruCache<K, V> =
    InstrumentedLruCache<K, V, BuildHasherDefault<rustc_hash::FxHasher>>;

pub(in crate::view) fn new_lru_cache<K: std::hash::Hash + Eq, V>(cap: usize) -> LruCache<K, V> {
    InstrumentedLruCache::new(cap)
}

/// Create a new FxHasher-backed LRU cache with the given capacity.
pub(in crate::view) fn new_fx_lru_cache<K: std::hash::Hash + Eq, V>(
    cap: usize,
) -> FxLruCache<K, V> {
    InstrumentedLruCache::with_hasher(cap, BuildHasherDefault::default())
}

#[derive(Debug)]
pub(in crate::view) struct CommitFilePathLabelsCache<K: Eq + Clone> {
    cached: Option<(K, Arc<[SharedString]>)>,
}

impl<K: Eq + Clone> Default for CommitFilePathLabelsCache<K> {
    fn default() -> Self {
        Self { cached: None }
    }
}

impl<K: Eq + Clone> CommitFilePathLabelsCache<K> {
    pub(in crate::view) fn labels_for(
        &mut self,
        key: &K,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[SharedString]> {
        if let Some((cached_key, labels)) = &self.cached
            && cached_key == key
        {
            return labels.clone();
        }

        let labels: Arc<[SharedString]> = files
            .iter()
            .map(|file| super::path_display::path_display_shared_fast(&file.path))
            .collect::<Vec<_>>()
            .into();
        self.cached = Some((key.clone(), labels.clone()));
        labels
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn clear(&mut self) {
        self.cached = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum CommitFileKindTone {
    Success,
    Warning,
    Danger,
    Accent,
}

impl CommitFileKindTone {
    #[inline]
    pub(in crate::view) fn color(self, theme: &AppTheme) -> gpui::Rgba {
        match self {
            Self::Success => theme.colors.success,
            Self::Warning => theme.colors.warning,
            Self::Danger => theme.colors.danger,
            Self::Accent => theme.colors.accent,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct CommitFileKindVisuals {
    pub(in crate::view) icon: &'static str,
    pub(in crate::view) kind_key: u8,
    tone: CommitFileKindTone,
}

impl CommitFileKindVisuals {
    #[inline]
    pub(in crate::view) fn color(self, theme: &AppTheme) -> gpui::Rgba {
        self.tone.color(theme)
    }
}

const COMMIT_FILE_KIND_VISUALS: [CommitFileKindVisuals; 6] = [
    CommitFileKindVisuals {
        icon: "icons/question.svg",
        kind_key: 4,
        tone: CommitFileKindTone::Warning,
    },
    CommitFileKindVisuals {
        icon: "icons/pencil.svg",
        kind_key: 1,
        tone: CommitFileKindTone::Warning,
    },
    CommitFileKindVisuals {
        icon: "icons/plus.svg",
        kind_key: 0,
        tone: CommitFileKindTone::Success,
    },
    CommitFileKindVisuals {
        icon: "icons/minus.svg",
        kind_key: 2,
        tone: CommitFileKindTone::Danger,
    },
    CommitFileKindVisuals {
        icon: "icons/swap.svg",
        kind_key: 3,
        tone: CommitFileKindTone::Accent,
    },
    CommitFileKindVisuals {
        icon: "icons/warning.svg",
        kind_key: 5,
        tone: CommitFileKindTone::Danger,
    },
];

#[inline]
pub(in crate::view) const fn commit_file_kind_visuals(
    kind: FileStatusKind,
) -> CommitFileKindVisuals {
    COMMIT_FILE_KIND_VISUALS[kind as usize]
}

thread_local! {
    static LINE_NUMBER_STRINGS: RefCell<Vec<SharedString>> =
        RefCell::new(vec![SharedString::default()]);
}

fn line_number_string(n: Option<u32>) -> SharedString {
    let Some(n) = n else {
        return SharedString::default();
    };
    let ix = n as usize;
    if ix > MAX_CACHED_LINE_NUMBER {
        return n.to_string().into();
    }
    LINE_NUMBER_STRINGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() <= ix {
            let start = cache.len();
            cache.reserve(ix + 1 - start);
            for v in start..=ix {
                cache.push(v.to_string().into());
            }
        }
        cache[ix].clone()
    })
}

mod canvas;
#[cfg(test)]
mod canvas_tests;
mod conflict_canvas;
mod conflict_resolver;
mod diff;
mod diff_canvas;
mod diff_text;
mod history;
mod history_canvas;
mod history_graph_paint;
mod sidebar;
mod status;

#[cfg(feature = "benchmarks")]
pub(crate) mod benchmarks;

pub(in crate::view) use diff_text::{
    BackgroundPreparedDiffSyntaxDocument, DiffSyntaxBudget, DiffSyntaxEdit, DiffSyntaxLanguage,
    DiffSyntaxMode, PrepareDiffSyntaxDocumentResult, PreparedDiffSyntaxDocument,
    PreparedDiffSyntaxLine, PreparedDiffSyntaxReparseSeed,
    diff_syntax_language_for_code_fence_info, diff_syntax_language_for_path,
    drain_completed_prepared_diff_syntax_chunk_builds,
    drain_completed_prepared_diff_syntax_chunk_builds_for_document,
    has_pending_prepared_diff_syntax_chunk_builds,
    has_pending_prepared_diff_syntax_chunk_builds_for_document,
    inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_in_background_text_with_reuse,
    prepare_diff_syntax_document_with_budget_reuse_text,
    prepared_diff_syntax_line_for_inline_diff_row, prepared_diff_syntax_line_for_one_based_line,
    prepared_diff_syntax_reparse_seed, request_syntax_highlights_for_prepared_document_byte_range,
    resolved_output_line_text, syntax_highlights_for_line,
};

#[cfg(test)]
pub(in crate::view) use self::diff_canvas::{
    DiffPaintRecord, clear_diff_paint_log_for_tests, diff_paint_log_for_tests,
};

#[cfg(test)]
pub(in crate::view) use diff_text::{
    PreparedDiffSyntaxParseMode, prepare_diff_syntax_document_in_background_text,
    prepared_diff_syntax_parse_mode, prepared_diff_syntax_source_version,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_line_number_string_cache() {
        LINE_NUMBER_STRINGS.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.clear();
            cache.push(SharedString::default());
        });
    }

    fn line_number_string_cache_len() -> usize {
        LINE_NUMBER_STRINGS.with(|cache| cache.borrow().len())
    }

    #[test]
    fn line_number_cache_does_not_grow_for_uncached_large_numbers() {
        reset_line_number_string_cache();
        assert_eq!(line_number_string_cache_len(), 1);

        assert_eq!(line_number_string(Some(8)), SharedString::from("8"));
        assert_eq!(line_number_string_cache_len(), 9);

        let uncached_line = (MAX_CACHED_LINE_NUMBER as u32).saturating_add(1);
        assert_eq!(
            line_number_string(Some(uncached_line)),
            uncached_line.to_string()
        );
        assert_eq!(line_number_string_cache_len(), 9);
    }

    #[test]
    fn line_number_cache_still_caches_small_numbers() {
        reset_line_number_string_cache();
        assert_eq!(line_number_string_cache_len(), 1);

        assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
        assert_eq!(line_number_string(Some(3)), SharedString::from("3"));
        assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
        assert_eq!(line_number_string_cache_len(), 4);
    }

    #[test]
    fn lru_cache_evicts_least_recently_used() {
        let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(8);
        for key in 0..8u64 {
            cache.put(key, key);
        }
        assert_eq!(cache.len(), 8);

        // Insert a 9th entry — should evict key 0 (LRU)
        cache.put(999, 999);
        assert_eq!(cache.len(), 8);
        assert!(cache.peek(&999).is_some());
        assert!(cache.peek(&0).is_none(), "LRU entry should be evicted");
        assert!(cache.peek(&7).is_some(), "MRU entry should remain");
    }

    #[test]
    fn lru_cache_promotes_on_get() {
        let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(4);
        for key in 0..4u64 {
            cache.put(key, key);
        }

        // Access key 0 to promote it to MRU
        assert_eq!(cache.get(&0), Some(&0));

        // Insert 4 more entries — key 0 should survive (was promoted)
        cache.put(10, 10);
        cache.put(11, 11);
        cache.put(12, 12);

        assert!(cache.peek(&0).is_some(), "promoted entry should survive");
        assert!(
            cache.peek(&1).is_none(),
            "unpromoted old entry should be evicted"
        );
    }

    #[test]
    fn lru_cache_metrics_track_hits_misses_evictions_and_clears() {
        let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(2);

        assert_eq!(cache.get(&1), None);
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 0,
                misses: 1,
                evictions: 0,
                clears: 0,
            }
        );

        cache.put(1, 10);
        cache.put(2, 20);
        assert_eq!(cache.get(&1), Some(&10));
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 1,
                misses: 1,
                evictions: 0,
                clears: 0,
            }
        );

        cache.put(3, 30);
        assert_eq!(
            cache.peek(&2),
            None,
            "least-recently used entry should evict"
        );
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 1,
                misses: 1,
                evictions: 1,
                clears: 0,
            }
        );

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 1,
                misses: 1,
                evictions: 1,
                clears: 1,
            }
        );
    }
}
