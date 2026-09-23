#!/usr/bin/env python3
"""Repeat execution of already compiled tests, retaining every coverage report."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import uuid

import run as runner


def measure(output, samples, schedule, threads, nextest_profile="ci", ui_threads=None, session=None, batch_pure_tests="auto"):
    if samples < 1 or any(value is not None and (value < 1 or schedule != "serial")
                          for value in (threads, ui_threads)):
        raise ValueError("positive samples/threads required; threads requires serial")
    if nextest_profile not in runner.NEXTEST_PROFILES:
        raise ValueError(f"Unsupported nextest profile: {nextest_profile}")
    runner.batch_pure_enabled(batch_pure_tests)
    if any(os.environ.get(name) for name in ("RUST_TEST_THREADS", "NEXTEST_TEST_THREADS")):
        raise ValueError("Use --ui-threads/--nextest-threads instead of thread environment overrides for measurements")
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
        "batch_pure_tests": batch_pure_tests,
        "machine_id": platform.node(), "local_session": session,
        "runner_environment": os.environ.get("RUNNER_ENVIRONMENT"),
        # Raw bytes, like local-performance.py: text mode rejects non-UTF-8 and rewrites CRLF.
        "source_diff_sha256": hashlib.sha256(subprocess.check_output(
            ["git", "diff", "--binary", "HEAD"], cwd=runner.ROOT)).hexdigest(),
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
                runner.execute("workspace", schedule, threads, nextest_profile, ui_threads, batch_pure_tests)
            finally:
                runner.REPORTS = original_reports
                if summary.exists():
                    metadata["samples"].append(json.loads(summary.read_text(encoding="utf-8")))
                else:
                    metadata["samples"].append({"success": False, "seconds": None,
                                                "nextest_profile": nextest_profile,
                                                "schedule": schedule, "nextest_threads": threads,
                                                "ui_threads": ui_threads})
    finally:
        runner.REPORTS = original_reports
        (output / "runtime.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="New directory outside target/ci-reports/workspace")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--schedule", choices=("serial", "balanced"), default="serial")
    parser.add_argument("--nextest-threads", type=int)
    parser.add_argument("--ui-threads", type=int)
    parser.add_argument("--nextest-profile", choices=runner.NEXTEST_PROFILES, default="ci")
    parser.add_argument("--session", help="Independent local measurement session identifier")
    parser.add_argument("--batch-pure-tests", choices=("auto", "on", "off"), default="auto")
    parser.add_argument("--checkout", type=Path, help="Use this checkout's compiled inventory with the current driver")
    args = parser.parse_args()
    if args.checkout:
        runner.ROOT = args.checkout.resolve()
        runner.REPORTS = runner.ROOT / "target/ci-reports"
    measure(args.output, args.samples, args.schedule, args.nextest_threads, args.nextest_profile,
            args.ui_threads, args.session, args.batch_pure_tests)


if __name__ == "__main__":
    main()
