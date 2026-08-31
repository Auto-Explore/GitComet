#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/cli.sh"

usage() {
  cat <<'EOF'
Usage: scripts/emit-breakpad-symbols.sh --input PATH --store DIR [--arch ARCH] [--module-name NAME] [--min-bytes N] [--allow-missing-inlines] [--allow-missing-cfi]

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
  --module-name        Module name to record, overriding the one dump_syms
                       derives from the input file name. Required for a .dSYM,
                       whose name matches neither the executable a crash report
                       carries nor what the store is keyed on. Rejected for a
                       .pdb, where dump_syms already names it correctly.
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

input=""
store=""
arch=""
module_name=""
module_name_given=0
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
    --module-name)
      require_value "$@"
      module_name="$2"
      module_name_given=1
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

# Empty is otherwise indistinguishable from absent, and both the renaming and
# its verification are skipped -- so a CI variable expanding to nothing would
# publish the layout this flag exists to prevent, with a green build.
if [[ "$module_name_given" -eq 1 && -z "$module_name" ]]; then
  echo "--module-name was given an empty value." >&2
  exit 2
fi

# It becomes a file name below, where a path would fail at `ln` against a
# temporary directory the caller never named.
if [[ "$module_name" == */* || "$module_name" == "." || "$module_name" == ".." ]]; then
  echo "--module-name must be a plain file name, got '${module_name}'." >&2
  exit 2
fi

# Enforced here rather than at the call site: this is what knows the input kind,
# and both names dump_syms can infer from a bundle are ones nothing asks for.
# Windows keys the store on the .pdb name, which is what a stackwalker
# resolving a WER minidump asks for, so any override here can only be wrong.
if [[ -n "$module_name" && "$input" == *.pdb ]]; then
  echo "--module-name must not be used with a .pdb: $input" >&2
  echo "Windows symbol lookups key on the .pdb name, so dump_syms' own naming is" >&2
  echo "already correct and overriding it would make the symbols unreachable." >&2
  exit 2
fi

if [[ -z "$module_name" && ( -d "$input" || "$input" == *.dSYM ) ]]; then
  echo "--module-name is required when reading a .dSYM: $input" >&2
  echo "Without it the module is named after the bundle or the hashed file inside" >&2
  echo "it, and a stackwalker looking up MODULE/DEBUG_ID/MODULE.sym finds neither." >&2
  echo "Pass the executable's name, e.g. --module-name gitcomet." >&2
  exit 2
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
link_dir="$(mktemp -d)"
sym_list="$(mktemp)"
trap 'rm -rf "$staging" "$link_dir" "$sym_list"' EXIT

# dump_syms names the module after the file it is handed, and the store path
# follows that name. Hand it a correctly named symlink rather than rewriting its
# output, so the MODULE line and the store path stay consistent by construction.
dump_input="$input"
if [[ -n "$module_name" ]]; then
  dwarf_input="$input"

  if [[ -d "$input" ]]; then
    dwarf_dir="${input%/}/Contents/Resources/DWARF"
    if [[ ! -d "$dwarf_dir" ]]; then
      echo "--module-name expects a file or a .dSYM bundle, and ${input} is neither." >&2
      exit 1
    fi

    # Dotfiles excluded so a .DS_Store cannot fail a valid bundle. Counted with
    # grep rather than an array: macOS runners ship bash 3.2, which has no
    # mapfile.
    dwarf_files="$(find "$dwarf_dir" -maxdepth 1 -type f ! -name '.*' | sort)"
    dwarf_count="$(printf '%s' "$dwarf_files" | grep -c . || true)"
    if [[ "$dwarf_count" -ne 1 ]]; then
      echo "Expected exactly one Mach-O under ${dwarf_dir}, found ${dwarf_count}." >&2
      printf '%s\n' "$dwarf_files" >&2
      exit 1
    fi
    dwarf_input="$dwarf_files"
  fi

  if [[ "$dwarf_input" != /* ]]; then
    dwarf_input="$(cd "$(dirname "$dwarf_input")" && pwd)/$(basename "$dwarf_input")"
  fi

  dump_input="${link_dir}/${module_name}"
  ln -s "$dwarf_input" "$dump_input"
fi

dump_syms_args=(--store "$staging" --inlines)
if [[ -n "$arch" ]]; then
  # Guards against picking the wrong slice out of a universal binary.
  dump_syms_args+=(--arch "$arch")
fi

echo "Generating Breakpad symbols from ${input}"
dump_syms "${dump_syms_args[@]}" "$dump_input"

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

  # If dump_syms ever stops deriving this from the file name, the store path
  # silently stops matching what a stackwalker asks for.
  if [[ -n "$module_name" ]]; then
    recorded_name="${module_line##* }"
    if [[ "$recorded_name" != "$module_name" ]]; then
      echo "Module is recorded as '${recorded_name}', expected '${module_name}': ${rel}" >&2
      echo "A stackwalker looks up ${module_name}/DEBUG_ID/${module_name}.sym and would miss this file." >&2
      exit 1
    fi
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
