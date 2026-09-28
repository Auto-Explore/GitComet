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
        "libEGL.so.1", "libvulkan.so.1",
        "libwayland-client.so.0", "libwayland-egl.so.1",
#ifdef DLOPEN_XKB
        "libxkbcommon-x11.so.0",
#endif
    };
    for (unsigned i = 0; i < sizeof(libraries) / sizeof(libraries[0]); ++i) {
        void *handle = dlopen(libraries[i], RTLD_NOW);
        if (handle) dlclose(handle);
    }
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

"$script_dir/check-linux-runtime-deps.sh" "$temp_dir/baseline"
"$script_dir/check-linux-runtime-deps.sh" "$temp_dir/linked"

if "$script_dir/check-linux-runtime-deps.sh" "$temp_dir/dlopen" > "$temp_dir/dlopen.log" 2>&1; then
  cat "$temp_dir/dlopen.log" >&2
  echo 'Expected the undeclared dlopen dependency to be rejected.' >&2
  exit 1
fi
if ! grep -Fq 'error: new runtime-loaded library libxkbcommon-x11.so.0.' "$temp_dir/dlopen.log"; then
  cat "$temp_dir/dlopen.log" >&2
  echo 'Expected a diagnostic for the undeclared dlopen dependency.' >&2
  exit 1
fi

echo 'Linux runtime dependency regression checks passed.'
