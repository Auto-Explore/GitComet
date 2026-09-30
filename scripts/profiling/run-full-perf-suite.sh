#!/usr/bin/env bash
set -euo pipefail

app_launch_environment_blocker_exit_code=3

usage() {
  cat <<'EOF'
Usage: scripts/profiling/run-full-perf-suite.sh [options]

Runs the full local performance suite:
  0. Build every measuring executable, then freeze copies of them
  1. Criterion benchmark suite
  2. Idle resource harness cases
  3. App launch harness cases
  4. Performance budget report
  5. Completeness check against the run's scenario manifest

Measurements run the frozen copies directly; nothing compiles between cases.

Options:
  --cargo-profile NAME     Cargo profile for every measuring executable.
                           Default: release (the shipping settings). Use
                           release-with-debug only for symbolized diagnostics.
  --run-dir PATH           Directory for frozen binaries, the scenario
                           manifest, per-case logs and environment metadata.
                           Default: target/perf-runs/<UTC timestamp>
  --fail-fast              Stop at the first failed case. By default every
                           selected case runs and failures are reported at the
                           end.
  --profile NAME           Workload profile to run.
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
    Defaults to 1000 (ms) unless already set: mimalloc 3.5's built-in
    default, so harness processes purge like the shipped application.
  MIMALLOC_PURGE_DECOMMITS
    Defaults to 1 unless already set, also the built-in default. Every
    MIMALLOC_* value in effect is recorded in the manifest and in sidecars.
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
  - Each sidecar's .measurement.kind says what it timed; see
    scripts/profiling/README.md before reading timings as user latency.
  - The run is accepted only when every selected scenario exited 0 and left
    fresh artifacts: Criterion estimates newer than the suite start, and
    sidecars stamped with this run's id. manifest.json records the verdict;
    an incomplete run exits 4.
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
  if [[ ${dry_run} -ne 1 ]]; then
    report_args+=(--summary-json "${budget_summary}")
  fi
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
  run_bench_binary --list --format terse |
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
      "${frozen_launch}" \
      --bench "${bench}" \
      --timeout-ms "${launch_timeout_ms}"
    return 0
  fi

  echo
  echo "==> App launch: ${bench}"

  local launch_status=0
  run_case launch "${bench}" \
    "${frozen_launch}" --bench "${bench}" --timeout-ms "${launch_timeout_ms}" || launch_status=$?
  if [[ ${launch_status} -eq 0 ]]; then
    emit_bench_summary "${bench}" "launch"
    return 0
  fi
  if [[ ${launch_status} -eq ${app_launch_environment_blocker_exit_code} ]]; then
    launch_suite_environment_blocked=1
    echo "Skipping remaining app-launch cases because perf-app-launch reported an environment blocker during ${bench}." >&2
    return 0
  fi
  return "${launch_status}"
}

run_launch_suite() {
  local bench=""

  for bench in "${launch_benches[@]}"; do
    run_launch_case "${bench}" || case_failed_or_stop || return $?
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
    run_cmd "${frozen_bench}" --bench --list --format terse
    run_cmd "${frozen_bench}" --bench --noplot \
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
  printf '%s\n' "${main_benches[@]}" > "${run_dir}/criterion-benches.txt"
  for bench in "${main_benches[@]}"; do
    echo
    echo "==> Criterion: ${bench}"
    if run_case criterion "${bench}" \
      run_bench_binary --noplot "${main_criterion_args[@]}" --exact "${bench}"; then
      emit_bench_summary "${bench}" "criterion"
    else
      case_failed_or_stop || return $?
    fi
  done
}

run_bench_binary() {
  # `cargo bench` runs the harness from the package directory with --bench.
  (cd "${repo_root}/crates/gitcomet-ui-gpui" &&
    env GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE=1 "${frozen_bench}" --bench "$@")
}

# Runs one measured case, logging its output and recording its outcome in
# cases.jsonl. A crash or non-zero exit is recorded, never skipped silently.
run_case() {
  local section="$1"
  local bench="$2"
  shift 2
  local slug="${bench//\//__}"
  local log="${run_dir}/logs/${section}-${slug}.log"
  local started_ms ended_ms status=0
  started_ms="$(date +%s%3N)"
  # One stream through one tee: two writers on one file overwrite each other.
  set +e
  "$@" 2>&1 | tee "${log}"
  status=${PIPESTATUS[0]}
  set -e
  ended_ms="$(date +%s%3N)"
  jq -nc \
    --arg section "${section}" --arg bench "${bench}" --arg log "${log}" \
    --argjson status "${status}" --argjson started_ms "${started_ms}" --argjson ended_ms "${ended_ms}" \
    '{section: $section, bench: $bench, exit_status: $status, started_unix_ms: $started_ms,
      ended_unix_ms: $ended_ms, log: $log}' >> "${run_dir}/cases.jsonl"
  if [[ ${status} -ne 0 ]]; then
    echo "Case failed with exit ${status}: ${section} ${bench} (log: ${log})" >&2
  fi
  return "${status}"
}

case_failed_or_stop() {
  if [[ ${fail_fast} -eq 1 ]]; then
    return 1
  fi
  return 0
}

# Builds one Cargo target with the selected profile and copies the executable
# into the run directory, so later edits or builds cannot change what runs.
build_and_freeze() {
  local name="$1"
  shift
  local frozen="${run_dir}/bin/${name}"
  if [[ ${dry_run} -eq 1 ]]; then
    run_cmd cargo build --locked --profile "${cargo_profile}" "$@" >&2
    printf '%s\n' "${frozen}"
    return 0
  fi
  local messages="${run_dir}/logs/build-${name}.jsonl"
  echo "==> Build ${name} (${cargo_profile}): cargo build --locked --profile ${cargo_profile} $*" >&2
  cargo build --locked --profile "${cargo_profile}" --message-format=json-render-diagnostics "$@" \
    > "${messages}"
  local built
  built="$(jq -r --arg name "${name}" \
    'select(.reason == "compiler-artifact" and .target.name == $name and .executable != null)
     | .executable' "${messages}" | tail -n 1)"
  if [[ -z "${built}" || ! -x "${built}" ]]; then
    echo "Could not find the built executable for ${name} in ${messages}" >&2
    return 1
  fi
  cp -p "${built}" "${frozen}"
  jq -nc --arg name "${name}" --arg source "${built}" --arg frozen "${frozen}" \
    --arg sha256 "$(sha256sum "${frozen}" | cut -d' ' -f1)" \
    --arg cargo_args "$*" \
    '{name: $name, source: $source, frozen: $frozen, sha256: $sha256, cargo_args: $cargo_args}' \
    >> "${run_dir}/binaries.jsonl"
  printf '%s\n' "${frozen}"
}

build_measurement_binaries() {
  echo
  echo "==> Build and freeze measuring executables (profile: ${cargo_profile})"
  if [[ ${dry_run} -ne 1 ]]; then
    mkdir -p "${run_dir}/bin" "${run_dir}/logs"
    : > "${run_dir}/binaries.jsonl"
  fi
  if [[ ${run_main} -eq 1 ]]; then
    frozen_bench="$(build_and_freeze performance \
      -p gitcomet-ui-gpui --features benchmarks --bench performance)"
  fi
  if [[ ${run_idle} -eq 1 ]]; then
    frozen_idle="$(build_and_freeze perf_idle_resource \
      -p gitcomet-ui-gpui --features benchmarks --bin perf_idle_resource)"
  fi
  if [[ ${run_launch} -eq 1 ]]; then
    frozen_launch="$(build_and_freeze perf-app-launch -p gitcomet --bin perf-app-launch)"
  fi
  if [[ ${run_report} -eq 1 ]]; then
    frozen_report="$(build_and_freeze perf_budget_report \
      -p gitcomet-ui-gpui --bin perf_budget_report)"
  fi
}

# Every selected scenario must have exited 0 and left fresh artifacts: a
# Criterion estimate newer than the suite start, or a sidecar stamped with this
# run's id. Anything else makes the run incomplete.
write_manifest_and_verify() {
  local selected="${run_dir}/selected.jsonl"
  : > "${selected}"
  local bench
  if [[ ${run_main} -eq 1 && -f "${run_dir}/criterion-benches.txt" ]]; then
    while IFS= read -r bench; do
      jq -nc --arg bench "${bench}" --arg path "${criterion_root}/${bench}/new/estimates.json" \
        '{section: "criterion", bench: $bench, artifact: $path}' >> "${selected}"
    done < "${run_dir}/criterion-benches.txt"
  fi
  if [[ ${run_idle} -eq 1 ]]; then
    for bench in "${idle_benches[@]}"; do
      should_run_idle_bench "${bench}" || continue
      jq -nc --arg bench "${bench}" --arg path "$(launch_sidecar_path "${bench}")" \
        '{section: "idle", bench: $bench, artifact: $path}' >> "${selected}"
    done
  fi
  if [[ ${run_launch} -eq 1 ]]; then
    for bench in "${launch_benches[@]}"; do
      jq -nc --arg bench "${bench}" --arg path "$(launch_sidecar_path "${bench}")" \
        '{section: "launch", bench: $bench, artifact: $path}' >> "${selected}"
    done
  fi

  local checked="${run_dir}/checked.jsonl"
  : > "${checked}"
  local line section artifact status problem
  while IFS= read -r line; do
    section="$(jq -r .section <<< "${line}")"
    bench="$(jq -r .bench <<< "${line}")"
    artifact="$(jq -r .artifact <<< "${line}")"
    status="$(jq -r --arg section "${section}" --arg bench "${bench}" \
      'select(.section == $section and .bench == $bench) | .exit_status' \
      "${run_dir}/cases.jsonl" 2>/dev/null | tail -n 1)"
    problem=""
    if [[ -z "${status}" ]]; then
      problem="not run"
    elif [[ "${status}" != "0" ]]; then
      problem="exited ${status}"
    elif [[ ! -f "${artifact}" ]]; then
      problem="missing artifact"
    elif [[ "${artifact}" -ot "${run_start_stamp}" ]]; then
      problem="stale artifact (older than suite start)"
    elif [[ "${section}" != "criterion" ]] &&
      ! jq -e --arg run "${GITCOMET_PERF_RUN_ID}" '.measurement.run_id == $run' "${artifact}" >/dev/null; then
      problem="sidecar from another run"
    elif [[ -f "${budget_summary}" ]]; then
      # Structural budgets are deterministic witnesses of the work done
      # (rows built, calls made); a failed one voids the timing beside it.
      problem="$(jq -r --arg bench "${bench}" \
        '[.structural[] | select(.bench == $bench and .status == "alert")
          | "failed witness \(.metric) \(.expectation), observed \(.observed)"] | join("; ")' \
        "${budget_summary}")"
    fi
    jq -c --arg problem "${problem}" '. + {problem: (if $problem == "" then null else $problem end)}' \
      <<< "${line}" >> "${checked}"
  done < "${selected}"

  local complete
  complete="$(jq -s 'length > 0 and all(.problem == null)' "${checked}")"
  jq -n \
    --arg run_id "${GITCOMET_PERF_RUN_ID}" \
    --arg workload_profile "${profile}" \
    --arg cargo_profile "${cargo_profile}" \
    --arg source_revision "$(git -C "${repo_root}" rev-parse HEAD)" \
    --arg patch_sha256 "$(git -C "${repo_root}" diff HEAD --binary | sha256sum | cut -d' ' -f1)" \
    --arg criterion_root "${criterion_root}" \
    --arg fresh_reference "${run_start_stamp}" \
    --arg command "${suite_command}" \
    --argjson complete "${complete}" \
    --argjson launch_blocked "${launch_suite_environment_blocked}" \
    --arg budget_summary "$([[ -f "${budget_summary}" ]] && echo "${budget_summary}")" \
    --slurpfile binaries "${run_dir}/binaries.jsonl" \
    --slurpfile scenarios "${checked}" \
    --slurpfile cases "${run_dir}/cases.jsonl" \
    '{version: 1, run_id: $run_id, workload_profile: $workload_profile,
      cargo_profile: $cargo_profile, source_revision: $source_revision,
      patch_sha256: $patch_sha256, criterion_root: $criterion_root,
      fresh_reference: $fresh_reference, command: $command,
      binaries: $binaries, scenarios: $scenarios, cases: $cases,
      launch_environment_blocked: ($launch_blocked == 1),
      budget_summary: $budget_summary,
      complete: $complete}' > "${run_dir}/manifest.json"

  manifest_written=1
  jq -r 'select(.problem != null) | "  \(.section) \(.bench): \(.problem)"' "${checked}" >&2
  [[ "${complete}" == "true" ]]
}

profile="full"
cargo_profile="release"
run_dir=""
fail_fast=0
incomplete_run_exit_code=4
frozen_bench=""
frozen_idle=""
frozen_launch=""
frozen_report=""
budget_summary=""
suite_command="$0 $*"
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
    --cargo-profile)
      cargo_profile="$2"
      shift 2
      ;;
    --run-dir)
      run_dir="$2"
      shift 2
      ;;
    --fail-fast)
      fail_fast=1
      shift
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

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"
# Absolute, so Criterion estimates and sidecars land in the same tree no
# matter which directory a harness runs from.
if [[ "${criterion_root}" != /* ]]; then
  criterion_root="${repo_root}/${criterion_root}"
fi
export GITCOMET_PERF_CRITERION_ROOT="${criterion_root}"
export CRITERION_HOME="${criterion_root}"
if [[ -z "${run_dir}" ]]; then
  run_dir="target/perf-runs/$(date -u +%Y%m%d-%H%M%SZ)"
fi
if [[ "${run_dir}" != /* ]]; then
  run_dir="${repo_root}/${run_dir}"
fi
if [[ ${dry_run} -ne 1 ]]; then
  if [[ -e "${run_dir}" ]]; then
    echo "Run directory already exists; use a fresh one: ${run_dir}" >&2
    exit 2
  fi
  mkdir -p "${run_dir}/logs"
  : > "${run_dir}/cases.jsonl"
  require_jq || exit 2
fi
budget_summary="${run_dir}/budget-summary.json"
export GITCOMET_PERF_RUN_ID="${GITCOMET_PERF_RUN_ID:-$(cat /proc/sys/kernel/random/uuid 2>/dev/null || date +%s%N)}"
export GITCOMET_PERF_CARGO_PROFILE="${cargo_profile}"

if [[ -n "${summary_log_path}" ]]; then
  mkdir -p "$(dirname "${summary_log_path}")"
  : > "${summary_log_path}"
fi
if [[ -n "${summary_jsonl_path}" ]]; then
  mkdir -p "$(dirname "${summary_jsonl_path}")"
  : > "${summary_jsonl_path}"
fi

if [[ -z "${MIMALLOC_PURGE_DELAY+x}" ]]; then
  export MIMALLOC_PURGE_DELAY=1000
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

manifest_written=0
finish_run() {
  local status="${1:-$?}"
  if [[ ${dry_run} -ne 1 && ${manifest_written} -eq 0 && -d "${run_dir}" ]]; then
    write_manifest_and_verify || true
    echo "Run stopped early (exit ${status}); manifest marks it incomplete: ${run_dir}/manifest.json" >&2
  fi
  return "${status}"
}
if [[ ${dry_run} -ne 1 ]]; then
  run_start_stamp="${run_dir}/suite-start.stamp"
  : > "${run_start_stamp}"
  trap finish_run EXIT
fi

build_measurement_binaries

record_environment() {
  local output="$1"
  local -a metadata_args=(--output "${output}" --cargo-profile "${cargo_profile}"
    --features benchmarks --command "${suite_command}")
  local name frozen
  while IFS=$'\t' read -r name frozen; do
    metadata_args+=(--binary "${name}=${frozen}")
  done < <(jq -r '[.name, .frozen] | @tsv' "${run_dir}/binaries.jsonl")
  python3 "${repo_root}/scripts/profiling/perf_metadata.py" "${metadata_args[@]}" ||
    echo "warning: could not record ${output}" >&2
}
if [[ ${dry_run} -ne 1 ]]; then
  # Machine state before and after: load, thermals and swap drift over a
  # multi-hour suite, and a busy start is reason enough to rerun.
  record_environment "${run_dir}/environment.json"
  trap 'suite_exit=$?; record_environment "${run_dir}/environment-end.json"; finish_run "${suite_exit}"' EXIT
fi

echo "Running full performance suite from: ${repo_root}"
echo "Workload profile: ${profile}"
echo "Cargo profile: ${cargo_profile}"
echo "Run directory: ${run_dir}"
echo "Run id: ${GITCOMET_PERF_RUN_ID}"
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
    if [[ ${dry_run} -eq 1 ]]; then
      run_section "Idle resource: ${bench}" "${frozen_idle}" --bench "${bench}"
      continue
    fi
    echo
    echo "==> Idle resource: ${bench}"
    if run_case idle "${bench}" "${frozen_idle}" --bench "${bench}"; then
      emit_bench_summary "${bench}" "idle"
    else
      case_failed_or_stop || exit $?
    fi
  done
fi

if [[ ${run_launch} -eq 1 ]]; then
  run_launch_suite
fi

if [[ ${run_launch} -eq 1 ]]; then
  verify_launch_sidecars || case_failed_or_stop || exit $?
fi

if [[ ${run_report} -eq 1 ]]; then
  run_section \
    "Performance budget report" \
    "${frozen_report}" \
    "${report_args[@]}"
fi

if [[ ${dry_run} -ne 1 ]]; then
  if write_manifest_and_verify; then
    echo "Run complete: every selected scenario produced fresh results (${run_dir}/manifest.json)."
  elif [[ ${launch_suite_environment_blocked} -eq 1 ]]; then
    # A blocked launch suite is incomplete by construction; its own exit
    # code tells callers to fix the environment rather than the build.
    echo "App launch suite did not complete because perf-app-launch reported an environment blocker; returning exit ${app_launch_environment_blocker_exit_code} after report completion (${run_dir}/manifest.json)." >&2
    exit "${app_launch_environment_blocker_exit_code}"
  else
    echo "Run INCOMPLETE: see ${run_dir}/manifest.json. Do not accept results from this run." >&2
    exit "${incomplete_run_exit_code}"
  fi
fi
