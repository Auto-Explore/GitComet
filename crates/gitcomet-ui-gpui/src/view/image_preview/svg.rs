//! Font resolution and rasterization shared by bounded previews and detail tiles.
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, LazyLock};

// Filters need a whole-image canvas and can allocate additional intermediate
// layers. Larger renders keep the bounded preview instead of losing filter input.
const MAX_RASTER_BYTES: u64 = 16 * 1024 * 1024;

static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    crate::bundled_fonts::load_into_fontdb(&mut fonts);
    let sans = crate::bundled_fonts::IBM_PLEX_SANS_FONT_FAMILY;
    fonts.set_sans_serif_family(sans);
    fonts.set_serif_family(sans);
    fonts.set_cursive_family(sans);
    fonts.set_fantasy_family(sans);
    fonts.set_monospace_family(crate::bundled_fonts::LILEX_FONT_FAMILY);
    Arc::new(fonts)
});

static OPTIONS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
    let select_font = usvg::FontResolver::default_font_selector();
    usvg::Options {
        font_resolver: usvg::FontResolver {
            select_font: Box::new(move |font, fonts| {
                // As in GPUI, defer the font scan until a document contains text.
                if fonts.is_empty() {
                    *fonts = FONTS.clone();
                }
                select_font(font, fonts).or_else(|| {
                    // CSS system font names and missing families need a fallback.
                    fonts.query(&usvg::fontdb::Query {
                        families: &[usvg::fontdb::Family::SansSerif],
                        ..Default::default()
                    })
                })
            }),
            ..Default::default()
        },
        ..Default::default()
    }
});

pub(in crate::view) fn parse(bytes: &[u8]) -> Result<usvg::Tree, usvg::Error> {
    usvg::Tree::from_data(bytes, &OPTIONS)
}

/// Opaque RGBA in global device-pixel coordinates.
pub(in crate::view) fn raster_region(
    tree: &usvg::Tree,
    output: (u32, u32),
    region: (u32, u32, u32, u32),
) -> Result<image::RgbaImage, String> {
    let (x, y, w, h) = region;
    if w == 0
        || h == 0
        || x.checked_add(w).is_none_or(|end| end > output.0)
        || y.checked_add(h).is_none_or(|end| end > output.1)
    {
        return Err("Invalid SVG raster region".into());
    }
    // resvg clips filter intermediates relative to the destination canvas. A
    // fixed tile overlap cannot preserve arbitrary offsets, blur, or nesting.
    // Use the same canvas as a whole-image render whenever filters are present.
    let whole = !tree.filters().is_empty() || region == (0, 0, output.0, output.1);
    const HALO: u32 = 32;
    let (width, height, crop_x, crop_y, translate_x, translate_y) = if whole {
        (output.0, output.1, x, y, 0.0, 0.0)
    } else {
        (
            w.checked_add(HALO * 2).ok_or("SVG tile width overflow")?,
            h.checked_add(HALO * 2).ok_or("SVG tile height overflow")?,
            HALO,
            HALO,
            HALO as f32 - x as f32,
            HALO as f32 - y as f32,
        )
    };
    if u64::from(width) * u64::from(height) > MAX_RASTER_BYTES / 4 {
        return Err("SVG render at this zoom exceeds the detailed preview limit".into());
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("Cannot allocate SVG render")?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        tree,
        tiny_skia::Transform::from_row(
            output.0 as f32 / tree.size().width(),
            0.0,
            0.0,
            output.1 as f32 / tree.size().height(),
            translate_x,
            translate_y,
        ),
        &mut pixmap.as_mut(),
    );
    let pixels =
        image::RgbaImage::from_raw(width, height, pixmap.take()).ok_or("Invalid SVG render")?;
    if (crop_x, crop_y, w, h) == (0, 0, width, height) {
        Ok(pixels)
    } else {
        Ok(image::imageops::crop_imm(&pixels, crop_x, crop_y, w, h).to_image())
    }
}
