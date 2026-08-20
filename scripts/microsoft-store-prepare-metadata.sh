#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/microsoft-store-prepare-metadata.sh \
  --product-id PRODUCT_ID \
  --languages LANGUAGE[,LANGUAGE...] \
  --release-notes PATH \
  --release-url URL \
  --output-dir PATH

Performs the read-only Microsoft Store listing preflight used by the deployment
workflow and generates full listing PUT payloads with an updated WhatsNew field.
It never updates or publishes a Store submission. A redacted JSON summary is
printed to stdout; raw listing responses and generated payloads remain in the
output directory for local inspection. The Microsoft Store CLI must already be
installed and configured before running this command locally.

The command succeeds when at least one requested listing produces a payload.
Missing candidate languages are represented by a "partial" status. It fails when
no payload can be prepared or a fetch/transform command itself fails.

Requires: msstore, jq, awk, tr
USAGE
}

stop_with_diagnostic() {
  local message="$1"
  local exit_code="$2"
  printf 'ERROR (exit %s): %s\n' "$exit_code" "$message" >&2
  exit "$exit_code"
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
product_id=""
languages_csv=""
release_notes=""
release_url=""
output_dir=""

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
    --release-notes)
      release_notes="${2:-}"
      shift 2
      ;;
    --release-url)
      release_url="${2:-}"
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

if [[ -z "$product_id" || -z "$languages_csv" || -z "$release_notes" || -z "$release_url" || -z "$output_dir" ]]; then
  usage >&2
  stop_with_diagnostic 'All arguments are required.' 2
fi
[[ -f "$release_notes" ]] ||
  stop_with_diagnostic "Release notes file not found: $release_notes. Create a real text file first; /path/to/release-notes.txt in the example was a placeholder." 2

command -v msstore >/dev/null 2>&1 ||
  stop_with_diagnostic 'Required command not found: msstore. On Linux/macOS with Homebrew, install it with: brew install microsoft/msstore-cli/msstore-cli' 2

for dependency in jq awk tr; do
  command -v "$dependency" >/dev/null 2>&1 ||
    stop_with_diagnostic "Required command not found: $dependency" 2
done

listings_dir="$output_dir/listings"
metadata_dir="$output_dir/metadata"
listing_files="$output_dir/listing-files.txt"
payload_files="$output_dir/payloads.txt"
mkdir -p "$listings_dir" "$metadata_dir"
: > "$listing_files"
: > "$payload_files"

fetch_failed=false
if ! "$script_dir/microsoft-store-fetch-listings.sh" \
  --product-id "$product_id" \
  --languages "$languages_csv" \
  --output-dir "$listings_dir" \
  > "$listing_files"; then
  fetch_failed=true
fi

transform_failed=false
while IFS= read -r listing_file; do
  [[ -n "$listing_file" ]] || continue
  listing_name="$(basename "$listing_file" .json)"
  generated_files="$metadata_dir/${listing_name}-payloads.txt"

  if ! "$script_dir/microsoft-store-listing-metadata.sh" \
    --listings-json "$listing_file" \
    --release-notes "$release_notes" \
    --release-url "$release_url" \
    --output-dir "$metadata_dir/$listing_name" \
    > "$generated_files"; then
    transform_failed=true
    continue
  fi

  cat "$generated_files" >> "$payload_files"
done < "$listing_files"

requested_languages="$(
  printf '%s' "$languages_csv" \
    | tr ',' '\n' \
    | tr '[:upper:]' '[:lower:]' \
    | awk '{ gsub(/[[:space:]]/, ""); if (length > 0 && !seen[$0]++) print }'
)"
requested_json="$(printf '%s\n' "$requested_languages" | jq -Rsc 'split("\n") | map(select(length > 0))')"
listing_files_json="$(jq -Rsc 'split("\n") | map(select(length > 0))' "$listing_files")"
payload_files_json="$(jq -Rsc 'split("\n") | map(select(length > 0))' "$payload_files")"

matched_languages="$(
  while IFS= read -r payload_file; do
    [[ -n "$payload_file" ]] || continue
    jq -r '.Listings | .Language // .language // empty' "$payload_file"
  done < "$payload_files" \
    | tr '[:upper:]' '[:lower:]' \
    | awk 'NF && !seen[$0]++'
)"
matched_json="$(printf '%s\n' "$matched_languages" | jq -Rsc 'split("\n") | map(select(length > 0))')"

requested_count="$(jq -r 'length' <<< "$requested_json")"
matched_count="$(jq -r 'length' <<< "$matched_json")"
payload_count="$(jq -r 'length' <<< "$payload_files_json")"

if [[ "$payload_count" -eq 0 ]]; then
  status="unavailable"
elif [[ "$matched_count" -lt "$requested_count" || "$fetch_failed" == "true" || "$transform_failed" == "true" ]]; then
  status="partial"
else
  status="ready"
fi

is_success=true
if [[ "$payload_count" -eq 0 || "$fetch_failed" == "true" || "$transform_failed" == "true" ]]; then
  is_success=false
fi

jq -nc \
  --argjson isSuccess "$is_success" \
  --arg status "$status" \
  --argjson requestedLanguages "$requested_json" \
  --argjson matchedLanguages "$matched_json" \
  --argjson listingFiles "$listing_files_json" \
  --argjson payloadFiles "$payload_files_json" \
  --argjson fetchFailed "$fetch_failed" \
  --argjson transformFailed "$transform_failed" \
  '{
    isSuccess: $isSuccess,
    status: $status,
    requestedLanguages: $requestedLanguages,
    matchedLanguages: $matchedLanguages,
    listingFiles: $listingFiles,
    payloadFiles: $payloadFiles,
    errors: [
      if $fetchFailed then "listing fetch command failed" else empty end,
      if $transformFailed then "listing payload generation failed" else empty end,
      if ($payloadFiles | length) == 0 then "no listing metadata payloads generated" else empty end
    ]
  }'

[[ "$is_success" == "true" ]]
