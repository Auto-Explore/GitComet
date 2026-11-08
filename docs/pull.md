# Pull

Pull fetches and integrates one upstream branch; Fetch All updates the configured remote refs and performs optional pruning.

## Branch selection

Pull is available when the checked-out branch's upstream is a fetched branch on a
named remote. A branch without an upstream, such as one that was never pushed,
cannot be pulled, even when the remote has a branch of the same name; set its
upstream first. After **Create new**, push the branch before pulling it. URL and
`.` upstreams cannot be pulled from the toolbar, menu, or command palette. When
Pull is unavailable, its tooltip and palette entry say why.

Pull reads the checked-out branch's current `branch.<name>.remote` and
`branch.<name>.merge` configuration when it starts. It passes that repository and
full source ref explicitly to Git, so the local and upstream branch names may differ.

Pull into current uses its selected repository and `refs/heads/<branch>`.
Neither action performs a preliminary Fetch All or pruning fetch. Both disable
pruning even when Git's configuration enables it. Deleted upstreams fail without
retargeting the branch or removing its upstream configuration. Use Fetch All to
refresh other branches and prune stale tracking refs.

Git's tag settings still apply, including `remote.<name>.tagOpt=--no-tags` or
`--tags`. An explicit tag setting can transfer additional data. Pull does not
delete local tags.

Incomplete or ambiguous upstream configuration and detached HEAD fail before
fetching.

The selected merge, rebase, or fast-forward strategy is preserved. Adjusted
git-annex branches retain their configured git-annex routing; see
[Git Annex](git-annex.md).

## Progress and cancellation

Pull shows a progress card immediately, including its repository and branch
context, elapsed time, the last progress phase Git reported, and the latest output
line. Percentages describe individual Git phases, not the whole Pull. Silent work
shows a spinner without an estimated percentage.

**Details** opens the current operation in **Git activity**, where the command
wraps for reading and can be copied. The displayed command includes the actual
Git arguments with URL credentials masked; authentication environment variables
are excluded. Captured output and the completed operation remain available from
the activity button in the status bar, within the existing history and output
limits.

**Stop** cancels the active Git operation through its process group. It is
available in the progress card and details. Each Pull keeps its own progress card
when hooks or large-file transfers run, including its repository and branch context.
Cancellation does not undo work Git has already completed; inspect the repository
state before continuing.

## Timeouts

Git commands time out after 300 seconds without output or progress. Set
`GITCOMET_GIT_COMMAND_TIMEOUT_SECS` in the environment used to launch the app to
override this inactivity limit. Output, Git Trace2 activity, and LFS progress keep
an active command alive; the elapsed-time display does not. A transfer can run
longer than the limit while it continues making progress.

Timeouts are reported separately from cancellation and ordinary Git failures.
Error details retain the command, inactivity reason, and captured output. Git
activity retains the last reported phase and output for diagnosis. A stalled
transport, credential helper, hook, or large-file transfer can still cause a
timeout even when only one branch is pulled. Pull does not retry with a broader
fetch.
