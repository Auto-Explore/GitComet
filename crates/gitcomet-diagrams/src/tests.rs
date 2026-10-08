use super::*;
use scene::PreparedScene;

fn fixture(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(path),
    )
    .unwrap()
}
fn assert_render(source: &str, expected_label: &str) -> Arc<[u8]> {
    assert_eq!(DiagramKind::detect(source), Some(DiagramKind::Mermaid));
    let output = render(DiagramKind::Mermaid, source).unwrap();
    assert_eq!(output.pages.len(), 1);
    let svg = output.pages[0].svg.clone();
    let doc = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
    let text = doc
        .descendants()
        .filter_map(|node| node.text())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.split_whitespace()
            .collect::<String>()
            .contains(&expected_label.split_whitespace().collect::<String>()),
        "missing {expected_label:?} in {text}"
    );
    for node in doc.descendants() {
        for attribute in node.attributes() {
            assert!(
                !attribute.value().contains("NaN") && !attribute.value().contains("Infinity"),
                "{attribute:?}"
            );
        }
    }
    let scene = PreparedScene::new(svg.clone()).unwrap();
    assert!(scene.width().is_finite() && scene.height().is_finite());
    let scale = (512.0 / scene.width().max(scene.height())).min(1.0);
    let pixels = scene
        .raster_region(
            scale,
            0,
            0,
            (scene.width() * scale).ceil() as u32,
            (scene.height() * scale).ceil() as u32,
        )
        .unwrap();
    assert!(
        pixels.as_chunks::<4>().0.iter().any(|p| *p != [255; 4]),
        "empty raster"
    );
    svg
}
macro_rules! family_cases {
    ($family:ident, $basic:literal, $styled:literal) => {
        mod $family {
            use super::*;
            #[test]
            fn basic() {
                assert_render(&fixture(concat!(stringify!($family), "/basic.mmd")), $basic);
            }
            #[test]
            fn styled() {
                let source = fixture(concat!(stringify!($family), "/styled.mmd"));
                let svg = assert_render(&source, $styled);
                assert_ne!(
                    svg,
                    render(
                        DiagramKind::Mermaid,
                        &source.lines().skip(1).collect::<Vec<_>>().join(
                            "
"
                        )
                    )
                    .unwrap()
                    .pages[0]
                        .svg
                );
            }
            #[test]
            fn invalid_directive() {
                let error = render(
                    DiagramKind::Mermaid,
                    &fixture(concat!(stringify!($family), "/invalid.mmd")),
                )
                .unwrap_err();
                assert!(error.to_ascii_lowercase().contains("directive"), "{error}");
            }
        }
    };
}
include!("corpus_tests.rs");

#[test]
fn focused_upstream_cases_preserve_finite_visible_geometry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut count = 0;
    for directory in std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .filter(|p| p.path().is_dir())
    {
        for entry in std::fs::read_dir(directory.path()).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "mmd")
                && !matches!(
                    path.file_stem().unwrap().to_str().unwrap(),
                    "basic" | "styled" | "invalid"
                )
            {
                assert_render(&std::fs::read_to_string(&path).unwrap(), "");
                count += 1;
            }
        }
    }
    assert!(count >= 20);
}
#[test]
fn aliases_frontmatter_bom_and_compact_syntax_are_detected() {
    for header in [
        "graph LR;A-->B",
        "flowchart TB;A-->B",
        "block-beta",
        "block",
        "packet-beta",
        "packet",
        "sankey-beta",
        "sankey",
        "treemap-beta",
        "treemap",
        "xychart",
        "xychart-beta",
        "architecture-beta",
        "radar-beta",
        "C4Context",
        "C4Container",
        "C4Component",
        "C4Dynamic",
        "C4Deployment",
    ] {
        for prefix in [
            "",
            "%% comment
",
            "﻿",
            "---
title: Header
---
",
        ] {
            assert_eq!(
                DiagramKind::detect(&format!("{prefix}{header}")),
                Some(DiagramKind::Mermaid),
                "{prefix}{header}"
            );
        }
    }
    assert_render("flowchart LR;A[Compact]-->B", "Compact");
    assert_render(
        "﻿%% comment
flowchart LR;A[BOM]-->B",
        "BOM",
    );
}
#[test]
fn removed_formats_and_explicit_code_languages_stay_code() {
    for path in [
        "x.d2",
        "x.dot",
        "x.gv",
        "x.graphviz",
        "x.puml",
        "x.uml",
        "x.plantuml",
        "x.wavejson",
        "x.wavedrom",
        "x.json",
        "x.json5",
    ] {
        assert_eq!(
            DiagramKind::for_file(Path::new(path), "flowchart LR;A-->B"),
            None
        );
    }
    for token in [
        "d2", "dot", "graphviz", "plantuml", "wavedrom", "json", "json5", "rust", "text",
    ] {
        assert_eq!(DiagramKind::for_fence(token, "flowchart LR;A-->B"), None);
    }
    for source in [
        "a -> b",
        "A-->B",
        "graph = value",
        "graph { a -> b }",
        "digraph { a -> b }",
        "@startuml
@enduml",
        "{signal:[{wave:'01'}]}",
        "sequenceDiagramVariable",
    ] {
        assert_eq!(DiagramKind::detect(source), None, "{source}");
    }
    for info in [
        "mermaid",
        "MERMAID title=x",
        "{.mermaid}",
        ".mmd",
        "language-mermaid",
    ] {
        assert_eq!(
            DiagramKind::for_fence(info, "broken"),
            Some(DiagramKind::Mermaid)
        );
    }
}
#[test]
fn supplied_mermaid_fixture_renders_labels_styles_and_reports_unsupported_curve() {
    let source = include_str!("../tests/fixtures/flowchart/supplied.mmd");
    let svg = assert_render(source, "emoji 😀");
    let svg = std::str::from_utf8(&svg).unwrap();
    for color in ["#eef2ff", "#dcfce7", "#fef3c7", "#ef4444"] {
        assert!(svg.contains(color), "{color}");
    }
    let output = render(DiagramKind::Mermaid, source).unwrap();
    assert_eq!(
        output.diagnostics,
        ["Mermaid setting 'flowchart.curve' is not supported by the native preview."]
    );
}
#[test]
fn source_config_changes_theme_spacing_and_supports_frontmatter() {
    let base = "flowchart LR
A[Configuration]-->B";
    let plain = render(DiagramKind::Mermaid, base).unwrap();
    let directive = format!(
        r##"%%{{init: {{"theme":"dark","themeVariables":{{"primaryColor":"#123456"}},"flowchart":{{"nodeSpacing":90,"rankSpacing":100}}}}}}%%
{base}"##
    );
    let configured = render(DiagramKind::Mermaid, &directive).unwrap();
    assert!(configured.diagnostics.is_empty());
    assert_ne!(plain.pages, configured.pages);
    assert!(
        std::str::from_utf8(&configured.pages[0].svg)
            .unwrap()
            .contains("#123456")
    );
    let yaml = format!(
        "---
config:
  theme: dark
  themeVariables:
    primaryColor: '#123456'
  flowchart:
    nodeSpacing: 90
    rankSpacing: 100
---
{base}"
    );
    assert_eq!(configured, render(DiagramKind::Mermaid, &yaml).unwrap());
}
#[test]
fn errors_are_explicit_and_do_not_poison_the_next_render() {
    for source in [
        "",
        "invalid diagram",
        "flowchart LR
subgraph x
A-->B",
        "flowchart LR
end",
        "flowchart LR
-->A",
        "sequenceDiagram
participant Alice
Alice->>Ghost: hello",
        "---
config: dark",
    ] {
        assert!(render(DiagramKind::Mermaid, source).is_err(), "{source}");
    }
    assert!(
        render(DiagramKind::Mermaid, &"a".repeat(MAX_SOURCE_BYTES + 1))
            .unwrap_err()
            .contains("limit")
    );
    assert_render("flowchart LR;A[Still works]-->B", "Still works");
}
#[test]
fn bounded_generated_graphs_render_deterministically() {
    for seed in 0..16 {
        let mut source = format!(
            "flowchart {}
",
            ["LR", "TB", "RL", "BT"][seed % 4]
        );
        for i in 0..3 + seed {
            source.push_str(&format!(
                "N{i}[Label {i} αβ]-->|edge {i}|N{}
",
                (i + 1) % (3 + seed)
            ));
        }
        let first = assert_render(&source, "αβ");
        assert_eq!(
            first,
            render(DiagramKind::Mermaid, &source).unwrap().pages[0].svg
        );
    }
}
#[test]
fn large_scenes_keep_natural_dimensions_and_tiles_keep_color() {
    let scene=PreparedScene::new(Arc::from(&br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100"><rect width="100000" height="100" fill="red"/></svg>"#[..])).unwrap();
    assert_eq!(scene.width(), 100000.0);
    assert_eq!(
        &scene.raster_tile(4.0, 10, 0).unwrap()[..4],
        &[255, 0, 0, 255]
    );
    assert!(scene.raster_region(f32::NAN, 0, 0, 10, 10).is_err());
    assert!(scene.raster_region(1.0, 0, 0, 4097, 1).is_err());
}
#[test]
fn tiles_match_whole_scene_at_boundaries_and_multiple_device_scales() {
    let svg = render(
        DiagramKind::Mermaid,
        "flowchart LR
A[Long first label]-->B[Long second label]-->C[Long third label]",
    )
    .unwrap()
    .pages
    .remove(0)
    .svg;
    let scene = PreparedScene::new(svg).unwrap();
    for scale in [0.5, 1.0, 1.25, 2.0, 4.0, 8.0] {
        let whole = scene.raster_region(scale, 0, 0, 1024, 512).unwrap();
        let mut max_difference = 0;
        let mut total_difference = 0u64;
        let mut seam_max = 0;
        for column in 0..2 {
            let tile = scene.raster_tile(scale, column, 0).unwrap();
            for row in 0..512 {
                let start = (row * 1024 * 4 + column * 512 * 4) as usize;
                let expected = &whole[start..start + 512 * 4];
                let actual = &tile[(row * 512 * 4) as usize..((row + 1) * 512 * 4) as usize];
                for (index, (a, b)) in expected.iter().zip(actual).enumerate() {
                    let difference = a.abs_diff(*b);
                    max_difference = max_difference.max(difference);
                    total_difference += difference as u64;
                    let x = column * 512 + index as u32 / 4;
                    if (508..516).contains(&x) {
                        seam_max = seam_max.max(difference);
                    }
                }
            }
        }
        // tiny-skia may round a few isolated antialias samples differently
        // when clipping at half scale. The shared tile boundary stays exact.
        assert_eq!(seam_max, 0, "tile seam at scale {scale}");
        assert!(
            max_difference <= 16 && total_difference <= 128,
            "scale {scale}: max={max_difference} sum={total_difference}"
        );
    }
}

#[test]
fn reviewed_family_png_goldens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for entry in std::fs::read_dir(root.join("goldens")).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "png") {
            continue;
        }
        let family = path.file_stem().unwrap().to_str().unwrap();
        let svg = render(
            DiagramKind::Mermaid,
            &fixture(&format!("{family}/basic.mmd")),
        )
        .unwrap()
        .pages
        .remove(0)
        .svg;
        let scene = PreparedScene::new(svg).unwrap();
        let (width, height) = (scene.width().ceil() as u32, scene.height().ceil() as u32);
        let actual = image::RgbaImage::from_raw(
            width,
            height,
            scene.raster_region(1.0, 0, 0, width, height).unwrap(),
        )
        .unwrap();
        let expected = image::open(&path).unwrap().into_rgba8();
        assert_eq!(
            actual.dimensions(),
            expected.dimensions(),
            "{family} dimensions"
        );
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) && actual != expected {
            let failed = root.join("../../../target/mermaid-golden-failures");
            std::fs::create_dir_all(&failed).unwrap();
            actual
                .save(failed.join(format!("{family}-actual.png")))
                .unwrap();
            expected
                .save(failed.join(format!("{family}-expected.png")))
                .unwrap();
            let mut diff = actual.clone();
            for ((out, a), b) in diff
                .pixels_mut()
                .zip(actual.pixels())
                .zip(expected.pixels())
            {
                *out = if a == b {
                    image::Rgba([255, 255, 255, 255])
                } else {
                    image::Rgba([255, 0, 0, 255])
                };
            }
            diff.save(failed.join(format!("{family}-diff.png")))
                .unwrap();
            panic!("{family} golden differs: {}", failed.display());
        }
    }
    assert_eq!(
        std::fs::read_dir(root.join("goldens"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
            .count(),
        23
    );
}

#[test]
fn all_native_aliases_render_complete_bodies() {
    for (family, header, label) in [
        ("flowchart", "graph LR", "Start"),
        ("state", "stateDiagram", "Idle"),
        ("block", "block-beta", "A"),
        ("packet", "packet-beta", "Type"),
        ("sankey", "sankey-beta", "A"),
        ("treemap", "treemap", "Root"),
        ("xychart", "xychart", "Q1"),
        ("c4", "C4Context", "Admin"),
        ("c4", "C4Container", "Admin"),
        ("c4", "C4Component", "Admin"),
        ("c4", "C4Dynamic", "Admin"),
        ("c4", "C4Deployment", "Admin"),
    ] {
        let source = fixture(&format!("{family}/basic.mmd"));
        let body = source.split_once('\n').unwrap().1;
        assert_render(&format!("{header}\n{body}"), label);
    }
}
#[test]
fn requirement_text_baselines_clear_borders_and_dividers() {
    let svg = render(DiagramKind::Mermaid, &fixture("requirement/basic.mmd"))
        .unwrap()
        .pages
        .remove(0)
        .svg;
    let doc = roxmltree::Document::parse(std::str::from_utf8(&svg).unwrap()).unwrap();
    let boxes = doc
        .descendants()
        .filter(|node| {
            node.tag_name().name() == "rect" && node.attribute("fill") == Some("#ECECFF")
        })
        .collect::<Vec<_>>();
    assert_eq!(boxes.len(), 2);
    for node in doc
        .descendants()
        .filter(|node| node.tag_name().name() == "text")
    {
        let label = node.text().unwrap_or_default();
        if !(label.contains("Requirement") || label.starts_with("ID:")) {
            continue;
        }
        let x = node.attribute("x").unwrap().parse::<f32>().unwrap();
        let y = node.attribute("y").unwrap().parse::<f32>().unwrap();
        let rect = boxes
            .iter()
            .find(|rect| {
                let rx = rect.attribute("x").unwrap().parse::<f32>().unwrap();
                let ry = rect.attribute("y").unwrap().parse::<f32>().unwrap();
                x >= rx
                    && y >= ry
                    && y <= ry + rect.attribute("height").unwrap().parse::<f32>().unwrap()
            })
            .unwrap();
        let ry = rect.attribute("y").unwrap().parse::<f32>().unwrap();
        assert!(y - ry >= 14.0);
        if label.starts_with("ID:") {
            assert!(y - ry >= 52.0 + 14.0);
        }
    }
}
#[test]
fn bundled_fonts_and_config_diagnostics_are_consistent() {
    let output = render(
        DiagramKind::Mermaid,
        r#"%%{init: {"themeVariables":{"fontFamily":"Missing Custom Font"},"unknown":1}}%%
flowchart LR;A[Latin αβ Ж 😀]-->B"#,
    )
    .unwrap();
    assert_eq!(output.diagnostics.len(), 2);
    let svg = std::str::from_utf8(&output.pages[0].svg).unwrap();
    assert!(svg.contains("IBM Plex Sans") && svg.contains("Noto Emoji"));
    let scene = PreparedScene::new(output.pages[0].svg.clone()).unwrap();
    assert!(scene.raster_tile(1.0, 0, 0).is_ok());
}

#[test]
fn bundled_fallback_renders_unicode_and_emoji_glyphs() {
    for glyph in ["A", "α", "Ж", "😀"] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="48"><text x="4" y="36" font-family="IBM Plex Sans, Noto Emoji" font-size="32">{glyph}</text></svg>"#
        );
        let scene = PreparedScene::new(svg.into_bytes().into()).unwrap();
        let pixels = scene.raster_region(1.0, 0, 0, 64, 48).unwrap();
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| **p != [255; 4])
                .count()
                > 40,
            "Missing glyph {glyph}"
        );
    }
}
