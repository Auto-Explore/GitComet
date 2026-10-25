import contextlib
import importlib.util
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("bench-ref-comparisons.py")
SPEC = importlib.util.spec_from_file_location("comparison_bench", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)


class ComparisonBenchTests(unittest.TestCase):
    def test_init_templates_cannot_install_external_hooks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            templates = root / "templates"
            hooks = templates / "hooks"
            hooks.mkdir(parents=True)
            marker = root / "hook-ran"
            hook = hooks / "reference-transaction"
            hook.write_text(
                f"#!{sys.executable}\nfrom pathlib import Path\nPath({str(marker)!r}).touch()\n",
                encoding="utf-8",
            )
            hook.chmod(0o700)
            config = root / "gitconfig"
            config.write_text(f"[init]\n\ttemplateDir = {templates}\n", encoding="utf-8")
            owned = root / "owned"
            owned.mkdir()
            with patch.dict(os.environ, {
                "GIT_TEMPLATE_DIR": str(templates), "GIT_CONFIG_GLOBAL": str(config),
            }):
                repo, _, target = bench._history(owned, 3, 2)
            self.assertEqual(bench._git(repo, "rev-parse", "history").decode().strip(), target)
            self.assertFalse((repo / ".git" / "hooks" / "reference-transaction").exists())
            self.assertFalse(marker.exists(), "fixture writes executed an inherited hook")

    def test_git_overrides_cannot_redirect_fixture_writes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            victim = root / "unrelated"
            victim.mkdir()
            bench._git(victim, "init", "-q")
            config = (victim / ".git" / "config").read_bytes()
            objects = sorted((victim / ".git" / "objects").rglob("*"))
            owned = root / "owned"
            owned.mkdir()
            with patch.dict(os.environ, {
                "GIT_DIR": str(victim / ".git"),
                "GIT_WORK_TREE": str(victim),
                "GIT_OBJECT_DIRECTORY": str(victim / ".git" / "objects"),
                "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "gc.auto",
                "GIT_CONFIG_VALUE_0": "123",
            }):
                repo, tips, target = bench._history(owned, 3, 2)
                self.assertTrue((repo / ".git").is_dir())
                self.assertEqual(len(tips), 1)
                self.assertEqual(bench._git(repo, "rev-parse", "history").decode().strip(), target)
            self.assertEqual((victim / ".git" / "config").read_bytes(), config)
            self.assertEqual(bench._git(victim, "for-each-ref"), b"")
            self.assertEqual(sorted((victim / ".git" / "objects").rglob("*")), objects)

    @unittest.skipUnless(bench.NATIVE_PROBES_SUPPORTED, "Linux process handles")
    def test_interrupt_after_cold_measurement_stops_later_children(self) -> None:
        self.assert_interrupted_children_stop(leader_exits=False)

    @unittest.skipUnless(bench.NATIVE_PROBES_SUPPORTED, "Linux process handles")
    def test_interrupt_after_leader_exit_stops_children_in_other_groups(self) -> None:
        self.assert_interrupted_children_stop(leader_exits=True)

    def assert_interrupted_children_stop(self, *, leader_exits: bool) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake = root / "probe"
            fake.write_text(
                f"#!{sys.executable}\n"
                "import json, os, signal, subprocess, sys\n"
                "print(json.dumps({'event':'cold_complete'}), flush=True)\n"
                "read_fd, write_fd = os.pipe()\n"
                "child = subprocess.Popen([sys.executable, '-c', "
                "\"import json, os, signal, sys; os.setpgrp(); signal.signal(signal.SIGINT, signal.SIG_IGN); os.read(int(sys.argv[1]),1); print(json.dumps({'event':'child_ready','pid':os.getpid()}), flush=True); signal.pause()\", str(read_fd)], pass_fds=(read_fd,))\n"
                "os.close(read_fd)\n"
                + ("os._exit(9)\n" if leader_exits else "os.close(write_fd)\n") +
                "child.wait()\n",
                encoding="utf-8",
            )
            fake.chmod(0o700)
            driver = (
                "import importlib.util, pathlib, sys\n"
                f"spec = importlib.util.spec_from_file_location('bench', {str(SCRIPT)!r})\n"
                "bench = importlib.util.module_from_spec(spec); spec.loader.exec_module(bench)\n"
                "try:\n"
                f"    bench._probe(pathlib.Path({str(fake)!r}), pathlib.Path({str(root)!r}), pathlib.Path({str(root)!r}), 'case')\n"
                "except KeyboardInterrupt:\n"
                "    sys.exit(130)\n"
            )
            with subprocess.Popen(
                [sys.executable, "-c", driver], stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True, start_new_session=True,
            ) as process:
                assert process.stdout is not None
                child = None
                try:
                    while True:
                        line = process.stdout.readline()
                        self.assertTrue(line, "probe ended before its child handshake")
                        event = json.loads(line.split(": ", 1)[1])
                        if event["event"] == "child_ready":
                            child = os.pidfd_open(event["pid"])
                            break
                    os.killpg(process.pid, signal.SIGINT)
                    self.assertEqual(process.wait(timeout=10), 130)
                    self.assertTrue(select.select([child], [], [], 5)[0], "post-cold child survived")
                finally:
                    if child is not None:
                        with contextlib.suppress(ProcessLookupError):
                            signal.pidfd_send_signal(child, signal.SIGKILL)
                        os.close(child)
                    if process.poll() is None:
                        process.kill()
                    process.communicate(timeout=10)


if __name__ == "__main__":
    unittest.main()
