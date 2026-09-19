# CI performance and coverage

`CI` is the PR/push entrypoint. It calls Rust, Cross-Platform Tests, and Benchmark
Targets concurrently, then publishes `CI complete`. The final check requires all
three reusable workflows to succeed; cancellation, failure, or unexpected skips
cannot produce a green gate. Each child workflow remains manually dispatchable.

There are 16 validation jobs plus the final gate, down from 25 validation jobs.
Configure branch rules against `CI complete` if a required aggregate check is
wanted. Existing individual check names change with the reusable-workflow layout.

## Preserved coverage

| Original execution | New execution |
| --- | --- |
| Formatting in two workflows | One Rustfmt check |
| Headless library/app builds | Rust headless job, both original selections |
| Core merge algorithm, labels, fixtures, extraction, permutation corpus, library tests | Complete core package context in Rust headless job |
| Filtered Meld tests followed by core library tests | Complete core library invocation, including Meld tests |
| State package tests | State package context in Rust headless job |
| Backend tests and repeated CLI scoreboard | Backend package context; successful scoreboard output is retained |
| Headless app E2E and binary unit tests | App package context in Rust headless job |
| Package-only GPUI suite | Workspace GPUI suite, explicitly enabling default package features |
| Full workspace suites | All eight original OS/architecture/image environments |
| Three Linux display matrices | Three fresh subprocess environments using the headless app and workspace UI executables |
| Windows CMD smoke | Original package-only app context, executed from CMD on both architectures |
| macOS informational signing checks | Original release build and checks on all three macOS runners |
| Criterion and three performance support binaries | All original targets linked on all four benchmark runners |

Core, state, backend, and headless app package contexts remain separate from the
workspace feature graph. `cargo test -p …` and `cargo test --workspace` can unify
different dependency features, even when they enumerate the same test names.
The duplicate Linux UI context was audited before consolidation: all 4,166 test
identities and ignore flags matched; the UI default feature is empty, and the
dependency feature differences were Clap features not used by UI code. The
workspace explicitly enables `gitcomet-ui-gpui/default`, including future changes
to that feature. Recheck feature equivalence if UI dependencies change.
Display-profile tests are headless environment-selection tests, as before; they
do not start GNOME, KDE, X11, or Wayland sessions.

The full suite is compiled once per context by `cargo nextest list`. Its binary
metadata is then reused without recompilation. Non-GPUI targets run with nextest;
GPUI targets run with libtest to avoid thousands of process launches. Doctests
run separately with Cargo. New test targets are discovered automatically.

On Windows, mergetool, difftool, standalone-tool, submodule, remote-management,
status, refs, upstream, upstream-divergence, and log integration binaries use
libtest. The default `serial` schedule runs these binaries sequentially with at
most two test threads, sharing isolated Git environments and preserving their
internal mutexes. The opt-in `balanced` schedule first runs nextest unchanged,
then two Git binaries at a time with one test thread each, longest suites first.
Submodules and UI run exclusively. On Linux/macOS, the candidate splits available
CPUs between UI and nextest (half for UI, rounded down, and the remainder for
nextest). Single-CPU and package-only contexts retain serial execution. The first
local experiment used one UI thread but inflated its duration to 207 seconds,
so the candidate now preserves parallelism inside the UI suite. Operational failures cancel and join active process trees;
ordinary test failures still allow the remaining suites to run. Redundant Git
capability probes and their early returns have been removed: real Git commands
must succeed for their tests to pass. Preflight uses a small Git shell alias
instead of enumerating every installed difftool. Their identities and counts
still belong to the complete inventory. Other non-GPUI targets use nextest.

The dispatcher verifies that nextest's executed test identities exactly match the
non-ignored inventory assigned to it. Direct libtest runs verify executed counts,
and smoke selectors must match a nonempty inventory. Retries and fail-fast are
disabled. Git for Windows shell-heavy tests have bounded concurrency, including
core merge extraction and state clone tests. Nextest terminates individual tests
after three minutes and fails leaked subprocesses. Direct libtest suites have a
ten-minute deadline (three minutes for smoke selectors), with process-tree cleanup
on timeout. Both Windows architectures run the subprocess-cleanup regression.

Linux watcher tests fence the real inotify callback queue with a marker outside
the repository and wait for pending debounce/rebuild work. This replaces the
three-second settling delay after each positive refresh. Marker events are
excluded only from test instrumentation, and a regression verifies that actual
file events still produce refreshes before the fence completes. Three-second
negative quiet checks, lifecycle iteration counts, production debounce timings,
and Windows settling windows remain unchanged. On macOS, test-only
`FSEventStreamFlushSync` calls flush every live stream, followed by dispatch-queue
and monitor acknowledgements after debounce/rebuild work. A watcher-generation
change repeats the fence against the replacement streams. The acknowledgement
has a ten-second deadline. Empty/failed watchers and tests deliberately diverting
callbacks to simulate native event loss retain the existing settling window.
The macOS fence uses Apple's documented
[synchronous event-delivery guarantee](https://developer.apple.com/documentation/coreservices/1445629-fseventstreamflushsync).
The Linux fence relies on the ordering guarantee of a single
[inotify event queue](https://man7.org/linux/man-pages/man7/inotify.7.html).

UI spinner fixtures hold backend operations until their loading assertions finish.
Submodule prefetch is held before repository startup, so it cannot finish before
the section is expanded. The guard releases workers on success or panic; ordinary
uses of those fake backends no longer sleep for 250 ms.

Previously ignored tests stay ignored. Existing optional syntax-corpus fixtures
are still not fetched by CI. Successful nextest output is retained in JUnit, and
messages reporting runtime exclusions are collected in `runtime-exclusions.log`.
Git prerequisite skip messages fail validation even when the harness reports
success. Compare other exclusion reasons and ignored-test inventories during rollout.

## Build profiles and caches

`ci-test` retains optimization level 1 except for `gitcomet-ui-gpui`, which uses
level 0. Debug assertions and overflow checks remain enabled for every crate. It
omits debugger information, split-symbol packaging, incremental artifacts, and
LTO, and uses 256 codegen units. `ci-bench` provides compile/link coverage at
optimization level 1 without fat LTO. It retains release assertion behavior.
`ci-bench-candidate` inherits it and sets only the UI crate to optimization level
0. Both benchmark feature builds remain separate: the Criterion/support targets
use `benchmarks`, and the app-launch harness uses its original default features.
Select the candidate with the manual `benchmark-profile` input; it is not the
default or a profile for actual performance measurements.
Normal development, production builds, and actual performance measurements retain
their original profiles. Use `--cargo-profile test` for a diagnostic test build
with the original debugger information.

Cache v2 separates compatibility from dependency identity. The compiled bundle
key includes runner image/architecture/context, actual Rust version, profile and
Cargo configuration, compiler environment, linker and CI helper implementation.
Manifests and the lockfile form a separate dependency suffix. Restore first tries
the exact key, then only the same compatibility prefix; a lockfile change can
reuse unchanged dependencies without crossing a toolchain, profile, architecture,
or image boundary. Prefix matches are unpacked even when `cache-hit` is false.

Only successful pushes or manual runs on the repository's default branch (`dev`)
write caches. PRs restore only. A `cold-cache` run bypasses restore, save and
pruning; it does not establish a warm cache for a following PR. After these
changes reach `dev`, run a normal successful CI cycle there to seed v2.

Compiled bundles always include their matching Cargo sources/downloads, preserving
source timestamps. Workspace/vendored compiled crates, test binaries, credentials
and reports are excluded. An oversized bundle falls back to sources, then crate
downloads, then no cache, with a warning when compiled reuse is unavailable. The
embedded manifest records the actual mode. `cache-restore.json` distinguishes an
exact hit, a compatible-prefix restore, source fallback, miss and cold bypass.

On a compiled-bundle miss, every job can restore one shared source cache for its
OS family. Benchmarks and Clippy use only this fallback, without separate cache
allocations. Ubuntu 22 x64, macOS 15 ARM64 and Windows x64 are the designated
source writers. Source lookup remains cheap when compiled dependencies restored;
the separate source archive is not unpacked over their matching sources.

| Context | Count | Maximum compressed cache each |
| --- | ---: | ---: |
| Ubuntu x86_64, including headless display-selection app context | 1 | 1,200 MiB |
| Windows, including package-only CMD context | 2 | 800 MiB |
| Remaining native platform suites | 5 | 700 MiB |
| Headless package suites | 1 | 400 MiB |
| Shared source fallback (Linux, macOS, Windows) | 3 | 256 MiB |
| cargo-audit executable | 1 | 64 MiB, binary only |

The maximum active allocation is 7,532 MiB (7.90 GB). The final gate prunes
superseded owned validation caches within each ref/context and checks the 8 GB
budget. The last v1 bundle for a context is retained until a v2 replacement is
published; old per-job source caches retire once their OS source fallback exists
in the same ref. Unrelated release/performance caches are never deleted. Temporary
overlap between versions remains possible while a run is in progress.

Both GPUI dependencies and the lockfile pin
[`279ab2a52ae54dd3ad4e86a73f052a08a949e045`](https://github.com/Havunen/gpui-ce/commit/279ab2a52ae54dd3ad4e86a73f052a08a949e045),
the inspected revision of [GPUI PR #19](https://github.com/Havunen/gpui-ce/pull/19).
It supplies generated Windows bindings, including shader build dependencies;
GitComet does not duplicate that upstream implementation. Validate native
Windows builds on both architectures before adopting any later revision.

## Fixture setup and diagnostics

Ordinary mergetool, difftool, standalone, status-conflict and submodule fixtures
write initial Git configuration directly, using a shared escaping helper tested
against Git's parser. Windows libtest suites reuse an immutable empty repository
seed within the process and copy its files for each test. Config, refs, objects,
locks and indexes are independent; no hardlinks are used. Unix nextest keeps
fresh initialization because each test normally has its own process. Tests of
repository initialization, config mutations and per-test global configurations
still run the relevant real Git operations.

A small native Rust fixture executable replaces PowerShell/touch used only to
write, copy or delete files and control mtimes in status tests. The real tool
launch path and CMD smoke tests remain covered. Worktree error assertions compare
canonical paths while still checking the operation's error identity and branch.

The manual `fixture-timings` input enables `GITCOMET_CI_FIXTURE_TIMINGS`, an
absolute report directory. Instrumented helpers write one TSV per process with
test name, phase, operation and microseconds. `setup`, `config` and `subprocess`
measurements can nest: compare them separately, never sum them as wall time.
Collection is off by default, and reports are uploaded with the other artifacts.

```sh
python3 scripts/ci/report.py fixtures target/ci-reports/fixture-timings
```

## Running and inspecting validation

Install the pinned nextest executable into a local tool directory and put that
directory on PATH. The installer verifies the recorded release SHA-256 and picks
the native host, including Windows ARM:

```sh
python3 scripts/ci/install-nextest.py --bin-dir /tmp/gitcomet-ci-bin
export PATH="/tmp/gitcomet-ci-bin:$PATH"
python3 scripts/ci/run.py compile --context workspace
python3 scripts/ci/run.py test --context workspace
python3 scripts/ci/run.py doc --context workspace
```

Other contexts are `core`, `state`, `backend`, `app`, and `ui`. Compile `app` and
`workspace` before `run.py display`; compile `app` before `run.py cmd-smoke`. Compilation
and execution must use the same checkout and target directory.

Each job uploads `ci-*` artifacts for seven days, including Cargo build timings,
command logs, phase durations, feature metadata, test inventories, JUnit reports,
cache mode, scheduling choice/wall time (`execution.json`), and runtime exclusions. Reports live locally in `target/ci-reports`.
Artifacts are uploaded on failure as well as success.

## Measuring the 50% target

The optimization is not considered a measured 50% improvement until hosted-runner
comparisons pass. Workstation timings cannot establish that claim.

1. Compare representative core, UI, and dependency-change revisions with the same
   runner labels. Run baseline/candidate cycles sequentially to avoid artificially
   competing for the same runners.
2. Use the original workflows for the baseline. Dispatch `CI` with `cold-cache:
   true` for cold candidate runs. Compare cold and warm cohorts separately. PRs
   need an existing default-branch cache before they can be called warm runs.
3. Collect at least three cold and five warm paired runs per platform. Record all
   three baseline workflow run IDs for each validation cycle; the new CI workflow
   needs only its orchestrator run ID.
4. Compare inventories and runtime-exclusion reasons on every platform. The
   inventory comparison rejects removed tests and new ignored tests, rather than
   accepting equal totals. Also review package/target feature metadata.
5. Require at least 50% lower median completion time including queues, and at
   least 50% lower median duration for lanes previously taking 20+ minutes. Intel
   macOS must complete within 30 minutes; its old timeout is a lower-bound
   baseline, not a successful measurement. Total runner minutes must not grow.
6. Require cold builds to complete without timeouts, then monitor median, p95,
   cache hits, and timeouts over the next 20 qualifying runs. Keep current timeout
   limits during rollout.

```sh
python3 scripts/ci/report.py runs BASELINE_RUST_ID BASELINE_PLATFORM_ID BASELINE_BENCH_ID --output baseline.json
python3 scripts/ci/report.py runs CANDIDATE_CI_ID --output candidate.json
python3 scripts/ci/report.py compare baseline.json candidate.json
python3 scripts/ci/report.py coverage baseline/coverage.json candidate/coverage.json
python3 scripts/ci/report.py caches
```

Supply one complete validation cycle per SHA/branch/event to each report file.
Use separate files for reruns of the same revision and for cold/warm cohorts.
The timing collector includes individual job and step durations. `start_delay`
includes dependency waits; it is not a pure runner-queue metric. Failed and
cancelled cycles remain visible in the report but are excluded from successful
duration statistics. Classify superseded cancellations separately from actual
timeouts when assessing reliability. The comparison command reports measurements;
it does not declare the rollout complete from a small or incomplete sample.

If a target is slower or incompatible under nextest, preserve its tests under the
libtest dispatcher and document the measured reason. If compiled caches repeatedly
fall back to sources, inspect `cache.json` before changing their allocations.
Performance measurements and release packaging are outside this rollout.

## Follow-up findings from PR #502

[PR #502's validation run](https://github.com/Auto-Explore/GitComet/actions/runs/35449240055)
was incomplete, so its shorter consumed runner time is not proof of a successful
50% improvement over PR #501. All observed dependency cache restores missed.
Windows ARM timed out during test execution.

Windows x64 recorded 168 status tests spending about eight seconds apiece in
`git difftool --tool-help`, then returning without their assertions. Those tests
also reported leaked subprocesses. Nextest's process-per-test model defeated the
probe's process-local cache, and its default leak policy accepted those results.
The redundant probes and 362 early-return call sites were removed across the
integration suites. Test bodies, isolated environments, and suite locks remain.
This is a confirmed test-harness defect; these results do not establish the same
leak in GitComet's application subprocess implementation.

Local Linux measurements for this follow-up:

| Measurement | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Directory-recreation regression, all three CRUD cycles | 60.77 s | 12.79 s | 79% |
| Sum of state-test durations (not suite wall time) | 699.47 s | 296.08 s | 58% |
| UI test-binary compilation, isolated profile experiment | 142.0 s | 57.1 s | 60% |

In the profile experiment, UI execution increased from 44.1 to 59.6 seconds, still
leaving a substantial net saving on compilation plus execution. These are local
samples, not hosted-runner acceptance measurements. The watcher fence and root
replacement cases also passed three stress iterations. The workspace inventory
retained every existing test and ignore flag, adding one Linux fence regression.
The duplicate Linux UI compile/test/doctest context previously cost about seven
minutes in PR #502; its full suite and all three display profiles now reuse the
workspace UI binary.

Local verification passed 2,837 nextest tests and 4,157 UI/support tests (6,994
enabled workspace tests in total). All 24 existing ignored tests remained
unchanged. The three spinner tests passed ten further repetitions; all display
profiles, workspace doctests, 17 CI-helper tests, Rustfmt, and workflow lint passed.

`report.py coverage` intentionally refuses inventories with different feature
selections. For the transition that explicitly enables the UI's empty `default`
feature, audit the feature metadata and compare test identities/ignore flags as
well; equal counts alone are insufficient. Subsequent runs use the same explicit
selection and can use the normal comparison command directly.

Windows and macOS need hosted runs to validate their final timings. Keep the
multi-run cold/warm acceptance criteria above before claiming the overall target.

## Second follow-up: PR #502 native bottlenecks

[Run 35458979772](https://github.com/Auto-Explore/GitComet/actions/runs/35458979772)
failed and all 14 dependency-cache restores missed. These timings locate work;
they are not a successful acceptance baseline.

| Native lane | Job | Compile/inventory | Test execution |
| --- | ---: | ---: | ---: |
| Windows x64 | 36m58s | 19m31s | 15m58s |
| Windows ARM64 | 36m37s | 14m40s | 20m32s |
| macOS 15 Intel | 31m52s | 17m53s | 9m24s |
| macOS 15 ARM64 | 27m47s | 15m19s | 9m29s |
| Ubuntu x64 | 18m48s | 11m53s | 4m28s |

Windows status failures in this run comprised twelve incidental PowerShell
fixture failures and three path-format assertions. The native helper and
canonical-path assertions address those failures. Shell startup and repeated Git
fixture configuration account for additional test cost; scheduling remains an
experiment until hosted measurements establish its net benefit.

For rollout, keep the default `test-schedule: serial` and
`benchmark-profile: ci-bench` while establishing a green native baseline. Compare
`balanced` against `serial` on identical revisions/cache states, then compare
`ci-bench-candidate` against `ci-bench` independently. Promote a candidate only if
its affected job's median improves by at least 10%, no other lane regresses by
more than 5%, total runner minutes do not increase, and inventories/exclusions
remain equivalent. Use the three-cold/five-warm cohorts described above; optional
fixture diagnostics should be enabled consistently within a pair.

On macOS, run the new multi-stream flush and monitor-fence tests, then repeat the
monitor suite twenty times. Existing root replacement, ignore-policy rebuild,
lost-callback, overflow recovery, quiet-window and lifecycle tests must all pass:

```sh
cargo test --locked --profile ci-test -p gitcomet-fs-watch
cargo test --locked --profile ci-test -p gitcomet-state native_fsevents_fence
for iteration in $(seq 1 20); do
  cargo test --locked --profile ci-test -p gitcomet-state store::repo_monitor::tests -- --test-threads 2 || exit 1
done
```

On Windows x64 and ARM64, run the full native job, including the CI helper tests,
status integration, remaining Git integration suites, doctests and CMD smoke.
Check that every inventoried enabled test executed, Unicode output remains intact,
and no prerequisite skip or cleanup failure was accepted. The GPUI build, link
and shader steps need native verification on both architectures. On every OS,
confirm compiled restores report `mode: dependencies` and Cargo timings show real
reuse; a source-only hit does not demonstrate compilation savings.

## Local verification of the second follow-up

On the Linux workstation (32 logical CPUs), both serial and revised balanced
workspace runs passed all 6,996 enabled tests, with the same 24 ignored tests.
The inventory comparison found no removed or newly ignored tests and two added
fixture regressions. One serial/balanced execution pair measured 80.946s/61.449s
(24.1% lower test wall time); this is one warm local pair, not a hosted CI or
compilation-speed claim. The initial single-thread UI experiment also passed but
took 206.826s overall and was replaced by the CPU-splitting candidate.

The 403 headless-app tests, all three display profiles, workspace doctest commands,
25 CI-helper tests, required CI Clippy checks, the fixture helper's library Clippy
check, Rustfmt and Actionlint passed. Dependency audit completed with the two
already-allowed maintenance warnings for `paste` and `ttf-parser`. Both separate
benchmark feature builds linked
successfully with `ci-bench-candidate`. The FSEvents wrapper and its tests passed
an `aarch64-apple-darwin` cross-compile check.

Native Windows/macOS execution and default-branch cache seeding remain hosted
validation steps. Cross-checking the complete macOS state crate on Linux stopped
at missing Apple C compiler/SDK support in native dependencies. An extra Clippy
check of all Rust test targets encountered three existing warnings in
`gitcomet-core/src/history_index.rs` (two `manual_is_multiple_of`, one
`needless_range_loop`); the repository's configured CI Clippy commands passed.
