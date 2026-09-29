#!/usr/bin/env python3
"""Snapshot Rust test inventories and prove that a test move kept every test.

snapshot OUT [cargo test selection...]
    Build the selected test harnesses and record each harness's tests with
    their ignored flag: {"<target kind>:<target name>": {"<test path>": ignored}}.
compare OLD NEW [--mapping OUT] [--harness OLD=NEW ...] [--split OLD=NEW ...] [--replaced FILE]
    Map every OLD test to exactly one NEW test in the same harness. A test may
    move into a child module (its old module path is a prefix of the new one and
    its leaf name is unchanged) but may not change harness, name, or ignored
    state. `--harness` declares that one harness's tests moved to another (a
    crate split); `--replaced` names tests deliberately removed or added, as a
    JSON object {"removed": {"harness test": "why"}, "added": {...}}. `--split`
    declares that some of OLD's tests moved to NEW (a module moved to another
    crate): tests left unmatched on both sides then pair by leaf name when the
    leaf is unique among them. Fails on any other dropped, added, re-ignored, or
    ambiguous test.
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
    # Every old module must survive in order; a move may only insert modules
    # (a child group, or a parent when a crate root becomes a module).
    remaining = iter(new_parts[:-1])
    return all(part in remaining for part in old_parts[:-1])


def compare(old_path, new_path, mapping_out=None, harness_moves=(), replaced_path=None, splits=()):
    old = json.loads(Path(old_path).read_text(encoding="utf-8"))
    new = json.loads(Path(new_path).read_text(encoding="utf-8"))
    replaced = json.loads(Path(replaced_path).read_text(encoding="utf-8")) if replaced_path else {}
    removed = {tuple(key.split(" ", 1)) for key in replaced.get("removed", {})}
    added = {tuple(key.split(" ", 1)) for key in replaced.get("added", {})}
    # Fold a moved harness into its destination under the destination's name;
    # an emptied source harness may disappear.
    for move in harness_moves:
        source, destination = move.split("=", 1)
        if source not in old:
            raise SystemExit(f"--harness {move}: {source} is not in the old inventory")
        moved = old.pop(source)
        overlap = set(moved) & set(old.get(destination, {}))
        if overlap:
            raise SystemExit(f"--harness {move}: {len(overlap)} names already in {destination}")
        old.setdefault(destination, {}).update(moved)
        new.setdefault(source, {})
        old.setdefault(source, {})
    for harness, name in removed:
        if old.get(harness, {}).pop(name, None) is None:
            raise SystemExit(f"--replaced: {harness} {name} is not in the old inventory")
    for harness, name in added:
        if new.get(harness, {}).pop(name, None) is None:
            raise SystemExit(f"--replaced: {harness} {name} is not in the new inventory")
    errors = []
    mapping = {}
    leftover_old, leftover_new = {}, {}
    for harness in sorted(set(old) | set(new)):
        if harness not in old:
            leftover_new.update({(harness, name): "in an added harness" for name in new[harness]})
            continue
        if harness not in new:
            leftover_old.update({(harness, name): "in a removed harness" for name in old[harness]})
            continue
        before, after = old[harness], new[harness]
        unchanged = set(before) & set(after)
        remaining_new = set(after) - unchanged
        for name in sorted(unchanged):
            mapping.setdefault(harness, {})[name] = name
        for name in sorted(set(before) - unchanged):
            candidates = [candidate for candidate in remaining_new if moved_to(name, candidate)]
            if len(candidates) != 1:
                leftover_old[(harness, name)] = f"{len(candidates)} candidates {sorted(candidates)[:3]}"
                continue
            remaining_new.discard(candidates[0])
            mapping.setdefault(harness, {})[name] = candidates[0]
        for name in sorted(remaining_new):
            leftover_new[(harness, name)] = "new test without an old counterpart"
        for name, target in mapping.get(harness, {}).items():
            if before[name] != after[target]:
                errors.append(f"{harness}: {name}: ignored {before[name]} -> {after[target]}")
    # Declared splits: leftovers pair across (or within) the named harnesses by
    # a leaf name that is unique on both sides.
    for split in splits:
        source, destination = split.split("=", 1)
        harnesses = {source, destination}
        leaf = lambda name: name.rsplit("::", 1)[-1]
        olds = [key for key in leftover_old if key[0] == source]
        news = [key for key in leftover_new if key[0] in harnesses]
        old_leaves = {}
        for key in olds:
            old_leaves.setdefault(leaf(key[1]), []).append(key)
        new_leaves = {}
        for key in news:
            new_leaves.setdefault(leaf(key[1]), []).append(key)
        for name, keys in old_leaves.items():
            targets = new_leaves.get(name, [])
            if len(keys) != 1 or len(targets) != 1:
                continue
            (old_key,), (new_key,) = keys, targets
            del leftover_old[old_key], leftover_new[new_key]
            mapping.setdefault(old_key[0], {})[old_key[1]] = f"{new_key[0]} {new_key[1]}"
            if old[old_key[0]][old_key[1]] != new[new_key[0]][new_key[1]]:
                errors.append(f"{old_key[0]}: {old_key[1]}: ignored state changed in the split")
    errors.extend(f"{harness}: {name}: {why}" for (harness, name), why in sorted(leftover_old.items()))
    errors.extend(f"{harness}: {name}: {why}" for (harness, name), why in sorted(leftover_new.items()))
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
    cmp.add_argument("--harness", action="append", default=[], metavar="OLD=NEW")
    cmp.add_argument("--replaced")
    cmp.add_argument("--split", action="append", default=[], metavar="OLD=NEW")
    args = parser.parse_args()
    if args.command == "snapshot":
        selection = args.selection[1:] if args.selection[:1] == ["--"] else args.selection
        snapshot(args.out, selection)
        return 0
    return compare(args.old, args.new, args.mapping, args.harness, args.replaced, args.split)


if __name__ == "__main__":
    sys.exit(main())
