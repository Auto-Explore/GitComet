#!/usr/bin/env python3
"""Paired native GPU experiments. Keeps every sample and rejects missing coverage.

Same binary, alternating runtime flags; scripts never enable shipping defaults.
Use --display desktop on Windows/macOS. Close other windows before desktop runs.
"""
import argparse
import json
from pathlib import Path
import platform
import statistics

import perf_metadata
import perf_workloads

live = perf_workloads.module("gpu_matrix_live", "live-ui.py")
multi = perf_workloads.module("gpu_matrix_multi", "multi-window.py")
FOUNDATION = "cropped-paths,shared-resources"
VARIANTS = {
    "crop": ("", "cropped-paths"),
    "share": ("cropped-paths", FOUNDATION),
    "cache": (FOUNDATION, FOUNDATION + ",cached-layers"),
    "batch": (FOUNDATION, FOUNDATION + ",batched-paths"),
    "paths": (FOUNDATION, FOUNDATION + ",cached-layers,batched-paths"),
    "cache-after-batch": (FOUNDATION + ",batched-paths", FOUNDATION + ",batched-paths,cached-layers"),
    "damage": (FOUNDATION, FOUNDATION + ",partial-redraw"),
    "pool": (FOUNDATION, FOUNDATION + ",pooled-targets"),
    "combined": ("", FOUNDATION + ",cached-layers,batched-paths,partial-redraw,pooled-targets"),
    "connectors": ("cropped-paths", "cropped-paths"),
}


def evaluate(pairs, variant, gpu_timings=True):
    """Evaluate paired medians, never compare an absent timestamp to zero."""
    reasons, improvements, metrics = [], [], {}
    if len(pairs) < 5:
        reasons.append("at least five pairs are required")
    focus_phases = sorted({name for pair in pairs for side in pair for name in side.get("phases", {}) if name.startswith("focus_")})
    for phase in ("select", "hover", "scroll", *focus_phases):
        values = {key: [] for key in ("gpu_p50_change", "gpu_p95_delta", "gpu_p95_limit", "input_p95_delta", "input_p95_limit", "draw_p95_delta", "draw_p95_limit")}
        for baseline, candidate in pairs:
            if not baseline.get("valid") or not candidate.get("valid"):
                reasons.append("invalid application run")
                continue
            sides = [side.get("phases", {}).get(phase, {}) for side in (baseline, candidate)]
            gpu = [side.get("gpu_frames", {}) for side in sides]
            if gpu_timings:
                if any((g.get("coverage") or 0) < .95 or g.get("timed_frames", 0) < 20
                       or g.get("query_samples_dropped", 0) or g.get("records_dropped") != 0 for g in gpu):
                    reasons.append(f"{phase}: missing or dropped GPU measurements")
                    continue
                # The CPU profiler records only newly drawn presentations. GPU IDs
                # also include presentations of an unchanged scene (e.g. input-rate
                # keepalive); those are real work and must remain in the comparison.
                if any(g.get("missing_submission_records", 0) for g in gpu):
                    reasons.append(f"{phase}: GPU submission sequence has gaps")
                    continue
                requested = set(VARIANTS[variant][1].split(",")) - {""}
                if not requested.issubset(gpu[1].get("implemented_experiments", [])):
                    reasons.append(f"{phase}: requested experiment is not implemented by this backend")
                    continue
                for g in gpu:
                    memory = g.get("allocation_bytes", {})
                    for key, limit in (("cached_bytes", 8 * 1024 * 1024), ("device_retention_bytes", 32 * 1024 * 1024)):
                        peak = memory.get(key, {}).get("peak")
                        if peak is not None and peak > limit:
                            reasons.append(f"{phase}: {key} exceeded its retention budget")
                b, c = (g["frame_ms"] for g in gpu)
                if b["p50"] <= 0:
                    reasons.append(f"{phase}: baseline duration is not positive")
                    continue
                values["gpu_p50_change"].append(c["p50"] / b["p50"] - 1)
                values["gpu_p95_delta"].append(c["p95"] - b["p95"])
                values["gpu_p95_limit"].append(max(.1, b["p95"] * .05))
            latency = [s.get("inputs", {}).get("input_to_submit_ms", {}).get("p95") for s in sides]
            # Pointer motion has no causal witness; use the recorded draw duration
            # as a separate CPU guard without presenting it as input latency.
            if all(v is not None for v in latency):
                values["input_p95_delta"].append(latency[1] - latency[0])
                values["input_p95_limit"].append(max(1., latency[0] * .05))
            elif phase != "hover":
                reasons.append(f"{phase}: missing input-to-submit measurements")
            draw = [s.get("draw_ms", {}).get("p95") for s in sides]
            if all(v is not None for v in draw):
                values["draw_p95_delta"].append(draw[1] - draw[0])
                values["draw_p95_limit"].append(max(1., draw[0] * .05))
            else:
                reasons.append(f"{phase}: missing CPU draw measurements")
        medians = {key: statistics.median(v) if v else None for key, v in values.items()}
        metrics[phase] = medians
        if medians["gpu_p50_change"] is not None:
            improvements.append(medians["gpu_p50_change"] <= -.10)
            if medians["gpu_p95_delta"] > medians["gpu_p95_limit"]:
                reasons.append(f"{phase}: GPU p95 regression")
        if medians["draw_p95_delta"] is not None and medians["draw_p95_delta"] > medians["draw_p95_limit"]:
            reasons.append(f"{phase}: CPU draw p95 regression")
        if medians["input_p95_delta"] is not None and medians["input_p95_delta"] > medians["input_p95_limit"]:
            reasons.append(f"{phase}: input p95 regression")
    if gpu_timings and variant in ("cache", "batch", "damage", "paths", "cache-after-batch") and not any(improvements):
        reasons.append("no active phase achieved the 10% median GPU improvement gate")
    return {"status": "needs_review" if reasons else ("local_gates_passed" if gpu_timings else "input_guards_passed"),
            "gpu_timings": gpu_timings,
            "reasons": sorted(set(reasons)), "phases": metrics,
            "shipping_default": "disabled_pending_all_native_backends"}


def environment(variant, side):
    return {"GPUI_GPU_EXPERIMENTS": VARIANTS[variant][side],
            "GITCOMET_GPU_GRAPH_QUADS": "1" if side and variant in ("connectors", "combined") else "0"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--repositories", type=Path, nargs="+", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--variants", choices=VARIANTS, nargs="+", default=list(VARIANTS))
    parser.add_argument("--windows", type=int, nargs="+", choices=(1, 2, 4), default=[1, 2, 4])
    parser.add_argument("--sizes", nargs="+", default=["1400x900", "2560x1440"])
    parser.add_argument("--scales", type=int, nargs="+", default=[100, 200])
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--inputs", type=int, default=600)
    parser.add_argument("--idle-ms", type=int, default=10000)
    parser.add_argument("--lifecycle", type=int, default=0, help="Also measure this many open/close cycles per variant")
    parser.add_argument("--graph-hidden", action="store_true")
    parser.add_argument("--display", choices=("headless", "desktop"), default="headless" if platform.system() == "Linux" else "desktop")
    parser.add_argument("--max-load", type=float, help="Wait for the native one-minute CPU load average before each capture")
    parser.add_argument("--timing-mode", choices=("gpu", "clean"), default="gpu",
                        help="GPU timestamps or clean input/CPU validation without GPU instrumentation")
    parser.add_argument("--gpu", action="store_true", help="Also collect optional vendor process/device counters")
    args = parser.parse_args()
    if args.timing_mode == "clean" and args.gpu:
        parser.error("--gpu vendor sampling is incompatible with --timing-mode clean")
    if (args.pairs < 1 or args.inputs < 1 or args.lifecycle < 0 or args.idle_ms < 0
            or min(args.scales) <= 0 or args.max_load is not None and args.max_load <= 0):
        parser.error("pairs, inputs, scales and max-load must be positive; idle-ms and lifecycle must be nonnegative")
    args.output.mkdir(parents=True, exist_ok=False)
    args.binary = args.binary.resolve()
    repositories = [repo.resolve() for repo in args.repositories]
    identities = [(live.git(repo, "rev-parse", "HEAD").stdout, live.git(repo, "status", "--porcelain").stdout) for repo in repositories]
    digest = perf_metadata.sha256_file(args.binary)
    manifest = {"schema": 1, "binary_sha256": digest, "options": vars(args).copy(), "samples": [], "gates": {}, "complete": False, "harness_sha256": {name: perf_metadata.sha256_file(Path(__file__).with_name(name)) for name in ("gpu-matrix.py", "live-ui.py", "multi-window.py", "perf_gpu_frames.py")}}
    save = lambda: (args.output / "matrix.json").write_text(json.dumps(manifest, indent=2, default=str) + "\n", encoding="utf-8")
    save()
    try:
        for variant in args.variants:
            for count in args.windows:
                for dimensions in args.sizes:
                    size = tuple(map(int, dimensions.split("x")))
                    if len(size) != 2 or min(size) < 1:
                        raise ValueError("size must be positive WIDTHxHEIGHT")
                    for scale in args.scales:
                        key = f"{variant}-{count}w-{dimensions}-{scale}pct"
                        pairs = []
                        for pair in range(args.pairs):
                            results = {}
                            for side in ([0, 1] if pair % 2 == 0 else [1, 0]):
                                for repo, identity in zip(repositories, identities):
                                    if (live.git(repo, "rev-parse", "HEAD").stdout, live.git(repo, "status", "--porcelain").stdout) != identity:
                                        raise ValueError(f"fixture changed during capture: {repo}")
                                name = f"{key}-pair{pair + 1}-{'candidate' if side else 'baseline'}"
                                print(name, flush=True)
                                if args.max_load is not None:
                                    live.wait_for_quiet(args.max_load)
                                directory = args.output / name
                                result = live.run_once(args.binary, repositories[0], name, directory, 600,
                                    scenario_steps=multi.matrix(repositories, count, args.idle_ms, args.inputs),
                                    gpu=args.gpu, gpu_timings=args.timing_mode == "gpu", ping_ms=1000, display=args.display,
                                    window_size=size, ui_scale=scale, extra_env=environment(variant, side),
                                    session_overrides={"history_show_graph": not args.graph_hidden})
                                if not result["valid"] or result["binary_sha256"] != digest:
                                    raise ValueError(f"invalid run or binary changed: {directory}")
                                multi.verify_isolation(directory)
                                results[side] = result
                                manifest["samples"].append(dict(case=key, pair=pair + 1, candidate=bool(side), directory=str(directory), result=result))
                                save()
                            pairs.append((results[0], results[1]))
                        manifest["gates"][key] = evaluate(pairs, variant, args.timing_mode == "gpu")
                        save()
            if args.lifecycle:
                for side in (0, 1):
                    name = f"{variant}-lifecycle-{'candidate' if side else 'baseline'}"
                    if args.max_load is not None:
                        live.wait_for_quiet(args.max_load)
                    result = live.run_once(args.binary, repositories[0], name, args.output / name, 3600,
                        scenario_steps=multi.lifecycle(repositories[-1], args.lifecycle, args.idle_ms),
                        gpu=args.gpu, gpu_timings=args.timing_mode == "gpu", ping_ms=1000, display=args.display,
                        extra_env=environment(variant, side))
                    manifest["samples"].append(dict(case=name, result=result, retention=multi.retention(result, args.lifecycle)))
                    if not result["valid"]:
                        raise ValueError(f"invalid lifecycle run: {name}")
                    save()
        manifest["complete"] = True
    finally:
        save()


if __name__ == "__main__":
    main()
