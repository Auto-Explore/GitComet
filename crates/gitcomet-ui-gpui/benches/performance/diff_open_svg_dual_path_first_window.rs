use super::common::*;

pub(crate) fn bench_diff_open_svg_dual_path_first_window(c: &mut Criterion) {
    let shapes = env_usize("GITCOMET_BENCH_SVG_SHAPES", 200);
    let fallback_bytes = env_usize("GITCOMET_BENCH_SVG_FALLBACK_BYTES", 64 * 1024);
    let window = env_usize("GITCOMET_BENCH_SVG_WINDOW", 200);
    let fixture = SvgDualPathFirstWindowFixture::new(shapes, fallback_bytes);
    let metrics = measure_sidecar_allocations(|| fixture.measure_first_window());

    let mut group = c.benchmark_group("diff_open_svg_dual_path_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_first_window_step(window)),
    );
    group.finish();
    emit_svg_dual_path_first_window_sidecar(window, &metrics);
}
