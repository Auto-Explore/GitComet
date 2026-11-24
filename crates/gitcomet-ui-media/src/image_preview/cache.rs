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
mod tests;
