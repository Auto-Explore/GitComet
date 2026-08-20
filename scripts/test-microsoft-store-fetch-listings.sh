#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
temp_dir="$(mktemp -d)"
trap 'rm -rf -- "$temp_dir"' EXIT

fake_bin="$temp_dir/bin"
mkdir -p "$fake_bin"

cat > "$fake_bin/msstore" <<'FAKE_MSSTORE'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "submission" || "${2:-}" != "get" ]]; then
  printf 'Unexpected mutating or unsupported msstore command: %s\n' "$*" >&2
  exit 90
fi

language=""
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "--language" ]]; then
    language="${2:-}"
    break
  fi
  shift
done

printf '%s\n' "$language" >> "$FAKE_MSSTORE_CALL_LOG"

case "$FAKE_MSSTORE_SCENARIO" in
  en-success)
    if [[ "$language" != "en" ]]; then
      printf 'nodatafound - No Listing Data Found for Language: %s.\n' "$language" >&2
      exit 255
    fi
    ;;
  en-fallback)
    if [[ "$language" == "en-us" ]]; then
      printf 'nodatafound - No Listing Data Found for Language: en-us.\n' >&2
      exit 255
    fi
    ;;
  all-fail)
    printf 'nodatafound - No Listing Data Found for Language: %s.\n' "$language" >&2
    exit 255
    ;;
  malformed)
    printf '{not-json\n'
    exit 0
    ;;
  empty)
    printf '{"isSuccess":true,"errors":[],"Listings":[]}\n'
    exit 0
    ;;
  missing-dotnet)
    printf "Framework: 'Microsoft.NETCore.App', version '9.0.0'\n" >&2
    printf 'https://aka.ms/dotnet-core-applaunch?framework=Microsoft.NETCore.App&framework_version=9.0.0\n' >&2
    exit 150
    ;;
  missing-config)
    printf '01:57:50 crit: MSStore.CLI.Program[0] SellerId is not set.\n' >&2
    exit 255
    ;;
  ansi-json)
    printf '{"isSuccess":true,"errors":[],"Listings":[{"Language":"\033[38;5;2men\033[0m","Description":"Synthetic listing"}]}\n'
    exit 0
    ;;
  wrapped-json)
    printf '{"isSuccess":true,"errors":[],"Listings":[{"Language":"en","Description":"GitComet stays \nresponsive after a hard wrap and preserves intentional\\nparagraph breaks."}]}\n'
    exit 0
    ;;
  *)
    printf 'Unknown fake scenario: %s\n' "$FAKE_MSSTORE_SCENARIO" >&2
    exit 91
    ;;
esac

printf '{"isSuccess":true,"errors":[],"Listings":[{"Language":"%s","Description":"Synthetic listing"}]}\n' "$language"
FAKE_MSSTORE
chmod +x "$fake_bin/msstore"

run_fetch() {
  local scenario="$1"
  local languages="$2"
  local case_dir="$temp_dir/$scenario"

  mkdir -p "$case_dir"
  : > "$case_dir/calls.txt"
  PATH="$fake_bin:$PATH" \
    FAKE_MSSTORE_SCENARIO="$scenario" \
    FAKE_MSSTORE_CALL_LOG="$case_dir/calls.txt" \
    GITHUB_ACTIONS=true \
    "$repo_dir/scripts/microsoft-store-fetch-listings.sh" \
      --product-id synthetic-product \
      --languages "$languages" \
      --output-dir "$case_dir/responses" \
      > "$case_dir/paths.txt" \
      2> "$case_dir/warnings.txt"
}

run_fetch en-success en
test "$(wc -l < "$temp_dir/en-success/paths.txt")" -eq 1
test "$(jq -r '.Listings[0].Language' "$(head -n 1 "$temp_dir/en-success/paths.txt")")" = "en"
test "$(cat "$temp_dir/en-success/calls.txt")" = "en"

run_fetch en-fallback en-us,en,en
test "$(wc -l < "$temp_dir/en-fallback/paths.txt")" -eq 1
test "$(jq -r '.Listings[0].Language' "$(head -n 1 "$temp_dir/en-fallback/paths.txt")")" = "en"
test "$(paste -sd, "$temp_dir/en-fallback/calls.txt")" = "en-us,en"
grep -Fq 'No Listing Data Found for Language: en-us' "$temp_dir/en-fallback/warnings.txt"

run_fetch all-fail en-us,en
test ! -s "$temp_dir/all-fail/paths.txt"
test "$(wc -l < "$temp_dir/all-fail/calls.txt")" -eq 2
grep -Fq 'Microsoft Store listing unavailable' "$temp_dir/all-fail/warnings.txt"

run_fetch malformed en
test ! -s "$temp_dir/malformed/paths.txt"
grep -Fq 'invalid JSON' "$temp_dir/malformed/warnings.txt"

run_fetch empty en
test ! -s "$temp_dir/empty/paths.txt"
grep -Fq 'no draft listing' "$temp_dir/empty/warnings.txt"

run_fetch missing-dotnet en
test ! -s "$temp_dir/missing-dotnet/paths.txt"
grep -Fq 'requires the .NET 9 runtime' "$temp_dir/missing-dotnet/warnings.txt"

run_fetch missing-config en
test ! -s "$temp_dir/missing-config/paths.txt"
grep -Fq 'credentials are not configured (SellerId is not set)' "$temp_dir/missing-config/warnings.txt"

run_fetch ansi-json en
test "$(wc -l < "$temp_dir/ansi-json/paths.txt")" -eq 1
test "$(jq -r '.Listings[0].Language' "$(head -n 1 "$temp_dir/ansi-json/paths.txt")")" = "en"
grep -q $'\033' "$temp_dir/ansi-json/responses/listings-en.raw"
if grep -q $'\033' "$temp_dir/ansi-json/responses/listings-en.json"; then
  echo 'Expected ANSI control sequences to be removed from the normalized JSON.' >&2
  exit 1
fi

run_fetch wrapped-json en
test "$(wc -l < "$temp_dir/wrapped-json/paths.txt")" -eq 1
test "$(jq -r '.Listings[0].Description' "$(head -n 1 "$temp_dir/wrapped-json/paths.txt")")" = $'GitComet stays responsive after a hard wrap and preserves intentional\nparagraph breaks.'

printf '%s\n' 'Synthetic release notes' > "$temp_dir/release-notes.txt"
prepare_dir="$temp_dir/prepare-fallback"
mkdir -p "$prepare_dir"
: > "$prepare_dir/calls.txt"
PATH="$fake_bin:$PATH" \
  FAKE_MSSTORE_SCENARIO=en-fallback \
  FAKE_MSSTORE_CALL_LOG="$prepare_dir/calls.txt" \
  GITHUB_ACTIONS=true \
  "$repo_dir/scripts/microsoft-store-prepare-metadata.sh" \
    --product-id synthetic-product \
    --languages en-us,en \
    --release-notes "$temp_dir/release-notes.txt" \
    --release-url https://example.invalid/releases/v0.0.0-test \
    --output-dir "$prepare_dir/output" \
    > "$prepare_dir/summary.json" \
    2> "$prepare_dir/warnings.txt"

jq -e '
  .isSuccess == true
  and .status == "partial"
  and .requestedLanguages == ["en-us", "en"]
  and .matchedLanguages == ["en"]
  and (.payloadFiles | length) == 1
' "$prepare_dir/summary.json" >/dev/null
payload_file="$(jq -r '.payloadFiles[0]' "$prepare_dir/summary.json")"
test "$(jq -r '.Listings.Language' "$payload_file")" = "en"
test "$(jq -r '.Listings.WhatsNew' "$payload_file")" = $'Synthetic release notes\n\nFull release notes:\nhttps://example.invalid/releases/v0.0.0-test'

prepare_fail_dir="$temp_dir/prepare-all-fail"
mkdir -p "$prepare_fail_dir"
: > "$prepare_fail_dir/calls.txt"
if PATH="$fake_bin:$PATH" \
  FAKE_MSSTORE_SCENARIO=all-fail \
  FAKE_MSSTORE_CALL_LOG="$prepare_fail_dir/calls.txt" \
  GITHUB_ACTIONS=true \
  "$repo_dir/scripts/microsoft-store-prepare-metadata.sh" \
    --product-id synthetic-product \
    --languages en-us,en \
    --release-notes "$temp_dir/release-notes.txt" \
    --release-url https://example.invalid/releases/v0.0.0-test \
    --output-dir "$prepare_fail_dir/output" \
    > "$prepare_fail_dir/summary.json" \
    2> "$prepare_fail_dir/warnings.txt"; then
  echo 'Expected metadata preparation to fail when all listing lookups fail.' >&2
  exit 1
fi
jq -e '.isSuccess == false and .status == "unavailable" and (.payloadFiles | length) == 0' \
  "$prepare_fail_dir/summary.json" >/dev/null

printf '%s\n' 'Microsoft Store listing fetch tests passed.'
