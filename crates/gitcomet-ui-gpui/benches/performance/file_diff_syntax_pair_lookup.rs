use super::common::*;

pub(crate) fn bench_file_diff_syntax_pair_lookup(c: &mut Criterion) {
    let json_elements = env_usize("GITCOMET_BENCH_SYNTAX_PAIR_JSON_ELEMENTS", 250_000);
    let python_statements = env_usize("GITCOMET_BENCH_SYNTAX_PAIR_PYTHON_STATEMENTS", 100_000);
    let json_open = SyntaxPairLookupFixture::wide_json_array(json_elements, true);
    let json_middle = SyntaxPairLookupFixture::wide_json_array(json_elements, false);
    let python_end = SyntaxPairLookupFixture::wide_python_module(python_statements);

    let mut group = c.benchmark_group("file_diff_syntax_pair_lookup");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("wide_json_open"), |b| {
        b.iter(|| json_open.run_lookup())
    });
    group.bench_function(BenchmarkId::from_parameter("wide_json_middle"), |b| {
        b.iter(|| json_middle.run_lookup())
    });
    group.bench_function(BenchmarkId::from_parameter("wide_python_module_end"), |b| {
        b.iter(|| python_end.run_lookup())
    });
    group.finish();
}
