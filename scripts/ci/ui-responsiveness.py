#!/usr/bin/env python3
"""Measure frozen Windows GUI binaries in alternating pairs, or summarize a capture."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
SCENARIOS = ("idle", "move", "resize", "native-move", "native-resize", "hover", "scroll", "click", "typing")


def distribution(values):
    values = sorted(values)
    if not values:
        return {"count": 0, "mean": None, "p50": None, "p95": None, "max": None}
    return {"count": len(values), "mean": statistics.mean(values), "p50": statistics.median(values),
            "p95": values[math.ceil(len(values) * .95) - 1], "max": values[-1]}


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
        if capture["probe"] and phase["name"] == "typing" and len(draws) < phase["actions"] / 3:
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
        }
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
               for metric in ("draw_ms", "submit_ms", "dirty_to_draw_ms", "wake_ms")
               for quantile in ("p50", "p95")}
    metrics.update({metric: (metric,) for metric in ("process_cpu_cores", "actions_per_second")})
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
                            value = value[key]
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
    native = any(name in ("native-move", "native-resize", "typing") for name in args.scenarios)
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
                command = "& " + ps_quote(ROOT / "scripts/measure-ui-responsiveness.ps1")
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
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
    if args.command == "summarize":
        print(json.dumps(summarize(args.directory), indent=2))
    elif args.command == "report":
        print(json.dumps(report_sessions(args.directories), indent=2))
    else:
        measure(args)


if __name__ == "__main__":
    main()
