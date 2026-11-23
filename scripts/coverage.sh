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

for tool in rustup python3 genhtml; do
  if ! command -v "$tool" >/dev/null; then
    echo "Coverage requires $tool. See CONTRIBUTING.md for setup." >&2
    exit 1
  fi
done
cargo nextest --version
cargo llvm-cov --version
export CARGO_TARGET_DIR="$PWD/target/llvm-cov-target"
coverage_env=$(cargo llvm-cov show-env --sh)
eval "$coverage_env"
exec python3 scripts/ci/source_coverage.py
