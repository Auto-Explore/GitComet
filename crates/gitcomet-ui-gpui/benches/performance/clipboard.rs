use super::common::*;

pub(crate) fn bench_clipboard(c: &mut Criterion) {
    let copy_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_COPY_LINES", 10_000);
    let paste_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_PASTE_LINES", 2_000);
    let paste_line_bytes = env_usize("GITCOMET_BENCH_CLIPBOARD_PASTE_LINE_BYTES", 96);
    let select_total_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_SELECT_TOTAL_LINES", 10_000);
    let select_range_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_SELECT_RANGE_LINES", 5_000);

    let copy_fixture = ClipboardFixture::copy_from_diff(copy_lines);
    let paste_fixture = ClipboardFixture::paste_into_commit_message(paste_lines, paste_line_bytes);
    let select_fixture =
        ClipboardFixture::select_range_in_diff(select_total_lines, select_range_lines);

    let mut group = c.benchmark_group("clipboard");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("copy_10k_lines_from_diff"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = copy_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) = measure_sidecar_allocations(|| copy_fixture.run_with_metrics());
                emit_clipboard_sidecar("copy_10k_lines_from_diff", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("paste_large_text_into_commit_message"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = paste_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) = measure_sidecar_allocations(|| paste_fixture.run_with_metrics());
                emit_clipboard_sidecar("paste_large_text_into_commit_message", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("select_range_5k_lines_in_diff"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = select_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) =
                    measure_sidecar_allocations(|| select_fixture.run_with_metrics());
                emit_clipboard_sidecar("select_range_5k_lines_in_diff", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}
