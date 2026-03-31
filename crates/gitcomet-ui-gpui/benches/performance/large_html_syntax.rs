use super::common::*;

pub(crate) fn bench_large_html_syntax(c: &mut Criterion) {
    let fixture_path = env_string("GITCOMET_BENCH_HTML_FIXTURE_PATH");
    let synthetic_lines = env_usize("GITCOMET_BENCH_HTML_LINES", 20_000);
    let synthetic_line_bytes = env_usize("GITCOMET_BENCH_HTML_LINE_BYTES", 192);
    let window_lines = env_usize("GITCOMET_BENCH_HTML_WINDOW_LINES", 160);
    let prepare_fixture = LargeHtmlSyntaxFixture::new(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let source_label = prepare_fixture.source_label().to_string();

    let mut group = c.benchmark_group("large_html_syntax");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::new(source_label.as_str(), "background_prepare"),
        |b| b.iter(|| prepare_fixture.run_background_prepare_step()),
    );
    let pending_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let mut pending_start_line = 0usize;
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_pending"),
        &window_lines,
        |b, &window_lines| {
            b.iter(|| {
                let hash = pending_fixture
                    .run_visible_window_pending_step(pending_start_line, window_lines);
                pending_start_line =
                    pending_fixture.next_start_line(pending_start_line, window_lines);
                hash
            })
        },
    );
    let visible_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    visible_fixture.prime_visible_window(window_lines);
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_steady"),
        &window_lines,
        |b, &window_lines| b.iter(|| visible_fixture.run_visible_window_step(0, window_lines)),
    );
    let mut start_line = 0usize;
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_sweep"),
        &window_lines,
        |b, &window_lines| {
            b.iter(|| {
                let hash = visible_fixture.run_visible_window_step(start_line, window_lines);
                start_line = visible_fixture.next_start_line(start_line, window_lines);
                hash
            })
        },
    );
    group.finish();

    let (_, prepare_metrics) = measure_sidecar_allocations(|| {
        LargeHtmlSyntaxFixture::new(
            fixture_path.as_deref(),
            synthetic_lines,
            synthetic_line_bytes,
        )
        .run_background_prepare_with_metrics()
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/background_prepare"),
        prepare_metrics,
    );

    let pending_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let (_, pending_metrics) = measure_sidecar_allocations(|| {
        pending_metrics_fixture.run_visible_window_pending_with_metrics(0, window_lines)
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/visible_window_pending"),
        pending_metrics,
    );

    let steady_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    steady_metrics_fixture.prime_visible_window_until_ready(window_lines);
    let (_, steady_metrics) = measure_sidecar_allocations(|| {
        steady_metrics_fixture.run_visible_window_with_metrics(0, window_lines)
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/visible_window_steady"),
        steady_metrics,
    );

    let sweep_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    sweep_metrics_fixture.prime_visible_window_until_ready(window_lines);
    let sweep_start_line = sweep_metrics_fixture.next_start_line(0, window_lines);
    let (_, sweep_metrics) = measure_sidecar_allocations(|| {
        sweep_metrics_fixture.run_visible_window_with_metrics(sweep_start_line, window_lines)
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/visible_window_sweep"),
        sweep_metrics,
    );
}
