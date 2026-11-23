import os
import re
import shutil
import subprocess
import sys
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

            # A checkout and its LLVM source paths can use different spellings of the same root.
            (root / "subdir").mkdir()
            lines = coverage.product_lines(root / "subdir" / "..", [gui, headless], [unit, integration])
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
                    filename = f"crates/{command[-1]}/src/main.rs"
                    pattern, = (arg.split("=", 1)[1] for arg in command[2:]
                                if str(arg).startswith("-include-filename-regex="))
                    if not re.search(pattern, filename):
                        return
                    profile = next(arg.removeprefix("-instr-profile=") for arg in command[2:]
                                   if str(arg).startswith("-instr-profile="))
                    line = Path(profile).read_text()
                    kwargs["stdout"].write(f"SF:{filename}\nDA:{line},1\n".encode())

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

    @unittest.skipUnless(sys.platform.startswith("linux"), "the ELF build ID is used on Linux")
    def test_linux_compiler_wrapper_keeps_cargo_binary_paths_and_requests_a_build_id(self):
        env = dict(os.environ, GITCOMET_COVERAGE_RUSTC_WRAPPER=sys.executable)
        env["CARGO_BIN_EXE_gitcomet-test-mergetool"] = "a path with spaces"
        compiler = ('import os, sys; print(os.environ["CARGO_BIN_EXE_gitcomet-test-mergetool"]); '
                    'print(*sys.argv[1:], sep="\\n")')
        output = subprocess.check_output([coverage.ROOT / "scripts/linux/coverage-rustc.sh",
                                          "-c", compiler, "main.rs"], env=env, text=True)
        self.assertEqual(output.splitlines(), ["a path with spaces", "main.rs", "-C", "link-arg=-Wl,--build-id"])

    def test_failed_run_keeps_its_own_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            diagnostics = target / "source-coverage-diagnostics"
            diagnostics.mkdir()
            (diagnostics / "runtime-exclusions.log").write_text("previous run\n")
            llvm = target / "bin"
            llvm.mkdir()
            (llvm / ("llvm-cov" + coverage.EXE)).touch()

            def run(command, **kwargs):
                if command[:2] == ["cargo", "build"]:
                    kwargs["stdout"].write(b'{"executable":"gitcomet"}\n')
                elif "compile" in command:
                    report_dir = Path(kwargs["env"]["GITCOMET_CI_REPORT_DIR"])
                    report_dir.mkdir(exist_ok=True)
                    (report_dir / "workspace-compile.log").write_text("compiler error\n")
                    raise subprocess.CalledProcessError(1, command)

            with (
                patch.object(coverage, "TARGET", target),
                patch.object(coverage.subprocess, "check_output", return_value=str(target / "lib")),
                patch.object(coverage.subprocess, "run", side_effect=run),
                self.assertRaises(subprocess.CalledProcessError),
            ):
                coverage.main()
            self.assertEqual((diagnostics / "workspace-compile.log").read_text(), "compiler error\n")
            self.assertFalse((diagnostics / "runtime-exclusions.log").exists())

    @unittest.skipUnless(shutil.which("perl"), "the coverage renderer requires Perl")
    def test_publishes_html_with_an_extensionless_perl_renderer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "checkout with spaces"
            root.mkdir()
            genhtml = root / "genhtml"
            genhtml.write_text('#!/usr/bin/perl\nuse File::Copy; mkdir $ARGV[-2] or die $!; '
                               'copy $ARGV[-1], "$ARGV[-2]/index.html" or die $!;\n')
            genhtml.chmod(0o755)
            target = root / "target"
            with (
                patch.dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"]),
                patch.object(coverage, "ROOT", root),
                patch.object(coverage, "TARGET", target),
                patch.object(coverage, "collect", return_value={"crates/probe/src/lib.rs": {1: True}}),
            ):
                coverage.main()
            self.assertIn("DA:1,1", (target / "llvm-cov/html/index.html").read_text())

    def test_publishing_preserves_other_reports_and_restores_the_last_success_on_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "llvm-cov"
            output.mkdir()
            (output / "baseline.info").write_text("saved baseline")
            for attempt in ("first", "incomplete"):
                staged = root / attempt / "report"
                staged.mkdir(parents=True)
                (staged / "html").mkdir()
                (staged / "html/index.html").write_text(attempt)
                (staged / "lcov.info").write_text(attempt)
                if attempt == "first":
                    (staged / "summary.md").write_text(attempt)
                    coverage.publish_reports(staged, output)
                else:
                    with self.assertRaises(FileNotFoundError):
                        coverage.publish_reports(staged, output)
            for name in ("html/index.html", "lcov.info", "summary.md"):
                self.assertEqual((output / name).read_text(), "first")
            self.assertEqual((output / "baseline.info").read_text(), "saved baseline")
