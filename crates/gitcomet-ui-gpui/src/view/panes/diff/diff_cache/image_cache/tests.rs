use super::*;
use std::path::Path;

struct TestDecoder {
    dimensions: (u32, u32),
    orientation_error: bool,
    read_called: Arc<std::sync::atomic::AtomicBool>,
}

impl image::ImageDecoder for TestDecoder {
    fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }

    fn color_type(&self) -> image::ColorType {
        image::ColorType::Rgba8
    }

    fn orientation(&mut self) -> image::ImageResult<image::metadata::Orientation> {
        if self.orientation_error {
            Err(image::ImageError::IoError(std::io::Error::other(
                "metadata read failed",
            )))
        } else {
            Ok(image::metadata::Orientation::NoTransforms)
        }
    }

    fn read_image(self, buffer: &mut [u8]) -> image::ImageResult<()> {
        self.read_called
            .store(true, std::sync::atomic::Ordering::Release);
        for pixel in buffer.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[10, 20, 30, 255]);
        }
        Ok(())
    }

    fn read_image_boxed(self: Box<Self>, buffer: &mut [u8]) -> image::ImageResult<()> {
        (*self).read_image(buffer)
    }
}

fn solid_rect_svg(width: u32, height: u32) -> Vec<u8> {
    format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<rect width="{width}" height="{height}" fill="#00aaff"/>
</svg>"##
        )
        .into_bytes()
}

fn inset_rect_svg(width: u32, height: u32, inset_x: u32, inset_y: u32) -> Vec<u8> {
    let inner_width = width.saturating_sub(inset_x.saturating_mul(2));
    let inner_height = height.saturating_sub(inset_y.saturating_mul(2));
    format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<rect x="{inset_x}" y="{inset_y}" width="{inner_width}" height="{inner_height}" fill="#00aaff"/>
</svg>"##
        )
        .into_bytes()
}

fn render_pixel_bgra(render: &gpui::RenderImage, x: usize, y: usize) -> [u8; 4] {
    let size = render.size(0);
    let width = size.width.0 as usize;
    let offset = (y.saturating_mul(width).saturating_add(x)).saturating_mul(4);
    let bytes = render.as_bytes(0).expect("render bytes");
    [
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]
}

fn write_test_file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write test file");
    path
}

fn rotate_90_exif() -> Vec<u8> {
    vec![
        b'I', b'I', 42, 0, // Little-endian TIFF header.
        8, 0, 0, 0, // Offset to the first IFD.
        1, 0, // One directory entry.
        0x12, 0x01, // Orientation tag (0x0112).
        3, 0, // SHORT.
        1, 0, 0, 0, // One value.
        6, 0, 0, 0, // Rotate 90 degrees clockwise.
        0, 0, 0, 0, // No next IFD.
    ]
}

fn oriented_tiff_fixture() -> Vec<u8> {
    fn push_ifd_entry(bytes: &mut Vec<u8>, tag: u16, field_type: u16, value: u32) {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&field_type.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    const ENTRY_COUNT: u16 = 9;
    const PIXEL_OFFSET: u32 = 8 + 2 + ENTRY_COUNT as u32 * 12 + 4;
    let mut bytes = Vec::with_capacity(PIXEL_OFFSET as usize + 2);
    bytes.extend_from_slice(b"II");
    bytes.extend_from_slice(&42_u16.to_le_bytes());
    bytes.extend_from_slice(&8_u32.to_le_bytes());
    bytes.extend_from_slice(&ENTRY_COUNT.to_le_bytes());
    push_ifd_entry(&mut bytes, 256, 3, 2); // ImageWidth.
    push_ifd_entry(&mut bytes, 257, 3, 1); // ImageLength.
    push_ifd_entry(&mut bytes, 258, 3, 8); // BitsPerSample.
    push_ifd_entry(&mut bytes, 259, 3, 1); // No compression.
    push_ifd_entry(&mut bytes, 262, 3, 1); // BlackIsZero.
    push_ifd_entry(&mut bytes, 273, 4, PIXEL_OFFSET); // StripOffsets.
    push_ifd_entry(&mut bytes, 274, 3, 6); // Orientation: rotate 90.
    push_ifd_entry(&mut bytes, 278, 4, 1); // RowsPerStrip.
    push_ifd_entry(&mut bytes, 279, 4, 2); // StripByteCounts.
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&[0, 255]);
    bytes
}

fn oriented_raster_fixture(format: gpui::ImageFormat) -> Vec<u8> {
    use image::ImageEncoder as _;

    let mut encoded = Vec::new();
    let exif = rotate_90_exif();
    match format {
        gpui::ImageFormat::Jpeg => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut encoded);
            encoder.set_exif_metadata(exif).expect("JPEG EXIF metadata");
            encoder
                .write_image(
                    &[255, 0, 0, 0, 255, 0],
                    2,
                    1,
                    image::ExtendedColorType::Rgb8,
                )
                .expect("encode oriented JPEG");
        }
        gpui::ImageFormat::Png => {
            let mut encoder = image::codecs::png::PngEncoder::new(&mut encoded);
            encoder.set_exif_metadata(exif).expect("PNG EXIF metadata");
            encoder
                .write_image(
                    &[255, 0, 0, 255, 0, 255, 0, 255],
                    2,
                    1,
                    image::ExtendedColorType::Rgba8,
                )
                .expect("encode oriented PNG");
        }
        gpui::ImageFormat::Tiff => return oriented_tiff_fixture(),
        gpui::ImageFormat::Webp => {
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut encoded);
            encoder.set_exif_metadata(exif).expect("WebP EXIF metadata");
            encoder
                .write_image(
                    &[255, 0, 0, 255, 0, 255, 0, 255],
                    2,
                    1,
                    image::ExtendedColorType::Rgba8,
                )
                .expect("encode oriented WebP");
        }
        _ => panic!("unsupported oriented raster fixture format: {format:?}"),
    }
    encoded
}

#[test]
fn transparent_edge_extension_keeps_alpha_and_uses_alpha_weighted_rgb() {
    let mut buffer = image::RgbaImage::from_raw(
        5,
        1,
        vec![
            9, 8, 7, 0, // Transparent left edge.
            255, 0, 0, 255, // Opaque red.
            0, 0, 0, 0, // Between red and blue.
            0, 0, 255, 128, // Half-alpha blue.
            6, 5, 4, 0, // Transparent right edge.
        ],
    )
    .expect("test buffer");

    extend_transparent_edge_rgb(&mut buffer);

    assert_eq!(buffer.get_pixel(0, 0).0, [255, 0, 0, 0]);
    assert_eq!(buffer.get_pixel(1, 0).0, [255, 0, 0, 255]);
    assert_eq!(buffer.get_pixel(2, 0).0, [170, 0, 85, 0]);
    assert_eq!(buffer.get_pixel(3, 0).0, [0, 0, 255, 128]);
    assert_eq!(buffer.get_pixel(4, 0).0, [0, 0, 255, 0]);
}

#[test]
fn transparent_edge_extension_is_one_texel_and_leaves_empty_images_unchanged() {
    let mut buffer = image::RgbaImage::from_raw(
        5,
        1,
        vec![
            7, 8, 9, 0, 0, 0, 0, 0, 40, 80, 120, 255, 0, 0, 0, 0, 3, 2, 1, 0,
        ],
    )
    .expect("test buffer");

    extend_transparent_edge_rgb(&mut buffer);

    assert_eq!(buffer.get_pixel(0, 0).0, [7, 8, 9, 0]);
    assert_eq!(buffer.get_pixel(1, 0).0, [40, 80, 120, 0]);
    assert_eq!(buffer.get_pixel(3, 0).0, [40, 80, 120, 0]);
    assert_eq!(buffer.get_pixel(4, 0).0, [3, 2, 1, 0]);

    let mut empty = image::RgbaImage::from_pixel(2, 2, image::Rgba([3, 4, 5, 0]));
    extend_transparent_edge_rgb(&mut empty);
    assert!(empty.pixels().all(|pixel| pixel.0 == [3, 4, 5, 0]));
}

#[test]
fn alpha_correct_thumbnail_keeps_straight_edge_color() {
    let source =
        image::RgbaImage::from_raw(2, 1, vec![200, 100, 50, 255, 0, 0, 0, 0]).expect("test buffer");

    let resized = alpha_correct_thumbnail(source, 1);
    let pixel = resized.get_pixel(0, 0).0;

    assert_eq!(resized.dimensions(), (1, 1));
    assert!(pixel[0].abs_diff(200) <= 1, "red={}", pixel[0]);
    assert!(pixel[1].abs_diff(100) <= 1, "green={}", pixel[1]);
    assert!(pixel[2].abs_diff(50) <= 1, "blue={}", pixel[2]);
    assert!(pixel[3].abs_diff(128) <= 1, "alpha={}", pixel[3]);
}

#[test]
fn decode_allocation_limit_is_checked_before_reading_pixels() {
    let read_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let decoder = TestDecoder {
        dimensions: (10, 10),
        orientation_error: false,
        read_called: Arc::clone(&read_called),
    };
    let policy = RasterPreviewPolicy {
        max_edge_px: 10,
        max_frames: 1,
        max_retained_bytes: usize::MAX,
        // Decoder RGBA output plus the projected prepared RGBA buffer is
        // 800 bytes, so this must reject without calling read_image.
        max_decode_bytes: 799,
    };

    assert!(decode_oriented_static_raster_frame(decoder, policy).is_none());
    assert!(!read_called.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn orientation_metadata_failure_falls_back_to_unrotated_decode() {
    let read_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let decoder = TestDecoder {
        dimensions: (1, 1),
        orientation_error: true,
        read_called: Arc::clone(&read_called),
    };
    let policy = RasterPreviewPolicy {
        max_edge_px: 1,
        max_frames: 1,
        max_retained_bytes: 4,
        max_decode_bytes: 8,
    };

    let frame = decode_oriented_static_raster_frame(decoder, policy)
        .expect("pixel decode should survive metadata failure");
    assert!(read_called.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(frame.buffer().get_pixel(0, 0).0, [30, 20, 10, 255]);
}

#[test]
fn prepared_raster_frame_preserves_timing_and_extends_bgra_edge_color() {
    let delay = image::Delay::from_numer_denom_ms(17, 1);
    let frame = image::Frame::from_parts(
        image::RgbaImage::from_raw(3, 1, vec![0, 0, 0, 0, 12, 34, 56, 128, 0, 0, 0, 0])
            .expect("test buffer"),
        2,
        3,
        delay,
    );

    let prepared = prepare_raster_preview_frame(frame, FILE_DIFF_RASTER_PREVIEW_POLICY);

    assert_eq!(prepared.delay(), delay);
    assert_eq!(prepared.left(), 2);
    assert_eq!(prepared.top(), 3);
    assert_eq!(prepared.buffer().get_pixel(0, 0).0, [56, 34, 12, 0]);
    assert_eq!(prepared.buffer().get_pixel(1, 0).0, [56, 34, 12, 128]);
    assert_eq!(prepared.buffer().get_pixel(2, 0).0, [56, 34, 12, 0]);
}

#[test]
fn raster_preview_png_uploads_transparent_edge_with_extended_bgra() {
    let image = image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 0, 0, 0, 0]).expect("test buffer"),
    );
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .expect("encode png");

    let render = render_raster_image_diff_preview(gpui::ImageFormat::Png, &encoded.into_inner())
        .expect("render image");

    assert_eq!(render_pixel_bgra(&render, 0, 0), [30, 20, 10, 255]);
    assert_eq!(render_pixel_bgra(&render, 1, 0), [30, 20, 10, 0]);
}

#[test]
fn raster_preview_gif_preserves_frames_delays_and_transparent_edge_color() {
    let frames = [
        image::Frame::from_parts(
            image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 0, 0, 0, 0])
                .expect("first GIF frame"),
            0,
            0,
            image::Delay::from_numer_denom_ms(40, 1),
        ),
        image::Frame::from_parts(
            image::RgbaImage::from_raw(2, 1, vec![70, 80, 90, 255, 0, 0, 0, 0])
                .expect("second GIF frame"),
            0,
            0,
            image::Delay::from_numer_denom_ms(70, 1),
        ),
    ];
    let mut encoded = Vec::new();
    image::codecs::gif::GifEncoder::new(&mut encoded)
        .encode_frames(frames)
        .expect("encode GIF frames");

    let render =
        render_raster_image_diff_preview(gpui::ImageFormat::Gif, &encoded).expect("render GIF");

    assert_eq!(render.frame_count(), 2);
    assert_eq!(render.delay(0).numer_denom_ms(), (40, 1));
    assert_eq!(render.delay(1).numer_denom_ms(), (70, 1));
    assert_eq!(render_pixel_bgra(&render, 0, 0), [30, 20, 10, 255]);
    assert_eq!(render_pixel_bgra(&render, 1, 0), [30, 20, 10, 0]);
    assert_eq!(
        render.as_bytes(1).expect("second frame")[0..4],
        [90, 80, 70, 255]
    );
    assert_eq!(
        render.as_bytes(1).expect("second frame")[4..8],
        [90, 80, 70, 0]
    );
}

#[test]
fn animation_frame_preparation_stops_at_frame_and_byte_limits() {
    let decoded_for_frame_limit = std::cell::Cell::new(0_usize);
    let frames = (0..10).map(|_| {
        decoded_for_frame_limit.set(decoded_for_frame_limit.get() + 1);
        Ok(image::Frame::new(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        )))
    });
    let prepared = prepare_raster_preview_animation_frames(
        frames,
        (2, 2),
        image::metadata::Orientation::NoTransforms,
        RasterPreviewPolicy {
            max_edge_px: 2,
            max_frames: 2,
            max_retained_bytes: usize::MAX,
            max_decode_bytes: u64::MAX,
        },
    );

    assert_eq!(prepared.len(), 2);
    assert_eq!(decoded_for_frame_limit.get(), 2);

    let decoded_for_byte_limit = std::cell::Cell::new(0_usize);
    let frames = (0..10).map(|_| {
        decoded_for_byte_limit.set(decoded_for_byte_limit.get() + 1);
        Ok(image::Frame::new(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        )))
    });
    let prepared = prepare_raster_preview_animation_frames(
        frames,
        (2, 2),
        image::metadata::Orientation::NoTransforms,
        RasterPreviewPolicy {
            max_edge_px: 2,
            max_frames: usize::MAX,
            max_retained_bytes: 32,
            max_decode_bytes: u64::MAX,
        },
    );
    let retained_bytes = prepared
        .iter()
        .map(|frame| frame.buffer().as_raw().len())
        .sum::<usize>();

    assert_eq!(prepared.len(), 2);
    assert_eq!(retained_bytes, 32);
    assert_eq!(decoded_for_byte_limit.get(), 2);
}

#[test]
fn animation_frame_preparation_stops_after_first_decode_error() {
    let polls = std::cell::Cell::new(0_usize);
    let frames = std::iter::from_fn(|| {
        polls.set(polls.get() + 1);
        Some(Err(image::ImageError::IoError(std::io::Error::other(
            "corrupt frame",
        ))))
    });

    let prepared = prepare_raster_preview_animation_frames(
        frames,
        (1, 1),
        image::metadata::Orientation::NoTransforms,
        FILE_DIFF_RASTER_PREVIEW_POLICY,
    );

    assert!(prepared.is_empty());
    assert_eq!(polls.get(), 1);
}

#[test]
fn animation_frame_preparation_applies_orientation_to_every_frame() {
    let frames = (0..2).map(|_| {
        Ok(image::Frame::new(image::RgbaImage::from_pixel(
            2,
            1,
            image::Rgba([10, 20, 30, 255]),
        )))
    });
    let prepared = prepare_raster_preview_animation_frames(
        frames,
        (2, 1),
        image::metadata::Orientation::Rotate90,
        RasterPreviewPolicy {
            max_edge_px: 2,
            max_frames: 2,
            max_retained_bytes: 16,
            max_decode_bytes: 16,
        },
    );

    assert_eq!(prepared.len(), 2);
    assert!(
        prepared
            .iter()
            .all(|frame| frame.buffer().dimensions() == (1, 2))
    );
}

#[test]
fn alpha_correct_thumbnail_never_upscales_or_changes_empty_images() {
    let source = image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]));
    assert_eq!(alpha_correct_thumbnail(source, 1920).dimensions(), (4, 2));

    let empty = image::RgbaImage::new(0, 0);
    assert_eq!(alpha_correct_thumbnail(empty, 1920).dimensions(), (0, 0));
}

#[test]
fn raster_preview_gif_caps_retained_frame_count() {
    let mut encoded = Vec::new();
    image::codecs::gif::GifEncoder::new(&mut encoded)
        .encode_frames(
            (0..IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES + 5).map(|index| {
                image::Frame::from_parts(
                    image::RgbaImage::from_pixel(1, 1, image::Rgba([index as u8, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(10, 1),
                )
            }),
        )
        .expect("encode many-frame GIF");

    let render = render_raster_image_diff_preview(gpui::ImageFormat::Gif, &encoded)
        .expect("render capped GIF");

    assert_eq!(
        render.frame_count(),
        IMAGE_DIFF_RASTER_PREVIEW_MAX_ANIMATION_FRAMES
    );
}

#[test]
fn conflict_raster_preview_keeps_only_a_512px_first_frame() {
    let frames = (0..2).map(|index| {
        image::Frame::from_parts(
            image::RgbaImage::from_pixel(1024, 512, image::Rgba([index as u8, 20, 30, 255])),
            0,
            0,
            image::Delay::from_numer_denom_ms(40, 1),
        )
    });
    let mut encoded = Vec::new();
    image::codecs::gif::GifEncoder::new(&mut encoded)
        .encode_frames(frames)
        .expect("encode conflict GIF");

    let render = render_raster_conflict_preview(gpui::ImageFormat::Gif, &encoded)
        .expect("render conflict GIF");
    assert_eq!(render.frame_count(), 1);
    assert_eq!(render.size(0).width.0, 512);
    assert_eq!(render.size(0).height.0, 256);
}

#[test]
fn file_animation_state_follows_delays_and_resets_for_new_sources() {
    let image = Arc::new(gpui::RenderImage::new(vec![
        image::Frame::from_parts(
            image::RgbaImage::new(1, 1),
            0,
            0,
            image::Delay::from_numer_denom_ms(40, 1),
        ),
        image::Frame::from_parts(
            image::RgbaImage::new(1, 1),
            0,
            0,
            image::Delay::from_numer_denom_ms(70, 1),
        ),
    ]));
    let start = std::time::Instant::now();
    let mut state = FileImagePreviewAnimationSide::default();

    assert_eq!(
        DiffRenderer::advance_file_image_preview_side(&mut state, Some(&image), start, true,),
        Some(start + std::time::Duration::from_millis(40))
    );
    assert_eq!(state.frame_index, 0);
    DiffRenderer::advance_file_image_preview_side(
        &mut state,
        Some(&image),
        start + std::time::Duration::from_millis(40),
        true,
    );
    assert_eq!(state.frame_index, 1);

    let replacement = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::new(1, 1),
    )]));
    DiffRenderer::advance_file_image_preview_side(
        &mut state,
        Some(&replacement),
        start + std::time::Duration::from_millis(50),
        true,
    );
    assert_eq!(state.image_id, Some(replacement.id));
    assert_eq!(state.frame_index, 0);
    assert!(state.frame_started_at.is_none());
}

#[test]
fn static_raster_previews_apply_metadata_orientation() {
    for format in [
        gpui::ImageFormat::Jpeg,
        gpui::ImageFormat::Png,
        gpui::ImageFormat::Tiff,
        gpui::ImageFormat::Webp,
    ] {
        let encoded = oriented_raster_fixture(format);
        let render = render_raster_image_diff_preview(format, &encoded)
            .unwrap_or_else(|| panic!("render oriented {format:?}"));
        let size = render.size(0);

        assert_eq!(
            (size.width.0, size.height.0),
            (1, 2),
            "{format:?} preview should apply its rotate-90 metadata"
        );
    }
}

#[test]
fn build_file_image_diff_cache_rebuild_decodes_ico_with_alpha_correct_edges() {
    let mut source = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 0]));
    *source.get_pixel_mut(0, 0) = image::Rgba([10, 20, 30, 255]);
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(source)
        .write_to(&mut encoded, image::ImageFormat::Ico)
        .expect("encode ICO");
    let bytes = encoded.into_inner();
    let file = gitcomet_core::domain::FileDiffImage {
        path: Path::new("images/sample.ico").to_path_buf(),
        old: Some(bytes.clone()),
        new: Some(bytes),
        ..Default::default()
    };

    let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
    let old = rebuild.old.expect("old ICO preview");
    let new = rebuild.new.expect("new ICO preview");

    assert!(Arc::ptr_eq(&old, &new));
    assert_eq!(render_pixel_bgra(&old, 0, 0), [30, 20, 10, 255]);
    assert_eq!(render_pixel_bgra(&old, 1, 0), [30, 20, 10, 0]);
    assert!(rebuild.old_svg_path.is_none());
    assert!(rebuild.new_svg_path.is_none());
}

#[test]
fn file_image_diff_signature_changes_with_payload() {
    let base = gitcomet_core::domain::FileDiffImage {
        path: Path::new("image.png").to_path_buf(),
        old: Some(vec![1, 2, 3]),
        new: Some(vec![4, 5, 6]),
        ..Default::default()
    };
    let changed = gitcomet_core::domain::FileDiffImage {
        path: Path::new("image.png").to_path_buf(),
        old: Some(vec![1, 2, 3, 4]),
        new: Some(vec![4, 5, 6]),
        ..Default::default()
    };

    assert_ne!(
        file_image_diff_signature(&base),
        file_image_diff_signature(&changed)
    );
}

#[test]
fn build_file_image_diff_cache_rebuild_resolves_absolute_preview_path() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let file = gitcomet_core::domain::FileDiffImage {
        path: Path::new("images/sample.png").to_path_buf(),
        old: Some(vec![1, 2, 3]),
        new: Some(vec![4, 5, 6]),
        ..Default::default()
    };

    let rebuild = build_file_image_diff_cache_rebuild(&file, temp_dir.path());
    let expected = temp_dir.path().join("images/sample.png");
    assert_eq!(rebuild.file_path.as_deref(), Some(expected.as_path()));
    assert!(rebuild.old.is_none());
    assert!(rebuild.new.is_none());
}

#[test]
fn decode_file_image_diff_preview_side_clamps_large_png_to_preview_bounds() {
    let width = IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX * 2;
    let height = IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX;
    let image = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
        width,
        height,
        image::Rgba([12, 34, 56, 255]),
    ));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .expect("encode png");

    let preview =
        decode_file_image_diff_preview_side(gpui::ImageFormat::Png, &encoded.into_inner());
    let render = preview.render.expect("preview render image");
    let size = render.size(0);
    assert_eq!(size.width.0, IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX as i32);
    assert_eq!(
        size.height.0,
        (IMAGE_DIFF_RASTER_PREVIEW_MAX_EDGE_PX / 2) as i32
    );
    assert!(preview.cached_path.is_none());
}

#[test]
fn decode_file_image_diff_preview_side_rasterizes_svg_without_path_fallback() {
    let svg = solid_rect_svg(4096, 2048);
    let preview = decode_file_image_diff_preview_side(gpui::ImageFormat::Svg, &svg);
    let render = preview.render.expect("svg render image");
    let size = render.size(0);
    assert_eq!(size.width.0, IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX as i32);
    assert_eq!(
        size.height.0,
        (IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / 2.0) as i32
    );
    assert!(preview.cached_path.is_none());
}

#[test]
fn render_svg_image_diff_preview_fills_transparent_viewport_white() {
    let svg = inset_rect_svg(4, 4, 1, 1);
    let render =
        render_svg_image_diff_preview(&svg::parse(&svg).unwrap()).expect("svg render image");
    let size = render.size(0);

    assert_eq!(render_pixel_bgra(&render, 0, 0), [255, 255, 255, 255]);
    assert_eq!(
        render_pixel_bgra(
            &render,
            (size.width.0 as usize) / 2,
            (size.height.0 as usize) / 2,
        ),
        [255, 170, 0, 255]
    );
}

#[test]
fn decode_file_image_diff_preview_side_upscales_small_svg_to_target_width() {
    let svg = solid_rect_svg(32, 16);
    let preview = decode_file_image_diff_preview_side(gpui::ImageFormat::Svg, &svg);
    let render = preview.render.expect("svg render image");
    let size = render.size(0);
    assert_eq!(size.width.0, IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX as i32);
    assert_eq!(
        size.height.0,
        (IMAGE_DIFF_SVG_PREVIEW_TARGET_WIDTH_PX / 2.0) as i32
    );
    assert!(preview.cached_path.is_none());
}

#[test]
fn decode_file_image_diff_preview_side_keeps_svg_path_fallback_for_invalid_svg() {
    let preview = decode_file_image_diff_preview_side(gpui::ImageFormat::Svg, b"<not-valid-svg>");
    assert!(preview.render.is_none());
    assert!(preview.cached_path.is_some());
    assert!(preview.cached_path.unwrap().exists());
}

#[test]
fn build_file_image_diff_cache_rebuild_reuses_identical_render_preview() {
    let image = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
        64,
        32,
        image::Rgba([200, 100, 50, 255]),
    ));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .expect("encode png");
    let bytes = encoded.into_inner();

    let file = gitcomet_core::domain::FileDiffImage {
        path: Path::new("images/sample.png").to_path_buf(),
        old: Some(bytes.clone()),
        new: Some(bytes),
        ..Default::default()
    };

    let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
    let old = rebuild.old.expect("old preview");
    let new = rebuild.new.expect("new preview");
    assert!(Arc::ptr_eq(&old, &new));
}

#[test]
fn build_file_image_diff_cache_rebuild_reuses_identical_svg_render_preview() {
    let svg = solid_rect_svg(2048, 1024);
    let file = gitcomet_core::domain::FileDiffImage {
        path: Path::new("images/sample.svg").to_path_buf(),
        old: Some(svg.clone()),
        new: Some(svg),
        ..Default::default()
    };

    let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
    let old = rebuild.old.expect("old preview");
    let new = rebuild.new.expect("new preview");
    assert!(Arc::ptr_eq(&old, &new));
    assert!(rebuild.old_svg_path.is_none());
    assert!(rebuild.new_svg_path.is_none());
}

#[test]
fn build_file_image_diff_cache_rebuild_rasterizes_distinct_svg_sides_without_fallback_paths() {
    let file = gitcomet_core::domain::FileDiffImage {
        path: Path::new("images/sample.svg").to_path_buf(),
        old: Some(solid_rect_svg(4096, 2048)),
        new: Some(solid_rect_svg(2048, 4096)),
        ..Default::default()
    };

    let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
    let old = rebuild.old.expect("old preview");
    let new = rebuild.new.expect("new preview");
    assert_eq!(
        old.size(0).width.0,
        IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX as i32
    );
    assert_eq!(
        old.size(0).height.0,
        (IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / 2.0) as i32
    );
    assert_eq!(
        new.size(0).width.0,
        (IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX / 2.0) as i32
    );
    assert_eq!(
        new.size(0).height.0,
        IMAGE_DIFF_SVG_PREVIEW_MAX_EDGE_PX as i32
    );
    assert!(rebuild.old_svg_path.is_none());
    assert!(rebuild.new_svg_path.is_none());
}

#[test]
fn build_file_image_diff_cache_rebuild_uses_fallback_paths_for_invalid_distinct_svg_sides() {
    let file = gitcomet_core::domain::FileDiffImage {
        path: Path::new("images/sample.svg").to_path_buf(),
        old: Some(b"<not-valid-svg-old>".to_vec()),
        new: Some(b"<not-valid-svg-new>".to_vec()),
        ..Default::default()
    };

    let rebuild = build_file_image_diff_cache_rebuild(&file, Path::new("/tmp"));
    assert!(rebuild.old.is_none());
    assert!(rebuild.new.is_none());
    assert!(
        rebuild
            .old_svg_path
            .as_ref()
            .is_some_and(|path| path.exists())
    );
    assert!(
        rebuild
            .new_svg_path
            .as_ref()
            .is_some_and(|path| path.exists())
    );
}

#[test]
fn image_format_for_path_detects_known_extensions_case_insensitively() {
    assert_eq!(
        image_format_for_path(Path::new("x.PNG")),
        Some(gpui::ImageFormat::Png)
    );
    assert_eq!(
        image_format_for_path(Path::new("x.JpEg")),
        Some(gpui::ImageFormat::Jpeg)
    );
    assert_eq!(
        image_format_for_path(Path::new("x.GiF")),
        Some(gpui::ImageFormat::Gif)
    );
    assert_eq!(
        image_format_for_path(Path::new("x.webp")),
        Some(gpui::ImageFormat::Webp)
    );
    assert_eq!(
        image_format_for_path(Path::new("x.BMP")),
        Some(gpui::ImageFormat::Bmp)
    );
    assert_eq!(
        image_format_for_path(Path::new("x.TiFf")),
        Some(gpui::ImageFormat::Tiff)
    );
    assert_eq!(
        image_format_for_path(Path::new("x.ICO")),
        Some(gpui::ImageFormat::Ico)
    );
}

#[test]
fn image_format_for_path_returns_none_for_unknown_or_missing_extension() {
    assert_eq!(image_format_for_path(Path::new("x.heic")), None);
    assert_eq!(image_format_for_path(Path::new("x")), None);
}

#[test]
fn decode_file_image_diff_bytes_keeps_non_svg_bytes() {
    let bytes = [1_u8, 2, 3, 4, 5];
    let mut svg_path = None;
    let image =
        decode_file_image_diff_bytes(gpui::ImageFormat::Png, &bytes, Some(&mut svg_path)).unwrap();
    assert_eq!(image.format(), gpui::ImageFormat::Png);
    assert_eq!(image.bytes(), bytes);
    assert!(svg_path.is_none());
}

#[test]
fn decode_file_image_diff_bytes_uses_cached_svg_file_for_valid_svg() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
<rect width="16" height="16" fill="#00aaff"/>
</svg>"##;
    let mut svg_path = None;
    let image = decode_file_image_diff_bytes(gpui::ImageFormat::Svg, svg, Some(&mut svg_path));
    assert!(image.is_none());
    let svg_path = svg_path.expect("svg should produce a cached file path");
    assert!(svg_path.exists());
    assert_eq!(svg_path.extension().and_then(|s| s.to_str()), Some("svg"));
}

#[test]
fn decode_file_image_diff_bytes_keeps_svg_path_fallback_for_invalid_svg() {
    let mut svg_path = None;
    let image = decode_file_image_diff_bytes(
        gpui::ImageFormat::Svg,
        b"<not-valid-svg>",
        Some(&mut svg_path),
    );
    assert!(image.is_none());
    assert!(svg_path.is_some());
    assert!(svg_path.unwrap().exists());
}

#[test]
fn image_diff_cache_reuses_the_content_addressed_file() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = [0_u8, 0, 1, 0, 1, 0, 16, 16];

    let path = image_diff_cache_path(dir.path(), &bytes, "ico").expect("cached path");
    let reused = image_diff_cache_path(dir.path(), &bytes, "ico").expect("second cached path");

    assert_eq!(path, reused);
    assert_eq!(path.extension().and_then(|s| s.to_str()), Some("ico"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(std::fs::symlink_metadata(&path).unwrap().is_file());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "a cache hit must not write a second file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn image_diff_cache_dir_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    // A content-addressed name is guessable, so the directory holding it
    // must not be one other users can create entries in.
    let dir = image_diff_cache_dir().expect("cache dir");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn image_diff_cache_dir_rejects_a_world_writable_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(IMAGE_DIFF_CACHE_DIR_NAME);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();

    let result = image_diff_cache_dir_in(root.path());

    assert!(result.is_err(), "{result:?}");
}

#[cfg(unix)]
#[test]
fn image_diff_cache_does_not_reuse_a_planted_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();

    // Plant the symlink under the exact name this content hashes to.
    let planted = image_diff_cache_path(dir.path(), b"preview", "svg").unwrap();
    std::fs::remove_file(&planted).unwrap();
    std::os::unix::fs::symlink(&victim, &planted).unwrap();

    let err = image_diff_cache_path(dir.path(), b"preview", "svg")
        .expect_err("a planted symlink must never be served as a cache hit");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
}

#[test]
fn cleanup_image_diff_cache_dir_removes_stale_prefixed_files() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let stale = write_test_file(
        temp_dir.path(),
        "gitcomet-image-diff-stale.svg",
        b"old-cache",
    );
    let non_cache = write_test_file(temp_dir.path(), "keep-me.txt", b"keep");

    cleanup_image_diff_cache_dir(
        temp_dir.path(),
        std::time::Duration::from_secs(60),
        u64::MAX,
        std::time::SystemTime::now() + std::time::Duration::from_secs(60 * 60),
    )
    .expect("cleanup");

    assert!(!stale.exists());
    assert!(non_cache.exists());
}

#[test]
fn cleanup_image_diff_cache_dir_prunes_to_max_total_size() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let a = write_test_file(temp_dir.path(), "gitcomet-image-diff-a.svg", b"1234");
    let b = write_test_file(temp_dir.path(), "gitcomet-image-diff-b.svg", b"1234");
    let c = write_test_file(temp_dir.path(), "gitcomet-image-diff-c.svg", b"1234");
    let non_cache = write_test_file(temp_dir.path(), "unrelated.bin", b"1234567890");

    cleanup_image_diff_cache_dir(
        temp_dir.path(),
        std::time::Duration::from_secs(60 * 60 * 24),
        8,
        std::time::SystemTime::now(),
    )
    .expect("cleanup");

    let cache_paths = [&a, &b, &c];
    let remaining_count = cache_paths.iter().filter(|path| path.exists()).count();
    assert_eq!(remaining_count, 2);

    let remaining_total = cache_paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| std::fs::metadata(path).expect("metadata").len())
        .sum::<u64>();
    assert!(remaining_total <= 8);
    assert!(non_cache.exists());
}
