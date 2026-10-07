"""Cross-platform validation plans must retain instrumentation and pin boundaries."""
import argparse
from pathlib import Path
import tempfile
import unittest

import perf_workloads

validation = perf_workloads.module("gpu_native_validation_test", "validate-gpu.py")


class ValidationTests(unittest.TestCase):
    def test_native_tests_choose_the_host_renderer(self):
        for host, package in (("Darwin", "gpui_ce_apple"), ("Windows", "gpui_ce_windows"), ("Linux", "gpui_ce_wgpu")):
            with self.subTest(host=host):
                tests = validation.native_tests(host)
                self.assertTrue(any(package in argv for _, argv in tests))
                self.assertTrue(any("shared_atlas" in argv for _, argv in tests))
        with self.assertRaisesRegex(ValueError, "unsupported"):
            validation.native_tests("Unknown")

    def test_clean_commands_exclude_instrumentation_and_lifecycle_exercises_both_sides(self):
        args = argparse.Namespace(repositories=[Path("repository with spaces"), Path("history")],
                                  pairs=5, inputs=120, display="desktop", variants=["batch"],
                                  windows=[1, 4], sizes=["1400x900"], scales=[100],
                                  gpu=True, cycles=100, output=Path("output with spaces"))
        commands = dict(validation.measurement_commands(args, Path("frozen/gitcomet.exe")))
        self.assertIn("--gpu", commands["gpu-matrix"])
        clean = commands["clean-input"]
        self.assertNotIn("--gpu", clean)
        self.assertEqual(clean[clean.index("--timing-mode") + 1], "clean")
        self.assertIn("repository with spaces", clean)
        baseline, candidate = commands["lifecycle-baseline"], commands["lifecycle-candidate"]
        self.assertIn("GPUI_GPU_EXPERIMENTS=", baseline)
        self.assertTrue(any("cached-layers" in arg and "pooled-targets" in arg for arg in candidate))
        self.assertIn(str(Path("frozen/gitcomet.exe")), candidate)
        self.assertEqual(candidate[candidate.index("--cycles") + 1], "100")
        args.cycles = 0
        self.assertFalse(any(name.startswith("lifecycle") for name, _ in validation.measurement_commands(args, Path("binary"))))

    def test_mismatched_dependency_pins_fail_before_testing_another_renderer(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "Cargo.toml"
            lines = ['[workspace.dependencies]']
            for name in ("gpui", "gpui_platform", "gpui_wgpu"):
                lines.append(f'{name} = {{ git = "https://example.invalid/gpui.git", rev = "same" }}')
            path.write_text("\n".join(lines), encoding="utf-8")
            self.assertEqual(validation.pinned_renderer(path)[1], "same")
            path.write_text(path.read_text().replace('rev = "same"', 'rev = "different"', 1), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "same pinned revision"):
                validation.pinned_renderer(path)


if __name__ == "__main__":
    unittest.main()
