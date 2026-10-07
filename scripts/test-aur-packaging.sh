#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

run_logged() {
  if ! "$@" > "$work/command.log" 2>&1; then
    cat "$work/command.log" >&2
    exit 1
  fi
}

cd "$repo_dir"
for version in 0.0.0 0.0.1-rc.1; do
  fixtures="$work/$version"
  aur_dir="$fixtures/aur"
  mkdir -p "$aur_dir"
  for arch in x86_64 arm64; do
    # Distinct ELF fixtures catch accidentally packaging the other architecture.
    cp /usr/bin/true "$fixtures/binary-$arch"
    printf '%s\n' "$arch" >> "$fixtures/binary-$arch"
    run_logged scripts/package-linux.sh stage --binary "$fixtures/binary-$arch" \
      --source "$repo_dir" --out "$fixtures/payload-$arch"
    # GitHub's RUNNER_TEMP is owned by root in the job container; builder needs
    # its own writable scratch directory when invoking the packaging helper.
    RUNNER_TEMP="$work" scripts/package-linux.sh tarball --payload "$fixtures/payload-$arch" \
      --version "$version" --arch "$arch" --out "$fixtures"
  done
  git archive --format=tar --prefix="GitComet-$version/" HEAD \
    assets/linux LICENSE-AGPL-3.0 NOTICE README.md | gzip > "$fixtures/gitcomet-source-v$version.tar.gz"

  update_args=(
    --aur-dir "$aur_dir" --version "v$version"
    --binary-tar "$fixtures/gitcomet-v$version-linux-x86_64.tar.gz"
    --aarch64-binary-tar "$fixtures/gitcomet-v$version-linux-arm64.tar.gz"
    --source-tar "$fixtures/gitcomet-source-v$version.tar.gz"
  )
  run_logged scripts/update-aur.sh "${update_args[@]}" --verify-source
  pkgver="${version/-rc./rc.}"
  grep -Eq '^[[:space:]]*pkgbase = gitcomet-bin$' "$aur_dir/.SRCINFO"
  grep -Eq "^[[:space:]]*pkgver = $pkgver$" "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*pkgrel = 1$' "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*license = AGPL-3.0-only$' "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*depends = fontconfig$' "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*depends = libglvnd$' "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*optdepends = wayland: ' "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*optdepends = vulkan-icd-loader: ' "$aur_dir/.SRCINFO"
  grep -Fq "gitcomet-v$version-linux-arm64.tar.gz" "$aur_dir/.SRCINFO"
  [[ ! -e "$aur_dir/gitcomet-source-v$version.tar.gz" ]] # verification cleans its staged archives

  # Identical reruns must not produce another update or increase pkgrel.
  cp "$aur_dir/PKGBUILD" "$fixtures/unchanged-PKGBUILD"
  cp "$aur_dir/.SRCINFO" "$fixtures/unchanged-SRCINFO"
  run_logged scripts/update-aur.sh "${update_args[@]}"
  cmp "$aur_dir/PKGBUILD" "$fixtures/unchanged-PKGBUILD"
  cmp "$aur_dir/.SRCINFO" "$fixtures/unchanged-SRCINFO"

  # Existing package metadata is replaced, including previously missing runtime deps.
  sed -i "/  'libglvnd'/d" "$aur_dir/PKGBUILD"
  run_logged scripts/update-aur.sh "${update_args[@]}"
  grep -Eq '^[[:space:]]*pkgrel = 2$' "$aur_dir/.SRCINFO"
  grep -Eq '^[[:space:]]*depends = libglvnd$' "$aur_dir/.SRCINFO"

  # Resolve all declared required and optional package names against Arch's DB.
  mapfile -t dependencies < <(awk '
    $1 == "depends" || $1 == "optdepends" {sub(/:.*/, "", $3); print $3}
  ' "$aur_dir/.SRCINFO")
  run_logged pacman -Sp --print-format '%n' -- "${dependencies[@]}"

  cp "$fixtures/"*.tar.gz "$aur_dir/"
  for arch in x86_64 aarch64; do
    binary_arch="$arch"
    [[ "$arch" != aarch64 ]] || binary_arch=arm64
    archive="$aur_dir/gitcomet-v$version-linux-$binary_arch.tar.gz"
    printf 'source /etc/makepkg.conf\nCARCH=%s\n' "$arch" > "$fixtures/makepkg.conf"

    # Corrupt each architecture's archive and require makepkg to reject it.
    printf 'corrupted' >> "$archive"
    if (cd "$aur_dir" && makepkg --config "$fixtures/makepkg.conf" --verifysource) > "$work/corrupt.log" 2>&1; then
      echo "makepkg accepted a corrupt $arch archive." >&2
      exit 1
    fi
    grep -q 'FAILED' "$work/corrupt.log"
    cp "$fixtures/gitcomet-v$version-linux-$binary_arch.tar.gz" "$archive"

    (
      cd "$aur_dir"
      run_logged makepkg --config "$fixtures/makepkg.conf" --nodeps --clean --noconfirm
      package_file="$(makepkg --config "$fixtures/makepkg.conf" --packagelist)"
      installed="$fixtures/installed-$arch"
      mkdir -p "$installed"
      tar -xf "$package_file" -C "$installed"
      grep -Fxq "pkgname = gitcomet-bin" "$installed/.PKGINFO"
      grep -Fxq "pkgver = $pkgver-2" "$installed/.PKGINFO"
      grep -Fxq "arch = $arch" "$installed/.PKGINFO"
      cmp "$fixtures/binary-$binary_arch" "$installed/usr/bin/gitcomet"
      test -x "$installed/usr/bin/gitcomet"
      desktop-file-validate "$installed/usr/share/applications/gitcomet.desktop"
      for size in 32 48 128 256 512; do
        cmp "$repo_dir/assets/linux/hicolor/${size}x${size}/apps/gitcomet.png" \
          "$installed/usr/share/icons/hicolor/${size}x${size}/apps/gitcomet.png"
      done
      for name in LICENSE-AGPL-3.0 NOTICE; do
        cmp "$repo_dir/$name" "$installed/usr/share/licenses/gitcomet-bin/$name"
      done
      # This comes from the tagged checkout, not whichever branch runs the workflow.
      tar -xOf "$fixtures/gitcomet-source-v$version.tar.gz" "GitComet-$version/README.md" | \
        cmp - "$installed/usr/share/doc/gitcomet-bin/README.md"
    )
  done

  # A new upstream version resets the package revision.
  sed -i 's/pkgver = .*/pkgver = 9.9.9/' "$aur_dir/.SRCINFO"
  run_logged scripts/update-aur.sh "${update_args[@]}"
  grep -Eq '^[[:space:]]*pkgrel = 1$' "$aur_dir/.SRCINFO"
  echo "AUR metadata and both architecture payloads passed for $version."
done

if scripts/update-aur.sh --version > "$work/missing-value.log" 2>&1; then
  echo 'Expected a missing option value to be rejected.' >&2
  exit 1
fi
grep -Fq 'Option --version requires a value.' "$work/missing-value.log"

if scripts/update-aur.sh "${update_args[@]}" --version 0.0.0-beta.1 > "$work/version.log" 2>&1; then
  echo 'Expected an invalid release version to be rejected.' >&2
  exit 1
fi
grep -Fq 'Invalid --version' "$work/version.log"

[[ "$(vercmp 0.0.1rc.1 0.0.1)" == -1 ]]
echo 'AUR packaging regression checks passed.'
