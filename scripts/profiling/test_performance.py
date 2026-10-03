"""Run: python -m unittest discover -s scripts/profiling -p 'test_*.py'."""
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import perf_corpus
import perf_platform
import perf_report
import perf_workloads

live = perf_workloads.module("test_live_ui", "live-ui.py")


class ReportingTests(unittest.TestCase):
    def test_unsupported_headless_mode_does_not_silently_use_the_desktop(self):
        with patch.object(live.platform, "system", return_value="Windows"):
            with self.assertRaisesRegex(ValueError, "Headless captures require Linux"):
                live.run_once(Path("app"), Path("repo"), "startup", Path("unused"), 1)

    def test_profiler_loss_or_missing_quality_is_not_a_valid_capture(self):
        self.assertTrue(perf_report.perf_quality("# Total Lost Samples: 0\n")["valid"])
        self.assertFalse(perf_report.perf_quality("# Total Lost Samples: 54\n")["valid"])
        self.assertFalse(perf_report.perf_quality("")["valid"])
        self.assertFalse(perf_report.perf_quality("# Total Lost Samples: 0\n", 1)["valid"])

    def manifest(self, session="one", candidate=130, baseline=100, pairs=3):
        cases = []
        for pair in range(pairs):
            for variant, value in (("baseline", baseline), ("candidate", candidate)):
                cases.append({"key": "case", "variant": variant, "pair": pair, "status": "passed",
                              "workload": {"fixture": "same"}, "measurement_kind": "backend_operation",
                              "result": {"milliseconds": value}})
        return {"session": session, "comparison_environment": {"host": "test"}, "cases": cases}

    def test_regression_requires_independent_pairs_and_two_sessions(self):
        first = self.manifest()
        self.assertEqual(perf_report.compare([first])["comparisons"][0]["verdict"], "needs_confirmation")
        second = self.manifest("two", pairs=2)
        self.assertEqual(perf_report.compare([first, second])["comparisons"][0]["verdict"], "regression")

    def test_aa_is_not_a_regression(self):
        result = perf_report.compare([self.manifest(candidate=100), self.manifest("two", candidate=100)])
        self.assertTrue(result["valid"])
        self.assertEqual(result["comparisons"][0]["verdict"], "no_detected_regression")

    def test_identical_binaries_report_noise_instead_of_a_code_regression(self):
        first, second = self.manifest(), self.manifest("two", pairs=2)
        for manifest in (first, second):
            manifest["binaries"] = {variant: {"sha256": "same"} for variant in ("baseline", "candidate")}
        result = perf_report.compare([first, second])
        self.assertEqual(result["experiment"], "A/A")
        self.assertEqual(result["comparisons"][0]["verdict"], "noise_alert")

    def test_environment_workload_duplicates_and_missing_pairs_are_rejected(self):
        for modification in ("workload", "missing", "duplicate"):
            data = self.manifest(pairs=1)
            if modification == "workload":
                data["cases"][1]["workload"] = {"fixture": "different"}
            elif modification == "missing":
                data["cases"].pop()
            else:
                data["cases"].append(copy.deepcopy(data["cases"][0]))
            self.assertFalse(perf_report.compare([data])["valid"], modification)

    def test_diagnostic_latency_is_not_shipping_latency(self):
        data = self.manifest(pairs=1)
        data["cases"][0]["measurement_kind"] = "diagnostic"
        self.assertFalse(perf_report.compare([data])["valid"])

    def test_zero_baseline_does_not_invent_a_ratio(self):
        self.assertIsNone(perf_report.compare([self.manifest(baseline=0)])["comparisons"][0]["ratio"])

    def test_failed_pairs_cannot_disappear_behind_successful_pairs(self):
        data = self.manifest()
        for case in data["cases"][:2]:
            case["status"] = "failed"
        self.assertFalse(perf_report.compare([data])["valid"])

    def test_zero_baselines_do_not_count_toward_ratio_sample_size(self):
        first, second = self.manifest(), self.manifest("two", pairs=2)
        first["cases"][0]["result"]["milliseconds"] = 0
        result = perf_report.compare([first, second])["comparisons"][0]
        self.assertEqual(result["ratio_pairs"], 4)
        self.assertEqual(result["verdict"], "needs_confirmation")

    def test_changed_binaries_and_workloads_between_sessions_are_rejected(self):
        first, second = self.manifest(), self.manifest("two")
        first["binaries"] = {"candidate": {"sha256": "one"}}
        second["binaries"] = {"candidate": {"sha256": "two"}}
        self.assertFalse(perf_report.compare([first, second])["valid"])
        second["binaries"] = first["binaries"]
        for case in second["cases"]:
            case["workload"] = {"fixture": "changed"}
        self.assertFalse(perf_report.compare([first, second])["valid"])

    def test_allocation_comparison_excludes_instrumented_timings(self):
        data = self.manifest()
        for case in data["cases"]:
            case["measurement_kind"] = "diagnostic"
            case["result"].update(allocation_tracking=True, allocation_phases=[
                {"phase": "hover", "alloc_ops": 100, "alloc_bytes": 4000}])
        result = perf_report.compare([data], allocations=True)
        self.assertTrue(result["valid"])
        self.assertEqual({r["metric"] for r in result["comparisons"]}, {"hover.alloc_ops", "hover.alloc_bytes"})

    def test_missing_metric_cannot_silently_pass_comparison(self):
        data = self.manifest(pairs=1)
        data["cases"][0]["result"]["cpu_s"] = 1
        self.assertFalse(perf_report.compare([data])["valid"])
        self.assertFalse(perf_report.compare([])["valid"])

    def test_stall_and_retention_are_separate_from_invalid_capture(self):
        result = {"valid": True, "phases": {"hover": {"wake_ms": {"max": 1200}}},
                  "retention": {"rss_kib": {"growth_per_cycle": 100}}}
        issues = perf_report.findings(result)
        self.assertEqual({i["kind"] for i in issues}, {"target_exceeded", "retention_candidate"})
        self.assertEqual(issues[0]["severity"], "high")

    def test_multiple_windows_cannot_satisfy_another_windows_input(self):
        records = [{"event": "scenario_input", "detail": {"op": 1, "window": "A"}}]
        stages = {1: [{"stage": "input", "at_ms": 10, "a": 10_000_000, "b": 1},
                      {"stage": "input_handled", "at_ms": 11, "a": 1_000_000},
                      {"stage": "witness", "at_ms": 12, "a": 1}]}
        draws = [(13, {"window": "B", "at_ms": 14, "duration_ms": 1}),
                 (30, {"window": "A", "at_ms": 35, "duration_ms": 5})]
        result = live.analyse_phase({"at_ms": 0}, {"at_ms": 100}, draws, [], stages, [], records, [], [], {"unix_ms": 0})
        self.assertEqual(result["inputs"]["input_to_draw_ms"]["p50"], 25)
        records[0]["detail"]["intentional_dwell"] = True
        result = live.analyse_phase({"at_ms": 0}, {"at_ms": 100}, draws, [], stages, [], records, [], [], {"unix_ms": 0})
        self.assertEqual(result["inputs"]["input_to_draw_ms"]["count"], 0)

    def test_long_frame_is_retained_and_dropped_records_still_invalidate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture = {"run_id": "test", "scenario": "startup", "outcome": "passed",
                       "binary_sha256": "b", "repository_head": "r", "spawn_unix_ms": 1000}
            records = [{"event": "start", "run_id": "test", "unix_ms": 1000},
                       {"event": "scenario_ready", "at_ms": 1, "unix_ms": 1001},
                       {"event": "draw", "window": "A", "dirty_ms": 5, "start_ms": 1500,
                        "at_ms": 1501, "duration_ms": 1},
                       {"event": "scenario_end", "detail": {"outcome": "passed"}}]
            (root / "capture.json").write_text(json.dumps(capture))
            (root / "process.jsonl").write_text("")
            def summarize():
                (root / "frames.jsonl").write_text("".join(json.dumps(r) + "\n" for r in records))
                return live.summarize(root)
            result = summarize()
            self.assertTrue(result["valid"])
            self.assertEqual(len(result["long_frames"]), 1)
            records.append({"event": "interval", "records_dropped": 1})
            self.assertFalse(summarize()["valid"])

    def test_missing_optional_counters_are_not_zero(self):
        process = [dict(unix_ms=1, cpu_s=None, rss_kib=20, pss_kib=None, threads=None, fds=None),
                   dict(unix_ms=90, cpu_s=None, rss_kib=21, pss_kib=None, threads=None, fds=None)]
        result = live.analyse_phase({"at_ms": 0}, {"at_ms": 100}, [], [], {}, [], [], [], process, {"unix_ms": 0})
        self.assertIsNone(result["process_cpu_cores"])
        self.assertIsNone(result["pss_kib"]["max"])
        self.assertIsNone(result["threads"])


class FixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        cls.root = Path(cls.directory.name)
        cls.seed = live.create_fixture(cls.root / "seed", 100, 10)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def test_transport_clone_fetch_pull_and_push_have_real_witnesses(self):
        original = perf_corpus.identity(self.seed)
        for operation in ("clone", "fetch", "pull", "pull-merge", "pull-rebase", "push", "fetch-noop"):
            with self.subTest(operation=operation):
                with perf_workloads.transfer(self.root / operation, self.seed, operation, live) as fixture:
                    if operation == "fetch":
                        with self.assertRaises(AssertionError):
                            fixture["verify"]()
                    perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
                    self.assertIn("expected_head", fixture["verify"]())
                    self.assertTrue(fixture["server"].records)
                    if operation in ("clone", "fetch", "pull"):
                        self.assertGreaterEqual(sum(r["sent_bytes"] for r in fixture["server"].records), fixture["bytes"])
        self.assertEqual(perf_corpus.identity(self.seed), original)

    def test_disconnect_is_not_a_successful_fetch(self):
        with perf_workloads.transfer(self.root / "disconnect", self.seed, "fetch", live, fault="disconnect") as fixture:
            with self.assertRaises(RuntimeError):
                perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
            with self.assertRaises(AssertionError):
                fixture["verify"]()

    def test_worktree_identity_detects_changes_without_changing_refs(self):
        before = perf_corpus.worktree_identity(self.seed)
        file = self.seed / "unexpected.txt"
        file.write_text("first", encoding="utf-8")
        self.assertNotEqual(perf_corpus.worktree_identity(self.seed), before)
        changed = perf_corpus.worktree_identity(self.seed)
        file.write_text("other", encoding="utf-8")
        self.assertNotEqual(perf_corpus.worktree_identity(self.seed), changed)
        file.unlink()
        self.assertEqual(perf_corpus.worktree_identity(self.seed), before)

    def test_owned_read_only_annex_directories_can_be_cleaned(self):
        path = self.root / "readonly-fixture"
        path.mkdir()
        (path / "content").write_bytes(b"annex")
        (path / "content").chmod(0o444)
        path.chmod(0o555)
        perf_platform.remove_owned_tree(path)
        self.assertFalse(path.exists())

    def test_real_snapshot_detached_head_is_cloneable(self):
        seed = self.root / "detached-seed"
        subprocess.run(["git", "clone", "-q", str(self.seed), str(seed)], check=True)
        perf_workloads.git(seed, live.fixture_env(), "checkout", "--detach", "HEAD")
        with perf_workloads.transfer(self.root / "detached-clone", seed, "clone", live) as fixture:
            perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
            self.assertIn("expected_head", fixture["verify"]())

    def test_lfs_download_upload_and_checkout_hash_the_content(self):
        if subprocess.run(["git", "lfs", "version"], capture_output=True).returncode:
            self.skipTest("git-lfs not installed")
        for operation in ("lfs-fetch", "lfs-push", "lfs-pull"):
            with self.subTest(operation=operation):
                with perf_workloads.transfer(self.root / operation, self.seed, operation, live) as fixture:
                    perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
                    self.assertEqual(fixture["verify"]()["objects"], 4)

    def test_annex_uses_real_content_and_directory_remote(self):
        if subprocess.run(["git", "annex", "version"], capture_output=True).returncode:
            self.skipTest("git-annex not installed")
        for operation in ("annex-get", "annex-copy", "annex-pull", "annex-push", "annex-sync"):
            with self.subTest(operation=operation):
                with perf_workloads.transfer(self.root / operation, self.seed, operation, live) as fixture:
                    perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
                    self.assertEqual(fixture["verify"]()["objects"], 4)


if __name__ == "__main__":
    unittest.main()
