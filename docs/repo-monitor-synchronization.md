# Repository monitor test synchronization

`MonitorMsg::Drain` is available under `cfg(test)` on every platform. It finishes
messages ordered before it, including their debounce, policy/index processing,
watch rebuilds, and refresh publication. It does **not** flush the filesystem or
guarantee delivery of notifications still pending in the OS.

Production debounce (250 ms), maximum delay (2 s), native event flags, exclusions,
and recovery intervals are unchanged. Test instrumentation is absent from normal
state builds. Native helper APIs use the default-off `gitcomet-fs-watch/test-support`
feature, enabled through the state's platform-specific development dependency.

## Choose the assertion, not an arbitrary delay

| Helper | Use | Guarantee |
| --- | --- | --- |
| `drain_delivered()` | Explicitly enqueued events | Processing has finished; pending debounce is respected. |
| `refresh_delivered()` | An injected/forwarded event must refresh | Drain, then assert that the refresh was published. |
| `expect_change(path, operation)` | Positive assertion for an operation-unique path | Observe that native path after arming, then drain and return the combined refresh. |
| `settle()` | General native settling | Linux native fence when healthy; guarded settling otherwise. |
| `quiet()` | No refresh may occur | Keep observing the store channel for three seconds. |

Keep the raw callback-count assertions in lifecycle tests as well as `quiet()`.
A quiet store channel alone does not prove that obsolete native watches detached.

`settle()` reports whether it used a native fence or observed a full quiet window.
The counted-monitor startup helper adds `quiet()` only after the fence case;
guarded platforms already observed the required interval and need not repeat it
without an intervening mutation. Reload/build counter assertions remain intact.

Do not use `expect_change` to distinguish successive writes to the same path:
a delayed notification from the previous operation could satisfy it. Preserve
the quiet guard between such operations. Likewise, callback-redirection tests
may use `refresh_delivered` for deliberately injected messages, but must still
observe native delivery when testing the callback itself.

## Native checkpoints

- **Linux:** use an external cookie directory on the repository watcher's same
  inotify instance. The watch and cookie use the same canonical spelling. Cookie
  traffic does not affect repository callback counts or policy.
- **Windows:** explicitly controlled, closed-writer NTFS tests can request
  `checkpoint_native`. Cookies are created and removed inside every successful
  recursive root, not in a separate independently watched directory. Each root
  must be local fixed NTFS without unvalidated reparse-point traversal. The
  regression writes/syncs/closes its fixture files before requesting the fence.
  General `settle()` remains guarded, including startup, recovery, and uncertain
  write/cache timing. The required NTFS regression must actually exercise the
  fast path; running it requires NTFS-backed temporary directories.
- **macOS:** asynchronously checkpoint each live stream's existing serial
  callback queue, then drain monitor processing. This covers callbacks queued
  before the checkpoint, not events still inside the kernel or `fseventsd`.
  General settling retains the three-second quiet guard. Checkpoint/drain time
  overlaps that guard; late relevant callbacks, refreshes, and generation
  changes extend it. No extra FSEvents paths or streams are added.

Every native watcher instance has a unique generation, including intermediate
setup attempts. A native checkpoint must receive one acknowledgement per live
registration and then drain that same generation. Rebuilds invalidate pending
cookies and cause a fresh checkpoint within the original deadline. A native
fence cannot succeed with missing/degraded coverage; draining explicitly
delivered work is still possible.

Cookie paths are owned exactly (including late cleanup notifications). They are
filtered before callback counters and policy. Mixed events retain ordinary
paths, and overflow/error flags are never suppressed. Ordinary event forwarding
precedes cookie acknowledgement. Windows cookie files use exclusive creation
and 8.3-compatible names, are immediately removed, and do not modify Git ignore
files or the index.

Acknowledgement waits have a ten-second deadline, not a ten-second sleep. The
guarded-settle loop has a twenty-second total bound and permits at most three
follow-up refreshes. Timeouts report the outstanding generation/registrations
and recent native observations; they do not silently fall back to successful
sleep-based assertions. Unsupported/degraded capabilities select guarded
settling explicitly. The monitor and native callback threads never wait for a
caller to process their acknowledgements.

## Validation and timings

Install the pinned runner and use the repository Rust toolchain and Git LFS:

```sh
python3 scripts/ci/install-nextest.py --bin-dir target/ci-bin
export PATH="$PWD/target/ci-bin:$PATH"
cargo nextest run --locked -p gitcomet-state -p gitcomet-fs-watch \
  --cargo-profile ci-test --profile ci --no-fail-fast

GITCOMET_TEST_SYNC_TRACE=1 cargo nextest run --locked \
  -p gitcomet-state -p gitcomet-fs-watch --cargo-profile ci-test \
  --profile ci-watch-stress --stress-count 10 --no-tests=fail \
  -E 'package(=gitcomet-fs-watch) | test(native_sync_) | test(native_barrier_) | test(::selective::synchronization::) | test(::selective::native_lifecycle::)'
```

Use Python 3.12 for the CI helper test suite, matching CI's configured version.
With `GITCOMET_TEST_SYNC_TRACE=1`, test output records milliseconds spent in
native acknowledgement, Drain, registration, guarded-settle, and quiet phases.
Drain includes pending debounce/processing; registration and guard measurements
can overlap it and must not be summed as independent wall-clock durations.

The cross-platform workflow has an opt-in `watcher-stress` input. It reuses the
already compiled workspace binaries at normal parallelism and performs ten
iterations without retries. It does not multiply ordinary CI runs. Stress
results go to `target/nextest/ci-watch-stress/`, separate from the ordinary
inventory-checked `target/nextest/ci/junit.xml`; both are uploaded as diagnostics.

Compare the same optimized profile and runner before/after, excluding compilation.
Report per-test medians/tails and total test time, not a blanket whole-CI speedup.
The remaining lifecycle/absence guards are intentional.

### Local implementation check (2026-09-19)

On Apple Silicon running macOS 27.0, the final `ci-test` build passed all 1,041
state/native-watcher tests without retries or skips in 77.313 seconds. The native
watcher/lifecycle selection plus both lost-boundary recovery tests also passed
all ten stress iterations: 310 executions in 615.491 seconds. The existing
repeated-directory-recreation test still takes about 61 seconds because its
absence observations remain intact. Seven focused state synchronization
regressions also passed ten additional repetitions against the final binary.

Repeated executions of retained before/after optimized test binaries on the same
machine, excluding compilation, gave these wall-clock medians:

| Selection | Repetitions | Before | After |
| --- | --- | --- | --- |
| Both `lost_boundary_events` tests, two threads | 5 | 9.70 s | 6.68 s |
| `stable_setup_reloads_once`, one thread | 3 | 6.12 s | 3.13 s |

These are targeted savings, not a measurement of whole-workspace or cross-platform
CI speedup. The full-suite JUnit report and command log are
`target/nextest/ci/junit.xml` and `target/ci-reports/drain-final-suite.log`; stress
results use `target/nextest/ci-watch-stress/junit.xml`.

Formatting, warning-free Clippy checks for both crates, and all 17 CI helper tests
under Python 3.12 passed. The native crate's test-support feature also cross-checks
for `x86_64-pc-windows-msvc`. Checking the entire state test build from macOS is
blocked by the existing mimalloc C dependency's requirement for a Windows MSVC
toolchain. Native Windows NTFS execution and Linux execution still require their
CI runners; a cross-check is not a substitute for those runtime validations.

## Why these boundaries matter

[Git's fsmonitor implementation](https://github.com/git/git/blob/master/builtin/fsmonitor--daemon.c)
publishes changes before acknowledging cookies, and
[Watchman's cookie synchronization](https://github.com/facebook/watchman/blob/main/watchman/CookieSync.cpp)
tracks outstanding cookies and retries after resynchronization.

[Microsoft documents per-handle notification buffers and cache-delayed write/size detection](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-readdirectorychangesw).
Flushing a cookie file does not flush preceding writes to other files.

[Watchman documents the FSEvents synchronization limitation](https://facebook.github.io/watchman/docs/cookies):
earlier changes can arrive after a cookie and `FSEventStreamFlushSync`. Neither
the existing separate-stream startup marker nor a dispatch-queue checkpoint is
a global OS barrier. Timed quiet observation is a bounded test guard, not a proof
that no future notification can arrive.
