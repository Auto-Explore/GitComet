"""Regression coverage for failure propagation, inventory accounting, and cache isolation."""

import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import cache
import report

spec = importlib.util.spec_from_file_location("ci_runner", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class CacheTests(unittest.TestCase):
    def test_roundtrip_preserves_dependency_and_source_but_no_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cargo = root / "cargo-home"
            source = cargo / "registry/src/registry/demo-1.0/lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("source")
            source.chmod(0o755)
            os.utime(source, (1700000000, 1700000000))
            (cargo / "credentials.toml").write_text("never archive")
            bundle = root / "bundle.tar.gz"
            cache.write_bundle(bundle, list(cache.source_entries(cargo)))
            restored = root / "restored"
            cache.restore(bundle, restored, root / "target")
            result = restored / source.relative_to(cargo)
            self.assertEqual(result.read_text(), "source")
            self.assertEqual(int(result.stat().st_mtime), 1700000000)
            self.assertFalse((restored / "credentials.toml").exists())

    def test_archive_cannot_escape_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "bundle.tar.gz"
            with tarfile.open(bundle, "w:gz") as archive:
                member = tarfile.TarInfo("cargo/../escaped")
                member.size = 1
                archive.addfile(member, io.BytesIO(b"x"))
            with self.assertRaises(tarfile.FilterError):
                cache.restore(bundle, root / "cargo", root / "target")
            self.assertFalse((root / "escaped").exists())

    def test_dependency_cache_excludes_workspace_and_vendored_code(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            deps = root / "ci-test/deps"
            deps.mkdir(parents=True)
            for name in ("libserde-0123456789abcdef.rlib", "libgitcomet_core-0123456789abcdef.rlib",
                         "libvendored_grammar-0123456789abcdef.rlib", "random-test-executable"):
                (deps / name).touch()
            metadata = {"packages": [
                {"name": "serde", "source": "registry+url", "targets": [{"name": "serde"}]},
                {"name": "gitcomet-core", "source": None, "targets": [{"name": "gitcomet_core"}]},
                {"name": "vendored-grammar", "source": None, "targets": [{"name": "vendored_grammar"}]},
            ]}
            entries = list(cache.dependency_entries(root, metadata))
            self.assertEqual([p.name for p, _ in entries], ["libserde-0123456789abcdef.rlib"])

    def test_oversized_bundle_is_not_published(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / "cargo/registry/cache"
            downloads.mkdir(parents=True)
            (downloads / "crate").write_bytes(os.urandom(4096))
            with patch.object(cache, "ROOT", root), patch.object(cache, "output") as output:
                cache.pack(root / "bundle.tar.gz", root / "cargo", root / "target", 512, False)
            self.assertIn(unittest.mock.call("save", "false"), output.call_args_list)
            self.assertFalse((root / "bundle.tar.gz").exists())

    def test_pruning_only_retires_owned_superseded_contexts(self):
        caches = [
            {"key": "gitcomet-ci-v1-macos15-arm-test-aaa", "created_at": "2026-09-18"},
            {"key": "gitcomet-ci-v1-macos15-arm-test-bbb", "created_at": "2026-09-19"},
            {"key": "gitcomet-ci-v1-macos26-arm-test-ccc", "created_at": "2026-09-18"},
            {"key": "unrelated-release-cache", "created_at": "2026-09-17"},
        ]
        self.assertEqual(report.obsolete_caches(caches), [caches[0]])


class RunnerTests(unittest.TestCase):
    def test_windows_capability_probes_keep_process_local_cache(self):
        suite = {"binary-name": "mergetool_git_integration"}
        self.assertTrue(runner.uses_libtest("gitcomet", suite, "win32"))
        self.assertFalse(runner.uses_libtest("gitcomet", suite, "linux"))
        self.assertFalse(runner.uses_libtest("gitcomet-core", {"binary-name": "gitcomet_core"}, "win32"))

    def test_nonzero_child_status_is_not_masked(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                runner.run("failure", [sys.executable, "-c", "print('failure output'); raise SystemExit(7)"])
            self.assertEqual(raised.exception.returncode, 7)
            timing = json.loads((Path(directory) / "timings.jsonl").read_text())
            self.assertEqual(timing["returncode"], 7)
            self.assertIn("failure output", (Path(directory) / "failure.log").read_text())

    def test_missing_smoke_selector_fails_before_execution(self):
        suite = {"testcases": {"real_test": {"ignored": False}}}
        with self.assertRaisesRegex(RuntimeError, "matches no tests"):
            runner.run_suite("app", "app::smoke", suite, test_filter="misspelled", exact=True)

    def test_ignored_or_unreported_test_is_not_counted_as_executed(self):
        suites = {"core": {"package-id": "core", "testcases": {"required": {"ignored": False}}}}
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="core"><testcase name="required"><skipped/></testcase></testsuite></testsuites>')
            with self.assertRaisesRegex(RuntimeError, "coverage mismatch"):
                runner.check_nextest_results("core", suites, {"core": "gitcomet-core"}, junit)

    def test_ui_owned_tests_are_not_required_in_nextest_results(self):
        suites = {
            "core": {"package-id": "core", "testcases": {"required": {"ignored": False}}},
            "ui": {"package-id": "ui", "testcases": {"render": {"ignored": False}}},
        }
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="core"><testcase name="required"/></testsuite></testsuites>')
            runner.check_nextest_results("workspace", suites, {"core": "gitcomet-core", "ui": runner.UI}, junit)


class ReportTests(unittest.TestCase):
    def test_renamed_platform_lanes_match_without_treating_timeouts_as_success(self):
        records = [{"jobs": [
            {"name": "Windows Tests (aarch64-windows)", "conclusion": "success", "seconds": 3300},
            {"name": "platforms / Native Tests (windows-arm64)", "conclusion": "success", "seconds": 1500},
            {"name": "macOS Tests (intel (macos-15-intel))", "conclusion": "cancelled", "seconds": 3600},
        ]}]
        lanes = report.lane_statistics(records)
        self.assertEqual(lanes["native/windows-arm64"]["median_seconds"], 2400)
        self.assertIsNone(lanes["native/macos15-x64"]["median_seconds"])
        self.assertEqual(len(lanes["native/macos15-x64"]["incomplete_samples"]), 1)

    def test_equal_counts_cannot_hide_a_missing_test_or_new_ignore(self):
        baseline = {"context": "workspace", "selection": ["--workspace"], "tests": [
            {"package": "core", "binary": "core", "test": name, "ignored": False} for name in ("a", "b")]}
        candidate = copy.deepcopy(baseline)
        candidate["tests"][0]["ignored"] = True
        candidate["tests"][1]["test"] = "replacement"
        result = report.coverage_difference(baseline, candidate)
        self.assertEqual(result["missing"], [("core", "core", "b")])
        self.assertEqual(result["newly_ignored"], [("core", "core", "a")])

    def test_workflow_wall_time_includes_queue_and_excludes_failed_samples(self):
        common = dict(sha="abc", branch="dev", event="push", conclusion="success")
        runs = [dict(common, created_at="2026-09-19T10:00:00Z", completed_at="2026-09-19T10:20:00Z", runner_seconds=600),
                dict(common, created_at="2026-09-19T10:00:00Z", completed_at="2026-09-19T10:30:00Z", runner_seconds=900),
                dict(common, sha="failed", conclusion="cancelled", created_at="2026-09-19T11:00:00Z", completed_at="2026-09-19T12:00:00Z", runner_seconds=3600)]
        summary = report.summarize(runs)
        self.assertEqual(summary["median_wall_seconds"], 1800)
        self.assertEqual(summary["median_runner_seconds"], 1500)
        self.assertEqual(len(summary["excluded_samples"]), 1)


if __name__ == "__main__":
    unittest.main()
