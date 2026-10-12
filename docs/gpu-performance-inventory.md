# GPU performance inventory

This is every rendering optimization on `perf/consolidate_results` as of
2026-10-12: GitComet after the vendored renderer's removal, and GPUI at
`ec31c4aee2`. "Default" means the behavior with no environment overrides. The
macOS runs and the Windows off/on run include matched A/A sessions; the Linux
runs and the Windows single-run spot checks don't.

## GPUI (gpui-ce)

| Optimization | Commit | Backends | Default | Tests | Evidence |
|---|---|---|---|---|---|
| Shared damage planner (`gpui_render/src/damage.rs`): per-64-px-tile hashes of what each frame paints, in painter order and without the global draw order, into a retained target. Dirty tiles repaint as one box, or the whole frame when that box covers over half the target (`82dccfc444`) | `001647ec24`…`82dccfc444` | D3D11, WGPU, Metal | On for D3D11, and for WGPU on Windows and Linux (`8066816f55`). `GPUI_GPU_EXPERIMENTS=no-partial-redraw` turns it off on every backend. Metal: see the next row | `damage.rs` and `retained.rs` unit tests; D3D11 `retained_*` pixel tests (Windows); WGPU headless `retained_*`; the Metal pixel test | Off → on at `528e7d4b`. Linux GTX 1080, 6 pairs: hover GPU ×0.29; scroll GPU ×0.87 (within noise) at +0.07 ms present; select and diff-open neutral. Windows Radeon 890M, single runs against last night's off medians: scroll app GPU 25.9% → 11.6% (DX12) and 15.7% → 7.3% (DX11); select 3.0% → 1.3%. One box against the earlier regions: on real history (a gpui-ce clone, Linux) scroll GPU ×1.34 at ×0.69 present; on Windows scroll 10.0% → 11.6% (DX12) and 5.4% → 7.3% (DX11). Regions cost up to +0.23 ms present in scroll and select |
| Metal Copy, Partial (one scissor over the damage), Retained and Direct frames. Its own tile-hash tracker moved into the shared planner (`d7edc63041`, `c9dd7f32fa`) | `effbe66cc2`, `d7edc63041`, `c9dd7f32fa` | Metal | On. `GPUI_GPU_EXPERIMENTS=no-partial-redraw` turns it off | One Metal pixel test, 15 frames | `macos-gpu-usage.md`. M5 2026-10-12, 6 pairs against the old tracker (`9fcdc3d0`) with a matched A/A: GPU at parity in every scenario, and present +0.006–0.012 ms. Diff-open GPU went from ×1.22 at `7a08c54e` (more than 8 regions or 40% of the window repainted everything) to ×1.05 with one box |
| Retained frames kept while an unfocused window is hovered: present publishes focus or the platform's hover state, and leaving repaints once, which frees the target, also when the exit arrives mid-draw | `65fd1428c0`, `995982f372` | D3D11, WGPU | Same as the shared damage planner | `unfocused_windows_keep_their_retained_frame_while_hovered` and `a_pointer_exit_during_a_draw_still_frees_the_retained_frame` (`test_app.rs`); WGPU `retained_targets_return_their_budget_at_once`; D3D11 `unfocused_repaints_return_the_retained_budget` (Windows) | Radeon 890M, unfocused history sweeps, 3 pairs each, measured with the Windows patch it consolidates: app GPU 18.2% → 5.0% (DX12, 2100×1350), 26.5% → 7.0% (DX12, 2790×1650), 17.0% → 3.5% (DX11) at the same frame rate; DX11 submit CPU p95 1.18 → 1.81 ms. The clean `528e7d4b` build is within ±2% of that patch, and with a real cursor unfocused hover equals focused (1.05% app GPU) |
| Retained copy to the swapchain | `be63d15823` | WGPU DX12 | On with partial redraw (`8066816f55`) | WGPU headless, Windows only | Windows off/on runs, with the planner |
| Full frames when the retained texture can't be allocated (one out-of-memory error scope) | `11e35b011b` | WGPU (native) | On | None: a GPU allocation can't be made to fail without an injection hook | None |
| Cropped path targets | `d0e01e8854` | WGPU | On (`8066816f55`, switch removed in `ec31c4aee2`) | `cropped_path_targets_match_full_viewport_pixels_*` | Profiling README; raw data only under `target/` |
| Quad vertex clipping to the content mask | `c18428832b` | All | On | `vertex_clipping_matches_fragment_masking_on_pixel_center_edges` | Commit message: rasterized quad area went from 10.6× to 3.6× the window |
| Shadow vertex clipping to the content mask | `30e0192c0c` (from GitComet `8a3b2ec4`) | All | On | Same test | Linux record (the clipped-shadow candidate) |
| Cropped path prepass, at most 3 layers | `c3c2ef65c1` | Metal | On; opt-out removed in `8066816f55` | `path_prepass_plans_layers_by_sampled_region` (the pixel comparison against per-batch passes went with the switch in `ec31c4aee2`) | `macos-gpu-usage.md` |
| Opaque quads, hidden-quad skip, split rounded quads | `c3c2ef65c1`, `caf833a3c3` | Metal | On; opt-outs removed in `8066816f55` | `only_plain_opaque_quads_skip_blending`, `only_large_opaque_rounded_quads_split` | `macos-gpu-usage.md` |
| Input-rate keepalive skipped on X11, whose windows never get variable refresh | `76c3d5bd29` | X11 | On | Keepalive policy test in `test_app.rs` | None |

## GitComet

| Change | Commit | Platforms | Tests | Evidence |
|---|---|---|---|---|
| Warm inactive repository monitors | `ecf54d9f` | 2 on macOS, 0 elsewhere | `active_repo.rs`, `repo_monitor.rs` | Commit message: switching between three tabs went from 190 to 56 ms/s CPU. A freshness race is open |
| Session-file stamp (length + mtime) | `344bb4f9` | macOS | None | `macos-gpu-usage.md` |
| File-line cache, notify-only hover, timer toasts, 30 fps spinner | `25521fb3` | All | Toast lifetime arithmetic only | `macos-gpu-usage.md` |
| Pixel-aligned graph connectors | `a64ca402` | All | Three count tests in CI; the pixel test is `#[ignore]`d | Linux record |
| No shadows on floating surfaces or the window frame | `b0bf1604`, `4c5ea2fd` | All | Smoke tests | Linux record (shadowless frame) |

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

Removed in `8066816f55` and `ec31c4aee2`: the `partial-redraw`, `retained-copy`
and `cropped-paths` tokens, `GPUI_METAL_EXPERIMENTS`, the
`GPUI_PROFILE_PATH_TARGET` diagnostic, the `path_target_footprint` example, and
the test-only switches for Metal's optimizations and WGPU path cropping. GitComet
no longer sets Windows renderer environment defaults (`b0bf1604`): GPUI now
defaults to the same policy.
