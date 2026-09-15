# Repository watching

Read-only Git status can invoke the LFS clean filter even when a file's contents
are unchanged: its modification time may differ from the index's cached stat.
The filter creates and removes temporary files in LFS storage. Refreshing status
in response to those writes produces a feedback loop in which repository loads
never drain.

## Discovery and cache boundaries

`GitBackend::repository_watch_info` owns discovery of Git/common directories,
initialized submodules, indexed gitlinks without `.gitmodules`, linked worktrees,
and retained administrative repositories. An absent first index is an empty
snapshot; an unreadable or corrupt index remains an error.

Git `objects/` and `lfs/` directories are private caches. Configured `lfs.storage`
also contributes its `objects/`, `tmp/`, `logs/`, and `incomplete/` directories.
Only those owned subdirectories are excluded: storage may share a parent with
source or Git metadata. LFS paths are cleaned lexically before resolving links,
matching Git LFS's path handling. Explicit policy paths preserve configured
spellings, link targets, and filesystem-resolved identities, including Windows
casing and existing parents of missing inputs.

## Platform strategy

| Platform | Native coverage | Ignored directories |
| --- | --- | --- |
| Linux | Shallow inotify registrations, parents registered before enumeration | Pruned before registration; new eligible trees are added incrementally |
| Windows | Recursive `ReadDirectoryChangesW` on minimal repository roots | Events are filtered against the current policy |
| macOS | One FSEvents stream per minimal repository root | Up to eight native exclusions per stream, caches first, then shallow ignore boundaries; remaining events are filtered |

Windows uses recursive roots because a native regression demonstrated that shallow
handles on descendants can deny renaming the watched directory. The regression
also exercises deletion/recreation, `git mv`, and checkout. macOS does not use
kqueue or partitions. `notify` comes from the registry without a local patch.
The small FSEvents crate owns the FFI, contains callback panics, drains callbacks
before releasing streams, and enforces a process-wide file-descriptor budget.

## Policy and coverage updates

Callbacks and the monitor share immutable policy snapshots and one triage
function. Classification uses hash sets and an allocation-free ancestor walk,
with caches taking precedence over control inputs, indexes, Git metadata,
ignored boundaries, and worktree paths. Access-only reads are dropped; overflow,
rescan and error signals survive filtering. Git-directory timestamp echoes are
dropped: cache and Watchman-cookie writes can produce those parent notifications
on Windows. Actual HEAD, index and ref events remain observed. Replacing a Git
directory, or editing a `.git` file, changes policy. Watchman cookies directly
inside discovered Git roots are also private metadata; similarly named source
files and refs remain observable.

A stable setup loads the backend once and scans each eligible directory once.
Shallow registration precedes enumeration. Input and index stamps bracket setup;
changes retry setup within a bounded pass limit. Traversal keeps useful breadth
first coverage when it reaches its directory budget.

Ordinary commits and staging do not replace watches. At a debounced flush, policy
changes rebuild coverage; an index change reloads matching rules once and only
rebuilds if repository discovery changed or a previously ignored boundary now
contains a tracked exception. Every actual replacement refreshes all repository
state to reconcile its registration gap. Newly ignored directories update the
policy immediately; their creation and removal do not cause full rebuilds.
Removed directory boundaries are pruned from the plan and callback policy.
Full reloads rediscover nested `.gitignore` inputs; only index reloads retain them.

Overflow, rescans and native errors rebuild coverage and exclusion policy on all
platforms, since lost events can leave stale boundaries even with intact native
roots. A degraded warning describes failed
coverage, rather than an isolated callback error. Recovery is throttled, and a
failed reload retains the last usable matcher. Failed path lookups are never
cached. Trace records `repo_monitor_reload scope=`, `repo_monitor_rebuild reason=`,
and `repo_monitor_flush` make the cost of these decisions observable.

## External inputs and limits

External config/ignore files and symlink hops are stamped, without native watches
on home directories. Stamps include existence, size and timestamps, link targets,
and Unix inode/ctime. Directory stamps track identity rather than entry changes.
Inputs are revalidated on repository activation, before debounced flushes, and
on the existing 30-second idle tick. In-repository policy files also retain native
coverage. An unchanged failed input waits for throttled recovery; a new version
can trigger immediate recovery.

Defaults are a 250 ms debounce, a 2-second maximum burst delay, three setup passes,
4,096 worktree directories plus a separate metadata budget, and a 120-second
recovery interval. Monitoring is best effort; activation also refreshes repository
state independently of watcher delivery.
Root identity stamps also recover when a backend omits a notification for moving
the watched root itself. Without another event or activation this uses the idle
tick, so detection can take up to 30 seconds.

## Verification

Run Cargo from PowerShell on Windows so the MSVC linker receives its normal
environment. Use an isolated Git configuration for reproducible tests.

```text
cargo test -p gitcomet-state store::repo_monitor --locked
cargo test -p gitcomet-git-gix --locked
cargo test -p gitcomet-fs-watch --locked
cargo clippy -p gitcomet-state -p gitcomet-git-gix -p gitcomet-fs-watch --all-targets -- -D warnings
cargo fmt --all --check
```

Run the native suites on Windows, Linux and macOS. Cross-compiling the macOS test
target checks platform code but does not replace native execution. CI installs
Git LFS and runs the workspace tests on all three systems. The real-LFS tests use
an independent observer to prove cache churn occurred while GitComet stayed quiet,
and then verify that a real source edit still refreshes the repository.

[Watcher regression coverage](repository-watcher-regressions.md) explains the
additional lifecycle tests adapted from VS Code and the differences in contracts.
