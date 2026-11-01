//! Explicit maintainer command; tests never rewrite expected images.
use gitcomet_diagrams::{DiagramKind, render, scene::PreparedScene};
use std::path::Path;
fn main() {
    assert_eq!(
        std::env::args().nth(1).as_deref(),
        Some("--write"),
        "Use --write to explicitly replace reviewed goldens."
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    std::fs::create_dir_all(root.join("goldens")).unwrap();
    for entry in std::fs::read_dir(root.join("fixtures"))
        .unwrap()
        .flatten()
        .filter(|p| p.path().is_dir())
    {
        let family = entry.file_name().into_string().unwrap();
        let source = std::fs::read_to_string(entry.path().join("basic.mmd")).unwrap();
        let svg = render(DiagramKind::Mermaid, &source)
            .unwrap()
            .pages
            .remove(0)
            .svg;
        let scene = PreparedScene::new(svg).unwrap();
        let (width, height) = (scene.width().ceil() as u32, scene.height().ceil() as u32);
        let pixels = scene.raster_region(1.0, 0, 0, width, height).unwrap();
        image::RgbaImage::from_raw(width, height, pixels)
            .unwrap()
            .save(root.join("goldens").join(format!("{family}.png")))
            .unwrap();
    }
}
