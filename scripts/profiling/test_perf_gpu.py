"""GPU telemetry attribution, missing-data and sampling-boundary regressions."""
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import perf_gpu
import perf_workloads

live = perf_workloads.module("gpu_live_test", "live-ui.py")
multi_window = perf_workloads.module("gpu_multi_window_test", "multi-window.py")

HEADER = "#Date Time gpu pid type sm mem enc dec jpg ofa fb ccpm command\n"


def row(second, pid=41, gpu=0, sm="12", fb="128"):
    return f"20261004 14:00:{second:02} {gpu} {pid} C+G {sm} 4 - - - - {fb} 0 process with spaces\n"


class GpuTests(unittest.TestCase):
    def test_process_parser_keeps_busy_memory_distinct_from_framebuffer(self):
        parser = perf_gpu.PmonParser()
        parser.parse(HEADER)
        result = parser.parse(row(1))
        self.assertEqual(result["pid"], 41)
        self.assertEqual(result["sm_percent"], 12)
        self.assertEqual(result["memory_busy_percent"], 4)
        self.assertEqual(result["framebuffer_mb"], 128)
        self.assertIsNone(result["interval_start_unix_ms"])
        self.assertIsNone(result["encoder_percent"])
        missing = parser.parse(row(2, sm="-", fb="-"))
        self.assertIsNone(missing["sm_percent"])
        self.assertIsNone(missing["framebuffer_mb"])

    def test_reordered_or_older_headers_and_multiple_gpus(self):
        parser = perf_gpu.PmonParser()
        parser.parse("#Date Time gpu pid type fb mem sm enc dec command\n")
        a = parser.parse("20261004 14:00:01 0 41 G 128 4 12 - - app")
        b = parser.parse("20261004 14:00:01 1 41 G 256 3 7 - - app")
        # Different PIDs in a batch must not advance its time boundary.
        parser.parse("20261004 14:00:01 0 99 G 512 8 30 - - other")
        c = parser.parse("20261004 14:00:02 0 41 G 128 4 12 - - app")
        self.assertEqual((a["sm_percent"], b["framebuffer_mb"]), (12, 256))
        self.assertEqual(c["interval_start_unix_ms"], a["sample_start_unix_ms"])
        self.assertIsNone(b["interval_start_unix_ms"])

    def test_summaries_exclude_boundary_buckets_without_inventing_idle_zeros(self):
        parser = perf_gpu.PmonParser()
        parser.parse(HEADER)
        records = [dict(parser.parse(row(sec, sm=sm)), role="app")
                   for sec, sm in ((1, "99"), (2, "10"), (3, "-"), (4, "88"))]
        lo = records[0]["sample_start_unix_ms"]
        summary = perf_gpu.summarize_phase(records, lo, lo + 3000, live.distribution)
        app = summary["processes"]["app"]["0"]
        self.assertEqual(app["sm_percent"]["count"], 1)
        self.assertEqual(app["sm_percent"]["mean"], 10)
        self.assertEqual(app["sm_percent"]["missing"], 1)
        self.assertEqual(app["framebuffer_mb"]["count"], 3)
        short = perf_gpu.summarize_phase(records, lo + 100, lo + 900, live.distribution)
        self.assertIsNone(short["processes"]["app"]["0"]["sm_percent"]["mean"])

    def test_device_counters_and_process_counters_are_not_combined(self):
        device = perf_gpu.parse_device('2026/10/04 14:00:03.100, 0, GPU-test, "NVIDIA, test", 90, 12, 500, 8192, [N/A], 32, 1700, 5005, P0')
        self.assertEqual(device["name"], "NVIDIA, test")
        self.assertIsNone(device["power_w"])
        self.assertEqual(device["graphics_clock_mhz"], 1700)
        self.assertEqual(device["pstate"], "P0")
        parser = perf_gpu.PmonParser()
        parser.parse(HEADER)
        parser.parse(row(1))
        app = dict(parser.parse(row(2, sm="5")), role="app")
        summary = perf_gpu.summarize_phase([app, device], app["interval_start_unix_ms"], device["sample_end_unix_ms"], live.distribution)
        self.assertEqual(summary["processes"]["app"]["0"]["sm_percent"]["mean"], 5)
        self.assertEqual(summary["devices"]["0"]["gpu_busy_percent"]["mean"], 90)
        self.assertEqual(summary["devices"]["0"]["memory_used_mib"]["mean"], 500)

    def test_unavailable_collector_is_explicit_and_does_not_launch_helpers(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(perf_gpu.shutil, "which", return_value=None), \
                patch.object(perf_gpu.subprocess, "Popen") as launch:
            sampler = perf_gpu.Sampler(directory, {"app": 41})
            result = sampler.close()
            self.assertEqual(result["status"], "unavailable")
            self.assertIn("requires Linux", result["reason"])
            self.assertEqual(perf_gpu.load(directory), [])
            launch.assert_not_called()

    def test_collector_filters_other_processes_and_keeps_compositor_separate(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(perf_gpu.shutil, "which", return_value=None):
            sampler = perf_gpu.Sampler(directory, {"app": 41, "compositor": 42})
            process = type("FakeProcess", (), {"stdout": io.StringIO(HEADER + row(1, pid=99) + row(1, pid=41) + row(1, pid=42))})()
            sampler._read("process", process)
            sampler.close()
            records = perf_gpu.load(directory)
            self.assertEqual([r["role"] for r in records], ["app", "compositor"])
            self.assertEqual([r["pid"] for r in records], [41, 42])
            self.assertNotIn("command", records[0])

    def test_invalid_numeric_values_remain_missing(self):
        for value in ("-", "N/A", "[Not Supported]", "nan", "inf", None):
            self.assertIsNone(perf_gpu.number(value))
        self.assertEqual(perf_gpu.number("0"), 0)

    def test_gpu_summary_uses_the_same_completed_phase_as_cpu(self):
        parser = perf_gpu.PmonParser()
        parser.parse(HEADER)
        gpu = [dict(parser.parse(row(sec, sm=sm)), role="app")
               for sec, sm in ((1, "90"), (2, "90"), (10, "5"), (11, "5"), (12, "5"))]
        anchor = gpu[0]["sample_start_unix_ms"] - 1000
        records = [{"event": "start", "run_id": "test", "unix_ms": anchor}]
        # CPU summary retains the last completed occurrence of a repeated name.
        # GPU attribution must not span the earlier occurrence or intervening gap.
        records += [{"event": "scenario_phase", "at_ms": at,
                     "detail": {"name": "select", "state": state}}
                    for state, at in (("begin", 0), ("end", 4000),
                                      ("begin", 10_000), ("end", 14_000))]
        records += [{"event": "scenario_end", "detail": {"outcome": "passed"}}]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "capture.json").write_text(json.dumps({
                "run_id": "test", "scenario": "select", "outcome": "passed",
                "binary_sha256": "b", "repository_head": "r", "gpu_capture": {"requested": True}}))
            (root / "process.jsonl").write_text("")
            (root / "frames.jsonl").write_text("".join(json.dumps(r) + "\n" for r in records))
            (root / "gpu.jsonl").write_text("".join(json.dumps(r) + "\n" for r in gpu))
            result = live.summarize(root)
            self.assertTrue(result["valid"])
            phase = result["phases"]["select"]
            self.assertEqual(phase["seconds"], 4)
            self.assertEqual(phase["gpu"]["processes"]["app"]["0"]["sm_percent"]["mean"], 5)

    def test_partial_device_collection_still_reports_available_process_metrics(self):
        phase = {"gpu": {"processes": {"app": {"0": {"framebuffer_mb": {"max": 128}}}}}}
        report = multi_window.render([{"case": "windows-1", "valid": True, "result": {
            "phases": {"idle": phase}, "gpu_capture": {"requested": True, "status": "partial"}}}])
        self.assertIn("GPU 0:", report)
        self.assertIn("app framebuffer max=128", report)
        self.assertIn("whole-device busy mean=None", report)
        self.assertIn("GPU collector status: partial", report)

    def test_size_and_graph_diagnostics_use_only_the_isolated_session(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            live.seed_profile(root, Path("/fixture"), (2560, 1440), {"history_show_graph": False})
            session = json.loads((root / "session.json").read_text())
            self.assertEqual((session["window_width"], session["window_height"]), (2560, 1440))
            self.assertFalse(session["history_show_graph"])
            self.assertEqual(session["open_repos"], ["/fixture"])
            self.assertFalse(session["check_for_updates_on_startup"])


if __name__ == "__main__":
    unittest.main()
