#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
cd "$repo_root"

production_limit="${GITCOMET_MAX_PRODUCTION_RS_LINES:-4500}"
test_limit="${GITCOMET_MAX_TEST_RS_LINES:-6500}"

for value in "$production_limit" "$test_limit"; do
  if [[ ! "$value" =~ ^[1-9][0-9]*$ ]]; then
    echo "Rust source-size limits must be positive integers (got '$value')." >&2
    exit 2
  fi
done

failed=0
checked=0
while IFS= read -r -d '' file; do
  checked=$((checked + 1))
  lines="$(wc -l < "$file")"
  case "$file" in
    */tests/* | */benches/* | */tests.rs | *_tests.rs)
      kind="test/benchmark"
      limit="$test_limit"
      ;;
    *)
      kind="production"
      limit="$production_limit"
      ;;
  esac

  if ((lines > limit)); then
    printf '%s has %d lines; %s Rust files are limited to %d.\n' \
      "$file" "$lines" "$kind" "$limit" >&2
    failed=1
  fi
done < <(find crates -type f -name '*.rs' -print0)

if ((failed)); then
  echo "Split the reported file into cohesive modules before adding more code." >&2
  exit 1
fi

printf 'Rust source layout check passed for %d files (production <= %d, tests/benches <= %d lines).\n' \
  "$checked" "$production_limit" "$test_limit"
