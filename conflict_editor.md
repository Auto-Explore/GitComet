# Conflict Editor Unification Plan

## Implementation Progress

- ✅ `0a. Unified line-ending detection` — Added shared `gitgpui-core::text_utils::detect_line_ending_from_texts` with explicit heuristics, switched focused merge and subchunk autosolve to use it, and documented why `merge.rs` keeps its existing full-file heuristic.
- ✅ `0b. Unified marker parsing` — Added shared `gitgpui_core::conflict_session::parse_conflict_marker_segments` + segment/block types, converted core region parsing and autosolve text parsing to thin wrappers over it, removed `MergedSpan`, and switched UI `parse_conflict_markers` to map from core segments.
- ✅ `0c. Marker-preserving output generation` — Added shared `gitgpui_core::conflict_output::{generate_resolved_text, render_unresolved_marker_block, detect_conflict_block_line_ending}` plus output options/labels, delegated UI `conflict_resolver::generate_resolved_text` and focused merge save rendering to core, and removed focused-merge-local output generator helpers.
- ✅ `0d. Consolidate auto-resolve on segments` — Added core `safe_auto_resolve_pick(base, ours, theirs, whitespace_normalize)` and delegated UI `auto_resolve_segments_with_options` to it, removing duplicated Pass-1 rule logic while keeping regex/history paths delegated to core helpers.
- ✅ `0e. Tests for consolidated primitives` — Added direct `gitgpui_core::conflict_output` tests for resolved/unresolved/mixed marker rendering across LF/CRLF/CR and missing-trailing-newline cases, plus parse→generate→parse round-trip coverage via `parse_conflict_marker_segments`; ran required workspace test/clippy commands (currently failing in pre-existing difftool integration and unrelated workspace clippy checks).
- 🔧 `1. Add Focused Mergetool View Mode to Existing UI` — Added `GitGpuiViewMode` + `GitGpuiViewConfig` in `gitgpui-ui-gpui::view`, introduced `GitGpuiView::new_with_config`, threaded view mode into `MainPaneView`, and implemented focused-mode chrome suppression in `GitGpuiView::render` (title + main pane only; hides repo tabs/open-repo panel/action bar/sidebar/details pane/resize handles). Focused-mode action/exit semantics are still pending.
- ⬜ `2. New GPUI Entrypoint for Mergetool Focused Window`
- ⬜ `3. Focused Mode State Bootstrapping`
- ⬜ `4. Save/Cancel/Exit Semantics`
- ⬜ `5. Remove Old Standalone Merge UI`

## Goal

Unify `git mergetool --gui` and in-app conflict resolution so both use the same `gitgpui-app` conflict resolver surface, theme, and settings. Consolidate all duplicate marker parsing, output generation, auto-resolve, and line-ending detection into single implementations. Remove the standalone focused merge after parity.

## Current State

- CLI mergetool flow currently branches into a separate focused UI:
  - `crates/gitgpui-app/src/main.rs` (`should_launch_focused_merge_gui`, `run_focused_merge` call)
  - `crates/gitgpui-ui-gpui/src/focused_merge.rs`
- Main app has a separate, richer resolver pipeline:
  - `crates/gitgpui-ui-gpui/src/view/panes/main.rs` (`sync_conflict_resolver`)
  - `crates/gitgpui-ui-gpui/src/view/panels/main.rs` conflict toolbar/actions
- Full app chrome currently always renders tabs/sidebar/details:
  - `crates/gitgpui-ui-gpui/src/view/mod.rs`

## Non-Negotiable Requirements

- `git mergetool` opens a separate external window.
- Use same app theme/settings path as normal `gitgpui-app`.
- Open directly in conflict resolution for target file.
- Focused mergetool window must not show sidebar, repo tabs, repo picker/file selection chrome.
- Preserve mergetool exit contract:
  - `0` resolved/success
  - `1` canceled or unresolved
  - `>=2` operational error

## Duplication Inventory

The following parallel implementations exist and must be consolidated:

### Marker Parsing (3 implementations)

| Function | Location | Returns | Context preserved? |
|----------|----------|---------|-------------------|
| `parse_conflict_markers()` | `conflict_resolver.rs:233` | `Vec<ConflictSegment>` | Yes (Text segments) |
| `parse_conflict_regions_from_markers()` | `conflict_session.rs:683` | `Vec<ConflictRegion>` | No (regions only) |
| `parse_merged_spans()` | `conflict_session.rs:761` | `Vec<MergedSpan>` | Yes (Context spans) |

All three use the same `split_inclusive('\n')` + marker-detection loop. `parse_conflict_markers` and `parse_merged_spans` both preserve context and handle malformed markers identically. `parse_conflict_regions_from_markers` discards context.

### Output Generation (2 implementations)

| Function | Location | Unresolved marker support? |
|----------|----------|---------------------------|
| `generate_resolved_text()` | `conflict_resolver.rs:960` | No — collapses to chosen side |
| `build_output()` → `block_output_text()` → `render_unresolved_marker_block()` | `focused_merge.rs:206-306` | Yes — regenerates markers with labels |

### Auto-Resolve (2 parallel paths)

| Function | Location | Operates on |
|----------|----------|-------------|
| `auto_resolve_segments_with_options()` | `conflict_resolver.rs:727` | `ConflictSegment`/`ConflictBlock` |
| `safe_auto_resolve()` | `conflict_session.rs:988` | `ConflictRegion` |
| `try_resolve_single_block()` | `conflict_session.rs:870` | raw strings (chains passes 1-4) |

The UI-layer `auto_resolve_segments_*` functions duplicate core's `safe_auto_resolve` logic — same rules (identical sides, single-side change, whitespace-only) applied to a different struct type.

### Line-Ending Detection (3 implementations)

| Function | Location | Scope |
|----------|----------|-------|
| `detect_line_ending()` | `focused_merge.rs:308` | Per-block (ours/theirs/base) |
| `detect_line_ending()` | `merge.rs:761` | Across all 3 full-file sides |
| `detect_subchunk_line_ending()` | `conflict_session.rs:1482` | Subchunk text slices |

### Data Types (3 parallel representations)

| Type | Location | Purpose |
|------|----------|---------|
| `ConflictSegment` / `ConflictBlock` | `conflict_resolver.rs:40-55` | UI model (choice + resolved flag) |
| `ConflictRegion` | `conflict_session.rs:238` | Core session model (resolution enum) |
| `MergedSpan` | `conflict_session.rs:748` | Standalone autosolve (private enum) |

`ConflictBlock` and `ConflictRegion` carry the same data (base/ours/theirs) with different resolution tracking. `MergedSpan` is a private analog of `ConflictSegment`.

## Implementation Phases

### Phase 0: Consolidate Core Primitives

Goal: single implementations in `gitgpui-core` for marker parsing, output generation, line-ending detection, and auto-resolve on the shared data types. No UI changes yet.

#### 0a. Unified line-ending detection

1. Move `detect_line_ending()` from `focused_merge.rs:308` into `gitgpui-core` (e.g. `text_utils` or `conflict_session`).
2. Generalize the signature to accept an iterator of string slices so it works for per-block, full-file, and subchunk contexts.
3. Replace `detect_subchunk_line_ending()` in `conflict_session.rs:1482` with a call to the unified function.
4. `merge.rs:detect_line_ending()` can remain as-is if its counting-based heuristic intentionally differs, but document why. Otherwise unify.

#### 0b. Unified marker parsing

1. Unify `parse_conflict_markers()` (conflict_resolver.rs) and `parse_merged_spans()` (conflict_session.rs) into a single core function.
   - Place in `gitgpui-core::conflict_session` (or a new `gitgpui-core::conflict_markers` module).
   - Return a type equivalent to `Vec<ConflictSegment>` — alternating context text and conflict blocks with base/ours/theirs.
   - Must preserve context (like both current implementations) and handle malformed markers robustly.
2. `parse_conflict_regions_from_markers()` becomes a thin wrapper that discards context from the unified parser output (or is replaced by callers filtering the unified output).
3. Delete `MergedSpan` enum — it is superseded by the unified segment type.
4. Update all call sites:
   - `conflict_resolver.rs` `parse_conflict_markers` → call core parser, map to UI type if needed.
   - `conflict_session.rs` `parse_merged_spans` → call core parser.
   - `conflict_session.rs` `parse_regions_from_merged_text` → call core parser, extract regions.

#### 0c. Marker-preserving output generation

1. Add `render_unresolved_marker_block()` and `detect_line_ending()` (the per-block variant) to core, ported from `focused_merge.rs:271-330`.
2. Create a single `generate_resolved_text()` in core that:
   - For resolved blocks: emits the chosen side text (current behavior).
   - For unresolved blocks: calls `render_unresolved_marker_block()` to regenerate markers with labels.
   - Accepts labels (local/remote/base) as parameters.
   - When called without labels (or with a "collapse" flag), falls back to current behavior (pick chosen side for unresolved too) for backward compatibility with the main app's live preview.
3. Delete `focused_merge.rs` functions: `build_output()`, `block_output_text()`, `chosen_block_text()`, `render_unresolved_marker_block()`, `detect_line_ending()`.
4. Update `conflict_resolver.rs:generate_resolved_text()` to delegate to the core function.

#### 0d. Consolidate auto-resolve on segments

The UI-layer `auto_resolve_segments_with_options()` duplicates core's `safe_auto_resolve()` rules operating on a different struct.

1. Make the core `safe_auto_resolve()` accept a trait or generic struct so it can operate on both `ConflictRegion` and `ConflictBlock`, OR:
2. Simpler: have `auto_resolve_segments_with_options()` delegate to `safe_auto_resolve()` per block by constructing a temporary `ConflictRegion` and mapping the result back. This removes the duplicated rule logic while keeping the segment-iteration wrapper in the UI layer.
3. Same approach for `auto_resolve_segments_regex()` → delegate to `regex_assisted_auto_resolve_pick()` (already partially done — verify no logic duplication remains).
4. Same for `auto_resolve_segments_history()` → delegate to `history_merge_region()`.

#### 0e. Tests for consolidated primitives

1. Port focused_merge.rs output tests (LF/CRLF/CR, resolved/unresolved/mixed) to test the new core functions directly.
2. Ensure `try_autosolve_merged_text()` still works (it uses `parse_merged_spans` internally — must use unified parser).
3. Add round-trip test: parse markers → generate output with unresolved markers → re-parse → same structure.
4. Run full workspace tests to catch regressions.

### Phase 1: Add Focused Mergetool View Mode to Existing UI

1. Add a view mode enum/config in `GitGpuiView` (e.g. `Normal`, `FocusedMergetool`).
2. Thread mode into `MainPaneView` and panels.
3. In focused mode render:
   - title bar + main pane only
   - hide repo tabs bar, open-repo panel, action bar, sidebar, details pane, pane resize handles
4. In focused mode disable irrelevant close behavior (`ClearDiffSelection` should exit/cancel instead of returning to file list).
5. Hide external-mergetool buttons inside conflict panels in focused mode:
   - binary conflict panel
   - keep/delete panel
   - decision-only panel
   - main conflict toolbar
6. Keep only actions relevant to resolving and exiting the focused window.

### Phase 2: New GPUI Entrypoint for Mergetool Focused Window

1. Add a new public entrypoint in `gitgpui-ui-gpui` (parallel to `run`), using same app bootstrap:
   - same assets, keybindings, window session size restore
   - same theme/appearance wiring
2. Input config should include:
   - repo/workdir path
   - conflicted file path
   - labels (local/remote/base) for marker rendering
3. Use a dedicated app/window title and focused window options, but keep shared style/theme pipeline.

### Phase 3: Focused Mode State Bootstrapping

1. Create store with real backend.
2. Open repo path.
3. Set active diff target to `DiffTarget::WorkingTree { area: Unstaged, path }`.
4. Trigger conflict file load through existing `Msg::LoadConflictFile`.
5. Ensure resolver appears without requiring sidebar/file click interaction.

### Phase 4: Save/Cancel/Exit Semantics

1. Add focused-mode save action in shared resolver:
   - write merged output file
   - use the consolidated marker-preserving output generation from Phase 0c
   - compute unresolved status from block resolved flags
2. Exit rules:
   - Save clean/resolved → `0`
   - Save unresolved → `1`
   - Escape/window close → `1`
   - write failure → `2`
3. Keep autosolve + A/B/C/D + F2/F3 behavior.

### Phase 5: Remove Old Standalone Merge UI

1. Rewire `gitgpui-app` mergetool GUI path to new unified entrypoint.
2. Remove:
   - `focused_merge.rs`
   - exports from `gitgpui-ui-gpui/src/lib.rs` (`FocusedMergeConfig`, `run_focused_merge`)
   - related focused-merge-specific launch/config code in `gitgpui-app/src/main.rs`
3. Keep headless mergetool core (`mergetool_mode.rs`) intact.

## State/Session Safety

- Keep UI settings persistence enabled (theme/window/conflict settings) to satisfy shared look/behavior.
- Avoid accidental repo session pollution from focused mergetool runs. Specific guards needed:
  - **Repo tab persistence**: do not add mergetool-opened repo to saved tab list.
  - **Conflict session persistence**: focused mergetool session is ephemeral — do not persist to disk.
  - **Window session**: window size/position can persist (harmless and useful).
- Thread the `ViewMode` into session save/load paths and skip repo-level persistence when `FocusedMergetool`.

## Test Plan

### Phase 0: Core consolidation

- Port all focused_merge.rs output tests to core (resolved, unresolved, mixed, LF/CRLF/CR).
- Round-trip test: parse → generate with unresolved markers → re-parse → structural equality.
- Verify `try_autosolve_merged_text()` produces identical output after parser unification.
- Verify `auto_resolve_segments_with_options()` produces identical results when delegating to core rules.
- Run full workspace test suite (`cargo test --workspace --no-default-features --features gix`).

### App / Routing

- Update mergetool GUI launch tests in `crates/gitgpui-app/src/main.rs` tests.
- Verify unresolved text conflict launches unified focused mode.
- Verify clean merge does not launch GUI.

### Integration

- Keep existing git tool-selection semantics:
  - `merge.tool`, `merge.guitool`, `mergetool.guiDefault`, `--gui/--no-gui`
  - existing coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`

### Resolver Semantics

- Add focused save exit-code tests (`0/1/2`) with unresolved/resolved/cancel/write-error cases.
- Test that unresolved blocks produce valid conflict markers in output.

### State Tests

- Revisit conflict-session tests around `LaunchMergetool` side effects in:
  - `crates/gitgpui-state/src/store/tests/conflict_session.rs`
- Adjust only if command semantics change.
- Add session-pollution guard tests: focused mergetool run does not modify repo tab list.

## Rollout Strategy

1. Land Phase 0 (core consolidation) — pure refactor, no behavior change.
2. Land Phase 1 + 2 (focused mode plumbing + entrypoint).
3. Land Phase 3 + 4 (state bootstrap + save/exit logic).
4. Switch CLI mergetool GUI path (Phase 5 step 1).
5. Remove legacy focused merge implementation (Phase 5 steps 2-3).
6. Run targeted tests first, then full workspace tests at each step.
