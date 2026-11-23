from pathlib import Path
import tempfile
import unittest

import ci_history

JUNIT = """<?xml version="1.0"?>
<testsuites>
  <testsuite name="gitcomet-state">
    <testcase name="store::passes" classname="gitcomet-state"/>
    <testcase name="store::fails" classname="gitcomet-state"><failure type="test failure"/></testcase>
    <testcase name="store::skipped" classname="gitcomet-state"><skipped/></testcase>
  </testsuite>
</testsuites>"""


class CiHistoryTests(unittest.TestCase):
    def test_junit_outcomes_skip_tests_that_did_not_run(self):
        self.assertEqual(ci_history.junit_outcomes(JUNIT), [
            ("gitcomet-state", "store::passes", False),
            ("gitcomet-state", "store::fails", True),
        ])

    def test_a_test_that_passes_and_fails_on_one_lane_at_one_commit_is_flaky(self):
        failing = [("state", "a", True), ("state", "b", True), ("state", "c", False)]
        passing = [("state", "a", False), ("state", "b", True), ("state", "c", False)]
        found = ci_history.flaky_tests(
            [("c1", "linux", failing), ("c1", "linux", passing), ("c1", "windows", passing)])
        self.assertEqual(
            [(item["lane"], item["test"], item["passed"], item["failed"], item["flaky"]) for item in found],
            [("linux", "b", 0, 2, False), ("linux", "a", 1, 1, True), ("windows", "b", 0, 1, False)])

    def test_a_test_fixed_by_a_later_commit_is_not_flaky(self):
        found = ci_history.flaky_tests([
            ("c1", "linux", [("state", "a", True)]),
            ("c2", "linux", [("state", "a", False)]),
        ])
        self.assertEqual([(item["passed"], item["failed"], item["flaky"]) for item in found], [(1, 1, False)])

    def test_libtest_logs_give_one_outcome_per_result_line(self):
        log = "running 2 tests\ntest view::a ... ok\ntest view::b ... FAILED\n\ntest result: FAILED. 1 passed\n"
        self.assertEqual(ci_history.libtest_outcomes(log, "ui-all"), [
            ("ui-all", "view::a", False),
            ("ui-all", "view::b", True),
        ])

    def test_libtest_logs_count_tests_that_should_panic(self):
        log = "test view::a - should panic ... ok\ntest view::b - should panic ... FAILED\n"
        self.assertEqual(ci_history.libtest_outcomes(log, "ui-all"), [
            ("ui-all", "view::a", False),
            ("ui-all", "view::b", True),
        ])

    def test_reports_count_each_junit_report_once_and_skip_unreadable_ones(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reports = root / "100" / "ci-linux" / "ci-reports" / "workspace"
            store = reports / "nextest-0123456789abcdef" / "ci"
            store.mkdir(parents=True)
            (reports / "junit.xml").write_text(JUNIT, encoding="utf-8")
            (store / "junit.xml").write_text(JUNIT, encoding="utf-8")
            broken = root / "100" / "ci-windows" / "ci-reports"
            broken.mkdir(parents=True)
            (broken / "junit.xml").write_text("<testsuites><testsuite>", encoding="utf-8")
            found = ci_history.test_reports(root)
            self.assertEqual([lane for _, lane, _ in found], ["ci-linux"])

    def test_reports_read_junit_and_libtest_logs_but_not_nextest_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reports = root / "100" / "ci-linux" / "ci-reports"
            reports.mkdir(parents=True)
            (root / "100" / ci_history.COMMIT_FILE).write_text("abc\n", encoding="utf-8")
            (reports / "junit.xml").write_text(JUNIT, encoding="utf-8")
            (reports / "ui-all.log").write_text("test view::b ... FAILED\n", encoding="utf-8")
            (reports / "workspace-nextest.log").write_text("test store::fails ... FAILED\n", encoding="utf-8")
            (reports / "compile.log").write_text("Compiling\n", encoding="utf-8")
            self.assertEqual(ci_history.test_reports(root), [
                ("abc", "ci-linux", [("gitcomet-state", "store::passes", False),
                                     ("gitcomet-state", "store::fails", True)]),
                ("abc", "ci-linux", [("ui-all", "view::b", True)]),
            ])

    def test_timing_units_split_the_library_from_its_test_build(self):
        html = ('<script>const UNIT_DATA = [{"name":"gitcomet-ui-gpui","target":"","duration":90.5,'
                '"sections":[["frontend",{"start":0.0,"end":40.0}],["codegen",{"start":40.0,"end":90.5}]]},'
                '{"name":"gitcomet-ui-gpui","target":" gitcomet_ui_gpui \\"lib\\" (test)","duration":200.0,"sections":null},'
                '{"name":"serde","target":"","duration":3.0,"sections":null}];\n</script>')
        self.assertEqual(ci_history.timing_units(html, "gitcomet-ui-gpui"), [
            {"target": "lib", "seconds": 90.5, "frontend_seconds": 40.0},
            {"target": 'gitcomet_ui_gpui "lib" (test)', "seconds": 200.0, "frontend_seconds": None},
        ])
        self.assertEqual(ci_history.timing_units("<html></html>", "gitcomet-ui-gpui"), [])


if __name__ == "__main__":
    unittest.main()
