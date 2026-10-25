#!/usr/bin/env python3
"""Local, witnessed GitComet performance investigations on Linux, macOS and Windows.

Build first. Measurements never build or download repositories. Use doctor,
prepare, run, compare and profile --help for the individual commands.
"""
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys
import time
import uuid

import perf_corpus
import perf_metadata
import perf_platform
import perf_report
import perf_workloads

live = perf_workloads.module("performance_live", "live-ui.py")
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CORPUS = Path(os.environ.get("GITCOMET_PERF_CORPUS", str(Path.home() / "git/git_test_repos")))
EXE = ".exe" if os.name == "nt" else ""


def save(path, data):
    temporary = Path(path).with_suffix(".tmp")
    temporary.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def doctor(args):
    tools = {name: perf_metadata.run(command) for name, command in
             {"git": ["git", "--version"], "git-lfs": ["git", "lfs", "version"],
              "git-annex": ["git", "annex", "version", "--raw"], "rust": ["rustc", "--version"]}.items()}
    root = args.repo_root.resolve()
    ancestor = next((p for p in [root, *root.parents] if p.exists()), Path.cwd())
    data = {"version": 1, "python": sys.version, "capabilities": perf_platform.capabilities(),
            "tools": tools, "repo_root": str(root), "free_bytes": shutil.disk_usage(ancestor).free,
            "headless_tools": {name: shutil.which(name) for name in ("mutter", "dbus-run-session")},
            "sources": {name: {"path": path, "available": Path(path).is_dir()} for name, path in perf_corpus.SOURCES.items()},
            "note": "Windows and macOS capabilities must be validated on those hosts; no emulation is implied."}
    print(json.dumps(data, indent=2))


def cases(corpus, suite):
    names = list(corpus["fixtures"])
    small = next((n for n in names if n.startswith("history-")), names[0] if names else None)
    if small is None:
        raise ValueError("empty corpus; run prepare first")
    ui = ["startup", "repo-picker", "history-hover", "history-hover-stationary", "history-scroll", "history-drag", "history-select", "idle"]
    for scenario in ui:
        yield {"key": f"ui/{small}/{scenario}", "fixture": small, "scenario": scenario, "layer": "ui"}
    if suite == "deep":
        for name in names:
            for scenario in (["startup", "history-hover", "history-hover-stationary", "history-scroll", "history-drag", "history-select-burst", "history-jump", "history-reopen"] if name != small else
                             ["history-jump", "history-reopen", "history-select-burst", "status-save", "status-touch", "status-burst", "ignored-churn", "diff-search", "idle-minimized", "two-windows-idle", "lifecycle"]):
                yield {"key": f"ui/{name}/{scenario}", "fixture": name, "scenario": scenario, "layer": "ui"}
    operations = ["clone", "fetch", "pull", "push", "lfs-fetch", "lfs-push", "annex-get", "annex-copy"]
    if suite == "deep":
        operations += ["fetch-noop", "pull-noop", "push-noop", "pull-merge", "pull-rebase", "lfs-pull", "annex-pull", "annex-push", "annex-sync"]
    for operation in operations:
        for layer in ("cli", "backend", "live"):
            if operation == "clone" and layer == "backend":
                continue  # Cloning is owned by the production store effect.
            yield {"key": f"transfer/{layer}/{operation}", "fixture": small, "scenario": operation, "layer": layer}
            if suite == "deep" and operation.startswith(("lfs-", "annex-")):
                for cache in ("mixed", "warm"):
                    yield {"key": f"transfer/{layer}/{operation}-{cache}", "fixture": small,
                           "scenario": operation, "layer": layer, "content_cache": cache}
    if suite == "deep":
        for name in names:
            if corpus["fixtures"][name]["kind"] == "real":
                for layer in ("cli", "live"):
                    yield {"key": f"clone/{name}/{layer}", "fixture": name, "scenario": "clone", "layer": layer}
        for operation in ("fetch", "clone", "lfs-fetch"):
            yield {"key": f"cancel/{operation}", "fixture": small, "scenario": operation, "layer": "live", "cancel": True}


def run_case(case, args, binary, backend, seed, output):
    if case["layer"] == "ui":
        secondary = args.repo_root / next((n for n in perf_corpus.read(args.repo_root)["fixtures"] if n != case["fixture"]), case["fixture"])
        if case["scenario"] == "lifecycle" and secondary == seed:
            raise ValueError("lifecycle requires a second prepared repository")
        result = live.run_once(binary, seed, case["scenario"], output, args.timeout,
                               display=args.display, secondary=secondary, cycles=args.cycles,
                               ui_scale=args.ui_scale, refresh_hz=args.refresh_hz, wrap=args.wrap)
        return result
    scratch = args.repo_root / ".scratch" / uuid.uuid4().hex
    scratch.parent.mkdir(parents=True, exist_ok=True)
    success = False
    try:
        with perf_workloads.transfer(scratch, seed, case["scenario"], live, shape=args.shape,
                latency_ms=args.latency_ms, bandwidth_mib=args.bandwidth_mib,
                content_cache=case.get("content_cache", "cold"),
                fault="stall" if case.get("cancel") else "none") as fixture:
            if case["layer"] == "live":
                result = live.run_once(binary, fixture["repo"], case["scenario"], output, min(args.timeout, 45) if case.get("cancel") else args.timeout,
                                      display=args.display, scenario_steps=perf_workloads.transfer_scenario(fixture, case.get("cancel", False), args.timeout * 1000),
                                      ui_scale=args.ui_scale, refresh_hz=args.refresh_hz, wrap=args.wrap)
            else:
                output.mkdir(parents=True)
                command = (["git", "-C", str(fixture["repo"]), *fixture["cli"]] if case["layer"] == "cli" else
                           [str(backend), str(fixture["repo"]), fixture["operation"], "-", "1", "0", "context"])
                env = {**fixture["env"], "GIT_TRACE2_EVENT": str(output / "git-trace2.jsonl")}
                started = time.perf_counter()
                with (output / "stdout.log").open("wb") as stdout, (output / "stderr.log").open("wb") as stderr:
                    process = subprocess.Popen(command, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
                    try:
                        code = process.wait(timeout=args.timeout)
                    except BaseException:
                        perf_platform.stop_tree(process)
                        raise
                result = {"valid": code == 0, "milliseconds": (time.perf_counter() - started) * 1000,
                          "command": command, "problems": [] if code == 0 else [f"exit {code}; see stderr.log"]}
                if case["layer"] == "backend" and code == 0:
                    result["backend"] = json.loads((output / "stdout.log").read_text(encoding="utf-8"))
                    result["process_milliseconds"] = result["milliseconds"]
                    result["milliseconds"] = result["backend"]["samples"][0]["milliseconds"]
            if result["valid"] and not case.get("cancel"):
                result["witness"] = fixture["verify"]()
                result["bytes"] = result["witness"]["bytes"]
                if "required_content_bytes" in result["witness"]:
                    result["required_content_bytes"] = result["witness"]["required_content_bytes"]
            elif case.get("cancel"):
                result["witness"] = {"cancelled": any(r["detail"].get("cancelled") for r in result.get("operations", []))}
                result["valid"] = result["valid"] and result["witness"]["cancelled"]
            if fixture.get("server"):
                result["requests"] = list(fixture["server"].records)
                result["transport_bytes"] = sum(r.get("sent_bytes", 0) + r.get("received_bytes", 0)
                                                 for r in result["requests"]) if not case["scenario"].startswith("lfs-") else None
            save(output / "result.json", result)
            success = result["valid"]
            return result
    finally:
        if success and not args.keep_fixtures:
            perf_platform.remove_owned_tree(scratch)


def run(args):
    args.repo_root = args.repo_root.resolve()
    corpus = perf_corpus.read(args.repo_root)
    selected = [case for case in cases(corpus, args.suite) if not args.scenario or any(fnmatch.fnmatchcase(case["key"], pattern) for pattern in args.scenario)]
    if not selected:
        raise ValueError("no scenarios matched")
    if args.list:
        print("\n".join(c["key"] for c in selected))
        return 0
    # Full Chromium pack transfer is an hours-long experiment at the default
    # bandwidth. Keep it discoverable, but require an explicit scenario match.
    if not args.scenario:
        selected = [case for case in selected if not case["key"].startswith("clone/")]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binaries = {"candidate": args.binary.resolve()}
    if args.baseline:
        binaries["baseline"] = args.baseline.resolve()
    frozen = output / "binaries"
    frozen.mkdir()
    for variant, binary in binaries.items():
        destination = frozen / f"{variant}{EXE}"
        shutil.copy2(binary, destination)
        binaries[variant] = destination
    backends = {"candidate": args.backend, "baseline": args.baseline_backend}
    for variant in binaries:
        if backends.get(variant):
            target = frozen / f"{variant}-backend{EXE}"
            shutil.copy2(backends[variant], target)
            backends[variant] = target
    environment = perf_metadata.collect(binaries=list(binaries.items()) + [(f"{v}-backend", p) for v, p in backends.items() if p and v in binaries], cargo_profile=args.cargo_profile,
                                        command=shlex.join(sys.argv))
    save(output / "environment.json", environment)
    invariant = {key: perf_metadata.lookup(environment, key) for key in perf_metadata.PAIR_INVARIANT_FIELDS}
    invariant.update(hostname=platform.node(), platform=platform.platform(), ui_scale=args.ui_scale,
                     display=args.display, refresh_hz=args.refresh_hz)
    harness = hashlib.sha256()
    for path in sorted(Path(__file__).parent.glob("*.py")):
        harness.update(path.name.encode())
        harness.update(path.read_bytes())
    invariant["harness_sha256"] = harness.hexdigest()
    manifest = {"version": 1, "session": args.session, "suite": args.suite,
                "comparison_environment": invariant, "capabilities": perf_platform.capabilities(),
                "binaries": environment["binaries"], "corpus": corpus,
                "command": sys.argv, "cases": [], "complete": False}
    save(output / "manifest.json", manifest)
    seeds = {name: perf_corpus.validate(args.repo_root, corpus["fixtures"][name]) for name in {c["fixture"] for c in selected}}
    worktrees = {name: perf_corpus.worktree_identity(seed) for name, seed in seeds.items()}
    try:
        for pair in range(args.pairs):
            order = [v for v in ("baseline", "candidate") if v in binaries]
            if (pair + args.reverse) % 2:
                order.reverse()
            for case in selected:
                for variant in order:
                    directory = output / f"p{pair}-{variant}" / case["key"]
                    record = {**case, "variant": variant, "pair": pair, "directory": str(directory),
                              "measurement_kind": "live_application" if case["layer"] in ("ui", "live") else "backend_operation",
                              "workload": {"fixture": corpus["fixtures"][case["fixture"]]["identity"],
                                           "worktree_sha256": worktrees[case["fixture"]],
                                           "shape": args.shape, "latency_ms": args.latency_ms, "bandwidth_mib": args.bandwidth_mib,
                                           "cycles": args.cycles, "fault": "stall" if case.get("cancel") else "none",
                                           "content_cache": case.get("content_cache", "cold"),
                                           "transport": "directory" if case["scenario"].startswith("annex-") else "loopback_http" if case["layer"] != "ui" else None}, "status": "failed"}
                    print(f"{variant} pair={pair} {case['key']}", flush=True)
                    try:
                        if case["layer"] == "backend" and not backends.get(variant):
                            record.update(status="skipped", reason=f"--{'' if variant == 'candidate' else 'baseline-'}backend was not supplied")
                        elif case["scenario"].startswith("annex-") and not isinstance(perf_metadata.run(["git", "annex", "version", "--raw"]), str):
                            record.update(status="skipped", reason="git-annex unavailable")
                        elif case["scenario"].startswith("lfs-") and not isinstance(perf_metadata.run(["git", "lfs", "version"]), str):
                            record.update(status="skipped", reason="git-lfs unavailable")
                        else:
                            if perf_corpus.worktree_identity(seeds[case["fixture"]]) != worktrees[case["fixture"]]:
                                raise ValueError("fixture worktree changed since the start of this session")
                            result = run_case(case, args, binaries[variant], backends.get(variant), seeds[case["fixture"]], directory)
                            if perf_corpus.worktree_identity(seeds[case["fixture"]]) != worktrees[case["fixture"]]:
                                raise ValueError("scenario did not restore the fixture worktree; inspect retained artifacts")
                            record.update(result=result, status="passed" if result["valid"] else "failed",
                                          findings=perf_report.findings(result, args.refresh_hz))
                            if result.get("allocation_tracking") or (args.wrap and case["layer"] in ("ui", "live")):
                                record["measurement_kind"] = "diagnostic"
                                record["findings"] = [f for f in record["findings"] if f["kind"] not in ("target_exceeded", "long_frame")]
                            if args.purpose == "validation":
                                record["measurement_kind"] = "validation"
                                record["findings"] = [f for f in record["findings"] if f["kind"] not in ("target_exceeded", "long_frame", "retention_candidate")]
                    except (OSError, ValueError, AssertionError, RuntimeError, subprocess.SubprocessError) as error:
                        record["reason"] = str(error)
                        print(f"  failed: {error}", flush=True)
                    manifest["cases"].append(record)
                    save(output / "manifest.json", manifest)
                    save(output / "findings.json", perf_report.ranked_findings(manifest))
                    (output / "report.txt").write_text(perf_report.render(manifest), encoding="utf-8")
        for name, seed in seeds.items():
            perf_corpus.validate(args.repo_root, corpus["fixtures"][name])
        manifest["complete"] = all(c["status"] == "passed" for c in manifest["cases"])
    finally:
        save(output / "manifest.json", manifest)
    print(perf_report.render(manifest))
    return perf_report.exit_status(manifest["cases"], args.strict)


def profile(args):
    """Print exact native commands by default; --execute is explicit capture."""
    binary, output = args.binary.resolve(), args.output.resolve()
    tools = {"cpu": {"Linux": ["perf", "record", "-F", "199", "--call-graph", "dwarf,16384", "-o", str(output / "cpu.data"), "--", str(binary)],
                     "Darwin": ["xctrace", "record", "--template", "Time Profiler", "--output", str(output / "cpu.trace"), "--launch", "--", str(binary)],
                     "Windows": ["wpr", "-start", "CPU", "-filemode"]},
             "waits": {"Linux": ["strace", "-ff", "-tt", "-T", "-o", str(output / "syscalls"), str(binary)],
                       "Windows": ["wpr", "-start", "GeneralProfile", "-filemode"],
                       "Darwin": ["xctrace", "record", "--template", "System Trace", "--output", str(output / "waits.trace"), "--launch", "--", str(binary)]}}
    command = tools[args.kind][platform.system()]
    print(json.dumps({"measurement_kind": "diagnostic", "command": command,
                      "note": "Use release-with-debug symbols. Windows: run the scenario while recording, then wpr -stop capture.etl. Allocation capture: build with --features perf-alloc and run the same scenario; its timings are diagnostic."}, indent=2))
    if args.execute:
        if args.repository is None:
            raise ValueError("--execute requires --repository so the capture runs an isolated, witnessed scenario")
        if platform.system() == "Linux":
            result = live.run_once(binary, args.repository, args.scenario, output, args.timeout,
                                   display=args.display, wrap=shlex.join(command[:-1]),
                                   secondary=args.secondary_repository, cycles=args.cycles)
        elif platform.system() == "Windows":
            subprocess.run(command, check=True)
            try:
                result = live.run_once(binary, args.repository, args.scenario, output, args.timeout,
                                       display="desktop", secondary=args.secondary_repository, cycles=args.cycles)
            finally:
                output.mkdir(parents=True, exist_ok=True)
                subprocess.run(["wpr", "-stop", str(output / "capture.etl")], check=True)
        else:
            raise ValueError("macOS: attach the printed Instruments template to a desktop scenario; automatic profiler launch is not implemented")
        result["measurement_kind"] = "diagnostic"
        if platform.system() == "Linux" and args.kind == "cpu":
            report = subprocess.run(["perf", "report", "--stdio", "--no-children", "-g", "none",
                                     "--percent-limit", "100", "-i", str(output / "cpu.data")],
                                    capture_output=True, text=True, timeout=120)
            (output / "profiler-quality.txt").write_text(report.stdout + report.stderr, encoding="utf-8")
            result["profiler_quality"] = perf_report.perf_quality(report.stdout, report.returncode)
            if not result["profiler_quality"]["valid"]:
                result["valid"] = False
                result["problems"].append("perf lost samples or could not establish capture quality; inspect profiler-quality.txt")
        save(output / "result.json", result)
        if not result["valid"]:
            raise ValueError("diagnostic scenario failed; inspect result.json and stderr.log")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    doc = commands.add_parser("doctor")
    doc.add_argument("--repo-root", type=Path, default=DEFAULT_CORPUS)
    prep = commands.add_parser("prepare")
    prep.add_argument("--repo-root", type=Path, default=DEFAULT_CORPUS)
    prep.add_argument("--suite", choices=("smoke", "deep"), default="smoke")
    prep.add_argument("--real", action="store_true", help="include the three existing real repositories")
    prep.add_argument("--source", action="append", metavar="NAME=PATH", help="override real sources on another machine")
    prep.add_argument("--commits", nargs="+", type=int, help="custom fixture sizes (recorded in the corpus)")
    measure = commands.add_parser("run")
    measure.add_argument("--repo-root", type=Path, default=DEFAULT_CORPUS)
    measure.add_argument("--binary", type=Path, default=ROOT / f"target/release/gitcomet{EXE}")
    measure.add_argument("--baseline", type=Path)
    measure.add_argument("--backend", type=Path)
    measure.add_argument("--baseline-backend", type=Path)
    measure.add_argument("--output", type=Path, required=True)
    measure.add_argument("--session", default="first")
    measure.add_argument("--suite", choices=("smoke", "deep"), default="smoke")
    measure.add_argument("--scenario", action="append", help="case name or glob; repeat to select several")
    measure.add_argument("--pairs", type=int, default=1)
    measure.add_argument("--reverse", action="store_true")
    measure.add_argument("--display", choices=("headless", "desktop"), default="headless" if platform.system() == "Linux" else "desktop")
    measure.add_argument("--refresh-hz", type=int, default=60)
    measure.add_argument("--ui-scale", type=int, default=100)
    measure.add_argument("--cycles", type=int, default=100)
    measure.add_argument("--timeout", type=int, default=600)
    measure.add_argument("--latency-ms", type=float, default=50)
    measure.add_argument("--bandwidth-mib", type=float, default=8)
    measure.add_argument("--shape", choices=perf_workloads.lfs.SHAPES, default="smoke")
    measure.add_argument("--cargo-profile", default="release")
    measure.add_argument("--purpose", choices=("measurement", "validation"), default="measurement",
                         help="validation checks witnesses while excluding its timings from comparisons")
    measure.add_argument("--keep-fixtures", action="store_true")
    measure.add_argument("--wrap", help="Linux diagnostic profiler prefix for native app cases; supports {output}")
    measure.add_argument("--list", action="store_true")
    measure.add_argument("--strict", action="store_true", help="fail on timing alerts as well as invalid measurements")
    comparison = commands.add_parser("compare")
    comparison.add_argument("runs", type=Path, nargs="+")
    comparison.add_argument("--output", type=Path)
    comparison.add_argument("--allocations", action="store_true", help="compare allocation counts from two diagnostic builds, excluding their timings")
    diagnostic = commands.add_parser("profile")
    diagnostic.add_argument("--binary", type=Path, required=True)
    diagnostic.add_argument("--output", type=Path, required=True)
    diagnostic.add_argument("--kind", choices=("cpu", "waits"), default="cpu")
    diagnostic.add_argument("--repository", type=Path)
    diagnostic.add_argument("--secondary-repository", type=Path)
    diagnostic.add_argument("--scenario", choices=live.SCENARIOS, default="history-hover")
    diagnostic.add_argument("--cycles", type=int, default=100)
    diagnostic.add_argument("--timeout", type=int, default=600)
    diagnostic.add_argument("--display", choices=("headless", "desktop"), default="headless" if platform.system() == "Linux" else "desktop")
    diagnostic.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "doctor":
            doctor(args)
        elif args.command == "prepare":
            sources = dict(item.split("=", 1) for item in args.source) if args.source else None
            if sources and any(not name.isidentifier() for name in sources):
                parser.error("source names must be identifiers")
            if args.commits and any(n < 100 for n in args.commits):
                parser.error("fixtures need at least 100 commits")
            perf_corpus.prepare(args.repo_root, args.suite, live, sources, args.real, args.commits)
        elif args.command == "run":
            if min(args.pairs, args.cycles, args.timeout, args.refresh_hz, args.ui_scale) <= 0 or min(args.latency_ms, args.bandwidth_mib) < 0:
                parser.error("counts must be positive; transport limits must be non-negative")
            if not 80 <= args.ui_scale <= 200:
                parser.error("GitComet supports UI scale percentages from 80 to 200")
            return run(args)
        elif args.command == "compare":
            result = perf_report.compare([json.loads((p / "manifest.json").read_text(encoding="utf-8")) for p in args.runs], allocations=args.allocations)
            if args.output:
                save(args.output, result)
            print(json.dumps(result, indent=2))
            return int(not result["valid"])
        else:
            profile(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"performance: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
