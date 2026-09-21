# Windows test execution

## Follow-up after merge to dev (2026-09-21)

The successful [dev run at 5aa600da](https://github.com/Auto-Explore/GitComet/actions/runs/35517925640)
measured the following workspace execution times. These are single observations,
not comparable before/after medians or end-to-end workflow speedups:

| Platform | nextest | Main UI libtest | Workspace execution |
| --- | ---: | ---: | ---: |
| Windows x64 | 480.200 s | 170.012 s | 650.497 s |
| Windows ARM64 | 695.558 s | 142.872 s | 838.626 s |
| Ubuntu 22 x64 | 151.845 s | 148.688 s | 300.706 s |
| macOS 15 Intel | 369.328 s | 206.556 s | 576.279 s |

Windows x64 nextest cases sum to 1,915.499 test-seconds: almost four times its
elapsed duration. Slots are occupied by sleeps and I/O as well as CPU work.
Watcher tests account for 717.244 of those overlapping seconds, and the six real
Git tool-help cases account for 139.710. Test ordering alone has little headroom
at four slots. The ARM64 UI suite is slightly faster than Linux x64 in this run;
there is no evidence here of a uniform application CPU slowdown on Windows.

The follow-up implementation:

- Removes five Git config launches from fresh GitComet/kdiff3 mergetool fixtures
  and seven from meld fixtures. Fresh difftool configurations save four. Config
  replacement helpers and operations being asserted still go through Git.
- Imports base/theirs history in ordinary mergetool conflicts and base/ours in
  three standalone setup scenarios, saving four launches per applicable fixture.
  Real checkout, branch commit and merge still construct the conflicted index;
  Unicode paths, local/global setup, hooks and tool-help checks remain exercised.
- Uses registration readiness for three more tests whose first positive native
  event has a unique path. Settling before independent external-policy checks,
  repeated-path edits and negative quiet windows remain intact.
- Checks UI readiness immediately after pumping the executor. Optional
  `ui-lock-wait` and `ui-lock-held` records identify serialized visual/clipboard
  work without removing the locks or splitting the UI harness into processes.
- Adds `ui-threads` to the workspace workflow and `--ui-threads` to the runner
  and repetition harness. The top-level CI dispatch exposes workspace repetition,
  nextest profile/thread and UI thread controls. Requested and effective thread
  counts remain separate report dimensions, with nearest-rank p95 alongside the
  median. Measurements reject thread environment overrides; use explicit flags.

Normal PR concurrency, profiles, coverage, runner hardware and cache policy are
unchanged. Screen nextest 6, 8 and 8 with `ci-git-limited` independently on Windows
x64/ARM64 and Intel macOS. Confirm the fastest passing candidate and the default
with three repetitions in each of two independent hosted jobs. Compare UI 4/8
separately at a fixed nextest policy. Promote per platform only for at least 10%
median improvement, at most 5% p95 regression, unchanged coverage, and passing
watcher/UI stress. Final Windows x64 acceptance requires both at least 30% lower
median than the fresh merged baseline and less than 480 seconds. No native
Windows speedup is certified by local Linux verification.

### Application attribution

`application-probe: true` in CI or Cross-Platform Tests opts into disposable
plain, LFS and submodule probes after the normal checks. It compares raw Git,
backend calls and attached operation contexts for identical remote mutations and
full status reads. It builds `ci-test` and release separately, then runs 35
retained samples per mode after five warmups with rotated/reversed mode order.
Raw Git status disables optional index writes so it cannot refresh the fixture's
index stat cache on behalf of subsequent backend measurements. Both implementations
must report the expected dirty paths before timing, including the LFS asset or
submodule. The probe process itself also starts with empty global configuration
so in-process gix and Git subprocesses are isolated consistently. External
tracing and worker overrides are rejected; keep
`fixture-timings: false` for this probe.

```sh
python3 scripts/ci/application-probe.py
# Short local smoke verification; not performance acceptance:
python3 scripts/ci/application-probe.py --profiles ci-test --samples 5
```

Each invocation requires a new `target/ci-reports/application-probe` directory;
archive or move the prior directory before repeating it. Its artifacts include
the environment/revision, raw samples, median/p95, and separate latency and
diagnostic reports. Logs and a failed environment record remain if a probe fails.
The workflow publishes these with its other CI diagnostics.

Diagnostic captures partition wrapper time into preparation, spawn, worker
startup, child waiting, output drain, worker joins, activity completion, Trace2
completion and final processing. Capture is explicit and thread-local; attach
inside the worker when measuring asynchronous application work. Command counts
cover GitComet's wrappers, not nested Git or gix's Git LFS filter processes.
Diagnostic capture is compiled out of normal application builds, and its
instrumented latencies must not be used for acceptance.

The merged Trace2 wakeup and full-refresh status batching already exist; neither
is counted as a new improvement. Keep the remaining Windows process-wait change
gated on native evidence: at least 1 ms of avoidable completion overhead, then
at least 10% lower representative operation median and no more than 5% p95
regression with cancellation, deadlines, inherited pipes and output intact.
This follow-up adds attribution without introducing an unmeasured production
wait implementation or a persistent status cache.

## Baseline and attribution

The supplied Windows x64 and Ubuntu 22 x64 logs run the same revision. These
figures exclude compilation and cache handling:

| Measurement | Windows | Linux |
| --- | ---: | ---: |
| Workspace execution step | 704.569 s | 189.409 s |
| nextest | 536.107 s | 99.358 s |
| Main UI libtest suite | 164.79 s | 89.51 s |
| nextest tests passed | 2,767 | 2,850 |

Platform-specific inventories differ; compare identities within each platform,
not counts across operating systems. Both nextest runs already sustain roughly
four concurrent tests. “Serial” schedules nextest before UI; it does not mean
one test at a time. Neither log contains retries, failures, or timeouts.

Summed test durations identify three concentrations (overlapping execution means
these are not wall-clock totals): selective watchers 750.082 s / 270.209 s,
CLI 550.453 s / 33.819 s, and git-gix 750.225 s / 64.473 s, Windows / Linux.
Matched CLI integration cases often take about 18 times longer on Windows.
Git process creation and fixture filesystem work are strong suspects, but the
logs cannot isolate antivirus, storage, process launch, and backend computation.

There are also known test costs: the directory-recreation watcher test prescribed
57 seconds of quiet/settling waits on Windows, and the missing-ancestor history
fixture unpacked approximately 1,800 objects to remove one commit. Six tool-help
tests spent 145.447 aggregate seconds on Windows, including Git's tool discovery.
Those real compatibility tests remain enabled.

GitComet itself contributed unnecessary subprocesses: isolated Git Trace2 captures
showed setup launching 58 Git processes and uninstall after setup launching 86.
The command-local configuration snapshot reduces those to 29 and 29. Repeated
setup now uses 1 (down from 19 after the initial snapshot change); repeated
uninstall uses 1. Setup skips an entry only when its single value matches exactly.
Regression coverage checks both local
and isolated global scopes. Git still performs writes and locking. Snapshots
last only for one command; this does not introduce a persistent config cache or
make the entire sequence transactional against concurrent config edits.

## Implemented changes

- Read the required setup/uninstall configuration keys in one scoped Git call.
  Preserve ordered multiple values, multiline/empty values, backups, user edits,
  dry runs, missing configurations, and write failures. Skip unchanged setup
  writes while retaining Git's error on duplicate values and repairing missing
  or changed entries.
- Wake the activity-tracing worker on completion and drop instead of waiting
  for its 20 ms polling sleep. Final hook events and output are still drained.
- Write ordinary fresh-fixture identity configuration directly in the log, refs,
  upstream, remote-management, and watcher helpers. Remote-management setup
  removes five Git calls per fresh repository; clones keep Git's replacement
  semantics. Keep Git for later configuration mutations and for replacing init's
  existing `core.fileMode` key.
- Share a deterministic fast-import helper for ordinary histories. Log fixtures
  retain their authors, timestamps, messages, and topology; refs fixtures retain
  their base content. The missing-ancestor test keeps 600 commits and packs all
  retained objects instead of expanding them into loose files.
  Status conflict fixtures import their first two ordinary commits, then use real
  checkout, commit and merge commands, saving four processes per fixture. Submodule
  seed fixtures use the importer and reset instead of a separate synced seed-file
  write, add and commit; this changes fixture I/O, not the process count.
  Imported paths support spaces, UTF-8 and Git's quoted-path escaping.
- Replace selected independent positive watcher waits with native path
  observations followed by delivery barriers. Fresh-path startup requires
  registration readiness. Same-path edits, stale callback assertions, negative
  quiet checks, recovery, and deadlines retain their existing safeguards.
  In particular, the 57-second directory-recreation guard sequence is retained:
  removing it safely needs a stronger native same-path event boundary.
- Add opt-in watcher wait and cleanup timings to the existing fixture TSV stream.
  Extend fixture subprocess coverage and report nested Git Trace2 processes.
- Replace six fixed 120 ms UI waits with observable tab, splash and tooltip
  readiness. Animation waits and negative tooltip checks retain their deadlines.
  Optional `ui-wait` timings identify remaining waits by test and operation.
- Add explicit nextest concurrency and repeated execution measurements. Normal
  CI remains serial with nextest's default thread count and the full inventory.
  The optional `ci-git-limited` profile inherits CI checks and limits backend/CLI
  tests to four concurrent cases while leaving watcher slots unrestricted.

## Native acceptance experiment

Use the existing Cross-Platform Tests workflow. `execution-samples: 5` repeats
execution after compiling once; `fixture-timings: false` is required for these
measurements. Repeated jobs have a longer timeout; ordinary CI retains 60 minutes.
Run baseline and candidate on at least two independent hosted jobs per Windows
architecture. Keep runner image, Rust/Git versions, features, profile and
hardware comparable. Record image changes rather than attributing them to code.

Evaluate these policies separately:

1. `test-schedule: serial`, empty `nextest-threads` (default).
2. `test-schedule: serial`, `nextest-threads: 6`, `nextest-profile: ci`.
3. `test-schedule: serial`, `nextest-threads: 8`, `nextest-profile: ci`.
4. Eight threads with `nextest-profile: ci-git-limited` if unrestricted concurrency
   causes contention. This profile limits test cases, not individual child processes.
5. `test-schedule: balanced`, empty `nextest-threads`, as a lower-priority experiment.

Do not combine the explicit override with balanced scheduling. Do not promote a
scheduling policy unless its median improves by at least 10% and stress tests
remain stable. Do not drop tests, increase ignored counts, or enable retries to
reach a timing target. The existing runner checks inventory against nextest and
libtest results on every repetition.

Local/native equivalents, from the repository root:

```sh
python3 scripts/ci/run.py compile --context workspace
python3 scripts/ci/runtime.py target/ci-reports/runtime-default --samples 5
python3 scripts/ci/runtime.py target/ci-reports/runtime-threads --samples 5 --nextest-threads 8
python3 scripts/ci/runtime.py target/ci-reports/runtime-limited --samples 5 --nextest-threads 8 --nextest-profile ci-git-limited
python3 scripts/ci/runtime.py target/ci-reports/runtime-balanced --samples 5 --schedule balanced
python3 scripts/ci/report.py runtime target/ci-reports
```

The example thread count assumes four CPUs. Each output directory must be new.
The harness rejects enabled timing/trace instrumentation, records the revision,
dirty state, Git/Rust versions, runner image, platform, CPU count, build profile,
selection and job identity. Each sample owns a complete report tree, including
raw nextest/UI logs and timing records. The report groups revisions, environments,
thread counts and nextest profiles separately and ignores duplicate copies of the same measurement;
failed runs never contribute successful duration samples. Local
repetitions do not count as independent hosted jobs. Keep baseline and candidate
artifacts, including their inventories. For an older baseline without this
harness, copy the measurement scripts into that checkout and record that fact.
Use committed, clean revisions for acceptance; dirty checkouts remain diagnostic
and do not receive the report's `enough_samples` flag. Each sample stores its
inventory under `sample-N/workspace/coverage.json` and its raw logs beside the
`workspace` directory.

Acceptance requires **at least 30% lower median workspace execution time and
at most 480 seconds** on Windows x64, with at least five samples across two jobs
for each revision. Also run Windows ARM64 and Linux/macOS controls; investigate
regressions greater than 5%. Enable `watcher-stress: true` for ten repetitions of
the native synchronization and lifecycle suites. Compare inventories with
`report.py coverage BASELINE_INVENTORY CANDIDATE_INVENTORY`; added regressions
are expected, missing/newly ignored tests are not.
Watcher stress uses the same explicit `nextest-threads` override when supplied.

Native Windows results are still required. Process-count reductions and Linux
test passes do not establish the 480-second target.

## Separating fixture cost from application cost

Run a separate workflow with `fixture-timings: true` and the normal single
execution. Download its CI artifact and run:

```sh
python3 scripts/ci/report.py fixtures path/to/fixture-timings
```

`setup`, `config`, `subprocess`, `backend`, `watcher-wait`, `ui-wait`, and `cleanup` are
separate categories. Some are nested; never sum them as wall-clock time. Current
coverage is targeted, not a complete accounting of every fixture/cleanup path.

For selected CLI or backend tests, set `GIT_TRACE2_EVENT` to a new absolute file
path and run one test at a time. Use a different file per test invocation so
nested Git children can be attributed to that test. For example, in PowerShell:

```powershell
$env:GIT_TRACE2_EVENT = Join-Path $PWD 'target/setup-trace.jsonl'
cargo test -p gitcomet --no-default-features --features gix --profile ci-test --test standalone_tool_mode_integration setup_local_is_idempotent_and_preserves_original_backup_state -- --exact
Remove-Item Env:GIT_TRACE2_EVENT
python3 scripts/ci/report.py trace2 target/setup-trace.jsonl
```

Trace2 lifetimes overlap across children and omit part of OS process launch;
compare parent operation timers too. Do not reuse traced timings for acceptance.

For application/backend comparisons, prepare a disposable fixture once, then
measure the existing release benchmark independently of fixture creation:

```sh
cargo build -p gitcomet-git-gix --features benchmarks --release --example status-refresh-bench
target/release/examples/status-refresh-bench init target/runtime-status-plain plain
target/release/examples/status-refresh-bench run target/runtime-status-plain 100
```

Use the `.exe` executable on Windows. Repeat with the benchmark's `mixed` and
`dirty-lfs` fixtures. For history/diff/blame/ref operation attribution, use
`git_ops_trace_integration` and the existing backend benchmarks. For UI CPU work,
use `cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance`
and compare like-for-like cases. The UI's `ci-test` build uses optimization level
zero; its test durations alone do not establish release application slowness.

See [the implementation review](windows-test-runtime-review.md) for verified
limitations, the tracing improvement's before/after measurement, and the next
optimization priorities.
