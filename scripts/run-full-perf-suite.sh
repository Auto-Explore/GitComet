#!/usr/bin/env bash
set -euo pipefail

app_launch_environment_blocker_exit_code=3

usage() {
  cat <<'EOF'
Usage: scripts/run-full-perf-suite.sh [options]

Runs the full local performance suite:
  1. Criterion benchmark suite
  2. Idle resource harness cases
  3. App launch harness cases
  4. Performance budget report

Options:
  --profile NAME           Perf profile to run.
                           full (default): current full suite.
                           balanced: shorter local iteration mode that
                           trims Criterion measurement time and skips the
                           two 10-minute idle memory-growth cases.
  --criterion-root PATH    Primary Criterion sidecar root for benchmark
                           sidecars and app-launch verification. The budget
                           report searches this root first, then
                           target/criterion and criterion as fallbacks.
                           Default: target/criterion
  --fresh-reference PATH   Require report inputs and app-launch sidecars to be
                           at least as new as the existing PATH stamp. Useful
                           when stale sidecars from earlier runs still exist
                           on disk.
  --launch-timeout-ms MS   Timeout passed to each perf-app-launch case.
                           Default: 30000
  --main-measurement-time S
                           Criterion --measurement-time override in seconds
                           for each sharded main benchmark process.
  --main-filter TEXT       Run only Criterion benchmarks whose full name
                           contains TEXT. Applies only to the main suite.
  --skip-idle-memory-growth
                           Skip idle/memory_growth_*_10min cases.
  --skip-main              Skip the main Criterion benchmark suite.
  --skip-idle              Skip idle resource harness cases.
  --skip-launch            Skip app launch harness cases.
  --skip-report            Skip the final perf budget report.
  --strict                 Run perf_budget_report with --strict.
                           Default is --skip-missing.
  --dry-run                Print commands without running them.
  -h, --help               Show this help.

Environment:
  GITCOMET_PERF_REAL_REPO_ROOT
    Optional real-repo snapshot root used by real_repo benchmarks.
  GITCOMET_PERF_RUNNER_CLASS
    Optional stable label recorded into new perf sidecars under
    .runner.runner_class. Set it before measured runs if artifacts may later
    be compared across sessions or machines.
  MIMALLOC_PURGE_DELAY
    Defaults to 0 in this script unless already set.
  MIMALLOC_PURGE_DECOMMITS
    Defaults to 1 in this script unless already set.
  GITCOMET_BENCH_HISTORY_HEAVY_COMMITS
    Defaults to 10000 in this script unless already set.

Notes:
  - The main Criterion suite is sharded into one benchmark per process to keep
    RSS bounded under the benchmark RAM guard.
  - The idle resource harness includes 10-minute cases and can take a long time.
  - The balanced profile is intended for quicker local iteration; keep the
    default full profile for authoritative end-to-end perf results.
  - When the report is enabled and the script runs at least one measurement
    section, it auto-creates a suite-start freshness stamp under tmp/ unless
    --fresh-reference PATH is provided explicitly.
  - The script intentionally does not run perf-app-launch --preflight-only
    before measured app-launch cases. That probe reaches first_interactive and
    can warm caches enough to taint authoritative cold-launch baselines.
  - If a measured app-launch case reports a local Wayland/X11 environment
    blocker with exit code 3, the script skips the remaining app-launch cases,
    still runs the report, and then exits 3 so an incomplete launch rerun
    cannot be mistaken for a successful baseline refresh.
  - When the app-launch suite runs with a freshness reference, the script
    verifies that all six app-launch sidecars are not older than that stamp
    and still contain the required launch timing + allocation fields before
    running the budget report. That verification uses jq.
  - The budget report searches the selected --criterion-root first and still
    falls back to target/criterion and criterion so explicit sidecar-root
    overrides do not hide fresh Criterion timing estimates.
  - Treat the suite as authoritative only when main, idle, and app-launch data
    come from the same runner class. If you move app-launch to a different
    runner class to escape sandboxed display restrictions, rerun the other
    measured sections there too.
  - On Linux, GPUI benchmarks require native UI link dependencies such as:
    pkg-config, libxcb1-dev, libxkbcommon-dev, libxkbcommon-x11-dev
EOF
}

run_cmd() {
  if [[ ${dry_run} -eq 1 ]]; then
    printf '+'
    printf ' %q' "$@"
    printf '\n'
    return 0
  fi

  "$@"
}

run_section() {
  local title="$1"
  shift
  echo
  echo "==> ${title}"
  run_cmd "$@"
}

require_jq() {
  if command -v jq >/dev/null 2>&1; then
    return 0
  fi

  echo "jq is required to verify app launch sidecars." >&2
  return 1
}

launch_sidecar_matches_expected_shape() {
  local sidecar="$1"
  local bench="$2"

  jq -e --arg bench "${bench}" "${launch_sidecar_validation_jq}" "${sidecar}" >/dev/null
}

launch_sidecar_path() {
  local bench="$1"

  printf '%s/%s/new/sidecar.json\n' "${criterion_root}" "${bench}"
}

prepare_fresh_reference() {
  if [[ ${run_report} -ne 1 || -n "${fresh_reference}" ]]; then
    return 0
  fi

  if [[ ${run_main} -eq 0 && ${run_idle} -eq 0 && ${run_launch} -eq 0 ]]; then
    return 0
  fi

  auto_fresh_reference=1
  if [[ ${dry_run} -eq 1 ]]; then
    fresh_reference="${repo_root}/tmp/perf-suite-start.AUTO.stamp"
    return 0
  fi

  mkdir -p "${repo_root}/tmp"
  fresh_reference="$(mktemp "${repo_root}/tmp/perf-suite-start.XXXXXX.stamp")"
}

validate_fresh_reference() {
  if [[ -z "${fresh_reference}" ]]; then
    return 0
  fi

  if [[ ${dry_run} -eq 1 && ${auto_fresh_reference} -eq 1 ]]; then
    return 0
  fi

  if [[ -e "${fresh_reference}" ]]; then
    return 0
  fi

  echo "Freshness reference path does not exist: ${fresh_reference}" >&2
  return 1
}

append_unique_report_root() {
  local candidate="$1"
  local existing=""

  if [[ -z "${candidate}" ]]; then
    return 0
  fi

  for existing in "${report_criterion_roots[@]:-}"; do
    if [[ "${existing}" == "${candidate}" ]]; then
      return 0
    fi
  done

  report_criterion_roots+=("${candidate}")
}

build_report_args() {
  local root=""

  report_args=("${report_mode[@]}")
  report_criterion_roots=()
  append_unique_report_root "${criterion_root}"
  append_unique_report_root "target/criterion"
  append_unique_report_root "criterion"

  for root in "${report_criterion_roots[@]}"; do
    report_args+=(--criterion-root "${root}")
  done

  if [[ -n "${fresh_reference}" ]]; then
    report_args+=(--fresh-reference "${fresh_reference}")
  fi
}

discover_main_benchmarks() {
  env GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE=1 \
    cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --list --format terse |
    while IFS= read -r line; do
      [[ "${line}" == *": benchmark" ]] || continue
      local bench_name="${line%: benchmark}"
      if [[ -n "${main_filter}" && "${bench_name}" != *"${main_filter}"* ]]; then
        continue
      fi
      printf '%s\n' "${bench_name}"
    done
}

should_run_idle_bench() {
  local bench="$1"

  if [[ ${skip_idle_memory_growth} -eq 1 && "${bench}" == idle/memory_growth_* ]]; then
    return 1
  fi

  return 0
}

run_launch_case() {
  local bench="$1"

  if [[ ${dry_run} -eq 1 ]]; then
    run_section \
      "App launch: ${bench}" \
      cargo run -p gitcomet --bin perf-app-launch -- \
      --bench "${bench}" \
      --timeout-ms "${launch_timeout_ms}"
    return 0
  fi

  echo
  echo "==> App launch: ${bench}"

  local launch_output=""
  local launch_status=0
  if launch_output="$(
    cargo run -p gitcomet --bin perf-app-launch -- \
      --bench "${bench}" \
      --timeout-ms "${launch_timeout_ms}" \
      2>&1
  )"; then
    if [[ -n "${launch_output}" ]]; then
      printf '%s\n' "${launch_output}"
    fi
    return 0
  else
    launch_status=$?
    printf '%s\n' "${launch_output}" >&2
    if [[ ${launch_status} -eq ${app_launch_environment_blocker_exit_code} ]]; then
      launch_suite_environment_blocked=1
      echo "Skipping remaining app-launch cases because perf-app-launch reported an environment blocker during ${bench}." >&2
      return 0
    fi

    return "${launch_status}"
  fi
}

run_launch_suite() {
  local bench=""

  for bench in "${launch_benches[@]}"; do
    run_launch_case "${bench}" || return $?
    if [[ ${launch_suite_environment_blocked} -eq 1 ]]; then
      return 0
    fi
  done
}

verify_launch_sidecars() {
  if [[ ${launch_suite_environment_blocked} -eq 1 ]]; then
    return 0
  fi

  if [[ -z "${fresh_reference}" ]]; then
    echo "Skipping app launch sidecar freshness verification because no freshness reference is available." >&2
    return 0
  fi

  echo
  echo "==> Verify app launch sidecars"

  if [[ ${dry_run} -eq 1 ]]; then
    echo "+ command -v jq >/dev/null"
    for bench in "${launch_benches[@]}"; do
      local sidecar
      sidecar="$(launch_sidecar_path "${bench}")"
      echo "+ test -f ${sidecar}"
      echo "+ test ${sidecar} is not older than ${fresh_reference}"
      printf '+ jq -e --arg bench %q %q %q >/dev/null\n' \
        "${bench}" \
        "${launch_sidecar_validation_jq}" \
        "${sidecar}"
    done
    return 0
  fi

  require_jq || return 1

  local failed=0
  for bench in "${launch_benches[@]}"; do
    local sidecar
    sidecar="$(launch_sidecar_path "${bench}")"
    if [[ ! -f "${sidecar}" ]]; then
      echo "Missing app launch sidecar: ${sidecar}" >&2
      failed=1
      continue
    fi

    if [[ "${sidecar}" -ot "${fresh_reference}" ]]; then
      echo "Stale app launch sidecar (older than ${fresh_reference}): ${sidecar}" >&2
      failed=1
    fi

    if ! launch_sidecar_matches_expected_shape "${sidecar}" "${bench}"; then
      echo "App launch sidecar is missing the expected bench label or required numeric launch metrics: ${sidecar}" >&2
      failed=1
    fi
  done

  if [[ ${failed} -ne 0 ]]; then
    return 1
  fi

  echo "Verified fresh app-launch sidecars against ${fresh_reference}."
}

run_main_suite() {
  if [[ ${dry_run} -eq 1 ]]; then
    echo
    echo "==> Criterion benchmark suite (sharded)"
    run_cmd env GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE=1 \
      cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --list --format terse
    run_cmd env GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE=1 \
      cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --noplot \
      "${main_criterion_args[@]}" --exact "<benchmark-name>"
    if [[ -n "${main_filter}" ]]; then
      echo "Main suite filter: ${main_filter}"
    fi
    return 0
  fi

  local -a main_benches=()
  mapfile -t main_benches < <(discover_main_benchmarks)

  if [[ ${#main_benches[@]} -eq 0 ]]; then
    echo "No Criterion benchmarks matched the requested main-suite filter." >&2
    return 1
  fi

  echo
  echo "Discovered ${#main_benches[@]} Criterion benchmarks; running one benchmark per process to bound RSS."
  for bench in "${main_benches[@]}"; do
    run_section \
      "Criterion: ${bench}" \
      env GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE=1 \
      cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --noplot \
      "${main_criterion_args[@]}" --exact "${bench}"
  done
}

profile="full"
criterion_root="target/criterion"
fresh_reference=""
launch_timeout_ms="30000"
main_measurement_time=""
main_filter=""
run_main=1
run_idle=1
run_launch=1
run_report=1
strict_report=0
skip_idle_memory_growth=0
dry_run=0
main_measurement_time_set=0
skip_idle_memory_growth_set=0
auto_fresh_reference=0
launch_suite_environment_blocked=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      profile="$2"
      shift 2
      ;;
    --criterion-root)
      criterion_root="$2"
      shift 2
      ;;
    --fresh-reference)
      fresh_reference="$2"
      shift 2
      ;;
    --launch-timeout-ms)
      launch_timeout_ms="$2"
      shift 2
      ;;
    --main-measurement-time)
      main_measurement_time="$2"
      main_measurement_time_set=1
      shift 2
      ;;
    --main-filter)
      main_filter="$2"
      shift 2
      ;;
    --skip-idle-memory-growth)
      skip_idle_memory_growth=1
      skip_idle_memory_growth_set=1
      shift
      ;;
    --skip-main)
      run_main=0
      shift
      ;;
    --skip-idle)
      run_idle=0
      shift
      ;;
    --skip-launch)
      run_launch=0
      shift
      ;;
    --skip-report)
      run_report=0
      shift
      ;;
    --strict)
      strict_report=1
      shift
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "${profile}" in
  full)
    ;;
  balanced)
    if [[ ${main_measurement_time_set} -eq 0 ]]; then
      main_measurement_time="2"
    fi
    if [[ ${skip_idle_memory_growth_set} -eq 0 ]]; then
      skip_idle_memory_growth=1
    fi
    ;;
  *)
    echo "Unknown --profile value: ${profile}" >&2
    usage >&2
    exit 2
    ;;
esac

if [[ ${skip_idle_memory_growth} -eq 1 && ${strict_report} -eq 1 && ${run_report} -eq 1 ]]; then
  echo "--skip-idle-memory-growth cannot be combined with --strict while the report is enabled." >&2
  echo "Use the default report mode, add --skip-report, or run the full idle suite." >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
export GITCOMET_PERF_CRITERION_ROOT="${criterion_root}"

if [[ -z "${MIMALLOC_PURGE_DELAY+x}" ]]; then
  export MIMALLOC_PURGE_DELAY=0
fi
if [[ -z "${MIMALLOC_PURGE_DECOMMITS+x}" ]]; then
  export MIMALLOC_PURGE_DECOMMITS=1
fi
if [[ -z "${GITCOMET_BENCH_HISTORY_HEAVY_COMMITS+x}" ]]; then
  export GITCOMET_BENCH_HISTORY_HEAVY_COMMITS=10000
fi

prepare_fresh_reference
validate_fresh_reference

main_criterion_args=()
if [[ -n "${main_measurement_time}" ]]; then
  main_criterion_args+=(--measurement-time "${main_measurement_time}")
fi

idle_benches=(
  "idle/cpu_usage_single_repo_60s"
  "idle/cpu_usage_ten_repos_60s"
  "idle/memory_growth_single_repo_10min"
  "idle/memory_growth_ten_repos_10min"
  "idle/background_refresh_cost_per_cycle"
  "idle/wake_from_sleep_resume"
)

launch_benches=(
  "app_launch/cold_empty_workspace"
  "app_launch/cold_single_repo"
  "app_launch/cold_five_repos"
  "app_launch/cold_twenty_repos"
  "app_launch/warm_single_repo"
  "app_launch/warm_twenty_repos"
)
launch_sidecar_validation_jq='.bench == $bench and (.metrics.first_paint_ms | type == "number") and (.metrics.first_interactive_ms | type == "number") and (.metrics.first_paint_alloc_ops | type == "number") and (.metrics.first_paint_alloc_bytes | type == "number") and (.metrics.first_interactive_alloc_ops | type == "number") and (.metrics.first_interactive_alloc_bytes | type == "number") and (.metrics.repos_loaded | type == "number")'

report_mode=(--skip-missing)
if [[ ${strict_report} -eq 1 ]]; then
  report_mode=(--strict)
fi

if [[ ${run_report} -eq 1 ]]; then
  build_report_args
fi

echo "Running full performance suite from: ${repo_root}"
echo "Perf profile: ${profile}"
echo "Using primary Criterion sidecar root: ${GITCOMET_PERF_CRITERION_ROOT}"
if [[ ${run_report} -eq 1 ]]; then
  echo "Budget report search roots: ${report_criterion_roots[*]}"
fi
if [[ -n "${GITCOMET_PERF_RUNNER_CLASS:-}" ]]; then
  echo "Using perf runner class label: ${GITCOMET_PERF_RUNNER_CLASS}"
fi
if [[ -n "${GITCOMET_PERF_REAL_REPO_ROOT:-}" ]]; then
  echo "Using real repo snapshots from: ${GITCOMET_PERF_REAL_REPO_ROOT}"
fi
echo "Using mimalloc purge settings: MIMALLOC_PURGE_DELAY=${MIMALLOC_PURGE_DELAY} MIMALLOC_PURGE_DECOMMITS=${MIMALLOC_PURGE_DECOMMITS}"
echo "Using synthetic history-heavy commits: GITCOMET_BENCH_HISTORY_HEAVY_COMMITS=${GITCOMET_BENCH_HISTORY_HEAVY_COMMITS}"
if [[ -n "${fresh_reference}" ]]; then
  if [[ ${auto_fresh_reference} -eq 1 ]]; then
    echo "Using auto-generated report freshness reference: ${fresh_reference}"
    if [[ ${dry_run} -eq 1 ]]; then
      echo "Dry run note: the suite-start freshness stamp is created only on a real run."
    fi
  else
    echo "Using report freshness reference: ${fresh_reference}"
  fi
fi
if [[ -n "${main_measurement_time}" ]]; then
  echo "Using Criterion measurement override: ${main_measurement_time}s"
fi
if [[ ${skip_idle_memory_growth} -eq 1 ]]; then
  echo "Skipping idle memory-growth cases."
fi
if [[ ${dry_run} -eq 1 ]]; then
  echo "Dry run mode enabled."
fi

if [[ ${run_main} -eq 1 ]]; then
  run_main_suite
fi

if [[ ${run_idle} -eq 1 ]]; then
  for bench in "${idle_benches[@]}"; do
    if ! should_run_idle_bench "${bench}"; then
      continue
    fi
    run_section \
      "Idle resource: ${bench}" \
      cargo run -p gitcomet-ui-gpui --features benchmarks --bin perf_idle_resource -- \
      --bench "${bench}"
  done
fi

if [[ ${run_launch} -eq 1 ]]; then
  run_launch_suite
fi

if [[ ${run_launch} -eq 1 ]]; then
  verify_launch_sidecars
fi

if [[ ${run_report} -eq 1 ]]; then
  run_section \
    "Performance budget report" \
    cargo run -p gitcomet-ui-gpui --bin perf_budget_report -- \
    "${report_args[@]}"
fi

if [[ ${dry_run} -ne 1 && ${run_launch} -eq 1 && ${launch_suite_environment_blocked} -eq 1 ]]; then
  echo "App launch suite did not complete because perf-app-launch reported an environment blocker; returning exit ${app_launch_environment_blocker_exit_code} after report completion." >&2
  exit "${app_launch_environment_blocker_exit_code}"
fi
