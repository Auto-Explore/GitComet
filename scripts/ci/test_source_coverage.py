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

    @unittest.skipUnless(shutil.which("rustc"), "the coverage compiler requires Rust")
    def test_remapped_sources_keep_their_test_profiles_and_cargo_binary_paths(self):
        libdir = subprocess.check_output(["rustc", "--print", "target-libdir"], text=True).strip()
        llvm = Path(libdir).parent / "bin"
        if not (llvm / ("llvm-cov" + coverage.EXE)).is_file():
            self.skipTest("requires the Rust llvm-tools-preview component")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = Path("crates/probe/src/main.rs")
            (root / source).parent.mkdir(parents=True)
            (root / source).write_text('fn main() { assert_eq!(env!("CARGO_BIN_EXE_test-tool"), "a b"); }\n')
            compiler = coverage.build_compiler_wrapper(root)
            binary = root / ("probe" + coverage.EXE)
            env = dict(os.environ, GITCOMET_COVERAGE_RUSTC_WRAPPER=shutil.which("rustc"))
            env["CARGO_BIN_EXE_test-tool"] = "a b"
            linux = sys.platform.startswith("linux")
            for name, path, prefix in (("workspace", root / source, root), ("partial", source, source.parent)):
                with self.subTest(remap=name):
                    raw = root / name
                    raw.mkdir()
                    subprocess.run([compiler, path, "-C", "instrument-coverage", f"--remap-path-prefix={prefix}=/src",
                                    *(["-Clink-arg=-Wl,--build-id=none"] if linux else []), "-o", binary],
                                   cwd=root, env=env, check=True)
                    pattern = "%b-%4m.profraw" if linux else "%4m.profraw"
                    subprocess.run([binary], env=dict(env, LLVM_PROFILE_FILE=str(raw / pattern)), check=True)
                    trace, = coverage.test_traces(llvm, raw, [binary])
                    self.assertEqual(list(coverage.trace_lines(root, trace)), [(source.as_posix(), 1, 1)])

    @unittest.skipIf(os.name == "nt", "uses POSIX shell fixtures")
    def test_build_cache_ignores_runtime_profile_pool_and_keeps_instrumented_crates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scripts = root / "scripts"
            (scripts / "ci").mkdir(parents=True)
            shutil.copy(coverage.ROOT / "scripts/coverage.sh", scripts)
            shutil.copy(coverage.ROOT / "scripts/ci/coverage_rustc.rs", scripts / "ci")
            (scripts / "ci/source_coverage.py").write_text(
                'import os\nassert os.environ["CARGO_BUILD_BUILD_DIR"] == os.environ["CARGO_TARGET_DIR"]\n'
                'print(os.environ["CARGO_TARGET_DIR"])\n')
            cargo = root / "cargo"
            cargo.write_text('#!/bin/sh\nif [ "$2" = show-env ]; then\n'
                             '  echo "export LLVM_PROFILE_FILE=%${TEST_POOL}m.profraw"\n'
                             '  echo "export __CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES=$TEST_CRATES"\n'
                             '  echo "export RUSTC_WRAPPER=cargo-llvm-cov"\nfi\n')
            cargo.chmod(0o755)
            for tool in ("rustup", "perl", "genhtml"):
                (root / tool).symlink_to(cargo)

            def build_dir(pool, crates):
                env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                           TEST_POOL=pool, TEST_CRATES=crates)
                output = subprocess.check_output(["bash", scripts / "coverage.sh"], env=env, text=True)
                return Path(output.splitlines()[-1])

            first = build_dir("1", "probe")
            self.assertEqual(build_dir("8", "probe"), first)
            self.assertNotEqual(build_dir("1", "probe,dependency"), first)

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
    def test_publishes_html_with_homebrew_and_msys2_renderer_layouts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "checkout with spaces"
            root.mkdir()
            genhtml = root / "genhtml"
            program = genhtml
            if os.name != "nt":
                program = root / "genhtml.pl"
                genhtml.write_text('#!/bin/bash\nexport PERL5LIB="$PWD"\n'
                                   'exec perl "$(dirname "$0")/genhtml.pl" "$@"\n')
            program.write_text('#!/usr/bin/perl\nuse File::Copy; mkdir $ARGV[-2] or die $!; '
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
