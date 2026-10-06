#!/usr/bin/env python3
"""Build and validate the pinned renderer on the current OS; Python 3.11+, Git, Rust.

Run on an otherwise idle, unlocked desktop. Each capture has an isolated app
profile. GPU timestamp and clean input captures are kept separate. This script
never changes shipping defaults. --list prints measurement commands only.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import tomllib

import perf_metadata
import perf_workloads

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
matrix = perf_workloads.module("native_validation_matrix", "gpu-matrix.py")


def pinned_renderer(manifest):
    dependencies = tomllib.loads(manifest.read_text(encoding="utf-8"))["workspace"]["dependencies"]
    pins = {(dependencies[name]["git"], dependencies[name]["rev"])
            for name in ("gpui", "gpui_platform", "gpui_wgpu")}
    if len(pins) != 1:
        raise ValueError("all GPUI dependencies must use the same pinned revision")
    return pins.pop()


def measurement_commands(args, binary):
    common = [sys.executable, str(HERE / "gpu-matrix.py"), "--binary", str(binary),
              "--repositories", *map(str, args.repositories), "--pairs", str(args.pairs),
              "--inputs", str(args.inputs), "--idle-ms", "5000", "--display", args.display]
    cases = ["--variants", *args.variants, "--windows", *map(str, args.windows),
             "--sizes", *args.sizes, "--scales", *map(str, args.scales)]
    gpu = ["--gpu"] if args.gpu else []
    commands = [
        ("gpu-matrix", common + cases + gpu + ["--output", str(args.output / "gpu-matrix")]),
        ("clean-input", common + cases + ["--timing-mode", "clean", "--output", str(args.output / "clean-input")]),
        ("graph-hidden", common + ["--variants", "paths", "--windows", "1", "--sizes", args.sizes[0],
                                   "--scales", str(args.scales[0]), "--graph-hidden", *gpu,
                                   "--output", str(args.output / "graph-hidden")]),
    ]
    if args.cycles:
        # Separate the lifetime soak from each timing case. Exercise every optional
        # resource together, once with flags off and once with flags on.
        for side, name in ((0, "baseline"), (1, "candidate")):
            command = [sys.executable, str(HERE / "multi-window.py"), "--binary", str(binary),
                       "--repositories", str(args.repositories[0]), str(args.repositories[-1]),
                       "--mode", "lifecycle", "--samples", "1", "--cycles", str(args.cycles),
                       "--idle-seconds", "5", "--timeout", "7200", "--display", args.display,
                       "--gpu-timings", *gpu, "--output", str(args.output / ("lifecycle-" + name))]
            for key, value in matrix.environment("combined", side).items():
                command += ["--env", f"{key}={value}"]
            commands.append(("lifecycle-" + name, command))
    return commands


def native_tests(system):
    commands = [("render-tests", ["-p", "gpui_ce_render", "--lib"])]
    if system in ("Darwin", "Windows"):
        package = "gpui_ce_apple" if system == "Darwin" else "gpui_ce_windows"
        for test in ("cropped_path_targets", "native_gpu_experiments", "shared_atlas"):
            commands.append((test, ["-p", package, "--lib", test]))
    elif system == "Linux":
        for test in ("path_target_tests", "path_cache::tests", "shared_atlas", "shared::tests", "texture_pool::tests"):
            commands.append((test.replace("::", "-"), ["-p", "gpui_ce_wgpu", "--features", "test-support", "--lib", test]))
    else:
        raise ValueError(f"unsupported native validation host: {system}")
    return commands


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repositories", type=Path, nargs="+", required=True,
                        help="existing test repositories (for example Git, Bun, history-100000)")
    parser.add_argument("--output", type=Path, required=True, help="new directory for logs and captures")
    parser.add_argument("--variants", nargs="+", choices=matrix.VARIANTS,
                        default=["crop", "share", "batch", "cache-after-batch", "paths", "damage", "pool", "connectors"])
    parser.add_argument("--windows", type=int, nargs="+", choices=(1, 2, 4), default=[1, 2, 4])
    parser.add_argument("--sizes", nargs="+", default=["1400x900", "2560x1440"])
    parser.add_argument("--scales", type=int, nargs="+", default=[100, 200])
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--inputs", type=int, default=240)
    parser.add_argument("--cycles", type=int, default=100, help="close/reopen cycles per side; 0 skips the soak")
    parser.add_argument("--display", choices=("headless", "desktop"),
                        default="headless" if platform.system() == "Linux" else "desktop")
    parser.add_argument("--gpu", action="store_true", help="optional vendor sampling; currently Linux/NVIDIA only")
    parser.add_argument("--stage", choices=("all", "tests", "measure"), default="all")
    parser.add_argument("--binary", type=Path, help="reuse a release binary instead of building (hash recorded)")
    parser.add_argument("--list", action="store_true", help="print commands without cloning, building or opening windows")
    args = parser.parse_args()
    if min(args.pairs, args.inputs, *args.scales) <= 0 or args.cycles < 0:
        parser.error("pairs, inputs and scales must be positive; cycles must be nonnegative")
    for dimensions in args.sizes:
        try:
            width, height = map(int, dimensions.split("x"))
            assert min(width, height) > 0
        except (ValueError, AssertionError):
            parser.error("sizes must be positive WIDTHxHEIGHT")
    if platform.system() != "Linux" and args.display == "headless":
        parser.error("Windows and macOS validation requires --display desktop")
    args.output = args.output.resolve()
    args.repositories = [repo.resolve() for repo in args.repositories]
    binary = args.output / ("gitcomet.exe" if platform.system() == "Windows" else "gitcomet")
    url, revision = pinned_renderer(ROOT / "Cargo.toml")
    tests = native_tests(platform.system())
    if args.list:
        print(json.dumps({"gpui": {"url": url, "revision": revision}, "tests": tests,
                          "measurements": measurement_commands(args, binary)}, indent=2))
        return 0
    for repo in args.repositories:
        subprocess.run(["git", "-C", str(repo), "rev-parse", "--git-dir"], check=True, stdout=subprocess.DEVNULL)
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {"schema": 1, "status": "running", "platform": platform.system(),
                "gpui": {"url": url, "revision": revision}, "commands": [],
                "source": perf_metadata.source_info(), "options": vars(args), "gates": {},
                "shipping_defaults": "disabled"}

    def save():
        (args.output / "validation.json").write_text(json.dumps(manifest, indent=2, default=str) + "\n", encoding="utf-8")

    # Do not inherit experimental renderer flags or cross-compilation shortcuts.
    env = dict(os.environ)
    for key in ("GPUI_GPU_TIMINGS", "GPUI_GPU_EXPERIMENTS", "GITCOMET_GPU_GRAPH_QUADS",
                "GPUI_RENDER_ALLOW_MISSING_DXBC"):
        env.pop(key, None)

    def run(name, command, extra_env=None):
        print(f"{name}: starting (log: {args.output / (name + '.log')})", flush=True)
        record = {"name": name, "argv": command, "started_unix": time.time()}
        manifest["commands"].append(record)
        save()
        with (args.output / (name + ".log")).open("w", encoding="utf-8") as log:
            result = subprocess.run(command, cwd=ROOT, env=dict(env, **(extra_env or {})), stdout=log, stderr=subprocess.STDOUT)
        record.update(returncode=result.returncode, seconds=time.time() - record["started_unix"])
        save()
        if result.returncode:
            raise RuntimeError(f"{name} failed; see {args.output / (name + '.log')}")

    try:
        rustc = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True)
        host = next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
        manifest["rustc"] = rustc
        if args.stage != "measure":
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
                           *arguments, "--", "--test-threads=1"], {"GPUI_GPU_TIMINGS": "1"})
            shutil.copy2(source / "Cargo.lock", args.output / "renderer-Cargo.lock")
        if args.stage != "tests":
            if args.binary:
                built = args.binary.resolve(strict=True)
            else:
                target = ROOT / "target" / "gpu-validation-app"
                run("build-app", ["cargo", "build", "--locked", "--release", "-p", "gitcomet", "--bin", "gitcomet",
                                  "--target", host, "--target-dir", str(target)])
                built = target / host / "release" / binary.name
            shutil.copy2(built, binary)
            manifest["binary_sha256"] = perf_metadata.sha256_file(binary)
            for name, command in measurement_commands(args, binary):
                run(name, command)
                matrix_file = args.output / name / "matrix.json"
                if matrix_file.exists():
                    data = json.loads(matrix_file.read_text(encoding="utf-8"))
                    if not data["complete"]:
                        raise ValueError(f"incomplete matrix: {matrix_file}")
                    manifest["gates"][name] = data["gates"]
        rejected = [f"{group}/{case}" for group, gates in manifest["gates"].items()
                    for case, gate in gates.items() if gate["status"] == "needs_review"]
        manifest.update(status="complete", rejected_cases=rejected)
        print(f"Complete. {len(rejected)} performance cases need review; no defaults changed.\nResults: {args.output / 'validation.json'}")
    except (Exception, KeyboardInterrupt) as error:
        manifest.update(status="failed", error=str(error))
        raise
    finally:
        save()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
