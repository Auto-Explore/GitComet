#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/update-aur.sh \
  --aur-dir PATH \
  --version VERSION \
  --binary-tar PATH \
  --source-tar PATH \
  [--verify-source]

Updates PKGBUILD metadata for an AUR package repo checkout, regenerates
.SRCINFO, and optionally verifies the referenced sources with makepkg.
USAGE
}

aur_dir=""
version=""
binary_tar=""
source_tar=""
verify_source="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --aur-dir)
      aur_dir="${2:-}"
      shift 2
      ;;
    --version)
      version="${2:-}"
      shift 2
      ;;
    --binary-tar)
      binary_tar="${2:-}"
      shift 2
      ;;
    --source-tar)
      source_tar="${2:-}"
      shift 2
      ;;
    --verify-source)
      verify_source="true"
      shift
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

if [[ -z "$aur_dir" || -z "$version" || -z "$binary_tar" || -z "$source_tar" ]]; then
  echo "All required arguments must be provided." >&2
  usage
  exit 2
fi

version="${version#v}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]]; then
  echo "Invalid --version '$version'. Expected semver like 1.2.3 or 1.2.3-rc.1." >&2
  exit 2
fi

pkgbuild="${aur_dir}/PKGBUILD"
srcinfo="${aur_dir}/.SRCINFO"
expected_binary_name="gitcomet-v${version}-linux-x86_64.tar.gz"
expected_source_name="gitcomet-source-v${version}.tar.gz"

if [[ ! -f "$pkgbuild" ]]; then
  echo "PKGBUILD not found: $pkgbuild" >&2
  exit 1
fi

if [[ ! -f "$binary_tar" ]]; then
  echo "Binary tarball not found: $binary_tar" >&2
  exit 1
fi

if [[ ! -f "$source_tar" ]]; then
  echo "Source tarball not found: $source_tar" >&2
  exit 1
fi

if [[ "$(basename "$binary_tar")" != "$expected_binary_name" ]]; then
  echo "Binary tarball must be named $expected_binary_name." >&2
  exit 2
fi

if [[ "$(basename "$source_tar")" != "$expected_source_name" ]]; then
  echo "Source tarball must be named $expected_source_name." >&2
  exit 2
fi

sha256_file() {
  local file="$1"
  sha256sum "$file" | awk '{print $1}'
}

binary_sha="$(sha256_file "$binary_tar")"
source_sha="$(sha256_file "$source_tar")"

GITCOMET_PKGVER="$version" \
GITCOMET_BIN_SHA="$binary_sha" \
GITCOMET_SRC_SHA="$source_sha" \
perl -0pi -e '
  my $pkgver = $ENV{GITCOMET_PKGVER};
  my $bin_sha = $ENV{GITCOMET_BIN_SHA};
  my $src_sha = $ENV{GITCOMET_SRC_SHA};

  s/^pkgver=.*/pkgver=$pkgver/m
    or die "Failed to update pkgver\n";
  s/^sha256sums=\([^)]+\)/sprintf("sha256sums=(\x27%s\x27\n            \x27%s\x27)", $bin_sha, $src_sha)/mse
    or die "Failed to update sha256sums\n";
' "$pkgbuild"

pushd "$aur_dir" >/dev/null
makepkg --printsrcinfo > "$srcinfo"

cleanup_binary=""
cleanup_source=""
if [[ "$verify_source" == "true" ]]; then
  staged_binary="$PWD/$expected_binary_name"
  staged_source="$PWD/$expected_source_name"

  if [[ "$binary_tar" != "$staged_binary" ]]; then
    cp "$binary_tar" "$staged_binary"
    cleanup_binary="$staged_binary"
  fi

  if [[ "$source_tar" != "$staged_source" ]]; then
    cp "$source_tar" "$staged_source"
    cleanup_source="$staged_source"
  fi

  cleanup() {
    [[ -n "$cleanup_binary" ]] && rm -f "$cleanup_binary"
    [[ -n "$cleanup_source" ]] && rm -f "$cleanup_source"
  }
  trap cleanup EXIT

  makepkg --verifysource
fi
popd >/dev/null

echo "Updated AUR metadata in $aur_dir"
