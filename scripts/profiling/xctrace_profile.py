#!/usr/bin/env python3
"""Self and inclusive CPU time per function from an Instruments Time Profiler trace.

  xctrace_profile.py TRACE.trace [--frames frames.jsonl --phase NAME] [--thread SUBSTRING]
                     [--top N] [--demangler PATH] [--focus REGEX]

Samples can be limited to a live-ui.py scenario phase: the phase's wall-clock
bounds come from the run's frames.jsonl and the trace's start date. Rust v0
symbols are demangled with a `rustc-demangle` filter (one symbol per line on
stdin) when --demangler is given. --focus keeps only stacks containing a
matching frame and reports callees below it.
"""
import argparse
import collections
from datetime import datetime
import json
from pathlib import Path
import re
import subprocess
import xml.etree.ElementTree as ET


def export(trace, schema):
    xpath = f'/trace-toc/run[@number="1"]/data/table[@schema="{schema}"]'
    return subprocess.run(["xcrun", "xctrace", "export", "--input", str(trace), "--xpath", xpath],
                          check=True, capture_output=True).stdout


def trace_start_unix_ms(trace):
    toc = subprocess.run(["xcrun", "xctrace", "export", "--input", str(trace), "--toc"],
                         check=True, capture_output=True, text=True).stdout
    match = re.search(r"<start-date>([^<]+)</start-date>", toc)
    return datetime.fromisoformat(match.group(1)).timestamp() * 1000


def samples(xml_bytes):
    """Yields (time_ns, thread_name, weight_ns, [frame names leaf first])."""
    root = ET.fromstring(xml_bytes)
    node = root.find("node")
    seen = {}

    def remember(element):
        if element.get("id") is not None:
            seen[element.get("id")] = element
        for child in element:
            remember(child)

    for row in node.findall("row"):
        remember(row)

        def resolve(element):
            ref = element.get("ref")
            return seen.get(ref, element) if ref is not None else element

        cells = {child.tag: resolve(child) for child in row}
        time_ns = int(cells["sample-time"].text)
        thread = cells["thread"].get("fmt", "")
        weight = int(cells["weight"].text) if cells.get("weight") is not None and cells["weight"].text else 1_000_000
        stack = cells.get("tagged-backtrace") or cells.get("backtrace")
        frames = []
        if stack is not None:
            for frame in stack.iter("frame"):
                frame = resolve(frame)
                frames.append(frame.get("name") or frame.get("addr") or "?")
        yield time_ns, thread, weight, frames


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--frames", type=Path, help="live-ui frames.jsonl for phase bounds")
    parser.add_argument("--phase", help="scenario phase to keep")
    parser.add_argument("--thread", default="Main Thread", help="thread name substring ('' for all)")
    parser.add_argument("--top", type=int, default=40)
    parser.add_argument("--demangler", type=Path)
    parser.add_argument("--focus", help="regex; keep stacks through a matching frame, report its callees")
    args = parser.parse_args()

    window = None
    if args.frames and args.phase:
        bounds = {}
        for line in args.frames.read_text().splitlines():
            record = json.loads(line)
            if record.get("event") == "scenario_phase" and record["detail"]["name"] == args.phase:
                bounds[record["detail"]["state"]] = record["unix_ms"]
        start = trace_start_unix_ms(args.trace)
        window = ((bounds["begin"] - start) * 1e6, (bounds["end"] - start) * 1e6)

    rows = [s for s in samples(export(args.trace, "time-profile"))
            if (window is None or window[0] <= s[0] <= window[1]) and args.thread in s[1]]
    names = sorted({name for _, _, _, frames in rows for name in frames})
    if args.demangler:
        out = subprocess.run([str(args.demangler)], input="\n".join(names), capture_output=True, text=True).stdout
        mapping = dict(zip(names, out.splitlines()))
    else:
        mapping = {name: name for name in names}
    clean = lambda name: re.sub(r"::h[0-9a-f]{16}$", "", mapping.get(name, name))  # noqa: E731

    focus = re.compile(args.focus) if args.focus else None
    total = 0
    self_time, inclusive = collections.Counter(), collections.Counter()
    for _, _, weight, frames in rows:
        frames = [clean(name) for name in frames]
        if focus:
            index = next((i for i, name in enumerate(frames) if focus.search(name)), None)
            if index is None:
                continue
            frames = frames[:index + 1]
        total += weight
        if frames:
            self_time[frames[0]] += weight
        for name in set(frames):
            inclusive[name] += weight
    span = (window[1] - window[0]) / 1e9 if window else None
    print(f"samples: {len(rows)}  weight: {total / 1e6:.0f} ms" + (f" over {span:.2f} s" if span else ""))
    print("\nself time:")
    for name, weight in self_time.most_common(args.top):
        print(f"  {weight / 1e6:8.1f} ms  {100 * weight / max(total, 1):5.1f}%  {name[:160]}")
    print("\ninclusive time:")
    for name, weight in inclusive.most_common(args.top):
        print(f"  {weight / 1e6:8.1f} ms  {100 * weight / max(total, 1):5.1f}%  {name[:160]}")


if __name__ == "__main__":
    main()
