#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/check-linux-runtime-deps.sh BINARY

Fails when the Linux binary's runtime library needs drift from what the
packages declare:

- a linked (DT_NEEDED) library outside the known set,
- a runtime-loaded (dlopen) library added or removed,
- a glibc symbol version newer than the supported baseline.

Linked libraries reach the .deb and .rpm through dpkg-shlibdeps and rpm
AutoReq. Runtime-loaded ones are invisible to both, so they are declared by
hand in packaging/linux/debian-control.in and packaging/linux/gitcomet.spec.
AUR dependencies are declared explicitly in packaging/linux/PKGBUILD.in.
Update all three templates together with the lists below.
EOF
}

linked_allowed=(
  libfontconfig.so.1
  libxcb.so.1
  libxkbcommon.so.0
  libxkbcommon-x11.so.0
  # Linked when the build host has zlib headers; aarch64 builds bundle it.
  libz.so.1
  libgcc_s.so.1
  libm.so.6
  libc.so.6
  ld-linux-x86-64.so.2
  ld-linux-aarch64.so.1
)

# Vulkan is tried first, then EGL/GL. Wayland is only used in Wayland sessions.
dlopen_expected=(
  libEGL.so.1
  libvulkan.so.1
  libwayland-client.so.0
  libwayland-egl.so.1
)

# Ubuntu 22.04, the release build host. The RPM targets Fedora 42 (glibc 2.41).
max_glibc="2.35"

if [[ $# -ne 1 || "$1" == -h || "$1" == --help ]]; then
  usage
  [[ $# -eq 1 ]] && exit 0
  exit 2
fi

binary="$1"
if [[ ! -f "$binary" ]]; then
  echo "Binary not found: $binary" >&2
  exit 1
fi

in_list() {
  local needle="$1"
  shift
  local item
  for item in "$@"; do
    [[ "$item" == "$needle" ]] && return 0
  done
  return 1
}

failed=0

mapfile -t needed < <(readelf -d "$binary" | sed -n 's/.*(NEEDED).*\[\(.*\)\]$/\1/p' | LC_ALL=C sort -u)
echo "Linked: ${needed[*]}"
for lib in "${needed[@]}"; do
  if ! in_list "$lib" "${linked_allowed[@]}"; then
    echo "error: new linked library ${lib}. dpkg-shlibdeps and rpm AutoReq declare it, but add it to packaging/linux/PKGBUILD.in, linked_allowed here and the README runtime-library note." >&2
    failed=1
  fi
done

# .dynstr also holds the DT_NEEDED names, so drop those from the soname strings.
# Only actual linked dependencies are declared by the package generators. A
# library in linked_allowed still needs a manual declaration if it moves to dlopen.
mapfile -t loaded < <(
  grep -aoE 'lib[A-Za-z0-9_+-]+\.so\.[0-9]+' "$binary" | LC_ALL=C sort -u |
    while IFS= read -r lib; do
      in_list "$lib" "${needed[@]}" || echo "$lib"
    done
)
echo "Runtime-loaded: ${loaded[*]}"
for lib in "${loaded[@]}"; do
  if ! in_list "$lib" "${dlopen_expected[@]}"; then
    echo "error: new runtime-loaded library ${lib}. Declare it in packaging/linux/debian-control.in, packaging/linux/gitcomet.spec and packaging/linux/PKGBUILD.in, then add it to dlopen_expected." >&2
    failed=1
  fi
done
for lib in "${dlopen_expected[@]}"; do
  if ! in_list "$lib" "${loaded[@]}"; then
    echo "error: ${lib} is no longer loaded. Drop it from the package dependencies and from dlopen_expected." >&2
    failed=1
  fi
done

glibc="$(readelf -V "$binary" | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/^GLIBC_//' | LC_ALL=C sort -uV | tail -n1)"
echo "Highest glibc symbol version: ${glibc:-none}"
if [[ -n "$glibc" && "$(printf '%s\n%s\n' "$glibc" "$max_glibc" | LC_ALL=C sort -V | tail -n1)" != "$max_glibc" ]]; then
  echo "error: needs glibc ${glibc}, above the ${max_glibc} baseline. Supported distros would fail to install or start it." >&2
  failed=1
fi

exit "$failed"
