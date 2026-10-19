#!/usr/bin/env python3
"""Fail when two linked packages define the same tree-sitter language function.

Grammar crates declare no `links` key, so Cargo builds both copies of a grammar
and the linker keeps whichever definition it meets first: the app then parses
one language with another crate's grammar, and nothing fails. This scans each
generated `parser.c` across the packages the application links, and fails on a
generated parser it cannot read a definition from rather than passing it.
"""

import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

APPLICATION = "gitcomet"
GENERATED_PARSER = "tree_sitter/parser.h"
LANGUAGE_FUNCTION = re.compile(r"TSLanguage\s*\*\s*(tree_sitter_\w+)\s*\(\s*(?:void)?\s*\)\s*\{")


def metadata():
    output = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked", "--all-features"],
                            stdout=subprocess.PIPE, check=True, text=True).stdout
    return json.loads(output)


def linked_packages(meta, root_name):
    packages = {package["id"]: package for package in meta["packages"]}
    nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
    root = next(package["id"] for package in meta["packages"]
                if package["name"] == root_name and package["id"] in meta["workspace_members"])
    pending, seen = [root], set()
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind["kind"] is None for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    return [packages[package_id] for package_id in sorted(seen)]


def language_functions(package_dir):
    found, unreadable = set(), []
    for parser in sorted(package_dir.rglob("parser.c")):
        if "target" in parser.relative_to(package_dir).parts:
            continue
        text = parser.read_text(encoding="utf-8", errors="replace")
        functions = LANGUAGE_FUNCTION.findall(text)
        if not functions and GENERATED_PARSER in text:
            unreadable.append(parser)
        found.update(functions)
    return found, unreadable


def describe(package):
    origin = package["source"] or str(Path(package["manifest_path"]).parent)
    return f"{package['name']} {package['version']} ({origin})"


def clashes(functions_by_package):
    owners = defaultdict(list)
    for package, functions in functions_by_package.items():
        for function in functions:
            owners[function].append(package)
    return {function: sorted(packages) for function, packages in owners.items() if len(packages) > 1}


def main():
    functions_by_package, unreadable = {}, []
    for package in linked_packages(metadata(), APPLICATION):
        functions, missed = language_functions(Path(package["manifest_path"]).parent)
        if functions:
            functions_by_package[describe(package)] = functions
        unreadable.extend((describe(package), parser) for parser in missed)
    for package, parser in unreadable:
        print(f"::error::{package}: no tree-sitter language function found in {parser}", file=sys.stderr)
    found = clashes(functions_by_package)
    for function, packages in sorted(found.items()):
        print(f"::error::{function} is defined by {', '.join(packages)}", file=sys.stderr)
    if found or unreadable:
        return 1
    count = sum(len(functions) for functions in functions_by_package.values())
    print(f"ok   {count} language functions in {len(functions_by_package)} linked packages, none defined twice")
    return 0


if __name__ == "__main__":
    sys.exit(main())
