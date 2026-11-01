#!/usr/bin/env python3
"""Prove the crate layering with Cargo's resolved dependency graph.

Each rule names a package, a feature selection, and packages (names or glob
patterns) that must not be reachable through normal or build dependencies on
any target. Development dependencies are exempt: a test may use GPUI's test
support. A rule whose package is not in the workspace yet is reported and
skipped, so the check can land before the crate it guards.
"""

import json
import subprocess
import sys
from fnmatch import fnmatchcase

GPUI = ("gpui-ce", "gpui_ce_platform")
UI_HOST = ("gitcomet-ui-gpui",)
APPLICATION = ("gitcomet", "gitcomet-app")

RULES = (
    ("gitcomet-diagrams", (), ("gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    # Shared foundations: no workspace crate below or beside them.
    ("gitcomet-base", (), ("gitcomet-core", "gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    # Text foundations: GPUI's standalone string and tree crates, never GPUI.
    ("gitcomet-text", (), ("gitcomet-base", "gitcomet-core", "gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    # Syntax highlighting: tree-sitter and its grammars, never GPUI or the kit.
    ("gitcomet-syntax", (), ("gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    ("gitcomet-difftastic", (), ("gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    # Vendored upstream source: GitComet adapts it from the outside.
    ("difftastic", (), ("gitcomet", "gitcomet-*")),
    # The conflict resolver's model: no state, GPUI or UI kit.
    ("gitcomet-conflict-resolver", (), ("gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    # The markdown preview model: parsing, flattening and wrapping, never GPUI or the kit.
    ("gitcomet-markdown", (), ("gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    # Measurement records and their report: serde only.
    ("gitcomet-perf", (), ("gitcomet-core", "gitcomet-state", *GPUI, "gitcomet-ui-kit", *UI_HOST, *APPLICATION)),
    ("gitcomet-core", (), ("gitcomet-state", *GPUI, *UI_HOST, *APPLICATION)),
    ("gitcomet-state", (), (*GPUI, "gitcomet-ui-kit", "gitcomet-extension-api", *UI_HOST, *APPLICATION)),
    ("gitcomet-ui-kit", (), ("gitcomet-core", "gitcomet-state", "gitcomet-extension-api", *UI_HOST, *APPLICATION)),
    # The embedded terminal widget builds on the kit, like any downstream component.
    ("gitcomet-ui-terminal", (), ("gitcomet-core", "gitcomet-state", "gitcomet-extension-api", *UI_HOST, *APPLICATION)),
    # Image and diagram previews: the kit and the diagram renderer, never Git or state.
    ("gitcomet-ui-media", (), ("gitcomet-core", "gitcomet-state", "gitcomet-extension-api", *UI_HOST, *APPLICATION)),
    ("gitcomet-extension-api", (), (*UI_HOST, *APPLICATION)),
    # A downstream product library reaches the host only through the API.
    ("gitcomet-extension-example", (), (*UI_HOST, *APPLICATION)),
    # The headless application: no windowing stack at all.
    ("gitcomet", ("--no-default-features", "--features", "gix"), (*GPUI, "gitcomet-ui-kit", *UI_HOST)),
    ("gitcomet-app", ("--no-default-features",), (*GPUI, "gitcomet-ui-kit", *UI_HOST)),
)


def workspace_packages():
    output = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
                            stdout=subprocess.PIPE, check=True, text=True).stdout
    return {package["name"] for package in json.loads(output)["packages"]}


def reachable(package, features):
    command = ["cargo", "tree", "--locked", "-p", package, *features, "--target", "all",
               "--edges", "normal,build", "--prefix", "none", "--format", "{p}"]
    output = subprocess.run(command, stdout=subprocess.PIPE, check=True, text=True).stdout
    return parse_tree(output)


def parse_tree(output):
    # `{p}` prints `name v1.2.3 (source)`; a repeated subtree ends in `(*)`.
    return {line.split()[0] for line in output.splitlines() if line.strip()}


def violations(package, reached, forbidden):
    return sorted(name for name in reached
                  if name != package and any(fnmatchcase(name, pattern) for pattern in forbidden))


def main():
    present = workspace_packages()
    failed = False
    for package, features, forbidden in RULES:
        if package not in present:
            print(f"skip {package}: not in the workspace")
            continue
        found = violations(package, reachable(package, features), forbidden)
        selection = " ".join(features) or "default features"
        if found:
            failed = True
            print(f"::error::{package} ({selection}) reaches forbidden packages: {', '.join(found)}", file=sys.stderr)
        else:
            print(f"ok   {package} ({selection})")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
