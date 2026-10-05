# Performance finalization

The application series is on `perf/verified_improvements`, based on GitComet
`dev` at `6b4eed45c6c141376fd494ee615d415525552989`. Framework changes are in
[`Havunen/gpui-ce`](https://github.com/Havunen/gpui-ce), on the same branch name,
based on its updated `upgrades` at `0b416666cf8cb67ded66a9fc093a2e674c40edd0`.
GitComet's four GPUI dependencies and its exported `GPUI_REVISION` pin
`80c32256b87ff82b13373b297388ea730c5da4df`. There are no local GPUI path patches
or vendored GPUI crates. Existing tree-sitter grammar vendoring is unrelated.

This application finalization ran on Linux. **Fresh native GitComet macOS and
Windows checks remain pending**:
the owner explicitly deferred them until after committing the branches. The
original macOS review, the original Linux measurements in commit messages, and
the Windows fixes in the source series are historical evidence, not results
from the updated fork. The supplied Windows prose duplicated the macOS review;
the actual Windows changes were identified from repository history.

The final GPUI revision separately passed all **21 hosted workflow jobs**,
including macOS ARM/Intel, Windows x64/ARM, Linux and headless configurations.
[The job record](performance/finalization-gpui-hosted-validation.json) pins the
revision and links every result. These are framework build/test checks, not native
GitComet application checks or platform performance measurements.

Work was isolated in `GitComet-verified-improvements` and
`gpui-ce-verified-improvements`. The original `GitComet3` and `gpui-ce2`
checkouts retain their original branches and contents. No merge to `dev` or
`upgrades` is part of this work.

## Commit disposition

[The machine-readable ledger](performance-finalization-commits.json) records full
source and destination SHAs. The source series ends at
`516a38608eb2a89dc2c66257f7392fb8f5aa11ea`. Application changes remain separate
commits; recovery and portability fixes are folded into the changes they repair.

| Change | Source | Final application commit or framework commit | Decision |
| --- | --- | --- | --- |
| Bundled fonts avoid catalog scans | `98738917` | GitComet `93f3ab27` | Retained; custom font validation still scans |
| Initial focus refresh throttle | `faf0174f` | GitComet `5b242fa1` | Retained for Linux duplicate-load reduction; no macOS timing win claimed |
| Queued commit-detail coalescing | `c17a4f11` | GitComet `f066b35d` | Retained for bounded queued work; no macOS timing win claimed |
| One topology scan without commit-graph | `9a11c19c` | GitComet `09cc6cce` | Retained |
| Compact shaped lines | `f070ffd3` | Updated GPUI shared layouts | Old inline decoration representation has been replaced; patch is superseded |
| Shared repository reads and history indexes | `3ff7d610` | GitComet `86182818` | Retained with `40fa5079` canonical-path and `2510d221` Windows handle/shell fixes |
| Binary path truncation | `3a8fe01e` | GitComet `95bff357` | Retained |
| Cropped WGPU path targets | `8eb03f97` | GPUI `e3c1706b73` | Opt-in; includes Metal sampling, dithering and GPU fixture fixes |
| Python platform mock isolation | `531bf500` | GitComet `b1d51f0e` | Retained |
| Early Git runtime probe | `78f59b2c` | GitComet `0c93e12b` | Retained with duplicate-report, unrun-probe and forced-recheck fixes |
| LFS unstaged line stats | `f7d2ebc8` | GitComet `2a49e497` | Retained; deleted LFS files keep their counts |
| Interaction tracker without wrapper node | `9c28d8ce` | GitComet `8dfb2b55` | Retained |
| Idle carets | `5f9a5194` | GitComet `9110406c` | Retained; visible ten-second idle policy |
| Precompiled Metal shaders | `edd5f1a4` | GPUI `49125d63b4` | Ported to the new generated shader pipeline, with runtime fallback |
| Fixed-refresh unchanged-frame presentation | `bc2b2ca3` | GPUI `505816444c` | Retained; variable-refresh behavior preserved |
| Core Text descriptor font loading | `3ec504db` | Not replayed | Updated fork uses Parley; old font engine no longer exists. Native memory equivalence is pending |
| Cropped Direct3D path targets | `efefb02e` | GPUI `37b0d3d2c2` | Opt-in; fresh native pixel/performance checks pending |
| Windows fixed-refresh presentation | `934c5c1f` | GPUI `be74171c5f` | Explicit opt-in; fresh native checks pending |
| Windows native lint and shader label fixes | `9fc8282d` | GPUI `1538c3c85c` | Retained |
| Windows symlink privilege fixtures | `c7939624` | GitComet `943fe014` | Only error 1314 permits an explicit unavailable-fixture diagnostic |
| Restored-tab reuse | `73658b92` | GitComet `e99eeada` | Retained |
| Apple/macOS and Windows vendoring | `c1a733a4`, `2c8a6f15` | Omitted | Dependencies come from the fork |
| Terminal shutdown fix | `f67e73e4` | Already in `dev` | No duplicate replay |

The old macOS descriptor patch's reported 365 → 187 MiB footprint is **not a
verified result for Parley**. The updated font store shares font blobs, but native
startup and retained-font memory need measurement before claiming equivalence.
Likewise, the old shaped-line size assertion cannot validate the replacement
layout representation; scroll-retention measurements must cover the new backend.

## Compatibility work required by the updated fork

GitComet `5d74edfa` adapts rendering and hit testing to the updated text backend.
GPUI adds cap-height metrics (`738e96bd58`), canvas truncation and alignment
(`3c8df570ba`), injectable real test text systems (`1fefe72423`), and independent
custom text layouts (`4e56fe637d`). GPUI `58eaec912a` preserves emergency cluster
wrapping for oversized words, while keeping ordinary word boundaries and nowrap.

Editor tab expansion preserves original UTF-8 source offsets through wrapping,
caret movement, selection and hit testing. Markdown's custom text elements retain
their independent layout and clip selection rectangles to measured bounds.
Terminal painting retains Alacritty's column grid for wide and combining
characters. Layout tests use bundled fonts and the real Parley shaper instead of
the single-line test stub. These are compatibility changes with correctness
coverage, not new performance claims.

Current Rust lint fixes in GPUI are confined to macros and synchronous shader
build scripts (`c30b8df199`, `fdb60b0950`). The application CI runner also reads
nextest's configured report store rather than assuming it is Cargo's target
directory. Reused binary metadata and a custom `CARGO_TARGET_DIR` now work together;
stale reports are removed before execution. Nextest documents these paths in its
[configuration reference](https://nexte.st/docs/configuration/reference/).

Hosted software renderers found a one-byte gradient difference against a legacy
Metal golden. GPUI `80c32256b8` restores the upgraded fork's existing two-unit
UNORM8 software rounding tolerance, while keeping alpha and same-backend
cropped/full comparisons exact. This repair changes only test code and comments.

## Linux validation

Host: AMD Ryzen 9 5950X, 32 logical CPUs, 125 GiB RAM, NVIDIA GTX 1080,
driver 580.178.04, Linux Wayland desktop, Rust 1.98.1. Headless live UI captures
use a real Mutter compositor at 1400 × 900 and 60 Hz. Our builds and tests finished
before latency captures, and each run waited for one-minute load below four.
Other builds occasionally started on this shared workstation; the load samples
and intervals are retained, and these results are workstation estimates rather
than shipping budgets. Instrumented diagnostics are separate from ordinary paired
release runs with the application's opt-in probe enabled.

| Check | Result |
| --- | --- |
| GitComet full workspace runner | 8,701 passed, 48 ignored; inventory checked |
| Application UI harness | 4,483 passed, 20 ignored |
| UI-kit harness | 488 passed |
| Budget report harness | 111 passed |
| Backend/state/CLI nextest | 3,619 passed, 28 skipped |
| Python CI tooling | 97 tests, one skipped |
| Downstream example and API contracts | 11 passed |
| Workspace doctests | Pass |
| Application Clippy | All three CI package sets pass with `-D warnings` |
| Application all-target benchmark check | Passes with `--features benchmarks --all-targets` |
| Formatting and architecture boundaries | Pass |
| GPUI core | 477 passed |
| GPUI Parley | 61 passed |
| GPUI shared render/shaders | 34 passed |
| GPUI WGPU on GTX 1080 | 30 passed, one ignored |
| GPUI Linux platform | 84 passed |
| GPUI focused core/text/render/WGPU Clippy | Passes with `-D warnings` |

[The Linux validation record](performance/finalization-linux-validation.json)
retains execution settings, revisions and log hashes. Full logs and inventory
remain in `target/ci-reports/` and the recorded local paths.

The broader GPUI Linux platform Clippy command encounters three existing `gpui_xim`
warnings under Rust 1.98: wildcard fields in `client.rs:179`, an elidable lifetime
in `x11rb.rs:122`, and a collapsible `if` in `x11rb.rs:455`. They are outside this
series. The strict GitComet CI commands pass; a whole-fork strict lint pass is not
claimed. The hosted fork matrix passes but is not a substitute for measured
application checks.

The fresh renderer footprint probe compares the same executable with cropping
off and on, in two sessions of three alternating pairs. Every pair measured
**132 → 68 MiB of process GPU footprint (−48.5%)**. This is a 2560 × 1440
headless renderer retaining a 512 × 1408 path target, using the NVIDIA per-process
memory counter. The debug build measures allocation only; it establishes no
application latency, total application memory, or GPU execution-time improvement.
Default/full and cropped pixel tests pass, including resizing and target growth.
The exact binary hash, driver, revision and raw samples are in
[the renderer evidence](performance/finalization-cropped-paths-linux.json).

The [release UI evidence](performance/finalization-live-ui-linux.json) records
**84 valid application captures**, exact binary hashes, paired observations and
bootstrap intervals. The fixture has 20,401 commits and 2,000 main-line files.
Each comparison uses two independent sessions with three alternating pairs per
session, reversing the starting order in the second session. Frozen binaries and
full raw captures remain in `target/profiling/finalization/`.

The compatibility baseline is `5d74edfa`, before replaying the application series.
It uses the same renderer/text runtime as the measured candidate: intervening
framework changes address macros and build-script lints. This comparison separates
the application optimizations from the text migration. These are combined-series
measurements, not isolated per-commit timing claims.

| Compatibility baseline → candidate | Baseline | Candidate |
| --- | ---: | ---: |
| Launch → repository ready, median | 289.6 ms | 232.6 ms |
| Selection input → submit p95 | 10.13 ms | 10.06 ms |
| Scroll input → submit p95 | 17.39 ms | 17.33 ms |
| Scroll with terminal output, input → submit p95 | 17.83 ms | 17.72 ms |
| Scroll CPU draw p95 | 1.380 ms | 1.352 ms |
| Terminal whole-capture sampled process CPU | 2.41 s | 2.35 s |

The ready-time paired median ratio is **0.803** (95% interval **0.782–0.917**),
supporting a roughly 20% startup improvement on this fixture. Interaction timing
intervals include equality; they establish no additional speedup. Every run
witnessed all 240 selections, 1,200 scroll inputs or 400 scroll-with-output inputs
as applicable. The repository idle scenario had one incidental redraw per
60-second interval in both variants and similar idle CPU; it does not reproduce
the historical focused-caret saving.

A separate `dev` → candidate comparison checks the migration and full series:
first draw **200.2 → 179.3 ms**, ready **279.1 → 218.2 ms**, scroll input → submit
p95 **17.24 → 17.34 ms**. Scroll CPU draw p95 rises **1.245 → 1.314 ms**
(+0.069 ms); total CPU's paired median ratio is 1.046 with interval 0.999–1.053,
and peak PSS's paired ratio is 1.000 with interval 0.989–1.051. This is no measured
scroll or memory improvement over `dev`, and no detected input-latency regression
beyond `max(5%, 1 ms)`. It is a short oscillating scroll, not the native 12,000-row
retention workload. The original reviewed release is frozen for later comparisons
but was not part of these paired timing claims.

The measured candidate is GitComet `3b0fea82` with GPUI `fdb60b0950`; the final
GPUI pin `80c32256b8` differs only in `cfg(test)` expectations and comments. The
final pin was checked with the full UI-kit suite, all-target benchmark compilation,
local WGPU tests and the 21-job hosted framework matrix. No rendering or application
behavior changed between the captured release and the final pin. Native GitComet
builds and performance measurements remain part of the deferred handoff.

## Reproduction

Run GitComet's CI commands from the repository root, using the pinned nextest
version installed by `scripts/ci/install-nextest.py`:

```sh
python3 -m unittest discover -s scripts/ci -p test_ci.py
python3 scripts/ci/boundaries.py
python3 scripts/ci/run.py compile --context workspace --cargo-profile ci-test
python3 scripts/ci/run.py test --context workspace --nextest-threads 16 --ui-threads 16
cargo check --profile ci-test -p gitcomet-ui-gpui --features benchmarks --all-targets
cargo fmt --all --check
```

The project's workflow is authoritative for the three Clippy package sets,
additional example context and platform smoke tests. A custom `CARGO_TARGET_DIR`
is allowed; nextest reports still follow `.config/nextest.toml`'s store setting.

Build both release binaries before measuring. Freeze each binary and record its
Git SHA, GPUI SHA, lockfile hash and executable hash. When sharing one Cargo target
directory between worktrees, clean workspace packages between builds: otherwise
path dependency artifacts can be reused across different checkout roots. Do not
compare different profiles, trace-instrumented runs, or changing fixture contents.

```sh
python3 scripts/profiling/live-ui.py fixture --help
python3 scripts/profiling/live-ui.py measure \
  --baseline /path/to/compatibility-gitcomet --candidate /path/to/final-gitcomet \
  --repository /path/to/fixture --output target/profiling/first \
  --session first --pairs 3 \
  --scenarios startup history-select history-scroll terminal-output idle
python3 scripts/profiling/live-ui.py measure \
  --baseline /path/to/compatibility-gitcomet --candidate /path/to/final-gitcomet \
  --repository /path/to/fixture --output target/profiling/second \
  --session second --pairs 3 --reverse \
  --scenarios startup history-select history-scroll terminal-output idle
python3 scripts/profiling/live-ui.py report target/profiling/first target/profiling/second
```

The live driver rejects missing completion witnesses, dropped probe records and
invalid captures. Compare completed work as well as p50/p95/p99 input-to-draw,
main-thread CPU, total CPU, memory, frame gaps and redraws. CPU draw/submit timing
does not measure compositor presentation or GPU execution. Require repeatable
benefit and investigate guarded regressions above `max(5%, 1 ms)` before promoting
a later optimization; retain raw pairs rather than selecting favorable runs.

The GPU-memory probe runs from the fork on a supported NVIDIA Linux host:

```sh
cargo build -p gpui_ce_wgpu --example path_target_footprint --features test-support
python3 scripts/profile-path-targets.py \
  --binary target/debug/examples/path_target_footprint \
  --output target/profiling/cropped-paths.json --pairs 3
```

Use `GPUI_GPU_EXPERIMENTS=cropped-paths` for opt-in WGPU/Direct3D path target
cropping. Targets retain their growth capacity until a resize. Windows also
supports `GPUI_GPU_EXPERIMENTS=fixed-refresh` only when the operator knows every
display used by the process has a fixed rate. Combine the values with commas when
testing both. Keep separate off/on controls for each feature.

## Deferred native verification

Fetch both branches on each native machine. Confirm all four Cargo pins equal the
published fork SHA, and build with `--locked` without path patches. Run the full
workspace CI inventory and runner, the example context, the three strict Clippy
sets, and the native GPUI core/text/render/platform suites. Preserve every ignored
test and prerequisite diagnostic in the report; unavailable coverage is not a
pass. Check app startup, restore, close/reopen, settings/custom fonts, navigation,
Markdown selection, long-token wrapping, tabs/Unicode, high DPI, terminal wide and
combining characters, diff search and the branch/merge curves and splash logo.

For each application commit, build it and its parent in release mode, then run
3–8 alternating pairs on the same fixture and unchanged machine/display settings.
For a framework commit, build two application copies differing only in that fork
pin. Keep cold and warm caches separate. Record CPU time, native memory metric,
p50/p95/p99 input-to-presentation and worst stalls as applicable; record driver,
OS, refresh policy, scale and workload. Only attribute an improvement to a commit
whose parent comparison demonstrates it. Hardware-specific gains must retain
their stated platform and workload scope.

| Change | Native workload and guard |
| --- | --- |
| Font catalog | Cold/warm startup with bundled defaults; custom and missing fonts still validate; count catalog calls/file opens separately from launch latency |
| Initial focus refresh | Ready-before-activation and activation-before-ready; count loads; later reactivation must refresh |
| Commit-detail coalescing | Ordinary selection and burst under deliberately busy worker pool; compare requests/completions and newest-selection latency |
| No-graph history | 100k and 2M histories without commit-graph, plus commit-graph control; first page, ready index, CPU and peak memory; verify timestamp and SHA order |
| Shared reads | One/four windows, same/linked/unrelated repositories; one compatible topology, cancellation/closure, physical replacement and memory plateau |
| Path truncation | 340/1,300-byte paths and Unicode; candidate shaping count, keystroke latency, highlighted range and visible width |
| Early Git probe | First launch and multiple windows; one report/store, forced rechecks, failed spawn/unrun probe recovery |
| LFS stats | 20 modified 1 MiB LFS payloads; real configured filter, deleted files, non-LFS pointer text and staged controls |
| Interaction tracker | Indexed history with 38 visible rows and scrolling; real frame allocations, input/draw p95 and pointer hit boxes |
| Idle carets | Focused editor/filter and terminal; 20-second interval after ten-second timeout; zero blink redraws, visible caret, immediate activity/focus reset |
| Restored tabs | Restored workspace with one/many repositories; no duplicate backend opens or missing restored state |

On macOS, measure **Activity Monitor memory footprint**, rather than treating
Linux PSS as equivalent. Use Instruments Time Profiler/Allocations and Metal System
Trace for attribution. Explicitly measure the new Parley backend's startup peak
and memory after a 12,000-row scroll against both `dev` and the reviewed branch.
This resolves the unverified descriptor-font and compact-layout equivalence.

For Metal, test compiled libraries present, absent, rejected and missing required
entry points; each fallback must draw the paths correctly. Build at the supported
minimum macOS deployment target. `GPUI_RENDER_REQUIRE_METALLIB=1` makes a missing
Metal Toolchain a build failure for a release verification build; ordinary builds
fall back to runtime compilation. Compare truly cold shader-cache startup
separately from warm launches. Measure fixed-rate display GPU time during the
three-second scroll tail, then verify ProMotion/variable-refresh and forced redraw
behavior. The old cold-cache ~1-second win and fixed-rate −96% GPU-time claim are
historical until reproduced on this shader pipeline.

On Windows, run from a native interactive desktop with an ordinary `%ComSpec%`,
including paths with spaces and non-ASCII characters. Check shared repository
owners release file handles before fixture cleanup. Error 1314 may mark symlink
fixtures unavailable; run them again on a host with Developer Mode or the required
privilege. Exercise native Direct3D and WGPU path pixels with gradients, dithering,
offsets, clipping, growth and resize in both crop modes. Measure VRAM, CPU and
input/presentation latency independently. Test fixed-refresh opt-in on a known
fixed display, and leave it unset on variable-refresh systems. Follow
[the existing Windows runtime guide](windows-test-runtime.md) and profiling
PowerShell/UI-responsiveness drivers.

Carets resting visibly after ten seconds is an intentional behavior change.
Mac fixed-refresh suppression is display-dependent; Windows suppression and GPU
target cropping remain opt-in. None of these commits enables broader renderer
pooling, batching or cache experiments.

## Performance test coverage

The suite has strong structural checks: startup dispatch/load counts, coalescing
and stale results, single-pass topology and commit-graph controls, SHA/timestamp
ordering, shared allocation identity and worktree isolation, cancellation and
reopen/replacement recovery, bounded path shaping, LFS filter controls, caret idle
transitions and full/cropped renderer pixels. Existing ignored benchmarks cover
history index/frame phases, allocations, line stats, terminal rendering and live
resource guards. See [profiling instructions](../scripts/profiling/README.md) and
[indexed-history measurements](indexed-history-performance.md).

This finalization adds real text-backend coverage for migration-sensitive wrapping,
Unicode/tab source offsets, terminal cell positions and custom Markdown layouts,
plus a reproducible paired GPU-footprint driver and nextest report-location
coverage. Renderer pixel tests are correctness gates, not timing benchmarks.

Obvious remaining gaps are automated native cold-Metal-cache/fallback integration,
native retained-font memory after repeated use, repeatable per-frame allocation
baselines for the new backend, native fixed/variable-refresh presentation budgets,
and end-to-end input latency under busy worker/LFS/terminal workloads on dedicated
machines. Existing local live drivers cover several of these workloads, but they
are not stable CI budgets. Add calibrated dedicated-runner jobs after native
baselines exist; keep portable unit tests focused on work counts, bounds and
correctness rather than absolute hardware timing thresholds.

The terminal live scenario witnesses scrolling but does not witness consumption
of all generated output. PTY parsing can finish before the scroll phase in one
variant and during it in another. Compare whole-capture CPU as a guard, and add a
terminal byte-count/end-marker witness before attributing phase CPU changes to
terminal performance. Equal scroll completions alone do not establish equal
terminal work within that phase.
