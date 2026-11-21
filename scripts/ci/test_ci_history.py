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

    def test_a_test_that_passes_and_fails_on_one_lane_is_flaky(self):
        failing = [("state", "a", True), ("state", "b", True), ("state", "c", False)]
        passing = [("state", "a", False), ("state", "b", True), ("state", "c", False)]
        found = ci_history.flaky_tests([("linux", failing), ("linux", passing), ("windows", passing)])
        self.assertEqual(
            [(item["lane"], item["test"], item["passed"], item["failed"], item["flaky"]) for item in found],
            [("linux", "b", 0, 2, False), ("linux", "a", 1, 1, True), ("windows", "b", 0, 1, False)])

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
