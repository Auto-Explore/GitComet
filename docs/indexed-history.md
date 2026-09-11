# Indexed history scrolling

The main history opens with the existing recent page while a background task
builds the selected query's complete topology. The header shows indexing
progress. Once graph checkpoints are ready, the scrollbar represents the entire
visible history. Wheel and trackpad input stays linear; thumb dragging can jump
directly to any position. Nonlinear scrolling and disk persistence are deferred.

## Loading presentation

The recent page remains usable while indexing runs. Its provisional scrollbar
thumb is capped at 48 logical pixels, with the existing 24-pixel minimum. Once
the full presentation is ready, normal proportional sizing resumes. The cap and
scroll range stay tied to the displayed page and cannot change during a drag.
Complete short histories use normal sizing; histories that fit have no thumb.

Missing rows reserve their normal height immediately. Only rows still missing
after 300 ms show static, theme-aware skeleton bars aligned with visible columns.
Ready content replaces them immediately, without animation or minimum dwell.
One view-owned timer handles visible-row deadlines and a one-second delay for
the secondary header status. Scrolling out of view, changing query, or loading
the row cancels its pending indicator. Initial skeletons are decorative and never
contribute to the list's scroll extent. Errors keep their clickable retry rows.

An indexed presentation is published together with its first viewport cache.
Already-loaded commit objects seed that cache by immutable ID, so switching from
the recent page or refreshing the index does not blank surviving visible rows.
The latest viewport anchor is rechecked before publication; the old presentation
stays interactive while the new graph window is prepared in the background.

## Data and rendering

- `gitcomet-core::history_index` stores binary object IDs, ordered parent ranks,
  timestamps, and a sparse projection hiding stash helper rows. Snapshot identity
  includes the history mode, normalized author filter, selected tips and shallow
  boundary, matching the existing history reader. External parent IDs remain
  available for comparisons even when the query omits their commits.
- `gitcomet-git-gix` builds the index on a dedicated worker. An exact count requires
  a traversal; it is not assumed to be instantaneous. First-parent timestamps and
  author filtering read object headers when the walker cannot supply them.
- Text and full commits are read directly by indexed object ID in 256-row blocks.
  The reducer allows two range requests in flight and retains at most 32 blocks
  (8,192 decoded commits) using an LRU. The viewport requests visible rows first,
  then two screens in the direction of travel and one behind. Changed requests
  cancel obsolete work; snapshot and request generations reject stale responses.
- The graph records traversal and branch-attribution checkpoints every 1,024
  visible commits. Window construction replays from the preceding checkpoint on
  GPUI's background executor. Lane lifetimes preserve selection highlighting when
  its commit is outside the rendered window.
- The physical view contains only a screenful of fixed-height rows. Its position
  is an integer row plus an `f64` pixel remainder, avoiding loss of wheel precision
  from converting millions of row heights to GPUI's `f32` Pixels. Unavailable
  entries keep their row geometry and display a loading or retry affordance.

## Refresh and navigation

The displayed index, graph and synthetic worktree rows stay together during
refresh. Publication preserves the latest top commit and within-row offset.
A background merge of the two indexes prepares nearest surviving anchors for
rewritten commits. Worktree rows use worktree paths as their identity. Index and
row-plan publication pauses while the thumb is held; user scrolling cancels a
pending programmatic reveal.

Reveal, keyboard navigation and range selection use the full projection. A
refreshed initial page cannot discard a distant selection. Comparison endpoints,
squash eligibility and cherry-pick selection resolve against indexed topology;
commit details and operation messages continue to use their existing loaders.
The legacy paged renderer remains available for backends without index support.

## Verification and diagnostics

Focused checks:

```sh
cargo test -p gitcomet-core -p gitcomet-state --lib
cargo test -p gitcomet-git-gix --test log_integration
cargo test -p gitcomet-ui-gpui --lib history -- --test-threads=4
```

Opt-in benchmarks (the repository benchmark only reads its input):

```sh
GITCOMET_HISTORY_BENCH_REPO=/path/to/large/repo cargo test -p gitcomet-git-gix --test log_integration indexed_history_large_repository_benchmark -- --ignored --nocapture
cargo test -p gitcomet-ui-gpui --lib indexed_history_two_million_graph_window_benchmark -- --ignored --nocapture
```

In a debug build, set `GITCOMET_HISTORY_SCROLL_TRACE=1` to log gesture generation,
logical row position, sub-row offset, viewport height and publication events.
The trace does not include commit contents or IDs.

Measured in the optimized test profile on the development machine:

| Workload | Result |
| --- | --- |
| Chromium all-branches index, 1,922,916 commits | 17.864 s; 80.8 MiB estimated retained topology |
| Chromium direct 256-commit reads, 40 samples | p50 3.61 ms; p95 23.98 ms |
| Synthetic 2,000,000-row graph, periodic merges, two lanes | 0.578 s to build 1,954 checkpoints |
| Synthetic 256-row graph windows, 100 distant positions | p50 0.176 ms; p95 0.284 ms |

These measure index construction, block decoding and graph replay separately;
they are not end-to-end frame timings. Cold object reads are slower than warm
reads. Index construction and graph work stay off the UI thread.
