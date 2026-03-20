#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/package-macos.sh --version VERSION [--arch arm64|x86_64] [--release|--debug] [--no-build] [--skip-dmg] [--out-dir PATH] [--codesign-identity NAME] [--codesign-keychain PATH]

Builds a macOS app bundle and release artifacts:
  - gitcomet-v<VERSION>-macos-<ARCH>.tar.gz
  - gitcomet-v<VERSION>-macos-<ARCH>.dmg

Defaults:
  --release, build if needed, output to ./dist

Environment:
  GITCOMET_MACOS_X86_RELEASE_LTO=thin|fat|false|off|inherit
    Overrides release LTO for Intel macOS builds. Default: thin.
USAGE
}

version=""
arch=""
mode="release"
build=1
create_dmg=1
out_dir="dist"
codesign_identity=""
codesign_keychain=""

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
    --release)
      mode="release"
      shift
      ;;
    --debug)
      mode="debug"
      shift
      ;;
    --no-build)
      build=0
      shift
      ;;
    --skip-dmg)
      create_dmg=0
      shift
      ;;
    --out-dir)
      out_dir="${2:-}"
      shift 2
      ;;
    --codesign-identity)
      codesign_identity="${2:-}"
      shift 2
      ;;
    --codesign-keychain)
      codesign_keychain="${2:-}"
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

if [[ "$arch" != "$host_arch" ]]; then
  echo "Requested --arch '$arch' does not match host architecture '$host_arch'." >&2
  echo "Use a native runner for each architecture." >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_src="${repo_root}/target/${mode}/gitcomet"

if [[ $build -eq 1 && ! -x "$bin_src" ]]; then
  cargo_config_output=""
  cargo_config_output="$("${repo_root}/scripts/macos-cargo-config.sh" "$arch" "$mode")"
  (
    cd "$repo_root"
    set --
    if [[ -n "$cargo_config_output" ]]; then
      while IFS= read -r arg; do
        set -- "$@" "$arg"
      done <<<"$cargo_config_output"
    fi
    if [[ "$mode" == "release" ]]; then
      set -- "$@" --release
    fi
    set -- "$@" -p gitcomet --locked --features ui-gpui,gix --bins
    cargo build "$@"
  )
fi

if [[ ! -x "$bin_src" ]]; then
  echo "Binary not found or not executable: $bin_src" >&2
  echo "Build first or omit --no-build." >&2
  exit 1
fi

if [[ "$out_dir" = /* ]]; then
  mkdir -p "$out_dir"
  out_abs="$(cd "$out_dir" && pwd)"
else
  mkdir -p "${repo_root}/${out_dir}"
  out_abs="$(cd "${repo_root}/${out_dir}" && pwd)"
fi

stage_root="${out_abs}/stage"
release_root="gitcomet-v${version}-macos-${arch}"
release_dir="${stage_root}/${release_root}"
app_bundle="${release_dir}/GitComet.app"
contents_dir="${app_bundle}/Contents"
macos_dir="${contents_dir}/MacOS"
resources_dir="${contents_dir}/Resources"

rm -rf "$release_dir"
mkdir -p "$macos_dir" "$resources_dir"

install -m755 "$bin_src" "${macos_dir}/gitcomet"
install -m755 "$bin_src" "${release_dir}/gitcomet"
install -m644 "${repo_root}/README.md" "${release_dir}/README.md"
install -m644 "${repo_root}/LICENSE-AGPL-3.0" "${release_dir}/LICENSE-AGPL-3.0"
install -m644 "${repo_root}/NOTICE" "${release_dir}/NOTICE"

icon_png="${repo_root}/assets/gitcomet-512.png"
icon_icns="${resources_dir}/GitComet.icns"

if [[ ! -f "$icon_png" ]]; then
  echo "Missing macOS icon source: $icon_png" >&2
  exit 1
fi
if ! command -v sips >/dev/null 2>&1; then
  echo "sips is required to build the macOS app icon." >&2
  exit 1
fi
sips -s format icns "$icon_png" --out "$icon_icns" >/dev/null

cat > "${contents_dir}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>GitComet</string>
  <key>CFBundleExecutable</key>
  <string>gitcomet</string>
  <key>CFBundleIdentifier</key>
  <string>ai.autoexplore.gitcomet</string>
  <key>CFBundleIconFile</key>
  <string>GitComet.icns</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>GitComet</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

if [[ -n "$codesign_identity" ]]; then
  if ! command -v codesign >/dev/null 2>&1; then
    echo "codesign is required when --codesign-identity is set." >&2
    exit 1
  fi

  sign_args=(--force --timestamp --options runtime --sign "$codesign_identity")
  if [[ -n "$codesign_keychain" ]]; then
    sign_args+=(--keychain "$codesign_keychain")
  fi

  echo "Signing macOS artifacts with identity: $codesign_identity"
  codesign "${sign_args[@]}" "${macos_dir}/gitcomet"
  codesign "${sign_args[@]}" "${release_dir}/gitcomet"
  codesign "${sign_args[@]}" "$app_bundle"

  codesign --verify --strict --verbose=2 "${release_dir}/gitcomet"
  codesign --verify --strict --verbose=2 "$app_bundle"
fi

# Create a deterministic tarball root directory per version/arch.
tarball_path="${out_abs}/${release_root}.tar.gz"
rm -f "$tarball_path"
tar -C "$stage_root" -czf "$tarball_path" "$release_root"

dmg_path="${out_abs}/${release_root}.dmg"
if [[ $create_dmg -eq 1 ]]; then
  # Build a drag-and-drop DMG with an /Applications shortcut.
  dmg_stage="${out_abs}/dmg-stage-${arch}"
  rm -rf "$dmg_stage"
  mkdir -p "$dmg_stage"
  cp -R "$app_bundle" "${dmg_stage}/GitComet.app"
  ln -s /Applications "${dmg_stage}/Applications"

  # Preserve compatibility with older macOS tooling.
  rm -f "$dmg_path"
  hdiutil create \
    -volname "GitComet" \
    -srcfolder "$dmg_stage" \
    -ov \
    -format UDZO \
    "$dmg_path" >/dev/null

  rm -rf "$dmg_stage"
fi

echo "Packaged macOS artifacts:"
echo "  $tarball_path"
if [[ $create_dmg -eq 1 ]]; then
  echo "  $dmg_path"
else
  echo "  (skipped DMG creation via --skip-dmg)"
fi
