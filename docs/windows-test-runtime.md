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
compiling that checkout's inventory. `--nextest-threads`, `--ui-threads`,
`--pure-threads` and `--nextest-profile` select concurrency explicitly.
`balanced` runs nextest, rendered UI, and audited pure batches concurrently when
capacity permits. Their thread budgets sum to available CPUs, accounting for
affinity and visible Linux cgroup quotas. Explicit balanced budgets must leave
at least one slot per remaining family. Its default UI budget is capped at 16.
On smaller hosts the runner falls back to serial execution.
Disable fixture/trace instrumentation for comparable execution timings.

`--batch-pure-tests auto` retains Windows batching; `on` enables it on any host,
and `off` retains the complete GPUI harness and nextest partition. The audited
allowlist covers core conflict sessions, text search and encoding, plus UI word
diff and history walking. Each GPUI package's rendered tests still run in one
harness process. Every executed partition is checked against exact inventory
names; duplicate results and ambiguous prefix matches fail coverage checks.
Libtest stdout and background stderr are captured separately, then retained in
the complete log. Name checks use libtest's final successful-test list so a
diagnostic or a captured output string cannot corrupt result accounting.

`ci-throughput` inherits the existing four-slot Git-heavy group and starts the
six longest watcher tests first. All CI timeouts, leak checks, retries, ignored
tests and prerequisite requirements remain inherited from `ci`. Normal CI keeps
the serial schedule and `ci` profile until repeated platform measurements pass
the promotion gates below. Manual workflow inputs expose all four profiles and
the batching and thread controls.

```sh
python scripts/ci/runtime.py target/profiling/throughput --samples 3 --session first \
  --schedule balanced --nextest-profile ci-throughput --batch-pure-tests on
python scripts/ci/runtime.py target/profiling/resources --samples 1 \
  --schedule balanced --nextest-profile ci-throughput --batch-pure-tests on --resource-stats
```

Each execution receives its own nextest configuration and store directory,
including concurrent executions and repetitions. The copied configuration
changes only the store location. Reports record test-inventory and executable
SHA-256 fingerprints, requested/effective thread budgets, and raw results.

`--resource-stats` adds optional process-tree sampling: observed peak summed RSS,
native thread counts and observed CPU seconds. Linux uses procfs and Windows
uses native process APIs; macOS uses `ps`, with a capability probe for thread
enumeration because some releases restrict it. Missing counters are `null`.
Sampling can miss short-lived children
and peaks, CPU is a sampled lower bound, and summed RSS includes shared pages
more than once. These diagnostic runs are marked instrumented and never qualify
as timing acceptance samples. Compare memory on the same platform and collector
version, with separate uninstrumented runs for wall time.

Require six successful local samples across two independent sessions and five
successful hosted samples across two jobs per native lane. Preserve complete
coverage, including required syntax corpus and LFS tests. Before promoting the
throughput profile, require at least 10% lower median execution time, no more
than 5% p95 regression, no peak-RSS regression and zero failures. Promote a UI
thread cap separately only with at least 10% peak-RSS reduction and no more than
3% median-time regression. Do not combine distinct binaries, inventories,
hardware, profiles, schedules, thread budgets or instrumented runs into one
acceptance group. Native Windows/macOS and hosted repetitions must run on those
platforms; Linux evidence cannot substitute for them.

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

Test-support stores expose an ordering barrier that acknowledges queued
reductions and their publication sequence without modifying preferences. It
does not await effects; tests wait for the relevant revision/loadable afterwards.
Shutdown acknowledges that store's worker, private pools and tracked executor
tasks, while other stores keep using the shared pools. Staging scenarios own a
fresh child app and repository copied from immutable seed bytes, then close the
app and await shutdown before removing files. Large conflict fixtures own their
temporary directories and share text buffers through `Arc<str>`.

Syntax tests give prepared documents a test-owned scope/generation. Cache reset
and seed eviction affect that scope, stale background results cannot restore
retired seeds, and deferred-drop counters follow each payload's receipt even
after a counter reset. Panic tests retain their catch assertions without
changing the process-wide panic hook. Deterministic pane tests pump to completed
state; native watcher quiet windows and actual deadline coverage remain intact.

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

Local validation on 2026-10-08 used native Linux, Rust 1.98.1, and the existing
`test` Cargo profile. Serial and balanced execution each passed all 8,841 active
tests from the 8,891-name inventory, with required syntax corpus and Git LFS.
Their coverage comparison found no missing or newly ignored tests. Comparing
the previous 4,584 UI names also found none removed or newly ignored; the new
inventory adds three syntax isolation regressions. Twenty shuffled syntax
seeds at 48 threads passed 476 tests each, and twenty shuffled visual seeds at
16 threads passed 75 affected staging, marker, pane and layout tests each.
The 117 Python checks, state/UI clippy checks, doctests, formatting, dependency
boundaries and identity audit passed (one Windows-only Python check was skipped
on Linux).

The uninstrumented balanced validation took 119.237 seconds with UI/nextest/pure
budgets of 16/8/8. A separate diagnostic passed with no collector errors and
observed peak summed RSS of 2,653,065,216 bytes. That diagnostic overlapped visual
stress and is not timing acceptance evidence. These are validation samples,
not the repeated local/hosted measurements required for promotion. Native
Windows/macOS verification and hosted repetitions remain pending, so the normal
CI schedule and profile remain serial and `ci`.
