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
import performance
import perf_report
import perf_workloads

live = perf_workloads.module("test_live_ui", "live-ui.py")
multi_window = perf_workloads.module("test_multi_window", "multi-window.py")


class ReportingTests(unittest.TestCase):
    def test_strict_runs_cannot_pass_missing_cases(self):
        cases = [{"status": "passed"}, {"status": "skipped", "reason": "backend unavailable"}]
        self.assertEqual(perf_report.exit_status(cases), 0)
        self.assertEqual(perf_report.exit_status(cases, strict=True), 1)
        self.assertEqual(perf_report.exit_status([{"status": "failed"}]), 1)
        self.assertEqual(perf_report.exit_status([{"status": "passed"}], strict=True), 0)

    def test_dev_latency_has_separate_alerts_and_does_not_hide_second_long_stalls(self):
        result = {"debug_assertions": True, "phases": {"typing": {
            "draw_ms": {"p95": 30}, "wake_ms": {"max": 200},
            "inputs": {"input_to_draw_ms": {"p95": 90}}}}}
        self.assertEqual(perf_report.findings(result), [])
        result["debug_assertions"] = False
        self.assertEqual(len(perf_report.findings(result)), 3)
        result["debug_assertions"] = True
        result["phases"]["typing"]["wake_ms"]["max"] = 1500
        issues = perf_report.findings(result)
        self.assertEqual([(i["build_mode"], i["severity"]) for i in issues], [("dev", "high")])
        result["allocation_tracking"] = True
        self.assertEqual(perf_report.findings(result), [])

    def test_dev_pairs_are_comparable_but_dev_release_pairs_are_not(self):
        manifest = self.manifest(pairs=1)
        for case in manifest["cases"]:
            case["measurement_kind"] = "live_application"
            case["result"]["debug_assertions"] = True
        self.assertTrue(perf_report.compare([manifest])["valid"])
        manifest["cases"][1]["result"]["debug_assertions"] = False
        self.assertFalse(perf_report.compare([manifest])["valid"])

    def test_probe_cadence_must_match_between_pairs(self):
        manifest = self.manifest(pairs=1)
        for case, interval in zip(manifest["cases"], (250, 1000)):
            case["result"]["probe_interval_ms"] = interval
        self.assertFalse(perf_report.compare([manifest])["valid"])

    def test_native_work_counters_stay_in_their_phase_and_missing_data_is_not_zero(self):
        records = [{"event": "scenario_work", "detail": {"phase": phase, "counts": {"text_measurement": value}}}
                   for phase, value in (("name", 10), ("path", 90))]
        data = live.analyse_phase({"at_ms": 0, "detail": {"name": "path"}}, {"at_ms": 100},
                                  [], [], {}, [], records, [], [], {"unix_ms": 0})
        self.assertEqual(data["work_units"], {"text_measurement": 90})
        manifest = self.manifest(pairs=1)
        manifest["cases"][0]["result"]["phases"] = {"path": data}
        self.assertFalse(perf_report.compare([manifest])["valid"])
        self.assertEqual(perf_report.metrics({"phases": {"path": data}})["path.work_units.text_measurement"], 90)

    def test_findings_rank_stalls_and_include_artifacts_and_work_evidence(self):
        manifest = self.manifest(pairs=1)
        case = manifest["cases"][0]
        case.update(directory="capture/typing", findings=[
            {"kind": "retention_candidate", "severity": "investigate"},
            {"kind": "target_exceeded", "severity": "high", "metric": "path.draw_ms.max", "value": 1200, "limit": 250}])
        case["result"].update(debug_assertions=True, phases={"path": {"work_units": {"text_measurement": 1000}}})
        queue = perf_report.ranked_findings(manifest)
        self.assertEqual(queue[0]["severity"], "high")
        self.assertEqual(queue[0]["artifacts"], "capture/typing")
        self.assertEqual(queue[0]["work_units"], {"text_measurement": 1000})

    def test_transfers_without_overlapping_input_report_the_coverage_gap(self):
        result = {"operation_results": [{"name": "fetch", "overlapping_inputs": 0}]}
        self.assertEqual(perf_report.findings(result)[0]["kind"], "concurrency_not_exercised")
        result["operation_results"][0]["overlapping_inputs"] = 5
        self.assertEqual(perf_report.findings(result), [])

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

    def test_no_store_work_is_inapplicable_latency_not_a_zero_or_lost_sample(self):
        manifest = self.manifest(pairs=1)
        for case, count in zip(manifest["cases"], (0, 2)):
            case["result"]["phases"] = {"typing": {
                "inputs": {"store_queue_ms": {"count": count, "p95": .1 if count else None}},
                "work_counts": {"received:IndexedHistory": count}}}
        compared = perf_report.compare([manifest])
        self.assertTrue(compared["valid"], compared["errors"])
        self.assertEqual(compared["excluded_metrics"][0]["stage_samples"], [0, 2])
        self.assertFalse(any(c["metric"] == "typing.store_queue_ms.p95" for c in compared["comparisons"]))
        work = next(c for c in compared["comparisons"] if ".work." in c["metric"])
        self.assertEqual((work["baseline"], work["candidate"]), (0, 2))
        # Absent sample counts are not proof that no work occurred.
        del manifest["cases"][0]["result"]["phases"]["typing"]["inputs"]["store_queue_ms"]["count"]
        self.assertFalse(perf_report.compare([manifest])["valid"])

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

    def test_applied_publications_are_scoped_to_the_input_window(self):
        records = [
            {"event": "scenario_input", "detail": {"op": 1, "window": "A"}},
            {"event": "state_applied", "at_ms": 13,
             "detail": {"window": "B", "publication": 50, "duration_ns": 9_000_000}},
            {"event": "state_applied", "at_ms": 20,
             "detail": {"window": "A", "publication": 2, "duration_ns": 1_000_000}},
            {"event": "scenario_windows", "at_ms": 0, "detail": {"windows": [
                {"name": "primary", "window": "A", "active": True},
                {"name": "other", "window": "B", "active": False}]}}
        ]
        stages = {1: [{"stage": "input", "at_ms": 10, "a": 10_000_000, "b": 1},
                      {"stage": "reduced", "at_ms": 11, "a": 1000, "b": 2},
                      {"stage": "witness", "at_ms": 21, "a": 1}]}
        draws = [(30, {"window": "A", "at_ms": 35, "duration_ms": 5})]
        args = ({"at_ms": 0}, {"at_ms": 100}, draws, [], stages, [], records, [], [], {"unix_ms": 0})
        result = live.analyse_phase(*args)
        self.assertEqual(result["inputs"]["published_to_applied_ms"]["p50"], 9)
        self.assertEqual(result["inputs"]["apply_ms"]["p50"], 1)
        self.assertEqual(result["windows"]["B"]["frames"], 0)
        self.assertEqual(result["windows"]["A"]["frames"], 1)
        # A different store's larger sequence must never satisfy window A.
        records.pop(2)
        result = live.analyse_phase(*args)
        self.assertEqual(result["inputs"]["apply_ms"]["count"], 0)
        # Legacy unscoped events are ambiguous even if only one window drew.
        records.pop(1)
        applied = [(2, {"a": 2, "b": 9_000_000, "at_ms": 13})]
        result = live.analyse_phase(*args[:5], applied, *args[6:])
        self.assertEqual(result["inputs"]["apply_ms"]["count"], 0)

    def test_multi_window_isolation_uses_snapshot_after_last_input(self):
        def snapshot(at, primary, other):
            return {"event": "scenario_windows", "at_ms": at, "detail": {"windows": [
                {"name": "primary", "window": "A", "selected_commit": primary},
                {"name": "window-1", "window": "B", "selected_commit": other}]}}
        records = [snapshot(0, "one", "two"),
                   {"event": "scenario_phase", "at_ms": 1, "detail": {"name": "focus_1", "state": "begin"}},
                   snapshot(2, "one", "two"),  # focus changed, before the first input
                   {"event": "scenario_phase", "at_ms": 10, "detail": {"name": "focus_1", "state": "end"}},
                   snapshot(11, "one", "three")]
        with patch.object(multi_window.live, "load_records", return_value=records):
            multi_window.verify_isolation(Path("unused"))
            records[-1] = snapshot(11, "changed incorrectly", "three")
            with self.assertRaisesRegex(ValueError, "another window's selection"):
                multi_window.verify_isolation(Path("unused"))
            records.pop()
            with self.assertRaisesRegex(ValueError, "missing window snapshot"):
                multi_window.verify_isolation(Path("unused"))

    def test_repository_loading_is_not_reported_as_keystroke_latency(self):
        result = {"valid": True, "phases": {name: {
            "inputs": {"input_to_draw_ms": {"p95": 1000}}}
            for name in ("warmup", "open_2", "cycle_0", "select")}}
        findings = multi_window.findings(result)
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["metric"], "select.input_to_draw_ms.p95")

    def test_window_retention_uses_settled_resources_after_closing(self):
        result = {"phases": {
            "before_cycles": {"end_sample": {"fds": 80, "pss_kib": 1000}},
            "cycle_9": {"fds": 95, "end_sample": {"fds": 80, "pss_kib": 1500}},
            "after_cycles": {"fds": 82, "end_sample": {"fds": 80, "pss_kib": 1300}}}}
        data = multi_window.retention(result, 10)
        self.assertEqual(data["fds"]["growth_per_cycle"], 0)
        self.assertEqual(data["pss_kib"]["growth_per_cycle"], 30)
        self.assertIsNone(data["private_kib"]["growth_per_cycle"])

    def test_background_coverage_requires_input_in_another_window_during_its_load(self):
        records = [{"event": "scenario_phase", "at_ms": at, "detail": {"name": name, "state": state}}
                   for name, state, at in (("background_load", "begin", 0), ("background_load", "end", 10),
                                          ("select_while_loading", "begin", 10), ("select_while_loading", "end", 100))]
        records += [{"event": "scenario_input", "at_ms": at, "detail": {"op": op, "window": window}}
                    for op, window, at in ((1, "background", 5), (2, "primary", 40), (3, "background", 40))]
        records += [{"event": "stage", "stage": "input", "at_ms": 40, "op": op} for op in (2, 3)]
        records += [{"event": "stage", "stage": "task_finished", "label": "LoadLog", "op": 1,
                     "at_ms": 50, "a": 30_000_000}]
        with patch.object(multi_window.live, "load_records", return_value=records):
            self.assertEqual(multi_window.background_overlap(Path("unused"))["overlapping_inputs"], 1)
            del records[5]["detail"]["window"]
            self.assertEqual(multi_window.background_overlap(Path("unused"))["overlapping_inputs"], 0)
            records[5]["detail"]["window"] = "primary"
            records[-1]["at_ms"] = 30
            self.assertEqual(multi_window.background_overlap(Path("unused"))["overlapping_inputs"], 0)
            # An unrelated load cannot establish background-window coverage.
            records[-1].update(op=0, at_ms=50)
            self.assertEqual(multi_window.background_overlap(Path("unused"))["overlapping_inputs"], 0)

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
            records.append({"event": "interval", "records_dropped": 1, "work_units": {"index_object_read": 10}})
            incomplete = summarize()
            self.assertFalse(incomplete["valid"])
            self.assertFalse(incomplete["work_units_complete"])
            self.assertEqual(incomplete["work_units"], {"index_object_read": 10})
            records.append({"event": "scenario_work_total", "detail": {"counts": {"index_object_read": 12}}})
            complete = summarize()
            self.assertFalse(complete["valid"], "a final work snapshot must not hide dropped records")
            self.assertTrue(complete["work_units_complete"])
            self.assertEqual(complete["work_units"], {"index_object_read": 12})

    def test_missing_optional_counters_are_not_zero(self):
        process = [dict(unix_ms=1, cpu_s=None, rss_kib=20, pss_kib=None, threads=None, fds=None),
                   dict(unix_ms=90, cpu_s=None, rss_kib=21, pss_kib=None, threads=None, fds=None)]
        result = live.analyse_phase({"at_ms": 0}, {"at_ms": 100}, [], [], {}, [], [], [], process, {"unix_ms": 0})
        self.assertIsNone(result["process_cpu_cores"])
        self.assertIsNone(result["pss_kib"]["max"])
        self.assertIsNone(result["threads"])


class ScenarioTests(unittest.TestCase):
    def test_history_interactions_wait_for_the_index_and_jumps_wait_for_content(self):
        for name in ("history-hover", "history-scroll", "history-select-burst", "history-jump"):
            steps = live.scenario(name, Path("/fixture"))["steps"]
            self.assertEqual(steps[0]["do"], "wait_ready")
            self.assertTrue(steps[1]["indexed"], name)
            if name == "history-jump":
                jumps = [step for step in steps if step["do"] == "drag_history"]
                self.assertEqual(len(jumps), 6)
                self.assertTrue(all(step["wait_for_rows"] for step in jumps))

    def test_picker_uses_both_shortcuts_real_paste_and_checks_final_results(self):
        steps = live.scenario("repo-picker", Path("/fixture"))["steps"]
        self.assertEqual({s["key"] for s in steps if s["do"] == "keys" and "shift" in s["key"]},
                         {"secondary-shift-a", "secondary-shift-o"})
        self.assertTrue(all(s.get("witness") == {"kind": "repo_picker_filtered"}
                            for s in steps if s["do"] in ("type", "paste")))
        assertions = [s for s in steps if s["do"] == "expect_repo_picker"]
        self.assertEqual({s["matches"] for s in assertions}, {0, 1, live.PICKER_RECENT_COUNT, live.PICKER_UNFILTERED_COUNT})
        paths = live.picker_recent_paths()
        self.assertEqual(len(set(paths)), live.PICKER_RECENT_COUNT)
        self.assertTrue(all("GitComet" in p and "company" in p and len(p) > 300 for p in paths))
        self.assertEqual(sum("component-017" in p for p in paths), 1)
        for repository in (Path("/home/developer/git/GitComet"), Path("/home/developer/git/gitcomet_pro")):
            name_query = next(s["text"] for s in live.scenario("repo-picker", repository)["steps"]
                              if s["do"] == "type")
            self.assertEqual(sum(name_query.casefold() in p.casefold() for p in paths), live.PICKER_RECENT_COUNT)
            self.assertNotIn(name_query.casefold(), str(repository).casefold())

    def test_smoke_covers_typing_and_deep_exercises_large_history(self):
        corpus = {"fixtures": {name: {"kind": "synthetic"} for name in ("history-20000", "history-2000000")}}
        smoke = list(performance.cases(corpus, "smoke"))
        self.assertTrue(any(c["scenario"] == "repo-picker" for c in smoke))
        deep = list(performance.cases(corpus, "deep"))
        scenarios = {c["scenario"] for c in deep if c["fixture"] == "history-2000000"}
        self.assertLessEqual({"history-jump", "history-reopen", "history-hover-stationary", "history-select-burst"}, scenarios)
        self.assertTrue(all(c["scenario"] in live.SCENARIOS for c in deep if c["layer"] == "ui"))


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

    def test_lfs_warm_and_mixed_destinations_transfer_only_missing_content(self):
        if subprocess.run(["git", "lfs", "version"], capture_output=True).returncode:
            self.skipTest("git-lfs not installed")
        for operation in ("lfs-fetch", "lfs-pull", "lfs-push"):
            for cache, transfers in (("mixed", 2), ("warm", 0)):
                with self.subTest(operation=operation, content_cache=cache):
                    with perf_workloads.transfer(self.root / f"{operation}-{cache}", self.seed, operation, live, content_cache=cache) as fixture:
                        perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
                        witness = fixture["verify"]()
                        self.assertEqual(witness["objects"], 4)
                        self.assertEqual(witness["required_content_bytes"], transfers * 4096)
                        self.assertEqual(len(fixture["server"].records), transfers)

    def test_annex_warm_and_mixed_destinations_preserve_content_witnesses(self):
        if subprocess.run(["git", "annex", "version"], capture_output=True).returncode:
            self.skipTest("git-annex not installed")
        for operation in ("annex-get", "annex-copy"):
            for cache, missing in (("mixed", 2), ("warm", 0)):
                with self.subTest(operation=operation, content_cache=cache):
                    with perf_workloads.transfer(self.root / f"{operation}-{cache}", self.seed, operation, live, content_cache=cache) as fixture:
                        if missing:
                            with self.assertRaises((AssertionError, FileNotFoundError)):
                                fixture["verify"]()
                        perf_workloads.git(fixture["repo"], fixture["env"], *fixture["cli"])
                        self.assertEqual(fixture["verify"]()["required_content_bytes"], missing * 4096)

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
