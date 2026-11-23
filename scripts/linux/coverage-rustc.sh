#!/usr/bin/env bash
# LLVM expands %b in profile filenames only when the executable has an ELF build ID.
exec "$GITCOMET_COVERAGE_RUSTC_WRAPPER" "$@" -C link-arg=-Wl,--build-id
