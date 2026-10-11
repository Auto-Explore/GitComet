# GPU performance inventory

This is every rendering optimization on `perf/consolidate_results` as of
2026-10-11: GitComet after the vendored renderer's removal, and GPUI at
`7a08c54e62`. "Default" means the behavior with no environment overrides. No
entry has a paired A/A baseline yet, so every gain listed is a hypothesis until
the consolidated validation measures it.

## GPUI (gpui-ce)

| Optimization | Commit | Backends | Default | Tests | Evidence |
|---|---|---|---|---|---|
| Shared damage planner (`gpui_render/src/damage.rs`): per-64-px-tile hashes of what each frame paints, in painter order and without the global draw order, into a retained target | `001647ec24`…`c9dd7f32fa` | D3D11, WGPU, Metal | D3D11 and WGPU: off. `GPUI_GPU_EXPERIMENTS=partial-redraw`, which GitComet sets on Windows. Metal: see the next row | `damage.rs` and `retained.rs` unit tests; D3D11 `retained_*` pixel tests (Windows); WGPU headless `retained_*`; the Metal pixel test | Linux GTX 1080, 6 pairs: hover GPU ×0.21 at no CPU cost, scroll GPU ×0.35 at +0.19 ms present; select +0.23 ms present |
| Metal Copy, Partial (one scissor over the damage), Retained and Direct frames. Its own tile-hash tracker moved into the shared planner (`d7edc63041`, `c9dd7f32fa`) | `effbe66cc2`, `d7edc63041`, `c9dd7f32fa` | Metal | On. `GPUI_METAL_EXPERIMENTS=no-partial-redraw` turns it off | One Metal pixel test, 15 frames | `macos-gpu-usage.md`; M5 2026-10-11: the interim exact planner cost ×2.4 hover present and ×1.26 scroll GPU against the tracker |
| Retained copy to the swapchain | `be63d15823` | WGPU DX12 | Off. `retained-copy`, which GitComet sets | WGPU headless, Windows only | None in the tree |
| Cropped path targets | `d0e01e8854` | WGPU | On for Linux. `cropped-paths`, which GitComet sets on Windows | `cropped_path_targets_match_full_viewport_pixels_*` | Profiling README; raw data only under `target/` |
| Quad vertex clipping to the content mask | `c18428832b` | All | On | `vertex_clipping_matches_fragment_masking_on_pixel_center_edges` | Commit message: rasterized quad area went from 10.6× to 3.6× the window |
| Shadow vertex clipping to the content mask | `30e0192c0c` (from GitComet `8a3b2ec4`) | All | On | Same test | Linux record (the clipped-shadow candidate) |
| Cropped path prepass, at most 3 layers | `c3c2ef65c1` | Metal | On. `no-path-prepass` | Metal pixel test | `macos-gpu-usage.md` |
| Opaque quads, hidden-quad skip, split rounded quads | `c3c2ef65c1`, `caf833a3c3` | Metal | On. `no-opaque-quads`, `no-split-rounded` | Metal pixel test | `macos-gpu-usage.md` |
| Input-rate keepalive skipped on X11, whose windows never get variable refresh | `76c3d5bd29` | X11 | On | Keepalive policy test in `test_app.rs` | None |

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

Removed in `4194aee0aa`: quad scissor runs (`clipped-quads`, redundant with
vertex clipping), GPU timestamp telemetry, renderer capability records,
diagnostic ablations, the scene-diff dump, Metal frame stats and dumps,
`GPUI_TRACE_NOTIFY`, the Windows native input hooks and the Windows keepalive
override. GPU work is measured outside the app (PDH, PresentMon, nvidia-smi,
powermetrics).
