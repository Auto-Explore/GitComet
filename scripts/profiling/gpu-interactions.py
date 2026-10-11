#!/usr/bin/env python3
"""Paired native GPU captures of history hover, diff viewing and repo switches.

Supply frozen release binaries and two disposable repositories. The first repo
must have at least two unstaged tracked files. This harness does not edit files.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import statistics

spec = importlib.util.spec_from_file_location("live_ui", Path(__file__).with_name("live-ui.py"))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


def scenario(repository, secondary, seconds, changes_per_second=10):
    settle = {"do": "settle", "ms": 3000}
    ready = {"do": "wait_ready", "indexed": True, "timeout_ms": 180_000}
    steps = [ready, settle, {"do": "phase", "name": "history_hover"},
             {"do": "move_pointer", "positions": [[.55, .08 + row * .022] for row in range(24)],
              "repeat": int(seconds * 125), "interval_ms": 8},
             {"do": "phase", "name": "diff_setup"},
             {"do": "click", "target": {"list": "unstaged_row", "index": 0},
              "witness": {"kind": "diff_loaded"}}, settle,
             {"do": "phase", "name": "diff_view"}]
    for index in range(int(seconds * changes_per_second)):
        steps.extend([
            {"do": "click", "target": {"list": "unstaged_row", "index": (index + 1) % 2},
             "witness": {"kind": "diff_loaded"}},
            {"do": "settle", "ms": round(1000 / changes_per_second)},
        ])
    steps.extend([{"do": "phase", "name": "tabs_setup"},
                  {"do": "open_repo", "path": str(secondary)}, ready, settle,
                  {"do": "phase", "name": "repo_tabs"}])
    for index in range(int(seconds * changes_per_second)):
        expected = repository if index % 2 == 0 else secondary
        steps.extend([
            {"do": "keys", "key": "ctrl-tab", "repeat": 1, "interval_ms": 0,
             "witness": {"kind": "repo_open", "path": str(expected)}},
            {"do": "settle", "ms": round(1000 / changes_per_second)},
        ])
    steps.extend([{"do": "phase", "name": "settled"}, {"do": "settle", "ms": 5000}])
    return {"version": 1, "steps": steps}


def brief(result, phase):
    value = result["phases"][phase]
    device = value.get("gpu", {}).get("devices", {}).get("0", {})
    process = value.get("gpu", {}).get("processes", {}).get("app", {}).get("0", {})
    return {
        "seconds": value["seconds"], "draws": value["frames"],
        "draw_p95_ms": value["draw_ms"]["p95"],
        "submit_p95_ms": value["submit_ms"]["p95"],
        "input_to_draw_p95_ms": value["inputs"]["input_to_draw_ms"]["p95"],
        "inputs": value["inputs"]["count"],
        "witnessed": value["inputs"]["witnessed"],
        "sm_percent": process.get("sm_percent", {}),
        "memory_busy_percent": process.get("memory_busy_percent", {}),
        "framebuffer_mb": process.get("framebuffer_mb", {}),
        "graphics_clock_mhz": device.get("graphics_clock_mhz", {}),
        "memory_clock_mhz": device.get("memory_clock_mhz", {}),
        "work_units": value.get("work_units"),
    }


def binary_hash(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def fixture_identity(repository):
    return {"head": live.git(repository, "rev-parse", "HEAD").stdout.decode().strip(),
            "diff_sha256": hashlib.sha256(live.git(repository, "diff", "--binary", "HEAD").stdout).hexdigest(),
            "status_sha256": hashlib.sha256(live.git(repository, "status", "--porcelain=v1", "-z").stdout).hexdigest()}


def clock_comparable(baseline, candidate):
    return all(baseline.get(metric) is not None and candidate.get(metric) is not None
               and abs(baseline[metric] - candidate[metric]) <= .05 * max(baseline[metric], candidate[metric])
               for metric in ["graphics_clock_mhz", "memory_clock_mhz"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--repository", required=True, type=Path)
    parser.add_argument("--secondary-repository", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sizes", nargs="+", default=["1400x900", "2560x1440"])
    parser.add_argument("--scales", nargs="+", type=int, default=[100])
    parser.add_argument("--pairs", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=20)
    parser.add_argument("--refresh-hz", type=int, default=60,
                        help="refresh rate of the private native display")
    parser.add_argument("--changes-per-second", type=int, default=10,
                        help="diff and repository switches per second; sustained stress, not human click rate")
    parser.add_argument("--reverse", action="store_true")
    parser.add_argument("--clean", action="store_true", help="disable GPU sampling for input/CPU checks")
    args = parser.parse_args()
    if args.pairs < 1 or args.seconds < 5:
        parser.error("use at least one pair and five seconds per active phase")
    if not 1 <= args.changes_per_second <= 60:
        parser.error("changes-per-second must be between 1 and 60")
    if args.refresh_hz < 1:
        parser.error("refresh-hz must be positive")
    args.output.mkdir(parents=True, exist_ok=False)
    repository, secondary = args.repository.resolve(), args.secondary_repository.resolve()
    unstaged = live.git(repository, "diff", "--name-only", "-z").stdout.split(b"\0")
    if len([path for path in unstaged if path]) < 2:
        parser.error("the first repository needs at least two unstaged tracked files for diff switching")
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    report = {"version": 1, "gpu_sampling": not args.clean,
              "binary_hashes": {name: binary_hash(path) for name, path in binaries.items()},
              "harness_hashes": {name: binary_hash(Path(__file__).with_name(name))
                                 for name in ["gpu-interactions.py", "live-ui.py", "perf_gpu.py"]},
              "fixtures": {str(path): fixture_identity(path) for path in [repository, secondary]},
              "seconds_per_phase": args.seconds, "changes_per_second": args.changes_per_second,
              "refresh_hz": args.refresh_hz,
              "pairs": args.pairs, "runs": [], "complete": False}
    try:
        for geometry in args.sizes:
            width, height = (int(part) for part in geometry.split("x"))
            for scale in args.scales:
                for pair in range(args.pairs):
                    order = ["baseline", "candidate"] if (pair + args.reverse) % 2 == 0 else ["candidate", "baseline"]
                    for variant in order:
                        output = args.output / f"{geometry}-{scale}-{pair}-{variant}"
                        result = live.run_once(
                            binaries[variant], repository, "gpu-interactions", output, 300,
                            scenario_steps=scenario(repository, secondary, args.seconds, args.changes_per_second),
                            window_size=[width, height], ui_scale=scale, gpu=not args.clean,
                            refresh_hz=args.refresh_hz)
                        if not result["valid"]:
                            raise RuntimeError(f"invalid capture {output}: {result['problems']}")
                        if result.get("debug_assertions") is not False:
                            raise RuntimeError(f"a release binary is required: {binaries[variant]}")
                        capture = json.loads((output / "capture.json").read_text())
                        if capture["binary_sha256"] != report["binary_hashes"][variant]:
                            raise RuntimeError(f"binary changed during measurement: {binaries[variant]}")
                        for path in [repository, secondary]:
                            if fixture_identity(path) != report["fixtures"][str(path)]:
                                raise RuntimeError(f"fixture changed during measurement: {path}")
                        record = {"geometry": geometry, "scale": scale, "pair": pair, "variant": variant,
                                  "capture": str(output), "graphics": result.get("graphics"),
                                  "phases": {phase: brief(result, phase)
                                             for phase in ["history_hover", "diff_view", "repo_tabs"]}}
                        if not args.clean:
                            for phase, data in record["phases"].items():
                                for metric in ["sm_percent", "graphics_clock_mhz", "memory_clock_mhz"]:
                                    minimum = max(3, min(10, args.seconds - 2))
                                    if data[metric].get("count", 0) < minimum:
                                        raise RuntimeError(f"insufficient {metric} samples in {output}/{phase}")
                        report["runs"].append(record)
                        print(f"{geometry} scale={scale} pair={pair} {variant}: " + ", ".join(
                            f"{phase} GPU={data['sm_percent'].get('mean')}% draw_p95={data['draw_p95_ms']:.3f}ms"
                            for phase, data in record["phases"].items()), flush=True)
                        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        comparisons = []
        for geometry in args.sizes:
            for scale in args.scales:
                for phase in ["history_hover", "diff_view", "repo_tabs"]:
                    runs = [run for run in report["runs"] if run["geometry"] == geometry and run["scale"] == scale]
                    values = {}
                    for variant in binaries:
                        samples = [run["phases"][phase] for run in runs if run["variant"] == variant]
                        values[variant] = {
                            metric: statistics.median(sample[metric]["mean"] for sample in samples
                                                       if sample[metric].get("mean") is not None)
                            if any(sample[metric].get("mean") is not None for sample in samples) else None
                            for metric in ["sm_percent", "memory_busy_percent", "framebuffer_mb", "graphics_clock_mhz", "memory_clock_mhz"]}
                        for metric in ["draw_p95_ms", "submit_p95_ms", "input_to_draw_p95_ms"]:
                            timings = [sample[metric] for sample in samples if sample[metric] is not None]
                            values[variant][metric] = statistics.median(timings) if timings else None
                    comparison = {"geometry": geometry, "scale": scale, "phase": phase, **values,
                                  "gpu_clocks_comparable": clock_comparable(values["baseline"], values["candidate"])}
                    paired_reductions = []
                    for pair in range(args.pairs):
                        metrics = {}
                        for variant in binaries:
                            data = next(run for run in runs if run["pair"] == pair
                                        and run["variant"] == variant)["phases"][phase]
                            metrics[variant] = {metric: data[metric].get("mean") for metric in
                                                ["sm_percent", "graphics_clock_mhz", "memory_clock_mhz"]}
                        before, after = metrics["baseline"], metrics["candidate"]
                        if clock_comparable(before, after) and before["sm_percent"] and after["sm_percent"] is not None:
                            paired_reductions.append(100 * (1 - after["sm_percent"] / before["sm_percent"]))
                    comparison["paired_sm_reductions_percent"] = paired_reductions
                    baseline, candidate = values["baseline"]["sm_percent"], values["candidate"]["sm_percent"]
                    comparison["sm_reduction_percent"] = 100 * (1 - candidate / baseline) if (
                        comparison["gpu_clocks_comparable"] and baseline and candidate is not None) else None
                    comparisons.append(comparison)
        report["comparisons"] = comparisons
        report["complete"] = True
    finally:
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
