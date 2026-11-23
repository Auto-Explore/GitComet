import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import source_coverage as coverage


class ProductLineCoverageTests(unittest.TestCase):
    def test_product_variants_define_the_denominator_and_test_contexts_share_hits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = "crates/gitcomet/src/main.rs"
            unused = "crates/gitcomet-core/src/unused.rs"

            def trace(name, files):
                path = root / name
                with path.open("w", encoding="utf-8") as out:
                    for filename, counts in files.items():
                        out.write(f"SF:{root / filename}\n")
                        for number, hits in counts:
                            out.write(f"DA:{number},{hits}\n")
                        out.write("end_of_record\n")
                return path

            gui = trace("gui.info", {file: [(2, 0), (3, 0)], unused: [(7, 0)],
                                     "vendor/demo/src/lib.rs": [(2, 0)]})
            headless = trace("headless.info", {file: [(2, 0), (5, 0)]})
            unit = trace("unit.info", {file: [(2, 7), (3, 0), (90, 8)]})
            integration = trace("integration.info", {"crates/gitcomet/src/../src/main.rs": [(2, 0), (5, 4)],
                                                     "crates/gitcomet/tests/cli.rs": [(10, 2)]})

            lines = coverage.product_lines(root, [gui, headless], [unit, integration])
            coverage.write_reports(root, lines)
            self.assertEqual((root / "lcov.info").read_text().splitlines(), [
                f"SF:{unused}", "DA:7,0", "LF:1", "LH:0", "end_of_record",
                f"SF:{file}", "DA:2,1", "DA:3,0", "DA:5,1", "LF:3", "LH:2", "end_of_record",
            ])
            self.assertIn("| **Total** | **2** | **4** | **50.00%** |",
                          (root / "summary.md").read_text())

    def test_empty_product_mapping_fails_instead_of_reporting_a_percentage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            empty = root / "empty.info"
            empty.write_text("")
            gui = root / "gui.info"
            gui.write_text("SF:crates/gitcomet/src/main.rs\nDA:1,0\nend_of_record\n")
            with self.assertRaisesRegex(ValueError, "no measurable"):
                coverage.product_lines(root, [gui, empty], [])

    def test_linux_matches_each_execution_profile_to_its_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw = root / "workspace"
            raw.mkdir()
            (raw / "bb-pool_0.profraw").write_text("2")
            (raw / "aa-pool_0.profraw").write_text("1")
            binaries = {"unit": "aa", "cli": "bb", "unused": "cc"}

            def sections(command, **kwargs):
                return f"Name: __llvm_covmap\nBuild ID: {binaries[command[-1]]}\n"

            def llvm(command, **kwargs):
                if command[1] == "merge":
                    command[-1].write_text(command[-3].read_text())
                else:
                    profile = next(arg.removeprefix("-instr-profile=") for arg in command[2:]
                                   if str(arg).startswith("-instr-profile="))
                    line = Path(profile).read_text()
                    kwargs["stdout"].write(f"SF:crates/{command[-1]}/src/main.rs\nDA:{line},1\n".encode())

            with (
                patch.object(coverage.sys, "platform", "linux"),
                patch.object(coverage.subprocess, "check_output", side_effect=sections),
                patch.object(coverage.subprocess, "run", side_effect=llvm),
            ):
                traces = coverage.test_traces(root, raw, binaries)
                hits = [row for trace in traces for row in coverage.trace_lines(root, trace)]
                self.assertEqual(hits, [("crates/unit/src/main.rs", 1, 1), ("crates/cli/src/main.rs", 2, 1)])
                (raw / "ff-pool_0.profraw").touch()
                with self.assertRaisesRegex(ValueError, "outside its Cargo executable inventory"):
                    coverage.test_traces(root, raw, binaries)
