# Profiling GitComet

Profiling and benchmark drivers live here. CI execution, cache management and
test-runtime reports stay in `scripts/ci/`. Application profiling runs locally;
it does not add application builds to the CI test matrix. Capture-policy and
fixture tests run in the existing CI configuration lane; deterministic history
and picker work limits also run in the performance workflow.

The [performance finalization record](../../docs/performance-finalization.md)
tracks the verified branch, framework fork ports, fresh Linux evidence and the
deferred macOS/Windows verification checklist.

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
cargo build --release -p gitcomet-git-gix --features benchmarks --example interaction-probe
cargo build --release -p gitcomet --bin gitcomet
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
Build the backend probe and app separately, as above, so Cargo feature
unification does not enable benchmark-only instrumentation in the app.

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
| Startup/history | Process spawn to first draw, usable first page/status, and fully indexed history; generated and real repositories; refs and worktree fingerprints. History interactions wait for the index and visible text, so a jump cannot pass against just the 200-row bootstrap page. |
| Recent repository filter | `repo-picker` opens both Ctrl/Cmd+Shift+A and Ctrl/Cmd+Shift+O, types names and long path components, selects all, pastes, finds no matches, and backspaces. Forty synthetic recent paths exercise truncation; every edit has a filter witness, and each sequence asserts the exact query, result count and selection. |
| Hover | Rapid row sweeps, repeated stationary pointer events, witnessed message-card dwell and idle after dwell; handler/dispatch delays, draw cost, invalidations and store/worker counts. Sweep events have no per-row visual witness, so they do not claim input-to-draw latency. |
| Scroll/drag/select | Real production input handlers and rendered scroll-position/commit-detail witnesses; frame, input, store queue, worker queue and UI apply distributions. Deep runs include distant scrollbar jumps with destination text loaded, close/reopen, stationary hover and burst selection on every corpus size and real snapshot. Burst selection also counts superseded work. |
| Clone/fetch/pull/push | CLI, backend (where available) and live store paths; resulting refs, ancestry and payload hashes. Clone uses smart HTTP, so local hardlink shortcuts cannot satisfy the test. No-op and divergent merge/rebase cases are in the deep suite. |
| LFS | Real batch/upload/download endpoints with content hashes; fetch, checkout/pull and push, including UI interaction during transfers. Deep cases add `-mixed` (half the content present) and `-warm` (all present) destinations, asserting exactly which objects transferred. |
| Annex | Real git-annex with a disposable Git peer and directory content remote; get/copy/pull/push/sync and content hashes, with cold/mixed/warm destinations in deep runs. These measure local content transport, not SSH/cloud latency. |
| Background work | Save, identical-byte touch, file bursts, ignored churn, diff search, idle, minimized and multiple-window idle. The save generator writes off the UI thread. |
| Cancellation | Stalled HTTP fetch/clone/LFS fetch, followed by continued input, cancellation acknowledgement and a usable UI. Failed/unfinished operations fail validation. |
| Retention | Ten warm-up cycles, repeated repository open/select/close, then a settling plateau; repeat at 100 and 200 cycles to distinguish retention from continuing growth. |

`--latency-ms 50 --bandwidth-mib 8` adds per-request delay and an aggregate
body-byte limit to the loopback Git/LFS server. This is a controlled application
transport, not a simulation of packet loss, TCP congestion or a production
hosting service. Annex's directory remote ignores these HTTP settings.
`--shape` selects the existing LFS payload shapes and small/large Git payloads.
For example, select `--suite deep --scenario 'transfer/*/lfs-fetch-*' --shape dense`
to compare partially populated and warm caches across CLI/backend/live paths.
Cache state refers to content at the transfer destination, not the OS page cache.
`bytes` counts verified content; `required_content_bytes` counts initially missing
content. A warm operation can verify many bytes while transferring none.
Real clone cases (`clone/chromium/live`, for example) copy the full repository
and can take hours at a low bandwidth limit. They appear in `--list` but require
an explicit `--scenario 'clone/chromium/live'` selection; raise `--timeout` as
needed. Verified payload bytes and HTTP wire bytes are separate.

Every run writes `manifest.json`, `report.txt`, `findings.json`, environment/binary identities
and case artifacts. UI cases retain scenario, frame/stage JSONL, process
samples and stderr. CLI/backend cases retain command, stdout/stderr, Git
Trace2, command-stage timings where supported, and final witnesses. Failed
transfer fixtures remain under the corpus's `.scratch` directory for inspection;
successful ones are removed unless `--keep-fixtures` is set. Do not run two
sessions concurrently against the same corpus.
`findings.json` ranks invalid captures, severe stalls and investigation candidates;
each entry includes its case, pair, build mode, artifact directory and available
phase work/CPU/wait evidence. Use the retained `scenario.json` and frozen executable
to reproduce the exact interaction before attributing a cause or changing code.
Picker phases include a 750 ms settling tail and use 250 ms probe intervals
(other scenarios use 1 s), so their short phases retain complete wake and process
samples. Captures record this cadence, and comparisons reject a cadence change.

### Regression decisions

First run an A/A check by giving the same executable as `--binary` and
`--baseline`. For a change, use frozen release executables from the two
revisions, with corresponding backend probes when selecting backend cases.
Repeat separately with normal dev executables for development responsiveness;
dev-to-dev comparisons are supported, while dev-to-release pairs are rejected:

```sh
python3 scripts/profiling/performance.py run --binary CANDIDATE --baseline BASELINE --scenario 'ui/history-20000/history-scroll' --pairs 3 --session first --output target/performance/first
python3 scripts/profiling/performance.py run --binary CANDIDATE --baseline BASELINE --scenario 'ui/history-20000/history-scroll' --pairs 2 --reverse --session second --output target/performance/second
python3 scripts/profiling/performance.py compare target/performance/first target/performance/second --output target/performance/comparison.json

# Dev A/A: build once, then measure the same binary in two sessions.
cargo build -p gitcomet
python3 scripts/profiling/performance.py run --binary target/debug/gitcomet --baseline target/debug/gitcomet --cargo-profile dev --scenario 'ui/*/repo-picker' --pairs 3 --session dev-aa-first --output target/performance/dev-aa-first
python3 scripts/profiling/performance.py run --binary target/debug/gitcomet --baseline target/debug/gitcomet --cargo-profile dev --scenario 'ui/*/repo-picker' --pairs 2 --reverse --session dev-aa-second --output target/performance/dev-aa-second
python3 scripts/profiling/performance.py compare target/performance/dev-aa-first target/performance/dev-aa-second --output target/performance/dev-aa.json
```

Pairs alternate order. Reports bootstrap **run-level paired ratios**, not
individual frames. A regression requires at least five pairs in two sessions,
a median slowdown of at least 20%, and a 95% interval excluding parity.
Fewer observations are labelled as needing confirmation. Changed binaries
between sessions, unmatched pairs, different workload/environment identities,
missing metrics, dropped records and failed witnesses cannot pass silently.
When a complete trace has zero store/worker/apply stages for an input phase,
its stage latency is not applicable. Such paired distributions appear in
`excluded_metrics` with their sample counts; work counters still compare, and
neither latency zeros nor samples are invented. Missing sampled data remains
an error.
Allocation diagnostic timings are excluded. A/A noise must be assessed before
using the 20% policy on a particular machine. Identical-binary slowdowns are
labelled `noise_alert`, not code regressions. Shared-desktop activity, thermal
state and background builds can still confound an otherwise valid capture.

Initial **release** alerts are p95 CPU draw over one frame interval (16.7 ms at
60 Hz), witnessed input-to-draw p95 over 50 ms or p99 over 100 ms, and
draw/handler/dispatch/wake/apply stalls over 100 ms. **Dev** alerts use 50 ms draw
p95, 100/250 ms input p95/p99 and 250 ms stalls. One second is severe in either
mode. These provisional thresholds live in `perf_report.py`; the running binary's
debug-assertion flag identifies the mode, independent of its filename. Intentional
tooltip dwell is excluded from immediate-input latency. Draw correlation uses
the input's window; another window's draw cannot satisfy it. Long frames remain
evidence rather than automatically being rejected as occlusion. These are
investigation targets, not universal startup/network time limits. `--strict`
also exits unsuccessfully on alerts or skipped selected cases; ordinary runs
fail on invalid cases and report optional missing tools as skipped.

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
That produces a `concurrency_not_exercised` finding, including in functional
validation runs. Increase transport delay or payload size and repeat; warm no-op
operations can finish too quickly to establish concurrent responsiveness.

For a main-thread stall, inspect a native CPU **and wait** capture: filesystem
calls, child waits, mutex contention and queue delay can be slow with little CPU.
Use operation traces to locate the interval before reading a whole flame graph.
For unnecessary work, compare `work_counts` (store/task stages), `work_units`
(history reads and topology builds, graph transitions, painted rows/paths/quads, checkpoints,
decorations, containment walks, text windows/measurements, picker model builds
and filtered items), background refreshes, invalidations and superseded tasks
between stationary hover, sweep and idle. Fine-grained work counters are enabled
by the existing operation trace, aggregated across threads without per-row JSON
records, and reported both per phase and per process. They add atomic counter
overhead while profiling. Interval snapshots retain partial work evidence if a
run times out; `work_units_complete` identifies a final snapshot, and incomplete
captures remain invalid for comparison. Counters have no allocations or locks
on the measured work path; without tracing they only check its enable flag.
Missing work instrumentation is unavailable data, not zero work. Existing
indexed-history Criterion groups provide allocation/row/graph mechanism probes;
they remain distinct from native application latency.

`log_walk_object_read` counts topology header lookups, and `log_topology_build`
counts full topology scans when no Git commit-graph is available. A bootstrap
page followed by full indexing should scan each reachable header once and reuse
that snapshot. Ref changes or object-store replacement invalidate it. The
`indexed_history_scans_headers_once_and_reuses_bootstrap_topology` regression in
the existing backend CI job enforces this work budget without timing thresholds.

The picker regression tests bound text measurements as hidden path length grows,
require one filter pass per edit and no row-model rebuilds during typing. These
structural checks run without machine-dependent timing limits. Run all Python
policy/fixture regressions with
`python3 -m unittest discover -s scripts/profiling`; missing LFS/annex tools are
reported as skipped tests.

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

For local LFS/annex metadata and commit-click costs, the same probe supports
`large-file-support`, `large-file-status` (status plus row classification), and
`commit-details`. The third argument is a commit for `commit-details` and an
unused placeholder for the other two. These report result counts as witnesses
and commands issued through GitComet's command runner. Filter processes started
inside gix are not included; use a process trace when checking all subprocess
work. For example:

```sh
target/release/examples/interaction-probe REPO large-file-support . 20 0
target/release/examples/interaction-probe REPO commit-details HEAD 20 0
target/release/examples/interaction-probe REPO large-file-status . 20 0
```

Keep the first sample to assess a cold backend handle; subsequent samples reuse
that handle. This does not flush the operating system's filesystem cache.

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

## Multiple native windows

`multi-window.py` measures several populated windows in **one application
process**, using the production new-window, focus, and close paths. It freezes
the binary, isolates the application profile, verifies native focus and loaded
history, and checks that selecting commits in each window leaves the others'
selections unchanged. It does not change worktree files.

```sh
# Repeat with --repositories /path/to/git alone to compare copies of the same repo.
python3 scripts/profiling/multi-window.py --binary target/release/gitcomet \
  --repositories /path/to/git /path/to/bun /path/to/history-100000 /path/to/history-20000 \
  --windows 1 2 4 --samples 3 --output target/profiling/windows-different

# Exercise the foreground window while a large repository opens in another.
python3 scripts/profiling/multi-window.py --binary target/release/gitcomet \
  --repositories /path/to/git /path/to/history-2000000 --mode background \
  --samples 3 --output target/profiling/windows-background

# Compare 10 and 30 cycles in separate output directories after three warmups.
python3 scripts/profiling/multi-window.py --binary target/release/gitcomet \
  --repositories /path/to/git /path/to/history-20000 --mode lifecycle \
  --cycles 10 --samples 3 --output target/profiling/windows-close-10
```

The matrix alternates window-count order between repetitions and measures idle,
commit selection, hover, scrolling, then selection in every secondary window.
`results.json` and `report.txt` contain phase latency, CPU, PSS, thread/descriptor
counts, and per-window draw counts (including zero draws in inactive windows).
State-publication timing is scoped to the input's window; stores have independent
sequence numbers. Repository-open latency is kept separate from keystroke budgets.

Use a release binary and a quiet machine for timings; mark driver smoke checks
with `--purpose validation`. Default headless runs use hardware rendering through
an isolated Mutter compositor on one monitor, with overlapping windows. They do
not establish rendering performance for several fully visible monitors, other
graphics backends, or other operating systems. CPU draw completion is not display
completion. Background runs require traced history loading to overlap input in
the other window; a fast repository with no overlap fails validation. Growing
RSS/PSS after close alone does not prove a leak; compare longer cycle runs, live
allocations, and surviving threads/descriptors.

For live-allocation diagnostics, build with `--features perf-alloc` and repeat
the lifecycle case with `--purpose diagnostic`. Use its allocation counts, not
its instrumented timings, to distinguish live Rust allocations from retained
allocator pages. `--env KEY=VALUE` records application-only runtime experiments,
for example `--env MIMALLOC_ALLOW_THP=0`; keep those results separate from the
default allocator measurements.

## GPU utilization and memory

Add `--gpu` to `live-ui.py run`, `live-ui.py measure`, or `multi-window.py` to
capture GPU telemetry alongside the existing CPU/input traces:

```sh
python3 scripts/profiling/multi-window.py --binary target/release/gitcomet \
  --repositories /path/to/git /path/to/bun /path/to/history-100000 /path/to/history-20000 \
  --windows 1 2 4 --samples 2 --inputs 600 --idle-seconds 10 --gpu \
  --output target/profiling/windows-gpu
```

On **Linux with NVIDIA's `nvidia-smi`**, the collector runs two
persistent external monitors at one-second intervals, without adding work to the
application's main thread. It records:

- **GitComet process:** SM activity, memory-engine activity, framebuffer memory,
  and encoder/decoder activity when the driver provides them. All GitComet
  windows in the process are included. This is not attribution to each window.
- **Isolated Mutter compositor:** the same counters, under its own PID. Desktop
  captures do not attribute the shared desktop compositor to GitComet.
- **Whole device:** GPU/memory activity, used/total VRAM, board power and
  temperature, graphics/memory clocks, and performance state. These include
  other applications; board power is not app power. Clock changes can explain
  utilization differences between otherwise identical captures.

`gpu.jsonl` contains timestamped samples, `gpu-capture.json` records collector
status and commands, and `summary.json` contains per-phase `gpu.processes` and
`gpu.devices` distributions with sample/missing counts. The multi-window text
report includes app, compositor and whole-device metrics separately. NVIDIA
labels `pmon` framebuffer quantities MB, preserved as `framebuffer_mb`; device
query memory is reported in MiB. Memory-engine busy percentage is activity, not
the fraction of VRAM occupied.

The collector follows [NVIDIA's process-monitoring semantics](https://docs.nvidia.com/deploy/nvidia-smi/index.html#process-monitoring).
A dash/unsupported counter is **missing**, never zero, even for an idle process.
`pmon` timestamps have one-second precision, so utilization uses conservative
interval bounds and excludes buckets that cross a phase boundary. The first
unprimed interval is excluded too. Short phases may have no usable samples;
prefer phases lasting at least five to ten seconds. Means use only available
driver samples, and do not fill unreported intervals with zero.

On macOS, `--gpu` uses IOKit and IOReport instead, without root:
- each process's Metal GPU time (`accumulatedGPUTime` of its AGX clients), for
  GitComet and for WindowServer;
- whole-device utilization;
- whole-GPU energy and performance-state residency.

`perf_macos.py` has the details. `macos-usage.py` runs the hover, scroll,
selection, diff-open, repo-tab and idle set with interleaved repeats, and
`macos-usage-report.py` prints GPU ms/s, GPU ms per frame, GPU mW and the mean
P-state per phase. See [macOS CPU and GPU usage](../../docs/macos-gpu-usage.md)
for the method and results. Launch the executable inside an app bundle
(`GitComet.app/Contents/MacOS/gitcomet`). A bare binary relaunches itself from
a bundle and exits.

These measurements describe coarse GPU utilization and memory, not GPU duration
per frame or display completion. GPU monitoring is opt-in; enable it on both
sides of a timing comparison. Unsupported GPUs/operating systems produce an
explicit unavailable result rather than fabricated counters. Windows PDH collection
supports AMD/Intel/NVIDIA drivers exposing the standard counters; macOS GPU
occupancy collection remains unavailable.

To investigate pixel-dependent costs, repeat the same workload at different
window sizes. `multi-window.py` and `live-ui.py run` accept
`--window-size WIDTH HEIGHT`; the headless monitor grows to fit. For example:

```sh
python3 scripts/profiling/multi-window.py --binary target/release/gitcomet \
  --repositories /path/to/git --windows 1 --samples 2 --inputs 600 \
  --idle-seconds 10 --gpu --window-size 2560 1440 \
  --output target/profiling/gpu-large
```

Repeat with `multi-window.py --hide-graph` to separate graph painting from the
rest of the interface. Both settings apply only to the disposable session and
are recorded with the capture. Hiding the graph does not necessarily release
path textures: window decorations can use the same renderer resources.

### Paired history, diff and repository-tab captures

`gpu-interactions.py` compares two frozen release binaries using the production
input handlers. Supply two disposable repositories; the first needs at least
two unstaged tracked files. It sweeps history rows, switches between those diffs
and alternates repository tabs. Diff-loaded and repository-open witnesses must
complete, and binary/fixture fingerprints must stay unchanged. It uses isolated
profiles and a private headless Mutter display; it does not edit the fixtures.

```sh
python3 scripts/profiling/gpu-interactions.py \
  --baseline /path/to/frozen-original --candidate /path/to/frozen-optimized \
  --repository /path/to/modified-fixture --secondary-repository /path/to/second-fixture \
  --sizes 1400x900 2560x1440 --pairs 3 --seconds 30 --changes-per-second 20 \
  --output target/performance/gpu-interactions
```

Pairs alternate execution order. `report.json` contains every run and median
per-run GPU activity, VRAM, clocks and CPU/input timings. Raw counters, frame
records, scenario steps and environment metadata remain in each capture.
Insufficient samples fail the capture. GPU reductions are reported only when
graphics and memory clocks agree within 5%; these are process SM activity
percentages, not GPU execution times. Repeat with `--reverse` in a fresh output
directory to confirm the result, and with `--clean` to check CPU/input timings
without vendor monitoring. `--scales 100 125 200` changes application UI scale;
native display scaling requires separate validation.
`--refresh-hz 144` selects a 144 Hz private display; its default is 60 Hz.
Compare the same display rate within a pair. No renderer experiment override is
set by this harness; the pinned renderer uses its normal Linux path cropping.
The default switch rate is 10 per second. `--changes-per-second` selects a
sustained stress rate; increase duration when the driver leaves missing buckets,
and keep the rate identical within a comparison. Missing counters stay missing.

The connector regression compares actual hardware-rendered pixels against the
previous drawing behavior. It is explicitly selected because normal unit tests
must not require a GPU:

```sh
cargo test -p gitcomet-ui-gpui --lib pixel_aligned_connectors_match_original_gpu_paths -- --include-ignored --nocapture
```

Application window frames omit shadows. The native connector comparisons cover
fractional scales, endpoints, clipping and alpha. GPUI clips quad and shadow
vertices to their content mask, since fragment masking alone still rasterized
and blended transparent rectangles. In a gpui-ce checkout,
`cargo test -p gpui_ce_wgpu --features test-support --lib vertex_clipping`
checks that clipping against unmasked frames. The Linux captures below used GPUI
`f7474ab` with the shadow clip applied as a local renderer patch, since removed.
Straight graph connectors use ordinary quads only when every physical edge is
aligned and the stroke is fully inside a mask without fades. Other connectors
retain the previous path rasterization and antialiasing.

### Linux shadow and connector results (2026-10-11)

These measurements describe the earlier frozen candidate with clipped window
shadows and connector quads. Window-frame shadows were subsequently removed
entirely at the user's request. The numbers below apply to that earlier
candidate; measurements of the shadowless frame follow in the next section.
The obsolete edge-band renderer and shadow-only tests/benchmark were removed.

The [Linux evidence record](gpu-efficiency-linux.json) preserves the frozen
binary/source hashes, fixture fingerprints, individual 2560×1440 runs, both
sessions, other window sizes, clock comparisons and validation results. Raw
captures and frozen binaries remain under `target/performance/gpu-efficiency/`.
Windows and macOS results will be supplied separately for consolidation.

Release builds used the same GPUI pin, NVIDIA GTX 1080 / driver 580.178.04,
Vulkan/Wayland, a private 60 Hz Mutter display and 100% application UI scale.
Each active phase ran for 30 seconds: 125 pointer events per second, or 600
witnessed diff/repository changes at 20 per second. No runtime experiment
overrides were set. These are sustained interaction measurements.

Initial-session medians of three paired runs at **2560×1440**, using application
process [SM utilization](https://docs.nvidia.com/deploy/nvidia-smi/index.html#process-monitoring):

| Interaction | Original GPU activity | Optimized GPU activity | Reduction |
| --- | ---: | ---: | ---: |
| History hover | 15.33% | 9.00% | 41.3% |
| Diff changes | 12.37% | 7.61% | 38.5% |
| Repository tabs | 7.48% | 4.93% | 34.1% |

Graphics and memory clocks matched at 1240 / 5005 MHz in that session. Two
additional pairs in a second session confirmed savings in every interaction.
Across all five pairs, the median paired reductions were **41.1% hover, 34.3%
diffs and 34.1% tabs**. The exact run-pair bootstrap 95% intervals were 39.9–43.9%,
32.4–40.5% and 29.0–48.7%, respectively. These intervals describe this small
host/workload sample, not variation across hardware.

Hover/diff GPU framebuffer usage fell from 117 MB to 85 MB in the first session.
The history graph's path count fell from one per frame to zero for this fixture;
the equivalent connector became a quad. Repository tabs can still retain path
textures for their curved outlines. Input counts, completion witnesses and
frame counts stayed comparable. Instrumented first-session diff draw p95 was
7.53 → 7.56 ms and witnessed input-to-draw p95 was 17.98 → 17.93 ms; tab values
were 8.26 → 8.11 ms and 14.20 → 13.02 ms. Hover has no per-row completion witness.

At 1400×900, the initial session measured 31–35% lower process activity. At 4K,
the builds boosted to different GPU clocks; the record retains raw activity
and clock data without attributing a percentage reduction. The isolated frame
diagnostic reduced submission-plus-GPU-completion p50 from 1.18–1.36 ms to
0.61–0.64 ms; it is not a pure GPU execution timer.

Validation passed: 4,644 UI tests, 34 renderer shader/ABI tests, 41 graph tests,
four shadow tests and 21 GPU collector tests. The native image tests performed
336 comparisons with at most one channel value of difference. All 22 completed
application captures were valid, with 26,400 witnessed diff/repository inputs.
The record identifies the production and test source hashes of that frozen
optimized release, before window-frame shadow removal.

The extended dense-merge queue was interrupted when wrap-up was requested;
its partial capture is excluded. Separate clean CPU/input and 144 Hz performance
captures were not run before wrap-up. Native fractional display scales are
represented in connector pixel comparisons, rather than live performance data.
Linux results establish no Windows or macOS performance claim.

### Linux results after removing the window-frame shadow (2026-10-11)

The shadowless release was measured against both the earlier optimized release
and the original baseline. The renderer clipping patch and graph connector
optimizations were retained. Fourteen native application captures used the same
GTX 1080, driver, disposable repositories, 2560×1440 window, private 60 Hz display,
100% application UI scale and 30-second interaction phases described above.
Every capture passed, including all **16,800 witnessed diff/repository switches**.
Paired frame counts differed by less than 0.5%.

The [Linux evidence record](gpu-efficiency-linux.json), under
`window_frame_shadow_removal`, contains every run, source/binary fingerprints,
clock exclusions, individual paired ratios and timing distributions. The frozen
current binary is `target/performance/gpu-efficiency/binaries/candidate-shadowless`
(SHA-256 `df71e1abc0646f2d7bf4a2218ad1af1b65c6d7e97ea694dc16c3a7be6e33815d`).
Its source patch and archive are stored alongside it. Raw captures remain in
`paired-shadowless-frame`, `paired-shadowless-baseline` and
`paired-shadowless-baseline-confirm` under the same artifact directory.

**Effect of removing the frame shadow**, with three clock-comparable pairs:

| Interaction | Optimized with frame shadow | Shadowless | Paired activity reduction |
| --- | ---: | ---: | ---: |
| History hover | 7.81% | 6.68% | 5.7% |
| Diff changes | 6.71% | 7.48% | −2.2%; no consistent gain |
| Repository tabs | 4.54% | 4.00% | 11.8% |

Activity columns show medians of per-run process SM means. Reduction columns
show medians of individual paired ratios; each pair's values are preserved in
the record. The run-pair bootstrap 95% intervals for the reductions are
5.4–18.0% for hover, −26.5–0.3% for diffs and 9.7–18.6% for tabs. The diff results
do not support an additional saving from frame-shadow removal. Framebuffer
usage was unchanged from the optimized shadow-enabled release.

**Total improvement from the original build**:

| Interaction | Original | Current shadowless release | Paired activity reduction |
| --- | ---: | ---: | ---: |
| History hover | 13.43% | 7.32% | 46.9% |
| Diff changes | 10.44% | 6.56% | 38.6% |
| Repository tabs | 6.45% | 4.38% | 32.1% |

All attributed comparisons passed the 5% graphics/memory-clock threshold. The
last pair of the initial baseline repeat boosted to different clocks for hover
and tabs. Those two phase comparisons were retained and excluded from the
aggregate savings, then a confirmation pair was run in reverse order. Hover
and tabs therefore use three clock-comparable pairs; diffs use four. Their
run-pair bootstrap 95% intervals are 39.1–47.7%, 35.2–40.8% and 17.1–46.5%,
respectively. Hover/diff framebuffer usage remains **117 → 85 MB** relative to
the original; tab framebuffer usage remains 117 MB.

These percentages measure application-process SM activity from vendor sampling.
They describe this small Linux host/workload sample. GPU execution timestamps,
separate clean CPU/input timings, 144 Hz performance captures and Windows/macOS
results were not added by this repeat. The release build and all 21 targeted
frame, resize, focus and component tests passed.

The Linux captures above used GPUI revision `f7474ab`; the consolidated
workspace uses a newer pin with the Windows diagnostics and renderer
optimizations described below. Archived measurements describe their frozen
binaries. For new captures, verify the backend capability and active settings
rather than assuming support from an older revision.

## Windows GPU and native hover measurements

### Application defaults and runtime overrides

Normal Windows `gitcomet.exe` startup supplies these defaults before initializing
GPUI. It sets each variable only when that variable is absent; an explicit value,
including an empty value, is preserved.

| Variable | Application default | Effect |
| --- | --- | --- |
| `GPUI_GPU_EXPERIMENTS` | `partial-redraw,retained-copy,cropped-paths` | Retained damage redraw on both Windows backends; cropped paths and retained texture copy on supported DX12 targets. |
| `GPUI_WINDOWS_INPUT_RATE_KEEPALIVE` | `off` | Avoid unchanged input-rate presentations while retaining dirty, forced and required presentations. |
| `GPUI_WINDOWS_DX12_PRESENT_MODE` | `immediate` | Use the measured DX12 presentation policy. |

The renderer preference is unchanged: **Automatic** tries DirectX 12 and falls
back to DirectX 11; the saved DirectX 11 preference and an explicit
`GPUI_WINDOWS_RENDERER` override still apply. Native DirectX 11 uses
`partial-redraw`; it does not implement `retained-copy` or `cropped-paths`.
Retained copy remains guarded by the actual target format, copy support and alpha
mode. Unsupported scenes, target changes or retention budgets use a full redraw.

In earlier experimental builds, `GITCOMET_GPU_GRAPH_QUADS=1` separately replaced
axis-aligned connector paths with butt-cap quads. The consolidated implementation
uses the pixel-aligned optimization without that environment flag.

The `GPUI_GPU_EXPERIMENTS` name is retained for compatibility, but the tokens above
are now application defaults. Pass an explicit empty value to disable those
optimizations for a baseline, or an explicit comma-separated set to isolate
them. Set `GPUI_WINDOWS_INPUT_RATE_KEEPALIVE=auto` to restore the previous
refresh-dependent policy, or `on` to require input-rate keepalive. Profiling
runners pass explicit runtime settings for each side; their baseline is therefore
distinct from an ordinary launch with the new defaults. Freeze a newly built
binary after changing defaults; archived captures retain their original policy.

GPU timestamp queries, UI probes, scenarios and pixel-removal ablations remain
opt-in. The defaults do not enable `GPUI_GPU_TIMINGS`,
`GPUI_GPU_DIAGNOSTIC_ABLATIONS` or `GITCOMET_GPU_GRAPH_QUADS`;
`clipped-quads` remains disabled. Pixel-aligned straight graph connectors now
use quads by default; fractional, clipped and faded connectors retain paths.
Cheap scene/submission context needed for retention does not enable timing
queries or detailed observer journals.

`live-ui.py --gpu` now collects Windows PDH GPU Engine and GPU Process/Adapter
Memory counters on a background thread at one-second intervals. It keeps the
GitComet PID, same-session DWM PID and whole adapter separate. Per-process busiest
engine occupancy is not GPU power, shader throughput, or execution milliseconds.
Shared memory is never summed across processes. Invalid/unsupported counters stay
missing. Phase occupancy summaries exclude the first unprimed rate sample and
intervals crossing either phase boundary, with valid sample counts, covered time
and duration-weighted means. Memory samples use their collection timestamps.
Memory-only adapter records remain in memory summaries and are excluded from
engine occupancy; an observed invalid engine still makes occupancy unavailable.
DWM discovery uses Toolhelp names and a WTS session fallback when protected DWM
denies session lookup. The collector records its PID, session and missing reason.
Root-cost and formal reports expose app, DWM and whole-device occupancy separately.
Each is the busiest observed adapter's duration-weighted engine occupancy, never
a sum across processes or GPUs. App occupancy remains the acceptance counter;
DWM/device rankings are descriptive desktop context and cannot attribute work
to GitComet. Missing or invalid active-adapter counters remain unavailable.
Windows GPU clocks, thermals, HAGS and power policy may need separate recording;
unavailable metadata never establishes identical operating conditions.

A release binary with the new GPUI diagnostic instrumentation supports
`--gpu-timings`. The `renderer_capabilities` event is the authority for each
backend's timestamp support and implemented experiments. Unsupported timestamp
queries, disjoint/error queries, readback ring exhaustion, incomplete shutdown
queries and probe drops are explicit failure statuses. `gpu_frame` uses the
original submission timestamp even when readback completes in another phase.
`submission` records every presentation, including reused scenes; `draw` and the
legacy `submit` records alone cannot count input-rate keepalive work. Summaries
join exact window/submission IDs and require complete coverage.

Measured GPU render elapsed spans the first through last pass timestamps and can
include gaps. `render_elapsed_ms_per_second` totals every timed presentation in a
phase. It is a useful local rendering metric. It is not the union of GPU engine
execution, nor display completion. The latter require GPU ETW execution intervals
and PresentMon/WPR display events. Access-denied or absent external collection
must remain unavailable. No collector substitutes zero for these metrics.

`live-ui.py run --display desktop --input-route native` replays Windows pointer
moves with `SendInput`, preserving the synthetic route for controls. The app
writes physical screen coordinates and operation sequence requests; the Python
worker requires that HWND to be visible, foreground and owned by the measured
process. It never steals focus. Tagged native arrivals distinguish replay input
from ordinary user input. Each request has a flushed ACK with QPC timestamps.
The marker fits in the preserved low 32 bits of Windows mouse extra info:
`0x4743` occupies the upper 16 bits and sequences occupy the lower 16 bits.
Sequences must be 1..65535; overflow fails instead of aliasing earlier input.
The app schedules requests independently of painting, so native coalescing is
observed instead of forcing a paint after every move. The final requested history
hover must settle. History `move_pointer` steps use `hover_rows: true`.

Causal `hover_target`/`hover_paint` witnesses validate the requested row and clearing
of the old hover, then join the painted scene revision to its actual submission.
Same-target unchanged moves and inputs explicitly superseded before paint are
counted separately. A delivered native event that never targets a history row
fails coverage; only absent native arrivals can be counted as OS coalescing.
`native_dispatch_to_submit_ms` uses the probe's bracketed QPC anchor and the
SendInput QPC timestamp, using the conservative latency bound; anchor uncertainty
is retained. This is immediate row
feedback latency to submission. Tooltip dwell is a separate intentional delay.
Displayed latency is still unavailable until a native display trace is joined.

The Windows matrix isolates the `dx11`/`wgpu-dx12` choices, WGPU
`cropped-paths`, `partial-redraw`, and
`GPUI_WINDOWS_INPUT_RATE_KEEPALIVE=off`. The diagnostic `fifo-dx12` variant
sets `GPUI_WINDOWS_DX12_PRESENT_MODE=fifo`; its actual capability must report FIFO.
Baseline DX12 explicitly requests immediate and disables renderer experiments. A
capability record must confirm every requested renderer experiment. Support
alone does not establish activation. Current gates require `active_experiments`
to equal the requested set even when timestamp queries are disabled. Older
`shared-resources`, `cached-layers`, `batched-paths` and `pooled-targets` matrix
claims do not describe the current dependency pin and have been removed from the
available matrix. Historical results below describe their archived binaries only.

To isolate the dominant draw cost, both Windows backends implement the opt-in
`GPUI_GPU_DIAGNOSTIC_ABLATIONS` controls `no-mono-glyphs`,
`no-subpixel-glyphs`, `no-shadows`, `no-paths`, `no-quads`, `no-images`, and
`clear-only`. Use comma-separated tokens and compare one category at a time.
`clear-only` still clears and presents the window; `no-paths` removes path
rasterization, resolve, and compositing together. Capabilities report supported
and active diagnostics, and every GPU timing event records the active set.
Diagnostics bypass retained frames. These captures deliberately omit pixels and
cannot pass visual or hover latency acceptance. Corroborate their timing results
with PDH captures that disable GPU timestamps, since different query counts can
change observer overhead. All diagnostic controls are disabled by default.

`windows-hover-root-cost.py` is the tracked root-cost screening runner. It
compares renderer choices, keepalive, retained damage, graph visibility, width,
FIFO, existing WGPU cropped paths and individual pixel-removal controls. The
`crop-dx12`, `crop-keepalive-dx12` and `crop-damage-keepalive-dx12` variants isolate
crop alone and its combinations. The default-off `clipped-quads`
prototype uses ordered hardware scissors around unchanged quad vertices and
shader interpolation; measure and validate it before relying on it. Query-enabled
GPU events report actual `quad_draw_calls` and additional `quad_scissor_splits`.
Phase summaries include means, complete sums/rates and explicit counter coverage;
missing older fields remain unavailable rather than zero. Only fully covered
counters produce a phase total or rate.

New renderer capabilities report the actual `surface_alpha_mode`; GPU events
report `damage_fallback_reason` separately from timestamp errors. Summaries keep
each reason's count, explicit nulls, missing legacy fields and coverage. A null
reason means the renderer reported no fallback; an absent field does not.
Query-free occupancy runs cannot supply per-frame damage fallback counts.

The caller must declare the full GPUI revision used to build the frozen binary;
the runner verifies that checkout and freezes its patch/untracked content hashes,
GitComet source, fixture and executable before and after every capture. This
declaration is build provenance supplied by the caller, not compiler attestation.

```powershell
python scripts/profiling/windows-hover-root-cost.py --binary C:/path/gitcomet.exe --repository C:/path/history --output target/profiling/root-cost --gpui-path C:/path/gpui-fork --expected-gpui-head FULL_40_CHARACTER_BUILD_REVISION --variants baseline-dx12 no-glyphs-dx12 no-quads-dx12 clear-only-dx12 --mode diagnostic --input-route native
```

Run `--mode occupancy` for the same PDH/native/scene/submission checks with GPU
timestamps disabled, to corroborate apparent ablation savings without differing
query counts. `--input-route synthetic` supplies a separate renderer-isolation
control, with separate experimental rankings and no latency acceptance; it
cannot be combined with clean latency mode. All routes report actual HWND pixel
size and DPI. Native is required for the formal matrix. `--list` only prints the
planned work. Short protocol debugging can reduce warmup/sweep/tail durations,
but only relaxes the minimum PDH interval count; missing occupancy, unsupported
flags, native ACK/row witness failures and missing GPU joins still fail.

Run five A/A pairs before A/B. Split each stage into independently launched
sessions of three and two pairs, with alternating order. Use the same frozen
release binary, fixture, source patch, display/scale and workload in all stages:

```powershell
python scripts/profiling/windows-hover.py measure --binary C:/path/gitcomet.exe --repository C:/path/history --output target/profiling/hover-aa-first --stage aa --session first
python scripts/profiling/windows-hover.py measure --binary C:/path/gitcomet.exe --repository C:/path/history --output target/profiling/hover-aa-second --stage aa --session second
python scripts/profiling/windows-hover.py measure --binary C:/path/gitcomet.exe --repository C:/path/history --output target/profiling/hover-ab-first --stage ab --session first
python scripts/profiling/windows-hover.py measure --binary C:/path/gitcomet.exe --repository C:/path/history --output target/profiling/hover-ab-second --stage ab --session second
python scripts/profiling/windows-hover.py report target/profiling/hover-aa-first target/profiling/hover-aa-second target/profiling/hover-ab-first target/profiling/hover-ab-second --output target/profiling/hover-report.json
```

The window becomes active before a ten-second warmup, then receives thirty
seconds of history-viewport row moves and a five-second tail. Each pair has
separate diagnostic GPU/PDH and clean native latency runs. Clean
runs keep causal probe records but disable GPU queries/PDH observers. GPU ranking
requires both total query render elapsed and per-process PDH busiest-engine
occupancy to improve. Each uses paired run-level log ratios, seeded bootstrap
95% intervals, a reduction larger than the A/A noise envelope and the same
direction in both sessions. There
is no blanket ten-percent threshold in this matrix: cheap, reproducible smaller
gains can pass. Native submission latency guards require p95 regression below
`max(5%, 1 ms)` and p99 below `max(5%, 2 ms)`, plus absolute 50/100 ms caps. Missing
causal or GPU coverage fails closed. Session environment differences and clock/
thermal drift must be reviewed before relying on a local rank.
Formal diagnostic hover captures requesting `partial-redraw` must observe actual
Partial frames and a positive measured retained allocation peak. Capability
activation, reuse-only captures and Full-only fallbacks do not establish the
retained hover optimization. Clean latency runs are unaffected; the root-cost
screen still accepts truthful Full-only captures as diagnostic evidence.

A passing local result is `local_gpu_and_submit_gates_passed`, accompanied by
`display_validation_pending`; enabling application defaults does not establish
displayed latency or representative low-end performance.
Verify displayed latency, pixels, resize/DPI/occlusion/device recovery and resource
lifetime on affected native backends, then repeat on representative low-end
hardware before making broader performance claims. Use `validate-gpu.py --stage tests` for native
renderer checks. Every cargo test command must execute at least one passing test;
a successful command with a stale filter and zero tests now fails validation.
`gpu-matrix.py` retains simpler legacy per-frame gates for cross-platform
exploration; use `windows-hover.py` for Windows A/A noise, all-presentation GPU
work and native row latency decisions.

Each session freezes `environment.json` and a readable `environment.txt` with
the binary hash, tracked patch hash, content hashes of untracked sources, Windows
active base power plan/effective power overlay, driver, nominal display refresh
and collector DPI. Native replay also records actual app HWND physical client
dimensions/origin and DPI. Missing API fields remain explicit. Nominal refresh
does not establish VRR state, and GPU clocks/thermals/HAGS still need a separate
collector. Preserve the separate GPUI checkout's source identity as well as the
GitComet source identity when using a local dependency checkout.

Measure observer overhead before interpreting small GPU gains. A previously
validated native run provides the physical history sweep for an external replay:

```powershell
python scripts/profiling/windows-hover-occupancy.py pilot --binary C:/path/gitcomet.exe --repository C:/path/history --validated-run target/profiling/valid-native-run --output target/profiling/observer-pilot
python scripts/profiling/windows-hover-occupancy.py run --binary C:/path/gitcomet.exe --repository C:/path/history --validated-run target/profiling/valid-native-run --output target/profiling/shipping-occupancy --variant dx11
```

The pilot runs three alternating shipping/probe pairs, then three alternating
probe/probe-with-timestamps pairs. All use the same external 8 ms sweep and PDH
collector: ten-second warmup, thirty-second hover and five-second tail. The
shipping occupancy route clears inherited probe, scenario, optrace and timestamp
settings. It requires the same frozen binary/fixture, matching physical client
size/DPI/monitor and an already-foreground HWND owned by the newly launched app.
It translates previously validated points by client origin and stops on focus or
geometry changes. It never activates another window. It requires at least 25
fully contained valid PDH intervals and 80 percent hover coverage.

This route provides occupancy evidence only. With app observers disabled it has
no fresh ready/row/submission witness or latency metric, and cannot satisfy the
causal, displayed-latency or shipping acceptance gates. Three-pair bootstrap
results assess observer overhead as a pilot; formal decisions still require the
five-pair A/A and A/B sessions above.
The observer pilot uses external replay in every mode. Its pairs isolate the
probe/trace and timestamp factors, but do not quantify the app scenario's native
request/ACK and causal-witness overhead in the root-cost runner. Probe modes
require actual backend, active experiment set, empty diagnostic controls,
keepalive/present policy and timestamp activation to match the requested runtime.

### Linux comparison of GPUI 325cedb (2026-10-05)

The requested `325cedb8a5c57104578f557d4652b1e5800e01b3` was compared with the
current `2ba9c0718644c28852f898402d92b40ba5cc4b57`. Both release executables
include the repository-sharing review fixes and the compact text patch. Only
the GPUI revision, its profiler implementation and the reported revision
constant differ in executable sources. The candidate was built in an isolated
worktree; the main workspace dependency pin was preserved.
This four-window workload confirmed that the new scratch-pool expiry releases
unused textures, but did not establish a meaningful Linux gain with shipping
defaults. The revision was not promoted to the main workspace.

Five alternating pairs per setting use four windows on the same 204,001-commit
repository, at 1400 x 900 in isolated Mutter Wayland sessions. The host has a
Ryzen 9 5950X and a GTX 1080 using hardware Vulkan, NVIDIA 580.178.04. Settings
are all experiments disabled, and
`cropped-paths,shared-resources,pooled-targets` enabled together. Connector quads
remain disabled. GPU diagnostics use 240 selection/scroll inputs and 480 hover
moves; separate clean runs use 120 inputs and 240 hover moves without GPU
queries, vendor monitoring or allocation instrumentation. Each additional
window receives 60 selections after native focus changes.

All 40 captures passed the scenario and window-isolation checks. The 20 clean
captures passed every `max(5%, 1 ms)` input-to-draw, input-to-submit and CPU draw
p95 guard for both revision comparisons. Comparing the candidate with and
without the flags also passed those clean guards. Values below are medians of
the five per-run measurements, shown as baseline to candidate:

| Setting | Selection input-to-draw p95 (ms) | Scroll input-to-draw p95 (ms) | Hover draw p95 (ms) | Settled process PSS (MiB) |
| --- | ---: | ---: | ---: | ---: |
| Default | 18.99 → 18.93 | 17.04 → 17.08 | 1.55 → 1.56 | 538.1 → 534.1 |
| Cropping + sharing + pooling | 18.97 → 18.95 | 16.88 → 17.12 | 1.52 → 1.58 | 530.6 → 526.6 |

Startup to indexed readiness was 813 → 815 ms at defaults and 815 → 811 ms
with flags. The roughly 4 MiB PSS median differences have overlapping run
ranges; they do not establish a consistent memory reduction. File descriptors
remained at 134 for both revisions/settings. The raw runs and all focus-phase
guards remain in the analysis.

All 20 GPU captures completed. Across their 120 active phases, 20,817 frames
had timestamps: coverage was 100%, every phase had at least 110 timed frames,
and there were no query drops, record drops or submission gaps. Optional device
retention peaked at 13.18 MiB, below the 32 MiB limit. The allocation estimates
show a concrete pooling improvement: the current revision still retained
13.18 MiB at the end of selection and at the final settled frame sample in
every run, while the candidate retained zero in all five runs. The new expiry
therefore releases unused scratch textures as intended. Both revisions had the
same peak retention, and driver-reported app framebuffer memory remained
205 MB with pooling enabled. This benefit requires the opt-in pool; default
rendering retained no optional textures on either revision.

Automatic GPU clocks prevent attributing the raw timing differences to the
revision: memory clocks alternated between 405/810 and 5,005 MHz in both
executables. For example, default hover time was about 5.54 ms at the lower
memory clock and 1.32 ms at the higher clock for both revisions. The analysis
retains every capture and fails the GPU comparison when paired graphics or
memory clocks differ by more than 5%. The following high-clock hover groups
are descriptive only, with two baseline and three candidate samples; they do
not replace the five-pair acceptance gate.

| Setting | Baseline hover GPU p50 | Candidate hover GPU p50 | Baseline app framebuffer | Candidate app framebuffer |
| --- | ---: | ---: | ---: | ---: |
| Default | 1.319 ms | 1.319 ms | 269 MB | 269 MB |
| Cropping + sharing + pooling | 0.917 ms | 0.915 ms | 205 MB | 205 MB |

Hover groups use steady sampled graphics/memory clocks of 1,721/5,005 MHz.
Framebuffer values are medians across all five settled runs per side, in
NVIDIA `pmon`'s reported MB units. The memory reduction from these existing
flags is present in both revisions. Process SM activity also differed with
clocks and does not establish reduced work. Missing utilization samples in
short phases stay unavailable; whole-device utilization and board power are
not attributed to GitComet.

The candidate passed 51 shared renderer tests and all four upstream source
audits. Its native wgpu suite passed 54 tests, ignored one, and failed three
existing alpha-contract cases, which also failed on the current revision:
`quad_backgrounds_match_legacy_metal_pixels`,
`quad_border_backgrounds_use_the_fill_coordinate_space`, and
`underline_opacity_is_applied_once`. New optimization fixtures and resource
lifetime tests passed. macOS font loading/Metal shader startup and native
Windows behavior were not measured on this host. Frozen macOS controls and
shipping GPU/allocator defaults remain unchanged.

Captures, native test logs, source archives, hashes and frozen executables are
under `target/performance/gpui-325cedb/`. `measurements.json` preserves the
individual runs; `analysis.json` includes clean latency comparisons, memory,
GPU coverage/budgets and clock diagnostics. `measure.py` and `analyze.py` in
that directory reproduce the comparison with the frozen executables.

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
