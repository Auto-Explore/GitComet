"""Windows GPU occupancy attribution, DWM discovery and run metadata."""
import ctypes as c
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import perf_gpu
import perf_gpu_windows as pdh
import perf_metadata
import perf_workloads

live = perf_workloads.module("windows_tests_live", "live-ui.py")


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

    def test_memory_only_adapter_reports_no_engine_occupancy(self):
        secondary="pid_41_luid_0x00000000_0x456_phys_0"
        raw={"unix_ms":1000,"engines":[engine(41,17.5)],"shared_mib":[{"instance":secondary,"value":0}]}
        next_sample=dict(raw,unix_ms=2000)
        rows=pdh.records(next_sample,raw,{"app":41})
        gpu=perf_gpu.summarize_phase(rows,1000,2000,live.distribution)
        groups=gpu["processes"]["app"]
        self.assertEqual(groups["luid_0x00000000_0x123_phys_0"]["engine_instances_observed"],1)
        self.assertEqual(groups["luid_0x00000000_0x456_phys_0"]["engine_instances_observed"],0)
        self.assertIsNone(groups["luid_0x00000000_0x456_phys_0"]["busiest_engine_percent"]["duration_weighted_mean"])


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


class MetadataTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
