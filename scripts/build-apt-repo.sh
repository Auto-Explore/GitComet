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
Repeated --deb and --architecture arguments are allowed.
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
origin="GitComet"
label="GitComet"
description="GitComet APT repository"
signing_key=""
gpg_passphrase=""
repo_url=""
declare -a deb_files=()
declare -a architectures=()

add_architectures() {
  local raw="${1:-}"
  raw="${raw//,/ }"

  for arch in $raw; do
    arch="$(echo "$arch" | tr -d '[:space:]')"
    if [[ -n "$arch" ]]; then
      architectures+=("$arch")
    fi
  done
}

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
      add_architectures "${2:-}"
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

if [[ ${#architectures[@]} -eq 0 ]]; then
  architectures=("amd64")
fi

declare -A seen_architectures=()
deduped_architectures=()
for architecture in "${architectures[@]}"; do
  if [[ -n "${seen_architectures[$architecture]:-}" ]]; then
    continue
  fi
  seen_architectures["$architecture"]=1
  deduped_architectures+=("$architecture")
done
architectures=("${deduped_architectures[@]}")

if ! [[ "$distribution" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  echo "Invalid --distribution '$distribution'." >&2
  exit 2
fi

if ! [[ "$component" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  echo "Invalid --component '$component'." >&2
  exit 2
fi

for architecture in "${architectures[@]}"; do
  if ! [[ "$architecture" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
    echo "Invalid --architecture '$architecture'." >&2
    exit 2
  fi
done

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
architectures_release="$(printf '%s ' "${architectures[@]}")"
architectures_release="${architectures_release% }"
architectures_csv="$(IFS=,; echo "${architectures[*]}")"

for deb in "${deb_files[@]}"; do
  if [[ ! -f "$deb" ]]; then
    echo "Debian package not found: $deb" >&2
    exit 1
  fi
done

for deb in "${deb_files[@]}"; do
  package_name="$(dpkg-deb -f "$deb" Package)"
  package_arch="$(dpkg-deb -f "$deb" Architecture)"

  if [[ "$package_arch" != "all" ]]; then
    package_arch_supported=0
    for architecture in "${architectures[@]}"; do
      if [[ "$package_arch" == "$architecture" ]]; then
        package_arch_supported=1
        break
      fi
    done

    if [[ "$package_arch_supported" -ne 1 ]]; then
      echo "Package '$deb' has architecture '$package_arch', expected one of '${architectures_csv}' or 'all'." >&2
      exit 1
    fi
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

rm -rf "$dist_dir"
for architecture in "${architectures[@]}"; do
  binary_dir="${dist_dir}/${component}/binary-${architecture}"
  mkdir -p "$binary_dir"

  (
    cd "$repo_dir"
    dpkg-scanpackages --multiversion -a "$architecture" pool /dev/null > "${binary_dir#${repo_dir}/}/Packages"
  )

  if ! grep -q '^Package:' "${binary_dir}/Packages"; then
    echo "Generated Packages index is empty for architecture '$architecture'." >&2
    exit 1
  fi

  gzip -9 -n -c "${binary_dir}/Packages" > "${binary_dir}/Packages.gz"
done

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
}

# `Release` is parsed as a single deb822 paragraph. Blank lines would split
# the checksum stanzas into separate paragraphs and make APT ignore them.
{
  echo "Origin: ${origin}"
  echo "Label: ${label}"
  echo "Suite: ${distribution}"
  echo "Codename: ${distribution}"
  echo "Date: $(LC_ALL=C date -Ru)"
  echo "Architectures: ${architectures_release}"
  echo "Components: ${component}"
  echo "Description: ${description}"
  write_release_checksums "MD5Sum" md5sum
  write_release_checksums "SHA256" sha256sum
  write_release_checksums "SHA512" sha512sum
} > "$release_file"

run_gpg() {
  local -a args
  args=(--batch --yes --pinentry-mode loopback --local-user "$signing_key")

  if [[ -n "$gpg_passphrase" ]]; then
    gpg "${args[@]}" --passphrase-fd 0 "$@" <<<"$gpg_passphrase"
  else
    gpg "${args[@]}" "$@"
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
Architectures: ${architectures_release}
Signed-By: /usr/share/keyrings/gitcomet-archive-keyring.gpg
EOF

  cat > "${repo_dir}/gitcomet.list" <<EOF
deb [arch=${architectures_csv} signed-by=/usr/share/keyrings/gitcomet-archive-keyring.gpg] ${repo_url} ${distribution} ${component}
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
for architecture in "${architectures[@]}"; do
  echo "  ${dist_dir}/${component}/binary-${architecture}/Packages"
done
echo "  ${dist_dir}/InRelease"
echo "  ${repo_dir}/gitcomet-archive-keyring.gpg"
