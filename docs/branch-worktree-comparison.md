# Branch and Worktree Comparison

Branch and worktree rows compare committed history with a locally available target without changing upstream tracking indicators.

## Target and counts

The default target is the ref named by the locally recorded `refs/remotes/origin/HEAD`. No branch name is guessed when that symbolic ref is missing. The Local Branches and Worktrees section menus offer an explicit comparison ref, a return to `origin/HEAD`, and a deeper check. The override is a shared repository preference persisted across worktrees and windows.

The heading names the target. Row counts are exact ancestry counts: ↑ ahead and ↓ behind. A squash or rebase patch match can leave nonzero counts. Worktrees use their recorded HEAD, including detached checkouts. Tips shared by several branches or worktrees share analysis.

Comparison inputs are captured OIDs. A shared `rev-list --topo-order --parents --stdin` walk propagates per-seed reachability, excluding ancestry proven common to every seed. Exact numbers are published only after the walk completes successfully. Shallow, replacement, grafted, missing-object, interrupted, and budget-limited histories cannot establish exact results.

## Scheduling and display

Analysis runs outside listing and rendering on a shared pool with two workers and one active job per repository. The active branch and visible subjects precede other worktree HEADs and remaining local branches. Graph counts run before deeper evidence searches. Automatic count batches contain at most 128 distinct subjects and have a five-second deadline. A deeper check allows 30 seconds.

The first result publishes immediately, then publications are spaced by at least 100 ms. Results update immutable decoration pages without rebuilding the structural sidebar. Refreshes hide confirmation until accepted snapshots agree. Generation and repository-lifetime checks reject stale completions. An unchanged accepted refresh reuses existing results.

The backend shares caches through its repository and object-store identity boundary, which includes filesystem identity. Results are keyed by immutable subject and target, graph context, analysis depth, and algorithm version. Closing a repository releases its queued work and cache ownership.

## Integration evidence

The integration-evidence and explicit worktree-observation slices are under implementation. Counts currently establish Up to date for equal tips and Merged when the subject has no commits ahead. Committed evidence is independent of uncommitted files and does not change deletion or pruning behavior.

## Validation

Backend tests compare the shared walk with independent immutable-OID `rev-list --left-right --count` calls, including skewed commit dates, nested tips, merges, disconnected roots, and all 128 membership bits. Reducer tests cover stale generations, repository replacement, immutable publications, structural revisions, and unchanged refresh reuse.

Large-scale latency, frame cost, graph reads, cache reuse, memory limits, and release measurements remain a release gate while this feature is under implementation.

The Git behavior used here is documented in [git-rev-list](https://git-scm.com/docs/git-rev-list) and [git-patch-id](https://git-scm.com/docs/git-patch-id).
