# `speed_up_ci` review follow-up

Checked against the local `speed_up_ci` branch on 2026-09-23. The numbers below
correspond to the supplied review. Findings were checked in the implementation
and, for the process harness, the installed `process-wrap` 10.0.0 sources.

| Item | Verdict | Resolution |
| --- | --- | --- |
| 1. Windows launch cleanup hangs | Valid. `JobObjectChild::try_wait` consumes a completion-port message without caching the job completion; `wait` subsequently performs an unbounded receive. | Poll the inner leader on Windows, preserving the job notifications for cleanup. Added a Windows test that repeatedly polls a naturally exited child and requires cleanup to finish within a deadline. |
| 2. Windows identity checks block saves | Valid. The sharing flags explicitly deny writers, and the old test required this behavior. Brief acquisition still allows an editor save to fail. | Removed the Windows identity optimization. Windows verifies bytes on cache reuse, using ordinary file reads that allow sharing. Added tests for in-place saves, replacement/deletion, and same-length edits with an open writer. |
| 3. Search snapshots copy caches | Valid for eager caches and projections. The statement that every diff is duplicated overstates it: paged providers were already held in `Arc`. | Share eager row caches, visible indices, collapsed/wrapped rows, and inline mapping runs through `Arc<[T]>`. A snapshot test checks shared ownership and that replacing live caches leaves the old search document intact. |
| 4. Worker setup also scans synchronously | Valid when the visible projection is stale. Its rebuild called the synchronous search recomputation. | Added a projection-only preparation path for worker startup. The asynchronous search regression now starts with a matching query and a deliberately stale projection, and asserts that scheduling does not publish synchronous matches. |
| 5. Resize invalidates unwrapped search | Valid. Both window dimensions were unconditional document-key fields. | Removed the raw viewport dimensions. Actual wrap-plan keys still invalidate searches when visual rows change. Added coverage for both cases. |
| 6. Ordinary staged paths broaden gitlink status | Valid within repositories that need supplementation. Repositories without gitlink capability already bypass this query. | Include current index gitlinks and staged paths whose HEAD entries were gitlinks. Added coverage for ordinary staged files, removed/replaced/retained gitlinks, and an empty selection after removal. |
| 7. Store performs quadratic checks under its write lock | Valid. Every task token searched the repository vector and locked its selection slot. | Index selections once outside the write lock. Unchanged selections and tokens without selected-diff work skip the mutex. Coverage checks cancellation on target/revision changes and clearing, and verifies the unchanged-selection path without releasing a held task mutex. |
| 8. Tests automatically search twice | Valid for synchronous scanner calls, rather than every background search. The second scan used the worker implementation but ran synchronously too. | Removed the implicit parity scan from the production search method. Dedicated query-semantics, cancellation, snapshot, and asynchronous UI tests provide targeted coverage. |
| 9. Windows tests assume NTFS and sleep | Valid. The tests required an identity even though production could reject the filesystem/journal, and slept three times for 2.1 seconds. | Replaced them with sharing/content-verification tests without NTFS/USN assumptions or sleeps. These exercise the supported fallback on every Windows volume. |
| 10. Interrupted reads fail | Valid, also in `copy_and_hash`. | All three chunked read paths retry `Interrupted`, checking cancellation between retries. Added an interrupted-reader regression that also cancels during retry. |
| 11. Probe flushes only on quit and silently loses overflow | Mostly invalid. The writer already flushed after each batch, and JSON interval records already included the cumulative `records_dropped` count. An abrupt process exit can still lose the queued or currently buffered tail. | Retained bounded asynchronous logging and its bounded quit flush. Added live-flushing and counted-overflow tests. This diagnostic stream remains best-effort on abrupt termination. |
| 12a. Navigation code is unreachable | Valid for the pending-query flush calls: the preceding pending-result guard always returns first. The subsequent empty-result branch itself remains reachable. | Removed the dead flush calls and unused flush helper; preserved empty-result recomputation and navigation. |
| 12b. Test comments are stale | Valid. Unix can trust a cache file it just created; it does not need the second read to age it. | Updated the two comments. |
| 12c. Inline dependency declarations | Valid as manifest cleanup. The inline rustix declaration also requested default features. | Moved process-wrap to workspace dependencies and inherited workspace rustix settings. |
| 12d. UI appdata directories leak | Valid on Windows. The per-run UUID directory had no cleanup owner. | Own it through `TemporaryDirectory` and an `ExitStack` surrounding suite execution. Added tests for success, nonzero exit, timeout, and interruption. |

The Windows cache fix deliberately gives up metadata-only memo hits. Adding
`FILE_SHARE_WRITE` alone would make the existing memo unreliable: Windows can
defer file timestamps until writers close. See Microsoft's
[file time guarantees](https://learn.microsoft.com/en-us/windows/win32/sysinfo/file-times).
The content-addressed cache and full content verification remain in use.

Native Windows runtime tests must run on Windows CI; cross-compilation on Linux
does not prove job-object or file-sharing behavior at runtime.

Validation on Linux:

- Backend library: 302 passed, 5 existing ignored tests. Run with
  `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 cargo test -p gitcomet-git-gix --lib --offline`
  to keep fixture commits independent of personal signing configuration.
- State library: 1,027 passed with `cargo test -p gitcomet-state --lib --offline`.
- Targeted UI tests: 265 passed, covering `diff_search`, `background_search`,
  `search_snapshot`, `ui_probe`, `file_diff::`, `diff_wrap`, and `collapsed_diff`.
- Perf launch harness: 36 passed with
  `cargo test -p gitcomet --bin perf-app-launch --offline`.
- CI runner: 57 passed, 1 existing Windows-only test skipped, using
  `python3 -m unittest discover -s scripts/ci -p test_ci.py`.
- Backend and its tests cross-check for `x86_64-pc-windows-gnu`.
- The application and perf-harness tests also cross-check for Windows with
  `GPUI_RENDER_ALLOW_MISSING_DXBC=1 cargo check -p gitcomet --bin perf-app-launch --tests --target x86_64-pc-windows-gnu --offline`.
  GPUI requires that option for check-only Windows builds on a Linux host;
  this does not build runnable Windows shaders or execute Windows tests.
- `cargo fmt --all -- --check` and `git diff --check` pass.
