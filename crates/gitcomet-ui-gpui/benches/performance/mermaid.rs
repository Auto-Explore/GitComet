use super::common::*;
use gitcomet_ui_gpui::benchmarks::MermaidPreviewFixture;
pub(crate) fn bench_mermaid_preview(c: &mut Criterion) {
    for (name, nodes) in [("small", 4), ("typical", 50), ("large", 500)] {
        let fixture =
            MermaidPreviewFixture::new(nodes, gitcomet_ui_kit::theme::AppTheme::gitcomet_dark());
        let mut group = c.benchmark_group("mermaid_preview");
        group.sample_size(10);
        group.bench_function(BenchmarkId::new("cold_prepare", name), |b| {
            b.iter(|| fixture.cold_prepare())
        });
        group.bench_function(BenchmarkId::new("scene_cache_hit", name), |b| {
            b.iter(|| fixture.cache_hit())
        });
        let mut step = 0;
        group.bench_function(BenchmarkId::new("pan_zoom_tiles", name), |b| {
            b.iter(|| {
                step += 1;
                fixture.pan_zoom_tiles(step)
            })
        });
        fixture.preview_frame();
        group.bench_function(BenchmarkId::new("preview_frame", name), |b| {
            b.iter(|| fixture.preview_frame())
        });
        group.bench_function(BenchmarkId::new("concurrent_identical", name), |b| {
            b.iter(|| fixture.concurrent_identical())
        });
        group.finish();
        measure_sidecar_allocations(|| fixture.cache_hit());
        emit_allocation_only_sidecar(&format!("mermaid_preview/scene_cache_hit/{name}"));
        measure_sidecar_allocations(|| fixture.pan_zoom_tiles(3));
        emit_allocation_only_sidecar(&format!("mermaid_preview/pan_zoom_tiles/{name}"));
    }
    let fixture = MermaidPreviewFixture::new(4, gitcomet_ui_kit::theme::AppTheme::gitcomet_dark());
    c.bench_function("mermaid_preview/many_markdown_diagrams", |b| {
        b.iter(|| fixture.many_markdown_diagrams())
    });
}
