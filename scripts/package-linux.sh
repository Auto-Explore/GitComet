#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/cli.sh"
TEMPLATE_DIR="${SCRIPT_DIR}/../packaging/linux"

usage() {
  cat <<'EOF'
Usage:
  scripts/package-linux.sh stage    --binary PATH --source DIR --out DIR
  scripts/package-linux.sh tarball  --payload DIR --version VERSION --arch ARCH --out DIR
  scripts/package-linux.sh deb      --payload DIR --version VERSION --revision N --arch DEB_ARCH --out DIR
  scripts/package-linux.sh rpm      --payload DIR --version VERSION --release N --arch RPM_ARCH --out DIR
  scripts/package-linux.sh appimage --payload DIR --version VERSION --arch ARCH --appimage-arch ARCH
                                    --appimagetool PATH --out DIR

`stage` lays out the shared payload: the binary, desktop entry, hicolor icons,
licence files and README. The .deb, .rpm and AppImage include the full payload.
The tarball keeps its historical layout with only the binary, README,
LICENSE-AGPL-3.0 and NOTICE. Each packaging command writes one file in --out.

--source is the release source checkout that holds assets/, README.md,
LICENSE-AGPL-3.0 and NOTICE. VERSION is the release version (1.2.3 or
1.2.3-rc.1). Package versions use `~rc.N` so a pre-release sorts before the
final release; file names replace `~` with `.` because GitHub renames it on
upload, and checksums must match the published names.
EOF
}

ICON_SIZES=(32 48 128 256 512)

fail() {
  echo "$*" >&2
  exit 1
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || fail "Required tool not found: $1"
}

# One command runs per invocation, so one scratch directory is enough.
WORK=""
trap '[[ -z "$WORK" ]] || rm -rf "$WORK"' EXIT

make_workdir() {
  WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gitcomet-package.XXXXXX")"
}

validate_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] ||
    fail "Invalid --version '$1'. Expected 1.2.3 or 1.2.3-rc.1."
}

validate_counter() {
  [[ "$2" =~ ^[1-9][0-9]*$ ]] || fail "Invalid $1 '$2'. Expected a positive integer."
}

package_version() {
  echo "${1/-rc./~rc.}"
}

asset_version() {
  echo "${1//\~/.}"
}

require_payload() {
  [[ -x "$1/usr/bin/gitcomet" ]] || fail "Not a staged payload (missing usr/bin/gitcomet): $1"
}

cmd_stage() {
  local out="$1" binary="$2" source="$3" size
  [[ -f "$binary" ]] || fail "Binary not found: $binary"
  [[ -d "$source/assets/linux" ]] || fail "Source checkout has no assets/linux: $source"
  if [[ -e "$out" ]] && [[ -n "$(ls -A "$out")" ]]; then
    fail "Payload directory is not empty: $out"
  fi
  require_tool desktop-file-validate

  install -Dm755 "$binary" "$out/usr/bin/gitcomet"
  install -Dm644 "$source/assets/linux/gitcomet.desktop" "$out/usr/share/applications/gitcomet.desktop"
  desktop-file-validate "$out/usr/share/applications/gitcomet.desktop"
  for size in "${ICON_SIZES[@]}"; do
    install -Dm644 "$source/assets/linux/hicolor/${size}x${size}/apps/gitcomet.png" \
      "$out/usr/share/icons/hicolor/${size}x${size}/apps/gitcomet.png"
  done
  install -Dm644 "$source/LICENSE-AGPL-3.0" "$out/usr/share/licenses/gitcomet/LICENSE-AGPL-3.0"
  install -Dm644 "$source/NOTICE" "$out/usr/share/licenses/gitcomet/NOTICE"
  install -Dm644 "$source/README.md" "$out/usr/share/doc/gitcomet/README.md"
  echo "Staged payload in $out"
}

cmd_tarball() {
  local payload="$1" out="$2" version="$3" arch="$4"
  local root="gitcomet-v${version}-linux-${arch}" work
  make_workdir
  work="$WORK"

  # The historical layout, which third-party packages (e.g. AUR gitcomet-bin) unpack.
  install -Dm755 "$payload/usr/bin/gitcomet" "$work/$root/gitcomet"
  install -m644 "$payload/usr/share/doc/gitcomet/README.md" "$work/$root/README.md"
  install -m644 "$payload/usr/share/licenses/gitcomet/LICENSE-AGPL-3.0" "$work/$root/LICENSE-AGPL-3.0"
  install -m644 "$payload/usr/share/licenses/gitcomet/NOTICE" "$work/$root/NOTICE"
  tar -C "$work" -czf "$out/${root}.tar.gz" "$root"
  echo "Wrote $out/${root}.tar.gz"
}

cmd_deb() {
  local payload="$1" out="$2" version="$3" revision="$4" arch="$5"
  local deb_version work pkg_root pkg_file
  deb_version="$(package_version "$version")-${revision}"
  pkg_file="$out/gitcomet_$(asset_version "$deb_version")_${arch}.deb"
  require_tool dpkg-shlibdeps
  require_tool dpkg-gencontrol
  require_tool dpkg-deb
  make_workdir
  work="$WORK"

  pkg_root="$work/debian/gitcomet"
  mkdir -p "$pkg_root/DEBIAN"
  cp -a "$payload/." "$pkg_root/"
  # Debian keeps licence text in /usr/share/doc/<package>/copyright.
  {
    cat "$pkg_root/usr/share/licenses/gitcomet/NOTICE"
    echo
    cat "$pkg_root/usr/share/licenses/gitcomet/LICENSE-AGPL-3.0"
  } > "$pkg_root/usr/share/doc/gitcomet/copyright"
  rm -rf "$pkg_root/usr/share/licenses"

  sed "s/@DEB_ARCH@/${arch}/" "$TEMPLATE_DIR/debian-control.in" > "$work/debian/control"
  {
    echo "gitcomet (${deb_version}) unstable; urgency=medium"
    echo
    echo "  * Release ${version}."
    echo
    echo " -- AutoExplore Oy <info@autoexplore.ai>  $(date -Ru)"
  } > "$work/debian/changelog"

  (
    cd "$work"
    dpkg-shlibdeps -Tdebian/substvars debian/gitcomet/usr/bin/gitcomet
    dpkg-gencontrol -pgitcomet -Pdebian/gitcomet -Tdebian/substvars
  )
  dpkg-deb --root-owner-group --build "$pkg_root" "$pkg_file"

  local name got_version got_arch depends
  name="$(dpkg-deb -f "$pkg_file" Package)"
  got_version="$(dpkg-deb -f "$pkg_file" Version)"
  got_arch="$(dpkg-deb -f "$pkg_file" Architecture)"
  depends="$(dpkg-deb -f "$pkg_file" Depends)"
  echo "Depends: $depends"
  [[ "$name" == gitcomet ]] || fail "Unexpected .deb package name: $name"
  [[ "$got_version" == "$deb_version" ]] || fail "Unexpected .deb version: $got_version, expected $deb_version"
  [[ "$got_arch" == "$arch" ]] || fail "Unexpected .deb architecture: $got_arch, expected $arch"
  grep -Eq '(^|, )git($|, )' <<<"$depends" || fail "Missing git dependency in .deb: $depends"
  grep -q 'libc6' <<<"$depends" || fail "Missing shared library dependencies in .deb: $depends"
  grep -Fq 'libvulkan1 | libegl1' <<<"$depends" || fail "Missing renderer dependency in .deb: $depends"
  echo "Wrote $pkg_file"
}

cmd_rpm() {
  local payload="$1" out="$2" version="$3" release="$4" arch="$5"
  local rpm_version work built pkg_file
  rpm_version="$(package_version "$version")"
  pkg_file="$out/gitcomet-$(asset_version "$rpm_version")-${release}.${arch}.rpm"
  require_tool rpmbuild
  require_tool rpm
  make_workdir
  work="$WORK"

  rpmbuild -bb \
    --define "_topdir $work" \
    --define "gc_version $rpm_version" \
    --define "gc_release $release" \
    --define "gc_payload $(cd "$payload" && pwd)" \
    --target "$arch" \
    "$TEMPLATE_DIR/gitcomet.spec"

  built="$work/RPMS/${arch}/gitcomet-${rpm_version}-${release}.${arch}.rpm"
  [[ -f "$built" ]] || fail "rpmbuild did not produce $built"
  cp "$built" "$pkg_file"

  local fields requires
  fields="$(rpm -qp --queryformat '%{NAME}|%{VERSION}|%{RELEASE}|%{ARCH}' "$pkg_file")"
  requires="$(rpm -qp --requires "$pkg_file")"
  echo "Requires:"
  echo "$requires"
  [[ "$fields" == "gitcomet|${rpm_version}|${release}|${arch}" ]] ||
    fail "Unexpected RPM metadata: $fields, expected gitcomet|${rpm_version}|${release}|${arch}"
  grep -qx 'git' <<<"$requires" || fail "Missing git dependency in RPM."
  grep -q '^libc\.so\.6' <<<"$requires" || fail "Missing shared library dependencies in RPM (AutoReq did not run)."
  grep -Fqx '(vulkan-loader or libglvnd-egl)' <<<"$requires" || fail "Missing renderer dependency in RPM."
  echo "Wrote $pkg_file"
}

cmd_appimage() {
  local payload="$1" out="$2" version="$3" arch="$4" appimage_arch="$5" tool="$6"
  local appdir work
  [[ -x "$tool" ]] || fail "appimagetool not found or not executable: $tool"
  make_workdir
  work="$WORK"

  appdir="$work/AppDir"
  mkdir -p "$appdir"
  cp -a "$payload/." "$appdir/"
  install -m644 "$payload/usr/share/applications/gitcomet.desktop" "$appdir/gitcomet.desktop"
  install -m644 "$payload/usr/share/icons/hicolor/512x512/apps/gitcomet.png" "$appdir/gitcomet.png"
  cat > "$appdir/AppRun" <<'APPRUN'
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${HERE}/usr/bin/gitcomet" "$@"
APPRUN
  chmod +x "$appdir/AppRun"

  ARCH="$appimage_arch" "$tool" --appimage-extract-and-run \
    "$appdir" "$out/gitcomet-v${version}-linux-${arch}.AppImage"
  echo "Wrote $out/gitcomet-v${version}-linux-${arch}.AppImage"
}

[[ $# -ge 1 ]] || { usage >&2; exit 2; }
command="$1"
shift
case "$command" in
  -h|--help) usage; exit 0 ;;
  stage|tarball|deb|rpm|appimage) ;;
  *) echo "Unknown command: $command" >&2; usage >&2; exit 2 ;;
esac

binary="" source="" payload="" out="" version="" revision="" release="" arch="" appimage_arch="" appimagetool=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) require_value "$@"; binary="$2"; shift 2 ;;
    --source) require_value "$@"; source="$2"; shift 2 ;;
    --payload) require_value "$@"; payload="$2"; shift 2 ;;
    --out) require_value "$@"; out="$2"; shift 2 ;;
    --version) require_value "$@"; version="$2"; shift 2 ;;
    --revision) require_value "$@"; revision="$2"; shift 2 ;;
    --release) require_value "$@"; release="$2"; shift 2 ;;
    --arch) require_value "$@"; arch="$2"; shift 2 ;;
    --appimage-arch) require_value "$@"; appimage_arch="$2"; shift 2 ;;
    --appimagetool) require_value "$@"; appimagetool="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$out" ]] || fail "--out is required."
if [[ "$command" == stage ]]; then
  [[ -n "$binary" && -n "$source" ]] || fail "stage requires --binary and --source."
  cmd_stage "$out" "$binary" "$source"
  exit 0
fi

[[ -n "$payload" && -n "$version" && -n "$arch" ]] || fail "$command requires --payload, --version and --arch."
require_payload "$payload"
validate_version "$version"
mkdir -p "$out"
case "$command" in
  tarball)
    cmd_tarball "$payload" "$out" "$version" "$arch"
    ;;
  deb)
    validate_counter --revision "$revision"
    cmd_deb "$payload" "$out" "$version" "$revision" "$arch"
    ;;
  rpm)
    validate_counter --release "$release"
    cmd_rpm "$payload" "$out" "$version" "$release" "$arch"
    ;;
  appimage)
    [[ -n "$appimage_arch" && -n "$appimagetool" ]] || fail "appimage requires --appimage-arch and --appimagetool."
    cmd_appimage "$payload" "$out" "$version" "$arch" "$appimage_arch" "$appimagetool"
    ;;
esac
