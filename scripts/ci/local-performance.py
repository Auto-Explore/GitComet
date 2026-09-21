#!/usr/bin/env python3
"""Build a shared latency driver, then compare isolated checkouts on one machine.

Build is deliberately a separate phase. Never compile or run tests alongside
measure. Local sessions are evidence for this machine, not hosted CI acceptance.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tomllib
import uuid

import run as runner

SOURCE = Path("crates/gitcomet-git-gix/examples/operation-context-probe.rs")
CASES = [("plain", "warm"), ("lfs", "warm"), ("lfs", "stale"),
         ("mixed", "stale"), ("submodule", "warm")]
INSTRUMENTATION = ("GITCOMET_CI_FIXTURE_TIMINGS", "GITCOMET_TEST_SYNC_TRACE",
                   "GITCOMET_BENCH_STATUS_WORKERS", "GIT_TRACE", "GIT_TRACE_PERFORMANCE",
                   "GIT_TRACE2", "GIT_TRACE2_EVENT", "GIT_TRACE2_PERF")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def command(args, cwd=runner.ROOT):
    return subprocess.check_output(args, cwd=cwd, text=True, encoding="utf-8").strip()


def environment():
    return dict(host=platform.node(), platform=platform.platform(), machine=platform.machine(),
                cpus=os.cpu_count(), rust=command(["rustc", "-vV"]),
                git=command(["git", "--version"]), git_lfs=command(["git", "lfs", "version"]),
                git_executable=shutil.which("git"), git_lfs_executable=shutil.which("git-lfs"),
                build_settings={key: value for key, value in os.environ.items()
                                if key.startswith(("CARGO_PROFILE_", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"))
                                or key in ("RUSTC", "CARGO_BUILD_TARGET", "GITCOMET_TARGET_ARCH")},
                python=platform.python_version())


def revision(root):
    return dict(sha=command(["git", "rev-parse", "HEAD"], root),
                diff_sha256=hashlib.sha256(subprocess.check_output(
                    ["git", "diff", "--binary", "HEAD"], cwd=root)).hexdigest(),
                status=command(["git", "status", "--porcelain"], root))


def build(args):
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    source = (runner.ROOT / SOURCE).read_bytes()
    (directory / "driver.rs").write_bytes(source)
    shutil.copy2(__file__, directory / "driver.py")
    shutil.copy2(runner.__file__, directory / "driver-runner.py")
    record = dict(schema_version=1, evidence="local", driver_sha256=hashlib.sha256(source).hexdigest(),
                  orchestrator_sha256=digest(Path(__file__)),
                  profile=args.profile, success=False, builds={})
    original_reports = runner.REPORTS
    runner.REPORTS = directory
    try:
        record.update(environment=environment(), driver_revision=revision(runner.ROOT))
        roots = dict(baseline=args.baseline.resolve(), candidate=runner.ROOT)
        if roots["baseline"] == roots["candidate"]:
            raise ValueError("Baseline needs a separate checkout and target directory")
        for label, root in roots.items():
            (directory / f"{label}.patch").write_bytes(subprocess.check_output(
                ["git", "diff", "--binary", "HEAD"], cwd=root))
            driver = root / "target/local-performance-driver"
            driver.mkdir(parents=True, exist_ok=True)
            source_path = driver / "main.rs"
            if not source_path.exists() or source_path.read_bytes() != source:
                source_path.write_bytes(source)
            manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
            # The driver has no benchmarks feature enabled, so diagnostic-only
            # imports are compiled out even on older revisions without them.
            text = '[package]\nname = "gitcomet-local-probe"\nversion = "0.0.0"\nedition = "2024"\n'
            text += '[workspace]\n[features]\nbenchmarks = []\n[[bin]]\nname = "gitcomet-local-probe"\npath = "main.rs"\n'
            text += '[dependencies]\n'
            for package in ("gitcomet-core", "gitcomet-git-gix"):
                features = ["benchmarks", "test-support"] if package == "gitcomet-core" else ["benchmarks"]
                text += f'{package} = {{ path = {json.dumps((root / "crates" / package).as_posix())}, features = {json.dumps(features)} }}\n'
            for package in ("tempfile", "serde_json"):
                text += f'{package} = {json.dumps(manifest["workspace"]["dependencies"][package])}\n'
            for profile in ("ci-test", "release"):
                text += f'[profile.{profile}]\n'
                for key, value in manifest["profile"][profile].items():
                    if key != "package":
                        text += f'{key} = {json.dumps(value)}\n'
            (driver / "Cargo.toml").write_text(text, encoding="utf-8")
            shutil.copy2(root / "Cargo.lock", driver / "Cargo.lock")
            # Offline lock pruning adds only this standalone harness package.
            runner.run(f"{label}-lock", ["cargo", "metadata", "--offline", "--manifest-path",
                       str(driver / "Cargo.toml"), "--format-version", "1"], cwd=root,
                       output=directory / f"{label}-metadata.json")
            output = directory / f"{label}-build.jsonl"
            runner.run(f"{label}-build", ["cargo", "build", "--offline", "--locked", "--manifest-path",
                       str(driver / "Cargo.toml"), "--target-dir", str(root / "target"),
                       "--profile", args.profile, "--message-format", "json"], cwd=root, output=output, timeout=1800)
            artifacts = [value for line in output.read_text(encoding="utf-8").splitlines()
                         if (value := json.loads(line)).get("reason") == "compiler-artifact"
                         and value["target"]["name"] == "gitcomet-local-probe" and value.get("executable")]
            if len(artifacts) != 1:
                raise RuntimeError("Expected exactly one probe binary")
            binary = Path(artifacts[0]["executable"])
            # Freeze the executable so later builds cannot silently change it.
            frozen = directory / (label + binary.suffix)
            shutil.copy2(binary, frozen)
            dependencies = tomllib.loads((driver / "Cargo.lock").read_text(encoding="utf-8"))["package"]
            resolved = sorted((p["name"], p["version"], p.get("source")) for p in dependencies)
            record["builds"][label] = dict(root=str(root), revision=revision(root), binary=str(frozen),
                                           binary_sha256=digest(frozen), dependencies=resolved)
        if record["builds"]["baseline"]["dependencies"] != record["builds"]["candidate"]["dependencies"]:
            raise RuntimeError("Dependency versions differ; paired application timing would be confounded")
        record["success"] = True
    finally:
        runner.REPORTS = original_reports
        write(directory / "build.json", record)


def measure(args):
    contaminated = [name for name in INSTRUMENTATION if name in os.environ]
    if contaminated:
        raise ValueError(f"Remove instrumentation/worker overrides: {contaminated}")
    directory = args.build.resolve()
    build_record = json.loads((directory / "build.json").read_text(encoding="utf-8"))
    if not build_record["success"] or environment() != build_record["environment"]:
        raise ValueError("Build must succeed and measurement environment must match")
    for binary in build_record["builds"].values():
        if digest(Path(binary["binary"])) != binary["binary_sha256"]:
            raise ValueError("Probe binary changed since build")
    output = directory / args.session
    output.mkdir(exist_ok=False)
    original_reports = runner.REPORTS
    runner.REPORTS = output
    cases = [(fixture, state) for fixture, state in CASES if fixture in args.fixtures]
    record = dict(schema_version=1, evidence="local", session=args.session, measurement_id=str(uuid.uuid4()),
                  recorded_at=datetime.now(timezone.utc).isoformat(), environment=environment(),
                  samples=args.samples, warmups=5, cases=cases, pairs=[], success=False)
    try:
        for pair in range(args.pairs):
            order = ["baseline", "candidate"]
            if bool(pair % 2) != args.reverse:
                order.reverse()
            item = dict(pair=pair + 1, order=order, results={})
            record["pairs"].append(item)
            for label in order:
                item["results"][label] = []
                for fixture, state in cases:
                    name = f'{pair + 1}-{label}-{fixture}-{state}'
                    path = output / f"{name}.json"
                    runner.run(name, [build_record["builds"][label]["binary"], "--fixture", fixture,
                               "--status-state", state, "--warmups", "5", "--samples", str(args.samples),
                               "--profile-label", build_record["profile"], "--output", str(path)],
                               timeout=600, live=False, output=output / f"{name}.stdout.json")
                    item["results"][label].append(path.name)
        record["success"] = True
    finally:
        runner.REPORTS = original_reports
        write(output / "session.json", record)


def summarize(directory):
    rows, sessions, seen = {}, set(), set()
    build_record = json.loads((directory / "build.json").read_text(encoding="utf-8"))
    if not build_record["success"]:
        raise ValueError("Failed build cannot certify results")
    qualified = True
    for path in sorted(directory.glob("*/session.json")):
        session = json.loads(path.read_text(encoding="utf-8"))
        if session["measurement_id"] in seen:
            raise ValueError("Copied session would duplicate evidence")
        seen.add(session["measurement_id"])
        if not session["success"]:
            raise ValueError(f"Failed session cannot certify results: {path}")
        if session["environment"] != build_record["environment"]:
            raise ValueError("Measurement environments differ")
        qualified &= session["samples"] >= 35 and session["warmups"] >= 5
        sessions.add(session["session"])
        for pair in session["pairs"]:
            if set(pair["results"]) != {"baseline", "candidate"}:
                raise ValueError("Incomplete pair")
            cases = session.get("cases", CASES)
            if any(len(pair["results"][label]) != len(cases) for label in pair["results"]):
                raise ValueError("Incomplete fixture selection")
            for label in ("baseline", "candidate"):
                observed = set()
                for name in pair["results"][label]:
                    report = json.loads((path.parent / name).read_text(encoding="utf-8"))
                    if report["diagnostics"]:
                        raise ValueError("Diagnostic capture is not latency evidence")
                    case = (report["fixture"], report["status_state"])
                    if case in observed or case not in [tuple(value) for value in cases]:
                        raise ValueError("Duplicate or unexpected fixture")
                    observed.add(case)
                    for result in report["results"]:
                        key = (report["fixture"], report["status_state"], result["operation"], result["mode"])
                        row = rows.setdefault(key, dict(baseline=[], candidate=[], baseline_p95=[], candidate_p95=[]))
                        row[label].append(result["median_ms"])
                        row[label + "_p95"].append(result["p95_ms"])
    results = []
    for key, row in rows.items():
        base, candidate = (statistics.median(row[label]) for label in ("baseline", "candidate"))
        tail = statistics.median(row["candidate_p95"]) / statistics.median(row["baseline_p95"]) - 1
        enough = qualified and len(sessions) >= 2 and len(row["baseline"]) >= 6
        results.append(dict(zip(("fixture", "status_state", "operation", "mode"), key)) |
                       dict(baseline_ms=base, candidate_ms=candidate, improvement_percent=(1 - candidate / base) * 100,
                            p95_change_percent=tail * 100, pairs=len(row["baseline"]),
                            passes_local_gate=enough and candidate <= base * .9 and tail <= .05, raw_pairs=row))
    return dict(evidence="local", sessions=sorted(sessions), results=results)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="phase", required=True)
    build_parser = sub.add_parser("build")
    build_parser.add_argument("--baseline", type=Path, required=True)
    build_parser.add_argument("--output", type=Path, required=True)
    build_parser.add_argument("--profile", choices=("ci-test", "release"), default="release")
    measure_parser = sub.add_parser("measure")
    measure_parser.add_argument("--build", type=Path, required=True)
    measure_parser.add_argument("--session", required=True)
    measure_parser.add_argument("--pairs", type=int, choices=range(1, 11), default=3)
    measure_parser.add_argument("--samples", type=int, choices=range(1, 10001), default=35)
    measure_parser.add_argument("--reverse", action="store_true")
    measure_parser.add_argument("--fixtures", nargs="+", choices=("plain", "lfs", "mixed", "submodule"),
                                default=["plain", "lfs", "mixed", "submodule"])
    report_parser = sub.add_parser("report")
    report_parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    if args.phase == "build":
        build(args)
    elif args.phase == "measure":
        if Path(args.session).name != args.session or args.session in (".", ".."):
            parser.error("--session must be a directory name")
        measure(args)
    else:
        report = summarize(args.directory)
        write(args.directory / "comparison.json", report)
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
