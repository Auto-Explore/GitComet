#!/usr/bin/env python3
"""Replace `use super::*;` in Rust files with the names each file uses.

Usage: expand_glob_imports.py --package PKG [--features F] FILE...

For each FILE the glob is removed, the package is checked (lib and tests),
and every name the compiler reports as unresolved in that file is imported
from `super::`. Trait methods the compiler finds out of scope are imported
from the path it suggests. The loop repeats until the files compile; a name
that `super::` cannot provide falls back to the compiler's suggestion.
"""
import argparse
import collections
import json
import pathlib
import re
import subprocess
import sys

UNRESOLVED = re.compile(
    r"cannot find (?:type|function|value|trait|struct, variant or union type|macro|"
    r"unit struct, unit variant or constant|tuple struct or tuple variant|module or crate|"
    r"crate or module|derive macro|attribute macro) `([A-Za-z_][A-Za-z0-9_]*)`|"
    r"use of undeclared (?:type|crate or module) `([A-Za-z_][A-Za-z0-9_]*)`|"
    r"failed to resolve: use of undeclared (?:type|crate or module) `([A-Za-z_][A-Za-z0-9_]*)`"
)
BAD_IMPORT = re.compile(r"unresolved import `super::([A-Za-z_][A-Za-z0-9_]*)`|no `([A-Za-z_][A-Za-z0-9_]*)` in")
GLOB = "use super::*;\n"


def check(package, features):
    args = ["cargo", "check", "--offline", "-p", package, "--tests", "--message-format=json"]
    if features:
        args += ["--features", features]
    out = subprocess.run(args, capture_output=True, text=True).stdout
    for line in out.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get("reason") == "compiler-message" and message["message"]["level"] == "error":
            yield message["message"]


def suggestions(message):
    for child in message.get("children", []):
        for span in child.get("spans", []):
            replacement = span.get("suggested_replacement") or ""
            match = re.match(r"use ([A-Za-z_][\w:]*);", replacement.strip())
            if match:
                yield match.group(1)


def primary_file(message):
    for span in message["spans"]:
        if span["is_primary"]:
            return span["file_name"]
    return None


def write_imports(path, from_super, full_paths):
    text = pathlib.Path(path).read_text()
    block = []
    if from_super:
        block.append("use super::{" + ", ".join(sorted(from_super)) + "};")
    block.extend(f"use {p};" for p in sorted(full_paths))
    marker = "// expand_glob_imports\n"
    if marker in text:
        start = text.index(marker)
        end = text.index(marker, start + len(marker)) + len(marker)
        text = text[:start] + text[end:]
    else:
        text = text.replace(GLOB, "", 1)
    lines = text.split("\n")
    insert = 0
    while insert < len(lines) and (lines[insert].startswith("//!") or lines[insert] == ""):
        insert += 1
    lines[insert:insert] = [marker.rstrip("\n")] + block + [marker.rstrip("\n")]
    pathlib.Path(path).write_text("\n".join(lines))


def finish(path):
    file = pathlib.Path(path)
    file.write_text(re.sub(r"// expand_glob_imports\n", "", file.read_text()))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", required=True)
    parser.add_argument("--features", default="")
    parser.add_argument("files", nargs="+")
    args = parser.parse_args()
    files = [f for f in args.files if GLOB in pathlib.Path(f).read_text()]
    names = {f: set() for f in files}
    full = {f: set() for f in files}
    for f in files:
        write_imports(f, names[f], full[f])
    for round_ in range(12):
        found = collections.defaultdict(set)
        found_full = collections.defaultdict(set)
        dropped = collections.defaultdict(set)
        other = 0
        for message in check(args.package, args.features):
            path = primary_file(message)
            key = next((f for f in files if path and path.endswith(f.split("crates/", 1)[-1])), None)
            if key is None:
                other += 1
                continue
            text = message["message"]
            bad = BAD_IMPORT.search(text) if "unresolved import" in text or "no `" in text else None
            if bad and "super::" in json.dumps(message["spans"]):
                name = bad.group(1) or bad.group(2)
                dropped[key].add(name)
                for suggestion in suggestions(message):
                    found_full[key].add(suggestion)
                continue
            match = UNRESOLVED.search(text)
            if match:
                found[key].add(next(g for g in match.groups() if g))
                continue
            for suggestion in suggestions(message):
                found_full[key].add(suggestion)
        changed = False
        for f in files:
            before = (set(names[f]), set(full[f]))
            names[f] |= found[f]
            names[f] -= dropped[f]
            full[f] |= found_full[f]
            full[f] = {p for p in full[f] if p.rsplit("::", 1)[-1] not in names[f]}
            if (names[f], full[f]) != before:
                write_imports(f, names[f], full[f])
                changed = True
        print(f"round {round_}: {sum(len(v) for v in found.values())} names, "
              f"{sum(len(v) for v in found_full.values())} paths, {other} errors elsewhere")
        if not changed:
            break
    for f in files:
        finish(f)
    return 0


if __name__ == "__main__":
    sys.exit(main())
