"""The refactor ratchets: classification, each metric, and the baseline comparison."""

from pathlib import Path
import tempfile
import unittest

import ratchets

UI = ratchets.UI
KIT = ratchets.KIT
STATE = ratchets.STATE

ROOT_VIEW = (
    "pub struct GitCometView {\n"
    "    pub(super) store: Arc<AppStore>,\n"
    "    /// A comment line.\n"
    "    pub(super) theme: AppTheme,\n"
    "    #[cfg(test)]\n"
    "    pub(super) probe: u64,\n"
    "    window_gated: bool,\n"
    "}\n"
    "\n"
    "pub struct Other {\n"
    "    pub field: u8,\n"
    "}\n"
)

TEXTS = {
    UI + "view/mod_helpers/mod.rs": ROOT_VIEW,
    UI + "view/gitcomet_view.rs": "use super::*;\nimpl GitCometView {}\n",
    UI + "view/gitcomet_view_render.rs": "impl GitCometView {}\n",
    UI + "view/home.rs": "use super::*;\nimpl GitCometView {\n    fn a(&self) { self.x.set_theme(t); self.y.set_theme(t); }\n}\n",
    UI + "view/tests/home.rs": "use super::*;\nimpl GitCometView {}\n" + "x\n" * 3100,
    UI + "view/rows/benchmarks/syntax.rs": "use super::*;\n" + "x\n" * 10,
    UI + "view/panes/sidebar.rs": "x\n" * 3001,
    KIT + "theme.rs": "use gitcomet_core::identity;\nfn f() { gitcomet_core::identity::current(); }\n",
    KIT + "theme/tests.rs": "use gitcomet_core::identity;\n",
    STATE + "store/reducer.rs": "state.repos.iter_mut().find(|r| r.id == id);\nrepos.iter_mut()\n    .find(|r| true);\n",
    STATE + "store/tests/reducer.rs": "state.repos.iter_mut().find(|r| r.id == id);\n",
    "crates/gitcomet-git-gix/tests/a_integration.rs": "\n",
    "crates/gitcomet-git-gix/tests/b_integration.rs": "\n",
    "crates/gitcomet-git-gix/tests/support/helper.rs": "\n",
    "crates/gitcomet-git-gix/tests/merged/main.rs": "\n",
    "crates/gitcomet-git-gix/tests/merged/module.rs": "\n",
    "crates/gitcomet-git-gix/src/lib.rs": "\n",
    ratchets.CI_RUNNER: 'UI = "gitcomet-ui-gpui"\nGPUI_PACKAGES = (UI, "gitcomet-ui-kit",\n    "gitcomet-extension-example")\n',
}


class Classification(unittest.TestCase):
    def test_test_paths_follow_the_size_guard(self):
        for path in ("a/tests/b.rs", "a/benches/b.rs", "a/benchmarks/b.rs", "a/tests.rs", "a/b_tests.rs"):
            self.assertTrue(ratchets.is_test_path(path), path)
        for path in ("a/test_support.rs", "a/testing.rs", "a/benchmarks.rs"):
            self.assertFalse(ratchets.is_test_path(path), path)

    def test_production_excludes_benchmarks_and_smoke_tests(self):
        self.assertFalse(ratchets.is_production("a/benchmarks/b.rs"))
        self.assertFalse(ratchets.is_production("a/smoke_tests/b.rs"))
        self.assertTrue(ratchets.is_production("a/b.rs"))


class Metrics(unittest.TestCase):
    def setUp(self):
        self.found = ratchets.measure(TEXTS)

    def test_root_view_fields_count_declarations_only(self):
        self.assertEqual(self.found["gitcomet_view_fields"], 4)

    def test_ui_production_lines_skip_tests_and_benchmarks(self):
        expected = sum(ratchets.lines(TEXTS[p]) for p in (
            UI + "view/mod_helpers/mod.rs", UI + "view/gitcomet_view.rs", UI + "view/gitcomet_view_render.rs",
            UI + "view/home.rs", UI + "view/panes/sidebar.rs"))
        self.assertEqual(self.found["ui_gpui_production_lines"], expected)

    def test_glob_imports_count_production_files(self):
        self.assertEqual(self.found["super_glob_imports_in_ui_production"], 2)

    def test_impl_blocks_outside_the_root_files(self):
        self.assertEqual(self.found["impl_gitcomet_view_files_outside_root"], 2)

    def test_set_theme_calls(self):
        self.assertEqual(self.found["set_theme_calls_in_ui"], 2)

    def test_kit_core_refs_count_production_occurrences(self):
        self.assertEqual(self.found["kit_gitcomet_core_refs"], 2)

    def test_oversized_production_files(self):
        self.assertEqual(self.found["production_files_over_3000_lines"], 1)

    def test_repo_iter_mut_find_sites_allow_line_breaks(self):
        self.assertEqual(self.found["state_repo_iter_mut_find_sites"], 2)

    def test_gix_integration_binaries_are_top_level_files_and_main_dirs(self):
        self.assertEqual(self.found["gix_integration_test_binaries"], 3)

    def test_gpui_test_packages_parse_the_runner(self):
        self.assertEqual(self.found["gpui_test_packages"], 3)


class Baseline(unittest.TestCase):
    def setUp(self):
        self.found = ratchets.measure(TEXTS)
        self.directory = tempfile.TemporaryDirectory()
        self.path = Path(self.directory.name) / "ratchets.txt"
        ratchets.write(self.found, self.path)

    def tearDown(self):
        self.directory.cleanup()

    def test_recorded_values_round_trip(self):
        self.assertEqual(ratchets.compare(self.found, ratchets.read_baseline(self.path)), [])
        self.assertTrue(self.path.read_text().startswith("#"))

    def test_regression_and_improvement_are_both_reported(self):
        baseline = ratchets.read_baseline(self.path)
        worse = dict(self.found, set_theme_calls_in_ui=3, gpui_test_packages=2)
        problems = ratchets.compare(worse, baseline)
        self.assertEqual(len(problems), 2)
        self.assertTrue(all("regressed" in problem for problem in problems))
        better = dict(self.found, set_theme_calls_in_ui=1, gpui_test_packages=4)
        problems = ratchets.compare(better, baseline)
        self.assertEqual(len(problems), 2)
        self.assertTrue(all("--write" in problem for problem in problems))

    def test_missing_and_stale_entries_are_reported(self):
        baseline = ratchets.read_baseline(self.path)
        del baseline["kit_gitcomet_core_refs"]
        baseline["old_metric"] = ("max", 1)
        problems = ratchets.compare(self.found, baseline)
        self.assertEqual(len(problems), 2)

    def test_malformed_lines_are_rejected(self):
        self.path.write_text("set_theme_calls_in_ui sideways 1\n")
        with self.assertRaises(ValueError):
            ratchets.read_baseline(self.path)

    def test_write_rewrites_values_and_keeps_comments(self):
        self.path.write_text("# kept\n# also kept\nset_theme_calls_in_ui max 99\n")
        ratchets.write(self.found, self.path)
        text = self.path.read_text()
        self.assertIn("# also kept\n", text)
        self.assertIn("set_theme_calls_in_ui max 2\n", text)


if __name__ == "__main__":
    unittest.main()
