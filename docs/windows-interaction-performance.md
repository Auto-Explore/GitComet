# Windows interaction and LFS performance

This implements the interaction investigation on `speed_up_ci`. The branch
comparison is `dev` (`9e4c75aa2fa870061d18023399bb812f23843e16`). Before/after
measurements isolate the changes in this investigation against the original
branch HEAD, `0cf90ae1d5c7d7f73c4820ffe0e4af6bbd9d2712`; earlier branch gains are
not counted again. `main` is not a performance baseline.

The GPUI dependency remains at `9ff6e7f487f9cba1a8d5a1fc45aa173d094ff9e7`.
The separately prepared window-loop patch is deferred for later testing.

## Changes

- **Diff search:** capture an immutable document and search it on the background
  executor. Keep one active search and one replaceable pending query; cancel
  obsolete work and check both generation and document identity before applying
  results. Reuse bounded ASCII query-result candidates, including broader
  queries when backspacing, without building an eager document index. Preserve
  regex, Unicode, whole-word, multiline, Markdown, wrapped-line, editor and
  conflict navigation semantics. Search option changes and edits use this path
  too. The previous 150 ms typing debounce is removed.
  The candidate cache holds at most eight queries, each limited to 16,384 matching
  rows and 4,096 query bytes. Cancelled searches cannot populate it.
  Editor refreshes retain their current-match anchor across successive edits;
  queued Next/Previous actions apply relative to that anchor and wrap normally.
- **Text inputs:** notify filter subscribers only when content changes. Focus,
  caret movement and selection no longer recreate filter strings and recompute
  them. Diff search compares a shared text snapshot before materializing text.
- **Diff scheduling:** cancel work when its selected target is replaced or
  cleared. Each result kind has one replaceable queued request. Prioritize the
  patch and text sources, with optional previews following them. Check the
  target again before publishing. Streamed copies, hashing, image reads and
  owned Git children check cancellation.
- **Windows source reuse:** use a short-lived NTFS handle identity, file ID,
  volume ID, size, timestamps and journal version to reuse already verified
  source files. Exclude current writers, reparse points, network paths, external
  filters, and files changed within the last two seconds. Before verifying
  content for reuse, force a USN close record to separate future writes from
  earlier journal reasons. Cache hits only query the version. The cached output
  must also have an aged identity captured before verification. Unsupported
  paths or journal access use the existing verification path. Blob verification
  hashes 64 KiB chunks instead of loading another complete copy into memory.
- **Git/LFS progress:** give buffered activity output a deadline 100 ms after its
  first byte, or at the byte limit. Continuous small writes previously restarted
  the timeout and could hide progress until the operation ended.
- **Measurement:** record text revisions and search-result witnesses through
  rendering and submission. A bounded background writer removes synchronous
  JSONL I/O from the UI thread. Reject missing witnesses or dropped probe records.
  Add disposable backend and real loopback LFS fixtures with content hashes.

Cancellation cannot interrupt a gix blob decompression or an external filter's
blocking read in the middle of that call. Checks around those calls and bounded
queues limit obsolete work, but do not promise immediate cancellation there.
Some view/projection refresh paths still use the existing synchronous scanner.

An earlier background-search prototype built an eager trigram index. Reviewing
the first query separately exposed a debug regression from 237 to 721 ms despite
fast repeated queries. It was replaced by a single initial scan and the bounded
query cache. The harness now requires a first-query result witness, and regression
tests cover single-pass initial work, refinement and backspacing.

A second rejected shortcut used NTFS timestamps without a journal boundary.
On this machine, mapped writes preserved both modification and change times.
Reading the file's USN alone also missed repeated writes while another reader
stayed open, because reasons coalesced in the journal. The final implementation
records the journal identifier and seals a close record before verification;
the next write gets a new version. The OS regression test repeats that sequence
with a persistent reader and verifies that unchanged cache checks stay stable.
It neither enables a journal nor requests elevated access.

## Evidence and reproduction

Machine: Windows 10 build 19045, Ryzen 5 3600 / 12 logical CPUs, Radeon RX 5700 XT,
1920 x 1080 at 60 Hz, Python 3.14.7, Rust 1.98.1, Git for Windows 2.53.0 and
Git LFS 3.7.1. These measurements do not establish Linux/macOS parity or hosted
CI speedups.

### Measured backend results

Six alternating pairs in two sessions, five warmups and 35 retained samples per
invocation. Values below are medians of the six run medians; the p95 column
compares the medians of each run's raw-sample p95. Every old/new source hash was
verified outside the timer. All source-load cases pass the 10% improvement and
5% p95 regression gates.

| Warm source load | Debug before / after | Release before / after | Release p95 reduction |
| --- | ---: | ---: | ---: |
| 32 KiB | 3.69 / 0.84 ms | 1.46 / 0.54 ms | 63.1% |
| 1 MiB | 64.20 / 0.84 ms | 4.56 / 0.55 ms | 88.5% |
| 16 MiB | 1,003.27 / 0.84 ms | 58.11 / 0.54 ms | 99.0% |

These are repeated loads of unchanged, aged files. Initial loads, recent edits,
LFS/external-filter normalization and unsupported filesystems still require
verification. They are not whole-diff rendering or cold-start timings.
These results use the final journal-aware binaries, in
`backend-journal-{debug,release}-{first,second}/report.json` and
`backend-journal-comparison.json`. The earlier timestamp-only source-load
measurements are superseded. Only the affected `diff` cases were repeated;
the unchanged status/progress controls below retain their original runs.

The status control remained approximately 1,045 ms in debug; release changed
from 46.04 to 47.07 ms (p95 +2.0%, within the guard). This fixture forces status
to examine all three changed files. Source reuse does not accelerate that work.

Across six real pre-push hook invocations, median first-progress notification
fell from 1,381 to 261 ms in debug and 1,415 to 279 ms in release (about 80%).
Total command duration changed by less than 1%. This fixes delayed progress,
without claiming faster Git transfer throughput. Raw data and hashes are in
`backend-{debug,release}-{first,second}/report.json` and
`backend-comparison.json` under the artifact directory below.

### Measured UI results

Six alternating pairs in two sessions per profile; all 24 captures passed the
focus, text revision, query result and dropped-record checks. Values are medians
of the per-capture p50/p95 measurements, in milliseconds, from application
handling to submission of the witnessed window.

| Interaction | Debug p50 before / after | Debug p95 before / after | Release p50 before / after | Release p95 before / after |
| --- | ---: | ---: | ---: | ---: |
| Branch filter typing | 11.57 / 10.92 | 18.57 / 18.52 | 8.74 / 8.99 | 16.82 / 16.55 |
| Commit typing | 10.88 / 10.85 | 18.22 / 17.77 | 9.54 / 9.47 | 17.08 / 17.07 |
| Search results | 210.25 / 20.16 | 250.84 / 25.01 | 174.86 / 13.59 | 185.25 / 21.40 |
| Search field text | 11.92 / 20.25 | 25.08 / 25.11 | 11.25 / 13.61 | 21.56 / 20.68 |
| Click acceptance | 20.81 / 22.05 | 28.55 / 29.32 | 15.04 / 15.64 | 22.14 / 22.79 |

Search-result p95 improves by 90.0% in debug and 88.4% in release. Process CPU
during the search phase falls by 68.5% and 38.9%, respectively. Ordinary typing,
click acceptance, search-field text and CPU draw p95 stay within the 5% regression guard. These
measurements do not establish a general click/typing speedup.

First queries are measured separately before priming the search cache. There
are six initial queries per variant/profile; these are medians, not tail estimates.

| First query | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Debug | 236.12 ms | 121.43 ms | 48.6% |
| Release | 173.12 ms | 19.26 ms | 88.9% |

The search-field text has a trade-off: its median submission moves later as
text and new results are rendered together (debug +8.33 ms, release +2.36 ms).
Both text and result witnesses are retained; raw text-paint latency must not be
presented as an improvement. The search results themselves arrive much earlier.

Comparisons are in `ui-v2-{debug,release}-comparison.json`, with complete
per-capture counts, frame data and session provenance under
`ui-v2-{debug,release}-{first,second}/`. Boundary/coalesced actions without a
matching submission are counted, not assigned artificial zero latencies.
Earlier `ui-{debug,release}-*` captures retain the rejected eager-index prototype
for audit and are excluded from these results.

Artifacts are under `target/ci-reports/windows-interactions/`. Freeze binaries,
record SHA-256 hashes and finish builds/tests before measuring. Use independent
disposable repositories, not linked worktrees. The harness isolates session
preferences and Git configuration, owns the app's child processes, and restores
pointer/focus/environment after native input scenarios.

### LFS screening results

All 80 timed CLI transfers passed object-count and SHA-256 checks. This is a
one-round screen, not evidence for changing a global default. Selected 8/16
worker results are below; the full 1/2/4/8/16 sweep is in
`lfs-screen-comparison.json` and each `lfs-screen-*/report.json`.

| Shape | Delay per object | Upload, 8 / 16 workers | Download, 8 / 16 workers |
| --- | ---: | ---: | ---: |
| 60 x 1 MiB | 0 ms | 364 / 606 ms | 543 / 562 ms |
| 512 x 64 KiB | 0 ms | 661 / 640 ms | 776 / 813 ms |
| 4 x 64 MiB | 0 ms | 486 / 1,720 ms | 478 / 436 ms |
| LFS + 4,000 plain files | 0 ms | 565 / 575 ms | 600 / 809 ms |
| 60 x 1 MiB | 50 ms | 749 / 684 ms | 774 / 700 ms |
| 512 x 64 KiB | 50 ms | 3,897 / 2,300 ms | 4,086 / 2,533 ms |
| 4 x 64 MiB | 50 ms | 507 / 576 ms | 540 / 526 ms |
| LFS + 4,000 plain files | 50 ms | 745 / 679 ms | 1,045 / 970 ms |

Sixteen workers help the delayed dense fixture, but the zero-delay results
include regressions and noise. Keep existing production transfer settings.
Repeat any promising setting against the actual LFS server before promoting it;
loopback does not reproduce that server's bandwidth or throttling.

The candidate backend also passed status, normalized-pointer diff, stage,
checkout and real push checks on the mixed fixture. Four initial UI captures
completed verified 512-object uploads while accepting 80 edits each; 76 text
submissions per capture remained within the measurement boundaries. The typing
p95 values were 17.72 / 17.62 ms in debug and 16.84 / 16.65 ms in release
(baseline / initial candidate). These single-pair checks establish operation
and responsiveness under this controlled load, without claiming a latency gain.
Two additional captures with the corrected search implementation also passed
the same 512-object/80-edit checks, with 76 submitted text revisions each.
Typing p95 was 17.67 ms in debug and 16.92 ms in release. These are in
`lfs-ui-v2-screen.json`; they precede the later NTFS cache-identity correction.

Generated LFS repository/server payloads were removed after verification to
recover 2.77 GiB. Request records, object hashes, transfer results and the cleanup
manifest are retained; the deterministic scripts recreate the payloads.

### Build the baseline with identical measurement hooks

```powershell
python scripts/ci/prepare-interaction-baseline.py --revision 0cf90ae1 --output C:/perf/interaction-baseline
cargo build --manifest-path C:/perf/interaction-baseline/Cargo.toml --target-dir C:/perf/baseline-target -p gitcomet -p gitcomet-git-gix --bin gitcomet --example interaction-probe
# Repeat with --release, and freeze both executables from each profile.
```

The preparation script copies only the probe, render witnesses and shared
backend driver; it does not copy the candidate's search/cache/scheduling fixes.
Use distinct Cargo target directories for baseline and candidate. A shared
target can incorrectly reuse an excluded path dependency from the other source
tree; this occurred with `gitcomet-win32-window-utils` during this investigation.
The affected package was cleaned separately for debug and release before the
candidate was rebuilt. Failed build attempts and smoke captures are retained
but excluded from timing results.

### Backend sources and progress

```powershell
python scripts/ci/interaction-performance.py --baseline C:/perf/backend-baseline.exe --candidate C:/perf/backend-candidate.exe --output C:/perf/backend-first --session first --pairs 3
python scripts/ci/interaction-performance.py --baseline C:/perf/backend-baseline.exe --candidate C:/perf/backend-candidate.exe --output C:/perf/backend-second --session second --pairs 3 --reverse
```

The final journal correction was remeasured with `--operations diff` added to
both commands, preserving six pairs and 35 samples while repeating the affected
source cases. Omitting that option also runs status and progress controls.

The fixture has changed 32 KiB, 1 MiB and 16 MiB files. Disk caches are primed and
allowed to age outside timing; each invocation has five warmups and 35 retained
samples. Every source's old/new content hash and status result is checked.
A real pre-push hook emits 60 small writes over 1.2 seconds to measure the first
progress notification through the production operation context.

### UI interactions

```powershell
python scripts/ci/ui-responsiveness.py fixture C:/perf/ui-fixture
python scripts/ci/ui-responsiveness.py measure --baseline C:/perf/baseline.exe --candidate C:/perf/candidate.exe --repository C:/perf/ui-fixture --output C:/perf/ui-first --session first --pairs 3 --seconds 6 --scenarios typing commit-typing file-search click --native-gestures
python scripts/ci/ui-responsiveness.py measure --baseline C:/perf/baseline.exe --candidate C:/perf/candidate.exe --repository C:/perf/ui-fixture --output C:/perf/ui-second --session second --pairs 3 --seconds 6 --scenarios typing commit-typing file-search click --native-gestures --reverse
python scripts/ci/ui-responsiveness.py report C:/perf/ui-first C:/perf/ui-second
```

Repeat for debug and release. Search alternates `needl`/`needle` in a 100,000-row
file after priming the document/search cache, and requires exactly 100 matching
rows in `b.txt`. The first query is also reported separately and requires its
own result witness; warm-query gains must not hide initial-search regressions.
Typing runs at 10 edits/second and clicking/search at approximately 3.3/second.
The analysis excludes the first/last 200 ms of each phase.

Latency begins when the application handles the action, so it excludes time
waiting in the Windows input queue. Text/search actions require a model/result
witness rendered in the same window before submission. Coalesced or unwitnessed
actions are counted separately. Clicks measure acceptance to next submission;
they do not establish completion of the clicked operation. Submission measures
CPU/platform work, not visible display completion. Raw histogram buckets are
merged with their counts; interval percentiles are never averaged.

### LFS transfers

```powershell
python scripts/ci/lfs-performance.py --output C:/perf/lfs-small --shape small --latency-ms 50 --workers 1 2 4 8 16 --rounds 3 --backend C:/perf/backend-candidate.exe
```

Shapes: 60 x 1 MiB (`small`), 512 x 64 KiB (`dense`), 4 x 64 MiB (`large`), and
60 x 1 MiB plus 4,000 ordinary files (`mixed`). Repeat at 0 and 50 ms per object
request. Every upload starts with an empty server; downloads use fresh LFS
storage. Count and hash all objects outside timing. Command-local worker
overrides leave user/repository defaults unchanged. One-round sweeps are
screening evidence only; repeat promising settings before changing production.

For UI during upload, start the same script with `--serve-seconds 120`, wait for
`ready.json`, then run `measure-ui-responsiveness.ps1` with its repository,
`-Scenarios commit-typing -LfsTransferReport <fixture>/transfer.json`,
`-NativeGestures` and an eight-second phase. The harness starts Push, minimizes
hook activity, and requires overlapping HTTP requests, rendered text revisions,
the final Git ref, and verified payload hashes. Create a `stop` file in the
fixture directory when finished. Use a new fixture for every variant.

Loopback delays are controlled server delays, not a WAN bandwidth/TLS/packet-loss
model. They support local concurrency decisions, not universal transfer claims.

## Correctness checks

- UI library: 4,021 passed, 9 existing ignored tests.
- Git backend library: 283 passed, 5 existing ignored tests.
- Windows helper library: 4 passed, including repeated mapped writes with a
  persistent reader and rejection of an active writable mapping.
- State library: 1,012 passed; 9 existing Windows symlink tests fail with OS
  error 1314 because this process lacks symlink privilege.
- Clippy for the changed libraries, Windows helper tests and new backend probe:
  passed. An additional backend `--all-targets` run found an existing needless
  borrow at `tests/status_integration/status_and_diff.rs:835`.
- Python measurement/CI tests: 57 passed.

Regression coverage includes writer exclusion, replacement and same-length
edits with restored modification time, fresh-cache corruption, continuous
progress output, cancellation isolation, replacement of queued diff work,
text-only notifications, search refinement and stale A/B/C query publication.
Initial query work and backspacing have explicit regression coverage. Background
search is also checked against existing search fixtures across
Markdown, conflicts, wrapped rows and file-backed sources.

The UI timing binaries are frozen in `candidate-v2-source.json`. Subsequent
correctness corrections preserve the editor's current match during repeated
edits/queued navigation and add NTFS journal verification. The timed UI search
scans and query cache are unchanged. `navigation-followup.patch` records the
navigation correction. Source-load timings must use the later journal binaries;
the release UI timing binary predates these corrections. The final debug app
and both backend probes use `candidate-final-source.json`. Binary hashes are in
`binary-manifest.json`.
The final debug app also passed a separate search/commit-typing smoke pair in
`ui-final-debug-smoke/`; this confirms integration and is excluded from the
six-pair performance tables. Formatting, whitespace and final source-hash checks
passed (`final-verification.json`).

The timestamp safeguards follow Microsoft's [file time documentation](https://learn.microsoft.com/en-us/windows/win32/sysinfo/file-times),
[file identity API](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
and [handle path API](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew).
Omitting `FILE_SHARE_WRITE` also excludes existing writable file mappings,
as specified by [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew).
That matters because writes through a mapping need not update
[modification time](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-flushviewoffile).
NTFS [change records](https://learn.microsoft.com/en-us/windows/win32/fileio/change-journal-records)
can coalesce while readers remain open. The verification boundary uses
[FSCTL_WRITE_USN_CLOSE_RECORD](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_write_usn_close_record)
and checks the [journal identifier](https://learn.microsoft.com/en-us/windows/win32/fileio/using-the-change-journal-identifier)
to reject versions from a recreated journal.
The LFS fixture implements the [Basic Batch API](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/api/batch.md);
concurrency overrides follow the [Git LFS 3.7.1 configuration](https://github.com/git-lfs/git-lfs/blob/v3.7.1/docs/man/git-lfs-config.adoc).
