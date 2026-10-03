# Profiling GitComet

Profiling and benchmark drivers live here. CI execution, cache management and
test-runtime reports stay in `scripts/ci/`. Application profiling runs locally;
it does not add jobs or application builds to the CI test matrix.

Run examples from the repository root. Python tools require Python 3.11 or newer;
use `python3` instead of `python` where appropriate. Build probes with the repo's
Rust toolchain. LFS fixtures need `git-lfs` on `PATH`. Windows GUI captures need
Windows PowerShell and an interactive desktop. Linux process tracing requires
Valgrind and strace; shell report comparison uses `jq`. Commands expose `--help`
(PowerShell scripts expose parameters through `Get-Help`).
`measure-process-tree.ps1` and `ui-responsiveness.cs` are shared helpers.

Supply repository, baseline, binary and output paths for your environment.
Default binary paths are relative to this checkout; override them for custom
Cargo target directories. Fixed fixture contents and sizes define repeatable
workloads, rather than machine settings. Build first, then measure without
competing builds or tests, using matching profiles and dependencies. Keep
instrumented captures separate from latency measurements. Paired drivers
alternate execution order and retain raw samples and metadata. Use fresh output
directories, normally under `target/` or `tmp/`.

## Application investigation and regression workflow

`performance.py` joins corpus preparation, native UI scenarios, Git/LFS/annex
transfers, diagnostics and paired reports. It is a local/manual workflow; no CI
job or network account is needed. Start with:

```sh
python3 scripts/profiling/performance.py doctor
cargo build --release -p gitcomet -p gitcomet-git-gix --features gitcomet-git-gix/benchmarks --bin gitcomet --example interaction-probe
python3 scripts/profiling/performance.py prepare --suite deep --real --repo-root /home/sampo/git/git_test_repos
python3 scripts/profiling/performance.py run --suite smoke --backend target/release/examples/interaction-probe --output target/performance/smoke --session initial
```

Preparation is outside measurements. The generated history sizes are 20,000,
100,000 and 2,000,000 mainline steps, with additional merge-side commits and
fixture files. The large fixture streams fast-import input through a temporary
file and bounds directory fanout. `--commits 20000` prepares just that size.
`--real` snapshots Git, Bun and Chromium from the existing local sources; it
does not fetch updates. Original repositories are only read. Mirrors have
independent object files; checkouts may hardlink objects from those owned
mirrors. All ref/worktree mutations for transfers happen in disposable copies.
`corpus.json` pins refs, HEAD, object format, pack sizes and commit graphs;
run manifests also hash worktree differences, binaries and the harness.
Existing snapshots are validated rather than overwritten.

On another host pass `--repo-root PATH` to prepare and run, and override sources
with repeated `--source git=PATH --source bun=PATH --source chromium=PATH`.
The default corpus is `~/git/git_test_repos`; `GITCOMET_PERF_CORPUS` overrides it.
On Windows use `python` and `.exe` executable paths. Git LFS and git-annex are
optional: missing tools produce explicit skipped cases and an incomplete suite.
Linux GUI runs require headless Mutter and D-Bus; `--display desktop` uses an
interactive native desktop instead. Profiles, sessions and crash reports are
isolated through `GITCOMET_PROFILE_ROOT`; the user's HOME is not replaced.
Only the disposable profile enables HTTP for the loopback transport fixtures.
Use `run --purpose validation` for functional checks during builds; those runs
are explicitly excluded from timing comparisons.

List or select workloads before an expensive run:

```sh
python3 scripts/profiling/performance.py run --suite deep --list --output target/unused
python3 scripts/profiling/performance.py run --suite deep --scenario 'ui/*/history-*' --output target/performance/history --session history
python3 scripts/profiling/performance.py run --suite deep --scenario 'transfer/*/*' --scenario 'cancel/*' --backend target/release/examples/interaction-probe --output target/performance/transfers --session transfers
python3 scripts/profiling/performance.py run --suite deep --scenario 'ui/*/lifecycle' --cycles 100 --output target/performance/soak-100 --session soak-100
```

| Area | Measurement and completion evidence |
| --- | --- |
| Startup/history | Process spawn to first draw and usable history/status; generated and real repositories; refs and worktree fingerprints. |
| Hover | Rapid row sweeps, repeated stationary pointer events, witnessed message-card dwell and idle after dwell; handler/dispatch delays, draw cost, invalidations and store/worker counts. Sweep events have no per-row visual witness, so they do not claim input-to-draw latency. |
| Scroll/drag/select | Real production input handlers and rendered scroll-position/commit-detail witnesses; frame, input, store queue, worker queue and UI apply distributions. Burst selection also counts superseded work. |
| Clone/fetch/pull/push | CLI, backend (where available) and live store paths; resulting refs, ancestry and payload hashes. Clone uses smart HTTP, so local hardlink shortcuts cannot satisfy the test. No-op and divergent merge/rebase cases are in the deep suite. |
| LFS | Real batch/upload/download endpoints with content hashes; fetch, checkout/pull and push, including UI interaction during transfers. |
| Annex | Real git-annex with a disposable Git peer and directory content remote; get/copy/pull/push/sync and content hashes. These measure local content transport, not SSH/cloud latency. |
| Background work | Save, identical-byte touch, file bursts, ignored churn, diff search, idle, minimized and multiple-window idle. The save generator writes off the UI thread. |
| Cancellation | Stalled HTTP fetch/clone/LFS fetch, followed by continued input, cancellation acknowledgement and a usable UI. Failed/unfinished operations fail validation. |
| Retention | Ten warm-up cycles, repeated repository open/select/close, then a settling plateau; repeat at 100 and 200 cycles to distinguish retention from continuing growth. |

`--latency-ms 50 --bandwidth-mib 8` adds per-request delay and an aggregate
body-byte limit to the loopback Git/LFS server. This is a controlled application
transport, not a simulation of packet loss, TCP congestion or a production
hosting service. Annex's directory remote ignores these HTTP settings.
`--shape` selects the existing LFS payload shapes and small/large Git payloads.
Real clone cases (`clone/chromium/live`, for example) copy the full repository
and can take hours at a low bandwidth limit. They appear in `--list` but require
an explicit `--scenario 'clone/chromium/live'` selection; raise `--timeout` as
needed. Verified payload bytes and HTTP wire bytes are separate.

Every run writes `manifest.json`, `report.txt`, environment/binary identities
and case artifacts. UI cases retain scenario, frame/stage JSONL, process
samples and stderr. CLI/backend cases retain command, stdout/stderr, Git
Trace2, command-stage timings where supported, and final witnesses. Failed
transfer fixtures remain under the corpus's `.scratch` directory for inspection;
successful ones are removed unless `--keep-fixtures` is set. Do not run two
sessions concurrently against the same corpus.

### Regression decisions

First run an A/A check by giving the same executable as `--binary` and
`--baseline`. For a change, use frozen release executables from the two
revisions, with corresponding backend probes when selecting backend cases:

```sh
python3 scripts/profiling/performance.py run --binary CANDIDATE --baseline BASELINE --scenario 'ui/history-20000/history-scroll' --pairs 3 --session first --output target/performance/first
python3 scripts/profiling/performance.py run --binary CANDIDATE --baseline BASELINE --scenario 'ui/history-20000/history-scroll' --pairs 2 --reverse --session second --output target/performance/second
python3 scripts/profiling/performance.py compare target/performance/first target/performance/second --output target/performance/comparison.json
```

Pairs alternate order. Reports bootstrap **run-level paired ratios**, not
individual frames. A regression requires at least five pairs in two sessions,
a median slowdown of at least 20%, and a 95% interval excluding parity.
Fewer observations are labelled as needing confirmation. Changed binaries
between sessions, unmatched pairs, different workload/environment identities,
missing metrics, dropped records and failed witnesses cannot pass silently.
Allocation diagnostic timings are excluded. A/A noise must be assessed before
using the 20% policy on a particular machine. Identical-binary slowdowns are
labelled `noise_alert`, not code regressions. Shared-desktop activity, thermal
state and background builds can still confound an otherwise valid capture.

Initial alerts are p95 CPU draw over one frame interval (16.7 ms at 60 Hz),
immediate witnessed input-to-draw p95 over 50 ms or p99 over 100 ms, and
handler/dispatch/wake/apply stalls over 100 ms (1 s is severe). Intentional
tooltip dwell is excluded from immediate-input latency. Draw correlation uses
the input's window; another window's draw cannot satisfy it. Long frames remain
evidence rather than automatically being rejected as occlusion. These are
investigation targets, not universal startup/network time limits. `--strict`
also exits unsuccessfully on alerts; ordinary runs fail on invalid cases.

Fresh application processes/profiles and warm shared shader/OS caches are the
default. A new process is **not** a cold filesystem-cache experiment. For
cold-cache investigations use a dedicated rebooted host or documented external
cache control, record that condition, and keep it in separate sessions. The
existing `live-ui.py --cold-gpu-cache` controls only the shader cache. Keep the
same commit-graph condition in a pair; compare graph-enabled/disabled copies
in separate experiments, never by modifying the original source repository.

### Finding the cause

Use the slow phase's operation id to follow input → store receive/reduce →
task queue/start/finish → publication/UI apply → draw. `command_stage` records
partition instrumented Git subprocess wall time and carry a separate command
id; concurrent commands must not have their times added as sequential latency.
Compare direct CLI, backend and live operation results to distinguish transport
or Git cost from application scheduling/refresh cost. `overlapping_inputs`
proves whether the recorded gestures actually ran before operation completion;
zero means the run established no concurrent-interaction coverage.

For a main-thread stall, inspect a native CPU **and wait** capture: filesystem
calls, child waits, mutex contention and queue delay can be slow with little CPU.
Use operation traces to locate the interval before reading a whole flame graph.
For unnecessary work, compare `work_counts`, background refreshes, invalidations
and superseded tasks between stationary hover, sweep and idle. Existing
indexed-history Criterion groups provide allocation/row/graph mechanism probes;
they remain distinct from native application latency.

```sh
scripts/profiling/build_release_debug.sh
python3 scripts/profiling/performance.py profile --kind cpu --binary target/release-with-debug/gitcomet --repository /home/sampo/git/git_test_repos/history-20000 --scenario history-hover --output target/performance/cpu --execute
python3 scripts/profiling/performance.py profile --kind waits --binary target/release-with-debug/gitcomet --repository /home/sampo/git/git_test_repos/history-20000 --scenario history-scroll --output target/performance/waits --execute
python3 scripts/profiling/performance.py run --suite deep --scenario 'cancel/clone' --wrap 'strace -ff -tt -T -o {output}/syscalls' --output target/performance/clone-waits --session clone-waits
cargo build --release -p gitcomet --features perf-alloc
python3 scripts/profiling/performance.py run --scenario 'ui/*/history-hover*' --output target/performance/allocations --session allocations
```

Freeze the normal executable before building `perf-alloc`, which replaces the
binary in that Cargo profile. The opt-in tracking allocator records Rust
allocation/reallocation counts, bytes and net bytes for each phase, including
background work and observer overhead. It is a diagnostic build; pair it with
another allocation build and use `compare --allocations` to compare counts
without comparing instrumented timings. Net bytes and RSS alone do not prove a
leak. Use a heap profiler and surviving allocation stacks, plus the two cycle
counts, before making that claim. Existing heaptrack guidance below requires a
build without mimalloc interception conflicts.

Linux supports automated perf/strace capture. CPU captures retain a profiler
quality report and reject lost samples or unavailable loss counts; rerun under
lower load before using the profile for conclusions. Windows `profile --execute`
starts WPR, runs the native desktop scenario, and stops into an ETL; inspect
the application and descendants in WPA. macOS prints an Instruments Time
Profiler/System Trace recipe; attach it to a desktop run (automatic Instruments
launch is not implemented). Linux collects procfs PSS, mappings and thread
counters; Windows samples process CPU, working set, private bytes and handles;
macOS samples process CPU/RSS through `ps`. Unsupported counters stay null.
No collector here measures GPU/display completion. Native Windows/macOS runs
must be validated on those machines; a successful Linux run makes no claim
about their performance or collector availability.

Harness verification: `python3 -m unittest discover -s scripts/profiling -p
'test_*.py' -v`, plus the Rust scenario-driver and platform-directory tests.

## Backend measurements

```sh
python scripts/profiling/application-probe.py --profiles release --samples 35 --output target/profiling/application
python scripts/profiling/local-performance.py build --baseline ../GitComet-baseline --output target/profiling/backend
python scripts/profiling/local-performance.py measure --build target/profiling/backend --session first --pairs 3 --samples 35 --warmups 5
python scripts/profiling/local-performance.py measure --build target/profiling/backend --session second --pairs 3 --samples 35 --reverse
python scripts/profiling/local-performance.py report target/profiling/backend
```

Create the baseline at your chosen revision, for example with
`git worktree add --detach ../GitComet-baseline dev`. The paired build installs
the same standalone driver under each checkout's `target/` and freezes its
executable in the output directory. Both revisions must support the driver's
APIs and build profile. Use `build --offline` only after caching dependencies.
Report acceptance thresholds are measurement policy, not predictions of hosted
CI performance. Builds are excluded from latency samples.

Build `interaction-probe` with `cargo build -p gitcomet-git-gix --example
interaction-probe --features benchmarks --release`. Then run
`interaction-performance.py` with `--baseline`, `--candidate`, `--output` and
`--session`; choose `--operations`, `--pairs` and `--samples` as needed. It creates
disposable diff/status/push fixtures and verifies their results.

`lfs-performance.py --output PATH` creates a local HTTP LFS fixture. Configure
`--shape`, `--latency-ms`, `--workers` and `--rounds`, and optionally pass
`--backend` to include the interaction probe. No remote account is required.
On Windows, `benchmark-status-workers.ps1 -FixtureRoot PATH -OutputFile PATH
-Binary PATH` compares worker counts on disposable fixtures; select `-Workers`
and `-Rounds` explicitly when comparing policies.

## Live application on Linux

`live-ui.py` runs the ordinary application binary (native window, live store,
real workers, normal rendering) with the opt-in scenario driver
(`GITCOMET_UI_SCENARIO`, crates/gitcomet-ui-gpui/src/view/scenario_driver.rs).
The driver dispatches scripted input through production handlers on a fixed
schedule and waits for a completion witness per input; the UI probe traces
each input's stages (dispatch, store queue, reducer, worker tasks, state
publication, UI application, draw) under one operation id.

```sh
python3 scripts/profiling/live-ui.py fixture target/profiling/live-fixture
python3 scripts/profiling/live-ui.py clone ~/git/bun target/profiling/bun --revision <sha>
python3 scripts/profiling/live-ui.py run --binary target/release/gitcomet --repository target/profiling/live-fixture --scenario history-select --output target/profiling/live/select-1
python3 scripts/profiling/live-ui.py measure --baseline base/gitcomet --candidate cand/gitcomet --repository target/profiling/live-fixture --scenarios history-select diff-search --session first --pairs 3 --output target/profiling/live/s1
python3 scripts/profiling/live-ui.py measure ... --session second --reverse --output target/profiling/live/s2
python3 scripts/profiling/live-ui.py report target/profiling/live/s1 target/profiling/live/s2
```

- **Scenarios:** `startup`, `idle`, `idle-minimized`, `two-windows-idle`,
  `idle-hidden-terminal`, `history-select`, `history-select-burst` (selections
  faster than details load; superseded inputs and their worker time are
  reported), `history-scroll`, `status-save`, `status-burst` and `status-touch`
  (real writes through the native watcher; `status-touch` rewrites unchanged
  bytes), `ignored-churn` (build output in an ignored directory), `diff-search`
  (first and repeated search), `terminal-output` (output while scrolling) and
  `lifecycle` (10 warm-up plus `--cycles` open/select/close cycles of a
  `--secondary-repository`, then a plateau phase for retained memory, threads
  and descriptors).
- **Display:** each run gets a private headless mutter with one virtual
  monitor, so the window is focused and paced by a real compositor.
  `--display desktop` uses the session instead, where GNOME denies a
  background launch focus and an occluded window gets no frame callbacks.
- **GPU cache:** runs share a warm shader cache under
  `target/profiling/gpu-shader-cache`; `--cold-gpu-cache` measures a first
  launch.
- **Rejection:** a run is rejected when the app exits non-zero, a witness never
  holds, the probe drops records, or explicit visibility evidence invalidates
  the scenario. A frame waiting over a second is retained as a stall finding.
- **Output:** per-phase draw time, dirty-to-draw, wake delay, input to
  witness/draw, store/worker stage times, main-thread and per-thread CPU,
  wakeups, RSS/PSS (split into allocator heap, mapped Git packs, GPU driver
  and binary), threads and file descriptors. Linux records no present timing,
  and draw is CPU work: neither is GPU or display completion.
- **Runtime knobs:** `run --env KEY=VALUE` sets and records them;
  `measure --candidate-wrap PREFIX` / `--candidate-env KEY=VALUE` measure a
  runtime-only candidate against the same binary. `--wrap` runs the app under
  a tool: comparing two `--cycles` counts under
  `--wrap 'heaptrack --record-only -o {output}/heap'` on a build without
  mimalloc separates live-heap growth from allocator retention.
- **Pairs:** freeze (copy) both binaries first. The report refuses to combine
  sessions that ran different candidate settings.
- **Witnesses:** a witness reads the applied state after each publication, so
  a step that changes state (`command` with a `repo_closed` witness,
  `open_repo`) must be witnessed before the next step reads it.

## GUI and process captures

For a responsiveness report, first use **Settings → Environment → Copy
environment details** on the affected machine. This records the GPU and backend
selected for the application's windows, including `Hardware`, `Software (CPU)`,
or `Unavailable`. Different window configurations are listed separately. Native
macOS GPUI currently does not expose GPU or driver specs; those fields remain
`Unavailable`.

To capture the same environment with UI timings on Linux:

```sh
GITCOMET_UI_PROBE=1 \
GITCOMET_UI_PROBE_LOG=/tmp/gitcomet-ui.log \
GITCOMET_UI_PROBE_JSONL=/tmp/gitcomet-ui.jsonl \
GITCOMET_REPO_LOAD_TRACE=/tmp/gitcomet-repository.jsonl \
target/release/gitcomet /path/to/repository
```

The probe writes environment updates directly to stderr and its optional logs;
it does not require `RUST_LOG=info`. Reproduce the slow action and compare `draw`,
`submit`, and `wake` timings with the repository/process captures below. A slow
splash alone does not isolate repository loading: startup also validates Git.
An environment export identifies the renderer but does not establish the cause
of a slowdown. Collect matched traces on the affected machine before changing
renderer selection or input behavior.

Environment snapshots are cached and saved as `environment-<pid>.json` alongside
the process's crash artifacts. Recovered reports use that process's recorded
environment, even if the next launch selects another renderer. Clean shutdown
and successful recovery remove these per-process files.

```sh
python scripts/profiling/ui-responsiveness.py fixture target/profiling/ui-fixture
python scripts/profiling/ui-responsiveness.py measure --baseline /path/to/baseline.exe --candidate /path/to/candidate.exe --repository target/profiling/ui-fixture --output target/profiling/ui-session --session first --scenarios idle scroll click
python scripts/profiling/ui-responsiveness.py report target/profiling/ui-session
```

Use binaries with matching probe support. Typing, search and native move/resize
scenarios require `--native-gestures` and temporarily control focus and the pointer.
The direct `measure-ui-responsiveness.ps1` harness also supports `-InputRate`,
`-WarmupSeconds`, `-CaptureScreenshots` and `-LfsTransferReport`.
`profile-client-cpu.ps1` takes `-Repository`, `-OutputDirectory`, optional
`-Binary`, `-Scenario` and `-Seconds` for Windows process-tree CPU captures.

On Linux, use `profile-gitcomet-process-tree.sh --binary PATH --out-dir PATH
--timeout 60 /path/to/repository` for Callgrind, strace and Git Trace2 captures.
`valgrind_cdp` provides manual Callgrind control; set `CALLGRIND_OUT_FILE` to
choose its output. `build_release_debug.sh` builds the symbolized profile and
forwards additional Cargo build arguments.

## Suites and workflow timing

| Driver | Purpose |
| --- | --- |
| `run-full-perf-suite.sh` | Criterion, idle-resource and app-launch suites; `--cargo-profile` (default release) builds and freezes every executable first, and `manifest.json` accepts the run only when every selected scenario left fresh results and passed its structural witnesses. |
| `perf_metadata.py` | Records source/patch and binary hashes, toolchain, CPU governor, GPU/driver, display, Git and allocator settings; `--compare` lists differences that invalidate a pair. |
| `archive-perf-run.sh` | Run and archive a suite with metadata; forwards suite arguments. |
| `compare-perf-runs.sh` | Compare two archives with metric and regression filters. |
| `benchmark-indexed-history.py` | Paired backend probes with `--before`, `--after`, repeatable `--repository`, `--profile`, `--pairs` and `--output`. |
| `benchmark-indexed-history-frames.py` | Paired frame probes; `--case columns:pixels:scale` selects graph geometry. |
| `calibrate-indexed-history.py` | Record five accepted release Criterion roots for budget calibration. |
| `windows-workflow-probe.py` | Cache traversal and linker launch timing with `--baseline`, `--output` and `--samples`; both checkouts need the corresponding CI helpers. |

Results are labelled by what they time (sidecar `measurement.kind`):
`backend_operation`, `prepared_row_work` (row preparation, no layout or
paint: this includes the `frame_timing`, `display` and `keyboard` groups),
`gpui_test_platform_draw`, or `live_application`. Criterion and harness
binaries count every allocation, so use their timings to understand
mechanisms and confirm user-facing claims with `live-ui.py` on a release
build. Symbolized CPU profiles come from the `release-with-debug` profile,
for example `perf record -F 199 --call-graph dwarf,16384 -o cpu.data --
target/release-with-debug/gitcomet` with the scenario environment; those runs
are diagnostics, not latency evidence.

See [indexed-history measurements](../../docs/indexed-history-performance.md)
for benchmark contracts and [test-runtime measurements](../../docs/windows-test-runtime.md)
for CI execution timing. Keep machine-specific timing reports out of source control.
