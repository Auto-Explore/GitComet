#!/usr/bin/env python3
"""Bounded dependency caches. Never cache credentials, workspace code, or test results."""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tarfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]
PREFIX = "gitcomet-ci-v2-"


def output(key, value):
    print(f"{key}={value}")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write(f"{key}={value}\n")


def cache_keys(context):
    dependencies = hashlib.sha256((ROOT / "Cargo.lock").read_bytes())
    compatibility = hashlib.sha256(context.encode())
    for directory, dirs, files in os.walk(ROOT):
        # Local comparison checkouts are independent workspaces. Traversing
        # them both slows hashing and makes their manifests invalidate ours.
        dirs[:] = sorted(name for name in dirs if name not in ("target", ".git", ".worktrees"))
        if "Cargo.toml" not in files:
            continue
        path = Path(directory) / "Cargo.toml"
        manifest = tomllib.loads(path.read_text(encoding="utf-8"))
        declarations = {key: value for key, value in manifest.items()
                        if key in ("dependencies", "dev-dependencies", "build-dependencies",
                                   "target", "features", "workspace", "patch", "replace")}
        dependencies.update(str(path.relative_to(ROOT)).encode())
        dependencies.update(json.dumps(declarations, sort_keys=True).encode())
        compatibility.update(json.dumps(manifest.get("profile", {}), sort_keys=True).encode())
    for path in [*sorted((ROOT / ".cargo").glob("*.toml")), ROOT / "rust-toolchain.toml",
                 ROOT / "scripts/windows/msvc-linker.cmd", ROOT / "scripts/ci/cache.py",
                 ROOT / "scripts/ci/run.py"]:
        if path.exists():
            compatibility.update(path.read_bytes())
    compatibility.update(subprocess.check_output(["rustc", "-vV"]))
    compatibility.update(platform.platform().encode())
    for name, value in sorted(os.environ.items()):
        # Windows exposes environment-variable names in uppercase.
        if name.upper() in ("IMAGEOS", "IMAGEVERSION") or name.startswith(("CARGO_PROFILE_", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CC", "CXX", "CFLAGS", "CMAKE")):
            compatibility.update(f"{name}={value}".encode())
    restore_key = f"{PREFIX}deps-{context}-{compatibility.hexdigest()[:16]}-"
    # Sources restore by prefix, so a packing change must not fall back to older bundles.
    layout = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()[:16]
    source_prefix = f"{PREFIX}sources-{platform.system().lower()}-{layout}-"
    dependency_hash = dependencies.hexdigest()[:16]
    return {"key": restore_key + dependency_hash, "restore-key": restore_key,
            "source-key": source_prefix + dependency_hash, "source-restore-key": source_prefix}


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


def write_bundle(destination, entries, *, mode=None):
    def allowed(member):
        # Skip build output at a git checkout's root only; crates have `target` modules (cc).
        parts = Path(member.name).parts
        if parts[:3] == ("cargo", "git", "checkouts") and parts[5:] == ("target",):
            return None
        return member

    destination.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(destination, "w:gz", compresslevel=6) as archive:
        if mode:
            data = json.dumps({"version": 2, "mode": mode}).encode()
            member = tarfile.TarInfo("cache-manifest.json")
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
        for source, name in entries:
            archive.add(source, arcname=name, filter=allowed)


def pack(destination, cargo_home, target, budget, compiled, report_name="cache"):
    start = time.monotonic()
    sources = list(source_entries(cargo_home))
    entries = list(sources)
    mode = "sources"
    if compiled:
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
        artifacts = list(dependency_entries(target, metadata))
        entries += artifacts
        if artifacts:
            mode = "dependencies"
    write_bundle(destination, entries, mode=mode)
    attempts = {mode: destination.stat().st_size}
    if destination.stat().st_size > budget and compiled:
        # Never publish an oversized cache which evicts other platforms.
        mode = "sources"
        entries = sources
        write_bundle(destination, entries, mode=mode)
        attempts[mode] = destination.stat().st_size
    if destination.stat().st_size > budget:
        # Compressed crate downloads give useful reuse even on very small budgets.
        mode = "downloads"
        downloads = cargo_home / "registry/cache"
        entries = [(downloads, "cargo/registry/cache")] if downloads.is_dir() else []
        write_bundle(destination, entries, mode=mode)
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
    (report_dir / f"{report_name}.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if compiled and mode != "dependencies":
        print(f"::warning::Compiled dependency cache fell back to {mode}; compilation reuse is unavailable")
    if not save:
        destination.unlink()
        print("Cache is empty or exceeds its budget; leaving this cache unsaved")


def restore(bundle, cargo_home, target):
    start = time.monotonic()
    if not bundle.exists():
        return {"mode": "miss", "seconds": 0, "bytes": 0}
    # A cache archive can only populate these two designated build/cache roots.
    # Extract each member with Python's data filter, including symlink checks.
    if not hasattr(tarfile, "data_filter"):
        raise RuntimeError("Cache extraction requires a Python release with tarfile.data_filter")
    mode = "unknown"
    with tarfile.open(bundle, "r:gz") as archive:
        for member in archive:
            if member.name == "cache-manifest.json":
                if not member.isfile() or member.size > 4096:
                    raise ValueError("Invalid cache manifest")
                manifest = json.load(archive.extractfile(member))
                if manifest.get("version") != 2 or manifest.get("mode") not in ("dependencies", "sources", "downloads"):
                    raise ValueError("Unsupported cache manifest")
                mode = manifest["mode"]
                continue
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
    return {"mode": mode, "seconds": round(time.monotonic() - start, 3), "bytes": bundle.stat().st_size}


def report_restore():
    directory = ROOT / "target/ci-reports"
    directory.mkdir(parents=True, exist_ok=True)
    unpack = directory / "cache-unpack.json"
    matched = os.environ.get("CI_CACHE_MATCHED_KEY") or os.environ.get("CI_SOURCE_MATCHED_KEY")
    if os.environ.get("CI_COLD_CACHE") == "true":
        details = {"mode": "bypassed"}
    else:
        details = json.loads(unpack.read_text(encoding="utf-8")) if matched and unpack.exists() else {"mode": "miss"}
    details.update({name: os.environ.get(name, "") for name in (
        "CI_CACHE_HIT", "CI_CACHE_RESTORED", "CI_COLD_CACHE", "CI_CACHE_CONTEXT",
        "CI_CACHE_KEY", "CI_CACHE_MATCHED_KEY", "CI_SOURCE_KEY", "CI_SOURCE_MATCHED_KEY")})
    (directory / "cache-restore.json").write_text(json.dumps(details, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(details, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["key", "pack", "restore", "state"])
    parser.add_argument("--context", default="local")
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    parser.add_argument("--target", type=Path, default=ROOT / "target")
    parser.add_argument("--budget-mib", type=int, default=700)
    parser.add_argument("--compiled", action="store_true")
    parser.add_argument("--report-name", default="cache")
    args = parser.parse_args()
    if args.operation == "key":
        for key, value in cache_keys(args.context).items():
            output(key, value)
    elif args.operation == "state":
        report_restore()
    elif args.operation == "pack":
        pack(args.bundle, args.cargo_home, args.target, args.budget_mib * 1024**2, args.compiled, args.report_name)
    else:
        report = restore(args.bundle, args.cargo_home, args.target)
        directory = ROOT / "target/ci-reports"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "cache-unpack.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
