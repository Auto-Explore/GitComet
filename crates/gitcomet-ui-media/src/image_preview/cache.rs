use super::*;
use gpui::App;
use std::sync::Arc;
use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{Condvar, Mutex},
};

const ORIGINAL_BYTES: usize = 256 * 1024 * 1024;
pub(super) const TILE_COUNT: usize = 64; // At most 64 MiB of 512² BGRA tiles.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct TileKey {
    pub source: u64,
    pub frame: usize,
    pub output: (u32, u32),
    pub sampling: filter::Sampling,
    pub column: u32,
    pub row: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct OriginalKey {
    source: u64,
    frame: usize,
    // Filtered SVGs are rasterized as a whole at this size before tiling.
    output: Option<(u32, u32)>,
}

/// Decoded images for the whole app, kept only while a viewport shows them.
pub(super) struct Originals {
    inner: Mutex<OriginalsInner>,
    decode: Mutex<()>,
    budget: usize,
}

struct OriginalsInner {
    lru: lru::LruCache<OriginalKey, Arc<source::Decoded>>,
    bytes: usize,
    // Viewport sides showing each source.
    shown: HashMap<u64, usize>,
}

impl Default for Originals {
    fn default() -> Self {
        Self {
            inner: Mutex::new(OriginalsInner {
                lru: lru::LruCache::new(NonZeroUsize::new(32).unwrap()),
                bytes: 0,
                shown: HashMap::new(),
            }),
            decode: Mutex::new(()),
            budget: ORIGINAL_BYTES,
        }
    }
}

#[derive(Default)]
pub(super) struct SharedOriginals(pub Arc<Originals>);
impl gpui::Global for SharedOriginals {}

impl Originals {
    fn get(&self, key: &OriginalKey) -> Option<Arc<source::Decoded>> {
        self.inner.lock().unwrap().lru.get(key).cloned()
    }

    fn insert(&self, key: OriginalKey, image: Arc<source::Decoded>) {
        let mut inner = self.inner.lock().unwrap();
        // A finished decode outlives camera changes, not its source leaving every viewport.
        if !inner.shown.contains_key(&key.source) {
            return;
        }
        let bytes = image.bytes();
        // An original over the budget is kept alone, so panning it never decodes again.
        while !inner.lru.is_empty()
            && (inner.bytes + bytes > self.budget || inner.lru.len() == inner.lru.cap().get())
        {
            if let Some((_, old)) = inner.lru.pop_lru() {
                inner.bytes -= old.bytes();
            }
        }
        inner.bytes += bytes;
        if let Some(old) = inner.lru.put(key, image) {
            inner.bytes -= old.bytes();
        }
    }
}

struct Inner {
    tiles: lru::LruCache<TileKey, Arc<gpui::RenderImage>>,
    dropped: Vec<Arc<gpui::RenderImage>>,
    closed: bool,
}

/// Tiles for one pane (and the documents that share it), over app-wide originals.
pub(super) struct Cache {
    originals: Arc<Originals>,
    inner: Mutex<Inner>,
    running: Mutex<usize>,
    ready: Condvar,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}
struct Permit<'a>(&'a Cache);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.running.lock().unwrap() -= 1;
        self.0.ready.notify_one();
    }
}

impl Cache {
    pub fn new(originals: Arc<Originals>) -> Self {
        Self {
            originals,
            inner: Mutex::new(Inner {
                tiles: lru::LruCache::new(NonZeroUsize::new(TILE_COUNT).unwrap()),
                dropped: Vec::new(),
                closed: false,
            }),
            running: Mutex::new(0),
            ready: Condvar::new(),
        }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut running = self.running.lock().unwrap();
        while *running >= 2 {
            running = self.ready.wait(running).unwrap();
        }
        *running += 1;
        Permit(self)
    }

    pub fn peek(&self, key: &TileKey) -> Option<Arc<gpui::RenderImage>> {
        self.inner.lock().unwrap().tiles.get(key).cloned()
    }

    /// Count the sources viewport sides show; release the pixels of the rest.
    pub fn show(&self, removed: &[u64], added: &[u64]) {
        let mut originals = self.originals.inner.lock().unwrap();
        let OriginalsInner { lru, bytes, shown } = &mut *originals;
        for id in added {
            *shown.entry(*id).or_default() += 1;
        }
        for id in removed {
            if let Some(count) = shown.get_mut(id) {
                *count -= 1;
                if *count == 0 {
                    shown.remove(id);
                }
            }
        }
        let hidden: Vec<_> = lru
            .iter()
            .map(|(key, _)| *key)
            .filter(|key| !shown.contains_key(&key.source))
            .collect();
        for key in hidden {
            if let Some(image) = lru.pop(&key) {
                *bytes -= image.bytes();
            }
        }
        let mut inner = self.inner.lock().unwrap();
        let hidden: Vec<_> = inner
            .tiles
            .iter()
            .map(|(key, _)| *key)
            .filter(|key| !shown.contains_key(&key.source))
            .collect();
        for key in hidden {
            if let Some(image) = inner.tiles.pop(&key) {
                inner.dropped.push(image);
            }
        }
    }

    fn original(
        &self,
        source: &Source,
        tile: &TileKey,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<Option<Arc<source::Decoded>>, String> {
        let key = OriginalKey {
            source: source.id,
            frame: tile.frame,
            output: source.filtered_svg.then_some(tile.output),
        };
        // Cache hits never wait for another image's decode.
        if let Some(image) = self.originals.get(&key) {
            return Ok(Some(image));
        }
        let _decode = self.originals.decode.lock().unwrap();
        if generation.load(Ordering::Relaxed) != expected {
            return Ok(None);
        }
        if let Some(image) = self.originals.get(&key) {
            return Ok(Some(image));
        }
        let Some(decoded) = source.decode(tile.frame, generation, expected)? else {
            return Ok(None);
        };
        // A completed original is keyed by source/frame, not by the camera
        // request that started it. Keep it for the next pan or zoom request.
        let image = Arc::new(decoded.for_output(tile.output)?);
        self.originals.insert(key, image.clone());
        Ok(Some(image))
    }

    pub fn render(
        &self,
        source: &Source,
        keys: &[TileKey],
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<(), String> {
        let _permit = self.acquire();
        if generation.load(Ordering::Relaxed) != expected {
            return Ok(());
        }
        let Some(first) = keys.first() else {
            return Ok(());
        };
        if keys.iter().all(|key| self.peek(key).is_some()) {
            return Ok(());
        }
        let Some(original) = self.original(source, first, generation, expected)? else {
            return Ok(());
        };
        for key in keys {
            if generation.load(Ordering::Relaxed) != expected {
                break;
            }
            if self.peek(key).is_some() {
                continue;
            }
            let pixels = resample::tile(&original, key.output, key.column, key.row, key.sampling)?;
            let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(pixels)]));
            let mut inner = self.inner.lock().unwrap();
            if inner.closed || generation.load(Ordering::Relaxed) != expected {
                break;
            }
            if inner.tiles.contains(key) {
                continue;
            }
            if let Some((_, old)) = inner.tiles.push(*key, image) {
                inner.dropped.push(old);
            }
        }
        Ok(())
    }

    pub fn drain(&self, cx: &mut App) {
        Self::release_images(std::mem::take(&mut self.inner.lock().unwrap().dropped), cx);
    }

    fn release_images(images: Vec<Arc<gpui::RenderImage>>, cx: &mut App) {
        if images.is_empty() {
            return;
        }
        // Drawing temporarily removes the active window from App.windows.
        // Retire textures after the frame so every atlas is available and no
        // image gathered during prepaint can be uploaded after its retirement.
        cx.defer(move |cx| {
            for image in images {
                cx.drop_image(image, None);
            }
        });
    }

    pub fn close(&self, cx: &mut App) {
        let images = {
            let mut inner = self.inner.lock().unwrap();
            inner.closed = true;
            while let Some((_, image)) = inner.tiles.pop_lru() {
                inner.dropped.push(image);
            }
            std::mem::take(&mut inner.dropped)
        };
        Self::release_images(images, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(source: &Source, output: (u32, u32)) -> TileKey {
        TileKey {
            source: source.id,
            frame: 0,
            output,
            sampling: filter::Sampling::Nearest,
            column: 0,
            row: 0,
        }
    }

    fn originals(cache: &Cache) -> (usize, usize) {
        let inner = cache.originals.inner.lock().unwrap();
        (inner.lru.len(), inner.bytes)
    }

    #[test]
    fn image_preview_lifecycle_finished_decode_survives_camera_cancellation() {
        let mut source = super::super::tests::source(64, 64);
        let (done, decoded) = std::sync::mpsc::channel();
        let (resume, paused) = std::sync::mpsc::channel();
        Arc::get_mut(&mut source).unwrap().decode_gate = Some(Arc::new(source::DecodeGate {
            done,
            resume: Mutex::new(paused),
        }));
        let cache = Arc::new(Cache::default());
        cache.show(&[], &[source.id]);
        let generation = Arc::new(AtomicU64::new(0));
        let key = key(&source, (64, 64));
        let worker = {
            let source = source.clone();
            let cache = cache.clone();
            let generation = generation.clone();
            std::thread::spawn(move || cache.render(&source, &[key], &generation, 0).unwrap())
        };
        decoded
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        generation.store(1, Ordering::Relaxed);
        resume.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(
            originals(&cache),
            (1, 64 * 64 * 4),
            "a completed original remains useful after panning or zooming"
        );
        assert!(
            cache.inner.lock().unwrap().tiles.is_empty(),
            "obsolete tiles must still be canceled"
        );
        // The decoder gate cannot run twice. This request must reuse the
        // original from the canceled request and produce its current tile.
        cache.render(&source, &[key], &generation, 1).unwrap();
        assert!(cache.peek(&key).is_some());
    }

    #[test]
    fn unfiltered_svg_reuses_its_parsed_tree_across_zoom_levels() {
        let source = Source::new(gpui::ImageFormat::Svg,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#,
            1024 * 1024).unwrap();
        let cache = Cache::default();
        cache.show(&[], &[source.id]);
        let generation = AtomicU64::new(0);
        let key = key(&source, (20, 10));
        let original = cache
            .original(&source, &key, &generation, 0)
            .unwrap()
            .unwrap();
        assert!(matches!(*original, source::Decoded::Vector(..)));
        let zoomed = TileKey {
            output: (1280, 640),
            ..key
        };
        let zoomed = cache
            .original(&source, &zoomed, &generation, 0)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&original, &zoomed));
        assert_eq!(originals(&cache).0, 1);
    }

    #[test]
    fn filtered_svg_rasters_are_shared_accounted_and_bounded() {
        let source = Source::new(
            gpui::ImageFormat::Svg,
            super::super::tests::OFFSET_SVG.as_bytes(),
            1024 * 1024,
        )
        .unwrap();
        let cache = Cache::default();
        cache.show(&[], &[source.id]);
        let generation = AtomicU64::new(0);
        let key = TileKey {
            column: 2,
            ..key(&source, (1100, 300))
        };
        cache.render(&source, &[key], &generation, 0).unwrap();
        let tile = cache.peek(&key).unwrap();
        assert_eq!(
            &tile.as_bytes(0).unwrap()[80 * 76 * 4..80 * 76 * 4 + 4],
            &[16, 32, 239, 255]
        );
        let original = cache
            .original(&source, &key, &generation, 0)
            .unwrap()
            .unwrap();
        let other = TileKey { column: 1, ..key };
        cache.render(&source, &[other], &generation, 0).unwrap();
        assert!(Arc::ptr_eq(
            &original,
            &cache
                .original(&source, &other, &generation, 0)
                .unwrap()
                .unwrap()
        ));
        assert_eq!(originals(&cache), (1, 1100 * 300 * 4));
        let zoomed = TileKey {
            output: (2200, 600),
            ..key
        };
        cache.render(&source, &[zoomed], &generation, 0).unwrap();
        assert_eq!(originals(&cache).1, (1100 * 300 + 2200 * 600) * 4);
        let oversized = TileKey {
            output: (70400, 19200),
            ..key
        };
        assert!(
            cache
                .render(&source, &[oversized], &generation, 0)
                .unwrap_err()
                .contains("limit")
        );
        assert!(
            cache.peek(&oversized).is_none(),
            "keep the bounded preview instead of caching incorrect tiles"
        );
        assert_eq!(originals(&cache).0, 2);
    }

    #[test]
    fn queued_obsolete_work_never_decodes_or_populates_cache() {
        let source = super::super::tests::source(64, 64);
        let cache = Arc::new(Cache::default());
        cache.show(&[], &[source.id]);
        let generation = Arc::new(AtomicU64::new(0));
        let first = cache.acquire();
        let second = cache.acquire();
        let (started, ready) = std::sync::mpsc::sync_channel(0);
        let worker = {
            let cache = cache.clone();
            let generation = generation.clone();
            std::thread::spawn(move || {
                started.send(()).unwrap();
                let key = key(&source, (128, 128));
                cache.render(&source, &[key], &generation, 0).unwrap();
            })
        };
        ready.recv().unwrap();
        assert_eq!(*cache.running.lock().unwrap(), 2);
        generation.store(1, Ordering::Relaxed);
        drop(first);
        drop(second);
        worker.join().unwrap();
        assert_eq!(originals(&cache).0, 0);
        assert!(cache.inner.lock().unwrap().tiles.is_empty());
    }

    #[gpui::test]
    fn closing_releases_retained_pixels_and_rejects_late_tiles(cx: &mut gpui::TestAppContext) {
        let source = super::super::tests::source(64, 64);
        let cache = Cache::default();
        cache.show(&[], &[source.id]);
        let key = key(&source, (128, 128));
        cache
            .render(&source, &[key], &AtomicU64::new(0), 0)
            .unwrap();
        cache.show(&[source.id], &[]);
        cx.update(|cx| cache.close(cx));
        cache
            .render(&source, &[key], &AtomicU64::new(0), 0)
            .unwrap();
        let inner = cache.inner.lock().unwrap();
        assert!(inner.closed && inner.tiles.is_empty() && inner.dropped.is_empty());
        assert_eq!(originals(&cache), (0, 0));
    }

    #[test]
    fn tiles_reuse_originals_and_lru_eviction_is_bounded() {
        let source = super::super::tests::source(64, 64);
        let cache = Cache::default();
        cache.show(&[], &[source.id]);
        let generation = AtomicU64::new(1);
        let key = key(&source, (64, 64));
        cache.render(&source, &[key], &generation, 0).unwrap();
        assert!(cache.peek(&key).is_none());
        cache.render(&source, &[key], &generation, 1).unwrap();
        let first = cache.peek(&key).unwrap();
        cache.render(&source, &[key], &generation, 1).unwrap();
        assert!(Arc::ptr_eq(&first, &cache.peek(&key).unwrap()));
        assert_eq!(originals(&cache).0, 1);
        for w in 65..130 {
            cache
                .render(
                    &source,
                    &[TileKey {
                        output: (w, 64),
                        ..key
                    }],
                    &generation,
                    1,
                )
                .unwrap();
        }
        let inner = cache.inner.lock().unwrap();
        assert_eq!(inner.tiles.len(), TILE_COUNT);
        assert_eq!(inner.dropped.len(), 2);
        assert!(!inner.tiles.contains(&key));
        assert_eq!(originals(&cache).0, 1);
    }

    /// An original over the budget used to be decoded for one request and
    /// dropped, so every pan or zoom decoded the whole image again (420 ms per
    /// tile for an 8200² PNG). It now stays, alone.
    #[test]
    fn oversized_original_is_kept_alone_for_the_next_pan() {
        let small = super::super::tests::source(32, 32);
        let large = super::super::tests::source(64, 64);
        let cache = Cache::new(Arc::new(Originals {
            budget: 64 * 64 * 4 - 1,
            ..Default::default()
        }));
        cache.show(&[], &[small.id, large.id]);
        let generation = AtomicU64::new(0);
        cache
            .render(&small, &[key(&small, (32, 32))], &generation, 0)
            .unwrap();
        assert_eq!(originals(&cache), (1, 32 * 32 * 4));
        cache
            .render(&large, &[key(&large, (64, 64))], &generation, 0)
            .unwrap();
        assert_eq!(originals(&cache), (1, 64 * 64 * 4));
        let pan = TileKey {
            output: (128, 128),
            column: 0,
            ..key(&large, (64, 64))
        };
        assert!(
            cache
                .original(&large, &pan, &AtomicU64::new(1), 0)
                .unwrap()
                .is_some(),
            "a cached original answers even a superseded request"
        );
    }

    /// The decode lock was taken before the cache lookup, so a pane whose
    /// original was already decoded waited behind another pane's decode.
    #[test]
    fn cached_original_does_not_wait_for_another_decode() {
        let mut slow = super::super::tests::source(64, 64);
        let fast = super::super::tests::source(32, 32);
        let (done, decoded) = std::sync::mpsc::channel();
        let (resume, paused) = std::sync::mpsc::channel();
        Arc::get_mut(&mut slow).unwrap().decode_gate = Some(Arc::new(source::DecodeGate {
            done,
            resume: Mutex::new(paused),
        }));
        let cache = Arc::new(Cache::default());
        cache.show(&[], &[slow.id, fast.id]);
        cache
            .render(&fast, &[key(&fast, (32, 32))], &AtomicU64::new(0), 0)
            .unwrap();
        let worker = {
            let cache = cache.clone();
            let key = key(&slow, (64, 64));
            std::thread::spawn(move || cache.render(&slow, &[key], &AtomicU64::new(0), 0))
        };
        decoded
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let (finished, pan) = std::sync::mpsc::channel();
        {
            let cache = cache.clone();
            let key = key(&fast, (64, 64));
            std::thread::spawn(move || {
                cache.render(&fast, &[key], &AtomicU64::new(0), 0).unwrap();
                finished.send(()).unwrap();
            });
        }
        let waited = pan.recv_timeout(std::time::Duration::from_secs(2)).is_err();
        resume.send(()).unwrap();
        worker.join().unwrap().unwrap();
        assert!(
            !waited,
            "a cached original waited behind another image's decode"
        );
    }

    /// Tiles and decoded pixels stayed resident for the session after the
    /// pane moved on to a text diff, including a decode finishing late.
    #[gpui::test]
    fn leaving_an_image_releases_its_tiles_and_pixels(cx: &mut gpui::TestAppContext) {
        let image = super::super::tests::source(64, 64);
        let group = cx.new(Group::new);
        let cache = group.update(cx, |group, cx| {
            group.set_inputs(
                "image".into(),
                [
                    Input {
                        source: Some(image.clone()),
                        ..Default::default()
                    },
                    Input::default(),
                    Input::default(),
                ],
                false,
                cx,
            );
            group.cache.clone()
        });
        let key = key(&image, (64, 64));
        cache.render(&image, &[key], &AtomicU64::new(0), 0).unwrap();
        assert!(cache.peek(&key).is_some());
        assert_eq!(originals(&cache).0, 1);
        group.update(cx, |group, cx| {
            group.set_inputs("text diff".into(), Default::default(), false, cx)
        });
        cx.run_until_parked();
        assert!(
            cache.peek(&key).is_none(),
            "tiles of a closed image stay resident"
        );
        assert!(cache.inner.lock().unwrap().dropped.is_empty());
        assert_eq!(originals(&cache), (0, 0));
        let late = TileKey {
            output: (128, 128),
            ..key
        };
        cache
            .render(&image, &[late], &AtomicU64::new(0), 0)
            .unwrap();
        assert_eq!(
            originals(&cache),
            (0, 0),
            "a late decode cached pixels nobody shows"
        );
    }
}
