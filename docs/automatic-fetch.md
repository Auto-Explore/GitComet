# Automatic Fetch

Automatic fetch keeps remote-tracking information current for repositories open in GitComet.

## Settings

In **Settings → Remotes**, turn on **Automatically fetch repositories**. It is off
by default. **Fetch interval (minutes)** starts at **30** and accepts whole numbers
from **1 to 1,440**. Press Enter or leave the field to save a valid value. Invalid
input leaves the saved interval unchanged. The interval is remembered while
automatic fetching is off.

The setting applies to every open workspace window. Enabling it queues a fetch
for repositories already open. Opening or restoring a repository also queues a
fetch, including tabs that have not been selected. Fetches continue while the
app is minimized. Saved workspaces whose windows are closed do not participate.

## Fetch behavior

Automatic fetch uses the same remote selection, refspecs, and pruning preference
as **Fetch all**. It updates remote information without initiating pull, merge,
rebase, or checkout. The existing pruning safeguards for local branches and tags
also apply.

Fetches run one repository at a time. Multiple windows showing the same folder
share one schedule. Linked worktrees keep their own schedules so their Git
configuration is respected. Manual remote commands and repository maintenance
take priority over automatic fetching.

The next attempt is scheduled after the previous attempt finishes, whether it
succeeded or failed. A successful manual **Fetch all** restarts that interval.
Changing the interval updates the schedule. Returning from sleep causes one
overdue attempt rather than replaying missed intervals. Switching tabs or
focusing the app does not request an extra fetch.

Closing the last tab for a folder cancels its automatic fetch. Turning the
setting off cancels queued and running automatic fetches. Fetching stops when
GitComet exits.

## Status and authentication

Repository tabs show **↓N** in place of the repository initials when their
current branch is behind its configured remote upstream, including inactive and
restored tabs that have not been selected. Counts above 99 show as **↓99+**.
Hover the tab to see the branch, upstream and exact incoming commit count. While
a tab shows its loading spinner or missing-folder warning, that takes the place
of the count. Selecting a tab keeps the indicator; it clears when the branch has
no incoming commits. Changes on unrelated remote branches do not add an
indicator.

A grey count is last-known tracking information that may be out of date, for
example after a failed check. A fetch in progress leaves the last count as it
is. The tab tooltip explains its freshness. Unknown counts, detached HEAD and
branches without a remote upstream have no incoming indicator. These indicators
also reflect manual fetches and refreshes when automatic fetching is off.

Open the **Pull** dropdown to see the automatic-fetch result and the last
successful fetch time. A failure includes its reason in the menu; hover a
truncated reason to read it in full. Existing data stays visible when the remote
is unavailable.

Automatic fetch can use stored credentials and unlocked SSH keys. It does not
open sign-in dialogs or interrupt the current task with notifications. If
authentication needs input, use **Fetch all** to complete it. Failed attempts
retry at the next interval.
