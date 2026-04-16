#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/notarize-macos.sh --version VERSION [--arch arm64|x86_64] [--out-dir PATH] [--timeout DURATION] [--keychain PATH] (--keychain-profile PROFILE | --api-key PATH --key-id ID [--issuer UUID])

Submits an already-packaged macOS DMG to Apple's notary service, staples the
resulting ticket to the staged .app bundle and DMG, and refreshes the macOS
tarball so the archived app bundle also carries the stapled ticket.

Expected inputs:
  - <out-dir>/gitcomet-v<VERSION>-macos-<ARCH>.dmg
  - <out-dir>/stage/gitcomet-v<VERSION>-macos-<ARCH>/GitComet.app

Authentication:
  --keychain-profile PROFILE
    Use credentials previously saved with:
      xcrun notarytool store-credentials PROFILE ...

  --api-key PATH --key-id ID [--issuer UUID]
    Use an App Store Connect API key directly. This matches the CI workflow.

Notes:
  - The macOS tarball is rebuilt after stapling so the bundled .app is up to date.
  - The standalone gitcomet binary at the tarball root is only code-signed.
    Apple's notary service does not accept .tar.gz uploads directly.

Defaults:
  --arch matches the host architecture
  --out-dir ./dist
  --timeout 2h
USAGE
}

version=""
arch=""
out_dir="dist"
wait_timeout="2h"
keychain_profile=""
keychain_path=""
api_key_path=""
api_key_id=""
api_issuer=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      version="${2:-}"
      shift 2
      ;;
    --arch)
      arch="${2:-}"
      shift 2
      ;;
    --out-dir)
      out_dir="${2:-}"
      shift 2
      ;;
    --timeout)
      wait_timeout="${2:-}"
      shift 2
      ;;
    --keychain-profile)
      keychain_profile="${2:-}"
      shift 2
      ;;
    --keychain)
      keychain_path="${2:-}"
      shift 2
      ;;
    --api-key)
      api_key_path="${2:-}"
      shift 2
      ;;
    --key-id)
      api_key_id="${2:-}"
      shift 2
      ;;
    --issuer)
      api_issuer="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$version" ]]; then
  echo "--version is required (e.g. --version 0.2.0)." >&2
  exit 2
fi

host_arch_raw="$(uname -m)"
case "$host_arch_raw" in
  arm64|aarch64) host_arch="arm64" ;;
  x86_64|amd64) host_arch="x86_64" ;;
  *)
    echo "Unsupported machine architecture: $host_arch_raw" >&2
    exit 1
    ;;
esac

if [[ -z "$arch" ]]; then
  arch="$host_arch"
fi

if [[ "$arch" != "arm64" && "$arch" != "x86_64" ]]; then
  echo "Unsupported --arch '$arch'. Expected arm64 or x86_64." >&2
  exit 2
fi

if [[ -n "$keychain_profile" ]]; then
  if [[ -n "$api_key_path" || -n "$api_key_id" || -n "$api_issuer" ]]; then
    echo "Use either --keychain-profile or --api-key/--key-id/--issuer, not both." >&2
    exit 2
  fi
else
  if [[ -z "$api_key_path" || -z "$api_key_id" ]]; then
    echo "Provide either --keychain-profile or --api-key PATH --key-id ID." >&2
    exit 2
  fi
  if [[ -n "$keychain_path" ]]; then
    echo "--keychain only applies with --keychain-profile." >&2
    exit 2
  fi
fi

if [[ "$out_dir" = /* ]]; then
  mkdir -p "$out_dir"
  out_abs="$(cd "$out_dir" && pwd)"
else
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  mkdir -p "${repo_root}/${out_dir}"
  out_abs="$(cd "${repo_root}/${out_dir}" && pwd)"
fi

release_root="gitcomet-v${version}-macos-${arch}"
stage_root="${out_abs}/stage"
release_dir="${stage_root}/${release_root}"
app_path="${release_dir}/GitComet.app"
app_binary="${app_path}/Contents/MacOS/gitcomet"
tarball_path="${out_abs}/${release_root}.tar.gz"
dmg_path="${out_abs}/${release_root}.dmg"
standalone_binary="${release_dir}/gitcomet"

for tool in xcrun codesign spctl tar; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Required tool not found: $tool" >&2
    exit 1
  fi
done

if [[ ! -f "$dmg_path" ]]; then
  echo "DMG not found: $dmg_path" >&2
  exit 1
fi

if [[ ! -d "$app_path" ]]; then
  echo "App bundle not found: $app_path" >&2
  exit 1
fi

if [[ ! -x "$app_binary" ]]; then
  echo "App binary not found or not executable: $app_binary" >&2
  exit 1
fi

notary_args=()
if [[ -n "$keychain_profile" ]]; then
  notary_args+=(--keychain-profile "$keychain_profile")
  if [[ -n "$keychain_path" ]]; then
    notary_args+=(--keychain "$keychain_path")
  fi
else
  if [[ ! -f "$api_key_path" ]]; then
    echo "API key file not found: $api_key_path" >&2
    exit 1
  fi
  notary_args+=(
    --key "$api_key_path"
    --key-id "$api_key_id"
  )
  if [[ -n "$api_issuer" ]]; then
    notary_args+=(--issuer "$api_issuer")
  fi
fi

echo "Submitting $dmg_path for notarization"
submit_log="$(mktemp -t gitcomet-notary-submit.XXXXXX)"
set +e
xcrun notarytool submit "$dmg_path" "${notary_args[@]}" --wait --timeout "$wait_timeout" 2>&1 | tee "$submit_log"
submit_status=${PIPESTATUS[0]}
set -e

if [[ $submit_status -ne 0 ]]; then
  submission_id="$(sed -n 's/^  id: //p' "$submit_log" | tail -n1)"
  if [[ -n "$submission_id" ]]; then
    echo "Submission did not complete successfully; current status for $submission_id:"
    xcrun notarytool info "$submission_id" "${notary_args[@]}" || true
  else
    echo "Submission did not complete successfully and no submission id could be parsed from notarytool output." >&2
  fi
  echo "Apple's notary service can hold uploads for additional analysis. If this stays in progress for many hours, treat it as an Apple-side queue or account issue rather than a local packaging failure." >&2
  rm -f "$submit_log"
  exit "$submit_status"
fi
rm -f "$submit_log"

echo "Stapling notarization tickets"
xcrun stapler staple "$app_path"
xcrun stapler staple "$dmg_path"
xcrun stapler validate "$app_path"
xcrun stapler validate "$dmg_path"

if [[ -f "$tarball_path" ]]; then
  echo "Refreshing $tarball_path with stapled app bundle"
  rm -f "$tarball_path"
  tar -C "$stage_root" -czf "$tarball_path" "$release_root"
fi

echo "Verifying signed macOS artifacts"
codesign --verify --deep --strict --verbose=2 "$app_path"
if [[ -f "$standalone_binary" ]]; then
  codesign --verify --strict --verbose=2 "$standalone_binary"
fi
spctl --assess --type open --context context:primary-signature --verbose=4 "$app_path"
spctl --assess --type execute --verbose=4 "$app_binary"
spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg_path"

echo "Notarized macOS artifacts:"
echo "  $app_path"
echo "  $dmg_path"
if [[ -f "$tarball_path" ]]; then
  echo "  $tarball_path"
fi
