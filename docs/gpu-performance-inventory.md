# GPU performance inventory

This is every rendering optimization on `perf/consolidate_results` as of
2026-10-13: GitComet after the vendored renderer's removal, and GPUI at
`020fb167`, the fork's `main`. There, gpui-ce PR #29 is squashed into
`6900056152` and followed by the upstream merge #30. The GPUI commits below are
#29's own, which the pull request keeps reachable. "Default" means the behavior
with no environment overrides. The macOS runs and the Windows off/on run include
matched A/A sessions; the Linux runs and the Windows single-run spot checks
don't.

## GPUI (gpui-ce)

| Optimization | Commit | Backends | Default | Tests | Evidence |
|---|---|---|---|---|---|
| Shared damage planner (`gpui_render/src/damage.rs`): per-64-px-tile hashes of what each frame paints, in painter order and without the global draw order, into a retained target. Path hashes include their batch's membership, each member's bounds and mask, since coincident paths share MSAA samples only within one batch (`5a37d2def7`, `642e6b94eb`). Underlines damage only their bounds, which is all their shader rasterizes (`6a13833d1d`). A target without room in the budget draws in full before hashing anything (`4c199b31fc`). Dirty tiles repaint as one box, or the whole frame when that box covers over half the target (`82dccfc444`) | `001647ec24`…`82dccfc444` | D3D11, WGPU, Metal | On for D3D11, and for WGPU on Windows and Linux (`8066816f55`); no run-time switch since `409338f77a`. Metal: see the next row | `damage.rs` and `retained.rs` unit tests; D3D11 `retained_*` pixel tests (Windows); WGPU headless `retained_*`; the Metal pixel test | Off → on at `528e7d4b`. Linux GTX 1080, 6 pairs: hover GPU ×0.29; scroll GPU ×0.87 (within noise) at +0.07 ms present; select and diff-open neutral. Windows Radeon 890M, single runs against last night's off medians: scroll app GPU 25.9% → 11.6% (DX12) and 15.7% → 7.3% (DX11); select 3.0% → 1.3%. One box against the earlier regions: on real history (a gpui-ce clone, Linux) scroll GPU ×1.34 at ×0.69 present; on Windows scroll 10.0% → 11.6% (DX12) and 5.4% → 7.3% (DX11). Regions cost up to +0.23 ms present in scroll and select |
| Metal Copy, Partial (one scissor over the damage), Retained and Direct frames. Its own tile-hash tracker moved into the shared planner (`d7edc63041`, `c9dd7f32fa`) | `effbe66cc2`, `d7edc63041`, `c9dd7f32fa` | Metal | On | One Metal pixel test, 15 frames | [macOS](#macos); M5 2026-10-12, 6 pairs against the old tracker (`9fcdc3d0`) with a matched A/A: GPU at parity in every scenario, and present +0.006–0.012 ms. Diff-open GPU went from ×1.22 at `7a08c54e` (more than 8 regions or 40% of the window repainted everything) to ×1.05 with one box |
| Retained frames kept while an unfocused window is hovered: present publishes focus or the platform's hover state, and leaving repaints once, which frees the target, also when the exit arrives mid-draw | `65fd1428c0`, `995982f372` | D3D11, WGPU | Same as the shared damage planner | `unfocused_windows_keep_their_retained_frame_while_hovered` and `a_pointer_exit_during_a_draw_still_frees_the_retained_frame` (`test_app.rs`); WGPU `retained_targets_return_their_budget_at_once`; D3D11 `unfocused_repaints_return_the_retained_budget` (Windows) | Radeon 890M, unfocused history sweeps, 3 pairs each, measured with the Windows patch it consolidates: app GPU 18.2% → 5.0% (DX12, 2100×1350), 26.5% → 7.0% (DX12, 2790×1650), 17.0% → 3.5% (DX11) at the same frame rate; DX11 submit CPU p95 1.18 → 1.81 ms. The clean `528e7d4b` build is within ±2% of that patch, and with a real cursor unfocused hover equals focused (1.05% app GPU) |
| Retained copy to the swapchain. Each frame checks only what can differ: the frame texture's COPY_DST usage, size and format (`6e66dcf13f`) | `be63d15823` | WGPU DX12 | On with partial redraw (`8066816f55`) | WGPU headless, Windows only | Windows off/on runs, with the planner |
| Full frames when the retained texture can't be allocated (WGPU: one out-of-memory error scope; D3D11: a failed texture or view creation) | `11e35b011b`, `702db5bf6c` | WGPU (native), D3D11 | On | None: no allocation can be made to fail without an injection hook. WARP, CI's only D3D11 device, even creates textures past the documented 16384 limit (`8405259c87`) | None |
| Cropped path targets | `d0e01e8854` | WGPU | On (`8066816f55`, switch removed in `ec31c4aee2`) | `cropped_path_targets_match_full_viewport_pixels_*` | [Linux](#linux), 2026-10-05, together with two experiments that never shipped |
| Quad vertex clipping to the content mask. Masks are half-open like the rasterizer for every primitive (left/top edge pixels in, right/bottom out), so clipped and fragment-masked primitives agree on edge pixels (`28045891bf`) | `c18428832b`, `28045891bf` | All | On | `vertex_clipping_matches_fragment_masking_on_pixel_center_edges`, `clipped_quads_and_underlines_share_mask_edge_pixels` | Commit message: rasterized quad area went from 10.6× to 3.6× the window |
| Shadow vertex clipping to the content mask | `30e0192c0c` (from GitComet `8a3b2ec4`) | All | On | Same test | [Linux](#linux) (the clipped-shadow candidate) |
| Cropped path prepass, at most 3 layers | `c3c2ef65c1` | Metal | On; opt-out removed in `8066816f55` | `path_prepass_plans_layers_by_sampled_region`; `path_prepass_draws_what_per_batch_passes_draw` compares pixels with the per-batch fallback, which transparent batches past the layer limit force (`e3073b2357`) | [macOS](#macos) |
| Opaque quads, hidden-quad skip, split rounded quads | `c3c2ef65c1`, `caf833a3c3` | Metal | On; opt-outs removed in `8066816f55` | `only_plain_opaque_quads_skip_blending`, `only_large_opaque_rounded_quads_split`, `opaque_and_split_quads_match_nearly_opaque_blending` (`49748b0105`) | [macOS](#macos) |
| Input-rate keepalive skipped on X11, whose windows never get variable refresh | `76c3d5bd29` | X11 | On | Keepalive policy test in `test_app.rs` | None |

## GitComet

| Change | Commit | Platforms | Tests | Evidence |
|---|---|---|---|---|
| Warm inactive repository monitors; tabs left mid-refresh, switch races and dead monitors refresh (`9f2b6ae4`) | `ecf54d9f`, `9f2b6ae4` | 2 on macOS, 0 elsewhere | `active_repo.rs`, `repo_monitor.rs`, monitor `synchronization.rs` | [macOS](#macos): switching between three tabs went from 190 to 56 ms/s CPU |
| Session-file stamp (length + mtime) | `344bb4f9` | macOS | `load_if_changed_rereads_only_a_written_file` | [macOS](#macos) |
| File-line cache, notify-only hover, timer toasts, 30 fps spinner | `25521fb3` | All | `file_backed_lines_are_read_once_across_frames`, `expiring_toasts_fade_out_then_leave` | [macOS](#macos) |
| Pixel-aligned graph connectors | `a64ca402` | All | Count tests for the quads and for each fallback to a path | [Linux](#linux) |
| No shadows on floating surfaces or the window frame; tooltips get a stroke instead | `b0bf1604`, `4c5ea2fd` | All | Smoke tests | [Linux](#linux) (shadowless frame) |

## Measurements

### macOS

MacBook Air (Apple M5, 8 GPU cores), macOS 27.0.1, 2560×1664 at 60 Hz, with a
1400×900-point (2800×1706-pixel) window. `live-ui.py --gpu` reads each process's
Metal GPU time (`accumulatedGPUTime` of its AGX clients, as Activity Monitor
does), whole-GPU energy and P-states (IOReport), and CPU time and energy
(`proc_pid_rusage`); none of it needs root. "GPU per frame" is the app's GPU
time per drawn frame. "GPU power" covers every process, WindowServer included.
Tables show medians of 3 interleaved runs.

Before the renderer fixes, Metal System Trace showed 11 render passes per frame:
five path batches each resolved a full-window 4× MSAA texture. Quads rasterized
10.6× the window area while 3.6× was visible, and every quad blended, so stacked
opaque backgrounds were all shaded. Fixing those three cut a hover frame from
8.48 to 3.69 ms and whole-GPU power during the sweep from 768 to 245 mW. Splitting
large rounded opaque quads then took it to 2.75 ms and 190 mW.

Partial redraw off and on, same build (the off switch is gone since):

| Interaction | GPU per frame off | on | GPU power off | on |
| --- | ---: | ---: | ---: | ---: |
| Hover sweep | 2.64 ms | 1.27 ms | 182 mW | 92 mW |
| History scroll | 2.60 ms | 2.02 ms | 198 mW | 147 mW |
| Commit selection | 2.62 ms | 1.95 ms | 63 mW | 48 mW |
| Open diffs | 2.72 ms | 2.07 ms | 26 mW | 22 mW |
| Switch repository tabs | 2.60 ms | 2.26 ms | 21 mW | 19 mW |

- Against the original build, a hover frame costs 85% less GPU time
  (8.5 → 1.27 ms), and whole-GPU power during the sweep is 88% lower
  (768 → 92 mW).
- Process CPU didn't change beyond run-to-run noise. Damage tracking costs about 1 ms/s.
- WindowServer still composites the whole window, about 76–79 ms/s of its GPU
  time during hover. Copying the retained texture to the drawable, about
  0.46 ms at the lowest clock, is most of a hover frame.
- These runs predate the shared planner and its one-box rule. The M5 parity
  run in the table above covers that move.

Warm monitors, 30 switches between three repositories:

| | Before | After |
| --- | ---: | ---: |
| Process CPU | 190 ms/s | 56 ms/s |
| Process CPU energy | 113 mW | 23 mW |
| State applications | 340 | 185 |
| Input to first draw (p50) | 15 ms | 16 ms |

The `repo-tabs-edit` scenario writes a file in the inactive tab and switches
back. The file is listed 11 ms after the switch.

The GitComet CPU fixes came from these findings:
- Each frame re-read every visible diff line from disk.
- Each publication read and parsed the session file on the UI thread.
- Success and warning toasts animated a 6–10 s hold, redrawing the window at
  display rate.
- Spinners redrew at display rate.
- The refs hover card called `window.refresh()`, invalidating every cached pane.

Measured but not changed:
- **Hover CPU:** a hovered row re-renders the whole history view, about half of
  a hover frame's main-thread time.
- **Diff open:** about 38 ms of main-thread time and 5 frames. Four store
  publications each re-render the main pane, and a notify during paint costs
  2–3 follow-up frames.
- **GPUI bookkeeping per frame:** bounds-tree inserts take about 8% of the main
  thread, replaying cached views about 8%, and tab stops about 5%.
- **Idle:** the CVDisplayLink wakes the main thread every vsync while the window
  is visible, about 40 wakeups/s.

### Linux

NVIDIA GTX 1080 (driver 580.178.04, Vulkan on Wayland), a private 60 Hz Mutter
display, 100% UI scale and 2560×1440. Phases ran 30 s, with 125 pointer events/s
or 20 witnessed diff or tab changes/s. Values are app-process SM activity from
`nvidia-smi pmon` at one-second resolution: activity, not GPU execution time.
Pairs whose graphics or memory clocks differed by more than 5% were excluded.
Raw captures stay under `target/performance/gpu-efficiency/`.

| Interaction | Original | Shadowless release | Paired reduction (95% CI) |
| --- | ---: | ---: | ---: |
| History hover | 13.43% | 7.32% | 46.9% (39.1–47.7%) |
| Diff changes | 10.44% | 6.56% | 38.6% (35.2–40.8%) |
| Repository tabs | 6.45% | 4.38% | 32.1% (17.1–46.5%) |

- The clipped-shadow candidate, before the frame shadow went, had already cut
  41.1% (hover), 34.3% (diffs) and 34.1% (tabs) over five pairs. Hover and diff
  framebuffer use fell from 117 to 85 MB.
- Removing the frame shadow saved another 5.7% (hover) and 11.8% (tabs), with no
  consistent gain for diffs.
- Path cropping was measured on 2026-10-05 together with path sharing and
  pooling, neither of which shipped. Hover GPU p50 went from 1.32 to 0.92 ms
  at matched clocks (two and three samples, so descriptive only), and app
  framebuffer from 269 to 205 MB.

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
no longer sets the Windows renderer environment defaults that `b0bf1604` added
(`6631f7a3`): GPUI now defaults to the same policy.

A review round on 2026-10-12 (`b9397ac50e`…`28045891bf`) fixed a leak, in which
the WGPU blur bind-group cache kept released retained targets alive (each target
now owns its blit). It also removed waste and duplication:
- double instance uploads;
- per-frame allocations in the damage planner and in Metal's quad split and path prepass;
- D3D11 fence retirement and `GPUI_WINDOWS_DX12_PRESENT_MODE`;
- duplicate Metal pipeline, copy and scissor helpers;
- per-frame re-validation of the retained copy;
- unused primitive `PartialEq` derives;
- a `String` allocated for every retained-budget reservation.

New tests cover underline and polychrome damage and regrouped path batches,
which review found the planner missed (`5a37d2def7`, `642e6b94eb`), and quads and
underlines that disagreed at mask edges on pixel centers (`28045891bf`). `GPUI_GPU_EXPERIMENTS` is
gone (`409338f77a`), so partial redraw has no run-time switch. CI now also
compiles the Windows WGPU renderer, which GitComet ships.
