#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/emit-breakpad-symbols.sh --input PATH --store DIR [--arch ARCH] [--min-bytes N] [--allow-missing-inlines] [--allow-missing-cfi]

Converts a binary, .pdb or .dSYM into a Breakpad .sym file and writes it into a
symbol store laid out as NAME/DEBUG_ID/NAME.sym, which is what
`minidump-stackwalk --symbols-url` expects.

The generated file is then sanity checked. Symbols that are present but empty
are worse than no symbols at all: they make a crash look symbolized while every
frame stays unresolved, and nothing else in the pipeline would notice.

Options:
  --input              Binary, .pdb or .dSYM to read symbols from
  --store              Symbol store root to write into (created if missing)
  --arch               Architecture to select from a fat binary (macOS)
  --min-bytes          Minimum acceptable .sym size (default: 1048576)
  --allow-missing-inlines
                       Warn instead of failing when no INLINE records are
                       produced. Inline records depend on rustc and dump_syms
                       behaviour that a toolchain bump could change; that is a
                       symbol-quality regression, not a reason to redden six
                       release jobs with no override.
  --allow-missing-cfi  Warn instead of failing when no STACK records are
                       produced. Needed on macOS, where dsymutil keeps DWARF in
                       the .dSYM but leaves __eh_frame in the linked binary, and
                       dump_syms reads only one file (unlike Windows, which
                       re-pairs a .pdb with its .exe).
EOF
}

# `shift 2` on a flag whose value is missing fails under `set -e`, killing the
# script before the diagnostics below can run.
require_value() {
  if [[ $# -lt 2 ]]; then
    echo "Option $1 requires a value." >&2
    exit 2
  fi
}

input=""
store=""
arch=""
min_bytes=1048576
allow_missing_cfi=0
allow_missing_inlines=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input)
      require_value "$@"
      input="$2"
      shift 2
      ;;
    --store)
      require_value "$@"
      store="$2"
      shift 2
      ;;
    --arch)
      require_value "$@"
      arch="$2"
      shift 2
      ;;
    --min-bytes)
      require_value "$@"
      min_bytes="$2"
      shift 2
      ;;
    --allow-missing-cfi)
      allow_missing_cfi=1
      shift
      ;;
    --allow-missing-inlines)
      allow_missing_inlines=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$input" || -z "$store" ]]; then
  echo "--input and --store are both required." >&2
  usage >&2
  exit 2
fi

if [[ ! "$min_bytes" =~ ^[0-9]+$ ]]; then
  echo "--min-bytes must be a non-negative integer, got '${min_bytes}'." >&2
  exit 2
fi

if [[ ! -e "$input" ]]; then
  echo "Symbol input not found: $input" >&2
  exit 1
fi

if ! command -v dump_syms >/dev/null 2>&1; then
  echo "dump_syms is not on PATH. Install it with scripts/install-dump-syms.sh." >&2
  exit 1
fi

# Dump into a staging store so everything found afterwards is known to be from
# this run. Timestamp comparison against the destination would be wrong on any
# filesystem whose mtime granularity exceeds a fast dump_syms run, and a store
# may legitimately already hold symbols for other modules or earlier builds.
staging="$(mktemp -d)"
sym_list="$(mktemp)"
trap 'rm -rf "$staging" "$sym_list"' EXIT

dump_syms_args=(--store "$staging" --inlines)
if [[ -n "$arch" ]]; then
  # Guards against picking the wrong slice out of a universal binary.
  dump_syms_args+=(--arch "$arch")
fi

echo "Generating Breakpad symbols from ${input}"
dump_syms "${dump_syms_args[@]}" "$input"

find "$staging" -type f -name '*.sym' | sort >"$sym_list"

if [[ ! -s "$sym_list" ]]; then
  echo "dump_syms reported success but wrote no .sym file." >&2
  exit 1
fi

while IFS= read -r sym; do
  rel="${sym#"$staging"/}"

  # `wc -c` on a regular file is a stat, not a read. Everything else comes from
  # a single awk pass: separate greps would each re-read a ~239 MiB file, and
  # `^STACK` in particular cannot short-circuit because Breakpad emits STACK
  # records last.
  size="$(wc -c <"$sym" | tr -d '[:space:]')"
  # `func` is a reserved word in awk, hence the saw_* names.
  IFS=' ' read -r has_func has_inline has_stack module_line <<<"$(awk '
    NR == 1 { module = $0 }
    /^FUNC / { saw_func = 1 }
    /^INLINE / { saw_inline = 1 }
    /^STACK / { saw_stack = 1 }
    END { printf "%d %d %d %s", saw_func + 0, saw_inline + 0, saw_stack + 0, module }
  ' "$sym")"

  if [[ "$size" -lt "$min_bytes" ]]; then
    echo "Symbol file is implausibly small: ${rel} (${size} bytes, expected >= ${min_bytes})." >&2
    echo "This usually means the binary was stripped before symbols were extracted." >&2
    exit 1
  fi

  if [[ "$has_func" -eq 0 ]]; then
    echo "Symbol file has no FUNC records: ${rel}" >&2
    echo "This usually means the binary was stripped before symbols were extracted." >&2
    exit 1
  fi

  if [[ "$has_inline" -eq 1 ]]; then
    :
  elif [[ "$allow_missing_inlines" -eq 1 ]]; then
    echo "::warning title=No inline records::${rel} has no INLINE records; frames inlined by LTO will collapse."
  else
    echo "Symbol file has no INLINE records: ${rel}" >&2
    echo "GitComet builds with fat LTO, so missing inline records collapse most frames." >&2
    echo "Pass --allow-missing-inlines to downgrade this to a warning." >&2
    exit 1
  fi

  # STACK records are the unwind data. Without them a stackwalker falls back to
  # scanning, which is what produces the "Found by: stack scanning" frames that
  # make a crash report unreadable.
  if [[ "$has_stack" -eq 1 ]]; then
    echo "  CFI: present"
  elif [[ "$allow_missing_cfi" -eq 1 ]]; then
    echo "::warning title=No CFI in symbols::${rel} has no STACK records; stacks will rely on scanning."
  else
    echo "Symbol file has no STACK records (no CFI): ${rel}" >&2
    echo "Pass --allow-missing-cfi only if this input genuinely cannot carry unwind data." >&2
    exit 1
  fi

  echo "  ${module_line}"
  echo "  ${store}/${rel} (${size} bytes)"
done <"$sym_list"

# Only after every file passed, so a rejected run leaves the store untouched.
# `mv` avoids copying another ~239 MiB per file; it falls back to `cp` when the
# staging directory is on a different filesystem than the store.
while IFS= read -r sym; do
  rel="${sym#"$staging"/}"
  mkdir -p "${store}/$(dirname "$rel")"
  mv "$sym" "${store}/${rel}" 2>/dev/null || cp "$sym" "${store}/${rel}"
done <"$sym_list"
