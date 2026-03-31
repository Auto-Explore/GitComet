use super::common::*;

pub(crate) fn bench_markdown_preview_parse_build(c: &mut Criterion) {
    {
        let medium_sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_MEDIUM_SECTIONS", 256);
        let large_sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_LARGE_SECTIONS", 768);
        let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_LINE_BYTES", 128);
        let medium = MarkdownPreviewFixture::new(medium_sections, line_bytes);
        let large = MarkdownPreviewFixture::new(large_sections, line_bytes);

        let mut group = c.benchmark_group("markdown_preview_parse_build");
        group.sample_size(10);
        group.warm_up_time(Duration::from_secs(1));

        for (label, fixture) in [("medium", &medium), ("large", &large)] {
            group.bench_function(BenchmarkId::new("single_document", label), |b| {
                b.iter(|| fixture.run_parse_single_step())
            });
            group.bench_function(BenchmarkId::new("two_sided_diff", label), |b| {
                b.iter(|| fixture.run_parse_diff_step())
            });
        }

        group.finish();

        let _ = measure_sidecar_allocations(|| medium.run_parse_single_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/single_document/medium");
        let _ = measure_sidecar_allocations(|| medium.run_parse_diff_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/two_sided_diff/medium");
        let _ = measure_sidecar_allocations(|| large.run_parse_single_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/single_document/large");
        let _ = measure_sidecar_allocations(|| large.run_parse_diff_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/two_sided_diff/large");
    }

    settle_markdown_allocator_pages();
}
