# Repository resource budget — implementation draft

This draft targets sessions with hundreds of open tabs and large repositories.
Tabs should cost little until used. Repositories refresh when selected; inactive
tabs do not need live status updates.

The existing session restore already opens only the selected repository, and
the monitor manager already bounds warm native watchers. The remaining lifetime
problem is that backend handles and loaded history data survive every visit.
Increasing the descriptor limit alone does not bound that growth.

## Lifecycle

| Repository | Backend and reloadable data | Watching |
| --- | --- | --- |
| Active | Kept open | Existing active monitor |
| Two most recently used inactive tabs | Kept warm | Existing platform policy: two on macOS, zero elsewhere |
| Other inactive tabs | Backend parked; large reloadable payloads released | Stopped |
| Document leases, held hosted panes/lists, or unfinished operations | Pinned until closed, released or finished | Existing watcher lease policy |

`repository_budget` applies the budget after each worker reduction. Parking
keeps the repository identity, lifetime, session entry, navigation stacks,
selection paths, preferences and commit-message suggestions. It invalidates
ordinary read generations, cancels history/index/search work, revokes worktree
cache ownership, and retires the backend. History pages, index topology, blame,
diff/image payloads, file listings and large status/reference lists are released.
Parking defers backend and shared payload destruction until outside the state
write lock, before replacement effects are scheduled. The UI also discards
retained history graphs on snapshots, including while History is hidden, and
clears parked reflog row caches.

Selecting a parked tab opens its backend again and refreshes its data. The
completion distinguishes this from an explicit reopen, so it preserves the
existing navigation and document state. Existing tab-switch selection behavior
still applies. Background document requests count as recent use; acquiring a
watch lease also wakes a parked repository without changing the active tab.
Explicit hosted-pane requests wake a missing backend and queue their latest
target until it opens; completion messages never wake a tab or update recency.
Primary file selections made during reopening also reload decoding attributes.

Mutation counters, automatic fetch in progress, maintenance, bare conversion,
file editing and conflict sessions prevent normal eviction. Repository and
linked-worktree watch leases pin their owners. Held hosted diff panes and file
lists also pin their repository until their owning handles close. These exceptions can increase
the live count above three; the budget is per store/window, not process-wide.
Extension views are cached until their repository closes, so their hosted panes
can remain pinned even when hidden. Visiting an extension view that creates a
file list in many repositories can therefore exceed the three-backend bound.
Visibility-based leases for those cached extension views are outside this draft.
Explicit automatic fetching retains its existing scheduling and can temporarily
open a cold repository for a fetch.

Pending authentication, branch/worktree confirmations, submodule trust checks
and prompts, and bare-conversion confirmations also pin their repository. These
continuations resume directly without selecting their tab first. Dismissal
releases the pin; approval transfers ownership to the new in-flight operation.
The push safety check after a commit is counted as an in-flight local action,
including authentication retries, until it hands off to a push or finishes.
A missing backend still produces its completion error and releases that count.

Filesystem operations resolve only affected parked roots on their filesystem
worker. A nested repository still supplies the mode and operation-lease checks;
a cold nested root is not bypassed in favor of its open parent. The temporary
backend is released after the operation without activating or warming a tab.
Live handles in other windows are reused, while an initial open keeps its
existing wait policy. These are admission checks before writing, not write retries.

## Descriptor exhaustion

`Error::is_file_descriptor_exhaustion` walks typed error causes for Unix EMFILE
and ENFILE. It does not match localized error messages.

For an eligible ordinary read or repository open:

1. Validate its repository read generation before attempting recovery.
2. Drop the inactive warm allowance to zero for 30 seconds and park eligible
   inactive repositories.
3. If the affected repository has no document lease or unfinished operation/editor,
   retire its backend and reopen the active tab once. Inactive tabs wait until
   selected. Leased documents retain their backend and receive the normal error.
4. Mark the replacement read generation as already retried. A slow failing
   reopen cannot restart recovery when the cooldown expires. A later explicit
   tab switch or reload permits a new attempt.
5. Deliver persistent errors through the existing error UI. Descriptor failures
   do not discard provisional tabs as though their paths were invalid.

The cooldown is shared by replies in one store to avoid a refresh fan-out
starting many recovery attempts. Mutations are never replayed. Request-specific
popovers, trust checks and previews retain their ordinary error handling.
If an error has lost its typed OS cause, or Git reports exhaustion only in its
stderr after starting successfully, this classifier cannot recognize it.

Cancellation is cooperative. Existing tasks, another window, automatic fetch,
or readers holding a snapshot/backend may delay actual resource release. The
retry is best effort and bounded; no thread is forcibly terminated.

## Startup headroom

Before crash logging, workers or Git child processes start, AppLaunch attempts
to raise its own Unix soft RLIMIT_NOFILE toward 8192, capped by the inherited
hard limit. An existing higher or unlimited soft limit is preserved. Smaller
targets handle kernel ceilings that are below the reported hard limit. Failure
is reported once and launch continues. No machine-wide limit or launchd setting
is changed, and the Windows path does nothing. The implementation reuses the
workspace's safe rustix wrappers.

## Validation status

Written with Codex. This is an implementation draft for human review.

Validation on this Mac (Rust 1.99) used a combined local draft that also includes
a separate macOS zoom-wheel policy change:

- A broad headless run exercised 3,054 cases across base, core, state, backend,
  app and CLI targets. All passed across environment-specific reruns: app
  configuration tests ran outside the checkout, while automatic-fetch tests
  retained the Mac's existing Git/proxy configuration.
- The final full state run passed all 1,678 enabled cases (9 skipped), including
  initial/cold background opens, latest-target queuing, document pinning,
  pending confirmation/authentication ownership, and commit-to-push handoffs.
- The broad GUI run passed 4,049 cases and identified four failures. Two were
  old Control-wheel fixtures, one an unrealistic activity timestamp fixture,
  and one exposed nested-save/attribute-resume integration bugs. After fixes,
  affected groups passed (413 cases), the remaining editor race passed its
  focused run, and all 56 hosted-pane cases passed after the final changes.
- The shared UI kit passed 536 enabled cases across its run and a corrected
  working-directory rerun; all 76 media cases passed. Ignored cases stay ignored.
- Workspace formatting, whitespace, all three documented Clippy lanes,
  all 18 dependency-boundary contexts, grammar symbols, identity literals and
  the open-source licence inventory pass.
- Subagent static reviews found lifecycle, UI retention and hosted-document
  integration issues. The fixes were reviewed again with no outstanding
  actionable production finding.

Tests cover the 264-tab backend bound and deferred release, lease/document,
pending-continuation and mutation exemptions, parked resumption, background wakeup, stale replies,
bounded descriptor recovery, typed OS errors, nested filesystem/bare checks,
decoding-attribute resume and startup fallback. A subprocess exercises the real
startup limit helper with a restricted hard limit without changing its parent.

The native app was packaged separately with an isolated profile and copied
264-tab session. With an inherited soft limit of 64 and hard limit of 256, five
complete passes plus 24 cold revisits passed (1,344 readiness witnesses). Sampled
FD peak was 90; settled counts were 55–64 after visiting repositories, with no
descriptor-exhaustion errors. The app could not raise that test process's hard
ceiling. A separate two-pass run also passed, with a sampled FD peak of 107.
These smoke runs preceded the final continuation-pin fixes; focused and full
state regressions validate those follow-up changes separately.

The five-pass status/history-ready witness measured 42.8 ms median, 111.2 ms p95
and 3,594 ms maximum. These are test-build diagnostics: the UI crate uses the
project's lighter CI-style optimization settings, and probes are enabled.
They are not release comparisons or full-frame/index-window latency claims.

RSS at the end of successive passes was 1,035, 1,126, 1,174, 1,217 and 1,227 MiB.
It reached 1,228 MiB during the final 30-second idle probe, with about 0.9% CPU.
Growth slowed to about 9 MiB on the fifth pass, but this does not establish a
long-term plateau. Static review found bounded caches and weak backend
registries, without identifying a concrete per-reopen ownership leak. Mimalloc's
exit statistics reported 44.5 MiB still allocated and 1.2 GiB of committed arenas;
these shutdown figures make allocator retention a possible contributor, not a
measurement of live allocations during the scenario. RSS remains a performance
limitation to investigate before making a low-memory claim.
Artifacts, including the five-pass summary and allocator statistics, are under
`target/resource-budget-smoke/real-repos-five-passes/`; they are ignored by Git.

The entire native cross-platform/release CI matrix, long-duration idle behavior
and all interactive conflict/editor workflows have
not been exhaustively tested. Held/cached extension panes remain an explicit
exception to the three-backend budget, as described above.
