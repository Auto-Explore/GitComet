import unittest
import perf_workloads

shared = perf_workloads.module("shared_reads_test", "shared-reads.py")


class ComparisonTests(unittest.TestCase):
    def records(self, baseline, candidate, pairs=5):
        return [dict(case="same-4", pair=pair, variant=variant,
                     result=dict(valid=True, phases={"select": {"draw_ms": {"p95": value},
                         "inputs": {"input_to_draw_ms": {"p95": value}}}}))
                for pair in range(pairs) for variant, value in (("baseline", baseline), ("candidate", candidate))]

    def test_latency_gate_uses_larger_of_one_ms_and_five_percent(self):
        self.assertTrue(shared.comparison(self.records(2, 2.9), 5)[0]["passes_latency"])
        self.assertFalse(shared.comparison(self.records(2, 3.1), 5)[0]["passes_latency"])
        self.assertTrue(shared.comparison(self.records(100, 104), 5)[0]["passes_latency"])
        self.assertFalse(shared.comparison(self.records(100, 106), 5)[0]["passes_latency"])

    def test_missing_or_failed_pairs_cannot_pass(self):
        records = self.records(2, 2)
        self.assertFalse(shared.comparison(records[:-1], 5)[0]["passes_latency"])
        records[0]["result"]["valid"] = False
        self.assertFalse(shared.comparison(records, 5)[0]["passes_latency"])

    def test_missing_metric_or_phase_cannot_disappear_from_the_gate(self):
        records = self.records(2, 2)
        records[0]["result"]["phases"]["select"].pop("inputs")
        self.assertFalse(shared.comparison(records, 5)[0]["passes_latency"])
        records[0]["result"]["phases"].clear()
        self.assertFalse(shared.comparison(records, 5)[0]["passes_latency"])

    def test_missing_work_counter_is_not_reported_as_zero(self):
        self.assertIsNone(shared.comparison(self.records(2, 2), 5)[0]["work"]["baseline"]["history_index_build"])

    def test_incremental_report_before_the_first_candidate(self):
        report = shared.comparison(self.records(2, 2)[:1], 5)[0]
        self.assertFalse(report["passes_latency"])
        self.assertIsNone(report["work"]["candidate"]["log_topology_build"])

    def test_development_or_allocation_diagnostics_cannot_pass_latency(self):
        for key in ("allocation_tracking", "debug_assertions", "gpu_timings_requested"):
            records = self.records(2, 2)
            records[0]["result"][key] = True
            self.assertFalse(shared.comparison(records, 5)[0]["passes_latency"])

    def test_hover_has_draw_timing_without_a_publication_witness(self):
        records = self.records(2, 2)
        for record in records:
            record["result"]["phases"]["hover"] = {"draw_ms": {"p95": 2}}
        report = shared.comparison(records, 5)[0]
        self.assertTrue(report["passes_latency"])
        self.assertEqual(len([m for m in report["metrics"] if m["phase"] == "hover"]), 1)


if __name__ == "__main__":
    unittest.main()
