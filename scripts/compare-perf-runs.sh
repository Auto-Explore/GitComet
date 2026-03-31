#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/compare-perf-runs.sh [options] BASE_RUN CANDIDATE_RUN

Compare two archived performance runs produced by scripts/archive-perf-run.sh.
Matching is done by benchmark name, with fallbacks for alternate field names.

Run arguments may be:
  - a run id under tmp/perf-records (for example 20260331-062033Z)
  - a path to an archived run directory
  - a path to benchmark-metrics.jsonl

Options:
  --archive-root PATH   Base directory for run ids. Default: tmp/perf-records
  --metric NAME         Metric to compare. Repeatable.
                        Default:
                          mean_ns
                          avg_cpu_pct
                          rss_delta_kib
                          alloc_bytes
                          net_alloc_bytes
                          first_paint_alloc_bytes
                          first_interactive_alloc_bytes
                        Criterion aliases:
                          mean_ns, mean_upper_ns, median_ns, std_dev_ns
                        Sidecar examples:
                          avg_cpu_pct, rss_delta_kib, alloc_bytes,
                          net_alloc_bytes, first_paint_ms
  --kind NAME           Filter by benchmark kind: all, criterion, idle, launch
                        Default: all
  --direction MODE      Metric direction: lower, higher, neutral
                        Default: lower
  --sort FIELD          Sort rows by: regression, delta, abs_delta, name
                        Default: name
  --limit N             Max rows. Use 0 for all rows.
                        Default: 0
  --only-regressions    Show only worsening rows according to --direction
  -h, --help            Show this help.

Examples:
  scripts/compare-perf-runs.sh linux-before linux-after
  scripts/compare-perf-runs.sh --kind idle linux-before linux-after
  scripts/compare-perf-runs.sh --metric mean_ns --metric avg_cpu_pct --metric alloc_bytes linux-before linux-after
EOF
}

require_jq() {
  if command -v jq >/dev/null 2>&1; then
    return 0
  fi

  echo "jq is required to compare archived performance runs." >&2
  exit 1
}

format_number() {
  awk -v value="$1" 'BEGIN {
    if (value == "" || value == "null") {
      printf "n/a";
    } else if (value == int(value)) {
      printf "%.0f", value;
    } else {
      printf "%.3f", value;
    }
  }'
}

format_bytes() {
  awk -v value="$1" 'BEGIN {
    abs = value;
    if (abs < 0) abs = -abs;
    if (value == "" || value == "null") {
      printf "n/a";
    } else if (abs >= 1073741824) {
      printf "%.2f GiB", value / 1073741824;
    } else if (abs >= 1048576) {
      printf "%.2f MiB", value / 1048576;
    } else if (abs >= 1024) {
      printf "%.2f KiB", value / 1024;
    } else {
      printf "%.0f B", value;
    }
  }'
}

format_ms() {
  awk -v value="$1" 'BEGIN {
    if (value == "" || value == "null") {
      printf "n/a";
    } else {
      printf "%.3f ms", value;
    }
  }'
}

format_kib() {
  awk -v value="$1" 'BEGIN {
    abs = value;
    if (abs < 0) abs = -abs;
    if (value == "" || value == "null") {
      printf "n/a";
    } else if (abs >= 1048576) {
      printf "%.2f GiB", value / 1048576;
    } else if (abs >= 1024) {
      printf "%.2f MiB", value / 1024;
    } else {
      printf "%.0f KiB", value;
    }
  }'
}

format_pct_value() {
  awk -v value="$1" 'BEGIN {
    if (value == "" || value == "null") {
      printf "n/a";
    } else {
      printf "%.3f%%", value;
    }
  }'
}

format_duration_ns() {
  awk -v ns="$1" 'BEGIN {
    if (ns == "" || ns == "null") {
      printf "n/a";
    } else if (ns >= 1000000 || ns <= -1000000) {
      printf "%.3f ms", ns / 1000000;
    } else if (ns >= 1000 || ns <= -1000) {
      printf "%.3f us", ns / 1000;
    } else {
      printf "%.0f ns", ns;
    }
  }'
}

metric_key() {
  case "$1" in
    mean_ns|criterion.mean_ns)
      printf 'criterion.mean_ns\n'
      ;;
    mean_upper_ns|criterion.mean_upper_ns)
      printf 'criterion.mean_upper_ns\n'
      ;;
    median_ns|criterion.median_ns)
      printf 'criterion.median_ns\n'
      ;;
    std_dev_ns|criterion.std_dev_ns)
      printf 'criterion.std_dev_ns\n'
      ;;
    *)
      printf 'sidecar:%s\n' "$1"
      ;;
  esac
}

metric_column_stem() {
  case "$1" in
    criterion.mean_ns)
      printf 'time\n'
      ;;
    criterion.mean_upper_ns)
      printf 'time95\n'
      ;;
    criterion.median_ns)
      printf 'median\n'
      ;;
    criterion.std_dev_ns)
      printf 'stdev\n'
      ;;
    sidecar:avg_cpu_pct)
      printf 'cpu\n'
      ;;
    sidecar:peak_cpu_pct)
      printf 'cpu_peak\n'
      ;;
    sidecar:rss_delta_kib)
      printf 'rss\n'
      ;;
    sidecar:alloc_bytes)
      printf 'alloc\n'
      ;;
    sidecar:net_alloc_bytes)
      printf 'net_alloc\n'
      ;;
    sidecar:first_paint_alloc_bytes)
      printf 'fp_alloc\n'
      ;;
    sidecar:first_interactive_alloc_bytes)
      printf 'fi_alloc\n'
      ;;
    sidecar:first_paint_ms)
      printf 'fp_ms\n'
      ;;
    sidecar:first_interactive_ms)
      printf 'fi_ms\n'
      ;;
    sidecar:*)
      printf '%s\n' "${1#sidecar:}"
      ;;
  esac
}

format_metric_value() {
  local metric="$1"
  local value="$2"

  case "$metric" in
    criterion.*_ns|criterion.mean_ns|criterion.median_ns)
      format_duration_ns "${value}"
      ;;
    sidecar:*_bytes)
      format_bytes "${value}"
      ;;
    sidecar:*_kib)
      format_kib "${value}"
      ;;
    sidecar:*_ms)
      format_ms "${value}"
      ;;
    sidecar:*_pct)
      format_pct_value "${value}"
      ;;
    *)
      format_number "${value}"
      ;;
  esac
}

format_change_cell() {
  local status="$1"
  local delta_pct="$2"

  case "$status" in
    ""|null)
      printf -- '-'
      ;;
    unchanged)
      printf 'same'
      ;;
    improved)
      awk -v v="${delta_pct}" 'BEGIN {
        if (v < 0) v = -v;
        printf "imp %.2f%%", v;
      }'
      ;;
    regressed)
      awk -v v="${delta_pct}" 'BEGIN {
        if (v < 0) v = -v;
        printf "reg %.2f%%", v;
      }'
      ;;
    missing_base|missing_candidate|incomparable)
      printf -- '-'
      ;;
    *)
      awk -v v="${delta_pct}" 'BEGIN {
        if (v < 0) v = -v;
        printf "chg %.2f%%", v;
      }'
      ;;
  esac
}

markdown_escape() {
  local value="$1"
  value="${value//$'\n'/ }"
  value="${value//\\/\\\\}"
  value="${value//|/\\|}"
  printf '%s' "${value}"
}

metadata_get() {
  local file="$1"
  local key="$2"
  awk -F': ' -v wanted="$key" '$1 == wanted { print substr($0, index($0, ": ") + 2); exit }' "$file"
}

abs_path() {
  local path="$1"
  if [[ -d "$path" ]]; then
    (cd "$path" && pwd)
  else
    local parent
    local base
    parent="$(cd "$(dirname "$path")" && pwd)"
    base="$(basename "$path")"
    printf '%s/%s\n' "$parent" "$base"
  fi
}

resolve_run() {
  local input="$1"
  local run_dir=""
  local jsonl_path=""
  local metadata_path=""

  if [[ -f "$input" ]]; then
    jsonl_path="$(abs_path "$input")"
    run_dir="$(dirname "$jsonl_path")"
  elif [[ -d "$input" ]]; then
    run_dir="$(abs_path "$input")"
    jsonl_path="${run_dir}/benchmark-metrics.jsonl"
  else
    run_dir="$(abs_path "${archive_root%/}/${input}")"
    jsonl_path="${run_dir}/benchmark-metrics.jsonl"
  fi

  metadata_path="${run_dir}/metadata.txt"

  if [[ ! -f "${jsonl_path}" ]]; then
    echo "Missing benchmark metrics file: ${jsonl_path}" >&2
    return 1
  fi
  if [[ ! -f "${metadata_path}" ]]; then
    echo "Missing metadata file: ${metadata_path}" >&2
    return 1
  fi

  printf '%s|%s|%s\n' "${run_dir}" "${jsonl_path}" "${metadata_path}"
}

print_run_summary() {
  local label="$1"
  local run_dir="$2"
  local metadata_path="$3"
  local run_id
  local git_head
  local git_branch
  local runner_class
  local real_repo_root

  run_id="$(metadata_get "${metadata_path}" "run_id")"
  git_head="$(metadata_get "${metadata_path}" "git_head")"
  git_branch="$(metadata_get "${metadata_path}" "git_branch")"
  runner_class="$(metadata_get "${metadata_path}" "runner_class")"
  real_repo_root="$(metadata_get "${metadata_path}" "real_repo_root")"

  echo "## ${label}"
  echo
  echo "- run: ${run_id:-$(basename "${run_dir}")}"
  echo "- dir: ${run_dir}"
  echo "- git: ${git_branch:-unknown} ${git_head:-unknown}"
  echo "- runner_class: ${runner_class:-<unset>}"
  echo "- real_repo_root: ${real_repo_root:-<unset>}"
}

compare_metrics_table() {
  local tmp_file
  local only_regressions_json="false"
  local metrics_json="["
  local metric=""
  local metric_jq=""
  local metric_count=0
  local group_count=0
  local group_index=0

  tmp_file="$(mktemp)"
  if [[ ${only_regressions} -eq 1 ]]; then
    only_regressions_json="true"
  fi

  for metric in "${metrics[@]}"; do
    metric_jq="$(metric_key "${metric}")"
    if [[ ${metric_count} -gt 0 ]]; then
      metrics_json+=", "
    fi
    metrics_json+="\"${metric_jq}\""
    metric_count=$((metric_count + 1))
  done
  metrics_json+="]"

  jq -s \
    --slurpfile cand "${candidate_jsonl}" \
    --argjson metrics "${metrics_json}" \
    --arg kind "${kind_filter}" \
    --arg direction "${direction}" \
    --arg sort_by "${sort_by}" \
    --argjson limit "${limit}" \
    --argjson only_regressions "${only_regressions_json}" '
      def benchmark_name($row):
        $row.bench
        // $row.benchmark_name
        // $row.benchmark
        // $row.name
        // $row.id
        // (
          [
            $row.estimates_path?,
            $row.sidecar_path?
          ]
          | map(select(. != null and . != ""))
          | map(
              if test("/new/") then
                split("/new/")[0]
              else
                .
              end
              | sub("^target/criterion/"; "")
              | sub("^crates/gitcomet-ui-gpui/target/criterion/"; "")
            )
          | .[0]?
        );

      def idx(xs):
        reduce xs[] as $x (
          {};
          (benchmark_name($x)) as $name
          | if $name == null or $name == "" then
              .
            else
              .[$name] = ((.[$name] // []) + [$x])
            end
        );

      def metric_value($row; $metric):
        if $metric == "criterion.mean_ns" then $row.criterion.mean_ns
        elif $metric == "criterion.mean_upper_ns" then $row.criterion.mean_upper_ns
        elif $metric == "criterion.median_ns" then $row.criterion.median_ns
        elif $metric == "criterion.std_dev_ns" then $row.criterion.std_dev_ns
        elif ($metric | startswith("sidecar:")) then $row.metrics[$metric | ltrimstr("sidecar:")]
        else null
        end;

      def status_for($direction; $delta):
        if $delta == 0 then
          "unchanged"
        elif $direction == "lower" then
          (if $delta < 0 then "improved" else "regressed" end)
        elif $direction == "higher" then
          (if $delta > 0 then "improved" else "regressed" end)
        else
          "changed"
        end;

      def metric_entry($old; $new; $metric; $direction):
        (metric_value($old; $metric)) as $before
        | (metric_value($new; $metric)) as $after
        | if $before == null or $after == null then
            {
              metric: $metric,
              before: null,
              after: null,
              delta_pct: null,
              status: null
            }
          else
            (($after - $before)) as $delta
            | (
                if $before == 0 and $after == 0 then
                  0
                elif $before == 0 then
                  null
                else
                  (($delta / $before) * 100)
                end
              ) as $delta_pct
            | (status_for($direction; $delta)) as $status
            | {
                metric: $metric,
                before: $before,
                after: $after,
                delta_pct: $delta_pct,
                status: $status
              }
          end;

      . as $base
      | (idx($base)) as $base_idx
      | (idx($cand)) as $cand_idx
      | (
          [
            $base_idx
            | to_entries[]
            | select((.value | length) > 1)
            | .key
          ]
        ) as $base_duplicate_names
      | (
          [
            $cand_idx
            | to_entries[]
            | select((.value | length) > 1)
            | .key
          ]
        ) as $candidate_duplicate_names
      | [
          (
            (($base_idx | keys_unsorted) + ($cand_idx | keys_unsorted))
            | unique[]
          ) as $bench_name
          | ($base_idx[$bench_name] // []) as $old_rows
          | ($cand_idx[$bench_name] // []) as $new_rows
          | select(($old_rows | length) <= 1 and ($new_rows | length) <= 1)
          | ($old_rows[0]?) as $old
          | ($new_rows[0]?) as $new
          | (($new.kind // $old.kind) // null) as $resolved_kind
          | select($kind == "all" or $resolved_kind == $kind)
          | {
              kind: $resolved_kind,
              base_kind: $old.kind,
              candidate_kind: $new.kind,
              bench: $bench_name,
              base_present: ($old != null),
              candidate_present: ($new != null),
              metrics: [ $metrics[] as $metric | metric_entry($old; $new; $metric; $direction) ]
            }
          | . + {
              comparable_count: ([ .metrics[] | select(.status != null) ] | length),
              improved_count: ([ .metrics[] | select(.status == "improved") ] | length),
              regressed_count: ([ .metrics[] | select(.status == "regressed") ] | length),
              changed_count: ([ .metrics[] | select(.status == "changed") ] | length),
              first_delta_pct: ([ .metrics[] | select(.delta_pct != null) | .delta_pct ][0] // null),
              max_regression_score: (
                [ .metrics[]
                  | select(.status == "regressed" and .delta_pct != null)
                  | (if .delta_pct < 0 then -.delta_pct else .delta_pct end)
                ] | max // 0
              ),
              max_abs_delta_score: (
                [ .metrics[]
                  | select(.delta_pct != null)
                  | (if .delta_pct < 0 then -.delta_pct else .delta_pct end)
                ] | max // 0
              )
            }
          | . + {
              status: (
                if (.base_present | not) then
                  "missing_base"
                elif (.candidate_present | not) then
                  "missing_candidate"
                elif .comparable_count == 0 then
                  "incomparable"
                elif .improved_count > 0 and .regressed_count == 0 and .changed_count == 0 then
                  "improved"
                elif .regressed_count > 0 and .improved_count == 0 and .changed_count == 0 then
                  "regressed"
                elif .improved_count == 0 and .regressed_count == 0 and .changed_count == 0 then
                  "unchanged"
                else
                  "mixed"
                end
              )
            }
        ] as $rows
      | ($rows | length) as $matched
      | (
          if $only_regressions then
            [ $rows[] | select(.max_regression_score > 0) ]
          else
            $rows
          end
        ) as $filtered
      | (
          if $sort_by == "name" then
            ($filtered | sort_by(.bench, .kind))
          elif $sort_by == "delta" then
            ($filtered | sort_by(.first_delta_pct // -1e308, .bench, .kind) | reverse)
          elif $sort_by == "abs_delta" then
            ($filtered | sort_by(.max_abs_delta_score, .bench, .kind) | reverse)
          else
            ($filtered | sort_by(.max_regression_score, .bench, .kind) | reverse)
          end
        ) as $sorted
      | {
          matched: $matched,
          displayed: (
            if $limit == 0 then
              ($sorted | length)
            else
              ($sorted[:$limit] | length)
            end
          ),
          duplicate_names: {
            base: $base_duplicate_names,
            candidate: $candidate_duplicate_names
          },
          groups: (
            [
              "criterion",
              "idle",
              "launch"
            ]
            | map(
                . as $kind_name
                | (
                    if $limit == 0 then
                      $sorted
                    else
                      $sorted[:$limit]
                    end
                    | map(select(.kind == $kind_name))
                  ) as $kind_rows
                | select(($kind_rows | length) > 0)
                | {
                    kind: $kind_name,
                    metrics: [
                      $metrics[] as $metric
                      | select(any($kind_rows[]; any(.metrics[]; .metric == $metric and .status != null)))
                      | $metric
                    ],
                    rows: $kind_rows
                  }
              )
          )
        }
    ' "${base_jsonl}" > "${tmp_file}"

  if [[ "$(jq -r '.matched' "${tmp_file}")" == "0" ]]; then
    rm -f "${tmp_file}"
    return 0
  fi

  local matched_count
  local displayed_count
  local duplicate_base_count
  local duplicate_candidate_count
  matched_count="$(jq -r '.matched' "${tmp_file}")"
  displayed_count="$(jq -r '.displayed' "${tmp_file}")"
  duplicate_base_count="$(jq -r '.duplicate_names.base | length' "${tmp_file}")"
  duplicate_candidate_count="$(jq -r '.duplicate_names.candidate | length' "${tmp_file}")"

  echo
  echo "- matched benchmarks: ${matched_count}"
  if [[ "${displayed_count}" != "${matched_count}" ]]; then
    echo "- showing benchmarks: ${displayed_count} (truncated by --limit=${limit})"
  fi
  if [[ "${duplicate_base_count}" != "0" ]]; then
    echo "warning: skipped ${duplicate_base_count} ambiguous benchmark name(s) in base run" >&2
  fi
  if [[ "${duplicate_candidate_count}" != "0" ]]; then
    echo "warning: skipped ${duplicate_candidate_count} ambiguous benchmark name(s) in candidate run" >&2
  fi

  group_count="$(jq -r '.groups | length' "${tmp_file}")"
  for ((group_index = 0; group_index < group_count; group_index++)); do
    local kind_name
    local row_tsv_filter=""
    local -a group_metrics=()
    local -a header=()

    kind_name="$(jq -r ".groups[${group_index}].kind" "${tmp_file}")"
    mapfile -t group_metrics < <(jq -r ".groups[${group_index}].metrics[]" "${tmp_file}")

    echo
    echo "### ${kind_name}"
    echo

    header=("bench" "status")
    for metric_jq in "${group_metrics[@]}"; do
      header+=("$(metric_column_stem "${metric_jq}")_base")
      header+=("$(metric_column_stem "${metric_jq}")_cand")
      header+=("$(metric_column_stem "${metric_jq}")%")
    done

    printf '|'
    for column in "${header[@]}"; do
      printf ' %s |' "$(markdown_escape "${column}")"
    done
    printf '\n'

    printf '|'
    for ((i = 0; i < ${#header[@]}; i++)); do
      printf ' --- |'
    done
    printf '\n'

    row_tsv_filter=".groups[${group_index}] as \$g | \$g.rows[] | . as \$row | [\$row.bench, \$row.status]"
    for metric_jq in "${group_metrics[@]}"; do
      row_tsv_filter+=" + [((\$row.metrics[] | select(.metric == \"${metric_jq}\") | (.before // \"null\")) | tostring)]"
      row_tsv_filter+=" + [((\$row.metrics[] | select(.metric == \"${metric_jq}\") | (.after // \"null\")) | tostring)]"
      row_tsv_filter+=" + [((\$row.metrics[] | select(.metric == \"${metric_jq}\") | (.delta_pct // \"null\")) | tostring)]"
      row_tsv_filter+=" + [((\$row.metrics[] | select(.metric == \"${metric_jq}\") | (.status // \"null\")) | tostring)]"
    done
    row_tsv_filter+=" | @tsv"

    while IFS=$'\t' read -r -a fields; do
      local field_index=2
      printf '| %s | %s |' \
        "$(markdown_escape "${fields[0]}")" \
        "$(markdown_escape "${fields[1]}")"
      for metric_jq in "${group_metrics[@]}"; do
        local before_raw="${fields[$field_index]}"
        local after_raw="${fields[$((field_index + 1))]}"
        local delta_pct_raw="${fields[$((field_index + 2))]}"
        local status_raw="${fields[$((field_index + 3))]}"
        printf ' %s |' "$(markdown_escape "$(format_metric_value "${metric_jq}" "${before_raw}")")"
        printf ' %s |' "$(markdown_escape "$(format_metric_value "${metric_jq}" "${after_raw}")")"
        printf ' %s |' "$(markdown_escape "$(format_change_cell "${status_raw}" "${delta_pct_raw}")")"
        field_index=$((field_index + 4))
      done
      printf '\n'
    done < <(jq -r "${row_tsv_filter}" "${tmp_file}")
  done

  rm -f "${tmp_file}"
}

archive_root="tmp/perf-records"
kind_filter="all"
direction="lower"
sort_by="name"
limit=0
only_regressions=0
metrics=()
positionals=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --archive-root)
      archive_root="$2"
      shift 2
      ;;
    --metric)
      metrics+=("$2")
      shift 2
      ;;
    --kind)
      kind_filter="$2"
      shift 2
      ;;
    --direction)
      direction="$2"
      shift 2
      ;;
    --sort)
      sort_by="$2"
      shift 2
      ;;
    --limit)
      limit="$2"
      shift 2
      ;;
    --only-regressions)
      only_regressions=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      positionals+=("$1")
      shift
      ;;
  esac
done

if [[ ${#metrics[@]} -eq 0 ]]; then
  metrics=(
    "mean_ns"
    "avg_cpu_pct"
    "rss_delta_kib"
    "alloc_bytes"
    "net_alloc_bytes"
    "first_paint_ms"
    "first_interactive_ms"
    "first_paint_alloc_bytes"
    "first_interactive_alloc_bytes"
  )
fi

case "${kind_filter}" in
  all|criterion|idle|launch)
    ;;
  *)
    echo "Unknown --kind value: ${kind_filter}" >&2
    exit 2
    ;;
esac

case "${direction}" in
  lower|higher|neutral)
    ;;
  *)
    echo "Unknown --direction value: ${direction}" >&2
    exit 2
    ;;
esac

case "${sort_by}" in
  regression|delta|abs_delta|name)
    ;;
  *)
    echo "Unknown --sort value: ${sort_by}" >&2
    exit 2
    ;;
esac

if ! [[ "${limit}" =~ ^[0-9]+$ ]]; then
  echo "--limit must be a non-negative integer" >&2
  exit 2
fi

if [[ ${#positionals[@]} -ne 2 ]]; then
  usage >&2
  exit 2
fi

require_jq

if ! IFS='|' read -r base_run_dir base_jsonl base_metadata <<< "$(resolve_run "${positionals[0]}")"; then
  exit 1
fi
if ! IFS='|' read -r candidate_run_dir candidate_jsonl candidate_metadata <<< "$(resolve_run "${positionals[1]}")"; then
  exit 1
fi

echo "# Comparing archived performance runs"
echo
print_run_summary "base" "${base_run_dir}" "${base_metadata}"
echo
print_run_summary "candidate" "${candidate_run_dir}" "${candidate_metadata}"
if [[ "${limit}" == "0" ]]; then
  limit_label="all"
else
  limit_label="${limit}"
fi

echo
echo "- filters: kind=${kind_filter} direction=${direction} sort=${sort_by} limit=${limit_label} only_regressions=${only_regressions}"

base_git_head="$(metadata_get "${base_metadata}" "git_head")"
candidate_git_head="$(metadata_get "${candidate_metadata}" "git_head")"
base_runner_class="$(metadata_get "${base_metadata}" "runner_class")"
candidate_runner_class="$(metadata_get "${candidate_metadata}" "runner_class")"
base_real_repo_root="$(metadata_get "${base_metadata}" "real_repo_root")"
candidate_real_repo_root="$(metadata_get "${candidate_metadata}" "real_repo_root")"

if [[ "${base_git_head}" != "${candidate_git_head}" ]]; then
  echo "> Warning: git_head differs between runs"
fi
if [[ "${base_runner_class}" != "${candidate_runner_class}" ]]; then
  echo "> Warning: runner_class differs between runs"
fi
if [[ "${base_real_repo_root}" != "${candidate_real_repo_root}" ]]; then
  echo "> Warning: real_repo_root differs between runs"
fi

echo
compare_metrics_table
