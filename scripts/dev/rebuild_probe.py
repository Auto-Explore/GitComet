#!/usr/bin/env python3
"""Time an incremental rebuild after touching one source file.

    rebuild_probe.py --touch crates/gitcomet-ui-gpui/src/view/mod.rs --runs 3 \\
        -- cargo check --locked -p gitcomet-ui-gpui

The file's modification time is bumped, its content is not changed. Run the
command once beforehand so the first sample is not a cold build.
"""

import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--touch", required=True, help="source file whose change is simulated")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("command", nargs=argparse.REMAINDER, help="-- build command")
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("give the build command after --")
    touched = ROOT / args.touch
    seconds = []
    for _ in range(args.runs):
        os.utime(touched)
        started = time.monotonic()
        subprocess.run(command, cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        seconds.append(round(time.monotonic() - started, 2))
    print(json.dumps({"touch": args.touch, "command": command, "seconds": seconds,
                      "median_seconds": statistics.median(seconds)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
