#!/usr/bin/env python3
"""Measure cache-key traversal and linker discovery on the same Windows inputs.

These are phase measurements, not clean-build or end-to-end CI timings. Run
without competing compilation, tests or application latency measurements.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

import cache
import run as runner


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=35)
    args = parser.parse_args()
    if os.name != "nt" or args.samples < 1:
        parser.error("Windows and a positive sample count are required")
    baseline = args.baseline.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    record = dict(evidence="local-phase-screening", platform=platform.platform(), cpus=os.cpu_count(),
                  baseline=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=baseline, text=True).strip(),
                  success=False, results=[])
    original_reports = runner.REPORTS
    runner.REPORTS = args.output
    try:
        spec = importlib.util.spec_from_file_location("baseline_cache", baseline / "scripts/ci/cache.py")
        before = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(before)
        # Both implementations see exactly the same manifests, including the
        # disposable .worktrees checkout that exposed the traversal issue.
        before.ROOT = cache.ROOT
        started = time.perf_counter()
        linker_env = runner.windows_linker_environment()
        record["linker_bootstrap_ms"] = (time.perf_counter() - started) * 1000
        clean_env = {key: value for key, value in os.environ.items() if not key.startswith("GITCOMET_LINKER_")}

        def link(script, env):
            result = subprocess.run(["cmd.exe", "/d", "/c", str(script), "/?"], cwd=runner.ROOT,
                                    env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=True)
            # Both paths must reach the same linker with the same arguments.
            if b"USAGE:" not in result.stdout.upper():
                raise RuntimeError("Linker help did not execute")

        actions = {
            "cache-key": (lambda: before.cache_keys("windows-local-workspace-ci-test"),
                          lambda: cache.cache_keys("windows-local-workspace-ci-test")),
            "linker-launch": (lambda: link(baseline / "scripts/windows/msvc-linker.cmd", clean_env),
                              lambda: link(runner.ROOT / "scripts/windows/msvc-linker.cmd", linker_env)),
        }
        for name, modes in actions.items():
            values = [[], []]
            for sample in range(5 + args.samples):
                for mode in ([0, 1] if sample % 2 == 0 else [1, 0]):
                    started = time.perf_counter()
                    modes[mode]()
                    elapsed = (time.perf_counter() - started) * 1000
                    if sample >= 5:
                        values[mode].append(elapsed)
            summaries = [dict(median_ms=statistics.median(value),
                              p95_ms=sorted(value)[-(len(value) // 20 + 1)], samples_ms=value)
                         for value in values]
            record["results"].append(dict(phase=name, baseline=summaries[0], candidate=summaries[1],
                                          improvement_percent=(1 - summaries[1]["median_ms"] / summaries[0]["median_ms"]) * 100))
            print(f'{name}: {summaries[0]["median_ms"]:.2f} -> {summaries[1]["median_ms"]:.2f} ms', flush=True)
        record["success"] = True
    finally:
        runner.REPORTS = original_reports
        (args.output / "workflow.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
