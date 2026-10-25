#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/cli.sh
source "$script_dir/lib/cli.sh"
template="$script_dir/../packaging/linux/PKGBUILD.in"

usage() {
  cat <<'USAGE'
Usage: scripts/update-aur.sh \
  --aur-dir PATH \
  --version VERSION \
  --binary-tar PATH \
  --aarch64-binary-tar PATH \
  --source-tar PATH \
  [--verify-source]

Generates gitcomet-bin's PKGBUILD from packaging/linux/PKGBUILD.in and
regenerates .SRCINFO. --binary-tar is the x86_64 release; --aarch64-binary-tar
is the linux-arm64 release. --verify-source checks both architectures with
makepkg. Run as a non-root user with Arch packaging tools installed.
USAGE
}

fail() {
  echo "$*" >&2
  exit 2
}

aur_dir="" version="" binary_tar="" aarch64_binary_tar="" source_tar=""
verify_source=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --aur-dir) require_value "$@"; aur_dir="$2"; shift 2 ;;
    --version) require_value "$@"; version="$2"; shift 2 ;;
    --binary-tar) require_value "$@"; binary_tar="$2"; shift 2 ;;
    --aarch64-binary-tar) require_value "$@"; aarch64_binary_tar="$2"; shift 2 ;;
    --source-tar) require_value "$@"; source_tar="$2"; shift 2 ;;
    --verify-source) verify_source=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$aur_dir" && -n "$version" && -n "$binary_tar" && -n "$aarch64_binary_tar" && -n "$source_tar" ]] ||
  fail 'All required arguments must be provided. See --help.'
version="${version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] ||
  fail "Invalid --version '$version'. Expected semver like 1.2.3 or 1.2.3-rc.1."
[[ -d "$aur_dir" ]] || fail "AUR directory not found: $aur_dir"
command -v makepkg >/dev/null 2>&1 || fail 'Required tool not found: makepkg'

archives=("$binary_tar" "$aarch64_binary_tar" "$source_tar")
names=("gitcomet-v${version}-linux-x86_64.tar.gz" "gitcomet-v${version}-linux-arm64.tar.gz" "gitcomet-source-v${version}.tar.gz")
checksums=()
for i in "${!archives[@]}"; do
  [[ -f "${archives[i]}" ]] || fail "Archive not found: ${archives[i]}"
  [[ "$(basename "${archives[i]}")" == "${names[i]}" ]] || fail "Archive must be named ${names[i]}."
  archives[i]="$(realpath "${archives[i]}")"
  checksums[i]="$(sha256sum "${archives[i]}" | awk '{print $1}')"
done
aur_dir="$(realpath "$aur_dir")"
pkgbuild="$aur_dir/PKGBUILD"
srcinfo="$aur_dir/.SRCINFO"
# Arch forbids hyphens in pkgver; retain the upstream version separately for URLs.
pkgver="${version/-rc./rc.}"
pkgrel=1
previous_version=""
if [[ -f "$srcinfo" ]]; then
  previous_version="$(awk '$1 == "pkgver" && $2 == "=" {print $3; exit}' "$srcinfo")"
  if [[ "$previous_version" == "$pkgver" ]]; then
    pkgrel="$(awk '$1 == "pkgrel" && $2 == "=" {print $3; exit}' "$srcinfo")"
    [[ "$pkgrel" =~ ^[1-9][0-9]*$ ]] || fail "Invalid existing pkgrel: $pkgrel"
  fi
fi

work="$(mktemp -d)"
staged=()
cleanup() {
  rm -rf -- "$work"
  if [[ ${#staged[@]} -gt 0 ]]; then
    rm -f -- "${staged[@]}"
  fi
}
trap cleanup EXIT

render() {
  sed \
    -e "s/@VERSION@/$version/g" \
    -e "s/@PKGVER@/$pkgver/g" \
    -e "s/@PKGREL@/$pkgrel/g" \
    -e "s/@X86_64_SHA256@/${checksums[0]}/g" \
    -e "s/@AARCH64_SHA256@/${checksums[1]}/g" \
    -e "s/@SOURCE_SHA256@/${checksums[2]}/g" \
    "$template" > "$work/PKGBUILD"
}
render
# A packaging change for the same release must upgrade the installed package.
# Identical reruns keep pkgrel stable, and new upstream versions reset it to 1.
if [[ "$previous_version" == "$pkgver" && -f "$pkgbuild" ]] && ! cmp -s "$work/PKGBUILD" "$pkgbuild"; then
  pkgrel=$((pkgrel + 1))
  render
fi
cp "$work/PKGBUILD" "$pkgbuild"

cd "$aur_dir"
makepkg --printsrcinfo > "$work/.SRCINFO"
cp "$work/.SRCINFO" "$srcinfo"

if [[ "$verify_source" == true ]]; then
  for i in "${!archives[@]}"; do
    target="$aur_dir/${names[i]}"
    if [[ "${archives[i]}" != "$target" ]]; then
      [[ ! -e "$target" ]] || fail "Refusing to overwrite an existing source archive: $target"
      cp "${archives[i]}" "$target"
      staged+=("$target")
    fi
  done
  for arch in x86_64 aarch64; do
    # Verification needs no cross toolchain: only CARCH selects the source list.
    printf 'source /etc/makepkg.conf\nCARCH=%s\n' "$arch" > "$work/makepkg.conf"
    makepkg --config "$work/makepkg.conf" --verifysource
  done
fi

echo "Updated gitcomet-bin metadata in $aur_dir"
