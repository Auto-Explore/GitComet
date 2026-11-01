#!/usr/bin/env python3
"""Run the pinned renderer's native tests on the current OS; Python 3.11+, Git, Rust.

The renderer is checked out at the exact revision Cargo.lock resolves, and each
test command must execute at least one passing test. --list prints the pin and
test commands only.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time
import tomllib

import perf_metadata

ROOT = Path(__file__).resolve().parents[2]


def pinned_renderer(manifest, lock):
    """The one GPUI pin, once the lock resolves every GPUI package from it."""
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    dependencies = data["workspace"]["dependencies"]
    pins = {(dependencies[name]["git"], dependencies[name]["rev"])
            for name in ("gpui", "gpui_platform", "gpui_wgpu", "gpui_parley")}
    if len(pins) != 1:
        raise ValueError("all GPUI dependencies must use the same pinned revision")
    url, revision = pins.pop()
    if any(url in source for source in data.get("patch", {})):
        raise ValueError("GPUI crates must not be patched")
    pinned = f"git+{url}?rev={revision}#{revision}"
    packages = [package for package in tomllib.loads(lock.read_text(encoding="utf-8")).get("package", [])
                if package["name"] in ("gpui-ce", "gpui_xim") or package["name"].startswith("gpui_ce_")
                or url in package.get("source", "")]
    unpinned = sorted(package["name"] for package in packages if package.get("source") != pinned)
    if not packages or unpinned:
        raise ValueError(f"Cargo.lock resolves GPUI outside {revision}: {unpinned or 'no packages'}")
    return url, revision


def native_tests(system):
    commands = [("render-tests", ["-p", "gpui_ce_render", "--lib"])]
    wgpu = ("wgpu-renderer-tests", ["-p", "gpui_ce_wgpu", "--features", "test-support", "--lib"])
    if system == "Windows":
        commands += [("windows-renderer-tests", ["-p", "gpui_ce_windows", "--features", "windows-wgpu", "--lib"]), wgpu]
    elif system == "Darwin":
        commands.append(("metal-renderer-tests", ["-p", "gpui_ce_apple", "--lib"]))
    elif system == "Linux":
        commands.append(wgpu)
    else:
        raise ValueError(f"unsupported native validation host: {system}")
    return commands


def require_executed_tests(log):
    passed = sum(int(count) for count in re.findall(r"test result: ok\. (\d+) passed", log))
    if passed <= 0:
        raise ValueError("native test command executed zero passing tests; stale filter or unavailable suite")
    return passed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new directory for logs")
    parser.add_argument("--list", action="store_true", help="print the pin and test commands without running them")
    args = parser.parse_args()
    args.output = args.output.resolve()
    url, revision = pinned_renderer(ROOT / "Cargo.toml", ROOT / "Cargo.lock")
    tests = native_tests(platform.system())
    if args.list:
        print(json.dumps({"gpui": {"url": url, "revision": revision}, "tests": tests}, indent=2))
        return 0
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {"schema": 2, "status": "running", "platform": platform.system(),
                "gpui": {"url": url, "revision": revision}, "commands": [],
                "source": perf_metadata.source_info()}

    def save():
        (args.output / "validation.json").write_text(json.dumps(manifest, indent=2, default=str) + "\n", encoding="utf-8")

    # Do not inherit cross-compilation shortcuts.
    env = dict(os.environ)
    env.pop("GPUI_RENDER_ALLOW_MISSING_DXBC", None)

    def run(name, command):
        print(f"{name}: starting (log: {args.output / (name + '.log')})", flush=True)
        record = {"name": name, "argv": command, "started_unix": time.time()}
        manifest["commands"].append(record)
        save()
        with (args.output / (name + ".log")).open("w", encoding="utf-8") as log:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
        record.update(returncode=result.returncode, seconds=time.time() - record["started_unix"])
        save()
        if result.returncode:
            raise RuntimeError(f"{name} failed; see {args.output / (name + '.log')}")

    try:
        rustc = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True)
        host = next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
        manifest["rustc"] = rustc
        source = ROOT / "target" / "gpu-validation-sources" / revision
        if not source.exists():
            source.parent.mkdir(parents=True, exist_ok=True)
            run("clone-renderer", ["git", "clone", "--no-checkout", "--filter=blob:none", url, str(source)])
            run("checkout-renderer", ["git", "-C", str(source), "checkout", "--detach", revision])
        actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
        dirty = subprocess.check_output(["git", "-C", str(source), "status", "--porcelain"], text=True).strip()
        if actual != revision or dirty:
            raise ValueError(f"renderer test checkout differs from the pin: {source}")
        for name, arguments in tests:
            run(name, ["cargo", "test", "--manifest-path", str(source / "Cargo.toml"),
                       "--target", host, "--target-dir", str(ROOT / "target" / "gpu-validation-renderer"),
                       *arguments, "--", "--test-threads=1"])
            count = require_executed_tests((args.output / (name + ".log")).read_text(encoding="utf-8"))
            manifest["commands"][-1]["passed_tests"] = count
        shutil.copy2(source / "Cargo.lock", args.output / "renderer-Cargo.lock")
        manifest.update(status="complete")
        print(f"Complete.\nResults: {args.output / 'validation.json'}")
    except (Exception, KeyboardInterrupt) as error:
        manifest.update(status="failed", error=str(error))
        raise
    finally:
        save()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
