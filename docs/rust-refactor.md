# Rust Architecture Refactor

This records the approved refactor, its implementation status, and source migrations.

## Contract

The owner approved staged implementation and coordinated Rust API breaks. Preserve
the crate graph, GPUI, gix, MVU snapshots, executor topology, user-visible features,
persisted formats, CLI behavior, native paths, subprocess bytes, and performance.
Keep Rust 1.99 and edition 2024. Preserve vendor patches and documented filesystem
ownership. Public facades and useful API documentation remain supported.

Every slice migrates its production callers, test doubles, examples and documentation
together. Replaced implementation paths are removed. Behavior corrections receive
regression tests and separate commits where separation needs no compatibility shim.
Independent review precedes each completed
subsystem's acceptance. No release, merge, or push is part of this implementation.

## Initial architecture checklist

- [x] Establish baseline tests and representative performance evidence.
- [x] Fix ConfiguredBackend object-store release delegation with a regression test.
- [x] Fix worktree-cache ownership collisions between stores using process-unique repository lifetimes.
- [x] Reject late cache insertions after repository close and release cache leases on shutdown.
- [x] Canonicalize repository API families: cancellation-aware reads; encoded text/conflict reads; fetch/pull pruning; remote/submodule policies.
- [x] Preserve error causes and operation/path context while retaining classifications and cloning.
- [x] Replace public raw load masks with LoadKind; give logs, line statistics and scoped worktree refreshes dedicated protocols.
- [x] Introduce distinct log IDs, load epochs, line-stat generations and repository lifetime types; completion methods reject stale IDs.
- [x] Group persistence/control, status, reference metadata and repository-command effects; each family owns runtime requirements and completion construction.
- [x] Centralize repository runtime resources and accepted close/shutdown handling, keeping refresh cancellation distinct from closure.
- [x] Prepare filesystem/Git inputs outside the state write lock while preserving synchronous worker ordering.
- [x] Encapsulate backend history cache construction/invalidation without changing locks, keys, bounds or sharing.
- [x] Establish one document owner per WindowHost with stable BufferId, load/save generations, disk versions, encoding, dirty state and retention.
- [x] Connect sidebar/close guards directly to document ownership and emit dirty changes at mutation time.
- [x] Extract a root-independent DiffRenderer after document ownership; preserve MainPaneView composition.
- [x] Migrate normal/hosted/settings/pop-out diff contexts, removing fake repository snapshots, projected AppState clones and duplicate rendering/search.
- [x] Remove deprecated launch wrappers replaced by UiLaunch.
- [x] Gate backend test overrides behind test-support and enable only test/probe configurations.
- [x] Narrow internal visibility/imports in refactored modules and update architecture/build/migration documentation.
- [x] Complete targeted and workspace validation and independent review.

## Target interfaces and ownership

GitBackend and GitRepository remain object-safe external extension points.
ConfiguredBackend binds product-wide RepositoryOptions. Equivalent overload-style
methods become one base-named method accepting cancellation, encoding, pruning or
URL policy explicitly. Streaming versus indexed history and unified versus parsed
diffs remain separate capabilities. Optional capabilities retain explicit absent or
unsupported results.

Ordinary load kinds use compact private masks. Log requests, line statistics and
worktree dirty scopes have dedicated state transitions. StatusLane determines its
own replay effect. Preserve log cancellation completions, file-browser stale-result
suppression, complementary selected-diff slots and signature work lifetimes.

RepositoryRuntime owns a checked lifetime identity, backend handle, task scope and
worktree-cache lease. The shared cache stays canonical-path keyed and capped at its
existing capacity. Close, bulk close and shutdown use the same owner operations.
Refreshes and tab switches retain reusable resources. Prepare canonical paths,
saved preferences, timestamps and required gitlink classification before acquiring
the state write lock; drop temporary snapshots before copy-on-write mutation.

Each WindowHost has one document owner. Independent GitCometView hosts have
independent owners; extension pop-outs share their originating host and close with
it. Settings snapshots remain read-only and create no document records. The owner
keeps BufferId distinct from DocumentIdentity: retained buffers may share a path.
Repository association is optional; closing a repository retains dirty buffers
with the same IDs. Presenters retain input widgets, syntax and geometry, with
existing undo/stashing/clean-buffer eviction semantics. Named owner operations
cover loading, editing, saving, discarding, dirty summaries and pending writes.
Read and save completion are independent; a receipt cannot replace a newer read
or clear edits made after the saved revision.

DiffPaneInput distinguishes a real Arc<AppState> plus checked repository/session
binding from a standalone Arc<DiffState> plus path. DiffRenderer takes explicit
services, preferences, document ownership and weak command routing, never the root
GitCometView. Keep live linked-worktree mutation checks, annotations, event ordering,
independent cancellation, SharedFileDiffCache and retained view boundaries.

## Initial validation scope

- Existing CI contexts: core, diagrams, state, backend, app, UI, example and workspace.
- Formatting, Clippy, source layout, dependency boundaries, rustdoc and native platform suites.
- Regression tests: backend release delegation and equal RepoId values in separate stores.
- Protocols: stale/repeated requests, A-B-A selection, coalesced refresh, failure cleanup and shutdown ordering.
- Cache lifetimes: closing during a blocked repository open cannot restore a released owner; store shutdown releases ownership.
- Concurrency: snapshots remain readable during blocking preparation.
- Documents: edits during saves, failed saves, encoding errors, rename/deletion, save-as collisions and close waiting for writes.
- Diff surfaces: stale retargets, linked worktrees, repository-free settings snapshots, pop-outs and one row/search preparation per update.
- Performance: existing deterministic budgets plus matched release history/scroll/select/warm-tab/idle/memory workloads.

## Initial architecture results

The approved architecture changes are implemented on `refactor`, starting from
`3b31ffb8`. Source changes through `cd65ca70` have passed independent review. The
[architecture guide](architecture.md) describes the resulting owners and contracts;
[CONTRIBUTING](../CONTRIBUTING.md) records the workspace and build contexts. The
migration notes below cover the coordinated public Rust API changes.

All eight declared CI contexts completed their compile, test and doctest commands
against the architecture implementation. The full workspace run passed 9,189 tests
with 50 existing ignores, plus two doctests; the example context passed 11 tests.
All three declared Clippy contexts, strict UI all-target/all-feature Clippy and
strict workspace rustdoc passed. The original rustdoc failure and its two reviewed
documentation corrections remain recorded separately.

Later corrections have scoped validation against their final source: 1,362 state
tests with six existing measurement skips; 247 UI benchmark and source-accounting
checks; 111 reporter tests; and 257 affected conflict and merge-output tests with
required fixtures and no ignores. Strict Clippy and the final formatting, source
layout, identity and dependency-boundary checks pass. The full workspace suite was
not repeated after those corrections.

The broader benchmark-feature run exposed twelve failures that also reproduced
on the immutable starting revision. The corrections preserve the measured
workloads while fixing inconsistent fixture expectations, process-wide allocation
test isolation, scan/byte accounting and syntax readiness. Sparse line-index
metadata now allocates its large payload separately, keeping small fragments
compact; output, clone, capacity-accounting and growth regressions pass. Deliberate
mutations failed their intended assertions, and exact sources were restored.

The final normal release app was built separately from the benchmark and reporter
binaries. Source snapshots remained unchanged, and the application excludes
benchmark, test-support and allocation-tracing features. The fresh 73-case
Criterion subset completed with all 167 measured structural checks passing.
Timing results were 44 passing checks and one alert: `repo_switch/two_hot_repos`
measured 121.231 microseconds against the unchanged 100-microsecond budget.
That subset leaves 186 timing and 630 structural budgets unmeasured; 28 measured
cases have no registered timing budget.

The paired repository-switch comparison completed all 18 captures with equal work
and unchanged source and binaries. Each of the nine candidate/baseline ratios was
below one. Median ratios for warm switching, selected-file switching and merge
draft restoration were 0.753, 0.750 and 0.755. The repeated-status-scan regression
is resolved in those measured cases. Both baseline and candidate exceed the
absolute warm-switch budget on this host: baseline means were 160–165 microseconds
and candidate means were 121–124 microseconds. The alert remains recorded.

Four metadata captures completed without timing alerts; one pair per case is
exploratory and does not establish statistical equivalence or a speedup. A separate
candidate-only search-refinement capture confirmed the corrected prepared-input
accounting and structural constraints, with no comparative timing claim.

The final normal-build follow-up passed all four native captures: repository
switching and diff-open on both the earlier and final refactored builds. All 90
inputs and 88 requested witnesses completed, with no superseded inputs. Both
repository-switch captures passed eight status assertions and proved that the
close update was applied and drawn before the final pointer input. Backend work
matched within both pairs. Source, binaries and fixtures remained unchanged.
These captures establish the tested behavior and work, with no comparative timing
or leak claim; the normal probe does not expose the exact selected diff target.
Two earlier control attempts remain recorded as failed harness setup, followed by
independently reviewed readiness and draw-order corrections.

The earlier normal-release native comparison completed 232 captures at
`3ad0814d`, including calibrated paired interactions, shared reads and lifecycle
workloads. Input witnesses and backend-work checks passed. No guarded primary
interaction regression was established; diff-open median and tiny idle-CPU
differences remained inconclusive. Both lifecycle runs retained positive memory
growth, so these measurements establish neither a plateau nor leak absence.
The reviewed source mapping identifies the subsequent state-classification and
sparse-metadata changes covered by the focused tests, fresh benchmarks and final
native follow-up above.

Linux X11 GNOME, Wayland GNOME and Wayland KDE display-profile smoke checks passed.
Native measurements cover this headless Linux host; macOS and Windows native
suites were not run. The Linux portability checker fails on this Arch host because
the baseline and candidate require glibc 2.44, above the intended Ubuntu 22.04
baseline of 2.35. A successful supported-distribution release package is not part
of these results.

Local evidence is retained under `target/refactor-final-integration/`,
`target/refactor-repo-switch-fix/`, `target/refactor-benchmark-followup/`,
`target/refactor-streamed-output-layout/` and `target/refactor-whole-review/`.
Final release and measurement records are under `target/refactor-production-002/`,
`target/refactor-pr-perf-002/`, `target/refactor-repo-switch-final-001/` and
`target/refactor-refinement-sidecar/run-001/`. The final native records are under
`target/refactor-native-state-fix/run-003/`, with prior failed attempts retained
alongside them.
The original failures remain alongside their diagnoses and subsequent checks.

## Correctness and module follow-up

The approved follow-up from `cb52f7ab` is implemented through `6d2f7e60`.
The [architecture guide](architecture.md) describes the resulting boundaries;
the migration notes below cover the coordinated request and menu API changes.

- File history and conflict loads have distinct opaque request identities. Each
  feature rejects obsolete completions before changing pagination, restoration
  or conflict state.
- TextInput shares edit bookkeeping and separates platform input, selection,
  clipboard, configuration, highlighting and layout concerns. Platform edits
  invalidate obsolete redo history, and IME selection uses replacement-local
  UTF-16 offsets.
- Registry construction validates key contexts with GPUI's parser. Menu rows
  accept explicit assets and tones; Git and English-label inference belongs to
  the host. Settings and TextInput reuse the kit's interaction primitives.
- Closing an inactive repository reconciles retained dirty buffers even when
  the rendered input fingerprint is unchanged. Store-backed tests recover and
  save the same buffers after active and inactive repository closure.
- State load owners, diff row rendering and canvas concerns have private child
  modules. Large state, editor, cache/blame, settings, smoke and TextInput test
  suites have cohesive child files in their original harnesses. Test attributes,
  isolation and runner classification are preserved; the obsolete ignored close
  prompt case is replaced with actual recovery coverage.

Fresh workspace validation passed 9,222 tests with 49 existing ignores, plus two
doctests. The downstream example passed 11 tests and the headless app passed 422.
All three Linux display profiles passed their declared integration smoke checks.
These test executions identify source `af455b01`; the only subsequent Rust
changes remove seven leading newline bytes from extracted test modules. An
independent byte-for-byte bridge preserves all other source and test contents.
Formatting, source-size limits, identity literals, dependency boundaries, all
three declared Clippy configurations and strict workspace rustdoc pass against
`6d2f7e60`. Scoped all-target/all-feature state and UI Clippy checks also pass.
The CI runner's Python suite ran 123 tests with one existing skip.

The normal release app was built separately from the benchmark and reporter
binaries, with benchmark, test-support and allocation instrumentation features
absent from its production dependency graph. The fresh Linux dependency check
still fails: this Arch-built binary requires glibc 2.44, above the intended
Ubuntu 22.04 baseline of 2.35.

The six-case Criterion subset passed all 16 registered structural checks for
the selected cases.
It leaves 226 timing and 781 structural budgets unmeasured. The TextInput case
has fresh timing and work sidecars but no registered reporter budget. Two timing
alerts remain: `repo_switch/two_hot_repos` measured 134.543 microseconds against
100 microseconds, and `status_select_diff_open/staged` measured 1.071 milliseconds
against 10 microseconds. A separate staged-selection control reproduced the
alert on the preserved baseline and candidate, with identical reported work and
allocation metrics. Host load was uncontrolled; these measurements establish
neither timing equivalence nor a speedup, regression or cause. No threshold changed.

All eight normal-build native captures passed their validators and completed
their requested work: `diff-search`, `settings`, `history-reopen` and
`repo-tabs-edit` on both builds. They establish the tested functional witnesses
and Linux draw submission, without comparative timing or GPU/display-completion
claims. The settings capture opens the settings view; hosted-menu interactions,
IME/edit history and unsaved-buffer recovery use the focused deterministic tests.
Native macOS and Windows suites were not run. Lifecycle growth was not
remeasured; the earlier limits above remain.

Each implementation slice and the complete source received independent review.
Regression failures and deliberate mutations, exact move and inventory proofs,
integration logs, the formatting bridge and runtime records are retained under
`target/next-refactor/`. The earlier architecture results above identify their
own source and are separate from this follow-up's validation.

## Migration notes

`GitRepository` remains object-safe for external backends. Each equivalent method
family now has one entry point; migrate implementations and callers together.
Optional unsupported or absent results retain their previous meanings. Streaming
and indexed history, supplied-status line statistics, unified and parsed diffs,
images and file-backed previews remain distinct capabilities.

| Previous use | Canonical call and preserved default |
|---|---|
| Plain or `_cancellable` reads | Pass `&CancellationToken` to the base method, such as `status`. Forward existing tokens; otherwise pass a fresh token. |
| Encoded `diff_parsed` / `diff_file_text` overloads | Pass `(target, encoding, cancellation)`; `encoding: None` keeps the previous default. |
| `conflict_session_with_encoding` | Use `conflict_session(path, encoding)`. |
| Signature verification overloads | Use `verify_commit_signatures(ids, formats, cancellation)`; former plain calls use `SignatureFormats::ALL`. |
| URL-policy overloads | Append `RemoteUrlPolicy` to the base method; former policy-less calls use `RemoteUrlPolicy::default()`. |
| Fetch overloads | Required `fetch_all(prune) -> Result<CommandOutput>`; former calls without pruning arguments pass `true`. |
| Pull overloads | Required `pull(mode, prune) -> Result<CommandOutput>`; former calls without pruning arguments pass `true`. |
| Branch-pull overloads | Optional `pull_branch_with_output(remote, branch, prune)`; former plain calls pass `false`. |

Custom implementations that previously relied on a cancellable wrapper must keep
its cancellation checks before work and after a successful result. Implementations
that supplied only unit-returning fetch/pull methods preserve their effects and
errors, returning `CommandOutput::empty_success("git fetch --all")` or
`CommandOutput::empty_success("git pull")` on success. Implementations with captured
output keep that output. No deprecated forwarding overloads remain.

External test harnesses using `install_test_git_command_environment` or
`allow_test_repo_local_mergetool_command` must enable
`gitcomet-git-gix/test-support`. In-tree tests need no additional flags. Direct
`operation-context-probe` builds enable `benchmarks,test-support`; the profiling
helpers select those features automatically and retain support for older baseline
manifests. Build the production app separately from probes to keep test overrides
and benchmark instrumentation out of its dependency features. The shared profiling
driver selects its historical API adapter from checked signatures and records that
selection while keeping the workload source identical across compared revisions.

Ordinary load queries and transitions now take `LoadKind`, for example
`LoadKind::HeadBranch`, instead of `RepoLoadsInFlight` bit constants. Logs,
line statistics and linked-worktree dirty scans use their dedicated methods.
Use `any_foreground_in_flight()` for an active-tab spinner; linked-worktree scans
alone do not keep that spinner running.

Keep the issued `LogLoadSeq`, `RepoLoadEpoch` and `LineStatsGeneration` values
when forwarding requests and completions. Their tuple constructors are internal.
`finish_log` takes its request identity and rejects stale completions itself.
`RepoState::lifetime_id()` provides a process-unique `RepoLifetime`; the existing
scalar `lifetime()` accessor remains available to consumers that only compare it.

`Effect::LoadFileHistory` and `InternalMsg::FileHistoryLoaded` carry a
`FileHistoryRequestId` in their `request` field. Forward the issued identity into
the completion and every continuation; do not replace it with a new identity.
`Msg::LoadFileHistory` retains its existing arguments. New openings and repository
reactivation issue new identities, so stale results for the same path are rejected.

`Effect::LoadConflictFile`, `Effect::LoadSelectedConflictFile` and
`InternalMsg::ConflictFileLoaded` carry a `ConflictLoadRequestId` in their
`request` field. Forward the issued identity unchanged into the completion.
`Msg::LoadConflictFile` retains its existing arguments. Each reload issues a new
identity, including the provisional `CurrentOnly` to `Full` upgrade; conflict
editing revisions and repository load epochs remain separate protocols.

`ContextMenuIconSlot::Icon` now takes the asset path to draw directly. Migrate
semantic icon names or inferred labels to explicit paths, and select appearance
with `ContextMenuEntry::tone(ContextMenuTone::...)`. The default tone is `Accent`;
`Neutral`, `Success`, `Warning` and `Danger` express the other supported roles.
The removed `components::context_menu_icon_path` helper is replaced by presentation
policy at the caller; the GitComet host owns its Git and English-label mapping in
`view::menu_presentation`. Explicit `icons/` and `extensions/` assets keep their
identity even when an action label would infer a different icon.

Test harnesses using `AppStore::insert_repo_for_test` must first seed the matching
repository through `replace_snapshot_for_test`. The runtime uses that model's
lifetime; a backend inserted without a repository model is invalid test setup.

Persistence, control, status, reference and repository-command requests now use
typed `Effect` payloads exported from `gitcomet_state::msg`. For example,
`Effect::LoadBranches { repo_id }` becomes
`Effect::References(ReferenceEffect::LoadBranches { repo_id })`; saving a session
uses `Effect::Persistence(PersistenceEffect::PersistSession { repo_id, action })`.
The other envelopes are `Control(ControlEffect)`, `Status(StatusEffect)` and
`RepoCommand(RepoCommandEffect)`. Match the same nested shape when inspecting
effects. Completion messages keep their existing `InternalMsg` variants.

The runtime requirement match covers every effect envelope explicitly. Persistence
and controls remain usable without Git. Status and reference owners construct
both missing-handle and unavailable-Git failures, retaining load guards and the
foreground/metadata executor split. Command scheduling retains its existing
operation progress, authentication retries and special completion paths. Operation
trace labels continue to name the individual request.

`Error::kind()` retains its existing classifications. Use `Error::from(io_error)`
to keep an I/O cause, then `with_context(operation, path)` to identify the failed
operation. `source()` exposes the original typed cause and `context()` keeps the
native path. Clones share the error payload; adding context to one clone leaves
other snapshots unchanged. Render diagnostics through `Error` rather than a raw
`ErrorKind::Backend` string.

The deprecated `gitcomet_ui_gpui::run` and `run_with_startup_crash_report*`
entry points are replaced by `UiLaunch::new(backend)`. Configure the builder with
`initial_request`, `startup_crash_report`, `browser_requests`, and, when present,
`on_shutdown`, then call `run`. The return value is `Result<UiRunOutcome,
UiLaunchError>`; callers of the former unit-returning `run` can map the outcome
to `()` at their own boundary. These wrappers had no remaining in-tree callers.

## Round 2

The second round, planned on 2026-10-15, is recorded in
[refactor-round-2.md](refactor-round-2.md). It amends two lines of the contract
above: the layering direction is preserved while crates may be added and
`gitcomet-state` may be split into an API crate and a runtime crate, and tests
move with the code they test, proven by `inventory.py` parity modulo the
documented module-prefix mapping. `scripts/ci/ratchets.py` records the structure
counts the round drives and fails when one regresses.

### Round 2 migration notes

| Previous use | Replacement |
|---|---|
| `gitcomet-ui-kit` re-exporting core types through its API | The kit depends on `gitcomet-base`; core paths such as `gitcomet_core::identity` still resolve through re-exports. |
| `clipboard::FilePayload::intent: TransferIntent` | `gpui::FileTransferOperation`; convert at the filesystem boundary. |
| `clipboard::write_files(cx, paths, intent)` | Same call with a `gpui::FileTransferOperation`; ownership ids come from `gitcomet_base::next_operation_id()`. |
| `PasteReceipt::from_drop(paths, transfer, intent)`, `capture_paste(cx, paths, intent)` | Add an `identity: fn(&Path) -> PathBuf` spelling sources as completions report them. |
| `MinimapColumn::new(id, bands)` with core's `MinimapRowKind` | `MinimapColumn::new(id, bands, color)` for any `Copy + PartialEq` band type and a `MinimapBandColor`. |
| `QuickSearchBar::options(TextSearchOptions, ..)` | `QuickSearchBar::options(SearchToggles, ..)`. |
| `gitcomet_core::filesystem::OperationId::allocate()` sequence | Unchanged values, now drawn from `gitcomet_base::next_operation_id()`. |
