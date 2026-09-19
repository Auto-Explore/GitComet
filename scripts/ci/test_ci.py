"""Regression coverage for failure propagation, inventory accounting, and cache isolation."""

import copy
from contextlib import redirect_stdout
from functools import partial
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import time
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
    def test_windows_git_suites_preserve_shared_setup_and_mutexes(self):
        for name in ("mergetool_git_integration", "status_integration", "refs_integration",
                     "upstream_integration", "upstream_divergence_integration", "log_integration"):
            suite = {"binary-name": name}
            self.assertTrue(runner.uses_libtest("gitcomet-git-gix", suite, "win32"))
            self.assertFalse(runner.uses_libtest("gitcomet-git-gix", suite, "linux"))
        self.assertFalse(runner.uses_libtest("gitcomet-core", {"binary-name": "gitcomet_core"}, "win32"))

    def test_silent_timeout_kills_descendants_and_records_failure(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}), redirect_stdout(io.StringIO()):
            heartbeat = Path(directory) / "heartbeat"
            child = ("import pathlib, sys, time\n"
                     "path = pathlib.Path(sys.argv[1])\n"
                     "while True:\n"
                     "    path.write_text(str(time.monotonic_ns()))\n"
                     "    time.sleep(0.02)\n")
            parent = ("import subprocess, sys, time; "
                      "subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]]); "
                      "time.sleep(60)")
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                runner.run("hung-suite", [sys.executable, "-c", parent, child, str(heartbeat)],
                           timeout=3 if os.name == "nt" else 1)
            self.assertEqual(raised.exception.returncode, 124)
            before = heartbeat.read_text()
            time.sleep(0.15)
            self.assertEqual(heartbeat.read_text(), before, "descendant survived timeout")
            timing = json.loads((Path(directory) / "timings.jsonl").read_text())
            self.assertTrue(timing["timed_out"])
            self.assertEqual(timing["returncode"], 124)

    def test_nonzero_child_status_is_not_masked(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}), redirect_stdout(io.StringIO()):
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                runner.run("failure", [sys.executable, "-c", "print('failure output'); raise SystemExit(7)"])
            self.assertEqual(raised.exception.returncode, 7)
            timing = json.loads((Path(directory) / "timings.jsonl").read_text())
            self.assertEqual(timing["returncode"], 7)
            self.assertIn("failure output", (Path(directory) / "failure.log").read_text())

    def test_cli_preserves_unicode_output_with_legacy_stdio_encoding(self):
        stdout = "──────── nextest ────────\nPASS 日本語 🦀\n"
        stderr = "diagnostic: 中文 → 🦀\n"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "scripts/ci/run.py"
            script.parent.mkdir(parents=True)
            script.write_bytes(Path(runner.__file__).read_bytes())
            env = dict(os.environ, PYTHONIOENCODING="cp1252:strict", PYTHONUTF8="0", GITHUB_STEP_SUMMARY="")
            for code in (0, 7):
                with self.subTest(child_exit=code):
                    name = f"unicode-{code}"
                    child = (f"import sys; sys.stdout.buffer.write({stdout.encode('utf-8')!r}); "
                             f"sys.stdout.buffer.flush(); sys.stderr.buffer.write({stderr.encode('utf-8')!r}); "
                             f"sys.stderr.buffer.flush(); sys.exit({code})")
                    # Success exercises log forwarding; failure also exercises
                    # a Unicode command argument in the CLI's error on stderr.
                    extra = ["日本語/🦀.txt"] if code else []
                    result = subprocess.run(
                        [sys.executable, str(script), "command", "--name", name, "--",
                         sys.executable, "-c", child, *extra], env=env,
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                    self.assertEqual(result.returncode, 1 if code else 0,
                                     result.stderr.decode("utf-8", errors="replace"))
                    console = result.stdout.decode("utf-8").replace("\r\n", "\n")
                    self.assertIn(stdout, console)
                    self.assertIn(stderr, console)
                    reports = root / "target/ci-reports"
                    self.assertEqual((reports / f"{name}.log").read_text(encoding="utf-8"), stdout + stderr)
                    timing = json.loads((reports / "timings.jsonl").read_text(encoding="utf-8").splitlines()[-1])
                    self.assertEqual(timing["returncode"], code)
                    self.assertFalse(timing["timed_out"])
                    if code:
                        self.assertIn(extra[0], result.stderr.decode("utf-8"))

    def test_missing_smoke_selector_fails_before_execution(self):
        suite = {"testcases": {"real_test": {"ignored": False}}}
        with self.assertRaisesRegex(RuntimeError, "matches no tests"):
            runner.run_suite("app", "app::smoke", suite, test_filter="misspelled", exact=True)

    def test_ignored_or_unreported_test_is_not_counted_as_executed(self):
        suites = {"core": {"package-id": "core", "binary-name": "gitcomet_core",
                           "testcases": {"required": {"ignored": False}}}}
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="core"><testcase name="required"><skipped/></testcase></testsuite></testsuites>')
            for platform_name in ("linux", "darwin", "win32"):
                select_runner = partial(runner.uses_libtest, platform_name=platform_name)
                with self.subTest(platform=platform_name), patch.object(runner, "uses_libtest", select_runner):
                    with self.assertRaisesRegex(RuntimeError, "coverage mismatch"):
                        runner.check_nextest_results("core", suites, {"core": "gitcomet-core"}, junit)

    def test_ui_owned_tests_are_not_required_in_nextest_results(self):
        suites = {
            "core": {"package-id": "core", "binary-name": "gitcomet_core",
                     "testcases": {"required": {"ignored": False}}},
            "ui": {"package-id": "ui", "binary-name": "gitcomet_ui_gpui",
                   "testcases": {"render": {"ignored": False}}},
        }
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="core"><testcase name="required"/></testsuite></testsuites>')
            for platform_name in ("linux", "darwin", "win32"):
                select_runner = partial(runner.uses_libtest, platform_name=platform_name)
                with self.subTest(platform=platform_name), patch.object(runner, "uses_libtest", select_runner):
                    runner.check_nextest_results("workspace", suites, {"core": "gitcomet-core", "ui": runner.UI}, junit)

    def test_successful_nextest_exit_cannot_hide_git_prerequisite_skip(self):
        suites = {"core": {"package-id": "core", "binary-name": "gitcomet_core",
                           "testcases": {"required": {"ignored": False}}}}
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="core"><testcase name="required">'
                             '<system-err>skipping status integration test: Git-for-Windows shell startup failed</system-err>'
                             '</testcase></testsuite></testsuites>')
            for platform_name in ("linux", "darwin", "win32"):
                select_runner = partial(runner.uses_libtest, platform_name=platform_name)
                with self.subTest(platform=platform_name), patch.object(runner, "uses_libtest", select_runner):
                    with self.assertRaisesRegex(RuntimeError, "required Git test did not run"):
                        runner.check_nextest_results("core", suites, {"core": "gitcomet-core"}, junit)

    def test_libtest_count_cannot_hide_git_prerequisite_skip(self):
        suite = {"binary-path": "unused", "binary-name": "status_integration", "cwd": runner.ROOT,
                 "testcases": {"required": {"ignored": False}}}
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                patch.object(runner, "suite_env", return_value={}), patch.object(runner, "run", return_value=0):
            (Path(directory) / "workspace-status_integration-all.log").write_text(
                "skipping status integration test: Git-for-Windows local push shell startup failed\n"
                "test result: ok. 1 passed; 0 failed;\n")
            with self.assertRaisesRegex(RuntimeError, "required Git test did not run"):
                runner.run_suite("workspace", "status_integration", suite)

    def test_display_profiles_reuse_workspace_ui_and_headless_app(self):
        with patch.object(sys, "argv", ["run.py", "display"]), patch.object(runner, "smoke") as smoke:
            runner.main()
        self.assertEqual(smoke.call_count, 9)
        for index, values in enumerate(runner.DISPLAY_PROFILES.values()):
            calls = smoke.call_args_list[index * 3:index * 3 + 3]
            self.assertEqual([call.args[0] for call in calls], ["app", "app", "workspace"])
            self.assertEqual(calls[2].args[1], "gitcomet_ui_gpui")
            self.assertEqual(calls[2].kwargs["env"]["XDG_CURRENT_DESKTOP"], values[3])


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
