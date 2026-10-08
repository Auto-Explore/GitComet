use super::*;
use gitcomet_diagrams::scene::{PreparedScene, TILE_SIZE};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub(super) const MAX_SCENE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_RASTER_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Eq, PartialEq)]
pub(super) struct CacheKey(DiagramKind, Arc<str>, u64);
impl CacheKey {
    pub(super) fn new(kind: DiagramKind, source: Arc<str>) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        Self(kind, source, hasher.finish())
    }
}
impl std::hash::Hash for CacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
        self.2.hash(state);
    }
}
pub(super) struct RasterDiagram {
    pub(super) scene: PreparedScene,
    pub(super) diagnostics: Vec<String>,
    pub(super) id: u64,
    pub(super) bytes: usize,
}
pub(super) type RasterResult = Result<Arc<RasterDiagram>, String>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct TileKey {
    pub(super) scene: u64,
    pub(super) zoom: u8,
    pub(super) dpr: u32,
    pub(super) column: u32,
    pub(super) row: u32,
}
impl TileKey {
    fn scale(self) -> f32 {
        ZOOM_STEPS[self.zoom as usize] * f32::from_bits(self.dpr)
    }
}
struct Flight {
    result: Mutex<Option<RasterResult>>,
    ready: Condvar,
}
pub(super) struct CacheInner {
    pub(super) entries: lru::LruCache<CacheKey, RasterResult>,
    flights: HashMap<CacheKey, Arc<Flight>>,
    tile_locks: HashMap<TileKey, std::sync::Weak<Mutex<()>>>,
    pub(super) bytes: usize,
    pub(super) tiles: lru::LruCache<TileKey, Arc<gpui::RenderImage>>,
    pub(super) dropped: VecDeque<Arc<gpui::RenderImage>>,
}
impl CacheInner {
    pub(super) fn insert(&mut self, key: CacheKey, result: RasterResult) {
        let bytes = result.as_ref().map_or(0, |value| value.bytes);
        if bytes > MAX_SCENE_BYTES {
            return;
        }
        if let Some(old) = self.entries.pop(&key) {
            self.bytes -= old.as_ref().map_or(0, |v| v.bytes);
        }
        while self.bytes + bytes > MAX_SCENE_BYTES || self.entries.len() == self.entries.cap().get()
        {
            let Some((_, old)) = self.entries.pop_lru() else {
                break;
            };
            self.bytes -= old.as_ref().map_or(0, |v| v.bytes);
        }
        self.bytes += bytes;
        self.entries.put(key, result);
    }
}
#[derive(Default)]
struct WorkGate {
    running: Mutex<(usize, usize)>,
    ready: Condvar,
}
struct WorkPermit<'a>(&'a WorkGate);
impl WorkGate {
    fn acquire(&self, visible_tile: bool) -> WorkPermit<'_> {
        let mut running = self.running.lock().unwrap();
        if visible_tile {
            running.1 += 1;
        }
        while running.0 >= 2 || (!visible_tile && running.1 > 0) {
            running = self.ready.wait(running).unwrap();
        }
        if visible_tile {
            running.1 -= 1;
        }
        running.0 += 1;
        WorkPermit(self)
    }
}
impl Drop for WorkPermit<'_> {
    fn drop(&mut self) {
        self.0.running.lock().unwrap().0 -= 1;
        self.0.ready.notify_all();
    }
}
pub(super) struct DiagramCache {
    pub(super) inner: Mutex<CacheInner>,
    work: WorkGate,
    next_id: AtomicU64,
    pub(super) preparations: AtomicU64,
    pub(super) rasterizations: AtomicU64,
}
impl Default for DiagramCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                entries: lru::LruCache::new(NonZeroUsize::new(32).unwrap()),
                flights: HashMap::new(),
                tile_locks: HashMap::new(),
                bytes: 0,
                tiles: lru::LruCache::new(
                    NonZeroUsize::new(MAX_RASTER_BYTES / (TILE_SIZE * TILE_SIZE * 4) as usize)
                        .unwrap(),
                ),
                dropped: VecDeque::new(),
            }),
            work: WorkGate::default(),
            next_id: AtomicU64::new(1),
            preparations: AtomicU64::new(0),
            rasterizations: AtomicU64::new(0),
        }
    }
}
impl DiagramCache {
    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn render(&self, kind: DiagramKind, source: Arc<str>) -> RasterResult {
        self.render_current(kind, source, &AtomicU64::new(0), 0)
    }
    pub(super) fn render_current(
        &self,
        kind: DiagramKind,
        source: Arc<str>,
        generation: &AtomicU64,
        expected: u64,
    ) -> RasterResult {
        let key = CacheKey::new(kind, source.clone());
        let (flight, owner) = {
            let mut inner = self.inner.lock().unwrap();
            if let Some(result) = inner.entries.get(&key) {
                return result.clone();
            }
            if let Some(flight) = inner.flights.get(&key) {
                (flight.clone(), false)
            } else {
                let flight = Arc::new(Flight {
                    result: Mutex::new(None),
                    ready: Condvar::new(),
                });
                inner.flights.insert(key.clone(), flight.clone());
                (flight, true)
            }
        };
        if !owner {
            let mut result = flight.result.lock().unwrap();
            while result.is_none() {
                result = flight.ready.wait(result).unwrap();
            }
            let result = result.as_ref().unwrap().clone();
            if result
                .as_ref()
                .is_err_and(|e| e == "Obsolete diagram request.")
                && generation.load(Ordering::Relaxed) == expected
            {
                return self.render_current(kind, source, generation, expected);
            }
            return result;
        }
        let result = {
            let _permit = self.work.acquire(false);
            if generation.load(Ordering::Relaxed) == expected {
                self.preparations.fetch_add(1, Ordering::Relaxed);
                gitcomet_diagrams::render(kind, &source).and_then(|output| {
                    let page = output
                        .pages
                        .into_iter()
                        .next()
                        .ok_or("No Mermaid SVG produced.")?;
                    let bytes = page.svg.len();
                    Ok(Arc::new(RasterDiagram {
                        scene: PreparedScene::new(page.svg)?,
                        diagnostics: output.diagnostics,
                        id: self.next_id.fetch_add(1, Ordering::Relaxed),
                        bytes,
                    }))
                })
            } else {
                Err("Obsolete diagram request.".into())
            }
        };
        // Publish before removing the flight, including scenes that bypass retention.
        *flight.result.lock().unwrap() = Some(result.clone());
        {
            let mut inner = self.inner.lock().unwrap();
            if generation.load(Ordering::Relaxed) == expected {
                inner.insert(key.clone(), result.clone());
            }
            inner.flights.remove(&key);
        }
        flight.ready.notify_all();
        result
    }
    pub(super) fn peek_tile(&self, key: TileKey) -> Option<Arc<gpui::RenderImage>> {
        self.inner.lock().unwrap().tiles.get(&key).cloned()
    }
    pub(super) fn tile(
        &self,
        diagram: &RasterDiagram,
        key: TileKey,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<Option<Arc<gpui::RenderImage>>, String> {
        if generation.load(Ordering::Relaxed) != expected {
            return Ok(None);
        }
        if let Some(tile) = self.peek_tile(key) {
            return Ok(Some(tile));
        }
        let lock = {
            let mut inner = self.inner.lock().unwrap();
            inner.tile_locks.retain(|_, lock| lock.strong_count() > 0);
            let lock = inner
                .tile_locks
                .get(&key)
                .and_then(|lock| lock.upgrade())
                .unwrap_or_else(|| Arc::new(Mutex::new(())));
            inner.tile_locks.insert(key, Arc::downgrade(&lock));
            lock
        };
        let _tile_lock = lock.lock().unwrap();
        if let Some(tile) = self.peek_tile(key) {
            return Ok(Some(tile));
        }
        let _permit = self.work.acquire(true);
        if generation.load(Ordering::Relaxed) != expected {
            return Ok(None);
        }
        if let Some(tile) = self.peek_tile(key) {
            return Ok(Some(tile));
        }
        self.rasterizations.fetch_add(1, Ordering::Relaxed);
        let mut pixels = diagram
            .scene
            .raster_tile(key.scale(), key.column, key.row)?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_raw(TILE_SIZE, TILE_SIZE, pixels)
                .ok_or("Invalid diagram pixels.")?,
        )]));
        let mut inner = self.inner.lock().unwrap();
        if let Some((_, old)) = inner.tiles.push(key, image.clone()) {
            inner.dropped.push_back(old);
        }
        Ok(Some(image))
    }
}
pub(super) struct AppDiagramCache(pub(super) Arc<DiagramCache>);
impl gpui::Global for AppDiagramCache {}
pub(super) fn cache(cx: &mut App) -> Arc<DiagramCache> {
    if !cx.has_global::<AppDiagramCache>() {
        cx.set_global(AppDiagramCache(Arc::new(DiagramCache::default())));
    }
    let cache = cx.global::<AppDiagramCache>().0.clone();
    let dropped = std::mem::take(&mut cache.inner.lock().unwrap().dropped);
    for image in dropped {
        cx.drop_image(image, None);
    }
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_obsolete_preparation_does_not_run_or_enter_cache() {
        let cache = Arc::new(DiagramCache::default());
        let first = cache.work.acquire(false);
        let second = cache.work.acquire(false);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_cache = cache.clone();
        let worker_generation = generation.clone();
        let (started, ready) = std::sync::mpsc::sync_channel(0);
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            worker_cache.render_current(
                DiagramKind::Mermaid,
                "flowchart LR;A-->B".into(),
                &worker_generation,
                0,
            )
        });
        ready.recv().unwrap();
        assert_eq!(cache.work.running.lock().unwrap().0, 2);
        generation.store(1, Ordering::Relaxed);
        drop(first);
        drop(second);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(cache.preparations.load(Ordering::Relaxed), 0);
        assert!(cache.inner.lock().unwrap().entries.is_empty());
    }
    #[test]
    fn concurrent_identical_tiles_rasterize_once() {
        let cache = Arc::new(DiagramCache::default());
        let scene = cache
            .render(DiagramKind::Mermaid, "flowchart LR;A-->B".into())
            .unwrap();
        let key = TileKey {
            scene: scene.id,
            zoom: 2,
            dpr: 1.0f32.to_bits(),
            column: 0,
            row: 0,
        };
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let cache = cache.clone();
                let scene = scene.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    cache
                        .tile(&scene, key, &AtomicU64::new(0), 0)
                        .unwrap()
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let tiles = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(tiles.iter().all(|tile| Arc::ptr_eq(tile, &tiles[0])));
        assert_eq!(cache.rasterizations.load(Ordering::Relaxed), 1);
    }
}
