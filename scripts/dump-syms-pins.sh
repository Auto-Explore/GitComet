#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/cli.sh"

usage() {
  cat <<'EOF'
Usage: scripts/dump-syms-pins.sh <command>

The single source of truth for the pinned mozilla/dump_syms release: version,
asset name, and SHA-256 per target triple. Release and deployment workflows
resolve their pins here instead of duplicating them, so there is nothing to
skew between workflows.

Commands:
  --version            Print the pinned release version (without the leading "v")
  --asset <triple>     Print the asset name for a target triple
  --sha256 <triple>    Print the expected SHA-256 for a target triple
  --triples            Print all pinned target triples, one per line
EOF
}

VERSION="2.3.9"

# asset name, SHA-256 by target triple.
X86_64_WINDOWS="dump_syms-x86_64-pc-windows-msvc.zip"
X86_64_WINDOWS_SHA="bdf48486220708808a3e00aec78856ee1cce096189d47a1e2cb1c635f93bacc7"
X86_64_LINUX="dump_syms-x86_64-unknown-linux-gnu.tar.xz"
X86_64_LINUX_SHA="0fc852a86b00337407d9d423cc388a24c3b489ccaaedcf92623cad57af5ca8ad"
AARCH64_LINUX="dump_syms-aarch64-unknown-linux-gnu.tar.xz"
AARCH64_LINUX_SHA="1b8d6ab436fbfeb9820d4a21413cc77c50c6a4221cff25a830f5d53ba0aaa732"
AARCH64_MACOS="dump_syms-aarch64-apple-darwin.tar.xz"
AARCH64_MACOS_SHA="4945b7e5de0d7ad403822635f0fb912b69990fc412741ff2b7b512e168c40a32"
X86_64_MACOS="dump_syms-x86_64-apple-darwin.tar.xz"
X86_64_MACOS_SHA="e255caf99e403b31613c756babcead8463f6e600c1a9e1e5531fbe27a7e29cf2"

asset_for_triple() {
  case "$1" in
    x86_64-pc-windows-msvc)    echo "${X86_64_WINDOWS}";;
    x86_64-unknown-linux-gnu)  echo "${X86_64_LINUX}";;
    aarch64-unknown-linux-gnu) echo "${AARCH64_LINUX}";;
    aarch64-apple-darwin)      echo "${AARCH64_MACOS}";;
    x86_64-apple-darwin)       echo "${X86_64_MACOS}";;
    *)
      echo "Unknown dump_syms target triple: $1" >&2
      echo "Pinned triples: x86_64-pc-windows-msvc, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, aarch64-apple-darwin, x86_64-apple-darwin" >&2
      exit 2
      ;;
  esac
}

sha256_for_triple() {
  case "$1" in
    x86_64-pc-windows-msvc)    echo "${X86_64_WINDOWS_SHA}";;
    x86_64-unknown-linux-gnu)  echo "${X86_64_LINUX_SHA}";;
    aarch64-unknown-linux-gnu) echo "${AARCH64_LINUX_SHA}";;
    aarch64-apple-darwin)      echo "${AARCH64_MACOS_SHA}";;
    x86_64-apple-darwin)       echo "${X86_64_MACOS_SHA}";;
    *)
      echo "Unknown dump_syms target triple: $1" >&2
      exit 2
      ;;
  esac
}

command="${1:-}"
case "${command}" in
  --version)
    echo "${VERSION}"
    ;;
  --asset)
    require_value "$@"
    asset_for_triple "$2"
    ;;
  --sha256)
    require_value "$@"
    sha256_for_triple "$2"
    ;;
  --triples)
    printf '%s\n' \
      x86_64-pc-windows-msvc \
      x86_64-unknown-linux-gnu \
      aarch64-unknown-linux-gnu \
      aarch64-apple-darwin \
      x86_64-apple-darwin
    ;;
  -h|--help)
    usage
    ;;
  *)
    echo "Unknown command: ${command:-<none>}" >&2
    usage >&2
    exit 2
    ;;
esac
