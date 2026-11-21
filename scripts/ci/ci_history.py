#!/usr/bin/env python3
"""Read what CI runs already upload: which tests fail, and where compile time goes.

    ci_history.py download --runs 30 --dir target/ci-history
    ci_history.py flakes target/ci-history
    ci_history.py timings target/ci-history

`download` fetches the `ci-*` diagnostics artifacts (kept for seven days) of
recent runs with `gh`. `flakes` lists tests that failed, from the JUnit reports
and the libtest logs, marking those that also passed on the same lane at the
same commit. `timings` shows a package's compile units per lane from Cargo's
timing reports.
"""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import statistics
import subprocess
import sys
import xml.etree.ElementTree as ElementTree

UNIT_DATA = re.compile(r"const UNIT_DATA = (\[.*?\]);\s*\n", re.S)
COMMIT_FILE = "commit.txt"
LIBTEST_RESULT = re.compile(r"^test (\S+) \.\.\. (ok|FAILED)$", re.M)


def junit_outcomes(xml_text):
    """(suite, test, failed) for every executed test case."""
    outcomes = []
    for case in ElementTree.fromstring(xml_text).iter("testcase"):
        if case.find("skipped") is not None:
            continue
        failed = case.find("failure") is not None or case.find("error") is not None
        outcomes.append((case.get("classname", ""), case.get("name", ""), failed))
    return outcomes


def libtest_outcomes(log_text, suite):
    """(suite, test, failed) for every result line of a libtest log."""
    return [(suite, test, result == "FAILED") for test, result in LIBTEST_RESULT.findall(log_text)]


def flaky_tests(runs):
    """`runs` is [(commit, lane, outcomes)]. Tests with a failure, most failures first.

    A test is flaky when one commit both passed and failed it on a lane; a
    test that fails on one commit and passes on another was broken and fixed.
    """
    by_commit = defaultdict(lambda: defaultdict(lambda: [0, 0]))
    for commit, lane, outcomes in runs:
        for suite, test, failed in outcomes:
            by_commit[(lane, suite, test)][commit][failed] += 1
    found = []
    for (lane, suite, test), commits in by_commit.items():
        failed = sum(counts[1] for counts in commits.values())
        if failed:
            found.append({"lane": lane, "suite": suite, "test": test, "failed": failed,
                          "passed": sum(counts[0] for counts in commits.values()),
                          "flaky": any(passed and failed for passed, failed in commits.values())})
    return sorted(found, key=lambda item: (-item["failed"], item["lane"], item["suite"], item["test"]))


def timing_units(html, package):
    """Compile units of `package` in one Cargo timing report."""
    match = UNIT_DATA.search(html)
    if not match:
        return []
    units = []
    for unit in json.loads(match.group(1)):
        if unit["name"] != package:
            continue
        sections = dict(unit.get("sections") or [])
        frontend = sections.get("frontend")
        units.append({
            "target": unit["target"].strip() or "lib",
            "seconds": unit["duration"],
            "frontend_seconds": frontend["end"] - frontend["start"] if frontend else None,
        })
    return units


def lane_of(path, root):
    """`<root>/<run>/<lane>/...` names the lane."""
    parts = path.relative_to(root).parts
    return parts[1] if len(parts) > 2 else parts[0]


def download(args):
    runs = json.loads(subprocess.check_output(
        ["gh", "run", "list", "--workflow", args.workflow, "--limit", str(args.runs),
         "--status", "completed", "--json", "databaseId,headSha"], text=True))
    root = Path(args.dir)
    for run in runs:
        target = root / str(run["databaseId"])
        if not target.exists():
            result = subprocess.run(["gh", "run", "download", str(run["databaseId"]), "-p", "ci-*", "-D", str(target)])
            if result.returncode:
                print(f"run {run['databaseId']}: no artifacts left", file=sys.stderr)
        if target.exists():
            (target / COMMIT_FILE).write_text(run["headSha"] + "\n", encoding="utf-8")
    return 0


def test_reports(root):
    """[(commit, lane, outcomes)] for every JUnit report and libtest log under `root`."""
    reports = []
    for run in sorted(path for path in root.iterdir() if path.is_dir()):
        commit_file = run / COMMIT_FILE
        # Without the commit, every run counts as its own.
        commit = commit_file.read_text(encoding="utf-8").strip() if commit_file.exists() else run.name
        for path in sorted(run.rglob("junit.xml")):
            reports.append((commit, lane_of(path, root), junit_outcomes(path.read_text(encoding="utf-8"))))
        for path in sorted(run.rglob("*.log")):
            # nextest logs repeat the libtest output of tests its JUnit report already counts.
            if "nextest" in path.name:
                continue
            outcomes = libtest_outcomes(path.read_text(encoding="utf-8", errors="replace"), path.stem)
            if outcomes:
                reports.append((commit, lane_of(path, root), outcomes))
    return reports


def flakes(args):
    runs = test_reports(Path(args.dir))
    found = flaky_tests(runs)
    print(f"{len(runs)} test reports, {len(found)} tests with a failure")
    for item in found[:args.limit]:
        kind = "flaky " if item["flaky"] else "failed"
        print(f"{kind} {item['failed']:>3} of {item['failed'] + item['passed']:<3} {item['lane']}  "
              f"{item['suite']}  {item['test']}")
    return 0


def timings(args):
    root = Path(args.dir)
    samples = defaultdict(list)
    for path in sorted(root.rglob("cargo-timing-*.html")):
        for unit in timing_units(path.read_text(encoding="utf-8"), args.package):
            samples[(lane_of(path, root), unit["target"])].append(unit)
    for (lane, target), units in sorted(samples.items()):
        frontends = [unit["frontend_seconds"] for unit in units if unit["frontend_seconds"] is not None]
        frontend = f", frontend {statistics.median(frontends):.0f}s" if frontends else ""
        print(f"{lane:<28} {target:<44} median {statistics.median(u['seconds'] for u in units):.0f}s"
              f"{frontend} ({len(units)} samples)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    fetch = commands.add_parser("download")
    fetch.add_argument("--runs", type=int, default=30)
    fetch.add_argument("--dir", required=True)
    fetch.add_argument("--workflow", default="ci.yml")
    fetch.set_defaults(run=download)
    flaky = commands.add_parser("flakes")
    flaky.add_argument("dir")
    flaky.add_argument("--limit", type=int, default=50)
    flaky.set_defaults(run=flakes)
    timing = commands.add_parser("timings")
    timing.add_argument("dir")
    timing.add_argument("--package", default="gitcomet-ui-gpui")
    timing.set_defaults(run=timings)
    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
