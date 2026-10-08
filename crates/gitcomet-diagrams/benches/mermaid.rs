use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use gitcomet_diagrams::{DiagramKind, render, scene::PreparedScene};
use std::{hint::black_box, sync::Arc, time::Duration};

fn graph(count: usize, dense: bool) -> String {
    let mut source = String::from("flowchart LR\n");
    for i in 0..count {
        source.push_str(&format!("N{i}[Repeated label]-->N{}\n", i + 1));
        if dense && i > 2 {
            source.push_str(&format!("N{}-.->N{i}\n", i - 3));
        }
    }
    source
}
fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("mermaid");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(1));
    for (name, count, dense) in [
        ("small", 4, false),
        ("typical", 50, false),
        ("dense", 100, true),
        ("large", 500, false),
    ] {
        let source = graph(count, dense);
        group.bench_function(BenchmarkId::new("parse_layout_svg", name), |b| {
            b.iter(|| black_box(render(DiagramKind::Mermaid, &source).unwrap()))
        });
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(&source).unwrap();
        let mut theme = mermaid_rs_renderer::Theme::modern();
        theme.font_family = "IBM Plex Sans, Noto Emoji".into();
        let config = mermaid_rs_renderer::LayoutConfig::default();
        group.bench_function(BenchmarkId::new("parse", name), |b| {
            b.iter(|| black_box(mermaid_rs_renderer::parse_mermaid_strict(&source).unwrap()))
        });
        group.bench_function(BenchmarkId::new("layout", name), |b| {
            b.iter(|| {
                black_box(mermaid_rs_renderer::compute_layout(
                    &parsed.graph,
                    &theme,
                    &config,
                ))
            })
        });
        let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &theme, &config);
        group.bench_function(BenchmarkId::new("svg", name), |b| {
            b.iter(|| {
                black_box(mermaid_rs_renderer::render::render_svg_with_dimensions(
                    &layout,
                    &theme,
                    &config,
                    Some((layout.width, layout.height)),
                ))
            })
        });
        let svg = render(DiagramKind::Mermaid, &source)
            .unwrap()
            .pages
            .remove(0)
            .svg;
        group.bench_function(BenchmarkId::new("prepare_scene", name), |b| {
            b.iter(|| black_box(PreparedScene::new(svg.clone()).unwrap()))
        });
        let scene = PreparedScene::new(svg).unwrap();
        group.bench_function(BenchmarkId::new("visible_tile_100_percent", name), |b| {
            b.iter(|| black_box(scene.raster_tile(1.0, 0, 0).unwrap()))
        });
        group.bench_function(
            BenchmarkId::new("visible_tile_400_percent_dpr2", name),
            |b| b.iter(|| black_box(scene.raster_tile(8.0, 1, 0).unwrap())),
        );
    }
    group.finish();
    let source: Arc<str> = graph(50, false).into();
    c.bench_function("mermaid/header_recognition", |b| {
        b.iter(|| black_box(DiagramKind::detect(&source)))
    });
}
criterion_group!(benches, bench);
criterion_main!(benches);
