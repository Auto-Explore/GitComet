#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/microsoft-store-normalize-json.sh --input PATH

Normalizes JSON printed by Microsoft Store CLI v0.3.9. The CLI can decorate output
with ANSI terminal sequences and hard-wrap long JSON strings with physical newlines.
ANSI sequences and carriage returns are removed. Physical newlines inside quoted
strings are removed, while JSON whitespace outside strings and escaped \n sequences
inside strings are preserved.

The normalized JSON is written to stdout.

Requires: awk, sed, tr
USAGE
}

stop_with_diagnostic() {
  local message="$1"
  local exit_code="$2"
  printf 'ERROR (exit %s): %s\n' "$exit_code" "$message" >&2
  exit "$exit_code"
}

input=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --input)
      input="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      stop_with_diagnostic "Unknown argument: $1" 2
      ;;
  esac
done

[[ -n "$input" ]] || stop_with_diagnostic 'Missing --input.' 2
[[ -f "$input" ]] || stop_with_diagnostic "Input file not found: $input" 2

for dependency in awk sed tr; do
  command -v "$dependency" >/dev/null 2>&1 ||
    stop_with_diagnostic "Required command not found: $dependency" 2
done

sed 's/\x1b\[[0-9;?]*[ -\/]*[@-~]//g' "$input" \
  | tr -d '\r' \
  | awk '
      BEGIN {
        in_string = 0
        escaped = 0
      }

      {
        for (position = 1; position <= length($0); position++) {
          character = substr($0, position, 1)

          if (in_string) {
            if (escaped) {
              printf "%s", character
              escaped = 0
            } else if (character == "\\") {
              printf "%s", character
              escaped = 1
            } else if (character == "\"") {
              printf "%s", character
              in_string = 0
            } else if (character !~ /[[:cntrl:]]/) {
              printf "%s", character
            }
          } else {
            printf "%s", character
            if (character == "\"") {
              in_string = 1
              escaped = 0
            }
          }
        }

        # awk removes each physical record separator. Restore it only when it was
        # JSON whitespace outside a string; inside a string it was a CLI hard wrap.
        if (!in_string) {
          printf "\n"
        }
      }
    '
