//! Prepared SVG scenes and bounded raster regions. Zoom never repeats layout.
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, LazyLock};

pub const TILE_SIZE: u32 = 512;
// Native Mermaid shadows use small blurs; overlap protects tile boundaries
// before cropping to the exact device pixel grid.
const HALO: u32 = 32;

static OPTIONS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
    let mut options = usvg::Options::default();
    let fonts = options.fontdb_mut();
    for bytes in gitcomet_fonts::BUNDLED_FONT_BYTES {
        fonts.load_font_source(usvg::fontdb::Source::Binary(Arc::new(*bytes)));
    }
    fonts.set_sans_serif_family(gitcomet_fonts::IBM_PLEX_SANS_FONT_FAMILY);
    fonts.set_serif_family(gitcomet_fonts::IBM_PLEX_SANS_FONT_FAMILY);
    fonts.set_monospace_family(gitcomet_fonts::LILEX_FONT_FAMILY);
    options.font_family = gitcomet_fonts::IBM_PLEX_SANS_FONT_FAMILY.into();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options
});

pub struct PreparedScene {
    pub svg: Arc<[u8]>,
    tree: usvg::Tree,
}
impl PreparedScene {
    pub fn new(svg: Arc<[u8]>) -> Result<Self, String> {
        let tree = usvg::Tree::from_data(&svg, &OPTIONS).map_err(|e| e.to_string())?;
        Ok(Self { svg, tree })
    }
    pub fn width(&self) -> f32 {
        self.tree.size().width()
    }
    pub fn height(&self) -> f32 {
        self.tree.size().height()
    }

    /// Opaque RGBA at the requested scale, with device pixel aligned origins.
    pub fn raster_region(
        &self,
        scale: f32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        if !scale.is_finite()
            || scale <= 0.0
            || width == 0
            || height == 0
            || width > 4096
            || height > 4096
        {
            return Err("Invalid diagram raster region.".into());
        }
        let mut pixmap =
            tiny_skia::Pixmap::new(width, height).ok_or("Cannot allocate diagram tile.")?;
        pixmap.fill(tiny_skia::Color::WHITE);
        resvg::render(
            &self.tree,
            tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, -(x as f32), -(y as f32)),
            &mut pixmap.as_mut(),
        );
        Ok(pixmap.take())
    }

    pub fn raster_tile(&self, scale: f32, column: u32, row: u32) -> Result<Vec<u8>, String> {
        let x = column
            .checked_mul(TILE_SIZE)
            .and_then(|x| i32::try_from(x).ok())
            .ok_or("Diagram tile coordinate overflow.")?;
        let y = row
            .checked_mul(TILE_SIZE)
            .and_then(|y| i32::try_from(y).ok())
            .ok_or("Diagram tile coordinate overflow.")?;
        let expanded = TILE_SIZE + HALO * 2;
        let pixels =
            self.raster_region(scale, x - HALO as i32, y - HALO as i32, expanded, expanded)?;
        let mut tile = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 4) as usize);
        for row in HALO..HALO + TILE_SIZE {
            let start = ((row * expanded + HALO) * 4) as usize;
            tile.extend_from_slice(&pixels[start..start + (TILE_SIZE * 4) as usize]);
        }
        Ok(tile)
    }
}
