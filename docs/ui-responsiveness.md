# UI Responsiveness Measurements

These measurements cover input, rendering and background scheduling on Linux x86-64, using GitComet `bc8276ba6` as the application baseline.

## Interaction baselines

The live runs used a Ryzen 9 5950X, an NVIDIA GTX 1080 and a private 60 Hz Mutter session. The Git fixture had 20,000 generated history commits plus merges. The navigation fixture had one changed file with 100,000 lines and 100 changed hunks.

| Interaction and clock | Samples | Baseline p50 | Baseline p95 | Changed p50 | Changed p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| F7 and Shift-F7, key handler | 2 pairs of 240 key dispatches | 0.0168 ms | 2.958 ms | 0.0132 ms | 2.060 ms |
| F7 and Shift-F7, CPU draw | 241 draws in each of four runs | 2.561 ms | 7.560 ms | 2.381 ms | 4.165 ms |
| Native window activation, request to verified active window | 4 pairs of 20 switches between two windows | 11.53 ms | 12.82 ms | 11.74 ms | 12.95 ms |
| Insert and render, unwrapped 4 MiB editor | 6 pairs of 50 edits | 0.416 ms | 0.462 ms | 0.429 ms | 0.468 ms |
| Insert and render, soft-wrapped 4 MiB editor | 6 pairs of 50 edits | 3.126 ms | 3.634 ms | 2.281 ms | 2.451 ms |

The navigation pairs compared the baseline and the integrated release at `6b838d02d` on the same eight physical cores, reversing run order in the second pair. The table gives medians of per-run percentiles. Draw p95 was 4.44 and 10.68 ms for the two baseline runs and 4.41 and 3.92 ms for the changed runs. The activation clock includes polling for native focus. The harness reports nearest-rank percentiles across the 20 switches in `summary.json` under `window_activation_ms`.

The activation row uses the standard one-second wait after opening both windows. The competing local verification builds were paused during all four pairs, and every run verified all 20 switches. With compilation running, four more pairs gave median per-run p95 of 38.61 ms before and 80.63 ms after. Another reversed-order pair gave 104.32 and 102.24 ms. In that pair, both builds spent about 15 ms on the main thread and another 15 ms waiting to run per draw, averaged over complete one-second process samples. With a three-second wait, the two paired p95 results were 14.51 → 16.15 ms and 36.71 → 14.01 ms.

The editor numbers are medians of per-run percentiles from paired GPUI test binaries with the same optimized test profile and four-CPU affinity. Each baseline wrapped run materialized the whole document 50 times. The revised editor performed zero whole-document materializations.

The merge editor resolves lines from the text snapshot, including when it builds an outline. The file and merge gutters now follow caret reveals on the next frame. In the 400-line file regression, moving to the end used to leave gutter rows 9–41 on screen. The rendered gutter now shows rows 368–400.

## Repository and renderer work

| Operation | Baseline | Changed | Measurement |
| --- | ---: | ---: | --- |
| Visible 256-row Chromium history block queued during status | 1,022.52 ms | 0.012 ms | Median queue wait, two logical CPUs |
| Same block from dispatch to publication | 1,105.38 ms | 5.41 ms | Nine warm reads under real status |
| Block from dispatch to publication during a full refresh | 55.73 ms | 4.22 ms | All 32 logical CPUs |
| Repeated committed blame of `builtin/diff.c` in Git's own repo | 1,386.41 ms | 0.140 ms | 651 lines, warm immutable revision |
| Newest requested syntax viewport becomes available | 40.132 ms | 1.063 ms | Six paired runs, 24 disjoint viewports of Chromium C++ |
| Transfer a 100,000-entry syntax cache for destruction | 77.95 µs | 0.030 µs | Caller time in an optimized build |
| Build both split line-to-row maps on 100,000 unchanged lines | 1.205 ms | 0.767 ms | Eight pairs of 200 builds |
| Four retained split and inline row maps for that file | 6.10 MiB | 1.53 MiB | Array storage on 64-bit Linux |
| Conflict-marker check before staging, UI handler | 6.006 ms | 0.057 ms | Five runs with a 2.9 MB file |
| Markdown task toggle, UI handler | 13.729 ms | 0.013 ms | Five runs with a 3.6 MB file |

The older-fetch tests replayed an incoming history prefix against the same revisions used for cold rebuilds. Across three alternating pairs per fixture, the 102,000-commit case fell from 411.42 ms to 11.37 ms, the 2.04-million-commit case from 4,608.01 ms to 69.19 ms, and Chromium from 676.33 ms to 202.93 ms. All nine avoided the full rebuild. The graphless fixtures needed 105 object-header reads each. Chromium used its native commit graph and needed two. Deep insertions fall back after 20,000 previously indexed candidates, whether read from objects or from the commit graph. Tests compare tied-timestamp ordering, merges, stashes and external parent references against a fresh full index.

Reusing the search finder reduced case-sensitive multi-byte literal search time by 8.5–40.8% across the tested queries on a 20,234-line Chromium file. Match counts agreed. The scrollbar continues to notify its painting view once per pointer event. Its earlier paired drag runs reduced draw p95 from 3.86 ms to 1.34 ms.

A separate local gpui-ce patch limits click-state repaint to the owning view. The click regression reduced rerenders of an unrelated cached sibling from four to zero and covers pointer press and release. The framework patch and matching dependency pin are available together for integration.

The reserved worker serves visible requests ahead of background reads. A second two-CPU experiment ran a two-million-commit index refresh alongside Chromium. Changing only the background index thread’s Linux nice value from 0 to +5 moved foreground range-publication p95 from 10.44 ms to 12.98 ms across 36 reads per arm. The thread keeps its existing priority.

The local upstream gix patch targets gitoxide `9abb53183a`. Opening a fresh object store and reading the same Chromium object took 45.568 ms before and 0.259 ms after on a 973 MiB pack index. Each of nine pairs used 11 fresh opens per process. Strict whole-pack enumeration still validates the complete index. Opening and iterating all 29,340,201 offsets took 290.15 ms before and 288.05 ms after across eight pairs. These measurements use the patched dependency directly. The GitComet application timings use the released gix 0.90.0 and gix-pack 0.77.0.

Visible syntax requests supersede chunks that have scrolled out of view. In the six paired runs, the retained chunk count after scrolling fell from 72 to three. A second window with visible work still gets a turn during continuous scrolling in the first window.

The first text diff and visible image decodes use high-priority background work. Rebuilding a diff with rows already on screen uses normal priority. Saving mergetool window dimensions and UI preferences uses low priority. In the image regression, switching files before a queued SVG decode ran skipped the old decode and displayed the newly selected image.

Warming Chromium syntax after the first window submission sampled 128 regular tracked paths from the index in 0.166 ms on a clean checkout. Those paths identified 15 languages. Compiling their queries took a median 329.6 ms wall time, 320 ms of process CPU and 14.86 MiB of additional resident memory across five fresh processes. Rechecking all 15 then took 1.26 µs. The low-priority worker yields between languages.

Staging actions read conflicted files on a worker. If status is still loading, the worker asks the repository which paths are conflicted. Immediately before staging a selection that passed the initial check, the worker checks the relevant files again. If a file gained conflict markers, the worker leaves the index as it was and the user can retry to review the markers. A new action from the status menu can proceed even if a diff shortcut for the same path is still pending. Markdown task edits verify the bound worktree document and original bytes before writing. The table reports time on the input handler. The save acknowledgement still waits for the write.

Rust extensions that build `Msg::StagePaths` with an explicitly typed `RepoPathList` now pass `paths.into()` in that field. Existing calls that already convert a vector with `.into()` use the same syntax.

“Copy environment details” now includes draw and input percentiles across open windows and the latest 16 foreground stalls lasting at least 200 ms. Static labels and durations stay on the machine. A low-priority task samples the journal every two seconds. Across two 25-second instrumented idle pairs there were zero draws in both builds. Baseline main-thread CPU ranged from 0.12% to 0.26% and the changed build from 0.15% to 0.18%. Process wakeups were about 119/s and 121–122/s respectively with the probe running.

## Window state updates

Each window keeps the latest pending store snapshot. It caches the active repository’s position, checks the ID on use and locates it again if the repository order changes. A repository switch or ready details for the newly selected commit can use an available frame immediately. Other updates after an idle period get a one-millisecond window to arrive together. Subsequent updates wait for the next GPUI frame callback. GPUI can run a callback without drawing when the window is clean, so consecutive callbacks can fall between two recorded draws.

The history run sends 240 Down keys at 120 ms intervals and checks that each selected commit matches its loaded details. The tab run switches among 32 open repositories 30 times, checking the active path and its ready state each time. The history view requests more rows after layout or scrolling. It also checks whether pagination is already in flight.

| Workload and run order | Store updates delivered, before → after | Total observer time | Input to submission p50 | Input to submission p95 |
| --- | ---: | ---: | ---: | ---: |
| History, baseline first | 478 → 242 | 18.26 → 9.30 ms | 12.35 → 11.78 ms | 20.75 → 20.90 ms |
| History, changed build first | 481 → 242 | 14.50 → 9.36 ms | 11.48 → 11.69 ms | 18.49 → 18.68 ms |
| 32 tabs, baseline first | 173 → 100 | 16.89 → 12.41 ms | 10.68 → 10.31 ms | 32.14 → 17.24 ms |
| 32 tabs, changed build first | 133 → 107 | 12.74 → 11.38 ms | 10.28 → 10.50 ms | 13.16 → 11.35 ms |

All eight runs observed every requested selection or tab switch. Coalescing roughly halved history observer deliveries while history submission p95 differed by under 0.2 ms in both pairs. Each changed history run showed a separate loading state once in 240 selections. In a minimized window, four real filesystem edits were observed without drawing. The pending-update fallback raised edit-to-status p95 from 255.73 to 500.59 ms and reduced deliveries from ten to five. Both minimized windows had zero draws and zero observer deliveries during the three-second idle phase.

## Diff opening and startup

Three release-build runs alternated between two files. Each of 24 inputs selected the expected file, painted a row matching its contents and submitted the containing frame. Git's Trace2 events identified the patch child independently of the other repository commands.

| Working-tree file | Selections | Input to first text row | Input to submission | Parent spawn | Spawn share of time to row | Git child lifetime |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 45 bytes | 12 | 12.59 ms | 14.16 ms | 0.300 ms | 2.50% | 1.425 ms |
| 2,599,300 bytes, 100,000 lines | 12 | 50.22 ms | 52.32 ms | 0.293 ms | 0.57% | 31.77 ms |

The percentages are medians of per-selection ratios. The larger file spent 60.9% of its time to first row inside the Git child. The application continues using its existing patch backend.

Five alternating Linux startup pairs measured early font registration at 1.769 ms for all 16 faces and 0.180 ms for the selected regular face in the integrated release. Deferred registration took 0.910 ms after submission. The median from spawn to the first platform submission was 189.11 ms before and 197.61 ms after. An earlier five-pair run gave 215.13 and 219.58 ms. The startup probe records the observed submission. Extension startup workflows can open their own first window and get all faces before the workflow runs.

When the remaining fonts arrive, the application clears retained glyph shapes and widths. A focused diff then uses its selected editor font on the next redraw and reuses the new shape on subsequent draws. The real-text-backend editor test also checks the wrap count and height when a line first painted with the fallback font occupies more rows in Lilex.

The file-history sidebar is created when it first opens. Across 20 fresh processes opening a repository, the reflog shell retained 388 bytes and took a median 5.82 µs to create. Its filter and rows already wait until it opens. The command palette and reveal-commit shell took about 5 µs and 4 µs. The popover host took 116 µs and must subscribe at startup to receive automatic prompts.

The sidebar, details pane and diff renderer each used to reserve 950,320 bytes for path labels before displaying a path. An empty window’s three caches now save 2.72 MiB of heap reservations. One cache holding 32 paths retains 41,168 bytes, down from 953,056. Across 32 alternating release pairs the first 256 paths took 9.69 µs before and 9.26 µs after. The first 257-path batch pays a one-time reallocation and took 11.50 µs before and 13.70 µs after in a separate 16-pair run. A prefilled 10,000-path pass took 563.47→568.49 µs. Both versions retain at most 8,192 entries and use the same storage when full.

## Scrolling with parked repositories

A Linux 60 Hz run replayed 1,200 wheel inputs in the active indexed fixture at each tab count. All 1,200 inputs changed the history scroll position in each run. The secondary paths pointed at separate one-commit repositories. The following are gaps between consecutive GPUI platform submissions during scrolling, with no external accessibility client attached to the test workload.

| Open tabs | Submissions | Median gap | p95 gap | p99 gap |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1,149 | 16.65 ms | 17.71 ms | 20.78 ms |
| 32 | 1,149 | 16.65 ms | 17.80 ms | 19.39 ms |
| 264 | 1,145 | 16.63 ms | 18.42 ms | 29.39 ms |

The accessibility run used a private AT-SPI bus. An external reader verified GitComet’s process identity, called `GetChildren` on the application and its first window, and waited 100 ms between rounds. The counts below include only successful queries during the 1,200-input phase.

| Open tabs | Window queries | Submissions | Median gap | p95 gap | p99 gap |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 190 | 1,138 | 16.64 ms | 19.50 ms | 29.37 ms |
| 32 | 190 | 1,133 | 16.74 ms | 20.90 ms | 22.48 ms |
| 264 | 188 | 380 | 32.78 ms | 144.69 ms | 162.76 ms |

These runs use the application baseline. The 120 Hz macOS report involves native momentum, accessibility queries and the physical presentation clock. [Original report](https://github.com/Auto-Explore/GitComet/issues/741).

The same Linux workload also ran against the tab change’s exact parent and head, both built with Rust 1.99 and GPUI `c55d7365`. Each cell below is the p95 gap between platform submissions. Every run observed 1,200 changes to the active history scroll position. Each run with an external reader completed 190 or 191 window queries during scrolling.

| Open tabs | Parent, no reader | PR, no reader | Parent, AT-SPI reader | PR, AT-SPI reader |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 17.29 ms | 17.19 ms | 18.12 ms | 18.16 ms |
| 32 | 17.27 ms | 17.20 ms | 18.75 ms | 18.13 ms |
| 264 | 17.84 ms | 17.43 ms | 47.79 ms | 19.35 ms |
| 264, reversed run order | 17.97 ms | 17.93 ms | 36.32 ms | 22.74 ms |

At 264 tabs with the reader, submission counts increased from 517 to 1,143 in the first pair and from 627 to 1,071 in the reverse pair. The smaller tab counts had one pair each. [Repository-tab change](https://github.com/Auto-Explore/GitComet/pull/749).

For the repository-tab change, 32 published native recordings compare its parent `14b3cba1` with `7b0229c8` on one 120 Hz M5 Max with 264 tabs. Recalculating intervals between unique drawable presentations inside the prescribed motion windows gave the following per-run p95 in every pair. CPU is mean application root CPU time per actual presentation. [Frozen macOS data](https://github.com/wowlocal/GitComet/tree/31556a9c0c646a186b781c7783ba741f8a6bf2c5/benchmarks/2026-11-10-pr749/data).

| macOS condition | Pairs | Parent p95 | PR p95 | Parent CPU | PR CPU |
| --- | ---: | ---: | ---: | ---: | ---: |
| Accessibility inactive | 8 | 8.33 ms | 8.33 ms | 2.17 ms | 1.92 ms |
| Framework accessibility processing forced | 8 | 16.67 ms | 8.33 ms | 5.40 ms | 3.28 ms |

These recordings used generated wheel input. The separate recorder-off CPU control was inconclusive. Native-client presentation data for the 1-, 32- and 264-tab macOS workloads and a physical-trackpad comparison remain open. [PR measurements and controls](https://github.com/Auto-Explore/GitComet/pull/749#issuecomment-6761683613).

The separate momentum draft delays its logical sampling clock by 12.5 ms. Its published four-pair experiment used an older combined build, `fe34d063`. Retiming reduced consecutive held logical positions from 18 of 1,966 to zero of 1,949. Median actual presentations per second moved from 117.95 to 116.19, and median application CPU rose from 29.15% to 30.55% of one core. [Frozen momentum experiment](https://github.com/wowlocal/GitComet/tree/fdb3b3b26c4f7a88a5554af80362c6eec8238036/benchmarks/2026-11-09-native-history-momentum).

An offline replay exercised the draft’s exact `93b988f5` controller with the same 240 recorded packets, assuming continuous interior scrolling and an ideal 120 Hz clock. Across eight clock alignments, direct updates held the logical position for 65 of 1,005 adjacent samples in the prescribed motion windows. Retiming held zero. In every alignment the controller drained the full 3,468 pixels. Its five controller tests also passed. Physical presentation, energy and trackpad feel on this revision still require a native run. [Momentum change](https://github.com/Auto-Explore/GitComet/pull/748), [recorded input](https://github.com/wowlocal/GitComet/blob/fdb3b3b26c4f7a88a5554af80362c6eec8238036/benchmarks/2026-11-09-native-history-momentum/input-plan.json).

## Linux profile-guided build

Both binaries used identical GitComet source, Rust 1.100, the release profile and `x86-64-v3`. Training covered startup, history selection, history scrolling, burst selection and opening a diff. Two diff-search attempts failed to open the search field and were excluded from the profile.

Each workload had six pairs across two sessions with reversed starting orders. The table gives the median paired candidate-to-baseline ratio. A ratio below 1 means the profiled build was faster.

| Workload | Paired ratio | Bootstrap 95% interval |
| --- | ---: | ---: |
| Startup to first draw | 0.998 | 0.917–3.129 |
| History selection, median input to submission | 1.021 | 0.927–1.193 |
| Diff opening, median input to submission | 1.112 | 0.788–2.379 |

The executable size fell from 178,002,320 to 170,087,632 bytes. The timing results missed the 5% adoption threshold and overlap the baseline in all three cases. Concurrent compilation contributed to host variability. The experiment leaves the release build configuration in place.

## Reproducing the workloads

Create a disposable fixture with `python3 scripts/profiling/live-ui.py fixture /tmp/gitcomet-perf`. The fresh fixture has a single unstaged file, `search-target.txt`, with enough hunks for 40 consecutive navigation keys. Use an unused capture directory for each run. On macOS and Windows pass `--display desktop`. On macOS pass the executable inside `GitComet.app/Contents/MacOS/` to `--binary`.

```sh
python3 scripts/profiling/live-ui.py run --binary target/release/gitcomet \
  --repository /tmp/gitcomet-perf --scenario diff-navigation --output /tmp/gitcomet-keys
python3 scripts/profiling/live-ui.py run --binary target/release/gitcomet \
  --repository /tmp/gitcomet-perf --scenario window-activation --output /tmp/gitcomet-focus
cargo test -p gitcomet-ui-kit typing_fifty_edits_in_a_four_mib_document_keeps_snapshots_lazy -- --nocapture
GITCOMET_HISTORY_TEST_REPO=/path/to/chromium \
  cargo test --release -p gitcomet-state --test history_under_load -- --ignored --nocapture
```

The `history-scroll-tabs` scenario replays the same witnessed 1,200 wheel inputs at the primary repository after opening each `--secondary-repository`. Passing 0, 31 or 263 distinct secondary paths gives the 1-, 32- and 264-tab configurations. Capture physical presentation and native trackpad momentum separately on the affected macOS hardware. [Many-tab scrolling issue](https://github.com/Auto-Explore/GitComet/issues/741), [repository-tab presentation change](https://github.com/Auto-Explore/GitComet/pull/749), [native momentum change](https://github.com/Auto-Explore/GitComet/pull/748).

The profiling guide describes the capture format, frame clocks and comparison commands. The history report covers repository opening and long range reads. [Profiling GitComet](../scripts/profiling/README.md), [indexed history measurements](indexed-history-performance.md).
