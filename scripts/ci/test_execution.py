"""Execution partition, capacity, diagnostics, and concurrent report ownership."""

from concurrent.futures import ThreadPoolExecutor
import json
import io
from contextlib import redirect_stdout
from pathlib import Path
import sys
import tempfile
import threading
import tomllib
import unittest
from unittest.mock import patch

import report
import resources
import run as runner


class ExecutionTests(unittest.TestCase):
    def test_background_stderr_and_captured_result_text_cannot_corrupt_coverage(self):
        output = ("running 1 test\ntest required ... ok\n\nsuccesses:\n\n"
                  "---- required stdout ----\ntest forged ... ok\n\nsuccesses:\n"
                  "    required\n\ntest result: ok. 1 passed; 0 failed;\n")
        diagnostic = "test stderr_forged ... ok\nbackground diagnostic\n"
        script = ("import sys, threading, time\n"
                  f"out={output!r}; err={diagnostic!r}\n"
                  "def write(stream, text):\n"
                  " for char in text:\n  stream.write(char); stream.flush(); time.sleep(0.0001)\n"
                  "worker=threading.Thread(target=write,args=(sys.stderr,err))\n"
                  "worker.start(); write(sys.stdout,out); worker.join()\n")
        suite = dict(**{"binary-path": "unused"}, cwd=runner.ROOT, testcases={"required": {"ignored": False}})
        real_run = runner.run
        def execute(name, command, **kwargs):
            return real_run(name, [sys.executable, "-c", script], **kwargs)
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                patch.object(runner, "suite_env", return_value={}), patch.object(runner, "run", side_effect=execute), \
                redirect_stdout(io.StringIO()):
            self.assertEqual(runner.run_suite("ui", "suite", suite), 0)
            raw = (Path(directory) / "ui-suite-all.log").read_text()
            self.assertEqual(raw, output + "\n--- subprocess stderr ---\n" + diagnostic)
            self.assertEqual((Path(directory) / "ui-suite-all.stderr.log").read_text(), diagnostic)

    def test_partition_rejects_overlapping_and_ambiguous_prefixes(self):
        suite = {"package-id": "core", "testcases": {"pure::a": {"ignored": False}}}
        suites, packages = {"core": suite}, {"core": "gitcomet-core"}
        batches = [("core", suite, "pure::")]
        self.assertEqual(runner.validate_partition(suites, packages, batches), ({("core", "pure::a")}, False))
        with self.assertRaisesRegex(RuntimeError, "duplicates"):
            runner.validate_partition(suites, packages, batches * 2)
        suite["testcases"]["other::pure::a"] = {"ignored": False}
        with self.assertRaisesRegex(RuntimeError, "ambiguous"):
            runner.validate_partition(suites, packages, batches)

    def test_balanced_budgets_reserve_all_families_and_reject_overcommit(self):
        for cpus in (3, 4, 8, 32, 64):
            budgets = runner.thread_budgets(cpus, ["ui", "nextest", "pure"], dict(ui=None, nextest=None, pure=None))
            self.assertEqual(sum(budgets.values()), cpus)
            self.assertGreaterEqual(min(budgets.values()), 1)
            self.assertLessEqual(budgets["ui"], 16)
        self.assertEqual(runner.thread_budgets(8, ["ui", "nextest", "pure"], dict(ui=2, nextest=None, pure=3)),
                         dict(ui=2, nextest=3, pure=3))
        with self.assertRaisesRegex(ValueError, "exceed"):
            runner.thread_budgets(8, ["ui", "nextest", "pure"], dict(ui=4, nextest=4, pure=None))

    def test_cpu_capacity_respects_affinity_and_parent_container_quota(self):
        files = {"/proc/self/cgroup": "0::/test/child\n", "/sys/fs/cgroup/test/child/cpu.max": "max 100000\n",
                 "/sys/fs/cgroup/test/cpu.max": "600000 100000\n", "/sys/fs/cgroup/cpu.max": "max 100000\n"}
        with patch.object(runner.os, "cpu_count", return_value=64), \
                patch.object(runner.os, "sched_getaffinity", return_value=set(range(32)), create=True), \
                patch.object(runner.os, "process_cpu_count", return_value=64, create=True), \
                patch.object(runner.sys, "platform", "linux"), \
                patch.object(Path, "read_text", lambda path: files[str(path)]):
            self.assertEqual(runner.available_cpus(), 6)

    def test_three_families_overlap_and_skip_only_audited_ui_tests(self):
        def suite(package, names):
            return {"package-id": package, "package-name": "gitcomet-core" if package == "core" else runner.UI,
                    "kind": "lib", "testcases": {
                name: {"ignored": False} for name in names}}
        suites = {"gitcomet-core": suite("core", ["conflict_session::a", "process::isolated"]),
                  runner.UI: suite("ui", ["view::word_diff::tests::a", "view::rendered"])}
        packages = dict(core="gitcomet-core", ui=runner.UI)
        barrier, calls = threading.Barrier(3), []
        with tempfile.TemporaryDirectory() as directory, patch.object(runner, "REPORTS", Path(directory)), \
                patch.object(runner, "package_names", return_value=packages), \
                patch.object(runner, "prepare_runtime_binaries"), patch.object(runner, "available_cpus", return_value=8), \
                patch.object(runner, "inventory", return_value={"rust-suites": suites}):
            def nextest(name, command, **kwargs):
                config = command[command.index("--config-file") + 1]
                junit = runner.nextest_junit_path("ci-throughput", Path(config))
                junit.parent.mkdir(parents=True)
                junit.write_text('<testsuites><testsuite name="gitcomet-core">'
                                 '<testcase name="process::isolated"/></testsuite></testsuites>')
                self.assertEqual(command[-2:], ["--test-threads", "2"])
                calls.append("nextest")
                barrier.wait(timeout=5)
                return 0

            def libtest(context, binary, suite, **kwargs):
                if kwargs.get("test_filter"):
                    self.assertEqual(kwargs["threads"], 2)
                    self.assertTrue(kwargs["verify_names"])
                    calls.append("pure")
                    if binary == "gitcomet-core":
                        barrier.wait(timeout=5)
                else:
                    self.assertEqual(kwargs["threads"], 4)
                    self.assertEqual(kwargs["skip_prefixes"], ("view::word_diff::tests::",))
                    calls.append("ui")
                    barrier.wait(timeout=5)
                return 0

            with patch.object(runner, "run", side_effect=nextest), patch.object(runner, "run_suite", side_effect=libtest):
                runner.execute("workspace", "balanced", nextest_profile="ci-throughput", batch_pure_tests="on")
            self.assertCountEqual(calls, ["ui", "nextest", "pure", "pure"])
            execution = json.loads((runner.paths("workspace") / "execution.json").read_text())
            self.assertTrue(execution["success"])
            self.assertEqual(execution["effective_schedule"], "balanced")

    def test_concurrent_nextest_configs_preserve_settings_and_own_distinct_stores(self):
        for store_section in ('', '[store] # custom store\ndir = "old"\n', 'store.dir = "old"\n'):
            with self.subTest(store=store_section), tempfile.TemporaryDirectory() as directory, \
                    patch.object(runner, "ROOT", Path(directory)), patch.object(runner, "REPORTS", Path(directory) / "reports"):
                config = Path(directory) / ".config/nextest.toml"
                config.parent.mkdir()
                raw = store_section + '[profile.ci.junit]\npath="junit.xml"\n[profile.child]\ninherits="ci"\n'
                config.write_text(raw)
                with ThreadPoolExecutor(2) as executor:
                    copies = list(executor.map(runner.isolated_nextest_config, ("workspace", "workspace")))
                paths = [runner.nextest_junit_path("child", copy) for copy in copies]
                self.assertNotEqual(paths[0], paths[1])
                expected = tomllib.loads(raw)
                expected.pop("store", None)
                for copy in copies:
                    parsed = tomllib.loads(copy.read_text())
                    parsed.pop("store")
                    self.assertEqual(parsed, expected)
                self.assertEqual(config.read_text(), raw)

    def test_inventory_fingerprint_tracks_names_and_binary_fingerprint_tracks_content(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "tests"
            binary.write_bytes(b"first")
            suites = {"core": {"binary-path": str(binary), "testcases": {"a": {"ignored": False}}}}
            before = runner.execution_fingerprints(suites)
            binary.write_bytes(b"second-binary")
            after = runner.execution_fingerprints(suites)
            self.assertEqual(before[0], after[0])
            self.assertNotEqual(before[1], after[1])
            suites["core"]["testcases"]["a"]["ignored"] = True
            self.assertNotEqual(after[0], runner.execution_fingerprints(suites)[0])

    def test_resource_instrumentation_never_qualifies_for_acceptance(self):
        with tempfile.TemporaryDirectory() as directory:
            record = dict(resource_stats=True, job="local/1", local_session="one", samples=[
                dict(success=True, seconds=1, resource_stats={"observed_peak_rss_bytes": 1})] * 6)
            (Path(directory) / "runtime.json").write_text(json.dumps(record))
            row, = report.runtime_statistics(Path(directory))
            self.assertTrue(row["resource_instrumented"])
            self.assertFalse(row["local_enough_samples"])
            self.assertFalse(row["enough_samples"])
            self.assertEqual(row["resource_diagnostics"]["observed_peak_rss_bytes"], dict(samples=6, median=1, max=1))


class ResourceTests(unittest.TestCase):
    def test_macos_thread_probe_does_not_misreport_entitlement_fallback(self):
        process = "10 1 20 1:02.00 Thu Oct 8 12:00:00 2026\n"
        for thread_rows, expected in (("99\n10\n", None), ("99\n99\n10\n10\n10\n", 3)):
            with self.subTest(threads=thread_rows), patch.object(resources.os, "getpid", return_value=99), \
                    patch.object(resources.subprocess, "check_output", side_effect=[process, thread_rows]):
                row = resources.macos_processes()[10]
                self.assertEqual(row["threads"], expected)
                self.assertEqual(row["rss"], 20480)
                self.assertEqual(row["cpu"], 62)

    def test_sampler_tracks_descendants_retains_exited_cpu_and_rejects_pid_reuse(self):
        def row(parent, identity, rss, cpu):
            return dict(parent=parent, identity=identity, rss=rss, cpu=cpu, threads=2)
        snapshots = iter([{1: row(0, "root", 10, 1), 2: row(1, "child", 20, 2), 3: row(0, "other", 100, 100)},
                          {2: row(0, "child", 30, 3)}, {2: row(0, "reused", 100, 100)}])
        sampler = resources.ProcessSampler(collector=lambda: next(snapshots))
        sampler.add(1)
        sampler._sample()
        sampler.remove(1)
        sampler._sample()
        sampler._sample()
        summary = sampler.summary()
        self.assertEqual(summary["observed_peak_rss_bytes"], 30)
        self.assertEqual(summary["observed_peak_threads"], 4)
        self.assertEqual(summary["observed_cpu_seconds"], 4)
        self.assertEqual(summary["samples"], 2)

    def test_unavailable_memory_is_missing_and_collector_failure_is_reported(self):
        sampler = resources.ProcessSampler(collector=lambda: {1: dict(parent=0, identity=1, rss=None, cpu=1, threads=None)})
        sampler.add(1)
        sampler._sample()
        self.assertIsNone(sampler.summary()["observed_peak_rss_bytes"])
        entered = threading.Event()
        def fail():
            entered.set()
            raise OSError("fixture failure")
        failed = resources.ProcessSampler(collector=fail)
        failed.start()
        self.assertTrue(entered.wait(timeout=3))
        failed.stop()
        self.assertIn("fixture failure", str(failed.summary()["errors"]))
