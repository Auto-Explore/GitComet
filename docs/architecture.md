# Architecture

GitComet separates Git contracts, application state, and GPUI presentation, with explicit owners for repository work and editable documents.

## Crate boundaries

The [workspace layout](../CONTRIBUTING.md#workspace-layout) lists each crate's role. `gitcomet-base` holds product identity, per-user directories and instrumentation without Git domain types, so the UI kit builds on it rather than on core. `gitcomet-core` re-exports those modules and defines domain types and service contracts; `gitcomet-git-gix` implements those contracts using gix and Git subprocesses. `gitcomet-state` depends on core and receives its backend through the service traits. The GPUI host composes state, reusable UI components, and compiled-in extensions.

Products launch through [`AppLaunch`](../crates/gitcomet-app/src/lib.rs) or [`UiLaunch`](../crates/gitcomet-ui-gpui/src/app/launch.rs). Extensions use [`gitcomet-extension-api`](../crates/gitcomet-extension-api/src/lib.rs), including weak window and repository handles; its contract does not depend on the UI host. The [example product](../crates/gitcomet-extension-example-app) exercises these public interfaces in its own build context.

The extension [`Registry`](../crates/gitcomet-extension-api/src/registry.rs) validates key chords and optional context predicates with GPUI's parsers before keymap installation. Invalid contexts join the existing registration errors, preserving declaration order and reporting all registration problems together.

[Dependency checks](../scripts/ci/boundaries.py) enforce the layering across normal and build dependencies. GPUI revisions and gix features are selected in the [workspace manifest](../Cargo.toml); local dependency patches and grammar changes are documented in [vendor/README.md](../vendor/README.md).

## Git contracts and errors

[`GitBackend` and `GitRepository`](../crates/gitcomet-core/src/services.rs) are `Send + Sync` traits used through `Arc<dyn ...>`, including by downstream backend implementations. Required methods define the common behavior; optional capabilities have conservative defaults. The canonical cancellable read families accept an explicit token. Implementations that perform these reads preserve the token when delegating and check it before work and before returning success; optional absent/unsupported defaults retain their capability-specific behavior. `ConfiguredBackend` applies repository options and forwards backend-wide object-store release for maintenance.

The gix backend owns its repository handles and private [history cache](../crates/gitcomet-git-gix/src/repo/log/cache.rs). Cache keys, retention limits, canonical repository identity, and shared history data remain backend concerns.

Core [`Error`](../crates/gitcomet-core/src/error.rs) retains an `ErrorKind` for recovery decisions, with optional typed source and operation/path context. Backend and state wrappers preserve native I/O causes; recovery and cleanup errors retain independent failures together. Paths keep their native representation, and `GitFailure` retains raw stdout and stderr bytes. The shared error representation makes errors cheap to clone into state. Formatting includes diagnostic context; callers classify failures through `kind()`.

## State and repository work

[`AppStore`](../crates/gitcomet-state/src/store/mod.rs) processes messages on its worker and publishes `Arc<AppState>` snapshots. [`PreparedReduction`](../crates/gitcomet-state/src/store/reducer/prepared.rs) resolves the required time, paths, saved preferences, and selected-file Git/filesystem facts before the model write lock. Short read-lock sections capture the required inputs without retaining a full snapshot during those queries. Reducers consume the prepared facts, update the model, and return effects; the scheduler dispatches work and sends completion messages back through the store. Failed repository-open cleanup retains synchronous session and recent-repository persistence.

```text
UI or filesystem event → Msg → prepare inputs → reducer → AppState snapshot
                                                 |
                                                 → Effect → scheduler → Git/filesystem work
                                                                |
                                                                → completion Msg → worker
```

The public [`msg`](../crates/gitcomet-state/src/msg.rs) facade exposes typed effect families for persistence, control, status, references, and repository commands. Their [schedulers](../crates/gitcomet-state/src/store/effects.rs) select the existing executor pools. Background worktree scans, history searches, persistence, and foreground loads retain separate scheduling needs; file-management mutations use the shared filesystem executor.

File-history and conflict loading have private feature owners on both sides of the effect boundary. Their [history reducer](../crates/gitcomet-state/src/store/reducer/file_history.rs) and [conflict reducer](../crates/gitcomet-state/src/store/reducer/conflict_load.rs) own feature transitions and completion handling. The matching [history scheduler](../crates/gitcomet-state/src/store/effects/file_history.rs) and [conflict scheduler](../crates/gitcomet-state/src/store/effects/conflict_load.rs) own executor payloads and reads. The central reducer and scheduler dispatch to these modules; request admission remains in the model owners described below.

Each file-history opening issues a [`FileHistoryRequestId`](../crates/gitcomet-state/src/model/history.rs). Its first page and continuation chain retain that identity. Completions must match the active request, path and expected page before replacing or extending history; an earlier opening cannot overwrite a newer result for the same path.

Conflict loads issue a separate [`ConflictLoadRequestId`](../crates/gitcomet-state/src/model/diff.rs). The conflict owner begins each load and accepts only completions matching its current identity and path. Admission precedes session restoration and autosolve, so an earlier same-path load cannot consume a pending restore or replace a newer full conflict load with provisional content.

[`RepoLoadsInFlight`](../crates/gitcomet-state/src/model/loads.rs) coalesces ordinary loads by `LoadKind`. Repository load epochs, log sequences, and line-statistics generations identify different kinds of stale work. Log walks, linked-worktree scans, and line statistics keep their own completion protocols. These loads check the current epoch or request when accepting a completion.

The worker's private [`RepositoryRuntimes`](../crates/gitcomet-state/src/store/repository_runtime.rs) pairs backend connections, ordinary repository load scopes, and linked-worktree cache leases with each model repository lifetime. These live resources belong to the worker. Calls within the same load epoch reuse an active scope; replacing it cancels the previous scope while retaining the connection and lease. Removing a repository retires the owner; disposal outside the state write lock revokes the lease, releases cache ownership, and cancels its load scope. Tasks retaining a lease cannot repopulate a cache after its owner closes. Signature verification and history find retain independent cancellation protocols.

## Reusable UI components

The reusable [`TextInput`](../crates/gitcomet-ui-kit/src/text_input.rs) owns text-edit history and selection/composition bookkeeping. Platform typing, IME composition and programmatic range replacements share one private edit operation. Composition selections are relative to the inserted text, and new replacements invalidate redo history. Entry points retain their focus, read-only and protected-content rules; programmatic view-preserving replacements retain their caret and scroll policy.

[`ContextMenuEntry`](../crates/gitcomet-ui-kit/src/components/context_menu.rs) receives an explicit asset path and `ContextMenuTone`. Git action names and English-label inference belong to the host's [menu presentation policy](../crates/gitcomet-ui-gpui/src/view/menu_presentation.rs); supplied assets take precedence over inferred icons. TextInput and settings-hosted menus share the entry's disabled gate, focus preservation and activation on completed clicks. Settings closes its menu and restores focus before invoking the action.

## Documents and diff presentation

Each `WindowHost` shares one [`DocumentOwner`](../crates/gitcomet-ui-gpui/src/view/documents/buffers.rs) among its panes. The repository window's [UI model](../crates/gitcomet-ui-gpui/src/view/app_model.rs) creates the owner used by repository editors, standalone documents, and hosted diff panes. The [settings host](../crates/gitcomet-ui-gpui/src/view/settings_window/extension_host.rs) supplies its own owner with read-only services. A `BufferId` identifies an editing session independently of its path; disk identity and version track the file being edited. Dirty text, format choices, content fingerprints, and pending writes belong to the buffer. Save completion matches the operation and compares the current fingerprint with the submitted one, preserving edits made after the write was submitted.

The reusable [`DiffRenderer`](../crates/gitcomet-ui-gpui/src/view/panes/diff.rs) receives explicit [`DiffPaneInput`](../crates/gitcomet-ui-gpui/src/view/panes/diff/input.rs), preferences, and [`DiffServices`](../crates/gitcomet-ui-gpui/src/view/panes/diff/services.rs). Input selects a repository/session binding or supplied diff snapshot. Services provide document ownership, store access, commands, navigation, and tooltips. Repository bindings check the repository lifetime and session so a detached pane cannot silently attach to a replacement repository. The host-only [environment](../crates/gitcomet-ui-gpui/src/view/hosted/environment.rs) assembles services and a preference source; the renderer copies the preference values it needs.

The [row dispatcher](../crates/gitcomet-ui-gpui/src/view/rows/diff.rs) prepares shared render-pass inputs and delegates to [file rows](../crates/gitcomet-ui-gpui/src/view/rows/diff/file_rows.rs), [patch rows](../crates/gitcomet-ui-gpui/src/view/rows/diff/patch_rows.rs) and [blame presentation](../crates/gitcomet-ui-gpui/src/view/rows/diff/blame_presentation.rs). Branch helpers receive the renderer, visible range and GPUI context explicitly. The [canvas element](../crates/gitcomet-ui-gpui/src/view/rows/diff_canvas.rs) owns row layout and painting; its private modules own annotations, stage gutters, streamed text and pointer interaction around the shared geometry.

[`MainPaneView`](../crates/gitcomet-ui-gpui/src/view/panes/main.rs) retains its history and diff child views and composes interactive rebase and cherry-pick setup for the repository window. [Hosted diff panes](../crates/gitcomet-ui-gpui/src/view/hosted/diff_pane.rs) use the same renderer for their session or snapshot; pop-outs remain bound to their originating window host. Hosted presentation options are supplied separately from repository binding, and snapshots use read-only services. Repository window actions and popovers pass through typed commands in the host's [weak command adapter](../crates/gitcomet-ui-gpui/src/view/diff_commands.rs), while each renderer retains its selection, scroll, search, and presentation caches.

The sidebar holds an on-demand [file history tab](../crates/gitcomet-ui-gpui/src/view/panes/file_history.rs) bound to one file. It drives the main diff through the existing `ShowFileChangesAtCommit` and `OpenFileAtCommit` messages and follows later diff target changes.
