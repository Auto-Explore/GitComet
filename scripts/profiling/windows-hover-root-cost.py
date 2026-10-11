#!/usr/bin/env python3
"""Windows renderer root-cost screening; writes captures only to --output.

Single-run rankings are diagnostic evidence, never formal/default acceptance.
No displayed latency or ETW execution-union metric is supplied by this runner.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/"scripts/profiling"))
import perf_gpu_frames
import perf_metadata
import perf_workloads

matrix = perf_workloads.module("root_cause_matrix","windows-hover.py")
live = matrix.live
VARIANTS = {
    "baseline-dx12": ({}, {}, 1400),
    "dx11": ({"GPUI_WINDOWS_RENDERER":"dx11"}, {}, 1400),
    "keepalive-off-dx12": ({"GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "keepalive-off-dx11": ({"GPUI_WINDOWS_RENDERER":"dx11","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "damage-dx12": ({"GPUI_GPU_EXPERIMENTS":"partial-redraw"}, {}, 1400),
    "damage-dx11": ({"GPUI_WINDOWS_RENDERER":"dx11","GPUI_GPU_EXPERIMENTS":"partial-redraw"}, {}, 1400),
    "crop-dx12": ({"GPUI_GPU_EXPERIMENTS":"cropped-paths"}, {}, 1400),
    "crop-keepalive-dx12": ({"GPUI_GPU_EXPERIMENTS":"cropped-paths","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "crop-damage-keepalive-dx12": ({"GPUI_GPU_EXPERIMENTS":"cropped-paths,partial-redraw","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "clipped-quads-dx12": ({"GPUI_GPU_EXPERIMENTS":"clipped-quads"}, {}, 1400),
    "clipped-quads-dx11": ({"GPUI_WINDOWS_RENDERER":"dx11","GPUI_GPU_EXPERIMENTS":"clipped-quads"}, {}, 1400),
    "clipped-quads-keepalive-dx12": ({"GPUI_GPU_EXPERIMENTS":"clipped-quads","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "clipped-quads-keepalive-dx11": ({"GPUI_WINDOWS_RENDERER":"dx11","GPUI_GPU_EXPERIMENTS":"clipped-quads","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "clipped-quads-damage-keepalive-dx12": ({"GPUI_GPU_EXPERIMENTS":"clipped-quads,partial-redraw","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "clipped-quads-damage-keepalive-dx11": ({"GPUI_WINDOWS_RENDERER":"dx11","GPUI_GPU_EXPERIMENTS":"clipped-quads,partial-redraw","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "combined-dx12": ({"GPUI_GPU_EXPERIMENTS":"partial-redraw","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "combined-dx11": ({"GPUI_WINDOWS_RENDERER":"dx11","GPUI_GPU_EXPERIMENTS":"partial-redraw","GPUI_WINDOWS_INPUT_RATE_KEEPALIVE":"off"}, {}, 1400),
    "graph-hidden-dx12": ({}, {"history_show_graph":False}, 1400),
    "width700-dx12": ({}, {}, 700),
    "width1400-dx12": ({}, {}, 1400),
    "fifo-dx12": ({"GPUI_WINDOWS_DX12_PRESENT_MODE":"fifo"}, {}, 1400),
}
NORMAL_DEFAULTS=[k for k in VARIANTS if k!="fifo-dx12"]
ABLATIONS={"no-mono-glyphs":"no-mono-glyphs","no-subpixel-glyphs":"no-subpixel-glyphs",
           "no-glyphs":"no-mono-glyphs,no-subpixel-glyphs","no-shadows":"no-shadows",
           "no-paths":"no-paths","no-quads":"no-quads","no-images":"no-images","clear-only":"clear-only"}
for backend in ("dx12","dx11"):
    for name,flags in ABLATIONS.items():
        VARIANTS[f"{name}-{backend}"]=({"GPUI_GPU_DIAGNOSTIC_ABLATIONS":flags,
             "GPUI_WINDOWS_RENDERER":"dx11" if backend=="dx11" else "wgpu-dx12"}, {}, 1400)


def checkout_identity(path):
    """Freeze separate dependency checkout, including untracked content."""
    path=Path(path).resolve(strict=True)
    def git(*args):
        return subprocess.check_output(["git","-C",str(path),*args])
    names=[name.decode("utf-8") for name in git("ls-files","--others","--exclude-standard","-z").split(b"\0") if name]
    return {"path":str(path),"head":git("rev-parse","HEAD").decode().strip(),
            "patch_sha256":hashlib.sha256(git("diff","HEAD","--binary")).hexdigest(),
            "untracked_sha256":{name:perf_metadata.sha256_file(path/name) for name in names}}


def declared_dependency(path, expected_head):
    if not expected_head or not re.fullmatch(r"[0-9a-fA-F]{40}",expected_head):
        raise ValueError("--expected-gpui-head must declare the full 40-character framework build revision")
    identity = checkout_identity(path)
    if identity["head"] != expected_head.lower():
        raise ValueError("GPUI checkout HEAD differs from the declared binary build revision")
    return identity


def plan(args):
    schedule=matrix.scenario(args.duration_ms,8)
    settles=[s for s in schedule["steps"] if s["do"]=="settle"]
    settles[0]["ms"],settles[-1]["ms"]=args.warmup_ms,args.tail_ms
    return schedule


def screen_errors(result,runtime,mode,short,records=(),input_route="native"):
    diagnostic=mode=="diagnostic"
    errors=list(result.get("problems",[]))
    if not result.get("valid") or result.get("debug_assertions") is not False or result.get("input_route")!=input_route:
        errors.append(f"valid release {input_route} capture required")
    geometries=result.get("native_window_geometries",{}) if input_route=="native" else result.get("window_geometries",{})
    if not geometries or any(g.get("unavailable") or not g.get("window_dpi") or not g.get("client_width_px") or not g.get("client_height_px") for g in geometries.values()):
        errors.append("actual HWND physical dimensions and DPI unavailable")
    capabilities=result.get("renderer_capabilities",[])
    backend="d3d11" if runtime["GPUI_WINDOWS_RENDERER"]=="dx11" else "wgpu-dx12"
    errors+=perf_gpu_frames.capability_errors([{"event":"renderer_capabilities",**r} for r in capabilities],
        list(filter(None,runtime["GPUI_GPU_EXPERIMENTS"].split(","))),backend,diagnostic)
    for capability in capabilities:
        if capability.get("windows_keepalive_policy")!=runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"]:
            errors.append("Windows keepalive policy capability does not match requested flag")
        if runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"]=="off" and capability.get("windows_input_rate_keepalive") is not False:
            errors.append("keepalive-off request is unsupported or unresolved")
        expected_present="immediate" if backend=="d3d11" else runtime["GPUI_WINDOWS_DX12_PRESENT_MODE"]
        if capability.get("present_mode")!=expected_present:
            errors.append("actual present mode capability does not match requested flag")
        if not diagnostic and capability.get("timing_enabled") is not False:
            errors.append("capture reports GPU timestamps enabled or unknown")
    phase=result.get("phases",{}).get("hover",{})
    hover=phase.get("hover",{})
    if hover.get("problems") or hover.get("painted_and_submitted",0)<20 or hover.get("unaccounted") or not hover.get("final_witness"):
        errors.append("structural hover scene/submission coverage missing")
    if input_route=="native" and (not hover.get("requested") or hover.get("native_acknowledged")!=hover.get("requested")):
        errors.append("complete native input ACK coverage missing")
    if not phase.get("all_submissions",{}).get("count"):
        errors.append("all-present submission telemetry missing")
    if diagnostic:
        gpu=phase.get("gpu_frames",{})
        if gpu.get("coverage")!=1 or gpu.get("timed_frames",0)<20 or any(gpu.get(k) for k in
                ("missing_submission_records","unexpected_submission_records","duplicate_submission_records","query_samples_dropped")):
            errors.append("complete GPU query coverage for every presentation required")
        if set(gpu.get("statuses",{}))!={"complete"} or gpu.get("records_dropped")!=0:
            errors.append("GPU timing error/drop status")
        if result.get("gpu_timings_requested") is not True:
            errors.append("diagnostic GPU timestamps not requested")
    elif result.get("gpu_timings_requested") or any(r.get("event")=="gpu_frame" for r in records):
        errors.append("capture unexpectedly includes GPU timestamp observers")
    if mode in ("diagnostic","occupancy"):
        if result.get("gpu_capture",{}).get("requested") is not True or result.get("gpu_capture",{}).get("status") not in ("available","partial"):
            errors.append("PDH process GPU occupancy unavailable")
        app_gpu=phase.get("gpu",{}).get("processes",{}).get("app",{})
        if matrix.occupancy(result) is None:
            errors.append("PDH phase has no finite occupancy for its observed app engine adapters")
        if not any(g.get("busiest_engine_percent",{}).get("count",0)>=25 and g.get("busiest_engine_percent",{}).get("covered_ms",0)>=phase.get("seconds",0)*800 for g in app_gpu.values()):
            errors.append("PDH requires at least 25 fully contained valid intervals and 80 percent phase coverage")
    else:
        if input_route!="native" or hover.get("latency_origin")!="SendInput QPC":
            errors.append("native dispatch QPC anchor required for latency screening")
        if result.get("gpu_capture",{}).get("requested"):
            errors.append("clean latency capture includes PDH observers")
    requested=set(filter(None,runtime.get("GPUI_GPU_DIAGNOSTIC_ABLATIONS","").split(",")))
    requested_experiments=set(filter(None,runtime.get("GPUI_GPU_EXPERIMENTS","").split(",")))
    if requested and "partial-redraw" in requested_experiments:
        errors.append("root-cost controls cannot combine ablations with bypassed retained damage")
    for capability in capabilities:
        if not requested.issubset(capability.get("implemented_diagnostic_ablations",[])):
            errors.append("requested diagnostic ablation is unsupported")
        if set(capability.get("active_diagnostic_ablations",["unreported"]))!=requested:
            errors.append("actual diagnostic ablations do not match requested flags")
    expected_debug=[]
    if short:
        expected_debug=[e for e in errors if e.startswith("PDH requires")]
        errors=[e for e in errors if e not in expected_debug]
    if diagnostic:
        phase=result.get("phases",{}).get("hover",{})
        gpu=phase.get("gpu_frames",{})
        if not capabilities or not all(c.get("per_pass_timestamps") for c in capabilities):
            errors.append("per-pass timestamp capability required for root-cost split")
        if gpu.get("pass_timed_frames")!=gpu.get("timed_frames") or not gpu.get("pass_ms"):
            errors.append("per-pass timing coverage missing")
        gpu_records=[r for r in records if r.get("event")=="gpu_frame"]
        if not gpu_records or any(set(r.get("experiments",["unreported"]))!=requested_experiments for r in gpu_records):
            errors.append("GPU event active experiment context missing or mismatched")
        if not gpu_records or any(set(r.get("diagnostic_ablations",["unreported"]))!=requested for r in gpu_records):
            errors.append("GPU event diagnostic ablation context missing or mismatched")
        if requested and (set(gpu.get("damage_kinds",{}))!={"full"} or any(r.get("retained_bytes",0) for r in gpu_records)):
            errors.append("diagnostic ablations must bypass retained damage/resources")
        if requested=={"clear-only"} and set(gpu.get("pass_ms",{}))!={"clear_only"}:
            errors.append("clear-only requires its explicit clear_only pass timing")
    return sorted(set(errors)),expected_debug


def extract(result,ablation=False,input_route="native"):
    phase=result.get("phases",{}).get("hover",{})
    gpu=phase.get("gpu_frames",{})
    seconds=phase.get("seconds",0)
    passes={name:{"elapsed_ms_per_second":d.get("mean")*d.get("count",0)/seconds if d.get("mean") is not None and seconds else None,
                  "mean_ms_per_present":d.get("mean"),"p95_ms_per_present":d.get("p95"),"frames":d.get("count")}
            for name,d in gpu.get("pass_ms",{}).items()}
    geometry=next(iter(result.get("native_window_geometries" if input_route=="native" else "window_geometries",{}).values()),{})
    hover=phase.get("hover",{})
    if ablation or input_route!="native":
        # Scene callback/submit joins remain useful structurally, but cannot
        # prove a skipped primitive reached the framebuffer or display.
        hover={k:v for k,v in hover.items() if not k.endswith("_ms") and k not in ("samples","latency_origin")}
        hover.update(pixel_witness=False,latency_validation="ineligible: rendered pixels deliberately omitted" if ablation else "ineligible: synthetic renderer-isolation input")
    return {"gpu_render_elapsed_ms_per_second":gpu.get("render_elapsed_ms_per_second"),
            "app_pdh_busiest_engine_percent":matrix.occupancy(result),
            "dwm_pdh_busiest_engine_percent":matrix.occupancy(result,"desktop_compositor"),
            "device_pdh_busiest_engine_percent":matrix.occupancy(result,"device"),
            "surface_alpha_modes":perf_gpu_frames.optional_labels(
                [perf_gpu_frames.capability_payload(r) for r in result.get("renderer_capabilities",[])],"surface_alpha_mode"),
            "damage_fallback_reasons":gpu.get("damage_fallback_reasons") if result.get("gpu_timings_requested") else None,
            "gpu_execution_union_ms_per_second":None,"physical_window":geometry,
            "physical_client_pixels":geometry.get("client_width_px",0)*geometry.get("client_height_px",0),
            "fresh_draws_per_second":phase.get("frames_per_second"),"all_submissions":phase.get("all_submissions"),
            "gpu_query_status":"requested" if result.get("gpu_timings_requested") else "not_requested",
            "quad_draw_calls":gpu.get("quad_draw_calls") if result.get("gpu_timings_requested") else None,
            "quad_scissor_splits":gpu.get("quad_scissor_splits") if result.get("gpu_timings_requested") else None,
            "gpu_coverage":{k:gpu.get(k) for k in ("all_submission_records","recorded_frames","timed_frames","pass_timed_frames","coverage","missing_submission_records","unexpected_submission_records","duplicate_submission_records","query_samples_dropped","records_dropped","statuses","damage_kinds","pass_count")},
            "causal_hover_structure" if ablation or input_route!="native" else "causal_hover":hover,"per_pass":passes if result.get("gpu_timings_requested") else None,
            "process_gpu_and_memory":phase.get("gpu"),"rss_kib":phase.get("rss_kib"),"private_kib":phase.get("private_kib")}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("binary","repository","output"):
        parser.add_argument("--"+name,type=Path,required=True)
    parser.add_argument("--gpui-path",type=Path,default=Path("C:/git/gpui-ce-windows-hover"))
    parser.add_argument("--expected-gpui-head",help="Full framework revision declared when building the frozen binary; required for captures")
    parser.add_argument("--input-route",choices=("native","synthetic"),default="native")
    parser.add_argument("--variants",nargs="+",choices=VARIANTS,default=NORMAL_DEFAULTS)
    parser.add_argument("--duration-ms",type=int,default=30000)
    parser.add_argument("--warmup-ms",type=int,default=10000)
    parser.add_argument("--tail-ms",type=int,default=5000)
    parser.add_argument("--height",type=int,default=900)
    parser.add_argument("--ui-scale",type=int,default=100)
    parser.add_argument("--repeats",type=int,default=1)
    parser.add_argument("--mode",choices=("diagnostic","occupancy","clean"),default="diagnostic")
    parser.add_argument("--reverse",action="store_true")
    parser.add_argument("--continue-on-invalid",action="store_true")
    parser.add_argument("--list",action="store_true")
    args=parser.parse_args()
    if min(args.duration_ms,args.warmup_ms,args.tail_ms,args.height,args.ui_scale,args.repeats)<=0:
        parser.error("durations, size, scale and repeats must be positive")
    if args.duration_ms%8:
        parser.error("hover duration must be a multiple of 8 ms")
    if args.mode=="clean" and any(VARIANTS[name][0].get("GPUI_GPU_DIAGNOSTIC_ABLATIONS") for name in args.variants):
        parser.error("visually incomplete ablations are diagnostic occupancy controls and cannot enter latency acceptance")
    if args.mode=="clean" and args.input_route!="native":
        parser.error("synthetic renderer-isolation controls cannot enter latency screening")
    schedule=plan(args)
    plans=[{"name":name,"runtime":dict(matrix.BASELINE,**{"GPUI_GPU_DIAGNOSTIC_ABLATIONS":"",**VARIANTS[name][0]}),
            "evidence_scope":"synthetic_renderer_isolation_control" if args.input_route=="synthetic" else "visually_incomplete_diagnostic_control" if VARIANTS[name][0].get("GPUI_GPU_DIAGNOSTIC_ABLATIONS") else "render_complete_candidate_screen",
            "session_overrides":VARIANTS[name][1],"window_size":[VARIANTS[name][2],args.height]} for name in args.variants]
    if args.list:
        print(json.dumps({"plans":plans,"scenario":schedule,"mode":args.mode,"input_route":args.input_route},indent=2))
        return
    binary,repository,output=args.binary.resolve(strict=True),args.repository.resolve(strict=True),args.output.resolve()
    dependency=declared_dependency(args.gpui_path,args.expected_gpui_head)
    output.mkdir(parents=True,exist_ok=False)
    environment=perf_metadata.collect(binaries=[("frozen",binary)],fixtures=[("history",repository)])
    source=environment["source"]
    binary_hash=perf_metadata.sha256_file(binary)
    fixture_head=live.git(repository,"rev-parse","HEAD").stdout
    fixture_status=live.git(repository,"status","--porcelain=v1","-z").stdout
    manifest={"schema":2,"complete":False,"started_unix":time.time(),"environment":environment,"gpui_source":dependency,
              "declared_binary_gpui_head":args.expected_gpui_head.lower(),
              "dependency_provenance":"Caller declares the frozen binary build revision; matching checkout identity and content hashes are frozen. This runner does not attest the compiler's embedded dependency provenance.",
              "pilot_script_sha256":perf_metadata.sha256_file(__file__),
              "mode":args.mode,"input_route":args.input_route,"scenario":schedule,"plans":plans,"samples":[],"rankings":{},"diagnostic_ablation_rankings":{},"synthetic_experimental_rankings":{},
              "kind":"short_protocol_debug" if args.duration_ms<30000 else "single_run_root_cost_screen",
              "accepted_shipping_candidates":[],"display_validation_pending":True,
              "limitation":"Query render elapsed includes gaps and is not GPU execution union or engine occupancy. PDH corroborates occupancy. Probe/scenario/trace/timestamp observers remain enabled in diagnostic captures; occupancy retains native scenario/probes/all submissions and PDH but disables GPU timestamps; clean mode retains causal probes without GPU observers. Quantify observer overhead with windows-hover-occupancy.py. Single-run rankings do not replace five-pair A/A+A/B or displayed latency/correctness validation. Ablations intentionally omit pixels, so their scene callback/submit witnesses are structural only and never prove hover pixels or latency acceptance."}
    (output/"environment.json").write_text(json.dumps(environment,indent=2)+"\n",encoding="utf-8")
    (output/"environment.txt").write_text(perf_metadata.readable(environment)+"\nGPUI source:\n"+json.dumps(dependency,indent=2)+"\n",encoding="utf-8")
    def save():
        for metric in ("gpu_render_elapsed_ms_per_second","app_pdh_busiest_engine_percent",
                       "dwm_pdh_busiest_engine_percent","device_pdh_busiest_engine_percent"):
            for scope,destination in (("render_complete_candidate_screen","rankings"),("visually_incomplete_diagnostic_control","diagnostic_ablation_rankings"),("synthetic_renderer_isolation_control","synthetic_experimental_rankings")):
                manifest[destination][metric]=[{"name":r["name"],"repeat":r["repeat"],"value":r["metrics"][metric]} for r in sorted(manifest["samples"],key=lambda r:r["metrics"][metric] if r["metrics"][metric] is not None else float("inf")) if r["evidence_scope"]==scope and not r["gate_errors"] and r["metrics"][metric] is not None]
        (output/"pilot.json").write_text(json.dumps(manifest,indent=2)+"\n",encoding="utf-8")
    def verify_frozen():
        if perf_metadata.source_info()!=source or checkout_identity(args.gpui_path)!=dependency or perf_metadata.sha256_file(binary)!=binary_hash or live.git(repository,"rev-parse","HEAD").stdout!=fixture_head or live.git(repository,"status","--porcelain=v1","-z").stdout!=fixture_status:
            raise ValueError("frozen source/dependency/binary/fixture changed")
    try:
        for repeat in range(args.repeats):
            ordered=list(reversed(plans)) if (repeat+args.reverse)%2 else plans
            for item in ordered:
                verify_frozen()
                destination=output/f"{repeat+1}-{item['name']}-{args.mode}"
                print(f"{destination.name}: starting",flush=True)
                diagnostic=args.mode=="diagnostic"
                result=live.run_once(binary,repository,"history-hover",destination,240,metadata=False,display="desktop",ping_ms=200,
                                     extra_env=item["runtime"],scenario_steps=schedule,ui_scale=args.ui_scale,
                                     window_size=item["window_size"],session_overrides=item["session_overrides"],gpu=args.mode in ("diagnostic","occupancy"),
                                     gpu_timings=diagnostic,input_route=args.input_route)
                verify_frozen()
                errors,debug_expected=screen_errors(result,item["runtime"],args.mode,args.duration_ms<30000,live.load_records(destination),args.input_route)
                actual_ms=result.get("phases",{}).get("hover",{}).get("seconds",0)*1000
                if abs(actual_ms-args.duration_ms)>max(250,args.duration_ms*.01):
                    errors.append("actual hover phase duration differs from scheduled sweep")
                row={"name":item["name"],"repeat":repeat+1,"path":str(destination),"runtime":item["runtime"],
                     "evidence_scope":item["evidence_scope"],"input_route":args.input_route,"latency_acceptance_eligible":False,
                     "native_submit_latency_screen_eligible":args.mode=="clean" and args.input_route=="native" and item["evidence_scope"]=="render_complete_candidate_screen",
                     "requested_window_size":item["window_size"],"session_overrides":item["session_overrides"],
                     "gate_errors":errors,"expected_short_debug_limitations":debug_expected,"metrics":extract(result,bool(item["runtime"].get("GPUI_GPU_DIAGNOSTIC_ABLATIONS")),args.input_route)}
                manifest["samples"].append(row)
                save()
                print(json.dumps({"name":row["name"],"gate_errors":errors,"gpu_render_elapsed_ms_per_second":row["metrics"]["gpu_render_elapsed_ms_per_second"],
                                  "app_pdh_busiest_engine_percent":row["metrics"]["app_pdh_busiest_engine_percent"],
                                  "all_submissions":row["metrics"]["all_submissions"]},indent=2),flush=True)
                if errors and not args.continue_on_invalid:
                    raise ValueError(f"invalid pilot capture {destination}: {errors}")
        manifest.update(complete=True,finished_unix=time.time())
    finally:
        save()


if __name__=="__main__":
    main()
