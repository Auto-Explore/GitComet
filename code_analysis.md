# GitComet Code Analysis

Date: 2026-03-16

Scope: static source review of all crates — `gitcomet-core`, `gitcomet-app`, `gitcomet-git-gix`, `gitcomet-state`, and `gitcomet-ui-gpui`.

What I did not do in this pass:
- I did not run Criterion benches or add fresh measurements.
- I did not change production code.

Useful existing perf entry points:
- `crates/gitcomet-ui-gpui/benches/performance.rs`
- `crates/gitcomet-ui-gpui/src/bin/perf_budget_report.rs`
- `crates/gitcomet-ui-gpui/src/view/perf.rs`

---

## Executive Summary

The highest-value work is in the diff pipeline.

The current core diff engine already has a streamed plan representation, but several higher-level features fall back to fully materialized `Vec<FileDiffRow>` data and owned `String` copies. That means the project pays for both the fast path and the convenience path.

The other major hot path is repo monitoring and status refresh. Both still depend on extra `git` subprocess work in places where the rest of the stack is already using `gix`, which adds avoidable process and parsing overhead.

The main technical-debt theme is custom infrastructure with high maintenance surface:
- custom cache eviction in multiple places
- custom ignore matching via `git check-ignore`
- a large custom text model/input stack
- several very large source files that mix unrelated responsibilities

Additional themes from this deep-dive:
- **Rendering hot paths** in diff_text.rs and conflict_resolver.rs allocate strings, Vecs, and format! outputs on every frame
- **CLI argument parsing** in the app crate has duplicated logic, unnecessary PathBuf clones, and deeply nested match chains
- **Build configuration** has a duplicate resvg dependency and no dev/test profile tuning
- **Domain types** use owned Strings where Arc<str> would avoid cloning overhead

---

## Priority Backlog

### P0: Replacement alignment in `file_diff.rs` does repeated expensive similarity work

Evidence:
- `crates/gitcomet-core/src/file_diff.rs:593-602`
- `crates/gitcomet-core/src/file_diff.rs:1078-1151`

Why it matters:
- `push_aligned_replacement_runs_to_plan()` computes a DP matrix for delete/insert blocks.
- Every cell calls `replacement_pair_cost()`.
- `replacement_pair_cost()` calls `levenshtein_distance()`.
- `levenshtein_distance()` allocates `Vec<char>` for both strings on every call (lines 1127-1128).
- The prev/curr DP vectors (lines 1137-1138) are also freshly allocated per call.
- Near `REPLACEMENT_ALIGN_CELL_BUDGET` this becomes tens of thousands of repeated allocations and UTF-8 traversals in one diff region.

Action:
- [x] Add a dedicated benchmark for replacement-heavy blocks, not just scroll/render benches.
- [x] Pre-trim shared prefix/suffix before distance calculation.
- [x] Cache per-line metadata needed by `replacement_pair_cost()` instead of recomputing it per cell.
- [x] Rework the current Levenshtein path to reuse scratch buffers instead of allocating per pair.
- [x] Replaced the scratch-buffer Levenshtein with `strsim::generic_levenshtein` after benchmarking it against the old implementation on the replacement-alignment Criterion fixture. On this machine, balanced blocks dropped from ~406 ms to ~241 ms and skewed blocks from ~390 ms to ~235 ms; the old scratch path remains only as a benchmark control.
- [x] Cache pair costs for repeated line pairs inside the same replacement block (HashMap keyed on `(&str, &str)` in `replacement_alignment_ops`).
- [x] Add a cheap initial heuristic: when either trimmed side is empty after shared prefix/suffix removal, compute exact distance without the DP. Also skip DP for duplicate text pairs via the cache.

Validation:
- Extend the Criterion suite with a replacement-alignment benchmark.
- Watch both runtime and allocation count before/after.

### P0: Streamed diff providers still rematerialize owned rows and full inline text

Evidence:
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:250-318`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:321-385`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:497-509`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:538-571`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:649-682`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1752-1759`

Why it matters:
- `split_row()` and `inline_row()` convert borrowed line slices into owned `String`s on demand.
- Page caches store owned `FileDiffRow` / `AnnotatedDiffLine` values instead of lightweight refs.
- `build_inline_text()` walks the entire inline diff and rebuilds a full `SharedString`.
- `ensure_file_diff_inline_text_materialized()` triggers that full reconstruction even when a row provider already exists.

Action:
- N/A — Compact row references assessed: `split_row()` and `inline_row()` already produce `Arc<str>` text (not owned `String`). The remaining struct overhead per row (~64 bytes for kind + line numbers + Arc pointers) across 64 cached pages of 256 rows is ~1MB total — negligible versus the complexity of a compact reference scheme that would require keeping the `StreamedFileDiffSource` alive for every access.
- [x] Reuse `Arc<str>` for streamed split `FileDiffRow` text so cached pages and row clones stop duplicating line buffers.
- [x] Cache provider-built inline full text and build it directly from the plan/source texts, so word-wrap mode no longer materializes `AnnotatedDiffLine` rows just to concatenate them.
- N/A — Only one consumer of `ensure_file_diff_inline_text_materialized` exists (word-wrap mode in `diff.rs`), and it genuinely needs the full text string to feed to `TextInput`. Removing it requires `TextInput` to accept a streamed/paged source instead of materialized text — that's an editor-stack architectural change, not a diff-cache fix.
- [x] Add streamed file-diff debug counters for page cache hit/miss, inline full-text materializations, and rows materialized into cached pages.

Validation:
- Existing benches: large diff scroll, paged rows, patch diff search.
- Add memory snapshots around opening and scrolling large diffs.

### P0: Multiple secondary features re-diff and rematerialize whole documents

Evidence:
- `crates/gitcomet-ui-gpui/src/view/markdown_preview.rs:248-268`
- `crates/gitcomet-ui-gpui/src/view/panes/main/helpers.rs:705-770`
- `crates/gitcomet-ui-gpui/src/view/conflict_resolver.rs:1975-1989`
- `crates/gitcomet-ui-gpui/src/view/conflict_resolver.rs:2945-3068`

Why it matters:
- Markdown diff preview calls `side_by_side_rows()` to compute change masks and row alignment.
- Conflict decision-region logic calls `side_by_side_rows_with_anchors()` even though it mainly needs anchors and line-emission counts.
- Conflict word highlighting calls `side_by_side_rows()` again before doing per-line word diffing.

Action:
- Extend `FileDiffPlan` with helpers/iterators for:
  - [x] changed-line masks
  - [x] region anchors
  - [x] modify-pair iteration (via `for_each_side_by_side_row` + `PlanRowView`)
  - [x] prefix counts of "emits line on old/new side"
- [x] Migrate markdown preview and conflict decision-region calculation to those plan-level APIs.
- [x] Migrate conflict word highlighting to plan-level modify-pair APIs.
- Keep `side_by_side_rows()` as a compatibility API for tests and small utilities only.

Validation:
- Existing markdown preview and conflict benches are the right guardrails.
- Add one benchmark that includes word-highlighting over a multi-block conflict.

---

### P1: Rendering hot paths allocate per-frame in diff_text.rs

Evidence:
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_text.rs:254-380` (diff_text_line_for_region)
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_text.rs:262` (expand_tabs)
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_text.rs:305,311,326,341,367` (SharedString clones)
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_text.rs:454` (format! in selection)
- `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs:398-416` (segment boundary Vec)
- `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs:900-902` (tab expansion when no tabs)

Why it matters:
- `diff_text_line_for_region()` clones SharedString at 5 return points per call, called O(visible_rows) per render.
- Tab expansion (line 262): when a line has no tabs, still does `s.to_string().into()` instead of `SharedString::from(s)` — unnecessary intermediate String.
- Line 454: `format!("{}\t{}", left, right)` builds a combined string for every row in split-view selection, even when the selection doesn't include that row.
- Segment boundary building (lines 398-416): allocates and sorts a Vec per line per render for syntax highlight boundaries.
- Tab expansion (line 900-902): when no tabs present, still allocates `text.to_string().into()` and clones the highlights Vec.

Action:
- N/A — `Cow<'_, SharedString>` assessed as low-value: SharedString clones are cheap atomic ref-count bumps (Arc<str>), not heap copies. The 5 clone sites in `diff_text_line_for_region` are already O(1).
- [x] Fix `expand_tabs` to use `SharedString::from(s)` directly when no tabs present.
- N/A — Selection combined-string (`format!`) only runs on copy action (user-triggered), not per render frame. Already acceptable.
- [x] Reuse the boundary Vec across line renders via thread-local buffer in `build_diff_text_segments()`.
- [x] Tab expansion in `rows/diff_text.rs` `maybe_expand_tabs` also fixed to use `SharedString::new(s)` instead of `s.to_string().into()` when no tabs present (matching the `panes/main/diff_text.rs` fix).

Validation:
- Profile with a 10k-line diff file and measure allocations per scroll frame.

### P1: Conflict resolver rendering allocates heavily per-frame

Evidence:
- `crates/gitcomet-ui-gpui/src/view/rows/conflict_resolver.rs:168-187` (Vec allocations)
- `crates/gitcomet-ui-gpui/src/view/rows/conflict_resolver.rs:197-277` (HashMap lookups in loop)
- `crates/gitcomet-ui-gpui/src/view/rows/conflict_resolver.rs:303,357,682-686,734-738` (format! for menu IDs)
- `crates/gitcomet-ui-gpui/src/view/rows/conflict_resolver.rs:548,556,568,607,636,666` (line.to_string() -> SharedString)

Why it matters:
- `conflict_choices` Vec and `real_line_indices` Vec are freshly allocated every render call.
- Context menu ID strings are built via `format!()` on every render even when no menu is open.
- `.map(|line| SharedString::from(line.to_string()))` appears 6+ times — intermediate String is unnecessary, use `SharedString::from(line)` directly.
- Multiple `.clone()` calls on closures/handlers per conflict chunk (lines 363, 367, 699-703, 751-755).

Action:
- [x] Cache `conflict_choices` at the model layer; update only when segments change.
- [x] Stop collecting `real_line_indices` into a temporary Vec; iterate the visible range directly for prewarming.
- [x] Use lazy evaluation for context menu strings — only build when a menu is opened.
- [x] Replace `SharedString::from(line.to_string())` with `SharedString::from(line)`.
- N/A — Handler data assessed: `selected_choices` is `Vec<ConflictChoice>` where `ConflictChoice` is a 1-byte `Copy` enum. The Vec is cloned into `cx.listener()` closures during render, but these closures only execute on user mouse-down events. The per-chunk Vec is tiny (N conflicts * 1 byte). Total clone cost is negligible versus the complexity of wrapping in `Arc`.

Validation:
- Open a conflict with 20+ blocks and profile scroll performance.

### P1: Repo monitor ignore matching is process-heavy and semantically complex

Evidence:
- `crates/gitcomet-state/src/store/repo_monitor.rs:356-596`
- `crates/gitcomet-state/src/store/repo_monitor.rs:793-850`

Why it matters:
- Cache misses spawn `git check-ignore` work, sometimes in batches and sometimes with synthetic probe paths to emulate directory-only rules.
- This lives inside the file-event classification loop for the active repo.
- The code is careful, but it is also expensive and brittle: subprocess spawn, I/O, parsing, TTL cache, and rule-probe semantics all sit on the hot path.

Action:
- [x] Add counters for ignore lookups and average/burst latency first. `repo_monitor.rs` now records ignore request count, cache hits/misses, fallback count, and average/max uncached lookup latency.
- [x] Replace subprocess-based matching with an in-process `gix` matcher. `GitignoreRules` now caches a `gix` exclude stack and short-circuits tracked paths via the index; subprocess `git check-ignore` remains only as a narrow fallback if matcher setup/query fails.
- [x] Prefer `gix` ignore support if it covers nested `.gitignore`, excludesfile, and tracked-vs-ignored behavior cleanly.
- [x] Removed the `git check-ignore` subprocess fallback entirely. The gix matcher handles all standard gitignore semantics (nested `.gitignore`, `info/exclude`, `core.excludesFile`, tracked-path short-circuit). When the matcher fails (e.g., no index, broken repo), paths default to not-ignored (safe: may cause extra refreshes, never misses changes). Also removed `prefetch_ignored_rels` and the two-pass event classification, since batch subprocess calls were the only reason for that design. Fixed pre-existing test issue: `init_repo_for_ignore_tests` now creates an initial commit so the index file exists for the gix excludes stack.

Validation:
- Reuse the existing repo-monitor tests as regression coverage.
- Add a filesystem-event burst perf smoke test.

### P1: Status refresh still shells out to `git status` after running `gix` status

Evidence:
- `crates/gitcomet-git-gix/src/repo/status.rs:132-185`
- `crates/gitcomet-git-gix/src/repo/status.rs:356-398`

Why it matters:
- `status_impl()` already builds status with `gix`.
- It then always runs `git status --porcelain=v2 -z --ignore-submodules=none` to supplement gitlink/submodule entries.
- On frequently refreshed status paths this duplicates repository scanning and output parsing, even for repos that do not use submodules.

Action:
- [x] Gate `supplement_gitlink_status_from_porcelain()` behind a cheap "repo likely has gitlinks/submodules" check. Guarded by `.gitmodules` existence check plus index scan for mode-160000 entries — repos without submodules or gitlinks skip the subprocess entirely.
- [x] Cache that capability check per repo, invalidating on `.gitmodules` / index file stamp changes so the index scan is skipped on steady-state refreshes.
- Long-term, move gitlink status supplementation in-process if `gix` can provide enough data.
- [x] Removed redundant linear `push_status_entry()` dedup scan — `sort_and_dedup()` already handles deduplication downstream.

Validation:
- Compare repo-open and status-refresh timings on:
  - repo without submodules
  - repo with many submodules/gitlinks

### P1: Custom cache eviction is arbitrary, not usage-based

Evidence:
- `crates/gitcomet-ui-gpui/src/view/rows/mod.rs:36-57`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:76-99`

Why it matters:
- These helpers evict `HashMap` keys by iterating `cache.keys().take(remove_count)`.
- That is arbitrary hash iteration order, not LRU or MRU.
- Hot entries can be evicted while cold entries survive, which creates unpredictable cache behavior and makes perf debugging harder.
- The eviction helper also allocates a `Vec<u64>` of keys to remove (could iterate and remove directly).

Action:
- [x] Deduplicated the two identical eviction helpers into one shared generic function (`rows::insert_with_partial_cache_eviction`).
- [x] Removed intermediate `Vec` allocation in eviction path (now uses `retain`).
- [x] Replace with a real LRU cache policy — all 6 HashMap+eviction caches migrated to `lru::LruCache` (thread-local text layout caches use FxHasher-backed LRU, page caches use default hasher LRU). Removed `insert_with_partial_cache_eviction` entirely.
- [x] Centralize cache wrappers so all six UI LRU caches share one instrumented wrapper with the same hit/miss/evict/clear counters and invalidation semantics.

Validation:
- Measure hit ratio for history text shaping and diff page caches before/after.

### P1: Domain types use owned Strings where Arc<str> would save cloning

Evidence:
- `crates/gitcomet-core/src/domain.rs:16-29` (CommitId, Commit)

Why it matters:
- `CommitId(pub String)` and `Commit::summary`, `Commit::author` are frequently shared/passed by value.
- `DiffLine::text` already uses `Arc<str>` (line 226), but domain-level types don't follow the same pattern.
- Commit data is immutable after creation — perfect candidate for Arc<str>.

Action:
- [x] Change `CommitId` to wrap `Arc<str>` instead of `String`. Added `Display` impl for format string ergonomics.
- [x] Use `Arc<str>` for `Commit::summary` and `Commit::author`. (`Arc<str>` does implement `Display` via `str: Display`; the deferral reason was incorrect. Migration was mechanical: construction sites use `.into()`, test assertions use `&*` deref.)
- [x] Extend the immutable shared-string audit to stash/reflog/blame metadata: `StashEntry::message`, `ReflogEntry::message`/`selector`, and `BlameLine::commit_id`/`author`/`summary` now use `Arc<str>` so large list clones stop copying repeated text payloads.
- [x] Audit other domain types passed by value to see if they should follow. Assessed `Branch::name`, `RemoteBranch::remote/name`, `Remote::name`, `Tag::name`, `Upstream::remote/branch`, `Worktree::branch` — these are short strings cloned infrequently (sidebar rebuilds), and migrating them would touch 20+ files across message/effect/reducer types for minimal benefit. `CommitDetails::message`/`committed_at` are already behind `Arc<CommitDetails>` in state. Not worth migrating.

Validation:
- Check that existing tests pass and profile clone-heavy paths (history list, branch list).

---

### P2: text_input.rs has multiple per-edit and per-render allocation issues

Evidence:
- `crates/gitcomet-ui-gpui/src/kit/text_input.rs:1877-1878,2614-2615,2651-2652` (double .clone() on range/inserted)
- `crates/gitcomet-ui-gpui/src/kit/text_input.rs:2285,2296,2301,2316,2350` (debug_selector .to_string())
- `crates/gitcomet-ui-gpui/src/kit/text_input.rs:1232,1822,1830,2563` (selected text .to_string())

Why it matters:
- On every keystroke, `range.clone()` and `inserted.clone()` are called twice in a row to feed two consumers.
- `debug_selector(|| "text_input_context_cut".to_string())` creates new String allocations on every render, even in release builds.
- Selected text is freshly allocated via `.to_string()` on each access.

Action:
- [x] Extract range/inserted pair once, pass by reference to both consumers. (QW#5)
- N/A — `debug_selector` is already a noop in release builds (gpui cfg). (QW#4)
- N/A — `selected_text()` and clipboard `.to_string()` calls are all event-driven (user-triggered cut/copy/paste), not per-frame. Caching would add complexity for no measurable benefit.

Validation:
- Profile keystroke latency in a large text input.

### ~~P2: text_model.rs piece operations clone unnecessarily~~ [DONE]

Made `Piece` derive `Copy` (all fields are `Copy`), eliminating `.clone()` overhead in `split_pieces_at` and `merge_adjacent_pieces`. `ranges.iter().cloned()` was assessed — `Range<usize>` is `Clone` but not `Copy`, and the range is needed owned for the worker assignment Vec, so the clone is necessary; changed to `.iter().enumerate()` with explicit `.clone()` at the use site for clarity.

### P2: The custom text model still pays full-document costs in important paths

Evidence:
- `crates/gitcomet-ui-gpui/src/kit/text_model.rs:75-121`
- `crates/gitcomet-ui-gpui/src/kit/text_model.rs:184-195`
- `crates/gitcomet-ui-gpui/src/kit/text_model.rs:321-348`

Why it matters:
- `LineIndex::apply_edit()` rebuilds the edited line-start array and then sorts/dedups it.
- `materialized()` reconstructs the whole document into a `SharedString`.
- The project already has dedicated benchmarks for text model load and snapshot clone cost, which is a sign this area is performance-sensitive.

Action:
- [x] Rewrite `LineIndex::apply_edit()` to emit monotonic output directly — removed `sort_unstable()` / `dedup()` and the redundant `first != 0` guard. The three sections (prefix, inserted breaks, shifted suffix) occupy non-overlapping value ranges and are already in order. Added 10 boundary-condition test cases.
- [x] Add benchmarks for fragmented-buffer random edits and repeated `as_str()` / `as_shared_string()` access after edits. `TextModelFragmentedEditFixture` covers piece-table edit throughput, `as_str()` materialization after fragmentation, `as_shared_string()` repeated reads, and a `String` control baseline.
- If editor ambitions keep expanding, evaluate `ropey` or `xi-rope` against the existing benchmarks before adding more bespoke structure around the current model.

Validation:
- Use the existing text-model and text-input benches as the decision point for whether a rope migration is justified.

### P2: Store mutation cost is hidden behind `Arc::make_mut`

Evidence:
- `crates/gitcomet-state/src/store/mod.rs:91-102`
- `crates/gitcomet-state/src/model.rs:136-143`
- `crates/gitcomet-state/src/model.rs:315-370`

Why it matters:
- Every dispatched message mutates `Arc<AppState>` through `Arc::make_mut()`.
- If the UI is holding a snapshot, the whole top-level state tree is cloned before the reducer mutates it.
- Many large payloads are behind `Arc`, so this may be acceptable today, but the cost is invisible without instrumentation.

Action:
- [x] Add reducer timing and clone-cost counters before changing architecture. `AppStore::reducer_diagnostics()` now exposes dispatch count, total/max reducer nanos, clone-on-write count, total/max clone nanos, and the max number of extra shared state handles observed before `Arc::make_mut()`.
- If it becomes visible in traces, split state into smaller shared nodes or move to a more selective propagation model.

Validation:
- Measure dispatch throughput during repo open, refresh storms, and conflict-view interactions.

### ~~P2: Diff line classification uses linear prefix chain~~ [DONE]

Evidence:
- `crates/gitcomet-core/src/domain.rs:344-366` (classify_unified_line)

Why it matters:
- 10+ sequential `starts_with()` checks on every diff line.
- Could use first-byte dispatch for faster classification.

Action:
- [x] Refactored to match on `raw.as_bytes().first()` and only perform the relevant prefix checks for each leading byte, preserving header handling while fixing `---`/`+++` content classification.
```rust
match raw.as_bytes().first() {
    Some(b'@') if raw.starts_with("@@") => DiffLineKind::Hunk,
    Some(b'd') if raw.starts_with("diff ") => DiffLineKind::Header,
    Some(b'+') if !raw.starts_with("+++ ") => DiffLineKind::Add,
    Some(b'-') if !raw.starts_with("--- ") => DiffLineKind::Remove,
    _ => DiffLineKind::Context,
}
```

Validation:
- Benchmark against a large unified diff (10k+ lines).

### ~~P2: Page caching has double allocation in load_page~~ [DONE]

Evidence:
- `crates/gitcomet-core/src/domain.rs:270-285` (PagedDiffLineProvider::load_page)

Why it matters:
- `self.lines[start..end].to_vec()` creates a temporary Vec, then converts to `Arc<[DiffLine]>` (two allocations where one suffices).

Action:
- [x] Switched `PagedDiffLineProvider::load_page()` to build `Arc<[DiffLine]>` directly from the backing slice, removing the temporary `Vec`.

### ~~P2: Histogram diff creates unnecessary slice copies~~ [DONE]

Evidence:
- `crates/gitcomet-core/src/file_diff.rs:1180-1181`

Why it matters:
- `old[old_start..old_end].to_vec()` and `new[new_start..new_end].to_vec()` allocate Vecs of `&str` references when slices could be passed directly.

Action:
- [x] Passed slice refs directly into the Myers fallback instead of allocating temporary `Vec<&str>` copies.

---

## Code Complexity and Duplication (App Crate)

### Deeply nested match chains in CLI argument validation

Evidence:
- `crates/gitcomet-app/src/cli.rs:295-342` (classify_difftool_input — 3-level nested match, 9 arms)
- `crates/gitcomet-app/src/cli.rs:374-411` (validate_existing_merged_output_path — near-duplicate logic)
- `crates/gitcomet-app/src/cli.rs:474-554` (resolve_mergetool_with_env — 81 lines, 6 responsibilities)

Action:
- [x] Extract `classify_path()` helper with `ResolvedPathKind` enum to deduplicate symlink metadata logic between `classify_difftool_input` and `validate_existing_merged_output_path`. Both functions are now flat match statements over the shared enum.
- [x] Extract `parse_conflict_style()` and `parse_diff_algorithm()` from `resolve_mergetool_with_env()`. `parse_marker_size()` was already extracted.
- [x] Extract `resolve_and_validate_mergetool_paths()` plus small path helper functions so `resolve_mergetool_with_env()` now delegates path resolution/validation and only assembles the final config.

### ~~Compat argument parsing is a 240-line if-chain~~ [DONE]

Evidence:
- `crates/gitcomet-app/src/cli/compat.rs:184-410` (parse_compat_external_mode_with_config)

Why it matters:
- 15+ sequential if-statements for CLI argument parsing.
- PathBuf clones from `positionals` vector instead of using moves (lines 343-344, 369-371, 378-380, 393-394).

Action:
- [x] Refactored `parse_compat_external_mode_with_config()` into an explicit token classifier plus cursor/state parser, keeping the merge/diff resolution logic in separate finish functions instead of one long `if` chain.
- [x] Use `std::mem::take()` to move PathBufs instead of cloning (all 5 clone sites in merge and diff mode construction).

### ~~Duplicated hex encoding~~ [DONE]

Evidence:
- `crates/gitcomet-app/src/cli/compat.rs:89-97`
- `crates/gitcomet-app/src/crashlog.rs:256-264`

Action:
- [x] Moved `hex_encode()` to shared app-crate scope (`main.rs`) and reused it from both `compat.rs` and `crashlog.rs`.

### ~~Error formatting boilerplate~~ [DONE]

Created `io_err!` macro with three variants (no-path, one-path, two-path) using `concat!` for compile-time format string construction. Applied to 16 of ~20 error sites; remaining 7 have unique message structures (label variables, post-path context phrases, non-"Failed to" prefixes) and are left as-is.

### ~~UTF-8 fallback conversion is suboptimal~~ [DONE]

Replaced `write!(out, "\\x{byte:02x}")` with direct char pushes using a `HEX_DIGITS` lookup table. Added 5 unit tests covering pure ASCII, valid UTF-8, invalid bytes, empty input, and leading invalid bytes.

---

## Code Complexity (Core Crate)

### ~~Trailing newline merge logic~~ [DONE]

Extracted to `apply_trailing_newline_decision()` helper with clear parameters.

### ~~Redundant allocation in conflict merge~~ [DONE]

Added hunk-level short-circuit: when `ours_hunks == theirs_hunks` (structurally identical), skip `reconstruct_side` for theirs entirely. Falls back to content comparison when hunks differ.

### ~~Mutex handling pattern repeated~~ [DONE]

Evidence:
- `crates/gitcomet-core/src/auth.rs:44,50,56`

Action:
- [x] Extracted `lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T>` helper in `auth.rs` and routed all three call sites through it.

### ~~Custom error boilerplate~~ [DONE]

Migrated `Error` and `ErrorKind` to `thiserror::Error` derive. `GitFailure` keeps its manual `Display` (conditional timeout/failure logic). Removed ~15 lines of manual `Display`/`Error` impls.

---

## Build and Dependency Issues

### Duplicate resvg versions (blocked on gpui upgrade)

Evidence:
- `Cargo.lock` contains both `resvg v0.45.1` and `resvg v0.47.0`
- Investigation: `gpui 0.2.2` (registry dependency) pulls `resvg 0.45.1`, while the workspace depends on `resvg 0.47.0`.

Why it matters:
- Both versions compile, increasing build time and binary size.

Action:
- Blocked: requires `gpui` crate to update its resvg dependency. Cannot be fixed by workspace version override since the versions are semver-incompatible.

### ~~Missing dev/test build profiles~~ [DONE]

Added `[profile.dev]` (incremental, opt-level=0) and `[profile.test]` (opt-level=1, split-debuginfo=packed) to workspace Cargo.toml.

### ~~Tree-sitter grammars are not feature-gated~~ [DONE]

Evidence:
- 12 language grammars (bash, css, go, html, javascript, json, python, rust, typescript, xml, yaml) always compiled.
- Each grammar adds ~500KB-2MB to the binary.

Action:
- [x] Added grouped grammar features in `gitcomet-ui-gpui`: `syntax-minimal`, `syntax-common`, `syntax-all`, plus per-group toggles (`syntax-web`, `syntax-rust`, `syntax-python`, `syntax-go`, `syntax-data`, `syntax-shell`, `syntax-xml`).
- [x] Made the tree-sitter grammar crates optional and gated grammar/query wiring in `diff_text/syntax.rs` so disabled grammars cleanly fall back to heuristic highlighting (`tree_sitter_grammar()` / `tree_sitter_highlight_spec()` return `None`).
- [x] Kept the default workspace behavior unchanged by making `syntax-all` the default set, while exposing smaller syntax subsets through `gitcomet-app` features (`ui-gpui-syntax-common`, `ui-gpui-syntax-minimal`) for leaner builds.

### Large test/benchmark files slow incremental compilation

Evidence:
- `view/panels/tests.rs`: 9,899 lines
- `view/rows/benchmarks.rs`: 6,304 lines
- `view/conflict_resolver/tests.rs`: 4,707 lines
- `view/panels/popover/tests.rs`: 2,109 lines

Action:
- [x] Split `view/panels/tests.rs` (9,899 lines) into a directory module with 5 submodules: `file_diff.rs` (1,663 lines), `large_file_diff.rs` (1,157 lines), `conflict.rs` (3,541 lines), `markdown.rs` (1,191 lines), `file_status.rs` (1,590 lines). Shared helpers and fixtures remain in `mod.rs` (774 lines). All 78 tests pass.
- [x] Split `view/panels/popover/tests.rs` (2,109 lines) into a directory module with 4 submodules: `file_actions.rs` (832 lines), `refs.rs` (684 lines), `status.rs` (467 lines), `stash.rs` (113 lines). Shared imports and the `TestBackend` fixture live in `mod.rs` (22 lines). All 22 popover tests pass.
- [x] Split `view/conflict_resolver/tests.rs` (4,705 lines) into a directory module with 5 submodules: `parsing.rs` (715 lines), `resolution.rs` (1,063 lines), `visibility.rs` (1,350 lines), `block_diff.rs` (520 lines), `split_row_index.rs` (1,045 lines). Shared imports and `mark_block_resolved` helper in `mod.rs` (27 lines). All 172 tests pass.
- [x] Split `view/rows/benchmarks.rs` (6,636 lines) into a topic-focused root plus sibling modules: `benchmarks/syntax.rs` (syntax + preview fixtures), `benchmarks/conflict.rs` (conflict fixtures + helpers), and `benchmarks/tests.rs` (79 benchmark regression tests). The root `benchmarks.rs` now holds repo/text/patch fixtures plus shared cross-cutting helpers and re-exports the moved fixture APIs. `cargo check --tests --benches -p gitcomet-ui-gpui --features benchmarks` is clean and `cargo test -p gitcomet-ui-gpui --features benchmarks --lib -- benchmarks::tests::` passes all 79 tests.

### Binary size: 41MB release build

Evidence:
- gitcomet-ui-gpui: 29MB, gitcomet-app: 41MB total

Action:
- Audit with `cargo bloat` after fixing resvg duplication and feature-gating tree-sitter.

---

## Simplification and Reuse Opportunities

### Reuse the streamed diff plan everywhere

This is the best reuse opportunity in the codebase.

Today, the codebase has both:
- a compact plan representation in `gitcomet_core::file_diff`
- several consumers that fall back to materialized `Vec<FileDiffRow>`

Consolidating on the plan-level API will:
- remove duplicate diff work
- reduce allocations
- make correctness fixes land in one place
- shrink the number of rendering/data adapters the team needs to maintain

### Replace ad hoc UI caches with a shared cache abstraction

The current cache story is fragmented:
- page caches in `diff_cache.rs`
- text-shape caches in `rows/mod.rs` and `history_canvas.rs`
- provider highlight caches in `text_input.rs`

A small shared cache module with:
- explicit policy (LRU via `lru` or `mini-moka`)
- counters
- uniform invalidation

would remove duplicated eviction code and make cache tuning much easier.

### Replace subprocess ignore matching with an in-process matcher

This is both a performance improvement and a simplification.

The current code works hard to preserve Git semantics. That effort is valuable, but it belongs inside a proper ignore engine rather than in bespoke subprocess orchestration and synthetic path probes.

### ~~Deduplicate symlink classification in app crate~~ [DONE]

Extracted `classify_path()` returning `ResolvedPathKind` enum. Both `classify_difftool_input` and `validate_existing_merged_output_path` are now flat match statements over the shared classification result.

---

## Large-File Technical Debt Map

These files are large enough that perf work inside them will remain risky until responsibilities are split:

| File | LOC | Risk |
|------|-----|------|
| `view/rows/diff_text/syntax.rs` | ~5.9k | Syntax parsing + projection + reuse |
| `kit/text_input.rs` | ~5.6k | Selection + wrap + highlight + paint + actions |
| `view/panes/main/diff_cache.rs` | ~1.9k | Syntax cache + pane orchestration |
| `view/conflict_resolver.rs` | ~3.3k | Bootstrap + render + actions (word highlight + split row index extracted) |
| `gitcomet-core/file_diff.rs` | ~2.0k | Plan + algorithms + anchors + materialize |
| `cli/compat.rs` | ~450 | Parsing + label assignment + mode detection |

Suggested decomposition:

- `text_input.rs`
  - `selection.rs`
  - `wrap.rs`
  - `highlight_provider.rs`
  - `paint_cache.rs`
  - `actions.rs`

- `diff_text/syntax.rs`
  - `prepared_document.rs`
  - `background_jobs.rs`
  - `line_projection.rs`
  - `reuse.rs`

- `diff_cache.rs`
  - `syntax_cache.rs`
  - `pane_adapter.rs`

- [x] Follow-up maintenance split: extracted the streamed file-diff providers/rebuild logic into `view/panes/main/diff_cache/file_diff.rs`, reducing `diff_cache.rs` to ~3.3k LOC while leaving patch diff, syntax cache, image cache, and pane orchestration in the parent module.
- [x] Follow-up maintenance split: extracted paged patch diff types (`PagedPatchDiffRows`, `PagedPatchSplitRows`, `PatchInlineVisibleMap`, `PatchSplitVisibleMeta`, visibility helpers) and their 6 tests into `view/panes/main/diff_cache/patch_diff.rs` (~938 LOC), reducing `diff_cache.rs` from ~3.3k to ~2.4k LOC.
- [x] Follow-up maintenance split: extracted the file image-diff cache helpers, SVG rasterization/cache fallback path, and related tests into `view/panes/main/diff_cache/image_cache.rs` (~548 LOC), reducing `diff_cache.rs` from ~2.4k to ~1.9k LOC and leaving syntax cache plus pane orchestration in the parent module.

- `conflict_resolver.rs`
  - `bootstrap.rs`
  - `render_model.rs`
  - `word_highlight.rs`
  - `large_block_preview.rs`
  - `actions.rs`
- [x] Follow-up maintenance split: extracted word-highlight computation (`compute_three_way_word_highlights`, `compute_two_way_word_highlights`, `compute_word_highlights_for_row`, `merge_ranges`, `should_skip_large_block_word_highlights`) into `view/conflict_resolver/word_highlight.rs` (~277 LOC).
- [x] Follow-up maintenance split: extracted split row index (`SparseLineIndex`, `ConflictSplitPageCache`, `ConflictSplitRowIndex`, `TwoWaySplitSpan`, `TwoWaySplitVisibleRow`, `TwoWaySplitProjection`) into `view/conflict_resolver/split_row_index.rs` (~771 LOC), reducing `conflict_resolver.rs` from ~4.3k to ~3.3k LOC.

- `file_diff.rs`
  - `plan.rs`
  - `algorithms/myers.rs`
  - `algorithms/patience.rs`
  - `anchors.rs`
  - `materialize.rs`
  - `tests.rs`

---

## Quick Wins (can be done independently, low risk)

These items require minimal context and can be picked up in any order:

| # | Item | File(s) | Est. effort |
|---|------|---------|-------------|
| 1 | [x] Fix `expand_tabs` to skip allocation when no tabs | diff_text.rs:262 | 5 min |
| 2 | [x] Replace `line.to_string()` -> SharedString with direct conversion | conflict_resolver.rs:548+ (6 places) | 10 min |
| 3 | N/A — Range<usize> is Clone, not Copy | conflict_resolver.rs:109 | 2 min |
| 4 | N/A — debug_selector is already noop in release builds (gpui cfg) | text_input.rs:2285-2350 | 10 min |
| 5 | [x] Fix double .clone() on range/inserted in edit path | text_input.rs:1877-1878 | 5 min |
| 6 | [x] Pass slice refs instead of `.to_vec()` in histogram diff | file_diff.rs:1180-1181 | 5 min |
| 7 | [x] Deduplicate `hex_encode()` | compat.rs + crashlog.rs | 15 min |
| 8 | [x] Extract mutex lock helper | auth.rs:44,50,56 | 5 min |
| 9 | [x] Use first-byte dispatch in classify_unified_line | domain.rs:344-366 | 15 min |
| 10 | [x] Fix page cache double-allocation | domain.rs:270-285 | 10 min |

---

## Recommended Handover Plan

### Phase 0: Measure first

Add counters/benchmarks for:
- replacement-alignment cost in `file_diff.rs`
- full inline-text materialization count
- repo-monitor ignore lookup count and latency
- gitlink-status supplement frequency
- reducer clone time / dispatch latency
- rendering allocations per frame in diff_text and conflict_resolver

Tie new measurements into the existing Criterion and perf-budget infrastructure where it makes sense.

### Phase 1: Quick wins + low-risk improvements

Do these first (all independent, parallelizable):
- All items from the Quick Wins table above.
- Gate gitlink status supplementation so non-submodule repos do not pay for it.
- Replace arbitrary `HashMap` partial-eviction caches with a real LRU cache.
- Fix duplicate resvg dependency.
- Add dev/test build profiles.
- Stop using `side_by_side_rows*()` in helpers that only need anchors, masks, or modify-pair traversal.
- Remove `sort_unstable()` / `dedup()` from `LineIndex::apply_edit()` by constructing ordered output directly.

### Phase 2: Medium refactors

- Add plan-level iterators/helpers to `FileDiffPlan`.
- Migrate markdown preview and conflict highlighting to those plan-level APIs.
- Rework streamed diff page providers so they cache compact refs instead of owned strings.
- Refactor rendering hot paths in diff_text.rs to use buffers/caches instead of per-frame allocation.
- Cache conflict_choices and context menu IDs in conflict_resolver.
- Use Arc<str> for domain types (CommitId, Commit fields).
- Feature-gate tree-sitter language grammars.
- Extract large test/benchmark files to dedicated modules.

### Phase 3: Architectural bets

- Replace repo-monitor `git check-ignore` subprocess logic with an in-process matcher.
- Decide whether the custom text model remains worth the maintenance cost or whether a rope crate should replace it.
- Keep retiring `git` CLI fallbacks from hot paths where `gix` can cover the behavior.
- Split large files per the decomposition plan.
- Refactor compat argument parsing from if-chain to state machine.

## Recommended Order of Work

1. Quick wins (Phase 1 table) — get familiar with the codebase, build momentum.
2. `file_diff.rs` plus streamed diff provider cleanup.
3. Rendering hot path cleanup (diff_text + conflict_resolver).
4. Repo monitor ignore matching.
5. Cache-policy cleanup.
6. Domain type Arc<str> migration.
7. Build/dependency cleanup.
8. Text model and store architecture, but only after measurement.

## Questions to Answer Before Starting

- Which UI flows truly require full inline diff text instead of paged row access? I don't know, inline diff is used as toggle button functionality when diffing changes, but even that can use the streaming approach.
- How common are submodules/gitlinks in the repos you care about most? Submodules are rare
- Does repo monitoring need exact Git ignore semantics in all cases, or only enough fidelity to suppress noise? This is Git GUI so we should follow gitignore semantics
- Is the custom editor stack a strategic part of the product, or would a rope crate reduce more risk than it adds? Custom editor fits strategy, we may extend it later
- What's the target binary size? Is 41MB acceptable or should tree-sitter grammars be trimmed? 41MB is acceptable
