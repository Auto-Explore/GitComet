# Windows startup investigation

Measured on October 20, 2026 with a Ryzen AI 9 HX 370 and Radeon 890M.
DX12 remains the default; DX11 was used for diagnostic comparisons and
compatibility checks.

The latest completed application changes use opaque Windows surfaces, coalesce
startup resizes, defer unused filter shaders and optional-tool discovery, adopt
Git discovery before attaching Home, and reuse validated watcher ancestor
metadata. DX12 remains the default.

The final five-pair confirmation session reached median **440.5 ms first CPU
draw**, **481.0 ms first platform submission**, and **495.8 ms repository
readiness**. All five pairs improved. Three of five candidate runs reached ready
below 500 ms; the range was **466.5–512.5 ms**. This is not a reliable sub-500 ms
guarantee: an earlier session with the same binaries regressed, and its slower
renderer-construction state was not reproduced in the successful CPU capture.
Both sessions are retained below.

The measured candidate is GitComet
`8bd06006d90b96026611928d15663371ceb0d781`, pinned to GPUI
`a4ee85008357c2e8e659bf8383ccae65ab62f94f`. All comparisons reused OS/driver
caches; reboot-cold startup was not measured. Callback, CPU-submission, and ETW
Present times do not prove physical display scanout.

## Fresh samply capture: remaining costs

A new 1 kHz CPU/context-switch/graphics capture on October 20 used the same
frozen release-with-debug executable from its original path. The foreground
repository-startup check passed on hardware DX12. No compilation ran during
capture. This is one diagnostic run with warm OS/driver caches, not a replacement
for the unprofiled five-pair confirmation above.

| Milestone, from process spawn | Fresh sampled run | Previous sampled run |
| --- | ---: | ---: |
| First recorded CPU draw complete | 561.2 ms | 526.3 ms |
| First platform submission complete | 616.1 ms | 561.0 ms |
| Repository-ready observation | 689.2 ms | 602.9 ms |
| Status task complete | 516.4 ms | 507.6 ms |

The remaining critical path is predominantly renderer initialization and the
first two frame submissions. Required repository data is already available
before the first recorded draw starts.

| Work | Fresh evidence | Interpretation |
| --- | --- | --- |
| DX12 context initialization | Main-thread samples span 71.2–274.6 ms of ETW process age; 84.6 ms sampled CPU | Adapter enumeration accounts for 43.4 ms CPU and device creation 27.4 ms CPU within this category. The observed interval also contains 75.7 ms off-CPU time following preemption. |
| Ordinary render pipelines | Main-thread samples span 283.7–490.4 ms; 485.1 ms compiler CPU across the main thread and two workers | Largest remaining compiler cost. The main pipeline interval has only about 2.0 ms off CPU; this is predominantly executing work, not a long worker join. |
| First platform submission | 55.0 ms instrumented elapsed time | Includes surface resize/configuration, atlas upload and render-target allocation. Its native sampled interval has 7.1 ms sampled CPU and 41.1 ms off CPU. |
| Second draw and submission | 16.1 ms drawing, then 47.5 ms submitting | Its native submission interval has 6.1 ms sampled CPU and 37.8 ms off CPU. Atlas staging-buffer and render-target allocations appear near the long gaps. No compiler CPU was sampled in either submission. |

CPU totals are summed across threads where stated. Inclusive categories overlap
and must not be added to elapsed intervals. Sample spans are observed first/last
samples, not instrumented function entry/exit. Scheduler on-CPU markers and
sampled `threadCPUDelta` are separate measurements.

The relevant frame-allocation call chains are
`PreparedTargets::prepare -> Device::create_texture` and
`WgpuAtlas::before_frame -> StagingBuffer::new -> create_buffer`, reaching
`gpu_allocator::MemoryBlock::new -> ID3D12Device::CreateHeap`. The longest
nearby off-CPU intervals are 10.9 ms in the first frame and 7.3/28.3 ms in the
second. Samples near these gaps include
`NtGdiDdDDIWaitForSynchronizationObjectFromCpu`; however, the switch-out events
are labelled preempted. They establish resource-call-associated scheduling
gaps, not exact driver-wait durations. The trace does not label texture extents
or prove which path/filter target was allocated.

The first-submission interval includes an observed 18.7 ms resize/configure
span. This is a changed-size configuration: native DX12 construction starts
with a 1×1 surface, the first Present uses 2100×1350, and equal dimensions return
before configuration. It is not evidence of a redundant equal-size resize.
Investigate whether construction can use the intended size directly.
Retained path textures can grow with scene bounds; confirming why another
allocation is needed in frame two requires target labels, sizes and invalidation
reasons. GPUI does not rotate these textures through retained per-frame slots.
This capture is not a Rust heap-allocation profile.

Repository work still consumes substantial parallel resources:

- Status takes 322.1 ms elapsed and finishes at 516.4 ms after spawn. Its result
  is present in the initial recorded view state applications, before drawing.
  Initial log loading finishes at 227.0 ms.
- The watcher thread consumes 142.9 ms CPU in the first 1.1 seconds, including
  52.5 ms in watcher reload and 25.5 ms in watch-path expansion. These nested
  values are not additive.
- Optional LFS/annex discovery consumes 125.0 ms CPU, with sampled work beginning
  around ETW process age 624 ms, after the first successful native Present at
  615.1 ms. It no longer contributes sampled CPU before that presentation.
  This does not exclude contention during the subsequent readiness interval.

The tail after first submission is observable on one journal clock:
8.7 ms until the second draw, 16.1 ms drawing, 47.5 ms submitting, then 0.7 ms
until ready. Failed readiness predicates are not logged, so the full 73.1 ms
tail cannot be called polling waste or assumed removable. Indexed-history
state is applied immediately before readiness, but full indexing is not a
startup requirement and no late initial status/log result appears.

Compared with the preceding sampled run, ordinary pipeline compiler CPU rose
from 383.1 to 485.1 ms, while the main pipeline sample span grew by about
41.7 ms. First/second submission durations rose from 34.7/19.2 to 55.0/47.5 ms.
Status elapsed time remained essentially unchanged at 321.9/322.1 ms.
These observations locate the slower work in this run; they do not identify
the cause of run-to-run variation. The two runs used different executable
launch paths and opened different repository revisions.

The next concrete investigations are render-target/atlas allocation and
surface configuration during the first two frames, followed by per-pipeline
release timings to reduce the remaining shader work. Status and watcher work
remain relevant to CPU contention and to establishing reliable sub-500 ms
readiness once rendering improves. No additive speedup is claimed from these
overlapping costs.

Evidence is retained under
`target/startup-investigation/samply-remaining-20261020/`: `startup.json.gz`,
matching symbols, kernel/user ETLs, `recording.json`, `process-analysis.json`,
`renderer-findings.txt`, `renderer-phase-evidence.json`, `renderer-submission-evidence.json`,
`remaining-readiness-findings.{txt,json}`, and `status-thread-evidence.json`.
`pre-present-analysis.json` covers native process age 0–615.0769 ms;
`post-present-to1100-analysis.json` covers 615.0769–1100 ms. They contain
1290.6 and 195.7 ms aggregate app CPU respectively. All 505.1 ms sampled compiler
CPU, including approximately 20 ms of internal device initialization, precedes
the first native Present.

The capture run is `ed9bbc81-abd2-4893-8ca1-093fb9943733`, PID 14888.
The executable remains source `8bd06006` / GPUI `a4ee8500`, SHA256
`d62aa0087d9e9e6e3925b3d35a46ab2c0898e41dd2c0a2e4b496d62aace20584`.
The measured repository checkout is `c4b00cb7`; this is not the executable's
source revision. The binary includes all runtime optimizations but predates
the diagnostic-only log batching correction. ETW process-age, app-stage and
spawn-aligned journal clocks are kept separate; profile epoch metadata was
not used to align ETW to readiness. No display scanout or reboot-cold latency
was measured.

## Opaque surfaces, deferred work, and Home adoption

The implementation consists of these committed changes:

- `afcb771a`: opaque Windows main and Settings windows. The user inspected and
  accepted the appearance. The explicit background diagnostic override remains.
- `c6f19db7`: pin GPUI to checked startup-resize coalescing and demand-driven
  filter pipelines. Initial configuration/fallback remains checked, runtime
  resizes remain immediate, and pending dimensions survive renderer recovery.
  Ordinary direct frames and retained texture copies do not create the filter
  group. Actual blur, opacity groups, or a retained shader blit create it on
  first use. Unsupported external-surface integrations omit their pipeline.
- `ebcf9d4e`: empty Home shares the completed background Git-probe handoff, with
  the same stale-result and exactly-once attachment guards as repository
  bootstrap. Its readiness predicate is unchanged.
- `71db65d2`: automatic LFS/annex discovery starts after the initial platform
  presentation attempt. This is frame-loop ordering, not a GPU/display fence.
  Settings rechecks, runtime invalidation, and descendant cancellation remain.
- `5a48946a`: watcher expansion reuses successful ordinary ancestor metadata
  within one expansion and revalidates reused identities. Terminal paths,
  missing/error results, symlinks, and Windows reparse points are not cached.
  Stable cache entries are discarded; native watch coverage is not deferred.
- `8bd06006`: the new watcher replacement-race test uses a directory junction
  when Windows does not grant symlink creation privileges, retaining all race
  and invalidation assertions.

GPUI commits `5fbcd42c`, `ccbedc27`, and `a4ee8500` are pushed to
`lazy_load_shaders` in the GPUI worktree. The GitComet branch has not been pushed.

## Repository measurements: both sessions retained

Each session alternates five pairs of the previous frozen `7ebb3e91` /
`164485d7` binary and the new `8bd06006` / `a4ee8500` binary. Both arms use
DX12/FXC, identical release-with-debug settings, fresh app profiles, and the
desktop startup scenario. All twenty runs are valid and use early repository
bootstrap. No compilation or sampling profiler ran during measurement.

| Median, five runs per arm | Earlier baseline | Earlier candidate | Confirmation baseline | Confirmation candidate |
| --- | ---: | ---: | ---: | ---: |
| Spawn to first CPU draw | 516.0 ms | 644.9 ms | 481.0 ms | 440.5 ms |
| Spawn to first submission | 596.3 ms | 705.9 ms | 561.2 ms | 481.0 ms |
| First submission duration | 78.8 ms | 47.5 ms | 70.6 ms | 32.7 ms |
| Spawn to repository ready | 655.3 ms | 707.2 ms | 566.7 ms | 495.8 ms |
| Candidate ready range | — | 557.9–726.6 ms | — | 466.5–512.5 ms |

The earlier median paired ready change was **+67.2 ms**. In confirmation it was
**−64.9 ms**, with every pair improving by 54.4–99.8 ms. Three confirmation
candidates were ready below 500 ms and four submitted below 500 ms. Independent
medians and median paired changes are different statistics.

Separate three-pair stage diagnostics reproduced the earlier slowdown before
renderer construction: open-window-to-renderer increased from 334.3 to
558.9 ms, while renderer-to-view initialization fell from 70.3 to 27.5 ms and
submission duration fell from 78.3 to 39.0 ms. These intervals include native
window/driver/pipeline work and do not identify a specific cause.

A subsequent three-pair control launched the *same candidate executable* from
its frozen path and the profiler's copied path. Both were fast: ready medians
499.7 and 492.8 ms. This does not establish a fixed-path effect. The successful
ETW capture and later five-pair confirmation also did not reproduce the slow
state. Temperature, frequency, driver-cache state, and competing workload were
not recorded sufficiently to assign the earlier regression to any of them.
The results are neither pooled nor selectively discarded.

Repository readiness requires open/status/history readiness and a laid-out
history viewport; the harness polls at 50 ms intervals. It does not require the
entire indexed history in the startup scenario. It is a usable-content
measurement with polling delay, not a timestamp for the exact last worker.
Status is the last required data publication in all five confirmation runs.
In pairs 1/4/5 it arrives after the first CPU draw and reaches the view after
first submission. In pairs 2/3, which miss 500 ms, status is already available
at the first state application. Both content readiness and rendering matter;
shorter status alone would not resolve those two misses.

## Home and diagnostic logging overhead

The original three-pair Home diagnostics retained one severe candidate outlier:
baseline Home-ready markers were 588.4/574.8/558.0 ms and candidates were
926.8/524.9/486.1 ms. A later three-pair confirmation improved all pairs,
641.8/586.4/612.4 → 544.2/519.3/581.2 ms. These are app-trace times, not
spawn-to-ready times.

Further investigation found that the callback gap after `first_interactive`
does not necessarily represent another frame. When Home is already ready,
`first_interactive` and `home_ready` run sequentially in the same callback.
Synchronous diagnostic output between them can inflate the interval.
The real serde_json emission experiment measured 59 writes per stderr payload
and 44 per trace record. Under PowerShell redirected stderr, a callback gap
reached 29.7 ms without GPUI; formatting each complete record before writing
reduced the experiment's median from 3.406 to 0.079 ms.

The diagnostic code now formats each complete JSON line before `write_all`,
while preserving immediate record visibility, flushing, escaping, and recorded
timestamps. Startup milestones also enter the UI journal before synchronous
output, so future captures can use its existing process-spawn clock anchor.
Readiness conditions are unchanged. The application timings in this document
use the frozen `8bd06006` binary, before this diagnostic-only correction;
the emission experiment is separate evidence, not a rebuilt-app speedup.
The correction passed all three startup-trace tests and all eight startup-probe
tests in the cached `ci-test` build.

An additional frozen-binary comparison used the existing Python harness's
direct stderr file and seeded empty profiles. Baseline Home-ready markers were
433.4/442.1/456.2 ms; candidates were **343.6/364.8/368.7 ms**. The candidate
interactive-to-Home interval was only 0.189–0.207 ms. This supports the early
Home handoff and identifies diagnostic I/O as a source of inflated callback
gaps. The Python profiles also disable background update checks, seed survey
preferences, and set window geometry, so the entire PowerShell-versus-Python
latency difference must not be attributed to stderr alone.

The earlier repository benchmark did not enable these startup-stage markers.
The logging finding therefore does **not** explain away its regression.

## Fresh trace and watcher verification

The successful `samply-lean-startup-final` capture used the frozen candidate,
matching PDB, foreground DX12, 1 kHz CPU sampling, scheduling events, and graphics
events. Two preceding elevation-helper failures recorded no valid trace and
are excluded. The fixed analysis window is process age 0–1100 ms.

| Inclusive sampled CPU | Previous capture | New capture |
| --- | ---: | ---: |
| Native shader compiler, all app threads | 447.2 ms | 402.1 ms |
| Main-thread compiler | 169.8 ms | 164.6 ms |
| Watcher thread | 127.6 ms | 100.1 ms |
| Watcher input reload | 74.9 ms | 34.5 ms |

These nested categories overlap and are not additive elapsed savings. Ordinary
pipeline creation accounts for 383.1 ms of the new compiler CPU across three
participants; device-internal compilation and earlier context work account for
the remainder. Both pipeline workers finish before the main compiler tail.
The trace has no long new main-thread join delay and no positive samples of
the separate DX11 device constructor.

A separate bounded-worker experiment compared 3/4/6/8 participants across four
alternating rounds, each with a fresh hardware DX12 device and the same eight
ordinary pipelines. With release-equivalent shader flags (no DEBUG/VALIDATION;
VALIDATION_INDIRECT_CALL retained), pipeline-construction medians were
195.7/195.2/199.5/198.4 ms. The first three-worker run was 258.5 ms and is retained.
The quad pipeline finished last in all sixteen runs. Increasing worker count
provided no convincing benefit, so the production limit stays at three.
The test host code was still a debug build and spent about 56 ms parsing/setup,
so these are relative diagnostic results, not release application latencies or
proof of the release trace's longest pipeline label. The temporary experiment
patch and logs are archived locally; no additional GPUI change was shipped.

The new trace reaches first CPU draw at 526.3 ms, first submission at 561.0 ms,
and scenario readiness at 602.9 ms after spawn. Status finishes at 507.6 ms,
13.4 ms before the first draw starts and 53.4 ms before submission. Overlapping
the roughly 115 ms gitlink child with the base status walk could shorten status,
but no end-to-end saving is established while renderer/viewport work finishes
later. First successful ETW Present is at 556.2561 ms process age with status
zero. Automatic LFS/annex probes start after that first Present.

Separate watcher diagnostics show one setup attempt and one policy revalidation
in every run, with no retry/rebuild loop. The sum of setup phase wall times
falls from median 233.6 to 192.3 ms across three pairs. Input expansion improves
while directory discovery is slower; these concurrent wall intervals include
scheduling and do not establish per-phase CPU savings.

The application validation reports 402 passes, with three explicit ignored
fixtures. Existing unrelated symlink-only tests may self-skip without Windows
privileges; the new replacement race explicitly executed via a directory
junction. GPUI validation passed 63 Windows tests, 66 WGPU default tests plus
five integration tests, and 67 surface-enabled tests plus five integration
tests. Default strict Clippy passed; the surface-enabled run narrowly allowed
two verified pre-existing warnings in unchanged code. DX12 history navigation
and Settings, and explicit DX11 Settings, passed on the frozen application.

## Historical handoff and DX11 deferral result

The preceding combined startup changes reached median **540.6 ms first CPU draw**,
**646.6 ms first platform submission**, and **658.8 ms repository readiness**
over five runs. Readiness improved in every pair, but first-frame submission
became slower in four of five pairs. The 500 ms target has not been achieved.
These runs reused OS/driver caches; reboot-cold startup was not measured.

The measured source integration is GitComet
`7ebb3e912d931944c4b26e21f1ed62a4459b4002`, including app bootstrap handoff
`c55d94a43d87a1373a49a4c0402cc8c4ce120a91` and GPUI
`164485d7d856c0e57607501480f255d027b92709`. The new GPUI revision defers the
separate DX11 support device while retaining DX12 rendering. Both changes are
committed, tested, and measured together. A fresh CPU/graphics trace confirms
early repository dispatch and finds no positive CPU samples from DX11 device
creation. Renderer initialization and first presentation gate readiness in
that capture.

## Combined handoff and DX11 deferral measurements

Five alternating pairs compared the frozen GPUI `18e1b2f` baseline with the
combined Git-probe handoff and GPUI `164485d7` candidate. Both used normal
transparent windows, DX12/FXC, the same `release-with-debug` profile, and the
visible-desktop startup scenario. No compilation or profiling ran during these
measurements.

| Median launch latency, five runs per arm | Exact-pin baseline | Combined candidate |
| --- | ---: | ---: |
| First CPU draw complete | 460.3 ms | 540.6 ms |
| First platform submission complete | 553.9 ms | 646.6 ms |
| Repository ready | 897.1 ms | 658.8 ms |
| Repository-ready range | 821.8–923.3 ms | 635.1–730.2 ms |

The candidate loaded repositories early in all five runs; the baseline loaded
early once and deferred four times. Ready improved in every pair by a median
**244.5 ms** (median paired ratio 0.729), while first submission regressed in
four pairs; the median paired change across all five was **+74.4 ms**.
The first-submit duration itself
decreased from a median 92.8 to 85.3 ms: the launch-to-submit regression occurs
before or around starting that submission, rather than a longer submission
duration. The first frame can contain a different amount of loaded repository
content. The fresh trace below shows longer renderer initialization overlapping
repository workers, but does not establish contention as the cause.

This is a readiness/first-frame tradeoff, not a uniform startup improvement.
These measurements evaluate both changes together and a changed bootstrap-path
mix; they do not isolate the DX11 deferral's elapsed saving. The single pair
where both loaded early also improved ready by 91.6 ms, which alone cannot
establish an independent cause. Compare against the baseline rerun in this
session, not the earlier session's 831.9 ms ready median.

Median sampled peak working set was 412.4 → 342.6 MiB, and sampled peak private
bytes were 656.1 → 523.3 MiB. Both were lower in all five pairs, but bootstrap
timing and lifetime overlap changed. These are short-run process samples
approximately every 250 ms, not exact native peaks, allocation stacks, retained
heap, steady-state memory, or GPU memory. No general memory reduction is
established.

DX12 history-jump and Settings checks passed on the frozen candidate. Explicit
DX11 Settings also passed, confirming that native DX11 remains available.
Source behavior and regression tests establish that DX12 selection does not
initialize the separate DX11 support device; the fresh capture has no positive
CPU samples from its constructor or D3D11 creation API. Submission completion
remains distinct from physical display scanout, and none of these candidate
runs achieved 500 ms loaded startup.

## Separate stage and empty-Home diagnostics

Three alternating pairs per scenario used the same frozen binaries with stage
tracing and no sampling profiler. These diagnostics are separate from the
primary five-pair benchmark and must not be pooled with it. Home runs used new
empty application profiles.

| Scenario and stage metric, median | Baseline | Combined candidate |
| --- | ---: | ---: |
| Repository: native application construction duration | 64.9 ms | 11.6 ms |
| Repository: open-window start to renderer-ready interval | 257.0 ms | 335.6 ms |
| Repository: first-paint callback | 408.3 ms | 446.7 ms |
| Repository: first-interactive callback | 427.8 ms | 466.2 ms |
| Repository: active-repository model ready | 796.2 ms | 455.3 ms |
| Empty Home: native application construction duration | 113.0 ms | 14.3 ms |
| Empty Home: open-window start to renderer-ready interval | 272.8 ms | 338.8 ms |
| Empty Home: first-paint callback | 474.2 ms | 451.9 ms |
| Empty Home: first-interactive callback | 546.5 ms | 522.6 ms |
| Empty Home: Home-ready callback | 576.5 ms | 625.1 ms |

Callback/model milestones use the app trace's own origin, not process-spawn
time. First paint is a prepaint callback, and first interactive is a subsequent
frame callback that can still show Git checking. Model readiness excludes the
harness's viewport-geometry and polling requirements. None is display scanout.
The separate Home parent observer also adds launch work and 100 ms polling;
its timings cannot be substituted for these stage values.

The worker handoff occurred at a median 94.2 ms, before view initialization in
all three repository runs. Repository model readiness improved, while first
paint and the interactive callback regressed. Empty Home painted/interacted
earlier, but **Home-ready regressed in all three pairs**, by 15.7, 38.2, and
73.3 ms (median paired regression 38.2 ms). Its interactive-to-Home-ready
interval grew from a median 29.0 to 105.8 ms. No repository handoff runs on
empty Home. The markers locate increased elapsed time; they do not establish
a shader, driver, scheduling, or repaint cause, or an overall Home improvement.

## Fresh combined CPU and presentation trace

The successful `samply-deferred-dx11-retry` capture used the frozen candidate,
hardware DX12, a visible foreground window, and 1 kHz sampling with graphics
events. The original `samply-deferred-dx11` attempt is invalid and excluded.
Both this capture and the clean pinned baseline were analyzed over the first
1100 ms of process age with matching symbols.

The probe completed at app-trace age 116.929 ms and the worker dispatched
bootstrap at **117.058 ms**, before renderer completion at **454.763 ms** and
view initialization at **538.428 ms**. Initial bootstrap preparation still
records deferred-for-Git, but the subsequent worker dispatch makes this an
early-loading run. Later UI adoption is not the start of repository loading.

| Inclusive sampled CPU | Clean pinned baseline | Combined capture |
| --- | ---: | ---: |
| Separate DX11 device constructor | 33.732 ms | No positive samples |
| Main DX12 context/surface initialization | 64.225 ms | 101.730 ms |
| Native shader compiler, all app threads | 368.236 ms | 447.192 ms |
| Warm first-present pipeline rebuild | 13.351 ms | 17.542 ms |

No positive samples show `DirectXDevices::*` or `D3D11CreateDevice` in the new
capture's analysis window. Sampling cannot exclude an arbitrarily brief
execution. DX12 setup starts earlier but finishes later; adapter/device initialization and ordinary
pipeline compilation remain substantial. Repository workers and watcher setup
now overlap them. Inclusive categories overlap, and one diagnostic capture per
source cannot isolate cache, driver, scheduling, clock-frequency, or source
effects. The trace does not prove that contention caused the increased time,
or that device-creation CPU moved one-for-one into DX12.

Status took 320.9 ms and completed at 542.6 ms after spawn, before the first
CPU draw at 607.5 ms. Log loading was also complete. First submission finished
at 695.0 ms and scenario ready followed at 697.1 ms. Repository status is
therefore no longer the final readiness dependency in this capture. These are
diagnostic timings, not additional warm-benchmark pairs.

The first successful ETW `Present/Info` was at **691.1924 ms process age** with
`ReturnStatus=0`. This confirms a present event, not GPU completion or display
scanout. ETL process age, app-trace age, and journal/process-spawn clocks remain
distinct; their raw timestamps were not directly subtracted.

Startup size changes and the first transparent frame still configure the
surface. The subsequent warm pipeline rebuild contains no sampled FXC work;
its 17.542 ms CPU is distinct from the 447.192 ms initial compiler-inclusive
CPU across threads. Watcher setup consumed 127.6 ms sampled
CPU, overlapping shader workers. Optional LFS/annex discovery consumed
128.6 ms sampled CPU around first presentation and beyond ready. The gitlink
subprocess still lasted roughly 101.5 ms, now hidden behind renderer startup.
These concurrent costs are not additive elapsed savings.

## Clean upstream-pin baseline

GitComet `1da63f84a8329efed88b90701c0a6a5ac336d8cb` replaced the vendored
renderer with GPUI `18e1b2f65899ccbc36304fbb8f9ee703ea86adc4`, which contains
the lazy-smoothing and parallel ordinary-pipeline changes. Five alternating
pairs compared this pin with the frozen previous parallel-renderer build.
Both used `release-with-debug`, DX12/FXC, optimization level 3, fat LTO, one
codegen unit, and `x86-64-v3`. The visible client area was 2100×1350 at 144 DPI,
using Radeon driver `32.0.13058.8001`. No compilation or profiling ran during
these latency measurements.

| Median launch latency, five runs per arm | Previous parallel build | Exact upstream pin |
| --- | ---: | ---: |
| First CPU draw complete | 458.5 ms | 455.4 ms |
| First platform submission complete | 540.2 ms | 538.0 ms |
| Repository ready | 828.2 ms | 831.9 ms |

The previous build loaded repositories early once and deferred four times;
the pinned build deferred in all five runs. Its ready range was
814.7–861.5 ms, with no result below 500 ms. The four deferred/deferred pairs
had median paired changes of +7.6 ms draw, +12.7 ms submission, and −14.8 ms
ready. This small sample establishes comparable practical latency; it does not
establish a pin-related improvement or regression.

First CPU draw ends scene construction before platform drawing. First-submit
completion includes CPU/platform submission work; neither metric establishes
GPU completion or physical display scanout. Repository ready is the startup
scenario's usable-repository condition, observed on a 50 ms polling interval.
These three milestones must remain separate when assessing the 500 ms target.

Earlier pinned measurements and the first pinned CPU capture were confounded
by temporary linked worktrees created for this investigation. GitComet scanned
those additional worktrees, whose generated parser files triggered substantial
normalization work against their indexes. After preserving and removing only
the experimental worktrees, we repeated the measurements and capture. The
clean results above supersede the earlier pinned 635.6 ms draw / 765.0 ms ready
medians; differences between those sessions are not a controlled optimization
gain.

Two diagnostic comparisons then used the exact pinned executable in both arms,
with five alternating pairs each:

| Experiment | Baseline draw / submit / ready | Candidate draw / submit / ready | Median paired submit change |
| --- | ---: | ---: | ---: |
| FXC → staged SDK DXC | 442.4 / 517.0 / 817.3 ms | 444.8 / 521.2 / 799.7 ms | −5.5 ms |
| Transparent → opaque background | 449.4 / 529.2 / 843.5 ms | 456.4 / 480.6 / 837.6 ms | −47.4 ms |

Staged DXC did not establish a compelling submission improvement under the
parallel renderer. Submission-duration medians were 74.1 ms with FXC and
74.2 ms with DXC. All FXC runs deferred; two DXC runs loaded early, confounding
the aggregate ready comparison. The three deferred/deferred pairs favored DXC
by medians of 11.1 ms submission and 17.6 ms ready. Production packaging still
does not bundle the SDK DLLs.

Opaque mode reduced submission duration from a median 86.5 to 21.9 ms. All five
pairs improved, with a median paired duration saving of 61.1 ms.
Launch-to-submit improved in four of five pairs by a median 47.4 ms.
The four deferred/deferred pairs saved
53.9 ms submission but regressed ready by 12.1 ms. Opaque mode can change the
desktop-visible pixels around GitComet's inset rounded frame. The user later
inspected this mode, saw no unwanted difference, and accepted it as the default
in `afcb771a`. This avoids starting opaque and then reconfiguring/rebuilding for
transparency. The table above records the earlier diagnostic, before that
product decision.

## Completed Git-probe handoff

The clean 1 kHz capture deferred bootstrap at app-entry age **89.2195 ms**.
The background Git probe finished at **95.9663 ms**, only **6.7468 ms** later,
but runtime adoption began at **434.0131 ms**. The completed result waited
338.0468 ms while native window/renderer initialization ran. This identifies
an overlap opportunity, not a guaranteed additive end-to-end saving.

GitComet commit `c55d94a43d87a1373a49a4c0402cc8c4ce120a91` lets the prepared
bootstrap adopt the completed probe and dispatch loading before view attachment.
Ownership transfers exactly once; stale or late completion cannot duplicate
recovery. Entry gates, focused tools, missing-Git recovery, multiwindow behavior,
and store release when a prepared window is dropped remain covered.

Validation against GPUI `18e1b2f` passed **168 tests**: bootstrap 12, app 67,
notifications 32, window lifecycle 53, runtime probe 2, and GPUI version contract
2. Formatting and diff checks passed. The retained-diagnostic fixture checks
acknowledgment of its specific seeded diagnostic and retains both exactly-once
toast assertions, allowing unrelated actor-handle errors from the synthetic
repository. Its release measurement is the combined comparison above;
the handoff was not benchmarked independently.

## Deferred DX11 support-device creation

GPUI `164485d7d856c0e57607501480f255d027b92709`, pushed to
`lazy_load_shaders`, creates shared DX11 devices only when DX11 is selected
or actual renderer fallback needs them. DX12 capability probing uses a DXGI
factory without creating a D3D11 device. Software DirectWrite glyph
initialization remains independent of renderer devices.

The windowed redraw/vsync loop runs with an empty DX11 device slot and observes
devices initialized later. Recovery takes an unlocked device snapshot and
publishes platform replacement before notifying windows. Consuming invalidation
before taking that snapshot preserves invalidation from devices created during
the tick. Initial alpha, surface resizing, shader behavior, and public APIs
remain unchanged.

Validation passed **58 Windows library tests**, including nine new regressions
covering unused DX11 devices on DX12 and forced-DX12 failure, retryable/shared
fallback creation, native device reuse, windowed/headless startup, redraw
without DX11, late-device recovery, recovery failure, and late invalidation.
The existing native DX11 pixel, software/headless glyph, and native color-glyph
recovery tests also passed. The final extended windowed/headless smoke test
passed separately.

Strict Clippy passed before the final log wording and smoke-test extension.
Both DX11-only configurations compiled: the default configuration with
test-support and tests, and the minimal production library with
`--no-default-features`. Formatting and diff checks passed on the final files.
Recovery control flow is covered by deterministic tests; physical GPU
removal/reset was not forced. The combined release comparison above does not
isolate this change's saving; the baseline's 33.7 ms device-creation CPU does
not establish an equal wall-time improvement.

## Historical baseline and bottleneck

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

The following earlier experiments used the renderer then vendored in
`vendor/gpui-wgpu`. Their within-session comparisons remain useful, but their
absolute timings and bootstrap-path mixtures must not be pooled with the clean
upstream-pin baseline above.

## Historical smoothing measurements

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
modules confirmed the DLLs used. Once smoothing was deferred, DXC added only a
small improvement in this earlier experiment. The current parallel-renderer
comparison is reported above.

## Historical repository bootstrap overlap

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

## Historical parallel ordinary pipeline comparison

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
History navigation and opening Settings passed live checks on that binary.

Parallel compilation moves work onto a bounded set of threads; these results
do not establish a reduction in total shader compiler CPU. The exact pre-format
compiled/test sources and their relationship to the rustfmt-formatted working
source are retained in the local provenance artifacts.

## Remaining work

The resize, opaque-background, optional-tool, watcher, and early-Home changes
described above are implemented and tested. The remaining priorities are:

- Establish more margin below 500 ms and understand the earlier slower renderer
  state. The later median alone does not establish consistent startup.
- Attribute individual pipelines in a release application capture before
  changing shader code. The bounded 3/4/6/8-worker experiment is complete and
  does not justify changing the current limit.
- Revisit status overlap when it delays first displayed content. It arrives
  after the first CPU draw in three confirmation runs, although it is already
  ready before rendering in both runs that miss 500 ms.
- Keep remaining shader deferral tied to actual scene requirements. Home uses
  a colored image; repository tabs and frame corners use paths. Shadows can be
  required by light-theme controls and focus rings.
- Treat diagnostic logging overhead separately from application work, and keep
  Home callback times separate from spawn-to-submit and repository-ready times.

The final confirmation median is below 500 ms, but its slowest candidate is
12.5 ms above it. The earlier session still required more than 200 ms of
improvement. These are observed gaps, not predicted additive savings.
Public `wgpu::PipelineCache` in version
29.0.4 supports Vulkan only, so it does not provide persistent DX12 shader
caching. More shader deferral requires evidence that the initial frame does
not need those pipelines.

## Validation and reproduction

Four smoothing regression tests passed on the earlier renderer candidate:
first-use pixel equivalence, backdrop/isolated-filter planning, creation rollback, and DX12
retry behavior. Three GPU tests explicitly selected the Radeon 890M/DX12;
the fourth tests planning. History navigation and opening Settings also passed
live UI smoke checks.

Use `scripts/profiling/live-ui.py measure` for alternating startup measurements.
The startup scenario disables updates, network tag fetching, and signature
checks through its isolated profile. The first-submit metric is derived from
the journal-start-to-spawn offset plus the first journal `submit.at_ms`.
Do not subtract timestamps across the distinct app-entry, UI probe, external
spawn, and ETL process-age clock origins. Memory is sampled every 250 ms and can
miss short peaks; it is not an allocation stack trace or exact retained Rust heap.

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
The historical parallel experiment adds `parallel-runtime/`, `repo-parallel-pairs/`,
`repo-parallel-confirmation/`, `parallel-comparison-summary.json`, the
`parallel-stage-*` traces, and `parallel-compiled-source/`.
Their availability is local; this document records the durable measurements.

The latest implementation is frozen in `lean-startup-runtime/` with source
revision `8bd06006`, clean patch, source hashes, and successful build log
`lean-startup-release-build.log`. Executable SHA256:
`d62aa0087d9e9e6e3925b3d35a46ab2c0898e41dd2c0a2e4b496d62aace20584`.
PDB SHA256:
`9c0f0a8165364dfb03b6cbc20536e704b87bb0164bc8cb447bc1025bfc0cb7e9`.
Its evidence includes:

- `lean-startup-comparison.json` and `repo-lean-startup-pairs/session.json`,
  session `b72848e8-ca7a-453f-98ff-fd6965ab6779`: earlier regression.
- `lean-confirmation-comparison.json` and `repo-lean-confirmation/session.json`,
  session `a4beaa49-9fb9-4ca5-8f72-e65bdf7f92f3`: final five-pair confirmation.
- `lean-confirmation-readiness-findings.txt` and
  `lean-confirmation-readiness-evidence.json`: per-run data/publication/frame
  dependencies and threshold counts.
- `lean-stage-home-comparison.json`, `lean-stage-home-findings.txt`,
  `lean-confirm-home-*`, and `lean-home-file-*`: separate stage/Home diagnostics.
- `lean-path-control-comparison.json` and `lean-dx11-control-comparison.json`:
  separate path and renderer controls.
- `lean-watcher-comparison.json` and `lean-watcher-findings.txt`: setup phases,
  attempts, and reload coverage.
- `samply-lean-startup-final/`: valid capture, matching symbols, ETLs,
  foreground evidence, CPU analysis, renderer findings, and
  `status-readiness-decision.txt`. The adjacent stage trace is
  `samply-lean-startup-final-stage.jsonl`.
- `gpui-final-startup-validation.txt` and `lean-startup-app-validation.txt`:
  test commands, complete logs, source hashes, fixture corrections and limits.
- `lean-startup-history-jump/`, `lean-startup-settings/`, and
  `lean-startup-explicit-dx11/`: successful frozen-binary live checks.
- `probe-emission/run-20261020-144202-187/summary.json`: isolated serde_json
  write-count and redirection experiment.
- `probe-emission/validation-20261020-144838/`: diagnostic correction's successful
  build, eleven focused test results, and source/executable hashes.
- `gpui-worker-count-release-flags.log`: bounded-worker control using shipping
  native shader compiler flags. The earlier `gpui-worker-count-diagnostic.log`
  uses debug flags and is retained separately.

The clean pin and runtime comparisons are `pinned-final-comparison.json`,
`pinned-dxc-comparison.json`, and `pinned-opaque-comparison.json`, backed by
`repo-pinned-final-pairs/session.json`, `repo-pinned-dxc-pairs/session.json`,
and `repo-pinned-opaque-pairs/session.json`. The clean diagnostic is
`samply-pinned-final/startup.json.gz`, its retained kernel/user ETLs,
`recording.json`, `process-analysis.json`, `process-analysis.symbols.json`,
`repository-startup-findings.txt`, and `live-ui/frames.jsonl`, with adjacent
`samply-pinned-final-stage.jsonl`. Source interpretation is preserved in
`pinned-renderer-opportunities.md`.

The previous executable SHA256 is
`fd0c7e7592aceb7d405d58d2459f539dfb7fa6df8a7df7ab5daceb5f45979b90`;
`parallel-runtime/provenance.json` maps its preserved sources to integrated
GitComet `b37e714ecf8218a06cd299cb33c5e8a32fb696a5`. The exact-pin executable
SHA256 is `c142745a43484a986df94b092b54bd3ab2ecb045758bd25842a9e4afd40b819d`
and PDB SHA256 is
`b03359b130dbfe55bc967531cbaad6a6378465ad881e8fe2848f4dc34f908d6c`.
`pinned-gpui-runtime/provenance.json` records its `1da63f84` / `18e1b2f`
source pair. DXC experiment DLL versions and hashes are in
`pinned-dxc-runtime/provenance.json`. Session checkout metadata alone does not
identify a separately frozen executable's sources.

The handoff's 168-test results, source hashes, executable hashes, and build/test
logs are under `app-bootstrap-handoff-preserved/`, particularly
`root-validation-summary.json`, `root-validated-source-sha256.json`, and
`root-validated-test-binaries-sha256.json`. Its validation summary records the
pre-commit base `1da63f84`; the source manifest identifies the changes later
committed as `c55d94a4`.

`gpui-lazy-dx11-validation.txt` records the renderer validation performed before
commit `164485d7`, with final smoke-test and minimal-configuration raw output in
`gpui-lazy-dx11-headless.log` and `gpui-lazy-dx11-minimal-check.log`.

The combined measurement is `deferred-dx11-comparison.json`, backed by
`repo-deferred-dx11-pairs/session.json` (session
`29550899-a4d9-4ca3-a73e-380f031cf1d6`). Frozen source/build provenance and
artifacts are in `deferred-dx11-runtime/`. Candidate executable SHA256 is
`9f7ebd94a7d9c38c69fe653f5096d95985e61b47757dd6613c394247cd234706`;
PDB SHA256 is
`0312e433a5f8c4e073581761fc8aa886052a3ba6532e961793c06ddb531ae127`.
The provenance identifies GitComet `7ebb3e91`, GPUI `164485d7`, and the included
handoff source hashes. Valid live-check summaries are
`deferred-dx11-history-jump/summary.json`,
`deferred-dx11-settings/summary.json`, and
`deferred-dx11-explicit-dx11/summary.json`, all with that executable hash.
The successful fresh diagnostic is `samply-deferred-dx11-retry/`, including
`startup.json.gz`, retained ETLs, `recording.json`, `foreground-summary.json`,
`startup-findings.txt`, `graphics-marker-evidence.json`, and
`main-initialization-evidence.json`. Its derived comparisons are
`deferred-dx11-retry-capture-comparison.json` and
`deferred-dx11-renderer-phase-comparison.json`. The failed
`samply-deferred-dx11/` capture is excluded. Separate three-pair stage/Home
results and clock definitions are in `deferred-stage-home-comparison.json`
and `deferred-stage-home-comparison.txt`.

## Historical upstream precompilation

Official GPUI-CE revision `9a82fba4e5f1f9194a5489b7a417d164083dd67a`
precompiles **DX11** shaders in
[gpui_render/build.rs](https://github.com/gpui-ce/gpui-ce/blob/9a82fba4e5f1f9194a5489b7a417d164083dd67a/crates/gpui_render/build.rs).
Its [WGPU pipeline implementation](https://github.com/gpui-ce/gpui-ce/blob/9a82fba4e5f1f9194a5489b7a417d164083dd67a/crates/gpui_wgpu/src/wgpu_renderer/pipelines.rs)
still creates pipelines from WGSL at runtime, including all four smoothing
variants eagerly. DX11 precompilation does not replace the lazy DX12 change.
