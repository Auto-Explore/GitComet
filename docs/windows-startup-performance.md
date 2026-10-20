# Windows startup investigation

Measured on October 20, 2026 with a Ryzen AI 9 HX 370 and Radeon 890M.
DX12 remains the default; DX11 was used only as a diagnostic comparison.

The final candidate's warm-start medians are **671.7 ms to first draw** and
**847.8 ms to repository readiness** across eight runs in two alternating-pair
sessions. Seven of eight runs reached readiness below one second; the remaining
run took 1,014.7 ms. The median target is met on this machine; a strict
subsecond maximum was not achieved, and reboot-cold startup was not measured.

## Baseline and bottleneck

GitComet baseline: `848514d8b4b9e3d11e37565ebc6399ded0a707fd`.
GPUI baseline: `0665fb07d59cf7af185e58f44b9f890590abe204`.
Lazy-smoothing GPUI change: `9362a02d98324c6d91a1bec58b90ab22ab667252`.

The original samply capture attributed 2,264 ms of sampled main-thread CPU
to `D3DCompiler_47.dll`: 85.6% of main-thread CPU sampled during the first
three seconds. Four optional corner-smoothing pipelines accounted for 1,878 ms:

| Pipeline | Inclusive sampled compiler CPU |
| --- | ---: |
| Smoothed quads | 789 ms |
| Smoothed shadows | 687 ms |
| Smoothed polychrome sprites | 213 ms |
| Smoothed blur composite | 188 ms |

GitComet does not request positive corner smoothing. Its controls use ordinary
circular rounded corners; direct image painting passes zero smoothing. This
optional GPUI feature produces continuous corners similar to iOS/Figma, and is
independent of normal antialiasing, text smoothing, and image filtering.

The GPUI change creates these four pipelines on first use. It retains their
original shaders and compiles before encoding the first frame that needs them.
It supports reuse, resource rebuilds, failure rollback, and retries. A future
feature requesting smoothing can still incur a one-time compilation delay.

The tested renderer is integrated through `vendor/gpui-wgpu`, with a Cargo
git-source patch affecting only `gpui_ce_wgpu`. Other GPUI crates retain the
baseline revision. This avoids depending on an unpublished local commit.
See `vendor/gpui-wgpu/UPSTREAM.md` for its provenance and removal procedure.

## Controlled measurements

Five alternating baseline/candidate pairs used the same `release-with-debug`
profile, DX12/FXC, real GitComet repository, isolated application profiles, and
visible desktop windows. No build ran during these measurements.

| Milestone | Eager smoothing | Lazy smoothing |
| --- | ---: | ---: |
| Launch to first draw, median | 2,361.6 ms | 656.9 ms |
| Launch to repository ready, median | 2,802.4 ms | 1,100.6 ms |
| Repository-ready range | 2,782.9–2,831.1 ms | 1,067.5–1,117.3 ms |
| Sampled peak working set, median | 855.9 MiB | 784.2 MiB |

The first observed candidate launch was slower: first draw 836.7 ms and
repository ready 1,280.1 ms. These are not reboot-cold measurements.

With the actual saved application profile, the parent launcher observed the
first interactive marker by 949.5 ms. App stage tracing recorded first paint
at 665.0 ms, first interactive at 837.3 ms, and repository ready at 1,104.1 ms.
App tracing starts after process creation; the parent measurement also includes
launch and polling overhead. Subsecond interaction was observed on this machine,
but strict subsecond fully loaded repository startup was not achieved.

The candidate's new 1 kHz samply capture attributed about 360 ms of compiler CPU
to ordinary pipelines and none to the four smoothed initialization callsites.
The original capture used 8 kHz, so the sampled reduction is approximate.
Unprofiled paired measurements establish the latency improvement.

Additional diagnostic comparisons:

| Experiment | First draw, median | Repository ready, median |
| --- | ---: | ---: |
| Existing release, eager DX12/FXC, 3 pairs | 2,529.6 ms | 2,875.9 ms |
| Existing release, native DX11, same 3 pairs | 178.7 ms | 530.5 ms |
| Existing release, eager DX12/FXC, separate 3 pairs | 2,616.4 ms | 2,999.7 ms |
| Same release executable, SDK DXC | 1,453.6 ms | 1,840.2 ms |
| Lazy smoothing, FXC, separate 5 pairs | 661.0 ms | 1,110.5 ms |
| Lazy smoothing, SDK DXC, same 5 pairs | 626.8 ms | 1,052.5 ms |

DXC was Windows SDK 1.8.2502.11, staged next to isolated executables; loaded
modules confirmed the DLLs used. Once smoothing is deferred, DXC adds only a
small improvement. Production packaging does not bundle these SDK DLLs.

## Repository bootstrap overlap

The next change creates the `AppStore` and prepares repository bootstrap before
`cx.open_window`. Repository workers can then load status and history while
the native window initializes its DX12 renderer. The previous saved-profile
trace had Git available by 96.3 ms, but created the store at 658.5 ms.

Preparation applies settings and entry gates before dispatching repository
requests. The view consumes one prepared bootstrap decision and attaches to
the latest store snapshot, including retained notifications and diagnostics.
Git-runtime recovery, focused tools, empty workspaces, saved workspace layout,
and failed-open handling retain their existing behavior.

Six new bootstrap tests pass. They cover loading before a window exists,
single dispatch and stable repository identity, entry gates, focused/empty
workspaces, failures before attachment, Git recovery, and feedback consumption.
Another 152 existing app, notification, and window-lifecycle tests pass.

Five alternating pairs compared the frozen lazy-only build with this bootstrap
candidate, using the same release profile and DX12/FXC:

| Milestone | Lazy only | Early bootstrap |
| --- | ---: | ---: |
| Launch to first draw, median | 708.2 ms | 868.6 ms |
| Launch to repository ready, median | 1,183.4 ms | 1,015.1 ms |
| Repository-ready range | 1,165.8–1,242.6 ms | 983.2–1,150.6 ms |

This is a tradeoff, not a uniform improvement. Three candidate runs overlapped
loading, while two waited for the Git runtime probe and restored only after
renderer initialization. First-frame CPU drawing was not slower; the delay
occurred before window attachment. Contention between renderer initialization
and repository workers is a possible explanation, not a proven attribution.
The first observed bootstrap launch reached harness readiness at 1,350.2 ms.
Neither these warm-cache pairs nor that first observation establish reboot-cold
subsecond startup. Dependency source identities and build paths also changed
when integrating the identical renderer source, potentially affecting layout.

With the actual saved profile, the parent observed interaction at 859.8 ms;
app tracing recorded repository readiness at 1,229.6 ms. That run took the
deferred path: bootstrap preparation was at 102.7 ms and the Git probe finished
at 107.5 ms, but its UI handoff waited until window initialization completed.

A fresh 1 kHz samply capture of an early-bootstrap run attributed 550.5 ms of
sampled compiler CPU to ordinary shaders (quads 188.1 ms, shadows 161.1 ms).
There were still no eager smoothing pipeline calls. This is diagnostic evidence
for the next renderer experiment, not a controlled attribution of contention.

## Parallel ordinary pipeline creation

The renderer now has a bounded parallel initialization path for Windows DX12.
It uses at most three compilation lanes, including the caller, and respects
available parallelism. All ordinary pipelines are created before rendering.
Other backends and warm runtime rebuilds keep serial initialization.

Each worker captures its own wgpu errors. All workers join before an error or
panic propagates, partial pipeline sets are discarded on failure, and pipelines
sharing a vertex stage remain in the same batch to avoid duplicate cache misses.
Two new DX12 tests pass: serial/parallel pixel equality over repeated frames,
and worker validation failure followed by a clean retry without error leakage.
The four existing smoothing regressions also pass on this renderer.

Eight alternating pairs across two sessions (five plus three) compared the
frozen bootstrap build with the parallel candidate using DX12/FXC and the same
`release-with-debug` profile. No compilation ran during measurements.

| Milestone | Bootstrap / serial pipelines | Bootstrap / parallel pipelines |
| --- | ---: | ---: |
| Launch to first draw, median | 801.6 ms | 671.7 ms |
| Launch to repository ready, median | 1,100.2 ms | 847.8 ms |
| Repository-ready range | 1,009.1–1,267.5 ms | 807.0–1,014.7 ms |

The candidate reached repository readiness faster in every pair. Median paired
saving was 251.1 ms, with a median paired ratio of 0.776 (22.4% lower latency).
Two pairs had small first-draw regressions of 14.5 and 23.8 ms despite faster
readiness. Bootstrap timing varied: the serial baseline overlapped loading in
four of eight runs, and the candidate in seven of eight. These are practical
end-to-end results with mixed bootstrap paths, not a shader-only microbenchmark.
The candidate's one deferred run was also its one result above one second.
The four pairs in which both variants overlapped loading still improved
readiness in every pair, with a median paired saving of 187.4 ms.

Memory results depend on that path mix. Across all eight runs, median sampled
peak working set was 675.1 MiB for serial creation and 669.1 MiB for parallel
creation; median sampled peak private bytes were 838.1 and 864.5 MiB.
In the four comparable early-loading pairs, parallel creation increased sampled
peak working set by a median 150.3 MiB and private bytes by 209.8 MiB.
These are maxima of current-process samples approximately every 260 ms over the
short run. They are not exact native peak counters, allocation stacks, retained
heap, steady-state memory, or GPU memory. No general memory reduction is claimed.

The first observed parallel-candidate launch reached first draw at 837.6 ms and
repository readiness at 1,019.6 ms. With the actual saved profile, app tracing
recorded first paint at 623.3 ms, repository ready at 709.8 ms, and interaction at
755.3 ms; the parent observed interaction at 879.2 ms. An empty profile reached
ready Home at 539.2 ms in app tracing, observed at 630.6 ms by the parent.
History navigation and opening Settings passed live checks on the final binary.

Parallel compilation moves work onto a bounded set of threads; these results
do not establish a reduction in total shader compiler CPU. The exact pre-format
compiled/test sources and their relationship to the rustfmt-formatted working
source are retained in the local provenance artifacts.

## Remaining work

The earlier lazy-only trace attributed ordinary shader work to quads (~121 ms
sampled compiler CPU), shadows (~67 ms), and path rasterization (~39 ms).
Those shaders still compile; their initialization can now overlap.

After window creation, repository `LoadStatus` took about 376 ms wall time
in the diagnostic trace. It includes a 149 ms child-command wait while
supplementing status for the populated `fixtures/syntax_test` gitlink.
That supplement preserves submodule correctness; the implementation already
limits its query to gitlinks and skips positively empty, unchanged checkouts.

Watcher initialization (~142 ms sampled CPU) and directory traversal run
concurrently. A worktree-dirty scan also overlaps the main status scan.
These inclusive and parallel costs must not be added as elapsed-time savings.

Further experiments include:

- Adopt a completed background Git probe before window attachment, with explicit
  ownership of the deferred restore and protection against duplicate recovery.
  The current UI handoff can still serialize loading when the probe finishes
  just after bootstrap preparation.
- Start the gitlink status supplement alongside the gix worktree scan after
  selecting paths. This could overlap part of the observed 149 ms wait while
  preserving submodule status, cancellation, and error handling.
- Reduce repeated filesystem lookups during watcher initialization while
  preserving watch coverage. Other-worktree scans already exclude the active
  checkout; they are not duplicate active-repository status requests.

These are unmeasured opportunities. Public `wgpu::PipelineCache` in version
29.0.4 supports Vulkan only, so it does not provide persistent DX12 shader
caching. More shader deferral requires evidence that the initial frame does
not need those pipelines.

## Validation and reproduction

Four smoothing regression tests passed on the candidate: first-use pixel
equivalence, backdrop/isolated-filter planning, creation rollback, and DX12
retry behavior. Three GPU tests explicitly selected the Radeon 890M/DX12;
the fourth tests planning. History navigation and opening Settings also passed
live UI smoke checks.

Use `scripts/profiling/live-ui.py measure` for alternating startup measurements.
The startup scenario disables updates, network tag fetching, and signature
checks through its isolated profile. First draw measures completed UI drawing,
not physical display presentation. Memory is sampled every 250 ms and can miss
short peaks; it is not an allocation stack trace or exact retained Rust heap.

The PowerShell helpers provide:

- `scripts/record-gitcomet-startup.ps1`: samply capture, frozen executable/PDB,
  scheduling/graphics events, and retained ETLs. Default 8 kHz is intentionally
  expensive; use `-Rate 1000` for a lighter diagnostic capture.
- `scripts/capture-gitcomet-startup.ps1`: startup stage tracing, with optional
  empty profile and graceful automatic exit.
- `scripts/view-gitcomet-startup.ps1`: matching symbols and Windows symbol server.
- `scripts/build-gitcomet-allocations.ps1` and
  `scripts/capture-gitcomet-allocations.ps1`: separate instrumented build and
  milestone allocation totals. These exclude some native compiler/driver
  allocations and do not provide allocation call stacks.

For sampled CPU analysis, use `threadCPUDelta`. Synthetic off-CPU sample
weights must not be counted as CPU. Keep profiling runs separate from ordinary
latency measurements and avoid compilation during either.

Large local artifacts remain under ignored `target/startup-investigation/`:
`repo-lazy-pairs/session.json`, `experiment-summary.json`, the frozen
`baseline-frozen/` and `lazy-runtime/` binaries/PDBs, and
`samply-lazy-candidate/` with the matching symbols, ETL, and analysis.
The bootstrap experiment adds `bootstrap-runtime/`, `repo-bootstrap-pairs/`,
`bootstrap-stage-real-profile/`, and `samply-bootstrap-candidate/`.
The final experiment adds `parallel-runtime/`, `repo-parallel-pairs/`,
`repo-parallel-confirmation/`, `parallel-comparison-summary.json`, the
`parallel-stage-*` traces, and `parallel-compiled-source/`.
Their availability is local; this document records the durable measurements.

## Upstream precompilation

Official GPUI-CE revision `9a82fba4e5f1f9194a5489b7a417d164083dd67a`
precompiles **DX11** shaders in
[gpui_render/build.rs](https://github.com/gpui-ce/gpui-ce/blob/9a82fba4e5f1f9194a5489b7a417d164083dd67a/crates/gpui_render/build.rs).
Its [WGPU pipeline implementation](https://github.com/gpui-ce/gpui-ce/blob/9a82fba4e5f1f9194a5489b7a417d164083dd67a/crates/gpui_wgpu/src/wgpu_renderer/pipelines.rs)
still creates pipelines from WGSL at runtime, including all four smoothing
variants eagerly. DX11 precompilation does not replace the lazy DX12 change.
