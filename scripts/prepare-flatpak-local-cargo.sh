#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
vendor_root="${repo_root}/flatpak/cargo"
vendor_dir="flatpak/cargo/vendor"
tmp_config="$(mktemp)"
trap 'rm -f "${tmp_config}"' EXIT

cd "${repo_root}"
rm -rf "${vendor_root}"
mkdir -p "${vendor_root}"

cargo vendor --locked --versioned-dirs "${vendor_dir}" > "${tmp_config}"
sed 's|directory = "flatpak/cargo/vendor"|directory = "cargo/vendor"|' "${tmp_config}" \
  > "${vendor_root}/config.toml"
