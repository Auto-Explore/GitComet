#!/usr/bin/env python3
"""Keep the refactor round 2 structure counts moving one way.

Each metric counts something in the checked-in sources that the round drives
down (or, for test processes, up). `ratchets.txt` records the accepted value:

    <metric> <max|min> <value>

A `max` metric may not rise above its value and a `min` metric may not fall
below it. A value that moved the right way is reported too: run `--write` to
record it, so the improvement cannot regress later. The metrics and their
targets are described in docs/refactor-round-2.md.
"""

import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).with_name("ratchets.txt")
UI = "crates/gitcomet-ui-gpui/src/"
KIT = "crates/gitcomet-ui-kit/src/"
STATE = "crates/gitcomet-state/src/"
CI_RUNNER = "scripts/ci/run.py"
FIELD = re.compile(r"^\s+(?:pub(?:\([^)]*\))?\s+)?[a-z_][a-z0-9_]*\s*:", re.M)


def is_test_path(path):
    """The classification of scripts/check-rust-source-size.sh."""
    parts = path.split("/")
    name = parts[-1]
    return ("tests" in parts or "benches" in parts or "benchmarks" in parts or name == "tests.rs"
            or name.endswith("_tests.rs"))


def is_production(path):
    parts = path.split("/")
    return not is_test_path(path) and "smoke_tests" not in parts


def lines(text):
    return text.count("\n")


def gitcomet_view_fields(texts):
    for path, text in texts.items():
        match = re.search(r"^pub struct GitCometView \{\n(.*?)^\}", text, re.M | re.S)
        if path.startswith(UI) and match:
            return len(FIELD.findall(match.group(1)))
    return 0


def gpui_test_packages(texts):
    match = re.search(r"^GPUI_PACKAGES\s*=\s*\((.*?)\)", texts.get(CI_RUNNER, ""), re.M | re.S)
    return len([item for item in match.group(1).split(",") if item.strip()]) if match else 0


def ui_production(texts):
    return {path: text for path, text in texts.items() if path.startswith(UI) and is_production(path)}


METRICS = {
    "ui_gpui_production_lines": ("max", lambda t: sum(lines(text) for text in ui_production(t).values())),
    "gitcomet_view_fields": ("max", gitcomet_view_fields),
    "super_glob_imports_in_ui_production": (
        "max", lambda t: sum(1 for text in ui_production(t).values() if re.search(r"^use super::\*;", text, re.M))),
    "impl_gitcomet_view_files_outside_root": (
        "max", lambda t: sum(1 for path, text in t.items() if path.startswith(UI)
                             and re.search(r"^impl GitCometView\b", text, re.M)
                             and not re.search(r"/view/gitcomet_view[^/]*\.rs$", path))),
    "set_theme_calls_in_ui": ("max", lambda t: sum(text.count(".set_theme(") for text in ui_production(t).values())),
    "kit_gitcomet_core_refs": (
        "max", lambda t: sum(len(re.findall(r"\bgitcomet_core\b", text)) for path, text in t.items()
                             if path.startswith(KIT) and is_production(path))),
    "production_files_over_3000_lines": (
        "max", lambda t: sum(1 for path, text in t.items() if path.endswith(".rs") and not is_test_path(path)
                             and lines(text) > 3000)),
    "state_repo_iter_mut_find_sites": (
        "max", lambda t: sum(len(re.findall(r"\.iter_mut\(\)\s*\.find\(", text)) for path, text in t.items()
                             if path.startswith(STATE) and is_production(path))),
    "gix_integration_test_binaries": (
        "max", lambda t: sum(1 for path in t
                             if re.fullmatch(r"crates/gitcomet-git-gix/tests/(?:[^/]+|[^/]+/main)\.rs", path))),
    "gpui_test_packages": ("min", gpui_test_packages),
}


def tracked_sources():
    output = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--",
                             "crates/**/*.rs", CI_RUNNER], cwd=ROOT, stdout=subprocess.PIPE, check=True).stdout
    return sorted({path for path in output.decode().split("\0") if path and (ROOT / path).is_file()})


def read_texts(root, files):
    return {path: (root / path).read_text(encoding="utf-8") for path in files}


def measure(texts):
    return {name: compute(texts) for name, (_, compute) in METRICS.items()}


def read_baseline(path=BASELINE):
    entries = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split()
        if len(fields) != 3 or fields[1] not in ("max", "min") or not fields[2].isdigit():
            raise ValueError(f"{path.name}:{number}: expected '<metric> <max|min> <value>'")
        entries[fields[0]] = (fields[1], int(fields[2]))
    return entries


def compare(found, baseline):
    problems = []
    for name in METRICS:
        direction = METRICS[name][0]
        have = found[name]
        if name not in baseline:
            problems.append(f"{name}: no recorded value; run ratchets.py --write")
            continue
        recorded_direction, limit = baseline[name]
        if recorded_direction != direction:
            problems.append(f"{name}: recorded as {recorded_direction}, metric is {direction}")
        elif (have > limit) if direction == "max" else (have < limit):
            problems.append(f"{name}: {have} regressed past the recorded {limit}")
        elif have != limit:
            problems.append(f"{name}: {have} improved on the recorded {limit}; run ratchets.py --write to keep it")
    for name in baseline:
        if name not in METRICS:
            problems.append(f"{name}: recorded but no longer measured; remove the line")
    return problems


def write(found, path=BASELINE):
    header = [line for line in path.read_text(encoding="utf-8").splitlines() if line.startswith("#")] \
        if path.exists() else ["# Refactor round 2 ratchets: <metric> <max|min> <value>. See docs/refactor-round-2.md."]
    body = [f"{name} {METRICS[name][0]} {found[name]}" for name in METRICS]
    path.write_text("\n".join(header + body) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--write", action="store_true", help="record the current values")
    parser.add_argument("--show", action="store_true", help="print the current values")
    args = parser.parse_args()
    found = measure(read_texts(ROOT, tracked_sources()))
    if args.show:
        for name, value in found.items():
            print(f"{name} {METRICS[name][0]} {value}")
        return 0
    if args.write:
        write(found)
        return 0
    problems = compare(found, read_baseline() if BASELINE.exists() else {})
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if not problems:
        print(f"Ratchet check passed ({len(found)} metrics).")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
