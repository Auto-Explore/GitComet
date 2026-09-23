# Profiling GitComet

Profiling and benchmark drivers live here. CI execution, cache management and
test-runtime reports stay in `scripts/ci/`. Application profiling runs locally;
it does not add jobs or application builds to the CI test matrix.

Run examples from the repository root. Python tools require Python 3.11 or newer;
use `python3` instead of `python` where appropriate. Build probes with the repo's
Rust toolchain. LFS fixtures need `git-lfs` on `PATH`. Windows GUI captures need
Windows PowerShell and an interactive desktop. Linux process tracing requires
Valgrind and strace; shell report comparison uses `jq`. Commands expose `--help`
(PowerShell scripts expose parameters through `Get-Help`).
`measure-process-tree.ps1` and `ui-responsiveness.cs` are shared helpers.

Supply repository, baseline, binary and output paths for your environment.
Default binary paths are relative to this checkout; override them for custom
Cargo target directories. Fixed fixture contents and sizes define repeatable
workloads, rather than machine settings. Build first, then measure without
competing builds or tests, using matching profiles and dependencies. Keep
instrumented captures separate from latency measurements. Paired drivers
alternate execution order and retain raw samples and metadata. Use fresh output
directories, normally under `target/` or `tmp/`.

## Backend measurements

```sh
python scripts/profiling/application-probe.py --profiles release --samples 35 --output target/profiling/application
python scripts/profiling/local-performance.py build --baseline ../GitComet-baseline --output target/profiling/backend
python scripts/profiling/local-performance.py measure --build target/profiling/backend --session first --pairs 3 --samples 35 --warmups 5
python scripts/profiling/local-performance.py measure --build target/profiling/backend --session second --pairs 3 --samples 35 --reverse
python scripts/profiling/local-performance.py report target/profiling/backend
```

Create the baseline at your chosen revision, for example with
`git worktree add --detach ../GitComet-baseline dev`. The paired build installs
the same standalone driver under each checkout's `target/` and freezes its
executable in the output directory. Both revisions must support the driver's
APIs and build profile. Use `build --offline` only after caching dependencies.
Report acceptance thresholds are measurement policy, not predictions of hosted
CI performance. Builds are excluded from latency samples.

Build `interaction-probe` with `cargo build -p gitcomet-git-gix --example
interaction-probe --features benchmarks --release`. Then run
`interaction-performance.py` with `--baseline`, `--candidate`, `--output` and
`--session`; choose `--operations`, `--pairs` and `--samples` as needed. It creates
disposable diff/status/push fixtures and verifies their results.

`lfs-performance.py --output PATH` creates a local HTTP LFS fixture. Configure
`--shape`, `--latency-ms`, `--workers` and `--rounds`, and optionally pass
`--backend` to include the interaction probe. No remote account is required.
On Windows, `benchmark-status-workers.ps1 -FixtureRoot PATH -OutputFile PATH
-Binary PATH` compares worker counts on disposable fixtures; select `-Workers`
and `-Rounds` explicitly when comparing policies.

## GUI and process captures

```sh
python scripts/profiling/ui-responsiveness.py fixture target/profiling/ui-fixture
python scripts/profiling/ui-responsiveness.py measure --baseline /path/to/baseline.exe --candidate /path/to/candidate.exe --repository target/profiling/ui-fixture --output target/profiling/ui-session --session first --scenarios idle scroll click
python scripts/profiling/ui-responsiveness.py report target/profiling/ui-session
```

Use binaries with matching probe support. Typing, search and native move/resize
scenarios require `--native-gestures` and temporarily control focus and the pointer.
The direct `measure-ui-responsiveness.ps1` harness also supports `-InputRate`,
`-WarmupSeconds`, `-CaptureScreenshots` and `-LfsTransferReport`.
`profile-client-cpu.ps1` takes `-Repository`, `-OutputDirectory`, optional
`-Binary`, `-Scenario` and `-Seconds` for Windows process-tree CPU captures.

On Linux, use `profile-gitcomet-process-tree.sh --binary PATH --out-dir PATH
--timeout 60 /path/to/repository` for Callgrind, strace and Git Trace2 captures.
`valgrind_cdp` provides manual Callgrind control; set `CALLGRIND_OUT_FILE` to
choose its output. `build_release_debug.sh` builds the symbolized profile and
forwards additional Cargo build arguments.

## Suites and workflow timing

| Driver | Purpose |
| --- | --- |
| `run-full-perf-suite.sh` | Criterion, idle-resource and app-launch suites; see `--help` for profiles and skip flags. |
| `archive-perf-run.sh` | Run and archive a suite with metadata; forwards suite arguments. |
| `compare-perf-runs.sh` | Compare two archives with metric and regression filters. |
| `benchmark-indexed-history.py` | Paired backend probes with `--before`, `--after`, repeatable `--repository`, `--profile`, `--pairs` and `--output`. |
| `benchmark-indexed-history-frames.py` | Paired frame probes; `--case columns:pixels:scale` selects graph geometry. |
| `calibrate-indexed-history.py` | Record five accepted release Criterion roots for budget calibration. |
| `windows-workflow-probe.py` | Cache traversal and linker launch timing with `--baseline`, `--output` and `--samples`; both checkouts need the corresponding CI helpers. |

See [indexed-history measurements](../../docs/indexed-history-performance.md)
for benchmark contracts and [test-runtime measurements](../../docs/windows-test-runtime.md)
for CI execution timing. Keep machine-specific timing reports out of source control.
