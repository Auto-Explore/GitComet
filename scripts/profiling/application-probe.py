#!/usr/bin/env python3
"""Measure GitComet and raw Git on disposable fixtures, outside test acceptance."""

import argparse
import json
import os
import platform
from pathlib import Path
import subprocess
import sys

# Resolve shared CI helpers from this file so invocation is independent of cwd.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci"))
import run as runner


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles", nargs="+", choices=("ci-test", "release"), default=["ci-test", "release"])
    parser.add_argument("--samples", type=int, default=35)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--fixtures", nargs="+", choices=("plain", "lfs", "mixed", "submodule"),
                        default=["plain", "lfs", "submodule"])
    parser.add_argument("--status-state", choices=("warm", "stale"), default="warm")
    parser.add_argument("--git-executable", type=Path)
    parser.add_argument("--latency-only", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.samples <= 10_000:
        parser.error("--samples must be between 1 and 10000")
    instrumentation = [name for name in (
        "GITCOMET_CI_FIXTURE_TIMINGS", "GITCOMET_TEST_SYNC_TRACE", "GITCOMET_BENCH_STATUS_WORKERS",
        "GIT_TRACE", "GIT_TRACE_PERFORMANCE", "GIT_TRACE2", "GIT_TRACE2_EVENT", "GIT_TRACE2_PERF",
    ) if os.environ.get(name) or (name == "GITCOMET_TEST_SYNC_TRACE" and name in os.environ)]
    if instrumentation:
        parser.error(f"Disable external instrumentation/worker overrides for comparable probes: {instrumentation}")
    directory = (args.output or runner.REPORTS / "application-probe").resolve()
    # Refuse to combine fresh results with stale successes after a failure.
    directory.mkdir(parents=True, exist_ok=False)
    metadata = dict(
        runner_image=os.environ.get("ImageVersion"),
        job="/".join(os.environ.get(key, "local") for key in
                     ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_JOB", "RUNNER_NAME")),
        profiles=args.profiles, samples=args.samples, fixtures=args.fixtures,
        status_state=args.status_state, git_executable=str(args.git_executable) if args.git_executable else None,
        success=False,
    )
    original_reports = runner.REPORTS
    try:
        runner.REPORTS = directory
        metadata.update(
            platform=platform.platform(), machine=platform.machine(), cpus=os.cpu_count(),
            sha=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=runner.ROOT, text=True).strip(),
            dirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=runner.ROOT)),
            rust=subprocess.check_output(["rustc", "-vV"], text=True).strip(),
            git=subprocess.check_output(["git", "--version"], text=True).strip(),
            git_lfs=subprocess.check_output(["git", "lfs", "version"], text=True).strip(),
        )
        for profile in args.profiles:
            build = directory / f"{profile}-build.jsonl"
            runner.run(f"application-probe-build-{profile}", [
                "cargo", "build", "--locked", "-p", "gitcomet-git-gix", "--features", "benchmarks",
                "--example", "operation-context-probe", "--profile", profile, "--message-format", "json",
            ], output=build, timeout=1800)
            artifacts = [item for line in build.read_text(encoding="utf-8").splitlines()
                         if (item := json.loads(line)).get("reason") == "compiler-artifact"
                         and item["target"]["name"] == "operation-context-probe" and item.get("executable")]
            if len(artifacts) != 1:
                raise RuntimeError("Expected exactly one application probe executable")
            binary = artifacts[0]["executable"]
            for fixture in args.fixtures:
                for diagnostics in ((False,) if args.latency_only else (False, True)):
                    name = f"{profile}-{fixture}-{'diagnostics' if diagnostics else 'latency'}"
                    runner.run(f"application-probe-{name}", [
                        binary, "--fixture", fixture, "--samples", str(args.samples),
                        "--profile-label", profile, "--output", str(directory / f"{name}.json"),
                        "--status-state", args.status_state,
                        *(["--git-executable", str(args.git_executable.resolve())] if args.git_executable else []),
                        *(["--diagnostics"] if diagnostics else []),
                    ], timeout=600, live=False)
        metadata["success"] = True
    finally:
        runner.REPORTS = original_reports
        (directory / "environment.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
