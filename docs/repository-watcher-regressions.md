# Watcher regression coverage

VS Code's watcher tests provide useful scenarios, but exercise several different
services. This comparison inspected the supplied checkout at
`b376c21d3e45b4d4fac2e084efc93696f1a733cd`; it did not run VS Code's test suites.
The GitComet counterparts are independent Rust tests in
`crates/gitcomet-state/src/store/repo_monitor/tests/selective/native_lifecycle.rs`.

## What the VS Code tests establish

Paths below are relative to the VS Code checkout.

| Suite | What it covers | What it does not establish |
| --- | --- | --- |
| `src/vs/platform/files/test/common/watcher.test.ts` | Enabled normalization tests: create/delete cancellation, delete/create becoming update, redundant child deletions, case-only renames, event filters | Native OS delivery or Git refresh behavior |
| `src/vs/platform/files/test/node/nodejsWatcher.test.ts` | Native atomic writes, includes/excludes, symlinks, UNC paths, missing paths and reattachment | The entire suite uses `suite.skip`; its comment cites pipeline hangs/timeouts and on-demand runs |
| `src/vs/platform/files/test/node/parcelWatcher.test.ts` | Recursive coverage, overlapping requests, exclusions, polling and suspension/recovery | Also skipped as a whole for the same reason |
| `src/vs/platform/files/test/browser/fileService.test.ts` | Enabled watch sharing, disposal/refcounts and correlated/global event routing | Kernel registrations or status feedback loops |
| `src/vs/workbench/api/test/browser/extHostFileSystemEventService.test.ts` | Enabled event-ignore flags and case-sensitive/insensitive matching | Filesystem delivery; tests inject events |
| `extensions/vscode-api-tests/src/singlefolder-tests/workspace.watcher.test.ts` | `*.txt` requests shallow coverage and `**/*.txt` requests recursive coverage | Real native watching; it uses a test filesystem provider |
| `src/vs/platform/agentHost/test/node/agentHostFileMonitorService.test.ts` | Enabled pre-debounce filtering of object files, index locks and Watchman cookies | Git extension behavior; this is a separate service |

The Node suite's Windows recreation regression is particularly valuable. It
recreates a watched directory three times, requires raw callback counts to stop
growing after reattachment, then performs CRUD operations and checks that the
watcher remains healthy. Checking only a final file change would miss a stale
registration spinning in the background.

## Git extension behavior

`extensions/git/src/repository.ts::DotGitWatcher` uses the helper in `watch.ts`
with a `RelativePattern(location, '*')`. It watches shallow Git-root entries and
adds a transient upstream-ref watch. The worktree event path filters `.git` out.
Its root filter removes `.git` itself, repository/worktree `index.lock` paths and
Watchman-cookie names. The literal regex does not describe every possible Git
administrative root, such as `.git/modules/<path>/index.lock` or a custom gitdir.

The inspected Git-extension tests do not directly test that regex or a complete
status/event/status loop. Submodule parsing and a smoke test that explicitly
calls status are not evidence of automatic refresh coverage. GitComet therefore
keeps its real-LFS tests with an independent native observer and tests metadata
filtering against discovered root identities.

The agent-host exclusion list is broader than the Git extension's. Excluding all
Git logs, FETCH_HEAD files or lock names would be a behavior change for GitComet,
which refreshes repository metadata as well as source status. Its patterns are
not copied wholesale.

## GitComet counterparts

| Scenario | Assertion |
| --- | --- |
| Recreate a nested tree three times | Each cycle settles at the raw callback level; subsequent create, in-place edit, rename and delete all refresh |
| Replace the repository root three times | Old-tree edits produce no callback; new-tree edits refresh; idle identity checks cover a missing native root-move event |
| Repeat atomic file replacement | In-place edits to each replacement remain visible |
| Case-only directory rename | Edits under the new spelling remain visible |
| Move across ignore boundaries in both directions | Eligible trees acquire coverage and ignored trees stop refreshing |
| Replace an external policy parent repeatedly | No external native callbacks; revalidation updates rules and coverage |
| Metadata noise in main, retained-submodule and retained-worktree roots | Cache files, index locks and Git-root Watchman cookies produce no refresh; HEAD, index and source edits still do |
| Mixed metadata/source event | Dropping noise and parent timestamp echoes preserves the real index/worktree change |
| Overlapping recursive roots | Covered descendants share a root; a sibling with the same string prefix is retained |
| Empty worktree-relative path | Root lifecycle events never fail an ignore lookup |
| Git-directory lifecycle and overflow | Timestamp filtering preserves create/remove/rename and rescan signals |
| Windows UNC spelling | Verbatim UNC paths retain cache and repository boundaries; another share remains outside |

The raw counter is local to the test's `MonitorConfig` and runs before callback
filtering. It observes real native callbacks without process-global hooks and is
compiled out of production builds. Native scenarios remain enabled on supported
platforms. The UNC test is a path-classification test, not a live SMB-server test.

Two production gaps were exposed by these scenarios: Git-root Watchman cookies
and their Windows parent timestamp echoes could request another refresh, and
Linux root lifecycle events could send an empty relative path to the ignore
stack. The tests preserve their intended fixes without suppressing root
replacement, index/ref changes, or similarly named source files.

GitComet publishes debounced repository-state categories, not per-file create,
update and delete lists. A transient create/delete may conservatively request
one worktree refresh. VS Code's exact event-list cancellation and extension
correlation/refcount contracts are therefore not imposed on this monitor. The
existing debouncer and monitor-manager tests cover GitComet's own contracts.

Run `cargo test -p gitcomet-state native_lifecycle --locked` for these scenarios,
or `cargo test -p gitcomet-state store::repo_monitor --locked` for the complete
watcher suite. Use an isolated Git configuration and run the native suite on
each OS; cross-compilation alone cannot validate native macOS delivery.
