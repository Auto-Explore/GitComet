# Test-runtime measurements

The CI runner builds and inventories tests before executing nextest and the GPUI
libtest harness. Keep compilation separate from timing and use the same profile,
test inventory, concurrency and dependency versions for both revisions.

```sh
python scripts/ci/run.py compile --context workspace
python scripts/ci/runtime.py target/profiling/test-runtime --samples 5 --session first
```

`runtime.py` reuses compiled inventory, writes separate logs for each repetition,
and records failures instead of treating incomplete runs as successful samples.
Use `--checkout PATH` to measure another checkout with the current driver after
compiling that checkout's inventory. `--nextest-threads`, `--ui-threads` and
`--nextest-profile` select concurrency explicitly. Custom thread counts require
`--schedule serial`; `balanced` divides capacity between harnesses.
Disable fixture/trace instrumentation for comparable execution timings.

The existing workflows expose these controls for manual experiments. Normal CI
still runs one workspace sample. The Windows CMD smoke build targets only
`standalone_tool_mode_integration`, which contains the smoke cases; rebuilding
unrelated application tests there would add work without coverage.

Use `scripts/ci/report.py --help` for aggregation commands. Compare successful
repetitions within the same environment and report execution separately from
cache restore, compilation and total workflow time. Summed per-test durations
overlap when tests run concurrently and cannot be read as elapsed workflow time.

Application and native UI profiling tools are documented in
[scripts/profiling](../scripts/profiling/README.md). They are independent of CI
test acceptance and are not run by the test workflows.
