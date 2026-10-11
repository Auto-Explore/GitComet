"""Observer validation uses actual activation, regardless of payload layout."""
import unittest

import perf_gpu_frames
import perf_workloads

occupancy = perf_workloads.module("observer_regression", "windows-hover-occupancy.py")


def cap(experiments=(), timings=True, **extra):
    return dict(event="renderer_capabilities", backend="wgpu-dx12",
                implemented_experiments=["cropped-paths", "partial-redraw"],
                active_experiments=list(experiments), gpu_timestamps=True,
                timing_enabled=timings, active_diagnostic_ablations=[],
                windows_keepalive_policy="auto", windows_input_rate_keepalive=True,
                present_mode="immediate", **extra)


class ObserverTests(unittest.TestCase):
    def validate(self, records, mode="probe-timings", **runtime):
        return occupancy.validate_observer_records(records, dict(occupancy.matrix.BASELINE, **runtime), mode)

    def test_flat_and_nested_payloads_preserve_actual_false(self):
        record = cap(timings=False)
        nested = dict(event="renderer_capabilities", detail={k: v for k, v in record.items() if k != "event"})
        self.assertEqual(self.validate([record], "probe"), [])
        self.assertEqual(self.validate([nested], "probe"), [])
        self.assertIs(perf_gpu_frames.capability_payload(nested)["timing_enabled"], False)
        # Explicit malformed detail cannot fall back to valid top-level fields.
        self.assertTrue(self.validate([dict(record, detail=None)], "probe"))

    def test_supported_but_inactive_requested_flags_fail(self):
        self.assertTrue(self.validate([cap()], GPUI_GPU_EXPERIMENTS="cropped-paths"))
        self.assertEqual(self.validate([cap(["cropped-paths"])], GPUI_GPU_EXPERIMENTS="cropped-paths"), [])

    def test_missing_malformed_and_unsupported_capabilities_fail(self):
        self.assertTrue(self.validate([]))
        for name in ("implemented_experiments", "active_experiments"):
            for invalid in (None, "cropped-paths", [None]):
                record = cap(["cropped-paths"])
                record[name] = invalid
                self.assertTrue(self.validate([record], GPUI_GPU_EXPERIMENTS="cropped-paths"))
        record = cap(["cropped-paths"])
        record["implemented_experiments"] = []
        self.assertTrue(self.validate([record], GPUI_GPU_EXPERIMENTS="cropped-paths"))

    def test_timestamp_activation_and_gpu_actual_flags(self):
        records = [cap(["cropped-paths"]), dict(event="gpu_frame", experiments=["cropped-paths"], diagnostic_ablations=[])]
        runtime = dict(GPUI_GPU_EXPERIMENTS="cropped-paths")
        self.assertEqual(self.validate(records, **runtime), [])
        records[1]["experiments"] = []
        self.assertTrue(self.validate(records, **runtime))
        records[1]["experiments"] = ["cropped-paths"]
        records[0]["timing_enabled"] = False
        self.assertTrue(self.validate(records, **runtime))

    def test_query_free_probe_requires_caps_and_no_query_events(self):
        self.assertEqual(self.validate([cap(timings=False)], "probe"), [])
        self.assertTrue(self.validate([cap(timings=False), dict(event="gpu_frame")], "probe"))
        record = cap(timings=False)
        record["active_diagnostic_ablations"] = None
        self.assertTrue(self.validate([record], "probe"))
        self.assertEqual(self.validate([], "shipping"), [])  # Explicit occupancy-only scope.

    def test_keepalive_and_present_policy_must_actually_match(self):
        record = cap(timings=False)
        record.update(windows_keepalive_policy="off", windows_input_rate_keepalive=False)
        self.assertEqual(self.validate([record], "probe", GPUI_WINDOWS_INPUT_RATE_KEEPALIVE="off"), [])
        record["windows_input_rate_keepalive"] = True
        self.assertTrue(self.validate([record], "probe", GPUI_WINDOWS_INPUT_RATE_KEEPALIVE="off"))
        record.update(windows_input_rate_keepalive=False, present_mode="fifo")
        self.assertTrue(self.validate([record], "probe", GPUI_WINDOWS_INPUT_RATE_KEEPALIVE="off"))


if __name__ == "__main__":
    unittest.main()
