#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/build-apt-repo.sh \
  --repo-dir PATH \
  --deb PATH \
  --distribution NAME \
  --component NAME \
  --architecture NAME \
  --origin TEXT \
  --label TEXT \
  --description TEXT \
  --signing-key KEY_ID \
  [--gpg-passphrase PASS] \
  [--repo-url URL]

Builds and signs a simple APT repository tree rooted at PATH.
Repeated --deb arguments are allowed.
USAGE
}

require_tool() {
  local tool="$1"
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Required tool not found: $tool" >&2
    exit 1
  fi
}

repo_dir=""
distribution="stable"
component="main"
architecture="amd64"
origin="GitComet"
label="GitComet"
description="GitComet APT repository"
signing_key=""
gpg_passphrase=""
repo_url=""
declare -a deb_files=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-dir)
      repo_dir="${2:-}"
      shift 2
      ;;
    --deb)
      deb_files+=("${2:-}")
      shift 2
      ;;
    --distribution)
      distribution="${2:-}"
      shift 2
      ;;
    --component)
      component="${2:-}"
      shift 2
      ;;
    --architecture)
      architecture="${2:-}"
      shift 2
      ;;
    --origin)
      origin="${2:-}"
      shift 2
      ;;
    --label)
      label="${2:-}"
      shift 2
      ;;
    --description)
      description="${2:-}"
      shift 2
      ;;
    --signing-key)
      signing_key="${2:-}"
      shift 2
      ;;
    --gpg-passphrase)
      gpg_passphrase="${2:-}"
      shift 2
      ;;
    --repo-url)
      repo_url="${2:-}"
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

if [[ -z "$repo_dir" || -z "$signing_key" || ${#deb_files[@]} -eq 0 ]]; then
  echo "--repo-dir, at least one --deb, and --signing-key are required." >&2
  usage
  exit 2
fi

if ! [[ "$distribution" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  echo "Invalid --distribution '$distribution'." >&2
  exit 2
fi

if ! [[ "$component" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  echo "Invalid --component '$component'." >&2
  exit 2
fi

if ! [[ "$architecture" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  echo "Invalid --architecture '$architecture'." >&2
  exit 2
fi

require_tool dpkg-deb
require_tool dpkg-scanpackages
require_tool gpg
require_tool gzip
require_tool md5sum
require_tool sha256sum
require_tool sha512sum

mkdir -p "$repo_dir"
repo_dir="$(cd "$repo_dir" && pwd)"
repo_url="${repo_url%/}"

for deb in "${deb_files[@]}"; do
  if [[ ! -f "$deb" ]]; then
    echo "Debian package not found: $deb" >&2
    exit 1
  fi
done

for deb in "${deb_files[@]}"; do
  package_name="$(dpkg-deb -f "$deb" Package)"
  package_arch="$(dpkg-deb -f "$deb" Architecture)"

  if [[ "$package_arch" != "$architecture" && "$package_arch" != "all" ]]; then
    echo "Package '$deb' has architecture '$package_arch', expected '$architecture' or 'all'." >&2
    exit 1
  fi

  package_letter="$(printf '%s' "$package_name" | cut -c1 | tr '[:upper:]' '[:lower:]')"
  if [[ -z "$package_letter" || ! "$package_letter" =~ ^[a-z0-9]$ ]]; then
    package_letter="misc"
  fi

  pool_dir="${repo_dir}/pool/${component}/${package_letter}/${package_name}"
  mkdir -p "$pool_dir"
  install -m644 "$deb" "${pool_dir}/$(basename "$deb")"
done

dist_dir="${repo_dir}/dists/${distribution}"
binary_dir="${dist_dir}/${component}/binary-${architecture}"

rm -rf "$dist_dir"
mkdir -p "$binary_dir"

(
  cd "$repo_dir"
  dpkg-scanpackages --multiversion pool /dev/null > "${binary_dir#${repo_dir}/}/Packages"
)

if ! grep -q '^Package:' "${binary_dir}/Packages"; then
  echo "Generated Packages index is empty." >&2
  exit 1
fi

gzip -9 -n -c "${binary_dir}/Packages" > "${binary_dir}/Packages.gz"

release_file="${dist_dir}/Release"

write_release_checksums() {
  local algo_name="$1"
  local sum_cmd="$2"

  echo "${algo_name}:"
  while IFS= read -r rel_path; do
    local full_path="${dist_dir}/${rel_path}"
    local checksum
    local size
    checksum="$($sum_cmd "$full_path" | awk '{print $1}')"
    size="$(wc -c < "$full_path" | tr -d '[:space:]')"
    printf " %s %16s %s\n" "$checksum" "$size" "$rel_path"
  done < <(
    cd "$dist_dir"
    find . -type f \
      ! -name 'InRelease' \
      ! -name 'Release' \
      ! -name 'Release.gpg' \
      -printf '%P\n' \
      | LC_ALL=C sort
  )
  echo ""
}

{
  echo "Origin: ${origin}"
  echo "Label: ${label}"
  echo "Suite: ${distribution}"
  echo "Codename: ${distribution}"
  echo "Date: $(LC_ALL=C date -Ru)"
  echo "Architectures: ${architecture}"
  echo "Components: ${component}"
  echo "Description: ${description}"
  echo ""
  write_release_checksums "MD5Sum" md5sum
  write_release_checksums "SHA256" sha256sum
  write_release_checksums "SHA512" sha512sum
} > "$release_file"

run_gpg() {
  local -a args
  args=(--batch --yes --pinentry-mode loopback --local-user "$signing_key")
  args+=("$@")

  if [[ -n "$gpg_passphrase" ]]; then
    gpg "${args[@]}" --passphrase-fd 0 <<<"$gpg_passphrase"
  else
    gpg "${args[@]}"
  fi
}

run_gpg --armor --detach-sign --output "${dist_dir}/Release.gpg" "$release_file"
run_gpg --clearsign --output "${dist_dir}/InRelease" "$release_file"

gpg --batch --yes --export-options export-minimal --output "${repo_dir}/gitcomet-archive-keyring.gpg" --export "$signing_key"
gpg --batch --yes --armor --export-options export-minimal --output "${repo_dir}/gitcomet-archive-keyring.asc" --export "$signing_key"

if [[ -n "$repo_url" ]]; then
  cat > "${repo_dir}/gitcomet.sources" <<EOF
Types: deb
URIs: ${repo_url}
Suites: ${distribution}
Components: ${component}
Architectures: ${architecture}
Signed-By: /usr/share/keyrings/gitcomet-archive-keyring.gpg
EOF

  cat > "${repo_dir}/gitcomet.list" <<EOF
deb [arch=${architecture} signed-by=/usr/share/keyrings/gitcomet-archive-keyring.gpg] ${repo_url} ${distribution} ${component}
EOF

  cat > "${repo_dir}/README.txt" <<EOF
GitComet APT repository

Install:
  curl -fsSL ${repo_url}/gitcomet-archive-keyring.gpg | sudo tee /usr/share/keyrings/gitcomet-archive-keyring.gpg >/dev/null
  curl -fsSL ${repo_url}/gitcomet.sources | sudo tee /etc/apt/sources.list.d/gitcomet.sources >/dev/null
  sudo apt-get update
  sudo apt-get install gitcomet
EOF
fi

echo "Built APT repository:"
echo "  ${repo_dir}"
echo "  ${binary_dir}/Packages"
echo "  ${dist_dir}/InRelease"
echo "  ${repo_dir}/gitcomet-archive-keyring.gpg"
