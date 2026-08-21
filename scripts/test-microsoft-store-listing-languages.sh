#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
temp_dir="$(mktemp -d)"
trap 'rm -rf -- "$temp_dir"' EXIT

cat > "$temp_dir/camel-case.json" <<'JSON'
{
  "isSuccess": true,
  "errors": [],
  "listingLanguages": ["en"]
}
JSON
test "$("$repo_dir/scripts/microsoft-store-listing-languages.sh" --response-json "$temp_dir/camel-case.json")" = "en"

cat > "$temp_dir/pascal-case.json" <<'JSON'
{
  "IsSuccess": true,
  "Errors": [],
  "ListingLanguages": ["FI-FI", "en-US", "en-US"]
}
JSON
test "$("$repo_dir/scripts/microsoft-store-listing-languages.sh" --response-json "$temp_dir/pascal-case.json" | paste -sd, -)" = "en-us,fi-fi"

cat > "$temp_dir/listings.json" <<'JSON'
{
  "Listings": [
    {"Language": "en", "Description": "Synthetic listing"},
    {"language": "fi", "description": "Synthetic listing"}
  ]
}
JSON
test "$("$repo_dir/scripts/microsoft-store-listing-languages.sh" --response-json "$temp_dir/listings.json" | paste -sd, -)" = "en,fi"

cat > "$temp_dir/api-failure.json" <<'JSON'
{"isSuccess": false, "errors": [{"code": "synthetic"}], "listingLanguages": ["en"]}
JSON
if "$repo_dir/scripts/microsoft-store-listing-languages.sh" --response-json "$temp_dir/api-failure.json" >/dev/null 2>&1; then
  echo 'Expected an API failure response to be rejected.' >&2
  exit 1
fi

printf '{not-json\n' > "$temp_dir/malformed.json"
if "$repo_dir/scripts/microsoft-store-listing-languages.sh" --response-json "$temp_dir/malformed.json" >/dev/null 2>&1; then
  echo 'Expected malformed JSON to be rejected.' >&2
  exit 1
fi

printf '%s\n' 'Microsoft Store listing language response tests passed.'
