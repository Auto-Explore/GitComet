# Local Windows performance work

The comparison baseline is `dev` at
`5aa600da905f9d18971254a694ea4e84c045353d`. Measurements here describe the
Windows 10 x64 / Ryzen 5 3600 machine with 12 logical CPUs, Rust 1.98.1,
Git for Windows 2.53.0 and Python 3.14.7. Cross-platform confirmation belongs
in the separate environment. Hosted runner targets in `windows-test-runtime.md`
are independent of local acceptance.

## Proven application gains on this machine

Six alternating baseline/candidate pairs across two separate sessions used
identical release driver source, five warmups and 35 retained samples per mode
per run. Both production binaries were frozen before measurement. No builds,
tests or diagnostic captures ran concurrently with these latency sessions.

| Operation | `dev` median | Candidate median | Median reduction | p95 reduction |
| --- | ---: | ---: | ---: | ---: |
| Remote URL update with operation context, plain repository | 50.62 ms | 34.19 ms | 32.5% | 36.0% |
| Submodule status, backend | 104.59 ms | 71.97 ms | 31.2% | 29.8% |
| Submodule status with operation context | 137.43 ms | 104.44 ms | 24.0% | 22.9% |

All three pass the local gate: at least 10% median improvement and no more than
5% p95 regression. The submodule fixture has one dirty child repository plus
4,000 ordinary files. Raw Git controls remained effectively unchanged. These
results establish operation latency gains for these fixtures, without claiming
a whole-application startup or whole-CI speedup.

Full provenance, raw samples, per-run median/p95 values and the generated
comparison are retained in `target/ci-reports/windows-local-final/`, particularly
`build.json`, `first/session.json`, `second/session.json` and `comparison.json`.

## Implemented changes

- Skip GitComet's hook Trace2 monitor for known `git remote set-url` and config
  reads. Git for Windows collects process ancestry when tracing starts, adding
  measurable work to these short operations. Hook-capable commands, unknown
  options and explicit custom executable paths retain tracing. Output streaming,
  cancellation and deadlines continue through the same command runner.
- Limit the submodule status supplement to index gitlinks and staged paths.
  This retains removed/replaced gitlinks without scanning every ordinary file
  twice. Literal pathspecs support spaces, Unicode and glob characters. Large
  path selections, sparse indexes or unreadable indexes fall back to the complete
  query.
- Compile only `standalone_tool_mode_integration` for the extra Windows CMD
  smoke context. The workspace inventory continues to contain every test.
  The runner records the narrower selection in the smoke coverage artifact.
- Discover MSVC, the Windows SDK and Rust's LLD once per CI compile invocation,
  then pass that environment to each link. Standalone Cargo commands retain
  automatic discovery. The bootstrap is never persisted across invocations or
  toolchain changes; incomplete or mismatched architecture data falls back to
  discovery in the linker script.
- Exclude `.worktrees` from dependency-cache key traversal. A disposable
  baseline checkout must neither slow hashing nor invalidate the active
  workspace's cache.
- Own the launch benchmark's child process tree before starting the app. A real
  Windows run hung after the app exited during creation of a background Git
  probe: a suspended descendant retained stderr, leaving the reader waiting
  indefinitely. The benchmark now stops descendants before joining output
  readers, including after a successful app exit. The regression test recreates
  that suspended-child case. Ordinary application process ownership is unchanged.
- Apply UTF-8 console configuration when the CI runner is imported, as well as
  when invoked directly. The first full-suite attempt through `runtime.py`
  aborted while forwarding nextest's Unicode banner to a redirected CP1252
  stream. The regression covers both entry paths, Unicode paths and child
  failures; full UTF-8 log files and exit codes are preserved.
- Remove inspected static-only native link directories from reused Windows
  test metadata's runtime search paths. The unfiltered workspace contained 83
  directories and produced an 8,307-character PATH on this machine. CMD drops
  inherited variables exceeding its [8,191-character limit](https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/command-line-string-limitation),
  causing the nested mergetool CMD test to fail even though direct execution
  passed. DLL directories, executable helpers and uninspectable directories
  remain. Original metadata and the removed paths are retained in reports.
  With the local nextest and Git shell paths included, preparation reduced PATH
  from 8,376 to 1,087 characters (baseline checkout: 10,331 to 1,133). All 83
  inspected native link directories in these builds contained static artifacts.

The Trace2 attribution follows the
[Git for Windows implementation](https://github.com/git-for-windows/git/blob/v2.53.0.windows.1/compat/win32/trace2_win32_process_info.c).

## Reproduce application measurements

Use separate checkouts and targets. Build both before collecting any latency
samples. The driver source is copied identically into small standalone harness
crates that depend on each checkout's production code. Diagnostics are disabled
in this shared driver, including when the baseline predates diagnostic APIs.
Lockfile versions are checked for equality, and executable hashes freeze the
tested artifacts. Build provenance records production revisions/diffs and the
measurement driver's hash separately.

```powershell
git worktree add --detach .worktrees/windows-dev dev
python scripts/ci/local-performance.py build --baseline .worktrees/windows-dev --output target/ci-reports/local-release --profile release
python scripts/ci/local-performance.py measure --build target/ci-reports/local-release --session first --pairs 3 --fixtures plain submodule
# Run the second session separately, with the initial order reversed.
python scripts/ci/local-performance.py measure --build target/ci-reports/local-release --session second --pairs 3 --fixtures plain submodule --reverse
python scripts/ci/local-performance.py report target/ci-reports/local-release
```

Each invocation alternates baseline/candidate order. Within a probe, raw Git,
backend and attached operation context rotate and reverse order. The default
is five warmups and 35 retained samples. The local gate requires six pairs in
two sessions, at least 10% lower median latency and at most 5% p95 regression.
The report compares medians of the per-run medians and per-run p95 values;
all individual measurements remain in the JSON artifacts. Failed sessions,
copied sessions, diagnostic captures and mismatched environments cannot certify
a result. Short screening runs do not satisfy the gate.

Omit `--fixtures` to include normal LFS, stale LFS metadata and a mixed
4,000-file repository. The submodule fixture also includes 4,000 ordinary files.
Stale metadata is recreated before each timed mode; fixture setup and rewrites
are excluded from timing. Every fixture checks raw Git/backend status parity.
Do not run builds, tests, cache packing or diagnostic captures concurrently
with measurements.

For standalone diagnostics and Git executable experiments:

```powershell
python scripts/ci/application-probe.py --profiles ci-test --fixtures lfs mixed --status-state stale --output target/ci-reports/lfs-diagnostics
python scripts/ci/application-probe.py --profiles release --fixtures submodule --git-executable 'C:\Program Files\Git\mingw64\bin\git.exe' --output target/ci-reports/native-git-experiment
```

These produce distinct latency/diagnostic files. `--latency-only` omits
diagnostics. Selecting another Git executable is an experiment, not automatic
Git discovery in the application.

## Test execution and workflow measurements

`runtime.py --session <name>` records a machine identity, source diff hash and
local session separately from hosted job metadata. `report.py` exposes
`local_enough_samples` independently from hosted `enough_samples`; local runs
cannot satisfy hosted acceptance. Keep profile, features, test identities and
UI thread policy fixed when screening nextest concurrency.

```powershell
python .worktrees/windows-dev/scripts/ci/run.py compile --context workspace
python scripts/ci/run.py compile --context workspace
# Ensure the installed Git for Windows shell is on PATH for shell compatibility tests.
# For the installation used here:
$env:PATH += ';C:\Program Files\Git\bin'
python scripts/ci/runtime.py target/ci-reports/runtime-dev-12 --checkout .worktrees/windows-dev --samples 1 --session screening --nextest-threads 12 --ui-threads 4
python scripts/ci/runtime.py target/ci-reports/runtime-12 --samples 1 --session screening --nextest-threads 12 --ui-threads 4
python scripts/ci/run.py compile --context app --test-target standalone_tool_mode_integration
cmd /d /c python scripts/ci/run.py cmd-smoke
```

Keep clean, restored-dependency and no-op compilation separate. Cache packing
and extraction are separate phases too. Existing target directories do not
establish a clean-build comparison, and a narrower smoke context does not
replace the full workspace inventory check.

For the cache-key and linker phases on identical local inputs:

```powershell
python scripts/ci/windows-workflow-probe.py --baseline .worktrees/windows-dev --output target/ci-reports/workflow-screening
```

The local 35-sample screen (five warmups, alternating order) measured:

| Phase | `dev` median | Candidate median | Reduction |
| --- | ---: | ---: | ---: |
| Cache-key generation, with a local comparison worktree | 204.96 ms | 123.94 ms | 39.5% |
| Linker invocation and discovery, using linker help | 158.98 ms | 22.48 ms | 85.9% |

The linker bootstrap costs about 155 ms once per Cargo invocation. These phase
measurements do not establish a clean-build or whole-workflow speedup. Actual
workspace compilation and the narrower CMD smoke compilation also completed.
Raw phase samples are in `target/ci-reports/windows-workflow-screening/`.

## Launch verification

The repaired launch driver was compiled against both production revisions with
identical driver source, then frozen. All 18 launches passed: three alternating
pairs each for empty cold launch, one-repository cold launch and one-repository
warm launch. Median first-interactive times were 174/167 ms, 166/166 ms and
158/156 ms (`dev`/candidate), respectively. These `ci-test` screening results
demonstrate cleanup and successful launches, with no established startup speedup.

The original hang and suspended-child evidence are retained under
`target/ci-reports/windows-launch-screening/`. Fixed driver provenance and raw
results are in `windows-launch-fixed/` and `windows-launch-verification/` under
the same reports directory. The initial hung run is excluded from timing evidence.

Rebuild the matching workspace feature context before using its test inventory
after a graphical app build: Cargo's unqualified `gitcomet.exe` output is shared
between feature configurations. The application latency driver above freezes
separate binaries specifically to prevent later builds from changing them.

## Candidates requiring further evidence

- The installed native Git executable saves process-launch overhead, but a
  transparent launcher bypass must reproduce HOME, MSYSTEM, helper search
  paths, PLINK_PROTOCOL and optional `etc/git-bash.config` handling. The current
  command API allows callers to change environment values after construction.
  A path substitution at construction time would not preserve that behavior.
  Explicit custom Git preferences remain available; automatic bypass is not
  promoted. See the [Git for Windows launcher source](https://github.com/git-for-windows/MINGW-packages/blob/main/mingw-w64-git/git-wrapper.c).
- The clean worker screen used three warmups and seven retained samples per
  fixture/policy, with status and line statistics measured separately. The
  production policy took 209 ms for 60 small LFS assets; four workers took
  231 ms and eight took 294 ms. Four workers improved the median for 512 assets
  (402 to 348 ms), but regressed the large-asset p95 from 409 to 500 ms and the
  mixed-repository p95 from 249 to 480 ms. One worker almost doubled mixed status
  latency (242 to 463 ms). Retain the current shape-dependent worker policy.
  Seven samples are screening evidence, not acceptance. Raw measurements are in
  `target/ci-reports/windows-workers-clean/`.
- Repeated-path watcher settling and negative quiet windows protect correctness.
  Registration readiness only replaces waits where the first event has a unique
  path. Keep stress checks when changing synchronization.
- Concurrency and benchmark compilation profiles remain experiments until
  repeated local measurements and complete coverage support promotion.
- Cache packing/restoration policy and clean-build profile changes have no new
  controlled local comparison in this pass. Existing target-directory compile
  times are not evidence for promoting either change.

## Local environment findings

The full Windows suite requires directory symlink creation. This local process
has no `SeCreateSymbolicLinkPrivilege`, and Windows returns error 1314 in nine
existing watcher/policy cases on `dev`. These tests remain enabled. Failing
executions cannot certify concurrency or whole-suite improvements.

This checkout also contained 572 merge fixture files converted from committed
LF to CRLF despite the fixtures' `-text` attribute. Three harness tests failed
because their inputs and expected output had different endings. Only files
whose bytes differed solely by that conversion were restored; originals and
failure output are backed up in `target/ci-reports/windows-fixture-originals/`.
Git index object IDs and modes were verified unchanged after refreshing their
stat metadata. This repairs local checkout data and introduces no fixture change
in the branch.

## Verification completed

- The final complete inventory executed 6,903 active tests: 2,771 nextest cases
  and 4,123 UI/budget cases passed; nine existing symlink cases failed with
  Windows error 1314. Their identities and errors match the `dev` failures.
  All tests outside those nine passed after the PATH and local fixture repairs.
- Coverage comparison against `dev` found no missing tests or new ignores.
  The 24 existing ignored tests are unchanged. The workspace contains four
  additional Windows tests, including the branch's command tracing regression.
- Both affected native watcher tests passed ten repetitions each. The full suite
  also passed the repeated-directory-recreation test without shortening its
  settling or negative quiet windows.
- All 35 launch-harness unit tests and 44 Python helper tests passed. The launch
  regression owns a suspended descendant and verifies that cleanup releases its
  inherited stderr pipe after the leader exits.
- Real Git hook output/exit codes, command cancellation/deadlines, gitlink status
  parity, literal Unicode paths and staged gitlink deletion passed natively.
  Rust formatting and `git diff --check` passed.

The inventory comparison and failure classification are retained in
`target/ci-reports/windows-final-checks/validation.json`; the final complete run
is in `target/ci-reports/windows-runtime-final/`. Earlier failed attempts remain
separate, including the console-encoding failure and an invalid baseline attempt
that reused the graphical app executable for headless CLI tests. The affected
baseline CLI cases all passed after restoring their matching feature context and
repairing the runtime search path. None of these failed runs certifies test
execution speed or a concurrency-policy change.
