**Windows linker benchmark scripts**

The collector and orchestrator compare Rust's bundled LLD with Microsoft's x64 `link.exe`. They implement the [measurement plan](../../../docs/windows-linker-performance-plan.md) with captured-input replay, paired trials, common Windows process accounting, Cargo build scenarios, and optional ETW diagnostics.

Run commands below from a PowerShell 7 shell at the repository root. The benchmark does not change the repository linker configuration. It copies source files and uses dedicated build directories under the campaign directory.

```powershell
# Verify the collector, actual Windows argument passing, child accounting,
# timeouts, cleanup guards, and statistical aggregation.
./scripts/windows/test-linker-bench.ps1
# Also exercise edit/clean/no-op orchestration on a tiny Rust workspace.
./scripts/windows/test-linker-bench.ps1 -BuildScenarios

$campaign = "$PWD/target/linker-bench/my-comparison"

# Snapshot the source/tool identity and compile/capture the dev application.
./scripts/windows/benchmark-linkers.ps1 -Stage Capture -Campaign $campaign -Profiles dev

# Compare sampling against minimal collection, then replay paired full links.
./scripts/windows/benchmark-linkers.ps1 -Stage Calibrate -Campaign $campaign -Profiles dev
./scripts/windows/benchmark-linkers.ps1 -Stage Replay -Campaign $campaign -Profiles dev -Pairs 10 -Warmups 2

# /TIME for both backends and LLD's JSON time trace, outside baseline timing.
./scripts/windows/benchmark-linkers.ps1 -Stage Diagnostics -Campaign $campaign -Profiles dev

# Build/capture/replay the actual Windows shipping profile, with line tables.
./scripts/windows/benchmark-linkers.ps1 -Stage Capture -Campaign $campaign -Profiles shipping-release
./scripts/windows/benchmark-linkers.ps1 -Stage Replay -Campaign $campaign -Profiles shipping-release

# Real Cargo edits, clean builds, and no-op checks; these can take much longer.
./scripts/windows/benchmark-linkers.ps1 -Stage Build -Campaign $campaign -Profiles dev -BuildScenarios edit,clean,noop -BuildPairs 3

# Or run capture, calibration, replay, diagnostics, and build scenarios for
# both primary profiles. Use a pilot to estimate runtime/storage first.
./scripts/windows/benchmark-linkers.ps1 -Stage All -Campaign $campaign

# Regenerate reports, including while another process is measuring.
./scripts/windows/summarize-linkers.ps1 -Campaign $campaign
```

`-Stage Initialize` only freezes the campaign's source, executable identities, and hardware/configuration manifest. `-Profiles` also accepts `release` and `release-with-debug`. `-SampleMs 0` disables memory/thread samples; native CPU/I/O and committed-memory accounting remain available. `-LldThreads N` supports optional scaling trials and is recorded separately from default-thread results. The initial random seed defaults to 20260913; all run orders are saved. Each stage can be repeated without overwriting earlier runs, but the campaign lock prevents concurrent stages in one campaign.

For background execution, use a hidden PowerShell process and redirect its streams:

```powershell
# -File accepts a single profile here; omit -Profiles to use both defaults.
$process = Start-Process pwsh -WindowStyle Hidden -PassThru `
    -ArgumentList @('-NoProfile','-File','scripts/windows/benchmark-linkers.ps1',
                    '-Stage','Replay','-Campaign',$campaign,'-Profiles','dev') `
    -RedirectStandardOutput "$campaign/runner.stdout.log" `
    -RedirectStandardError "$campaign/runner.stderr.log"
Get-Content "$campaign/status.json"
Get-Content "$campaign/current-run.json"
Get-Content "$campaign/runner.stdout.log" -Tail 10

# Graceful stop at the next process boundary, after the current link/build.
New-Item -ItemType File -Path "$campaign/stop.request"
# Remove that one request file before resuming the same campaign.
```

If paths contain spaces, use an appropriately quoted `-ArgumentList` or invoke the scripts directly in PowerShell. Measurements inherit the current normal process priority and affinity; the collector records the effective values. Keep other builds and heavy applications idle during a series. Different campaign directories are not a license to run measurements simultaneously.

**What is recorded**

- `manifest.json`: source and tool hashes, absolute linker paths, toolchain/SDK versions, hardware, power policy, original build environment, and benchmark policy. The compiled helper is copied into the campaign so rebuilding tools cannot silently change an existing campaign.
- `fixtures/<profile>/`: original command/arguments, LLD reproduction archive, immutable extracted inputs, input hashes/counts/bytes, and reviewed common arguments. Each resumed stage verifies the input files before timing. Replay clears `LIB`/`LIBPATH` and resolves libraries through the captured paths.
- `runs/<id>/metrics.json`: process and job CPU/I/O, wall times, committed-memory peaks, observed working-set/thread peaks, page faults, exit status, collection mode, pair/order identity, command hash, and artifact metadata. `samples.csv`, `samples.json`, `process-events.json`, raw stdout/stderr, and the exact run specification accompany it. Cargo runs have a `links/` subtree and copied Cargo HTML timings.
- `runs.jsonl` and `runs.csv`: consolidated raw records. `summary.csv`: per-scenario descriptive statistics with failures and unavailable values. `comparison.csv`/`.json`: paired changes and bootstrap intervals. `report.md`: the readable comparison and calibration results.
- Preflight records include PE headers/dependencies, matching PDB GUID/age, and `--version` smoke output. They are marked as warmups. CLI smoke/PDB identity checks do not claim GUI behavior or interactive debugger verification.

Primary link wall time uses a dedicated Windows process-exit waiter and includes process creation. It is independent of the sampling loop. OS process lifetime is also recorded as a cross-check. Job counters include descendants; resident-memory peaks are observed high-water marks and may miss a last unsampled peak. The collector records APIs it cannot supply as unavailable, not zero. Memory commit is not resident memory, process I/O is not physical storage traffic, and all page faults are not hard faults.

Some toolchains start persistent telemetry/helper processes. After the main process exits, the collector allows 10 ms for descendants to finish, records whether the tree drained naturally, then terminates only lingering descendants in its private job. A long drain would artificially delay returning control to Cargo. `wall_s` stops when the main process exits; `tree_wall_s` and tree totals include the drain interval. `post_exit_grace_ms`, `tree_complete_before_cleanup`, and `lingering_processes_terminated` make this visible. Results from the initial one-second-drain pilot are grouped separately. Pre-existing services outside the job require ETW attribution.

Failed links and validation failures remain in raw logs and are excluded from successful-run comparisons. Baseline statistics never pool tracing modes, sampling intervals, profiles, cache policies, or thread settings. Only complete unambiguous LLD/MSVC pairs contribute to paired effects. The default bootstrap uses 2,000 resamples of whole pairs; `-BootstrapSamples` on the summarizer changes that count. A p95 is emitted only with at least 30 successful observations in a group.

The build scenarios restore independent backend cache snapshots at the same active target path. The edit trial changes an executable string in the copied `main.rs`, using a saved before/after patch, and restores the source afterward. No-op trials must produce zero intercepted links. Large temporary build-cache snapshots are deleted after their scenario completes; raw results and artifact metadata remain. A clean build means an empty Cargo target directory with a warm filesystem cache.

Replays copy captured inputs into a compact directory layout because LLD's reproduction archive can produce paths longer than MSVC LINK accepts. Both linkers receive the same paths and bytes, preserving input and library-search order. `fixtures/<profile>/short/layout.json` records the mapping and hashes. Compact-layout results have a distinct fixture identity from earlier replays; corrupted copies fail verification.

**ETW diagnostics and limitations**

```powershell
# Requires an elevated shell for kernel tracing. Existing recordings are
# detected and left intact. WPR tracing is never enabled for baseline replay.
./scripts/windows/benchmark-linkers.ps1 -Stage Diagnostics -Campaign $campaign -Profiles dev -Etw

# Optionally export the same WPA tables for each backend using a saved profile.
./scripts/windows/benchmark-linkers.ps1 -Stage Diagnostics -Campaign $campaign -Profiles dev -Etw -WpaProfile C:/traces/linker.wpaProfile
```

ETL files and the selected WPR profile details are retained under `diagnostics/`. Inspect provider coverage, event loss, and symbol availability before attributing CPU stacks, physical I/O, or hard faults. The script accepts a WPA export profile because table choices depend on the installed toolkit version; it does not fabricate missing counters. Linker-specific `/TIME` phases and LLD traces are diagnostic evidence with different phase boundaries, not directly comparable phase totals.

Cold-cache reboot/eviction campaigns, CPU-affinity changes, PDB-on/off experiments, hardware performance counters, power/temperature sensors, and GUI/debugger smoke checks remain opt-in follow-up investigations. The scripts do not reset the system cache, change antivirus/power settings, or reboot the machine. Original environment variables are restored when an orchestration invocation finishes.
