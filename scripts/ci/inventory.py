#!/usr/bin/env python3
"""Snapshot Rust test inventories and prove that a test move kept every test.

snapshot OUT [cargo test selection...]
    Build the selected test harnesses and record each harness's tests with
    their ignored flag: {"<target kind>:<target name>": {"<test path>": ignored}}.
compare OLD NEW [--mapping OUT]
    Map every OLD test to exactly one NEW test in the same harness. A test may
    move into a child module (its old module path is a prefix of the new one and
    its leaf name is unchanged) but may not change harness, name, or ignored
    state. Fails on dropped, added, re-ignored, or ambiguous tests.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys


def package_name(package_id):
    # `path+file:///x/crates/name#0.1.0` or `registry+...#name@0.1.0`.
    location, _, fragment = package_id.partition("#")
    return fragment.split("@")[0] if "@" in fragment else location.rstrip("/").rsplit("/", 1)[-1]


def harnesses(selection):
    command = ["cargo", "test", "--no-run", "--message-format", "json", *selection]
    output = subprocess.run(command, stdout=subprocess.PIPE, check=True, text=True).stdout
    found = {}
    for line in output.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        if not message.get("profile", {}).get("test"):
            continue
        target = message["target"]
        package = package_name(message["package_id"])
        key = f"{package}:{target['kind'][0]}:{target['name']}"
        found[key] = message["executable"]
    return found


def list_tests(executable, ignored):
    command = [executable, "--list", "--format", "terse"] + (["--ignored"] if ignored else [])
    output = subprocess.run(command, stdout=subprocess.PIPE, check=True, text=True).stdout
    return [line.rsplit(": ", 1)[0] for line in output.splitlines() if line.endswith(": test")]


def snapshot(out, selection):
    inventory = {}
    for key, executable in sorted(harnesses(selection).items()):
        ignored = set(list_tests(executable, True))
        inventory[key] = {name: name in ignored for name in list_tests(executable, False)}
    Path(out).write_text(json.dumps(inventory, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    total = sum(len(tests) for tests in inventory.values())
    print(f"{out}: {total} tests in {len(inventory)} harnesses")


def moved_to(old, new):
    old_parts, new_parts = old.split("::"), new.split("::")
    if old_parts[-1] != new_parts[-1] or len(new_parts) < len(old_parts):
        return False
    # The old module path must survive as a prefix; the move may only insert
    # child modules between it and the leaf.
    return new_parts[:len(old_parts) - 1] == old_parts[:-1]


def compare(old_path, new_path, mapping_out=None):
    old = json.loads(Path(old_path).read_text(encoding="utf-8"))
    new = json.loads(Path(new_path).read_text(encoding="utf-8"))
    errors = []
    mapping = {}
    for harness in sorted(set(old) | set(new)):
        if harness not in old or harness not in new:
            errors.append(f"{harness}: harness {'added' if harness not in old else 'removed'}")
            continue
        before, after = old[harness], new[harness]
        unchanged = set(before) & set(after)
        remaining_new = set(after) - unchanged
        for name in sorted(unchanged):
            mapping.setdefault(harness, {})[name] = name
        for name in sorted(set(before) - unchanged):
            candidates = [candidate for candidate in remaining_new if moved_to(name, candidate)]
            if len(candidates) != 1:
                errors.append(f"{harness}: {name}: {len(candidates)} candidates {sorted(candidates)[:3]}")
                continue
            remaining_new.discard(candidates[0])
            mapping.setdefault(harness, {})[name] = candidates[0]
        for name in sorted(remaining_new):
            errors.append(f"{harness}: {name}: new test without an old counterpart")
        for name, target in mapping.get(harness, {}).items():
            if before[name] != after[target]:
                errors.append(f"{harness}: {name}: ignored {before[name]} -> {after[target]}")
    if mapping_out:
        Path(mapping_out).write_text(json.dumps(mapping, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    moved = sum(1 for tests in mapping.values() for old_name, new_name in tests.items() if old_name != new_name)
    total = sum(len(tests) for tests in mapping.values())
    for error in errors:
        print(error, file=sys.stderr)
    print(f"{total} tests mapped, {moved} moved, {len(errors)} problems")
    return 1 if errors else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    snap = sub.add_parser("snapshot")
    snap.add_argument("out")
    snap.add_argument("selection", nargs=argparse.REMAINDER)
    cmp = sub.add_parser("compare")
    cmp.add_argument("old")
    cmp.add_argument("new")
    cmp.add_argument("--mapping")
    args = parser.parse_args()
    if args.command == "snapshot":
        selection = args.selection[1:] if args.selection[:1] == ["--"] else args.selection
        snapshot(args.out, selection)
        return 0
    return compare(args.old, args.new, args.mapping)


if __name__ == "__main__":
    sys.exit(main())
