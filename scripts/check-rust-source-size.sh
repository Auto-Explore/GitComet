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

# Lines before the file's first top-level `#[cfg(test)]`, i.e. everything that
# ships. An inline test module is measured against the test limit instead, so a
# large module is never split just because its tests grew — splitting the tests
# out is a choice, not something this check forces.
production_lines() {
  awk '
    /^#\[cfg\(test\)\]/ { print NR - 1; found = 1; exit }
    END { if (!found) print NR }
  ' "$1"
}

failed=0
checked=0
# `git ls-files` rather than `find`: it lists exactly the tracked sources, so a
# nested `target/` or any other build output can never be measured.
while IFS= read -r file; do
  checked=$((checked + 1))
  case "$file" in
    */tests/* | */benches/* | */tests.rs | *_tests.rs)
      kind="test/benchmark"
      limit="$test_limit"
      lines="$(wc -l < "$file")"
      ;;
    *)
      kind="production"
      limit="$production_limit"
      lines="$(production_lines "$file")"
      ;;
  esac

  if ((lines > limit)); then
    printf '%s has %d %s lines; %s Rust files are limited to %d.\n' \
      "$file" "$lines" "$kind" "$kind" "$limit" >&2
    failed=1
  fi
done < <(git ls-files -z -- 'crates/**/*.rs' | tr '\0' '\n')

if ((failed)); then
  echo "Split the reported file into cohesive modules before adding more code." >&2
  exit 1
fi

printf 'Rust source layout check passed for %d files (production <= %d, tests/benches <= %d lines).\n' \
  "$checked" "$production_limit" "$test_limit"
