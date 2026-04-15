#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package-flatpak-cargo-vendor.sh --output PATH
EOF
}

output=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "$output" ]]; then
  usage
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stage_dir="$(mktemp -d)"
trap 'rm -rf "${stage_dir}"' EXIT

bash "${repo_root}/scripts/prepare-flatpak-local-cargo.sh"

mkdir -p "$(dirname "$output")"
mkdir -p "${stage_dir}/cargo"
cp -a "${repo_root}/flatpak/cargo/." "${stage_dir}/cargo/"
tar -C "${stage_dir}" -czf "${output}" cargo
