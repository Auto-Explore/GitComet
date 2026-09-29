#!/usr/bin/env python3
"""Prove the crate layering with Cargo's resolved dependency graph.

Each rule names a package, a feature selection, and packages that must not be
reachable through normal or build dependencies on any target. Development
dependencies are exempt: a test may use GPUI's test support. A rule whose
package is not in the workspace yet is reported and skipped, so the check can
land before the crate it guards.
"""

import json
import subprocess
import sys

GPUI = ("gpui-ce", "gpui_ce_platform")
UI_HOST = ("gitcomet-ui-gpui",)
APPLICATION = ("gitcomet", "gitcomet-app")

RULES = (
    ("gitcomet-core", (), ("gitcomet-state", *GPUI, *UI_HOST, *APPLICATION)),
    ("gitcomet-state", (), (*GPUI, "gitcomet-ui-kit", "gitcomet-extension-api", *UI_HOST, *APPLICATION)),
    ("gitcomet-ui-kit", (), ("gitcomet-state", "gitcomet-extension-api", *UI_HOST, *APPLICATION)),
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
    return sorted(name for name in reached if name in forbidden and name != package)


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
