use image::{AnimationDecoder as _, ImageDecoder as _};
use resvg::usvg;
use std::{
    hash::BuildHasher as _,
    io::Cursor,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

// Ids key cached pixels. Seeded, so repository bytes cannot be crafted to collide.
static IDS: LazyLock<std::hash::RandomState> = LazyLock::new(std::hash::RandomState::new);

#[derive(Debug)]
pub(crate) struct Source {
    pub id: u64,
    pub dimensions: (u32, u32),
    pub vector: bool,
    pub(super) filtered_svg: bool,
    content: Content,
    #[cfg(test)]
    pub(super) decode_gate: Option<Arc<DecodeGate>>,
}

#[derive(Debug)]
enum Content {
    Raster {
        bytes: Arc<[u8]>,
        format: image::ImageFormat,
        limit: u64,
    },
    // Parsed once; previews and detail tiles share the tree.
    Vector(Arc<usvg::Tree>, usize),
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct DecodeGate {
    pub done: std::sync::mpsc::Sender<()>,
    pub resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

pub(super) enum Decoded {
    Raster(image::RgbaImage),
    Vector(Arc<usvg::Tree>, usize),
}
impl Decoded {
    pub fn bytes(&self) -> usize {
        match self {
            Self::Raster(image) => image.as_raw().len(),
            Self::Vector(_, bytes) => *bytes,
        }
    }

    /// Cache a filtered SVG once per output size; all its tiles then share the
    /// same filter inputs, and its pixels count against the decoded-image budget.
    pub fn for_output(self, output: (u32, u32)) -> Result<Self, String> {
        match self {
            Self::Vector(tree, _) if !tree.filters().is_empty() => {
                super::svg::raster_region(&tree, output, (0, 0, output.0, output.1))
                    .map(Self::Raster)
            }
            image => Ok(image),
        }
    }
}

/// The `image` codec for a raster format; `None` for SVG.
pub(in crate::view) fn raster_format(format: gpui::ImageFormat) -> Option<image::ImageFormat> {
    match format {
        gpui::ImageFormat::Png => Some(image::ImageFormat::Png),
        gpui::ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
        gpui::ImageFormat::Gif => Some(image::ImageFormat::Gif),
        gpui::ImageFormat::Webp => Some(image::ImageFormat::WebP),
        gpui::ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
        gpui::ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
        gpui::ImageFormat::Ico => Some(image::ImageFormat::Ico),
        gpui::ImageFormat::Pnm => Some(image::ImageFormat::Pnm),
        gpui::ImageFormat::Svg => None,
    }
}

/// Bound a decode by its input and RGBA output allocations.
pub(in crate::view) fn limit_decoder(
    decoder: &mut impl image::ImageDecoder,
    max_alloc: u64,
) -> image::ImageResult<()> {
    let (width, height) = decoder.dimensions();
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            image::ImageError::Limits(image::error::LimitError::from_kind(
                image::error::LimitErrorKind::InsufficientMemory,
            ))
        })?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max_alloc);
    limits.reserve(decoder.total_bytes())?;
    limits.reserve(rgba_bytes)?;
    decoder.set_limits(limits)
}

fn configure(decoder: &mut impl image::ImageDecoder, limit: u64) -> Result<(), String> {
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return Err("Empty image".into());
    }
    limit_decoder(decoder, limit).map_err(|e| e.to_string())
}

impl Source {
    /// Header work stays alongside preview decoding, off the UI thread.
    pub fn new(format: gpui::ImageFormat, bytes: &[u8], limit: u64) -> Option<Arc<Self>> {
        if bytes.len() as u64 > limit {
            return None;
        }
        let (dimensions, filtered_svg, content) = match raster_format(format) {
            None => {
                let tree = super::svg::parse(bytes).ok()?;
                let dimensions = (
                    tree.size().width().ceil().max(1.0) as u32,
                    tree.size().height().ceil().max(1.0) as u32,
                );
                let filtered = !tree.filters().is_empty();
                (
                    dimensions,
                    filtered,
                    Content::Vector(Arc::new(tree), bytes.len()),
                )
            }
            Some(format) => {
                let mut decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
                    .into_decoder()
                    .ok()?;
                configure(&mut decoder, limit).ok()?;
                let (w, h) = decoder.dimensions();
                use image::metadata::Orientation::*;
                let dimensions = match decoder.orientation().unwrap_or(NoTransforms) {
                    Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH => (h, w),
                    _ => (w, h),
                };
                let content = Content::Raster {
                    bytes: Arc::from(bytes),
                    format,
                    limit,
                };
                (dimensions, false, content)
            }
        };
        Some(Arc::new(Self {
            id: IDS.hash_one((bytes, std::mem::discriminant(&format))),
            dimensions,
            vector: matches!(content, Content::Vector(..)),
            filtered_svg,
            content,
            #[cfg(test)]
            decode_gate: None,
        }))
    }

    /// The parsed SVG behind a vector source.
    pub(in crate::view) fn svg(&self) -> Option<&usvg::Tree> {
        match &self.content {
            Content::Vector(tree, _) => Some(tree),
            Content::Raster { .. } => None,
        }
    }

    pub(super) fn decode(
        &self,
        frame: usize,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<Option<Decoded>, String> {
        if generation.load(Ordering::Relaxed) != expected {
            return Ok(None);
        }
        let (bytes, format, limit) = match &self.content {
            Content::Vector(tree, bytes) => return Ok(Some(Decoded::Vector(tree.clone(), *bytes))),
            Content::Raster {
                bytes,
                format,
                limit,
            } => (bytes.as_ref(), *format, *limit),
        };
        let mut decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
            .into_decoder()
            .map_err(|e| e.to_string())?;
        configure(&mut decoder, limit)?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut image = match format {
            image::ImageFormat::Gif => {
                drop(decoder);
                let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
                    .map_err(|e| e.to_string())?;
                configure(&mut decoder, limit)?;
                let frame =
                    match selected_frame(decoder.into_frames(), frame, generation, expected)? {
                        Some(frame) => frame,
                        None => return Ok(None),
                    };
                image::DynamicImage::ImageRgba8(frame.into_buffer())
            }
            image::ImageFormat::WebP => {
                drop(decoder);
                let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
                    .map_err(|e| e.to_string())?;
                configure(&mut decoder, limit)?;
                if decoder.has_animation() {
                    let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                    let frame =
                        match selected_frame(decoder.into_frames(), frame, generation, expected)? {
                            Some(frame) => frame,
                            None => return Ok(None),
                        };
                    image::DynamicImage::ImageRgba8(frame.into_buffer())
                } else {
                    image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?
                }
            }
            _ => image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?,
        };
        image.apply_orientation(orientation);
        let decoded = Decoded::Raster(image.into_rgba8());
        #[cfg(test)]
        if let Some(gate) = self.decode_gate.as_ref() {
            gate.done.send(()).unwrap();
            gate.resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        Ok(Some(decoded))
    }
}

fn selected_frame(
    frames: image::Frames<'_>,
    selected: usize,
    generation: &AtomicU64,
    expected: u64,
) -> Result<Option<image::Frame>, String> {
    for (index, frame) in frames.enumerate() {
        if generation.load(Ordering::Relaxed) != expected {
            return Ok(None);
        }
        let frame = frame.map_err(|e| e.to_string())?;
        if index == selected {
            return Ok(Some(frame));
        }
    }
    Err("Animation frame unavailable".into())
}
