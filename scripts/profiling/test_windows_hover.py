"""Windows attribution, native rendezvous and run-level acceptance regressions."""
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

import perf_gpu
import perf_gpu_frames
import perf_gpu_windows as pdh
import perf_hover
import perf_native_input as native
import perf_metadata
import perf_workloads

live = perf_workloads.module("windows_tests_live", "live-ui.py")
matrix = perf_workloads.module("windows_tests_matrix", "windows-hover.py")
validation = perf_workloads.module("windows_tests_validation", "validate-gpu.py")
occupancy = perf_workloads.module("windows_tests_occupancy", "windows-hover-occupancy.py")


def engine(pid, percent, index=0):
    return {"instance": f"pid_{pid}_luid_0x00000000_0x123_phys_0_eng_{index}_engtype_3D", "value": percent}


class PdhTests(unittest.TestCase):
    def test_pid_attribution_desktop_sum_and_missing_engine_are_distinct(self):
        raw = {"unix_ms":2000, "engines":[engine(41,10),engine(42,20),engine(99,30)],
               "shared_mib":[{"instance":"pid_41_luid_0x00000000_0x123_phys_0", "value":2*1048576}],
               "device_shared_mib":[{"instance":"luid_0x00000000_0x123_phys_0", "value":7*1048576}]}
        rows = pdh.records(raw,{"unix_ms":1000},{"app":41,"desktop_compositor":42})
        app,dwm,device = [next(r for r in rows if r.get("role")==role) for role in ("app","desktop_compositor",None)]
        self.assertEqual(app["busiest_engine_percent"],10)
        self.assertEqual(dwm["busiest_engine_percent"],20)
        self.assertEqual(device["busiest_engine_percent"],60)
        self.assertEqual((app["shared_mib"],device["shared_mib"]),(2,7))
        self.assertNotIn(99,[r.get("pid") for r in rows])
        raw["engines"].append(engine(41,None,1))
        self.assertIsNone(next(r for r in pdh.records(raw,None,{"app":41}) if r.get("role")=="app")["busiest_engine_percent"])

    def test_phase_rate_bounds_priming_and_weighted_coverage(self):
        raw = {"unix_ms":1000,"engines":[engine(41,99)]}
        rows = pdh.records(raw,None,{"app":41})
        for timestamp,percent in ((2000,10),(4000,20),(5000,None),(6000,90)):
            newer = {"unix_ms":timestamp,"engines":[engine(41,percent)]}
            rows.extend(pdh.records(newer,raw,{"app":41}))
            raw=newer
        result=perf_gpu.summarize_phase(rows,1000,5000,live.distribution)
        metric=next(iter(result["processes"]["app"].values()))["busiest_engine_percent"]
        self.assertEqual(metric["count"],2)
        self.assertEqual(metric["missing"],1)
        self.assertEqual(metric["covered_ms"],3000)
        self.assertAlmostEqual(metric["duration_weighted_mean"],50/3)

    def test_memory_only_adapter_does_not_poison_valid_engine_occupancy(self):
        secondary="pid_41_luid_0x00000000_0x456_phys_0"
        raw={"unix_ms":1000,"engines":[engine(41,17.5)],"shared_mib":[{"instance":secondary,"value":0}]}
        next_sample=dict(raw,unix_ms=2000)
        rows=pdh.records(next_sample,raw,{"app":41})
        gpu=perf_gpu.summarize_phase(rows,1000,2000,live.distribution)
        groups=gpu["processes"]["app"]
        self.assertEqual(groups["luid_0x00000000_0x123_phys_0"]["engine_instances_observed"],1)
        self.assertEqual(groups["luid_0x00000000_0x456_phys_0"]["engine_instances_observed"],0)
        self.assertIsNone(groups["luid_0x00000000_0x456_phys_0"]["busiest_engine_percent"]["duration_weighted_mean"])
        result={"phases":{"hover":{"gpu":gpu}}}
        self.assertEqual(matrix.occupancy(result),17.5)
        # An observed but invalid engine on a second active adapter must fail;
        # only an adapter with no engine instances can be excluded.
        next_sample["engines"]=next_sample["engines"]+[{"instance":secondary+"_eng_0_engtype_3D","value":None}]
        gpu=perf_gpu.summarize_phase(pdh.records(next_sample,raw,{"app":41}),1000,2000,live.distribution)
        result["phases"]["hover"]["gpu"]=gpu
        self.assertIsNone(matrix.occupancy(result))
        self.assertTrue(any("no finite occupancy" in e for e in matrix.run_errors(result,matrix.BASELINE,True)))
        self.assertTrue(any("no finite occupancy" in e for e in occupancy.occupancy_errors(gpu,1)))

    def test_completely_missing_pdh_phase_is_fatal_even_before_count_guard(self):
        result={"phases":{"hover":{"seconds":2,"gpu":{"processes":{"app":{"memory-only":{"engine_instances_observed":0,"busiest_engine_percent":{"duration_weighted_mean":None}}}}}}}}
        errors=matrix.run_errors(result,matrix.BASELINE,True)
        self.assertIn("PDH phase has no finite occupancy for its observed app engine adapters",errors)
        self.assertTrue(any(e.startswith("PDH requires") for e in errors))
        self.assertIsNone(matrix.occupancy(result))


class NativeTests(unittest.TestCase):
    def test_native_marker_survives_low32_transport_without_sequence_aliasing(self):
        self.assertEqual(native.input_marker(1),0x47430001)
        self.assertEqual(native.input_marker(3750),0x47430000|3750)
        self.assertEqual(native.input_marker(65535),0x4743ffff)
        for invalid in (0,-1,65536,True,1.5):
            with self.assertRaisesRegex(ValueError,"reserved"):
                native.input_marker(invalid)
        pointer=native.WindowsPointer.__new__(native.WindowsPointer)
        pointer.user=Mock()
        with self.assertRaisesRegex(ValueError,"reserved"):
            pointer.send(dict(sequence=65536,hwnd=123,x_px=10,y_px=10),41)
        pointer.user.GetWindowThreadProcessId.assert_not_called()
        pointer.user.SendInput.assert_not_called()

    def test_inactive_target_is_rejected_before_sendinput(self):
        pointer = native.WindowsPointer.__new__(native.WindowsPointer)
        pointer.user=Mock()
        def owner(hwnd,out):
            c.cast(out,c.POINTER(w.DWORD)).contents.value=41
        pointer.user.GetWindowThreadProcessId.side_effect=owner
        pointer.user.GetForegroundWindow.return_value=999
        with self.assertRaisesRegex(RuntimeError,"foreground"):
            pointer.send({"sequence":1,"hwnd":123,"x_px":10,"y_px":10},41)
        pointer.user.SendInput.assert_not_called()

    def test_partial_request_waits_for_newline_and_duplicate_fails(self):
        fake = Mock()
        fake.send.return_value=dict(qpc_before=1,qpc_after=2,qpc_frequency=1000,dispatch_unix_ms=3)
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            replay=native.NativeReplay(root,41,pointer_factory=lambda:fake)
            replay.request.write_text('{"sequence":1}',encoding="utf-8")
            time.sleep(.015)
            fake.send.assert_not_called()
            with replay.request.open("a",encoding="utf-8") as stream:
                stream.write('\n')
            deadline=time.monotonic()+1
            while not fake.send.called and time.monotonic()<deadline:
                time.sleep(.005)
            with self.assertRaisesRegex(ValueError,"Duplicate"):
                replay.handle({"sequence":1})
            self.assertEqual(replay.close()["sent"],1)
            log=json.loads(replay.log.read_text(encoding="utf-8"))
            self.assertTrue(log["ack"]["success"])


class CausalTests(unittest.TestCase):
    def records(self):
        return [{"event":"start","qpc_before":1000,"qpc_after":1001,"qpc_frequency":1000},
                {"event":"scenario_input","at_ms":10,"detail":{"op":1}},
                {"event":"hover_target","at_ms":12,"detail":{"sequence":1,"row_id":3,"window":"w"}},
                {"event":"hover_paint","at_ms":14,"detail":{"sequence":1,"row_id":3,"window":"w","hovered":True,"previous_row_id":2,"previous_cleared":True,"scene_revision":8}},
                {"event":"submission","at_ms":16,"start_ms":15,"window":"w","scene_revision":8,"submission_id":4}]

    def test_native_latency_joins_exact_painted_scene_and_qpc(self):
        ack={"sequence":1,"success":True,"qpc_before":1010,"qpc_after":1011,"qpc_frequency":1000}
        summary=perf_hover.summarize(self.records(),0,30,live.distribution,[ack])
        self.assertEqual(summary["native_dispatch_to_submit_ms"]["p95"],7)
        self.assertEqual(summary["handler_arrival_to_submit_ms"]["p95"],4)
        self.assertEqual(summary["unaccounted"],0)
        self.assertEqual(summary["qpc_anchor_uncertainty_ms"],1)
        records=self.records()
        records[-1]["scene_revision"]=9
        self.assertEqual(perf_hover.summarize(records,0,30,live.distribution)["unaccounted"],1)

    def test_uncleared_old_hover_and_delivered_nonrow_are_failures(self):
        records=self.records()
        records[3]["detail"]["previous_cleared"]=False
        self.assertIn("previous hover",perf_hover.summarize(records,0,30,live.distribution)["problems"][0])
        records=self.records()+[{"event":"scenario_input","at_ms":11,"detail":{"op":2}},
                    {"event":"native_input_arrival","at_ms":11,"sequence":2},
                    {"event":"hover_superseded","at_ms":13,"detail":{"sequence":2,"reason":"native_event_coalesced"}}]
        self.assertTrue(any("without a row" in p for p in perf_hover.summarize(records,0,30,live.distribution)["problems"]))

    def test_allpresent_membership_includes_reuse_and_interleaved_ids(self):
        records=[]
        for window,seq in (("a",1),("b",2),("a",3)):
            records += [dict(event="submission",window=window,submission_id=seq,start_ms=110+seq,at_ms=115+seq),
                        dict(event="gpu_frame",window=window,submission_id=seq,renderer_id=window,status="complete",elapsed_ns=1000000,at_ms=110+seq)]
        result=perf_gpu_frames.summarize(records,100,200,live.distribution)
        self.assertEqual(result["coverage"],1)
        self.assertEqual(result["missing_submission_records"],0)
        self.assertEqual(result["render_elapsed_ms_per_second"],30)
        self.assertIsNone(result["gpu_execution_union_ms_per_second"])
        records.pop()
        result=perf_gpu_frames.summarize(records,100,200,live.distribution)
        self.assertEqual(result["missing_submission_records"],1)


class GateTests(unittest.TestCase):
    def gpu(self,value):
        return {"phases":{"hover":{"gpu_frames":{"render_elapsed_ms_per_second":value},
                    "gpu":{"processes":{"app":{"adapter":{"busiest_engine_percent":{"duration_weighted_mean":value}}}}}}}}

    def test_small_repeated_above_noise_gain_passes_but_display_stays_pending(self):
        sessions=["first"]*3+["second"]*2
        aa=[(s,self.gpu(100),self.gpu(100.1)) for s in sessions]
        ab=[(s,self.gpu(100),self.gpu(98)) for s in sessions]
        result=matrix.evaluate(aa,ab)
        self.assertEqual(result["status"],"local_gpu_and_submit_gates_passed")
        self.assertEqual(result["display_validation"],"pending")
        self.assertEqual(matrix.evaluate(aa,[(s,self.gpu(100),self.gpu(99.95)) for s in sessions])["status"],"needs_review")
        self.assertEqual(matrix.evaluate(aa,ab[:4])["status"],"needs_review")
        self.assertEqual(matrix.bootstrap([1,2,3],samples=1000),matrix.bootstrap([1,2,3],samples=1000))

    def test_capabilities_and_zero_executed_native_tests_fail_closed(self):
        self.assertTrue(perf_gpu_frames.capability_errors([],[]))
        caps=[dict(event="renderer_capabilities",backend="d3d11",gpu_timestamps=True,timing_enabled=True,implemented_experiments=[],active_experiments=[])]
        self.assertTrue(perf_gpu_frames.capability_errors(caps,["cropped-paths"]))
        self.assertFalse(perf_gpu_frames.capability_errors(caps,[],"d3d11"))
        with self.assertRaisesRegex(ValueError,"zero"):
            validation.require_executed_tests("test result: ok. 0 passed; 0 failed; 3 filtered out")
        self.assertEqual(validation.require_executed_tests("test result: ok. 2 passed; 0 failed"),2)

    def test_matrix_history_step_requires_actual_hover_witness(self):
        sweep=next(step for step in matrix.scenario()["steps"] if step["do"]=="move_pointer")
        self.assertTrue(sweep["hover_rows"])
        self.assertEqual(sweep["repeat"],3750)

    def test_ignored_keepalive_flag_and_clean_gpu_observers_are_rejected(self):
        runtime=dict(matrix.BASELINE,GPUI_WINDOWS_INPUT_RATE_KEEPALIVE="off")
        hover=dict(painted_and_submitted=25,unaccounted=0,final_witness=True,native_acknowledged=3750,
                   requested=3750,latency_origin="SendInput QPC")
        result=dict(valid=True,problems=[],debug_assertions=False,input_route="native",
                    native_window_geometries={"1":dict(window_dpi=144,client_width_px=2100,client_height_px=1350)},
                    gpu_timings_requested=False,gpu_capture=dict(requested=False),
                    renderer_capabilities=[dict(backend="wgpu-dx12",implemented_experiments=[],active_experiments=[],active_diagnostic_ablations=[],
                        windows_keepalive_policy="off",windows_input_rate_keepalive=False,present_mode="immediate")],
                    phases=dict(hover=dict(hover=hover,all_submissions=dict(count=30))))
        self.assertEqual(matrix.run_errors(result,runtime,False),[])
        result["renderer_capabilities"][0]["windows_input_rate_keepalive"]=True
        self.assertTrue(any("unresolved" in error for error in matrix.run_errors(result,runtime,False)))
        result["renderer_capabilities"][0]["windows_input_rate_keepalive"]=False
        result["gpu_capture"]["requested"]=True
        self.assertTrue(any("observers" in error for error in matrix.run_errors(result,runtime,False)))
        runtime["GPUI_WINDOWS_DX12_PRESENT_MODE"]="fifo"
        self.assertTrue(any("present mode" in error for error in matrix.run_errors(result,runtime,False)))
        result["renderer_capabilities"][0]["present_mode"]="fifo"
        self.assertFalse(any("present mode" in error for error in matrix.run_errors(result,runtime,False)))

    def test_negative_native_latency_never_passes_guards(self):
        def value(latency):
            return {"phases":{"hover":{"hover":{"native_dispatch_to_submit_ms":{"p95":latency,"p99":latency}}}}}
        pairs=[(s,value(3),value(-1)) for s in ["first"]*3+["second"]*2]
        self.assertFalse(matrix.latency_gate(pairs)["passed"])


class EnvironmentAndOccupancyTests(unittest.TestCase):
    def test_untracked_content_changes_source_identity_even_with_same_name(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            name="untracked source.py"
            (root/name).write_text("first",encoding="utf-8")
            def command(args,**kwargs):
                return Mock(stdout=(name+"\0").encode() if "ls-files" in args else b"",stderr=b"",returncode=0)
            with patch.object(perf_metadata,"ROOT",root),patch.object(perf_metadata.subprocess,"run",side_effect=command),patch.object(perf_metadata,"run",return_value="revision"):
                first=perf_metadata.source_info()
                (root/name).write_text("second",encoding="utf-8")
                second=perf_metadata.source_info()
            self.assertEqual(first["untracked_files"],second["untracked_files"])
            self.assertNotEqual(first["untracked_sha256"],second["untracked_sha256"])
            self.assertTrue(perf_metadata.compare({"source":first},{"source":second})["source_differs"])
            self.assertIn(name,perf_metadata.readable({"source":first}))

    def test_power_and_display_changes_invalidate_comparison(self):
        first={"machine":{"windows_power_overlay":{"name":"Balanced"}},"display":{"windows_native":{"monitors":[{"refresh_hz":120}]}}}
        second={"machine":{"windows_power_overlay":{"name":"Best performance"}},"display":{"windows_native":{"monitors":[{"refresh_hz":60}]}}}
        differences=perf_metadata.compare(first,second)["invalidating"]
        self.assertIn("machine.windows_power_overlay",differences)
        self.assertIn("display.windows_native",differences)

    def test_shipping_clears_inherited_observers_and_never_allows_override(self):
        env=dict(GITCOMET_UI_SCENARIO="foreign.json",GITCOMET_UI_PROBE="1",GITCOMET_REPO_LOAD_TRACE="x",GPUI_GPU_TIMINGS="1",GIT_TRACE2_EVENT="x",GPUI_D3D_DEBUG="1",GITCOMET_SESSION_FILE="owned")
        shipping=occupancy.environment(env,Path("owned"),"shipping",matrix.BASELINE)
        self.assertFalse(any(k.startswith("GITCOMET_UI_") or k.startswith("GIT_TRACE") for k in shipping))
        self.assertNotIn("GPUI_GPU_TIMINGS",shipping)
        self.assertNotIn("GPUI_D3D_DEBUG",shipping)
        self.assertEqual(shipping["GITCOMET_SESSION_FILE"],"owned")
        with self.assertRaisesRegex(ValueError,"override"):
            occupancy.environment(env,Path("owned"),"shipping",{"GITCOMET_UI_PROBE":"1"})
        self.assertNotIn("GPUI_GPU_TIMINGS",occupancy.environment(env,Path("owned"),"probe",matrix.BASELINE))
        self.assertEqual(occupancy.environment(env,Path("owned"),"probe-timings",matrix.BASELINE)["GPUI_GPU_TIMINGS"],"1")

    def test_clean_geometry_accepts_new_origin_but_rejects_dpi_or_size_change(self):
        geometry=dict(client_width_px=1400,client_height_px=900,client_origin_px=[5,10],window_dpi=144,monitor={"refresh_hz":120})
        self.assertTrue(occupancy.compatible_geometry(geometry,dict(geometry,client_origin_px=[50,100])))
        self.assertFalse(occupancy.compatible_geometry(geometry,dict(geometry,window_dpi=96)))
        self.assertFalse(occupancy.compatible_geometry(geometry,dict(geometry,client_width_px=800)))
        gpu={"processes":{"app":{"gpu":{"busiest_engine_percent":{"count":24,"covered_ms":29000,"duration_weighted_mean":3}}}}}
        self.assertTrue(occupancy.occupancy_errors(gpu,30))
        gpu["processes"]["app"]["gpu"]["busiest_engine_percent"]["count"]=25
        self.assertEqual(occupancy.occupancy_errors(gpu,30),[])

    def test_occupancy_replay_requires_prior_final_witness_and_contained_history_points(self):
        with tempfile.TemporaryDirectory() as temp:
            output=Path(temp)
            geometry=dict(client_width_px=1400,client_height_px=900,client_origin_px=[50,100],window_dpi=144)
            capture=dict(native_replay=dict(window_geometries={"1":geometry}))
            hover=dict(problems=[],unaccounted=0,final_witness=True,painted_and_submitted=50,native_acknowledged=3750,requested=3750)
            summary=dict(valid=True,debug_assertions=False,input_route="native",phases=dict(hover=dict(hover=hover)))
            for name,value in (("capture",capture),("summary",summary)):
                (output/(name+".json")).write_text(json.dumps(value),encoding="utf-8")
            frames=[dict(event="scenario_phase",at_ms=100,detail=dict(name="hover",state="begin",extra="allowed")),
                    dict(event="scenario_phase",at_ms=30100,detail=dict(name="hover",state="end"))]
            (output/"frames.jsonl").write_text("".join(json.dumps(r)+"\n" for r in frames),encoding="utf-8")
            requests=[dict(request=dict(sequence=ix+1,hwnd=1,x_px=100,y_px=200 if ix%2 else 800,scheduled_at_ms=100+ix*8),ack=dict(success=True)) for ix in range(3750)]
            path=output/"native-input.jsonl"
            path.write_text("".join(json.dumps(r)+"\n" for r in requests),encoding="utf-8")
            template=occupancy.validated_template(output)
            self.assertEqual(template["points"][1],(50,100))
            requests[-1]["request"]["x_px"]=2000
            path.write_text("".join(json.dumps(r)+"\n" for r in requests),encoding="utf-8")
            with self.assertRaisesRegex(ValueError,"escape"):
                occupancy.validated_template(output)
            hover["final_witness"]=False
            (output/"summary.json").write_text(json.dumps(summary),encoding="utf-8")
            with self.assertRaisesRegex(ValueError,"causal"):
                occupancy.validated_template(output)


if __name__ == "__main__":
    unittest.main()
