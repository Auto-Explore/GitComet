#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/archive-perf-run.sh [wrapper-options] [run-full-perf-suite options]

Runs scripts/run-full-perf-suite.sh, captures its console log, and snapshots
the resulting benchmark artifacts into a timestamped archive directory.

Wrapper options:
  --archive-root PATH   Parent directory for saved runs.
                        Default: tmp/perf-records
  --run-id NAME         Archive directory name under --archive-root.
                        Default: UTC timestamp (YYYYMMDD-HHMMSSZ)
  -h, --help            Show this help.

All other arguments are passed through to scripts/run-full-perf-suite.sh.

Reserved passthrough options:
  --criterion-root
  --fresh-reference

This wrapper manages those two options itself so the archived report can be
replayed later against the saved artifact tree with the same freshness stamp.

Artifacts written per archived run:
  full-suite.log        Full console output from run-full-perf-suite.sh
  benchmark-metrics.log Human-readable per-benchmark metric summaries
  benchmark-metrics.jsonl
                        Structured per-benchmark metric summaries
  budget-report.md      Perf budget report regenerated from archived artifacts
  metadata.txt          Run metadata and exact replay commands
  suite-start.stamp     Freshness reference used for this archived run
  criterion/            Snapshotted Criterion + sidecar artifact tree

Examples:
  scripts/archive-perf-run.sh
  scripts/archive-perf-run.sh --run-id linux-main --profile full --strict
  scripts/archive-perf-run.sh --archive-root tmp/perf-records-local --profile balanced
EOF
}

quote_args() {
  local quoted=""
  printf -v quoted '%q ' "$@"
  printf '%s\n' "${quoted% }"
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

archive_root="tmp/perf-records"
run_id="$(date -u +%Y%m%d-%H%M%SZ)"
suite_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --archive-root)
      archive_root="$2"
      shift 2
      ;;
    --run-id)
      run_id="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      suite_args+=("$@")
      break
      ;;
    *)
      suite_args+=("$1")
      shift
      ;;
  esac
done

strict_report=0
suite_dry_run=0
for ((i = 0; i < ${#suite_args[@]}; i++)); do
  case "${suite_args[$i]}" in
    --criterion-root|--fresh-reference)
      echo "Do not pass ${suite_args[$i]} to archive-perf-run.sh." >&2
      echo "This wrapper owns that option so the saved archive stays self-consistent." >&2
      exit 2
      ;;
    --criterion-root=*|--fresh-reference=*)
      echo "Do not pass ${suite_args[$i]%%=*} to archive-perf-run.sh." >&2
      echo "This wrapper owns that option so the saved archive stays self-consistent." >&2
      exit 2
      ;;
    --strict)
      strict_report=1
      ;;
    --dry-run)
      suite_dry_run=1
      ;;
  esac
done

archive_dir="${archive_root%/}/${run_id}"
fresh_reference="${archive_dir}/suite-start.stamp"
suite_log="${archive_dir}/full-suite.log"
budget_report="${archive_dir}/budget-report.md"
metadata_path="${archive_dir}/metadata.txt"
summary_log="${archive_dir}/benchmark-metrics.log"
summary_jsonl="${archive_dir}/benchmark-metrics.jsonl"
archive_criterion_root="${archive_dir}/criterion"
criterion_source_roots=(
  "target/criterion"
  "crates/gitcomet-ui-gpui/target/criterion"
)

if [[ -e "${archive_dir}" ]]; then
  echo "Archive directory already exists: ${archive_dir}" >&2
  exit 1
fi

mkdir -p "${archive_dir}"
: > "${fresh_reference}"

suite_cmd=(
  env
  GITCOMET_PERF_PRINT_BENCH_SUMMARY=1
  GITCOMET_PERF_SUMMARY_LOG="${summary_log}"
  GITCOMET_PERF_SUMMARY_JSONL="${summary_jsonl}"
  bash scripts/run-full-perf-suite.sh
  --fresh-reference "${fresh_reference}"
)
suite_cmd+=("${suite_args[@]}")

report_cmd=(
  cargo run -p gitcomet-ui-gpui --bin perf_budget_report --
  --criterion-root "${archive_criterion_root}"
  --fresh-reference "${fresh_reference}"
)
if [[ ${strict_report} -eq 1 ]]; then
  report_cmd+=(--strict)
else
  report_cmd+=(--skip-missing)
fi

git_head="$(git rev-parse HEAD 2>/dev/null || true)"
git_branch="$(git symbolic-ref --short HEAD 2>/dev/null || echo DETACHED)"
git_status="$(git status --short 2>/dev/null || true)"

{
  printf 'run_id: %s\n' "${run_id}"
  printf 'created_utc: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'archive_dir: %s\n' "${repo_root}/${archive_dir}"
  printf 'criterion_archive_root: %s\n' "${repo_root}/${archive_criterion_root}"
  printf 'fresh_reference: %s\n' "${repo_root}/${fresh_reference}"
  printf 'source_criterion_roots: %s\n' "$(quote_args "${criterion_source_roots[@]/#/${repo_root}/}")"
  printf 'benchmark_summary_log: %s\n' "${repo_root}/${summary_log}"
  printf 'benchmark_summary_jsonl: %s\n' "${repo_root}/${summary_jsonl}"
  printf 'git_head: %s\n' "${git_head}"
  printf 'git_branch: %s\n' "${git_branch}"
  printf 'runner_class: %s\n' "${GITCOMET_PERF_RUNNER_CLASS:-}"
  printf 'real_repo_root: %s\n' "${GITCOMET_PERF_REAL_REPO_ROOT:-}"
  printf 'suite_command: %s\n' "$(quote_args "${suite_cmd[@]}")"
  printf 'archived_report_command: %s\n' "$(quote_args "${report_cmd[@]}")"
  if [[ -n "${git_status}" ]]; then
    printf 'git_status:\n%s\n' "${git_status}"
  else
    printf 'git_status: clean\n'
  fi
} > "${metadata_path}"

echo "Archiving performance run under: ${repo_root}/${archive_dir}"
echo "Freshness reference: ${repo_root}/${fresh_reference}"
echo "Suite command: $(quote_args "${suite_cmd[@]}")"

set +e
"${suite_cmd[@]}" 2>&1 | tee "${suite_log}"
suite_status=${PIPESTATUS[0]}
set -e

report_status=0

if [[ ${suite_dry_run} -eq 1 ]]; then
  {
    echo "Dry run requested."
    echo "No benchmark artifacts were copied into this archive."
  } > "${budget_report}"
elif [[ ${suite_status} -ne 0 ]]; then
  {
    echo "Suite command failed with exit ${suite_status}."
    echo "Skipped artifact snapshot and archived budget replay to avoid archiving stale Criterion data."
    echo "See full-suite.log for the primary failure."
  } > "${budget_report}"
else
  copied_any_criterion_root=0
  mkdir -p "${archive_criterion_root}"
  for source_criterion_root in "${criterion_source_roots[@]}"; do
    if [[ ! -d "${source_criterion_root}" ]]; then
      continue
    fi
    cp -a "${source_criterion_root}/." "${archive_criterion_root}/"
    copied_any_criterion_root=1
  done

  if [[ ${copied_any_criterion_root} -eq 0 ]]; then
    {
      echo "Criterion artifact roots not found after suite run."
      for source_criterion_root in "${criterion_source_roots[@]}"; do
        echo "Missing: ${repo_root}/${source_criterion_root}"
      done
      echo "Later report replay from this archive is unavailable."
    } | tee -a "${suite_log}" >&2
    report_status=1
  fi

  if [[ ${report_status} -eq 0 ]]; then
    set +e
    "${report_cmd[@]}" 2>&1 | tee "${budget_report}"
    report_status=${PIPESTATUS[0]}
    set -e
  fi
fi

echo "Saved suite log: ${repo_root}/${suite_log}"
echo "Saved metadata: ${repo_root}/${metadata_path}"
if [[ -f "${summary_log}" ]]; then
  echo "Saved per-benchmark summary log: ${repo_root}/${summary_log}"
fi
if [[ -f "${summary_jsonl}" ]]; then
  echo "Saved per-benchmark summary JSONL: ${repo_root}/${summary_jsonl}"
fi
if [[ -f "${budget_report}" ]]; then
  echo "Saved archived report: ${repo_root}/${budget_report}"
fi
if [[ -d "${archive_criterion_root}" ]]; then
  echo "Saved artifact tree: ${repo_root}/${archive_criterion_root}"
fi

if [[ ${suite_status} -ne 0 ]]; then
  exit "${suite_status}"
fi

exit "${report_status}"
