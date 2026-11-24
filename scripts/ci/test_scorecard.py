import tempfile
import unittest
from pathlib import Path

import scorecard


def long_function(name, lines):
    return f"fn {name}() {{\n" + "    step();\n" * (lines - 2) + "}\n"


class ScorecardTests(unittest.TestCase):
    def test_function_lengths_span_signature_to_closing_brace(self):
        code = (
            "impl Thing {\n"
            "    pub(crate) async fn first(\n"
            "        &self,\n"
            "        buffer: [u8; 4],\n"
            "    ) -> Option<u8> {\n"
            "        let text = \"}\";\n"
            "        None\n"
            "    }\n"
            "    fn first(&self) {}\n"
            "}\n"
            "trait Shape { fn declared(&self); }\n"
        )
        self.assertEqual(scorecard.function_lengths(code), {"first": 7, "first#2": 1})

    def test_item_members_ignore_commas_inside_types_and_payloads(self):
        code = (
            "pub enum Msg {\n"
            "    Open(PathBuf),\n"
            "    Select { repo: RepoId, map: FxHashMap<String, Vec<u8>> },\n"
            "    #[allow(dead_code)]\n"
            "    Close,\n"
            "}\n"
            "pub struct View {\n"
            "    callback: Box<dyn Fn(u8, u8) -> Result<(), Error>>,\n"
            "    pub(in crate::view) rows: [u8; 2]\n"
            "}\n"
        )
        self.assertEqual(scorecard.item_members(code, "enum", "Msg"), 3)
        self.assertEqual(scorecard.item_members(code, "struct", "View"), 2)
        self.assertIsNone(scorecard.item_members(code, "enum", "Missing"))

    def test_measure_separates_production_from_test_code(self):
        production = "use super::*;\nfn set_theme() { let _ = std::time::Instant::now(); }\n"
        test_module = "#[cfg(test)]\nmod tests {\n    use super::*;\n    fn t() { view.diff_renderer.read(); }\n}\n"
        path = "crates/gitcomet-ui-gpui/src/view/pane.rs"
        tests = "crates/gitcomet-ui-gpui/src/view/tests/pane.rs"
        texts = {path: production + test_module, tests: "fn t() { a.diff_renderer; b.diff_renderer; }\n"}
        import identity_literals
        found = scorecard.measure(texts, identity_literals.production_code(texts))
        self.assertEqual(found[("glob-super", "gitcomet-ui-gpui")], 1)
        self.assertEqual(found[("clock-calls", "gitcomet-ui-gpui")], 1)
        self.assertEqual(found[("set-theme-fns", "gitcomet-ui-gpui")], 1)
        self.assertEqual(found[("diff-renderer-refs", "gitcomet-ui-gpui")], 3)

    def test_every_glob_import_of_a_parent_and_every_clock_read_count(self):
        code = (
            "use super::super::*;\npub(crate) use super::*;\n"
            "fn f() { let clock = Instant::now; let started = clock(); started.elapsed(); }\n"
        )
        path = "crates/gitcomet-state/src/store/x.rs"
        import identity_literals
        found = scorecard.measure({path: code}, identity_literals.production_code({path: code}))
        self.assertEqual(found.get(("glob-super", "gitcomet-state"), 0), 2)
        self.assertEqual(found.get(("clock-calls", "gitcomet-state"), 0), 2)

    def test_a_lower_layer_naming_a_higher_one_is_counted(self):
        view = "crates/gitcomet-ui-gpui/src/view"
        texts = {
            f"{view}/rows/history.rs": "use crate::view::panes::Pane;\nfn f() { panels::open(); rows::draw(); }\n",
            f"{view}/panes/diff.rs": "fn f() { rows::draw(); super::panels::open(); }\n",
            f"{view}/panels/popover.rs": "fn f() { panes::main(); rows::draw(); }\n",
        }
        found = scorecard.measure(texts, texts)
        self.assertEqual({key: value for key, value in found.items() if key[0] == "upward-refs"}, {
            ("upward-refs", "gitcomet-ui-gpui::rows->panes"): 1,
            ("upward-refs", "gitcomet-ui-gpui::rows->panels"): 1,
            ("upward-refs", "gitcomet-ui-gpui::panes->panels"): 1,
        })

    def test_long_functions_are_recorded_by_file_and_name(self):
        path = "crates/gitcomet-core/src/big.rs"
        texts = {path: long_function("small", 149) + long_function("big", 150)}
        found = scorecard.measure(texts, texts)
        self.assertEqual(found, {("fn-lines", f"{path}::big"): 150})

    def test_counts_must_match_and_long_functions_may_only_shrink(self):
        recorded = {
            ("glob-super", "gitcomet-core"): (5, "-"),
            ("fn-lines", "a.rs::grown"): (200, "-"),
            ("fn-lines", "a.rs::shrunk"): (200, "-"),
            ("fn-lines", "a.rs::split"): (200, "-"),
        }
        found = {
            ("glob-super", "gitcomet-core"): 4,
            ("glob-super", "gitcomet-state"): 1,
            ("fn-lines", "a.rs::grown"): 201,
            ("fn-lines", "a.rs::shrunk"): 180,
            ("fn-lines", "a.rs::fresh"): 150,
        }
        self.assertEqual(scorecard.compare(found, recorded), [
            "a.rs::fresh is a new function of 150 lines; keep functions under 150",
            "a.rs::grown grew from 200 to 201 lines; split it instead",
            "a.rs::split is gone or under 150 lines; remove its fn-lines entry",
            "glob-super gitcomet-core: recorded 5 but 4 remain; lower the count",
            "glob-super gitcomet-state: 1 exceeds the recorded 0",
        ])
        self.assertEqual(scorecard.compare({("fn-lines", "a.rs::shrunk"): 180}, {("fn-lines", "a.rs::shrunk"): (200, "-")}), [])

    def test_write_lowers_values_and_never_raises_or_adds_one(self):
        with tempfile.TemporaryDirectory() as directory:
            baseline = Path(directory) / "baseline.txt"
            baseline.write_text("# header\nfn-lines a.rs::gone 200 -\nfn-lines a.rs::table 300 a data table\n"
                                "glob-super gitcomet-state 9 -\n", encoding="utf-8")
            recorded = scorecard.read_baseline(baseline)
            found = {("fn-lines", "a.rs::table"): 310, ("glob-super", "gitcomet-core"): 2,
                     ("glob-super", "gitcomet-state"): 7}
            scorecard.write(found, recorded, baseline)
            self.assertEqual(baseline.read_text(encoding="utf-8"),
                             "# header\nfn-lines a.rs::table 300 a data table\nglob-super gitcomet-state 7 -\n")
            self.assertEqual(len(scorecard.compare(found, scorecard.read_baseline(baseline))), 2)
            baseline.unlink()
            scorecard.write(found, {}, baseline)
            self.assertEqual(scorecard.compare(found, scorecard.read_baseline(baseline)), [])
            with self.assertRaisesRegex(ValueError, "expected"):
                baseline.write_text("fn-lines broken\n", encoding="utf-8")
                scorecard.read_baseline(baseline)

    def test_churn_names_hubs_and_what_changes_with_them(self):
        commits = [
            {"crates/a/src/hub.rs", "crates/b/src/other.rs"},
            {"crates/a/src/hub.rs", "crates/b/src/other.rs", "crates/c/src/x.rs", "crates/d/src/y.rs", "crates/e/src/z.rs"},
            {"crates/c/src/leaf.rs"},
        ]
        result = scorecard.churn(commits, hubs=2, pairs=1)
        self.assertEqual(result["hub_files"], [{"path": "crates/a/src/hub.rs", "commits": 2},
                                               {"path": "crates/b/src/other.rs", "commits": 2}])
        self.assertEqual(result["commits_touching_a_hub"], 2)
        self.assertEqual(result["co_changes"],
                         [{"files": ["crates/a/src/hub.rs", "crates/b/src/other.rs"], "commits": 2}])
        self.assertEqual(result["commits_touching_four_or_more_crates"], 1)

    def test_ci_summary_reports_the_median_of_successful_runs(self):
        runs = [{"conclusion": "success", "seconds": 1800}, {"conclusion": "success", "seconds": 3600},
                {"conclusion": "failure", "seconds": 600}, {"conclusion": "cancelled", "seconds": 60}]
        self.assertEqual(scorecard.ci_summary(runs),
                         {"runs": 4, "succeeded": 2, "failed": 1, "median_success_minutes": 45.0})


if __name__ == "__main__":
    unittest.main()
