#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/microsoft-store-listing-metadata.sh \
  --listings-json PATH \
  --release-notes PATH \
  --release-url URL \
  --output-dir PATH

Builds Microsoft Store `updateMetadata` payloads for an unpackaged (MSI) product.

--listings-json is the raw stdout of `msstore submission get <productId> --module listings`,
which contains a `Listings` array. Every listing is copied verbatim into its own
`UpdateMetadataRequest` payload with only `WhatsNew` replaced by the sanitized release
notes, because the Store metadata endpoint is a PUT and drops fields that are omitted.

The release notes are stripped of markup, rewritten into short changelog bullets and
truncated at a line boundary so the result plus a link back to the GitHub release fits
the 1500 character `WhatsNew` limit.

Prints the generated payload paths to stdout, one per line.

Requires: jq, sed
USAGE
}

readonly WHATS_NEW_MAX_LENGTH=1500

stop_with_diagnostic() {
  local message="$1"
  local exit_code="$2"
  printf 'ERROR (exit %s): %s\n' "$exit_code" "$message" >&2
  exit "$exit_code"
}

# Locates the `Listings` array regardless of key casing and regardless of whether the
# CLI printed the bare module response or wrapped it in a response envelope.
readonly LISTINGS_LOOKUP='
  def listings_of:
    [.. | objects | to_entries[]
     | select(.key | ascii_downcase == "listings")
     | .value
     | select(type == "array")]
    | first // [];
'

listings_json=""
release_notes=""
release_url=""
output_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --listings-json)
      listings_json="${2:-}"
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

if [[ -z "$listings_json" || -z "$release_notes" || -z "$release_url" || -z "$output_dir" ]]; then
  usage >&2
  stop_with_diagnostic 'All arguments are required.' 2
fi

for dependency in jq sed; do
  command -v "$dependency" >/dev/null 2>&1 ||
    stop_with_diagnostic "Required command not found: $dependency" 2
done

[[ -f "$listings_json" ]] ||
  stop_with_diagnostic "Listings JSON not found: $listings_json" 2
[[ -f "$release_notes" ]] ||
  stop_with_diagnostic "Release notes not found: $release_notes" 2

if ! [[ "$release_url" =~ ^https?://[^[:space:]]+$ ]]; then
  stop_with_diagnostic "Invalid --release-url '$release_url'. Expected an absolute http(s) URL." 2
fi

if ! jq -e . "$listings_json" >/dev/null 2>&1; then
  stop_with_diagnostic "Listings JSON is not valid JSON: $listings_json" 3
fi

listing_count="$(jq -r "$LISTINGS_LOOKUP"'listings_of | length' "$listings_json")"

if [[ "$listing_count" -eq 0 ]]; then
  stop_with_diagnostic \
    "Draft submission metadata contains no listings. Fetch it with 'msstore submission get <productId> --module listings --language <language>' and make sure the language filter matches a listing language configured in Partner Center." \
    3
fi

# Markdown/HTML that would show up literally in the Store listing, plus the release-note
# bullet shape produced by GitHub ("* Title by @user in <pull request url>").
sanitized_notes="$(
  sed -E \
    -e '/^[[:space:]]*<[^>]*>[[:space:]]*$/d' \
    -e 's|<[^>]*>||g' \
    -e '/^[[:space:]]*\*\*Full Changelog\*\*/d' \
    -e 's|^[[:space:]]*#{1,6}[[:space:]]*||' \
    -e 's|^[[:space:]]*[*-][[:space:]]+(.*) by @[^[:space:]]+ in https?://[^[:space:]]+/pull/([0-9]+).*$|- \1 (#\2)|' \
    -e 's|^[[:space:]]*\*[[:space:]]+|- |' \
    -e 's|\[([^]]*)\]\([^)]*\)|\1|g' \
    -e 's|\*\*([^*]*)\*\*|\1|g' \
    -e 's|`||g' \
    -e 's|[[:space:]]+$||' \
    "$release_notes" | cat -s
)"

whats_new="$(
  printf '%s' "$sanitized_notes" | jq -Rrs \
    --arg release_url "$release_url" \
    --argjson limit "$WHATS_NEW_MAX_LENGTH" \
    '
      def rtrim: sub("[[:space:]]+$"; "");

      ("\n\nFull release notes:\n" + $release_url) as $suffix
      | ($limit - ($suffix | length)) as $budget
      | (rtrim | sub("^[[:space:]]+"; "")) as $body
      | if ($body | length) <= $budget then
          $body + $suffix
        else
          ($budget - 2) as $clipped_budget
          | ($body[:$clipped_budget]) as $head
          | (if ($head | test("\n")) then
               ($head | split("\n") | .[:-1] | join("\n"))
             else
               $head
             end)
          | rtrim
          | . + "\n…" + $suffix
        end
    '
)"

mkdir -p "$output_dir"

index=0
while [[ "$index" -lt "$listing_count" ]]; do
  language="$(
    jq -r "$LISTINGS_LOOKUP"'
      listings_of[$index | tonumber]
      | to_entries
      | map(select(.key | ascii_downcase == "language"))
      | (.[0].value // "unknown")
      | tostring
    ' --arg index "$index" "$listings_json"
  )"

  safe_language="$(printf '%s' "$language" | tr -c '[:alnum:]-' '-' | tr -s '-')"
  payload_path="$output_dir/listing-$(printf '%02d' "$index")-${safe_language:-unknown}.json"

  jq "$LISTINGS_LOOKUP"'
    listings_of[$index | tonumber]
    | with_entries(select(.key | ascii_downcase != "whatsnew"))
    | . + {WhatsNew: $whats_new}
    | {Listings: .}
  ' --arg index "$index" --arg whats_new "$whats_new" "$listings_json" > "$payload_path"

  printf '%s\n' "$payload_path"
  index=$((index + 1))
done
