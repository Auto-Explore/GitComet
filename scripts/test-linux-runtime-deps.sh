#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
temp_dir="$(mktemp -d)"
trap 'rm -rf -- "$temp_dir"' EXIT

# Use a real ELF and a local shared library so this test needs no GUI libraries.
# The same soname is automatically declared when linked, but must be declared
# manually when loaded with dlopen.
cat > "$temp_dir/library.c" <<'C'
int xkb_probe(void) { return 0; }
C
cat > "$temp_dir/probe.c" <<'C'
#include <dlfcn.h>

#ifdef LINK_XKB
extern int xkb_probe(void);
#endif

int main(void) {
    const char *libraries[] = {
        "libEGL.so.1",
#ifndef ASSEMBLED_VULKAN
        "libvulkan.so.1",
#endif
        "libwayland-client.so.0", "libwayland-egl.so.1",
#ifdef DLOPEN_XKB
        "libxkbcommon-x11.so.0",
#endif
    };
    for (unsigned i = 0; i < sizeof(libraries) / sizeof(libraries[0]); ++i) {
        void *handle = dlopen(libraries[i], RTLD_NOW);
        if (handle) dlclose(handle);
    }
#ifdef ASSEMBLED_VULKAN
    // Reproduce an optimized loader without relying on a compiler's decision
    // to split a literal into immediate stores. Volatile encoded bytes ensure
    // the full soname is absent, while the program still actually dlopens it.
    volatile const unsigned char encoded[] = {
        'L', 'I', 'B', 'V', 'U', 'L', 'K', 'A', 'N', 0x0e, 'S', 'O', 0x0e, 0x11,
    };
    char soname[sizeof(encoded) + 1];
    for (unsigned i = 0; i < sizeof(encoded); ++i) {
        soname[i] = encoded[i] ^ 0x20;
    }
    soname[sizeof(encoded)] = '\0';
    void *handle = dlopen(soname, RTLD_NOW);
    if (!handle) return 1;
    // This symbol identifies our local stub rather than an installed loader.
    if (!dlsym(handle, "xkb_probe")) return 1;
    if (dlclose(handle) != 0) return 1;
#endif
#ifdef LINK_XKB
    return xkb_probe();
#else
    return 0;
#endif
}
C

"${CC:-cc}" -shared -fPIC -Wl,-soname,libxkbcommon-x11.so.0 \
  "$temp_dir/library.c" -o "$temp_dir/libxkbcommon-x11.so.0"
"${CC:-cc}" "$temp_dir/probe.c" -ldl -o "$temp_dir/baseline"
"${CC:-cc}" -DLINK_XKB "$temp_dir/probe.c" \
  "$temp_dir/libxkbcommon-x11.so.0" -ldl -o "$temp_dir/linked"
"${CC:-cc}" -DDLOPEN_XKB "$temp_dir/probe.c" -ldl -o "$temp_dir/dlopen"
"${CC:-cc}" -shared -fPIC -Wl,-soname,libvulkan.so.1 \
  "$temp_dir/library.c" -o "$temp_dir/libvulkan.so.1"
"${CC:-cc}" -O2 -DASSEMBLED_VULKAN "$temp_dir/probe.c" -ldl -o "$temp_dir/assembled"

"$script_dir/check-linux-runtime-deps.sh" "$temp_dir/baseline"
"$script_dir/check-linux-runtime-deps.sh" "$temp_dir/linked"

# Prove both that the name is missing from the ELF and that it loads our local
# stub successfully. Missing soname strings must not reject a valid loader.
if LC_ALL=C grep -aFq 'libvulkan.so.1' "$temp_dir/assembled"; then
  echo 'Expected the assembled-soname fixture to contain no full Vulkan soname.' >&2
  exit 1
fi
LD_LIBRARY_PATH="$temp_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$temp_dir/assembled"
if ! "$script_dir/check-linux-runtime-deps.sh" "$temp_dir/assembled" > "$temp_dir/assembled.log" 2>&1; then
  cat "$temp_dir/assembled.log" >&2
  echo 'Expected a runtime-constructed soname to pass the dependency check.' >&2
  exit 1
fi
if ! grep -Fq 'warning: libvulkan.so.1 was not found' "$temp_dir/assembled.log"; then
  cat "$temp_dir/assembled.log" >&2
  echo 'Expected an advisory for the missing Vulkan soname string.' >&2
  exit 1
fi

if "$script_dir/check-linux-runtime-deps.sh" "$temp_dir/dlopen" > "$temp_dir/dlopen.log" 2>&1; then
  cat "$temp_dir/dlopen.log" >&2
  echo 'Expected the undeclared dlopen dependency to be rejected.' >&2
  exit 1
fi
if ! grep -Fq 'error: new runtime library string libxkbcommon-x11.so.0.' "$temp_dir/dlopen.log"; then
  cat "$temp_dir/dlopen.log" >&2
  echo 'Expected a diagnostic for the undeclared dlopen dependency.' >&2
  exit 1
fi

echo 'Linux runtime dependency regression checks passed.'
