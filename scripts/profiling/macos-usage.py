#!/usr/bin/env python3
"""CPU and GPU load of the native macOS application across the interaction set.

  macos-usage.py run --binary GitComet.app/Contents/MacOS/gitcomet --label NAME \
      --output DIR [--repeats 3] [--only CASE...] [--env KEY=VALUE...]
  macos-usage.py report DIR [DIR...]

Each case is one `live-ui.py run --display desktop --gpu` scenario on a fixed
repository. Repeats are interleaved (every case once, then again) so slow drift
in machine state spreads over all cases. Before every run the console session
must be unlocked: a locked screen draws nothing and still "passes".

Repositories (prepare once; see scripts/profiling/README.md):
  --synthetic  live-ui.py fixture DIR             (20k commits, one 100k-line diff)
  --real       live-ui.py clone <GitComet> DIR    (with unstaged edits for diff-open)
  --large      live-ui.py clone <large repo> DIR  (repo-tabs and hover on real history)

The report prints per-phase medians: frames/s, CPU draw time, process CPU ms/s,
the process's GPU ms/s (IOKit AGX accounting), GPU ms per frame, WindowServer
GPU ms/s, package-idle wakeups/s and the kernel's CPU energy estimate.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import perf_macos  # noqa: E402

PYTHON = sys.executable


def cases(args):
    synthetic, real, large = args.synthetic, args.real, args.large
    return {
        "idle": dict(scenario="idle", repository=synthetic),
        "hover": dict(scenario="history-hover", repository=synthetic),
        "hover-large": dict(scenario="history-hover", repository=large),
        "select": dict(scenario="history-select", repository=synthetic),
        "scroll": dict(scenario="history-scroll", repository=synthetic),
        "diff-open": dict(scenario="diff-open", repository=real),
        "repo-tabs": dict(scenario="repo-tabs", repository=synthetic, secondary=[real, large]),
    }


def wait_unlocked():
    warned = False
    while perf_macos.screen_locked():
        if not warned:
            print("screen is locked; waiting (GUI captures draw nothing while locked)", flush=True)
            warned = True
        time.sleep(10)


def run(args):
    selected = cases(args)
    names = args.only or list(selected)
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = {"label": args.label, "binary": str(args.binary), "env": args.env, "runs": []}
    for repeat in range(args.repeats):
        for name in names:
            case = selected[name]
            out = args.output / f"{name}-{repeat + 1}"
            if (out / "summary.json").exists():
                continue
            wait_unlocked()
            command = [PYTHON, str(HERE / "live-ui.py"), "run", "--binary", str(args.binary),
                       "--repository", str(case["repository"]), "--scenario", case["scenario"],
                       "--display", "desktop", "--gpu", "--output", str(out), "--timeout", "900"]
            for path in case.get("secondary", []):
                command += ["--secondary-repository", str(path)]
            for item in args.env:
                command += ["--env", item]
            started = time.time()
            result = subprocess.run(command, capture_output=True, text=True)
            valid = False
            if (out / "summary.json").exists():
                valid = json.loads((out / "summary.json").read_text()).get("valid", False)
            print(f"{args.label} {name} #{repeat + 1}: {'valid' if valid else 'INVALID'} "
                  f"({time.time() - started:.0f} s)", flush=True)
            if result.returncode != 0 and not valid:
                (args.output / f"{name}-{repeat + 1}.error.log").write_text(result.stdout + result.stderr)
            manifest["runs"].append({"case": name, "repeat": repeat + 1, "output": str(out), "valid": valid})
            (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def report(args):
    runs = sorted(path for directory in args.directories for path in directory.iterdir()
                  if (path / "summary.json").exists())
    subprocess.run([PYTHON, str(HERE / "macos-usage-report.py"), *map(str, runs),
                    *(["--json", str(args.json)] if args.json else [])], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    runner = commands.add_parser("run")
    runner.add_argument("--binary", type=Path, required=True)
    runner.add_argument("--label", required=True)
    runner.add_argument("--output", type=Path, required=True)
    runner.add_argument("--repeats", type=int, default=3)
    runner.add_argument("--only", nargs="+", choices=("idle", "hover", "hover-large", "select", "scroll",
                                                      "diff-open", "repo-tabs"))
    runner.add_argument("--env", action="append", default=[], metavar="KEY=VALUE")
    profiling = ROOT / "target/profiling"
    runner.add_argument("--synthetic", type=Path, default=profiling / "live-fixture")
    runner.add_argument("--real", type=Path, default=profiling / "gitcomet-clone")
    runner.add_argument("--large", type=Path, default=profiling / "zed-clone")
    reporter = commands.add_parser("report")
    reporter.add_argument("directories", nargs="+", type=Path)
    reporter.add_argument("--json", type=Path)
    args = parser.parse_args()
    run(args) if args.command == "run" else report(args)


if __name__ == "__main__":
    main()
