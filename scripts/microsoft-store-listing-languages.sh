#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/microsoft-store-listing-languages.sh --response-json PATH

Validates a read-only Microsoft Store listing response and prints its normalized
listing language codes, one per line. Both the language-list response and listing
objects returned by the Store tools are accepted.

Requires: jq
USAGE
}

stop_with_diagnostic() {
  local message="$1"
  local exit_code="$2"
  printf 'ERROR (exit %s): %s\n' "$exit_code" "$message" >&2
  exit "$exit_code"
}

response_json=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --response-json)
      response_json="${2:-}"
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

[[ -n "$response_json" ]] || stop_with_diagnostic 'Missing --response-json.' 2
[[ -f "$response_json" ]] || stop_with_diagnostic "Response file not found: $response_json" 2
command -v jq >/dev/null 2>&1 || stop_with_diagnostic 'Required command not found: jq' 2

if ! jq -e . "$response_json" >/dev/null 2>&1; then
  stop_with_diagnostic 'The Microsoft Store response is not valid JSON.' 3
fi

if ! jq -er '
  def request_failed:
    if has("isSuccess") then .isSuccess == false
    elif has("IsSuccess") then .IsSuccess == false
    else false
    end;

  if request_failed then
    error("Store API reported failure")
  else
    [
      .. | objects | to_entries[]
      | if ((.key | ascii_downcase) == "listinglanguages" and (.value | type) == "array") then
          .value[] | select(type == "string")
        elif ((.key | ascii_downcase) == "language" and (.value | type) == "string") then
          .value
        else
          empty
        end
      | ascii_downcase
    ]
    | unique
    | if length == 0 then error("No listing languages") else . end
    | if any(.[]; test("^[a-z]{2,3}(-[a-z0-9]{2,8})*$") | not) then
        error("Invalid listing language")
      else
        .[]
      end
  end
' "$response_json" 2>/dev/null; then
  stop_with_diagnostic 'The Microsoft Store response reported failure or contained no valid listing languages.' 3
fi
