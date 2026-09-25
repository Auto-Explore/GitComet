# Ad-hoc workflow profiling

GitComet's perf suite (`scripts/run-full-perf-suite.sh`) measures fixed
benchmarks offline. This is the other half: driving the real app by hand
through a specific workflow — open a large repository, scroll a long diff,
stage four hundred files — and capturing what the CPU, the allocator and the
GPU did while it happened.

Everything here is behind cargo features that are **off by default**. A normal
release build compiles none of it, keeps the plain mimalloc global allocator,
and registers no extra actions.

## Quick start

```bash
# CPU + allocations
scripts/profile-workflow.sh -- /path/to/some/repo

# ...and per-frame GPU execution time
scripts/profile-workflow.sh --gpu -- /path/to/some/repo
```

In the running app, `Ctrl-Alt-P` (`Cmd-Alt-P` on macOS) starts a capture;
pressing it again stops it and writes the report. The path is printed to
stderr.

To see the timeline live, attach the Tracy profiler at any point. Tracy runs
in on-demand mode, so nothing is collected until it connects and the app pays
almost nothing while it is not.

## What you get

### The Tracy timeline

Connect Tracy and you see one merged view:

* **CPU zones** from `#[profiling::function]` and `profiling::scope!`. This
  includes zones already present inside gpui-ce — window draw, presentation,
  text shaping, atlas tile allocation — so UI work is visible without
  annotating anything yourself.
* **Frame marks** at each present, from gpui-ce's existing
  `profiling::finish_frame!`. The timeline is frame-aligned and Tracy plots
  frame rate for free.
* **Allocations**, every alloc and free with a 16-frame call stack, because
  the workflow-profiler build wraps the global allocator in Tracy's tracker.
  Tracy's allocation views then attribute bytes to call sites and show
  what a workflow leaked or churned.
* **GPU zones** in a `--gpu` build: one span per frame showing how long the
  GPU actually spent executing it, on its own clock.
* **The workflow itself** as a single named zone spanning start to stop, so
  everything above can be filtered to just the region you care about.

Annotate any function you want named in the profile:

```rust
#[profiling::function]
fn expensive_thing(&mut self) { /* ... */ }

// or a sub-function region
profiling::scope!("diff hunk layout");
```

These compile to nothing in a normal build.

### The capture report

Stopping a capture writes a JSON sidecar to
`target/workflow-captures/<name>.json` (override with `--out-dir`, or the
`GITCOMET_WORKFLOW_CAPTURE_DIR` environment variable). It uses the same
`PerfSidecarReport` shape as the Criterion suite, so the existing comparison
tooling can read it:

```json
{
  "bench": "workflow_1756000000",
  "runner": { "hostname": "...", "os": "linux", "arch": "x86_64", "cpu_count": 32 },
  "metrics": {
    "wall_nanos": 8123456789,
    "alloc_alloc_ops": 412233,
    "alloc_alloc_bytes": 91230412,
    "alloc_net_alloc_bytes": 12040100,
    "frame_count": 431,
    "frame_interval_p50_nanos": 8210000,
    "frame_interval_p99_nanos": 31940000,
    "gpu_frame_count": 430,
    "gpu_frame_p50_nanos": 1240000,
    "gpu_frame_max_nanos": 9120000
  }
}
```

The allocation figures come from the same `StatsAlloc` counters the benchmark
suite uses, so a workflow capture and a benchmark sidecar are directly
comparable.

`frame_interval_*` is the gap between gpui frame-loop ticks. That tracks the
delivered frame rate closely, but it is not an exact present count: gpui runs
next-frame callbacks before deciding whether the window is dirty enough to
draw, so an idle tick counts too. When you need an exact presented-frame
count, use `gpu_frame_count` or Tracy's frame marks. Note also that observing
frames opts the window into gpui's inactive/thermal frame-rate throttling, so
a capture can show a slightly different cadence than an unobserved run.

`gpu_frame_*` is GPU execution time only, and appears solely in a `--gpu`
build.

## How GPU timing works

Real GPU execution time cannot be measured from GitComet: the app never
touches wgpu. It is measured inside gpui-ce's renderer and handed back.

`gpui_wgpu`, built with its `gpu-timing` feature, writes a timestamp query
before and after each frame's command encoder, resolves the pair into a
readback buffer, and reads it a few frames later without ever blocking on the
GPU. Completed spans go to a callback that GitComet installs in
`perf_capture::init`, which records them and republishes them as Tracy GPU
zones.

With the feature off, no query set is created and the frame path is exactly
what it was.

### The gpui-ce checkout

`--gpu` needs a gpui-ce checkout carrying that change, on branch
`feat/gpu-timing`.

> **Temporary: gpui-ce is currently loaded from disk.** The workspace
> `Cargo.toml` carries a `[patch."https://github.com/Havunen/gpui-ce.git"]`
> block pointing at `/home/sampo/git/gpui-ce`, which overrides the pinned
> `rev` for *every* build in this workspace, not just profiling ones. It is
> there so the profiler can be tested end to end before the change is
> upstreamed.
>
> To go back to the pinned rev: delete that block, drop
> `"gpui_wgpu/gpu-timing"` from the `workflow-profiler-gpu` feature in
> `crates/gitcomet-ui-gpui/Cargo.toml`, and run
> `cargo update -p gpui -p gpui_platform -p gpui_wgpu`.
>
> To make it permanent instead: push `feat/gpu-timing` to
> `Havunen/gpui-ce`, bump the three `gpui*` revs in the workspace
> `Cargo.toml`, and delete the `[patch]` block. The feature reference stays as
> it is.

Without the `[patch]` block the script falls back to patching gpui-ce in per
invocation with `cargo --config`, leaving the committed manifests untouched;
it looks for `../gpui-ce` by default, overridable with `--gpui-ce PATH`. That
path needs `"gpui_wgpu/gpu-timing"` removed from the feature list again,
since cargo resolves feature references against the pinned rev.

## Getting Tracy

This build links `tracy-client` 0.18.4 / `tracy-client-sys` 0.28.0, which
bundles **Tracy 0.13.1**. The profiler refuses to connect across a protocol
mismatch, so use a 0.13.x profiler:

```bash
git clone --branch v0.13.1 --depth 1 https://github.com/wolfpld/tracy.git
cmake -S tracy/profiler -B tracy/profiler/build -G Ninja -DCMAKE_BUILD_TYPE=Release
ninja -C tracy/profiler/build
./tracy/profiler/build/tracy-profiler
```

Releases at <https://github.com/wolfpld/tracy/releases> also carry prebuilt
binaries. Start GitComet first, then hit Connect in the profiler — GitComet
appears under "Discovered clients" while it is running.

## Cost when enabled

`release-with-debug` is the default profile: optimised like release, but with
the symbols Tracy needs to name zones and allocation call stacks. Expect the
allocator wrapper to be the dominant overhead when a profiler is attached,
since every allocation walks 16 stack frames. Lower
`perf_capture::ALLOC_CALLSTACK_DEPTH` if that distorts the workflow you are
measuring.

## Related tooling

* `scripts/run-full-perf-suite.sh` — the offline Criterion suite, idle and
  launch harnesses, and budget report.
* `scripts/compare-perf-runs.sh` — diffs two sets of sidecars.
* `scripts/profile-gitcomet-process-tree.sh` — callgrind, strace and git
  trace2 across the whole process tree, for subprocess-heavy investigations.
