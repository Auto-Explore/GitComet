"""Fail-closed root-cost controls, protected DWM discovery and source identity."""
import ctypes as c
import copy
import unittest
from unittest.mock import Mock, patch

import perf_gpu_frames
import perf_gpu_windows as pdh
import perf_workloads

root_cost=perf_workloads.module("root_cost_tests","windows-hover-root-cost.py")


def capture(route="native",mode="diagnostic",ablation="",experiment=""):
    requested=set(filter(None,experiment.split(",")))
    controls=set(filter(None,ablation.split(",")))
    geometry={"1":dict(window_dpi=144,client_width_px=2100,client_height_px=1350)}
    capability=dict(backend="wgpu-dx12",gpu_timestamps=True,per_pass_timestamps=True,
        timing_enabled=mode=="diagnostic",implemented_experiments=["partial-redraw","clipped-quads"],
        active_experiments=list(requested),windows_keepalive_policy="auto",windows_input_rate_keepalive=True,
        present_mode="immediate",implemented_diagnostic_ablations=list(controls),active_diagnostic_ablations=list(controls))
    records=[dict(event="renderer_capabilities",detail=capability),dict(event="interval",at_ms=0,gpu_records_dropped=0),
             dict(event="interval",at_ms=30000,gpu_records_dropped=0)]
    for i in range(30):
        at=100+i*900
        records.append(dict(event="submission",window="primary",at_ms=at,submission_id=i))
        if mode=="diagnostic":
            records.append(dict(event="gpu_frame",window="primary",at_ms=at,submission_id=i,status="complete",
                elapsed_ns=1000000,passes=[dict(name="clear_only" if ablation=="clear-only" else "main",elapsed_ns=900000)],
                experiments=list(requested),diagnostic_ablations=list(controls),damage_kind="full",retained_bytes=0))
    phase=dict(seconds=30,all_submissions=dict(count=30),
        hover=dict(painted_and_submitted=30,requested=3750,native_acknowledged=3750 if route=="native" else 0,
                   unaccounted=0,final_witness=True,problems=[],latency_origin="SendInput QPC" if route=="native" else "synthetic"),
        gpu=dict(processes=dict(app={"render":dict(engine_instances_observed=1,
                    busiest_engine_percent=dict(count=29,covered_ms=29000,duration_weighted_mean=30))})))
    if mode=="diagnostic":
        phase["gpu_frames"]=perf_gpu_frames.summarize(records,0,30000,root_cost.live.distribution)
    result=dict(valid=True,problems=[],debug_assertions=False,input_route=route,
        native_window_geometries=geometry if route=="native" else {},window_geometries=geometry,
        gpu_timings_requested=mode=="diagnostic",gpu_capture=dict(requested=mode!="clean",status="available"),
        renderer_capabilities=[capability],phases=dict(hover=phase))
    runtime=dict(root_cost.matrix.BASELINE,GPUI_GPU_DIAGNOSTIC_ABLATIONS=ablation,GPUI_GPU_EXPERIMENTS=experiment)
    return result,runtime,records


class RootCostTests(unittest.TestCase):
    def test_crop_controls_use_existing_flags_without_ablations(self):
        for name, experiments, keepalive in (
                ("crop-dx12", "cropped-paths", "auto"),
                ("crop-keepalive-dx12", "cropped-paths", "off"),
                ("crop-damage-keepalive-dx12", "cropped-paths,partial-redraw", "off")):
            flags, session, width = root_cost.VARIANTS[name]
            runtime = dict(root_cost.matrix.BASELINE, **flags)
            self.assertEqual((runtime["GPUI_GPU_EXPERIMENTS"], runtime["GPUI_WINDOWS_INPUT_RATE_KEEPALIVE"]),
                             (experiments, keepalive))
            self.assertEqual(runtime["GPUI_GPU_DIAGNOSTIC_ABLATIONS"], "")
            self.assertEqual((session, width), ({}, 1400))

    def test_device_dwm_occupancy_and_legacy_fallback_are_separate(self):
        result, _, _ = capture()
        phase = result["phases"]["hover"]
        phase["gpu"]["processes"]["desktop_compositor"] = {
            "adapter": dict(engine_instances_observed=1, busiest_engine_percent=dict(duration_weighted_mean=7))}
        phase["gpu"]["devices"] = {
            "adapter": dict(engine_instances_observed=3, busiest_engine_percent=dict(duration_weighted_mean=50))}
        metrics = root_cost.extract(result)
        self.assertEqual((metrics["app_pdh_busiest_engine_percent"], metrics["dwm_pdh_busiest_engine_percent"],
                          metrics["device_pdh_busiest_engine_percent"]), (30, 7, 50))
        self.assertEqual(metrics["surface_alpha_modes"]["missing"], 1)
        self.assertEqual(metrics["damage_fallback_reasons"]["missing"], 30)
        phase["gpu"]["devices"] = {}
        self.assertIsNone(root_cost.extract(result)["device_pdh_busiest_engine_percent"])

    def test_short_debug_never_suppresses_missing_occupancy(self):
        result,runtime,records=capture()
        gpu=result["phases"]["hover"]["gpu"]
        gpu["processes"]["app"]={"memory":dict(engine_instances_observed=0,
            busiest_engine_percent=dict(count=0,covered_ms=0,duration_weighted_mean=None))}
        errors,expected=root_cost.screen_errors(result,runtime,"diagnostic",True,records)
        self.assertTrue(any("no finite occupancy" in error for error in errors))
        self.assertTrue(any(error.startswith("PDH requires") for error in expected))

    def test_timestamp_free_occupancy_keeps_structural_and_actual_flags_gates(self):
        result,runtime,records=capture(mode="occupancy",experiment="clipped-quads")
        result["renderer_capabilities"][0].update(event="renderer_capabilities",at_ms=200,window="primary")
        self.assertEqual(root_cost.screen_errors(result,runtime,"occupancy",False,records)[0],[])
        self.assertEqual(root_cost.extract(result)["gpu_query_status"],"not_requested")
        self.assertIsNone(root_cost.extract(result)["per_pass"])
        self.assertIsNone(root_cost.extract(result)["quad_draw_calls"])
        self.assertIsNone(root_cost.extract(result)["quad_scissor_splits"])
        result["renderer_capabilities"][0]["active_experiments"]=[]
        errors,_=root_cost.screen_errors(result,runtime,"occupancy",False,records)
        self.assertTrue(any("active renderer experiments" in error for error in errors))

    def test_quad_counter_extract_retains_coverage_and_missing_legacy_fields(self):
        result,runtime,records=capture()
        metrics=root_cost.extract(result)
        self.assertEqual(metrics["quad_draw_calls"]["missing"],30)
        self.assertIsNone(metrics["quad_draw_calls"]["sum"])
        for record in records:
            if record.get("event")=="gpu_frame":
                record.update(quad_draw_calls=4,quad_scissor_splits=1)
        result["phases"]["hover"]["gpu_frames"]=perf_gpu_frames.summarize(records,0,30000,root_cost.live.distribution)
        metrics=root_cost.extract(result)
        self.assertEqual(metrics["quad_draw_calls"]["sum"],120)
        self.assertEqual(metrics["quad_draw_calls"]["per_second"],4)
        self.assertEqual(metrics["quad_scissor_splits"]["sum"],30)

    def test_synthetic_isolation_does_not_require_native_acks_or_pass_native_acceptance(self):
        result,runtime,records=capture(route="synthetic")
        self.assertEqual(root_cost.screen_errors(result,runtime,"diagnostic",False,records,"synthetic")[0],[])
        self.assertTrue(root_cost.matrix.run_errors(result,runtime,True))
        metrics=root_cost.extract(result,input_route="synthetic")
        self.assertNotIn("causal_hover",metrics)
        self.assertFalse(metrics["causal_hover_structure"]["pixel_witness"])
        self.assertTrue(root_cost.screen_errors(result,runtime,"clean",False,records,"synthetic")[0])

    def test_ablations_require_actual_caps_and_per_event_context_and_have_no_pixel_witness(self):
        result,runtime,records=capture(ablation="clear-only")
        self.assertEqual(root_cost.screen_errors(result,runtime,"diagnostic",False,records)[0],[])
        self.assertFalse(root_cost.extract(result,True)["causal_hover_structure"]["pixel_witness"])
        bad=copy.deepcopy(result)
        bad["renderer_capabilities"][0]["active_diagnostic_ablations"]=[]
        self.assertTrue(root_cost.screen_errors(bad,runtime,"diagnostic",False,records)[0])
        next(r for r in records if r.get("event")=="gpu_frame")["diagnostic_ablations"]=[]
        errors,_=root_cost.screen_errors(result,runtime,"diagnostic",False,records)
        self.assertTrue(any("context" in error for error in errors))

    def test_declared_framework_build_revision_is_mandatory_and_exact(self):
        head="a"*40
        with patch.object(root_cost,"checkout_identity",return_value=dict(head=head)) as checkout:
            for missing in (None,"","aaaaaaa"):
                with self.assertRaisesRegex(ValueError,"40-character"):
                    root_cost.declared_dependency("ignored",missing)
            checkout.assert_not_called()
            with self.assertRaisesRegex(ValueError,"declared binary"):
                root_cost.declared_dependency("ignored","b"*40)
            self.assertEqual(root_cost.declared_dependency("ignored",head)["head"],head)

    def test_formal_retained_hover_requires_partial_and_allocation_while_screen_preserves_full_fallback(self):
        result, runtime, records = capture(experiment="partial-redraw")
        result["renderer_capabilities"][0]["active_diagnostic_ablations"] = []
        # A truthful Full-only root-cost screen is still valuable diagnosis.
        self.assertEqual(root_cost.screen_errors(result, runtime, "diagnostic", False, records)[0], [])
        failures = root_cost.matrix.run_errors(result, runtime, True)
        self.assertTrue(any("observed Partial" in error for error in failures))
        self.assertTrue(any("allocation peak" in error for error in failures))
        for record in records:
            if record.get("event") == "gpu_frame":
                record.update(damage_kind="reuse", retained_bytes=4096)
        phase = result["phases"]["hover"]
        phase["gpu_frames"] = perf_gpu_frames.summarize(records, 0, 30000, root_cost.live.distribution)
        self.assertTrue(any("observed Partial" in error for error in root_cost.matrix.run_errors(result, runtime, True)))
        first = next(record for record in records if record.get("event") == "gpu_frame")
        first["damage_kind"] = "partial"
        phase["gpu_frames"] = perf_gpu_frames.summarize(records, 0, 30000, root_cost.live.distribution)
        self.assertEqual(root_cost.matrix.run_errors(result, runtime, True), [])
        for partial in (None, 0, -1, True, 1.5):
            phase["gpu_frames"]["damage_kinds"]["partial"] = partial
            self.assertTrue(any("observed Partial" in error for error in root_cost.matrix.run_errors(result, runtime, True)))
        phase["gpu_frames"]["damage_kinds"]["partial"] = 1
        for peak in (None, 0, -1, True, float("nan")):
            phase["gpu_frames"]["allocation_bytes"]["retained_bytes"]["peak"] = peak
            self.assertTrue(any("allocation peak" in error for error in root_cost.matrix.run_errors(result, runtime, True)))
        phase["gpu_frames"]["allocation_bytes"] = {}
        self.assertTrue(any("allocation peak" in error for error in root_cost.matrix.run_errors(result, runtime, True)))
        clean, clean_runtime, _ = capture(mode="clean", experiment="partial-redraw")
        self.assertEqual(root_cost.matrix.run_errors(clean, clean_runtime, False), [])
        baseline, baseline_runtime, _ = capture()
        self.assertEqual(root_cost.matrix.run_errors(baseline, baseline_runtime, True), [])


class CapabilityTests(unittest.TestCase):
    def test_supported_flag_is_insufficient_for_clean_activation(self):
        caps=[dict(event="renderer_capabilities",implemented_experiments=["clipped-quads"],active_experiments=[])]
        self.assertTrue(perf_gpu_frames.capability_errors(caps,["clipped-quads"],timings=False))
        caps[0]["active_experiments"]=["clipped-quads"]
        self.assertEqual(perf_gpu_frames.capability_errors(caps,["clipped-quads"],timings=False),[])
        caps[0]["active_experiments"].append("partial-redraw")
        self.assertTrue(perf_gpu_frames.capability_errors(caps,["clipped-quads"],timings=False))


class DwmTests(unittest.TestCase):
    def test_protected_dwm_session_uses_wts_without_openprocess(self):
        kernel=Mock()
        kernel.CreateToolhelp32Snapshot.return_value=123
        def current_session(pid,out):
            out._obj.value=3
            return pid==pdh.os.getpid()
        kernel.ProcessIdToSessionId.side_effect=current_session
        def first(snapshot,out):
            out._obj.pid=11576
            out._obj.name="dwm.exe"
            return True
        kernel.Process32FirstW.side_effect=first
        kernel.Process32NextW.return_value=False
        with patch.object(pdh.os,"name","nt"),patch.object(c,"WinDLL",return_value=kernel,create=True),\
                patch.object(c,"get_last_error",return_value=5,create=True),patch.object(pdh,"wts_dwm_pid",return_value=11576) as fallback:
            result=pdh.dwm_identity()
        self.assertEqual((result["pid"],result["session_id"]),(11576,3))
        fallback.assert_called_once_with(3,{11576})
        kernel.OpenProcess.assert_not_called()
        kernel.CloseHandle.assert_called_once_with(123)

    def test_unavailable_dwm_reason_is_explicit(self):
        with patch.object(pdh.os,"name","posix"):
            result=pdh.dwm_identity()
        self.assertIsNone(result["pid"])
        self.assertIn("unavailable",result)


if __name__=="__main__":
    unittest.main()
