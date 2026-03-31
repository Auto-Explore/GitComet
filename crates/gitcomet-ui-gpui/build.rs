mod build_themes;
use std::env;
use std::fs;
use std::path::PathBuf;

const SPLASH_BACKDROP_RASTER_WIDTH_PX: f32 = 960.0;
const SPLASH_BACKDROP_MAX_EDGE_PX: f32 = 1024.0;

fn rasterize_svg_png(svg_bytes: &[u8], target_width_px: f32, max_edge_px: f32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &resvg::usvg::Options::default()).ok()?;
    let svg_size = tree.size();
    let svg_width = svg_size.width();
    let svg_height = svg_size.height();
    if !svg_width.is_finite() || !svg_height.is_finite() || svg_width <= 0.0 || svg_height <= 0.0 {
        return None;
    }

    let upscale = if svg_width < target_width_px {
        target_width_px / svg_width
    } else {
        1.0
    };
    let mut raster_width = (svg_width * upscale).round();
    let mut raster_height = (svg_height * upscale).round();
    let max_edge = raster_width.max(raster_height);
    if max_edge > max_edge_px {
        let downscale = max_edge_px / max_edge;
        raster_width = (raster_width * downscale).round();
        raster_height = (raster_height * downscale).round();
    }

    let raster_width = raster_width.max(1.0) as u32;
    let raster_height = raster_height.max(1.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(raster_width, raster_height)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        raster_width as f32 / svg_width,
        raster_height as f32 / svg_height,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

fn main() {
    build_themes::generate_embedded_theme_registry();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/splash_backdrop.svg");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
    let svg_path = manifest_dir.join("../../assets/splash_backdrop.svg");
    let out_path =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing")).join("splash_backdrop.png");

    let svg_bytes = fs::read(&svg_path).expect("read splash_backdrop.svg");
    let png = rasterize_svg_png(
        &svg_bytes,
        SPLASH_BACKDROP_RASTER_WIDTH_PX,
        SPLASH_BACKDROP_MAX_EDGE_PX,
    )
    .expect("rasterize splash_backdrop.svg");
    fs::write(out_path, png).expect("write splash_backdrop.png");
}
