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
}
