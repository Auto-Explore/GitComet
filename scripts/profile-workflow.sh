#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/profile-workflow.sh [options] [-- extra_gitcomet_args...]

Builds and launches GitComet with the ad-hoc workflow profiler enabled, then
leaves you to drive the workflow by hand. Inside the app, secondary-alt-p
starts a capture and pressing it again stops it and writes the report.

Attach the Tracy profiler at any point to see CPU zones, frame marks,
allocations and (with --gpu) GPU frame spans live; Tracy's on-demand mode
means nothing is collected until it connects.

Options:
  --gpu                 Also record per-frame GPU execution time. Requires a
                        gpui-ce checkout carrying the `gpu-timing` change,
                        which this script patches in (see --gpui-ce).
  --gpui-ce PATH        gpui-ce checkout to patch in for --gpu.
                        Default: ../gpui-ce next to this repository.
  --profile NAME        Cargo profile. Default: release-with-debug, which
                        optimises like release but keeps the symbols Tracy
                        needs to name zones and allocation call stacks.
  --out-dir PATH        Where capture reports are written.
                        Default: target/workflow-captures
  --build-only          Build without launching.
  --dry-run             Print the composed cargo command and exit.
  -h, --help            Show this help.

Examples:
  scripts/profile-workflow.sh -- /path/to/some/repo
  scripts/profile-workflow.sh --gpu -- /path/to/some/repo
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
gpui_ce_path="$(dirname "$repo_root")/gpui-ce"
cargo_profile="release-with-debug"
out_dir="$repo_root/target/workflow-captures"
with_gpu=0
build_only=0
dry_run=0
app_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --gpu) with_gpu=1; shift ;;
    --gpui-ce) [[ $# -ge 2 ]] || die "--gpui-ce needs a path"; gpui_ce_path="$2"; shift 2 ;;
    --profile) [[ $# -ge 2 ]] || die "--profile needs a name"; cargo_profile="$2"; shift 2 ;;
    --out-dir) [[ $# -ge 2 ]] || die "--out-dir needs a path"; out_dir="$2"; shift 2 ;;
    --build-only) build_only=1; shift ;;
    --dry-run) dry_run=1; shift ;;
    -h|--help) usage; exit 0 ;;
    --) shift; app_args=("$@"); break ;;
    *) die "Unknown option: $1 (see --help)" ;;
  esac
done

cargo_args=(--profile "$cargo_profile" -p gitcomet)

if [[ $with_gpu -eq 1 ]]; then
  [[ -d "$gpui_ce_path/crates/gpui_wgpu" ]] ||
    die "No gpui-ce checkout at $gpui_ce_path (pass --gpui-ce PATH)"
  grep -q 'gpu-timing' "$gpui_ce_path/crates/gpui_wgpu/Cargo.toml" ||
    die "The gpui-ce checkout at $gpui_ce_path has no gpu-timing feature; check out the branch that carries it."

  gpui_ce_abs=$(cd "$gpui_ce_path" && pwd)
  source_url="https://github.com/Havunen/gpui-ce.git"
  cargo_args+=(--features workflow-profiler-gpu)

  # A [patch] block in the workspace manifest already redirects gpui-ce to
  # disk; adding --config patches on top of it would conflict.
  if grep -q "^\[patch\.\"$source_url\"\]" "$repo_root/Cargo.toml"; then
    echo "Using the [patch] block in Cargo.toml for gpui-ce (ignoring --gpui-ce)."
  else
    for crate in gpui gpui_platform gpui_wgpu; do
      cargo_args+=(--config "patch.\"$source_url\".$crate.path=\"$gpui_ce_abs/crates/$crate\"")
    done
  fi
else
  cargo_args+=(--features workflow-profiler)
fi

command=(cargo)
if [[ $build_only -eq 1 ]]; then
  command+=(build "${cargo_args[@]}")
else
  command+=(run "${cargo_args[@]}")
  if [[ ${#app_args[@]} -gt 0 ]]; then
    command+=(-- "${app_args[@]}")
  fi
fi

if [[ $dry_run -eq 1 ]]; then
  printf 'GITCOMET_WORKFLOW_CAPTURE_DIR=%q\n' "$out_dir"
  printf '%q ' "${command[@]}"
  printf '\n'
  exit 0
fi

mkdir -p "$out_dir"
echo "Capture reports will be written to $out_dir"
GITCOMET_WORKFLOW_CAPTURE_DIR="$out_dir" exec "${command[@]}"
