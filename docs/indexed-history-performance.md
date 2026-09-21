# Indexed history performance

The index stores topology and sparse probable-stash positions. Commit dates come
from range metadata. Large indexes use cached 64-bit prefixes for construction
and a 16-bit fanout table for full-ID lookup; the builder reports temporary sort
and parent storage separately from retained bytes. Range reads decode each object
once and retain the walk's parent order, including shallow and filtered edges.

The graph frontier maintains target lookup, ordered free columns and palette
occupancy. Its transition output describes affected lanes without materializing
paint arrays. Full-history attribution and span construction, and skipped
checkpoint prefixes, use those transitions. Checkpoints store explicit occupied
columns with 32-bit targets and compact attribution records. The stride remains
1,024 rows. The original frontier is retained only in tests as an oracle.

Graph windows, absolute selected-lane spans, metadata blocks and ref decorations
have independent cache dependencies. Geometry reuse checks the projection,
effective branch-head positions and resolved HEAD. Ref renames can reuse geometry
while recomputing attribution. Progress and unrelated block completions retain
the text/graph window and comparison cards. Tag visibility is part of the
decoration key, including when the tag revision is zero.

Coincident paths are coalesced per incoming, outgoing, elbow and synthetic-band
pass. The winner is the final stroke in the original pass order, including
selected-lane precedence and node-color overrides. Nodes retain their own layer.

Selections and squash plans share their ID arrays. Shift selection transfers its
materialized range into the selection. Large comparison ordering runs on the
background executor; the store worker prepares squash eligibility once per
selection/topology revision. Only visible comparison cards resolve metadata.
Small comparisons remain ready in their first frame. Refreshes compute nearest
survivors only after the current anchor disappears, keep the old view interactive,
and recheck the current anchor before publication.

## Measurements

[Recorded paired backend results](performance/indexed-history-2026-09-11.json)
contain five before/after samples, pinned HEADs, ref digests, commit-graph hashes,
CPU and profile. These are optimized **test-profile** measurements on a shared
developer workstation, not dedicated-runner shipping limits.

| Repository | Commits | Median index seconds, before → after | Retained MiB, before → after | Median 256-row read p50 ms, before → after |
| --- | ---: | ---: | ---: | ---: |
| chromium | 1,922,916 | 7.350 → 5.347 | 80.8 → 63.1 | 3.44 → 1.88 |
| git | 83,182 | 4.233 → 4.053 | 4.8 → 4.0 | 2.74 → 1.61 |
| bun | 72,926 | 4.679 → 4.557 | 4.7 → 3.8 | 3.46 → 2.26 |

Chromium's pooled range-read p95 increased from 3.82 to 4.62 ms. Its first-touch
reads became more expensive, consistent with moving object reads out of index
construction. Warm reads improved. The other two snapshots have no commit-graph,
and their index-build time is largely unchanged.

| Repository | First-touch read p95 ms, before → after | Warm read p95 ms, before → after |
| --- | ---: | ---: |
| chromium | 3.996 → 5.342 | 3.717 → 2.029 |
| git | 3.261 → 1.877 | 3.233 → 1.864 |
| bun | 3.970 → 2.684 | 3.972 → 2.640 |

These are medians of five runs, each containing ten first-touch and thirty warm
256-row reads. Do not infer draw time from path counts or graph replay time.

## Frame and release measurements

[Frame samples](performance/indexed-history-frames-2026-09-11.json) record five
paired runs at 5,261 frontier lanes and 80 graph pixels, plus one-pair width/scale
controls. The viewport displays 38 rows with the renderer's bounded overscan.

| Optimized test profile, default graph width | Before | After |
| --- | ---: | ---: |
| Median run draw p95 | 290.07 ms | 14.63 ms |
| Median run combined wheel/publication/draw p95 | 571.94 ms | 29.66 ms |
| Allocation operations per draw | 2,903,857.40 | 10,352.00 |
| Allocated bytes per draw | 2,861,201,251.70 bytes | 6,068,177.30 bytes |

Five separate release runs had a median draw p95 of **4.38 ms** at the
default graph width. This is below 16.7 ms on this workstation; dedicated-runner
calibration remains outstanding. These CPU draw timings exclude GPU/compositor
presentation. The default-width painter emitted 400 paths per frame.

The narrow two-million-row control had a median graph build of 291 → 321 ms and
first-touch 256-row window p95 of 0.170 → 0.195 ms. This modest narrow-graph
regression accompanies the wide-graph gains.

At 150% scale, minimum/default/wide graph cells emitted 80/400/1,200 paths per
frame, matching the corresponding 100% cases. Corrected scale controls use the
test profile and one pair per case. Their draw p95 values were 16.18/44.61/15.77 ms
amid substantial background-workstation noise; they do not establish timing
budgets. Recorded release cases use 100% scale. Earlier probes that configured
the view scale without applying it to the test window are retained only as
discarded samples in the JSON.

[Release Criterion results](performance/indexed-history-release-2026-09-11.json)
include narrow and wide construction, first touch, warm reuse, distant jumps,
changing selections, SHA-1/SHA-256 IDs, allocations and checkpoint payload bytes.
The eight 100k/2M phase probes are included in the frame results. At two million
rows and 5,261 lanes, combined index/graph construction took 1.41 seconds in the
test profile, with 191,526,448 bytes of checkpoint payload and a first-touch
40-row window p95 of 1.41 ms. Checkpoint payload estimates exclude Arc headers.

## Reproduction and budgets

- `cargo test -p gitcomet-core --lib history_index` checks SHA-1/SHA-256,
  random IDs, prefix collisions and the fanout threshold.
- `cargo test -p gitcomet-git-gix --test log_integration indexed` exercises all
  history modes, author filtering, commit-graph present/absent, shallow histories,
  timestamp parity, stash shapes, cancellation and object-read counts.
- `cargo test -p gitcomet-state --lib indexed_history` checks range concurrency,
  the 32-block bound, malformed responses, stale replies and shared selections.
- `cargo test -p gitcomet-ui-gpui --lib indexed_history` checks bounded work,
  decorations, graph reuse, comparison invalidation and refresh handoff.
- The `transitions_and_restored_checkpoints_match_original_frontier` and
  `checkpoint_windows_match_continuous_graph_and_attribution` UI tests compare
  geometry, attribution, spans and selected highlighting with the original.

The ignored `indexed_history_wide_graph_phase_benchmark` measures production
100k/2M graphs at 1, 64, 512 and 5,261 columns, and reports retained topology,
checkpoint bytes and first-touch/warm p50/p95/p99 window latency. The ignored
`indexed_history_real_frame_benchmark` feeds real GPUI wheel events, waits for
window publication and draws 38 rows. Set `GITCOMET_BENCH_GRAPH_WIDTH`,
`GITCOMET_BENCH_GRAPH_PIXELS` and `GITCOMET_BENCH_UI_SCALE` at setup to sweep
widths and UI scales. The normal wheel/thumb refresh test covers thumb input.

`cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- indexed_history`
uses the shipping release profile and existing allocation sidecars. Set
`GITCOMET_BENCH_COMMITS=2000000` for the large suite. `build_graph`, `first_touch`,
`warm` and `distant_jump` are distinct Criterion cases. Backend paired runs use
`scripts/benchmark-indexed-history.py` with two compiled integration-test binaries
and three `--repository` arguments. It rejects changes to refs or commit-graph
files during the five paired runs and alternates execution order.

`changing_selection` measures a fixed viewport with a changing off-screen anchor.
Warm sidecars include allocations, retained index bytes, estimated peak index
construction storage, checkpoint bytes and paint-row materializations. Peak index
storage is a conservative topology estimate, including external-parent capacity
growth, rather than a measurement of the process RSS.

`scripts/benchmark-indexed-history-frames.py --before BEFORE --after AFTER
--profile test --output OUTPUT` runs five alternating pairs of the ignored GPUI
frame probe. It records binary hashes, the 20k-commit fixture, the 38-row viewport,
100 frames per sample, draw and input/publication latency, and draw allocations.
Use `--case 5261:28:100`, `5261:80:100`, `5261:240:100` and `5261:80:150` for
minimum/default/wide graph cells and scaled UI; `--case` may be repeated. Compile
both binaries before starting the samples.
For archived-source builds, use separate `CARGO_TARGET_DIR` directories; the
pairing scripts reject identical binaries and record their hashes.
These measurements exercise GPUI
layout and path submission in its test platform; they exclude compositor and GPU
presentation latency.

On the dedicated runner, archive five accepted **release** Criterion roots and
run `scripts/calibrate-indexed-history.py --runs ROOT1 ROOT2 ROOT3 ROOT4 ROOT5
--runner NAME --output baseline.json`. Set `GITCOMET_INDEXED_HISTORY_BASELINE`
(or the workflow's `PERF_HISTORY_BASELINE`) to that file. The existing budget
report then limits each calibrated case to 125% of its five-run median. Invalid
or non-release calibration is rejected. No dedicated-runner calibration is
invented from these workstation test-profile results. The warm scrolling draw
p95 target remains 16.7 ms, to be validated on that runner.

The workflow runs deterministic regressions in a separate job that fails on a
regression. Shared runner timings remain alerts; dedicated runner timing reports
use strict mode.
Work counters require tests/the benchmark feature and an opt-in capture. They
compile away in shipping builds and never read environment variables per row.

## Follow-up: bounded painting, exact checkpoints, cached parent resolution

Changes on 2026-09-12, each with a deterministic test:

- The painter coalesces lanes by displayed column: columns before the edge each
  win outright, the pinned tail is resolved by a backward scan that stops at its
  winner, and rows carry `from_node_cols` so lanes born at the node that land on
  the edge line need no scan. `displayed_column_coalescing_matches_generic_coalescing`
  checks every winner and its order against the hash-based coalescing over random
  frontiers, widths, joins, connectors and selections.
- Checkpoints are collected at exact capacity and restores are pre-sized
  (`checkpoints_carry_no_capacity_slack`, in both the walk and the graph).
- Index construction keeps its sorted prefix cache through parent resolution, so
  a bucket is a few contiguous cache lines and only the final full-ID check reads
  the ID table (`large_index_resolves_external_and_colliding_parents_through_the_prefix_cache`).
- `HistoryProjection::raw_position` is one partition point instead of a binary
  search over partition points, `IndexedGraph::step` maps each row once, and
  per-frame block bookkeeping steps by block
  (`sparse_projection_maps_thousands_of_hidden_rows_both_ways`,
  `block_stepping_matches_a_row_by_row_scan_across_hidden_rows`).
- Integration-branch containment bitsets and checkpoint labels are reused across
  rebuilds while their inputs hold still, so a divergence-only ref change no
  longer replays the graph
  (`integration_containment_and_labels_are_reused_when_their_inputs_hold_still`).
- The stored index shares the log snapshot's allocation, so snapshot equality is
  a pointer comparison (`built_index_shares_the_log_snapshot_allocation`).

Same workstation and harnesses as above; "before" is commit 9b217890.

| Case | Before | After |
| --- | ---: | ---: |
| Release draw p50, 64 / 512 / 5,261 lanes, 80 px | 1.75 / 1.94 / 4.30 ms | 1.69 / 1.68 / 1.73 ms |
| Release draw p95, same | 1.90 / 2.35 / 4.65 ms | 1.72 / 1.72 / 1.85 ms |
| Test-profile draw p50, same | 5.48 / 4.18 / 16.43 ms | 2.78 / 2.67 / 2.67 ms |
| Allocations per release draw | 9,051 | 8,896 |
| Checkpoint payload, 2M rows × 5,261 lanes | 191,526,448 B | 123,042,752 B |
| Index + graph construction, 2M rows, width 1 / 5,261 | 1.33 / 1.45 s | 0.68 / 0.75 s |
| First-touch 40-row window p95, 2M rows × 5,261 lanes | 1.21 ms | 1.12 ms |
| `finish()` at 1,922,916 rows, release scratch harness | 0.823 s | 0.175 s |
| Sequential row mapping, 2M rows, 200 / 2,000 hidden rows | 0.39 / 0.87 s | 0.05 / 0.07 s |
| Graph rebuild after a divergence-only ref change, 2M rows | full replay | 0.001 s, zero walks |

The ignored `history_index_finish_phase_timing` and
`integration_containment_walk_timing` tests reproduce the last two rows in-tree.

## Follow-up 2: quads for straight lanes, one layout node per row

Profiling the release frame benchmark after the follow-up above put about 90%
of samples inside GPUI (taffy layout, bounds tree, style refinement, element-id
hashing) and 10% in the allocator; the graph painter was under 3%. Sweeping
`GITCOMET_BENCH_ROWS` (4 versus 38 visible rows on the same binary) showed the
history rows themselves cost 0.86 ms of a 2.69 ms test-profile frame, with
about 114 allocations and 89 KB per row, most of it lyon tessellation of
straight lane segments.

- Straight vertical lane runs are painted as quads: the same rectangle a
  butt-capped stroke yields, without a tessellated path per segment. Only
  elbows and join stubs remain paths. Quads paint under paths within a layer, so
  on the collapsed edge line an elbow now covers a straight run that used to
  be painted after it; nothing else changes.
  (`indexed_history_actual_paint_paths_are_bounded_by_displayed_columns` now
  asserts straight rows tessellate nothing.)
- Rows in the indexed viewport position themselves absolutely instead of each
  sitting in a wrapper layout node.
- The benchmark's `GPUI window rebuild request` probe measured the UI-thread
  cost of handing the shown window to a rebuild at about 12 µs, so sharing the
  window across threads was not pursued.

| Case, 80 px graph, before → after | Test profile | Release |
| --- | ---: | ---: |
| Draw p50 at 38 rows, 64 lanes | 2.69 → 2.49 ms | 1.69 → 1.54 ms |
| Draw p50 at 38 rows, 5,261 lanes | 2.67 → 2.42 ms | 1.73 → 1.56 ms |
| Draw p50 at 4 rows, 64 lanes | 1.83 → 1.84 ms | — → 1.17 ms |
| Allocations per draw, 38 rows | 10,199 → 7,169 | 8,896 → 5,987 |
| Allocated bytes per draw, 38 rows | 6.06 → 3.27 MB | 5.61 → 2.86 MB |
| Tessellated paths per frame | 400 → 0 | 400 → 0 |

### What the benchmark's default frame overstates

The test binary mounts the shell views uncached (`stable_cached_views_enabled`
is false under `cfg(test)`), and the measured draw forces `window.refresh()`.
Shipping builds wrap the title bar, action bar, tabs bar, status bar, sidebar,
main pane and details pane in `AnyView::cached`, and a wheel event only
notifies the history view, so a scroll frame reuses every other pane's
prepaint and paint and lays out only the dirty subtree. `GITCOMET_BENCH_CACHED_VIEWS=1`
opts the benchmark into the shipping configuration after setup and
`GITCOMET_BENCH_NO_REFRESH=1` times that notify-only frame:

| Shipping-shaped frame, 80 px graph | Test profile | Release |
| --- | ---: | ---: |
| Draw p50 at 38 rows, 64 lanes | 1.29 ms | 0.84 ms |
| Draw p50 at 38 rows, 5,261 lanes | 1.30 ms | 0.85 ms |
| Draw p50 at 4 rows, 64 lanes | 0.79 ms | 0.50 ms |
| Allocations per draw, 38 rows | 3,015 | 2,509 |
| Refresh frame (hover change) at 38 rows, cached views | — | 1.59 ms |

So a release scroll frame costs about 0.84 ms, of which the rows are about
0.34 ms, roughly 10 µs each; the rest is the window's own layout and the cached
placeholders. A hover change or tooltip forces a refresh frame at about twice
that. Lane count no longer matters.

## Follow-up 3: memory on large repositories

Measured with the ignored backend benchmark, which now reports resident and
peak-resident memory from `/proc/self/status`, on the chromium checkout
(1,922,916 commits, 66 GB of packs, a commit-graph chain):

| Phase | Resident MiB | Peak MiB |
| --- | ---: | ---: |
| Repository opened | 8.7 | 8.7 |
| Index built (59.1 MiB of index) | 222 | 707 |
| Forty 256-row blocks read | 406 | 707 |
| Every block read once, before this follow-up | 1,906 | 1,906 |
| Every block read once, after | 891 | 896 |

Three things stand out. The build's peak is gix's date-order topology walk,
which computes in-degrees over every commit before yielding, plus the mapped
commit-graph; the builder's own tables are the 59 MiB that remain. The heap
structures are modest: at two million rows and 5,261 lanes the checkpoints are
now 51 MB and the lane spans 0.45 MB. What grows as a large history is scrolled
is the pack mapping: every page of a pack that a commit read touches stays
resident until the mapping is dropped, and `unsafe_code = "forbid"` rules out
advising the kernel directly.

- Indexed range reads now go through a dedicated object store that is re-opened
  every 64 blocks (`range_reader_repo`). Dropping the previous store unmaps its
  packs; a fresh open costs a config parse. Warm block reads stayed at 2.1 ms
  p50 and `indexed_range_reads_reopen_their_object_store_every_sixty_four_blocks`
  pins the cadence and result equality. The remaining growth is dominated by
  pack index pages, which every lookup re-touches: chromium's nine `.idx` files
  total 974 MB and its commit-graph 123 MB, and there is no multi-pack index.
- Checkpoints store one target and one colour per column, five bytes a column,
  instead of a twelve-byte record per live lane carrying its column and a lane
  identity; only the main lane's column is kept, since the identity is compared
  with nothing else. 2M rows × 5,261 lanes: 123,042,752 → 51,359,273 bytes,
  first-touch 40-row p95 1.14 → 1.13 ms. `dense_checkpoints_keep_holes_and_the_main_lane`
  checks hole reuse and head handling across restores against the oracle.
- The finished index shrinks its tables to size: 63.1 → 59.1 MiB at chromium
  scale (`finished_index_retains_no_growth_slack`).

Further index compaction, not done: splitting parents into a first-parent
column plus an overflow table for merges would save about 6 MiB of the 59 on
chromium at unchanged lookup cost with a rank bitset; referencing IDs by
commit-graph position instead of storing the 20-byte table would save about
30 MiB where a complete commit-graph exists, at the cost of a fallback path and
ID reads through the mapped graph.

## Verification

The full core, state, backend log-integration and GPUI library suites passed:
557, 884, 51 and 3,781 tests respectively. The budget reporter's 111 tests also
passed. Ignored performance probes were run separately. Workspace targets with
the benchmark feature compiled, shipping libraries passed Clippy with warnings
denied, and formatting and whitespace checks passed.
The budget reporter accepted the recorded release draw sidecar (4.386 ms p95
against 16.7 ms) and the warm graph sidecar (zero paint-row materializations).

Deterministic coverage includes zero timestamp-only reads in first-parent mode,
one lookup per range commit, zero paint rows during full construction and skipped
prefix replay, independent window and comparison invalidation, unchanged
decoration reuse, shared 100k selections, bounded emitted paths, and the existing
32-block/two-request/stale-result contracts. Generated and targeted graph cases
compare transitions, checkpoint restoration, attribution, spans and selection
highlighting with the retained original algorithm.
