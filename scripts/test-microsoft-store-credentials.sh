#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/test-microsoft-store-credentials.sh \
  --tenant-id TENANT_ID \
  --client-id CLIENT_ID \
  --seller-id SELLER_ID \
  --product-id PRODUCT_ID

Validates Microsoft Store credentials by requesting an Entra ID token and then
performing a read-only request for the Partner Center product's listing languages.
The safe listing language codes are printed on success. The client secret is read
from a hidden prompt, never from a command-line option.

Requires: curl, jq, paste
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

stop_with_diagnostic() {
  local message="$1"
  local exit_code="$2"
  printf 'ERROR (exit %s): %s\n' "$exit_code" "$message" >&2
  exit "$exit_code"
}

trim_whitespace() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

get_response_header() {
  local headers_file="$1"
  local header_name
  local value

  for header_name in X-Correlation-ID client-request-id x-ms-request-id; do
    value="$({
      awk -v wanted="$header_name" '
        {
          line = $0
          sub(/\r$/, "", line)
          colon = index(line, ":")
          if (colon > 0 && tolower(substr(line, 1, colon - 1)) == tolower(wanted)) {
            value = substr(line, colon + 1)
            sub(/^[[:space:]]+/, "", value)
          }
        }
        END {
          if (value != "") {
            print value
          }
        }
      ' "$headers_file"
    } || true)"
    if [[ -n "$value" ]]; then
      printf '%s' "$value"
      return 0
    fi
  done

  return 1
}

get_safe_oauth_error() {
  local response_body_file="$1"
  local client_secret="$2"
  local details

  if ! [[ -s "$response_body_file" ]] ||
    ! grep -q '[^[:space:]]' "$response_body_file"; then
    printf '%s' 'Microsoft Entra ID returned no error details.'
    return
  fi

  if ! details="$(
    jq -er '
      if type == "object" then
        [.error?, .error_description?]
        | map(select(type == "string"))
        | select(length > 0)
        | join(": ")
      else
        empty
      end
    ' "$response_body_file" 2>/dev/null
  )"; then
    printf '%s' 'Microsoft Entra ID returned an unrecognized error response.'
    return
  fi

  if [[ -n "$client_secret" ]]; then
    details="${details//"$client_secret"/[REDACTED]}"
  fi
  printf '%s' "$details"
}

tenant_id=""
client_id=""
seller_id=""
product_id=""
client_secret=""
access_token=""
temp_dir=""
token_body=""
token_error=""
store_headers=""
store_body=""
store_error=""

cleanup() {
  client_secret=""
  access_token=""

  if [[ -n "$temp_dir" && -d "$temp_dir" ]]; then
    rm -f -- "$token_body" "$token_error" "$store_headers" "$store_body" "$store_error"
    rmdir -- "$temp_dir" 2>/dev/null || true
  fi
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tenant-id)
      [[ $# -ge 2 && "$2" != --* ]] ||
        stop_with_diagnostic 'Missing value for --tenant-id.' 1
      tenant_id="$2"
      shift 2
      ;;
    --client-id)
      [[ $# -ge 2 && "$2" != --* ]] ||
        stop_with_diagnostic 'Missing value for --client-id.' 1
      client_id="$2"
      shift 2
      ;;
    --seller-id)
      [[ $# -ge 2 && "$2" != --* ]] ||
        stop_with_diagnostic 'Missing value for --seller-id.' 1
      seller_id="$2"
      shift 2
      ;;
    --product-id)
      [[ $# -ge 2 && "$2" != --* ]] ||
        stop_with_diagnostic 'Missing value for --product-id.' 1
      product_id="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      stop_with_diagnostic "Unknown argument: $1" 1
      ;;
  esac
done

for dependency in curl jq awk grep mktemp paste; do
  command -v "$dependency" >/dev/null 2>&1 ||
    stop_with_diagnostic "Required command not found: $dependency" 1
done

tenant_id="$(trim_whitespace "$tenant_id")"
client_id="$(trim_whitespace "$client_id")"
seller_id="$(trim_whitespace "$seller_id")"
product_id="$(trim_whitespace "$product_id")"

guid_pattern='^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$'
empty_guid='00000000-0000-0000-0000-000000000000'

if ! [[ "$tenant_id" =~ $guid_pattern ]] || [[ "$tenant_id" == "$empty_guid" ]]; then
  stop_with_diagnostic 'TenantId must be a non-empty GUID copied from the Partner Center Microsoft Entra application.' 1
fi

if ! [[ "$client_id" =~ $guid_pattern ]] || [[ "$client_id" == "$empty_guid" ]]; then
  stop_with_diagnostic 'ClientId must be a non-empty GUID copied from the same Partner Center Microsoft Entra application.' 1
fi

if ! [[ "$seller_id" =~ ^[0-9]+$ ]]; then
  stop_with_diagnostic 'SellerId must contain only digits and must be copied from Partner Center account settings.' 1
fi

if ! [[ "$product_id" =~ ^[A-Za-z0-9][A-Za-z0-9.-]{0,127}$ ]]; then
  stop_with_diagnostic 'ProductId must be a non-empty Partner Center product ID containing only letters, digits, periods, or hyphens.' 1
fi

printf 'MICROSOFT_STORE_CLIENT_SECRET: ' >&2
IFS= read -r -s client_secret || true
printf '\n' >&2
if [[ -z "$client_secret" ]]; then
  stop_with_diagnostic 'Client secret cannot be empty.' 1
fi

temp_dir="$(mktemp -d)"
chmod 700 "$temp_dir"
token_body="$temp_dir/token-body.json"
token_error="$temp_dir/token-error.txt"
store_headers="$temp_dir/store-headers.txt"
store_body="$temp_dir/store-body.json"
store_error="$temp_dir/store-error.txt"

printf '%s\n' 'Requesting a Microsoft Store API token from Microsoft Entra ID...'

token_uri="https://login.microsoftonline.com/$tenant_id/oauth2/v2.0/token"
if ! token_status="$({
  printf '%s' "$client_secret" |
    curl \
      --silent \
      --show-error \
      --max-time 60 \
      --request POST \
      --data-urlencode 'grant_type=client_credentials' \
      --data-urlencode "client_id=$client_id" \
      --data-urlencode 'client_secret@-' \
      --data-urlencode 'scope=https://api.store.microsoft.com/.default' \
      --output "$token_body" \
      --write-out '%{http_code}' \
      "$token_uri" 2>"$token_error"
})"; then
  curl_details="$(<"$token_error")"
  curl_details="${curl_details//"$client_secret"/[REDACTED]}"
  stop_with_diagnostic "Could not contact Microsoft Entra ID: ${curl_details:-curl failed without error details.}" 2
fi

if [[ ! "$token_status" =~ ^2[0-9][0-9]$ ]]; then
  oauth_details="$(get_safe_oauth_error "$token_body" "$client_secret")"
  stop_with_diagnostic "Token request failed with HTTP $token_status. Check the tenant ID, client ID, client secret value, and secret expiration. $oauth_details" 2
fi

if ! jq -e . "$token_body" >/dev/null 2>&1; then
  stop_with_diagnostic 'Microsoft Entra ID returned a successful response that did not contain valid JSON.' 2
fi

if ! access_token="$(jq -er '
  .access_token
  | select(type == "string" and length > 0)
' "$token_body" 2>/dev/null)"; then
  stop_with_diagnostic 'Microsoft Entra ID returned a successful response without an access token.' 2
fi

client_secret=""
printf '%s\n' 'Token issued successfully. Checking read-only access to the Partner Center product...'

store_uri="https://api.store.microsoft.com/submission/v1/product/$product_id/metadata/listings?includelanguagelist=true"
if ! store_status="$({
  curl \
    --silent \
    --show-error \
    --max-time 60 \
    --request GET \
    --header @<(printf 'Authorization: Bearer %s\nX-Seller-Account-Id: %s\n' "$access_token" "$seller_id") \
    --dump-header "$store_headers" \
    --output "$store_body" \
    --write-out '%{http_code}' \
    "$store_uri" 2>"$store_error"
})"; then
  curl_details="$(<"$store_error")"
  stop_with_diagnostic "Could not contact the Microsoft Store submission API: ${curl_details:-curl failed without error details.}" 3
fi

correlation_id="$(get_response_header "$store_headers" || true)"
if [[ ! "$store_status" =~ ^2[0-9][0-9]$ ]]; then
  case "$store_status" in
    401)
      diagnostic='The token was rejected. Verify that the Entra application is associated with this Partner Center account and that SellerId belongs to the same account.'
      ;;
    403)
      diagnostic='The Entra application is authenticated but lacks Partner Center access to this product. Assign an appropriate role or read permission for the product.'
      ;;
    404)
      diagnostic='The product was not found. Verify ProductId and confirm that the Entra application can access that product.'
      ;;
    *)
      diagnostic='The Microsoft Store API rejected the read-only listing-language request.'
      ;;
  esac

  correlation_suffix=""
  if [[ -n "$correlation_id" ]]; then
    correlation_suffix=" Correlation ID: $correlation_id."
  fi
  stop_with_diagnostic "Store access check failed with HTTP $store_status. $diagnostic$correlation_suffix" 3
fi

if ! listing_languages="$({
  "$script_dir/microsoft-store-listing-languages.sh" --response-json "$store_body" | paste -sd, -
})"; then
  stop_with_diagnostic 'Store access succeeded, but the response did not contain usable listing languages. Share the redacted response shape so the parser fixture can be updated.' 3
fi

access_token=""
printf '%s\n' 'Microsoft Store credentials are valid: token issuance, seller authorization, and read-only product access all succeeded.'
printf 'Microsoft Store listing languages: %s\n' "$listing_languages"
if [[ -n "$correlation_id" ]]; then
  printf 'Microsoft correlation ID: %s\n' "$correlation_id"
fi
