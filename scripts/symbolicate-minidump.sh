#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/symbolicate-minidump.sh [--symbols-url URL] [--cache DIR] [--json] MINIDUMP [-- EXTRA_ARGS...]

Turns a user-submitted minidump into a readable stack trace by resolving it
against GitComet's Breakpad symbol store. Symbols are fetched lazily over HTTP
and cached, so only the modules a crash actually touches are downloaded, and
only once per build.

Only builds produced after the symbol store landed can be resolved; releases
built before it have no symbols anywhere and never will.

Options:
  --symbols-url  Symbol store base URL (default: https://apt.gitcomet.dev/symbols/)
  --cache        Local symbol cache (default: ${XDG_CACHE_HOME:-$HOME/.cache}/gitcomet-symbols)
  --json         Emit the JSON report instead of the human-readable one

Environment:
  GITCOMET_SYMBOLS_URL
    Overrides the symbol store base URL, for a staging store or a local mirror.
EOF
}

# `shift 2` on a flag whose value is missing fails under `set -e`, killing the
# script before the diagnostics below can run.
require_value() {
  if [[ $# -lt 2 ]]; then
    echo "Option $1 requires a value." >&2
    exit 2
  fi
}

# Published by the release workflow into the same static-website container that
# serves the APT repository and the Windows installer. Moving the store means
# changing this default as well as the SYMBOLS_STORAGE_ACCOUNT variable — see
# docs/crash-symbolication.md.
symbols_url="${GITCOMET_SYMBOLS_URL:-https://apt.gitcomet.dev/symbols/}"
cache_dir="${XDG_CACHE_HOME:-${HOME}/.cache}/gitcomet-symbols"
output_flag="--human"
minidump=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --symbols-url)
      require_value "$@"
      symbols_url="$2"
      shift 2
      ;;
    --cache)
      require_value "$@"
      cache_dir="$2"
      shift 2
      ;;
    --json)
      output_flag="--json"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$minidump" ]]; then
        echo "Only one minidump may be given (got '$minidump' and '$1')." >&2
        exit 2
      fi
      minidump="$1"
      shift
      ;;
  esac
done

if [[ -z "$minidump" ]]; then
  echo "A minidump path is required." >&2
  usage >&2
  exit 2
fi

if [[ ! -f "$minidump" ]]; then
  echo "Minidump not found: $minidump" >&2
  exit 1
fi

if ! command -v minidump-stackwalk >/dev/null 2>&1; then
  echo "minidump-stackwalk is not on PATH." >&2
  echo "Install it with: cargo install minidump-stackwalk --locked" >&2
  exit 1
fi

if [[ -z "$symbols_url" ]]; then
  echo "No symbol store URL. Pass --symbols-url or set GITCOMET_SYMBOLS_URL." >&2
  echo "See docs/crash-symbolication.md." >&2
  exit 2
fi

mkdir -p "$cache_dir"

# minidump-stackwalk is `[OPTIONS] <MINIDUMP> [SYMBOLS_PATHS]...`, so extra
# arguments have to precede the positional: after it they are parsed as local
# symbol paths and silently ignored.
exec minidump-stackwalk \
  "$output_flag" \
  --symbols-url "$symbols_url" \
  --symbols-cache "$cache_dir" \
  "$@" \
  "$minidump"
