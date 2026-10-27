# Pull

Pull fetches and integrates one upstream branch; Fetch All updates the configured remote refs and performs optional pruning.

## Branch selection

Pull reads the checked-out branch's current `branch.<name>.remote` and
`branch.<name>.merge` configuration. It passes that repository and full source
ref explicitly to Git. The local and upstream branch names may differ, and the
repository may be a named remote, a URL, or `.` for another local branch. A
missing tracking ref does not change which upstream is pulled.

Pull into current uses its selected repository and `refs/heads/<branch>`.
Neither action performs a preliminary Fetch All or pruning fetch. Both disable
pruning even when Git's configuration enables it. Deleted upstreams fail without
retargeting the branch or removing its upstream configuration. Use Fetch All to
refresh other branches and prune stale tracking refs.

Git's tag settings still apply, including `remote.<name>.tagOpt=--no-tags` or
`--tags`. An explicit tag setting can transfer additional data. Pull does not
delete local tags.

When neither upstream configuration key exists, Pull uses `origin`, or the first
configured remote by name, and the current branch's name. Its fetch refspecs must
map that branch to exactly one tracking destination that no other source or remote
claims. This check includes positive mappings even when a negative refspec excludes
their source, matching Git's upstream setup rules. After successful integration,
Git sets the upstream using that destination, including Git's `branch.autoSetupRebase`
behavior. Incomplete or ambiguous upstream configuration and detached HEAD fail
before fetching.

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
Cancellation does not undo
work Git has already completed; inspect the repository state before continuing.
