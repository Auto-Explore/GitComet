#!/usr/bin/env python3
"""Native same-process multi-window measurements with repository witnesses.

Use disposable or read-only repositories. Each run gets an isolated application
profile and compositor. Open/close/focus use production window paths; inputs
target the named, natively active window. No worktree files are changed.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import statistics

import perf_report


spec = importlib.util.spec_from_file_location("multi_window_live", Path(__file__).with_name("live-ui.py"))
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


def phase(name):
    return {"do": "phase", "name": name}


def ready():
    return {"do": "wait_ready", "indexed": True, "timeout_ms": 180_000}


def select(count):
    return [{"do": "focus", "target": "history"},
            {"do": "keys", "key": "down", "repeat": count, "interval_ms": 16,
             "witness": {"kind": "commit_details"}}]


def new_repo(name, repository):
    return [{"do": "new_window", "name": name},
            {"do": "open_repo", "path": str(repository)}, ready()]


def close(name):
    return [{"do": "switch_window", "name": "primary"},
            {"do": "close_window", "name": name},
            {"do": "expect_windows", "count": 1}]


def matrix(repositories, count, idle_ms, inputs):
    steps = [ready()]
    for index in range(1, count):
        steps += [phase(f"open_{index + 1}")]
        steps += new_repo(f"window-{index}", repositories[index % len(repositories)])
    steps += [{"do": "switch_window", "name": "primary"},
              {"do": "expect_windows", "count": count},
              phase("settle"), {"do": "settle", "ms": 3000},
              phase("idle"), {"do": "settle", "ms": idle_ms},
              phase("select"), *select(inputs),
              {"do": "expect_windows", "count": count},
              phase("hover"),
              {"do": "move_pointer", "positions": [[.55, .08 + row * .022] for row in range(24)],
               "repeat": inputs * 2, "interval_ms": 8},
              phase("scroll"),
              {"do": "scroll", "target": "history", "delta_px": -72, "repeat": inputs,
               "interval_ms": 16, "flip_every": 60, "witness": {"kind": "history_scrolled"}}]
    # Exercise every window, rather than merely counting constructed windows.
    for index in range(1, count):
        steps += [phase(f"focus_{index}"), {"do": "switch_window", "name": f"window-{index}"},
                  ready(), *select(min(inputs, 60))]
    steps += [{"do": "switch_window", "name": "primary"},
              {"do": "expect_windows", "count": count},
              phase("settled"), {"do": "settle", "ms": 3000}]
    return {"steps": steps}


def background(repository, inputs):
    return {"steps": [ready(), phase("baseline_select"), *select(inputs),
                      {"do": "new_window", "name": "background"},
                      phase("background_load"),
                      {"do": "open_repo", "path": str(repository), "wait": False},
                      {"do": "switch_window", "name": "primary"},
                      phase("select_while_loading"), *select(inputs * 2),
                      phase("await_background"),
                      {"do": "switch_window", "name": "background"}, ready(),
                      {"do": "expect_windows", "count": 2},
                      {"do": "switch_window", "name": "primary"},
                      phase("after_load_select"), *select(inputs),
                      phase("settled"), {"do": "settle", "ms": 3000}]}


def lifecycle(repository, cycles, idle_ms):
    steps = [ready(), phase("warmup")]
    for _ in range(3):
        steps += new_repo("temporary", repository) + close("temporary")
    steps += [phase("before_cycles"), {"do": "settle", "ms": idle_ms}]
    for index in range(cycles):
        steps += [phase(f"cycle_{index}")]
        steps += new_repo("temporary", repository) + close("temporary")
        # Give asynchronous store/watcher shutdown time to run.
        steps += [{"do": "settle", "ms": 1000}]
    steps += [phase("after_cycles"), {"do": "settle", "ms": idle_ms}]
    return {"steps": steps}


def verify_isolation(directory):
    records = live.load_records(directory)
    phases = [r for r in records if r["event"] == "scenario_phase"]
    snapshots = [r for r in records if r["event"] == "scenario_windows"]
    for phase_record in phases:
        name = phase_record["detail"]["name"]
        if phase_record["detail"]["state"] != "begin" or not (name == "select" or name.startswith("focus_")):
            continue
        begin = phase_record["at_ms"]
        end = next((r["at_ms"] for r in phases if r["detail"] == {"name": name, "state": "end"}), None)
        if end is None:
            raise ValueError(f"{name}: missing phase end for isolation check")
        target = "primary" if name == "select" else "window-" + name.removeprefix("focus_")
        before = next((r for r in reversed(snapshots) if r["at_ms"] <= begin), None)
        # The driver snapshots all windows immediately after ending a phase.
        # A snapshot before the end could miss the entire interaction sequence.
        after = next((r for r in snapshots if r["at_ms"] >= end), None)
        if before is None or after is None:
            raise ValueError(f"{name}: missing window snapshot for isolation check")
        selections = lambda r: {w["window"]: w["selected_commit"] for w in r["detail"]["windows"] if w["name"] != target}
        if selections(before) != selections(after):
            raise ValueError(f"{name}: foreground selection changed another window's selection")


def findings(result):
    # Repository-open witnesses include backend loading, so do not apply the
    # keystroke latency budget to them. Main-thread draw/wake stalls still count.
    phases = {}
    for name, data in result["phases"].items():
        if name == "warmup" or name.startswith(("open_", "cycle_")):
            data = dict(data, inputs={k: v for k, v in data["inputs"].items() if k != "input_to_draw_ms"})
        phases[name] = data
    return perf_report.findings(dict(result, phases=phases))


def retention(result, cycles):
    phases = result["phases"]
    warm, last, after = (phases.get(name, {}).get("end_sample") or {} for name in
                         ("before_cycles", f"cycle_{cycles - 1}", "after_cycles"))
    return {key: {"after_warmup": warm.get(key), "after_cycles": last.get(key),
                  "settled": after.get(key),
                  "growth_per_cycle": (after[key] - warm[key]) / cycles
                  if warm.get(key) is not None and after.get(key) is not None else None}
            for key in ("pss_kib", "rss_kib", "private_kib", "threads", "fds", "handles")}


def background_overlap(directory):
    records = live.load_records(directory)
    def interval(name):
        return tuple(next(r["at_ms"] for r in records if r["event"] == "scenario_phase"
                          and r["detail"] == {"name": name, "state": state}) for state in ("begin", "end"))
    open_lo, open_hi = interval("background_load")
    input_lo, input_hi = interval("select_while_loading")
    inputs = {r["detail"]["op"]: r for r in records if r["event"] == "scenario_input"}
    opened = {op for op, r in inputs.items() if open_lo <= r["at_ms"] <= open_hi}
    stages = [r for r in records if r["event"] == "stage"]
    loads = [r for r in stages if r["stage"] == "task_finished" and r["label"] == "LoadLog" and r["op"] in opened]
    windows = {inputs[r["op"]]["detail"]["window"] for r in loads}
    overlapping = [r for r in stages if r["stage"] == "input" and input_lo <= r["at_ms"] <= input_hi
                   and r["op"] in inputs and inputs[r["op"]]["detail"].get("window") is not None
                   and inputs[r["op"]]["detail"]["window"] not in windows
                   and any(load["at_ms"] - load["a"] / 1e6 <= r["at_ms"] <= load["at_ms"] for load in loads)]
    return {"log_load_ms": live.distribution(r["a"] / 1e6 for r in loads), "overlapping_inputs": len(overlapping)}


def render(records):
    lines = ["Native multi-window measurements (run medians; CPU draw is not GPU/display completion)."]
    for key in dict.fromkeys(r["case"] for r in records):
        cases = [r for r in records if r["case"] == key]
        lines.append(f"\n{key}: {sum(r['valid'] for r in cases)}/{len(cases)} valid")
        phases = dict.fromkeys(p for r in cases for p in r["result"]["phases"])
        for phase_name in phases:
            data = [r["result"]["phases"][phase_name] for r in cases if phase_name in r["result"]["phases"]]
            def median(path):
                values = []
                for row in data:
                    value = row
                    for part in path.split("."):
                        value = value.get(part) if isinstance(value, dict) else None
                    if value is not None:
                        values.append(value)
                return round(statistics.median(values), 3) if values else None
            lines.append(f"  {phase_name}: draw p95={median('draw_ms.p95')} ms; input p95={median('inputs.input_to_draw_ms.p95')} ms; "
                         f"CPU cores={median('process_cpu_cores')}; PSS max={median('pss_kib.max')} KiB; "
                         f"threads={median('threads')}; fds={median('fds')}")
            gpu_ids = sorted({gpu for row in data for gpu in row.get("gpu", {}).get("devices", {})})
            gpu_ids = sorted(set(gpu_ids) | {gpu for row in data
                                            for devices in row.get("gpu", {}).get("processes", {}).values()
                                            for gpu in devices})
            for gpu in gpu_ids:
                app = f"gpu.processes.app.{gpu}"
                compositor = f"gpu.processes.compositor.{gpu}"
                device = f"gpu.devices.{gpu}"
                lines.append(f"    GPU {gpu}: app SM mean={median(app + '.sm_percent.mean')}% "
                             f"(samples={median(app + '.sm_percent.count')}, missing={median(app + '.sm_percent.missing')}); "
                             f"app framebuffer max={median(app + '.framebuffer_mb.max')} NVIDIA MB; "
                             f"compositor SM mean={median(compositor + '.sm_percent.mean')}%; "
                             f"whole-device busy mean={median(device + '.gpu_busy_percent.mean')}%; "
                             f"whole-board power mean={median(device + '.power_w.mean')} W; "
                             f"graphics clock mean={median(device + '.graphics_clock_mhz.mean')} MHz; "
                             f"memory clock mean={median(device + '.memory_clock_mhz.mean')} MHz")
        if any(r["result"].get("gpu_capture", {}).get("requested") for r in cases):
            lines.append("  GPU collector status: " + ", ".join(r["result"]["gpu_capture"].get("status", "unavailable") for r in cases))
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--repositories", type=Path, nargs="+", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--windows", type=int, nargs="+", default=[1, 2, 4])
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--inputs", type=int, default=300)
    parser.add_argument("--idle-seconds", type=int, default=20)
    parser.add_argument("--mode", choices=["matrix", "background", "lifecycle"], default="matrix")
    parser.add_argument("--cycles", type=int, default=10)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--display", choices=["headless", "desktop"], default="headless")
    parser.add_argument("--gpu-timings", action="store_true", help="collect native GPU frame/pass timings")
    parser.add_argument("--gpu", action="store_true", help="record process and device GPU usage on Linux/NVIDIA")
    parser.add_argument("--window-size", type=live.positive_int, nargs=2, default=live.WINDOW_SIZE,
                        metavar=("WIDTH", "HEIGHT"), help="window dimensions in logical pixels")
    parser.add_argument("--hide-graph", action="store_true", help="diagnose rendering cost with the history graph hidden")
    parser.add_argument("--purpose", choices=["measurement", "validation", "diagnostic"], default="measurement")
    parser.add_argument("--env", action="append", default=[], metavar="KEY=VALUE",
                        help="recorded application environment override (repeatable)")
    args = parser.parse_args()
    if any(n < 1 for n in args.windows) or min(args.samples, args.inputs, args.idle_seconds, args.cycles) < 1:
        parser.error("window/sample/input/cycle counts and idle duration must be positive")
    if args.mode != "matrix" and len(args.repositories) < 2:
        parser.error("background/lifecycle require foreground and secondary repositories")
    if any("=" not in item or not item.split("=", 1)[0] for item in args.env):
        parser.error("--env requires KEY=VALUE")
    extra_env = dict(item.split("=", 1) for item in args.env)
    args.output.mkdir(parents=True, exist_ok=False)
    repositories = [p.resolve(strict=True) for p in args.repositories]
    binary = args.output / ("gitcomet.exe" if args.binary.suffix.lower() == ".exe" else "gitcomet")
    shutil.copy2(args.binary.resolve(strict=True), binary)
    records = []
    for sample in range(args.samples):
        counts = args.windows if sample % 2 == 0 else args.windows[::-1]
        for count in counts if args.mode == "matrix" else [None]:
            key = f"windows-{count}" if count else f"{args.mode}-{args.cycles}" if args.mode == "lifecycle" else args.mode
            steps = (matrix(repositories, count, args.idle_seconds * 1000, args.inputs) if count else
                     background(repositories[1], args.inputs) if args.mode == "background" else
                     lifecycle(repositories[1], args.cycles, args.idle_seconds * 1000))
            directory = args.output / f"sample-{sample}" / key
            print(f"sample={sample} {key}", flush=True)
            result = live.run_once(binary, repositories[0], key, directory, args.timeout,
                                   display=args.display, ping_ms=1000, scenario_steps=steps, extra_env=extra_env, gpu=args.gpu,
                                   window_size=args.window_size, gpu_timings=args.gpu_timings,
                                   session_overrides={"history_show_graph": False} if args.hide_graph else None)
            try:
                verify_isolation(directory)
            except ValueError as error:
                result["valid"] = False
                result["problems"].append(str(error))
            if args.mode == "lifecycle":
                result["retention"] = retention(result, args.cycles)
            if args.mode == "background" and result["valid"]:
                result["background_load_overlap"] = background_overlap(directory)
                if not result["background_load_overlap"]["overlapping_inputs"]:
                    result["valid"] = False
                    result["problems"].append("background history load did not overlap foreground input; use a larger repository")
            (directory / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
            record = {"case": key, "sample": sample, "repositories": [str(p) for p in repositories],
                      "window_size": args.window_size, "history_show_graph": not args.hide_graph,
                      "measurement_kind": args.purpose, "directory": str(directory.resolve()),
                      "valid": result["valid"], "result": result,
                      "findings": findings(result)}
            records.append(record)
            (args.output / "results.json").write_text(json.dumps(records, indent=2) + "\n")
            (args.output / "report.txt").write_text(render(records))
            if not result["valid"]:
                print(json.dumps(result["problems"]), flush=True)
                return 1
    print(render(records))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
