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
  GITCOMET_PERF_PRINT_BENCH_SUMMARY
    When truthy, print parsed artifact summaries after each benchmark case.
  GITCOMET_PERF_SUMMARY_LOG
    Optional file path that receives the same per-benchmark summary text.
  GITCOMET_PERF_SUMMARY_JSONL
    Optional file path that receives one JSON record per completed benchmark
    case with parsed Criterion estimates and sidecar metrics when available.

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

is_truthy() {
  local value="${1:-}"
  value="${value,,}"
  [[ "${value}" == "1" || "${value}" == "true" || "${value}" == "yes" || "${value}" == "on" ]]
}

summary_emit_line() {
  local line="$1"
  echo "${line}"
  if [[ -n "${summary_log_path}" ]]; then
    printf '%s\n' "${line}" >> "${summary_log_path}"
  fi
}

summary_emit_blank() {
  echo
  if [[ -n "${summary_log_path}" ]]; then
    printf '\n' >> "${summary_log_path}"
  fi
}

format_duration_ns() {
  awk -v ns="$1" 'BEGIN {
    if (ns == "" || ns == "null") {
      printf "n/a";
    } else if (ns >= 1000000) {
      printf "%.3f ms", ns / 1000000;
    } else if (ns >= 1000) {
      printf "%.3f us", ns / 1000;
    } else {
      printf "%.0f ns", ns;
    }
  }'
}

criterion_estimates_path() {
  local bench="$1"
  printf '%s/%s/new/estimates.json\n' "${criterion_root}" "${bench}"
}

crate_local_criterion_root() {
  if [[ "${criterion_root}" = /* ]]; then
    return 1
  fi

  printf 'crates/gitcomet-ui-gpui/%s\n' "${criterion_root}"
}

resolve_sidecar_path() {
  local bench="$1"
  local candidate=""

  candidate="${criterion_root}/${bench}/new/sidecar.json"
  if [[ -f "${candidate}" ]]; then
    printf '%s\n' "${candidate}"
    return 0
  fi

  if candidate="$(crate_local_criterion_root 2>/dev/null)"; then
    candidate="${candidate}/${bench}/new/sidecar.json"
    if [[ -f "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  fi

  return 1
}

emit_bench_summary() {
  local bench="$1"
  local kind="$2"

  if [[ ${dry_run} -eq 1 || ${print_bench_summary} -ne 1 ]]; then
    return 0
  fi

  if ! command -v jq >/dev/null 2>&1; then
    if [[ ${bench_summary_warned_missing_jq} -eq 0 ]]; then
      echo "Skipping per-benchmark metric summaries because jq is not installed." >&2
      bench_summary_warned_missing_jq=1
    fi
    return 0
  fi

  local estimates_path
  local sidecar_path=""
  local has_estimates=false
  local has_sidecar=false
  estimates_path="$(criterion_estimates_path "${bench}")"

  if [[ -f "${estimates_path}" ]]; then
    has_estimates=true
  fi
  if sidecar_path="$(resolve_sidecar_path "${bench}")"; then
    has_sidecar=true
  fi

  if [[ "${has_estimates}" != true && "${has_sidecar}" != true ]]; then
    return 0
  fi

  local mean_ns="null"
  local mean_upper_ns="null"
  local median_ns="null"
  local std_dev_ns="null"
  local metrics_json="null"
  local runner_json="null"

  summary_emit_blank
  summary_emit_line "-- Metrics: ${bench}"

  if [[ "${has_estimates}" == true ]]; then
    mean_ns="$(jq -r '.mean.point_estimate // "null"' "${estimates_path}")"
    mean_upper_ns="$(jq -r '.mean.confidence_interval.upper_bound // "null"' "${estimates_path}")"
    median_ns="$(jq -r '.median.point_estimate // "null"' "${estimates_path}")"
    std_dev_ns="$(jq -r '.std_dev.point_estimate // "null"' "${estimates_path}")"

    summary_emit_line "criterion mean=$(format_duration_ns "${mean_ns}") mean_95_upper=$(format_duration_ns "${mean_upper_ns}") median=$(format_duration_ns "${median_ns}") std_dev=$(format_duration_ns "${std_dev_ns}")"
  fi

  if [[ "${has_sidecar}" == true ]]; then
    metrics_json="$(jq -c '.metrics // {}' "${sidecar_path}")"
    runner_json="$(jq -c '.runner // null' "${sidecar_path}")"
    summary_emit_line "sidecar metrics:"
    while IFS= read -r metric_line; do
      summary_emit_line "${metric_line}"
    done < <(
      jq -r '
        (.metrics // {})
        | to_entries
        | map(select(.value | type == "number"))
        | sort_by(.key)
        | .[]
        | "  \(.key)=\(.value)"
      ' "${sidecar_path}"
    )
  fi

  if [[ -n "${summary_jsonl_path}" ]]; then
    jq -nc \
      --arg bench "${bench}" \
      --arg kind "${kind}" \
      --arg estimates_path "${estimates_path}" \
      --arg sidecar_path "${sidecar_path}" \
      --argjson has_estimates "${has_estimates}" \
      --argjson has_sidecar "${has_sidecar}" \
      --argjson mean_ns "${mean_ns}" \
      --argjson mean_upper_ns "${mean_upper_ns}" \
      --argjson median_ns "${median_ns}" \
      --argjson std_dev_ns "${std_dev_ns}" \
      --argjson metrics "${metrics_json}" \
      --argjson runner "${runner_json}" \
      '{
        bench: $bench,
        kind: $kind,
        estimates_path: (if $has_estimates then $estimates_path else null end),
        sidecar_path: (if $has_sidecar then $sidecar_path else null end),
        criterion: (
          if $has_estimates then
            {
              mean_ns: $mean_ns,
              mean_upper_ns: $mean_upper_ns,
              median_ns: $median_ns,
              std_dev_ns: $std_dev_ns
            }
          else
            null
          end
        ),
        metrics: (if $has_sidecar then $metrics else null end),
        runner: (if $has_sidecar then $runner else null end)
      }' >> "${summary_jsonl_path}"
  fi
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

  resolve_sidecar_path "${bench}" || printf '%s/%s/new/sidecar.json\n' "${criterion_root}" "${bench}"
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
  if crate_local_root="$(crate_local_criterion_root 2>/dev/null)"; then
    append_unique_report_root "${crate_local_root}"
  fi
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
    emit_bench_summary "${bench}" "launch"
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
  local bench_list_file=""
  bench_list_file="$(mktemp)"

  # `mapfile < <(...)` would hide a failing discovery command behind a
  # successful `mapfile`, which can turn compile errors into a misleading
  # "no benchmarks matched" message.
  if ! discover_main_benchmarks > "${bench_list_file}"; then
    rm -f "${bench_list_file}"
    echo "Failed to discover Criterion benchmarks." >&2
    return 1
  fi

  mapfile -t main_benches < "${bench_list_file}"
  rm -f "${bench_list_file}"

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
    emit_bench_summary "${bench}" "criterion"
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
print_bench_summary=0
summary_log_path="${GITCOMET_PERF_SUMMARY_LOG:-}"
summary_jsonl_path="${GITCOMET_PERF_SUMMARY_JSONL:-}"
bench_summary_warned_missing_jq=0

if is_truthy "${GITCOMET_PERF_PRINT_BENCH_SUMMARY:-}"; then
  print_bench_summary=1
fi
if [[ -n "${summary_log_path}" || -n "${summary_jsonl_path}" ]]; then
  print_bench_summary=1
fi

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

if [[ -n "${summary_log_path}" ]]; then
  mkdir -p "$(dirname "${summary_log_path}")"
  : > "${summary_log_path}"
fi
if [[ -n "${summary_jsonl_path}" ]]; then
  mkdir -p "$(dirname "${summary_jsonl_path}")"
  : > "${summary_jsonl_path}"
fi

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
if [[ ${print_bench_summary} -eq 1 ]]; then
  echo "Per-benchmark metric summaries enabled."
  if [[ -n "${summary_log_path}" ]]; then
    echo "Per-benchmark summary log: ${summary_log_path}"
  fi
  if [[ -n "${summary_jsonl_path}" ]]; then
    echo "Per-benchmark summary JSONL: ${summary_jsonl_path}"
  fi
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
    emit_bench_summary "${bench}" "idle"
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
