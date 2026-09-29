#!/usr/bin/env python3
"""Keep product identity out of production string literals.

Names, identifiers, and links come from `gitcomet_core::identity`, so a
downstream application can rename the product without patching sources. This
check scans production Rust sources (test files and inline `#[cfg(test)]`
modules excluded) for product literals and Cargo package metadata, and compares
each file's count with `identity-literals.txt`:

    <path> <kind> <count> <reason>

A count above the recorded one is a new literal: read it from the identity
instead. A count below it is a stale exception: lower the count. `--write`
regenerates the counts, keeping recorded reasons.
"""

import argparse
from collections import Counter
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
ALLOWLIST = Path(__file__).with_name("identity-literals.txt")
KINDS = {
    "display-name": re.compile(r"GitComet"),
    # `gitcomet` as a word: file, directory, app, or tool names. Crate paths
    # (`gitcomet_core`) and environment variables (`GITCOMET_*`) do not match.
    "identifier": re.compile(r"(?<![A-Za-z0-9_])gitcomet(?![A-Za-z0-9_])"),
    "vendor": re.compile(r"autoexplore|Auto-Explore"),
}
PACKAGE_METADATA = re.compile(r'env!\(\s*"CARGO_PKG_(?:NAME|VERSION|REPOSITORY|HOMEPAGE|DESCRIPTION)"\s*\)')
STRING = re.compile(r'(?:b|c)?r(#*)"(.*?)"\1|(?:b|c)?"((?:\\.|[^"\\])*)"', re.S)
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")


def is_test_path(path):
    parts = path.split("/")
    name = parts[-1]
    return ("tests" in parts or "benches" in parts or name in ("tests.rs", "test_support.rs")
            or name.endswith("_tests.rs"))


def strip_comments_and_test_modules(text):
    """Blank out comments and inline test modules, keeping line numbers."""
    out = []
    i, n = 0, len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
            continue
        if text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(re.sub(r"[^\n]", " ", text[i:j]))
            i = j
            continue
        match = STRING.match(text, i)
        if match and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            out.append(match.group(0))
            i = match.end()
            continue
        if text[i] == "'" and (m := re.match(r"'(?:\\u\{[0-9a-fA-F]+\}|\\.|[^\\'\n])'", text[i:])):
            # A char literal can hold a quote or brace; keep only its width.
            out.append("'" + "_" * (m.end() - 2) + "'")
            i += m.end()
            continue
        out.append(text[i])
        i += 1
    code = "".join(out)
    # Remove `#[cfg(test)] mod x { ... }` bodies by brace matching.
    result, position = [], 0
    for match in TEST_MODULE.finditer(code):
        if match.start() < position:
            continue
        depth, j = 1, match.end()
        while j < len(code) and depth:
            char = code[j]
            if char == '"':
                string = STRING.match(code, j)
                j = string.end() if string else j + 1
                continue
            depth += char == "{"
            depth -= char == "}"
            j += 1
        result.append(code[position:match.start()])
        result.append(re.sub(r"[^\n]", " ", code[match.start():j]))
        position = j
    result.append(code[position:])
    return "".join(result)


def scan_text(text):
    code = strip_comments_and_test_modules(text)
    counts = Counter()
    for match in STRING.finditer(code):
        literal = match.group(2) if match.group(2) is not None else match.group(3)
        for kind, pattern in KINDS.items():
            counts[kind] += len(pattern.findall(literal))
    counts["package-metadata"] += len(PACKAGE_METADATA.findall(code))
    return +counts


def scan():
    files = subprocess.run(["git", "ls-files", "-z", "--", "crates/**/*.rs"], cwd=ROOT,
                           stdout=subprocess.PIPE, check=True).stdout.decode().split("\0")
    found = {}
    for path in filter(None, files):
        if is_test_path(path):
            continue
        counts = scan_text((ROOT / path).read_text(encoding="utf-8"))
        for kind, count in counts.items():
            found[(path, kind)] = count
    return found


def read_allowlist(path=ALLOWLIST):
    entries = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split(None, 3)
        if len(fields) < 4 or not fields[2].isdigit():
            raise ValueError(f"{path.name}:{number}: expected '<path> <kind> <count> <reason>'")
        entries[(fields[0], fields[1])] = (int(fields[2]), fields[3])
    return entries


def compare(found, allowed):
    problems = []
    for key in sorted(set(found) | set(allowed)):
        have = found.get(key, 0)
        limit = allowed.get(key, (0, ""))[0]
        path, kind = key
        if have > limit:
            problems.append(f"{path}: {have - limit} new {kind} literal(s); use gitcomet_core::identity")
        elif have < limit:
            problems.append(f"{path}: {kind} exception allows {limit} but {have} remain; lower the count")
    return problems


def write(found, allowed, path=ALLOWLIST):
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line.startswith("#")] \
        if path.exists() else []
    for (file, kind), count in sorted(found.items()):
        reason = allowed.get((file, kind), (0, "TODO: explain why this cannot come from the identity"))[1]
        lines.append(f"{file} {kind} {count} {reason}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--write", action="store_true", help="rewrite the allowlist counts")
    args = parser.parse_args()
    found = scan()
    allowed = read_allowlist() if ALLOWLIST.exists() else {}
    if args.write:
        write(found, allowed)
        return 0
    problems = compare(found, allowed)
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if not problems:
        print(f"Identity literal check passed ({sum(found.values())} recorded exceptions).")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
