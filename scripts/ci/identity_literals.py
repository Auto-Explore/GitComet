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
# Code that never ships: `#[cfg(test)]`, `#[cfg(all(test, ...))]`, or the
# benchmark feature; never `#[cfg(not(test))]`.
TEST_CFG = (r"#\[cfg\((?![^\]]*not\(\s*(?:test\b|feature))"
            r"[^\]]*(?:\btest\b|feature\s*=\s*\"benchmarks\")[^\]]*\)\]\s*")
TEST_MODULE = re.compile(TEST_CFG + r"(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")
TEST_MODULE_FILE = re.compile(
    TEST_CFG + r"(?:#\[path\s*=\s*\"([^\"]+)\"\]\s*)?(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")


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


def occurrences(text):
    """(line, kind, excerpt) for every product literal in production code."""
    code = strip_comments_and_test_modules(text)
    found = []
    for match in STRING.finditer(code):
        literal = match.group(2) if match.group(2) is not None else match.group(3)
        line = code.count("\n", 0, match.start()) + 1
        for kind, pattern in KINDS.items():
            found.extend((line, kind, literal[:80]) for _ in pattern.findall(literal))
    for match in PACKAGE_METADATA.finditer(code):
        found.append((code.count("\n", 0, match.start()) + 1, "package-metadata", match.group(0)))
    return found


def scan_text(text):
    return +Counter(kind for _, kind, _ in occurrences(text))


def test_module_files(path, text):
    """Files that `path` declares as `#[cfg(test)] mod name;`."""
    source = Path(path)
    own_dir = source.parent
    child_dir = own_dir if source.name in ("mod.rs", "lib.rs", "main.rs") else own_dir / source.stem
    found = set()
    for match in TEST_MODULE_FILE.finditer(strip_comments_and_test_modules(text)):
        explicit, name = match.groups()
        if explicit:
            found.add((own_dir / explicit).as_posix())
        else:
            found.update({(child_dir / f"{name}.rs").as_posix(), (child_dir / name / "mod.rs").as_posix()})
    return found


def scan():
    files = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "crates/**/*.rs"], cwd=ROOT,
                           stdout=subprocess.PIPE, check=True).stdout.decode().split("\0")
    files = sorted({path for path in files if path and (ROOT / path).is_file()})
    texts = {path: (ROOT / path).read_text(encoding="utf-8") for path in files}
    test_files = set()
    for path, text in texts.items():
        test_files |= test_module_files(path, text)
    # A test module's descendants are test code too.
    def parent_module(path):
        # `a/b/c.rs` and `a/b/c/mod.rs` are children of `a/b.rs` or `a/b/mod.rs`.
        own = Path(path)
        container = own.parent.parent if own.name == "mod.rs" else own.parent
        candidates = (container.with_suffix(".rs"), container / "mod.rs", container / "lib.rs")
        return next((c.as_posix() for c in candidates if c.as_posix() in texts and c != own), None)

    def in_test_module(path, seen=()):
        if path in test_files:
            return True
        parent = parent_module(path)
        return parent is not None and parent not in seen and in_test_module(parent, (*seen, path))
    found = {}
    for path in files:
        if is_test_path(path) or in_test_module(path):
            continue
        counts = scan_text(texts[path])
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
    parser.add_argument("--list", nargs="*", metavar="PATH", help="print each literal in PATHs")
    args = parser.parse_args()
    if args.list is not None:
        for path in args.list:
            for line, kind, excerpt in occurrences((ROOT / path).read_text(encoding="utf-8")):
                print(f"{path}:{line}: {kind}: {excerpt}")
        return 0
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
