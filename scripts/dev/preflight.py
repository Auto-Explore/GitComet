#!/usr/bin/env python3
"""Run the quick checks that otherwise fail a whole CI run, before pushing.

Every check runs even after one fails, so a single pass lists everything to fix.
"""

from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
CHECKS = (
    ("formatting", ["cargo", "fmt", "--all", "--check"]),
    ("source layout", ["bash", "scripts/check-rust-source-size.sh"]),
    ("identity literals", [sys.executable, "scripts/ci/identity_literals.py"]),
    ("scorecard", [sys.executable, "scripts/ci/scorecard.py", "check"]),
    ("documentation links", [sys.executable, "scripts/ci/doc_links.py"]),
    ("licence list", ["bash", "scripts/generate-open-source-licenses.sh", "--check"]),
    ("crate boundaries", [sys.executable, "scripts/ci/boundaries.py"]),
)


def main():
    failed = []
    for name, command in CHECKS:
        started = time.monotonic()
        result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        print(f"{'ok  ' if result.returncode == 0 else 'FAIL'} {name} ({time.monotonic() - started:.1f}s)")
        if result.returncode:
            failed.append(name)
            print(result.stdout.rstrip())
    if failed:
        print(f"\n{len(failed)} check(s) failed: {', '.join(failed)}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
