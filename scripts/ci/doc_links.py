#!/usr/bin/env python3
"""Fail when a Markdown file links to a repository path that does not exist.

Checks relative links in tracked Markdown outside `vendor/`. Web links, mail
links and same-page anchors are not followed; fenced code is skipped.
"""

from pathlib import Path
import posixpath
import re
import subprocess
import sys
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[2]
LINK = re.compile(r"\[[^\]\n]*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
FENCE = re.compile(r"^(```|~~~)")
SKIPPED = re.compile(r"(?:^|/)(?:vendor|fixtures|test_data)/")


def targets(text):
    """(line, target) for every inline link outside fenced code."""
    found, fenced = [], False
    for number, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line.strip()):
            fenced = not fenced
            continue
        if not fenced:
            found.extend((number, match.group(1)) for match in LINK.finditer(line))
    return found


def is_local(target):
    return not re.match(r"[a-zA-Z][a-zA-Z0-9+.-]*:", target) and not target.startswith("#")


def broken(path, text, exists):
    """Problems as text. `exists(repo_relative_path)` answers for the file system."""
    problems = []
    for line, target in targets(text):
        if not is_local(target):
            continue
        file = unquote(target.split("#", 1)[0])
        candidate = posixpath.normpath(file.lstrip("/") if file.startswith("/")
                                       else posixpath.join(posixpath.dirname(path), file))
        if candidate.startswith("..") or not exists(candidate):
            problems.append(f"{path}:{line}: link target '{target}' does not exist")
    return problems


def main():
    files = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"],
                           cwd=ROOT, stdout=subprocess.PIPE, check=True).stdout.decode().split("\0")
    files = sorted(path for path in files if path and not SKIPPED.search(path) and (ROOT / path).is_file())
    problems = []
    for path in files:
        problems += broken(path, (ROOT / path).read_text(encoding="utf-8"), lambda target: (ROOT / target).exists())
    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if not problems:
        print(f"Documentation link check passed ({len(files)} files).")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
