# Performance Iteration Reset

Updated: March 25, 2026

## What changed

- The performance suite now measures memory allocations as well as CPU time.
- Every current benchmark entry point now writes allocation metrics into its sidecar JSON.
- Benchmarks that already had structural sidecars keep those metrics and now append allocation data.
- Benchmarks that previously only had Criterion timing now emit allocation-only sidecars.
- `app_launch/*` now records cumulative allocation snapshots at `first_paint` and `first_interactive`.
- `idle/*` now records allocation metrics alongside its existing CPU and RSS metrics.

## Allocation metric semantics

All new allocation fields come from `stats_alloc` wrapped around `MiMalloc`.

- `alloc_ops`
- `dealloc_ops`
- `realloc_ops`
- `alloc_bytes`
- `dealloc_bytes`
- `realloc_bytes_delta`
- `net_alloc_bytes`

These are requested-byte counters for one representative benchmark run.

- They are not RSS.
- They are not allocator usable-size totals.
- `net_alloc_bytes` is `alloc_bytes - dealloc_bytes` for the measured run.

For app launch we also emit milestone-prefixed variants:

- `first_paint_*`
- `first_interactive_*`

## Current coverage

The main Criterion suite in `crates/gitcomet-ui-gpui/benches/performance.rs` now covers CPU + allocations for:

- repo open and repo switch
- branch sidebar and sidebar cache invalidation
- history graph, history cache build, load-more, and scope switch
- commit details, status list, status selection, staging, undo/redo
- diff open, diff scroll, search, merge bootstrap, diff refresh
- resize, drag, frame-timing, keyboard, clipboard, display, network
- syntax, text input, text model, markdown preview, worktree preview
- conflict/mergetool scenarios, streamed providers, and resolved-output recompute
- filesystem event workflows
- real-repo snapshot scenarios

Standalone benchmark entry points:

- `app_launch/*`
- `idle/*`

Recent additions already in the suite:

- `search/file_diff_ctrl_f_open_and_type_100k_lines`
- `open_repo/extreme_metadata_fanout`

`open_repo/extreme_metadata_fanout` uses this default shape:

- 1,000 commits
- 1,000 local branches
- 10,000 remote branches
- 5,000 worktrees
- 1,000 submodules

## Fresh-start rules

- Treat March 24, 2026 as a reset point for the optimization iteration.
- Historical timing notes from the previous run are no longer the working baseline.
- Rebaseline both timing and allocations before doing new optimization work.
- Compare runs only on the same runner class and with the same benchmark inputs.
- New perf sidecars now record a top-level `.runner` object (`runner_class`, `hostname`, `os`, `arch`, `cpu_count`). If measured sections may move between sessions or machines, set `GITCOMET_PERF_RUNNER_CLASS` to a stable label before those runs so the resulting artifacts can still be audited against one another.
- If `app_launch/*` has to move to a different runner class because the local sandbox blocks display IPC, rerun the rest of the suite on that runner class too before treating the combined baseline as authoritative.
- Use allocation regressions as a first-class signal, not just timing regressions.

## Runbook

Main Criterion suite:

```bash
cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance
```

App launch:

```bash
cargo run -p gitcomet --bin perf-app-launch -- --bench app_launch/cold_single_repo
```

- The shorter command form above intentionally relies on `gitcomet`'s default feature set, which already includes `ui-gpui` and therefore `ui-gpui-runtime`; add `--features ui-gpui-runtime` only if you are overriding defaults.

Runner requirement for `app_launch/*` on Linux/FreeBSD:

- These cases require a usable local Wayland compositor or X11 server, not just `DISPLAY` / `WAYLAND_DISPLAY` environment variables.
- Do not switch these cases to `ZED_HEADLESS=1` / GPUI headless mode on Linux/FreeBSD as a fallback. That path cannot open the main window, and the startup probe records `first_paint` / `first_interactive` from real window prepaint/next-frame callbacks.
- If direct local socket probes fail with `Operation not permitted` (for example, `nc -U /run/user/1000/wayland-0` or `nc -U /tmp/.X11-unix/X0`), the runner is sandboxed too tightly for `app_launch/*`; rerun outside that sandbox or on a runner that permits local Unix-socket IPC.
- If the runner also rejects creating or binding fresh sockets at all (`socket()` / `bind()` return `Operation not permitted`), stop trying local workarounds there. Nested `Xvfb`, Wayland compositors, `dbus-run-session`, abstract Unix sockets, and loopback TCP listeners will all be blocked on that runner too.
- `perf-app-launch` now runs a local-session preflight internally on Linux/FreeBSD and will fail before child spawn when no usable local Wayland/X11 endpoint can be resolved, or when every discovered local endpoint fails a direct socket probe.
- `perf-app-launch --preflight-only` now does a cheap real launch probe after those checks: it launches a minimal empty-workspace child, waits for `first_interactive`, verifies that both `first_paint` and `first_interactive` carried allocation snapshots, and exits without writing a sidecar. A passing preflight therefore means GitComet could actually open its main window on that runner and emit allocation-aware startup milestones, not just connect to a socket path.
- Because that probe reaches `first_interactive`, do not run `--preflight-only` immediately before authoritative `cold_*` launch measurements on the same runner/session. It is a runner-diagnostic tool, not a zero-impact smoke test.
- Remote/non-local X11 displays such as `DISPLAY=localhost:10.0` are not accepted for `app_launch/*` baselines; use a local compositor or local X11 session on the same runner instead.
- That environment-blocker path now exits with code `3`, so scripts can distinguish "runner cannot measure this" from ordinary benchmark or harness failures without parsing stderr text.
- If `perf-app-launch` reports that the benchmark "is blocked on a usable local Wayland or X11 session" and labels it "an environment blocker, not a benchmark result", stop there and rerun on a compositor-capable runner.
- `perf_budget_report` now structurally checks the required launch allocation fields too. A fresh `app_launch/*` sidecar that is missing `first_paint_alloc_ops`, `first_paint_alloc_bytes`, `first_interactive_alloc_ops`, or `first_interactive_alloc_bytes` will surface as a report alert instead of passing as timing-only data.
- `perf_budget_report` now also fails closed on incomplete launch sidecars: if those required launch allocation fields are missing, every tracked `app_launch/*` row from that sidecar alerts instead of letting `first_paint_ms`, `first_interactive_ms`, or `repos_loaded` appear as misleading `OK` data.
- `scripts/run-full-perf-suite.sh` intentionally does not call `perf-app-launch --preflight-only` before measured `app_launch/*` cases. That real launch probe can warm caches and taint authoritative `cold_*` baselines.
- Instead, `scripts/run-full-perf-suite.sh` runs the measured launch cases in order starting with `app_launch/cold_empty_workspace`. If a case exits `3` with the same environment blocker, the script skips the remaining `app_launch/*` cases, still runs the budget report, and then returns exit `3` so the blocked launch request cannot be mistaken for a completed rerun.
- `scripts/run-full-perf-suite.sh` now also fails early if an explicit `--fresh-reference PATH` does not exist. That matches `perf_budget_report` freshness loading and prevents a mistyped stamp path from making shell-side `app_launch/*` freshness checks falsely treat every sidecar as "not older" than the missing reference.
- On a successful launch rerun with a freshness reference, `scripts/run-full-perf-suite.sh` now verifies the six rewritten `app_launch/*` sidecars before it runs `perf_budget_report`, so an incomplete launch refresh fails before any potentially misleading report summary. That verification is JSON-aware: it checks each sidecar's `.bench` label against the expected scenario and requires numeric `first_paint_ms`, `first_interactive_ms`, `first_paint_alloc_ops`, `first_paint_alloc_bytes`, `first_interactive_alloc_ops`, `first_interactive_alloc_bytes`, and `repos_loaded` fields via `jq`.
- When you override `--criterion-root`, that selected root still controls sidecar writes and `app_launch/*` freshness verification, but the script now invokes `perf_budget_report` with the selected root first and `target/criterion` plus `criterion` as fallbacks. This keeps repo-side launch sidecars under `criterion/` compatible with fresh Criterion timing estimates that still live under `target/criterion`.
- When that launch-blocked run uses an auto-generated freshness reference and no fresh measurements were produced, `perf_budget_report` now says that all tracked budgets were skipped instead of claiming everything is within budget.
- Do not compare or refresh `app_launch/*` sidecars from a different date or runner class when that blocker appears. Use the same Criterion artifact root for the launch rerun, sidecar audit, and budget report (`target/criterion` by default for local reruns, or the path passed via `--criterion-root` / `GITCOMET_PERF_CRITERION_ROOT`).
- To finish a launch-only rebaseline on another compositor-capable session after a blocked local attempt, prefer a session from the same runner class, export `GITCOMET_PERF_RUNNER_CLASS` first if you need a stable cross-session label, then create a suite-start freshness stamp there and run `./scripts/run-full-perf-suite.sh --skip-main --skip-idle --fresh-reference "$fresh_reference"` directly. That script now starts with the measured `app_launch/cold_empty_workspace` case itself, skips the remaining launch cases only if a measured case exits `3` with the same environment blocker, and verifies that all six `app_launch/*` sidecars under the active Criterion root are at least as new as the stamp, have the expected `.bench` labels, and still contain `first_paint_ms`, `first_interactive_ms`, `first_paint_alloc_ops`, `first_paint_alloc_bytes`, `first_interactive_alloc_ops`, `first_interactive_alloc_bytes`, and `repos_loaded` before it runs the budget report and exits `0`. A blocked run still reaches the report, but it now returns exit `3` afterward instead of looking successful. By default it sets that active root to `target/criterion`; add `--criterion-root criterion` only if you explicitly want to refresh the checked-in repo-side `criterion/*` artifacts and will keep that same root for the sidecar audit. If you can only move `app_launch/*` to a different runner class, rerun the main Criterion suite and `idle/*` there too before calling the full rebaseline complete. The report step will still search the selected root first and then fall back to `target/criterion` plus `criterion`.
- If you bypass that script and run the six launch cases manually, audit them with:

```bash
criterion_root="target/criterion" # script default; use criterion only if you passed --criterion-root criterion
for f in "$criterion_root"/app_launch/*/new/sidecar.json; do
  fresh=true
  [[ "$f" -ot "$fresh_reference" ]] && fresh=false
  jq -r --arg file "$f" --arg fresh "$fresh" '
    [
      .bench,
      $file,
      $fresh,
      (.runner.runner_class // "unset"),
      (.runner.hostname // "missing"),
      (.runner.os // "missing"),
      (.runner.arch // "missing"),
      (.runner.cpu_count // "missing"),
      (.metrics.first_paint_ms // "missing"),
      (.metrics.first_interactive_ms // "missing"),
      (.metrics.first_paint_alloc_ops // "missing"),
      (.metrics.first_paint_alloc_bytes // "missing"),
      (.metrics.first_interactive_alloc_ops // "missing"),
      (.metrics.first_interactive_alloc_bytes // "missing"),
      (.metrics.repos_loaded // "missing")
    ] | @tsv
  ' "$f"
done
```

- Treat the rerun as incomplete unless every row shows `true` for freshness (meaning the sidecar is not older than `"$fresh_reference"`), no timing/allocation field prints `missing`, and the `.runner` fingerprint still matches the runner class you are treating as authoritative.

Idle harness:

```bash
cargo run -p gitcomet-ui-gpui --features benchmarks --bin perf_idle_resource -- --bench idle/cpu_usage_single_repo_60s
```

Budget report:

```bash
cargo run -p gitcomet-ui-gpui --bin perf_budget_report -- --skip-missing
```

Freshness gate for rebaseline reports:

- If stale artifacts from an older run still exist on disk, pass `--fresh-reference PATH`, where `PATH` is an existing file touched before the fresh suite began.
- Use a real suite-start stamp for `PATH`, not a later same-day marker; a long run can cross midnight, and a too-new reference can incorrectly hide fresh sidecars that were emitted earlier in the same suite.
- Under `--skip-missing`, `perf_budget_report` will skip artifacts older than that reference instead of silently reusing stale sidecars; without `--skip-missing`, those stale artifacts surface as alerts.
- This matters most when blocked `app_launch/*` sidecars from an earlier date remain under the same Criterion root you are about to report on.
- `scripts/run-full-perf-suite.sh` now auto-creates a suite-start freshness stamp under `tmp/` whenever it runs at least one measurement section plus the report, unless `--fresh-reference PATH` is provided explicitly.
- Report-only invocations of `scripts/run-full-perf-suite.sh` do not auto-create a stamp; pass `--fresh-reference PATH` yourself if you need freshness filtering in that mode.

## What to do first

1. Run the full Criterion suite and capture a new timing + allocation baseline.
2. Run `app_launch/*` and `idle/*` on the same runner class used for the main suite.
3. Run `perf_budget_report --skip-missing`.
4. Record the top timing outliers and the top allocation outliers.
5. Turn those outliers into the next optimization task list.

## Important note about gating

Allocation data is now collected everywhere, but `perf_budget_report` is still primarily gating timing and structural metrics. The first task of the new iteration is to collect stable allocation baselines and then decide which allocation thresholds should become enforced budgets.

## Known missing scenario ideas

These are still useful benchmark candidates for the next expansion pass:

- typing a long commit message character by character into the commit-message input
- resizing width and height together while long history plus untracked/unstaged/staged sections are visible
