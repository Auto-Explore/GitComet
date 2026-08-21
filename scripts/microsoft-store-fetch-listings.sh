#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/microsoft-store-fetch-listings.sh \
  --product-id PRODUCT_ID \
  --languages LANGUAGE[,LANGUAGE...] \
  --output-dir PATH

Fetches Microsoft Store draft listing metadata one language at a time. Successful
JSON response paths are printed to stdout, one per line. A missing or unavailable
listing is reported as a warning and does not make the command fail, allowing Store
package publishing to continue when optional listing metadata is unavailable.

Requires: msstore, grep, jq, sed, tail, tr
USAGE
}

stop_with_diagnostic() {
  local message="$1"
  local exit_code="$2"
  printf 'ERROR (exit %s): %s\n' "$exit_code" "$message" >&2
  exit "$exit_code"
}

warn() {
  local title="$1"
  local message="$2"

  if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
    printf '::warning title=%s::%s\n' "$title" "$message" >&2
  else
    printf 'WARNING: %s: %s\n' "$title" "$message" >&2
  fi
}

readonly LISTINGS_COUNT='[
  .. | objects | to_entries[]
  | select(.key | ascii_downcase == "listings")
  | .value
  | select(type == "array")
] | first // [] | length'

product_id=""
languages_csv=""
output_dir=""
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --product-id)
      product_id="${2:-}"
      shift 2
      ;;
    --languages)
      languages_csv="${2:-}"
      shift 2
      ;;
    --output-dir)
      output_dir="${2:-}"
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

if [[ -z "$product_id" || -z "$languages_csv" || -z "$output_dir" ]]; then
  usage >&2
  stop_with_diagnostic 'All arguments are required.' 2
fi

for dependency in msstore grep jq sed tail tr; do
  command -v "$dependency" >/dev/null 2>&1 ||
    stop_with_diagnostic "Required command not found: $dependency" 2
done

mkdir -p "$output_dir"

declare -A seen_languages=()
language_tokens="${languages_csv//,/ }"
for language in $language_tokens; do
  language="$(printf '%s' "$language" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')"
  [[ -n "$language" ]] || continue

  if ! [[ "$language" =~ ^[a-z]{2,3}(-[a-z0-9]{2,8})*$ ]]; then
    stop_with_diagnostic "Invalid listing language '$language'." 2
  fi

  if [[ -n "${seen_languages[$language]:-}" ]]; then
    continue
  fi
  seen_languages[$language]=1

  safe_language="$(printf '%s' "$language" | tr -c '[:alnum:]-' '-' | tr -s '-')"
  raw_response_file="$output_dir/listings-${safe_language}.raw"
  response_file="$output_dir/listings-${safe_language}.json"
  error_file="$output_dir/listings-${safe_language}.log"

  if ! NO_COLOR=1 msstore submission get "$product_id" --module listings --language "$language" \
    > "$raw_response_file" 2> "$error_file"; then
    if grep -Eq 'Microsoft\.NETCore\.App.*9\.0\.0|framework_version=9\.0\.0' "$error_file"; then
      error_summary='Microsoft Store CLI requires the .NET 9 runtime (Microsoft.NETCore.App 9.x), but it is not installed or discoverable via DOTNET_ROOT'
    elif grep -Eq '(SellerId|TenantId|ClientId|ClientSecret) is not set' "$error_file"; then
      missing_setting="$(grep -m 1 -oE '(SellerId|TenantId|ClientId|ClientSecret) is not set' "$error_file")"
      error_summary="Microsoft Store CLI credentials are not configured (${missing_setting}); run 'msstore reconfigure' before the read-only preflight"
    else
      error_summary="$({
        sed 's/\x1b\[[0-9;]*[a-zA-Z]//g' "$error_file" |
          sed '/^[[:space:]]*$/d' |
          tail -n 1
      } || true)"
    fi
    warn \
      "Microsoft Store listing unavailable" \
      "Could not fetch listing '$language'${error_summary:+: $error_summary}"
    continue
  fi

  # Preserve the raw CLI response for diagnostics. The normalizer removes terminal
  # formatting and physical hard wraps inside JSON string values.
  "$script_dir/microsoft-store-normalize-json.sh" --input "$raw_response_file" \
    > "$response_file"

  if ! jq -e . "$response_file" >/dev/null 2>&1; then
    warn \
      "Invalid Microsoft Store listing response" \
      "The Store CLI returned invalid JSON for listing '$language' even after terminal formatting was removed. Raw response: '$raw_response_file'."
    continue
  fi

  listing_count="$(jq -r "$LISTINGS_COUNT" "$response_file")"
  if [[ "$listing_count" -eq 0 ]]; then
    warn \
      "Empty Microsoft Store listing response" \
      "The Store CLI returned no draft listing for language '$language'."
    continue
  fi

  printf '%s\n' "$response_file"
done
