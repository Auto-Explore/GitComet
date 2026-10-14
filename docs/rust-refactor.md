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
separate regression tests and commits. Independent review precedes each completed
subsystem's acceptance. No release, merge, or push is part of this implementation.

## Implementation checklist

- [ ] Establish baseline tests and representative performance evidence.
- [x] Fix ConfiguredBackend object-store release delegation with a regression test.
- [x] Fix worktree-cache ownership collisions between stores using process-unique repository lifetimes.
- [ ] Canonicalize repository API families: cancellation-aware reads; encoded text/conflict reads; fetch/pull pruning; remote/submodule policies.
- [ ] Preserve error causes and operation/path context while retaining classifications and cloning.
- [x] Replace public raw load masks with LoadKind; give logs, line statistics and scoped worktree refreshes dedicated protocols.
- [x] Introduce distinct log IDs, load epochs, line-stat generations and repository lifetime types; completion methods reject stale IDs.
- [x] Group persistence/control, status, reference metadata and repository-command effects; each family owns runtime requirements and completion construction.
- [ ] Centralize repository runtime resources and accepted close/shutdown handling, keeping refresh cancellation distinct from closure.
- [ ] Prepare filesystem/Git inputs outside the state write lock while preserving synchronous worker ordering.
- [x] Encapsulate backend history cache construction/invalidation without changing locks, keys, bounds or sharing.
- [ ] Establish one document owner per window with stable BufferId, load/save generations, disk versions, encoding, dirty state and retention.
- [ ] Connect sidebar/close guards directly to document ownership and emit dirty changes at mutation time.
- [ ] Extract a root-independent DiffRenderer after document ownership; preserve MainPaneView composition.
- [ ] Migrate normal/hosted/settings/pop-out diff contexts, removing fake repository snapshots, projected AppState clones and duplicate rendering/search.
- [x] Remove deprecated launch wrappers replaced by UiLaunch.
- [x] Gate backend test overrides behind test-support and enable only test/probe configurations.
- [ ] Narrow internal visibility/imports in refactored modules and update architecture/build/migration documentation.
- [ ] Complete targeted and workspace validation and independent review.

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

The document owner keeps BufferId distinct from DocumentIdentity: retained buffers
may share a path. Repository association is optional; closing a repository retains
dirty buffers. Presenters retain input widgets, syntax and geometry, with existing
undo/stashing/clean-buffer eviction semantics. Operations cover activation, editing,
saving, discarding, dirty summaries and pending writes.

DiffPaneInput distinguishes a real Arc<AppState> plus checked repository/session
binding from a standalone Arc<DiffState> plus path. DiffRenderer takes explicit
services, preferences, document ownership and weak command routing, never the root
GitCometView. Keep live linked-worktree mutation checks, annotations, event ordering,
independent cancellation, SharedFileDiffCache and retained view boundaries.

## Validation

- Existing CI contexts: core, state, backend, app, UI, example and workspace.
- Formatting, Clippy, source layout, dependency boundaries, rustdoc and native platform suites.
- Regression tests: backend release delegation and equal RepoId values in separate stores.
- Protocols: stale/repeated requests, A-B-A selection, coalesced refresh, failure cleanup and shutdown ordering.
- Cache lifetimes: closing during a blocked repository open cannot restore a released owner; store shutdown releases ownership.
- Concurrency: snapshots remain readable during blocking preparation.
- Documents: edits during saves, failed saves, encoding errors, rename/deletion, save-as collisions and close waiting for writes.
- Diff surfaces: stale retargets, linked worktrees, repository-free settings snapshots, pop-outs and one row/search preparation per update.
- Performance: existing deterministic budgets plus matched release history/scroll/select/warm-tab/idle/memory workloads.

## Implementation status

The starting revision is `3b31ffb8`. Planning-stage formatting, source layout and
dependency-boundary checks passed. The immutable baseline passed 3,290 core, state
and backend tests, with 28 existing skipped tests. Both new lifecycle regressions
failed against the original behavior before their fixes. The UI baseline passed
4,708 tests with 22 existing ignored tests, the UI kit passed 502 tests, and all
422 app tests passed. The CI/profiling Python suites passed with their existing
platform skips. The baseline release app and backend probe are built, and the
native history-selection driver passed. Matched performance comparisons follow
the implementation. The work is on the existing `refactor` branch.

The backend now groups history cache construction and object-store invalidation
in a private `HistoryCache` owner. Its six independent locks, cache keys and
bounds, range-reader recycling and shared history-store behavior remain unchanged.
Backend test environment and mergetool-consent overrides require the `test-support`
feature; default tests enable it through dev dependencies, while normal app
dependencies exclude it. The unchanged integration inventory passed 1,968
core/backend tests, and the strict LFS/annex supplement passed 98 tests without
runtime prerequisite skips. Normal and opt-in Clippy checks passed, including
the operation probe.

## Migration notes

External test harnesses using `install_test_git_command_environment` or
`allow_test_repo_local_mergetool_command` must enable
`gitcomet-git-gix/test-support`. In-tree tests need no additional flags. Direct
`operation-context-probe` builds enable `benchmarks,test-support`; the profiling
helpers select those features automatically and retain support for older baseline
manifests. Build the production app separately from probes to keep test overrides
and benchmark instrumentation out of its dependency features. Repository service
signatures are unchanged in the cache and test-support commits.

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
