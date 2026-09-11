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
