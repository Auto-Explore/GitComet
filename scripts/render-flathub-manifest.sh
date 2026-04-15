#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/render-flathub-manifest.sh --source-url URL --source-sha256 SHA256 --cargo-vendor-url URL --cargo-vendor-sha256 SHA256 --output PATH [--template PATH]
EOF
}

template="flatpak/dev.gitcomet.GitComet.yaml.in"
source_url=""
source_sha256=""
cargo_vendor_url=""
cargo_vendor_sha256=""
output=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --template) template="$2"; shift 2 ;;
    --source-url) source_url="$2"; shift 2 ;;
    --source-sha256) source_sha256="$2"; shift 2 ;;
    --cargo-vendor-url) cargo_vendor_url="$2"; shift 2 ;;
    --cargo-vendor-sha256) cargo_vendor_sha256="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "$source_url" || -z "$source_sha256" || -z "$cargo_vendor_url" || -z "$cargo_vendor_sha256" || -z "$output" ]]; then
  usage
  exit 2
fi

mkdir -p "$(dirname "$output")"
sed \
  -e "s|@SOURCE_URL@|${source_url}|g" \
  -e "s|@SOURCE_SHA256@|${source_sha256}|g" \
  -e "s|@CARGO_VENDOR_URL@|${cargo_vendor_url}|g" \
  -e "s|@CARGO_VENDOR_SHA256@|${cargo_vendor_sha256}|g" \
  "$template" >"$output"
