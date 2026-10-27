# Git Command Log

The Git command log records the latest 100 Git executions and meaningful gix operations for each repository in memory.

Implementation status: the backend is committed; the UI integration is implemented in the working tree and still needs final verification. See [the continuation handoff](git-command-log-handoff.md).

## Using the log

Open **Git Command Log** from View, the application menu, or **Show Git Command Log** in the command palette. The bottom panel shares space with Terminal and Reflog.

The table shows executions newest first, with command or operation, absolute start time, elapsed milliseconds, status, backend and source, each on one line. Git rows lead with the backend's readable label and dim the full invocation after it. Start times include seconds and use the preferred timezone. Select a row to inspect its issuer, thread, working directory, exit code and correlation identifiers as labelled, selectable values; values that do not apply are left out. The close button beside the tabs hides the details. The stdout and stderr tabs show selectable, bounded previews under a byte count; native gix operations report their result without inventing process output.

Search covers commands, sources, metadata and retained output. The backend control cycles through All, Git and gix; **Failures only** includes failed, cancelled, timed-out and interrupted executions. **Copy entry** copies the selection, **Copy visible** respects the current filters, and **Clear** clears the active repository's shared history. Closing the panel leaves collection enabled.

## Behavior

- The bottom panel opens from View and the command palette, alongside Terminal and Reflog.
- Canonical working-tree roots identify histories; bare repositories use their repository root. Windows showing the same root share history, while linked working trees have separate histories.
- History survives closing a repository tab and is released on Clear or application exit.
- Each subprocess and native operation consumes one entry. Action and parent execution identifiers connect related work and fallbacks.
- Collection runs while the panel is closed. Arbitrary terminal commands and Git's internally spawned children are outside this history.
- Each entry records its invocation or native operation, origin, backend issuer, thread, start time, elapsed time, outcome, and available output.
- Start timestamps use wall time; elapsed time uses a monotonic clock and excludes executor queue time.
- Retain 16 KiB per output stream: the first 4 KiB and last 12 KiB. Report omitted bytes. Command and error metadata are bounded as well.
- Preserve command behavior, parsed bytes, authentication, cancellation and existing tracing. Redact URL credentials and sensitive authentication arguments in retained diagnostics; do not record environment variables, stdin or askpass responses.

## Ownership

Core owns the journal, execution guards, bounded output previews and transferable action context. The backend records executions. State carries context through messages, effect scheduling and worker submission. The UI subscribes directly to the journal, without publishing entries through AppState.

```text
UI or system origin → message context → reducer → scheduled effect context
                                                    |
                                                    → worker → Git/gix execution → journal
                                                                                     |
                                                                                     → log pane
```

GitBackend, GitRepository, Msg and Effect retain their public method/variant contracts. A private scheduled-effect envelope freezes attribution before releasing the state lock. Coalesced requests retain separate active and pending causes; promotion moves request and cause together. Merged requests use the latest accepted requesting action.

The journal inserts Running entries at execution start and evicts by start sequence. Completion does not reorder entries, and an evicted or cleared entry cannot be recreated by later output or completion. Guards finalize abandoned executions as interrupted. Process exit status and the caller's interpretation remain separate, including accepted nonzero exits.

## Integration

Subprocess runners tap output before parsing it. Direct launch paths include repository initialization, discovery and configuration reads, clone, submodule checks, merge extraction, reference transactions and the detached annex webapp reaper. Global runtime/setup probes and arbitrary terminal subprocesses are outside repository history.

UI actions supply their menu label, palette command, button identifier or shortcut. Extension commands supply their contribution identifier. State-generated work identifies startup, repository watching, activation or automatic refresh; callers without an attached cause remain unknown. Deferred callbacks and shared workers transfer owned context and restore it only while synchronous work runs. An extension that starts its own thread can transfer its current cause with `git_execution_log::context::wrap_task` from `gitcomet-core`.

The pane subscribes before taking its initial snapshot, coalesces updates through a bounded channel, and refreshes running durations while visible. Output notifications do not publish AppState or invalidate the whole application. Selection uses execution identifiers, so new entries do not move it to a different command; eviction, filters and Clear remove selections that are no longer visible.

## Native gix logging

The workspace selects gix 0.90.0 and resolves gix-trace 0.3.1. The current gix feature selection omits tracing. Upstream supports `tracing` and `tracing-detail`, `gix::trace::in_thread`, and a ForestLayer collector. Native spans are selective internal diagnostics, so this log records explicit backend operation boundaries and derives outcomes from returned results. Iterators and streamed callbacks finish inside the measured operation. It does not require enabling native tracing.

Sources: [gix features](https://github.com/GitoxideLabs/gitoxide/blob/dd39fb66e3b1551c9fe849043e39b357a9799ca1/gix/Cargo.toml), [gix-trace API](https://github.com/GitoxideLabs/gitoxide/blob/dd39fb66e3b1551c9fe849043e39b357a9799ca1/gix-trace/src/lib.rs).

## Validation

Retention tests cover independent repositories, shared history, start order, concurrent completion, eviction and Clear. Output tests cover large streams, UTF-8 boundaries, invalid bytes, redaction and unchanged parser input. Execution tests cover accepted nonzero exits, failures, timeout, cancellation, clone and reference transactions. Attribution tests cover deferred UI and hosted actions, watchers, shared workers, replacement tasks and A/B/C coalescing. Native tests cover complete iterator timing and fallback results. GPUI tests cover real journal notifications, pane coexistence, selection, filtering, copying, clearing and hide/show behavior.
