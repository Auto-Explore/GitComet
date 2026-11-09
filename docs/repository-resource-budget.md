# Repository resource budget

Open tabs do not keep their repository resources indefinitely. Each store keeps
the active repository and its two most recently used inactive tabs warm. It parks
the rest: their backend handle and large reloadable data are released, while
the tab keeps its lifetime, session entry, navigation, selections, preferences
and commit-message suggestion.

## Lifecycle

| Repository | Backend and reloadable data | Watching |
| --- | --- | --- |
| Active | Kept | Active monitor |
| Two most recently used inactive tabs | Kept warm | Two on macOS, none elsewhere |
| Other inactive tabs | Parked | Stopped |
| Pinned tabs | Kept until the pin ends | Watch-lease policy |

Recency is the later of a tab's last activation and its last background use: a
document request, a hosted-pane request or a background reload. Background use
leaves `last_active_at` alone. The repository picker, the session's fallback
tab and the tab-switch refresh shortcut read that field as the last activation.

The budget is per store, one per window, not process-wide. The worker applies it
after each reduction, once that reduction's effects are scheduled. Work a
reduction starts holds its backend, so parking cancels that work instead of
failing it on a missing handle.

## What parking releases

- History pages, index topology, history find, commit signatures, file
  history, commit details and blame.
- Diff and image payloads. An inline submodule view keeps its selection.
- A comparison's file list. The comparison keeps its endpoints.
- The file listing, status, reference lists, stashes, reflog, reference
  metadata and line statistics.

Handles and payloads are dropped outside the state write lock. On the next
snapshot, the UI drops the retained history graphs and reflog rows of parked
tabs, including while History is hidden.

## Pins

A tab keeps its backend while any of these hold:

- A hosted diff pane or file list, until its handle closes.
- A repository or linked-worktree watch lease.
- An unfinished operation:
  - a pull, a push, a push-after-commit safety check or a commit;
  - a worktree, sequencer or local action;
  - adding a submodule, maintenance, bare conversion or an automatic fetch;
  - file editing or a conflict session.
- A pending continuation that resumes without activating the tab:
  - an authentication prompt;
  - a branch or worktree confirmation;
  - a submodule trust check or prompt;
  - a bare-conversion confirmation;
  - a maintenance recommendation.

Pins can take the live count above three. Extension views stay cached until
their repository closes, so their hosted panes keep it pinned even while
hidden.

## Resuming

The backend opens again when the parked tab is:

- selected;
- reloaded in the background;
- given a watch lease;
- asked for a hosted pane;
- given a maintenance recommendation.

This open resumes the parked lifetime, so the tab keeps its navigation,
selection and documents. The refresh shows the reads parking released as
loading, and a selected comparison or inline submodule view loads again.

Hosted reads requested while the backend is missing stay loading and run when
the open succeeds. A failed open answers them with its error. A cancelled open
leaves them empty until the next open. A tab whose open fails after parking
stays open with the error, and its next activation retries.

`AppStore::repository` answers for a parked tab with a handle opened for the
caller. That handle does not wake the tab or count against the budget.
Filesystem operations open affected parked roots the same way on their worker,
and the handle still passes the mode and operation-lease checks.

A command dispatched to a parked tab without activating it still fails with
"Repository handle not found". An extension command run from the tab's
context menu is one example.

## Descriptor exhaustion

`Error::is_file_descriptor_exhaustion` walks typed error causes for EMFILE and
ENFILE. It does not match message text. Repository opens keep the OS cause, and
so do reads that fail while spawning Git or reading a file directly. The gix
status, reference, history and file-browser reads flatten their errors, so
their exhaustion is reported but does not start recovery.

An eligible failure in the current load generation makes the store:

1. Set the warm allowance to zero for 30 seconds and park the eligible inactive
   tabs. On macOS, warm monitors stop as well.
2. Park the failing tab if it is not pinned, and reopen it once if it is
   active. An inactive tab waits until it is selected.
3. Mark the retry's load generation, so the retry's own failure is reported
   instead of retried. A later activation or reload may try again.

Replies in one store share the cooldown. Mutations are never replayed.

## Startup headroom

Before workers or Git processes start, `AppLaunch` raises the process's soft
`RLIMIT_NOFILE` toward 8192. The target is capped by the inherited hard limit,
and if the kernel refuses it, 4096, 2048 and 1024 are tried in turn. A higher or
unlimited soft limit is kept, and the hard limit never changes.

Child processes inherit the raised soft limit, including:

- Git;
- hooks and credential helpers;
- the embedded terminal's shell.

If the limit cannot be raised, launch continues after one message on stderr.
Windows is unchanged.
