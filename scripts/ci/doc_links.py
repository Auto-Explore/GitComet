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
# The target of an inline link or image, whatever its text holds: in
# `[![badge](img.svg)](LICENSE)` both targets count.
TARGET = re.compile(r"\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
CODE_SPAN = re.compile(r"(`+)(?:(?!\1).)+?\1")
SKIPPED = re.compile(r"(?:^|/)(?:vendor|fixtures|test_data)/")


def targets(text):
    """(line, target) for every inline link outside code."""
    found, fence = [], None
    for number, line in enumerate(text.splitlines(), 1):
        opening = FENCE.match(line)
        if fence is None and opening:
            fence = opening.group(1)
            continue
        if fence is not None:
            # Only a fence of the same character, at least as long, closes it.
            closing = opening and not opening.group(2).strip()
            if closing and opening.group(1)[0] == fence[0] and len(opening.group(1)) >= len(fence):
                fence = None
            continue
        line = CODE_SPAN.sub("", line)
        found.extend((number, match.group(1)) for match in TARGET.finditer(line))
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
