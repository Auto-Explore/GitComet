use super::common::*;

pub(crate) fn bench_diff_open_image_preview_first_paint(c: &mut Criterion) {
    let old_bytes = env_usize("GITCOMET_BENCH_IMAGE_PREVIEW_OLD_BYTES", 256 * 1024);
    let new_bytes = env_usize("GITCOMET_BENCH_IMAGE_PREVIEW_NEW_BYTES", 384 * 1024);
    let fixture = ImagePreviewFirstPaintFixture::new(old_bytes, new_bytes);
    let metrics = measure_sidecar_allocations(|| fixture.measure_first_paint());

    c.bench_function("diff_open_image_preview_first_paint", |b| {
        b.iter(|| fixture.run_first_paint_step())
    });
    emit_image_preview_first_paint_sidecar(&metrics);

    let inspection = gitcomet_ui_gpui::benchmarks::ImageInspectionFixture::default();
    for (name, scale, filter) in [
        ("fit_spline36", 0.25, "spline36"),
        ("125_spline36", 1.25, "spline36"),
        ("125_robidoux", 1.25, "robidoux"),
        ("125_hann", 1.25, "hann"),
        ("400_nearest", 4.0, "spline36"),
    ] {
        c.bench_function(
            &format!("diff_open_image_preview_first_paint/{name}"),
            |b| b.iter(|| inspection.resample_visible_tile(scale, filter)),
        );
    }
    inspection.cached_pan();
    c.bench_function("diff_open_image_preview_first_paint/cached_pan", |b| {
        b.iter(|| inspection.cached_pan())
    });
}
