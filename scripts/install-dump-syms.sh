#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/cli.sh"

usage() {
  cat <<'EOF'
Usage: scripts/install-dump-syms.sh --version VERSION --asset ASSET --sha256 SHA256 [--bin-dir DIR]

Downloads a pinned mozilla/dump_syms release, verifies its SHA-256 and extracts
the dump_syms binary into BIN_DIR. Prints BIN_DIR on stdout so the caller can
append it to PATH (in CI: >> "$GITHUB_PATH").

Used by the Linux and macOS release jobs. The Windows job installs the .zip
asset inline because it runs its steps under pwsh.

Options:
  --version   Release version without the leading "v" (e.g. 2.3.9)
  --asset     Asset file name (e.g. dump_syms-x86_64-unknown-linux-gnu.tar.xz)
  --sha256    Expected SHA-256 of the asset
  --bin-dir   Destination directory (default: ${RUNNER_TEMP:-/tmp}/dump-syms-bin)
EOF
}

version=""
asset=""
expected_sha=""
bin_dir="${RUNNER_TEMP:-/tmp}/dump-syms-bin"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      require_value "$@"
      version="$2"
      shift 2
      ;;
    --asset)
      require_value "$@"
      asset="$2"
      shift 2
      ;;
    --sha256)
      require_value "$@"
      expected_sha="$2"
      shift 2
      ;;
    --bin-dir)
      require_value "$@"
      bin_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$version" || -z "$asset" || -z "$expected_sha" ]]; then
  echo "--version, --asset and --sha256 are all required." >&2
  usage >&2
  exit 2
fi

# Linux runners ship sha256sum; macOS runners ship shasum.
verify_sha256() {
  local path="$1"
  local expected="$2"
  local actual

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$path" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$path" | awk '{print $1}')"
  else
    echo "Neither sha256sum nor shasum is available to verify $path." >&2
    exit 1
  fi

  if [[ "$actual" != "$expected" ]]; then
    echo "SHA-256 mismatch for $path" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi
}

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

archive="${work_dir}/${asset}"
url="https://github.com/mozilla/dump_syms/releases/download/v${version}/${asset}"

echo "Downloading ${url}" >&2
curl --proto '=https' --tlsv1.2 -sSfL "$url" -o "$archive"
verify_sha256 "$archive" "$expected_sha"

# cargo-dist lays the archive out as dump_syms-<target>/dump_syms.
tar -xJf "$archive" -C "$work_dir" --strip-components=1

if [[ ! -f "${work_dir}/dump_syms" ]]; then
  echo "Extracted archive did not contain a dump_syms binary." >&2
  exit 1
fi

mkdir -p "$bin_dir"
install -m755 "${work_dir}/dump_syms" "${bin_dir}/dump_syms"
"${bin_dir}/dump_syms" --version >&2

cd "$bin_dir" && pwd
