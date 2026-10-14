"""Native renderer validation must test the pinned revision and fail on empty suites."""
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
                self.assertTrue(any("--lib" in argv for _, argv in tests))
        # WGPU DX12 is Windows' default renderer, so its whole suite runs there.
        wgpu = ["-p", "gpui_ce_wgpu", "--features", "test-support", "--lib"]
        for host in ("Windows", "Linux"):
            self.assertIn(wgpu, [argv for _, argv in validation.native_tests(host)])
        # Match the application's Windows bridge so its WGPU integration tests execute.
        windows = ["-p", "gpui_ce_windows", "--features", "windows-wgpu", "--lib"]
        self.assertIn(windows, [argv for _, argv in validation.native_tests("Windows")])
        with self.assertRaisesRegex(ValueError, "unsupported"):
            validation.native_tests("Unknown")

    def test_zero_executed_tests_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "zero"):
            validation.require_executed_tests("test result: ok. 0 passed; 0 failed; 3 filtered out")
        self.assertEqual(validation.require_executed_tests("test result: ok. 2 passed; 0 failed"), 2)

    def test_mismatched_dependency_pins_fail_before_testing_another_renderer(self):
        url = "https://example.invalid/gpui.git"
        pinned = f'source = "git+{url}?rev=same#same"'
        pins = "\n".join(f'{name} = {{ git = "{url}", rev = "same" }}'
                         for name in ("gpui", "gpui_platform", "gpui_wgpu", "gpui_parley"))
        packages = {"gpui-ce": pinned, "gpui_ce_render": pinned, "xim-parser": pinned,
                    "serde": 'source = "registry+https://github.com/rust-lang/crates.io-index"'}

        def check(manifest=f"[workspace.dependencies]\n{pins}\n", **overrides):
            with tempfile.TemporaryDirectory() as tmp:
                lock = "version = 4\n" + "".join(
                    f'\n[[package]]\nname = "{name}"\nversion = "0.1.0"\n{source}\n'
                    for name, source in {**packages, **overrides}.items())
                (Path(tmp) / "Cargo.toml").write_text(manifest, encoding="utf-8")
                (Path(tmp) / "Cargo.lock").write_text(lock, encoding="utf-8")
                return validation.pinned_renderer(Path(tmp) / "Cargo.toml", Path(tmp) / "Cargo.lock")

        self.assertEqual(check(), (url, "same"))
        mismatched = pins.replace('rev = "same"', 'rev = "other"', 1)
        with self.assertRaisesRegex(ValueError, "same pinned revision"):
            check(f"[workspace.dependencies]\n{mismatched}\n")
        with self.assertRaisesRegex(ValueError, "must not be patched"):
            check(f'[workspace.dependencies]\n{pins}\n[patch."{url}"]\ngpui_ce_render = {{ path = "vendor" }}\n')
        with self.assertRaisesRegex(ValueError, r"outside same: \['gpui_ce_render'\]"):
            check(gpui_ce_render="")
        with self.assertRaisesRegex(ValueError, r"outside same: \['xim-parser'\]"):
            check(**{"xim-parser": f'source = "git+{url}?rev=old#old"'})


if __name__ == "__main__":
    unittest.main()
