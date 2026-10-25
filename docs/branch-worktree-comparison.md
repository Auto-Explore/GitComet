# Branch and Worktree Comparison

Branch and worktree rows compare committed history with a locally available target without changing upstream tracking indicators.

## Target and counts

The default target is the ref named by the locally recorded `refs/remotes/origin/HEAD`. No branch name is guessed when that symbolic ref is missing. The Local Branches and Worktrees section menus offer an explicit comparison ref, a return to `origin/HEAD`, and a deeper check. The override is a shared repository preference persisted across worktrees and windows.

The heading names the target. Row counts are exact ancestry counts: ↑ ahead and ↓ behind. A squash or rebase patch match can leave nonzero counts. Worktrees use their recorded HEAD, including detached checkouts. Tips shared by several branches or worktrees share analysis.

Comparison inputs are captured OIDs. Equal subject and target OIDs establish an immediate up-to-date result after target validation. Other inputs use a shared `rev-list --topo-order --parents --stdin` walk to propagate per-seed reachability, excluding ancestry proven common to every seed. Exact numbers are published only after that walk completes successfully. Shallow, replacement, grafted, missing-object, interrupted, and budget-limited histories cannot establish exact walk results.

## Scheduling and display

Analysis runs outside listing and rendering on a shared pool with two workers and one active job per repository. The active branch and visible subjects precede other worktree HEADs and remaining local branches. Graph counts run before deeper evidence searches. Automatic count batches contain at most 128 distinct subjects and have a five-second deadline. A deeper check allows 30 seconds.

The first result publishes immediately, then publications are spaced by at least 100 ms. Results update immutable decoration pages without rebuilding the structural sidebar. Refreshes hide confirmation until accepted snapshots agree. Generation and repository-lifetime checks reject stale completions. An unchanged accepted refresh reuses existing results.

The backend shares caches through its repository and object-store identity boundary, which includes filesystem identity. Comparison subprocesses discard inherited repository-selection, object-store, namespace, and per-command configuration overrides before applying their fixed analysis settings. Results are keyed by immutable subject and target, graph context, analysis depth, and algorithm version. Closing a repository releases its queued work and cache ownership.

The shared memory authority limits comparison reservations to 32 MiB per repository and 128 MiB globally. Reservations cover captured row projections, queued subject sets, coordinator results, active backend buffers, retained backend caches, publication buffers, and immutable decoration pages. Snapshots retain their own reservations until the last reader releases them. Each backend job reserves up to 16 MiB, assigns one quarter to application buffers and the remainder to its Git process, and leaves 1 MiB of repository headroom for control messages. Jobs that cannot reserve at least 8 MiB defer. Completed backend cache entries can be evicted to make room for analysis.

The process supervisor samples resident memory on Unix and private committed memory on Windows while enforcing cancellation and elapsed deadlines. A process above its quota is killed and reaped immediately; its unfinished walk supplies no exact numbers. Sampling runs at approximately 1 ms intervals and can overshoot between observations; it is not an operating-system memory sandbox. Scale probes record the observed Git peak separately from reservations, and strict process-memory bounds remain a release gate.

Exhaustion yields an unavailable or deferred comparison without repeated automatic retries. The queue admits at most 256 subscribers, and each accepted projection contains at most 65,536 combined local-branch and worktree rows. Retained row OID allocations are charged even when their values share a tip.

Visible subjects precede other worktree HEADs in both analysis and queued publications. Malformed or incomplete backend batches become unconfirmed results without automatic retries. A refresh with an unfinished worktree-list load waits for that snapshot before deciding whether comparison inputs changed.

## Integration evidence

Evidence runs after counts, in this order:

| Label | Evidence |
| --- | --- |
| Up to date | The subject and target identify the same commit. |
| Merged | The subject's history is reachable from the target. |
| Changes match | Every path changed since the single merge base has the same final object and exact mode, or deletion, in the target. |
| Equivalent history | An ordinary target commit matches the aggregate branch patch, or every unique ordinary branch commit has a distinct patch match. The tooltip names the route. |
| No net changes | The subject's tree has no final changes relative to its merge base. |
| Not confirmed | The available checks did not establish incorporation, or analysis reached a limit. |

Content comparisons include binary objects, rename source deletions, modes, symlinks, and gitlinks. Multiple merge bases prevent this content check. Unique merge commits prevent individual-commit proof; aggregate or content evidence can still succeed. Empty ordinary commits cannot supply patch evidence.

The shared target-history index incrementally examines up to 1,000 target commits automatically, or 10,000 during a deeper check. Diff settings are pinned, external diff and text conversion are disabled, and patch IDs use `--verbatim` to preserve whitespace. Exact inverse patches later in the examined target history cancel older matches. Historical matches describe historical patches; they do not assert current content, identical history, or safe deletion. Edited squashes, conflict resolutions, edited reverts, partial incorporation, and later rewrites may remain unconfirmed.

Worktree rows independently show Clean, Dirty, or Unknown from filesystem status scans. Every scanned linked checkout produces an explicit observation, including clean and inaccessible checkouts. Missing observations cannot establish cleanliness. Scan identities prevent older full or targeted results from replacing newer observations. Failed observations retain the last dirty file list and an open diff until a successful scan establishes cleanliness or a successful complete listing removes the checkout. The active checkout reuses its existing staged and unstaged status lanes without another scan or uncommitted-history row. Recorded HEAD changes invalidate cleanliness independently of comparison-target availability.

Filesystem scans use their existing separate worker pool and checkout-handle cache. Comparison analysis adds no checkout watchers. Committed evidence remains independent of uncommitted files and does not change deletion or pruning behavior.

## Validation

Backend tests compare the shared walk with independent immutable-OID `rev-list --left-right --count` calls, including skewed commit dates, nested tips, merges, disconnected roots, and all 128 membership bits. Reducer tests cover stale generations, repository replacement, immutable publications, structural revisions, and unchanged refresh reuse.

Build and run the isolated accepted-snapshot probe with:

```bash
cargo build --release -p gitcomet-state --example comparison-probe --features benchmarks,test-support
python3 scripts/bench-ref-comparisons.py --probe target/release/examples/comparison-probe
```

The generator owns a fresh directory under `target/comparison-scale`. It creates one million skew-dated commits, 20,000 local branches, and 5,000 worktrees, then runs shared and distinct tips with commit graphs disabled and enabled. The four-case driver requires Linux process handles for process-tree ownership and cleanup. `--commits`, `--branches`, and `--worktrees` allow small feasibility probes. Omitting `--probe` generates fixtures without running analysis and works on other platforms. Fixture commands discard inherited Git overrides, disable global/system configuration, and initialize without templates. The Rust probe can also read an existing repository with `comparison-probe REPO [--deep]`; it sanitizes Git overrides, isolates user settings, and disables repository monitors and session persistence. Cross-platform performance validation remains a release gate.

JSON-line records separate input-listing time from first-visible and full comparison time after snapshot acceptance. They report exact and unavailable counts, completed graph rows, completed jobs, cache hits, publication counts, warm local-branch refresh work, reservation peaks, supervised Git memory peaks, and release after closure. Warm records require a settled cold comparison and successful branch metadata and target; otherwise the measurement is skipped or unavailable. They exercise `RefreshBranches`, not a full metadata refresh. Up to eight exact results are checked against independent immutable-OID count commands after the measured cold phase. Linux sampling additionally records Git and probe RSS; independent count-oracle processes are excluded. Samples are lower bounds and include mapped/shared pages, so they cannot be added directly to reservations or treated as allocation totals. Run the tooling regressions with `python3 scripts/test_bench_ref_comparisons.py`.

The presentation benchmark also includes `branch_sidebar/20k_local_5k_worktrees_shared` and `branch_sidebar/20k_local_5k_worktrees_distinct`, alongside the existing remote-branch fixture. Large-scale throughput and strict native-memory bounds remain failing release gates: the million-commit distinct-tip probe defers walks under the configured budget, and sampled native memory can exceed its quota between observations. A completed deferred batch is not successful exact-count coverage.

The [2026-10-25 release-profile capture](benchmarks/ref-comparison-2026-10-25.json) records the small and million-commit probes, machine/tool versions, independent oracle coverage, reservations, RSS, and zero ownership after closure. These are single feasibility runs with warmed filesystem caches. Shared large tips produced an exact result in about 15 ms; distinct tips produced the identity result in about 21 ms and completed deferred publications in about 15.9 seconds. All 19,999 other distinct tips remained unavailable. Warm branch-only refreshes started no comparison jobs or publications. The native supervisor observed peaks above its quota, and 10 ms external sampling caught still higher short-lived peaks. An additional run refused target resolution under the budget; the unavailable-target path is recorded separately from valid warm measurements. Frame cost and cross-platform performance are outside this capture.

The [native UI capture](benchmarks/ref-comparison-frames-2026-10-25.json) measures synthetic 20,000-local-branch/5,000-worktree snapshots with native fonts and a headless GPU. Before timing, it asserts the actual cached list contains all 25,000 items and the expected active branch. Comparison publications retained that exact structural row allocation. Twenty distinct-tip publications averaged about 16.7 ms from snapshot application through draw, with a maximum of 18.3 ms; decoration publication peaked at about 0.114 ms. These elapsed test-profile measurements include harness pumping and exclude backend work, full reducer/IPC cost, and screenshot readback. The UI crate used optimization level zero, so this capture does not establish the release frame gate.

Run the opt-in Linux capture with `cargo test -p gitcomet-ui-gpui --lib --features benchmarks --config 'profile.test.package.gitcomet-ui-gpui.opt-level=0' comparison_rows_native_capture_and_publication_frames -- --ignored --nocapture`. It writes screenshots and measurements under `target/ref-comparison-preview`, including narrow/wide panes, 150% scale, light theme, pending targets, and unavailable targets. Automated geometry checks preserve a readable identity region and prevent overlap at 100% and 150% scale. Full status details remain in tooltips when supporting labels truncate.

The Git behavior used here is documented in [git-rev-list](https://git-scm.com/docs/git-rev-list) and [git-patch-id](https://git-scm.com/docs/git-patch-id).
