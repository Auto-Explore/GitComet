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

// Register alongside the original conflict benchmark so both use the existing
// allocator sidecars and production profile.
pub(crate) fn bench_bounded_live_syntax_edits(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_live_syntax_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    for language in ["rust", "markdown", "html"] {
        for lines in [64, 20_000] {
            let case = format!("{language}_{lines}");
            for phase in ["foreground", "recovery"] {
                group.bench_function(format!("{phase}_{case}"), |b| {
                    let mut fixture = LiveSyntaxEditTraceFixture::new(language, lines);
                    b.iter_custom(|iterations| {
                        let mut elapsed = Duration::ZERO;
                        for _ in 0..iterations {
                            if phase == "foreground" {
                                let started = Instant::now();
                                std::hint::black_box(fixture.edit());
                                elapsed += started.elapsed();
                            } else {
                                fixture.defer_edit();
                                let started = Instant::now();
                                assert!(fixture.recover());
                                elapsed += started.elapsed();
                            }
                        }
                        fixture.recover();
                        fixture.verify();
                        elapsed
                    });
                });
            }
            let (mut fixture, retained) =
                gitcomet_ui_gpui::perf_alloc::measure_allocation_channels(|| {
                    LiveSyntaxEditTraceFixture::new(language, lines)
                });
            let mut outcomes = [0usize; 3];
            let mut jobs = 0;
            let mut foreground = Duration::ZERO;
            let mut recovery = Duration::ZERO;
            measure_sidecar_allocations(|| {
                for _ in 0..8 {
                    // Four rapid edits share a single recovery computation.
                    for _ in 0..4 {
                        let started = Instant::now();
                        outcomes[fixture.edit() as usize] += 1;
                        foreground += started.elapsed();
                    }
                    let started = Instant::now();
                    jobs += usize::from(fixture.recover());
                    recovery += started.elapsed();
                }
            });
            fixture.verify();
            let mut payload = serde_json::Map::new();
            payload.insert("language".into(), serde_json::json!(language));
            payload.insert("lines".into(), serde_json::json!(lines));
            payload.insert(
                "retained_rust_bytes".into(),
                serde_json::json!(retained.rust.net_alloc_bytes),
            );
            payload.insert(
                "retained_tree_sitter_bytes".into(),
                serde_json::json!(retained.tree_sitter.net_alloc_bytes),
            );
            payload.insert("reparsed".into(), serde_json::json!(outcomes[0]));
            payload.insert("deferred".into(), serde_json::json!(outcomes[1]));
            payload.insert("abandoned".into(), serde_json::json!(outcomes[2]));
            payload.insert("background_computations".into(), serde_json::json!(jobs));
            payload.insert(
                "foreground_ns".into(),
                serde_json::json!(foreground.as_nanos()),
            );
            payload.insert("recovery_ns".into(), serde_json::json!(recovery.as_nanos()));
            payload.insert("text_bytes".into(), serde_json::json!(fixture.text_bytes()));
            emit_sidecar_metrics(
                &format!("bounded_live_syntax_edits/foreground_{case}"),
                payload,
            );
        }
    }
    group.finish();
}
