# Windows startup investigation

Measured on October 20, 2026 with a Ryzen AI 9 HX 370 and Radeon 890M.
DX12 remains the default; DX11 was used only as a diagnostic comparison.

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

## Remaining work

The candidate still compiles ordinary shaders, including quads (~121 ms
sampled compiler CPU), shadows (~67 ms), and path rasterization (~39 ms).

After window creation, repository `LoadStatus` took about 376 ms wall time
in the diagnostic trace. It includes a 149 ms child-command wait while
supplementing status for the populated `fixtures/syntax_test` gitlink.
That supplement preserves submodule correctness; the implementation already
limits its query to gitlinks and skips positively empty, unchanged checkouts.

Watcher initialization (~142 ms sampled CPU) and directory traversal run
concurrently. A worktree-dirty scan also overlaps the main status scan.
These inclusive and parallel costs must not be added as elapsed-time savings.

Starting repository bootstrap before window/GPU creation is the next candidate
for overlapping useful work. It must preserve settings-before-restore ordering,
entry gates, Git-runtime readiness, focused modes, and single ownership of
bootstrap dispatch. Any saving requires measurement.

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
Their availability is local; this document records the durable measurements.

## Upstream precompilation

Official GPUI-CE revision `9a82fba4e5f1f9194a5489b7a417d164083dd67a`
precompiles **DX11** shaders in
[gpui_render/build.rs](https://github.com/gpui-ce/gpui-ce/blob/9a82fba4e5f1f9194a5489b7a417d164083dd67a/crates/gpui_render/build.rs).
Its [WGPU pipeline implementation](https://github.com/gpui-ce/gpui-ce/blob/9a82fba4e5f1f9194a5489b7a417d164083dd67a/crates/gpui_wgpu/src/wgpu_renderer/pipelines.rs)
still creates pipelines from WGSL at runtime, including all four smoothing
variants eagerly. DX11 precompilation does not replace the lazy DX12 change.
