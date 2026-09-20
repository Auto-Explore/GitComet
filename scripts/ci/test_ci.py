"""Regression coverage for failure propagation, inventory accounting, and cache isolation."""

import copy
from contextlib import redirect_stdout
from functools import partial
import importlib.util
import io
from itertools import product
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import cache
import report
import runtime

spec = importlib.util.spec_from_file_location("ci_runner", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class CacheTests(unittest.TestCase):
    def test_dependency_changes_reuse_only_compatible_compiled_bundles(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(cache, "ROOT", Path(directory)), \
                patch.object(cache, "subprocess") as subprocess_mock, \
                patch.object(cache.platform, "platform", return_value="test-platform"), \
                patch.object(cache.platform, "system", return_value="Windows"), \
                patch.dict(os.environ, {}, clear=True):
            # Keep the Rust probe mock separate from subprocess calls in platform.
            subprocess_mock.check_output.return_value = b"rustc test host"
            root = Path(directory)
            manifest = root / "Cargo.toml"
            manifest.write_text('[package]\nname = "fixture"\nversion = "0.1.0"\n[profile.ci-test]\nopt-level = 1\n')
            lock = root / "Cargo.lock"
            lock.write_text("dependencies v1")
            before = cache.cache_keys("windows-arm64-workspace-ci-test")
            self.assertEqual(before["source-restore-key"], f"{cache.PREFIX}sources-windows-")
            self.assertTrue(before["source-key"].startswith(before["source-restore-key"]))
            lock.write_text("dependencies v2")
            after = cache.cache_keys("windows-arm64-workspace-ci-test")
            self.assertNotEqual(before["key"], after["key"])
            self.assertEqual(before["restore-key"], after["restore-key"])
            self.assertTrue(before["key"].startswith(after["restore-key"]))
            manifest.write_text(manifest.read_text().replace("opt-level = 1", "opt-level = 0"))
            profile = cache.cache_keys("windows-arm64-workspace-ci-test")
            self.assertNotEqual(after["restore-key"], profile["restore-key"])
            other = cache.cache_keys("windows-x64-workspace-ci-test")
            self.assertNotEqual(profile["restore-key"], other["restore-key"])
            self.assertEqual(profile["source-key"], other["source-key"])
            # Windows uppercases these names; exercise that spelling on every host.
            for name in ("ImageOS", "ImageVersion", "IMAGEOS", "IMAGEVERSION"):
                with self.subTest(image_variable=name), patch.dict(os.environ, {name: "runner image v1"}):
                    image_before = cache.cache_keys("windows-x64-workspace-ci-test")
                    self.assertNotEqual(other["restore-key"], image_before["restore-key"])
                    os.environ[name] = "runner image v2"
                    image_after = cache.cache_keys("windows-x64-workspace-ci-test")
                    self.assertNotEqual(image_before["restore-key"], image_after["restore-key"])
                    self.assertEqual(other["source-key"], image_after["source-key"])

    def test_compatible_prefix_restore_reports_actual_bundle_mode(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(cache, "ROOT", Path(directory)), \
                patch.dict(os.environ, {"CI_CACHE_HIT": "false", "CI_CACHE_RESTORED": "true",
                                       "CI_CACHE_MATCHED_KEY": "compatible-older-lockfile", "CI_COLD_CACHE": "false"}, clear=True), \
                redirect_stdout(io.StringIO()):
            root = Path(directory)
            source = root / "source"
            source.write_text("retained timestamp")
            os.utime(source, (1700000000, 1700000000))
            bundle = root / "cache.tar.gz"
            cache.write_bundle(bundle, [(source, "cargo/registry/src/source"), (source, "target/ci-test/deps/libexternal.rlib")], mode="dependencies")
            details = cache.restore(bundle, root / "cargo", root / "target")
            self.assertEqual(details["mode"], "dependencies")
            self.assertEqual((root / "cargo/registry/src/source").stat().st_mtime, 1700000000)
            self.assertTrue((root / "target/ci-test/deps/libexternal.rlib").exists())
            reports = root / "target/ci-reports"
            reports.mkdir()
            (reports / "cache-unpack.json").write_text(json.dumps(details))
            cache.report_restore()
            state = json.loads((reports / "cache-restore.json").read_text())
            self.assertEqual(state["mode"], "dependencies")
            self.assertEqual(state["CI_CACHE_HIT"], "false")
            self.assertEqual(state["CI_CACHE_RESTORED"], "true")
            with patch.dict(os.environ, {"CI_COLD_CACHE": "true"}):
                cache.report_restore()
                self.assertEqual(json.loads((reports / "cache-restore.json").read_text())["mode"], "bypassed")

    def test_cache_migration_waits_for_replacements_in_the_same_scope(self):
        def entry(key, scope="refs/heads/dev"):
            return {"key": key, "ref": scope, "created_at": "2026-09-19"}
        native = entry("gitcomet-ci-v1-windows-x64-workspace-ci-test-old")
        bench = entry("gitcomet-ci-v1-windows-x64-benchmarks-ci-bench-old")
        unrelated = entry("release-cache")
        other_scope = entry("gitcomet-ci-v2-deps-windows-x64-workspace-ci-test-compat-new", "refs/heads/main")
        self.assertEqual(report.obsolete_caches([native, bench, unrelated, other_scope]), [])
        replacement = entry("gitcomet-ci-v2-deps-windows-x64-workspace-ci-test-compat-new")
        sources = entry("gitcomet-ci-v2-sources-windows-new")
        obsolete = report.obsolete_caches([native, bench, unrelated, other_scope, replacement, sources])
        self.assertCountEqual(obsolete, [native, bench])

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
    @staticmethod
    def git_integration_suites():
        targets = {
            "gitcomet": ("difftool_git_integration", "mergetool_git_integration", "standalone_tool_mode_integration"),
            "gitcomet-git-gix": ("submodules_integration", "remote_management_integration", "status_integration",
                                 "refs_integration", "upstream_integration", "upstream_divergence_integration", "log_integration"),
        }
        return {name: {"package-id": package, "binary-name": name, "testcases": {"required": {"ignored": False}}}
                for package, names in targets.items() for name in names}

    @staticmethod
    def write_junit(path, suites):
        path.write_text('<testsuites>' + ''.join(
            f'<testsuite name="{name}"><testcase name="required"/></testsuite>' for name in suites
        ) + '</testsuites>')

    def test_schedules_share_routing_and_cpu_budgets_on_every_platform(self):
        packages = {name: name for name in ("gitcomet", "gitcomet-git-gix", "gitcomet-core", runner.UI)}
        nextest_suites = self.git_integration_suites()
        nextest_suites["core"] = {"package-id": "gitcomet-core", "binary-name": "gitcomet_core", "testcases": {"required": {"ignored": False}}}
        ui_suites = {"ui": {"package-id": runner.UI, "binary-name": "gitcomet_ui_gpui"}}
        for platform_name, (schedule, threads), (cpus, group), profile in product(
                ("linux", "darwin", "win32"), (("serial", None), ("serial", 8), ("balanced", None)),
                ((1, "both"), (3, "both"), (4, "both"), (4, "nextest"), (4, "libtest")), runner.NEXTEST_PROFILES):
            with self.subTest(platform=platform_name, schedule=schedule, cpus=cpus, group=group, profile=profile), \
                    tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                    patch.object(runner.sys, "platform", platform_name), \
                    patch.object(runner.os, "cpu_count", return_value=cpus):
                selected = (nextest_suites | ui_suites) if group == "both" else nextest_suites if group == "nextest" else ui_suites
                parallel = schedule == "balanced" and cpus > 1 and group == "both"
                target = Path(directory)
                (target / "workspace").mkdir()
                (target / "workspace/binaries.json").write_text(json.dumps({"rust-build-meta": {"target-directory": directory}}))
                (target / "nextest" / profile).mkdir(parents=True)
                barrier, completed = threading.Barrier(2), []

                def run_nextest(name, command, **kwargs):
                    self.assertNotEqual(group, "libtest")
                    self.assertEqual(command[command.index("-E") + 1], f"not package(={runner.UI})")
                    self.assertEqual(command[command.index("--profile") + 1], profile)
                    if parallel:
                        self.assertEqual(command[-2:], ["--test-threads", str(cpus - max(1, cpus // 2))])
                        barrier.wait(timeout=5)
                    elif threads is not None:
                        self.assertEqual(command[-2:], ["--test-threads", str(threads)])
                    else:
                        self.assertNotIn("--test-threads", command)
                    self.write_junit(target / "nextest" / profile / "junit.xml", nextest_suites)
                    completed.append("nextest")
                    return 0

                def run_ui(context, binary, suite, **kwargs):
                    self.assertEqual(binary, "ui", "Git integration suites must run through nextest")
                    if parallel:
                        self.assertEqual(kwargs["threads"], max(1, cpus // 2))
                        barrier.wait(timeout=5)
                    else:
                        self.assertEqual(completed, [] if group == "libtest" else ["nextest"])
                        self.assertIsNone(kwargs.get("threads"))
                    completed.append("ui")
                    return 0

                with patch.object(runner, "package_names", return_value=packages), \
                        patch.object(runner, "inventory", return_value={"rust-suites": selected}), \
                        patch.object(runner, "run", side_effect=run_nextest), patch.object(runner, "run_suite", side_effect=run_ui):
                    runner.execute("workspace", schedule, threads, profile)
                expected = ["nextest", "ui"] if group == "both" else ["nextest"] if group == "nextest" else ["ui"]
                self.assertCountEqual(completed, expected)
                execution = json.loads((target / "workspace/execution.json").read_text())
                self.assertTrue(execution["success"])
                self.assertEqual(execution["nextest_profile"], profile)
                self.assertEqual(execution["effective_schedule"], "balanced" if parallel else "serial")

    def test_invalid_concurrency_is_rejected_before_running(self):
        for schedule, threads in [("serial", 0), ("serial", -1), ("balanced", 8)]:
            with self.subTest(schedule=schedule, threads=threads), self.assertRaises(ValueError):
                runner.execute("workspace", schedule, threads)
        with self.assertRaises(ValueError):
            runner.execute("workspace", nextest_profile="unknown")

    def test_parallel_failure_cancels_a_running_process_tree_and_records_it(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}), redirect_stdout(io.StringIO()):
            heartbeat = Path(directory) / "heartbeat"
            child = ("import pathlib, sys, time\npath = pathlib.Path(sys.argv[1])\n"
                     "while True:\n    path.write_text(str(time.monotonic_ns()))\n    time.sleep(0.02)\n")
            parent = ("import subprocess, sys, time; "
                      "subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]]); time.sleep(60)")

            def fail_after_start(**kwargs):
                deadline = time.monotonic() + 10
                while not heartbeat.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(heartbeat.exists(), "child did not start")
                raise RuntimeError("inventory failure")

            with self.assertRaisesRegex(RuntimeError, "inventory failure"):
                runner.run_parallel([
                    partial(runner.run, "cancelled", [sys.executable, "-c", parent, child, str(heartbeat)], timeout=15),
                    fail_after_start,
                ])
            before = heartbeat.read_text()
            time.sleep(0.15)
            self.assertEqual(heartbeat.read_text(), before, "descendant survived cancellation")
            timing = json.loads((Path(directory) / "timings.jsonl").read_text())
            self.assertTrue(timing["cancelled"])
            self.assertEqual(timing["returncode"], 130)

    def test_failed_runner_does_not_skip_other_tests_on_any_platform(self):
        packages = {"core": "gitcomet-core", "ui": runner.UI}
        suites = {"core": {"package-id": "core", "binary-name": "gitcomet_core", "testcases": {"required": {"ignored": False}}},
                  "ui": {"package-id": "ui", "binary-name": "gitcomet_ui_gpui"}}
        for platform_name, schedule, failed in product(("linux", "darwin", "win32"), ("serial", "balanced"), ("nextest", "ui")):
            with self.subTest(platform=platform_name, schedule=schedule, failed=failed), \
                    tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                    patch.object(runner.sys, "platform", platform_name), patch.object(runner.os, "cpu_count", return_value=4):
                target = Path(directory)
                (target / "workspace").mkdir()
                (target / "workspace/binaries.json").write_text(json.dumps({"rust-build-meta": {"target-directory": directory}}))
                (target / "nextest/ci").mkdir(parents=True)
                completed = []

                def run_nextest(*args, **kwargs):
                    self.write_junit(target / "nextest/ci/junit.xml", ["core"])
                    completed.append("nextest")
                    return 100 if failed == "nextest" else 0

                def run_ui(*args, **kwargs):
                    completed.append("ui")
                    return 1 if failed == "ui" else 0

                with patch.object(runner, "package_names", return_value=packages), \
                        patch.object(runner, "inventory", return_value={"rust-suites": suites}), \
                        patch.object(runner, "run", side_effect=run_nextest), patch.object(runner, "run_suite", side_effect=run_ui):
                    with self.assertRaisesRegex(RuntimeError, "test execution failed"):
                        runner.execute("workspace", schedule)
                self.assertCountEqual(completed, ["nextest", "ui"], "one failed runner must not skip the rest")
                self.assertFalse(json.loads((target / "workspace/execution.json").read_text())["success"])

    def test_all_git_integration_suites_require_nextest_results_on_every_platform(self):
        suites = self.git_integration_suites()
        packages = {name: name for name in ("gitcomet", "gitcomet-git-gix")}
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            for platform_name in ("linux", "darwin", "win32"):
                with self.subTest(platform=platform_name), patch.object(runner.sys, "platform", platform_name):
                    self.write_junit(junit, suites)
                    runner.check_nextest_results("workspace", suites, packages, junit)
                    for missing in suites:
                        with self.subTest(missing=missing):
                            self.write_junit(junit, [name for name in suites if name != missing])
                            with self.assertRaisesRegex(RuntimeError, "1 missing, 0 unexpected"):
                                runner.check_nextest_results("workspace", suites, packages, junit)

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

    def test_libtest_uses_default_or_requested_threads_on_every_platform(self):
        suite = {"binary-path": "unused", "cwd": runner.ROOT, "testcases": {"required": {"ignored": False}}}
        for platform_name, threads in product(("linux", "darwin", "win32"), (None, 3)):
            with self.subTest(platform=platform_name, threads=threads), tempfile.TemporaryDirectory() as directory, \
                    patch.object(runner, "REPORTS", Path(directory)), patch.object(runner.sys, "platform", platform_name), \
                    patch.object(runner, "suite_env", return_value={}), patch.object(runner, "run", return_value=0) as run:
                (Path(directory) / "workspace-ui-all.log").write_text("test result: ok. 1 passed; 0 failed;\n")
                self.assertEqual(runner.run_suite("workspace", "ui", suite, threads=threads), 0)
                command = run.call_args.args[1]
                if threads is None:
                    self.assertNotIn("--test-threads", command)
                else:
                    self.assertEqual(command[-2:], ["--test-threads", str(threads)])

    def test_ignored_or_unreported_test_is_not_counted_as_executed(self):
        suites = {"core": {"package-id": "core", "binary-name": "gitcomet_core",
                           "testcases": {"required": {"ignored": False}}}}
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)):
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="core"><testcase name="required"><skipped/></testcase></testsuite></testsuites>')
            for platform_name in ("linux", "darwin", "win32"):
                with self.subTest(platform=platform_name), patch.object(runner.sys, "platform", platform_name):
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
                with self.subTest(platform=platform_name), patch.object(runner.sys, "platform", platform_name):
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
                with self.subTest(platform=platform_name), patch.object(runner.sys, "platform", platform_name):
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


class RuntimeTests(unittest.TestCase):
    def test_repetitions_keep_distinct_raw_logs_and_restore_report_directory(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            root = Path(directory)
            reports = root / "reports"
            source = reports / "workspace"
            source.mkdir(parents=True)
            (source / "coverage.json").write_text('{"profile": "ci-test", "selection": ["--workspace"]}')
            (source / "binaries.json").write_text('"compiled-once"')
            (source / "execution.json").write_text('{"success": true, "seconds": 99}')
            (source / "junit.xml").write_text("stale")
            calls = []

            def execute(context, schedule, threads, profile):
                self.assertEqual(profile, "ci-git-limited")
                calls.append(len(calls) + 1)
                sample = runtime.runner.paths(context)
                self.assertEqual((sample / "binaries.json").read_text(), '"compiled-once"')
                self.assertFalse((sample / "execution.json").exists())
                self.assertFalse((sample / "junit.xml").exists())
                for name in ("workspace-nextest.log", "workspace-ui-all.log", "timings.jsonl"):
                    (runtime.runner.REPORTS / name).write_text(str(len(calls)))
                (sample / "execution.json").write_text(json.dumps(dict(success=True, seconds=len(calls))))

            with patch.object(runtime.runner, "REPORTS", reports), \
                    patch.object(runtime.runner, "execute", side_effect=execute), \
                    patch.object(runtime.subprocess, "check_output", return_value="test"):
                runtime.measure(root / "output", 2, "serial", None, "ci-git-limited")
                self.assertEqual(runtime.runner.REPORTS, reports)
            for index in (1, 2):
                for name in ("workspace-nextest.log", "workspace-ui-all.log", "timings.jsonl"):
                    self.assertEqual((root / f"output/sample-{index}" / name).read_text(), str(index))
            self.assertEqual(json.loads((source / "execution.json").read_text())["seconds"], 99)

    def test_instrumented_acceptance_run_is_rejected(self):
        for name, value in [("GIT_TRACE2_EVENT", "trace.json"), ("GIT_TRACE2", "1"),
                            ("GIT_TRACE2_PERF", "trace.perf"), ("GITCOMET_TEST_SYNC_TRACE", "")]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory, \
                    patch.dict(os.environ, {name: value}, clear=True):
                with self.assertRaisesRegex(ValueError, "Disable instrumentation"):
                    runtime.measure(Path(directory) / "result", 5, "serial", None)
                self.assertFalse((Path(directory) / "result").exists())

    def test_failed_sample_retains_report_and_cannot_reuse_previous_success(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            root = Path(directory)
            reports = root / "reports"
            reports.mkdir()
            (reports / "execution.json").write_text('{"success": true, "seconds": 1}')
            with patch.object(runtime.runner, "paths", return_value=reports), \
                    patch.object(runtime.runner, "execute", side_effect=RuntimeError("failed")), \
                    patch.object(runtime.subprocess, "check_output", return_value="test"), \
                    self.assertRaisesRegex(RuntimeError, "failed"):
                runtime.measure(root / "output", 5, "serial", None)
            record = json.loads((root / "output/runtime.json").read_text())
            self.assertEqual(record["samples"], [{"success": False, "seconds": None, "schedule": "serial", "nextest_threads": None, "nextest_profile": "ci"}])
            self.assertTrue((root / "output/sample-1").is_dir())


class ReportTests(unittest.TestCase):
    def test_trace2_keeps_nested_processes_and_incomplete_exits(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.jsonl"
            events = [dict(event="start", sid="parent", argv=["git", "status"]),
                      dict(event="start", sid="parent/child", argv=["git", "config"]),
                      dict(event="exit", sid="parent/child", t_abs=0.25, code=0)]
            path.write_text("\n".join(json.dumps(event) for event in events))
            result = report.git_trace2(path)
            self.assertEqual(result["processes"], 2)
            self.assertEqual(result["incomplete"], 1)
            self.assertEqual(result["records"][1]["seconds"], 0.25)

    def test_runtime_report_separates_hardware_and_counts_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, (job, machine, samples) in enumerate([
                ("101/1/native/runner", "AMD64", [500, 510, 490]),
                ("102/1/native/runner", "AMD64", [505, 495]),
                ("103/1/native/runner", "ARM64", [600]),
            ]):
                target = root / str(index)
                target.mkdir()
                (target / "runtime.json").write_text(json.dumps(dict(
                    platform="win32", machine=machine, cpus=4, job=job, sha="abc",
                    os_version="windows", runner_image="2026.09", git="git 2", rust="rust 1",
                    profile="ci-test", selection=["--workspace"],
                    samples=[dict(success=True, seconds=s, schedule="serial", nextest_threads=None) for s in samples])))
            rows = report.runtime_statistics(root)
            x64 = next(row for row in rows if row["machine"] == "AMD64")
            self.assertEqual(x64["median_seconds"], 500)

            self.assertTrue(x64["enough_samples"])
            path = root / "0/runtime.json"
            record = json.loads(path.read_text())
            record["samples"].append(dict(success=False, seconds=900, schedule="serial", nextest_threads=None))
            path.write_text(json.dumps(record))
            x64 = next(row for row in report.runtime_statistics(root) if row["machine"] == "AMD64")
            self.assertFalse(x64["enough_samples"])
            self.assertEqual(x64["median_seconds"], 500)

    def test_runtime_report_does_not_mix_environments_or_count_copied_artifacts(self):
        record = dict(platform="win32", machine="AMD64", cpus=4, sha="abc", job="101/1/native/runner",
                      measurement_id="first", os_version="windows", runner_image="2026.09",
                      git="git 2", rust="rust 1", profile="ci-test", selection=["--workspace"],
                      samples=[dict(success=True, seconds=500, schedule="serial", nextest_threads=None)] * 3)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("first", "copy"):
                (root / name).mkdir()
                (root / name / "runtime.json").write_text(json.dumps(record))
            self.assertEqual(report.runtime_statistics(root)[0]["samples"], 3)
            for field in ("os_version", "runner_image", "git", "rust", "profile", "selection"):
                changed = dict(record, measurement_id=field, job="102/1/native/runner")
                changed[field] = ["-p", "core"] if field == "selection" else "different"
                (root / field).mkdir()
                (root / field / "runtime.json").write_text(json.dumps(changed))
            rows = report.runtime_statistics(root)
            self.assertEqual(len(rows), 7)
            self.assertTrue(all(row["samples"] == 3 and not row["enough_samples"] for row in rows))

    def test_fixture_metrics_keep_nested_setup_and_subprocess_costs_separate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "101.tsv").write_text("case\tsetup\tinit-repository\t1000000\ncase\tsubprocess\tinit\t600000\n")
            (root / "102.tsv").write_text("case\tsubprocess\tinit\t400000\ncase\tconfig\twrite\t2000\n")
            rows = report.fixture_timings(root)
            subprocess_row = next(row for row in rows if row["phase"] == "subprocess")
            self.assertEqual(subprocess_row["calls"], 2)
            self.assertEqual(subprocess_row["seconds"], 1)
            self.assertEqual(len(rows), 3)

    def test_runtime_report_keeps_concurrency_profiles_separate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = dict(platform="win32", machine="AMD64", cpus=4, sha="abc", job="local/test",
                          samples=[dict(success=True, seconds=100, schedule="serial", nextest_threads=8),
                                   dict(success=True, seconds=300, schedule="serial", nextest_threads=8,
                                        nextest_profile="ci-git-limited")])
            (root / "runtime.json").write_text(json.dumps(record))
            rows = report.runtime_statistics(root)
            self.assertEqual({row["nextest_profile"]: row["median_seconds"] for row in rows},
                             {"ci": 100, "ci-git-limited": 300})

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
