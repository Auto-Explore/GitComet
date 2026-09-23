#!/usr/bin/env python3
"""Compare coverage, collect Actions timings, and maintain the CI cache budget."""

import argparse
from datetime import datetime
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess

LEGACY_LANES = {
    "Linux Headless Suite (x86_64-linux / ubuntu-22.04)": "native/ubuntu22-x64",
    "Linux Headless Suite (aarch64-linux / ubuntu-22.04-arm)": "native/ubuntu22-arm64",
    "Linux Headless Suite (Fedora container)": "native/fedora41-x64",
    "macOS Tests (macbook-m1 / macos-15)": "native/macos15-arm64",
    "macOS Tests (latest-macos / macos-26)": "native/macos26-arm64",
    "macOS Tests (intel (macos-15-intel))": "native/macos15-x64",
    "Windows Tests (x86_64-windows)": "native/windows-x64",
    "Windows Tests (aarch64-windows)": "native/windows-arm64",
    "Benchmark Target Compile (linux / ubuntu-22.04)": "benchmark/ubuntu22-x64",
    "Benchmark Target Compile (macos / apple-silicon)": "benchmark/macos-latest-arm64",
    "Benchmark Target Compile (windows / x86_64)": "benchmark/windows-x64",
    "Benchmark Target Compile (windows / arm64)": "benchmark/windows-arm64",
}


def lane_statistics(records):
    lanes = {}
    for run in records:
        for job in run.get("jobs", []):
            lane = next((lane for name, lane in LEGACY_LANES.items() if job["name"].endswith(name)), None)
            match = re.search(r"(Native Tests|Benchmark Target Compile) \(([^()]+)\)$", job["name"])
            if lane is None and match:
                lane = ("native/" if match[1] == "Native Tests" else "benchmark/") + match[2]
            if lane:
                lanes.setdefault(lane, []).append(job)
    result = {}
    for lane, jobs in sorted(lanes.items()):
        durations = [job["seconds"] for job in jobs if job["conclusion"] == "success"]
        result[lane] = {"median_seconds": statistics.median(durations) if durations else None,
                        "successful_samples": len(durations),
                        "incomplete_samples": [{"seconds": job["seconds"], "conclusion": job["conclusion"]}
                                               for job in jobs if job["conclusion"] != "success"]}
    return result


def api(endpoint, method="GET"):
    return json.loads(subprocess.check_output(["gh", "api", "--method", method, endpoint], text=True) or "null")


def pages(endpoint, field):
    result = []
    for page in range(1, 100):
        batch = api(f"{endpoint}{'&' if '?' in endpoint else '?'}per_page=100&page={page}")[field]
        result.extend(batch)
        if len(batch) < 100:
            return result
    raise RuntimeError("API pagination exceeded 99 pages")


def elapsed(start, end):
    return (datetime.fromisoformat(end.replace("Z", "+00:00")) -
            datetime.fromisoformat(start.replace("Z", "+00:00"))).total_seconds()


def collect_runs(repository, ids):
    records = []
    for run_id in ids:
        run = api(f"repos/{repository}/actions/runs/{run_id}")
        jobs = pages(f"repos/{repository}/actions/runs/{run_id}/jobs", "jobs")
        completed = [job for job in jobs if job.get("started_at") and job.get("completed_at")]
        end = max((job["completed_at"] for job in completed), default=run["updated_at"])
        records.append({
            "id": run_id, "sha": run["head_sha"], "branch": run["head_branch"],
            "event": run["event"], "name": run["name"], "conclusion": run["conclusion"],
            "created_at": run["created_at"], "completed_at": end,
            "wall_seconds": elapsed(run["created_at"], end),
            "runner_seconds": sum(elapsed(j["started_at"], j["completed_at"]) for j in completed),
            "jobs": [{"name": job["name"], "conclusion": job["conclusion"],
                      "seconds": elapsed(job["started_at"], job["completed_at"]),
                      # This includes dependency waits when created_at is unavailable.
                      "start_delay_seconds": elapsed(run["created_at"], job["started_at"]),
                      "steps": [{"name": step["name"], "conclusion": step["conclusion"],
                                 "seconds": elapsed(step["started_at"], step["completed_at"])}
                                for step in job["steps"] if step.get("started_at") and step.get("completed_at")]}
                     for job in completed],
        })
    return records


def coverage_difference(baseline, candidate):
    def identities(document):
        return {(test["package"], test["binary"], test["test"]): test["ignored"] for test in document["tests"]}
    if baseline["context"] != candidate["context"] or baseline["selection"] != candidate["selection"]:
        raise ValueError("Compare the same platform/feature context; inventories from different contexts are not equivalent")
    before, after = identities(baseline), identities(candidate)
    missing = sorted(before.keys() - after.keys())
    newly_ignored = sorted(key for key in before.keys() & after.keys() if not before[key] and after[key])
    return {"missing": missing, "newly_ignored": newly_ignored,
            "added": sorted(after.keys() - before.keys())}


def summarize(records):
    # Caller supplies one complete validation cycle (all old workflows, or the
    # new orchestrator) per SHA/branch/event. Never count a timeout as success.
    groups = {}
    excluded = []
    for run in records:
        key = (run["sha"], run["branch"], run["event"])
        groups.setdefault(key, []).append(run)
    wall, compute = [], []
    for key, group in groups.items():
        if any(run["conclusion"] != "success" for run in group):
            excluded.append({"revision": key, "outcomes": [run["conclusion"] for run in group]})
            continue
        wall.append(elapsed(min(run["created_at"] for run in group), max(run["completed_at"] for run in group)))
        compute.append(sum(run["runner_seconds"] for run in group))
    return {"successful_samples": len(wall), "excluded_samples": excluded,
            "median_wall_seconds": statistics.median(wall) if wall else None,
            "p95_wall_seconds": sorted(wall)[max(0, math.ceil(len(wall) * .95) - 1)] if wall else None,
            "median_runner_seconds": statistics.median(compute) if compute else None,
            "lanes": lane_statistics(records)}


def cache_context(key):
    if key.startswith("gitcomet-ci-v2-deps-"):
        return "deps/" + key.removeprefix("gitcomet-ci-v2-deps-").rsplit("-", 2)[0]
    if key.startswith("gitcomet-ci-v2-sources-"):
        return "sources/" + key.removeprefix("gitcomet-ci-v2-sources-").rsplit("-", 1)[0]
    if key.startswith("gitcomet-ci-v1-"):
        return "deps/" + key.removeprefix("gitcomet-ci-v1-").rsplit("-", 1)[0]
    if key.startswith("gitcomet-ci-audit-"):
        return "gitcomet-ci-audit"
    return None


def fixture_timings(directory):
    """Aggregate opt-in helper timings. Nested phases are deliberately separate."""
    totals = {}
    for path in sorted(directory.glob("*.tsv")):
        for line in path.read_text(encoding="utf-8").splitlines():
            test, phase, operation, microseconds = line.split("\t")
            key = (test, phase, operation)
            count, elapsed = totals.get(key, (0, 0))
            totals[key] = (count + 1, elapsed + int(microseconds))
    return sorted((dict(test=test, phase=phase, operation=operation, calls=count, seconds=elapsed / 1e6)
                   for (test, phase, operation), (count, elapsed) in totals.items()),
                  key=lambda row: (-row["seconds"], row["test"], row["phase"], row["operation"]))


def runtime_statistics(directory):
    """Keep execution policies, revisions, and measurement environments separate."""
    groups, seen = {}, {}
    environment_fields = ("platform", "machine", "cpus", "sha", "os_version", "runner_image",
                          "git", "rust", "profile", "selection")
    for path in sorted(directory.rglob("runtime.json")):
        record = json.loads(path.read_text(encoding="utf-8"))
        if identity := record.get("measurement_id"):
            if identity in seen:
                if seen[identity] != record:
                    raise ValueError(f"Conflicting copies of measurement {identity}")
                continue
            seen[identity] = record
        environment = {field: record.get(field) for field in environment_fields}
        environment["dirty"] = record.get("dirty", False)
        # Hosted runners get a new hostname per job; their image identifies the machine.
        environment["runner_environment"] = record.get("runner_environment")
        environment["machine_id"] = (None if environment["runner_environment"] == "github-hosted"
                                     else record.get("machine_id"))
        environment["source_diff_sha256"] = record.get("source_diff_sha256")
        for sample in record["samples"]:
            descriptor = dict(environment, schedule=sample.get("schedule"),
                              batch_pure_tests=sample.get("batch_pure_tests", record.get("batch_pure_tests", "off")),
                              nextest_profile=sample.get("nextest_profile", "ci"),
                              nextest_threads=sample.get("nextest_threads"),
                              ui_threads=sample.get("ui_threads"),
                              effective_nextest_threads=sample.get("effective_nextest_threads"),
                              effective_ui_threads=sample.get("effective_ui_threads"))
            key = json.dumps(descriptor, sort_keys=True)
            group = groups.setdefault(key, {"descriptor": descriptor, "seconds": [], "failed": 0, "jobs": set(), "local_sessions": set()})
            group["jobs"].add(record["job"])
            if record["job"].startswith("local/") and record.get("local_session"):
                group["local_sessions"].add(record["local_session"])
            if sample["success"]:
                group["seconds"].append(sample["seconds"])
            else:
                group["failed"] += 1
    result = []
    for group in groups.values():
        values = sorted(group["seconds"])
        # Local repetitions do not establish independent hosted-job evidence.
        jobs = [job for job in group["jobs"] if not job.startswith("local/")]
        descriptor = group["descriptor"]
        environment_recorded = all(descriptor.get(field) is not None
                                   for field in environment_fields)
        local_recorded = all(descriptor.get(field) is not None for field in
                             (*[field for field in environment_fields if field != "runner_image"],
                              "machine_id", "source_diff_sha256"))
        result.append(dict(descriptor, samples=len(values), failed=group["failed"],
                           median_seconds=statistics.median(values) if values else None,
                           # Nearest-rank p95; with fewer than 20 samples this
                           # conservatively reports the slowest observation.
                           p95_seconds=values[-(len(values) // 20 + 1)] if values else None,
                           min_seconds=min(values) if values else None,
                           max_seconds=max(values) if values else None,
                           jobs=sorted(jobs), environment_recorded=environment_recorded,
                           local_sessions=sorted(group["local_sessions"]),
                           local_enough_samples=len(values) >= 6 and len(group["local_sessions"]) >= 2
                                                and not group["failed"] and local_recorded,
                           enough_samples=len(values) >= 5 and len(jobs) >= 2 and not group["failed"]
                                          and environment_recorded and not descriptor["dirty"]))
    return result


def git_trace2(path):
    """Count nested Git processes; their lifetimes overlap and are not additive."""
    processes = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        event = json.loads(line)
        if event.get("event") == "start":
            processes.setdefault(event["sid"], {})["argv"] = event["argv"]
        elif event.get("event") == "exit":
            processes.setdefault(event["sid"], {}).update(seconds=event["t_abs"], code=event["code"])
    return {"processes": len(processes), "incomplete": sum("seconds" not in p for p in processes.values()),
            "records": list(processes.values())}


def obsolete_caches(caches):
    seen, obsolete = set(), []
    # Prefer a published v2 replacement over v1 regardless of creation order.
    ordered = sorted(caches, key=lambda item: (item["key"].startswith("gitcomet-ci-v2-"), item["created_at"]), reverse=True)
    source_refs = {(cache.get("ref", ""), cache_context(cache["key"]).removeprefix("sources/"))
                   for cache in caches if cache["key"].startswith("gitcomet-ci-v2-sources-")}
    for cache in ordered:
        context = cache_context(cache["key"])
        if context is None:
            continue
        scope = cache.get("ref", "")
        # Retire the former source-only allocations only after the OS fallback exists.
        if cache["key"].startswith("gitcomet-ci-v1-") and ("benchmarks-ci-bench" in context or context.endswith("-clippy")):
            os_family = "windows" if "windows" in context else "darwin" if "macos" in context else "linux"
            if (scope, os_family) in source_refs:
                obsolete.append(cache)
                continue
        identity = (scope, context)
        if identity in seen:
            obsolete.append(cache)
        seen.add(identity)
    return obsolete


def manage_caches(repository, prune):
    caches = pages(f"repos/{repository}/actions/caches", "actions_caches")
    obsolete = obsolete_caches(caches)
    if prune:
        for cache in obsolete:
            print(f"Retiring superseded CI cache {cache['key']}")
            api(f"repos/{repository}/actions/caches/{cache['id']}", "DELETE")
    removed = {cache["id"] for cache in obsolete} if prune else set()
    owned = [cache for cache in caches if cache_context(cache["key"]) and cache["id"] not in removed]
    total = sum(cache["size_in_bytes"] for cache in owned)
    report = {"validation_cache_bytes": total, "budget_bytes": 8_000_000_000,
              "cache_count": len(owned), "obsolete_count": len(obsolete), "pruned": prune}
    print(json.dumps(report, indent=2))
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(f"\nValidation caches: {total / 1e9:.2f} GB / 8 GB in {len(owned)} entries.\n")
    if total > report["budget_bytes"]:
        raise RuntimeError("Validation cache budget exceeded; reduce cache quotas before adding contexts")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", "Auto-Explore/GitComet"))
    sub = parser.add_subparsers(dest="command", required=True)
    caches = sub.add_parser("caches")
    caches.add_argument("--prune", action="store_true")
    fixtures = sub.add_parser("fixtures")
    fixtures.add_argument("directory", type=Path)
    runtime = sub.add_parser("runtime")
    runtime.add_argument("directory", type=Path)
    trace = sub.add_parser("trace2")
    trace.add_argument("path", type=Path)
    runs = sub.add_parser("runs")
    runs.add_argument("ids", nargs="+", type=int)
    runs.add_argument("--output", type=Path, required=True)
    comparison = sub.add_parser("compare")
    comparison.add_argument("baseline", type=Path)
    comparison.add_argument("candidate", type=Path)
    coverage = sub.add_parser("coverage")
    coverage.add_argument("baseline", type=Path)
    coverage.add_argument("candidate", type=Path)
    args = parser.parse_args()
    if args.command == "caches":
        manage_caches(args.repository, args.prune)
    elif args.command == "fixtures":
        print(json.dumps(fixture_timings(args.directory), indent=2))
    elif args.command == "runtime":
        print(json.dumps(runtime_statistics(args.directory), indent=2))
    elif args.command == "trace2":
        print(json.dumps(git_trace2(args.path), indent=2))
    elif args.command == "runs":
        records = collect_runs(args.repository, args.ids)
        args.output.write_text(json.dumps(records, indent=2) + "\n")
        print(json.dumps(summarize(records), indent=2))
    elif args.command == "coverage":
        differences = coverage_difference(json.loads(args.baseline.read_text()), json.loads(args.candidate.read_text()))
        print(json.dumps(differences, indent=2))
        if differences["missing"] or differences["newly_ignored"]:
            raise SystemExit(1)
    else:
        baseline = summarize(json.loads(args.baseline.read_text()))
        candidate = summarize(json.loads(args.candidate.read_text()))
        result = {"baseline": baseline, "candidate": candidate}
        if baseline["median_wall_seconds"] and candidate["median_wall_seconds"]:
            result["wall_reduction_percent"] = 100 * (1 - candidate["median_wall_seconds"] / baseline["median_wall_seconds"])
            result["runner_reduction_percent"] = 100 * (1 - candidate["median_runner_seconds"] / baseline["median_runner_seconds"])
        result["long_lane_targets"] = {}
        for name, before in baseline["lanes"].items():
            if before["median_seconds"] is None or before["median_seconds"] < 1200:
                continue
            after = candidate["lanes"].get(name, {}).get("median_seconds")
            limit = before["median_seconds"] * .5
            result["long_lane_targets"][name] = {"limit_seconds": limit, "candidate_seconds": after,
                                                 "met": after is not None and after <= limit}
        intel = candidate["lanes"].get("native/macos15-x64", {}).get("median_seconds")
        result["intel_macos_30_minute_target"] = {"candidate_seconds": intel, "met": intel is not None and intel <= 1800}
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
