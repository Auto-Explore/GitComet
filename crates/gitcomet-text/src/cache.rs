//! LRU caches that count hits, misses, evictions and clears, and a recency
//! queue for caches that keep their own entry maps.
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use std::num::NonZeroUsize;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LruCacheMetrics {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub clears: u64,
}

#[derive(Debug)]
pub struct InstrumentedLruCache<
    K: std::hash::Hash + Eq,
    V,
    S: std::hash::BuildHasher = lru::DefaultHasher,
> {
    cache: lru::LruCache<K, V, S>,
    metrics: LruCacheMetrics,
}

impl<K: std::hash::Hash + Eq, V> InstrumentedLruCache<K, V> {
    pub fn new(cap: usize) -> Self {
        Self {
            cache: lru::LruCache::new(non_zero_lru_capacity(cap)),
            metrics: LruCacheMetrics::default(),
        }
    }
}

impl<K: std::hash::Hash + Eq, V, S: std::hash::BuildHasher> InstrumentedLruCache<K, V, S> {
    pub fn with_hasher(cap: usize, hash_builder: S) -> Self {
        Self {
            cache: lru::LruCache::with_hasher(non_zero_lru_capacity(cap), hash_builder),
            metrics: LruCacheMetrics::default(),
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let value = self.cache.get(key);
        if value.is_some() {
            self.metrics.hits = self.metrics.hits.saturating_add(1);
        } else {
            self.metrics.misses = self.metrics.misses.saturating_add(1);
        }
        value
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        self.cache.peek(key)
    }

    pub fn put(&mut self, key: K, value: V) -> Option<V> {
        let will_evict =
            self.cache.peek(&key).is_none() && self.cache.len() >= self.cache.cap().get();
        let previous = self.cache.put(key, value);
        if will_evict {
            self.metrics.evictions = self.metrics.evictions.saturating_add(1);
        }
        previous
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.metrics.clears = self.metrics.clears.saturating_add(1);
    }

    pub fn metrics(&self) -> LruCacheMetrics {
        self.metrics
    }
}

fn non_zero_lru_capacity(cap: usize) -> NonZeroUsize {
    NonZeroUsize::new(cap).expect("LRU cache capacity must be > 0")
}

pub type LruCache<K, V> = InstrumentedLruCache<K, V>;
/// LRU cache backed by FxHasher for fast hashing of u64 keys (text layout caches).
pub type FxLruCache<K, V> = InstrumentedLruCache<K, V, BuildHasherDefault<FxHasher>>;

pub fn new_lru_cache<K: std::hash::Hash + Eq, V>(cap: usize) -> LruCache<K, V> {
    InstrumentedLruCache::new(cap)
}

/// Create a new FxHasher-backed LRU cache with the given capacity.
pub fn new_fx_lru_cache<K: std::hash::Hash + Eq, V>(cap: usize) -> FxLruCache<K, V> {
    InstrumentedLruCache::with_hasher(cap, BuildHasherDefault::default())
}

/// A VecDeque-backed recency queue for caches that keep their own entry maps
/// and only need the touch/evict-order half of an LRU.
#[derive(Clone, Debug)]
pub struct LruTouchQueue<K> {
    order: std::collections::VecDeque<K>,
}

impl<K> Default for LruTouchQueue<K> {
    fn default() -> Self {
        Self {
            order: std::collections::VecDeque::new(),
        }
    }
}

impl<K: Eq> LruTouchQueue<K> {
    pub fn touch(&mut self, key: K) {
        if self.order.back() == Some(&key) {
            return;
        }
        if let Some(pos) = self.order.iter().position(|existing| *existing == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    pub fn remove(&mut self, key: &K) -> bool {
        if let Some(pos) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn back(&self) -> Option<&K> {
        self.order.back()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn clear(&mut self) {
        self.order.clear();
    }

    pub fn pop_oldest(&mut self) -> Option<K> {
        self.order.pop_front()
    }
}
