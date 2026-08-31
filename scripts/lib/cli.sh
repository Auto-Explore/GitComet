#!/usr/bin/env bash
# Shared CLI helpers for the release/symbol scripts. Sourced, not executed.

# `shift 2` on a flag whose value is missing fails under `set -e`, killing the
# script before the diagnostics below can run.
require_value() {
  if [[ $# -lt 2 ]]; then
    echo "Option $1 requires a value." >&2
    exit 2
  fi
}
