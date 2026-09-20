# Windows runtime implementation review

## Verdict

The configuration snapshot, tracing wakeup, ordinary-fixture changes, packed
missing-ancestor fixture and limited watcher/UI wait conversions pass Linux
verification. Default scheduling, existing coverage, ignore lists and production
watcher behavior are preserved. Native Windows validation is still required; this
review does not certify a 30% reduction or an eight-minute Windows run.

Latest verification after implementing the follow-up fixes (2026-09-20):

- The full Linux execution passed 2,859 nextest tests, 4,046 UI tests and 111 UI
  budget tests (7,016 active cases), with inventory checks and complete raw logs.
  The 15 nextest skips and nine UI ignores are unchanged. Inventory comparison
  with the preceding implementation found five added regressions, no missing
  cases and no newly ignored cases.
- The measurement/reporting regression suite passes 34 tests. Workflow YAML,
  Bash blocks, nextest profile configuration and Rust formatting checks pass.
- The three UI cases whose fixed waits changed passed ten repetitions each;
  their 120 optional wait records were successfully collected and summarized.
- The complete execution took 110.725 seconds on this 32-CPU Linux machine using
  eight nextest slots and `ci-git-limited`. It verifies the optional profile and
  its inherited JUnit/coverage checks; it is not a hosted-runner speedup estimate.
  The default `ci` profile has no tests assigned to the new group.

The first sandboxed run hit existing tests' GPG temporary-file and Unix-socket
access restrictions. Rerunning with filesystem/socket access passed every case;
the failed run remains a failed diagnostic sample. The preceding implementation
also passed two full Linux executions before these five regression tests were
added.

The supplied logs use PR merge revision `6283f260`, combining branch `174cc9a0`
with base `45355a5f`. This checkout is the branch revision, and already contains
a staged GPUI dependency update unrelated to the runtime changes. The PR
base adds two focusing-click UI tests absent from this branch, explaining the
4,048 versus 4,046 UI pass counts. No UI tests were removed by this work. Native
acceptance must compare equivalent merged inventories and dependency versions;
the local results cannot establish that comparison.

The previous changes are useful but do not yet address most of the runtime.
In particular, replacing the four selected watcher cases removes approximately
21 seconds of prescribed Windows waits in aggregate, or roughly five wall-clock
seconds at four-way concurrency if other costs are unchanged. The 61.5-second
directory-recreation test retains its safeguards. The packed history fixture
addresses one 38.3-second test, not the whole backend suite.

## Correctness findings and repairs

1. **Repeated execution lost raw diagnostic logs.** The original harness copied
   only `ci-reports/workspace`, while nextest/UI logs and timing records are
   written beside that directory and overwritten by the next invocation. Each
   sample now has its own complete report directory. It reuses compile metadata
   and starts without old execution/JUnit results. A regression test verifies
   two distinct sets of raw logs and restoration of the report directory.
2. **The summary mixed different measurement environments.** Samples were grouped
   by revision, platform, architecture, CPU count and scheduling, but ignored the
   recorded Git/Rust versions and runner image. A reproduction combined 700-second
   and 400-second samples from different environments into a misleading
   550-second median with `enough_samples: true`. The report now separates OS
   versions, images, Git/Rust versions, build profiles and selections too. Missing
   environment metadata prevents the sufficiency flag.
3. **Copied artifacts could inflate sample counts.** Measurements now have an ID;
   identical copies are counted once and conflicting copies are rejected.
4. **Some tracing modes bypassed the acceptance guard.** The harness now rejects
   `GIT_TRACE2` and `GIT_TRACE2_PERF`, as well as an empty
   `GITCOMET_TEST_SYNC_TRACE` variable (the watcher treats its presence as enabled).

The scoped configuration parser matches Git's documented NUL/newline format,
preserves multiple values and subsection case, and retains the existing include
semantics of explicit scopes. Regression checks cover missing config, locks,
invalid input, backups, dry runs, user edits, idempotency, and local/global Git
process counts. Writes still go through Git. The sequence is not a transaction
against another writer; that was also true before this change.
[Git configuration documentation](https://git-scm.com/docs/git-config).

The watcher conversions wait for a forwarded native event for the specified
path, then drain delivered work. Same-path edit sequences and negative quiet
assertions still settle. The fresh-path startup optimization is limited to a
path that did not exist during fixture creation. Broadly shortening the common
three-second quiet window would weaken the existing regressions.

## Application tracing bottleneck and implemented fix

Before the fix, `Trace2Monitor::finish` marked its worker done and joined it.
The worker used a 20 ms sleep while tailing its temporary trace file, without a wakeup on finish.
Actual user commands attach a `GitOperationContext`; many direct backend tests
do not. Consequently, those tests can miss an overhead present in the app.

A local Linux `ci-test` probe alternated the same `remote set-url` backend call
with and without the operation context. Fixture creation was outside timing,
the first five pairs were warmups, and 35 samples were retained per mode:

| Mode | Median | Range |
| --- | ---: | ---: |
| Direct backend call | 1.507 ms | 1.443–2.113 ms |
| Attached operation context | 20.230 ms | 20.193–20.277 ms |

The approximately 18.7 ms difference is consistent with waiting for the tracing
poll. It measures the entire context overhead, not a profiler-isolated sleep.
It is a real application optimization candidate, not evidence that this accounts
for the Windows/Linux test gap or that Windows has the same numbers.

Reproduce on native Windows and Linux:

```sh
cargo run -p gitcomet-git-gix --profile ci-test --example operation-context-probe
```

The worker now uses `park_timeout` and both explicit completion and `Drop` signal
the atomic stop flag and `unpark` it before joining. The stored wakeup token also
covers completion between the worker's stop check and parking. Final file reads
and trace parsing remain intact. Regression tests cover finish/drop wakeups and
an unterminated final hook event; all 41 Git-wrapper unit tests pass, including
real hook output, cancellation and process-tree cleanup.

The same local probe after the fix measured:

| Mode | Median | Range |
| --- | ---: | ---: |
| Direct backend call | 1.482 ms | 1.158–1.878 ms |
| Attached operation context | 1.619 ms | 1.534–3.525 ms |

That is approximately 92% less latency for this traced command in this Linux
probe. It is not a Windows or whole-application speedup claim.

An external `GIT_TRACE2_EVENT` file does not capture every attached operation:
the internal monitor replaces that destination. Diagnostics of the actual app
need to observe its internal trace or instrument the command wrapper too.

## Prioritized next experiments

### 1. Increase nextest concurrency with measured limits

The supplied Windows durations sum to 2,136.851 test-seconds. Dividing by
`4 × 536.107` yields 99.65% slot occupancy. This includes waiting and says nothing
about CPU utilization. It does show that changing test order alone has little
room to help at four slots: the fixed-duration lower bound is 534.213 seconds,
only 1.894 seconds below the observed nextest time.

Try serial nextest at 6 and 8 threads before promoting the balanced policy.
With the recorded individual durations held fixed, the nextest bounds are
356.142 and 267.106 seconds respectively. Eight threads plus the existing UI
time and overhead gives an idealized 435.6 seconds; six gives about 524.6.
These are planning models, not forecasts: more parallel Git processes may
increase disk, scanner, memory and CPU contention.

The existing balanced policy gives nextest only two threads on a four-CPU runner.
Its fixed-duration nextest bound becomes 1,068.426 seconds even while UI runs
concurrently. It needs a very large contention reduction to pay off. Keep it an
experiment, not the default.

If eight unrestricted threads contend, try a larger global limit with a group
limiting Git-heavy tests, allowing waiting watcher tests to occupy other slots.
Keep every test enabled and repeat native watcher stress at the candidate limit.
Nextest supports these group limits independently of its global limit.
[Nextest test groups](https://nexte.st/docs/configuration/test-groups/).

Implemented controls: select `nextest-profile: ci-git-limited` together with
`nextest-threads: 8` in the workflow, or pass the corresponding CLI arguments to
`runtime.py`. The experimental profile inherits `ci`, limits the backend package
and three CLI integration binaries to four concurrent tests, and preserves the
full inventory. JUnit files come from the selected profile directory, results
are grouped by profile, and watcher stress receives the chosen thread count.
The normal CI policy remains unchanged.

### 2. Expand ordinary fixture optimization in the largest remaining suites

| Windows suite | Tests | Summed duration |
| --- | ---: | ---: |
| mergetool integration | 64 | 262.373 s |
| standalone CLI integration | 65 | 194.708 s |
| status integration | 169 | 169.283 s |
| submodules integration | 21 | 78.267 s |
| remote management integration | 35 | 68.006 s |

These overlapping durations identify where to investigate; they are not
additive job time or promised savings. Some helpers were already optimized
before this change. Avoid counting those savings again.

Implemented: remote-management fresh initialization writes its five ordinary
identity/line-ending settings directly. Clone configuration keeps real Git
replacement semantics, as do remote mutations under test. Subprocess/setup
timings were added to the helper.

The shared importer now supports Git-quoted paths, with a regression check for
spaces, UTF-8, quotes, backslashes and control characters (the latter filenames
only on Unix). The common status text-conflict fixture imports base/theirs in
one process and retains real checkout, ours commit and merge operations. This
removes four Git processes per fixture without synthesizing a conflicted index.
Submodule seed fixtures use import/reset, removing the separate seed-file sync
and ordinary commit machinery; the process count is unchanged for those seeds.
Symlink conflicts, hooks, filters and behavior under test still use real Git.

Further fixture expansion should use the separate timing categories; repositories
remain independent and mutable fixtures are not shared between tests.

### 3. Reduce redundant application work

Implemented: the setup snapshot includes every managed entry and skips only a
single exactly matching value. Repeated setup needs one Git process, down from
19 after the initial snapshot implementation. Multiple matching values still
reach Git and retain its setter error. Tests check no-op setup without a write
lock, changed/missing value repairs, snapshot updates, backups and both scopes.

For repositories with submodules, inspect whether one refresh requests staged
and unstaged status separately: both paths can call the porcelain gitlink
supplement. Share one result within a refresh if measurements confirm duplicate
calls, with invalidation for index/worktree/submodule changes. Ordinary
repositories already avoid this subprocess; do not assume every status refresh
launches Git.

### 4. Profile UI execution and positive waits

The main UI suite still takes 164.79 seconds on Windows versus 89.51 on Linux.
The small wait changes below address only a fraction of it. Collect per-case or
per-group timings, then separate fixture/scene creation, CPU work, drawing, and
test waits.
Implemented: six 120 ms pumping intervals in the repository-tab/splash/tooltip
tests now wait for rendered selectors and tooltip disappearance. These check
completion; animation pumping and the subsequent negative tooltip observation
windows remain unchanged. The three common wait helpers emit optional `ui-wait`
timings keyed by case and operation through the existing fixture report.
This is at most 720 ms of prescribed waits across these cases, not a solution
to the whole UI gap. The compiler profile remains unchanged.

Run focused release benchmarks for history loading, repository switching and
diff opening on prepared fixtures. Compare against CI-test results without
equating an unoptimized UI test build to release application performance.

### 5. Measure Windows-specific launch, I/O and scanning overhead

On the same hosted image, compare repeated `git --version`, a cheap command on a
prepared repository, and the corresponding GitComet operation. Correlate Git
process lifetimes with the application's spawn/wait/output-drain timers. The
wrapper creates output-reader threads and polls process completion; measure
that cost before changing timeout/cancellation machinery.

If Defender is active and profiling is available, collect a performance trace
to identify expensive processes and paths. The supplied job logs do not prove
that antivirus is responsible. Use measurement to guide file/process reductions;
do not make disabling protection part of normal CI.
[Microsoft Defender performance analyzer](https://learn.microsoft.com/en-us/defender-endpoint/performance-analyzer-reference).

Six tool-help tests total 145.447 seconds, but they exercise Git's tool discovery
and real shell integration. Keep them enabled while measuring setup versus
discovery; avoid counting all that time as GitComet execution.
