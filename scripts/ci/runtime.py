#!/usr/bin/env python3
"""Repeat execution of already compiled tests, retaining every coverage report."""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import uuid

import run as runner


def measure(output, samples, schedule, threads, nextest_profile="ci"):
    if samples < 1 or (threads is not None and (threads < 1 or schedule != "serial")):
        raise ValueError("positive samples/threads required; threads requires serial")
    if nextest_profile not in runner.NEXTEST_PROFILES:
        raise ValueError(f"Unsupported nextest profile: {nextest_profile}")
    instrumentation = [name for name in ("GITCOMET_CI_FIXTURE_TIMINGS", "GITCOMET_TEST_SYNC_TRACE",
                                         "GIT_TRACE2", "GIT_TRACE2_EVENT", "GIT_TRACE2_PERF",
                                         "GIT_TRACE", "GIT_TRACE_PERFORMANCE")
                       if os.environ.get(name) or
                       (name == "GITCOMET_TEST_SYNC_TRACE" and name in os.environ)]
    if instrumentation:
        raise ValueError(f"Disable instrumentation for acceptance measurements: {instrumentation}")
    output = output.resolve()
    source = runner.paths("workspace").resolve()
    if output.is_relative_to(source):
        raise ValueError("output must be outside the workspace report directory")
    output.mkdir(parents=True, exist_ok=False)
    coverage = source / "coverage.json"
    build = json.loads(coverage.read_text(encoding="utf-8")) if coverage.exists() else {}
    metadata = {
        "measurement_id": str(uuid.uuid4()),
        "sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=runner.ROOT, text=True).strip(),
        "dirty": bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=runner.ROOT)),
        "platform": sys.platform, "machine": platform.machine(), "cpus": os.cpu_count(),
        "os_version": platform.version(), "runner_image": os.environ.get("ImageVersion"),
        "profile": build.get("profile"), "selection": build.get("selection"),
        "rust": subprocess.check_output(["rustc", "-Vv"], text=True).strip(),
        "job": "/".join(os.environ.get(key, "local") for key in
                        ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_JOB", "RUNNER_NAME")),
        "git": subprocess.check_output(["git", "--version"], text=True).strip(),
        "samples": [],
    }
    original_reports = runner.REPORTS
    try:
        for index in range(samples):
            # Each execution owns its complete report tree, including the raw
            # nextest/UI logs outside workspace/. Reuse compile metadata only;
            # stale summaries or JUnit files must never enter a new sample.
            sample = output / f"sample-{index + 1}"
            summary = sample / "workspace/execution.json"
            try:
                sample.mkdir()
                shutil.copytree(source, sample / "workspace",
                                ignore=shutil.ignore_patterns("execution.json", "junit.xml"))
                runner.REPORTS = sample
                runner.execute("workspace", schedule, threads, nextest_profile)
            finally:
                runner.REPORTS = original_reports
                if summary.exists():
                    metadata["samples"].append(json.loads(summary.read_text(encoding="utf-8")))
                else:
                    metadata["samples"].append({"success": False, "seconds": None,
                                                "nextest_profile": nextest_profile,
                                                "schedule": schedule, "nextest_threads": threads})
    finally:
        runner.REPORTS = original_reports
        (output / "runtime.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="New directory outside target/ci-reports/workspace")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--schedule", choices=("serial", "balanced"), default="serial")
    parser.add_argument("--nextest-threads", type=int)
    parser.add_argument("--nextest-profile", choices=runner.NEXTEST_PROFILES, default="ci")
    args = parser.parse_args()
    measure(args.output, args.samples, args.schedule, args.nextest_threads, args.nextest_profile)


if __name__ == "__main__":
    main()
