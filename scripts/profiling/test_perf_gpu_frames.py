"""Asynchronous GPU frame attribution and missing-data regressions."""
import unittest
import json
from pathlib import Path
import tempfile
import perf_gpu_frames
import perf_workloads
live = perf_workloads.module("frames_live", "live-ui.py")


def frame(at, duration=2_000_000, **values):
    return dict(event="gpu_frame", at_ms=at, status="available", gpu_duration_ns=duration,
                pass_timing_complete=True, **values)


class FrameTests(unittest.TestCase):
    def summary(self, records, begin=100, end=200):
        return perf_gpu_frames.summarize(records, begin, end, live.distribution)

    def test_phase_uses_submission_time_and_sorts_delayed_readback(self):
        result = self.summary([frame(190, memory={"path_bytes": 200}),
                               frame(120, memory={"path_bytes": 100}), frame(200)])
        self.assertEqual(result["recorded_frames"], 2)
        self.assertEqual(result["allocation_bytes"]["path_bytes"], dict(initial=100, peak=200, settled=200))

    def test_unavailable_is_not_zero_or_complete_coverage(self):
        result = self.summary([dict(event="gpu_frame", at_ms=101, status="unsupported"), frame(110)])
        self.assertEqual(result["coverage"], .5)
        self.assertEqual(result["frame_ms"]["count"], 1)
        self.assertEqual(self.summary([])["frame_ms"]["p50"], None)
        self.assertEqual(self.summary([frame(110, float("nan"))])["status"], "unavailable")

    def test_warm_cache_omits_pass_but_frame_only_timing_does_not_invent_zero(self):
        a = frame(110, passes=[dict(name="path", duration_ns=1_000_000)])
        b = frame(120, passes=[])
        c = dict(frame(130), pass_timing_complete=False)
        result = self.summary([a, b, c])
        self.assertEqual(result["pass_ms"]["path"]["mean"], .5)
        self.assertEqual(result["pass_ms"]["path"]["count"], 2)
        self.assertEqual(result["timed_frames"], 3)

    def test_dropped_counter_is_a_phase_delta_and_unknown_memory_stays_unknown(self):
        records = [dict(event="interval", at_ms=90, gpu_records_dropped=9),
                   dict(event="interval", at_ms=180, gpu_records_dropped=11),
                   frame(150, memory={"atlas_bytes": None})]
        result = self.summary(records)
        self.assertEqual(result["records_dropped"], 2)
        self.assertNotIn("atlas_bytes", result["allocation_bytes"])

    def test_report_rejects_mixed_timestamp_instrumentation(self):
        with tempfile.TemporaryDirectory() as tmp:
            directories = []
            for index, timed in enumerate((False, True)):
                directory = Path(tmp) / str(index)
                directory.mkdir()
                (directory / "session.json").write_text(json.dumps({
                    "complete": True, "measurement_id": str(index), "gpu_timings": timed,
                }), encoding="utf-8")
                directories.append(directory)
            with self.assertRaisesRegex(ValueError, "GPU timestamps"):
                live.report(directories)


matrix = perf_workloads.module("gpu_matrix_test", "gpu-matrix.py")


class GateTests(unittest.TestCase):
    def run_result(self, duration=2, latency=10, **gpu_fields):
        gpu = dict(coverage=1, timed_frames=30, recorded_frames=30, cpu_submission_records=30,
                   query_samples_dropped=0, records_dropped=0, frame_ms=dict(p50=duration, p95=duration),
                   implemented_experiments=list(matrix.VARIANTS["combined"][1].split(",")))
        gpu.update(gpu_fields)
        phase = dict(gpu_frames=gpu, draw_ms=dict(p95=1), inputs=dict(input_to_submit_ms=dict(p95=latency)))
        return dict(valid=True, phases={name: phase for name in ("select", "hover", "scroll")})

    def test_gates_require_five_pairs_coverage_and_implemented_backend(self):
        pair = (self.run_result(), self.run_result(1.5))
        self.assertEqual(matrix.evaluate([pair] * 5, "cache")["status"], "local_gates_passed")
        self.assertEqual(matrix.evaluate([pair], "cache")["status"], "needs_review")
        for overrides in (dict(coverage=.9), dict(records_dropped=2), dict(implemented_experiments=[]),
                          dict(missing_submission_records=1), dict(allocation_bytes={"device_retention_bytes": {"peak": 33 * 1024 * 1024}})):
            pair = (self.run_result(), self.run_result(1.5, **overrides))
            self.assertEqual(matrix.evaluate([pair] * 5, "cache")["status"], "needs_review")

    def test_clean_input_guards_require_latency_without_inventing_gpu_measurements(self):
        baseline, candidate = self.run_result(), self.run_result()
        for result in (baseline, candidate):
            for phase in result["phases"].values():
                phase.pop("gpu_frames", None)
        result = matrix.evaluate([(baseline, candidate)] * 5, "paths", gpu_timings=False)
        self.assertEqual(result["status"], "input_guards_passed")
        self.assertIsNone(result["phases"]["hover"]["gpu_p50_change"])
        self.assertEqual(matrix.evaluate([(baseline, candidate)] * 5, "paths")["status"], "needs_review")
        candidate["phases"]["select"]["inputs"]["input_to_submit_ms"]["p95"] = 20
        self.assertEqual(matrix.evaluate([(baseline, candidate)] * 5, "paths", gpu_timings=False)["status"], "needs_review")

    def test_gpu_improvement_does_not_excuse_input_regression(self):
        pair = (self.run_result(), self.run_result(1.5, latency=12))
        result = matrix.evaluate([pair] * 5, "cache")
        self.assertTrue(any("input p95 regression" in r for r in result["reasons"]))
        pair = (self.run_result(), self.run_result(1.99))
        self.assertTrue(any("10%" in r for r in matrix.evaluate([pair] * 5, "cache")["reasons"]))


if __name__ == "__main__":
    unittest.main()
