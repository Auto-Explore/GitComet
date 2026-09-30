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
  holds, the probe drops records, or a frame waits over a second to draw.
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
