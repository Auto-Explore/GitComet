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

# Every file is measured by its physical line count. An inline test module
# counts against its production file: move a large one into a child
# `tests.rs` (or a `tests/` directory), which is measured against the test
# limit. Small cohesive inline tests may stay.
failed=0
checked=0
# Include new sources during local development, and skip tracked deletions.
# Git's excludes keep build output out of this inventory.
while IFS= read -r -d '' file; do
  [[ -f "$file" ]] || continue
  checked=$((checked + 1))
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
  lines="$(wc -l < "$file")"

  if ((lines > limit)); then
    printf '%s has %d lines; %s Rust files are limited to %d.\n' \
      "$file" "$lines" "$kind" "$limit" >&2
    failed=1
  fi
done < <(git ls-files --cached --others --exclude-standard -z -- 'crates/**/*.rs')

if ((failed)); then
  echo "Split the reported file into cohesive modules before adding more code." >&2
  exit 1
fi

printf 'Rust source layout check passed for %d files (production <= %d, tests/benches <= %d lines).\n' \
  "$checked" "$production_limit" "$test_limit"
