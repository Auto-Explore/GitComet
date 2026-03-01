# Conflict Resolver UX and Syntax Redesign

Status: Approved for implementation
Owner: UI + State teams
Scope: `gitgpui` merge/diff/conflict resolver flows

## Implementation Progress

### Phase 1: Syntax + Output Structure
- ✅ `view/rows/conflict_resolver.rs` — switched all 7 `DiffSyntaxMode::HeuristicOnly` call sites to `DiffSyntaxMode::Auto` with tree-sitter language resolution from file path. All render functions (three-way, two-way split, two-way inline, compare split, compare inline) now pass the resolved `DiffSyntaxLanguage` and use Auto mode. Cache population is now triggered by syntax language availability (not just word highlights/search).
- ✅ `view/panes/main.rs` — added resolved-output outline lifecycle with hash/path tracking and debounced recompute scheduling (140ms idle window via `resolver_pending_recompute_seq` cancellation token); outline line splitting now preserves trailing newline rows; resolver output editor soft-wrap disabled to keep one logical row per newline.
- ✅ `view/mod.rs` — all new resolver state types and fields added: `ResolvedLineSource` enum (A/B/C/Manual with badge_char), `ResolvedLineMeta` struct (output_line, source, input_line), `SourceLineKey` struct (view_mode, side, line_no, content_hash) with FxHasher, `ConflictResolverHoverState` (hovered_chunk, hovered_line). New fields in `ConflictResolverUiState`: `resolved_line_meta`, `resolved_output_line_sources_index`, `resolver_hover`. Types defined in `conflict_resolver.rs` and imported.
- ✅ `view/conflict_resolver.rs` — complete: output-outline line splitter (`split_output_lines_for_outline`), provenance mapping (`compute_resolved_line_provenance` with A>B>C priority matching via exact text equality), dedupe key builder (`build_resolved_output_line_sources_index`), plus-icon visibility checker (`is_source_line_in_output`), `SourceLines` source data struct. 10 new unit tests covering provenance classification, priority ordering, empty cases, dedupe index, badge chars.
- ✅ `view/panes/main.rs` — provenance recompute wired into debounced outline lifecycle; two-way source lines extracted from `diff_rows` via `collect_two_way_source_lines`; three-way uses existing `three_way_{base,ours,theirs}_lines`; invalidation path clears `resolved_line_meta` and `resolved_output_line_sources_index`.

### Phase 2: Input Picking UX
- ✅ `view/rows/conflict_resolver.rs` — green chunk hover/active affordance for three-way chunks; two-way split and inline rows now use immediate line pick (click appends line to output) instead of selection toggle; green hover/active style applied consistently; plus icon shown on row hover when line is pickable and not already in resolved output (dedupe gated via `is_source_line_in_output`); `split_cell_bg` and `inline_row_bg` simplified (removed `selected` parameter).
- ✅ `view/panes/main.rs` — added 3 immediate-pick methods: `conflict_resolver_append_split_line_to_output`, `conflict_resolver_append_inline_line_to_output`, `conflict_resolver_append_three_way_line_to_output`; removed selection state fields (`split_selected`, `inline_selected`), toggle methods, `conflict_resolver_selection_is_empty`, `conflict_resolver_clear_selection`, `conflict_resolver_append_selection_to_output`; removed selection retention in visible map rebuild; removed selection clear from mode/view switching.
- ✅ `view/panels/main.rs` + `view/panels/popover.rs` — removed "Append selection" and "Clear selection" buttons and their closures from both main panel and popover layouts.
- ✅ `view/conflict_resolver.rs` — removed unused `collect_split_selection` and `collect_inline_selection` helpers.
- 🔧 Remaining: right-click row context menu for "Select line (N)" / "Select chunk (Ln X-Y)" actions; three-way row plus icon (method exists but not yet wired to UI).

### Phase 3: Resolved Output Context Menu + Bar Cleanup
- ⬜ `view/panels/main.rs` — remove duplicate nav controls from resolved output bar
- ⬜ `view/panels/mod.rs` — new resolver context actions
- ⬜ `view/panels/popover/context_menu.rs` — action handling dispatch
- ⬜ `view/panels/popover/context_menu/` — resolver output menu model

### Phase 4: Image/Markdown Preview Integration
- ⬜ Reuse existing preview/image plumbing for resolver-supported preview modes
- ⬜ Add resolver-mode preview toggles and conditional rendering

## 1. Goals

This document defines the implementation for the conflict resolver redesign with these user-approved constraints:

1. All source-code views must support syntax highlighting for supported languages.
2. Syntax highlighting must use tree-sitter only (no heuristic fallback in resolver flows).
3. File content rows are syntax highlighted; non-content chrome is not.
4. Resolved output must stay editable and also expose a highlighted line outline with source annotations (`A`, `B`, `C`, `M`).
5. Resolved output updates are debounced/idle-driven for expensive recompute paths.
6. Resolved output must show line structure with visible per-line rows (line numbers and source labels), always.
7. The missing-line-break behavior in resolved output is treated as always reproducible and must be fixed.
8. Resolved output must show whether each line came from merge input `A`, `B`, `C`, or `Manual`.
9. Manual source label is `M` in compact form and `Manual` in expanded text.
10. Any line not identical to source content from `A`/`B`/`C` is labeled `Manual`.
11. Left click keeps chunk selection behavior; hovered chunk gets green outline to indicate click target.
12. Row-level pick is immediate (no append button workflow).
13. Context menu line/chunk labels must use the same line numbers shown in merge input gutters.
14. Line-level pick must work in both 3-way and 2-way modes.
15. Row plus-icon is hidden when that source row is already present in resolved output to prevent duplicate insertion.
16. Remove duplicate conflict navigation controls (`#1 #2 ...`, `Prev`, `Next`) from resolved output bar only; keep keyboard workflow (`F1`-`F4`) unchanged.
17. Resolved output context menu includes `Copy`, `Cut`, `Paste`, `Undo`, `Redo`, `Pick Line A`, `Pick Line B`, `Pick Line C`.
18. Resolved output context menu must include both text-edit actions and conflict-aware actions.

## 2. Current State (Code Map)

Primary files currently involved:

1. Resolver panel layout and action bars:
   - `crates/gitgpui-ui-gpui/src/view/panels/main.rs`
2. Resolver row rendering:
   - `crates/gitgpui-ui-gpui/src/view/rows/conflict_resolver.rs`
3. Resolver core helpers (segment parsing, line mapping, selection collection):
   - `crates/gitgpui-ui-gpui/src/view/conflict_resolver.rs`
4. Resolver UI state:
   - `crates/gitgpui-ui-gpui/src/view/mod.rs`
5. Diff syntax pipeline:
   - `crates/gitgpui-ui-gpui/src/view/rows/diff_text/syntax.rs`
   - `crates/gitgpui-ui-gpui/src/view/rows/diff_text.rs`
6. Context menu plumbing:
   - `crates/gitgpui-ui-gpui/src/view/panels/mod.rs`
   - `crates/gitgpui-ui-gpui/src/view/panels/popover/context_menu.rs`
7. Text input widget:
   - `crates/gitgpui-ui-gpui/src/kit/text_input.rs`

Key known issues in current implementation:

1. Conflict rows mostly use `DiffSyntaxMode::HeuristicOnly`.
2. Resolved output is a plain `TextInput` without syntax-highlighted row rendering.
3. 2-way line/chunk picking is selection + append-button based, not immediate.
4. Resolved-output action bar duplicates conflict navigation controls.
5. No resolver-specific right-click context menu for row pick + text-edit combo actions.

## 3. UX Specification

## 3.1 Merge Input Panes (Top)

Applies to 3-way and 2-way views.

1. Syntax highlight source content rows using tree-sitter.
2. Hovering a selectable chunk shows a green outline with light fill.
3. Left click on chunk selects that chunk as today (unchanged behavior).
4. Hovering a row shows a left-edge plus icon for line pick, only if eligible (see dedupe rule).
5. Right click on row opens context menu:
   - `Select line (N)`
   - `Select chunk (Ln X - Y)`
6. Row-level pick executes immediately (no append button).

## 3.2 Resolved Output Pane (Bottom)

1. Resolved output remains editable with keyboard editing and selection behavior.
2. Resolved pane includes synchronized per-line outline:
   - line number gutter
   - source badge: `A`, `B`, `C`, `M`
   - syntax-highlighted content row
3. Source badge rules:
   - Exact match to source line from `A`, `B`, or `C` -> respective badge
   - Else -> `M` (`Manual`)
4. Right click context menu in resolved output:
   - `Copy`
   - `Cut`
   - `Paste`
   - `Undo`
   - `Redo`
   - `Pick Line A`
   - `Pick Line B`
   - `Pick Line C`
5. Remove duplicate conflict navigation controls from resolved-output bar:
   - remove queue buttons (`#1 #2 #3 ...`)
   - remove `Prev` / `Next` buttons in that bar
6. Keep conflict keybindings unchanged (`F1`-`F4` behavior remains).

## 3.3 Preview Behavior for Image and Markdown

1. Image files:
   - keep image preview path for visual content views (existing image flow is reused).
2. Markdown files:
   - support markdown content preview mode in resolver context (text view with syntax + optional rendered preview mode).
3. Non-text/binary conflicts:
   - continue using binary strategy views; no fake text editor for binary payload.

## 4. Data Model and State Changes

Add the following state to `ConflictResolverUiState` in `view/mod.rs`:

1. `resolved_line_meta: Vec<ResolvedLineMeta>`
2. `resolved_outline_lines: Vec<SharedString>`
3. `resolved_outline_syntax_language: Option<rows::DiffSyntaxLanguage>`
4. `resolved_outline_cache: HashMap<usize, CachedDiffStyledText>`
5. `resolved_output_line_sources_index: HashSet<SourceLineKey>` for dedupe/plus-icon gating
6. `resolver_hover: ConflictResolverHoverState` for chunk/row hover visuals
7. `resolver_pending_recompute_seq: u64` for debounce cancellation

New types:

1. `enum ResolvedLineSource { A, B, C, Manual }`
2. `struct ResolvedLineMeta { output_line: u32, source: ResolvedLineSource, input_line: Option<u32> }`
3. `enum ResolverPickTarget { Chunk { ... }, Line { ... } }`
4. `struct SourceLineKey { view_mode, side, line_no, content_hash }`
5. `struct ConflictResolverHoverState { hovered_chunk: Option<...>, hovered_line: Option<...> }`

## 5. Interaction Logic

## 5.1 Immediate Line/Chunk Pick

Replace "select + append" model with direct append:

1. 2-way split row pick:
   - append chosen side line immediately
   - record `SourceLineKey` and line origin metadata
2. 2-way inline row pick:
   - append row line immediately
   - record metadata
3. 3-way row pick:
   - `A/B/C` pick at line level appends that row immediately (context action / plus icon)
4. Chunk pick:
   - left click behavior remains chunk-level and immediate as existing resolver pick

Implementation:

1. Remove usage of:
   - `split_selected`
   - `inline_selected`
   - `conflict_resolver_append_selection_to_output`
2. Remove UI controls:
   - `Append selection`
   - `Clear selection`
3. Add direct methods:
   - `conflict_resolver_append_split_line_to_output(...)`
   - `conflict_resolver_append_inline_line_to_output(...)`
   - `conflict_resolver_append_three_way_line_to_output(...)`
   - `conflict_resolver_append_chunk_to_output(...)`

## 5.2 Dedupe Rule for Plus Icon

Plus icon visibility condition:

1. A row is pickable only when its `SourceLineKey` is not currently represented in resolved output metadata.
2. On resolved-output recompute, rebuild key index from metadata + text.
3. If a line is manually edited and no longer equal to original source content, key is dropped; plus icon becomes available again.

## 5.3 Provenance and Manual Labeling

Provenance recompute pipeline:

1. Start with event-driven metadata from known pick operations.
2. On debounced output change:
   - split output into lines
   - compare each line against source mappings (`A/B/C`) using exact text equality and displayed line-number context
   - if no exact source match -> `Manual`
3. Update outline rows and source badges.

## 6. Syntax Highlighting Strategy

## 6.1 Rule

Resolver flows must use tree-sitter parsing for syntax highlighting. No heuristic fallback path in resolver views.

## 6.2 Implementation Updates

1. In resolver row rendering (`rows/conflict_resolver.rs`):
   - replace `DiffSyntaxMode::HeuristicOnly` with `DiffSyntaxMode::Auto`
   - ensure language resolution comes from file path for all content rows
2. In syntax layer (`rows/diff_text/syntax.rs`):
   - tree-sitter becomes the primary tokenizer for supported programming languages
   - heuristic tokenizer path is disabled for resolver flows
3. Grammar coverage:
   - every language advertised in resolver-supported list must have tree-sitter grammar/query coverage
   - languages lacking grammar support are removed from resolver-supported list until grammar is added

## 6.3 Performance

1. Use line-level caches keyed by `(line_hash, language, theme_version)`.
2. Debounce expensive recompute (100-200ms idle window).
3. Do not block keystrokes; updates are async and cancelable via sequence token.

## 7. Resolved Output Rendering Model

Bottom pane changes from "single plain text area" to synchronized dual representation:

1. Editable source of truth:
   - existing `conflict_resolver_input` (`TextInput`) remains editable
2. Visual outline:
   - virtualized row list with line numbers, source badges, syntax highlighting
   - line numbers match merge input numbering conventions used in context menu labels

Scrolling:

1. Bottom pane keeps one shared scroll container for editor + outline alignment.
2. Soft-wrap disabled in resolved output (`soft_wrap = false`) to keep one logical row per newline.

## 8. Context Menu Design

## 8.1 Merge Input Rows

New context menu model (resolver-specific):

1. `Select line (N)`
2. `Select chunk (Ln X - Y)`

## 8.2 Resolved Output

Resolver output context menu actions:

1. `Copy`
2. `Cut`
3. `Paste`
4. `Undo`
5. `Redo`
6. `Pick Line A`
7. `Pick Line B`
8. `Pick Line C`

Code changes:

1. Extend `ContextMenuAction` enum in `view/panels/mod.rs` with resolver actions.
2. Add reducer dispatch and local handlers in `view/panels/popover/context_menu.rs`.
3. Add resolver context menu model builder under `view/panels/popover/context_menu/`.

## 9. Action Bar Changes

In resolved output header (`view/panels/main.rs`):

1. Remove:
   - unresolved queue `#...` buttons
   - `Prev`
   - `Next`
2. Keep:
   - `A/B/C/BC` picks
   - bulk unresolved picks
   - auto-resolve controls
   - reset/hide-resolved controls
   - save/save-stage controls
3. Keep keyboard bindings (`F1`-`F4`) unchanged.

## 10. File-Level Implementation Plan

## Phase 1: Syntax + Output Structure

1. `view/rows/conflict_resolver.rs`
   - switch resolver syntax mode to tree-sitter auto path
2. `view/panes/main.rs`
   - add resolved-output outline data lifecycle
   - add debounce recompute scheduling
3. `view/mod.rs`
   - add new resolver state fields
4. `view/conflict_resolver.rs`
   - add provenance mapping utilities and dedupe key builders

## Phase 2: Input Picking UX

1. `view/rows/conflict_resolver.rs`
   - add hover chunk outline (green)
   - add row hover plus icon
   - left-click chunk unchanged
   - row right-click menu and immediate pick actions
2. `view/panes/main.rs`
   - remove append/clear selection controls
   - remove old selection state behavior

## Phase 3: Resolved Output Context Menu + Bar Cleanup

1. `view/panels/main.rs`
   - remove duplicate nav controls from resolved output bar
2. `view/panels/mod.rs`
   - new resolver context actions
3. `view/panels/popover/context_menu.rs`
   - action handling dispatch
4. `view/panels/popover/context_menu/`
   - resolver output menu model

## Phase 4: Image/Markdown Preview Integration

1. Reuse existing preview/image plumbing for resolver-supported preview modes.
2. Add resolver-mode preview toggles and conditional rendering in `view/panels/main.rs`.

## 11. Testing Plan

Unit tests:

1. `view/conflict_resolver.rs`
   - provenance classification (`A/B/C/M`)
   - dedupe key visibility rules
   - immediate append behavior
2. `view/rows/diff_text/syntax.rs`
   - resolver syntax mode uses tree-sitter path

UI/state integration tests:

1. `gitgpui-state` conflict session tests remain green.
2. New tests for resolver context actions and output bar controls.

Manual QA checklist:

1. 3-way and 2-way source rows highlight syntax.
2. Hover chunk shows green target outline.
3. Right-click row menu shows `Select line` and `Select chunk`.
4. Plus icon appears only when row is not yet represented in output.
5. Resolved output displays line numbers and `A/B/C/M` source badges.
6. Editing output line converts badge to `M` when not matching source lines.
7. Resolved output right-click supports edit actions + pick-line actions.
8. `#...` and `Prev/Next` are removed from resolved output bar.
9. `F1`-`F4` behavior remains functional.

## 12. Acceptance Criteria

Implementation is complete when all items below are true:

1. Syntax highlighting in resolver source views is tree-sitter based.
2. Resolved output is editable and has synchronized highlighted line outline with source badges.
3. Line/chunk picks are immediate; append-button workflow is removed.
4. Hover and context menu behaviors match approved UX.
5. Duplicate row insertion guard for plus icon is active.
6. Duplicate conflict navigation controls are removed from resolved output bar only.
7. Resolved output context menu includes both text-edit and conflict-aware actions.
8. Image/markdown preview paths are available in resolver content views.

## 13. Notes for Execution

1. Keep changes incremental by phase to reduce UI regressions.
2. Do not change conflict session semantics in `gitgpui-core` unless required for provenance mapping.
3. Keep keyboard-first workflows intact while adding mouse-first UX improvements.
