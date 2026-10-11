#!/usr/bin/env python3
"""Per-phase CPU/GPU load table for live-ui.py runs captured with --gpu on macOS.

  macos-usage-report.py RUN_DIR... [--json OUT]

Each RUN_DIR is a `live-ui.py run` output (summary.json). Several runs of the
same scenario are reported side by side, then as medians per phase.

Columns, all per second of the phase unless noted:
  frames/s     CPU-drawn frames (the UI probe's draw records)
  draw p50/p95 CPU draw time per frame, ms
  cpu ms/s     process CPU time (all threads), from proc_pid_rusage
  gpu ms/s     the process's Metal GPU execution time (IOKit AGX AppUsage)
  gpu/frame    gpu ms/s divided by frames/s (includes non-drawing presents)
  ws gpu ms/s  WindowServer GPU time: composition of every app on screen
  soc gpu mW   whole-GPU power from IOReport energy counters (all processes)
  gpu P        mean GPU performance state while active (P1 = lowest clock)
  wake/s       package-idle wakeups of the process
  cpu mW       the kernel's CPU energy estimate for the process (CPU only)
"""
import argparse
import json
from pathlib import Path
import re
import statistics

COLUMNS = (
    ("frames/s", lambda p: p.get("frames_per_second")),
    ("draw p50", lambda p: (p.get("draw_ms") or {}).get("p50")),
    ("draw p95", lambda p: (p.get("draw_ms") or {}).get("p95")),
    ("cpu ms/s", lambda p: p["process_cpu_cores"] * 1000 if p.get("process_cpu_cores") is not None else None),
    ("gpu ms/s", lambda p: gpu(p, "app", "gpu_ms_per_s")),
    ("gpu/frame", lambda p: gpu(p, "app", "gpu_ms_per_s") / p["frames_per_second"]
     if gpu(p, "app", "gpu_ms_per_s") is not None and p.get("frames_per_second") else None),
    ("ws gpu ms/s", lambda p: gpu(p, "compositor", "gpu_ms_per_s")),
    ("soc gpu mW", lambda p: ((p.get("gpu") or {}).get("soc") or {}).get("gpu_power_mw")),
    ("gpu P", lambda p: ((p.get("gpu") or {}).get("soc") or {}).get("gpu_active_mean_pstate")),
    ("wake/s", lambda p: p.get("idle_wakeups_per_second")),
    ("cpu mW", lambda p: p.get("cpu_energy_mw")),
)


def gpu(phase, role, key):
    roles = ((phase.get("gpu") or {}).get("processes") or {}).get(role) or {}
    return next(iter(roles.values()), {}).get(key)


def fmt(value):
    if value is None:
        return "-"
    return f"{value:.1f}" if abs(value) >= 10 else f"{value:.2f}"


def table(rows, header):
    widths = [max(len(str(row[i])) for row in [header, *rows]) for i in range(len(header))]
    lines = ["  ".join(str(cell).rjust(width) if i else str(cell).ljust(width)
                       for i, (cell, width) in enumerate(zip(row, widths))) for row in [header, *rows]]
    lines.insert(1, "  ".join("-" * width for width in widths))
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("runs", nargs="+", type=Path)
    parser.add_argument("--json", type=Path, help="write the per-phase medians as JSON")
    args = parser.parse_args()
    header = ("run/phase", "s", *(name for name, _ in COLUMNS))
    rows, per_phase = [], {}
    for run in args.runs:
        summary = json.loads((run / "summary.json").read_text())
        label = run.name + ("" if summary.get("valid") else " (INVALID: " + "; ".join(summary["problems"]) + ")")
        for name, phase in summary.get("phases", {}).items():
            values = [extract(phase) for _, extract in COLUMNS]
            rows.append((f"{label}/{name}", fmt(phase.get("seconds")), *map(fmt, values)))
            # Repeats of a case are named CASE-N (macos-usage.py); group by case.
            case = re.sub(r"-\d+$", "", run.name) if re.search(r"-\d+$", run.name) else summary["scenario"]
            per_phase.setdefault((case, name), []).append(values)
    print(table(rows, header))
    if len(args.runs) > 1:
        medians, out = [], {}
        for (scenario, name), samples in per_phase.items():
            columns = list(zip(*samples))
            values = [statistics.median(v for v in column if v is not None) if any(v is not None for v in column) else None
                      for column in columns]
            medians.append((f"{scenario}/{name} (n={len(samples)})", "", *map(fmt, values)))
            out[f"{scenario}/{name}"] = {"runs": len(samples), **{name: value for (name, _), value in zip(COLUMNS, values)}}
        print()
        print(table(medians, ("median", "", *(name for name, _ in COLUMNS))))
        if args.json:
            args.json.write_text(json.dumps(out, indent=2) + "\n")


if __name__ == "__main__":
    main()
