#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/macos-cargo-config.sh <arch> [release|debug]

Prints extra Cargo CLI args for GitComet macOS builds, one argument per line.

Environment:
  GITCOMET_MACOS_X86_RELEASE_LTO
    Release LTO override for Intel macOS builds.
    Supported: thin, fat, false, off, inherit
    Default: thin
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage >&2
  exit 2
fi

arch="$1"
mode="${2:-release}"

case "$arch" in
  arm64|aarch64)
    arch="arm64"
    ;;
  x86_64|amd64)
    arch="x86_64"
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    echo "Unsupported macOS arch: $arch" >&2
    exit 2
    ;;
esac

case "$mode" in
  release|debug) ;;
  *)
    echo "Unsupported build mode: $mode" >&2
    exit 2
    ;;
esac

if [[ "$mode" != "release" || "$arch" != "x86_64" ]]; then
  exit 0
fi

lto_mode="${GITCOMET_MACOS_X86_RELEASE_LTO:-thin}"

case "$lto_mode" in
  ""|inherit)
    ;;
  thin|fat)
    printf '%s\n' --config "profile.release.lto=\"${lto_mode}\""
    ;;
  false|off)
    printf '%s\n' --config "profile.release.lto=false"
    ;;
  *)
    echo "Unsupported GITCOMET_MACOS_X86_RELEASE_LTO value: $lto_mode" >&2
    exit 2
    ;;
esac
