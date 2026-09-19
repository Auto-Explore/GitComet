#!/usr/bin/env python3
"""Bounded dependency caches. Never cache credentials, workspace code, or test results."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[2]
PREFIX = "gitcomet-ci-v1-"


def output(key, value):
    print(f"{key}={value}")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write(f"{key}={value}\n")


def cache_key(context):
    digest = hashlib.sha256()
    for pattern in ("Cargo.lock", "**/Cargo.toml", ".cargo/*.toml", "rust-toolchain.toml",
                    "scripts/ci/cache.py", "scripts/windows/msvc-linker.cmd"):
        for path in sorted(ROOT.glob(pattern)):
            if "target" in path.relative_to(ROOT).parts or ".git" in path.parts:
                continue
            digest.update(str(path.relative_to(ROOT)).encode())
            digest.update(path.read_bytes())
    digest.update(subprocess.check_output(["rustc", "-vV"]))
    digest.update(platform.platform().encode())
    for name, value in sorted(os.environ.items()):
        if name in ("ImageOS", "ImageVersion") or name.startswith(("CARGO_PROFILE_", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CC", "CXX", "CFLAGS", "CMAKE")):
            digest.update(f"{name}={value}".encode())
    return f"{PREFIX}{context}-{digest.hexdigest()[:24]}"


def dependency_entries(target, metadata):
    """Cache complete external dependency artifacts, never partial file groups."""
    names = set()
    for package in metadata["packages"]:
        if not package.get("source"):
            continue  # Workspace and vendored path dependencies rebuild from checkout.
        names.add(package["name"])
        names.update(item["name"].replace("-", "_") for item in package["targets"])
    lib_names = names | {"lib" + name.replace("-", "_") for name in names}
    for profile in ("ci-test", "ci-bench"):
        for kind in ("build", ".fingerprint", "deps"):
            directory = target / profile / kind
            if not directory.is_dir():
                continue
            for path in sorted(directory.iterdir()):
                stem = re.sub(r"-[0-9a-f]{16}(?:\..*)?$", "", path.name)
                if stem in (lib_names if kind == "deps" else names):
                    yield path, "target/" + str(path.relative_to(target)).replace(os.sep, "/")


def source_entries(cargo_home):
    # Preserve source timestamps with compiled artifacts (including -sys crates).
    for name in ("registry/cache", "registry/index", "registry/src", "git/db", "git/checkouts"):
        path = cargo_home / name
        if path.is_dir():
            yield path, "cargo/" + name


def write_bundle(destination, entries):
    def allowed(member):
        # Git checkouts can contain build artifacts; no need to archive those.
        if "target" in Path(member.name).parts[1:-1] and member.name.startswith("cargo/"):
            return None
        return member

    destination.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(destination, "w:gz", compresslevel=6) as archive:
        for source, name in entries:
            archive.add(source, arcname=name, filter=allowed)


def pack(destination, cargo_home, target, budget, compiled):
    start = time.monotonic()
    sources = list(source_entries(cargo_home))
    entries = list(sources)
    mode = "sources"
    if compiled:
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
        entries += list(dependency_entries(target, metadata))
        mode = "dependencies"
    write_bundle(destination, entries)
    attempts = {mode: destination.stat().st_size}
    if destination.stat().st_size > budget and compiled:
        # Never publish an oversized cache which evicts other platforms.
        mode = "sources"
        entries = sources
        write_bundle(destination, entries)
        attempts[mode] = destination.stat().st_size
    if destination.stat().st_size > budget:
        # Compressed crate downloads give useful reuse even on very small budgets.
        mode = "downloads"
        downloads = cargo_home / "registry/cache"
        entries = [(downloads, "cargo/registry/cache")] if downloads.is_dir() else []
        write_bundle(destination, entries)
        attempts[mode] = destination.stat().st_size
    size = destination.stat().st_size
    save = size <= budget and bool(entries)
    output("save", str(save).lower())
    output("bytes", size)
    output("mode", mode)
    report = dict(bytes=size, budget=budget, mode=mode, save=save, attempts=attempts,
                  seconds=round(time.monotonic() - start, 3))
    report_dir = ROOT / "target/ci-reports"
    report_dir.mkdir(parents=True, exist_ok=True)
    (report_dir / "cache.json").write_text(json.dumps(report, indent=2) + "\n")
    if not save:
        destination.unlink()
        print("Cache is empty or exceeds its budget; leaving this cache unsaved")


def restore(bundle, cargo_home, target):
    if not bundle.exists():
        return
    # A cache archive can only populate these two designated build/cache roots.
    # Extract each member with Python's data filter, including symlink checks.
    if not hasattr(tarfile, "data_filter"):
        raise RuntimeError("Cache extraction requires a Python release with tarfile.data_filter")
    with tarfile.open(bundle, "r:gz") as archive:
        for member in archive:
            prefix, separator, relative = member.name.partition("/")
            if prefix not in ("cargo", "target"):
                raise ValueError(f"Invalid cache member: {member.name}")
            if not separator:
                continue
            destination = cargo_home if prefix == "cargo" else target
            member.name = relative
            if member.islnk():
                link_prefix, _, member.linkname = member.linkname.partition("/")
                if link_prefix != prefix:
                    raise ValueError("Cross-root hard link in cache")
            archive.extract(member, destination, filter="data")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["key", "pack", "restore"])
    parser.add_argument("--context", default="local")
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    parser.add_argument("--target", type=Path, default=ROOT / "target")
    parser.add_argument("--budget-mib", type=int, default=700)
    parser.add_argument("--compiled", action="store_true")
    args = parser.parse_args()
    if args.operation == "key":
        output("key", cache_key(args.context))
    elif args.operation == "pack":
        pack(args.bundle, args.cargo_home, args.target, args.budget_mib * 1024**2, args.compiled)
    else:
        restore(args.bundle, args.cargo_home, args.target)


if __name__ == "__main__":
    main()
