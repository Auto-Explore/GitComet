#!/usr/bin/env python3
"""Alternate frozen backend drivers against identical, aged Windows fixtures.

The first pass primes on-disk previews outside timing. Each timed invocation
warms its own in-memory caches. Hash witnesses are checked outside timers.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import platform
import shlex
import statistics
import subprocess
import sys
import time

spec = importlib.util.spec_from_file_location("lfs_fixture", Path(__file__).with_name("lfs-performance.py"))
lfs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lfs)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def distribution(times):
    times = sorted(times)
    return {"count": len(times), "median_ms": statistics.median(times),
            "p95_ms": times[(len(times) * 95 + 99) // 100 - 1], "max_ms": max(times)}


def run(args):
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    env = lfs.isolated_environment(root)
    repo = root / "repo"
    repo.mkdir()
    lfs.git(repo, env, "init", "-q", "-b", "fixture")
    for key, value in {"user.name": "Probe", "user.email": "probe@example.invalid", "core.autocrlf": "false",
                       "commit.gpgsign": "false"}.items():
        lfs.git(repo, env, "config", key, value)
    expected = {}
    cases = []
    for size in (32 * 1024, 1024 * 1024, 16 * 1024 * 1024):
        name = f"file-{size}.txt"
        old = (b"old contents\n" * (size // 13 + 1))[:size]
        new = (b"new contents\n" * (size // 13 + 1))[:size]
        (repo / name).write_bytes(old)
        expected[name] = {"old": hashlib.sha256(old).hexdigest(), "new": hashlib.sha256(new).hexdigest()}
        cases.append(("diff", name))
    lfs.git(repo, env, "add", ".")
    lfs.git(repo, env, "commit", "-qm", "fixture")
    for _, name in cases:
        size = (repo / name).stat().st_size
        (repo / name).write_bytes((b"new contents\n" * (size // 13 + 1))[:size])
    cases.append(("status", "-"))
    lfs.git(root, env, "init", "--bare", "remote.git")
    lfs.git(repo, env, "remote", "add", "origin", root / "remote.git")
    lfs.git(repo, env, "push", "-u", "origin", "fixture")
    # Continuous low-volume output exposed the sliding flush deadline. Use
    # the real owned Git child, hook and operation context, with no network.
    code = 'import time; [(print("tick", flush=True), time.sleep(.02)) for _ in range(60)]'
    hook = repo / ".git/hooks/pre-push"
    hook.write_text("#!/bin/sh\n" + shlex.quote(sys.executable.replace("\\", "/")) + " -c " + shlex.quote(code) + "\n", newline="\n")
    hook.chmod(0o755)
    cases.append(("push", "-"))
    cases = [(operation, path) for operation, path in cases if operation in args.operations]
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    environments = {}
    for variant in binaries:
        temporary = root / (variant + "-temp")
        temporary.mkdir()
        environments[variant] = {**env, "TEMP":str(temporary), "TMP":str(temporary)}
    def invoke(variant, operation, path, samples, warmups):
        command = [str(binaries[variant]), str(repo), operation, path, str(samples), str(warmups)]
        if operation == "push":
            command.append("context")
        result = subprocess.run(command,
                                env=environments[variant], capture_output=True, text=True, timeout=300, check=True)
        record = json.loads(result.stdout)
        if operation == "diff":
            for sample in record["samples"]:
                for side in ("old", "new"):
                    if digest(Path(sample["witness"][side])) != expected[path][side]:
                        raise ValueError(f"Incorrect {variant} {operation} {side}")
        elif operation == "status":
            assert all(s["witness"] == {"staged":0,"unstaged":3} for s in record["samples"])
        else:
            assert all(sum(event["bytes"] for event in s["progress"]) >= 300 for s in record["samples"]), "Missing hook progress"
        return record
    # Establish identical warm disk state, then let NTFS ChangeTime age beyond
    # the correctness guard. This delay is never charged to an operation.
    for variant in binaries:
        for operation, path in cases:
            invoke(variant, operation, path, 1, 0)
    time.sleep(2.1)
    report = {"version":1,"machine":platform.node(),"session":args.session,"pairs":args.pairs,
              "operations":args.operations,
              "hashes":{name:digest(binary) for name,binary in binaries.items()},"samples":[],"complete":False}
    try:
        for pair in range(args.pairs):
            order = ("baseline","candidate") if (pair + args.reverse) % 2 == 0 else ("candidate","baseline")
            for operation, path in cases:
                for variant in order:
                    result = invoke(variant, operation, path, 1 if operation == "push" else args.samples, 0 if operation == "push" else 5)
                    record = {"pair":pair,"variant":variant,"case":f"{operation}/{path}",**result}
                    record["summary"] = distribution([s["milliseconds"] for s in record["samples"]])
                    if operation == "push":
                        record["summary"]["first_progress_ms"] = result["samples"][0]["progress"][0]["at_ms"]
                    report["samples"].append(record)
                    print(f"{variant} {record['case']}: {record['summary']['median_ms']:.3f} ms", flush=True)
        report["complete"] = True
    finally:
        (root / "report.json").write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("baseline","candidate","output"):
        parser.add_argument("--"+name,type=Path,required=True)
    parser.add_argument("--session",required=True)
    parser.add_argument("--pairs",type=int,default=3)
    parser.add_argument("--samples",type=int,default=35)
    parser.add_argument("--reverse",action="store_true")
    parser.add_argument("--operations",nargs="+",choices=("diff","status","push"),default=["diff","status","push"],
                        help="Operation groups to repeat; default includes source loads and controls")
    args = parser.parse_args()
    if args.pairs < 1 or not 20 <= args.samples <= 1000:
        parser.error("Positive pairs and 20..1000 samples required")
    run(args)


if __name__ == "__main__":
    main()
