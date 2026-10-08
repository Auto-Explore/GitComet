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

CI fetches the pinned `fixtures/syntax_test` submodule and sets
`GITCOMET_REQUIRE_SYNTAX_CORPUS=1` and `GITCOMET_REQUIRE_GIT_LFS=1`, so missing
fixtures cannot silently pass. For equivalent local coverage, run
`git submodule update --init fixtures/syntax_test` and install Git LFS. Use a full
Git for Windows installation for HTTP profiling fixtures; minimal distributions
may omit `git http-backend`. The Cargo identity audit probes Python aliases and
accepts `GITCOMET_TEST_PYTHON` as an explicit interpreter path; the CI runner
supplies its own interpreter automatically.

Optional prerequisites remain visible as warnings and in
`target/ci-reports/runtime-exclusions.log`, including Windows symlink privileges,
git-annex (required in the Fedora lane), and cross-volume assertions. A successful
test count alone does not establish that those assertions ran. Syntax dump/pair
probes are manual ignored diagnostics, invoked with `--ignored` and their sample
environment variables.

LFS parameterized tests share an immutable remote within each test but clone
separate worktrees and object stores per scenario. Tool-help fixtures use Git's
`MERGE_TOOLS_DIR` override to exclude unrelated built-in tools while exercising
the real configured GitComet entries. Correctness tests use generous parse
budgets; the separate deadline tests retain production timeout coverage.
The optional watcher stress run also repeats real worktree tests to check
cancellation cleanup under load. Windows `taskkill` helpers disable handle
inheritance, so asynchronous shutdown does not retain the parent's output pipes.

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
