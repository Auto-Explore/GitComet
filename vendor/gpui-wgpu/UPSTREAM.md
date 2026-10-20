# GPUI WGPU renderer

This directory vendors the `gpui_ce_wgpu` crate from
[`Havunen/gpui-ce`](https://github.com/Havunen/gpui-ce), licensed under Apache 2.0
(see `LICENSE-APACHE`).

The base is the same revision used by GitComet's other GPUI crates:
`0665fb07d59cf7af185e58f44b9f890590abe204`. The renderer source includes local
GPUI commit `9362a02d98324c6d91a1bec58b90ab22ab667252`,
`perf(wgpu): create smoothing pipelines on first use`. That commit is not
required to be available from a remote: all of the renderer source is included
here.

## Changes

The four smoothing pipelines are created only when a frame needs them. They are
prepared before that frame is encoded, preserving the existing shaders and
rendering quality. Failed creation removes only newly created pipelines and
uses the existing DX12 frame retry policy. Device/renderer reconstruction
starts with no smoothing pipelines.

The change affects these files relative to the base revision:

- `src/wgpu_renderer.rs`
- `src/wgpu_renderer/drawing.rs`
- `src/wgpu_renderer/filters.rs`
- `src/wgpu_renderer/frame.rs`
- `src/wgpu_renderer/lazy_smoothing_tests.rs`
- `src/wgpu_renderer/pipelines.rs`

All other upstream source files, tests, examples, and the license are retained
unchanged. `Cargo.toml` expands the upstream workspace dependencies and lints,
and resolves GPUI sibling crates from the pinned base revision. GitComet's root
manifest patches only `gpui_ce_wgpu`; it does not change the selected renderer
or the other GPUI crates.

## Validation

The exact renderer source was built in GitComet's `release-with-debug` profile
and tested on Windows with the AMD Radeon 890M using DX12. Four focused tests
passed: first-use rendering matched prepared-pipeline pixels, backdrop and
isolated-filter planning worked, failed creation rolled back correctly, and
the existing DX12 frame retry behavior was preserved. History navigation and
settings were also exercised in the application.

## Updating

When changing GitComet's GPUI revision, compare this crate with
`crates/gpui_wgpu` at the new revision. If upstream includes the same behavior,
remove this directory, its workspace exclusion, and the root git-source patch.
Otherwise, apply the smoothing changes to the new crate, expand its workspace
manifest values again, and update every sibling dependency revision here to
match the root GPUI revision.

Keep the upstream tests and run the smoothing tests on DX12 after an update.
Do not import temporary measurement instrumentation or absolute paths from an
experiment checkout.
