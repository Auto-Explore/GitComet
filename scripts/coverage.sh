#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
if [[ $# -eq 1 && "$1" == "--help" ]]; then
  echo "Run GitComet's native tests and measure source line coverage."
  echo "Usage: bash scripts/coverage.sh"
  echo "Reports are written to target/llvm-cov/."
  exit 0
fi
if [[ $# -ne 0 ]]; then
  echo "Usage: bash scripts/coverage.sh" >&2
  exit 2
fi

for tool in rustup python3 perl genhtml; do
  if ! command -v "$tool" >/dev/null; then
    echo "Coverage requires $tool. See CONTRIBUTING.md for setup." >&2
    exit 1
  fi
done
cargo nextest --version
coverage_version=$(cargo llvm-cov --version)
echo "$coverage_version"

coverage_setup() {
  # These first-party platform libraries are path dependencies outside the workspace.
  cargo llvm-cov show-env --sh --remap-path-prefix \
    --dep-coverage gitcomet-filesystem-native \
    --dep-coverage gitcomet-win32-window-utils
}

# Cargo does not fingerprint the coverage wrapper's crate selection. Give each
# instrumentation setup its own final artifacts and intermediate fingerprints.
export CARGO_TARGET_DIR="$PWD/target/source-coverage-build"
export CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR"
coverage_env=$(coverage_setup)
cache_key=$(
  {
    printf '%s\n' "$coverage_version"
    printf '%s\n' "$coverage_env" | sed '/^export LLVM_PROFILE_FILE=/d'
    cat scripts/ci/coverage_rustc.rs
  } | python3 -c 'import hashlib, sys; sys.stdout.write(hashlib.sha256(sys.stdin.buffer.read()).hexdigest()[:16])'
)
export CARGO_TARGET_DIR="$CARGO_TARGET_DIR/$cache_key"
export CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR"
coverage_env=$(coverage_setup)
eval "$coverage_env"
export GITCOMET_COVERAGE_RUSTC_WRAPPER="$RUSTC_WRAPPER"
exec python3 scripts/ci/source_coverage.py
