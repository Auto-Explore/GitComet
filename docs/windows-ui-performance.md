# Windows UI and test performance

This investigation uses the Windows machine and the `speed_up_ci` checkout.
The branch baseline is `dev`; `main` is not a performance baseline. Measurements
below compare frozen binaries that differ in one optimization at a time. They
do not establish results on Linux, macOS, Windows ARM64, or hosted CI runners.

Machine: Ryzen 5 3600 (12 logical CPUs), Radeon RX 5700 XT, driver
32.0.21045.5002, 1920 × 1080 at 60 Hz, Windows 10 build 19045. Python 3.14.7,
Rust 1.98.1, Git for Windows 2.53.0, nextest 0.9.145.

## Repeatable UI measurements

`scripts/measure-ui-responsiveness.ps1` launches an owned process tree with an
isolated session and fixed window bounds. It records executable hashes, GPU
and driver details, phase timestamps, process CPU time, input delivery rate,
and native Windows move/resize events. Failed gestures and abnormal shutdowns
fail the capture. A watchdog terminates only the owned app if it hangs.

Native scenarios temporarily control the pointer and foreground window. Run
them on an idle desktop. The harness restores pointer/focus/environment and
releases its timer-resolution request after each capture. Programmatic move
and resize are also available, but do not exercise the native modal loop.

Use a clean **independent clone outside this workspace**, with the same commit
and file contents for both variants. A linked worktree shares Git metadata;
source edits elsewhere can otherwise trigger repository work during a capture.
Finish all builds and tests before collecting timings. Do not edit source or
run other workloads during measurement.

```powershell
python scripts/ci/ui-responsiveness.py measure --baseline C:/perf/baseline.exe --candidate C:/perf/candidate.exe --repository C:/perf/fixture --output target/ci-reports/ui-first --session first --pairs 3 --native-gestures
# Collect another session separately, reversing the first pair.
python scripts/ci/ui-responsiveness.py measure --baseline C:/perf/baseline.exe --candidate C:/perf/candidate.exe --repository C:/perf/fixture --output target/ci-reports/ui-second --session second --pairs 3 --reverse --native-gestures
python scripts/ci/ui-responsiveness.py report target/ci-reports/ui-first target/ci-reports/ui-second
```

Use `--no-probe` for a control measuring process CPU and delivered actions
without instrumentation. `--d3d-validation auto|on|off` is effective only with
the GPUI patch below. Automatic validation retains the original debug/release
behavior. Do not compare a validation-off binary with an automatic baseline
and attribute that difference solely to a code change.

The optional `typing` scenario focuses the branch filter and sends text and
backspace. Native move/resize entry is verified
automatically. The coordinates for hover/scroll/click and the typing target
must be checked after changing the application's layout or the fixture.

### What the probe measures

Set `GITCOMET_UI_PROBE=1` and `GITCOMET_UI_PROBE_JSONL=<new file>` to collect:

- Every CPU draw (layout, prepaint, paint), with its invalidation timestamp.
- Every platform submission, separately from CPU drawing.
- Foreground wake samples and sampled main-thread CPU usage.
- Per-window input-to-submission histogram deltas and dropped mid-draw events.

The JSONL clock anchor aligns raw frame events with scenario boundaries. The
summary excludes boundary frames and computes percentiles from raw samples.
It never substitutes zero latency for a phase with no draws or averages
interval p95 values into a purported frame p95. Wake statistics use intervals
entirely inside a scenario. Input interval percentiles remain diagnostic data
in the raw log; they are not pooled into the frame statistics.

**Submission is not display completion.** It measures CPU/platform work, not
the moment a pixel reaches the display. Use [PresentMon](https://github.com/GameTechDev/PresentMon)
or WPR/WPA before changing swap-chain pacing. Microsoft's
[DXGI latency guidance](https://learn.microsoft.com/en-us/windows/uwp/gaming/reduce-latency-with-dxgi-1-3-swap-chains)
describes the distinction. The current evidence does not justify changing
swap-chain flags or coalescing resize notifications.

Acceptance requires at least six alternating pairs in two sessions, at least
10% improvement in the selected metric, and no unexplained p95 regression over
5%. A successful capture or enough samples alone does not establish acceptance.
Keep build time, test time, input rate, and correctness in the decision.

## GPUI window-loop patch

`patches/gpui-windows-responsiveness.patch` applies to GPUI commit
`9ff6e7f487f9cba1a8d5a1fc45aa173d094ff9e7`. It:

- Reuses the composited surface for Windows moves when viewport size, DPI,
  display and client-relative cursor position are unchanged. Bounds observers
  still run; resize, DPI/display changes and hover-sensitive moves refresh.
- Updates native IME positioning during moves that reuse the surface.
- Gives modal move/resize task dispatch the ordinary loop's 10 ms budget,
  leaving pending tasks and their normal-loop wake notification intact.
- Adds `GPUI_D3D_DEBUG=auto|on|off`, with automatic validation unchanged.

Regression tests cover scene reuse versus resize/hover invalidation, pending
tasks across modal ticks, and validation policy including release builds.
The window regression and all 16 Windows platform tests passed locally.

The normal GitComet dependency pin is unchanged: the fork patch must be
published before a portable Git revision can be pinned. A local review commit
is prepared on branch `windows-responsiveness-review`:
`930facd0d77ca30f3fea24f51b9a464161d335d3`. It contains exactly the patch above
and has not been pushed. The checkout is
`C:/Users/sanni/.codex/worktrees/gpui-windows-performance-20260921`.
An importable `git format-patch` is saved at
`target/ci-reports/windows-responsiveness/gpui-windows-responsiveness-review.patch`.
The delivery choice is to review/publish this GPUI change separately, then update
GitComet's Git revisions and lockfile to the published commit; no GPUI source
is vendored into GitComet.

Prepare a separate checkout for local review and reproduction:

```powershell
python scripts/ci/prepare-gpui-performance.py --checkout C:/perf/gpui-windows --config target/gpui-performance.toml
$lockPath = Join-Path $PWD 'Cargo.lock'
$lockBefore = [IO.File]::ReadAllBytes($lockPath)
try {
    cargo build -p gitcomet --bin gitcomet --config target/gpui-performance.toml
    if ($LASTEXITCODE -ne 0) { throw 'GPUI experiment failed to build' }
} finally {
    [IO.File]::WriteAllBytes($lockPath, $lockBefore)
}
```

The helper checks the base revision and existing checkout changes, applies the
patch (or verifies a clean, exact review commit), and generates all related
package overrides. It does not edit Cargo's
shared cache. Keep the GPUI checkout outside GitComet to avoid nested workspace
inheritance. Do not run another Cargo invocation while the temporary path
overrides are updating the lockfile. After publication, update the normal Git
pin and regenerate the lockfile instead of committing machine-specific paths.
The generated configuration also keeps all 21 GPUI packages non-incremental
with 16 codegen units, matching the pinned Git dependencies. Otherwise changing
from Git to path dependencies also changes code generation and confounds the
comparison. See [Cargo's profile rules](https://doc.rust-lang.org/cargo/reference/profiles.html).

The patch remains experimental: the repeated comparison below did not pass
every latency guard. Publishing and pinning it requires resolving those results
as well as making the fork commit available.

## Test execution changes

On Windows, the CI runner batches only the audited, in-memory
`gitcomet-core::conflict_session` group in one libtest process. All other
non-UI tests retain nextest isolation. Exact successful test names are checked
against the inventory; the complementary nextest report rejects missing,
unexpected, duplicate, or doubly executed tests. Failures and timeouts still
fail the execution. `--batch-pure-tests off` restores process-per-test execution
for comparison; `on` enables the experiment explicitly on another platform.

The same-binary five-pair screening measured 170 tests at median 0.675 s in
nextest versus 0.065 s in one process. This saves about 0.61 s for this group;
it is not a tenfold improvement for the whole suite. The approach addresses
[nextest's per-process overhead](https://www.nexte.st/docs/design/why-process-per-test/)
only where shared execution has been audited.

Four Git backend integration harnesses now use the existing fixture helpers
for fresh identity/signing configuration. One ordinary two-commit blame
fixture uses fast-import. The operation under test still uses the real backend
and Git, and the tests retain their original assertions. Later configuration
changes, hook behavior, signing, and porcelain-operation tests still use Git.

`--nextest-profile ci-watch-first` is an optional scheduling experiment. It
prioritizes the six longest observed watcher tests while leaving other worker
slots available for process-heavy work. It inherits CI timeouts, zero retries,
leak checking and JUnit coverage. It does not shorten event-settling or negative
quiet windows. The default scheduling policy remains `ci`.

```powershell
$env:PATH += ';C:\Program Files\Git\bin'
python scripts/ci/runtime.py target/ci-reports/watch-first --samples 1 --session screening --nextest-threads 12 --ui-threads 4 --nextest-profile ci-watch-first
```

Nine existing watcher tests require directory symlink creation, which fails
with Windows error 1314 in this process. They remain enabled. Their failures
prevent a passing whole-suite performance acceptance result on this machine.
The separate UI, pure-test and integration-fixture comparisons can be measured
without treating those failures as successes.

The runner isolates UI session storage even when a harness has been copied or
renamed. For manual binary freezing, preserve a Cargo-style `deps/` directory
and the `gitcomet_ui_gpui` executable name. Prefer the runner, which also removes
an inherited session-file override and supplies isolated Windows app data.

## Local results

Raw captures, build records, test logs and frozen executables are retained in
`target/ci-reports/windows-responsiveness/`. Screening captures are distinct
from repeated comparisons; early linked-worktree screens are not acceptance
evidence. The acceptance fixture is an independent, clean clone at
`0af9d5937f2358e2920ff98bc68d0e97806afcf8` (594 commits). Each UI comparison uses
six alternating pairs across two sessions, with the starting order reversed
in the second session. Scenarios last five seconds after warmup and settling.
These are local Windows results, not Windows-versus-Linux measurements.

### Default debug profile: accepted

Only `gpui-ce`, `gpui_ce_windows`, and `taffy` change from optimization level 0
to 2. GitComet's application crates remain at level 0 with debug assertions.
The test and CI profiles explicitly retain level 1 for those dependencies,
because their package settings otherwise inherit from `dev`.

Median of the six per-run p95 CPU draw times:

| Scenario | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Native window movement | 41.66 ms | 9.69 ms | 76.7% |
| Native window resizing | 43.82 ms | 10.93 ms | 75.1% |
| History scrolling | 41.83 ms | 9.41 ms | 77.5% |
| Sidebar tab clicks | 44.85 ms | 10.57 ms | 76.4% |
| Branch filter typing | 18.18 ms | 3.67 ms | 79.8% |

Every measured dirty-to-draw and wake p95 improved. Submission p95 improved
except typing (0.503 to 0.517 ms, +2.8%, within the 5% guard). Input delivery
rates matched within 0.15%. Process CPU use fell 34.5% while moving, 9.0% while
resizing, 39.8% while scrolling, 65.3% while clicking and 75.5% while typing.
A separate probe-disabled control pair also reduced move CPU from 5.406 to
3.344 seconds and scroll CPU from 5.031 to 3.109 seconds per five-second phase.
That control checks instrumentation sensitivity; it is not another acceptance
session.

Three alternating warm incremental rebuild pairs, after a warmup for each
variant, measured median 23.39 seconds before and 20.66 seconds after. Each
pair changed the same small runtime constant and restored the original source
afterward. No incremental rebuild penalty was observed. A cold dependency build
was not measured; the three optimized dependencies can cost more on a cold
build. The final ordinary `cargo build --locked --offline -p gitcomet --bin
gitcomet` succeeded with the promoted settings.

Reports: `accepted-dependencies-{first,second}/session.json`,
`accepted-dependencies-combined.json`, `control-dependencies-no-probe/`,
`debug-warm-builds.json`, and `promoted-debug-build.json`.

To create a baseline after this change, override the three settings, freeze
that executable, then build normally and freeze the candidate. Use the same
source and fixture for both:

```powershell
cargo build --locked -p gitcomet --bin gitcomet --config profile.dev.package.gpui-ce.opt-level=0 --config profile.dev.package.gpui_ce_windows.opt-level=0 --config profile.dev.package.taffy.opt-level=0
# Copy target/debug/gitcomet.exe to a separate baseline path here.
cargo build --locked -p gitcomet --bin gitcomet
# Copy target/debug/gitcomet.exe to a separate candidate path here.
```

### GPUI patch: promising, not accepted

Against the optimized dependency baseline, the patch reduced native move CPU
use from 0.706 to 0.106 CPU cores on average during the phase (85.0%). Move draw
p95 fell from 9.69 to 6.99 ms. A probe-disabled control pair also reduced move
CPU from 3.531 to 0.563 seconds. Resize and scroll CPU changed little.

The combined six-pair results exceeded the 5% guard for resize submission p95
(0.697 to 0.741 ms), click submission p95 (0.831 to 0.902 ms), and click wake
p95 (0.064 to 0.081 ms). These absolute differences are small, but their cause
is unresolved. The normal dependency pin therefore remains unchanged.
Dirty-to-draw p95 is also sensitive to which invalidations remain after removing
unnecessary draws; it does not measure compositor movement or displayed pixels.

Reports: `accepted-gpui-{first,second}/session.json`,
`accepted-gpui-combined.json`, `control-gpui-no-probe/`. Both variants use the
same compiler strategy, probe, validation mode, repository and input schedule.
Earlier patch screens with different Git/path compilation settings are excluded.

### Test speed and compilation tradeoffs

Six paired runs in two sessions of four representative fixture tests measured:

| Fixture case | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Two-commit blame | 0.370 s | 0.250 s | 32.3% |
| Cherry-pick with commit | 0.630 s | 0.584 s | 7.3% |
| Revert with commit | 0.593 s | 0.544 s | 8.3% |
| Squash a linear range | 0.941 s | 0.864 s | 8.2% |
| Four-case cohort | 2.540 s | 2.249 s | 11.4% |

The largest retained run also improved for each case. All 89 tests in the four
changed harnesses passed. The percentage applies to these four representative
cases, not the entire backend or workspace suite. Reports:
`fixture-pairs-{first,second,combined}.json` and `fixture-tests.json`.

For the UI test crate, an optimization-level experiment showed:

| CI UI optimization | Compile invocation | 4,016 tests | Combined |
| --- | ---: | ---: | ---: |
| Level 0 (retained default) | 106.6 s | 110.6 s | 217.2 s |
| Level 1 (optional) | 280.1 s | 77.8 s | 358.0 s |

These are screening runs, not repeated cold-build comparisons; the level-0
compile also rebuilt a backend dependency. Even with that conservative baseline,
the faster test execution did not offset compilation. Both runs passed exactly
the same 4,016 tests with the same nine existing ignores. The UI's default stays
at level 0. For repeated unchanged local runs, level 1 may become worthwhile
after roughly six runs. Keep its artifacts separate when measuring:

```powershell
cargo test --locked --profile ci-test -p gitcomet-ui-gpui --lib --config profile.ci-test.package.gitcomet-ui-gpui.opt-level=1
# Optional additional debug UI optimization; this also adds rebuild cost.
cargo build --locked -p gitcomet --bin gitcomet --config profile.dev.package.gitcomet-ui-gpui.opt-level=1
```

Reports: `ci-ui-builds.json`, `ci-ui-tests.json`, and `ci-ui-tests-coverage.json`.
An initial success-name parser missed the libtest `- should panic` suffix;
coverage was rechecked against both binary inventories using the corrected parser.

### Verification and limitations

- All 52 Python helper tests passed, including batch partition/coverage checks,
  relocated UI session isolation, frame alignment, and rejection of incomplete
  or nonresponsive measurements.
- Core execution passed both with and without batching: all 567 active tests
  executed, with the same four existing ignores. The batched mode ran 170 in
  libtest and 397 in nextest. The optional watcher-priority profile parsed and
  ran, but has no passing whole-workspace timing result here.
- The GPUI scene regression and all 16 Windows platform tests passed in the
  separate patched checkout. UI and fixture results are recorded above.
- Rust formatting and whitespace checks passed. The normal GPUI pin and
  `Cargo.lock` are unchanged by this implementation.
- One earlier core run hit an existing five-second Git executable probe timeout.
  Five isolated repeats and both subsequent complete core executions passed.
  The failed attempt remains in `runner-verification/`; it is not timing evidence.
- An early typing capture delivered text before the branch filter received
  focus. It is marked incomplete and excluded. The harness now waits for tab
  selection before the native focus click and rejects typing phases with too
  few resulting draws. Native movement/resizing must also have OS gesture events.
- An early renamed UI harness bypassed the application's Cargo test-path
  detection and wrote test preferences to the developer's saved session.
  The file was backed up after discovery; no earlier preference backup was
  available. The runner isolation regression prevents recurrence. A reset
  preview preserving repository lists was prepared in `session-reset-preview.json`.
  The user chose to leave the preferences as they are; no reset was applied.
  Repository data was unaffected. See `session-recovery.txt` for the local
  recovery record.
- WPR's GPU trace could not start: `0xc5585011`, “Failed to enable the policy
  to profile system performance.” No profiling policy or privilege was changed.
  The attempt is recorded in `wpr-gpu-attempt.json`. Display completion and
  production interaction latency remain unverified.

Next: collect production PresentMon/WPA evidence in an environment that permits
GPU tracing, resolve the GPUI submission/wake tails, then publish and pin a
reviewed patch. Repeat complete CI scheduling measurements where directory
symlink creation is available, and run the promoted profile changes in the
separate Linux/macOS environment before making cross-platform claims.
