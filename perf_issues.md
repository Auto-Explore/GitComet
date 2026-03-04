# Diff And Conflict Resolver Performance Design

## Implementation Progress

### Priority Status

- P0
  - ✅ Resolved output gutter virtualization (`uniform_list` + shared `conflict_resolved_preview_scroll`)
  - ✅ Render-time recomputation in conflict hot paths (two-way + three-way line/conflict maps precomputed in state; render/nav now use O(1) lookups)
  - ✅ Conflict rows on canvas fast path (two-way + three-way resolver rows and conflict compare split/inline rows now on keyed canvas behind `GITGPUI_CONFLICT_CANVAS_ROWS`, with fallback preserved)
- P1
  - ✅ Syntax mode/language caching in conflict renderers (cached per-file language in `ConflictResolverUiState` + per-batch syntax mode gating in conflict compare/resolver + three-way rows)
  - ✅ Cache invalidation scope reduction (split-resize cache churn removed; conflict split/inline search typing now uses stable base caches + volatile query overlay caches)
- P2
  - ✅ Precomputed state usage improved (two-way row/conflict maps + three-way column line maps in state)

### Phase Status

- ✅ Phase 1 (3/3 complete: gutter virtualization + two-way/three-way conflict map precompute + render-time map/range rebuild removal)
- ✅ Phase 2 (3/3 complete: two-way + three-way conflict resolver and compare keyed canvas paths + fallback flag)
- ✅ Phase 3 (2/2 complete: syntax mode/language caching done; split-resize + conflict search-typing invalidation now narrowed to volatile query overlays)
- 🔧 Phase 4 (1/2 complete: dedicated `conflict_three_way_scroll/style_window` benchmark added; tracing counters + CI budget reporting still pending)

### Benchmark Notes

- 2026-03-05 (`cargo bench -p gitgpui-ui-gpui --bench performance -- conflict_three_way_scroll/style_window`)
  - first run: `conflict_three_way_scroll/style_window/200` = `296.05 µs .. 299.31 µs`
  - note: new conflict-specific benchmark path added in this iteration (synthetic three-way conflict scroll fixture; 10k lines / 300 blocks defaults).
- 2026-03-05 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1837 ms .. 2.1955 ms`
  - after: `diff_scroll/style_window/200` = `2.1740 ms .. 2.1796 ms`
  - note: criterion showed an estimated improvement (`-1.8199% .. -0.5377%`) but still labeled this "Change within noise threshold"; this harness remains generic diff styling.
- 2026-03-05 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1872 ms .. 2.1997 ms`
  - after: `diff_scroll/style_window/200` = `2.1837 ms .. 2.1955 ms`
  - note: criterion reported "No change in performance detected" (estimated `-0.1778% .. +1.1635%`); this benchmark remains generic diff styling and does not isolate conflict search typing overlays directly.
- 2026-03-05 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1421 ms .. 2.1472 ms`
  - after: `diff_scroll/style_window/200` = `2.1872 ms .. 2.1997 ms`
  - note: criterion reported "Change within noise threshold" (estimated `+0.1954% .. +0.8947%`); this harness is generic diff styling and still does not isolate conflict search-typing overlay cache behavior directly.
- 2026-03-05 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1486 ms .. 2.1521 ms`
  - after: `diff_scroll/style_window/200` = `2.1421 ms .. 2.1472 ms`
  - note: criterion reported `Performance has improved` (`-1.4590% .. -1.1185%`); this harness is still generic diff styling and does not directly isolate split-resize drag cache invalidation.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1808 ms .. 2.1913 ms`
  - after: `diff_scroll/style_window/200` = `2.1697 ms .. 2.1733 ms`
  - note: criterion reported "Change within noise threshold" (estimated `-0.6238% .. -0.1803%`); this remains a generic diff-styling harness and does not isolate conflict syntax mode/language cache wins directly.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1486 ms .. 2.1521 ms`
  - after: `diff_scroll/style_window/200` = `2.1971 ms .. 2.2061 ms`
  - note: criterion reported "Change within noise threshold" (estimated `+0.9646% .. +1.3557%`); this harness remains generic diff styling and still does not isolate conflict compare canvas rendering directly.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1486 ms .. 2.1521 ms`
  - after: `diff_scroll/style_window/200` = `2.1730 ms .. 2.1811 ms`
  - note: criterion reported "Change within noise threshold" (estimated `+0.66% .. +1.02%`); this benchmark still does not isolate three-way conflict canvas row rendering directly.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1486 ms .. 2.1521 ms`
  - after: `diff_scroll/style_window/200` = `2.1565 ms .. 2.1669 ms`
  - note: criterion reported "Change within noise threshold"; benchmark remains generic diff styling and does not isolate two-way conflict canvas row rendering directly.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1615 ms .. 2.1768 ms`
  - after: `diff_scroll/style_window/200` = `2.1763 ms .. 2.1908 ms`
  - note: this harness does not cover the conflict-resolved gutter path directly; change is within expected noise for generic diff styling.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.1763 ms .. 2.1908 ms`
  - after: `diff_scroll/style_window/200` = `2.2042 ms .. 2.2140 ms`
  - note: criterion reported "Change within noise threshold"; this benchmark still does not isolate conflict resolver map lookup improvements directly.
- 2026-03-04 (`cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`)
  - before: `diff_scroll/style_window/200` = `2.2042 ms .. 2.2140 ms`
  - after: `diff_scroll/style_window/200` = `2.1486 ms .. 2.1521 ms`
  - note: criterion reported `Performance has improved` (~2.17%..2.88% faster); benchmark remains generic diff styling and does not isolate three-way conflict render lookups directly.

## Context

Large diff and conflict-resolution screens are currently less responsive than Zed on big files. The worst UX issues are:

- choppy scroll in large conflict views
- sluggish search/filter interactions while conflict panes are open
- resize drag hitching in split conflict mode

This document describes what to fix, how to fix it, and where to gather more data.

## Target Performance

Use these targets for large conflict cases (10k+ lines, 300+ conflict blocks):

- 60 Hz goal: p95 frame time under 10 ms, p99 under 16.6 ms while scrolling
- input latency: search keystroke to painted update under 50 ms p95
- split-resize drag: no sustained frame spikes over 25 ms
- no full-list re-render work proportional to total file size during steady-state scroll

## Baseline Signals

Current benchmark coverage includes generic diff styling, not conflict-render paths:

- `cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window`
- observed local baseline from this harness:
  - window 100: about 1.09 ms
  - window 200: about 2.19 ms
  - window 400: about 4.39 ms

Interpretation: core line styling scales roughly linearly and is not the largest bottleneck by itself. Conflict UI render/invalidation paths are likely the dominant jank source.

References:

- `crates/gitgpui-ui-gpui/benches/performance.rs:50`
- `crates/gitgpui-ui-gpui/src/view/rows/benchmarks.rs:110`

## Main Findings And Fixes

### P0: Resolved output gutter is not virtualized

Status: ✅ Implemented (iteration 1, 2026-03-04).

Historical evidence (before fix):

- `crates/gitgpui-ui-gpui/src/view/panels/main.rs:1894` computes `outline_len`
- `crates/gitgpui-ui-gpui/src/view/panels/main.rs:1896` calls `render_conflict_resolved_preview_rows(0..outline_len, ...)`
- `crates/gitgpui-ui-gpui/src/view/panels/main.rs:1951` renders all gutter rows as children

Impact:

- work scales with total line count, not visible window size
- large resolved outputs generate large element trees and expensive layout/paint

Fix:

- replace eager gutter row tree with `uniform_list` tied to `conflict_resolved_preview_scroll`
- render only visible gutter rows
- keep right-side text editor and gutter scroll fully synchronized
- optional follow-up: render gutter lane via canvas overlay in the editor viewport

Implementation anchors:

- render function: `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:565`
- scroll handle state: `crates/gitgpui-ui-gpui/src/view/panes/main.rs:1553`

### P0: Render-time recomputation in hot paths

Status: ✅ Implemented (iteration 3, 2026-03-04): two-way and three-way row/line conflict mappings are precomputed in `ConflictResolverUiState` and reused by render/navigation.

Historical evidence (three-way, before iteration 3):

- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:85` calls `build_three_way_column_conflict_ranges(...)` inside render
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:59` uses `position()` scan per line (`conflict_range_for_line`)
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:288` to `:292` does repeated lookup per rendered row

Evidence (two-way, historical before iteration 2):

- `render_conflict_resolver_diff_rows` rebuilt row-to-conflict mappings during render
- two-way conflict navigation helpers rebuilt mappings while jumping/selecting conflicts
- both were derived data that already originate from marker segments

Impact:

- repeated O(rows * conflicts) work in scroll hot path
- avoidable CPU and allocator pressure

Fix:

- precompute once when conflict state changes, store in `ConflictResolverUiState`
- add direct line-to-conflict index maps for O(1) lookup in row rendering
- remove range-building and map-building from render methods

State integration anchors:

- state struct: `crates/gitgpui-ui-gpui/src/view/mod.rs:462`
- state build path: `crates/gitgpui-ui-gpui/src/view/panes/main.rs:3633` and `:3750`
- mapping helpers: `crates/gitgpui-ui-gpui/src/view/conflict_resolver.rs:997` and `:1062`

### P0: Conflict rows are not on the canvas fast path

Status: ✅ Implemented (iteration 6, 2026-03-04): two-way and three-way conflict resolver rows plus conflict compare split/inline rows now use keyed canvas via `rows/conflict_canvas.rs`, guarded by `GITGPUI_CONFLICT_CANVAS_ROWS` fallback (enabled by default).

Evidence:

- conflict rows use `div` + `StyledText` through `conflict_diff_text_cell(...)`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1440`

Contrast:

- normal diff rows use keyed canvas and explicit paint path
- `crates/gitgpui-ui-gpui/src/view/rows/diff.rs:976`
- `crates/gitgpui-ui-gpui/src/view/rows/diff_canvas.rs:25`

Impact:

- more element construction, layout, and text-node churn in large conflict views
- weaker control over paint cost and reuse

Fix:

- create `conflict_canvas.rs` and port conflict split/inline/three-way line rendering to keyed canvas
- preserve interactions via hitboxes (selection, context menus, hover state)
- keep current non-canvas renderer behind a temporary fallback flag until parity is verified

Implementation anchors:

- canvas module: `crates/gitgpui-ui-gpui/src/view/rows/conflict_canvas.rs`
- two-way + three-way resolver + compare integration: `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs`
- fallback flag/state: `crates/gitgpui-ui-gpui/src/view/panes/main.rs`

### P1: Syntax mode and language resolution are too expensive in conflict renderers

Status: ✅ Implemented (iteration 7, 2026-03-04): conflict syntax language is now cached per file in `ConflictResolverUiState`, and conflict compare/resolver + three-way render paths compute syntax mode once per render batch using `MAX_LINES_FOR_SYNTAX_HIGHLIGHTING`.

Evidence:

- per-row language resolution in conflict compare/resolver rows:
  - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1007`
  - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1117`
  - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1336`
- conflict paths hard-code `DiffSyntaxMode::Auto` in many spots:
  - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1030`
  - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1144`
  - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1359`

Contrast:

- main diff renderer downgrades to `HeuristicOnly` for large line counts
- `crates/gitgpui-ui-gpui/src/view/rows/mod.rs:4`
- `crates/gitgpui-ui-gpui/src/view/rows/diff.rs:128`

Fix:

- cache syntax language once per conflict file in UI state
- pick syntax mode once per render batch based on total row count
- reuse main diff threshold strategy (`MAX_LINES_FOR_SYNTAX_HIGHLIGHTING`)
- optional second stage: split syntax tokens from query highlights so query updates do not force full styled-text rebuild

Implementation anchors:

- state field/default: `crates/gitgpui-ui-gpui/src/view/mod.rs`
- state sync + resync population: `crates/gitgpui-ui-gpui/src/view/panes/main.rs`
- conflict render batch syntax mode/language usage: `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs`

### P1: Cache invalidation is broad and frequent

Status: ✅ Implemented (iteration 9, 2026-03-05): split conflict resize drag no longer clears split caches on each move, and conflict split/inline search typing now invalidates only volatile query overlays while retaining stable syntax/word styled-text caches.

Evidence:

- historical (before iteration 9): diff search text changes cleared conflict split/inline styled caches.
- historical (before iteration 8): split resize drag cleared split conflict style cache each move.

Impact:

- frequent cache churn during typing and dragging
- user-visible stutter in already heavy views

Fix:

- split cache layers:
  - stable layer: syntax + structural word highlights
  - volatile layer: search query highlight overlay
- invalidate only volatile layer on each query keystroke
- for split resize, avoid full text-style cache clears; only invalidate geometry-dependent artifacts

Implementation anchors:

- resize ratio helper + drag invalidation scope: `crates/gitgpui-ui-gpui/src/view/panels/main.rs`
- query overlay cache state + targeted search invalidation: `crates/gitgpui-ui-gpui/src/view/panes/main.rs`
- split/inline stable-cache + query-overlay composition: `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs`
- query overlay composer: `crates/gitgpui-ui-gpui/src/view/rows/diff_text.rs`

### P2: Precomputed state is underused

Evidence:

- `ConflictResolverUiState` already stores precomputed visibility/ranges:
  - `crates/gitgpui-ui-gpui/src/view/mod.rs:479`
  - `crates/gitgpui-ui-gpui/src/view/mod.rs:487`
  - `crates/gitgpui-ui-gpui/src/view/mod.rs:488`
  - `crates/gitgpui-ui-gpui/src/view/mod.rs:489`
- render still recomputes related structures (see P0 above)

Fix:

- extend state with missing render indices/maps
- treat render code as pure lookup + paint work

## Proposed Delivery Plan

### Phase 1 (High impact, low risk)

- virtualize resolved output gutter list
- precompute three-way and two-way conflict maps in sync path
- remove render-time `map_two_way_rows_to_conflicts` and `build_three_way_column_conflict_ranges`

Expected result: major scroll smoothness gain on large conflict files.

### Phase 2 (High impact, medium risk)

- add conflict canvas renderer for split and inline modes
- add three-way canvas renderer after split/inline parity
- keep non-canvas fallback while validating behavior

Expected result: lower layout overhead and tighter frame-time distribution.

### Phase 3 (Medium impact, low risk)

- apply syntax-mode gating and language caching to conflict renderers
- reduce cache invalidation scope for search and resize

Expected result: improved responsiveness during search typing and pane resizing.

### Phase 4 (Measurement and guardrails)

- add dedicated conflict benchmarks and tracing counters
- set performance budgets in CI reports (alerting first, hard fail later)

Expected result: prevent regressions and quantify gains per phase.

## Instrumentation Plan

Add lightweight timing around hot functions (debug/dev builds first):

- `render_conflict_resolver_three_way_rows`
- `render_conflict_resolver_diff_rows`
- `render_conflict_resolved_preview_rows`
- conflict cache build paths (`build_cached_diff_styled_text` call sites)
- `recompute_conflict_resolved_outline_and_provenance`

Track at least:

- rows requested vs rows actually painted
- cache hit/miss ratio by cache type
- time spent in syntax highlighting vs word/query highlighting
- per-frame time spent in conflict render functions

## Benchmark Plan

Extend existing benchmark harness:

- file: `crates/gitgpui-ui-gpui/src/view/rows/benchmarks.rs`
- entrypoint: `crates/gitgpui-ui-gpui/benches/performance.rs`

Add fixtures/benches:

- ✅ `conflict_three_way_scroll/style_window` (window size configurable via `GITGPUI_BENCH_CONFLICT_WINDOW`)
- ⬜ `conflict_two_way_split_scroll/window_{100,200,400}`
- ⬜ `conflict_resolved_output_gutter_scroll/window_{100,200,400}`
- ⬜ `conflict_search_query_update` (simulate keystroke churn)
- ⬜ `conflict_split_resize_step` (simulate drag updates)

Acceptance thresholds (initial):

- three-way scroll window 200: p95 under 8 ms
- split scroll window 200: p95 under 6 ms
- search update on large conflict: p95 under 40 ms

## Validation Checklist

- verify identical UI behavior for:
  - row selection
  - context menus
  - hide-resolved behavior
  - active conflict navigation
  - conflict pick actions
- verify no visual regressions in:
  - syntax colors
  - word highlights
  - query highlights
  - line numbers and badges
- compare before/after traces on same synthetic and real large conflicts

## Where To Get More Information

Primary code hotspots:

- `crates/gitgpui-ui-gpui/src/view/panels/main.rs:1865`
- `crates/gitgpui-ui-gpui/src/view/panels/main.rs:1515`
- `crates/gitgpui-ui-gpui/src/view/panes/main.rs:1755`
- `crates/gitgpui-ui-gpui/src/view/panes/main.rs:3710`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:20`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:565`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:744`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1085`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1315`
- `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs:1440`
- `crates/gitgpui-ui-gpui/src/view/rows/diff.rs:6`
- `crates/gitgpui-ui-gpui/src/view/rows/diff_canvas.rs:25`
- `crates/gitgpui-ui-gpui/src/view/mod.rs:462`
- `crates/gitgpui-ui-gpui/src/view/conflict_resolver.rs:987`

Benchmark docs and harness:

- `docs/BENCHMARKS.md`
- `crates/gitgpui-ui-gpui/benches/performance.rs`
- `crates/gitgpui-ui-gpui/src/view/rows/benchmarks.rs`

Zed reference implementation patterns:

- editor-backed diff view:
  - `/home/sampo/git/zed/crates/git_ui/src/text_diff_view.rs:433`
  - `/home/sampo/git/zed/crates/git_ui/src/project_diff.rs:355`
- split companion + shared scroll anchor:
  - `/home/sampo/git/zed/crates/editor/src/split.rs:687`
  - `/home/sampo/git/zed/crates/editor/src/split.rs:692`
- incremental conflict updates (`ConflictSetUpdate`):
  - `/home/sampo/git/zed/crates/git_ui/src/conflict_view.rs:171`

Useful commands:

```bash
rg -n "render_conflict_resolved_preview_rows|render_conflict_resolver_diff_rows|render_conflict_resolver_three_way_rows|map_two_way_rows_to_conflicts|build_three_way_column_conflict_ranges|conflict_diff_text_cell" crates/gitgpui-ui-gpui/src/view
```

```bash
cargo bench -p gitgpui-ui-gpui --bench performance -- diff_scroll/style_window
```

## Recommended First PR Scope

Keep first PR narrowly focused to de-risk:

1. Virtualize resolved-output gutter.
2. Move three-way and two-way conflict mapping out of render.
3. Add a benchmark for three-way conflict scroll.

Then measure and iterate before moving conflict rows to canvas.
