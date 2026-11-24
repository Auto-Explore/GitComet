#!/usr/bin/env python3
"""Hold structural counts at their recorded values, and report where change concentrates.

`check` compares the tree with `scorecard-baseline.txt`:

    <metric> <key> <value> <reason>

- `fn-lines`: every production function of 150 lines or more. A longer or new
  one fails; one that is gone or under the threshold needs its entry removed.
- every other metric is a count: more fails, fewer needs the count lowered.

`check --write` lowers the baseline to the tree, keeping recorded reasons; it
never raises a value or adds an entry, which is done by hand with a reason. `report`
adds numbers no single change controls (hub-file churn, co-change, CI
duration), so those never fail a check.
"""

import argparse
from collections import Counter
from itertools import combinations
import json
from pathlib import Path
import re
import statistics
import subprocess
import sys

import identity_literals

ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).with_name("scorecard-baseline.txt")
LONG_FUNCTION_LINES = 150
FN_HEADER = re.compile(
    r'^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:(?:const|async|unsafe|extern\s+"[^"]*")\s+)*fn\s+(\w+)', re.M)
SIGNATURE_TOKEN = re.compile(identity_literals.STRING.pattern + r"|[{};()\[\]]", re.S)
ITEM_TOKEN = re.compile(identity_literals.STRING.pattern + r"|->|[{}()\[\]<>,]", re.S)
UI = "gitcomet-ui-gpui"
STATE = "gitcomet-state"
# (metric, scope, pattern, crates or None for every crate)
COUNTS = (
    # `use super::super::*;` and a re-exporting `pub(crate) use super::*;` too.
    ("glob-super", "production",
     re.compile(r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?use (?:super::)+\*;", re.M), None),
    # A read of the clock: a call, the function passed as a value, or `elapsed`.
    ("clock-calls", "production", re.compile(r"\b(?:Instant|SystemTime)::now\b|\.elapsed\(\)"),
     (UI, STATE, "gitcomet-git-gix", "gitcomet-core")),
    ("root-view-updates", "production", re.compile(r"\broot_view\s*\.update\("), (UI,)),
    ("set-theme-fns", "production", re.compile(r"\bfn set_theme\("), (UI,)),
    ("notify-fingerprint-fns", "production", re.compile(r"\bfn \w*notify_fingerprint\w*\("), (UI,)),
    ("diff-renderer-refs", "tests", re.compile(r"\.diff_renderer\b"), (UI,)),
    ("store-snapshot-syncs", "tests", re.compile(r"\bsync_store_snapshot\("), (UI,)),
    ("real-time-waits", "tests", re.compile(r"\brecv_timeout\(|\bthread::sleep\("), (STATE,)),
)
# (metric, crate, item keyword, item name)
SHAPES = (
    ("enum-variants", STATE, "enum", "Msg"),
    ("enum-variants", STATE, "enum", "InternalMsg"),
    ("enum-variants", STATE, "enum", "Effect"),
    ("struct-fields", UI, "struct", "DiffRenderer"),
    ("struct-fields", UI, "struct", "GitCometView"),
)
# (crate, folder, its sub-folders from the lowest layer to the highest). A
# lower layer naming a higher one is counted; the other direction is free.
LAYERS = (
    (UI, "src/view", ("rows", "panes", "panels")),
)


def crate_of(path):
    parts = path.split("/")
    return parts[1] if len(parts) > 2 and parts[0] == "crates" else None


def matching_brace(code, open_at):
    """Offset just past the brace that closes the one at `open_at`."""
    depth = 0
    for token in identity_literals.BRACE_OR_STRING.finditer(code, open_at):
        depth += token.group() == "{"
        depth -= token.group() == "}"
        if not depth:
            return token.end()
    return len(code)


def function_lengths(code):
    """{name or name#n: lines} for each function with a body, in source order."""
    lengths, seen = {}, Counter()
    for header in FN_HEADER.finditer(code):
        depth, body = 0, None
        for token in SIGNATURE_TOKEN.finditer(code, header.end()):
            text = token.group()
            if text in "([":
                depth += 1
            elif text in ")]":
                depth -= 1
            elif depth == 0 and text == "{":
                body = token.start()
                break
            elif depth == 0 and text == ";":
                break
        if body is None:
            continue
        name = header.group(1)
        seen[name] += 1
        key = name if seen[name] == 1 else f"{name}#{seen[name]}"
        start = code.count("\n", 0, header.start(1))
        lengths[key] = code.count("\n", 0, matching_brace(code, body)) - start + 1
    return lengths


def item_members(code, keyword, name):
    """How many variants or fields `keyword name { ... }` declares, or None when absent."""
    item = re.search(rf"\b{keyword}\s+{name}\b[^;{{]*\{{", code)
    if not item:
        return None
    body = code[item.end():matching_brace(code, item.end() - 1) - 1]
    # A member is a non-empty segment between commas outside every bracket.
    members, depth, segment_start = 0, 0, 0
    for token in ITEM_TOKEN.finditer(body):
        text = token.group()
        if text in ("{", "(", "[", "<"):
            depth += 1
        elif text in ("}", ")", "]", ">"):
            depth -= 1
        elif text == "," and depth == 0:
            members += bool(body[segment_start:token.start()].strip())
            segment_start = token.end()
    return members + bool(body[segment_start:].strip())


def measure(texts, production):
    """{(metric, key): value} for the whole tree."""
    found = {}
    for path, code in production.items():
        for name, lines in function_lengths(code).items():
            if lines >= LONG_FUNCTION_LINES:
                found[("fn-lines", f"{path}::{name}")] = lines
    for metric, scope, pattern, crates in COUNTS:
        totals = Counter()
        for path, text in texts.items():
            crate = crate_of(path)
            if crate is None or (crates is not None and crate not in crates):
                continue
            shipped = len(pattern.findall(production[path])) if path in production else 0
            if scope == "production":
                totals[crate] += shipped
            else:
                totals[crate] += len(pattern.findall(identity_literals.strip_comments(text))) - shipped
        found.update({(metric, crate): count for crate, count in totals.items() if count})
    for metric, crate, keyword, name in SHAPES:
        sizes = [size for path, code in production.items() if crate_of(path) == crate
                 for size in [item_members(code, keyword, name)] if size is not None]
        if sizes:
            found[(metric, f"{crate}::{name}")] = max(sizes)
    for crate, folder, layers in LAYERS:
        for depth, lower in enumerate(layers):
            prefix = f"crates/{crate}/{folder}/{lower}/"
            codes = [code for path, code in production.items() if path.startswith(prefix)]
            for higher in layers[depth + 1:]:
                count = sum(len(re.findall(rf"\b{higher}::", code)) for code in codes)
                if count:
                    found[("upward-refs", f"{crate}::{lower}->{higher}")] = count
    return found


def read_baseline(path=BASELINE):
    entries = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split(None, 3)
        if len(fields) < 4 or not fields[2].isdigit():
            raise ValueError(f"{path.name}:{number}: expected '<metric> <key> <value> <reason>'")
        entries[(fields[0], fields[1])] = (int(fields[2]), fields[3])
    return entries


def compare(found, recorded):
    """Problems as text."""
    problems = []
    for key in sorted(set(found) | set(recorded)):
        metric, name = key
        have = found.get(key, 0)
        limit = recorded.get(key, (0, ""))[0]
        if metric == "fn-lines":
            if have > limit and limit:
                problems.append(f"{name} grew from {limit} to {have} lines; split it instead")
            elif have > limit:
                problems.append(f"{name} is a new function of {have} lines; keep functions under {LONG_FUNCTION_LINES}")
            elif not have:
                problems.append(f"{name} is gone or under {LONG_FUNCTION_LINES} lines; remove its fn-lines entry")
        elif have > limit:
            problems.append(f"{metric} {name}: {have} exceeds the recorded {limit}")
        elif have < limit:
            problems.append(f"{metric} {name}: recorded {limit} but {have} remain; lower the count")
    return problems


def write(found, recorded, path=BASELINE):
    """Lowers recorded values to what was found and drops entries that are gone.

    A value above its entry and a new entry are left for `compare` to report: a
    line is added or raised by hand, with its reason. With no baseline yet,
    everything found is recorded.
    """
    first = not path.exists()
    lines = [] if first else [line for line in path.read_text(encoding="utf-8").splitlines() if line.startswith("#")]
    for key, value in sorted(found.items()):
        limit, reason = recorded.get(key, (value, "-"))
        if first or key in recorded:
            lines.append(f"{key[0]} {key[1]} {min(value, limit)} {reason}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def git_lines(*args):
    return subprocess.run(["git", *args], cwd=ROOT, stdout=subprocess.PIPE, check=True, text=True).stdout.splitlines()


def commit_files(since):
    """One set of changed Rust files per commit that touched crates/."""
    commits, current = [], None
    for line in git_lines("log", f"--since={since}", "--name-only", "--format=@%H", "--", "crates"):
        if line.startswith("@"):
            current = set()
            commits.append(current)
        elif line.endswith(".rs"):
            current.add(line)
    return commits


def churn(commits, hubs=13, pairs=10):
    def busiest(counter, limit):
        return sorted(counter.items(), key=lambda item: (-item[1], item[0]))[:limit]

    touches = Counter(path for files in commits for path in files)
    hub_files = [path for path, _ in busiest(touches, hubs)]
    hub_set = set(hub_files)
    together = Counter(pair for files in commits for pair in combinations(sorted(files & hub_set), 2))
    crate_counts = [len({crate_of(path) for path in files} - {None}) for files in commits]
    return {
        "commits": len(commits),
        "hub_files": [{"path": path, "commits": touches[path]} for path in hub_files],
        "commits_touching_a_hub": sum(1 for files in commits if files & hub_set),
        "co_changes": [{"files": list(pair), "commits": count} for pair, count in busiest(together, pairs)],
        "commits_touching_four_or_more_crates": sum(1 for count in crate_counts if count >= 4),
    }


def ci_runs(limit):
    from report import api, elapsed
    runs = api(f"repos/{{owner}}/{{repo}}/actions/workflows/ci.yml/runs?per_page={limit}")["workflow_runs"]
    return [{"conclusion": run["conclusion"], "seconds": elapsed(run["created_at"], run["updated_at"])}
            for run in runs]


def ci_summary(runs):
    succeeded = sorted(run["seconds"] for run in runs if run["conclusion"] == "success")
    failed = sum(1 for run in runs if run["conclusion"] == "failure")
    return {
        "runs": len(runs),
        "succeeded": len(succeeded),
        "failed": failed,
        "median_success_minutes": round(statistics.median(succeeded) / 60, 1) if succeeded else None,
    }


def totals(found):
    summary = Counter()
    for (metric, name), value in found.items():
        if metric == "fn-lines":
            summary["functions of 150+ lines"] += 1
        else:
            summary[f"{metric} {name}"] += value
    return dict(sorted(summary.items()))


def markdown(report):
    lines = ["# Scorecard", "", "| Count | Value |", "| --- | ---: |"]
    lines += [f"| {name} | {value} |" for name, value in report["counts"].items()]
    history = report["churn"]
    share = 100 * history["commits_touching_a_hub"] // max(history["commits"], 1)
    lines += ["", f"{history['commits_touching_a_hub']} of {history['commits']} commits ({share}%) touch a hub file; "
              f"{history['commits_touching_four_or_more_crates']} touch four or more crates.", "",
              "| Hub file | Commits |", "| --- | ---: |"]
    lines += [f"| {hub['path']} | {hub['commits']} |" for hub in history["hub_files"]]
    lines += ["", "| Changed together | Commits |", "| --- | ---: |"]
    lines += [f"| {' + '.join('/'.join(Path(path).parts[-2:]) for path in pair['files'])} | {pair['commits']} |"
              for pair in history["co_changes"]]
    if ci := report.get("ci"):
        lines += ["", f"CI: {ci['succeeded']} of {ci['runs']} runs succeeded, {ci['failed']} failed; "
                  f"median successful run {ci['median_success_minutes']} minutes."]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("check", help="compare the tree with the baseline")
    check.add_argument("--write", action="store_true", help="rewrite the baseline values")
    report = commands.add_parser("report", help="print counts, churn and CI numbers")
    report.add_argument("--since", default="6 months ago", help="git history window")
    report.add_argument("--ci-runs", type=int, default=0, help="include this many recent CI runs (needs gh)")
    report.add_argument("--json", action="store_true", help="print JSON instead of Markdown")
    args = parser.parse_args()

    texts = identity_literals.rust_sources()
    found = measure(texts, identity_literals.production_code(texts))
    if args.command == "report":
        result = {"counts": totals(found), "churn": churn(commit_files(args.since))}
        if args.ci_runs:
            result["ci"] = ci_summary(ci_runs(args.ci_runs))
        print(json.dumps(result, indent=2) if args.json else markdown(result), end="")
        return 0
    recorded = read_baseline() if BASELINE.exists() else {}
    if args.write:
        write(found, recorded)
        recorded = read_baseline()
    problems = compare(found, recorded)
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        print("Fix the code, or run `python3 scripts/ci/scorecard.py check --write` when a count went down.",
              file=sys.stderr)
    else:
        print(f"Scorecard check passed ({len(found)} recorded values).")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
