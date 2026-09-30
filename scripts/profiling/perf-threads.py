#!/usr/bin/env python3
"""Split a `perf record --call-graph` capture of GitComet into thread groups.

  perf-threads.py CPU.DATA [--since-ms MS] [--until-ms MS] [--top N] [--json OUT]

Groups: ui (the main thread), store (store worker and executor pools),
background (GPUI executor workers), git (gix threads), watcher, driver
(GPU driver threads), probe (UI probe overhead), other. For each group it
prints on-CPU time, then the hottest functions by self and by inclusive
samples, with the share of the group's samples and the absolute CPU time,
since percentages alone mislead once another hotspot shrinks. Allocator
frames (mimalloc, libc malloc) are summarised separately.

Times come from sample counts: pass the frequency the capture used
(`perf record -F`); the default matches the README's 199 Hz. Use
--since-ms/--until-ms (relative to the first sample) to isolate a phase.
"""

import argparse
import collections
import json
import re
import subprocess
import sys

GROUPS = [
    ("probe", re.compile(r"^ui-probe")),
    ("store", re.compile(r"^gitcomet-(store|repo-l|repo-load|history|sigint)")),
    ("watcher", re.compile(r"notify-rs|gitcomet-repo-mon|inotify")),
    ("git", re.compile(r"^(gix|gitoxide)")),
    ("background", re.compile(r"^(Worker|blocking|async-io|smol|Timer)")),
    ("driver", re.compile(r"^\[vk|^nvidia|^\[")),
]
ALLOCATOR = re.compile(r"^(mi_|_mi_|malloc|free|realloc|calloc|__libc_malloc|_int_malloc|_int_free|cfree)")


def group_of(comm, tid, main_tid):
    if tid == main_tid:
        return "ui"
    for name, pattern in GROUPS:
        if pattern.search(comm):
            return name
    return "other"


def parse(path):
    """Yield (comm, tid, time_s, frames) with frames leaf first."""
    script = subprocess.Popen(["perf", "script", "-i", str(path), "-F", "comm,tid,time,ip,sym,dso",
                               "--no-demangle", "--no-inline"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                              text=True, errors="replace")
    header = re.compile(r"^\s*(?P<comm>.+?)\s+(?P<tid>\d+)\s+(?P<time>\d+\.\d+):")
    current = None
    for line in script.stdout:
        if not line.strip():
            if current:
                yield current
            current = None
            continue
        match = header.match(line)
        if match and current is None:
            current = (match["comm"].strip(), int(match["tid"]), float(match["time"]), [])
            continue
        if current is not None:
            parts = line.strip().split(None, 1)
            if len(parts) == 2:
                symbol = parts[1].rsplit(" (", 1)[0]
                dso = parts[1].rsplit("(", 1)[-1].rstrip(")")
                current[3].append(symbol if symbol != "[unknown]" else f"[{dso.rsplit('/', 1)[-1]}]")
    if current:
        yield current
    script.wait()


def demangle(names):
    try:
        output = subprocess.run(["rustfilt"], input="\n".join(names), capture_output=True, text=True,
                                check=True).stdout.splitlines()
        if len(output) == len(names):
            return dict(zip(names, output))
    except (OSError, subprocess.CalledProcessError):
        pass
    return {name: re.sub(r"::h[0-9a-f]{16}$", "", name) for name in names}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("data")
    parser.add_argument("--frequency", type=float, default=199.0)
    parser.add_argument("--main-tid", type=int, help="defaults to the process's first thread")
    parser.add_argument("--since-ms", type=float)
    parser.add_argument("--until-ms", type=float)
    parser.add_argument("--top", type=int, default=15)
    parser.add_argument("--json", help="write the breakdown here too")
    args = parser.parse_args()

    samples = list(parse(args.data))
    if not samples:
        sys.exit("no samples")
    start = min(sample[2] for sample in samples)
    main_tid = args.main_tid or min(sample[1] for sample in samples if sample[0] == "gitcomet")
    by_group = collections.defaultdict(lambda: {"samples": 0, "self": collections.Counter(),
                                                "inclusive": collections.Counter(), "allocator": 0,
                                                "threads": collections.Counter()})
    for comm, tid, when, frames in samples:
        offset_ms = (when - start) * 1000
        if args.since_ms is not None and offset_ms < args.since_ms:
            continue
        if args.until_ms is not None and offset_ms > args.until_ms:
            continue
        group = by_group[group_of(comm, tid, main_tid)]
        group["samples"] += 1
        group["threads"][comm] += 1
        if frames:
            group["self"][frames[0]] += 1
            for name in set(frames):
                group["inclusive"][name] += 1
            if any(ALLOCATOR.match(frame) for frame in frames[:3]):
                group["allocator"] += 1

    names = {name for group in by_group.values() for counter in (group["self"], group["inclusive"])
             for name, _ in counter.most_common(args.top)}
    pretty = demangle(sorted(names))
    per_sample_ms = 1000.0 / args.frequency
    report = {}
    total = sum(group["samples"] for group in by_group.values())
    for name, group in sorted(by_group.items(), key=lambda item: -item[1]["samples"]):
        count = group["samples"]
        report[name] = {
            "cpu_ms": count * per_sample_ms, "share_of_process": count / total if total else 0,
            "allocator_ms": group["allocator"] * per_sample_ms,
            "threads": dict(group["threads"].most_common(8)),
            "self": [(pretty.get(fn, fn), n * per_sample_ms, n / count) for fn, n in group["self"].most_common(args.top)],
            "inclusive": [(pretty.get(fn, fn), n * per_sample_ms, n / count)
                          for fn, n in group["inclusive"].most_common(args.top)],
        }
        print(f"\n== {name}: {report[name]['cpu_ms']:.0f} ms on CPU "
              f"({report[name]['share_of_process']:.1%} of samples), allocator {report[name]['allocator_ms']:.0f} ms")
        print("   threads: " + ", ".join(f"{comm} {n}" for comm, n in group["threads"].most_common(6)))
        print("   self:")
        for fn, ms, share in report[name]["self"]:
            print(f"     {ms:8.1f} ms {share:6.1%}  {fn[:140]}")
        print("   inclusive:")
        for fn, ms, share in report[name]["inclusive"]:
            print(f"     {ms:8.1f} ms {share:6.1%}  {fn[:140]}")
    if args.json:
        with open(args.json, "w", encoding="utf-8") as output:
            json.dump(report, output, indent=2)


if __name__ == "__main__":
    main()
