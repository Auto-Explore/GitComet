#!/usr/bin/env bash
set -euo pipefail
shopt -s nullglob

usage() {
  cat <<'EOF'
Usage: scripts/profile-gitcomet-process-tree.sh [options] REPO_PATH [-- extra_gitcomet_args...]

Launch GitComet under a process-tree profiling wrapper that captures:
  - callgrind outputs for the parent process and traced child processes
  - strace -ff per-process syscall logs
  - Git Trace2 event/perf logs for git subprocesses

Options:
  --binary PATH             GitComet binary to launch.
                            Default: ./target/release-with-debug/gitcomet
  --out-dir PATH            Output directory for captured artifacts.
                            Default: tmp/gitcomet-profiles/<utc-timestamp>
  --timeout SEC             Terminate the profiled process group after SEC seconds.
  --manual-instrumentation  Start callgrind with instrumentation disabled.
                            Turn it on later with callgrind_control.
  --dry-run                 Print the composed command and output paths without running.
  -h, --help                Show this help.

Examples:
  scripts/profile-gitcomet-process-tree.sh /home/sampo/chromium/src
  scripts/profile-gitcomet-process-tree.sh --timeout 60 /home/sampo/chromium/src
  scripts/profile-gitcomet-process-tree.sh \
    --manual-instrumentation \
    /home/sampo/chromium/src -- --version
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

quote_args() {
  local quoted=""
  printf -v quoted '%q ' "$@"
  printf '%s\n' "${quoted% }"
}

require_tool() {
  local tool_name="$1"
  if ! command -v "$tool_name" >/dev/null 2>&1; then
    die "Required tool not found: ${tool_name}"
  fi
}

verify_strace_usable() {
  local probe_target="/usr/bin/true"
  if [[ ! -x "$probe_target" ]]; then
    probe_target="/bin/true"
  fi
  if [[ ! -x "$probe_target" ]]; then
    die "Could not find a probe target for strace usability checks."
  fi

  local probe_log
  probe_log="$(mktemp)"
  local probe_stderr=""
  if probe_stderr="$(strace -o "$probe_log" "$probe_target" 2>&1 >/dev/null)"; then
    rm -f "$probe_log"
    return
  fi
  rm -f "$probe_log"

  probe_stderr="$(trim_whitespace "${probe_stderr//$'\n'/ }")"
  if [[ -z "$probe_stderr" ]]; then
    probe_stderr="ptrace permission denied or strace is blocked in this environment"
  fi
  die "strace cannot trace processes in this environment: ${probe_stderr}"
}

trim_whitespace() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s\n' "$value"
}

resolve_existing_dir() {
  local raw="$1"
  local candidate="$raw"
  if [[ "$candidate" != /* ]]; then
    candidate="${orig_cwd}/${candidate}"
  fi
  [[ -d "$candidate" ]] || die "Repository path is not a directory: ${raw}"
  (
    cd "$candidate"
    pwd -P
  )
}

resolve_path() {
  local raw="$1"
  if [[ "$raw" = /* ]]; then
    printf '%s\n' "$raw"
  else
    printf '%s/%s\n' "$orig_cwd" "$raw"
  fi
}

host_name() {
  if command -v hostname >/dev/null 2>&1; then
    hostname
  else
    uname -n
  fi
}

append_summary_header() {
  local title="$1"
  {
    printf '\n== %s ==\n' "$title"
  } >>"$summary_path"
}

append_execve_summary() {
  local trace_files=( "$strace_dir"/trace.* )
  append_summary_header "Strace Execve Summary"
  if ((${#trace_files[@]} == 0)); then
    printf 'No strace trace files were captured.\n' >>"$summary_path"
    return
  fi

  {
    printf 'Trace files: %d\n' "${#trace_files[@]}"
  } >>"$summary_path"

  local emitted=0
  local trace_file=""
  local pid=""
  local line=""
  local display=""
  while IFS= read -r trace_file; do
    pid="${trace_file##*.}"
    line="$(awk '/execve\(/ { print; exit }' "$trace_file" || true)"
    if [[ -z "$line" ]]; then
      continue
    fi
    display="$(printf '%s\n' "$line" | sed -E 's/^[^e]*execve\("([^"]+)", \[(.*)\], .*$/\1 [\2]/')"
    if [[ "$display" == "$line" ]]; then
      display="$(trim_whitespace "$line")"
    fi
    {
      printf 'PID %s: %s\n' "$pid" "$display"
    } >>"$summary_path"
    emitted=$((emitted + 1))
    if ((emitted >= 80)); then
      {
        printf '...truncated after %d processes; inspect %s for full per-PID traces.\n' \
          "$emitted" "$strace_dir"
      } >>"$summary_path"
      return
    fi
  done < <(printf '%s\n' "${trace_files[@]}" | sort -t. -k2,2n)

  if ((emitted == 0)); then
    printf 'No execve records found in strace output.\n' >>"$summary_path"
  fi
}

append_trace2_summary() {
  append_summary_header "Git Trace2 Command Totals"

  local trace_files=( "$trace2_perf_dir"/* )
  if ((${#trace_files[@]} == 0)); then
    printf 'No Trace2 perf logs were captured.\n' >>"$summary_path"
    return
  fi

  local trace2_rows
  trace2_rows="$(mktemp)"
  local trace_file=""
  local row=""
  local cmd=""
  local duration=""

  for trace_file in "${trace_files[@]}"; do
    [[ -f "$trace_file" ]] || continue
    row="$(awk -F'\\|' '
      /\| start[[:space:]]+\|/ && cmd == "" { cmd = $NF }
      /\| atexit[[:space:]]+\|/ { dur = $6 }
      /\| exit[[:space:]]+\|/ && dur == "" { dur = $6 }
      END {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", cmd)
        gsub(/^[[:space:]]+|[[:space:]]+$/, "", dur)
        if (cmd != "") {
          printf "%s\t%s\t%s\n", FILENAME, dur, cmd
        }
      }
    ' "$trace_file")"
    if [[ -n "$row" ]]; then
      printf '%s\n' "$row" >>"$trace2_rows"
    fi
  done

  if [[ ! -s "$trace2_rows" ]]; then
    printf 'Trace2 perf logs were present, but no command start rows were parsed.\n' >>"$summary_path"
    rm -f "$trace2_rows"
    return
  fi

  {
    printf 'Raw Trace2 perf directory: %s\n' "$trace2_perf_dir"
    awk -F'\t' '
      {
        duration = $2 + 0
        command = $3
        count[command] += 1
        total[command] += duration
        if (duration > max[command]) {
          max[command] = duration
        }
      }
      END {
        for (command in count) {
          printf "%.6f\t%d\t%.6f\t%s\n", total[command], count[command], max[command], command
        }
      }
    ' "$trace2_rows" | sort -nr | awk -F'\t' '
      BEGIN { emitted = 0 }
      {
        printf "count=%d total=%.3fs max=%.3fs %s\n", $2, $1, $3, $4
        emitted += 1
        if (emitted >= 20) {
          exit
        }
      }
    '
  } >>"$summary_path"

  rm -f "$trace2_rows"
}

append_callgrind_summary() {
  append_summary_header "Callgrind Hotspots"

  local callgrind_files=( "$callgrind_dir"/callgrind.out.* )
  if ((${#callgrind_files[@]} == 0)); then
    printf 'No callgrind outputs were captured.\n' >>"$summary_path"
    return
  fi

  local callgrind_file=""
  local pid=""
  local annotation_file=""
  local hotspot=""
  for callgrind_file in "${callgrind_files[@]}"; do
    [[ -f "$callgrind_file" ]] || continue
    pid="${callgrind_file##*.}"
    annotation_file="${callgrind_annotation_dir}/$(basename "$callgrind_file").annotate.txt"
    if callgrind_annotate --auto=yes --inclusive=yes --threshold=95 "$callgrind_file" >"$annotation_file" 2>&1; then
      hotspot="$(awk '
        /^Ir[[:space:]]+file:function$/ { in_hotspots = 1; next }
        in_hotspots && $1 ~ /^[0-9,]+$/ { print; exit }
      ' "$annotation_file")"
      if [[ -z "$hotspot" ]]; then
        hotspot="(no hotspot line parsed)"
      fi
    else
      hotspot="(callgrind_annotate failed; inspect ${annotation_file})"
    fi
    {
      printf 'PID %s: %s\n' "$pid" "$hotspot"
      printf '  annotation: %s\n' "$annotation_file"
    } >>"$summary_path"
  done
}

write_metadata() {
  {
    printf 'created_utc: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'hostname: %s\n' "$(host_name)"
    printf 'repo_root: %s\n' "$repo_root"
    printf 'repo_path: %s\n' "$repo_path"
    printf 'binary: %s\n' "$binary_path"
    printf 'out_dir: %s\n' "$out_dir"
    printf 'stdout_log: %s\n' "$stdout_log"
    printf 'stderr_log: %s\n' "$stderr_log"
    printf 'strace_dir: %s\n' "$strace_dir"
    printf 'callgrind_dir: %s\n' "$callgrind_dir"
    printf 'trace2_event_dir: %s\n' "$trace2_event_dir"
    printf 'trace2_perf_dir: %s\n' "$trace2_perf_dir"
    printf 'manual_instrumentation: %s\n' "$manual_instrumentation"
    printf 'timeout_seconds: %s\n' "${timeout_secs:-none}"
    printf 'command: %s\n' "$(quote_args "${profile_command[@]}")"
  } >"$metadata_path"
}

finalize_metadata() {
  {
    printf 'root_pid: %s\n' "$root_pid"
    printf 'completed_utc: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'raw_exit_status: %s\n' "$run_status"
    printf 'timed_out: %s\n' "$timed_out"
    printf 'script_exit_status: %s\n' "$script_exit_status"
  } >>"$metadata_path"
}

write_summary() {
  {
    printf 'GitComet process-tree profile bundle\n'
    printf 'Repository: %s\n' "$repo_path"
    printf 'Binary: %s\n' "$binary_path"
    printf 'Output: %s\n' "$out_dir"
    printf 'Root PID: %s\n' "$root_pid"
    printf 'Raw exit status: %s\n' "$run_status"
    printf 'Timed out: %s\n' "$timed_out"
    if ((manual_instrumentation == 1)); then
      printf 'Manual instrumentation: enabled\n'
      printf 'Callgrind control hint: callgrind_control -i on\n'
      printf 'Callgrind control hint: callgrind_control -i off\n'
    fi
  } >"$summary_path"

  append_execve_summary
  append_trace2_summary
  append_callgrind_summary
}

cleanup_on_exit() {
  local trap_status=$?
  if [[ -n "${watchdog_pid:-}" ]]; then
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true
  fi
  if [[ -n "${runner_pid:-}" ]] && kill -0 "$runner_pid" 2>/dev/null; then
    kill -TERM -- "-${runner_pgid:-$runner_pid}" 2>/dev/null || true
    sleep 1
    kill -KILL -- "-${runner_pgid:-$runner_pid}" 2>/dev/null || true
  fi
  return "$trap_status"
}

orig_cwd="$PWD"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

binary_raw=""
out_dir_raw=""
timeout_secs=""
manual_instrumentation=0
dry_run=0
repo_arg=""
extra_gitcomet_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      [[ $# -ge 2 ]] || die "Missing value for --binary"
      binary_raw="$2"
      shift 2
      ;;
    --out-dir)
      [[ $# -ge 2 ]] || die "Missing value for --out-dir"
      out_dir_raw="$2"
      shift 2
      ;;
    --timeout)
      [[ $# -ge 2 ]] || die "Missing value for --timeout"
      timeout_secs="$2"
      shift 2
      ;;
    --manual-instrumentation)
      manual_instrumentation=1
      shift
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    --)
      shift
      extra_gitcomet_args+=("$@")
      break
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      die "Unknown option: $1"
      ;;
    *)
      if [[ -n "$repo_arg" ]]; then
        die "Unexpected extra positional argument: $1"
      fi
      repo_arg="$1"
      shift
      ;;
  esac
done

[[ -n "$repo_arg" ]] || {
  usage >&2
  exit 2
}

if [[ -n "$timeout_secs" ]] && [[ ! "$timeout_secs" =~ ^[0-9]+$ ]]; then
  die "--timeout expects a whole number of seconds"
fi

repo_path="$(resolve_existing_dir "$repo_arg")"
binary_path="${repo_root}/target/release-with-debug/gitcomet"
if [[ -n "$binary_raw" ]]; then
  binary_path="$(resolve_path "$binary_raw")"
fi

timestamp="$(date -u +%Y%m%d-%H%M%SZ)"
out_dir="${repo_root}/tmp/gitcomet-profiles/${timestamp}"
if [[ -n "$out_dir_raw" ]]; then
  out_dir="$(resolve_path "$out_dir_raw")"
fi

stdout_log="${out_dir}/stdout.log"
stderr_log="${out_dir}/stderr.log"
summary_path="${out_dir}/summary.txt"
metadata_path="${out_dir}/metadata.txt"
timed_out_flag="${out_dir}/timed_out.flag"
strace_dir="${out_dir}/strace"
strace_prefix="${strace_dir}/trace"
callgrind_dir="${out_dir}/callgrind"
callgrind_annotation_dir="${callgrind_dir}/annotations"
trace2_event_dir="${out_dir}/trace2-event"
trace2_perf_dir="${out_dir}/trace2-perf"

profile_command=(
  env
  "GIT_TRACE2_EVENT=${trace2_event_dir}"
  "GIT_TRACE2_PERF=${trace2_perf_dir}"
  strace
  -ff
  -ttT
  -o
  "$strace_prefix"
  valgrind
  --tool=callgrind
  --trace-children=yes
  --callgrind-out-file="${callgrind_dir}/callgrind.out.%p"
  --dump-instr=yes
  --collect-jumps=yes
  --sigill-diagnostics=no
  --error-limit=no
)

if ((manual_instrumentation == 1)); then
  profile_command+=(--instr-atstart=no)
fi

profile_command+=("$binary_path" "$repo_path")
if ((${#extra_gitcomet_args[@]} > 0)); then
  profile_command+=("${extra_gitcomet_args[@]}")
fi

if ((dry_run == 1)); then
  {
    printf 'Output directory: %s\n' "$out_dir"
    printf 'Profile command: %s\n' "$(quote_args "${profile_command[@]}")"
  }
  exit 0
fi

[[ ! -e "$out_dir" ]] || die "Output directory already exists: ${out_dir}"

require_tool git
require_tool setsid
require_tool strace
require_tool valgrind
require_tool callgrind_annotate
if ((manual_instrumentation == 1)); then
  require_tool callgrind_control
fi

if [[ ! -x "$binary_path" ]]; then
  die "GitComet binary not found or not executable at ${binary_path}. Build it with: bash scripts/build_release_debug.sh"
fi
verify_strace_usable

mkdir -p \
  "$out_dir" \
  "$strace_dir" \
  "$callgrind_dir" \
  "$callgrind_annotation_dir" \
  "$trace2_event_dir" \
  "$trace2_perf_dir"

runner_pid=""
runner_pgid=""
watchdog_pid=""
root_pid=""
run_status=0
timed_out=0
script_exit_status=0

trap cleanup_on_exit EXIT

write_metadata

setsid "${profile_command[@]}" >"$stdout_log" 2>"$stderr_log" &
runner_pid="$!"
runner_pgid="$runner_pid"
root_pid="$runner_pid"

if [[ -n "$timeout_secs" ]]; then
  (
    sleep "$timeout_secs"
    if kill -0 "$runner_pid" 2>/dev/null; then
      printf 'Timed out after %s seconds\n' "$timeout_secs" >"$timed_out_flag"
      kill -TERM -- "-$runner_pgid" 2>/dev/null || true
      sleep 5
      kill -KILL -- "-$runner_pgid" 2>/dev/null || true
    fi
  ) &
  watchdog_pid="$!"
fi

set +e
wait "$runner_pid"
run_status=$?
set -e

if [[ -n "$watchdog_pid" ]]; then
  kill "$watchdog_pid" 2>/dev/null || true
  wait "$watchdog_pid" 2>/dev/null || true
  watchdog_pid=""
fi

runner_pid=""
runner_pgid=""

if [[ -f "$timed_out_flag" ]]; then
  timed_out=1
fi

script_exit_status="$run_status"
if ((timed_out == 1)); then
  script_exit_status=124
fi

finalize_metadata
write_summary

exit "$script_exit_status"
