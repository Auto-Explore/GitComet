use super::common::*;

pub(crate) fn bench_conflict_resolved_output_live_syntax(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_LIVE_SYNTAX_LINES", 20_000);
    let conflicts = env_usize("GITCOMET_BENCH_LIVE_SYNTAX_CONFLICTS", 8);
    let window = env_usize("GITCOMET_BENCH_LIVE_SYNTAX_WINDOW", 60);

    let mut group = c.benchmark_group("conflict_resolved_output_live_syntax");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    // The headline number. Typing in the resolved output used to invalidate the
    // whole prepared document; this is `tree.edit` plus an incremental reparse.
    // It should stay well under the 1ms foreground budget at this size.
    group.bench_with_input(
        BenchmarkId::new("keystroke_reparse", lines),
        &lines,
        |b, _| {
            let mut fixture = ConflictResolvedOutputLiveSyntaxFixture::new(lines, conflicts);
            b.iter(|| fixture.run_keystroke_step())
        },
    );

    let fixture = ConflictResolvedOutputLiveSyntaxFixture::new(lines, conflicts);
    group.bench_with_input(
        BenchmarkId::new("visible_window_resolve", window),
        &window,
        |b, &w| b.iter(|| fixture.run_visible_window_resolve(lines / 2, w)),
    );
    group.bench_with_input(BenchmarkId::new("cold_parse", lines), &lines, |b, _| {
        b.iter(|| fixture.run_cold_parse())
    });
    group.finish();

    let mut alloc_fixture = ConflictResolvedOutputLiveSyntaxFixture::new(lines, conflicts);
    let _ = measure_sidecar_allocations(|| alloc_fixture.run_keystroke_step());
    emit_allocation_only_sidecar(&format!(
        "conflict_resolved_output_live_syntax/keystroke_reparse/{lines}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_visible_window_resolve(lines / 2, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_resolved_output_live_syntax/visible_window_resolve/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_cold_parse());
    emit_allocation_only_sidecar(&format!(
        "conflict_resolved_output_live_syntax/cold_parse/{lines}"
    ));
}
