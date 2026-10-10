# macOS CPU and GPU usage

Work log for the `macOS_gpu_usage` branch: how GitComet's CPU and GPU load is
measured on macOS, what the baseline is, and which changes reduce it. The target
interactions are hovering history rows, opening diffs and switching repository
tabs, plus idle.

## Plan

1. **Measure on macOS without root.**
   - Per-process GPU time comes from the AGX driver's accounting in the
     IORegistry: `accumulatedGPUTime` of each Metal client, keyed by its creating
     PID. This is the same counter Activity Monitor's GPU column uses. WindowServer
     is sampled the same way, and the device's utilization comes along with it.
   - CPU time, idle wakeups, instructions and the kernel's CPU energy estimate come
     from `proc_pid_rusage`.
   - Both are wired into the existing `live-ui.py` scenario harness (`--gpu`).
2. **Script the target interactions.**
   - `history-hover` already existed.
   - `diff-open` (new) opens each changed file's diff in turn.
   - `repo-tabs` (new) cycles three open repositories with `next-repo-tab`.
   - `idle`, `history-select` and `history-scroll` already existed and act as
     controls.
   - `macos-usage.py` runs the set with interleaved repeats.
3. **Attribute the cost.**
   - Instruments Metal System Trace gives render passes per frame and GPU time per
     pass (`xctrace_table.py` parses the export).
   - An opt-in GPUI scene dump (`GPUI_SCENE_STATS`) gives primitive counts,
     overdraw and path batches.
   - Instruments Time Profiler covers the CPU side.
4. **Reduce the GPU work per frame** in the gpui-ce Metal renderer: path batches
   that split the frame into extra full-window passes, and overdraw by stacked
   opaque backgrounds.
5. **Reduce the number of frames and the CPU per frame** in GitComet: hover
   invalidation scope, redundant publications and re-renders on diff open and tab
   switch, and continuous redraws (spinners, toast animation, idle display-link
   ticks).
6. **Verify every change** with repeated A/B runs of the same scenarios, a Metal
   System Trace before and after, visual checks and the existing tests.

## Method

The harness launches the application from an app bundle with an isolated
profile, drives scripted input through the production handlers, and records:
- per-frame CPU draw records from the UI probe;
- process samples every 250 ms;
- GPU samples every 250 ms.

Run the set:

```sh
python3 scripts/profiling/live-ui.py fixture target/profiling/live-fixture
python3 scripts/profiling/live-ui.py clone ~/path/to/GitComet target/profiling/gitcomet-clone
python3 scripts/profiling/live-ui.py clone ~/path/to/large-repo target/profiling/zed-clone
# diff-open needs unstaged edits in target/profiling/gitcomet-clone (at least two files)
python3 scripts/profiling/macos-usage.py run --binary path/to/GitComet.app/Contents/MacOS/gitcomet \
  --label baseline --output target/performance/macos-gpu/baseline
python3 scripts/profiling/macos-usage.py report target/performance/macos-gpu/baseline
```

Use an executable inside an app bundle. A bare `gitcomet` binary copies itself
into `GitComet.app`, relaunches from there and exits, so the measured process
would be gone in about a second.

Column meanings:

| Column | Meaning |
| --- | --- |
| `gpu ms/s` | The process's GPU execution time per second of the phase; 1000 means one GPU fully busy |
| `gpu/frame` | `gpu ms/s` divided by drawn frames per second |
| `ws gpu ms/s` | WindowServer's GPU time, which includes compositing every other application on screen |
| `cpu mW` | The kernel's estimate for the process's CPU energy only |

Machine: MacBook Air (Apple M5, 10 CPU cores, 8 GPU cores, 16 GB), macOS 27.0.1,
built-in 2560x1664 display at 60 Hz. The window is 1400x900 points, which is
2800x1706 pixels.

## Result

Interleaved A/B in one session: the dev revision (`fcf34312`) against this
branch, 3 runs of each case, medians.
- "GPU power" is whole-GPU energy from IOReport (all processes).
- "CPU power" is the kernel's CPU energy estimate for the GitComet process.
- The hover sweep, scroll and selection cases drive input at display rate or
  close to it, so they show the per-frame cost at its worst.

| Interaction | CPU ms/s (before → after) | App GPU ms/s (before → after) | GPU ms/frame (before → after) | GPU power mW (before → after) | CPU power mW (before → after) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Hover sweep (53 fps) | 299 → 280 (-6%) | 448 → 194 (-57%) | 8.48 → 3.69 (-56%) | 768 → 245 (-68%) | 44 → 40 (-10%) |
| History scroll | 300 → 285 (-5%) | 467 → 215 (-54%) | 7.84 → 3.61 (-54%) | 824 → 266 (-68%) | 42 → 39 (-6%) |
| Commit selection (keyboard) | 147 → 138 (-6%) | 148 → 61 (-59%) | 8.89 → 3.63 (-59%) | 265 → 82 (-69%) | 31 → 30 (-4%) |
| Open diffs | 76 → 75 (-2%) | 39 → 22 (-45%) | 7.17 → 3.58 (-50%) | 102 → 34 (-66%) | 59 → 59 (-1%) |
| Switch repo tabs | 188 → 53 (-72%) | 36 → 17 (-52%) | 7.49 → 3.59 (-52%) | 78 → 29 (-63%) | 106 → 21 (-81%) |
| Idle (focused) | 4.11 → 4.20 (+2%) | 0.17 → 0.07 (-60%) | 10 → 4.13 (-60%) | 9.13 → 8.49 (-7%) | 0.38 → 0.39 (+5%) |

## Root causes and fixes

### GPU: the Metal renderer did 2-3 times the necessary work per frame

Measurement on the synthetic repository, 1400x900 points, 2800x1706 pixels:
- Metal System Trace showed 11 render passes in every frame.
- `GPUI_SCENE_STATS` showed what each frame draws.

Three causes:

1. **Path batches split the frame.**
   - What the renderer did: for each batch of path primitives it ended the main
     pass, cleared and MSAA-resolved a full-window 4x intermediate texture (about
     19 MB written), then reloaded the drawable in a new pass.
   - Which paths: the selected repository tab's silhouette and outline, the
     content card's corner caps and history graph elbows. Five batches were on
     screen in every frame.
   - Fix (gpui-ce, Metal): rasterize every path batch of a frame before the main
     pass, in at most three layers. Batches whose sampled regions are disjoint
     share a layer. Each layer pass is cropped to its geometry's right and bottom
     extent. The frame becomes 3 passes instead of 11. A path pass costs 40-80 µs
     instead of about 470 µs.
2. **Content-mask clipping ran per fragment.**
   - What the shader did: quads were rasterized over their whole bounds, and the
     fragment shader discarded what fell outside the content mask.
   - Effect: quads rasterized 10.6 times the window area per frame, while only
     3.6 times was visible.
   - Fix (gpui-ce shared shader): rasterize the intersection of the quad and its
     content mask. Corner, border and paint geometry still use the quad's bounds.
3. **Every quad blended.**
   - Effect: the stacked opaque backgrounds (window surface, workspace, panes)
     were all shaded. Apple GPUs skip hidden fragments only for opaque,
     non-blended draws.
   - Fix (gpui-ce, Metal): draw rectangles without blending when they are opaque.
     That means a solid colour with alpha 1, no corner radii, no border, no fade,
     and entirely inside their content mask.

Rendering is unchanged:
- Frames rendered offscreen with `GPUI_DUMP_FRAME` are byte-identical before and
  after, on three repositories.
- New renderer tests compare legacy and new output pixel for pixel. These cover
  overlapping, disjoint and over-limit path batches, and clipped, faded, rounded,
  bordered and translucent quads.
- The tests fail when either fix is deliberately broken.

GPU time per frame, medians of 3 runs:

| Interaction | GitComet GPU ms/s before | after | GPU ms per frame before | after |
| --- | ---: | ---: | ---: | ---: |
| History hover sweep | 460 | 196 | 8.7 | 3.7 |
| History hover, large repository | 421 | 196 | 8.0 | 3.7 |
| History scroll | 483 | 222 | 8.1 | 3.7 |
| Keyboard commit selection | 150 | 60 | 9.0 | 3.6 |
| Open diffs | 39 | 20 | 6.9 | 3.7 |
| Switch repository tabs | 37 | 17 | 7.7 | 3.6 |

Whole-GPU power comes from IOReport energy counters and covers every process,
WindowServer included:

| Interaction | before | after |
| --- | ---: | ---: |
| Hover sweep | 767 mW | 293 mW |
| Scroll | 880 mW | 313 mW |

The mean active GPU performance state dropped from about 2.1 to 1.0, its lowest
clock. WindowServer's GPU time per second rises a little after the change
because its unchanged compositing work now runs at that lower clock, which costs
far less energy.

### CPU

- **Hover.** A hovered-row change re-renders the root view, the main pane and
  every visible history row. That is about 3.6 ms of main-thread work per frame,
  at up to 60 frames/s during a sweep. The work splits roughly into:
  - a quarter layout;
  - a quarter paint;
  - the rest prepaint, taffy, and replaying cached panes into the scene (bounds
    tree and tab stops).
- **Tab switch.** About 120 ms of process CPU per switch, mostly filesystem walks:
  - the file watcher is torn down and rebuilt for the new repository, and its
    setup walks the whole worktree;
  - Git status scans the worktree again;
  - branches, worktrees and other caches reload as well.
- **Diff open.** About 5 frames and 4 store publications per open. The diff view
  also re-read every visible line from disk (open, seek, read) on every frame,
  because rows built for rendering bypassed the page cache.
- **Every publication (macOS).** The session file was read and parsed on the UI
  thread to refresh the Recent menu.
- **Continuous redraws.**
  - Success and warning toasts animated a constant "hold" segment for 6-10 s, so
    the whole window redrew at display rate.
  - Spinners redrew at display rate while visible.
  - The refs hover card used `window.refresh()`, which invalidates every cached
    pane.

### Tab switching: warm file monitors (macOS)

The store used to stop the file monitor of the repository being left and
start one for the repository being entered. On macOS, starting a monitor walks
the whole worktree before creating its FSEvents stream. Because the left
repository was no longer watched, every switch also rescanned status and
reloaded history, branches and worktrees.

Now the two most recently active inactive repositories keep their monitors on
macOS (`WARM_INACTIVE_REPO_MONITORS`). An FSEvents stream is one recursive
registration per root; Linux keeps watching only the active repository because
inotify registers every directory.

How a switch decides whether to reload:
- Each monitor counts the changes it flushes, delivered or not, and publishes
  whether its coverage is degraded.
- When a tab is left, the store records the count.
- On returning, an unchanged count with full coverage means nothing changed
  meanwhile. If the retained state is also fully loaded and idle, the switch
  shows it instead of reloading status, history, refs and worktrees.
- Any change, a degraded watch, an evicted monitor or a missing load falls back
  to the previous refresh.

Results:

| Repository tab switching, 30 switches between three repositories | before | after |
| --- | ---: | ---: |
| Process CPU | 190 ms/s | 56 ms/s |
| Process CPU energy | 113 mW | 23 mW |
| State applications | 340 | 185 |
| Input to first draw | 15 ms (p50) | 16 ms (p50) |

The `repo-tabs-edit` scenario checks staleness. It writes a file in the inactive
tab and switches back; the file is listed 11 ms after the switch. After an
unchanged round trip it is still listed.

Fixed in GitComet so far:
- recent diff lines share one lazily loaded text, so a line is read once rather
  than every frame;
- the toast fade-out is timer-driven, so nothing animates during the hold;
- spinners are capped at 30 fps;
- the session file is reparsed only when its size or modification time changes;
- the refs hover card notifies its own overlay instead of refreshing the window.

## Tools added

- `scripts/profiling/perf_macos.py`: per-process GPU time (IOKit AGX accounting),
  CPU, wakeups and energy (`proc_pid_rusage`), and GPU energy and P-states
  (IOReport). Reading these needs no root.
- `scripts/profiling/macos-usage.py`: runs the interaction set with interleaved
  repeats. `macos-usage-report.py` prints the per-phase tables.
- `scripts/profiling/xctrace_table.py`: reads Instruments trace tables.
  `xctrace_profile.py` reports self and inclusive CPU time per function for a
  scenario phase.
- New `live-ui.py` scenarios: `diff-open`, `repo-tabs`, `repo-tabs-edit` and
  `still` (a settled window for pixel comparison). The scenario driver gained an
  `expect_status` step and a `keep` option for `write_files`. The `diff_loaded`
  witness now requires a new diff target.
- gpui-ce diagnostics:
  - `GPUI_SCENE_STATS=1|2|3` prints scene composition per frame.
  - `GPUI_DUMP_FRAME=prefix` writes the last frame as PNG.
  - `GPUI_METAL_EXPERIMENTS` provides opt-outs and attribution switches.

## Next steps

These were measured but not changed yet.

1. **Hover CPU.** Hovering a history row re-renders the whole history view. In a
   hover frame, the view's render, layout, prepaint and row painting are about
   half of the main thread's time. The root and main-pane renders are under 2%.
   - Proposal: move the rows into a cached child view. Draw the hover highlight
     and the lane badge from a small overlay that tracks the hovered row.
   - The row divs then drop their `.hover()` styles.
   - Expected gain: roughly half of the hover CPU.
2. **Diff open.** Opening a diff costs about 38 ms of main-thread time and about
   5 frames. The selection, text attributes, patch and file text arrive as
   separate store publications, and each one re-renders the main pane. Delivering
   them as one publication, and not bumping revisions when attributes are
   unchanged, would remove 2-3 frames per open.
3. **Rounded content card.** The white card behind the main and details panes
   has rounded left corners, so it blends. The GPU therefore shades the two
   opaque layers beneath it, about 69% of the window.
   - Option 1: drop the background layers that are hidden anyway (the window
     surface and workspace backgrounds under the card).
   - Option 2: split rounded opaque quads in the renderer into an opaque
     interior plus blended corners.
4. **GPUI per-frame bookkeeping.** In every frame:
   - inserting into the bounds tree takes about 8% of the main thread;
   - replaying cached views' paint takes about 8%;
   - inserting tab stops takes about 5%.
5. **Idle.** The idle cost is already small: about 4-6 ms/s of CPU and about
   40 idle wakeups per second. The CVDisplayLink still wakes the main thread
   on every vsync while the window is visible; pausing it after a few clean
   frames would remove most of the rest.

## Shipping the renderer changes

The renderer changes are in a gpui-ce checkout on branch `macOS_gpu_usage`.
GitComet pins gpui-ce by revision in `Cargo.toml`. To ship them:
1. Push that branch to the fork.
2. Bump the `rev` of `gpui`, `gpui_platform`, `gpui_wgpu` and `gpui_parley`.

Local builds used a `[patch]` config passed with `cargo --config`. The tracked
`Cargo.toml` did not change; the patch rewrites `Cargo.lock` while it is applied.

