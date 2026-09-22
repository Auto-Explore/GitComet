#!/usr/bin/env python3
"""Measure frozen Windows GUI binaries in alternating pairs, or summarize a capture."""

import argparse
import bisect
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
SCENARIOS = ("idle", "move", "resize", "native-move", "native-resize", "hover", "scroll", "click", "typing", "commit-typing", "file-search")


def distribution(values):
    values = sorted(values)
    if not values:
        return {"count": 0, "mean": None, "p50": None, "p95": None, "max": None}
    return {"count": len(values), "mean": statistics.mean(values), "p50": statistics.median(values),
            "p95": values[math.ceil(len(values) * .95) - 1], "max": values[-1]}


def histogram_distribution(buckets):
    """Merge raw HDR buckets; never take a percentile of interval percentiles."""
    totals = {}
    for value, count in buckets:
        if count < 0 or value < 0:
            raise ValueError("Invalid latency histogram")
        if count:
            totals[value] = totals.get(value, 0) + count
    count = sum(totals.values())
    if not count:
        return distribution([])
    ordered = sorted(totals.items())
    def quantile(fraction):
        remaining = math.ceil(count * fraction)
        for value, frequency in ordered:
            remaining -= frequency
            if remaining <= 0:
                return value / 1e6
    return {"count": count, "mean": sum(v * n for v, n in ordered) / count / 1e6,
            "p50": quantile(.5), "p95": quantile(.95), "max": ordered[-1][0] / 1e6}


def action_summary(records, begin, end):
    """Correlate model/render witnesses with submission of the same window.

    Starts are application handling times, not hardware input timestamps.
    Clicks have acceptance witnesses only and are reported separately.
    """
    actions, submissions = {}, {}
    for record in records:
        if record["event"] == "action":
            actions.setdefault(record["id"], {})[record["phase"]] = record
        elif record["event"] == "submit":
            submissions.setdefault(record.get("window"), []).append(record["start_ms"])
    for times in submissions.values():
        times.sort()
    output = {}
    for phases in actions.values():
        start = phases.get("begin")
        if start is None or not begin <= start["at_ms"] <= end:
            continue
        group = output.setdefault(start["kind"], {"started": 0, "applied": 0, "rendered": 0,
                                                "submitted": 0, "latencies": []})
        group["started"] += 1
        group["applied"] += "applied" in phases
        group["rendered"] += "rendered" in phases
        witness = phases.get("rendered") if start["kind"] != "click" else phases.get("accepted")
        if witness is None:
            continue
        times = submissions.get(witness.get("detail", {}).get("window"), [])
        index = bisect.bisect_left(times, witness["at_ms"])
        if index < len(times) and times[index] <= end:
            group["submitted"] += 1
            group["latencies"].append(times[index] - start["at_ms"])
    for kind, group in output.items():
        group["handling_to_submit_ms"] = distribution(group.pop("latencies"))
        group["unwitnessed"] = group["started"] - group["submitted"]
        group["witness"] = "acceptance only" if kind == "click" else "model revision rendered"
    return output


def initial_search_summary(records, phase_start):
    seeds = {r["id"] for r in records if r["event"] == "action" and r["phase"] == "applied"
             and r["at_ms"] < phase_start and r.get("detail", {}).get("query_bytes") == 5
             and r["detail"].get("matches") == 100 and "b.txt" in (r["detail"].get("target") or "")}
    selected = [r for r in records if r["event"] == "submit"
                or (r["event"] == "action" and r["id"] in seeds)]
    result = action_summary(selected, 0, phase_start).get("diff_search", {})
    if result.get("submitted") != 1:
        raise ValueError("Expected one witnessed initial search before the warm-query phase")
    return result["handling_to_submit_ms"]


def summarize(directory):
    directory = Path(directory)
    capture = json.loads((directory / "capture.json").read_text(encoding="utf-8-sig"))
    if capture["outcome"] != "passed" or not capture["phases"] or any(not phase["valid"] for phase in capture["phases"]):
        raise ValueError(f"Unsuccessful capture: {directory}")
    names = [phase["name"] for phase in capture["phases"]]
    if len(names) != len(set(names)):
        raise ValueError(f"Duplicate scenarios: {directory}")
    frames = directory / "frames.jsonl"
    records = [json.loads(line) for line in frames.read_text(encoding="utf-8").splitlines()] if frames.exists() else []
    anchors = [record for record in records if record["event"] == "start"]
    if capture["probe"] and len(anchors) != 1:
        raise ValueError(f"Expected one probe clock anchor: {directory}")
    anchor = anchors[0]["unix_ms"] if anchors else 0
    if any(record.get("records_dropped", 0) for record in records):
        raise ValueError(f"Probe lost records: {directory}")
    phases = {}
    for phase in capture["phases"]:
        # Require complete frames within the phase, excluding its first/last
        # 200 ms. Percentiles use raw observations, never interval percentiles.
        begin, end = phase["start_unix_ms"] + 200, phase["end_unix_ms"] - 200
        if end <= begin or phase["seconds"] <= 0 or (phase["name"] != "idle" and phase["actions"] <= 0):
            raise ValueError(f"Empty scenario: {directory}/{phase['name']}")
        if phase["name"].startswith("native-") and (phase["native_starts"] < 1 or phase["native_ends"] < 1):
            raise ValueError(f"Missing native gesture events: {directory}/{phase['name']}")
        selected = [record for record in records if record["event"] in ("draw", "submit")
                    and begin <= anchor + record["start_ms"] <= anchor + record["at_ms"] <= end]
        draws = [record for record in selected if record["event"] == "draw"]
        if capture["probe"] and phase["name"] in ("typing", "commit-typing", "file-search") and len(draws) < phase["actions"] / 3:
            raise ValueError(f"Typing did not produce enough UI updates; verify filter focus: {directory}")
        intervals = [record for record in records if record["event"] == "interval"
                     and begin <= anchor + record["at_ms"] - record["wall_ms"]
                     and anchor + record["at_ms"] <= end]
        phases[phase["name"]] = {
            "actions": phase["actions"], "actions_per_second": phase["actions"] / phase["seconds"],
            "process_cpu_seconds": phase["cpu_seconds"], "seconds": phase["seconds"],
            "process_cpu_cores": phase["cpu_seconds"] / phase["seconds"],
            "api_ms": distribution(phase["api_ms"]),
            "draw_ms": distribution(record["duration_ms"] for record in draws),
            "submit_ms": distribution(record["duration_ms"] for record in selected if record["event"] == "submit"),
            "dirty_to_draw_ms": distribution(record["at_ms"] - record["dirty_ms"] for record in draws
                                             if record["dirty_ms"] is not None and anchor + record["dirty_ms"] >= begin),
            "slow_frames": sum(record["duration_ms"] > 1000 / 60 for record in draws),
            "wake_ms": distribution(value for interval in intervals for value in interval["wake_ms"]),
            "native_starts": phase["native_starts"], "native_ends": phase["native_ends"],
            "semantic_actions": action_summary(records, begin - anchor, end - anchor),
            "input_ms": histogram_distribution(bucket for record in records
                if record["event"] == "input_interval" and "wall_ms" in record
                and begin <= anchor + record["at_ms"] - record["wall_ms"] <= anchor + record["at_ms"] <= end
                for bucket in record.get("histogram_ns", [])),
        }
        if capture["probe"] and phase["name"] in ("typing", "commit-typing"):
            actions = phases[phase["name"]]["semantic_actions"].get("typing", {})
            if actions.get("submitted", 0) < phase["actions"] / 3:
                raise ValueError(f"Missing text revision witnesses; verify input focus: {directory}")
        if capture["probe"] and phase["name"] == "file-search":
            # Report the first query too: priming a slow index must not hide a
            # cold-search regression behind fast repeated queries.
            phases[phase["name"]]["initial_query_ms"] = initial_search_summary(
                records, phase["start_unix_ms"] - anchor)
            actions = phases[phase["name"]]["semantic_actions"].get("diff_search", {})
            if actions.get("submitted", 0) < phase["actions"] / 3:
                raise ValueError(f"Missing query result witnesses; verify file/search focus: {directory}")
            correct = [record for record in records if record["event"] == "action" and record["phase"] == "applied"
                       and begin <= anchor + record["at_ms"] <= end
                       and record.get("detail", {}).get("query_bytes") == 6
                       and record["detail"].get("matches") == 100 and "b.txt" in (record["detail"].get("target") or "")]
            if not correct:
                raise ValueError(f"Expected needle to match 100 rows in b.txt; wrong fixture or stale results: {directory}")
    result = {"binary_sha256": capture["sha256"].lower(), "probe": capture["probe"], "gpu": capture["gpu"],
              "environment": {key: capture.get(key) for key in ("gpu", "repository_sha", "input_rate", "d3d_validation",
                                                                  "harness_ps1_sha256", "harness_cs_sha256")},
              "repository": capture.get("repository"), "repository_sha": capture.get("repository_sha"),
              "input_rate": capture.get("input_rate"), "d3d_validation": capture.get("d3d_validation"),
              "phases": phases, "note": "Submission measures CPU/platform work, not visible display completion."}
    (directory / "summary.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def ps_quote(value):
    return "'" + str(value).replace("'", "''") + "'"


def compare(samples, scenarios):
    comparisons = {}
    metrics = {f"{metric}.{quantile}": (metric, quantile)
               for metric in ("draw_ms", "submit_ms", "dirty_to_draw_ms", "wake_ms", "input_ms", "initial_query_ms")
               for quantile in ("p50", "p95")}
    metrics.update({metric: (metric,) for metric in ("process_cpu_cores", "actions_per_second")})
    for kind in ("typing", "diff_search", "click"):
        for quantile in ("p50", "p95"):
            metrics[f"{kind}_handling_to_submit_ms.{quantile}"] = ("semantic_actions", kind, "handling_to_submit_ms", quantile)
    for scenario in scenarios:
        comparisons[scenario] = {}
        for label, keys in metrics.items():
            medians = {}
            for variant in ("baseline", "candidate"):
                values = []
                for sample in samples:
                    if sample["variant"] == variant:
                        value = sample["summary"]["phases"][scenario]
                        for key in keys:
                            value = value.get(key) if isinstance(value, dict) else None
                        values.append(value)
                medians[variant] = statistics.median(values) if values and all(v is not None for v in values) else None
            base, candidate = medians["baseline"], medians["candidate"]
            comparisons[scenario][label] = {
                **medians, "reduction_percent": (1 - candidate / base) * 100 if base and candidate is not None else None}
    return comparisons


def report_sessions(directories):
    sessions = [json.loads((directory / "session.json").read_text(encoding="utf-8")) for directory in directories]
    if not sessions or any(not session["complete"] for session in sessions):
        raise ValueError("Only completed sessions can be compared")
    if len({session["measurement_id"] for session in sessions}) != len(sessions):
        raise ValueError("A copied session is not an independent measurement")
    reference = sessions[0]
    for session in sessions[1:]:
        for key in ("machine", "hashes", "scenarios", "seconds", "probe", "d3d_validation", "repository", "environment"):
            if session[key] != reference[key]:
                raise ValueError(f"Sessions disagree on {key}")
    samples = [sample for session in sessions for sample in session["samples"]]
    expected = sum(session["pairs"] for session in sessions)
    if len(samples) != expected * 2 or any(sum(s["variant"] == variant for s in samples) != expected
                                         for variant in ("baseline", "candidate")):
        raise ValueError("Incomplete baseline/candidate pairs")
    return {"pairs": expected, "sessions": len(sessions), "hashes": reference["hashes"],
            "enough_samples": expected >= 6 and len({session["session"] for session in sessions}) >= 2,
            "comparisons": compare(samples, reference["scenarios"]),
            "note": "Review the target metric (>=10% reduction) and p95 guards (<=5% regression); sample count alone does not establish acceptance."}


def measure(args):
    if sys.platform != "win32":
        raise ValueError("Native UI measurements require Windows")
    if args.pairs < 1 or not 2 <= args.seconds <= 120:
        raise ValueError("Positive pairs and 2..120 seconds required")
    if len(args.scenarios) != len(set(args.scenarios)):
        raise ValueError("Scenarios must be unique")
    native = any(name in ("native-move", "native-resize", "typing", "commit-typing", "file-search") for name in args.scenarios)
    if native and not args.native_gestures:
        raise ValueError("Native scenarios require --native-gestures on an idle desktop")
    repository = args.repository.resolve()
    revision = subprocess.check_output(["git", "-C", str(repository), "rev-parse", "HEAD"], text=True).strip()

    def verify_repository():
        current = subprocess.check_output(["git", "-C", str(repository), "rev-parse", "HEAD"], text=True).strip()
        dirty = subprocess.check_output(["git", "-C", str(repository), "status", "--porcelain"])
        if current != revision or dirty:
            raise ValueError("Paired measurements require an unchanged, clean fixture clone")

    verify_repository()
    args.output.mkdir(parents=True, exist_ok=False)
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    hashes = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()}
    report = {"measurement_id": str(uuid.uuid4()), "session": args.session, "machine": platform.node(),
              "scenarios": args.scenarios, "seconds": args.seconds, "pairs": args.pairs,
              "probe": not args.no_probe, "d3d_validation": args.d3d_validation, "repository": str(args.repository.resolve()),
              "binaries": {k: str(v) for k, v in binaries.items()},
              "hashes": hashes, "samples": [], "complete": False}
    try:
        for pair in range(args.pairs):
            order = ["baseline", "candidate"] if (pair + args.reverse) % 2 == 0 else ["candidate", "baseline"]
            for name in order:
                verify_repository()
                output = (args.output / f"pair-{pair + 1}-{name}").resolve()
                command = "& " + ps_quote(ROOT / "scripts/profiling/measure-ui-responsiveness.ps1")
                for key, value in {"OutputDirectory": output, "Repository": args.repository.resolve(),
                                   "Binary": binaries[name], "SecondsPerScenario": args.seconds,
                                   "D3DValidation": args.d3d_validation}.items():
                    command += " -" + key + " " + ps_quote(value)
                command += " -Scenarios @(" + ",".join(ps_quote(s) for s in args.scenarios) + ")"
                if native:
                    command += " -NativeGestures"
                if args.no_probe:
                    command += " -NoProbe"
                # The harness owns all app descendants in a Windows job and
                # restores cursor/focus/environment even after a failed phase.
                subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command],
                               cwd=ROOT, check=True)
                summary = summarize(output)
                verify_repository()
                if summary["binary_sha256"] != hashes[name]:
                    raise ValueError(f"Measured binary changed during session: {name}")
                if report.setdefault("environment", summary["environment"]) != summary["environment"]:
                    raise ValueError("Measurement environment or harness changed during session")
                report["samples"].append({"pair": pair + 1, "variant": name, "path": str(output), "summary": summary})
        report.update(complete=True, comparisons=compare(report["samples"], args.scenarios))
    finally:
        (args.output / "session.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def create_fixture(path):
    """100,000 rows per file; exactly 100 rows contain the complete needle."""
    path = path.resolve()
    path.mkdir(parents=True, exist_ok=False)
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0",
               GIT_AUTHOR_DATE="2020-01-01T00:00:00Z", GIT_COMMITTER_DATE="2020-01-01T00:00:00Z")
    def git(*arguments):
        subprocess.run(["git", "-C", str(path), *arguments], env=env, capture_output=True, check=True)
    git("init", "-q", "-b", "fixture")
    for key, value in {"user.name":"Probe", "user.email":"probe@example.invalid", "core.autocrlf":"false", "commit.gpgsign":"false"}.items():
        git("config", key, value)
    for name in ("a.txt", "b.txt"):
        with (path / name).open("w", encoding="utf-8", newline="\n") as output:
            for row in range(100_000):
                output.write(f"row {row:06}: " + ("needle" if row % 1000 == 0 else "plain content") + "\n")
    git("add", ".")
    git("commit", "-qm", "UI performance fixture")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    fixture = commands.add_parser("fixture")
    fixture.add_argument("directory", type=Path)
    summary = commands.add_parser("summarize")
    summary.add_argument("directory", type=Path)
    combined = commands.add_parser("report")
    combined.add_argument("directories", type=Path, nargs="+")
    paired = commands.add_parser("measure")
    for name in ("baseline", "candidate", "repository", "output"):
        paired.add_argument("--" + name, type=Path, required=True)
    paired.add_argument("--session", required=True)
    paired.add_argument("--pairs", type=int, default=3)
    paired.add_argument("--seconds", type=int, default=5)
    paired.add_argument("--scenarios", nargs="+", choices=SCENARIOS, default=["native-move", "native-resize", "scroll", "click"])
    paired.add_argument("--native-gestures", action="store_true")
    paired.add_argument("--no-probe", action="store_true")
    paired.add_argument("--reverse", action="store_true")
    paired.add_argument("--d3d-validation", choices=("auto", "on", "off"), default="auto")
    args = parser.parse_args()
    if args.command == "fixture":
        create_fixture(args.directory)
    elif args.command == "summarize":
        print(json.dumps(summarize(args.directory), indent=2))
    elif args.command == "report":
        print(json.dumps(report_sessions(args.directories), indent=2))
    else:
        measure(args)


if __name__ == "__main__":
    main()
