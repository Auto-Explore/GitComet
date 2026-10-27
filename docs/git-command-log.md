# Git Command Log

The Git command log records the latest 100 Git executions and meaningful gix operations for each repository in memory.

Status: implementation in progress.

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

## Implementation sequence

1. Build the repository journal, context, previews and subprocess instrumentation, including streamed stdout and direct launch sites.
2. Carry attribution through UI, deferred callbacks, state messages, workers, completions, coalesced loads and extension commands.
3. Record meaningful native gix operations and link fallback subprocesses to their native parent.
4. Add the independent bottom-panel pane, filtering, details, copying and clearing.
5. Verify retention, concurrency, output bounds, outcomes, provenance, coalescing, native timing and panel integration; run the repository checks.

## Native gix logging

The workspace selects gix 0.90.0 and resolves gix-trace 0.3.1. The current gix feature selection omits tracing. Upstream supports `tracing` and `tracing-detail`, `gix::trace::in_thread`, and a ForestLayer collector. Native spans are selective internal diagnostics, so this log records explicit backend operation boundaries and derives outcomes from returned results. Iterators and streamed callbacks finish inside the measured operation. It does not require enabling native tracing.

Sources: [gix features](https://github.com/GitoxideLabs/gitoxide/blob/dd39fb66e3b1551c9fe849043e39b357a9799ca1/gix/Cargo.toml), [gix-trace API](https://github.com/GitoxideLabs/gitoxide/blob/dd39fb66e3b1551c9fe849043e39b357a9799ca1/gix-trace/src/lib.rs).

## Validation

Retention tests cover independent repositories, shared windows, start order, concurrent completion, eviction and Clear. Output tests cover large streams, invalid bytes, redaction and unchanged parser input. Execution tests cover accepted nonzero exits, failures, timeout, cancellation, reference transactions and detached processes. Attribution tests cover deferred UI work, watchers, extension workers, replacement tasks and A/B/C coalescing. Native tests cover complete iterator timing and fallback results. GPUI tests cover tab coexistence, switching, selection, filtering, copying and clearing.
