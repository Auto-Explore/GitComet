#!/usr/bin/env python3
"""Windows native hover experiments with run-level A/A noise and paired gates.

Measure AA first/second (3+2 pairs), then AB first/second. Report all four
directories. No shipping setting changes; missing display timing blocks release
acceptance while local GPU/submission comparisons can still rank candidates.
"""
import argparse
from collections import Counter
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import time
import uuid

import perf_gpu_frames
import perf_metadata
import perf_workloads

live = perf_workloads.module("windows_hover_live", "live-ui.py")
BASELINE = {"GPUI_WINDOWS_RENDERER": "wgpu-dx12", "GPUI_GPU_EXPERIMENTS": "",
            "GPUI_GPU_DIAGNOSTIC_ABLATIONS": "",
            "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "auto", "GITCOMET_GPU_GRAPH_QUADS": "0",
            "GPUI_WINDOWS_DX12_PRESENT_MODE": "immediate"}
VARIANTS = {
    "fifo-dx12": {"GPUI_WINDOWS_DX12_PRESENT_MODE": "fifo"},
    "dx11": {"GPUI_WINDOWS_RENDERER": "dx11"},
    "crop": {"GPUI_GPU_EXPERIMENTS": "cropped-paths"},
    "keepalive-off": {"GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "damage-dx12": {"GPUI_GPU_EXPERIMENTS": "partial-redraw"},
    "damage-keepalive-dx12": {"GPUI_GPU_EXPERIMENTS": "partial-redraw", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "clipped-quads-dx12": {"GPUI_GPU_EXPERIMENTS": "clipped-quads"},
    "clipped-quads-dx11": {"GPUI_WINDOWS_RENDERER": "dx11", "GPUI_GPU_EXPERIMENTS": "clipped-quads"},
    "clipped-quads-keepalive-dx12": {"GPUI_GPU_EXPERIMENTS": "clipped-quads", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "clipped-quads-keepalive-dx11": {"GPUI_WINDOWS_RENDERER": "dx11", "GPUI_GPU_EXPERIMENTS": "clipped-quads", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "clipped-quads-damage-keepalive-dx12": {"GPUI_GPU_EXPERIMENTS": "clipped-quads,partial-redraw", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "clipped-quads-damage-keepalive-dx11": {"GPUI_WINDOWS_RENDERER": "dx11", "GPUI_GPU_EXPERIMENTS": "clipped-quads,partial-redraw", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "damage-dx11": {"GPUI_WINDOWS_RENDERER": "dx11", "GPUI_GPU_EXPERIMENTS": "partial-redraw"},
    "keepalive-off-dx11": {"GPUI_WINDOWS_RENDERER": "dx11", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "damage-keepalive-dx11": {"GPUI_WINDOWS_RENDERER": "dx11", "GPUI_GPU_EXPERIMENTS": "partial-redraw", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "crop-keepalive": {"GPUI_GPU_EXPERIMENTS": "cropped-paths", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
    "crop-damage-keepalive": {"GPUI_GPU_EXPERIMENTS": "cropped-paths,partial-redraw", "GPUI_WINDOWS_INPUT_RATE_KEEPALIVE": "off"},
}


def scenario(duration_ms=30000, interval_ms=8):
    repeat = math.ceil(duration_ms / interval_ms)
    steps = [{"do": "wait_ready", "timeout_ms": 180000, "indexed": True},
             {"do": "switch_window", "name": "primary"}, {"do": "settle", "ms": 10000}, {"do": "phase", "name": "hover"},
             {"do": "move_pointer", "positions": [[.05, .1], [.05, .9], [.05, .3], [.05, .7]],
              "repeat": repeat, "interval_ms": interval_ms, "hover_rows": True},
             {"do": "phase", "name": "settled"}, {"do": "settle", "ms": 5000}]
    return {"version": 1, "steps": steps}


def bootstrap(values, seed=20261010, samples=10000):
    """Resample independent paired runs, never correlated frames/PDH samples."""
    if not values or any(not math.isfinite(v) for v in values):
        return None
    rng = random.Random(seed)
    means = sorted(statistics.fmean(rng.choices(values, k=len(values))) for _ in range(samples))
    return {"mean": statistics.fmean(values), "low": means[int(samples*.025)],
            "high": means[min(samples-1, int(samples*.975))], "pairs": len(values), "seed": seed}


def run_errors(result, runtime, diagnostic):
    errors = list(result.get("problems", []))
    if not result.get("valid") or result.get("debug_assertions") is not False:
        errors.append("valid release capture required")
    if result.get("input_route") != "native":
        errors.append("native input capture required")
    geometries = result.get("native_window_geometries",{}).values()
    if not geometries or any(g.get("unavailable") or not g.get("window_dpi") or not g.get("client_width_px") or not g.get("client_height_px") for g in geometries):
        errors.append("actual native HWND physical dimensions and DPI unavailable")
    capabilities = [{"event": "renderer_capabilities", **r} for r in result.get("renderer_capabilities", [])]
    # Backend names are renderer-emitted; require requested DX11/DX12 spelling.
    backend = "d3d11" if runtime["GPUI_WINDOWS_RENDERER"] == "dx11" else "wgpu-dx12"
    errors += perf_gpu_frames.capability_errors(capabilities, runtime["GPUI_GPU_EXPERIMENTS"].split(",") if runtime["GPUI_GPU_EXPERIMENTS"] else (), backend, diagnostic)
    for capability in capabilities:
        if capability.get("active_diagnostic_ablations") != [] or runtime.get("GPUI_GPU_DIAGNOSTIC_ABLATIONS") != "":
            errors.append("formal captures require explicitly disabled diagnostic ablations")
        expected_present = "immediate" if backend == "d3d11" else runtime["GPUI_WINDOWS_DX12_PRESENT_MODE"]
        if capability.get("present_mode") != expected_present:
            errors.append("actual present mode capability does not match requested flag")
        if capability.get("windows_keepalive_policy") != runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"]:
            errors.append("Windows keepalive policy capability does not match requested flag")
        if runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"] == "off" and capability.get("windows_input_rate_keepalive") is not False:
            errors.append("keepalive-off request is unsupported or unresolved")
    phase = result.get("phases", {}).get("hover", {})
    hover = phase.get("hover", {})
    if hover.get("problems") or hover.get("painted_and_submitted", 0) < 20 or hover.get("unaccounted") or not hover.get("final_witness") or hover.get("native_acknowledged") != hover.get("requested"):
        errors.append("causal hover paint/submission coverage missing")
    if not phase.get("all_submissions", {}).get("count"):
        errors.append("all-present submission telemetry missing")
    if diagnostic:
        gpu = phase.get("gpu_frames", {})
        if gpu.get("coverage") != 1 or gpu.get("timed_frames", 0) < 20 or any(gpu.get(k) for k in
                ("missing_submission_records", "unexpected_submission_records", "duplicate_submission_records", "query_samples_dropped")):
            errors.append("complete GPU query coverage for every presentation required")
        if set(gpu.get("statuses", {})) != {"complete"} or gpu.get("records_dropped") != 0:
            errors.append("GPU timing error/drop status")
        if set(gpu.get("active_experiments", [])) != set(filter(None,runtime["GPUI_GPU_EXPERIMENTS"].split(","))):
            errors.append("GPU event active experiments do not match requested flags")
        expected_set=",".join(sorted(set(filter(None,runtime["GPUI_GPU_EXPERIMENTS"].split(",")))))
        if set(gpu.get("active_experiment_sets",{}))!={expected_set} or set(gpu.get("diagnostic_ablation_sets",{}))!={""}:
            errors.append("every GPU event must confirm actual flags and disabled diagnostic ablations")
        if "partial-redraw" in set(filter(None, runtime["GPUI_GPU_EXPERIMENTS"].split(","))):
            partial = gpu.get("damage_kinds", {}).get("partial")
            peak = gpu.get("allocation_bytes", {}).get("retained_bytes", {}).get("peak")
            if not isinstance(partial, int) or isinstance(partial, bool) or partial <= 0:
                errors.append("retained hover optimization requires observed Partial frames")
            if not isinstance(peak, (int, float)) or isinstance(peak, bool) or not math.isfinite(peak) or peak <= 0:
                errors.append("retained hover optimization requires positive measured retained allocation peak")
        if result.get("gpu_capture", {}).get("status") not in ("available", "partial"):
            errors.append("PDH process GPU occupancy unavailable")
        app_gpu = phase.get("gpu", {}).get("processes", {}).get("app", {})
        if occupancy(result) is None:
            errors.append("PDH phase has no finite occupancy for its observed app engine adapters")
        if not app_gpu or not any(g.get("busiest_engine_percent", {}).get("count", 0) >= 25 and
                g.get("busiest_engine_percent", {}).get("covered_ms", 0) >= phase.get("seconds", 0)*800 for g in app_gpu.values()):
            errors.append("PDH requires at least 25 fully contained valid intervals and 80 percent phase coverage")
    else:
        if result.get("gpu_timings_requested") or result.get("gpu_capture", {}).get("requested"):
            errors.append("clean latency capture includes GPU observers")
        if hover.get("latency_origin") != "SendInput QPC":
            errors.append("native dispatch QPC anchor missing")
    return sorted(set(errors))


def metric(result, mode, quantile=None):
    phase = result.get("phases", {}).get("hover", {})
    return phase.get("gpu_frames", {}).get("render_elapsed_ms_per_second") if mode == "diagnostic" else phase.get("hover", {}).get("native_dispatch_to_submit_ms", {}).get(quantile)


def occupancy(result, role="app"):
    gpu = result.get("phases", {}).get("hover", {}).get("gpu", {})
    adapters = gpu.get("devices", {}) if role == "device" else gpu.get("processes", {}).get(role, {})
    values=[]
    for group in adapters.values():
        value=group.get("busiest_engine_percent",{}).get("duration_weighted_mean")
        observed=group.get("engine_instances_observed")
        if observed==0 or (observed is None and value is None):
            continue
        values.append(value)
    return max(values) if values and all(isinstance(v,(float,int)) and math.isfinite(v) for v in values) else None


def occupancy_scopes(result):
    """Independent adapter maxima, never sums across processes or adapters."""
    return {role: occupancy(result, role) for role in ("app", "desktop_compositor", "device")}


def gpu_gate(aa_pairs, ab_pairs, read_metric, seed):
    reasons, ratios, session_ratios = [], [], {}
    positive = lambda value: isinstance(value, (float,int)) and math.isfinite(value) and value > 0
    aa = [math.log(read_metric(c) / read_metric(b)) for _, b, c in aa_pairs
          if positive(read_metric(b)) and positive(read_metric(c))]
    if len(aa) != 5 or len(ab_pairs) != 5:
        reasons.append("five independent AA and AB diagnostic pairs required")
    noise = max((abs(v) for v in aa), default=None)
    for session, baseline, candidate in ab_pairs:
        b, c = read_metric(baseline), read_metric(candidate)
        if not positive(b) or not positive(c):
            reasons.append("GPU comparison metric unavailable")
            continue
        ratio = math.log(c/b)
        ratios.append(ratio)
        session_ratios.setdefault(session, []).append(ratio)
    ci = bootstrap(ratios, seed)
    if not ci or ci["high"] >= 0 or noise is None or -ci["mean"] <= noise:
        reasons.append("GPU gain does not exceed AA noise with paired 95% confidence")
    if Counter(session for session,_,_ in ab_pairs) != {"first":3,"second":2} or any(statistics.fmean(v) >= 0 for v in session_ratios.values()):
        reasons.append("GPU gain must repeat in both independent sessions")
    return {"status": "needs_review" if reasons else "local_gpu_and_submit_gates_passed",
            "reasons": reasons, "paired_log_ratio_ci95": ci,
            "ratio": math.exp(ci["mean"]) if ci else None,
            "aa_max_abs_log_ratio": noise, "display_validation": "pending",
            "shipping_default": "unchanged_pending_display_and_correctness_validation"}


def evaluate(aa_pairs, ab_pairs, seed=20261010):
    elapsed = gpu_gate(aa_pairs, ab_pairs, lambda r:metric(r,"diagnostic"), seed)
    pdh = gpu_gate(aa_pairs, ab_pairs, occupancy, seed)
    reasons = ["render elapsed: "+r for r in elapsed["reasons"]]+["PDH occupancy: "+r for r in pdh["reasons"]]
    return {"status":"needs_review" if reasons else "local_gpu_and_submit_gates_passed",
            "reasons":reasons,"render_elapsed":elapsed,"pdh_busiest_engine":pdh,
            "render_elapsed_ratio":elapsed["ratio"],"display_validation":"pending",
            "shipping_default":"unchanged_pending_display_and_correctness_validation"}


def latency_gate(pairs, seed=20261010):
    reasons, metrics = [], {}
    if Counter(session for session,_,_ in pairs) != {"first":3,"second":2}:
        reasons.append("five clean pairs across two sessions required")
    for quantile, tolerance, absolute in (("p95", 1, 50), ("p99", 2, 100)):
        residuals, candidate_values = [], []
        for _, baseline, candidate in pairs:
            b, c = metric(baseline, "clean", quantile), metric(candidate, "clean", quantile)
            if b is None or c is None or not math.isfinite(b) or not math.isfinite(c) or min(b,c)<0:
                reasons.append(f"native dispatch-to-submit {quantile} unavailable")
                continue
            residuals.append(c-b-max(b*.05, tolerance))
            candidate_values.append(c)
        ci = bootstrap(residuals, seed)
        metrics[quantile] = ci
        if not ci or ci["high"] > 0 or max(candidate_values, default=math.inf) > absolute:
            reasons.append(f"immediate feedback {quantile} guard failed")
    return {"passed": not reasons, "reasons": reasons, "residual_ci95_ms": metrics}


def report(directories, seed=20261010):
    sessions = [json.loads((Path(d)/"matrix.json").read_text(encoding="utf-8")) for d in directories]
    if not all(s.get("complete") for s in sessions) or len({s["id"] for s in sessions}) != len(sessions):
        raise ValueError("require complete independent stage sessions")
    if len({s["identity"] for s in sessions}) != 1:
        raise ValueError("binary/source/fixture/workload identity differs")
    if {s["stage"] for s in sessions} != {"aa", "ab"} or {s["session"] for s in sessions} != {"first", "second"}:
        raise ValueError("require AA and AB in first/second sessions")
    if max(s["finished_unix"] for s in sessions if s["stage"] == "aa") > min(s["started_unix"] for s in sessions if s["stage"] == "ab"):
        raise ValueError("finish AA noise calibration before any AB measurement")
    samples = [r for s in sessions for r in s["samples"]]
    for row in samples:
        errors = run_errors(row["result"], row["runtime"], row["mode"] == "diagnostic")
        if errors or row.get("gate_errors"):
            raise ValueError(f"invalid archived capture: {errors or row['gate_errors']}")
    environments = [s.get("environment", {}) for s in sessions]
    environment_differences = [perf_metadata.compare(environments[0], env) for env in environments[1:]]
    if any(d["invalidating"] or d["source_differs"] for d in environment_differences):
        raise ValueError(f"session environment/source invariants differ: {environment_differences}")
    geometries = [sorted(r["result"].get("native_window_geometries",{}).values(),key=lambda g:json.dumps(g,sort_keys=True)) for r in samples]
    if not geometries or any(g != geometries[0] for g in geometries):
        raise ValueError("actual window DPI/physical client geometry differs")
    def pairs(stage, variant, mode):
        grouped = {}
        for r in samples:
            if (r["stage"], r["variant"], r["mode"]) == (stage, variant, mode):
                grouped.setdefault((r["session"], r["pair"]), {})[r["side"]] = r["result"]
        if any(set(p) != {"baseline", "candidate"} for p in grouped.values()):
            raise ValueError("incomplete pair")
        return [(session, p["baseline"], p["candidate"]) for (session, _), p in sorted(grouped.items())]
    gates = {}
    occupancy_context = {}
    for variant in sorted({r["variant"] for r in samples if r["stage"] == "ab"}):
        gate = evaluate(pairs("aa", "aa", "diagnostic"), pairs("ab", variant, "diagnostic"), seed)
        gate["latency"] = latency_gate(pairs("ab", variant, "clean"), seed)
        if not gate["latency"]["passed"]:
            gate["status"] = "needs_review"
            gate["reasons"] += gate["latency"]["reasons"]
        gates[variant] = gate
        diagnostic_pairs = pairs("ab", variant, "diagnostic")
        occupancy_context[variant] = {}
        for role in ("app", "desktop_compositor", "device"):
            values = {side: [occupancy(p[index], role) for p in diagnostic_pairs]
                      for side, index in (("baseline", 1), ("candidate", 2))}
            occupancy_context[variant][role] = {
                side: {"mean_percent": statistics.fmean(valid) if valid else None,
                       "valid_runs": len(valid), "missing_runs": len(rows)-len(valid),
                       "complete": bool(rows) and len(valid)==len(rows)}
                for side, rows in values.items() for valid in [[v for v in rows if v is not None]]}
    ranked = sorted((k for k,g in gates.items() if g["status"] == "local_gpu_and_submit_gates_passed"), key=lambda k:gates[k]["render_elapsed_ratio"])
    context_rankings = {role: sorted(
        (variant for variant, values in occupancy_context.items() if values[role]["candidate"]["complete"]),
        key=lambda variant: occupancy_context[variant][role]["candidate"]["mean_percent"])
        for role in ("app", "desktop_compositor", "device")}
    return {"schema": 1, "gates": gates, "local_candidate_ranking": ranked,
            "pdh_occupancy_context": occupancy_context,
            "pdh_occupancy_context_rankings": context_rankings,
            "environment_differences": environment_differences,
            "display_validation_pending": True, "accepted_shipping_candidates": [],
            "note": "PDH busiest engine corroborates occupancy only. App occupancy remains the acceptance counter. DWM and whole-device rankings are descriptive context affected by other desktop work; they do not attribute savings to GitComet. Query elapsed across all presents ranks local render work; ETW execution union and displayed latency remain required."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    command = parser.add_subparsers(dest="command", required=True)
    measure = command.add_parser("measure")
    for name in ("binary", "repository", "output"):
        measure.add_argument("--"+name, type=Path, required=True)
    measure.add_argument("--stage", choices=("aa", "ab"), required=True)
    measure.add_argument("--session", choices=("first", "second"), required=True)
    measure.add_argument("--variants", nargs="+", choices=VARIANTS, default=["dx11", "crop", "keepalive-off", "damage-dx12", "damage-dx11", "keepalive-off-dx11", "damage-keepalive-dx11", "fifo-dx12"])
    measure.add_argument("--duration-ms", type=int, default=30000)
    measure.add_argument("--interval-ms", type=int, default=8)
    measure.add_argument("--window-size", type=int, nargs=2, default=(1400,900))
    measure.add_argument("--ui-scale", type=int, default=100)
    measure.add_argument("--list", action="store_true")
    summary = command.add_parser("report")
    summary.add_argument("directories", nargs="+", type=Path)
    summary.add_argument("--output", type=Path)
    summary.add_argument("--seed", type=int, default=20261010)
    args = parser.parse_args()
    if args.command == "report":
        result = report(args.directories, args.seed)
        data = json.dumps(result, indent=2)+"\n"
        if args.output:
            args.output.write_text(data, encoding="utf-8")
        print(data)
        return
    if min(args.duration_ms, args.interval_ms, args.ui_scale, *args.window_size) <= 0:
        parser.error("durations, interval and scale must be positive")
    schedule = scenario(args.duration_ms, args.interval_ms)
    variants = ["aa"] if args.stage == "aa" else args.variants
    count = 3 if args.session == "first" else 2
    if args.list:
        print(json.dumps({"pairs": count, "variants": variants, "scenario": schedule,
                          "modes": ["diagnostic", "clean"], "native_input": True}, indent=2))
        return
    if platform.system() != "Windows":
        parser.error("this matrix requires a Windows desktop")
    binary, repository, output = args.binary.resolve(strict=True), args.repository.resolve(strict=True), args.output.resolve()
    fixture_head = subprocess.check_output(["git","-C",str(repository),"rev-parse","HEAD"], text=True).strip()
    fixture_dirty = subprocess.check_output(["git","-C",str(repository),"status","--porcelain"], text=True)
    digest = perf_metadata.sha256_file(binary)
    source = perf_metadata.source_info()
    harness = {name:perf_metadata.sha256_file(Path(__file__).with_name(name)) for name in
            ("windows-hover.py","live-ui.py","perf_hover.py","perf_gpu_frames.py","perf_gpu.py","perf_gpu_windows.py","perf_native_input.py","perf_metadata.py","perf_windows_environment.py","windows-hover-occupancy.py")}
    identity = hashlib.sha256(json.dumps({"binary":digest,"source":source,"fixture_head":fixture_head,
                     "fixture_dirty":fixture_dirty,"scenario":schedule,"size":args.window_size,"scale":args.ui_scale,"harness":harness},sort_keys=True,default=str).encode()).hexdigest()
    output.mkdir(parents=True,exist_ok=False)
    manifest = {"schema":1,"id":str(uuid.uuid4()),"identity":identity,"stage":args.stage,"session":args.session,
                "source":source,"harness_sha256":harness,"binary_sha256":digest,"repository_head":fixture_head,"scenario":schedule,
                "started_unix":time.time(),"samples":[],"complete":False,
                "environment":perf_metadata.collect(binaries=[("frozen",binary)],fixtures=[("history",repository)]),
                "unavailable_environment_fields":["GPU clocks","GPU thermals","HAGS state","VRR state"],
                "display_capture":{"status":"not_collected","reason":"PresentMon/ETW requires an authorized native collector; missing display latency never passes release gates"}}
    def save():
        (output/"matrix.json").write_text(json.dumps(manifest,indent=2,default=str)+"\n",encoding="utf-8")
        (output/"environment.json").write_text(json.dumps(manifest["environment"],indent=2,default=str)+"\n",encoding="utf-8")
        geometry = [r["result"].get("native_window_geometries",{}) for r in manifest["samples"]]
        (output/"environment.txt").write_text(perf_metadata.readable(manifest["environment"])+"\nMeasured application window geometry:\n"+json.dumps(geometry,indent=2)+"\n",encoding="utf-8")
    try:
        for variant in variants:
            for pair in range(count):
                order = ["baseline","candidate"] if (pair+(args.session=="second"))%2==0 else ["candidate","baseline"]
                for mode in ("diagnostic","clean"):
                    for side in order:
                        current_head=subprocess.check_output(["git","-C",str(repository),"rev-parse","HEAD"],text=True).strip()
                        if perf_metadata.sha256_file(binary)!=digest or perf_metadata.source_info()!=source or current_head!=fixture_head or subprocess.check_output(["git","-C",str(repository),"status","--porcelain"],text=True)!=fixture_dirty:
                            raise ValueError("binary or fixture changed during session")
                        runtime = dict(BASELINE, **(VARIANTS[variant] if args.stage=="ab" and side=="candidate" else {}))
                        destination=output/f"{variant}-{pair+1}-{mode}-{side}"
                        print(f"{destination.name}: starting",flush=True)
                        result=live.run_once(binary,repository,"history-hover",destination,240,metadata=False,
                                display="desktop",ping_ms=200,extra_env=runtime,scenario_steps=schedule,
                                ui_scale=args.ui_scale,window_size=args.window_size,gpu=mode=="diagnostic",
                                gpu_timings=mode=="diagnostic",input_route="native")
                        problems=run_errors(result,runtime,mode=="diagnostic")
                        manifest["samples"].append({"stage":args.stage,"session":args.session,"variant":variant,
                                "pair":pair+1,"mode":mode,"side":side,"runtime":runtime,"path":str(destination),"result":result,"gate_errors":problems})
                        save()
                        if problems:
                            raise ValueError(f"invalid capture {destination}: {problems}")
        manifest.update(complete=True,finished_unix=time.time())
    finally:
        save()


if __name__ == "__main__":
    main()
