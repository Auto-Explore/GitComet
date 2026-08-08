#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to manage Rust toolchain components." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to run coverage." >&2
  exit 1
fi

if ! rustup component list --installed | grep -Eq '^llvm-tools(-preview)?($|-)'; then
  echo "Missing rustup component: llvm-tools (aka llvm-tools-preview)." >&2
  echo "Install with: rustup component add llvm-tools-preview" >&2
  exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "Missing cargo subcommand: cargo-llvm-cov" >&2
  echo "Install with: cargo install --locked cargo-llvm-cov" >&2
  exit 1
fi

coverage_dir="target/llvm-cov"
lcov_path="${coverage_dir}/lcov.info"
html_dir="${coverage_dir}/html"

mkdir -p "$coverage_dir"

cargo llvm-cov \
  --workspace \
  --exclude gitcomet-ui-gpui \
  --no-default-features \
  --features gix \
  --lcov \
  --output-path "$lcov_path" \
  "$@"

cargo llvm-cov \
  --workspace \
  --exclude gitcomet-ui-gpui \
  --no-default-features \
  --features gix \
  --html \
  --output-dir "$html_dir" \
  --no-run \
  "$@"

if [[ -f "$lcov_path" && -f "${html_dir}/index.html" ]]; then
  echo "Coverage summary generated."
  echo "LCOV report: ${repo_root}/${lcov_path}"
  echo "HTML report: ${repo_root}/${html_dir}/index.html"
else
  echo "cargo llvm-cov completed."
fi
