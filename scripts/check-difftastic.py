#!/usr/bin/env python3

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

PIN = Path(__file__).resolve().parents[1] / "crates/gitcomet-difftastic/upstream.json"


def changed_tokens(chunks: list) -> list[str]:
    contents = []
    for chunk in chunks:
        if not isinstance(chunk, list):
            raise RuntimeError("helper returned an invalid change chunk")
        for line in chunk:
            if not isinstance(line, dict):
                raise RuntimeError("helper returned an invalid changed line")
            for side in [line.get("lhs"), line.get("rhs")]:
                if side is None:
                    continue
                if not isinstance(side, dict) or not isinstance(side.get("changes"), list):
                    raise RuntimeError("helper returned invalid token changes")
                for change in side["changes"]:
                    if not isinstance(change, dict) or not isinstance(change.get("content"), str):
                        raise RuntimeError("helper returned invalid token content")
                    contents.append(change["content"])
    return contents


def check(helper: Path, notices: Path) -> None:
    version = json.loads(PIN.read_text(encoding="utf-8"))["version"]
    if not notices.is_file() or notices.stat().st_size == 0:
        raise RuntimeError(f"missing Difftastic notices: {notices}")
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("DFT_", "GIT_")) and key != "RUST_LOG"}
    version_output = subprocess.check_output([str(helper), "--version"], env=environment, timeout=5)
    if not version_output.splitlines() or version_output.splitlines()[0] != f"Difftastic {version}".encode():
        raise RuntimeError("bundled Difftastic version does not match the pin")
    with tempfile.TemporaryDirectory(prefix="gitcomet-helper-smoke-") as temporary:
        old = Path(temporary) / "old.js"
        new = Path(temporary) / "new.js"
        old.write_bytes(b"const answer = 1;\n")
        new.write_bytes(b"const answer = 2;\n")
        environment["DFT_UNSTABLE"] = "yes"
        output = subprocess.check_output([
            str(helper), "--display=json", "--color=never", "--strip-cr=off",
            "--parse-error-limit=0", "--byte-limit=1000000", "--graph-limit=3000000",
            "--", str(old), str(new),
        ], env=environment, timeout=5)
        comparison = json.loads(output)
        if not isinstance(comparison, dict) or comparison.get("status") != "changed" or comparison.get("language") != "JavaScript":
            raise RuntimeError("helper did not produce a structural JavaScript comparison")
        aligned = comparison.get("aligned_lines")
        chunks = comparison.get("chunks")
        if not isinstance(aligned, list) or [0, 0] not in aligned or not isinstance(chunks, list) or not chunks:
            raise RuntimeError("helper output is missing source alignment or token changes")
        changed = changed_tokens(chunks)
        if "1" not in changed or "2" not in changed:
            raise RuntimeError("helper output does not identify both changed tokens")
    print(f"Difftastic {version} packaged helper passed: {helper}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Verify the installed Difftastic helper, notices, and JSON protocol.")
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--notices", type=Path, required=True)
    args = parser.parse_args()
    try:
        check(args.helper.resolve(), args.notices.resolve())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"check-difftastic: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
