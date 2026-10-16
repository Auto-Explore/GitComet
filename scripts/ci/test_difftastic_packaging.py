from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "check_difftastic", Path(__file__).resolve().parents[1] / "check-difftastic.py"
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
build_spec = importlib.util.spec_from_file_location(
    "build_difftastic", Path(__file__).resolve().parents[1] / "build-difftastic.py"
)
builder = importlib.util.module_from_spec(build_spec)
build_spec.loader.exec_module(builder)


class DifftasticPackagingTests(unittest.TestCase):
    def test_missing_runtime_notice_fails_and_pinned_parser_notice_is_copied(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = {"name": "unknown", "version": "1", "manifest_path": str(root / "source/Cargo.toml")}
            with self.assertRaisesRegex(RuntimeError, "Missing license notice"):
                builder.collect_dependency_notices(package, root / "notices")
            self.assertFalse((root / "notices").exists())
            package.update(name="tree-sitter-lua", version="0.5.0")
            builder.collect_dependency_notices(package, root / "notices")
            self.assertIn("2021 Munif Tanjim", (root / "notices/tree-sitter-lua-0.5.0/LICENSE").read_text())

    def test_collects_nested_native_library_notices_without_filename_collisions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            (source / "jemalloc").mkdir(parents=True)
            (source / "LICENSE").write_text("Rust wrapper", encoding="utf-8")
            (source / "jemalloc/COPYING").write_text("Native allocator", encoding="utf-8")
            (source / "jemalloc/LICENSE").write_text("Nested license", encoding="utf-8")
            builder.collect_notices(source, root / "notices")
            self.assertEqual((root / "notices/LICENSE").read_text(), "Rust wrapper")
            self.assertEqual((root / "notices/jemalloc/COPYING").read_text(), "Native allocator")
            self.assertEqual((root / "notices/jemalloc/LICENSE").read_text(), "Nested license")

    def check_output(self, version, comparison):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            notices = root / "notices.txt"
            notices.write_text("Difftastic license notices", encoding="utf-8")
            with patch.object(checker.subprocess, "check_output", side_effect=[version, json.dumps(comparison).encode()]), redirect_stdout(io.StringIO()):
                checker.check(root / "difft", notices)

    def test_accepts_pinned_helper_with_both_structural_tokens(self):
        self.check_output(b"Difftastic 0.71.0\n", {
            "status": "changed", "language": "JavaScript", "aligned_lines": [[0, 0]],
            "chunks": [[{"lhs": {"changes": [{"content": "1"}]},
                         "rhs": {"changes": [{"content": "2"}]}}]],
        })

    def test_rejects_missing_notices_before_executing_helper(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(checker.subprocess, "check_output") as execute:
            with self.assertRaisesRegex(RuntimeError, "missing Difftastic notices"):
                checker.check(Path(directory) / "difft", Path(directory) / "missing")
            execute.assert_not_called()

    def test_rejects_wrong_version(self):
        with self.assertRaisesRegex(RuntimeError, "version does not match"):
            self.check_output(b"Difftastic 0.70.0\n", {})

    def test_rejects_line_fallback_and_malformed_token_changes(self):
        for comparison in [
            {"status": "changed", "language": "Text"},
            {"status": "changed", "language": "JavaScript", "aligned_lines": [[0, 0]],
             "chunks": [[{"lhs": {"changes": None}}]]},
            {"status": "changed", "language": "JavaScript", "aligned_lines": [[0, 0]],
             "chunks": [[{"lhs": {"changes": [{"content": "1"}]}}]]},
        ]:
            with self.subTest(comparison=comparison), self.assertRaises(RuntimeError):
                self.check_output(b"Difftastic 0.71.0\n", comparison)


if __name__ == "__main__":
    unittest.main()
