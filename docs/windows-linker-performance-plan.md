**Plan: compare Rust LLD and Microsoft LINK on this Windows computer**

Use two complementary experiments: replay identical linker inputs to measure the linkers themselves, then measure real Cargo builds to establish how much developer waiting time changes. Collect the common statistics through the same Windows process monitor for both linkers. Collect internal linker profiles separately to explain the differences.

This plan was written before measurement. The implementation and commands are documented in [the benchmark README](../scripts/windows/linker-metrics/README.md); campaign data and reports are generated under `target/linker-bench/`.

**Verified starting point — 2026-09-13**

| Item | Observed value |
| --- | --- |
| CPU | AMD Ryzen 9 5950X, 16 cores / 32 logical processors |
| Memory | Approximately 128 GiB installed; Windows reports 137,343,381,504 usable bytes |
| OS | Windows 11 Pro, build 26200 |
| Storage | Three NVMe SSDs; repository is on C:, NTFS, approximately 366 GiB free |
| Power policy | High performance |
| Rust / Cargo | 1.98.1, host `x86_64-pc-windows-msvc` |
| Rust LLD | 22.1.8, bundled with the active Rust toolchain |
| Microsoft LINK | 14.51.36257.0, toolset directory `14.51.36231`, Visual Studio Community 2026 |
| Windows SDK | `10.0.26100.0` |
| Profiling tools | WPR, WPA, xperf, and wpaexporter are installed |
| Inspected commit | `5ec6e96345614e948bf8fb82c5c2ec6a410868ed` |

The actual linker executables discovered were:

```text
C:\Users\sampo\.rustup\toolchains\1.98-x86_64-pc-windows-msvc\lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe
C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\link.exe
```

The repository already uses LLD: [.cargo/config.toml](../.cargo/config.toml) selects [scripts/windows/msvc-linker.cmd](../scripts/windows/msvc-linker.cmd), which invokes `rust-lld.exe -flavor link`. The script also discovers MSVC and SDK directories on every call, invokes rustc to find LLD, sets library paths, and adds `/STACK:8388608`. Its name does not indicate the executable being measured.

[Cargo.toml](../Cargo.toml) enables incremental Rust compilation in development and fat LTO with one codegen unit in release. The x86-64 configuration uses `-Ctarget-cpu=x86-64-v3`. The Windows [release workflow](../.github/workflows/build-release-artifacts.yml) additionally sets `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`; the `release-with-debug` profile enables full debug information.

Ordinary Rust LTO runs in rustc's LLVM backend before the external native linker. Consequently, release time attributed to the final Rust crate must not all be described as native linker time. Keep existing LTO settings for the primary comparison. [Rust code generation options](https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-plugin-lto) distinguish ordinary LTO from explicitly deferring LTO to the linker.

**1. Define the measurements and their boundaries**

Record a separate scope for the native linker process, its process tree, the linker wrapper, and the whole Cargo build. This catches helper processes used by a linker without misattributing Cargo or rustc work to it.

| Statistic | Collection method for both linkers | Meaning and limitation |
| --- | --- | --- |
| Link wall time | `QueryPerformanceCounter` around native process launch and completion; also retain process creation/exit times | Primary latency measure; define whether launch overhead is included and keep that definition fixed |
| User and kernel CPU seconds | `GetProcessTimes`; Job Object accounting for descendants | Separates CPU work from time spent waiting; includes CPU time on every thread |
| Average parallelism / CPU utilization | `(user_s + kernel_s) / wall_s`; divide by allowed logical processors for machine-relative percentage | Average active logical CPUs, not a count of physical cores; use matching process/tree scope |
| Peak committed memory | Job Object `PeakJobMemoryUsed` and `PeakProcessMemoryUsed`; process memory counters while alive | Distinguish peak aggregate tree commit from maximum single-process commit |
| Resident and private memory | Poll `GetProcessMemoryInfo` for working set, observed working-set high water, and private commit | Record sample interval and label observed/sampled peaks; a missed final peak must not be presented as exact |
| I/O bytes and operations | `GetProcessIoCounters`, and Job Object basic-and-I/O accounting | Read/write/other bytes and operation counts; these are process I/O counters, not measured physical SSD traffic |
| Page faults | Process and Job Object counters | Combined soft/hard faults; a large count does not by itself demonstrate disk paging |
| Threads and child processes | Native process sampling and Job Object process notifications | Peak observed thread count, child identities, and process count; actual thread activity requires tracing |
| Exit status and diagnostics | Exit codes, raw stdout/stderr, warning and error summaries | Failed or incompatible links are results, but do not enter successful-run timing statistics |
| Workload size | Input manifest with hashes, counts, and sizes of objects, rlibs, native libraries, resources, and manifests | Enumerate/hash outside timed runs; distinguish archive file count from member/object count |
| Output size and structure | File sizes and PE/PDB inspection after timing | EXE, PDB, import library, ILK when present; section sizes, imports, subsystem, stack reserve, security flags |
| Total build performance | Same monitor around Cargo; separate record for each linker invocation | Build wall time, CPU, peak committed memory, I/O, link count, and final application link interval |
| Machine conditions | Lightweight system samples and run manifest | Background CPU/I/O, available RAM, power plan, CPU affinity, clock data where available, disk/free space, antivirus state |

Windows documents the underlying [CPU accounting](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes), [process memory counters](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex), [process I/O counters](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getprocessiocounters), [Job Object memory peaks](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information), and [Job Object CPU/I/O accounting](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_and_io_accounting_information). In particular, Windows' `PagefileUsage` memory field describes commit charge; it is not a measurement of bytes physically written to the pagefile.

Use native lifetime counters where available and validate whether each API remains useful after termination. Keep process handles until final accounting is collected. Use Job Object accounting to retain descendant totals even when short-lived helpers have already exited. Check diagnostic traces for shared or pre-existing helper services outside the job; record their activity and initial state separately. Do not sum individual memory peaks and call the result peak simultaneous memory, or add root counters to tree counters that already contain the root.

**2. Build a shared collector and preserve one workload**

Implement a PowerShell orchestration script plus a small compiled Rust Windows helper. Build the helper once before measurement. It can both launch a recorded command and act as Cargo's linker proxy, with backend selection supplied through a benchmark-specific environment variable. Keep the proxy path and MSVC argument flavor stable between backends, preserving the existing target CPU flags. Cargo supports selecting the target linker through [configuration or environment](https://doc.rust-lang.org/cargo/reference/config.html#targettriplelinker).

The helper should:

1. Resolve and record the actual backend executable, version, and hash. Initialize a common MSVC/SDK environment before timing; invoke the absolute paths above. Pass `-flavor link` only to `rust-lld.exe`.
2. Create the native process suspended, associate it with a Job Object, then resume it, so accounting includes early child activity. Wait for the relevant process tree to finish. Record assignment failures explicitly instead of silently dropping tree metrics.
3. Sample memory/thread counts initially every 20–50 ms, using identical settings for both backends. Record cumulative counters separately from sampled time series. Keep logging work outside the measured child job.
4. Preserve Windows argument quoting, Unicode, original response-file bytes and encoding, working directory, and selected environment values. Drain stdout and stderr without blocking the child. Emit one uniquely identified result per invocation, including concurrent links.
5. Calibrate monitoring overhead with a pilot, comparing minimal timing against timing plus sampling. Reduce sampling frequency if its effect is material relative to the difference being investigated.

Capture one final `gitcomet.exe` link per build profile in an unmeasured preparation build. The installed LLD supports `/reproduce:<file>`, which packages linker inputs and a replay command. Copy original arguments/response files and transient rustc objects while they still exist. Capture resources, native libraries, any auxiliary debug inputs, and the resolved SDK/CRT dependencies as well. Hash the resulting fixture and preflight both backends against it. LLD's [COFF option definitions](https://raw.githubusercontent.com/llvm/llvm-project/llvmorg-22.1.8/lld/COFF/Options.td) describe reproduction and profiling options.

Construct one common semantic argument set from that capture, with a reviewed adapter for driver-specific options. Preserve argument order and all meaningful settings: `/MACHINE`, `/SUBSYSTEM`, `/STACK:8388608`, library resolution, debug level, `/OPT`, entry point, manifests, and security options. Normalize output paths. Fail preflight if Microsoft LINK ignores or rejects an option required for equivalent output.

The primary replay must use `/INCREMENTAL:NO` and fresh output state for both linkers. Set optimization/debug options explicitly according to the captured build so defaults do not change the task. Remove only the benchmark's previous output files outside timing; reuse the same output location for matched runs. Never clean the user's normal target directory.

Microsoft's incremental linking is a separate experiment: its `.ilk` state and flag restrictions differ from Rust's incremental compilation. In particular `/OPT:REF` and `/OPT:ICF` disable MSVC incremental linking. LLD's help describes its `/incremental` option in terms of preserving an import library; it is not evidence of equivalent MSVC incremental relinking. [Microsoft incremental linking reference](https://learn.microsoft.com/en-us/cpp/build/reference/incremental-link-incrementally?view=msvc-170).

**3. Use a staged workload matrix**

Start with `gitcomet`'s application binary and default `ui-gpui,gix` features. Pin Rust 1.98.1 and `x86_64-pc-windows-msvc`. Keep each profile's input fixture separate.

| Profile | Purpose | Priority |
| --- | --- | --- |
| Current `dev` | Developer iteration with full debug information | First |
| Windows shipping release: `release` plus `DEBUG=line-tables-only` | Actual release pipeline, including fat LTO and symbol generation | First |
| Plain `release` | Existing local release settings without the CI debug override | Follow-up |
| `release-with-debug` | Optimized build with full debug information; potential PDB stress case | Follow-up |
| One representative large test binary | Test-build iteration, using the repository's test profile | Optional |

Do not infer that `debug=0` means no PDB will be produced. Check actual linker arguments and output files. A separate PDB-cost experiment can replay the same objects with `/DEBUG:FULL` and `/DEBUG:NONE`, holding optimization flags fixed. This estimates the cost of emitting debug output; it does not measure the compiler cost of generating debug information.

Run these scenarios:

| Scenario | Preparation and measurement | Suggested initial repetitions |
| --- | --- | --- |
| Link-only, warm filesystem cache | Captured fixture; no rustc or Cargo compilation; warm both backends first | Two warmups/backend, then 10 paired comparisons |
| Development edit rebuild | Independent source/build state per backend; same small code-changing patch and same starting incremental cache snapshot before each trial | 5–10 pairs |
| Clean Cargo build | Empty dedicated target directory per trial, dependencies already fetched, fixed Cargo job count and cache policy | 3–5 pairs, after a pilot estimates cost |
| No-op Cargo build | No source changes after a completed build | A few sanity runs; expect zero linker invocations |
| Cold filesystem cache | Controlled reboot or documented cache-eviction method before each individual measured link; balanced backend order | Optional separate campaign |

For Cargo builds, use the equivalent of `cargo +1.98.1 build --locked --offline -p gitcomet --bin gitcomet --target x86_64-pc-windows-msvc --timings`, with the profile, proxy, and dedicated target directory supplied by the orchestrator. Fetch dependencies before trials. Pin or disable compiler caches identically. Record which host build-script/proc-macro links the proxy actually intercepts; if any bypass it, state that coverage explicitly.

Cargo's [timing report](https://doc.rust-lang.org/cargo/reference/timings.html) provides compiler-unit durations and concurrency, not a standalone measurement of the external linker. Retain it alongside the native records. Record all intercepted links, label the final application link separately, and use process timelines to explain overlap. Summed link durations are not automatically elapsed build time saved.

A second `cargo build` often does no work. A touched source file causes compilation as well as linking. Cleaning Cargo outputs does not clear Windows' filesystem cache. Treat these as different scenarios and label them accurately.

**4. Make the comparison repeatable**

- Use a fixed source snapshot and lockfile. Save the patch used for incremental rebuild trials. Maintain independent build caches per backend; restore the same pre-edit state for every corresponding trial.
- Pin executable versions, SDK/CRT selection, Cargo jobs, features, profiles, `RUSTFLAGS`, cache settings, affinity, and priority. Preserve the existing CPU target and stack reserve. Explicitly control `LIB`, `LIBPATH`, `PATH`, and option-injecting variables such as `LINK` and `_LINK_`; archive their build-relevant settings.
- Run one trial at a time. Keep the current power policy and antivirus configuration consistent, record background interference, and let activity settle between trials.
- Use balanced randomized pairs: MSVC then LLD for some pairs, LLD then MSVC for others. Store the random seed and exact order. Avoid running the entire MSVC series before the entire LLD series.
- Warm both linker executables and the fixture before warm-cache timing. Hashing, archive extraction, copying, and reproduction logging belong outside timing. For cold-cache trials, complete these operations before the cache-reset step.
- Reset EXE/PDB/ILK/import-library output state equally before full-link trials. Keep existing state only in explicitly named incremental-link experiments.
- Use each linker's default parallelism first. On this 32-logical-processor machine, an optional LLD `/threads:1,2,4,8,16,32` sweep can explain scaling. This notation means separate trials with one value each. A matching affinity sweep for both linkers measures performance under equal CPU availability, but does not impose identical internal thread scheduling.
- Extend the replay campaign to 20–30 pairs if the initial uncertainty is material. Predeclare a practical threshold, for example a 95% interval within roughly five percentage points, and report remaining uncertainty at the cap. Estimate total runtime and storage from the pilot before expanding the matrix.

**5. Collect deeper diagnostics in separate runs**

The common process statistics support direct comparison. Internal phase names and nesting do not have a one-to-one mapping across these linkers.

| Diagnostic | Method | What it can explain |
| --- | --- | --- |
| Internal phase timing | `/TIME` for both installed linkers | Time spent in each linker's own phases; retain raw output and parser version |
| LLD phase timeline | `--time-trace=<unique-file>.json`; optional `--time-trace-granularity=<microseconds>` | Parallel work, symbol resolution, section processing, and PDB generation where exposed |
| CPU and scheduling | Same WPR/ETW profile for both, analyzed in WPA | CPU stacks where symbols are available, runnable/wait time, context switches, thread activity, serial sections |
| Storage and paging | WPR/ETW disk I/O, file I/O, and hard-fault events | Physical I/O latency, files accessed, memory-mapped reads and hard faults; distinguish cached process I/O from storage traffic |
| Output behavior | PE/PDB inspection and application smoke checks outside timing | Whether both builds retain imports, resources, stack/security settings, and usable debug symbols |

`/TIME` is [documented by Microsoft](https://learn.microsoft.com/en-us/cpp/build/reference/time-linker-time-information?view=msvc-170); its support and LLD's trace flags were also checked in the local executables' help. Do not add nested or overlapping phase durations and present their sum as wall time. Keep trace runs out of baseline timing summaries because trace collection and file writes add overhead.

Use the installed [Windows Performance Recorder and Analyzer](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/windows-performance-recorder). Start with a short diagnostic capture to verify providers, stack availability, and event loss; then capture representative paired runs. Save the WPR profile, ETL files, and WPA table-export configuration. Export identical tables with the installed `wpaexporter.exe` once validated. Kernel tracing generally requires elevation. Never interrupt an unrelated existing recording.

Keep profiler setup, trace flushing, symbol downloads, output hashing, and smoke checks outside the link timer. If possible write traces to a different physical SSD, documenting the mapping first. Memory-mapped I/O and background writeback complicate per-process physical-disk attribution; preserve system totals alongside attributed events. Neither process exit nor normal link completion guarantees that every output byte has reached durable storage.

Hardware instruction counts, cache misses, branch misses, package energy, and temperatures are optional investigations. Their availability depends on supported counters, drivers, and sensors; inspect support before promising them. They are not necessary for the initial linker decision. Mark unavailable data as null with a reason, never zero.

**6. Save raw data and produce a comparison**

Suggested implementation files:

```text
scripts/windows/benchmark-linkers.ps1       # prepare, capture, replay, build, profile
scripts/windows/linker-metrics/            # compiled Rust launcher/proxy
scripts/windows/summarize-linkers.ps1      # validation and comparison
```

Store generated artifacts under a timestamped ignored `target/linker-bench/` directory. Clean-build target directories must be separate subdirectories so their cleanup cannot remove fixtures or logs. Retain:

```text
<campaign>/manifest.json                   # schema, source/tool/hardware/config identity
<campaign>/fixtures/<profile>/             # captured inputs, hashes, original/adapted arguments
<campaign>/runs.jsonl                      # one row per Cargo run or linker invocation
<campaign>/runs.csv                        # flat comparable fields
<campaign>/runs/<id>/stdout.log
<campaign>/runs/<id>/stderr.log
<campaign>/runs/<id>/samples.csv
<campaign>/runs/<id>/artifacts.json
<campaign>/diagnostics/<id>/               # TIME logs, LLD trace, ETL and exports
<campaign>/summary.csv
<campaign>/report.md
```

Every result needs `schema_version`, campaign/run/pair IDs, parent build ID, scope, linker identity, profile, scenario, fixture hash, command hash, run order, UTC timestamp, cache/output-state policy, diagnostic mode, sample interval, exit status, and metric availability. Use seconds and bytes in raw data; convert to milliseconds and MiB only for presentation. Save only build-relevant environment values.

For each profile/scenario, report successful and failed run counts, median, mean, standard deviation, IQR, and min/max. Compute each pair's speedup as `MSVC_wall / LLD_wall` and time saved as `100 * (MSVC_wall - LLD_wall) / MSVC_wall`; summarize paired effects with a 95% bootstrap interval, resampling whole pairs. Report equivalent CPU, memory, I/O, and output-size differences. Tail percentiles require more observations: only add a descriptive p95 once there are at least 30 successful runs per backend, and flag its limited precision.

Retain all raw runs and predeclare exclusion rules for failures or externally interrupted trials. Mark interference rather than discarding unexplained slow results. Never combine profiles, warm/cold caches, or traced/untraced results in one speedup figure.

The final report should show side-by-side metrics, wall-time distributions, paired speedups, memory/CPU timelines for representative runs, and total Cargo build savings. Verify successful outputs with CLI smoke checks and a GUI launch, inspect PE settings, and validate useful symbols for debug-producing profiles. Binary hashes need not match across linkers because layout, timestamps, and debug identifiers may differ.

Complete the work in this order: collector and one dev fixture; correctness and overhead pilot; paired dev and shipping-release replay; representative ETW diagnostics; Cargo edit/clean build validation; then additional profiles or scaling experiments if the first results justify them. The decision should state time saved in ordinary builds, resource costs, output/debug compatibility, and uncertainty separately.
