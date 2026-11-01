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
import sys
import tarfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]
REPORTS = ROOT / "target/ci-reports"
PREFIX = "gitcomet-ci-v2-"
MIB = 1024**2


def load_budgets(path):
    budgets = tomllib.loads(path.read_text(encoding="utf-8"))
    if set(budgets) != {"allowance_bytes", "sources_mib", "compiled_mib"}:
        raise ValueError(f"{path.name} must define exactly allowance_bytes, sources_mib and [compiled_mib]")
    return budgets["allowance_bytes"], budgets["sources_mib"], budgets["compiled_mib"]


CACHE_ALLOWANCE_BYTES, SOURCES_BUDGET_MIB, COMPILED_BUDGETS_MIB = load_budgets(
    Path(__file__).with_name("cache-budgets.toml"))
UNIT_HASH = re.compile(r"-([0-9a-f]{16})(?:\..*)?$")
PACKAGE_ID = re.compile(r"(registry|sparse|git|path)\+[^#]+#(?:([^@#]+)@)?(\S+)")


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
                 ROOT / "scripts/linux/mold-linker.sh", ROOT / "scripts/install-mold.py",
                 ROOT / "scripts/windows/windows-lld-linker.cmd", ROOT / "scripts/ci/cache.py",
                 ROOT / "scripts/ci/run.py"]:
        if path.exists():
            compatibility.update(path.read_bytes())
    compatibility.update(subprocess.check_output(["rustc", "-vV"]))
    compatibility.update(platform.platform().encode())
    for name, value in sorted(os.environ.items()):
        # Windows exposes environment-variable names in uppercase.
        if name.upper() in ("IMAGEOS", "IMAGEVERSION") or name.startswith(("CARGO_PROFILE_", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CC", "CXX", "CFLAGS", "CMAKE")):
            compatibility.update(f"{name}={value}".encode())
    restore_key = f"{PREFIX}compiled-{context}-{compatibility.hexdigest()[:16]}-"
    # Sources restore by prefix, so a packing change must not fall back to older bundles.
    layout = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()[:16]
    source_prefix = f"{PREFIX}sources-{platform.system().lower()}-{layout}-"
    dependency_hash = dependencies.hexdigest()[:16]
    return {"key": restore_key + dependency_hash, "restore-key": restore_key,
            "source-key": source_prefix + dependency_hash, "source-restore-key": source_prefix}


def build_record():
    """Unit hashes and registry crates this job's cargo builds compiled or found fresh, from run.py's messages."""
    logs = sorted(REPORTS.glob("*/cargo-messages.jsonl"))
    if not logs:
        raise RuntimeError(f"no */cargo-messages.jsonl under {REPORTS}; compile with scripts/ci/run.py before packing")
    units, crates = set(), set()
    for log in logs:
        for line in log.read_text(encoding="utf-8").splitlines():
            message = json.loads(line)
            if "package_id" in message:
                package = PACKAGE_ID.fullmatch(message["package_id"])
                if not package or (package[1] in ("registry", "sparse") and not package[2]):
                    raise RuntimeError(f"unrecognised cargo package id {message['package_id']!r} in {log}")
                if package[1] in ("registry", "sparse"):
                    crates.add(f"{package[2]}-{package[3]}")
            paths = [*message.get("filenames", []), *([message["out_dir"]] if "out_dir" in message else [])]
            for path in map(Path, paths):
                # deps/libname-HASH.rlib, build/name-HASH/build-script-build, build/name-HASH/out
                match = UNIT_HASH.search(path.name) or UNIT_HASH.search(path.parent.name)
                if match:
                    units.add(match[1])
    return units, crates


def dependency_entries(target, metadata, units):
    """Complete external units this job used; a prefix restore's stale units stay behind."""
    names = set()
    for package in metadata["packages"]:
        if not package.get("source"):
            continue  # Workspace and vendored path dependencies rebuild from checkout.
        names.add(package["name"])
        names.update(item["name"].replace("-", "_") for item in package["targets"])
    lib_names = names | {"lib" + name.replace("-", "_") for name in names}
    for kind in ("build", ".fingerprint", "deps"):
        directory = target / "ci-test" / kind
        if not directory.is_dir():
            continue
        for path in sorted(directory.iterdir()):
            match = UNIT_HASH.search(path.name)
            if match and match[1] in units and path.name[:match.start()] in (lib_names if kind == "deps" else names):
                yield path, "target/" + str(path.relative_to(target)).replace(os.sep, "/")


def cargo_metadata():
    return json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))


def registry_crates(metadata):
    return {f"{package['name']}-{package['version']}" for package in metadata["packages"]
            if (package.get("source") or "").startswith("registry+")}


def registry_entries(cargo_home, kind, crates):
    # Only the current lockfile's crates, so prefix restores never accumulate old versions.
    root = cargo_home / "registry" / kind
    for index in sorted(root.iterdir()) if root.is_dir() else ():
        for path in sorted(index.iterdir()):
            if path.name.removesuffix(".crate") in crates:
                yield path, f"cargo/registry/{kind}/{index.name}/{path.name}"


def directory_entries(cargo_home, *names):
    for name in names:
        if (cargo_home / name).is_dir():
            yield cargo_home / name, "cargo/" + name


def git_checkouts(cargo_home, metadata):
    """Checkouts of the revisions the lockfile pins; cargo names each by a short prefix of its commit."""
    revisions = {package["source"].rpartition("#")[2] for package in metadata["packages"]
                 if (package.get("source") or "").startswith("git+")}
    root = cargo_home / "git/checkouts"
    return [checkout for repository in (sorted(root.iterdir()) if root.is_dir() else ())
            for checkout in sorted(repository.iterdir())
            if any(revision.startswith(checkout.name) for revision in revisions)]


def fetched_entries(cargo_home, crates, checkouts):
    """What cargo downloads otherwise: crate files (needed even when unpacked), the index, git databases."""
    yield from registry_entries(cargo_home, "cache", crates)
    yield from directory_entries(cargo_home, "registry/index")
    for repository in sorted({checkout.parent.name for checkout in checkouts}):
        yield from directory_entries(cargo_home, f"git/db/{repository}")


def extracted_entries(cargo_home, crates, checkouts):
    """Unpacked sources of built crates keep the timestamps their build-script fingerprints compare against."""
    yield from registry_entries(cargo_home, "src", crates)
    for checkout in checkouts:
        yield checkout, f"cargo/git/checkouts/{checkout.parent.name}/{checkout.name}"


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


def pack(destination, entries, *, mode, budget_mib):
    """Write one bundle; one over its budget fails rather than evicting other lanes or shrinking silently."""
    start = time.monotonic()
    if not entries:
        raise RuntimeError(f"nothing to cache for the {mode} bundle")
    write_bundle(destination, entries, mode=mode)
    size, budget = destination.stat().st_size, budget_mib * MIB
    report = dict(mode=mode, bytes=size, budget=budget, seconds=round(time.monotonic() - start, 3))
    REPORTS.mkdir(parents=True, exist_ok=True)
    (REPORTS / f"cache-{mode}.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if size > budget:
        destination.unlink()
        raise RuntimeError(f"{mode} cache bundle is {size // MIB} MiB, over its {budget_mib} MiB budget; "
                           "raise it in scripts/ci/cache-budgets.toml within the repository cache allowance")
    print(f"Packed {mode} cache: {size // MIB} of {budget_mib} MiB")


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
                if manifest.get("version") != 2 or manifest.get("mode") not in ("compiled", "sources"):
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
    REPORTS.mkdir(parents=True, exist_ok=True)
    unpack = REPORTS / "cache-unpack.json"
    matched = os.environ.get("CI_CACHE_MATCHED_KEY") or os.environ.get("CI_SOURCE_MATCHED_KEY")
    if os.environ.get("CI_COLD_CACHE") == "true":
        details = {"mode": "bypassed"}
    else:
        details = json.loads(unpack.read_text(encoding="utf-8")) if matched and unpack.exists() else {"mode": "miss"}
    details.update({name: os.environ.get(name, "") for name in (
        "CI_CACHE_HIT", "CI_CACHE_RESTORED", "CI_COLD_CACHE", "CI_CACHE_CONTEXT",
        "CI_CACHE_KEY", "CI_CACHE_MATCHED_KEY", "CI_SOURCE_KEY", "CI_SOURCE_MATCHED_KEY")})
    (REPORTS / "cache-restore.json").write_text(json.dumps(details, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(details, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["key", "pack-compiled", "pack-sources", "restore", "state"])
    parser.add_argument("--context", default="local")
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    parser.add_argument("--target", type=Path, default=ROOT / "target")
    args = parser.parse_args()
    if args.operation == "key":
        for key, value in cache_keys(args.context).items():
            output(key, value)
        output("compiled", str(args.context in COMPILED_BUDGETS_MIB).lower())
    elif args.operation == "state":
        report_restore()
    elif args.operation == "pack-sources":
        metadata = cargo_metadata()
        entries = fetched_entries(args.cargo_home, registry_crates(metadata), git_checkouts(args.cargo_home, metadata))
        pack(args.bundle, list(entries), mode="sources",
             budget_mib=SOURCES_BUDGET_MIB)
    elif args.operation == "pack-compiled":
        if args.context not in COMPILED_BUDGETS_MIB:
            parser.error(f"{args.context} has no compiled bundle budget in cache-budgets.toml")
        units, built_crates = build_record()
        metadata = cargo_metadata()
        checkouts = git_checkouts(args.cargo_home, metadata)
        entries = [*fetched_entries(args.cargo_home, registry_crates(metadata), checkouts),
                   *extracted_entries(args.cargo_home, built_crates, checkouts),
                   *dependency_entries(args.target, metadata, units)]
        pack(args.bundle, entries, mode="compiled", budget_mib=COMPILED_BUDGETS_MIB[args.context])
    else:
        report = restore(args.bundle, args.cargo_home, args.target)
        REPORTS.mkdir(parents=True, exist_ok=True)
        (REPORTS / "cache-unpack.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        print(f"::error::{error}", file=sys.stderr)
        sys.exit(1)
