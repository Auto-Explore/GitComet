//! Exercise the same source decoding, resampler and tile cache as the viewer.
use super::*;
use std::sync::Arc;

pub struct ImageInspectionFixture {
    source: Arc<Source>,
    original: source::Decoded,
    cache: Cache,
}

impl Default for ImageInspectionFixture {
    fn default() -> Self {
        let image = image::RgbaImage::from_fn(2400, 1600, |x, y| {
            image::Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + y) % 256) as u8,
                if x % 19 == 0 { 64 } else { 255 },
            ])
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let source =
            Source::new(gpui::ImageFormat::Png, encoded.get_ref(), 512 * 1024 * 1024).unwrap();
        // As in a viewport showing the image, so its original stays cached.
        let cache = Cache::default();
        cache.show(&[], &[source.id]);
        Self {
            source,
            original: source::Decoded::Raster(image),
            cache,
        }
    }
}

impl ImageInspectionFixture {
    pub fn resample_visible_tile(&self, scale: f64, filter: &str) -> usize {
        let output = ((2400.0 * scale) as u32, (1600.0 * scale) as u32);
        let sampling = filter::sampling(
            ImageSamplingMode::Auto,
            ImageSmoothFilter::parse(Some(filter)),
            scale,
        );
        resample::tile(&self.original, output, 0, 0, sampling)
            .unwrap()
            .as_raw()
            .len()
    }

    pub fn cached_pan(&self) -> usize {
        let keys = (0..4)
            .map(|column| TileKey {
                source: self.source.id,
                frame: 0,
                output: (9600, 6400),
                sampling: filter::Sampling::Nearest,
                column,
                row: 0,
            })
            .collect::<Vec<_>>();
        self.cache
            .render(&self.source, &keys, &AtomicU64::new(0), 0)
            .unwrap();
        keys.iter()
            .filter(|key| self.cache.peek(key).is_some())
            .count()
    }
}
