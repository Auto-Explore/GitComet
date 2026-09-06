use super::common::*;

/// The action-bar badge pickers. Three costs, measured apart because they change
/// apart:
///
/// * `rows_build` — building and filtering the row model. Once per change to the
///   repository, since `popover::rows_cache` memoises it.
/// * `hover_frame` — one frame of the picker as it ships. This is the cost a
///   hover moving from row to row pays, because the popover host is uncached and
///   a hover transition notifies it.
/// * `full_list_frame` — the same frame with an unbounded viewport, which builds
///   elements for every matched row. This is what a frame cost before the list
///   was windowed; the gap to `hover_frame` is what windowing saves.
pub(crate) fn bench_picker_prompt(c: &mut Criterion) {
    let scales = [
        env_usize("GITCOMET_BENCH_PICKER_REFS_SMALL", 20),
        env_usize("GITCOMET_BENCH_PICKER_REFS_TYPICAL", 200),
        env_usize("GITCOMET_BENCH_PICKER_REFS_LARGE", 1_200),
    ];
    let worktrees = env_usize("GITCOMET_BENCH_PICKER_WORKTREES", 8);

    let mut group = c.benchmark_group("picker_prompt");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    for refs in scales {
        group.bench_with_input(
            BenchmarkId::new("branch_rows_build", refs),
            &refs,
            |b, &refs| {
                let mut fixture =
                    PickerPromptFrameFixture::new(PickerPromptKind::BranchCheckout, refs, 0);
                b.iter(|| fixture.run_rows_build())
            },
        );
        group.bench_with_input(
            BenchmarkId::new("branch_hover_frame", refs),
            &refs,
            |b, &refs| {
                let mut fixture =
                    PickerPromptFrameFixture::new(PickerPromptKind::BranchCheckout, refs, 0);
                b.iter(|| fixture.run_frame())
            },
        );
        group.bench_with_input(
            BenchmarkId::new("branch_full_list_frame", refs),
            &refs,
            |b, &refs| {
                let mut fixture =
                    PickerPromptFrameFixture::new(PickerPromptKind::BranchCheckout, refs, 0);
                b.iter(|| fixture.run_frame_full_list())
            },
        );
        group.bench_with_input(
            BenchmarkId::new("branch_query_frame", refs),
            &refs,
            |b, &refs| {
                let mut fixture =
                    PickerPromptFrameFixture::new(PickerPromptKind::BranchCheckout, refs, 0)
                        .with_query("topic");
                b.iter(|| fixture.run_frame())
            },
        );
    }

    group.bench_with_input(
        BenchmarkId::new("workspace_hover_frame", worktrees),
        &worktrees,
        |b, &worktrees| {
            let mut fixture =
                PickerPromptFrameFixture::new(PickerPromptKind::Workspace, 32, worktrees);
            b.iter(|| fixture.run_frame())
        },
    );

    for commits in [200, 1_200, 5_000] {
        group.bench_with_input(
            BenchmarkId::new("file_history_rows_build", commits),
            &commits,
            |b, &commits| {
                let mut fixture =
                    PickerPromptFrameFixture::new(PickerPromptKind::FileHistory, commits, 0);
                b.iter(|| fixture.run_rows_build());
            },
        );
        group.bench_with_input(
            BenchmarkId::new("file_history_hover_frame", commits),
            &commits,
            |b, &commits| {
                let mut fixture =
                    PickerPromptFrameFixture::new(PickerPromptKind::FileHistory, commits, 0);
                fixture.run_frame();
                b.iter(|| fixture.run_frame());
            },
        );
    }
    group.bench_function("file_history_query_frame/1200", |b| {
        let mut fixture = PickerPromptFrameFixture::new(PickerPromptKind::FileHistory, 1_200, 0);
        fixture.run_frame();
        b.iter(|| fixture.run_file_history_query_frame());
    });

    group.finish();

    // Structural sidecars: element and tooltip counts per frame are what the
    // budgets guard, because they hold regardless of build profile or machine.
    for refs in scales {
        let mut fixture = PickerPromptFrameFixture::new(PickerPromptKind::BranchCheckout, refs, 0);
        let (_hash, metrics) = measure_sidecar_allocations(|| fixture.run_frame_with_metrics());
        emit_picker_prompt_sidecar(&format!("branch_hover_frame/{refs}"), &metrics);

        let mut fixture = PickerPromptFrameFixture::new(PickerPromptKind::BranchCheckout, refs, 0);
        let (_hash, metrics) =
            measure_sidecar_allocations(|| fixture.run_rows_build_with_metrics());
        emit_picker_prompt_sidecar(&format!("branch_rows_build/{refs}"), &metrics);
    }

    let mut fixture = PickerPromptFrameFixture::new(PickerPromptKind::Workspace, 32, worktrees);
    let (_hash, metrics) = measure_sidecar_allocations(|| fixture.run_frame_with_metrics());
    emit_picker_prompt_sidecar(&format!("workspace_hover_frame/{worktrees}"), &metrics);
    for commits in [200, 1_200, 5_000] {
        let mut fixture = PickerPromptFrameFixture::new(PickerPromptKind::FileHistory, commits, 0);
        fixture.run_frame();
        let (_, metrics) = measure_sidecar_allocations(|| fixture.run_frame_with_metrics());
        emit_picker_prompt_sidecar(&format!("file_history_hover_frame/{commits}"), &metrics);
        let (_, metrics) = measure_sidecar_allocations(|| fixture.run_rows_build_with_metrics());
        emit_picker_prompt_sidecar(&format!("file_history_rows_build/{commits}"), &metrics);
    }
}
