# GPU performance inventory

This is every rendering optimization on `perf/consolidate_results` as of
2026-10-11: GitComet after the vendored renderer's removal, and GPUI at
`626422d615`. "Default" means the behavior with no environment overrides. No
entry has a paired A/A baseline yet, so every gain listed is a hypothesis until
the consolidated validation measures it.

## GPUI (gpui-ce)

| Optimization | Commit | Backends | Default | Tests | Evidence |
|---|---|---|---|---|---|
| Shared damage planner (`gpui_render/src/damage.rs`): exact strict comparison, then a 64 px tiled comparison, into a retained target | `001647ec24`…`be63d15823` | D3D11; WGPU, Windows only | Off. `GPUI_GPU_EXPERIMENTS=partial-redraw`, which GitComet sets on Windows | `damage.rs` unit tests; D3D11 `retained_*` pixel tests (Windows); WGPU headless `retained_*` | None in the tree |
| Metal tile-hash damage with Copy, Partial, Retained and Direct frames | `effbe66cc2` | Metal | On. `GPUI_METAL_EXPERIMENTS=no-partial-redraw` turns it off | `gpui_apple` damage unit tests; one Metal pixel test with four Partial fixtures | `macos-gpu-usage.md`: 3 runs in one session |
| Retained copy to the swapchain | `be63d15823` | WGPU DX12 | Off. `retained-copy`, which GitComet sets | WGPU headless, Windows only | None in the tree |
| Cropped path targets | `d0e01e8854` | WGPU | On for Linux. `cropped-paths`, which GitComet sets on Windows | `cropped_path_targets_match_full_viewport_pixels_*` | Profiling README; raw data only under `target/` |
| Quad scissor runs (`quad_clipping.rs`) | `83fa34ea0f` | D3D11, WGPU | Off. `clipped-quads` | `clipped_quads_match_full_pixels_*` | None. Possibly redundant since vertex clipping |
| Quad vertex clipping to the content mask | `c18428832b` | All | On | `vertex_clipping_matches_fragment_masking_on_pixel_center_edges` | Commit message: rasterized quad area went from 10.6× to 3.6× the window |
| Shadow vertex clipping to the content mask | GitComet `8a3b2ec4`, moving to gpui-ce | All | On once ported | Same test | Linux record (the clipped-shadow candidate) |
| Cropped path prepass, at most 3 layers | `c3c2ef65c1` | Metal | On. `no-path-prepass` | Metal pixel test | `macos-gpu-usage.md` |
| Opaque quads, hidden-quad skip, split rounded quads | `c3c2ef65c1`, `caf833a3c3` | Metal | On. `no-opaque-quads`, `no-split-rounded` | Metal pixel test | `macos-gpu-usage.md` |
| Windows input-rate keepalive | `001647ec24` | Windows | GitComet sets `GPUI_WINDOWS_INPUT_RATE_KEEPALIVE=off` | `gpui_windows` unit tests | None in the tree |

## GitComet

| Change | Commit | Platforms | Tests | Evidence |
|---|---|---|---|---|
| Warm inactive repository monitors | `ecf54d9f` | 2 on macOS, 0 elsewhere | `active_repo.rs`, `repo_monitor.rs` | Commit message: switching between three tabs went from 190 to 56 ms/s CPU. A freshness race is open |
| Session-file stamp (length + mtime) | `344bb4f9` | macOS | None | `macos-gpu-usage.md` |
| File-line cache, notify-only hover, timer toasts, 30 fps spinner | `25521fb3` | All | Toast lifetime arithmetic only | `macos-gpu-usage.md` |
| Pixel-aligned graph connectors | `a64ca402` | All | Three count tests in CI; the pixel test is `#[ignore]`d | Linux record |
| No shadows on floating surfaces or the window frame | `b0bf1604`, `4c5ea2fd` | All | Smoke tests | Linux record (shadowless frame) |
| Windows renderer environment defaults | `b0bf1604` | Windows | Defaults table only | None; to be removed |

## Reference builds

In each of these, the manifest and lock agree. `ef3328ac` and `1b460071` don't
build as committed.

| Role | GitComet | GPUI |
|---|---|---|
| Shipping | `fcf34312` (v0.3.0 + packaging) | `f7474ab1` |
| Windows predecessor | `b0bf1604` | `be63d158` |
| macOS predecessor | `df2f597f` | `effbe66c` |
| Linux predecessor | `d22e3e6c` | `f7474ab1` + vendored shadow clip |
