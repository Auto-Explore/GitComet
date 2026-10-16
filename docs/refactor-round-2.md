# Refactor round 2: smaller units, narrower root, reusable kit

Planned 2026-10-15 on `refactor` after the merge of `dev` (#618). This records
the approved program, its baseline, and the rules each slice follows. Progress
is tracked by `scripts/ci/ratchets.py` and the per-stage notes added below as
slices land.

## Context

The `refactor` branch (47 commits ahead of `dev`) completed the first architecture
round recorded in `docs/rust-refactor.md`: canonical `GitRepository` families,
typed effect envelopes, request identities, one `DocumentOwner` per window host, a
root-independent `DiffRenderer`, and `UiLaunch`. Those slices fixed ownership and
protocol bugs. They did not change the *shape* of the two largest crates, and that
shape is now the dominant cost:

- `gitcomet-ui-gpui` is one crate: 735 files, ~298k production + ~226k test lines.
  It compiles as a single unit (lib 34–85 s, test harness 25–49 s on this machine,
  from `target/cargo-timings/`) and links ~75 tree-sitter grammars, so any UI edit
  recompiles everything. Its 4,048 tests run as one libtest process (89 s on Linux
  CI; 373–587 s on the macOS x64 runner against a 600 s cap).
- Its root view `GitCometView` (`crates/gitcomet-ui-gpui/src/view/mod_helpers/mod.rs:1025-1255`)
  has 173 `pub(super)` fields and 24 child entities. `impl GitCometView` blocks
  are spread over 23 files beyond the root's own; 384 production files begin with `use super::*;`, so
  `view/mod.rs` (875 lines of imports, constants and helpers) is a glob hub that
  couples every module to every other. Every UI test builds the whole root
  (1,268 `GitCometView::new*` calls in 144 files; 1,438 `.diff_renderer` reads in test files).
- `gitcomet-ui-kit` is reusable in principle (no state/host dependency, enforced by
  `scripts/ci/boundaries.py`) but depends on `gitcomet-core` (40 refs in 8 files),
  has no colour tokens (240 direct `theme.colors` reads inside components), no shared
  builder conventions, and ~20k lines of kit-shaped code still live in the host.
- `gitcomet-state`: one 297-arm `reduce_inner`, 74-arm `schedule_effect`, a 222-variant
  `Msg`, 201 raw `repos.iter_mut().find` sites; `AppState.repos` is `Vec<RepoState>`
  so every dispatch deep-clones every open repo. Its repository-monitor tests are
  274 s of the crate's 282 s cumulative CI test time (3 s quiet window per assertion).
- Build and test invocations share no artifacts (`profile.test` differs from `dev`
  in opt-level and debuginfo for every crate; `target/debug` is 32 GB).

Goal: smaller compilation units, a root view that owns only shell concerns, a kit a
second product can build on, feature-owned state, and tests that run in more
processes. The round-1 contract holds with two amendments recorded in the Decisions
section: the crate *graph* changes (new crates, one split) while its layering direction
is preserved, and tests move with the code they test. Everything else stands: GPUI, gix,
MVU snapshots, executor topology, user-visible behavior, persisted formats and
performance budgets; each slice migrates callers, tests and docs together and leaves
no forwarding shims. Every slice must move at least one number in the ratchet table.

## Baseline and ratchets

| Metric | Today | Target after round 2 | How measured |
|---|---|---|---|
| ui-gpui lib compile unit | 34–85 s | < 25 s (host shell only) | `cargo build --timings` |
| ui-gpui production lines | 298k | < 150k | `wc` over `src/` minus tests |
| GPUI test processes | 3 (ui, kit, example) | 7+ | `GPUI_PACKAGES` in `scripts/ci/run.py:33` |
| `GitCometView` fields | 173 | < 60 | `scripts/ci/ratchets.py` |
| `use super::*;` in production files (ui-gpui) | 384 | 0 | `scripts/ci/ratchets.py` |
| `impl GitCometView` blocks outside `gitcomet_view*.rs` | 23 files | 0 | `scripts/ci/ratchets.py` |
| `.set_theme(` calls in host | 130 | 0 | `scripts/ci/ratchets.py` |
| kit → `gitcomet-core` refs | 46 | 0 (rule in boundaries.py) | `scripts/ci/ratchets.py`, then boundaries.py |
| production `.rs` files > 3,000 lines | 19 | 0 (guard lowered to 3,000) | `scripts/check-rust-source-size.sh` |
| state `repos.iter_mut().find` sites | 213 | 0 | `scripts/ci/ratchets.py` |
| state test CPU (Linux CI, cumulative) | 282 s | < 60 s | nextest log |
| gix integration test binaries | 32 | 1 | `scripts/ci/ratchets.py` |
| build→test rebuild of dependencies | full | none (shared artifacts) | `cargo build && cargo test --no-run` timings |

`scripts/ci/ratchets.py` records the static counts above in `scripts/ci/ratchets.txt`
and fails when one regresses or improves without being recorded (`--write`), the same
way the identity-literal guard works. Existing guards stay green at
every commit: `scripts/check-rust-source-size.sh`, `scripts/ci/boundaries.py`,
`scripts/ci/identity_literals.py`, `scripts/ci/inventory.py` for test moves, the three
Clippy contexts, strict rustdoc.

## Working rules for this round

- Land as independent PRs on `dev`, not one long-lived branch: ten feature worktrees are
  active (`git worktree list`), and a months-long branch would conflict with all of them.
  Merge `refactor` first.
- Every extraction is "move, don't rewrite": `git mv` plus import fixes, verified by
  `scripts/ci/inventory.py` parity and a clean `git diff -M` rename detection.
- Each new crate gets a rule in `scripts/ci/boundaries.py` the commit it appears, and a
  line in `CONTRIBUTING.md`'s workspace layout.
- Public kit/extension API breaks are allowed with migration notes in
  `docs/rust-refactor.md` (same as round 1). Downstream proof: `gitcomet-extension-example`
  must build and its 11 tests pass in the `example` context after every kit change.
- Do not reverse round-1 decisions: gix runtime test overrides stay behind
  `gitcomet-git-gix/test-support`; production builds exclude benchmark and test-support
  features; `view::menu_presentation` stays host policy.
- Avoid files touched by in-flight branches until they merge: `fix/ui_test_speed`
  (spinner gating, `lock_visual_test` removal) and `diff_view_improvements`
  (main-pane action bars). Stage 2.7 waits for both.
- Each correctness bug found on the way gets a failing test first, in its own commit
  (one is already known: `PopoverKind::AddRepoMenu` and `PopoverKind::ChangeTrackingSettings`
  both hash as bare tag `66u8` in `view/panels/popover/fingerprint.rs:629,697`).

## Stage 0: build and test topology (manifest-level, fastest payback)

0.1 **Share dependency artifacts between `cargo build` and `cargo test`.** In the root
`Cargo.toml`, give `[profile.dev.package."*"]` and `[profile.test.package."*"]` identical
settings (opt-level, `debug = "line-tables-only"`, `split-debuginfo`) and leave workspace
crates at their current levels (dev 0, test 1; the test regime's budgets are tuned to opt-level 1). The gpui-ce/gpui_ce_windows/taffy overrides deliberately differ (dev 2,
test 1, "retain their measured settings", `Cargo.toml:357`): measure the UI suite with
those three at opt-level 2 in the test profile and keep the divergence only if the suite
gets slower. Measure: `cargo build -p gitcomet` then `cargo test -p gitcomet-ui-gpui --no-run`
must rebuild only workspace crates and harnesses; `du -sh target/debug` before/after.

0.2 **Remove internal feature divergence where it is pure helpers.** New dev-only crate
`crates/gitcomet-test-support` holding `gitcomet-core/src/test_support/` (git fixtures,
`UnconfiguredRepository`) and one shared `run_git` replacing the ~30 near-identical helpers
in `gitcomet-git-gix/tests/**`. Then audit whether `gitcomet-ui-gpui`'s dev-dependency on
`gitcomet-core` still needs `benchmarks`; drop it if UI tests compile without it. Keep the
gix `test-support` runtime hook (52 lines, `util/process.rs:47-56`) feature-gated. The CI
`app` context (`--no-default-features --features gix`, built under `profile.ci-test`)
rebuilds the gix stack because third-party features differ between contexts (55 s Linux,
2m42 Windows today); that is a separate problem from 0.1, so evaluate `cargo-hakari` for
it and decide by the measured rebuild.

0.3 **Move `perf_budget_report` out of the UI crate.** `crates/gitcomet-ui-gpui/src/bin/perf_budget_report/`
(10.4k lines, 111 tests) depends only on `perf_sidecar.rs` (347 lines, serde-only). Create
`crates/gitcomet-perf-sidecar` (the module) and `crates/gitcomet-perf-report` (the binary and
its tests). `scripts/profiling/` and `.github/workflows/perf.yml` paths update with it.
Effect: every UI test build stops linking a second full UI binary.

0.4 **Build scripts.** `crates/gitcomet-ui-kit/build.rs:54` reruns on the root manifest; read
the gpui revision from a dedicated `crates/gitcomet-ui-kit/gpui-revision` file instead.
`crates/gitcomet-ui-gpui/build.rs` rasterizes two splash SVGs with `resvg` as a
build-dependency (a second host-side compile of the resvg/usvg/tiny-skia stack); rasterize
at first use with the runtime `resvg` the host already links, cached in `assets.rs`, and
prove startup is unchanged with the `perf-app-launch` harness
(`crates/gitcomet/src/bin/perf_app_launch.rs`) before and after.

0.5 **Test topology.**
- gix: merge the 32 `tests/*.rs` binaries into `tests/integration/main.rs` with one module
  per former file (nextest still isolates per test). `inventory.py` comparison must map the
  module-prefix rename.
- state: extract `store/repo_monitor/` (3.0k prod + 7.3k tests) into
  `crates/gitcomet-repo-monitor` behind a sink trait (`RepoId`, `ChangedPaths`,
  `RepoExternalChange`, `RepoWatchDegradedReason`, three `Msg` variants, diagnostics; see
  `repo_monitor.rs:1-22`, `monitor.rs:644-704`). Then shorten the quiet waits without
  collapsing the three contracts `test_sync.rs:1-2` keeps distinct (delivered work, native
  checkpoints, quiet observation): bound each quiet observation by a native sync cookie
  (`test_sync.rs:111-132`) so that once the cookie is seen only the monitor's own debounce
  (250 ms) remains to observe, instead of the fixed 3 s `QUIET_WINDOW` (`test_sync.rs:8`);
  keep the full window only for tests that assert against OS behaviour the cookie cannot
  order. Spike first: convert the 8 slowest tests (10–19 s each) and measure on Linux,
  macOS and Windows.
- UI: each GPUI crate extracted in Stage 2 is added to `GPUI_PACKAGES` in
  `scripts/ci/run.py:33`; gpui-free crates run under nextest per test. This is what
  gets the macOS suite under its cap without touching individual tests.

## Stage 1: kit foundation (reuse)

1.1 **Cut `gitcomet-ui-kit → gitcomet-core`** (~200 lines, 8 files). Product name becomes
`kit::product::set_display_name` (host sets it at launch like `set_user_themes_dir`,
`theme.rs:1679`); `ui_probe`'s `op_trace`/`history_perf` bridge moves to the host behind
`kit::perf::set_recorder`; clipboard uses `gpui::FileTransferOperation` plus a kit id
counter and a host-installed diagnostics sink; `MinimapRowKind` → kit `MinimapBand`;
`TextSearchOptions` → kit `SearchToggles` with `From` in the host; `detect_is_wsl`/
`read_linux_osrelease` move down. Remove app policy from `ui_runtime` flags
(`uses_live_store_poller`, `uses_repo_tab_spinner_delay`, `auto_restores_session`,
`diff_syntax_foreground_parse_budget` become host-side policy fed in as values). Add
`gitcomet-core` to the kit's forbidden list in `boundaries.py:23`. Rename
`GitCometTooltipExt` and the `GITCOMET_UI_PROBE*` env names to product-neutral forms.

1.2 **Theme as a per-window global.** Mirror `WindowUiScales`/`UiScale::current`
(`ui_scale.rs:32-52,208`) with `WindowThemes` and `AppTheme::current(cx)`. Delete the 22
`set_theme` impls and 140 call sites in the host (`gitcomet_view.rs:2104-2163` fan-out).
Open the catalog: `ThemeCatalog::builder().bundled(..).defaults(dark, light)` so a product
registers its own bundled themes and defaults (`DEFAULT_DARK_THEME_KEY`, `theme.rs:14`);
add an `extensions` token map to the schema (22 `deny_unknown_fields` structs stay closed
for the core roles). Move `blame_*`/`historical_*` helpers (`theme.rs:2059-2146`) to the host.

1.3 **Component API normalization, one component at a time** (Button first: 233 host refs).
Per component: implement `RenderOnce`; `.on_click(handler)` becomes a normal builder step
(today `Button::on_click` is terminal, `button.rs:207-216`); shared `Tone` replaces
`ButtonStyle`/`ActionButtonStyle`/`SplitButtonStyle`/`ContextMenuTone`/`ToastKind`
variants where they mean the same; consistent `.id/.disabled/.tone/.tooltip`; per-component
colour tokens in `components/tokens.rs` replacing inline alphas (`button.rs:293-400`).
Merge `modal_surface`/`popover_surface` (`modal.rs:26-47`); make `panel`/`pill`
non-test (`components/mod.rs:46-47`); add Card, Notice (uses `NoticeColors`),
StatusBadge/status dot, DragPreview, and a selection indicator on `InteractiveRowStyle`;
replace the 13 inline rebuilds listed in the kit survey (e.g. `commit_message_hover.rs:487`,
`history_refs_hover.rs:612`, `home.rs:121`, `panels/main/diff_view.rs:1488`,
`command_palette.rs:1535` + `reveal_commit.rs:533`).

1.4 **Kit modules from host-independent code.** The *pieces* below were verified free of
state/root references; the files they come from are not, so each move cuts the file:
- `kit::window_chrome`: generic ~550 lines of `view/chrome.rs` (constants, controls cluster,
  traffic lights, `frame_insets`, `titlebar_control_button`, corner rounding, `resize_edge`,
  `window_frame`), `app/chrome.rs`, `window_controls.rs` (its one `session::UiSession` read
  at :73-95 becomes a mode value passed in), `window_root_hook.rs`, `app/placement.rs` over
  a kit-owned frame struct instead of `session::SavedWindowFrame`.
- `kit::HoverCardHost<T>`: the shared delay/grace/flip/clamp logic of
  `commit_message_hover.rs:324` and `history_refs_hover.rs:424`.
- `kit::ToastStack`: ids, TTL via `ui_runtime`, fade, corner, overflow, frame
  (~350 lines of `toast_host.rs`); the progress cards, `clone_progress` and `Msg` dispatch
  stay in the host and supply content as closures.
- `kit::MenuModel<A>` + `PickerNavigation`: the host menu model (`panels/mod.rs:646-720`,
  `context_menu.rs:169-266`), `menu_layer.rs` placement, and `picker_nav.rs` (232 lines)
  replacing the 12 `take_*_pressed()` polls with actions; `PickerSlot<P>` replacing the nine
  `ensure_*_search_input` functions (`panels/popover/search_inputs.rs:175-889`).
- `kit::text`: `date_time.rs` (696), `path_display.rs` (774), `tab_width.rs` (242),
  `shortcut_labels.rs` folded into `components/shortcut_keys.rs`; `word_diff.rs` → core
  (retarget the `view::word_diff::tests::` entry in `PURE_BATCHES`, `scripts/ci/run.py:44`).
  `word_diff` must land before 2.2, which uses it.
- `kit::QueryMatcher` and `kit::ResizeHandle`: replace the seven query matchers and six
  resize-handle renderers listed in the host survey.

1.5 **Kit test support for downstream crates.** Fonts via `gitcomet_fonts::BUNDLED_FONT_BYTES`
instead of `include_bytes!("../../gitcomet-fonts/…")` (`test_support.rs:25-30`); a themed
test-window fixture (theme + scale + runtime installed); a fake `WindowHostImpl` in
`gitcomet-extension-api` test-support so extension authors can test without the host.

## Stage 2: UI host crate splits (compile time and modularity)

Order is by (lines moved) / (cross-references to cut); each one removes a heavy dependency
from the host. All new crates: boundary rule, CONTRIBUTING line, `GPUI_PACKAGES` entry if
they use gpui.

2.0 **`gitcomet-text`** (gpui-free foundation for 2.1–2.3): the kit's `rope.rs` (843 lines,
built on `gpui::sum_tree`, `rope.rs:33`) rebased on the standalone `gpui_ce_sum_tree`
package; `SharedString` from `gpui_ce_shared_string`; `SyntaxTokenKind` and
`CachedDiffStyledText` (`view/diff_text_model.rs`, 65 lines); `LruTouchQueue`
(`view/rows/mod.rs:145`); `tab_width.rs`. The kit re-exports `rope` from it so kit callers
do not change. Boundary rule: `gitcomet-text` must not reach gpui-ce.

2.1 **`gitcomet-syntax`** (gpui-free): `view/rows/diff_text/syntax.rs` + `syntax/**`
(14.1k prod, 16.7k tests, 77 `.scm` queries, all 75 grammar dependencies, `tree-sitter`,
`gitcomet-tree-sitter-alloc`). Cuts: the 2.0 types; parse budget injected instead of
`ui_runtime::current()` (`syntax.rs:614`); `diff_utils::diff_text_display_len` moves with
it; the theme → `HighlightStyle` mapping in `syntax/live.rs:199,1202,1288` stays in the
host as a palette parameter. Prerequisite: replace `use super::super::*` in the syntax
tree with explicit imports (first use of the glob script in 3.1). Benchmarks in
`rows/benchmarks/syntax.rs` move with it under a `benchmarks` feature. Split
`syntax/heuristic.rs` (3,444) into detection tables and scorer on the way.

2.2 **`gitcomet-conflict-resolver`** (gpui-free, after 2.0 and the `word_diff` → core move
in 1.4): `view/conflict_resolver/` (8.3k prod, 7.6k tests). Cuts: `CachedDiffStyledText`
and `LruTouchQueue` (2.0), `word_diff::{WordDiffRanges, capped_word_diff_ranges,
compact_capped_word_diff_ranges}` (`word_highlight.rs:91,284,324`, `markers.rs:409`; from
core after 1.4), `gpui::SharedString` (`generated_text.rs:145`, `provenance.rs:409-411`;
from 2.0). Its `#[cfg(any(test, feature = "benchmarks"))]` oracles (~1.2k lines) go behind
a `test-support` feature. Public surface is the existing `pub use` list in
`conflict_resolver/mod.rs:9-20,211-219`; nine files use `use super::*;` over a `mod.rs`
with explicit imports, so the glob is bounded.

2.3 **`gitcomet-markdown`** (gpui-free, after 2.1): `view/markdown_preview/` model (5.1k prod,
5.0k tests, `pulldown-cmark`, `gitcomet-diagrams` for `DiagramKind`). Cuts:
`DiffSyntaxLanguage`/`diff_syntax_language_for_code_fence_info` behind a `CodeHighlighter`
trait; `scrollbar_markers_from_flags`, `ScrollbarMarker` and `ScrollbarMarkerKind` to the
kit (unifying the ~10 scrollbar-marker builders listed in the host survey);
`rows::markdown_preview_marker_label` (`anchors.rs:94`) moves with it; `SharedString` from 2.0.

2.4 **`gitcomet-ui-terminal`** (gpui + kit + core + `alacritty_terminal`): `terminal_alacritty.rs`
(1.7k + 1.4k inline tests → `tests.rs`), `terminal_preferences.rs`, `terminal_panel/{viewport,painting}.rs`,
and the terminal entity types from `mod_helpers/mode_impls.rs:941-1250`. Cuts: the
`TERMINAL_*` constants from `view/mod.rs` move in; `gitcomet_core::identity::current`
(`terminal_alacritty.rs:249,255`) and `kit::window_focus::is_active` (:1632,1678,1695) are
ordinary dependencies; `resolve_embedded_shell_program` becomes a parameter. Interface:
`TerminalView::new(TerminalSpawn{program,args,cwd,env,scrollback})` emitting
`TerminalEvent::{Title, Exited, Bell}`; the host keeps tabs, sessions and shutdown policy.
Resolve the duplicated cursor blink (root `terminal_panel.rs:114-163` vs
`viewport.rs:322-480`) by keeping only the viewport's.

2.5 **`gitcomet-ui-media`** (gpui + kit + `image`/`resvg`/`fast_image_resize`/`gitcomet-diagrams`):
`view/image_preview/**` minus `controls.rs`/`preferences.rs`, `diagram_preview/**`,
`rendered_preview.rs`. The three `impl DiffRenderer` blocks (`image_preview/controls.rs:13`,
`image_preview/preferences.rs:36`, `diagram_preview.rs:63`) move back to `panes/diff`; the
toolbar takes an `on_menu(anchor)` callback instead of `PopoverKind::TextFormatMenu`.

2.6 **`src/workspaces.rs`** (1.4k lines, zero `crate::` references) → `crates/gitcomet-ui-workspaces`
or into `gitcomet-ui-session` together with `session_ui.rs`; low priority, do when touched.

2.7 **`gitcomet-ui-diff`** (after 2.1–2.5 and the two in-flight branches): the `DiffRenderer`
cluster (`panes/diff/**`, `panels/main/**`, `rows/diff*`, `rows/conflict*`,
`rows/markdown_document.rs`, `rows/markdown_flow_text.rs`, `mod_helpers/conflict_resolver_ui_state.rs`;
~74k prod) plus `panels/tests` (83k, 53 of 69 files are DiffRenderer tests). The seam exists:
`panes/diff/services.rs` (`DiffServices`, `DiffCommand`, `DiffNavigation`) and
`hosted/diff_pane.rs:234` already builds a renderer without the root. Cuts: the 16
`PopoverKind` variants it names become a `DiffMenuRequest` carried by `DiffCommand::OpenPopover`;
pane-local types replace `GitCometViewMode`, `StatusSection`, `ChangeTrackingView`,
`ErrorReport`, `ToastKind` in its signatures; `view/mod.rs` diff-split helpers move; a
standalone `DiffRendererFixture` replaces the 1,312 `view.diff_renderer` test hits. This is
the slice that halves the host crate.

## Stage 3: root view decomposition (coupling)

3.1 **Kill the glob hub.** Script (`scripts/dev/expand_glob_imports.py`): for each production
file starting with `use super::*;`, drop the glob, run `cargo check --message-format=json`,
collect the unresolved names from E0412/E0425/E0433, write an explicit `use super::{…}` and
repeat until clean; commit per subtree. Then move the constants out of `view/mod.rs`
(`HISTORY_*` → `panes/history`, `PANE_*`/`SIDEBAR_MIN_PX` → `panels/layout`, `TOAST_*` →
toast host, `TERMINAL_*` → terminal) and the URL/product helpers to `app`. Ratchet:
`use super::*;` count in production files must not increase; target 0.

3.2 **Feature entities out of the root** (each one is a `WeakEntity`-free child with a
constructor, a small command enum and an event stream, like `DiffRenderer`):
- `TerminalPanelView`: 9 `terminal_*` root fields plus `pending_terminal_shutdown_prompt`
  and `next_terminal_session_seq`; 8 shared reads become constructor args or events;
  20 call sites (`close_guards.rs`, `bottom_panel_providers.rs`, popover menus).
- `HomeView`: 9 `home_*` fields, 5 call sites (`gitcomet_view.rs`, `state_apply.rs`, `splash.rs`).
- Documents UI: `documents.rs:1521` `impl GitCometView` (9 fields) into `DocumentsView`;
  the model layer (`buffers.rs`, `disk.rs`, `text.rs`, `repository.rs`) is already clean.
- Command palette: typed `Command` enum replacing the 92-arm string switch in
  `gitcomet_view.rs:321-1095` (`execute_command`); `on_close(Option<Command>)`.
- `PromptHost`: the `pending_*_prompt` fields and their render branches.
- `PaneLayout`: sidebar/details collapse, widths, animation sequences, `pane_resize`, and
  the eight `*ResizeState` structs (`mod_helpers/mod.rs:289-778`, `mode_impls.rs:1220`).
- `ToolProbes`: signing/large-file probe sequence, in-flight and cancellation fields.
- `UpdateCheck`: its three flags plus `update_check.rs`.
Target: root struct under 60 fields; `impl GitCometView` blocks only in `gitcomet_view*.rs`.

3.3 **One preference source for the shell panes.** `AppUiModel` (`view/app_model.rs`)
already holds `preferences`; the shell panes (sidebar, details, history, reflog, action
bar, status bar, repo tabs, popover host) read it through a `PreferenceSource` the way the
hosted diff pane already does (`DiffPreferences::Window(WeakEntity<AppUiModel>)`,
`hosted/environment.rs:1-9`). Delete the two-way root↔sibling fan-out in
`preference_sync.rs`, the 13 copies in `PopoverHost` (`panels/popover/mod.rs:174-219`) and
the copies in `HistoryView` (`panes/history.rs:1123-1127`) and `ReflogPaneView`
(`panes/reflog.rs:68-71`). `DiffRenderer` keeps its explicit preferences and its copy
(`panes/diff/helpers/state.rs:398-472`): that is a round-1 decision
(`docs/architecture.md:57`) and stays. Notify discipline stays per pane (fingerprint gates),
so this is a correctness-neutral move provable by the existing render-count tests.

3.4 **Seal the popover host.** Replace the 49 `|root, …|` closures calling ~45 root
methods/fields with a `PopoverServices` request enum handled by the root; stop the host
holding sibling entities (`PopoverHostInit`, `panels/popover/mod.rs:446`). Replace
`hash_popover_kind` (`fingerprint.rs:623-1254`) with `#[derive(Hash)]` after the failing
test for the tag-66 collision.

3.5 **Settings window.** `PreferenceChange` enum + `PreferenceSink::apply` on each main
window replaces the 41 setter closures through `update_main_windows`
(`settings_window/prefs.rs:371`). With 1.4's `window_chrome` and 2.7's preview factory,
`settings_window/` (10.2k) can become `gitcomet-ui-settings`.

3.6 **Modes.** `FocusedDiff` and `FocusedMergetool` (`mode_impls.rs:700-705`, 111 refs in
21 files) become thin roots built on the hosted diff pane instead of flags on the full
root (`FocusedDiff` today constructs every pane and renders one, `splash.rs:673`). Last,
optional.

3.7 **File-size ratchet.** Lower `scripts/check-rust-source-size.sh` defaults from 4,500/6,500
to 3,500/5,500 when Stage 2 lands and 3,000/5,000 when Stage 3 lands. Where each of the 19
production files above 3,000 lines gets handled:
- by a slice above: `gitcomet_view.rs` 3,597 (3.2), `syntax/heuristic.rs` 3,444 (2.1),
  `panes/diff/diff_text.rs` 3,314, `rows/diff_text.rs` 3,137 and `rows/conflict_resolver.rs`
  3,293 (2.7, split by render pass), `mod_helpers/conflict_resolver_ui_state.rs` 3,103 (2.7),
  `core/services.rs` 3,140 (4.4, one module per capability), `terminal_alacritty.rs` 3,172 (2.4);
- `rows/benchmarks/conflict.rs` 3,452: the guard counts `benchmarks/` as production; classify
  it as test code in the script;
- inline tests to `tests.rs`: `gix/repo/status.rs` 3,060 (1.6k inline), `gix/repo/diff.rs`
  3,044 (750 inline); ui-gpui carries ~40k inline test lines in 184 production files, so this
  is the cheap first cut everywhere;
- dedicated splits: `panes/sidebar.rs` 4,121 (into `panes/sidebar/`, which already has 11
  files), `panes/diff/diff_search.rs` 4,068 (query / worker / navigation), `panes/history.rs`
  3,371 (interaction vs render), `rows/history_canvas.rs` 3,550 (hit-testing vs painting),
  `panels/popover/context_menu.rs` 3,253 (per-target modules already exist under
  `context_menu/`), `branch_sidebar.rs` 3,202, `panes/details.rs` 3,255,
  `core/merge/plan.rs` 3,102 (plan builder vs block classification).

## Stage 4: state crate

4.1 **Per-repo copy-on-write.** `AppState.repos: Vec<Arc<RepoState>>` (`model/app.rs:138`)
behind `repo(id)`/`repo_mut(id)` accessors replacing the 201 `iter_mut().find` sites
(heaviest: `reducer/effects.rs` 66, `diff_selection.rs` 31, `reducer.rs` 25); `Arc`-wrap
`RepoFeedbackState` (three 200-entry `Vec`s), `FileBrowserState` sets, `NavStack`,
`ConflictSession`; gate `finalize_reduced_state`'s all-repo loop (`reducer.rs:616`) on a
touched set. Metric: the clone timing already recorded by `reducer_diagnostics.rs`, with the
paired live captures from `scripts/profiling/README.md` for repo-switch and history-scroll.
Follows the existing rule "unbounded lists in `RepoState` are `Arc<[T]>`".

4.2 **Feature owners**, in this order (cheapest seams first): conflict resolution
(`conflict_interactions.rs` + `conflict_load.rs`, 20 `Conflict*` Msg variants), file browser
(`reducer/effects.rs:1297-1818`, `reducer/filesystem.rs`, `src/explorer.rs`,
`repo_load/explorer_listing.rs`), selected diff (`diff_selection.rs`, `util.rs:185-664`,
`repo_load.rs:1459-1876`), history selection (the reveal/commit-details/log/signature arms
of `reducer/effects.rs` and `external_and_history.rs`, `effects/commit_details.rs`,
`util.rs:30-185`; note both files interleave these with sequencer arms such as
`prepare_squash`, `rebase_state_loaded`, `interactive_rebase_setup_loaded`, which go to
the sequencer owner below), then repository commands split by family (remote sync,
sequencer, branches/tags, submodules, worktrees, large files, stash) while merging the two
write protocols (`RepoAction`/`repo_actions.rs` and `RepoCommand`/`repo_commands.rs`) and
moving completions out of `reducer/auth.rs`. Each owner is `store/feature/<name>/{reducer,effects,tests}`
with a nested `Msg` sub-enum like `IndexedHistory`/`DiffSession`/`HistoryFind`. Each owner
removes its arms from `reduce_inner`, `schedule_effect`, `msg_requires_available_git`,
`is_view_navigation`, `is_control_msg`, `prepared.rs`, `unavailable.rs`, and the hand-written
`message_debug.rs` (derive where the payload allows). Tests move with the owner
(`store/tests/*` 49k → per-feature `tests/`).

4.3 **State API/impl split** (after 4.2 has made the seams): `gitcomet-state` keeps model,
msg, session, feature types and the `AppStore` handle; new `gitcomet-state-runtime` holds
reducer, effects, executors and the worker. Today the UI crate itself builds the store
(`AppStore::new` in `crates/gitcomet-ui-gpui/src/app/launch.rs:770`), so the split needs a
new injection path: `UiLaunch` takes a constructed `Arc<AppStore>` (or a store factory) and
`gitcomet-app`'s `AppLaunch` builds it from the runtime crate. The UI uses only `dispatch`
(454 production sites), `snapshot` (113), leases and publication, so reducer/effect edits
(~37k lines) stop recompiling `gitcomet-ui-gpui`. Boundary rule: `gitcomet-ui-gpui` must
not reach `gitcomet-state-runtime`.

4.4 **`GitRepository` capability traits** (`core/src/services.rs:571-1984`, 162 methods):
split by the families already present in `gitcomet-git-gix/src/repo/*.rs` (status, history,
diff, refs, ref writes, index/commit, conflicts, sequencer, stash, remotes, blame/files,
submodules, worktrees, lfs/annex, maintenance) with `GitRepository` as the composed supertrait
and a prelude for callers. Every capability trait stays object-safe (`docs/rust-refactor.md:252`).
Mechanical: one cross-family default (`compare_files`), ~34 implementors need
`impl Cap for T {}` (macro). Move off the public trait only methods no caller outside gix
uses: the transaction helpers `update_refs_with_output`, `apply_branch_updates_with_output`,
`recover_branch_updates_with_output`, `rebase_range_with_output`,
`push_refs_with_lease_with_output`, the duplicate `commit*` variants and the unused `push*`
ones; canonical entry points named in the round-1 migration table (for example
`pull_branch_with_output`) stay. Give extensions a read-only capability: `RepositoryReader`
(`extension-api/src/services.rs:11-36`) currently hands out the full write-capable object.
Regroup `gix/src/repo/history.rs` (branches, stash, stage, commit) and `porcelain.rs`
(sequencer) to match the families.

4.5 **Process supervision.** One implementation for core `process/probe.rs` (process-wrap)
and gix `util/process.rs` (custom pgid poll/kill); small, do when either is touched.

## Stage 5: extension API

5.1 **First-party extension.** Register built-ins through the same registry: commands (the
static `COMMANDS` table in `command_palette.rs:162` and the leaked `CommandEntry` rows in
`extension_host.rs:104-145`), Terminal/Reflog panels (`BottomPanelProvider` already unifies
them), settings pages (`SettingsTarget::Builtin(SharedString)` → typed ids), details/sidebar
tabs, then History as a `RepositoryViewDescriptor`. Descriptors capture a host-private
context so built-ins keep privileged access.

5.2 **Split `WindowHostImpl`** (~45 methods, `extension-api/src/host.rs:150-396`) into window
UI, repository access and workspace capability traits so `SettingsHost`
(`settings_window/extension_host.rs:204-447`) stops stubbing 12 methods.

5.3 **Contribution gaps**, each on top of a Stage 1 kit piece: context-menu locations for
host objects via `MenuModel<A>` (today `MenuLocation` is `Application`/`RepositoryTab` only;
the host's `ContextMenuAction` has ~141 variants); rich `HostedMenuItem` (tone, shortcut,
checked, submenu); settings rows inside built-in pages; sticky/progress toasts via
`ToastStack`; host commands and key bindings via the registry; a `TooltipHost` accessor;
theme contributions via the open catalog; selection/focus `ShellEvent`s.

## Sequencing

```
Stage 0 (0.1–0.5)  ──► 1.1, 1.2 ──► 2.0 ──► 2.1 ──► 2.3
                                 │        └──► 2.2 (also needs word_diff → core from 1.4)
                                 ├──► 1.3, 1.4, 1.5 (parallel, per component/module)
                                 ├──► 2.4, 2.5, 2.6 (parallel)
3.1 (glob script) runs per subtree just before that subtree is extracted or decomposed
3.2–3.5 after 2.1–2.5;  2.7 after 3.4 and the in-flight branches;  3.6 last
4.1 and 4.2 are independent of the UI stages and can run in parallel worktrees
4.3 after 4.2;  4.4 after 4.2 (touches the same effect modules);  5.x after 1.4 and 3.4
```

Independent worktrees can run: {0.x}, {1.1+1.2}, {2.4}, {2.5}, {4.1}, {4.2 first owner}
at the same time. Mind the disk: each worktree's `target` is 30+ GB.

## Verification

Per slice:
- `cargo fmt --all --check`; the three Clippy contexts from CONTRIBUTING; `cargo doc --no-deps -p gitcomet-ui-kit -p gitcomet-extension-api`.
- `python3 scripts/ci/boundaries.py`; `bash scripts/check-rust-source-size.sh`; `python3 scripts/ci/identity_literals.py`; `python3 scripts/ci/ratchets.py` (new).
- `python3 scripts/ci/inventory.py` before/after every test move; names and ignore flags must match modulo the documented module-prefix rename.
- `python3 scripts/ci/run.py test --context <crate>` for the touched crate, `--context example` after any kit or extension-API change, `--context workspace` before merge.
- For crate extractions: `cargo build --timings` before/after, report the ui-gpui unit time and critical path; `git diff -M --stat` shows renames, not rewrites.
- For 4.1: `reducer_diagnostics` clone timing and the paired repo-switch/history-scroll captures (`scripts/profiling/README.md`), with the Criterion budget subset from `docs/rust-refactor.md`.
- For 0.5 monitor: the converted tests' wall time; the full state suite under nextest must stay green on Linux, macOS and Windows (`cross-platform-tests.yml`).

End of round: full workspace run on all CI contexts, the Linux display-profile smoke checks, the normal release build without benchmark/test-support features, and a new "Round 2" section in `docs/rust-refactor.md` with migration notes for the kit, extension API, state message nesting and `GitRepository` capability traits, plus updated `docs/architecture.md` and `CONTRIBUTING.md` workspace layout.

## Decisions

1. Work lands as PRs on `dev` after `refactor` merges, not on a new long-lived branch.
2. Kit and extension-API breaking changes are allowed with migration notes (round-1 precedent).
3. gix runtime test overrides and benchmark instrumentation stay feature-gated out of
   production builds; only pure helpers move to the new `gitcomet-test-support` crate.
4. New crates are gpui-free wherever the survey showed it possible (text, syntax, conflict
   model, markdown model), so their tests run under nextest rather than the single-process
   libtest.
5. Two round-1 contract lines are amended, and `docs/rust-refactor.md` plus
   `CONTRIBUTING.md` are updated in the first slice to say so: "preserve the crate graph"
   becomes "preserve the layering direction; crates may be added and `gitcomet-state` may
   be split"; "large unit-test suites stay in their original crate harness" becomes "tests
   move with the code they test, proven by `inventory.py` parity modulo the documented
   module-prefix mapping".
6. The plan itself is committed as the first slice: `docs/refactor-round-2.md` (linked from
   `docs/rust-refactor.md`) plus `scripts/ci/ratchets.py` with today's baseline.

## Progress

### 0.1 Shared dependency artifacts

Dependencies build with one setting set (`opt-level = 1`, line-table debug info)
that the test profile inherits, so `cargo build` and `cargo test` reuse each
other's dependency artifacts. The test profile no longer packs split debug info,
and its gpui and taffy overrides now follow the dev profile's opt-level 2.
`ci-test` keeps dependency debug info off, so CI artifacts are unchanged.

| Measurement (this machine, 32 cores) | Before | After |
|---|---|---|
| Dependency units `cargo test -p gitcomet-ui-gpui --lib --no-run` compiles after `cargo build -p gitcomet` | all of them | 48 of 995 |
| UI suite, `--test-threads=8`, 4,809 tests | 249 s | 226 s |
| UI test binary plus `.dwp` | 584 MB + 1,395 MB | 1,637 MB |

The 48 remaining units are workspace crates, the gpui family (built with its
`test-support` feature) and test-only crates such as criterion and proptest.
The dev application now runs its dependencies at opt-level 1, so
dev-build timings are not comparable with captures taken before this change.

### 0.4 Build scripts

The kit reads its public `GPUI_REVISION` from `crates/gitcomet-ui-kit/gpui-revision`
instead of scanning the workspace manifest from its build script, which reran on
every manifest edit (50 in the 60 days before this change) and rebuilt the kit
and the UI host each time. After touching `Cargo.toml` the kit now stays fresh.
The existing contract test fails when the file and the manifest pin differ.

The splash backdrops stay rasterized at build time. The dark backdrop draws 148
circles over 10 radial gradients at 960 px; rasterizing it while the first
window opens would cost every launch, while the build-time `resvg` copy costs
only a clean build.

### 0.3 Perf report out of the UI host

`crates/gitcomet-perf` now holds the sidecar record format (`gitcomet_perf::sidecar`,
formerly `gitcomet_ui_gpui::perf_sidecar`) and the `perf_budget_report` binary.
One crate replaces the two the plan named: the report and the format change
together, and every writer needs only the format. Its dependencies are serde and
serde_json, enforced by a boundary rule. The UI crate loses 8,218 production
lines and its test builds no longer link a second full UI binary for the report.
All 117 moved tests (111 report, 6 sidecar) pass in the new crate. Workflows and
profiling scripts run the binary with `-p gitcomet-perf`.

### 0.5 Backend integration tests in one binary

The 32 `gitcomet-git-gix` integration test crates are modules of
`tests/integration/`, sharing one instance of each support module. The 1,114
backend tests map one to one (`module::test` names, unchanged ignore flags), and
pass under nextest and in a single libtest process. Sharing a process exposed a
second, identical test git environment in the status tests; they now use the
shared one. Three tests that re-run themselves in a child process qualify their
names through `support/child_test.rs`, which fails unless the child ran the test:
an unmatched `--exact` filter would otherwise pass without running anything.

| Measurement (`ci-test`, backend crate rebuilt from scratch) | Before | After |
|---|---|---|
| Test build, 4 jobs (CI runner size) | 26.1 s | 23.1 s |
| Test build, 32 jobs | 17.3 s | 14.7 s |
| Integration test binaries on disk (test profile) | 3.5 GB | 158 MB |

### 0.5 Monitor quiet checks on Linux

A monitor test's `quiet()` waited a fixed three seconds for no refresh. On
Linux, inotify queues events in order as they happen, so `quiet()` now repeats
native checkpoints (cookie round trip plus drain) until a round observes no
other native event, then requires that no refresh was sent. macOS and Windows
keep the timed window, as do monitors without native coverage: there a cookie
cannot order cache-delayed or coalesced events. A new test shows the faster
check still reports a refresh caused by an earlier edit.

| Measurement (Linux, `-E 'test(repo_monitor)'`, nextest `ci` profile) | Before | After |
|---|---|---|
| Cumulative test time | 274.0 s | 99.5 s |
| Wall time at 4 test threads | ~70 s | 26 s |
| Slowest test | 19.3 s | 10.1 s |

The monitor stays inside `gitcomet-state` for now. The remaining time is in
tests that wait for throttled recovery or churn for fixed periods by design, and
moving the module into its own crate would not change test time.

### 1.1 UI kit without core

The kit depends on the new `gitcomet-base` instead of `gitcomet-core`, and a
boundary rule now forbids core in the kit's graph. Product identity, per-user
directories, platform detection, file utilities, operation traces, work
counters and the environment snapshot moved from core to base unchanged; core
re-exports them, so `gitcomet_core::identity` and the other paths keep working.
This replaces the plan's runtime hooks (`set_display_name`, `set_recorder`): no
installation step can be forgotten, and counters, traces and the operation id
sequence stay single process-wide instances.

Three domain types left the kit's API. The file clipboard takes gpui's
`FileTransferOperation` and a caller-supplied identity function, and draws
ownership ids from the shared operation id sequence. `MinimapColumn` paints any
band type with a caller-supplied color; the merge colors moved to the host's
resolver view. The quick search bar's toggles are the kit's `SearchToggles`,
which the host converts to the search engine's options. All 31 moved base tests
pass (three are Windows-only); kit-facing UI tests and the downstream example
pass. The UI crate gained 118 production lines that moved out of the kit.

### 2.0 Text foundations

`crates/gitcomet-text` holds the rope and text model (from the kit, which
re-exports them at their old paths), tab expansion, the instrumented LRU caches
and recency queue (from the host's rows module) and `SyntaxTokenKind`. It
depends on GPUI's standalone string and sum-tree crates at the pinned revision,
never on GPUI itself, which a boundary rule enforces and the kit's revision
contract test now checks. The caches' test-only accessors (`peek`, `len`,
`clear`, `metrics`) became ordinary API, so no feature is needed to reach them.
All 50 moved tests pass in the new crate, and the UI suite count reconciles
exactly with the tests that left it.

### 2.1a Syntax styling in the kit

The theme's token styles (`syntax_highlight_color`, `syntax_highlight_style`,
`SyntaxHighlightPalette`) moved from the host's diff-row modules into the kit's
`syntax_style` module with their six tests. `gitcomet-text` gained
`SyntaxTokenKind::ALL` and a `SyntaxPalette` trait that the kit's palette
implements, so the syntax engine can produce styled runs for any renderer
without depending on GPUI.

### 2.1b `gitcomet-syntax`

The tree-sitter engine (`view/rows/diff_text/syntax.rs`, its 14 modules and 77
query files) is now `crates/gitcomet-syntax`, with all 76 grammar crates and
tree-sitter itself; the UI manifest lists none of them. The crate depends on
`gitcomet-core`, `gitcomet-text` and the allocator hooks, never on GPUI or the
kit, which a boundary rule enforces. Live documents style their runs through a
`SyntaxPalette` the caller passes to `snapshot`; the host passes the kit's
`SyntaxHighlightPalette`. `DiffSyntaxBudget` no longer reads the UI runtime: the
host's `default_diff_syntax_budget()` does. The UI host reaches the engine
through its existing facade in `rows/diff_text.rs`.

Engine items the UI's tests use compile under a `test-support` feature, so UI
tests keep the engine's test-mode cache scoping they ran with before; items only
the engine's own tests use stay `cfg(test)`. All 482 engine tests (3 ignored)
pass in the new crate, and the UI suite count drops by exactly those tests. The
UI crate lost 14,338 production lines.

Rebuilding only the UI crate under `ci-test` (non-incremental), three paired
runs against the commit before the move, on a machine shared with other builds:

| Measurement | Before | After |
|---|---|---|
| Median wall time | 259.6 s | 203.4 s |
| Median CPU time (user + system) | 2,796 s | 2,729 s |

Every pair favoured the move. CPU time fell 2.4%, in line with the code removed;
wall times varied by up to 40% between runs because of the other load.

### 2.2 `gitcomet-conflict-resolver`

`view/conflict_resolver/` (8.3k production and 7.6k test lines) is now
`crates/gitcomet-conflict-resolver`, depending on core and `gitcomet-text` only.
Two prerequisites moved to core first: the word-diff algorithm (`word_diff`) and
the autosolve pass counters (`ConflictAutosolveStats`, which state re-exports at
its message path). The split-row styled-text cache is presentation state and
moved to the host's `view/conflict_style_cache.rs` with its test. Test- and
benchmark-only items the UI's tests use compile under `test-support`, as for the
syntax engine. All 248 model tests pass (245 without the benchmark feature).

### 2.3 `gitcomet-markdown`

`view/markdown_preview/` (5.1k production and 4.9k test lines) is now
`crates/gitcomet-markdown`, depending on core, `gitcomet-diagrams` and
`gitcomet-syntax`. Two cuts differ from the plan. Code-fence languages come from
`gitcomet-syntax` directly instead of through a `CodeHighlighter` trait: the
engine is already GPUI-free, so a trait would add a seam without removing a
dependency. Scrollbar markers stay in the host: the model returns per-bucket
flags and the host's `view/markdown_scrollbar.rs` turns them into markers with
the existing diff helper, taking the five marker tests with it. Unifying the
host's marker builders in the kit is left to 1.4. The per-row styled-text cache
holds the renderer's type behind `Any`, so the model never names GPUI styles.
All 256 model tests pass under nextest; the UI suite runs 3,798 tests, 256 fewer.

### 2.4 `gitcomet-ui-terminal`

The embedded terminal widget (4.2k production and 2.4k test lines) is now
`crates/gitcomet-ui-terminal`: the alacritty backend, the viewport entity with
its layout and render caches, painting, and the clipboard command chord. It
depends on the kit and `gitcomet-base`, and its boundary rule forbids core as
the kit's does. The host passes the shell program to the spawn function and
keeps tabs, sessions, shutdown prompts and the panel.

Three choices differ from the plan:

- `terminal_preferences.rs` stays in the host. It converts to and from the
  state crate's session settings and launches external terminals, which is
  host policy rather than widget behaviour.
- The existing seam is kept instead of a new `TerminalView`/`TerminalEvent`
  API: the spawn function returns the terminal, its PTY sender and an event
  receiver, and the host's event task is the only consumer.
- The duplicated cursor blink was dead in the root (its active flag was never
  set), so it was deleted in its own commit rather than merged.

The host's twelve context-menu and focus tests drive the root and read the
viewport's selection, caret and PTY state, so those fields are public and the
recording PTY sender compiles under `test-support`. A narrower test surface
belongs with the terminal panel entity in 3.2. The crate's 104 tests run in
their own libtest process (`GPUI_PACKAGES`); one tab-title test moved back to
the host with the policy it covers.

### 2.5 `gitcomet-ui-media`

Image inspection, diagram previews and the source/rendered toggle (3.3k
production and 2.4k test lines) are now `crates/gitcomet-ui-media`, on the kit,
`gitcomet-base` and `gitcomet-diagrams`. The diff pane's three `impl
DiffRenderer` blocks went to the host's `panes/diff/media.rs`, together with
saving sampling preferences to the session. The crate owns the preference value
and its global; the host reads the session and passes it in.

Three cuts the plan did not name:

- The toolbar callback receives an `ImageMenu` and the chip's id instead of a
  host popover request. The host maps it to its text-format menu in one place.
- `Loadable` moved from state to `gitcomet-base`, and state re-exports it, so
  the diagram surface keeps its loading type without depending on state.
- The rendered-preview mode types moved into the crate's `rendered_preview`,
  next to the toggle that renders them.

Three tests moved to the host with the code they exercise: the conflict
image fallback drives the diff renderer, and two SVG text tests check the host's
rasterizers. They read `Group` through two `test-support` accessors. The crate's
55 tests run in their own libtest process.

### 1.4 Text helpers out of the host (first part)

`date_time.rs` and `path_display.rs` moved to `gitcomet-text` instead of a GPUI
`kit::text` module: neither touches GPUI, so their 20 Linux tests (22 on
Windows) run under nextest. `jiff` moves with the date code. The test-only
formatting wrappers compile under the text crate's `test-support`.

The platform-aware label helpers from `shortcut_labels.rs` (`secondary_shortcut`,
`alt_shortcut` and the command-palette `Shortcut`) moved into the kit's
`components::shortcut_keys` with their three tests. The host keeps only the
diff surfaces' file and change navigation key lists, which mirror its own
bindings. The remaining 1.4 pieces (window chrome, hover cards, toast stack,
menu model, query matcher, resize handle) are still in the host.

### 1.5 Kit and extension test support

The kit's real-text test backend loads its two faces from
`gitcomet_fonts::{IBM_PLEX_SANS_REGULAR_BYTES, LILEX_REGULAR_BYTES}` instead of
`include_bytes!` paths into a sibling crate, so it works for any downstream
crate. `gitcomet-extension-api` gains a `test-support` feature with
`fake_host::FakeWindowHost` (a module name the host's glob imports cannot
confuse with its own `test_support`): workspace state in memory, recorded requests (notifications,
toasts, dialogs, menus, navigation, panels, URLs) and dispatched messages, and
`HostError::Unsupported` for panes and lists that need a real window. The
example extension uses it for two new tests of review counting and the reset
prompt. The themed test-window fixture waits for 1.2: until the theme is a
window global there is nothing for a fixture to install.

### 1.2 Theme as a window global (first phase)

The kit now keeps each window's theme in a global, like zoom:
`theme::set_window_theme`, `follow_window_theme` for pop-outs and
`forget_window_theme` on close, read through `AppTheme::current(cx)` or
`AppTheme::for_window`. A window without one gets the default dark theme in the
current appearance. The root view installs its theme in `set_theme`, the
settings window in its render, and pop-outs follow their main window.

`TextInput::set_theme` is gone: an input restyles from its window's theme when
it renders, bumping its shaping epoch only when the style changed. That removed
every host call that themed an input, so host `set_theme` calls dropped from
130 to 29, and an unused rebase-onto input in the popover host went with them.

The remaining 23 calls are the root's fan-out to its panes. Several panes do
more than store the theme (the diff renderer clears style caches, documents
re-run syntax, the commit hover drops its body, extension hosts emit
`ThemeChanged`), and some read their theme from async completions, where no
window is current. Those move with 3.2, when each pane becomes its own entity.

### 3.2 Home out of the root

Home's search box, filtered lists, selection, scroll handles and pinned and
recent repositories moved from nine root fields into `HomeView`
(`view/home.rs`). It emits `HomeEvent::OpenWorkspace` and `OpenRepository`
instead of calling the root, reads its theme and zoom from the window, and is
told the window's workspace through `GitCometView::set_workspace_id`, the one
place the root now changes it. The root still composes the page (backdrop,
open, clone and init actions, the folder-picker fallback) around the entity.
Navigation keys are ignored while Home is hidden, as before; the root records
that when it renders the centre area. Root fields went from 168 to 161, at the
cost of 42 production lines of entity scaffolding.

### 3.7 Inline tests out of large files (first cut)

Eleven inline test modules (about 5,800 lines) moved to sibling files with
unchanged module paths, so test names did not change. That took eight files
under 3,000 lines (`diff_search.rs`, `history_canvas.rs`, `details.rs`,
`branch_sidebar.rs`, core `services.rs`, `conflict_resolver_ui_state.rs`,
`merge/plan.rs`, gix `repo/diff.rs`) and shrank `panes/sidebar.rs` by 530 lines.
Production files over 3,000 lines: 17 to 9. The remaining nine need real
splits; small cohesive inline tests stay where they are.

Two more files came under the limit: the syntax heuristic's keyword tables
(1,676 lines) moved to `heuristic/keywords.rs`, and gix `repo/status.rs`'s
`pub(crate) mod tests` (1,616 lines) to `status/tests.rs`. Seven remain.

Four more host files were split along their seams: the context menu's action
dispatcher (1,392 lines) to `context_menu/activate.rs`; history's column widths
and cache building to `history/columns.rs` and `history/cache_build.rs`; the
sidebar's file browser rendering to `sidebar/file_browser_view.rs`. The new
files import what they use explicitly, written by
`scripts/dev/expand_glob_imports.py` (the 3.1 tool: it drops `use super::*`,
checks the package, and imports each name the compiler reports from `super::`,
or traits from the path the compiler suggests). Explicit imports cost lines, so
the production-line ratchet records 64 more. Three production files remain
over 3,000 lines (`gitcomet_view.rs`, `panes/diff/diff_text.rs`,
`rows/conflict_resolver.rs`), plus `rows/benchmarks/conflict.rs`, which the
size guard counts as production code.

The last three went the same way: the command dispatcher (`execute_command`,
636 lines) to `gitcomet_view_commands.rs` beside the root, the pane resize
handle renderer into `gitcomet_view_render.rs`, the diff text's editor context
menu to `diff_text/editor_context_menu.rs`, and the three-way conflict column
rows to `conflict_resolver/three_way_rows.rs`. No production file is over 3,000
lines now and the ratchet holds that at zero. The size guard drops to 3,500
production and 5,500 test lines, the plan's first milestone, which leaves
in-flight branches room while the ratchet keeps the stricter line. The typed
`Command` enum for the dispatcher is still open.

Moving files broke a quiet assumption: the host's interaction guard allowlists
code by path and never noticed entries for files that had moved to the
terminal and media crates, so that code was no longer checked anywhere. The
kit's `discrete_control_violations` now reports an allowlist entry that is
missing or no longer needed, the host's lists were corrected (including a stale
`hosted/diff_pane.rs` entry from before this round), and the terminal and media
crates run the kit's control, clipboard and icon-size guards over their own
sources.

### 4.1 Copy-on-write feedback logs (first step)

An ignored timing probe (`timing_state_clone_with_full_repository_logs`) put a
number on the per-dispatch state copy: with each repository's bounded logs full
(200 diagnostics and 200 command-log entries), cloning the state cost 18.4 µs
for one repository and 245.7 µs for eight. Nearly all of it was the feedback
logs, so `RepoState::feedback` is now an `Arc<RepoFeedbackState>`, written
through `feedback_mut()`, which copies it only when a snapshot shares it. The
same probe now reads 1.6 µs for one repository and 5.2 µs for eight, and a test
asserts that a dispatch that leaves feedback alone keeps it shared.

Since `dev` started dropping feedback once the window acknowledges it, the
queues are short between acknowledgements; the shared `Arc` still keeps them and
the hook-activity log out of every copy, and an acknowledgement that releases
nothing leaves the feedback shared.

The rest of the plan's 4.1 (a copy-on-write repository list behind
`repo`/`repo_mut` accessors) would save the remaining 0.6 µs per repository at
the cost of rewriting about 550 test constructions and 200 lookups, so it waits
for the feature owners in 4.2, which rewrite those lookups anyway.

### 3.2 Typed commands

The window's command dispatcher matched 98 string ids. `view/commands.rs` now
defines `Command`, one variant per palette row, generated with its stable id by
a small macro; the palette table builds its ids from it and
`execute_builtin_command` matches the enum with no catch-all, so a listed
command without a handler no longer compiles. String ids remain the edge format
(scenario scripts, tests, extension rows): `execute_command` maps them with
`Command::from_id` and sends the rest to the extension registry. Nine
dispatcher arms for commands the palette never listed were empty TODO
placeholders and are gone. Tests check that every command has exactly one
palette row and that ids round-trip.

### 3.5 Settings send preference changes as data

The settings window updated main windows through 39 closures that each called
a root setter. It now sends a `PreferenceChange` value
(`view/preference_change.rs`) through `broadcast_preference`, and each main
window applies it in `GitCometView::apply_preference_change`. The settings
window's only remaining reads of the root are the window list and the active
repository's terminal launch context, which is the seam a later
`gitcomet-ui-settings` crate needs.

### 3.4 Popovers send root requests (first part)

Popovers called about 40 root methods and wrote three root fields through
`root_view.update` closures. They now send a `RootRequest` value
(`view/gitcomet_view_requests.rs`) through `RootRequests::request` on the same
weak handle, synchronously as before, passing the window only to requests that
act on it; the root handles them in one place. The popover-closed bookkeeping
(unsaved-edits prompt, hook-activity dialog, file-operation prompt, hover menu)
is one `PopoverClosed` request. What remains direct is reading: the window id,
the active context-menu invoker, error-details state and two terminal calls
that return values. Replacing the weak root handle with a request sink, and
the sibling entities `PopoverHostInit` hands the host, are the rest of 3.4.

### 1.4 `kit::window_chrome`

The client-side window frame was split between `view/chrome.rs`,
`app/chrome.rs`, `window_root_hook.rs` and `window_controls.rs`, all in the
host. The generic parts are now `gitcomet-ui-kit/src/window_chrome/`: frame and
title-bar geometry, title-bar drag and double-click handling, frame insets,
corner rounding, resize edges and cursors, the frame element, the control
buttons and their cluster (`mod.rs`); zoom, the system menu and native
move/resize grabs (`platform.rs`); the root mouse hook; and the window-controls
mode (`controls.rs`), which now takes the saved mode key instead of reading the
app session, so the host's `session_ui::window_controls` does that read. The
host keeps `TitleBarView`, workspace tints and the title-bar background, which
read product state. Their tests moved with them (19 kit tests), and the
host's fixed-px chrome scan now covers the kit module too.

`app/placement.rs` stays in the host: every function in it converts to or from
the persisted `session` placement types, so it is session restore policy, not
chrome.

