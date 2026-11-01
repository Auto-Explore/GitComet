#!/usr/bin/env bash
set -euo pipefail

# Keep linker selection independent of RUSTFLAGS (coverage and profiling set it).
# Put this last so it also overrides any linker selection passed by rustc.
exec clang "$@" -fuse-ld=mold
