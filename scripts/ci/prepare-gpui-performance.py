#!/usr/bin/env python3
"""Prepare the reviewable GPUI Windows patch without changing Cargo's global cache."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "https://github.com/Havunen/gpui-ce.git"
BASE = "9ff6e7f487f9cba1a8d5a1fc45aa173d094ff9e7"
PATCH = ROOT / "patches/gpui-windows-responsiveness.patch"


def git(checkout, *args):
    return subprocess.check_output(["git", "-c", "core.autocrlf=false", "-C", str(checkout), *args])


def prepare(checkout, config):
    checkout = checkout.resolve()
    if checkout == ROOT or ROOT in checkout.parents:
        raise ValueError("Use a checkout outside GitComet; nested workspaces have ambiguous dependency inheritance")
    patch = PATCH.read_bytes()
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    packages = {p["name"] for p in lock["package"] if p.get("source", "").startswith("git+" + SOURCE)}
    if not packages or any(BASE not in p["source"] for p in lock["package"] if p["name"] in packages):
        raise ValueError("The GPUI pin changed; rebase and review the patch before applying it")
    if not checkout.exists():
        subprocess.run(["git", "-c", "core.autocrlf=false", "clone", "--no-checkout", SOURCE, str(checkout)], check=True)
        git(checkout, "checkout", "--detach", BASE)
    head = git(checkout, "rev-parse", "HEAD").decode().strip()
    if head == BASE:
        current = git(checkout, "diff", "--binary", "HEAD", "--")
        if current != patch:
            if git(checkout, "status", "--porcelain").strip():
                raise ValueError("Checkout has unrelated changes; use a new checkout")
            git(checkout, "apply", "--check", str(PATCH))
            git(checkout, "apply", str(PATCH))
        elif git(checkout, "ls-files", "--others", "--exclude-standard").strip():
            raise ValueError("Checkout has unrelated untracked files; use a new checkout")
    else:
        # A locally prepared review commit is also reproducible, provided its
        # entire change is exactly this patch and no working changes remain.
        if git(checkout, "status", "--porcelain").strip():
            raise ValueError("Committed review checkout must be clean")
        if (git(checkout, "show", "-s", "--format=%P", "HEAD").decode().strip() != BASE
                or git(checkout, "diff", "--binary", BASE, "HEAD", "--") != patch):
            raise ValueError(f"Checkout must be at {BASE} or its exact review-patch commit")
    manifests = git(checkout, "ls-files", "**/Cargo.toml").decode().splitlines()
    paths = {}
    for manifest in manifests:
        data = tomllib.loads((checkout / manifest).read_text(encoding="utf-8"))
        name = data.get("package", {}).get("name")
        if name in packages:
            paths[name] = (checkout / manifest).parent.as_posix()
    if paths.keys() != packages:
        raise ValueError(f"Missing GPUI packages: {packages - paths.keys()}")
    config.parent.mkdir(parents=True, exist_ok=True)
    overrides = f'[patch."{SOURCE}"]\n' + "".join(
        f'{json.dumps(name)} = {{ path = {json.dumps(path)} }}\n' for name, path in sorted(paths.items()))
    # Git dependencies use non-incremental compilation (16 codegen units).
    # Path dependencies would otherwise inherit incremental dev compilation
    # and change generated code independently of the patch being measured.
    overrides += "\n# Match the pinned Git dependencies' compilation strategy.\n"
    overrides += "".join(f'\n[profile.dev.package.{json.dumps(name)}]\nincremental = false\ncodegen-units = 16\n'
                         for name in sorted(packages))
    config.write_text(overrides, encoding="utf-8")
    config.with_suffix(".json").write_text(json.dumps({
        "base": BASE, "head": head, "committed_patch": head != BASE,
        "patch_sha256": hashlib.sha256(patch).hexdigest(),
        "checkout": str(checkout), "packages": sorted(packages),
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Prepared {len(packages)} path overrides in {config.resolve()}")
    print("This is a local experiment. The normal dependency pin is unchanged until the fork patch is published.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=ROOT / "target/gpui-performance.toml")
    args = parser.parse_args()
    prepare(args.checkout, args.config)


if __name__ == "__main__":
    main()
