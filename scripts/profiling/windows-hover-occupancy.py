#!/usr/bin/env python3
"""Windows shipping-operation GPU occupancy and observer-overhead pilots.

Requires physical history coordinates from a valid causal native capture using
the same frozen binary/fixture. The shipping route disables app instrumentation;
there is consequently no fresh row witness, submission or display latency proof.
Only the newly launched app's already-foreground HWND can receive mouse moves.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import math
from pathlib import Path
import platform
import subprocess
import time
import uuid

import perf_gpu
import perf_gpu_frames
import perf_metadata
import perf_native_input
import perf_platform
import perf_windows_environment
import perf_workloads

live = perf_workloads.module("occupancy_live", "live-ui.py")
matrix = perf_workloads.module("occupancy_matrix", "windows-hover.py")
MODES = ("shipping", "probe", "probe-timings")


def validate_observer_records(records, runtime, mode):
    """Require actual capabilities even when this observer disables queries."""
    if mode == "shipping":
        return []  # No instrumentation means no fresh activation witness.
    timings = mode == "probe-timings"
    requested = set(filter(None, runtime.get("GPUI_GPU_EXPERIMENTS", "").split(",")))
    backend = "d3d11" if runtime["GPUI_WINDOWS_RENDERER"] == "dx11" else "wgpu-dx12"
    errors = perf_gpu_frames.capability_errors(records, requested, backend, timings)
    for record in records:
        if record.get("event") != "renderer_capabilities":
            continue
        capability = perf_gpu_frames.capability_payload(record)
        if capability.get("timing_enabled") is not timings:
            errors.append("actual observer timestamp activation does not match mode")
        if capability.get("active_diagnostic_ablations") != []:
            errors.append("observer occupancy requires reported empty diagnostic ablations")
        if capability.get("windows_keepalive_policy") != runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"]:
            errors.append("actual keepalive policy does not match runtime")
        if runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"] == "off" and capability.get("windows_input_rate_keepalive") is not False:
            errors.append("keepalive-off observer request is unresolved")
        present = "immediate" if backend == "d3d11" else runtime["GPUI_WINDOWS_DX12_PRESENT_MODE"]
        if capability.get("present_mode") != present:
            errors.append("actual observer present mode does not match runtime")
    gpu = [record for record in records if record.get("event") == "gpu_frame"]
    if not timings and gpu:
        errors.append("timestamp-free observer unexpectedly emitted GPU query records")
    if timings:
        for record in gpu:
            flags = record.get("experiments")
            if not isinstance(flags, list) or any(not isinstance(flag, str) for flag in flags) or set(flags) != requested:
                errors.append("GPU observer actual experiment set missing or mismatched")
            if record.get("diagnostic_ablations") != []:
                errors.append("GPU observer diagnostic controls missing or nonempty")
    return sorted(set(errors))


def environment(env, output, mode, runtime):
    """Inherited profiling variables cannot enable hidden observers."""
    if mode not in MODES:
        raise ValueError("unknown observer mode")
    result = {k:v for k,v in env.items() if not k.startswith(("GITCOMET_UI_", "GITCOMET_REPO_LOAD_TRACE", "GITCOMET_PERF_", "GIT_TRACE", "GPUI_"))}
    if any(k.startswith(("GITCOMET_UI_", "GIT_TRACE")) or k == "GPUI_GPU_TIMINGS" for k in runtime):
        raise ValueError("runtime flags cannot override observer mode")
    result.update(runtime)
    if mode != "shipping":
        result.update(GITCOMET_UI_PROBE="1",GITCOMET_UI_PROBE_JSONL=str(output/"frames.jsonl"),
                      GITCOMET_UI_PROBE_LOG=str(output/"ui.log"),GITCOMET_UI_PROBE_INTERVAL_MS="1000",
                      GITCOMET_UI_PROBE_PING_MS="200",GITCOMET_PERF_RUN_ID=str(uuid.uuid4()))
    if mode == "probe-timings":
        result["GPUI_GPU_TIMINGS"] = "1"
    return result


def validated_template(directory):
    directory = Path(directory)
    summary = json.loads((directory/"summary.json").read_text(encoding="utf-8"))
    capture = json.loads((directory/"capture.json").read_text(encoding="utf-8"))
    hover = summary.get("phases",{}).get("hover",{}).get("hover",{})
    if (not summary.get("valid") or summary.get("debug_assertions") is not False or summary.get("input_route") != "native"
            or hover.get("problems") or hover.get("unaccounted") or not hover.get("final_witness")
            or hover.get("painted_and_submitted",0)<20 or hover.get("native_acknowledged") != hover.get("requested")):
        raise ValueError("replay source requires a valid release native causal history capture")
    frames = [json.loads(line) for line in (directory/"frames.jsonl").read_text(encoding="utf-8").splitlines()]
    begin = next(r["at_ms"] for r in frames if r.get("event")=="scenario_phase" and r.get("detail",{}).get("name")=="hover" and r["detail"].get("state")=="begin")
    end = next(r["at_ms"] for r in frames if r.get("event")=="scenario_phase" and r.get("detail",{}).get("name")=="hover" and r["detail"].get("state")=="end")
    replay = [json.loads(line) for line in (directory/"native-input.jsonl").read_text(encoding="utf-8").splitlines()]
    requests = [r["request"] for r in replay if begin <= r["request"]["scheduled_at_ms"] < end]
    if len(requests)<3700 or len(requests)!=hover["requested"] or any(not r["ack"].get("success") for r in replay):
        raise ValueError("replay source does not contain the validated 30 second history sweep")
    handles = {str(r["hwnd"]) for r in requests}
    geometries = capture.get("native_replay",{}).get("window_geometries",{})
    if len(handles)!=1 or not handles.issubset(geometries):
        raise ValueError("single measured HWND physical geometry required")
    geometry = geometries[next(iter(handles))]
    if geometry.get("unavailable") or not geometry.get("window_dpi") or len(geometry.get("client_origin_px",[]))!=2:
        raise ValueError("source HWND DPI/client origin unavailable")
    origin = geometry["client_origin_px"]
    points = [(r["x_px"]-origin[0],r["y_px"]-origin[1]) for r in requests]
    if any(not (0 <= x < geometry["client_width_px"] and 0 <= y < geometry["client_height_px"]) for x,y in points):
        raise ValueError("validated coordinates escape app client bounds")
    intervals = [b["scheduled_at_ms"]-a["scheduled_at_ms"] for a,b in zip(requests,requests[1:])]
    if not intervals or any(abs(v-8)>.01 for v in intervals) or len(points)!=3750:
        raise ValueError("source must use the standard 3750-input 8 ms sweep")
    return {"capture":capture,"source":str(directory.resolve()),"geometry":geometry,"points":points,
            "source_sha256":{name:perf_metadata.sha256_file(directory/name) for name in ("summary.json","capture.json","frames.jsonl","native-input.jsonl")}}


class OwnedWindow:
    def __init__(self,pid):
        self.pid = pid
        self.user = c.WinDLL("user32",use_last_error=True)
        self.user.GetWindowThreadProcessId.argtypes = [w.HWND,c.POINTER(w.DWORD)]
        self.user.GetForegroundWindow.restype = w.HWND
        self.user.IsWindowVisible.argtypes = [w.HWND]
        self.user.IsIconic.argtypes = [w.HWND]

    def foreground(self):
        handle,owner = self.user.GetForegroundWindow(),w.DWORD()
        self.user.GetWindowThreadProcessId(handle,c.byref(owner))
        return handle if owner.value==self.pid and self.user.IsWindowVisible(handle) and not self.user.IsIconic(handle) else None


def compatible_geometry(reference,current):
    # Position may change between launches; source points are client-relative.
    return not current.get("unavailable") and all(current.get(k)==reference.get(k) for k in
            ("client_width_px","client_height_px","window_dpi","monitor"))


def occupancy_errors(gpu,seconds):
    values = list(gpu.get("processes",{}).get("app",{}).values())
    errors=[]
    if matrix.occupancy({"phases":{"hover":{"gpu":gpu}}}) is None:
        errors.append("PDH phase has no finite occupancy for its observed app engine adapters")
    if not any(g.get("busiest_engine_percent",{}).get("count",0)>=25 and g.get("busiest_engine_percent",{}).get("covered_ms",0)>=seconds*800 for g in values):
        errors.append("need 25 contained valid PDH intervals covering 80 percent of hover")
    return errors


def run_once(binary,repository,template,output,runtime,mode="shipping"):
    if platform.system() != "Windows":
        raise ValueError("occupancy replay requires Windows")
    if runtime.get("GPUI_GPU_DIAGNOSTIC_ABLATIONS") != "":
        raise ValueError("observer runtime must explicitly disable diagnostic controls")
    binary,repository,output = Path(binary).resolve(strict=True),Path(repository).resolve(strict=True),Path(output).resolve()
    source = template["capture"]
    binary_hash = perf_metadata.sha256_file(binary)
    fixture_status = hashlib.sha256(live.git(repository,"status","--porcelain=v1","-z").stdout).hexdigest()
    if binary_hash != source["binary_sha256"] or live.git(repository,"rev-parse","HEAD").stdout.decode().strip()!=source["repository_head"] or fixture_status!=source.get("repository_status_sha256"):
        raise ValueError("binary/fixture differs from validated source")
    output.mkdir(parents=True,exist_ok=False)
    env = live.seed_profile(output,repository,source["window_size"],source.get("session_overrides"))
    session = json.loads((output/"session.json").read_text(encoding="utf-8"))
    session["ui_scale_percent"] = source["ui_scale_percent"]
    (output/"session.json").write_text(json.dumps(session),encoding="utf-8")
    env = environment(env,output,mode,runtime)
    result = {"schema":1,"mode":mode,"measurement_kind":"occupancy_only","shipping_default_acceptance":False,
              "binary_sha256":binary_hash,"repository_head":source["repository_head"],"runtime":runtime,
              "validated_coordinates":template["source"],"validated_source_sha256":template["source_sha256"],
              "instrumentation":{"ui_probe_and_optrace":mode!="shipping","scenario":False,"gpu_timestamps":mode=="probe-timings","external_pdh":True},
              "renderer_activation_validation":"not_observable_with_app_observers_disabled" if mode=="shipping" else "actual_capabilities_checked",
              "latency":None,"fresh_hover_witness":False,"display_validation":"not_measured","problems":[],"phases":{},
              "note":"No fresh causal proof/readiness signal with app observers disabled. Prior validated coordinates preserve workload geometry; this run measures occupancy only."}
    pointer, sampler, process = perf_native_input.WindowsPointer(),None,None
    injected, phases = [], []
    try:
        with (output/"stderr.log").open("wb") as stderr:
            process = subprocess.Popen([str(binary)],env=env,cwd=output,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=stderr)
            owned = OwnedWindow(process.pid)
            deadline = time.perf_counter()+30
            hwnd = None
            while hwnd is None and time.perf_counter()<deadline:
                if process.poll() is not None:
                    raise RuntimeError("app exited before foreground window was available")
                hwnd = owned.foreground()
                if hwnd is None:
                    time.sleep(.1)
            if hwnd is None:
                raise RuntimeError("new app never became foreground; runner does not activate windows")
            geometry = perf_windows_environment.window(hwnd)
            if not compatible_geometry(template["geometry"],geometry):
                raise RuntimeError("app physical client dimensions, DPI or monitor differ from validated capture")
            result["window_geometry"] = geometry
            sampler = perf_gpu.Sampler(output,{"app":process.pid})
            def check():
                if process.poll() is not None or owned.foreground()!=hwnd:
                    raise RuntimeError("owned app is no longer the visible foreground window")
                if perf_windows_environment.window(hwnd)!=geometry:
                    raise RuntimeError("app window geometry changed during occupancy sweep")
            def wait_until(deadline):
                while time.perf_counter()<deadline:
                    check()
                    time.sleep(min(.1,max(0,deadline-time.perf_counter())))
            warm_begin = time.time()*1000
            wait_until(time.perf_counter()+10)
            hover_begin,start = time.time()*1000,time.perf_counter()
            origin = geometry["client_origin_px"]
            checked = start
            for ix,(x,y) in enumerate(template["points"]):
                target = start+ix*.008
                remaining = target-time.perf_counter()
                if remaining>0:
                    time.sleep(remaining)
                if time.perf_counter()-checked>=.25:
                    check()
                    checked=time.perf_counter()
                request = {"sequence":ix+1,"hwnd":hwnd,"x_px":x+origin[0],"y_px":y+origin[1],"scheduled_unix_ms":hover_begin+ix*8}
                ack = pointer.send(request,process.pid)
                injected.append({"request":request,"ack":dict(ack,success=True),"schedule_lateness_ms":(time.perf_counter()-target)*1000})
            wait_until(start+30)
            hover_end = time.time()*1000
            wait_until(time.perf_counter()+5)
            tail_end = time.time()*1000
            phases = [("warmup",warm_begin,hover_begin),("hover",hover_begin,hover_end),("tail",hover_end,tail_end)]
            if hover_end-hover_begin>30100:
                result["problems"].append("external replay exceeded the 30 second phase by more than 100 ms")
            if max(r["schedule_lateness_ms"] for r in injected)>32:
                result["problems"].append("external scheduler missed a deadline by more than 32 ms")
    except Exception as error:
        result["problems"].append(str(error))
    finally:
        if sampler:
            try:
                result["gpu_capture"] = sampler.close()
            except Exception as error:
                result["problems"].append(str(error))
        if process:
            try:
                perf_platform.stop_tree(process)
            except Exception as error:
                result["problems"].append(str(error))
            result["exit_code_after_owned_cleanup"] = process.returncode
        (output/"native-input.jsonl").write_text("".join(json.dumps(r)+"\n" for r in injected),encoding="utf-8")
        gpu = perf_gpu.load(output)
        for name,begin,end in phases:
            result["phases"][name] = {"begin_unix_ms":begin,"end_unix_ms":end,"seconds":(end-begin)/1000,
                                      "gpu":perf_gpu.summarize_phase(gpu,begin,end,live.distribution)}
        if "hover" in result["phases"]:
            phase = result["phases"]["hover"]
            result["problems"] += occupancy_errors(phase["gpu"],phase["seconds"])
        else:
            result["problems"].append("external hover phase incomplete")
        result["injected_inputs"] = len(injected)
        result["schedule_lateness_ms"] = live.distribution(r["schedule_lateness_ms"] for r in injected)
        if len(injected)!=3750 or perf_metadata.sha256_file(binary)!=binary_hash or hashlib.sha256(live.git(repository,"status","--porcelain=v1","-z").stdout).hexdigest()!=fixture_status:
            result["problems"].append("incomplete replay or frozen binary/fixture changed")
        if mode=="shipping" and (output/"frames.jsonl").exists():
            result["problems"].append("shipping occupancy unexpectedly created probe records")
        if mode!="shipping":
            try:
                records = live.load_records(output)
                starts = [r for r in records if r.get("event")=="start"]
                if len(starts)!=1 or starts[0].get("debug_assertions") is not False:
                    result["problems"].append("probe observer did not report a release start")
                result["problems"] += validate_observer_records(records,runtime,mode)
                if mode=="probe-timings":
                    if sum(r.get("event")=="gpu_frame" and r.get("status")=="complete" for r in records)<20:
                        result["problems"].append("timestamp observer lacks complete GPU query records")
            except (OSError,ValueError) as error:
                result["problems"].append(f"observer records unavailable: {error}")
        result["valid_occupancy_capture"] = not result["problems"]
        result["pdh_occupancy_scopes"] = matrix.occupancy_scopes(result)
        (output/"summary.json").write_text(json.dumps(result,indent=2)+"\n",encoding="utf-8")
    return result


def pilot(binary,repository,validated_run,output,runtime):
    """Three alternating pairs per observer factor; all replay externally."""
    template = validated_template(validated_run)
    output = Path(output).resolve()
    output.mkdir(parents=True,exist_ok=False)
    environment_snapshot = perf_metadata.collect(binaries=[("frozen",binary)],fixtures=[("history",repository)])
    manifest = {"schema":1,"id":str(uuid.uuid4()),"complete":False,"environment":environment_snapshot,
                "observer_pairs":[],"acceptance":"diagnostic_pilot_only","runtime":runtime,
                "observer_scope":"External replay in every mode measures probe/trace and timestamp factors. It does not quantify the app scenario request/ACK/causal-witness overhead used by the root-cost runner."}
    (output/"environment.json").write_text(json.dumps(environment_snapshot,indent=2)+"\n",encoding="utf-8")
    (output/"environment.txt").write_text(perf_metadata.readable(environment_snapshot),encoding="utf-8")
    try:
        for factor,baseline,candidate in (("probe","shipping","probe"),("timestamps","probe","probe-timings")):
            for pair in range(3):
                results={}
                for mode in ((baseline,candidate) if pair%2==0 else (candidate,baseline)):
                    if perf_metadata.source_info()!=environment_snapshot["source"]:
                        raise ValueError("source changed during observer pilot")
                    destination=output/f"{factor}-{pair+1}-{mode}"
                    print(f"{destination.name}: starting",flush=True)
                    result=run_once(binary,repository,template,destination,runtime,mode)
                    results[mode]=result
                    if not result["valid_occupancy_capture"]:
                        raise ValueError(f"invalid occupancy capture: {result['problems']}")
                b,candidate_result=results[baseline],results[candidate]
                b_gpu,c_gpu=matrix.occupancy(b),matrix.occupancy(candidate_result)
                manifest["observer_pairs"].append({"factor":factor,"pair":pair+1,"baseline":b,"candidate":candidate_result,
                                                   "pdh_busiest_engine_ratio":c_gpu/b_gpu if b_gpu else None})
        manifest["complete"]=True
        manifest["observer_overhead"]={factor:matrix.bootstrap([math.log(p["pdh_busiest_engine_ratio"]) for p in manifest["observer_pairs"] if p["factor"]==factor and p["pdh_busiest_engine_ratio"]]) for factor in ("probe","timestamps")}
    finally:
        (output/"observer-pilot.json").write_text(json.dumps(manifest,indent=2)+"\n",encoding="utf-8")
    return manifest


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command",choices=("run","pilot"))
    for name in ("binary","repository","validated-run","output"):
        parser.add_argument("--"+name,type=Path,required=True)
    parser.add_argument("--variant",choices=matrix.VARIANTS)
    parser.add_argument("--mode",choices=MODES,default="shipping")
    args=parser.parse_args()
    runtime=dict(matrix.BASELINE,**(matrix.VARIANTS[args.variant] if args.variant else {}))
    if args.command=="pilot":
        result=pilot(args.binary,args.repository,args.validated_run,args.output,runtime)
    else:
        result=run_once(args.binary,args.repository,validated_template(args.validated_run),args.output,runtime,args.mode)
    print(json.dumps({"valid":result.get("valid_occupancy_capture",result.get("complete")),"problems":result.get("problems",[])},indent=2))


if __name__=="__main__":
    main()
